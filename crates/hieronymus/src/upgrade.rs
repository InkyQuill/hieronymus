//! The write-side upgrade protocol (database-upgrade design, 2026-08-31;
//! ADR 0010; data-root config migration design): the one-way Python→Rust
//! cutover for an explicit data root, over the read-only preflight and typed
//! strict-terms converter in [`crate::migrate`].
//!
//! The thirteen protocol steps, in order: resolve + lock the data root and
//! refuse an active daemon; joint database/config preflight requiring a safe
//! result; render, parse back, cross-validate, fsync, and checksum staged
//! current-format config files WITHOUT promoting them; create and fsync a
//! timestamped sibling backup set; write a root-level cutover journal in
//! `prepared` state; begin an exclusive upgrade and create the schema-version
//! metadata; run the ordered SQL steps and typed converters through the same
//! transaction handle; rebuild the external-content FTS tables; run foreign
//! key, integrity, domain, and row-accounting checks; create the durable
//! semantic-rebuild job in `queued` (the design's "pending") state in that
//! same transaction; commit once → `database_committed`; atomically promote
//! the staged configs → `complete`; write the receipt.
//!
//! Recovery rules: the transaction commits once after verification, so an
//! interruption before the commit rolls back and the rerun starts fresh; an
//! interruption after the commit is an explicit `config_promotion_required`
//! state whose rerun verifies the staged checksums and promotes WITHOUT
//! rerunning the non-idempotent converters (the committed schema version and
//! the conversion ledger are the markers, never a journal guess). The daemon
//! starts only when the journal is absent or `complete`
//! ([`daemon_start_blocker`]; enforced by the daemon start path). The
//! pre-upgrade backup set is immutable: recovery copies it, imports it with
//! the current converter into a new Rust database, verifies, and promotes —
//! it never launches Python and never opens the backup in place.
//!
//! Secrets: keys are parsed into `Secret<String>` and rendered only by the
//! staged `provider.conf` writer. Journals, receipts, and reports carry
//! paths, checksums, counts, and codes — never key material.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use crate::data_root::HieronymusConfig;
use crate::db::{
    DatabaseState, SUPPORTED_RUST_SCHEMA_VERSION, apply_terminology_schema_steps, classify_database,
};
use crate::migrate::{
    MigrateError, REFUSAL_DAEMON_ACTIVE, REFUSAL_VERIFICATION_FAILED, TermConversionReport,
    VerificationReport, convert_strict_terms, integrity_result, open_read_only, run_preflight,
    sha256_file, verify_upgraded_target,
};
use crate::provider_config::{
    ProviderCatalog, ProviderProfile, default_provider_catalog, migrate_dream_provider_payload,
    provider_catalog_from_text,
};

// ---------------------------------------------------------------------------
// Protocol constants
// ---------------------------------------------------------------------------

/// Root-level cutover journal (design: a root-level journal in the data root).
pub const JOURNAL_FILE: &str = "cutover.json";
/// Sibling staging directory for rendered current-format config files.
pub const STAGING_DIR: &str = ".migrate-staging";
/// Data-root ownership lock held for the whole protocol, including promotion.
pub const LOCK_FILE: &str = ".migrate.lock";

pub const JOURNAL_STATE_PREPARED: &str = "prepared";
pub const JOURNAL_STATE_DATABASE_COMMITTED: &str = "database_committed";
pub const JOURNAL_STATE_CONFIG_PROMOTION_REQUIRED: &str = "config_promotion_required";
pub const JOURNAL_STATE_COMPLETE: &str = "complete";

const JOURNAL_VERSION: u32 = 1;
const RECEIPT_VERSION: u32 = 1;

/// The authoritative config files the cutover owns and backs up. Derived
/// state (`llmcache.tmp`) and generated plugins are never backed up as
/// configuration.
const CONFIG_FILE_NAMES: [&str; 4] = ["provider.conf", "dream.conf", "ingest.conf", "release.conf"];

/// External-content FTS projections rebuilt from authoritative rows (step 8).
const EXTERNAL_CONTENT_FTS_TABLES: [&str; 6] = [
    "short_term_memories_fts",
    "crystals_fts",
    "strict_terms_fts",
    "concepts_fts",
    "concept_facet_fts",
    "rag_chunks_fts",
];

// ---------------------------------------------------------------------------
// Options and reports
// ---------------------------------------------------------------------------

/// Protocol step boundaries that accept fault injection. This is the
/// acceptance seam for the design's failure-injection requirement: an
/// injected failure aborts the run exactly like a crashed process — no
/// cleanup, the data-root lock left behind — so tests can assert the three
/// possible end states (original intact / resumable
/// `config_promotion_required` / complete). Never set by the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionPoint {
    AfterPreflight,
    AfterStaging,
    AfterBackup,
    AfterJournalPrepared,
    /// After every in-transaction write, immediately before the single
    /// commit: the transaction is dropped without committing and rolls back.
    BeforeCommit,
    /// After the commit landed, before the journal records it.
    AfterCommit,
    AfterDatabaseCommitted,
    /// After the first staged file was promoted, mid-promotion.
    MidPromotion,
    AfterComplete,
}

/// Options for [`run_upgrade`].
#[derive(Debug, Clone, Default)]
pub struct UpgradeOptions {
    /// Failure-injection seam for the acceptance tests; `None` in production.
    pub injection: Option<InjectionPoint>,
}

/// Terminal outcome of a [`run_upgrade`] call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpgradeOutcome {
    /// The cutover finished on this run (fresh or resumed).
    Complete,
    /// The journal already said `complete`; nothing was touched.
    AlreadyComplete,
}

impl UpgradeOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            UpgradeOutcome::Complete => "complete",
            UpgradeOutcome::AlreadyComplete => "already-complete",
        }
    }
}

/// The durable semantic-rebuild job created inside the upgrade transaction
/// (design step 10). Task 8's job system spells the design's "pending" state
/// `queued`: durable, not yet claimed, and only claimable once the daemon
/// starts — which the journal gate holds until `complete`.
#[derive(Debug, Clone, Serialize)]
pub struct SemanticJobSummary {
    pub job_id: String,
    pub generation_id: String,
    pub status: String,
    pub total_chunks: i64,
}

/// What one [`run_upgrade`] call did. Only counts, paths, codes, and
/// checksums — never secret or memory text.
#[derive(Debug, Serialize)]
pub struct UpgradeReport {
    pub outcome: UpgradeOutcome,
    /// `true` when this run finished an earlier interrupted attempt instead
    /// of starting from a pristine root.
    pub resumed: bool,
    pub journal_state: String,
    pub source_state: String,
    pub target_schema_version: i64,
    pub conversion: Option<TermConversionReport>,
    pub verification: Option<VerificationReport>,
    pub semantic_job: Option<SemanticJobSummary>,
    pub backup_dir: Option<PathBuf>,
    pub backup_database_sha256: Option<String>,
    pub receipt_path: Option<PathBuf>,
}

