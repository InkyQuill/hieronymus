//! The shared bounded startup-state classifier (ADR 0009 "shared bounded
//! `StateClassifier`"; ADR 0010; database-upgrade and daemon-security designs,
//! 2026-08-31; Astra findings 15-16).
//!
//! [`classify`] is the single gate every startup path runs before it binds a
//! port, writes credentials, or publishes discovery, and the same gate the
//! `hiero migrate --dry-run` rehearsal begins with. It reads only version
//! markers, required-file presence, and the cutover-journal state. It does
//! **not** run typed converters, integrity scans, foreign-key checks, backups,
//! or index work, and it never mutates a byte of the data root.
//!
//! Only the current Rust schema, current config versions, and a complete or
//! absent cutover journal classify as startable ([`StartupState::Fresh`] or
//! [`StartupState::Current`]). A legacy Python schema is `migration_required`;
//! legacy config is `config_migration_required`; a committed database awaiting
//! config promotion is `config_promotion_required`. Newer, unknown, corrupt,
//! partially built, or internally inconsistent state fails closed.

use crate::data_root::HieronymusConfig;
use crate::db::{self, DatabaseState};
use crate::upgrade::{
    JOURNAL_STATE_CONFIG_PROMOTION_REQUIRED, JOURNAL_STATE_DATABASE_COMMITTED, daemon_start_blocker,
};

/// A startable data root. Both variants proceed into normal startup; every
/// other state is a [`ClassifyError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupState {
    /// An empty data root. Startup creates the current schema on first use.
    Fresh,
    /// A data root already at the current Rust schema and current config.
    Current,
}

/// Stable classifier codes. Each names a distinct remediation; callers must
/// surface [`ClassifyError::remediation`] verbatim rather than infer a command
/// from the code.
pub const CODE_MIGRATION_REQUIRED: &str = "migration_required";
pub const CODE_CONFIG_MIGRATION_REQUIRED: &str = "config_migration_required";
pub const CODE_CONFIG_PROMOTION_REQUIRED: &str = "config_promotion_required";
/// A partially applied upgrade (a `prepared` or otherwise non-terminal cutover
/// journal): fails closed, resumed with `hiero migrate`.
pub const CODE_UPGRADE_INCOMPLETE: &str = "upgrade_incomplete";
pub const CODE_INVALID: &str = "invalid_startup_state";

const MIGRATE_COMMAND: &str = "hiero migrate";
const DOCTOR_COMMAND: &str = "hiero doctor";

/// A rejected startup state. Carries a stable [`code`](ClassifyError::code) and
/// the exact [`remediation`](ClassifyError::remediation) command to run.
#[derive(Debug, thiserror::Error)]
#[error("{message} (run `{remediation}`)")]
pub struct ClassifyError {
    code: &'static str,
    message: String,
    remediation: String,
}

impl ClassifyError {
    fn new(code: &'static str, message: impl Into<String>, remediation: &str) -> Self {
        Self {
            code,
            message: message.into(),
            remediation: remediation.to_string(),
        }
    }

    fn migration_required(message: impl Into<String>) -> Self {
        Self::new(CODE_MIGRATION_REQUIRED, message, MIGRATE_COMMAND)
    }

    fn config_migration_required(message: impl Into<String>) -> Self {
        Self::new(CODE_CONFIG_MIGRATION_REQUIRED, message, MIGRATE_COMMAND)
    }

    fn config_promotion_required(message: impl Into<String>) -> Self {
        Self::new(CODE_CONFIG_PROMOTION_REQUIRED, message, MIGRATE_COMMAND)
    }

    fn upgrade_incomplete(message: impl Into<String>) -> Self {
        Self::new(CODE_UPGRADE_INCOMPLETE, message, MIGRATE_COMMAND)
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new(CODE_INVALID, message, DOCTOR_COMMAND)
    }

    /// The stable machine-readable code (one of the `CODE_*` constants). It is
    /// for logging and matching only: surface [`remediation`](Self::remediation)
    /// verbatim rather than deriving a command from the code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// The human-readable explanation, without the remediation suffix.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The exact command that resolves this state.
    pub fn remediation(&self) -> &str {
        &self.remediation
    }
}

/// Classify a data root for startup. See the module docs for the contract.
pub fn classify(config: &HieronymusConfig) -> Result<StartupState, ClassifyError> {
    classify_journal(config)?;
    let startup = classify_database(config)?;
    classify_config(config)?;
    Ok(startup)
}

