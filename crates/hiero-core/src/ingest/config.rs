use std::{
    io::Read,
    path::{Component, Path},
};

#[cfg(windows)]
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::ShortMemoryLimits;

use super::IngestError;

type Result<T> = std::result::Result<T, IngestError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LearnLimits {
    pub max_block_chars: usize,
}

impl Default for LearnLimits {
    fn default() -> Self {
        Self {
            max_block_chars: 1_200,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct IngestConfig {
    pub short_memory: ShortMemoryLimits,
    pub learn: LearnLimits,
}

impl IngestConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        load_anchored(path.as_ref())
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let config = self.validate()?;
        save_anchored(&config, path.as_ref())
    }

    pub fn validate(&self) -> Result<Self> {
        validate_minimum(
            "short_memory.warning_sentence_count",
            self.short_memory.warning_sentence_count,
            1,
        )?;
        validate_minimum(
            "short_memory.rejection_sentence_count",
            self.short_memory.rejection_sentence_count,
            1,
        )?;
        validate_minimum(
            "short_memory.warning_symbol_count",
            self.short_memory.warning_symbol_count,
            0,
        )?;
        validate_minimum(
            "short_memory.rejection_symbol_count",
            self.short_memory.rejection_symbol_count,
            0,
        )?;
        validate_minimum("learn.max_block_chars", self.learn.max_block_chars, 1)?;
        if self.short_memory.rejection_sentence_count < self.short_memory.warning_sentence_count {
            return Err(IngestError::Validation(
                "short_memory.rejection_sentence_count must be greater than or equal to short_memory.warning_sentence_count".into(),
            ));
        }
        if self.short_memory.warning_symbol_count != 0
            && self.short_memory.rejection_symbol_count != 0
            && self.short_memory.rejection_symbol_count < self.short_memory.warning_symbol_count
        {
            return Err(IngestError::Validation(
                "short_memory.rejection_symbol_count must be greater than or equal to short_memory.warning_symbol_count".into(),
            ));
        }
        Ok(*self)
    }
}

fn parse_config(contents: &str) -> Result<IngestConfig> {
    let value = contents
        .parse::<toml::Table>()
        .map_err(IngestError::InvalidToml)?;
    reject_unknown(&value, &["short_memory", "learn"], None)?;
    let defaults = IngestConfig::default();
    let short = optional_table(&value, "short_memory")?;
    reject_unknown(
        short,
        &[
            "warning_sentence_count",
            "rejection_sentence_count",
            "warning_symbol_count",
            "rejection_symbol_count",
        ],
        Some("short_memory"),
    )?;
    let learn = optional_table(&value, "learn")?;
    reject_unknown(learn, &["max_block_chars"], Some("learn"))?;
    Ok(IngestConfig {
        short_memory: ShortMemoryLimits {
            warning_sentence_count: integer_or(
                short,
                "warning_sentence_count",
                defaults.short_memory.warning_sentence_count,
                "short_memory.warning_sentence_count",
                1,
            )?,
            rejection_sentence_count: integer_or(
                short,
                "rejection_sentence_count",
                defaults.short_memory.rejection_sentence_count,
                "short_memory.rejection_sentence_count",
                1,
            )?,
            warning_symbol_count: integer_or(
                short,
                "warning_symbol_count",
                defaults.short_memory.warning_symbol_count,
                "short_memory.warning_symbol_count",
                0,
            )?,
            rejection_symbol_count: integer_or(
                short,
                "rejection_symbol_count",
                defaults.short_memory.rejection_symbol_count,
                "short_memory.rejection_symbol_count",
                0,
            )?,
        },
        learn: LearnLimits {
            max_block_chars: integer_or(
                learn,
                "max_block_chars",
                defaults.learn.max_block_chars,
                "learn.max_block_chars",
                1,
            )?,
        },
    })
}

fn optional_table<'a>(root: &'a toml::Table, key: &str) -> Result<&'a toml::Table> {
    match root.get(key) {
        None => Ok(empty_table()),
        Some(toml::Value::Table(table)) => Ok(table),
        Some(_) => Err(IngestError::Validation(format!("{key} must be a table"))),
    }
}

fn empty_table() -> &'static toml::Table {
    static EMPTY: std::sync::OnceLock<toml::Table> = std::sync::OnceLock::new();
    EMPTY.get_or_init(toml::Table::new)
}

fn integer_or(
    table: &toml::Table,
    key: &str,
    default: usize,
    field: &str,
    minimum: usize,
) -> Result<usize> {
    let Some(value) = table.get(key) else {
        return Ok(default);
    };
    let Some(value) = value.as_integer() else {
        return Err(IngestError::Validation(format!(
            "{field} must be an integer"
        )));
    };
    usize::try_from(value)
        .map_err(|_| IngestError::Validation(format!("{field} must be at least {minimum}")))
}

