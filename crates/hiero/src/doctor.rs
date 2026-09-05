//! `hiero doctor`: non-mutating health checks over the data root (security
//! design spec §Service Installation): database classification, config file
//! parsing and credential permissions, discovery-record consistency, daemon
//! reachability, protocol compatibility, semantic model/index health, and the
//! provider catalog resolve. The report renders human or JSON; exit codes
//! distinguish healthy (0) / degraded (1) / unhealthy (2). A doctor run never
//! downloads, never repairs, and never writes anything — every check is a
//! read over existing state.

use std::time::Duration;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::semantic_model::ModelStatus;

use crate::daemon::discovery::{self, DiscoveryError};
use crate::daemon::registry::PROTOCOL_REVISION;

/// How long the reachability probe waits per candidate address.
const DAEMON_PROBE_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Degraded,
    Unhealthy,
}

impl Health {
    pub fn as_str(&self) -> &'static str {
        match self {
            Health::Healthy => "healthy",
            Health::Degraded => "degraded",
            Health::Unhealthy => "unhealthy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warning,
    Error,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorFinding {
    pub level: Level,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub status: Health,
    pub findings: Vec<DoctorFinding>,
}

impl DoctorReport {
    /// Exit code contract: 0 healthy, 1 degraded, 2 unhealthy.
    pub fn exit_code(&self) -> u8 {
        match self.status {
            Health::Healthy => 0,
            Health::Degraded => 1,
            Health::Unhealthy => 2,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "status": self.status.as_str(),
            "findings": self.findings.iter().map(|finding| serde_json::json!({
                "level": finding.level.as_str(),
                "code": finding.code,
                "message": finding.message,
            })).collect::<Vec<_>>(),
        })
    }

    /// Human rendering: one line per finding, status last.
    pub fn render_human(&self) -> String {
        let mut text = String::new();
        for finding in &self.findings {
            text.push_str(&format!(
                "{:>7}  {:<28} {}\n",
                finding.level.as_str(),
                finding.code,
                finding.message,
            ));
        }
        text.push_str(&format!("overall status: {}\n", self.status.as_str()));
        text
    }

    fn push(&mut self, level: Level, code: &str, message: String) {
        self.findings.push(DoctorFinding {
            level,
            code: code.to_string(),
            message,
        });
    }
}

/// Runs every read-only check over `config` and returns the report.
pub fn run(config: &HieronymusConfig) -> DoctorReport {
    let mut report = DoctorReport {
        status: Health::Healthy,
        findings: Vec::new(),
    };
    check_database(config, &mut report);
    check_config_files(config, &mut report);
    check_credential_permissions(config, &mut report);
    check_discovery(config, &mut report);
    check_semantic(config, &mut report);
    if report
        .findings
        .iter()
        .any(|finding| finding.level == Level::Error)
    {
        report.status = Health::Unhealthy;
    } else if report
        .findings
        .iter()
        .any(|finding| finding.level == Level::Warning)
    {
        report.status = Health::Degraded;
    }
    report
}

/// Database classify (read-only): empty and supported Rust schemas are
/// healthy; the Python schema needs `hiero migrate` (degraded); anything
/// unreadable, foreign, or newer is unhealthy.
fn check_database(config: &HieronymusConfig, report: &mut DoctorReport) {
    let state = hieronymus::db::classify_database(&config.database_path());
    let code_suffix = state
        .schema_version()
        .map(|version| format!(" (schema version {version})"))
        .unwrap_or_default();
    match &state {
        hieronymus::db::DatabaseState::Empty => report.push(
            Level::Ok,
            "database",
            format!(
                "no database yet at {}; it is created on first use",
                config.database_path().display()
            ),
        ),
        hieronymus::db::DatabaseState::RustSchema { .. } => report.push(
            Level::Ok,
            "database",
            format!(
                "database is a supported Rust schema{code_suffix}: {}",
                config.database_path().display()
            ),
        ),
        hieronymus::db::DatabaseState::PythonSchema => report.push(
            Level::Warning,
            "database-upgrade-required",
            format!(
                "database uses the Python schema; run `hiero migrate` before the daemon starts: {}",
                config.database_path().display()
            ),
        ),
        hieronymus::db::DatabaseState::Corrupt => report.push(
            Level::Error,
            "database-unreadable",
            format!(
                "database file is unreadable: {}",
                config.database_path().display()
            ),
        ),
        other => report.push(
            Level::Error,
            "database-unsupported",
            format!(
                "database state '{}' is not supported: {}",
                other.as_str(),
                config.database_path().display()
            ),
        ),
    }
}

/// Config files: dream.conf and provider.conf must parse when present
/// (provider resolution is strictly report-only — no legacy migration
/// writes). A missing config root is a degraded fresh-install state.
fn check_config_files(config: &HieronymusConfig, report: &mut DoctorReport) {
    if !config.config_root().is_dir() {
        report.push(
            Level::Warning,
            "config-root-missing",
            format!(
                "config root does not exist yet: {}",
                config.config_root().display()
            ),
        );
        return;
    }
    match hieronymus::dream_config::resolve_dream_config_readonly(config) {
        Ok(_) => report.push(
            Level::Ok,
            "dream-conf",
            if config.dream_config_path().exists() {
                "dream.conf loaded".to_string()
            } else {
                "dream.conf is missing; built-in defaults apply".to_string()
            },
        ),
        Err(error) => report.push(
            Level::Warning,
            "dream-conf-invalid",
            format!("dream.conf is invalid: {error}"),
        ),
    }
    match hieronymus::provider_config::resolve_provider_catalog_readonly(config) {
        Ok(_) => report.push(
            Level::Ok,
            "provider-catalog",
            "provider catalog resolved (report-only; nothing was migrated)".to_string(),
        ),
        Err(error) => report.push(
            Level::Error,
            "provider-conf-invalid",
            format!("provider.conf is invalid: {error}"),
        ),
    }
}

/// Credential permissions: the bearer token must be user-only (0600). A
/// missing token is fine while no daemon ever ran; a missing token alongside
/// a discovery record is an inconsistency.
fn check_credential_permissions(config: &HieronymusConfig, report: &mut DoctorReport) {
    let token_path = config.daemon_token_path();
    let discovery_present = config.daemon_discovery_path().exists();
    if !token_path.exists() {
        let level = if discovery_present {
            Level::Warning
        } else {
            Level::Ok
        };
        report.push(
            level,
            "token-permissions",
            if discovery_present {
                "discovery record exists but the bearer token is missing".to_string()
            } else {
                "no bearer token yet (the daemon has not run)".to_string()
            },
        );
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(&token_path) {
            Ok(metadata) => {
                let mode = metadata.permissions().mode() & 0o777;
                if mode == 0o600 {
                    report.push(
                        Level::Ok,
                        "token-permissions",
                        "bearer token is user-only (0600)".to_string(),
                    );
                } else {
                    report.push(
                        Level::Warning,
                        "token-permissions",
                        format!(
                            "bearer token at {} is readable beyond its owner (mode {:o}); expected 600",
                            token_path.display(),
                            mode
                        ),
                    );
                }
            }
            Err(error) => report.push(
                Level::Warning,
                "token-permissions",
                format!("could not stat {}: {error}", token_path.display()),
            ),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = report;
    }
}

/// Discovery record consistency, protocol compatibility, and daemon
/// reachability (only when a record exists — a doctor run never starts one).
fn check_discovery(config: &HieronymusConfig, report: &mut DoctorReport) {
    let record = match discovery::read_discovery(config) {
        Ok(record) => record,
        Err(DiscoveryError::Missing { path }) => {
            report.push(
                Level::Ok,
                "daemon",
                format!(
                    "daemon not running (no discovery record at {})",
                    path.display()
                ),
            );
            return;
        }
        Err(DiscoveryError::Unreadable { path }) => {
            report.push(
                Level::Warning,
                "discovery-unreadable",
                format!(
                    "discovery record at {} is not valid JSON; remove it or restart the daemon",
                    path.display()
                ),
            );
            return;
        }
    };

    if record.protocol_version != PROTOCOL_REVISION {
        report.push(
            Level::Warning,
            "protocol-mismatch",
            format!(
                "discovery record speaks protocol {} but this binary serves {}; restart the daemon",
                record.protocol_version, PROTOCOL_REVISION
            ),
        );
    }
    if record.discovery_version != discovery::DISCOVERY_VERSION {
        report.push(
            Level::Warning,
            "discovery-version-mismatch",
            format!(
                "discovery record version {} differs from this binary's {}",
                record.discovery_version,
                discovery::DISCOVERY_VERSION
            ),
        );
    }
    let loopback = record
        .host
        .parse::<std::net::IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false);
    if !loopback {
        report.push(
            Level::Error,
            "discovery-not-loopback",
            format!(
                "discovery record advertises non-loopback host {} (ADR 0012: loopback only)",
                record.host
            ),
        );
        return;
    }
    match probe(&record.host, record.port) {
        true => report.push(
            Level::Ok,
            "daemon-reachable",
            format!(
                "daemon is reachable at {}:{} (pid {})",
                record.host, record.port, record.pid
            ),
        ),
        false => report.push(
            Level::Warning,
            "daemon-unreachable",
            format!(
                "discovery record points at {}:{} but nothing answered within {} ms (stale record or stopped daemon)",
                record.host,
                record.port,
                DAEMON_PROBE_TIMEOUT.as_millis()
            ),
        ),
    }
}

/// Whether anything accepts TCP connections at `host:port` right now.
fn probe(host: &str, port: u16) -> bool {
    use std::net::ToSocketAddrs;
    let Ok(addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    for address in addresses {
        if std::net::TcpStream::connect_timeout(&address, DAEMON_PROBE_TIMEOUT).is_ok() {
            return true;
        }
    }
    false
}

/// Semantic health without any download: model presence verdict (missing is
/// the supported FTS-only baseline), active generation, and index integrity.
fn check_semantic(config: &HieronymusConfig, report: &mut DoctorReport) {
    let status = match hieronymus::semantic_arming::semantic_status(config) {
        Ok(status) => status,
        Err(error) => {
            report.push(
                Level::Warning,
                "semantic-state",
                format!("semantic state could not be read: {error}"),
            );
            return;
        }
    };
    match &status.model_status {
        ModelStatus::Available => report.push(
            Level::Ok,
            "semantic-model",
            format!(
                "embedding model present at {} (tokenizer {})",
                hieronymus::semantic_store::SemanticStore::model_path_for(config).display(),
                status.tokenizer,
            ),
        ),
        ModelStatus::Missing => report.push(
            Level::Ok,
            "semantic-model",
            "embedding model not acquired; recall runs FTS-only (`hiero semantic enable` acquires it, never doctor)".to_string(),
        ),
        ModelStatus::Invalid(reason) => report.push(
            Level::Warning,
            "semantic-model",
            format!("embedding model failed its pre-check: {reason}"),
        ),
    }
    match &status.active_generation {
        None => report.push(
            Level::Ok,
            "semantic-generation",
            "no active semantic generation yet (rebuild populates one)".to_string(),
        ),
        Some(manifest) => {
            let identity = &manifest.identity;
            if status.generation_intact {
                report.push(
                    Level::Ok,
                    "semantic-generation",
                    format!(
                        "generation {} is active ({})",
                        manifest.generation_id,
                        describe_identity(identity),
                    ),
                );
            } else {
                report.push(
                    Level::Warning,
                    "semantic-generation",
                    format!(
                        "generation {} is active but its index is missing from disk; a rebuild is required",
                        manifest.generation_id,
                    ),
                );
            }
        }
    }
}

fn describe_identity(identity: &hieronymus::semantic_embeddings::EmbeddingIdentity) -> String {
    format!(
        "{} {}@{} ({} dims, tokenizer {})",
        identity.provider(),
        identity.model(),
        identity.revision(),
        identity.dimensions(),
        identity.tokenizer(),
    )
}
