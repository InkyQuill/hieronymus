use std::{
    fs,
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddrV4, TcpListener},
    path::Path,
};

use serde::{Deserialize, Serialize};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};

use crate::{agent::agent_plugins, config::HieronymusConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorCheck {
    fn new(name: impl Into<String>, status: CheckStatus, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
        }
    }
}

/// Observes the current installation without changing it.
///
/// Checks are independent snapshots. A successful result does not guarantee that a later startup
/// can still acquire the same filesystem or network resources.
pub async fn run_doctor(config: &HieronymusConfig) -> DoctorReport {
    let mut checks = Vec::new();

    for (name, path) in [
        ("provider.conf", config.provider_config_path()),
        ("dream.conf", config.dream_config_path()),
        ("ingest.conf", config.ingest_config_path()),
        ("semantic.conf", config.semantic_config_path()),
    ] {
        checks.push(check_toml_config(name, &path));
    }

    checks.push(check_database(config).await);
    checks.extend(check_agent_plugins());
    checks.push(check_bind_port(config));
    checks.push(check_derived_index(config));

    DoctorReport { checks }
}

fn check_toml_config(filename: &str, path: &Path) -> DoctorCheck {
    let name = format!("config:{filename}");
    match fs::read_to_string(path) {
        Ok(contents) => match contents.parse::<toml::Table>() {
            Ok(_) => DoctorCheck::new(name, CheckStatus::Ok, format!("{filename} is valid TOML")),
            Err(_) => DoctorCheck::new(
                name,
                CheckStatus::Fail,
                format!("{filename} is not valid TOML"),
            ),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => DoctorCheck::new(
            name,
            CheckStatus::Warn,
            format!("{filename} is absent; built-in defaults apply where supported"),
        ),
        Err(_) => DoctorCheck::new(
            name,
            CheckStatus::Fail,
            format!("{filename} cannot currently be read"),
        ),
    }
}

async fn check_database(config: &HieronymusConfig) -> DoctorCheck {
    let path = config.database_path();
    let Some(parent) = path.parent() else {
        return DoctorCheck::new(
            "database",
            CheckStatus::Fail,
            "database path has no parent directory",
        );
    };

    if !path.exists() {
        return match writable_directory_observation(parent) {
            Ok(()) => DoctorCheck::new(
                "database",
                CheckStatus::Warn,
                "database is absent; its nearest existing ancestor is currently writable",
            ),
            Err(()) => DoctorCheck::new(
                "database",
                CheckStatus::Fail,
                "database is absent and no writable existing ancestor was observed",
            ),
        };
    }

    let writable_file = fs::metadata(&path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.permissions().readonly());
    if !writable_file {
        return DoctorCheck::new(
            "database",
            CheckStatus::Fail,
            "database path is not a writable regular file",
        );
    }

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .read_only(true)
        .create_if_missing(false);
    let readable = match SqliteConnection::connect_with(&options).await {
        Ok(mut connection) => sqlx::query("PRAGMA schema_version")
            .execute(&mut connection)
            .await
            .is_ok(),
        Err(_) => false,
    };

    if !readable {
        return DoctorCheck::new(
            "database",
            CheckStatus::Fail,
            "database is not currently readable as SQLite",
        );
    }

    match writable_directory_observation(parent) {
        Ok(()) => DoctorCheck::new(
            "database",
            CheckStatus::Ok,
            "database is readable and its parent directory is currently writable",
        ),
        Err(()) => DoctorCheck::new(
            "database",
            CheckStatus::Fail,
            "database is readable but its parent directory is not currently writable",
        ),
    }
}

fn writable_directory_observation(path: &Path) -> Result<(), ()> {
    let mut candidate = path;
    while !candidate.exists() {
        candidate = candidate.parent().ok_or(())?;
    }
    if !candidate.is_dir() {
        return Err(());
    }
    tempfile::Builder::new()
        .prefix(".hiero-doctor-")
        .tempfile_in(candidate)
        .map(drop)
        .map_err(|_| ())
}

fn check_agent_plugins() -> Vec<DoctorCheck> {
    agent_plugins()
        .into_iter()
        .map(|plugin| {
            let availability = plugin.detect();
            let available = availability.detect_paths.iter().any(|path| path.exists());
            let (status, detail) = if availability.installed {
                (CheckStatus::Ok, "Hieronymus integration is installed")
            } else if available {
                (
                    CheckStatus::Warn,
                    "agent is available but Hieronymus integration is not installed",
                )
            } else {
                (CheckStatus::Ok, "agent is not detected on this system")
            };
            DoctorCheck::new(format!("agent:{}", plugin.name()), status, detail)
        })
        .collect()
}

fn check_bind_port(config: &HieronymusConfig) -> DoctorCheck {
    let port = config.resolve_port(None);
    match TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)) {
        Ok(listener) => {
            drop(listener);
            DoctorCheck::new(
                "bind-port",
                CheckStatus::Ok,
                format!("127.0.0.1:{port} is currently bindable"),
            )
        }
        Err(error) if error.kind() == ErrorKind::AddrInUse => DoctorCheck::new(
            "bind-port",
            CheckStatus::Warn,
            format!("127.0.0.1:{port} is currently in use"),
        ),
        Err(_) => DoctorCheck::new(
            "bind-port",
            CheckStatus::Fail,
            format!("127.0.0.1:{port} cannot currently be bound"),
        ),
    }
}

fn check_derived_index(config: &HieronymusConfig) -> DoctorCheck {
    let database_exists = config.database_path().is_file();
    let index_path = config.lancedb_dir();

    if index_path.exists() && !index_path.is_dir() {
        return DoctorCheck::new(
            "derived-index",
            CheckStatus::Fail,
            "derived index path is not a directory",
        );
    }

    match (index_path.is_dir(), database_exists) {
        (true, true) => DoctorCheck::new(
            "derived-index",
            CheckStatus::Ok,
            "derived index exists and can be rebuilt from the authoritative database",
        ),
        (true, false) => DoctorCheck::new(
            "derived-index",
            CheckStatus::Fail,
            "derived index exists without an authoritative database to rebuild it",
        ),
        (false, true) => DoctorCheck::new(
            "derived-index",
            CheckStatus::Warn,
            "derived index is absent and can be rebuilt from the authoritative database",
        ),
        (false, false) => DoctorCheck::new(
            "derived-index",
            CheckStatus::Ok,
            "no derived index exists before database initialization",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{CheckStatus, check_toml_config};

    #[test]
    fn config_check_does_not_echo_invalid_contents() {
        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let path = temporary.path().join("provider.conf");
        let secret = "super-secret-api-key";
        fs::write(&path, format!("api_key = '{secret}'\n[broken"))
            .expect("invalid config should be written");

        let check = check_toml_config("provider.conf", &path);

        assert_eq!(check.status, CheckStatus::Fail);
        assert!(!check.detail.contains(secret));
    }
}