fn reject_unknown(table: &toml::Table, allowed: &[&str], prefix: Option<&str>) -> Result<()> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            let setting = prefix.map_or_else(|| key.clone(), |prefix| format!("{prefix}.{key}"));
            return Err(IngestError::Validation(format!(
                "unknown ingest config setting: {setting}"
            )));
        }
    }
    Ok(())
}

fn validate_minimum(field: &str, value: usize, minimum: usize) -> Result<()> {
    if value < minimum {
        Err(IngestError::Validation(format!(
            "{field} must be at least {minimum}"
        )))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn load_anchored(path: &Path) -> Result<IngestConfig> {
    let anchor = match UnixAnchor::open(path, false) {
        Ok(anchor) => anchor,
        Err(error) if is_missing_error(&error) => return IngestConfig::default().validate(),
        Err(error) => return Err(error),
    };
    let descriptor = match rustix::fs::openat(
        anchor.parent(),
        &anchor.file_name,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(source) if source == rustix::io::Errno::NOENT => {
            return IngestConfig::default().validate();
        }
        Err(source) => return Err(unix_open_error(path, source)),
    };
    let mut file = std::fs::File::from(descriptor);
    if !file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .is_file()
    {
        return Err(unsafe_path(path, "configuration is not a regular file"));
    }
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|source| io_error(path, source))?;
    parse_config(&contents)?.validate()
}

#[cfg(unix)]
fn save_anchored(config: &IngestConfig, path: &Path) -> Result<()> {
    save_unix_with_hook(config, path, || {})
}

#[cfg(unix)]
fn save_unix_with_hook(config: &IngestConfig, path: &Path, hook: impl FnOnce()) -> Result<()> {
    use std::io::Write;

    let anchor = UnixAnchor::open(path, true)?;
    hook();
    let contents = toml::to_string_pretty(config).map_err(IngestError::Serialize)?;
    let temporary_name = format!(".ingest-{}.tmp", uuid::Uuid::new_v4());
    let descriptor = rustix::fs::openat(
        anchor.parent(),
        &temporary_name,
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::EXCL
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .map_err(|source| unix_open_error(path, source))?;
    let mut temporary = std::fs::File::from(descriptor);
    let result = (|| {
        temporary
            .write_all(contents.as_bytes())
            .and_then(|()| temporary.sync_all())
            .map_err(|source| io_error(path, source))?;
        reject_unix_destination_symlink(&anchor, path)?;
        rustix::fs::renameat(
            anchor.parent(),
            &temporary_name,
            anchor.parent(),
            &anchor.file_name,
        )
        .map_err(|source| io_error(path, rustix_error(source)))?;
        rustix::fs::fsync(anchor.parent())
            .map_err(|source| io_error(path, rustix_error(source)))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = rustix::fs::unlinkat(
            anchor.parent(),
            &temporary_name,
            rustix::fs::AtFlags::empty(),
        );
    }
    result
}

#[cfg(unix)]
struct UnixAnchor {
    handles: Vec<std::os::fd::OwnedFd>,
    file_name: std::ffi::OsString,
}

#[cfg(unix)]
impl UnixAnchor {
    fn open(path: &Path, create: bool) -> Result<Self> {
        let file_name = path
            .file_name()
            .ok_or_else(|| unsafe_path(path, "configuration path has no filename"))?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let start = if path.is_absolute() { "/" } else { "." };
        let descriptor = rustix::fs::open(
            start,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|source| unix_open_error(path, source))?;
        let mut handles = vec![descriptor];
        for component in parent.components() {
            let Component::Normal(component) = component else {
                if matches!(component, Component::RootDir | Component::CurDir) {
                    continue;
                }
                return Err(unsafe_path(
                    path,
                    "configuration path may not contain parent traversal",
                ));
            };
            let parent_fd = handles.last().expect("anchor always has a root handle");
            let flags = rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC;
            let child =
                match rustix::fs::openat(parent_fd, component, flags, rustix::fs::Mode::empty()) {
                    Ok(child) => child,
                    Err(source) if create && source == rustix::io::Errno::NOENT => {
                        match rustix::fs::mkdirat(
                            parent_fd,
                            component,
                            rustix::fs::Mode::RUSR
                                | rustix::fs::Mode::WUSR
                                | rustix::fs::Mode::XUSR,
                        ) {
                            Ok(()) => {}
                            Err(source) if source == rustix::io::Errno::EXIST => {}
                            Err(source) => return Err(unix_open_error(path, source)),
                        }
                        rustix::fs::openat(parent_fd, component, flags, rustix::fs::Mode::empty())
                            .map_err(|source| unix_open_error(path, source))?
                    }
                    Err(source) => return Err(unix_open_error(path, source)),
                };
            handles.push(child);
        }
        Ok(Self {
            handles,
            file_name: file_name.to_owned(),
        })
    }

    fn parent(&self) -> &std::os::fd::OwnedFd {
        self.handles
            .last()
            .expect("anchor always has a root handle")
    }
}

#[cfg(unix)]
fn reject_unix_destination_symlink(anchor: &UnixAnchor, path: &Path) -> Result<()> {
    match rustix::fs::statat(
        anchor.parent(),
        &anchor.file_name,
        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Ok(metadata) => {
            if rustix::fs::FileType::from_raw_mode(metadata.st_mode).is_symlink() {
                return Err(unsafe_path(path, "configuration path is a symbolic link"));
            }
            Ok(())
        }
        Err(source) if source == rustix::io::Errno::NOENT => Ok(()),
        Err(source) => Err(io_error(path, rustix_error(source))),
    }
}

#[cfg(unix)]
fn unix_open_error(path: &Path, source: rustix::io::Errno) -> IngestError {
    if matches!(source, rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) {
        unsafe_path(
            path,
            "configuration path contains a symbolic link or non-directory",
        )
    } else {
        io_error(path, rustix_error(source))
    }
}

#[cfg(unix)]
fn rustix_error(source: rustix::io::Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(source.raw_os_error())
}

#[cfg(windows)]
const WINDOWS_SHARE_WITHOUT_DELETE: u32 = 0x0000_0001 | 0x0000_0002;
#[cfg(windows)]
const WINDOWS_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const WINDOWS_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const WINDOWS_REPARSE_ATTRIBUTE: u32 = 0x0000_0400;

#[cfg(windows)]
fn load_anchored(path: &Path) -> Result<IngestConfig> {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};

    let anchor = match WindowsAnchor::open(path, false) {
        Ok(anchor) => anchor,
        Err(error) if is_missing_error(&error) => return IngestConfig::default().validate(),
        Err(error) => return Err(error),
    };
    let mut options = OpenOptions::new();
    options
        .read(true)
        .share_mode(WINDOWS_SHARE_WITHOUT_DELETE)
        .custom_flags(WINDOWS_OPEN_REPARSE_POINT);
    let mut file = match options.open(&anchor.final_path) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return IngestConfig::default().validate();
        }
        Err(source) => return Err(io_error(path, source)),
    };
    reject_windows_reparse(
        path,
        &file.metadata().map_err(|source| io_error(path, source))?,
    )?;
    if !file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .is_file()
    {
        return Err(unsafe_path(path, "configuration is not a regular file"));
    }
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|source| io_error(path, source))?;
    parse_config(&contents)?.validate()
}

