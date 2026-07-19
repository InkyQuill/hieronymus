use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::Duration,
};

use fs4::FileExt;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::DbError;
use crate::file_identity::{FileIdentity, identity_and_link_count, identity_for_handle};

static MEMORY_MIGRATION_LOCK: LazyLock<Arc<Mutex<()>>> = LazyLock::new(|| Arc::new(Mutex::new(())));
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(10);

pub(super) enum MigrationProtocolLock {
    Memory(OwnedMutexGuard<()>),
    File(FileMigrationLock),
}

pub(super) struct FileMigrationLock {
    file: File,
    path: PathBuf,
}

struct FileLockCandidate {
    file: File,
    path: PathBuf,
    identity: FileIdentity,
    database: DatabaseIdentityGuard,
}

struct DatabaseIdentityGuard {
    file: File,
    path: PathBuf,
    identity: FileIdentity,
}

enum LockAttempt {
    Acquired(FileMigrationLock),
    Contended(FileLockCandidate),
}

impl MigrationProtocolLock {
    pub(super) async fn acquire(pool: &SqlitePool) -> Result<Self, DbError> {
        let row = sqlx::query("SELECT file FROM pragma_database_list WHERE name = 'main'")
            .fetch_one(pool)
            .await
            .map_err(|source| DbError::MigrationLockIdentity { source })?;
        let database_file: Vec<u8> = row
            .try_get(0)
            .map_err(|source| DbError::MigrationLockIdentity { source })?;
        if database_file.is_empty() {
            return Ok(Self::Memory(
                Arc::clone(&MEMORY_MIGRATION_LOCK).lock_owned().await,
            ));
        }

        let database_path = database_path(&database_file)?;
        let mut candidate = tokio::task::spawn_blocking(move || prepare_file_lock(&database_path))
            .await
            .map_err(|source| DbError::MigrationLockWorker {
                operation: "preparation",
                source,
            })??;

        loop {
            match tokio::task::spawn_blocking(move || try_acquire_file_lock(candidate))
                .await
                .map_err(|source| DbError::MigrationLockWorker {
                    operation: "acquisition attempt",
                    source,
                })?? {
                LockAttempt::Acquired(lock) => return Ok(Self::File(lock)),
                LockAttempt::Contended(returned) => candidate = returned,
            }
            tokio::time::sleep(LOCK_RETRY_DELAY).await;
        }
    }

    pub(super) async fn finish(self, result: Result<(), DbError>) -> Result<(), DbError> {
        match self {
            Self::Memory(guard) => {
                drop(guard);
                result
            }
            Self::File(lock) => {
                let path = lock.path.clone();
                let release = tokio::task::spawn_blocking(move || FileExt::unlock(&lock.file))
                    .await
                    .map_err(|source| DbError::MigrationLockWorker {
                        operation: "release",
                        source,
                    })?;
                match release {
                    Ok(()) => result,
                    Err(source) => Err(DbError::MigrationLockRelease {
                        path,
                        outcome: result.as_ref().map_or_else(
                            |error| format!("migration error `{error}`"),
                            |()| "successful migration".to_owned(),
                        ),
                        source,
                    }),
                }
            }
        }
    }
}

#[cfg(unix)]
fn database_path(bytes: &[u8]) -> Result<PathBuf, DbError> {
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
}

#[cfg(windows)]
fn database_path(bytes: &[u8]) -> Result<PathBuf, DbError> {
    String::from_utf8(bytes.to_vec())
        .map(PathBuf::from)
        .map_err(|source| DbError::MigrationLockPath {
            reason: source.to_string(),
        })
}

