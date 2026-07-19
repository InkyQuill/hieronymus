use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use url::Url;

use super::{ProviderError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialSource {
    #[default]
    None,
    Environment {
        variable: String,
    },
    File {
        path: PathBuf,
    },
}

impl CredentialSource {
    pub fn resolve(&self) -> Result<SecretString> {
        let value = match self {
            Self::None => {
                return Err(ProviderError::Credential(
                    "no credential source configured".into(),
                ));
            }
            Self::Environment { variable } => std::env::var(variable).map_err(|_| {
                ProviderError::Credential(format!("environment variable {variable} is unavailable"))
            })?,
            Self::File { path } => read_secret_file(path)?,
        };
        let value = value.trim().to_owned();
        if value.is_empty() {
            return Err(ProviderError::Credential(
                "credential source was empty".into(),
            ));
        }
        Ok(SecretString::from(value))
    }
}

fn read_secret_file(path: &Path) -> Result<String> {
    secure_read(path)?
        .ok_or_else(|| ProviderError::Credential("credential file is unavailable".into()))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    id: String,
    provider: String,
    model: String,
    #[serde(default)]
    credential: CredentialSource,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: f64,
}

impl std::fmt::Debug for ProviderProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderProfile")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("credential", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .field("timeout_seconds", &self.timeout_seconds)
            .finish()
    }
}

fn default_timeout_seconds() -> f64 {
    30.0
}

impl ProviderProfile {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            model: model.into(),
            credential: CredentialSource::None,
            base_url: None,
            timeout_seconds: default_timeout_seconds(),
        }
    }
    #[must_use]
    pub fn with_credential_source(mut self, source: CredentialSource) -> Self {
        self.credential = source;
        self
    }
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout_seconds = timeout.as_secs_f64();
        self
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn provider(&self) -> &str {
        &self.provider
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }
    pub fn timeout(&self) -> Duration {
        Duration::from_secs_f64(self.timeout_seconds)
    }
    pub fn credential(&self) -> &CredentialSource {
        &self.credential
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(ProviderError::Config(
                "provider id must contain only ASCII letters, digits, '_' or '-'".into(),
            ));
        }
        if !matches!(
            self.provider.as_str(),
            "openai" | "anthropic" | "google" | "ollama"
        ) {
            return Err(ProviderError::Unsupported(self.provider.clone()));
        }
        if self.model.trim().is_empty() {
            return Err(ProviderError::Config(
                "provider model must not be empty".into(),
            ));
        }
        if !self.timeout_seconds.is_finite() || self.timeout_seconds <= 0.0 {
            return Err(ProviderError::Config(
                "provider timeout must be finite and positive".into(),
            ));
        }
        if let Some(url) = &self.base_url {
            let parsed = Url::parse(url)
                .map_err(|_| ProviderError::Config("provider base URL is invalid".into()))?;
            if !parsed.username().is_empty() || parsed.password().is_some() {
                return Err(ProviderError::UnsafeEndpoint(
                    "provider base URL must not contain credentials".into(),
                ));
            }
            if parsed.query().is_some() || parsed.fragment().is_some() {
                return Err(ProviderError::UnsafeEndpoint(
                    "provider base URL must not contain a query or fragment".into(),
                ));
            }
            if self.provider == "ollama" && !is_loopback_http(&parsed) {
                return Err(ProviderError::UnsafeEndpoint(
                    "native Ollama requires an HTTP loopback URL".into(),
                ));
            }
            if self.provider != "ollama"
                && parsed.scheme() != "https"
                && !parsed.host().is_some_and(is_loopback_host)
            {
                return Err(ProviderError::UnsafeEndpoint(
                    "remote providers require HTTPS".into(),
                ));
            }
        }
        Ok(())
    }
}