#[cfg(windows)]
fn save_anchored(config: &IngestConfig, path: &Path) -> Result<()> {
    use std::{fs::OpenOptions, io::Write, os::windows::fs::OpenOptionsExt};

    let anchor = WindowsAnchor::open(path, true)?;
    match OpenOptions::new()
        .read(true)
        .share_mode(WINDOWS_SHARE_WITHOUT_DELETE)
        .custom_flags(WINDOWS_OPEN_REPARSE_POINT)
        .open(&anchor.final_path)
    {
        Ok(existing) => reject_windows_reparse(
            path,
            &existing
                .metadata()
                .map_err(|source| io_error(path, source))?,
        )?,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error(path, source)),
    }
    let temporary_path = anchor
        .parent
        .join(format!(".ingest-{}.tmp", uuid::Uuid::new_v4()));
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(WINDOWS_SHARE_WITHOUT_DELETE)
        .custom_flags(WINDOWS_OPEN_REPARSE_POINT)
        .open(&temporary_path)
        .map_err(|source| io_error(path, source))?;
    let result = (|| {
        let contents = toml::to_string_pretty(config).map_err(IngestError::Serialize)?;
        temporary
            .write_all(contents.as_bytes())
            .and_then(|()| temporary.sync_all())
            .map_err(|source| io_error(path, source))?;
        drop(temporary);
        move_file_replace(&temporary_path, &anchor.final_path)
            .map_err(|source| io_error(path, source))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary_path);
    }
    result
}

#[cfg(windows)]
struct WindowsAnchor {
    _handles: Vec<std::fs::File>,
    parent: PathBuf,
    final_path: PathBuf,
}

