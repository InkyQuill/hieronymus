use std::{
    fmt,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use fs4::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    config::HieronymusConfig,
    file_identity::{identity_and_link_count, identity_for_handle},
};

const LOCK_NAME: &str = "dream-cycle.lock";
const STATE_NAME: &str = "dream-cycle.json";
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

#[must_use]
pub fn read_dream_cycle_state(config: &HieronymusConfig) -> Option<DreamCycleState> {
    let directory = SecureDataRoot::open(config.data_root.as_path()).ok()?;
    read_state(&directory, STATE_NAME)
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
    directory: SecureDataRoot,
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

    pub(crate) fn mark_audit_recovery(&mut self) -> io::Result<()> {
        let mut recovery_state = self.state.clone();
        recovery_state.owner = "audit-recovery".into();
        self.directory.write_state(&recovery_state)?;
        self.state = recovery_state;
        Ok(())
    }
}

impl Drop for DreamCycleGuard {
    fn drop(&mut self) {
        remove_state_if_owned(&self.directory, &self.state);
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
    let directory =
        SecureDataRoot::open(&config.data_root).map_err(DreamCycleAlreadyRunning::io)?;
    let file = directory
        .open_lock_file()
        .map_err(DreamCycleAlreadyRunning::io)?;

    let result = if wait {
        FileExt::lock_exclusive(&file)
    } else {
        FileExt::try_lock_exclusive(&file)
    };
    if let Err(source) = result {
        if source.kind() == io::ErrorKind::WouldBlock {
            return Err(DreamCycleAlreadyRunning::contention(read_state(
                &directory, STATE_NAME,
            )));
        }
        return Err(DreamCycleAlreadyRunning::io(source));
    }

    if let Err(source) = directory.validate_lock_identity(&file) {
        let _ = FileExt::unlock(&file);
        return Err(DreamCycleAlreadyRunning::io(source));
    }
    let state = DreamCycleState {
        owner,
        pid: std::process::id(),
        started_at: Utc::now(),
        token: Uuid::new_v4(),
    };
    if let Err(source) = directory.write_state(&state) {
        let _ = FileExt::unlock(&file);
        return Err(DreamCycleAlreadyRunning::io(source));
    }
    Ok(DreamCycleGuard {
        file,
        directory,
        state,
    })
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

fn read_state(directory: &SecureDataRoot, name: &str) -> Option<DreamCycleState> {
    let file = directory.open_state_file(name).ok()??;
    read_state_handle(file)
}

fn read_state_handle(file: File) -> Option<DreamCycleState> {
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

fn state_is_owned(state: &DreamCycleState, expected: &DreamCycleState) -> bool {
    state.token == expected.token && state.owner == expected.owner
}

fn remove_state_if_owned(directory: &SecureDataRoot, expected: &DreamCycleState) {
    remove_state_if_owned_with_hook(directory, expected, || {});
}

fn remove_state_if_owned_with_hook(
    directory: &SecureDataRoot,
    expected: &DreamCycleState,
    hook: impl FnOnce(),
) {
    let Ok(Some(opened)) = directory.open_state_file(STATE_NAME) else {
        return;
    };
    let Ok(opened_identity) = identity_for_handle(&opened) else {
        return;
    };
    let Some(opened_state) = read_state_handle(opened) else {
        return;
    };
    if !state_is_owned(&opened_state, expected) {
        return;
    }

    hook();
    let tombstone = format!(".dream-cycle-state-{}.removed", Uuid::new_v4());
    if directory.rename_state(STATE_NAME, &tombstone).is_err() {
        return;
    }
    let owned_tombstone = directory
        .open_state_file(&tombstone)
        .ok()
        .flatten()
        .and_then(|file| {
            let identity = identity_for_handle(&file).ok()?;
            let state = read_state_handle(file)?;
            Some(identity == opened_identity && state_is_owned(&state, expected))
        })
        .unwrap_or(false);
    if owned_tombstone {
        let _ = directory.remove_state(&tombstone);
    } else {
        // A no-clobber hard-link restore preserves a concurrently installed fixed-path state.
        // If that path is occupied, the replacement remains recoverable at the unique tombstone.
        let _ = directory.restore_state_no_clobber(&tombstone, STATE_NAME);
    }
    let _ = directory.sync();
}

#[cfg(unix)]
struct SecureDataRoot {
    handles: Vec<std::os::fd::OwnedFd>,
}

#[cfg(unix)]
impl SecureDataRoot {
    fn open(path: &Path) -> io::Result<Self> {
        use std::path::Component;

        let start = if path.is_absolute() { "/" } else { "." };
        let root = rustix::fs::open(
            start,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(errno)?;
        let mut handles = vec![root];
        for component in path.components() {
            let Component::Normal(component) = component else {
                if matches!(component, Component::RootDir | Component::CurDir) {
                    continue;
                }
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "dream-cycle data root may not contain parent traversal",
                ));
            };
            let current = handles
                .last()
                .expect("anchor starts with a root descriptor");
            let flags = rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC;
            let child =
                match rustix::fs::openat(current, component, flags, rustix::fs::Mode::empty()) {
                    Ok(child) => child,
                    Err(source) if source == rustix::io::Errno::NOENT => {
                        match rustix::fs::mkdirat(
                            current,
                            component,
                            rustix::fs::Mode::RUSR
                                | rustix::fs::Mode::WUSR
                                | rustix::fs::Mode::XUSR,
                        ) {
                            Ok(()) => {}
                            Err(source) if source == rustix::io::Errno::EXIST => {}
                            Err(source) => return Err(errno(source)),
                        }
                        rustix::fs::openat(current, component, flags, rustix::fs::Mode::empty())
                            .map_err(errno)?
                    }
                    Err(source) => return Err(errno(source)),
                };
            handles.push(child);
        }
        Ok(Self { handles })
    }

    fn parent(&self) -> &std::os::fd::OwnedFd {
        self.handles.last().expect("anchor has a data-root handle")
    }

    fn open_lock_file(&self) -> io::Result<File> {
        let descriptor = rustix::fs::openat(
            self.parent(),
            LOCK_NAME,
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .map_err(errno)?;
        let file = File::from(descriptor);
        self.validate_private_file(&file)?;
        Ok(file)
    }

    fn validate_lock_identity(&self, locked: &File) -> io::Result<()> {
        let observed = self.open_existing(LOCK_NAME, true)?;
        let (_, locked_identity) = identity_and_link_count(locked)?;
        let (_, observed_identity) = identity_and_link_count(&observed)?;
        if locked_identity == observed_identity {
            self.validate_private_file(locked)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "dream-cycle lock identity changed during acquisition",
            ))
        }
    }

    fn validate_private_file(&self, file: &File) -> io::Result<()> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let (links, _) = identity_and_link_count(file)?;
        let metadata = file.metadata()?;
        let parent = rustix::fs::fstat(self.parent()).map_err(errno)?;
        if links == 1
            && metadata.permissions().mode() & 0o777 == 0o600
            && metadata.uid() == parent.st_uid
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "dream-cycle file must be private 0600, single-link, and owned by the data root owner",
            ))
        }
    }

    fn open_existing(&self, name: &str, write: bool) -> io::Result<File> {
        let access = if write {
            rustix::fs::OFlags::RDWR
        } else {
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK
        };
        let descriptor = rustix::fs::openat(
            self.parent(),
            name,
            access | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(errno)?;
        Ok(File::from(descriptor))
    }

    fn open_state_file(&self, name: &str) -> io::Result<Option<File>> {
        match self.open_existing(name, false) {
            Ok(file) => {
                self.validate_private_file(&file)?;
                Ok(Some(file))
            }
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::NOENT.raw_os_error()) => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn write_state(&self, state: &DreamCycleState) -> io::Result<()> {
        match self.open_existing(STATE_NAME, false) {
            Ok(existing) => {
                let (links, _) = identity_and_link_count(&existing)?;
                if links != 1 {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "dream-cycle state must have one link",
                    ));
                }
            }
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::NOENT.raw_os_error()) => {
            }
            Err(error) => return Err(error),
        }
        let temporary = format!(".dream-cycle-state-{}.tmp", Uuid::new_v4());
        let descriptor = rustix::fs::openat(
            self.parent(),
            &temporary,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .map_err(errno)?;
        let mut file = File::from(descriptor);
        let result = (|| {
            file.write_all(&serde_json::to_vec(state).map_err(io::Error::other)?)?;
            file.sync_all()?;
            rustix::fs::renameat(self.parent(), &temporary, self.parent(), STATE_NAME)
                .map_err(errno)?;
            self.sync()
        })();
        if result.is_err() {
            let _ = rustix::fs::unlinkat(self.parent(), &temporary, rustix::fs::AtFlags::empty());
        }
        result
    }

    fn rename_state(&self, source: &str, destination: &str) -> io::Result<()> {
        rustix::fs::renameat(self.parent(), source, self.parent(), destination).map_err(errno)
    }

    fn remove_state(&self, name: &str) -> io::Result<()> {
        rustix::fs::unlinkat(self.parent(), name, rustix::fs::AtFlags::empty()).map_err(errno)
    }

    fn restore_state_no_clobber(&self, tombstone: &str, destination: &str) -> io::Result<()> {
        rustix::fs::linkat(
            self.parent(),
            tombstone,
            self.parent(),
            destination,
            rustix::fs::AtFlags::empty(),
        )
        .map_err(errno)?;
        self.remove_state(tombstone)
    }

    fn sync(&self) -> io::Result<()> {
        rustix::fs::fsync(self.parent()).map_err(errno)
    }
}