fn is_loopback_http(url: &Url) -> bool {
    url.scheme() == "http" && url.host().is_some_and(is_loopback_host)
}
fn is_loopback_host(host: url::Host<&str>) -> bool {
    match host {
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
        url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderCatalog {
    providers: BTreeMap<String, ProviderProfile>,
}

impl ProviderCatalog {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = match secure_read(path)? {
            Some(contents) => contents,
            None => return Ok(Self::default()),
        };
        let catalog: Self =
            toml::from_str(&contents).map_err(|error| ProviderError::Config(error.to_string()))?;
        for (key, profile) in &catalog.providers {
            if key != profile.id() {
                return Err(ProviderError::Config(format!(
                    "provider key {key} does not match profile id"
                )));
            }
            profile.validate()?;
        }
        Ok(catalog)
    }
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        for (key, profile) in &self.providers {
            if key != profile.id() {
                return Err(ProviderError::Config(
                    "provider key does not match profile id".into(),
                ));
            }
            profile.validate()?;
        }
        let contents = toml::to_string_pretty(self)
            .map_err(|error| ProviderError::Config(error.to_string()))?;
        secure_write(path.as_ref(), contents.as_bytes())
    }
    pub fn get(&self, id: &str) -> Option<&ProviderProfile> {
        self.providers.get(id)
    }
    pub fn upsert(&mut self, profile: ProviderProfile) -> Result<Option<ProviderProfile>> {
        profile.validate()?;
        Ok(self.providers.insert(profile.id.clone(), profile))
    }
    pub fn delete(&mut self, id: &str) -> bool {
        self.providers.remove(id).is_some()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ProviderProfile)> {
        self.providers
            .iter()
            .map(|(key, value)| (key.as_str(), value))
    }
}