#[cfg(windows)]
impl WindowsAnchor {
    fn open(path: &Path, create: bool) -> Result<Self> {
        let file_name = path
            .file_name()
            .ok_or_else(|| unsafe_path(path, "configuration path has no filename"))?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut current = PathBuf::new();
        let mut handles = Vec::new();
        if !path.is_absolute() {
            handles.push(open_windows_directory(path, Path::new("."))?);
        }
        for component in parent.components() {
            if matches!(component, Component::ParentDir) {
                return Err(unsafe_path(
                    path,
                    "configuration path may not contain parent traversal",
                ));
            }
            current.push(component.as_os_str());
            if matches!(component, Component::Prefix(_)) {
                continue;
            }
            if create && !current.exists() {
                std::fs::create_dir(&current).map_err(|source| io_error(path, source))?;
            }
            handles.push(open_windows_directory(path, &current)?);
        }
        if handles.is_empty() {
            handles.push(open_windows_directory(path, Path::new("."))?);
        }
        Ok(Self {
            _handles: handles,
            parent: parent.to_path_buf(),
            final_path: parent.join(file_name),
        })
    }
}

#[cfg(windows)]
fn open_windows_directory(path: &Path, directory_path: &Path) -> Result<std::fs::File> {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};

    let directory = OpenOptions::new()
        .read(true)
        .share_mode(WINDOWS_SHARE_WITHOUT_DELETE)
        .custom_flags(WINDOWS_OPEN_REPARSE_POINT | WINDOWS_BACKUP_SEMANTICS)
        .open(directory_path)
        .map_err(|source| io_error(path, source))?;
    let metadata = directory
        .metadata()
        .map_err(|source| io_error(path, source))?;
    reject_windows_reparse(path, &metadata)?;
    if !metadata.is_dir() {
        return Err(unsafe_path(
            path,
            "configuration ancestor is not a directory",
        ));
    }
    Ok(directory)
}

#[cfg(windows)]
fn reject_windows_reparse(path: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    if is_windows_reparse(metadata.file_attributes()) {
        Err(unsafe_path(
            path,
            "configuration path contains a reparse point",
        ))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
const fn is_windows_reparse(attributes: u32) -> bool {
    attributes & WINDOWS_REPARSE_ATTRIBUTE != 0
}

#[cfg(windows)]
fn move_file_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    tempfile::TempPath::try_from_path(source.to_path_buf())?
        .persist(destination)
        .map_err(|error| error.error)
}

#[cfg(not(any(unix, windows)))]
fn load_anchored(path: &Path) -> Result<IngestConfig> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return IngestConfig::default().validate();
        }
        Err(source) => return Err(io_error(path, source)),
    };
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|source| io_error(path, source))?;
    parse_config(&contents)?.validate()
}

#[cfg(not(any(unix, windows)))]
fn save_anchored(config: &IngestConfig, path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| unsafe_path(path, "configuration path has no parent directory"))?;
    std::fs::create_dir_all(parent).map_err(|source| io_error(path, source))?;
    let contents = toml::to_string_pretty(config).map_err(IngestError::Serialize)?;
    std::fs::write(path, contents).map_err(|source| io_error(path, source))
}

fn unsafe_path(path: &Path, reason: &'static str) -> IngestError {
    IngestError::UnsafePath {
        path: path.to_path_buf(),
        reason,
    }
}

fn io_error(path: &Path, source: std::io::Error) -> IngestError {
    IngestError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn is_missing_error(error: &IngestError) -> bool {
    matches!(
        error,
        IngestError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound
    )
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::symlink;

    use super::{IngestConfig, save_unix_with_hook};

    #[test]
    fn anchored_save_cannot_be_redirected_by_ancestor_swap() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("config");
        let displaced = root.path().join("anchored-config");
        let outside = root.path().join("outside");
        std::fs::create_dir(&parent).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("ingest.conf"), "outside remains untouched").unwrap();
        let path = parent.join("ingest.conf");

        save_unix_with_hook(&IngestConfig::default(), &path, || {
            std::fs::rename(&parent, &displaced).unwrap();
            symlink(&outside, &parent).unwrap();
        })
        .unwrap();

        assert!(
            std::fs::read_to_string(displaced.join("ingest.conf"))
                .unwrap()
                .contains("max_block_chars = 1200")
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("ingest.conf")).unwrap(),
            "outside remains untouched"
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::{
        WINDOWS_REPARSE_ATTRIBUTE, WINDOWS_SHARE_WITHOUT_DELETE, is_windows_reparse,
        move_file_replace,
    };

    #[test]
    fn held_handle_policy_blocks_delete_and_detects_arbitrary_reparse_attributes() {
        const FILE_SHARE_DELETE: u32 = 0x0000_0004;
        assert_eq!(WINDOWS_SHARE_WITHOUT_DELETE & FILE_SHARE_DELETE, 0);
        assert!(is_windows_reparse(WINDOWS_REPARSE_ATTRIBUTE));
        assert!(is_windows_reparse(WINDOWS_REPARSE_ATTRIBUTE | 0x20));
        assert!(!is_windows_reparse(0x20));
        let _: fn(&std::path::Path, &std::path::Path) -> std::io::Result<()> = move_file_replace;
    }
}