#[cfg(unix)]
fn errno(source: rustix::io::Errno) -> io::Error {
    io::Error::from_raw_os_error(source.raw_os_error())
}

#[cfg(windows)]
struct SecureDataRoot {
    _handles: Vec<File>,
    root: PathBuf,
}

#[cfg(windows)]
impl SecureDataRoot {
    fn open(path: &Path) -> io::Result<Self> {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt, path::Component};

        const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const SHARE_WITHOUT_DELETE: u32 = 0x0000_0001 | 0x0000_0002;
        let candidate = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut current = PathBuf::new();
        let mut handles = Vec::new();
        for component in candidate.components() {
            if matches!(component, Component::ParentDir) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "dream-cycle data root may not contain parent traversal",
                ));
            }
            current.push(component.as_os_str());
            if matches!(component, Component::Prefix(_)) {
                continue;
            }
            if !current.exists() {
                std::fs::create_dir(&current)?;
            }
            let handle = OpenOptions::new()
                .read(true)
                .share_mode(SHARE_WITHOUT_DELETE)
                .custom_flags(OPEN_REPARSE_POINT | BACKUP_SEMANTICS)
                .open(&current)?;
            validate_windows_directory(&handle)?;
            handles.push(handle);
        }
        Ok(Self {
            _handles: handles,
            root: candidate,
        })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn open_named(&self, name: &str, write: bool, create: bool) -> io::Result<File> {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
        const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const SHARE_WITHOUT_DELETE: u32 = 0x0000_0001 | 0x0000_0002;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(write)
            .create(create)
            .share_mode(SHARE_WITHOUT_DELETE)
            .custom_flags(OPEN_REPARSE_POINT);
        options.open(self.path(name))
    }

    fn open_lock_file(&self) -> io::Result<File> {
        let file = self.open_named(LOCK_NAME, true, true)?;
        self.validate_single_link(&file)?;
        Ok(file)
    }

    fn validate_lock_identity(&self, locked: &File) -> io::Result<()> {
        let observed = self.open_named(LOCK_NAME, true, false)?;
        let (_, locked_identity) = identity_and_link_count(locked)?;
        let (_, observed_identity) = identity_and_link_count(&observed)?;
        if locked_identity == observed_identity {
            self.validate_single_link(locked)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "dream-cycle lock identity changed during acquisition",
            ))
        }
    }

    fn validate_single_link(&self, file: &File) -> io::Result<()> {
        let (links, _) = identity_and_link_count(file)?;
        if links == 1 {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "dream-cycle file must have one link",
            ))
        }
    }

    fn open_state_file(&self, name: &str) -> io::Result<Option<File>> {
        match self.open_named(name, false, false) {
            Ok(file) => {
                self.validate_single_link(&file)?;
                Ok(Some(file))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write_state(&self, state: &DreamCycleState) -> io::Result<()> {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
        const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const SHARE_WITHOUT_DELETE: u32 = 0x0000_0001 | 0x0000_0002;
        match self.open_named(STATE_NAME, false, false) {
            Ok(existing) => self.validate_single_link(&existing)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let temporary = self.path(&format!(".dream-cycle-state-{}.tmp", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(SHARE_WITHOUT_DELETE)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(&serde_json::to_vec(state).map_err(io::Error::other)?)?;
            file.sync_all()?;
            drop(file);
            tempfile::TempPath::try_from_path(temporary.clone())?
                .persist(self.path(STATE_NAME))
                .map_err(|error| error.error)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }

    fn rename_state(&self, source: &str, destination: &str) -> io::Result<()> {
        std::fs::rename(self.path(source), self.path(destination))
    }

    fn remove_state(&self, name: &str) -> io::Result<()> {
        std::fs::remove_file(self.path(name))
    }

    fn restore_state_no_clobber(&self, tombstone: &str, destination: &str) -> io::Result<()> {
        std::fs::hard_link(self.path(tombstone), self.path(destination))?;
        self.remove_state(tombstone)
    }

    fn sync(&self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
fn validate_windows_directory(file: &File) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x0000_0400;
    let metadata = file.metadata()?;
    if metadata.is_dir() && metadata.file_attributes() & REPARSE_POINT == 0 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "dream-cycle path contains a reparse or non-directory component",
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{DreamCycleState, acquire_dream_cycle_lock, dream_cycle_paths};
    use crate::config::HieronymusConfig;

    #[test]
    fn cleanup_swap_hook_restores_replacement_state_instead_of_deleting_it() {
        let root = TempDir::new().unwrap();
        let config = HieronymusConfig::load(Some(root.path().to_owned())).unwrap();
        let guard = acquire_dream_cycle_lock(&config, "owner", false).unwrap();
        let replacement = DreamCycleState {
            owner: "replacement".to_owned(),
            pid: std::process::id(),
            started_at: chrono::Utc::now(),
            token: Uuid::new_v4(),
        };
        let paths = dream_cycle_paths(&config);

        super::remove_state_if_owned_with_hook(&guard.directory, guard.state(), || {
            let temporary = root.path().join("replacement.tmp");
            fs::write(&temporary, serde_json::to_vec(&replacement).unwrap()).unwrap();
            fs::rename(temporary, &paths.state_json).unwrap();
        });

        let observed: DreamCycleState =
            serde_json::from_slice(&fs::read(&paths.state_json).unwrap()).unwrap();
        assert_eq!(observed, replacement);
    }
}