#[cfg(not(any(unix, windows)))]
pub(super) fn secure_read(path: &Path) -> Result<Option<String>> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ProviderError::Io(error)),
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            return Err(ProviderError::Config(
                "provider file must be a regular non-symlink file".into(),
            ));
        }
        Ok(_) => {}
    }
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(ProviderError::Io)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn secure_write(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| ProviderError::Config("provider path has no parent".into()))?;
    reject_symlink_components(parent)?;
    std::fs::create_dir_all(parent).map_err(ProviderError::Io)?;
    reject_symlink_components(parent)?;
    if path
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err(ProviderError::Config(
            "provider file must not be a symlink".into(),
        ));
    }
    let temporary = parent.join(format!(".provider-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&temporary).map_err(ProviderError::Io)?;
    let result = (|| {
        file.write_all(contents)
            .and_then(|()| file.sync_all())
            .map_err(ProviderError::Io)?;
        if path
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(ProviderError::Config(
                "provider file must not be a symlink".into(),
            ));
        }
        std::fs::rename(&temporary, path).map_err(ProviderError::Io)?;
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(ProviderError::Io)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
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
pub(super) fn secure_read(path: &Path) -> Result<Option<String>> {
    use std::{fs::OpenOptions, io::Read, os::windows::fs::OpenOptionsExt};
    let path = normalize_windows_path(path)?;
    let anchor = match WindowsAnchor::open(&path, false) {
        Ok(anchor) => anchor,
        Err(ProviderError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let mut file = match OpenOptions::new()
        .read(true)
        .share_mode(WINDOWS_SHARE_WITHOUT_DELETE)
        .custom_flags(WINDOWS_OPEN_REPARSE_POINT)
        .open(&anchor.final_path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ProviderError::Io(error)),
    };
    let metadata = file.metadata().map_err(ProviderError::Io)?;
    reject_windows_reparse(&metadata)?;
    if !metadata.is_file() {
        return Err(ProviderError::Config(
            "provider file must be a regular file".into(),
        ));
    }
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(ProviderError::Io)?;
    Ok(Some(contents))
}

#[cfg(windows)]
pub(super) fn secure_write(path: &Path, contents: &[u8]) -> Result<()> {
    use std::{fs::OpenOptions, io::Write, os::windows::fs::OpenOptionsExt};
    let path = normalize_windows_path(path)?;
    let anchor = WindowsAnchor::open(&path, true)?;
    match OpenOptions::new()
        .read(true)
        .share_mode(WINDOWS_SHARE_WITHOUT_DELETE)
        .custom_flags(WINDOWS_OPEN_REPARSE_POINT)
        .open(&anchor.final_path)
    {
        Ok(existing) => reject_windows_reparse(&existing.metadata().map_err(ProviderError::Io)?)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(ProviderError::Io(error)),
    }
    let temporary_path = anchor
        .parent
        .join(format!(".provider-{}.tmp", uuid::Uuid::new_v4()));
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(WINDOWS_SHARE_WITHOUT_DELETE)
        .custom_flags(WINDOWS_OPEN_REPARSE_POINT)
        .open(&temporary_path)
        .map_err(ProviderError::Io)?;
    let result = (|| {
        temporary
            .write_all(contents)
            .and_then(|()| temporary.sync_all())
            .map_err(ProviderError::Io)?;
        drop(temporary);
        tempfile::TempPath::try_from_path(temporary_path.clone())
            .map_err(ProviderError::Io)?
            .persist(&anchor.final_path)
            .map_err(|error| ProviderError::Io(error.error))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary_path);
    }
    result
}

#[cfg(windows)]
fn normalize_windows_path(path: &Path) -> Result<PathBuf> {
    use std::path::Component;
    let current = std::env::current_dir().map_err(ProviderError::Io)?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(ProviderError::Config(
                        "provider path escapes filesystem root".into(),
                    ));
                }
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    if !normalized.is_absolute() {
        return Err(ProviderError::Config(
            "provider path cannot be normalized safely".into(),
        ));
    }
    Ok(normalized)
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
        use std::path::Component;
        let file_name = path
            .file_name()
            .ok_or_else(|| ProviderError::Config("provider path has no filename".into()))?;
        let parent = path
            .parent()
            .ok_or_else(|| ProviderError::Config("provider path has no parent".into()))?;
        let mut current = PathBuf::new();
        let mut handles = Vec::new();
        for component in parent.components() {
            if matches!(component, Component::ParentDir) {
                return Err(ProviderError::Config(
                    "provider path may not contain parent traversal".into(),
                ));
            }
            current.push(component.as_os_str());
            if matches!(component, Component::Prefix(_)) {
                continue;
            }
            if create && !current.exists() {
                std::fs::create_dir(&current).map_err(ProviderError::Io)?;
            }
            handles.push(open_windows_directory(&current)?);
        }
        Ok(Self {
            _handles: handles,
            parent: parent.to_path_buf(),
            final_path: parent.join(file_name),
        })
    }
}

#[cfg(windows)]
fn open_windows_directory(path: &Path) -> Result<std::fs::File> {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
    let directory = OpenOptions::new()
        .read(true)
        .share_mode(WINDOWS_SHARE_WITHOUT_DELETE)
        .custom_flags(WINDOWS_OPEN_REPARSE_POINT | WINDOWS_BACKUP_SEMANTICS)
        .open(path)
        .map_err(ProviderError::Io)?;
    let metadata = directory.metadata().map_err(ProviderError::Io)?;
    reject_windows_reparse(&metadata)?;
    if !metadata.is_dir() {
        return Err(ProviderError::Config(
            "provider ancestor is not a directory".into(),
        ));
    }
    Ok(directory)
}

#[cfg(windows)]
fn reject_windows_reparse(metadata: &std::fs::Metadata) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    if metadata.file_attributes() & WINDOWS_REPARSE_ATTRIBUTE != 0 {
        Err(ProviderError::Config(
            "provider path contains a reparse point".into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(super) fn secure_read(path: &Path) -> Result<Option<String>> {
    use std::io::Read;
    let anchor = match UnixAnchor::open(path, false) {
        Ok(anchor) => anchor,
        Err(ProviderError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let descriptor = match rustix::fs::openat(
        anchor.parent(),
        &anchor.file_name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(source) if source == rustix::io::Errno::NOENT => return Ok(None),
        Err(source) => return Err(unix_error(path, source)),
    };
    let mut file = std::fs::File::from(descriptor);
    if !file.metadata().map_err(ProviderError::Io)?.is_file() {
        return Err(ProviderError::Config(
            "provider file must be a regular file".into(),
        ));
    }
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(ProviderError::Io)?;
    Ok(Some(contents))
}

#[cfg(unix)]
pub(super) fn secure_write(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let anchor = UnixAnchor::open(path, true)?;
    let temporary_name = format!(".provider-{}.tmp", uuid::Uuid::new_v4());
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
    .map_err(|source| unix_error(path, source))?;
    let mut temporary = std::fs::File::from(descriptor);
    let result = (|| {
        temporary
            .write_all(contents)
            .and_then(|()| temporary.sync_all())
            .map_err(ProviderError::Io)?;
        match rustix::fs::statat(
            anchor.parent(),
            &anchor.file_name,
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(metadata) if rustix::fs::FileType::from_raw_mode(metadata.st_mode).is_symlink() => {
                return Err(ProviderError::Config(
                    "provider file must not be a symlink".into(),
                ));
            }
            Ok(_) => {}
            Err(source) if source == rustix::io::Errno::NOENT => {}
            Err(source) => return Err(unix_error(path, source)),
        }
        rustix::fs::renameat(
            anchor.parent(),
            &temporary_name,
            anchor.parent(),
            &anchor.file_name,
        )
        .map_err(|source| unix_error(path, source))?;
        rustix::fs::fsync(anchor.parent()).map_err(|source| unix_error(path, source))?;
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
        use std::path::Component;
        let file_name = path
            .file_name()
            .ok_or_else(|| ProviderError::Config("provider path has no filename".into()))?
            .to_owned();
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let start = if path.is_absolute() { "/" } else { "." };
        let root = rustix::fs::open(
            start,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|source| unix_error(path, source))?;
        let mut handles = vec![root];
        for component in parent.components() {
            let Component::Normal(component) = component else {
                if matches!(component, Component::RootDir | Component::CurDir) {
                    continue;
                }
                return Err(ProviderError::Config(
                    "provider path may not contain parent traversal".into(),
                ));
            };
            let current = handles.last().expect("anchor has root descriptor");
            let flags = rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC;
            let child =
                match rustix::fs::openat(current, component, flags, rustix::fs::Mode::empty()) {
                    Ok(child) => child,
                    Err(source) if create && source == rustix::io::Errno::NOENT => {
                        match rustix::fs::mkdirat(
                            current,
                            component,
                            rustix::fs::Mode::RUSR
                                | rustix::fs::Mode::WUSR
                                | rustix::fs::Mode::XUSR,
                        ) {
                            Ok(()) => {}
                            Err(source) if source == rustix::io::Errno::EXIST => {}
                            Err(source) => return Err(unix_error(path, source)),
                        }
                        rustix::fs::openat(current, component, flags, rustix::fs::Mode::empty())
                            .map_err(|source| unix_error(path, source))?
                    }
                    Err(source) => return Err(unix_error(path, source)),
                };
            handles.push(child);
        }
        Ok(Self { handles, file_name })
    }
    fn parent(&self) -> &std::os::fd::OwnedFd {
        self.handles.last().expect("anchor has root descriptor")
    }
}

#[cfg(unix)]
fn unix_error(path: &Path, source: rustix::io::Errno) -> ProviderError {
    if matches!(
        source,
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR | rustix::io::Errno::NXIO
    ) {
        ProviderError::Config(format!("provider path is unsafe: {}", path.display()))
    } else {
        ProviderError::Io(std::io::Error::from_raw_os_error(source.raw_os_error()))
    }
}

#[cfg(not(any(unix, windows)))]
fn reject_symlink_components(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if current
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(ProviderError::Config(
                "provider path contains a symlink".into(),
            ));
        }
    }
    Ok(())
}
