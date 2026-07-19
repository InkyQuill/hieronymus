use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use fs4::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::HieronymusConfig;

const MAX_STATE_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamCyclePaths {
    pub lock_file: PathBuf,
    pub state_json: PathBuf,
}

#[must_use]
pub fn dream_cycle_paths(config: &HieronymusConfig) -> DreamCyclePaths {
    DreamCyclePaths {
        lock_file: config.dream_cycle_lock_path(),
        state_json: config.dream_cycle_state_path(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DreamCycleState {
    pub owner: String,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub token: Uuid,
}

pub struct DreamCycleGuard {
    file: File,
    paths: DreamCyclePaths,
    state: DreamCycleState,
}

impl fmt::Debug for DreamCycleGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DreamCycleGuard")
            .field("state", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl DreamCycleGuard {
    #[must_use]
    pub fn state(&self) -> &DreamCycleState {
        &self.state
    }
}

impl Drop for DreamCycleGuard {
    fn drop(&mut self) {
        remove_state_if_owned(&self.paths.state_json, &self.state);
        let _ = FileExt::unlock(&self.file);
    }
}

pub struct DreamCycleAlreadyRunning {
    pub state: Option<DreamCycleState>,
    failure: Option<io::Error>,
}

impl DreamCycleAlreadyRunning {
    #[must_use]
    pub fn is_already_running(&self) -> bool {
        self.failure.is_none()
    }

    fn contention(state: Option<DreamCycleState>) -> Self {
        Self {
            state,
            failure: None,
        }
    }

    fn io(source: io::Error) -> Self {
        Self {
            state: None,
            failure: Some(source),
        }
    }
}

impl fmt::Debug for DreamCycleAlreadyRunning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DreamCycleAlreadyRunning")
            .field("state", &self.state.as_ref().map(|_| "[REDACTED]"))
            .field(
                "kind",
                &self.failure.as_ref().map_or("contention", |_| "io"),
            )
            .finish()
    }
}

impl fmt::Display for DreamCycleAlreadyRunning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.failure.is_some() {
            formatter.write_str("failed to acquire dream-cycle lock")
        } else {
            formatter.write_str("dream cycle already running")
        }
    }
}

impl std::error::Error for DreamCycleAlreadyRunning {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.failure
            .as_ref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

pub fn acquire_dream_cycle_lock(
    config: &HieronymusConfig,
    owner: &str,
    wait: bool,
) -> Result<DreamCycleGuard, DreamCycleAlreadyRunning> {
    let owner = validated_owner(owner).map_err(DreamCycleAlreadyRunning::io)?;
    let paths = dream_cycle_paths(config);
    prepare_data_root(&config.data_root).map_err(DreamCycleAlreadyRunning::io)?;
    let file = open_lock_file(&paths.lock_file).map_err(DreamCycleAlreadyRunning::io)?;

    let result = if wait {
        FileExt::lock_exclusive(&file)
    } else {
        FileExt::try_lock_exclusive(&file)
    };
    if let Err(source) = result {
        if source.kind() == io::ErrorKind::WouldBlock {
            return Err(DreamCycleAlreadyRunning::contention(read_state(
                &paths.state_json,
            )));
        }
        return Err(DreamCycleAlreadyRunning::io(source));
    }

    if let Err(source) = validate_lock_file(&paths.lock_file, &file) {
        let _ = FileExt::unlock(&file);
        return Err(DreamCycleAlreadyRunning::io(source));
    }

    let state = DreamCycleState {
        owner,
        pid: std::process::id(),
        started_at: Utc::now(),
        token: Uuid::new_v4(),
    };
    if let Err(source) = write_state(&paths.state_json, &state) {
        let _ = FileExt::unlock(&file);
        return Err(DreamCycleAlreadyRunning::io(source));
    }

    Ok(DreamCycleGuard { file, paths, state })
}

fn validated_owner(owner: &str) -> io::Result<String> {
    let owner = owner.trim();
    if owner.is_empty() || owner.len() > 64 || owner.chars().any(|character| character.is_control())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "dream-cycle owner must contain 1 to 64 printable bytes",
        ));
    }
    Ok(owner.to_owned())
}

fn prepare_data_root(root: &Path) -> io::Result<()> {
    if !root.exists() {
        fs::create_dir_all(root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        }
    }
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "dream-cycle data root must be a real directory",
        ));
    }
    Ok(())
}

fn open_lock_file(path: &Path) -> io::Result<File> {
    let mut create = OpenOptions::new();
    create.read(true).write(true).create_new(true);
    configure_secure_open(&mut create, true);
    let file = match create.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            validate_regular_path(path)?;
            let mut existing = OpenOptions::new();
            existing.read(true).write(true);
            configure_secure_open(&mut existing, false);
            existing.open(path)?
        }
        Err(error) => return Err(error),
    };
    validate_lock_file(path, &file)?;
    Ok(file)
}

#[cfg(unix)]
fn configure_secure_open(options: &mut OpenOptions, create: bool) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    if create {
        options.mode(0o600);
    }
}

#[cfg(windows)]
fn configure_secure_open(options: &mut OpenOptions, _create: bool) {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

fn validate_regular_path(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if !metadata.file_type().is_file()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "path is not a regular non-reparse file",
            ));
        }
    }
    #[cfg(not(windows))]
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "path is not a regular file",
        ));
    }
    Ok(())
}

fn validate_lock_file(path: &Path, file: &File) -> io::Result<()> {
    validate_regular_path(path)?;
    let handle = file.metadata()?;
    if !handle.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "dream-cycle lock handle is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let observed = fs::metadata(path)?;
        let parent = fs::metadata(path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "lock path has no parent")
        })?)?;
        if handle.dev() != observed.dev()
            || handle.ino() != observed.ino()
            || handle.nlink() != 1
            || handle.uid() != parent.uid()
            || handle.permissions().mode() & 0o777 != 0o600
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "dream-cycle lock must be a private 0600 file with one link and the directory owner",
            ));
        }
    }
    Ok(())
}

fn read_state(path: &Path) -> Option<DreamCycleState> {
    validate_regular_path(path).ok()?;
    let mut options = OpenOptions::new();
    options.read(true);
    configure_secure_open(&mut options, false);
    let file = options.open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_STATE_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return None;
    }
    let state: DreamCycleState = serde_json::from_slice(&bytes).ok()?;
    let owner = validated_owner(&state.owner).ok()?;
    if owner != state.owner || state.pid == 0 || state.token.is_nil() {
        return None;
    }
    Some(state)
}

fn write_state(path: &Path, state: &DreamCycleState) -> io::Result<()> {
    if path.exists() {
        validate_regular_path(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if fs::metadata(path)?.nlink() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "dream-cycle state must have one link",
                ));
            }
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "state path has no parent"))?;
    let temporary = parent.join(format!(".dream-cycle-state-{}.tmp", Uuid::new_v4()));
    let bytes = serde_json::to_vec(state).map_err(io::Error::other)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_secure_open(&mut options, true);
    let mut file = options.open(&temporary)?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        replace_state_file(&temporary, path)?;
        sync_parent(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn replace_state_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    tempfile::TempPath::try_from_path(temporary.to_owned())?
        .persist(destination)
        .map_err(|error| error.error)
}

#[cfg(not(windows))]
fn replace_state_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

fn remove_state_if_owned(path: &Path, expected: &DreamCycleState) {
    let Some(current) = read_state(path) else {
        return;
    };
    if current.token == expected.token && current.owner == expected.owner {
        let _ = fs::remove_file(path);
    }
}