impl UpgradeReport {
    /// Human-readable rendering; the library never prints, the CLI does.
    pub fn render_human(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "migrate upgrade (outcome: {}, journal: {}, resumed: {})\n",
            self.outcome.as_str(),
            self.journal_state,
            if self.resumed { "yes" } else { "no" }
        ));
        out.push_str(&format!(
            "schema: {} -> {}\n",
            self.source_state, self.target_schema_version
        ));
        if let Some(conversion) = &self.conversion {
            out.push_str(&format!(
                "conversion: {} converted, {} skipped, {} blocking\n",
                conversion.converted, conversion.skipped, conversion.blocking
            ));
        }
        if let Some(job) = &self.semantic_job {
            out.push_str(&format!(
                "semantic rebuild job: {} ({}, {} chunks)\n",
                job.job_id, job.status, job.total_chunks
            ));
        }
        if let Some(backup) = &self.backup_dir {
            out.push_str(&format!("backup: {}\n", backup.display()));
        }
        if let Some(receipt) = &self.receipt_path {
            out.push_str(&format!("receipt: {}\n", receipt.display()));
        }
        out
    }
}

/// What one [`run_recovery`] call did.
#[derive(Debug, Serialize)]
pub struct RecoveryReport {
    pub recovered_from: PathBuf,
    /// The replaced live database was moved here, never deleted.
    pub replacement_backup_dir: PathBuf,
    pub new_database_sha256: String,
    pub converted_terms: u64,
}

// ---------------------------------------------------------------------------
// Cutover journal
// ---------------------------------------------------------------------------

/// One staged file as recorded by the journal: its name inside the data
/// root, its verified checksum, and whether it is secret-bearing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedFileRecord {
    pub path: String,
    pub sha256: String,
    pub user_only: bool,
}

/// The root-level cutover journal. `prepared` → `database_committed` →
/// `config_promotion_required` → `complete`; every transition is an atomic
/// file rewrite plus an fsync of the data root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutoverJournal {
    pub journal_version: u32,
    pub state: String,
    pub source_state: String,
    pub target_schema_version: i64,
    pub staging_dir: String,
    /// Backup set directory, relative to the data root.
    pub backup_dir: String,
    pub staged: Vec<StagedFileRecord>,
    pub backup_database_sha256: String,
    /// Original config checksums at backup time, by file name.
    pub backup_configs: BTreeMap<String, String>,
    pub updated_at: String,
}

