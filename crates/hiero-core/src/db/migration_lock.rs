use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use fs4::FileExt;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::DbError;

static MEMORY_MIGRATION_LOCK: LazyLock<Arc<Mutex<()>>> = LazyLock::new(|| Arc::new(Mutex::new(())));

pub(super) enum MigrationProtocolLock {
    Memory(OwnedMutexGuard<()>),
    File(FileMigrationLock),
}

pub(super) struct FileMigrationLock {
    file: File,
    path: PathBuf,
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
        tokio::task::spawn_blocking(move || acquire_file_lock(&database_path))
            .await
            .map_err(|source| DbError::MigrationLockWorker {
                operation: "acquisition",
                source,
            })?
            .map(Self::File)
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

fn acquire_file_lock(database_path: &Path) -> Result<FileMigrationLock, DbError> {
    let canonical =
        std::fs::canonicalize(database_path).map_err(|source| DbError::MigrationLockIo {
            operation: "canonicalize database path for",
            path: database_path.to_owned(),
            source,
        })?;
    let parent = canonical.parent().ok_or_else(|| DbError::MigrationLockIo {
        operation: "resolve parent of",
        path: canonical.clone(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "canonical database path has no parent",
        ),
    })?;
    let digest = Sha256::digest(path_identity_bytes(&canonical));
    let path = parent.join(format!(".hieronymus-migrate-{digest:x}.lock"));
    let file = open_sidecar(&path)?;
    FileExt::lock_exclusive(&file).map_err(|source| DbError::MigrationLockIo {
        operation: "acquire",
        path: path.clone(),
        source,
    })?;
    Ok(FileMigrationLock { file, path })
}

fn open_sidecar(path: &Path) -> Result<File, DbError> {
    let mut create = OpenOptions::new();
    create.read(true).write(true).create_new(true);
    configure_private_create(&mut create);
    match create.open(path) {
        Ok(file) => {
            enforce_private_permissions(path, &file, true)?;
            Ok(file)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let before =
                std::fs::symlink_metadata(path).map_err(|source| lock_open_error(path, source))?;
            if !before.file_type().is_file() {
                return Err(lock_open_error(
                    path,
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "migration lock sidecar is not a regular file",
                    ),
                ));
            }
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|source| lock_open_error(path, source))?;
            validate_same_file(
                path,
                &before,
                &file
                    .metadata()
                    .map_err(|source| lock_open_error(path, source))?,
            )?;
            enforce_private_permissions(path, &file, false)?;
            Ok(file)
        }
        Err(source) => Err(lock_open_error(path, source)),
    }
}

fn lock_open_error(path: &Path, source: std::io::Error) -> DbError {
    DbError::MigrationLockIo {
        operation: "open",
        path: path.to_owned(),
        source,
    }
}

#[cfg(unix)]
fn configure_private_create(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(windows)]
fn configure_private_create(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn enforce_private_permissions(path: &Path, file: &File, created: bool) -> Result<(), DbError> {
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
    if metadata.permissions().mode() & 0o777 != 0o600 || metadata.nlink() != 1 {
        return Err(lock_open_error(
            path,
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "migration lock sidecar must be a private 0600 file with one link",
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn enforce_private_permissions(_path: &Path, _file: &File, _created: bool) -> Result<(), DbError> {
    Ok(())
}

#[cfg(unix)]
fn validate_same_file(
    path: &Path,
    before: &std::fs::Metadata,
    after: &std::fs::Metadata,
) -> Result<(), DbError> {
    use std::os::unix::fs::MetadataExt;
    if before.dev() == after.dev() && before.ino() == after.ino() {
        Ok(())
    } else {
        Err(lock_open_error(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "migration lock sidecar changed while it was opened",
            ),
        ))
    }
}

#[cfg(windows)]
fn validate_same_file(
    _path: &Path,
    _before: &std::fs::Metadata,
    _after: &std::fs::Metadata,
) -> Result<(), DbError> {
    Ok(())
}

#[cfg(unix)]
fn path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}
