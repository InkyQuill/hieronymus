use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use secrecy::{ExposeSecret, SecretString};
use url::Url;

use super::{ProviderError, Result};

#[derive(Clone, Default)]
pub enum CredentialSource {
    #[default]
    None,
    Inline(SecretString),
    Environment {
        variable: String,
    },
    File {
        path: PathBuf,
    },
}

impl std::fmt::Debug for CredentialSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CredentialSource([REDACTED])")
    }
}

impl CredentialSource {
    fn resolve(&self) -> Result<SecretString> {
        let value = match self {
            Self::None => {
                return Err(ProviderError::Credential(
                    "no credential source configured".into(),
                ));
            }
            Self::Inline(value) => value.expose_secret().to_owned(),
            Self::Environment { variable } => std::env::var(variable).map_err(|_| {
                ProviderError::Credential(format!("environment variable {variable} is unavailable"))
            })?,
            Self::File { path } => secure_read(path)?.ok_or_else(|| {
                ProviderError::Credential("credential file is unavailable".into())
            })?,
        };
        let value = value.trim().to_owned();
        if value.is_empty() {
            return Err(ProviderError::Credential(
                "credential source was empty".into(),
            ));
        }
        Ok(SecretString::from(value))
    }

    pub async fn resolve_async(&self) -> Result<SecretString> {
        match self {
            Self::File { .. } => {
                let source = self.clone();
                tokio::task::spawn_blocking(move || source.resolve())
                    .await
                    .map_err(|_| ProviderError::Transport)?
            }
            _ => self.resolve(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderDefaults {
    pub provider: String,
    pub model: String,
}

#[derive(Clone)]
pub struct ProviderProfile {
    id: String,
    name: String,
    provider_type: String,
    url: String,
    credential: CredentialSource,
    timeout_seconds: f64,
}

impl std::fmt::Debug for ProviderProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderProfile")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("provider_type", &self.provider_type)
            .field("url", &self.url)
            .field("credential", &"[REDACTED]")
            .field("timeout_seconds", &self.timeout_seconds)
            .finish()
    }
}

impl ProviderProfile {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        provider_type: impl Into<String>,
        url: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            provider_type: provider_type.into(),
            url: url.into(),
            credential: CredentialSource::None,
            timeout_seconds: 30.0,
        }
    }
    #[must_use]
    pub fn with_credential_source(mut self, source: CredentialSource) -> Self {
        self.credential = source;
        self
    }
    #[must_use]
    pub fn with_inline_credential(mut self, credential: impl Into<String>) -> Self {
        self.credential = CredentialSource::Inline(SecretString::from(credential.into()));
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
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn provider_type(&self) -> &str {
        &self.provider_type
    }
    pub fn base_url(&self) -> &str {
        &self.url
    }
    pub fn timeout(&self) -> Result<Duration> {
        if self.timeout_seconds <= 0.0 {
            return Err(ProviderError::Config(
                "provider timeout must be positive and representable".into(),
            ));
        }
        Duration::try_from_secs_f64(self.timeout_seconds).map_err(|_| {
            ProviderError::Config("provider timeout must be positive and representable".into())
        })
    }
    pub fn credential(&self) -> &CredentialSource {
        &self.credential
    }
    pub async fn resolve_credential_async(&self) -> Result<SecretString> {
        self.credential.resolve_async().await
    }

    pub fn validate(&self) -> Result<()> {
        validate_id(&self.id)?;
        if self.name.trim().is_empty() {
            return Err(ProviderError::Config(format!(
                "providers.{}.name is required",
                self.id
            )));
        }
        if !matches!(
            self.provider_type.as_str(),
            "openai" | "anthropic" | "google" | "ollama"
        ) {
            return Err(ProviderError::Unsupported(self.provider_type.clone()));
        }
        self.timeout()?;
        let parsed = Url::parse(&self.url)
            .map_err(|_| ProviderError::Config("provider base URL is invalid".into()))?;
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(ProviderError::UnsafeEndpoint(
                "provider base URL contains forbidden components".into(),
            ));
        }
        let literal_loopback = matches!(parsed.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
            || matches!(parsed.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback());
        if self.provider_type == "ollama" && !(parsed.scheme() == "http" && literal_loopback) {
            return Err(ProviderError::UnsafeEndpoint(
                "native Ollama requires an IP-literal loopback HTTP URL".into(),
            ));
        }
        if self.provider_type != "ollama" && parsed.scheme() != "https" && !literal_loopback {
            return Err(ProviderError::UnsafeEndpoint(
                "remote providers require HTTPS".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderCatalog {
    providers: BTreeMap<String, ProviderProfile>,
    defaults: ProviderDefaults,
}

impl ProviderCatalog {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let Some(contents) = secure_read(path.as_ref())? else {
            return Ok(Self::default());
        };
        let root = contents
            .parse::<toml::Table>()
            .map_err(|_| ProviderError::Config("provider catalog contains invalid TOML".into()))?;
        let mut catalog = Self::default();
        for (id, value) in root {
            let table = value
                .as_table()
                .ok_or_else(|| ProviderError::Config(format!("{id} must be a table")))?;
            if id == "defaults" {
                reject_unknown(table, &["provider", "model"], "defaults")?;
                catalog.defaults = ProviderDefaults {
                    provider: string_or(table, "provider", "")?,
                    model: string_or(table, "model", "")?,
                };
                continue;
            }
            validate_id(&id)?;
            reject_unknown(
                table,
                &[
                    "name",
                    "type",
                    "url",
                    "key",
                    "api_key",
                    "key_env",
                    "key_file",
                    "timeout_seconds",
                ],
                &id,
            )?;
            let profile = ProviderProfile {
                id: id.clone(),
                name: string_or(table, "name", &id)?,
                provider_type: canonical_provider_type(required_string(table, "type", &id)?),
                url: required_string(table, "url", &id)?,
                credential: credential_from_table(table, &id)?,
                timeout_seconds: number_or(table, "timeout_seconds", 30.0)?,
            };
            profile.validate()?;
            catalog.providers.insert(id, profile);
        }
        catalog.validate_defaults()?;
        Ok(catalog)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate_defaults()?;
        let mut root = toml::Table::new();
        for (id, profile) in &self.providers {
            profile.validate()?;
            let mut table = toml::Table::new();
            table.insert("name".into(), profile.name.clone().into());
            table.insert("type".into(), profile.provider_type.clone().into());
            table.insert("url".into(), profile.url.clone().into());
            match &profile.credential {
                CredentialSource::None => {}
                CredentialSource::Inline(value) => {
                    table.insert("key".into(), value.expose_secret().to_owned().into());
                }
                CredentialSource::Environment { variable } => {
                    table.insert("key_env".into(), variable.clone().into());
                }
                CredentialSource::File { path } => {
                    table.insert(
                        "key_file".into(),
                        path.to_string_lossy().into_owned().into(),
                    );
                }
            }
            table.insert("timeout_seconds".into(), profile.timeout_seconds.into());
            root.insert(id.clone(), table.into());
        }
        let mut defaults = toml::Table::new();
        defaults.insert("provider".into(), self.defaults.provider.clone().into());
        defaults.insert("model".into(), self.defaults.model.clone().into());
        root.insert("defaults".into(), defaults.into());
        let contents = toml::to_string_pretty(&root).map_err(|_| {
            ProviderError::Config("provider catalog could not be serialized".into())
        })?;
        secure_write(path.as_ref(), contents.as_bytes())
    }

    pub fn get(&self, id: &str) -> Option<&ProviderProfile> {
        self.providers.get(id)
    }
    pub fn defaults(&self) -> &ProviderDefaults {
        &self.defaults
    }
    pub fn set_defaults(&mut self, defaults: ProviderDefaults) {
        self.defaults = defaults;
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
            .map(|(id, profile)| (id.as_str(), profile))
    }

    fn validate_defaults(&self) -> Result<()> {
        if !self.defaults.provider.is_empty()
            && !self.providers.contains_key(&self.defaults.provider)
        {
            return Err(ProviderError::Config(format!(
                "default provider is missing: {}",
                self.defaults.provider
            )));
        }
        Ok(())
    }
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id == "defaults"
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        Err(ProviderError::Config(format!("invalid provider id: {id}")))
    } else {
        Ok(())
    }
}

fn reject_unknown(table: &toml::Table, allowed: &[&str], prefix: &str) -> Result<()> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ProviderError::Config(format!(
                "unknown provider config setting: {prefix}.{key}"
            )));
        }
    }
    Ok(())
}