fn prepare_file_lock(database_path: &Path) -> Result<FileLockCandidate, DbError> {
    let canonical =
        std::fs::canonicalize(database_path).map_err(|source| DbError::MigrationLockIo {
            operation: "canonicalize database path for",
            path: database_path.to_owned(),
            source,
        })?;
    let database = open_database_identity(&canonical)?;
    let (links, database_identity) = identity_and_link_count(&database.file)
        .map_err(|source| identity_error(&canonical, source))?;
    if links > 1 {
        return Err(DbError::UnsupportedDatabaseAlias { links });
    }
    let parent = canonical.parent().ok_or_else(|| DbError::MigrationLockIo {
        operation: "resolve parent of",
        path: canonical.clone(),
        source: io::Error::new(
            io::ErrorKind::InvalidInput,
            "canonical database path has no parent",
        ),
    })?;
    let mut hasher = Sha256::new();
    hasher.update(database_identity.first.to_le_bytes());
    hasher.update(database_identity.second.to_le_bytes());
    let digest = hasher.finalize();
    let path = parent.join(format!(".hieronymus-migrate-{digest:x}.lock"));
    let file = open_sidecar(&path)?;
    let identity = identity_for_handle(&file).map_err(|source| lock_open_error(&path, source))?;
    validate_path_identity(&path, identity)?;
    Ok(FileLockCandidate {
        file,
        path,
        identity,
        database,
    })
}

fn try_acquire_file_lock(candidate: FileLockCandidate) -> Result<LockAttempt, DbError> {
    match FileExt::try_lock_exclusive(&candidate.file) {
        Ok(()) => {
            if let Err(error) = enforce_sidecar_properties(&candidate.path, &candidate.file, false)
                .and_then(|()| validate_path_identity(&candidate.path, candidate.identity))
                .and_then(|()| validate_database_identity(&candidate.database))
            {
                let _ = FileExt::unlock(&candidate.file);
                return Err(error);
            }
            Ok(LockAttempt::Acquired(FileMigrationLock {
                file: candidate.file,
                path: candidate.path,
            }))
        }
        Err(source) if source.kind() == io::ErrorKind::WouldBlock => {
            Ok(LockAttempt::Contended(candidate))
        }
        Err(source) => Err(DbError::MigrationLockIo {
            operation: "try to acquire",
            path: candidate.path,
            source,
        }),
    }
}

fn open_database_identity(path: &Path) -> Result<DatabaseIdentityGuard, DbError> {
    let mut options = OpenOptions::new();
    options.read(true);
    configure_identity_open(&mut options);
    let file = options
        .open(path)
        .map_err(|source| DbError::MigrationLockIo {
            operation: "open database identity for",
            path: path.to_owned(),
            source,
        })?;
    let (_, identity) =
        identity_and_link_count(&file).map_err(|source| identity_error(path, source))?;
    Ok(DatabaseIdentityGuard {
        file,
        path: path.to_owned(),
        identity,
    })
}

fn validate_database_identity(database: &DatabaseIdentityGuard) -> Result<(), DbError> {
    let (links, handle_identity) = identity_and_link_count(&database.file)
        .map_err(|source| identity_error(&database.path, source))?;
    if links > 1 {
        return Err(DbError::UnsupportedDatabaseAlias { links });
    }
    let observed = open_database_identity(&database.path)?;
    if handle_identity == database.identity && observed.identity == database.identity {
        Ok(())
    } else {
        Err(DbError::MigrationLockIo {
            operation: "validate database identity for",
            path: database.path.clone(),
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                "database identity changed during migration-lock acquisition",
            ),
        })
    }
}

fn open_sidecar(path: &Path) -> Result<File, DbError> {
    let mut create = OpenOptions::new();
    create.read(true).write(true).create_new(true);
    configure_secure_open(&mut create, true);
    match create.open(path) {
        Ok(file) => {
            enforce_sidecar_properties(path, &file, true)?;
            Ok(file)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            validate_path_file_type(path)?;
            let mut existing = OpenOptions::new();
            existing.read(true).write(true);
            configure_secure_open(&mut existing, false);
            let file = existing
                .open(path)
                .map_err(|source| lock_open_error(path, source))?;
            enforce_sidecar_properties(path, &file, false)?;
            Ok(file)
        }
        Err(source) => Err(lock_open_error(path, source)),
    }
}

