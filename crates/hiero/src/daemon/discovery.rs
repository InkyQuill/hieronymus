//! Discovery record and per-installation credentials (ADR 0012 as amended
//! 2026-09-03): a non-secret discovery record written atomically, and one
//! static CSPRNG bearer token stored in a separate user-only (0600) file.
//! No rotation ceremony and no session state.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::secret::Secret;

/// Bumped only when the discovery record shape changes.
pub const DISCOVERY_VERSION: u32 = 1;

/// Token length in random bytes (hex-encoded: 64 characters).
const TOKEN_BYTES: usize = 32;

/// Instance id length in random bytes (hex-encoded: 32 characters).
const INSTANCE_ID_BYTES: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryRecord {
    pub discovery_version: u32,
    /// ADR 0015's exact MCP revision.
    pub protocol_version: String,
    pub host: String,
    pub port: u16,
    pub pid: u32,
    pub instance_id: String,
    /// RFC 3339 UTC timestamp.
    pub started_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("no running local service discovered: {path} is missing")]
    Missing { path: std::path::PathBuf },
    #[error("discovery record at {path} is unreadable")]
    Unreadable { path: std::path::PathBuf },
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("bearer token at {path} is missing")]
    Missing { path: std::path::PathBuf },
    #[error("bearer token at {path} is empty")]
    Empty { path: std::path::PathBuf },
}

/// Generate a bearer token from the operating system CSPRNG.
pub fn generate_bearer_token() -> Result<Secret<String>, getrandom::Error> {
    Ok(Secret::new(random_hex(TOKEN_BYTES)?))
}

pub(crate) fn generate_instance_id() -> Result<String, getrandom::Error> {
    random_hex(INSTANCE_ID_BYTES)
}

fn random_hex(bytes: usize) -> Result<String, getrandom::Error> {
    let mut buffer = vec![0_u8; bytes];
    getrandom::fill(&mut buffer)?;
    let mut text = String::with_capacity(bytes * 2);
    for byte in buffer {
        text.push_str(&format!("{byte:02x}"));
    }
    Ok(text)
}

fn unix_user_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o600);
            let _ = std::fs::set_permissions(path, permissions);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Write the bearer token atomically with user-only permissions. Windows is
/// outside the initial cutover (ADR 0009); the permission tightening is a
/// no-op there.
pub fn write_token(config: &HieronymusConfig, token: &Secret<String>) -> std::io::Result<()> {
    let path = config.daemon_token_path();
    hieronymus::atomic::atomic_write_text(&path, token.expose_secret())?;
    unix_user_only(&path);
    Ok(())
}

/// Read the bearer token back (trimmed).
pub fn read_token(config: &HieronymusConfig) -> Result<Secret<String>, CredentialError> {
    let path = config.daemon_token_path();
    let text = std::fs::read_to_string(&path).map_err(|_| CredentialError::Missing {
        path: path.clone(),
    })?;
    let token = text.trim();
    if token.is_empty() {
        return Err(CredentialError::Empty { path });
    }
    Ok(Secret::new(token.to_string()))
}

/// Publish the discovery record atomically.
pub fn write_discovery(
    config: &HieronymusConfig,
    record: &DiscoveryRecord,
) -> std::io::Result<()> {
    let mut text = serde_json::to_string_pretty(record)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    text.push('\n');
    hieronymus::atomic::atomic_write_text(&config.daemon_discovery_path(), &text)
}

/// Read the discovery record.
pub fn read_discovery(config: &HieronymusConfig) -> Result<DiscoveryRecord, DiscoveryError> {
    let path = config.daemon_discovery_path();
    let text = std::fs::read_to_string(&path).map_err(|_| DiscoveryError::Missing {
        path: path.clone(),
    })?;
    serde_json::from_str(&text).map_err(|_| DiscoveryError::Unreadable { path })
}

/// Remove the discovery record, but only if it still belongs to
/// `instance_id` (a newer daemon's record is never deleted).
pub fn remove_discovery(config: &HieronymusConfig, instance_id: &str) -> bool {
    match read_discovery(config) {
        Ok(record) if record.instance_id == instance_id => {
            std::fs::remove_file(config.daemon_discovery_path()).is_ok()
        }
        _ => false,
    }
}

/// A best-effort JSON object for diagnostics; never contains the token.
pub fn redacted_summary(record: &DiscoveryRecord) -> serde_json::Value {
    json!({
        "protocol_version": record.protocol_version,
        "host": record.host,
        "port": record.port,
        "pid": record.pid,
        "instance_id": record.instance_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_round_trip_and_permissions() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let token = generate_bearer_token().unwrap();
        write_token(&config, &token).unwrap();
        assert_eq!(*read_token(&config).unwrap().expose_secret(), *token.expose_secret());
        assert_eq!(token.expose_secret().len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(config.daemon_token_path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn discovery_round_trip_and_guarded_removal() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let record = DiscoveryRecord {
            discovery_version: DISCOVERY_VERSION,
            protocol_version: "2026-07-28".to_string(),
            host: "127.0.0.1".to_string(),
            port: 9768,
            pid: 42,
            instance_id: "ab".repeat(16),
            started_at: "2026-09-04T00:00:00+00:00".to_string(),
        };
        write_discovery(&config, &record).unwrap();
        assert_eq!(read_discovery(&config).unwrap(), record);

        // A different instance never deletes the record.
        assert!(!remove_discovery(&config, "other-instance"));
        assert!(remove_discovery(&config, &record.instance_id));
        assert!(!config.daemon_discovery_path().exists());
    }

    #[test]
    fn missing_discovery_and_token_are_typed_errors() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let error = read_discovery(&config).unwrap_err();
        assert!(error.to_string().contains("no running local service"));
        let error = read_token(&config).unwrap_err();
        assert!(error.to_string().contains("bearer token"));
    }

    #[test]
    fn instance_ids_differ() {
        let first = generate_instance_id().unwrap();
        let second = generate_instance_id().unwrap();
        assert_ne!(first, second);
        assert_eq!(first.len(), 32);
    }
}