fn required_string(table: &toml::Table, key: &str, prefix: &str) -> Result<String> {
    let value = table
        .get(key)
        .and_then(toml::Value::as_str)
        .ok_or_else(|| ProviderError::Config(format!("{prefix}.{key} is required")))?;
    if value.is_empty() {
        return Err(ProviderError::Config(format!("{prefix}.{key} is required")));
    }
    Ok(value.to_owned())
}

fn string_or(table: &toml::Table, key: &str, default: &str) -> Result<String> {
    match table.get(key) {
        None => Ok(default.to_owned()),
        Some(value) => value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| ProviderError::Config(format!("{key} must be a string"))),
    }
}

fn number_or(table: &toml::Table, key: &str, default: f64) -> Result<f64> {
    match table.get(key) {
        None => Ok(default),
        Some(toml::Value::Float(value)) => Ok(*value),
        Some(toml::Value::Integer(value)) => Ok(*value as f64),
        Some(_) => Err(ProviderError::Config(format!("{key} must be a number"))),
    }
}

fn credential_from_table(table: &toml::Table, prefix: &str) -> Result<CredentialSource> {
    let present = ["key", "api_key", "key_env", "key_file"]
        .into_iter()
        .filter(|key| table.contains_key(*key))
        .count();
    if present > 1 {
        return Err(ProviderError::Config(format!(
            "{prefix} has multiple credential sources"
        )));
    }
    if let Some(value) = table.get("key") {
        return value
            .as_str()
            .map(|value| CredentialSource::Inline(SecretString::from(value.to_owned())))
            .ok_or_else(|| ProviderError::Config(format!("{prefix}.key must be a string")));
    }
    if let Some(value) = table.get("api_key") {
        return value
            .as_str()
            .map(|value| CredentialSource::Inline(SecretString::from(value.to_owned())))
            .ok_or_else(|| ProviderError::Config(format!("{prefix}.api_key must be a string")));
    }
    if let Some(value) = table.get("key_env") {
        return value
            .as_str()
            .map(|variable| CredentialSource::Environment {
                variable: variable.to_owned(),
            })
            .ok_or_else(|| ProviderError::Config(format!("{prefix}.key_env must be a string")));
    }
    if let Some(value) = table.get("key_file") {
        return value
            .as_str()
            .map(|path| CredentialSource::File {
                path: PathBuf::from(path),
            })
            .ok_or_else(|| ProviderError::Config(format!("{prefix}.key_file must be a string")));
    }
    Ok(CredentialSource::None)
}