fn lock_open_error(path: &Path, source: io::Error) -> DbError {
    DbError::MigrationLockIo {
        operation: "open",
        path: path.to_owned(),
        source,
    }
}

#[cfg(unix)]
fn configure_secure_open(options: &mut OpenOptions, create: bool) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    if create {
        options.mode(0o600);
    }
}

#[cfg(unix)]
fn configure_identity_open(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
}

#[cfg(windows)]
fn configure_secure_open(options: &mut OpenOptions, _create: bool) {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(windows)]
fn configure_identity_open(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(unix)]
fn validate_path_file_type(path: &Path) -> Result<(), DbError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|source| lock_open_error(path, source))?;
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(lock_open_error(
            path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "migration lock sidecar is not a regular file",
            ),
        ))
    }
}

#[cfg(windows)]
fn validate_path_file_type(path: &Path) -> Result<(), DbError> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|source| lock_open_error(path, source))?;
    if metadata.file_type().is_file()
        && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    {
        Ok(())
    } else {
        Err(lock_open_error(
            path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "migration lock sidecar is not a regular non-reparse file",
            ),
        ))
    }
}

#[cfg(unix)]
fn enforce_sidecar_properties(path: &Path, file: &File, created: bool) -> Result<(), DbError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if created {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|source| DbError::MigrationLockIo {
                operation: "set private permissions on",
                path: path.to_owned(),
                source,
            })?;
    }
    let metadata = file
        .metadata()
        .map_err(|source| lock_open_error(path, source))?;
    let parent = path.parent().ok_or_else(|| {
        lock_open_error(
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "sidecar path has no parent"),
        )
    })?;
    let parent_metadata =
        std::fs::metadata(parent).map_err(|source| lock_open_error(path, source))?;
    if metadata.permissions().mode() & 0o777 != 0o600
        || metadata.nlink() != 1
        || metadata.uid() != parent_metadata.uid()
    {
        return Err(lock_open_error(
            path,
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "migration lock sidecar must be a private 0600 regular file with one link and the directory owner",
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn enforce_sidecar_properties(path: &Path, file: &File, _created: bool) -> Result<(), DbError> {
    let information = windows_file_information(file, path)?;
    validate_windows_regular_file(path, &information)?;
    let links = information.number_of_links();
    if links == 1 {
        Ok(())
    } else {
        Err(lock_open_error(
            path,
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "migration lock sidecar must have exactly one link",
            ),
        ))
    }
}

#[cfg(unix)]
fn validate_path_identity(path: &Path, expected: FileIdentity) -> Result<(), DbError> {
    use std::os::unix::fs::MetadataExt;
    validate_path_file_type(path)?;
    let metadata = std::fs::metadata(path).map_err(|source| lock_open_error(path, source))?;
    let observed = FileIdentity {
        first: metadata.dev(),
        second: metadata.ino(),
    };
    if observed == expected {
        Ok(())
    } else {
        Err(lock_open_error(
            path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "migration lock sidecar identity changed during acquisition",
            ),
        ))
    }
}

#[cfg(windows)]
fn validate_path_identity(path: &Path, expected: FileIdentity) -> Result<(), DbError> {
    validate_path_file_type(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    configure_secure_open(&mut options, false);
    let file = options
        .open(path)
        .map_err(|source| lock_open_error(path, source))?;
    let observed = identity_for_handle(&file).map_err(|source| lock_open_error(path, source))?;
    if observed == expected {
        Ok(())
    } else {
        Err(lock_open_error(
            path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "migration lock sidecar identity changed during acquisition",
            ),
        ))
    }
}

fn identity_error(path: &Path, source: io::Error) -> DbError {
    DbError::MigrationLockIo {
        operation: "inspect file identity for",
        path: path.to_owned(),
        source,
    }
}