/// Cutover-journal gate. Reuses the upgrade module's single journal reader
/// ([`daemon_start_blocker`]): absent or `complete` is fine; a committed but
/// unpromoted database is `config_promotion_required`; every other unfinished
/// state fails closed.
fn classify_journal(config: &HieronymusConfig) -> Result<(), ClassifyError> {
    let state = daemon_start_blocker(config).map_err(|error| {
        ClassifyError::invalid(format!("cutover journal is unreadable: {error}"))
    })?;
    match state {
        None => Ok(()),
        Some(state)
            if state == JOURNAL_STATE_DATABASE_COMMITTED
                || state == JOURNAL_STATE_CONFIG_PROMOTION_REQUIRED =>
        {
            Err(ClassifyError::config_promotion_required(format!(
                "the database is committed but config promotion is unfinished (journal state `{state}`)"
            )))
        }
        // A partially applied upgrade fails closed under its own code, and is
        // resumed with `hiero migrate` (not `hiero doctor`).
        Some(state) => Err(ClassifyError::upgrade_incomplete(format!(
            "an upgrade is in progress and unfinished (cutover journal state `{state}`)"
        ))),
    }
}

/// Database version markers (bounded, read-only).
fn classify_database(config: &HieronymusConfig) -> Result<StartupState, ClassifyError> {
    let path = config.database_path();
    match db::classify_database(&path) {
        DatabaseState::Empty => Ok(StartupState::Fresh),
        DatabaseState::RustSchema { version } if version == db::SUPPORTED_RUST_SCHEMA_VERSION => {
            db::verify_current_rust_schema(&path).map_err(|defect| {
                ClassifyError::invalid(format!(
                    "the database is marked as the current Rust schema but is not usable: {defect}"
                ))
            })?;
            Ok(StartupState::Current)
        }
        DatabaseState::RustSchema { version } => Err(ClassifyError::invalid(format!(
            "database schema version {version} is not the supported version {}",
            db::SUPPORTED_RUST_SCHEMA_VERSION
        ))),
        DatabaseState::NewerSchema { version } => Err(ClassifyError::invalid(format!(
            "database schema version {version} is newer than this binary supports (max {})",
            db::SUPPORTED_RUST_SCHEMA_VERSION
        ))),
        DatabaseState::PythonSchema => Err(ClassifyError::migration_required(
            "the database uses the legacy Python schema",
        )),
        DatabaseState::Unknown => Err(ClassifyError::invalid(
            "the database tables match no known schema",
        )),
        DatabaseState::Corrupt => Err(ClassifyError::invalid(
            "the database file cannot be opened as SQLite",
        )),
    }
}

/// Config syntax and recognized legacy shapes, via the read-only parsers. A
/// recognized legacy `dream.conf`/`provider.conf` shape is
/// `config_migration_required`; a malformed, unknown, or newer marker is a
/// fail-closed `invalid` verdict.
fn classify_config(config: &HieronymusConfig) -> Result<(), ClassifyError> {
    if let Err(error) = crate::dream_config::resolve_dream_config_readonly(config) {
        return Err(map_config_error(
            "dream.conf",
            error.is_migration_required(),
            &error.to_string(),
        ));
    }
    if let Err(error) = crate::provider_config::resolve_provider_catalog_readonly(config) {
        return Err(map_config_error(
            "provider.conf",
            error.is_migration_required(),
            &error.to_string(),
        ));
    }
    if let Err(error) = crate::ingest_config::load_ingest_config(config) {
        return Err(map_config_error("ingest.conf", false, &error.to_string()));
    }
    if let Err(error) = crate::release_config::load_release_config(config) {
        return Err(map_config_error("release.conf", false, &error.to_string()));
    }
    Ok(())
}

fn map_config_error(file: &str, migration_required: bool, detail: &str) -> ClassifyError {
    if migration_required {
        ClassifyError::config_migration_required(format!("{file} uses a legacy format: {detail}"))
    } else {
        ClassifyError::invalid(format!("{file} is not valid: {detail}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_root() -> (tempfile::TempDir, HieronymusConfig) {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path().join("data"));
        std::fs::create_dir_all(config.data_root()).unwrap();
        (root, config)
    }

    #[test]
    fn empty_root_is_fresh() {
        let (_root, config) = fresh_root();
        assert_eq!(classify(&config).unwrap(), StartupState::Fresh);
    }

    #[test]
    fn current_database_is_current() {
        let (_root, config) = fresh_root();
        db::open_migrated(&config.database_path()).unwrap();
        assert_eq!(classify(&config).unwrap(), StartupState::Current);
    }

    #[test]
    fn malformed_config_fails_closed_with_a_command() {
        let (_root, config) = fresh_root();
        std::fs::write(config.dream_config_path(), b"[nope\n").unwrap();
        let error = classify(&config).unwrap_err();
        assert_eq!(error.code(), CODE_INVALID);
        assert!(!error.remediation().is_empty());
    }
}