fn canonical_provider_type(value: String) -> String {
    if value == "gemini" {
        "google".to_owned()
    } else {
        value
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
pub(crate) fn secure_write(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| ProviderError::Config("provider path has no parent".into()))?;
    reject_symlink_components(parent)?;
    std::fs::create_dir_all(parent).map_err(ProviderError::Io)?;
    reject_symlink_components(parent)?;
    if let Ok(metadata) = path.symlink_metadata()
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(ProviderError::Config(
            "provider file must be a regular non-symlink file".into(),
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
        if let Ok(metadata) = path.symlink_metadata()
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(ProviderError::Config(
                "provider file must be a regular non-symlink file".into(),
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
pub(crate) fn secure_write(path: &Path, contents: &[u8]) -> Result<()> {
    use std::{fs::OpenOptions, io::Write, os::windows::fs::OpenOptionsExt};
    let path = normalize_windows_path(path)?;
    let anchor = WindowsAnchor::open(&path, true)?;
    match OpenOptions::new()
        .read(true)
        .share_mode(WINDOWS_SHARE_WITHOUT_DELETE)
        .custom_flags(WINDOWS_OPEN_REPARSE_POINT)
        .open(&anchor.final_path)
    {
        Ok(existing) => {
            let metadata = existing.metadata().map_err(ProviderError::Io)?;
            reject_windows_reparse(&metadata)?;
            if !metadata.is_file() {
                return Err(ProviderError::Config(
                    "provider file must be a regular file".into(),
                ));
            }
        }
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
pub(crate) fn secure_write(path: &Path, contents: &[u8]) -> Result<()> {
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
            Ok(metadata) if !rustix::fs::FileType::from_raw_mode(metadata.st_mode).is_file() => {
                return Err(ProviderError::Config(
                    "provider file must be a regular non-symlink file".into(),
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

#[cfg(unix)]
pub(crate) fn secure_read_bounded(path: &Path, max_bytes: usize) -> Result<Option<String>> {
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
    let file = std::fs::File::from(descriptor);
    if !file.metadata().map_err(ProviderError::Io)?.is_file() {
        return Err(ProviderError::Config(
            "cache file must be a regular file".into(),
        ));
    }
    let mut contents = String::new();
    file.take(max_bytes.saturating_add(1) as u64)
        .read_to_string(&mut contents)
        .map_err(ProviderError::Io)?;
    if contents.len() > max_bytes {
        return Err(ProviderError::ResponseTooLarge { limit: max_bytes });
    }
    Ok(Some(contents))
}

#[cfg(windows)]
pub(crate) fn secure_read_bounded(path: &Path, max_bytes: usize) -> Result<Option<String>> {
    use std::{fs::OpenOptions, io::Read, os::windows::fs::OpenOptionsExt};
    let path = normalize_windows_path(path)?;
    let anchor = match WindowsAnchor::open(&path, false) {
        Ok(anchor) => anchor,
        Err(ProviderError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let file = match OpenOptions::new()
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
            "cache file must be a regular file".into(),
        ));
    }
    let mut contents = String::new();
    file.take(max_bytes.saturating_add(1) as u64)
        .read_to_string(&mut contents)
        .map_err(ProviderError::Io)?;
    if contents.len() > max_bytes {
        return Err(ProviderError::ResponseTooLarge { limit: max_bytes });
    }
    Ok(Some(contents))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn secure_read_bounded(path: &Path, max_bytes: usize) -> Result<Option<String>> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ProviderError::Io(error)),
    };
    if !file.metadata().map_err(ProviderError::Io)?.is_file() {
        return Err(ProviderError::Config(
            "cache file must be a regular file".into(),
        ));
    }
    let mut contents = String::new();
    file.take(max_bytes.saturating_add(1) as u64)
        .read_to_string(&mut contents)
        .map_err(ProviderError::Io)?;
    if contents.len() > max_bytes {
        return Err(ProviderError::ResponseTooLarge { limit: max_bytes });
    }
    Ok(Some(contents))
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