/// Read the journal, if one exists. A malformed journal is an explicit
/// inconsistency, never silently ignored.
pub fn read_cutover_journal(
    config: &HieronymusConfig,
) -> Result<Option<CutoverJournal>, MigrateError> {
    let path = config.data_root().join(JOURNAL_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let journal: CutoverJournal = serde_json::from_str(&text).map_err(|error| {
        MigrateError::JournalInconsistent(format!("cutover.json is unreadable: {error}"))
    })?;
    Ok(Some(journal))
}

/// The daemon-start gate: `Some(state)` while an unfinished cutover journal
/// exists. The daemon start path refuses on every non-`complete` state, so a
/// middle state never serves traffic against mismatched database/config
/// versions.
pub fn daemon_start_blocker(config: &HieronymusConfig) -> Result<Option<String>, MigrateError> {
    match read_cutover_journal(config)? {
        None => Ok(None),
        Some(journal) if journal.state == JOURNAL_STATE_COMPLETE => Ok(None),
        Some(journal) => Ok(Some(journal.state)),
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn write_journal(
    config: &HieronymusConfig,
    journal: &mut CutoverJournal,
    state: &str,
) -> Result<(), MigrateError> {
    journal.state = state.to_string();
    journal.updated_at = now_rfc3339();
    let text = serde_json::to_string_pretty(journal).map_err(|error| {
        MigrateError::JournalInconsistent(format!("journal render failed: {error}"))
    })?;
    crate::atomic::atomic_write_text(&config.data_root().join(JOURNAL_FILE), &text)?;
    fsync_dir(config.data_root())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Small primitives: fsync, ownership lock
// ---------------------------------------------------------------------------

fn fsync_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let file = std::fs::File::open(path)?;
        file.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn copy_and_sync(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::copy(source, destination)?;
    let file = std::fs::File::open(destination)?;
    file.sync_all()
}

fn write_with_mode(path: &Path, text: &str, user_only: bool) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        if user_only {
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()
    }
    #[cfg(not(unix))]
    {
        crate::atomic::atomic_write_text(path, text)?;
        Ok(())
    }
}

#[cfg(unix)]
fn require_user_only(path: &Path) -> Result<(), MigrateError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(MigrateError::UnsafeCredentialPermissions(
            path.to_path_buf(),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn require_user_only(_path: &Path) -> Result<(), MigrateError> {
    Ok(())
}

/// The data-root ownership lock (design: resolve and lock the data root;
/// promotion takes the same lock). The lock file records the owning pid; a
/// lock from a dead pid is stale and stolen, a lock from this process is
/// re-acquired (the failure-injection tests resume in-process), and a lock
/// from another live pid refuses the run. An ownerless lock file — the
/// window between another process's exclusive create and its pid write — is
/// held, never stolen: stealing it would let two runs proceed. A crashed run
/// leaves the file behind on purpose; a dead owner's file is the stale case
/// the next run steals, and an ownerless file needs the documented manual
/// `rm` (fail closed beats mutual exclusion lost).
struct DataRootLock {
    path: PathBuf,
}

impl DataRootLock {
    fn acquire(config: &HieronymusConfig) -> Result<Self, MigrateError> {
        let path = config.data_root().join(LOCK_FILE);
        if Self::try_create(&path).is_ok() {
            return Ok(Self { path });
        }
        let owner = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());
        match owner {
            Some(pid) if pid == std::process::id() => Ok(Self { path }),
            Some(pid) if process_is_alive(pid) => Err(MigrateError::UpgradeLockHeld(pid)),
            // Unreadable or ownerless: another process may be inside the
            // create-to-write window right now, so the lock is held.
            None => Err(MigrateError::UpgradeLockHeldOwnerless),
            Some(_) => {
                std::fs::remove_file(&path).ok();
                Self::try_create(&path)?;
                Ok(Self { path })
            }
        }
    }

    fn try_create(path: &Path) -> std::io::Result<()> {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_all()
    }

    fn release(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    true
}

// ---------------------------------------------------------------------------
// Joint config preflight and staging (design steps 2-3)
// ---------------------------------------------------------------------------

/// Everything the config side of the preflight established about the data
/// root. Parsed once; staging reuses the payloads.
struct ConfigInventory {
    dream_text: Option<String>,
    provider_text: Option<String>,
    ingest_text: Option<String>,
    release_text: Option<String>,
    legacy_providers: Option<toml::Table>,
    merged_catalog: ProviderCatalog,
    legacy_provider_ids: BTreeSet<String>,
}

fn parse_toml_table(text: &str, name: &str) -> Result<toml::Table, MigrateError> {
    text.parse::<toml::Table>()
        .map_err(|error| MigrateError::ConfigInvalid(format!("{name} is not valid TOML: {error}")))
}

/// Whether any profile in a provider catalog carries a non-empty key; such a
/// catalog is secret-bearing wherever it is stored.
fn catalog_has_key(catalog: &ProviderCatalog) -> bool {
    catalog
        .providers
        .values()
        .any(|profile| !profile.key().expose_secret().is_empty())
}

/// Whether a legacy `dream.conf.providers` payload carries any non-empty key.
fn legacy_payload_has_key(payload: &toml::Table) -> bool {
    payload.values().any(|value| {
        value
            .as_table()
            .map(|profile| {
                profile
                    .get("api_key")
                    .or_else(|| profile.get("key"))
                    .and_then(|key| key.as_str())
                    .map(|key| !key.is_empty())
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    })
}

/// The joint preflight's config half: parse every authoritative file, type
/// check the round-trips, merge the legacy provider payload (a collision
/// fails closed here), and demand user-only permissions for any file that
/// carries credentials. Invalid config, unsafe permissions, a provider-id
/// collision, or an unresolvable enabled workflow all fail BEFORE database
/// mutation (config design §Preflight).
fn config_preflight(config: &HieronymusConfig) -> Result<ConfigInventory, MigrateError> {
    let root = config.data_root();
    let read = |name: &str| -> Result<Option<String>, MigrateError> {
        let path = root.join(name);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(std::fs::read_to_string(&path)?))
    };
    let dream_text = read("dream.conf")?;
    let provider_text = read("provider.conf")?;
    let ingest_text = read("ingest.conf")?;
    let release_text = read("release.conf")?;

    // Legacy provider payload shape: tolerated by the dream schema, consumed
    // by the conversion.
    let legacy_providers = match &dream_text {
        Some(text) => {
            let payload = parse_toml_table(text, "dream.conf")?;
            match payload.get("providers") {
                None => None,
                Some(value) => Some(value.as_table().cloned().ok_or_else(|| {
                    MigrateError::ConfigInvalid("dream.conf providers must be a table".to_string())
                })?),
            }
        }
        None => None,
    };

    // Typed round-trips: unknown keys, invalid channels, and broken schemas
    // fail here.
    let catalog = match &provider_text {
        Some(text) => provider_catalog_from_text(text)
            .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?,
        None => default_provider_catalog(),
    };
    let dream_config = match &dream_text {
        Some(text) => Some(
            crate::dream_config::dream_config_from_text(text)
                .map_err(|error| MigrateError::ConfigInvalid(format!("dream.conf: {error}")))?,
        ),
        None => None,
    };
    if let Some(text) = &ingest_text {
        crate::ingest_config::ingest_config_from_text(text)
            .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?;
    }
    if let Some(text) = &release_text {
        crate::release_config::release_config_from_text(text)
            .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?;
    }

    // Credential permissions: every file that carries keys must be
    // user-only before anything is staged. A file that holds no key (an
    // empty key counts as keyless) exposes nothing and is accepted.
    if provider_text.is_some() && catalog_has_key(&catalog) {
        require_user_only(&root.join("provider.conf"))?;
    }
    if let Some(payload) = &legacy_providers
        && legacy_payload_has_key(payload)
    {
        require_user_only(&root.join("dream.conf"))?;
    }

    // Merge the legacy payload through the shared converter: profile
    // derivation, field precedence, and the fail-closed collision check.
    let (merged_catalog, legacy_provider_ids) = match &legacy_providers {
        Some(payload) => {
            let merged = migrate_dream_provider_payload(payload, &catalog)
                .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?;
            (merged, payload.keys().cloned().collect::<BTreeSet<_>>())
        }
        None => (catalog, BTreeSet::new()),
    };

    // Cross-validate every enabled workflow against the converted catalog
    // (existing profiles plus the legacy-derived ones): an enabled workflow
    // that cannot resolve fails closed before any mutation.
    if let Some(dream) = &dream_config {
        for (name, workflow) in &dream.workflows {
            if workflow.enabled
                && !workflow.provider.is_empty()
                && !merged_catalog.providers.contains_key(&workflow.provider)
            {
                return Err(MigrateError::WorkflowUnresolved(format!(
                    "workflow {name} references provider {} which the converted catalog does not define",
                    workflow.provider
                )));
            }
        }
    }

    Ok(ConfigInventory {
        dream_text,
        provider_text,
        ingest_text,
        release_text,
        legacy_providers,
        merged_catalog,
        legacy_provider_ids,
    })
}

/// One staged file ready to write: rendered current-format text with its
/// permission class.
struct StagedFile {
    name: String,
    text: String,
    user_only: bool,
}

/// Render every authoritative config file into its current format (design
/// step 3). Files that are already current remain unstaged and therefore
/// byte-identical after the cutover. Legacy `dream.conf.providers` blocks are
/// removed with `toml_edit` so comments, ordering, and unknown supported keys
/// survive; every staged file is parsed back and cross-validated before it is
/// written with fsync and a checksum.
fn stage_configs(
    config: &HieronymusConfig,
    inventory: &ConfigInventory,
) -> Result<Vec<StagedFileRecord>, MigrateError> {
    let mut staged: Vec<StagedFile> = Vec::new();

    // dream.conf: surgical provider-block removal, or a canonical render when
    // legacy workflow names force a structural migration.
    if let Some(original) = &inventory.dream_text {
        let payload = parse_toml_table(original, "dream.conf")?;
        let legacy_workflows = crate::dream_config::payload_has_legacy_workflows(&payload);
        let dream_text = match (&inventory.legacy_providers, legacy_workflows) {
            (None, false) => None,
            (Some(_), false) => {
                let mut document = original
                    .parse::<toml_edit::DocumentMut>()
                    .map_err(|error| {
                        MigrateError::ConfigInvalid(format!(
                            "dream.conf is not valid TOML: {error}"
                        ))
                    })?;
                document.remove("providers");
                Some(document.to_string())
            }
            (_, true) => {
                let dream = crate::dream_config::dream_config_from_text(original)
                    .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?;
                Some(
                    crate::dream_config::dream_canonical_text(&dream)
                        .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?,
                )
            }
        };
        if let Some(text) = dream_text {
            // Parse back the exact bytes that will be staged; the enabled
            // workflow cross-validation ran in the preflight.
            crate::dream_config::dream_config_from_text(&text)
                .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?;
            staged.push(StagedFile {
                name: "dream.conf".to_string(),
                text,
                user_only: legacy_payload_has_key(
                    inventory
                        .legacy_providers
                        .as_ref()
                        .unwrap_or(&toml::Table::new()),
                ),
            });
        }
    }

    // provider.conf: only the legacy-derived profiles are edited into the
    // document; existing entries and formatting survive verbatim.
    if !inventory.legacy_provider_ids.is_empty() {
        let document_text = inventory.provider_text.as_deref().unwrap_or("");
        let mut document = document_text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| {
                MigrateError::ConfigInvalid(format!("provider.conf is not valid TOML: {error}"))
            })?;
        for id in &inventory.legacy_provider_ids {
            let profile: &ProviderProfile =
                inventory.merged_catalog.providers.get(id).ok_or_else(|| {
                    MigrateError::ConfigInvalid(format!(
                        "provider profile {id} vanished during conversion"
                    ))
                })?;
            let mut table = toml_edit::Table::new();
            table.insert("name", toml_edit::value(profile.name()));
            table.insert("type", toml_edit::value(profile.provider_type()));
            table.insert("url", toml_edit::value(profile.url()));
            table.insert(
                "key",
                toml_edit::value(profile.key().expose_secret().as_str()),
            );
            table.insert(
                "timeout_seconds",
                toml_edit::value(profile.timeout_seconds()),
            );
            document.insert(id, toml_edit::Item::Table(table));
        }
        let text = document.to_string();
        // Parse back and validate the exact bytes that will be staged.
        provider_catalog_from_text(&text)
            .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?;
        staged.push(StagedFile {
            name: "provider.conf".to_string(),
            text,
            user_only: catalog_has_key(&inventory.merged_catalog),
        });
    }

    // ingest.conf / release.conf: typed round-trip. A canonical render that
    // matches the file verbatim means nothing to migrate.
    if let Some(text) = &inventory.ingest_text {
        let parsed = crate::ingest_config::ingest_config_from_text(text)
            .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?;
        let canonical = crate::ingest_config::ingest_canonical_text(&parsed)
            .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?;
        if canonical != *text {
            staged.push(StagedFile {
                name: "ingest.conf".to_string(),
                text: canonical,
                user_only: false,
            });
        }
    }
    if let Some(text) = &inventory.release_text {
        let parsed = crate::release_config::release_config_from_text(text)
            .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?;
        let canonical = crate::release_config::release_canonical_text(&parsed)
            .map_err(|error| MigrateError::ConfigInvalid(error.to_string()))?;
        if canonical != *text {
            staged.push(StagedFile {
                name: "release.conf".to_string(),
                text: canonical,
                user_only: false,
            });
        }
    }

    if staged.is_empty() {
        return Ok(Vec::new());
    }

    // Write the staging directory fresh, fsync every file, checksum, and
    // parse the on-disk bytes back one last time.
    let staging = config.data_root().join(STAGING_DIR);
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let mut records = Vec::new();
    for file in &staged {
        let path = staging.join(&file.name);
        write_with_mode(&path, &file.text, file.user_only)?;
        let on_disk = std::fs::read_to_string(&path)?;
        if on_disk != file.text {
            return Err(MigrateError::ConfigInvalid(format!(
                "staged {} does not read back byte-identical",
                file.name
            )));
        }
        records.push(StagedFileRecord {
            path: file.name.clone(),
            sha256: sha256_file(&path)?,
            user_only: file.user_only,
        });
    }
    fsync_dir(&staging)?;
    Ok(records)
}

// ---------------------------------------------------------------------------
// Backup set (design step 4)
// ---------------------------------------------------------------------------

struct BackupSet {
    /// Directory name, relative to the data root.
    relative_dir: String,
    database_sha256: String,
    config_checksums: BTreeMap<String, String>,
}

/// Create and fsync the timestamped pre-upgrade backup set under `backups/`:
/// the database plus every original config file, then verified by opening
/// the backup read-only and running an integrity check.
fn create_backup_set(config: &HieronymusConfig) -> Result<BackupSet, MigrateError> {
    let backups = config.backups_root();
    std::fs::create_dir_all(&backups)?;
    let name = unique_directory_name(&backups, &format!("pre-upgrade-{}", timestamp_compact()))?;
    let set = backups.join(&name);
    std::fs::create_dir(&set)?;

    copy_and_sync(&config.database_path(), &set.join("hieronymus.sqlite"))?;
    for sidecar in ["hieronymus.sqlite-wal", "hieronymus.sqlite-shm"] {
        let source = config.data_root().join(sidecar);
        if source.exists() {
            copy_and_sync(&source, &set.join(sidecar))?;
        }
    }
    let mut config_checksums = BTreeMap::new();
    for file in CONFIG_FILE_NAMES {
        let source = config.data_root().join(file);
        if source.exists() {
            copy_and_sync(&source, &set.join(file))?;
            config_checksums.insert(file.to_string(), sha256_file(&set.join(file))?);
        }
    }
    fsync_dir(&set)?;
    fsync_dir(&backups)?;

    // The "verified" in "last verified pre-upgrade backup": the copy must
    // open and pass an integrity check before the protocol may proceed.
    let connection = open_read_only(&set.join("hieronymus.sqlite"))?;
    if integrity_result(&connection)? != "ok" {
        return Err(MigrateError::Refused(
            "backup-integrity-degraded".to_string(),
        ));
    }
    Ok(BackupSet {
        relative_dir: format!("backups/{name}"),
        database_sha256: sha256_file(&set.join("hieronymus.sqlite"))?,
        config_checksums,
    })
}

fn timestamp_compact() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string()
}

fn unique_directory_name(parent: &Path, base: &str) -> Result<String, MigrateError> {
    if !parent.join(base).exists() {
        return Ok(base.to_string());
    }
    for ordinal in 1..10_000 {
        let candidate = format!("{base}-{ordinal}");
        if !parent.join(&candidate).exists() {
            return Ok(candidate);
        }
    }
    Err(MigrateError::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!(
            "cannot find a unique directory name under {}",
            parent.display()
        ),
    )))
}

// ---------------------------------------------------------------------------
// The one transaction (design steps 6-10)
// ---------------------------------------------------------------------------

/// The durable semantic-rebuild job for the new authoritative generation:
/// registered through the caller's transaction so it commits with everything
/// else, pinned to the current (pinned-model) embedding identity, and left
/// for the daemon to claim only after the journal is `complete`.
fn create_semantic_rebuild_job(
    transaction: &Connection,
) -> Result<SemanticJobSummary, MigrateError> {
    let identity = crate::semantic_embeddings::OnnxEmbeddingProvider::static_identity();
    let generation_id = format!("upgrade-{}", nanos());
    let to_job_error =
        |error: crate::semantic_error::SemanticError| MigrateError::SemanticJob(error.to_string());
    crate::semantic_store::begin_generation_in_transaction(transaction, &generation_id, &identity)
        .map_err(to_job_error)?;
    let record = crate::semantic_jobs::enqueue_rebuild_in_transaction(
        transaction,
        &generation_id,
        &identity,
    )
    .map_err(to_job_error)?;
    Ok(SemanticJobSummary {
        job_id: record.job_id,
        generation_id: record.generation_id,
        status: record.status,
        total_chunks: record.total_chunks,
    })
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0)
}

/// Rebuild every present external-content FTS projection from its
/// authoritative rows (design step 8).
fn rebuild_external_content_fts(connection: &Connection) -> Result<(), MigrateError> {
    for table in EXTERNAL_CONTENT_FTS_TABLES {
        let present: i64 = connection.query_row(
            "select count(*) from sqlite_master where type = 'table' and name = ?1",
            [table],
            |row| row.get(0),
        )?;
        if present > 0 {
            let quoted = table.replace('"', "\"\"");
            connection.execute(
                &format!("insert into \"{0}\"(\"{0}\") values ('rebuild')", quoted),
                [],
            )?;
        }
    }
    Ok(())
}

/// Steps 6-11 as one exclusive transaction on the live database: schema
/// version metadata, the ordered SQL steps and the typed converter, the FTS
/// rebuild, verification (foreign keys, integrity, domain invariants, row
/// accounting), and the durable semantic-rebuild job — committed once, after
/// verification. Any error (including the failure injection) drops the
/// transaction and rolls everything back.
fn upgrade_database_transaction(
    config: &HieronymusConfig,
    options: &UpgradeOptions,
) -> Result<(TermConversionReport, VerificationReport, SemanticJobSummary), MigrateError> {
    let mut connection = Connection::open(config.database_path())?;
    connection.execute_batch("pragma foreign_keys = on;")?;
    // Matches every Rust-opened database (`open_migrated`); a pragma, so it
    // must run outside the transaction.
    connection.query_row("pragma journal_mode = wal", [], |row| {
        row.get::<_, String>(0)
    })?;

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
    apply_terminology_schema_steps(&transaction)?;
    let conversion = convert_strict_terms(&transaction)?;
    rebuild_external_content_fts(&transaction)?;
    let verification = verify_upgraded_target(&transaction, &conversion)?;
    if !verification.all_checks_pass() {
        // Rollback + explicit error; the original database is intact.
        return Err(MigrateError::Refused(
            REFUSAL_VERIFICATION_FAILED.to_string(),
        ));
    }
    let job = create_semantic_rebuild_job(&transaction)?;
    if options.injection == Some(InjectionPoint::BeforeCommit) {
        return Err(MigrateError::Injected(InjectionPoint::BeforeCommit));
    }
    transaction.commit()?;
    Ok((conversion, verification, job))
}

// ---------------------------------------------------------------------------
// Promotion, receipt (design steps 12-13)
// ---------------------------------------------------------------------------

struct PromotionOutcome {
    promoted: Vec<(String, Option<String>, String)>,
    cache_invalidated: bool,
}

/// Atomically promote the staged config files (design step 12): each original
/// is renamed into the immutable backup set, each staged file renamed into
/// place, and the data root fsynced. Idempotent under interruption: staged
/// files are checksum-verified against the journal first, and a file already
/// promoted by an earlier attempt is recognized and kept.
fn promote_staged(
    config: &HieronymusConfig,
    journal: &CutoverJournal,
    options: &UpgradeOptions,
) -> Result<PromotionOutcome, MigrateError> {
    let root = config.data_root();
    let staging = root.join(&journal.staging_dir);
    let backup_dir = root.join(&journal.backup_dir);

    // Verify every staged file against the journal checksums. A missing
    // staged file is only legal when the live file already is that file.
    for file in &journal.staged {
        let live = root.join(&file.path);
        let staged_path = staging.join(&file.path);
        if !staged_path.exists() {
            if !live.exists() {
                return Err(MigrateError::JournalInconsistent(format!(
                    "staged {} and live {} are both missing",
                    staged_path.display(),
                    live.display()
                )));
            }
            let actual = sha256_file(&live)?;
            if actual != file.sha256 {
                return Err(MigrateError::StagedChecksumMismatch {
                    path: file.path.clone(),
                    expected: file.sha256.clone(),
                    actual,
                });
            }
            continue;
        }
        let actual = sha256_file(&staged_path)?;
        if actual != file.sha256 {
            return Err(MigrateError::StagedChecksumMismatch {
                path: file.path.clone(),
                expected: file.sha256.clone(),
                actual,
            });
        }
    }

    let mut promoted = Vec::new();
    for (index, file) in journal.staged.iter().enumerate() {
        let live = root.join(&file.path);
        let staged_path = staging.join(&file.path);
        if staged_path.exists() {
            if live.exists() {
                let backup_target = backup_dir.join(&file.path);
                if backup_target.exists() {
                    // An interrupted attempt moved the original already; the
                    // live copy must match it before it may be dropped.
                    if sha256_file(&backup_target)? != sha256_file(&live)? {
                        return Err(MigrateError::JournalInconsistent(format!(
                            "backup and live {} disagree",
                            file.path
                        )));
                    }
                    std::fs::remove_file(&live)?;
                } else {
                    std::fs::rename(&live, &backup_target)?;
                }
            }
            std::fs::rename(&staged_path, &live)?;
            fsync_dir(root)?;
        }
        let old_sha256 = backup_dir
            .join(&file.path)
            .exists()
            .then(|| sha256_file(&backup_dir.join(&file.path)).ok())
            .flatten();
        promoted.push((file.path.clone(), old_sha256, file.sha256.clone()));
        if options.injection == Some(InjectionPoint::MidPromotion) && index == 0 {
            return Err(MigrateError::Injected(InjectionPoint::MidPromotion));
        }
    }

    // The derived provider-model cache cannot be matched against a converted
    // catalog, so the cutover invalidates it; it is derived state, never
    // authoritative configuration.
    let cache = config.llm_cache_path();
    let cache_invalidated = cache.exists();
    if cache_invalidated {
        std::fs::remove_file(&cache)?;
    }
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
        fsync_dir(root)?;
    }
    Ok(PromotionOutcome {
        promoted,
        cache_invalidated,
    })
}

fn ledger_checksum(config: &HieronymusConfig) -> Result<String, MigrateError> {
    // The ledger is the durable per-row migration report; its deterministic
    // serialization is the receipt's migration report checksum, computable
    // identically before and after a crash.
    #[derive(Serialize)]
    struct LedgerRow {
        source_table: String,
        source_id: String,
        outcome: String,
        reason_code: String,
        target_rule_id: Option<i64>,
        target_form_id: Option<i64>,
    }
    let connection = open_read_only(&config.database_path())?;
    let present: i64 = connection.query_row(
        "select count(*) from sqlite_master where type = 'table' and name = 'term_migration_ledger'",
        [],
        |row| row.get(0),
    )?;
    let mut rows: Vec<LedgerRow> = Vec::new();
    if present > 0 {
        let mut statement = connection.prepare(
            "select source_table, source_id, outcome, reason_code,
                    target_rule_id, target_form_id
             from term_migration_ledger order by source_table, source_id",
        )?;
        rows = statement
            .query_map([], |row| {
                Ok(LedgerRow {
                    source_table: row.get(0)?,
                    source_id: row.get(1)?,
                    outcome: row.get(2)?,
                    reason_code: row.get(3)?,
                    target_rule_id: row.get(4)?,
                    target_form_id: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
    }
    let serialized = serde_json::to_string_pretty(&rows).map_err(|error| {
        MigrateError::JournalInconsistent(format!("ledger render failed: {error}"))
    })?;
    Ok(format!("{:x}", sha2::Sha256::digest(serialized.as_bytes())))
}

#[derive(Serialize)]
struct UpgradeReceipt {
    receipt_version: u32,
    completed_at: String,
    upgraded_by: String,
    /// Application version before this cutover, when a marker exists. A
    /// Python-era database carries none, so a fresh cutover records null.
    prior_app_version: Option<String>,
    source_state: String,
    backup_database_sha256: String,
    target_schema_version: i64,
    migration_report_checksum: String,
    promoted: Vec<PromotedFileRecord>,
    cache_invalidated: bool,
}

#[derive(Serialize)]
struct PromotedFileRecord {
    path: String,
    old_sha256: Option<String>,
    new_sha256: String,
}

/// Write the successful receipt into the backup set (design step 13). Like
/// every durable protocol artifact it is fsynced before the journal says
/// `complete`.
fn write_receipt(
    config: &HieronymusConfig,
    journal: &CutoverJournal,
    promotion: &PromotionOutcome,
) -> Result<PathBuf, MigrateError> {
    let backup_dir = config.data_root().join(&journal.backup_dir);
    let receipt = UpgradeReceipt {
        receipt_version: RECEIPT_VERSION,
        completed_at: now_rfc3339(),
        upgraded_by: env!("CARGO_PKG_VERSION").to_string(),
        prior_app_version: None,
        source_state: journal.source_state.clone(),
        backup_database_sha256: journal.backup_database_sha256.clone(),
        target_schema_version: journal.target_schema_version,
        migration_report_checksum: ledger_checksum(config)?,
        promoted: promotion
            .promoted
            .iter()
            .map(|(path, old, new)| PromotedFileRecord {
                path: path.clone(),
                old_sha256: old.clone(),
                new_sha256: new.clone(),
            })
            .collect(),
        cache_invalidated: promotion.cache_invalidated,
    };
    let path = backup_dir.join("receipt.json");
    let text = serde_json::to_string_pretty(&receipt).map_err(|error| {
        MigrateError::JournalInconsistent(format!("receipt render failed: {error}"))
    })?;
    crate::atomic::atomic_write_text(&path, &text)?;
    fsync_dir(&backup_dir)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// The protocol
// ---------------------------------------------------------------------------

/// `hiero migrate` write mode. Fresh data roots run the full protocol;
/// interrupted runs are detected through the journal and resumed (a
/// `config_promotion_required` state verifies the staged checksums and
/// promotes without ever rerunning the committed converters); a `complete`
/// journal is a no-op.
pub fn run_upgrade(
    config: &HieronymusConfig,
    daemon_active: bool,
    options: &UpgradeOptions,
) -> Result<UpgradeReport, MigrateError> {
    // Step 1a: never touch a root a daemon is serving.
    if daemon_active {
        return Err(MigrateError::Refused(REFUSAL_DAEMON_ACTIVE.to_string()));
    }
    // Step 1b: resolve and lock the explicit data root.
    let lock = DataRootLock::acquire(config)?;
    let had_staging = config.data_root().join(STAGING_DIR).exists();
    let existing = read_cutover_journal(config)?;

    let result = match existing {
        Some(journal) if journal.state == JOURNAL_STATE_COMPLETE => {
            already_complete_report(config, &journal)
        }
        Some(journal) if journal.state == JOURNAL_STATE_PREPARED => {
            // The transaction never committed (or its commit outran the
            // journal write). Inspect the database, never the journal alone.
            let state = classify_database(&config.database_path());
            if matches!(
                state,
                DatabaseState::RustSchema { .. } | DatabaseState::NewerSchema { .. }
            ) {
                resume_promotion(config, &journal, options)
            } else {
                // Step 5 artifact only: the pre-commit failure restored the
                // original file set; staging may be deleted per the design.
                std::fs::remove_dir_all(config.data_root().join(STAGING_DIR)).ok();
                std::fs::remove_file(config.data_root().join(JOURNAL_FILE)).ok();
                run_fresh(config, options, true)
            }
        }
        Some(journal) if journal.state == JOURNAL_STATE_DATABASE_COMMITTED => {
            resume_promotion(config, &journal, options)
        }
        Some(journal) if journal.state == JOURNAL_STATE_CONFIG_PROMOTION_REQUIRED => {
            resume_promotion(config, &journal, options)
        }
        Some(journal) => Err(MigrateError::JournalInconsistent(format!(
            "unknown cutover journal state: {}",
            journal.state
        ))),
        None => run_fresh(config, options, had_staging),
    };

    match &result {
        // An injected failure simulates a crashed process: the lock file
        // stays behind for the stale-steal on the next run.
        Err(MigrateError::Injected(_)) => {}
        Ok(_) | Err(_) => lock.release(),
    }
    result
}

fn already_complete_report(
    config: &HieronymusConfig,
    journal: &CutoverJournal,
) -> Result<UpgradeReport, MigrateError> {
    let backup_dir = config.data_root().join(&journal.backup_dir);
    let receipt = backup_dir.join("receipt.json");
    Ok(UpgradeReport {
        outcome: UpgradeOutcome::AlreadyComplete,
        resumed: false,
        journal_state: journal.state.clone(),
        source_state: journal.source_state.clone(),
        target_schema_version: journal.target_schema_version,
        conversion: None,
        verification: None,
        semantic_job: None,
        backup_dir: Some(backup_dir),
        backup_database_sha256: Some(journal.backup_database_sha256.clone()),
        receipt_path: receipt.exists().then_some(receipt),
    })
}

/// A fresh full protocol run over a data root without an unfinished journal.
fn run_fresh(
    config: &HieronymusConfig,
    options: &UpgradeOptions,
    resumed: bool,
) -> Result<UpgradeReport, MigrateError> {
    // Step 2: joint database/config preflight, requiring a safe result.
    let preflight = run_preflight(config, false)?;
    if let Some(code) = preflight.refusal_code.clone() {
        return Err(MigrateError::Refused(code));
    }
    if !preflight.conversion_safe {
        return Err(MigrateError::Refused("conversion-unsafe".to_string()));
    }
    let inventory = config_preflight(config)?;
    if options.injection == Some(InjectionPoint::AfterPreflight) {
        return Err(MigrateError::Injected(InjectionPoint::AfterPreflight));
    }

    // Step 3: staged current-format config, never promoted.
    let staged = stage_configs(config, &inventory)?;
    if options.injection == Some(InjectionPoint::AfterStaging) {
        return Err(MigrateError::Injected(InjectionPoint::AfterStaging));
    }

    // Step 4: timestamped sibling backup set, fsynced and verified.
    let backup = create_backup_set(config)?;
    if options.injection == Some(InjectionPoint::AfterBackup) {
        return Err(MigrateError::Injected(InjectionPoint::AfterBackup));
    }

    // Step 5: the journal, `prepared`, carrying staged/backup checksums.
    let mut journal = CutoverJournal {
        journal_version: JOURNAL_VERSION,
        state: JOURNAL_STATE_PREPARED.to_string(),
        source_state: preflight.detected_state.clone(),
        target_schema_version: SUPPORTED_RUST_SCHEMA_VERSION,
        staging_dir: STAGING_DIR.to_string(),
        backup_dir: backup.relative_dir.clone(),
        staged,
        backup_database_sha256: backup.database_sha256.clone(),
        backup_configs: backup.config_checksums.clone(),
        updated_at: now_rfc3339(),
    };
    write_journal(config, &mut journal, JOURNAL_STATE_PREPARED)?;
    if options.injection == Some(InjectionPoint::AfterJournalPrepared) {
        return Err(MigrateError::Injected(InjectionPoint::AfterJournalPrepared));
    }

    // Steps 6-11: the one transaction, committed once.
    let (conversion, verification, job) = upgrade_database_transaction(config, options)?;
    if options.injection == Some(InjectionPoint::AfterCommit) {
        return Err(MigrateError::Injected(InjectionPoint::AfterCommit));
    }
    write_journal(config, &mut journal, JOURNAL_STATE_DATABASE_COMMITTED)?;
    if options.injection == Some(InjectionPoint::AfterDatabaseCommitted) {
        return Err(MigrateError::Injected(
            InjectionPoint::AfterDatabaseCommitted,
        ));
    }
    // The design's explicit middle state: committed database, pre-promotion
    // configs. Even a successful run passes through here on its way to done.
    write_journal(
        config,
        &mut journal,
        JOURNAL_STATE_CONFIG_PROMOTION_REQUIRED,
    )?;

    // Step 12: atomic promotion, then `complete`.
    let promotion = promote_staged(config, &journal, options)?;
    write_journal(config, &mut journal, JOURNAL_STATE_COMPLETE)?;

    // Step 13: the receipt.
    let receipt_path = write_receipt(config, &journal, &promotion)?;
    if options.injection == Some(InjectionPoint::AfterComplete) {
        return Err(MigrateError::Injected(InjectionPoint::AfterComplete));
    }

    Ok(UpgradeReport {
        outcome: UpgradeOutcome::Complete,
        resumed,
        journal_state: journal.state.clone(),
        source_state: journal.source_state.clone(),
        target_schema_version: journal.target_schema_version,
        conversion: Some(conversion),
        verification: Some(verification),
        semantic_job: Some(job),
        backup_dir: Some(config.data_root().join(&journal.backup_dir)),
        backup_database_sha256: Some(journal.backup_database_sha256.clone()),
        receipt_path: Some(receipt_path),
    })
}

/// Finish an interrupted cutover whose database already committed: verify the
/// staged checksums, promote atomically, complete the journal, and write the
/// receipt. The converters are never rerun — the committed schema version and
/// the conversion ledger in the database are the markers.
fn resume_promotion(
    config: &HieronymusConfig,
    journal: &CutoverJournal,
    options: &UpgradeOptions,
) -> Result<UpgradeReport, MigrateError> {
    let state = classify_database(&config.database_path());
    let committed = matches!(state, DatabaseState::RustSchema { version }
        if version == journal.target_schema_version);
    if !committed {
        return Err(MigrateError::JournalInconsistent(format!(
            "journal state '{}' requires a committed target, found '{}'",
            journal.state,
            state.as_str()
        )));
    }
    // The conversion ledger is the marker that the non-idempotent conversion
    // really committed; without it the journal lies about the database.
    let connection = open_read_only(&config.database_path())?;
    let ledger_present: i64 = connection.query_row(
        "select count(*) from sqlite_master where type = 'table' and name = 'term_migration_ledger'",
        [],
        |row| row.get(0),
    )?;
    drop(connection);
    if ledger_present == 0 {
        return Err(MigrateError::JournalInconsistent(
            "committed target carries no conversion ledger".to_string(),
        ));
    }

    let mut journal = journal.clone();
    if journal.state == JOURNAL_STATE_PREPARED {
        write_journal(config, &mut journal, JOURNAL_STATE_DATABASE_COMMITTED)?;
    }
    if journal.state != JOURNAL_STATE_CONFIG_PROMOTION_REQUIRED {
        write_journal(
            config,
            &mut journal,
            JOURNAL_STATE_CONFIG_PROMOTION_REQUIRED,
        )?;
    }

    let promotion = promote_staged(config, &journal, options)?;
    write_journal(config, &mut journal, JOURNAL_STATE_COMPLETE)?;
    let receipt_path = write_receipt(config, &journal, &promotion)?;

    Ok(UpgradeReport {
        outcome: UpgradeOutcome::Complete,
        resumed: true,
        journal_state: journal.state.clone(),
        source_state: journal.source_state.clone(),
        target_schema_version: journal.target_schema_version,
        // The committed conversion is not rerun, so there is nothing new to
        // report; the ledger remains the durable per-row record.
        conversion: None,
        verification: None,
        semantic_job: None,
        backup_dir: Some(config.data_root().join(&journal.backup_dir)),
        backup_database_sha256: Some(journal.backup_database_sha256.clone()),
        receipt_path: Some(receipt_path),
    })
}

// ---------------------------------------------------------------------------
// Rust recovery (design: Backup Retention And Rust Recovery)
// ---------------------------------------------------------------------------

/// Rebuild a broken live database from the last verified pre-upgrade backup:
/// the backup is copied, imported with the current converter into a new Rust
/// database outside the data root, verified, and atomically promoted. It is
/// never opened in place and Python is never launched; the backup itself is
/// never deleted or modified, and a mid-cutover state must be resumed with
/// [`run_upgrade`] instead of recovered over. Like the upgrade protocol it
/// refuses an active daemon.
pub fn run_recovery(
    config: &HieronymusConfig,
    daemon_active: bool,
) -> Result<RecoveryReport, MigrateError> {
    // Never take the database away from a daemon that is serving it: the
    // replaced file could stay open (unlinked) on the daemon side while the
    // root already carries the replacement.
    if daemon_active {
        return Err(MigrateError::Refused(REFUSAL_DAEMON_ACTIVE.to_string()));
    }
    if let Some(journal) = read_cutover_journal(config)?
        && journal.state != JOURNAL_STATE_COMPLETE
    {
        return Err(MigrateError::RecoveryBlocked(format!(
            "cutover journal state '{}' must be resumed with hiero migrate first",
            journal.state
        )));
    }

    // The last verified pre-upgrade backup: the newest `pre-upgrade-*` set
    // whose database still opens and passes an integrity check.
    let backups = config.backups_root();
    let mut sets: Vec<PathBuf> = std::fs::read_dir(&backups)
        .map_err(|_| MigrateError::BackupMissing)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .map(|name| name.to_string_lossy().starts_with("pre-upgrade-"))
                    .unwrap_or(false)
        })
        .collect();
    sets.sort();
    sets.reverse();
    let source = sets
        .into_iter()
        .find(|set| {
            set.join("hieronymus.sqlite").exists()
                && open_read_only(&set.join("hieronymus.sqlite"))
                    .and_then(|connection| integrity_result(&connection))
                    .map(|integrity| integrity == "ok")
                    .unwrap_or(false)
        })
        .ok_or(MigrateError::BackupMissing)?;

    // Copy out of the immutable set and import with the current converter
    // into a NEW Rust database, entirely outside the data root.
    let work = tempfile::tempdir()?;
    let work_root = work.path().join("rebuild");
    std::fs::create_dir_all(&work_root)?;
    std::fs::copy(
        source.join("hieronymus.sqlite"),
        work_root.join("hieronymus.sqlite"),
    )?;
    for file in CONFIG_FILE_NAMES {
        if source.join(file).exists() {
            std::fs::copy(source.join(file), work_root.join(file))?;
        }
    }
    let work_config = HieronymusConfig::new(&work_root);
    let preflight = run_preflight(&work_config, false)?;
    if let Some(code) = preflight.refusal_code.clone() {
        return Err(MigrateError::Refused(code));
    }
    if !preflight.conversion_safe {
        return Err(MigrateError::Refused("conversion-unsafe".to_string()));
    }
    let inventory = config_preflight(&work_config)?;
    stage_configs(&work_config, &inventory)?;
    let options = UpgradeOptions::default();
    let (conversion, _verification, _job) = upgrade_database_transaction(&work_config, &options)?;

    // Verify the rebuilt database before anything in the data root moves.
    let connection = open_read_only(&work_root.join("hieronymus.sqlite"))?;
    let verification = verify_upgraded_target(&connection, &conversion)?;
    drop(connection);
    if !verification.all_checks_pass() {
        return Err(MigrateError::Refused(
            REFUSAL_VERIFICATION_FAILED.to_string(),
        ));
    }
    let new_database_sha256 = sha256_file(&work_root.join("hieronymus.sqlite"))?;

    // Atomically promote: ownership lock; the live database secured into a
    // fresh backup directory (main file plus WAL sidecars, never deleted);
    // the rebuilt database copied next to its destination and swapped in
    // with ONE plain rename — POSIX rename replaces the existing file
    // atomically, so there is no moment at which the data root has no
    // database (a crash before the rename leaves the old database and the
    // staging copy; a crash after it leaves the new one).
    let lock = DataRootLock::acquire(config)?;
    let root = config.data_root();
    let backups_dir = config.backups_root();
    std::fs::create_dir_all(&backups_dir)?;
    let replacement_name = unique_directory_name(
        &backups_dir,
        &format!("replaced-before-recovery-{}", timestamp_compact()),
    )?;
    let replacement_dir = backups_dir.join(&replacement_name);
    std::fs::create_dir(&replacement_dir)?;
    let live = config.database_path();
    // Secure the live database exactly as found first: main file plus WAL
    // sidecars, byte for byte.
    if live.exists() {
        copy_and_sync(&live, &replacement_dir.join("hieronymus.sqlite"))?;
    }
    for sidecar in ["hieronymus.sqlite-wal", "hieronymus.sqlite-shm"] {
        let sidecar_path = root.join(sidecar);
        if sidecar_path.exists() {
            copy_and_sync(&sidecar_path, &replacement_dir.join(sidecar))?;
        }
    }
    fsync_dir(&replacement_dir)?;
    // Best-effort checkpoint of the (now secured) live database so its
    // sidecars carry nothing; a database too broken to checkpoint is exactly
    // the recovery scenario, and its bytes are already preserved.
    let _ = Connection::open(&live).and_then(|connection| {
        connection.query_row("pragma wal_checkpoint(truncate)", [], |row| {
            row.get::<_, i64>(0)
        })
    });
    let staging_path = root.join(".recovery-staging-hieronymus.sqlite");
    copy_and_sync(&work_root.join("hieronymus.sqlite"), &staging_path)?;
    // Stale sidecars must go before the swap: a foreign WAL next to the new
    // database could be replayed onto it. The preserved copies are in the
    // replacement directory, so this removal never loses data; the old main
    // file stays in place until the rename below replaces it atomically.
    for sidecar in ["hieronymus.sqlite-wal", "hieronymus.sqlite-shm"] {
        std::fs::remove_file(root.join(sidecar)).ok();
    }
    std::fs::rename(&staging_path, &live)?;
    fsync_dir(root)?;
    fsync_dir(&backups_dir)?;
    lock.release();

    Ok(RecoveryReport {
        recovered_from: source,
        replacement_backup_dir: replacement_dir,
        new_database_sha256,
        converted_terms: conversion.converted,
    })
}
