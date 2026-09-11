//! `hiero update` (distribution spec §Update And One-Way Cutover): resolve a
//! release from a release-feed directory, verify its checksum, stage it
//! alongside the current version, check protocol AND schema compatibility,
//! stop the service, switch the stable links, start and health-check the new
//! daemon. Health failure before a schema upgrade restores the prior links
//! automatically; a required database upgrade completes the install but keeps
//! the daemon stopped until the user runs `hiero migrate`.
//!
//! Feed format (the layout `scripts/release-build.sh` produces): a directory
//! holding `hieronymus-<version>-x86_64-unknown-linux-gnu.tar.gz` plus its
//! `.sha256` sibling, optionally described by a `release.json`
//! (`{"version", "archive", "sha256", "signature"}`). A non-null `signature`
//! is refused: signature verification is waived (not configured) for the
//! first release line, and the updater fails closed rather than trusting
//! something it cannot verify. HTTPS feeds are verified and staged by
//! `release_source`; this activation flow consumes the resulting local directory.
//!
//! Compatibility gates, in order, all before anything is changed:
//! candidate protocol revision must equal this binary's (a protocol change
//! needs an explicit bootstrap install), the candidate must support the
//! database schema on disk (a newer-schema target is refused — the updater
//! never launches an older binary against a newer schema), and no daemon may
//! be running unless it is the managed service (which the update stops).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use hieronymus::data_root::{HieronymusConfig, load_config};
use hieronymus::ownership::RootOwnership;

#[cfg(unix)]
use crate::app::LINK_NAMES;
use crate::app::{AppLayout, TARGET_TRIPLE, compare_versions};
#[cfg(test)]
use crate::daemon::discovery;
use crate::daemon::registry::PROTOCOL_REVISION;
use crate::lifecycle::operation::LifecycleOperation;
use crate::lifecycle::{self, DiscoveryHealth};
use crate::service::{self, ServiceManager, ServiceOptions, SystemdManager};

/// How often the post-activation readiness poll re-probes. Tighter under
/// `cfg(test)` so the unit suite that drives the poll loops stays snappy.
#[cfg(not(test))]
const READY_POLL: Duration = Duration::from_millis(100);
#[cfg(test)]
const READY_POLL: Duration = Duration::from_millis(20);

/// How long the updater waits for the started candidate to publish a live,
/// authenticated endpoint that reports the expected version, and how long a
/// rollback waits for the restored previous version to come back. Shortened
/// under `cfg(test)` so the unit tests that drive the rollback state machine
/// with no real daemon do not stall.
#[cfg(not(test))]
const READY_TIMEOUT: Duration = Duration::from_secs(90);
#[cfg(test)]
const READY_TIMEOUT: Duration = Duration::from_millis(300);

/// What one update run concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// Links were switched to the new version and the health check passed.
    Updated,
    /// The feed already offers the installed version; nothing was touched.
    UpToDate,
    /// The install completed, but the daemon stays stopped until the user
    /// runs `hiero migrate` (database upgrade required).
    MigrationPending,
}

impl UpdateOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            UpdateOutcome::Updated => "updated",
            UpdateOutcome::UpToDate => "up-to-date",
            UpdateOutcome::MigrationPending => "migration-pending",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// A gate refused the update before anything was modified.
    #[error("update refused (nothing was changed): {0}")]
    Refused(String),
    /// The update failed. If the failure was after the link switch, the ordered
    /// rollback ran and was verified — the previous version is restored. If it
    /// failed before any mutation (a pre-switch `stop` that could not stop the
    /// running service), nothing on disk was changed. The `message` is precise
    /// about which happened; the steps recorded before the failure are carried
    /// for the CLI to report.
    #[error("update failed: {message}")]
    Failed { message: String, steps: Vec<String> },
    /// The update failed AND the rollback that followed also failed. Both
    /// causes are carried; NOTHING was deleted, so the candidate and the
    /// previous version directories are still on disk for a human to recover
    /// from. The message never claims the previous version was restored,
    /// because it was not.
    #[error(
        "update failed ({original}) and the rollback did not complete ({rollback}); \
         the previous version was NOT fully restored — no artifacts were deleted, \
         recover manually"
    )]
    FailedAndRollbackFailed {
        original: String,
        rollback: String,
        steps: Vec<String>,
    },
    /// The release feed is missing or malformed.
    #[error("release source error: {0}")]
    Source(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

impl UpdateError {
    /// Exit-code contract: refusals are 2, applied-but-failed (rolled back)
    /// and i/o failures are 1.
    pub fn exit_code(&self) -> u8 {
        match self {
            UpdateError::Refused(_) | UpdateError::Source(_) => 2,
            UpdateError::Failed { .. }
            | UpdateError::FailedAndRollbackFailed { .. }
            | UpdateError::Io(_) => 1,
        }
    }

    /// The step trace recorded before the failure, when the variant carries
    /// one (for the CLI to print).
    pub fn steps(&self) -> &[String] {
        match self {
            UpdateError::Failed { steps, .. }
            | UpdateError::FailedAndRollbackFailed { steps, .. } => steps,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone)]
pub struct UpdateOptions {
    pub release_dir: PathBuf,
    /// The managed application root; derived from the running binary when
    /// omitted (developer builds must pass it).
    pub app_dir: Option<PathBuf>,
    pub data_root: Option<PathBuf>,
    /// Service unit directory override (`None` → the systemd user default).
    pub unit_dir: Option<PathBuf>,
}

/// The release a feed directory resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRelease {
    pub version: String,
    pub archive: PathBuf,
    pub sha256: String,
}

/// What the candidate binary reports about itself (`hiero version --json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateIdentity {
    pub version: String,
    pub protocol_revision: String,
    pub supported_schema_version: i64,
}

/// What one update run did, step by step (only counts and paths — never
/// secret or memory text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    pub outcome: UpdateOutcome,
    pub version: String,
    pub previous_version: Option<String>,
    pub daemon_started: bool,
    pub migration_required: bool,
    pub steps: Vec<String>,
}

impl UpdateReport {
    pub fn render_human(&self) -> String {
        let mut text = format!(
            "update ({}, version {}",
            self.outcome.as_str(),
            self.version
        );
        if let Some(previous) = &self.previous_version {
            text.push_str(&format!(", previous {previous}"));
        }
        text.push_str(")\n");
        for step in &self.steps {
            text.push_str(&format!("  {step}\n"));
        }
        text
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "outcome": self.outcome.as_str(),
            "version": self.version,
            "previous_version": self.previous_version,
            "daemon_started": self.daemon_started,
            "migration_required": self.migration_required,
            "steps": self.steps,
        })
    }
}

/// The full update flow. On refusal nothing on disk has changed; on a failure
/// after the switch every fallible step routes through the same ordered
/// rollback state machine, which restores the prior links, unit, and (when the
/// update had stopped one) the previous daemon — or, if the rollback itself
/// fails, leaves every artifact in place and reports both causes without
/// claiming a restoration that did not happen.
pub fn run_update(options: &UpdateOptions) -> Result<UpdateReport, UpdateError> {
    let config = load_config(options.data_root.as_deref());
    let operation = LifecycleOperation::acquire(&config)?;
    run_update_guarded(options, &operation)
}

/// Doctor exit-code contract for a started candidate: `0` is healthy, `1` is an
/// accepted non-semantic warning (degraded), and **everything else** — an
/// unexpected code like `42`, a process killed by a signal (`None`), a
/// negative code — rejects the candidate. `unwrap_or(2)` used to fold a
/// missing code into "unhealthy" but let `Some(42)` fall through every branch
/// to the healthy path (Astra finding 10); this is exhaustive instead.
///
/// `Ok(false)` = healthy, `Ok(true)` = accepted degraded, `Err` = reject.
pub fn accepted_doctor_exit(code: Option<i32>) -> Result<bool, String> {
    match code {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        other => Err(format!("candidate doctor failed: {other:?}")),
    }
}

/// S2 gate: assert the candidate daemon's semantic lane (working memory +
/// real semantic RAG) is armed and answering before an update is allowed to
/// keep the new binary. Reads the candidate's typed `semantic` state from the
/// SAME authenticated `GET /status` payload `poll_until_live` already trusts
/// (ADR 0009 — never a bare TCP connect), and routes it through
/// `daemon::semantic_worker::require_semantic_ready`. A missing semantic
/// surface (an FTS-only build), a `failed` lane, or a payload the gate
/// rejects refuses the update — release/update health cannot count a
/// disarmed lane as ready. Transient `acquiring`/`rebuilding` states are
/// retried within `READY_TIMEOUT`, since a freshly started candidate may
/// still be arming its assets.
///
/// The gate deliberately owns no semantic knowledge of its own — no manifest
/// peek, no index probe, no second opinion. The candidate's `semantic.state`
/// IS the supervised controller's state, and since Task C3 that state is
/// derived once per tick from
/// `daemon::semantic_worker::readiness_from_evidence` over real service
/// evidence (an installed query lane, a verified current generation or an
/// empty corpus, no rebuild in flight). So `ready` here means the same
/// service a `hieronymus_recall` request would reach, and this gate cannot
/// drift away from what requests actually see.
pub fn require_semantic_ready(config: &HieronymusConfig) -> Result<(), String> {
    use crate::daemon::semantic_worker::{RequiredSemanticState, require_semantic_ready as gate};
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        match semantic_state(config) {
            // Transient states retry until the deadline; everything else —
            // ready, failed, or an unusable payload — is the gate's verdict.
            Ok(RequiredSemanticState::Acquiring | RequiredSemanticState::Rebuilding)
                if Instant::now() < deadline =>
            {
                std::thread::sleep(READY_POLL);
            }
            Ok(state) => return gate(&state),
            Err(detail) => return Err(detail),
        }
    }
}

/// The candidate's semantic readiness decoded from its authenticated
/// `/status` payload (`semantic.state`: acquiring/rebuilding/ready/failed,
/// mirroring `rest::status::semantic_payload`). A payload without a
/// `semantic` surface is a disarmed lane, never a ready one.
fn semantic_state(
    config: &HieronymusConfig,
) -> Result<crate::daemon::semantic_worker::RequiredSemanticState, String> {
    use crate::daemon::semantic_worker::RequiredSemanticState;
    match lifecycle::probe(config) {
        lifecycle::DiscoveryHealth::Live { status, .. } => {
            let semantic = status.get("semantic").ok_or_else(|| {
                "the status payload carries no semantic lane; an FTS-only candidate is not ready".to_string()
            })?;
            let detail = semantic
                .get("detail")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("no failure detail reported")
                .to_string();
            match semantic.get("state").and_then(serde_json::Value::as_str) {
                Some("ready") => Ok(RequiredSemanticState::Ready),
                Some("acquiring") => Ok(RequiredSemanticState::Acquiring),
                Some("rebuilding") => Ok(RequiredSemanticState::Rebuilding),
                Some("failed") => Ok(RequiredSemanticState::Failed(detail)),
                other => Err(format!(
                    "the status payload reported an unusable semantic state {other:?}"
                )),
            }
        }
        other => Err(format!(
            "no live authenticated endpoint ({})",
            other.detail()
        )),
    }
}

/// Bootstrap's post-service-start gate uses the same authenticated version
/// and supervised semantic checks as update activation.
pub fn require_release_ready(config: &HieronymusConfig) -> Result<(), String> {
    poll_until_live(config, Some(env!("CARGO_PKG_VERSION")))?;
    require_semantic_ready(config)
}

/// Everything the rollback state machine needs to restore, captured before any
/// mutation. Holds *what* to restore, not *where* — the caller passes the
/// layout and config.
struct RestoreSnapshot {
    /// The version the stable links pointed at before this run (`None` when
    /// there was no prior install).
    previous_version: Option<String>,
    /// The exact service-unit bytes before this run (`None` = no unit).
    unit_before: Option<String>,
    unit_path: PathBuf,
    /// Whether the update stopped a daemon that was running; a rollback
    /// restarts the previous version only in that case.
    daemon_was_running: bool,
    helper_was_running: bool,
    desktop_registration: Option<crate::desktop::installation::Snapshot>,
    /// The version being installed — pruned (with its staging) only after a
    /// rollback is verified, never during one.
    candidate_version: String,
}

/// Update using an existing operation guard, so CLI release acquisition is
/// serialized with activation and rollback in the same critical section.
pub fn run_update_guarded(
    options: &UpdateOptions,
    operation: &LifecycleOperation,
) -> Result<UpdateReport, UpdateError> {
    run_update_guarded_impl(options, None, operation, None)
}

/// Standalone bootstrap and updater share one guarded activation/registration rollback.
pub fn run_desktop_install(
    options: &UpdateOptions,
    no_activate: bool,
) -> Result<UpdateReport, UpdateError> {
    let config = load_config(options.data_root.as_deref());
    let op = LifecycleOperation::acquire(&config)?;
    run_update_guarded_impl(options, None, &op, Some(no_activate))
}

#[cfg(test)]
fn run_update_impl(
    options: &UpdateOptions,
    manager: Option<&dyn ServiceManager>,
) -> Result<UpdateReport, UpdateError> {
    let config = load_config(options.data_root.as_deref());
    let operation = LifecycleOperation::acquire(&config)?;
    run_update_guarded_impl(options, manager, &operation, None)
}

struct LifecycleManager<'a> {
    options: ServiceOptions,
    operation: &'a LifecycleOperation,
}
impl ServiceManager for LifecycleManager<'_> {
    fn stop(&self) -> Result<(), service::ServiceError> {
        lifecycle::stop_guarded(
            &HieronymusConfig::new(&self.options.data_root),
            &self.options,
            self.operation,
        )
        .map(|_| ())
        .map_err(|e| service::ServiceError::Manager(e.to_string()))
    }
    fn start(&self) -> Result<(), service::ServiceError> {
        lifecycle::start_guarded(
            &HieronymusConfig::new(&self.options.data_root),
            &self.options,
            self.operation,
        )
        .map(|_| ())
        .map_err(|e| service::ServiceError::Manager(e.to_string()))
    }
    fn reload(&self) -> Result<(), service::ServiceError> {
        if self.options.unit_path().exists() {
            SystemdManager::new(self.options.clone(), self.operation).reload()
        } else {
            Ok(())
        }
    }
}

fn run_update_guarded_impl(
    options: &UpdateOptions,
    manager_override: Option<&dyn ServiceManager>,
    operation: &LifecycleOperation,
    desktop_install: Option<bool>,
) -> Result<UpdateReport, UpdateError> {
    let config = load_config(options.data_root.as_deref());
    operation.check(&config)?;
    let mut lines: Vec<String> = Vec::new();

    let root = match &options.app_dir {
        Some(directory) => directory.clone(),
        None => AppLayout::detect_from_exe().map_err(UpdateError::Refused)?,
    };
    let layout = AppLayout::new(root);
    let previous_version = layout.current_version();

    let mut service_options = ServiceOptions {
        data_root: config.data_root().to_path_buf(),
        unit_dir: options
            .unit_dir
            .clone()
            .unwrap_or_else(service::default_unit_dir),
        binary: layout.stable_link("hiero"),
        use_manager: true,
    };
    #[cfg(target_os = "linux")]
    if desktop_install == Some(true) && options.unit_dir.is_some() {
        service_options.use_manager = false;
    }
    operation.register_unit(&service_options)?;
    #[cfg(any(windows, target_os = "macos"))]
    operation.bind_native_broker(&service_options)?;
    service::validate_unit_root_guarded(&service_options, operation)
        .map_err(|error| UpdateError::Refused(error.to_string()))?;

    let acquired = if options
        .release_dir
        .join(crate::release_manifest::metadata_name(TARGET_TRIPLE))
        .try_exists()?
    {
        Some(
            crate::release_source::stage_local_pair(
                &options.release_dir,
                &layout.root().join("cache/models"),
            )
            .map_err(UpdateError::Source)?,
        )
    } else {
        None
    };
    let release_directory = acquired
        .as_ref()
        .map_or(options.release_dir.as_path(), |t| t.path());
    let release = resolve_release(release_directory)?;
    lines.push(format!(
        "release resolved: {} (archive {})",
        release.version,
        release.archive.display()
    ));

    let actual = sha256_file(&release.archive)?;
    if actual != release.sha256 {
        return Err(UpdateError::Refused(format!(
            "archive checksum mismatch: expected {}, got {actual}",
            release.sha256
        )));
    }
    lines.push("archive checksum verified".to_string());

    if acquired.is_none() {
        crate::release_source::inspect_archive(&release.archive).map_err(UpdateError::Refused)?;
    }

    if previous_version.as_deref() == Some(release.version.as_str()) {
        if acquired.is_some() {
            let pair =
                crate::release_archive::verify_split_directory(release_directory, TARGET_TRIPLE)
                    .map_err(UpdateError::Source)?;
            let installed = layout.version_dir(&release.version);
            if sha256_file(&installed.join("assets.json"))? != pair.assets_sha256 {
                return Err(UpdateError::Refused("immutable installed version has different asset identity; use a distinct release version".into()));
            }
            validate_payload(&installed).map_err(UpdateError::Refused)?;
        }
        lines.push("already up to date; nothing was changed".to_string());
        return Ok(UpdateReport {
            outcome: UpdateOutcome::UpToDate,
            version: release.version,
            previous_version,
            daemon_started: false,
            migration_required: false,
            steps: lines,
        });
    }
    if let Some(previous) = &previous_version
        && compare_versions(&release.version, previous) == std::cmp::Ordering::Less
    {
        return Err(UpdateError::Refused(format!(
            "candidate version {} is older than the installed {previous}; \
             downgrade is unsupported",
            release.version
        )));
    }

    // Stage alongside the current version; never in-place. Every gate below
    // runs against the staged copy, and any refusal discards it first.
    std::fs::create_dir_all(layout.versions_dir())?;
    let staging = layout
        .versions_dir()
        .join(format!(".staging-{}", release.version));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    if acquired.is_none() {
        std::fs::create_dir_all(&staging)?;
    }
    let refused = |message: String| {
        let _ = std::fs::remove_dir_all(&staging);
        UpdateError::Refused(message)
    };

    if let Err(error) = if acquired.is_some() {
        crate::release_archive::extract_split_directory(release_directory, TARGET_TRIPLE, &staging)
            .map(|_| ())
            .map_err(UpdateError::Source)
    } else {
        extract_archive(&release.archive, &staging)
    } {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(reason) = validate_payload(&staging) {
        return Err(refused(reason));
    }

    let candidate =
        match candidate_identity(&staging.join(crate::platform::install::executable_name("hiero")))
        {
            Ok(identity) => identity,
            Err(error) => return Err(refused(error.to_string())),
        };
    if candidate.version != release.version {
        return Err(refused(format!(
            "archive binary reports version {} but the release metadata says {}",
            candidate.version, release.version
        )));
    }
    if let Ok(helper) = crate::desktop::launch::sibling_binary(
        &staging.join(crate::platform::install::executable_name("hiero")),
        "hiero-desktop",
    ) {
        let output = Command::new(&helper)
            .args(["version", "--json"])
            .output()
            .map_err(|e| {
                refused(format!(
                    "matching helper/native GUI prerequisites unavailable: {e}"
                ))
            })?;
        let identity: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| refused(format!("helper identity is unavailable: {e}")))?;
        if !output.status.success()
            || identity.get("version").and_then(|v| v.as_str()) != Some(candidate.version.as_str())
            || identity.get("target").and_then(|v| v.as_str()) != Some(TARGET_TRIPLE)
        {
            return Err(refused(
                "helper version/target differs from candidate CLI".into(),
            ));
        }
    }
    if candidate.protocol_revision != PROTOCOL_REVISION {
        return Err(refused(format!(
            "candidate serves MCP protocol {} but this binary serves {PROTOCOL_REVISION}; \
             a protocol change needs an explicit bootstrap install, not an update",
            candidate.protocol_revision
        )));
    }
    lines.push(format!(
        "candidate compatibility: protocol {}, schema {}",
        candidate.protocol_revision, candidate.supported_schema_version
    ));

    if desktop_install.is_some() {
        crate::desktop::launch::sibling_binary(
            &staging.join(crate::platform::install::executable_name("hiero")),
            "hiero-desktop",
        )
        .map_err(&refused)?;
    }
    // Refuse incompatible releases before stopping a live service. Recheck
    // after ownership acquisition to close the preflight/mutation race.
    schema_gate(&config, &candidate).map_err(|error| refused(error.to_string()))?;

    let unit_installed = service_options.unit_path().exists();
    // An injected manager (tests) is by definition engaged; in production the
    // systemd user manager is engaged only for a default-location unit with
    // `systemctl` on PATH.

    // The exact unit content before this run: rollback restores it verbatim
    // instead of guessing what a pre-existing unit pointed at.
    let unit_before = if unit_installed {
        std::fs::read_to_string(service_options.unit_path()).ok()
    } else {
        None
    };

    let daemon_was_running = lifecycle::checked_probe(&config)
        .map_err(|error| refused(error.to_string()))?
        .is_live();
    #[cfg(target_os = "linux")]
    if manager_override.is_none()
        && !service::manager_enabled(&service_options)
        && (daemon_was_running || (desktop_install == Some(false) && previous_version.is_none()))
    {
        return Err(refused("This registration has no managed restart capability. Stop the daemon first and use --no-activate for a fresh custom-unit installation; the running installation was left untouched".into()));
    }
    if desktop_install == Some(true) && daemon_was_running {
        return Err(refused(
            "--no-activate cannot take over a running installation; stop it first".into(),
        ));
    }

    // The lifecycle and registration claims are already held. An apparently
    // offline root can still have an undiscovered owner: refuse before helper
    // retirement or native snapshot mutations, and retain this same guard until
    // activation. A live daemon instead follows the existing snapshot/stop path.
    let offline_owner = if daemon_was_running {
        None
    } else {
        Some(RootOwnership::acquire(&config, "update").map_err(|e| refused(e.to_string()))?)
    };

    // Snapshot the restore target before ANY mutation, and pick the service
    // manager. Every rollback below drives this one `&dyn ServiceManager`, so
    // tests can assert the exact call sequence.
    let systemd_manager = LifecycleManager {
        options: service_options.clone(),
        operation,
    };
    let manager: &dyn ServiceManager = manager_override.unwrap_or(&systemd_manager);
    if layout.version_dir(&release.version).try_exists()? {
        return Err(refused("immutable candidate version already exists; inspect the interrupted installation before retrying".into()));
    }
    if previous_version.as_ref().is_some_and(|v| {
        crate::desktop::launch::sibling_binary(
            &layout
                .version_dir(v)
                .join(crate::platform::install::executable_name("hiero")),
            "hiero-desktop",
        )
        .is_ok()
    }) {
        crate::desktop::launch::sibling_binary(
            &staging.join(crate::platform::install::executable_name("hiero")),
            "hiero-desktop",
        )
        .map_err(&refused)?;
    }
    // Capture authoritative manager state before retirement or stop can change it.
    // Native ordinary updates need the same rollback journal as bootstrap.
    let registration = if desktop_install.is_some() || cfg!(any(windows, target_os = "macos")) {
        Some(
            crate::desktop::installation::Snapshot::capture(&service_options, operation)
                .map_err(&refused)?,
        )
    } else {
        None
    };
    let retirement_result = crate::desktop::control::Retirement::begin_owned(
        &config,
        desktop_install != Some(true),
        Some(&layout.stable_link("hiero")),
    );
    let mut retirement = match retirement_result {
        Ok(value) => value,
        Err(error) => {
            if let Some(registration) = &registration {
                registration.commit(operation).map_err(&refused)?;
            }
            return Err(refused(error.to_string()));
        }
    };
    let snapshot = RestoreSnapshot {
        previous_version: previous_version.clone(),
        unit_before,
        unit_path: service_options.unit_path(),
        daemon_was_running,
        candidate_version: release.version.clone(),
        helper_was_running: retirement.was_running,
        desktop_registration: registration,
    };

    // Stop can partially suppress native recovery even if it fails. Restore
    // through the same snapshot/ownership protocol; never assume no mutation.
    if lifecycle::checked_probe(&config)
        .map_err(|e| refused(e.to_string()))?
        .is_live()
    {
        if let Err(error) = manager.stop() {
            retirement.allow_launch();
            return Err(activation_failed(
                manager,
                &layout,
                &config,
                &snapshot,
                &lines,
                format!("could not stop the running service: {error}"),
                operation,
            ));
        }
        lines.push("running daemon stopped".to_string());
    }

    // Discovery can fail while a daemon or offline writer still owns the
    // root. Only the OS lock admits mutation, including a migration-pending
    // install. Hold it until handing the root to the managed candidate.
    let prepared = (|| -> Result<_, String> {
        let ownership = match offline_owner {
            Some(owner) => owner,
            None => RootOwnership::acquire(&config, "update").map_err(|e| e.to_string())?,
        };
        let migration_required = schema_gate(&config, &candidate).map_err(|e| e.to_string())?;
        Ok((ownership, migration_required))
    })();
    let (owner, migration_required) = match prepared {
        Ok(value) => value,
        Err(cause) => {
            retirement.allow_launch();
            return Err(activation_failed(
                manager, &layout, &config, &snapshot, &lines, cause, operation,
            ));
        }
    };
    let mut ownership = Some(owner);
    let version_dir = layout.version_dir(&release.version);

    /// What the post-switch region concluded.
    enum PostSwitch {
        Activated {
            daemon_started: bool,
            degraded: bool,
        },
        MigrationPending,
    }

    // Everything from the link switch to a kept update is post-switch: one
    // fallible closure whose ONLY `Err` exit funnels through the rollback state
    // machine. A maintainer adding a step here cannot bypass rollback with a
    // stray `?` — "every post-switch failure rolls back" is a property of this
    // control flow, not of reviewer diligence.
    let outcome = (|| -> Result<PostSwitch, String> {
        // Promote the staged directory and switch the stable links.
        (|| -> std::io::Result<()> {
            if version_dir.try_exists()? { return Err(std::io::Error::other("immutable candidate version already exists; inspect the interrupted attempt before retrying")); }
            std::fs::rename(&staging, &version_dir)?;
            layout.switch_stable_links(&release.version)
        })()
        .map_err(|error| format!("install/link switch failed ({error})"))?;
        lines.push(format!(
            "installed into {} and switched the stable links",
            version_dir.display()
        ));

        // The unit execs an absolute path: re-render it so a start launches
        // the new binary, never the old one.
        if unit_installed {
            service_options.binary = layout.stable_link("hiero");
            service::install_guarded(&service_options, operation)
                .map_err(|error| format!("service unit update failed ({error})"))?;
            lines.push("service unit updated to the new binary".to_string());
        }

        if desktop_install.is_some()
            && let Some(registration) = &snapshot.desktop_registration
        {
            registration.install(operation)?;
        }
        if migration_required {
            retirement.allow_launch();
            if snapshot.helper_was_running {
                crate::desktop::control::restart(
                    &config,
                    &layout.stable_link("hiero"),
                    &service_options.unit_dir,
                )
                .map_err(|e| e.to_string())?;
            }
            if let Some(registration) = &snapshot.desktop_registration {
                registration.commit(operation)?;
            }
            return Ok(PostSwitch::MigrationPending);
        }

        let daemon_started = if (daemon_was_running
            || (desktop_install == Some(false) && previous_version.is_none()))
            && !crate::desktop::control::quit_requested(config.data_root())
                .map_err(|e| e.to_string())?
        {
            drop(ownership.take());
            manager
                .start()
                .map_err(|error| format!("service start failed ({error})"))?;
            lines.push("daemon started through the service manager".to_string());
            true
        } else {
            lines.push(format!(
                "no manager integration; start the daemon manually with `{hiero} daemon \
                 --data-root {}` (or `hiero service install`)",
                config.data_root().display(),
                hiero = layout.stable_link("hiero").display(),
            ));
            false
        };

        // Health gate: the candidate's non-mutating doctor. A `Command` that
        // will not even spawn (missing/non-executable candidate) is a failure
        // like any other here — it cannot escape this closure as a bare `?`.
        match service::check_unit_guarded(
            &service_options,
            &version_dir.join(crate::platform::install::executable_name("hiero")),
            operation,
        ) {
            service::UnitVerdict::Consistent => {}
            service::UnitVerdict::Absent if desktop_install.is_none() && !unit_installed => {}
            verdict => {
                return Err(format!(
                    "guarded native registration health failed: {verdict:?}"
                ));
            }
        }
        #[cfg(any(windows, target_os = "macos"))]
        crate::desktop::installation::validate_native(&service_options, operation)?;
        let doctor_output =
            Command::new(version_dir.join(crate::platform::install::executable_name("hiero")))
                .arg("doctor")
                .args(["--skip-registration", "--json"])
                .arg("--unit-dir")
                .arg(&service_options.unit_dir)
                .arg("--data-root")
                .arg(config.data_root())
                .output()
                .map_err(|error| {
                    format!("health check failed (could not run the candidate's doctor: {error})")
                })?;
        let degraded = accepted_doctor_exit(doctor_output.status.code())
            .map_err(|reason| format!("health check failed ({reason})"))?;
        let doctor_report: serde_json::Value = serde_json::from_slice(&doctor_output.stdout)
            .map_err(|error| format!("candidate doctor report is invalid: {error}"))?;
        require_independent_doctor_scope(&doctor_report)?;

        // Doctor exit 0/1 is necessary but never sufficient. A started
        // candidate must publish a live, authenticated endpoint that reports
        // the version we just installed (ADR 0009 — never a bare TCP connect
        // or a diagnostic warning); a degraded (exit 1) candidate with no
        // started daemon to authenticate is not activated.
        if daemon_started {
            poll_until_live(&config, Some(&release.version))
                .map_err(|reason| format!("candidate readiness check failed ({reason})"))?;
            lines.push(
                "candidate published a live, authenticated endpoint at the new version".to_string(),
            );
            // S2/C3 seam: the semantic lane must be armed and answering.
            // `require_semantic_ready` consumes the candidate controller's
            // own evidence-derived state — the same one requests see — so a
            // candidate with an uninstalled query lane, a stale generation,
            // or a cancelled first rebuild cannot be activated.
            require_semantic_ready(&config)
                .map_err(|reason| format!("candidate semantic lane not ready ({reason})"))?;
        } else if degraded {
            require_offline_doctor(&version_dir, &doctor_report)?;
            lines.push("candidate installed offline; native assets verified, authenticated daemon readiness deferred until explicit Start".into());
        }
        retirement.allow_launch();
        if snapshot.helper_was_running {
            crate::desktop::control::restart(
                &config,
                &layout.stable_link("hiero"),
                &service_options.unit_dir,
            )
            .map_err(|e| format!("replacement helper startup failed: {e}"))?;
        }

        if let Some(registration) = &snapshot.desktop_registration {
            registration.commit(operation)?;
        }
        Ok(PostSwitch::Activated {
            daemon_started,
            degraded,
        })
    })();

    let (daemon_started, degraded) = match outcome {
        Err(cause) => {
            // Rollback reacquires ownership after stopping the candidate.
            drop(ownership.take());
            retirement.allow_launch();
            return Err(activation_failed(
                manager, &layout, &config, &snapshot, &lines, cause, operation,
            ));
        }
        Ok(PostSwitch::MigrationPending) => {
            let hiero = layout.stable_link("hiero");
            lines.push(
                "database upgrade required: installation completed but the daemon stays \
                 stopped"
                    .to_string(),
            );
            lines.push(format!(
                "run `{} migrate --data-root {}` (it reports and backs up everything), \
                 then `{} service start`",
                hiero.display(),
                config.data_root().display(),
                hiero.display()
            ));
            return Ok(UpdateReport {
                outcome: UpdateOutcome::MigrationPending,
                version: release.version,
                previous_version,
                daemon_started: false,
                migration_required: true,
                steps: lines,
            });
        }
        Ok(PostSwitch::Activated {
            daemon_started,
            degraded,
        }) => (daemon_started, degraded),
    };

    lines.push(if degraded && !daemon_started { "offline installation verified; configuration initialization and daemon readiness are deferred until explicit Start".into() } else if degraded {
        "health check: degraded (doctor warnings) but the candidate confirmed ready; \
         update kept"
            .to_string()
    } else {
        "health check: healthy".to_string()
    });
    lines.push(
        "agent integrations keep using the stable hiero/hieronymus links, which now \
         serve the new version; host configuration was not modified"
            .to_string(),
    );

    Ok(UpdateReport {
        outcome: UpdateOutcome::Updated,
        version: release.version,
        previous_version,
        daemon_started,
        migration_required: false,
        steps: lines,
    })
}

/// The schema-compatibility gate (database-upgrade spec): the candidate must
/// support what is on disk. The comparison is against the ACTUAL on-disk
/// schema version, not a label — a candidate that supports a newer schema than
/// the database carries is a routine upgrade (install completes, the daemon
/// stays stopped until `hiero migrate`), while a candidate that supports less
/// than the disk holds is refused outright. A `NewerSchema` verdict is refused
/// unconditionally: this binary cannot validate a schema it does not know, so
/// it will not hand the root to another binary on the strength of a version
/// number it cannot interpret.
///
/// Returns whether a migration is required before the daemon may start;
/// refusals abort the update before any change.
fn schema_gate(
    config: &HieronymusConfig,
    candidate: &CandidateIdentity,
) -> Result<bool, UpdateError> {
    let state = hieronymus::db::classify_database(&config.database_path());
    match &state {
        hieronymus::db::DatabaseState::Empty => Ok(false),
        hieronymus::db::DatabaseState::RustSchema { version } => {
            if *version > candidate.supported_schema_version {
                Err(UpdateError::Refused(format!(
                    "database is at Rust schema {version} but the candidate supports only \
                     {}; the updater never launches an older binary against a newer schema",
                    candidate.supported_schema_version
                )))
            } else {
                Ok(*version < candidate.supported_schema_version)
            }
        }
        hieronymus::db::DatabaseState::NewerSchema { version } => {
            Err(UpdateError::Refused(format!(
                "database was written by a newer binary (schema {version}); the updater \
             never launches an older binary against a newer schema"
            )))
        }
        hieronymus::db::DatabaseState::PythonSchema => Ok(true),
        other => Err(UpdateError::Refused(format!(
            "database state '{}' is not compatible with an update; run `hiero doctor`",
            other.as_str()
        ))),
    }
}

/// Run the ordered rollback state machine and turn its result into the right
/// error. On a verified rollback the previous version is restored and the
/// candidate (plus any staging leftover) is pruned; on a rollback that itself
/// failed BOTH causes are carried and nothing is deleted, and the message
/// never claims a restoration that did not happen.
fn activation_failed(
    manager: &dyn ServiceManager,
    layout: &AppLayout,
    config: &HieronymusConfig,
    snapshot: &RestoreSnapshot,
    steps: &[String],
    cause: String,
    operation: &LifecycleOperation,
) -> UpdateError {
    match rollback(manager, layout, config, snapshot, operation) {
        Ok(()) => {
            // Only now — after the rollback verified — may the candidate go.
            prune_candidate(layout, &snapshot.candidate_version);
            let restored = snapshot
                .previous_version
                .as_deref()
                .unwrap_or("no previous version");
            UpdateError::Failed {
                message: format!("{cause}; rolled back — restored {restored}"),
                steps: steps.to_vec(),
            }
        }
        Err(rollback_error) => UpdateError::FailedAndRollbackFailed {
            original: cause,
            rollback: rollback_error,
            steps: steps.to_vec(),
        },
    }
}

/// Registration authority used by the outer rollback state machine.
trait RollbackRegistration {
    fn restore(&self, operation: &LifecycleOperation) -> Result<(), String>;
    fn restores_manager_state(&self) -> bool;
    fn commit(&self, operation: &LifecycleOperation) -> Result<(), String>;
}
impl RollbackRegistration for crate::desktop::installation::Snapshot {
    fn restore(&self, operation: &LifecycleOperation) -> Result<(), String> {
        self.restore(operation)
    }
    fn restores_manager_state(&self) -> bool {
        cfg!(any(windows, target_os = "macos"))
    }
    fn commit(&self, operation: &LifecycleOperation) -> Result<(), String> {
        self.commit(operation)
    }
}

/// The ordered rollback: stop the candidate, restore the links and unit,
/// reload file-based managers, and — only when the update had stopped a running
/// daemon — restart the previous version and confirm it is authentically
/// back. Each step propagates its error as a `String`; NO artifact is deleted
/// here, so a rollback that fails midway leaves everything on disk.
fn rollback(
    manager: &dyn ServiceManager,
    layout: &AppLayout,
    config: &HieronymusConfig,
    snapshot: &RestoreSnapshot,
    operation: &LifecycleOperation,
) -> Result<(), String> {
    rollback_with_registration(
        manager,
        layout,
        config,
        snapshot,
        operation,
        snapshot
            .desktop_registration
            .as_ref()
            .map(|registration| registration as &dyn RollbackRegistration),
    )
}

/// One outer rollback composition, with registration restoration substitutable
/// independently from the native manager for state-transition fault tests.
fn rollback_with_registration(
    manager: &dyn ServiceManager,
    layout: &AppLayout,
    config: &HieronymusConfig,
    snapshot: &RestoreSnapshot,
    operation: &LifecycleOperation,
    registration: Option<&dyn RollbackRegistration>,
) -> Result<(), String> {
    operation.check(config).map_err(|error| error.to_string())?;
    let service_options = ServiceOptions {
        data_root: config.data_root().to_path_buf(),
        unit_dir: snapshot
            .unit_path
            .parent()
            .ok_or("unit path has no parent")?
            .to_path_buf(),
        binary: layout.stable_link("hiero"),
        use_manager: true,
    };
    operation
        .register_unit(&service_options)
        .map_err(|error| error.to_string())?;
    service::validate_unit_root_guarded(&service_options, operation)
        .map_err(|error| error.to_string())?;
    let mut retirement = crate::desktop::control::Retirement::begin_owned(
        config,
        true,
        Some(&layout.stable_link("hiero")),
    )
    .map_err(|e| e.to_string())?;
    manager
        .stop()
        .map_err(|error| format!("stop the candidate: {error}"))?;
    let ownership = RootOwnership::acquire(config, "update-rollback")
        .map_err(|error| format!("acquire data-root ownership for rollback: {error}"))?;
    restore_links_and_unit(layout, snapshot)?;
    if let Some(registration) = registration {
        registration.restore(operation)?;
    }
    // Native package restore already reconciles and validates manager state.
    // A generic native reload may bootstrap an originally unloaded agent.
    // Linux restores files only, so its manager must still reload those files.
    if !registration.is_some_and(|registration| registration.restores_manager_state()) {
        manager
            .reload()
            .map_err(|error| format!("reload the service manager: {error}"))?;
    }
    if snapshot.daemon_was_running
        && !crate::desktop::control::quit_requested(config.data_root())
            .map_err(|e| e.to_string())?
    {
        drop(ownership);
        manager
            .start()
            .map_err(|error| format!("restart the previous version: {error}"))?;
        // Confirm the previous version is genuinely back — and that it is the
        // PREVIOUS version, not the candidate still winding down after a `stop`
        // that returned before its process exited.
        poll_until_live(config, snapshot.previous_version.as_deref())
            .map_err(|detail| format!("restarted the previous version but {detail}"))?;
    }
    retirement.allow_launch();
    if snapshot.helper_was_running {
        crate::desktop::control::restart(
            config,
            &layout.stable_link("hiero"),
            &service_options.unit_dir,
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(registration) = registration {
        registration.commit(operation)?;
    }
    Ok(())
}

/// Restore the stable links and the service unit to their pre-run state,
/// verbatim. Honest about the achieved state: it verifies where EVERY stable
/// link points rather than trusting the switch operation, so a promote that
/// failed on its very first link (state already correct) is a success, and a
/// switch that leaves any of the four links pointing at the wrong version is a
/// reported failure — never a silent `Ok` that then lets the candidate be
/// pruned out from under a dangling link.
fn restore_links_and_unit(layout: &AppLayout, snapshot: &RestoreSnapshot) -> Result<(), String> {
    match &snapshot.previous_version {
        Some(previous) => {
            if all_links_point_at(layout, previous).is_err() {
                let switched = layout.switch_stable_links(previous);
                if let Err(mismatch) = all_links_point_at(layout, previous) {
                    return Err(format!(
                        "restore links to {previous}: {}",
                        switched
                            .err()
                            .map(|error| error.to_string())
                            .unwrap_or(mismatch)
                    ));
                }
            }
        }
        None => {
            crate::platform::install::clear_selection(layout)
                .map_err(|error| format!("clear selection: {error}"))?;
        }
    }
    if cfg!(any(windows, target_os = "macos")) && snapshot.desktop_registration.is_some() {
        return Ok(()); // Native snapshot restores coherent record + manager state.
    }
    match &snapshot.unit_before {
        Some(content) => hieronymus::atomic::atomic_write_text(&snapshot.unit_path, content)
            .map_err(|error| format!("restore service unit: {error}"))?,
        None => remove_if_present(&snapshot.unit_path)
            .map_err(|error| format!("remove service unit: {error}"))?,
    }
    Ok(())
}

/// Whether every stable command link (`switch_stable_links` renames the four
/// one at a time) resolves to `../versions/<version>/<name>`.
fn all_links_point_at(layout: &AppLayout, version: &str) -> Result<(), String> {
    crate::platform::install::verify_selection(layout, version)
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Poll `lifecycle::probe` until it reports a live, authenticated endpoint —
/// and, when `expected_version` is given, one whose `GET /status` payload
/// serves exactly that version. A bare TCP connect, a diagnostic warning, or a
/// daemon lingering at the wrong version is never accepted. Bounded by
/// `READY_TIMEOUT`. Shared by the post-activation readiness gate (expects the
/// candidate's version) and rollback verification (expects the previous
/// version).
fn poll_until_live(
    config: &HieronymusConfig,
    expected_version: Option<&str>,
) -> Result<(), String> {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        let detail = match lifecycle::probe(config) {
            DiscoveryHealth::Live { status, .. } => match expected_version {
                None => return Ok(()),
                Some(expected) => {
                    let live_version = status
                        .get("version")
                        .and_then(|value| value.as_str())
                        .unwrap_or("<absent>");
                    if live_version == expected {
                        return Ok(());
                    }
                    format!(
                        "the endpoint is live but serves version {live_version:?}, \
                         not the expected {expected}"
                    )
                }
            },
            other => other.detail(),
        };
        if Instant::now() >= deadline {
            return Err(format!(
                "no live authenticated endpoint{} within {READY_TIMEOUT:?} ({detail})",
                expected_version
                    .map(|version| format!(" at version {version}"))
                    .unwrap_or_default()
            ));
        }
        std::thread::sleep(READY_POLL);
    }
}

/// Remove a rolled-back candidate and any staging leftover. Called only after
/// a rollback has been verified.
fn prune_candidate(layout: &AppLayout, candidate_version: &str) {
    let _ = std::fs::remove_dir_all(layout.version_dir(candidate_version));
    let _ = std::fs::remove_dir_all(
        layout
            .versions_dir()
            .join(format!(".staging-{candidate_version}")),
    );
}

/// Resolve the newest release from a feed directory: either a `release.json`
/// or exactly one `hieronymus-<version>-<target>.tar.gz` with its `.sha256`.
pub fn resolve_release(directory: &Path) -> Result<ResolvedRelease, UpdateError> {
    if !directory.is_dir() {
        return Err(UpdateError::Source(format!(
            "release directory does not exist: {}",
            directory.display()
        )));
    }
    if directory
        .join(crate::release_manifest::metadata_name(TARGET_TRIPLE))
        .try_exists()?
    {
        let pair = crate::release_archive::verify_split_directory(directory, TARGET_TRIPLE)
            .map_err(UpdateError::Source)?;
        return Ok(ResolvedRelease {
            version: pair.manifest.version,
            archive: pair.platform_archive,
            sha256: pair.manifest.platform.sha256,
        });
    }
    let metadata_path = directory.join("release.json");
    if metadata_path.exists() {
        let text = std::fs::read_to_string(&metadata_path)?;
        let payload =
            crate::release_source::parse_metadata(text.as_bytes()).map_err(UpdateError::Source)?;
        if payload
            .get("signature")
            .map(|signature| !signature.is_null())
            .unwrap_or(false)
        {
            return Err(UpdateError::Refused(
                "release metadata carries a signature, but signature verification is \
                 not configured in the waived first release line; refusing to trust it"
                    .to_string(),
            ));
        }
        crate::release_source::validate_metadata(&payload).map_err(UpdateError::Source)?;
        let version = string_field(&payload, "version")?;
        let archive_name = string_field(&payload, "archive")?;
        let sha256 = normalize_sha256(&string_field(&payload, "sha256")?)?;
        let archive = directory.join(&archive_name);
        if !archive.is_file() {
            return Err(UpdateError::Source(format!(
                "release.json points at {} which does not exist",
                archive.display()
            )));
        }
        return Ok(ResolvedRelease {
            version,
            archive,
            sha256,
        });
    }
    // No metadata: discover exactly one archive in the release-build layout.
    let mut found: Option<ResolvedRelease> = None;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix("hieronymus-") else {
            continue;
        };
        let Some(version) = rest.strip_suffix(&format!("-{TARGET_TRIPLE}.tar.gz")) else {
            continue;
        };
        if version.is_empty() || version.contains('/') {
            continue;
        }
        let archive = entry.path();
        let checksum_path = entry.path().with_file_name(format!("{name}.sha256"));
        if !checksum_path.is_file() {
            return Err(UpdateError::Source(format!(
                "archive {} has no .sha256 sibling",
                archive.display()
            )));
        }
        let checksum_text = std::fs::read_to_string(&checksum_path)?;
        let sha256 =
            normalize_sha256(checksum_text.split_whitespace().next().ok_or_else(|| {
                UpdateError::Source(format!("{} is empty", checksum_path.display()))
            })?)?;
        crate::release_source::validate_metadata(&serde_json::json!({
            "version": version, "archive": name, "sha256": sha256,
        }))
        .map_err(UpdateError::Source)?;
        if found.is_some() {
            return Err(UpdateError::Source(
                "release directory holds several archives and no release.json; \
                 add release.json or keep one archive"
                    .to_string(),
            ));
        }
        found = Some(ResolvedRelease {
            version: version.to_string(),
            archive,
            sha256,
        });
    }
    found.ok_or_else(|| {
        UpdateError::Source(format!(
            "no hieronymus-<version>-{TARGET_TRIPLE}.tar.gz in {} and no release.json",
            directory.display()
        ))
    })
}

fn string_field(payload: &serde_json::Value, field: &str) -> Result<String, UpdateError> {
    payload
        .get(field)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            UpdateError::Source(format!(
                "release.json field {field:?} is missing or not a string"
            ))
        })
}

fn normalize_sha256(value: &str) -> Result<String, UpdateError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(normalized);
    }
    Err(UpdateError::Source(format!(
        "{value:?} is not a SHA-256 hex digest"
    )))
}

/// SHA-256 of a file, streamed in chunks.
pub fn sha256_file(path: &Path) -> Result<String, UpdateError> {
    use sha2::Digest;
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut digest = sha2::Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Extract through the shared typed archive verifier, before activation.
fn extract_archive(archive: &Path, destination: &Path) -> Result<(), UpdateError> {
    crate::release_source::extract_archive(archive, destination).map_err(failed)
}

fn failed(message: impl Into<String>) -> UpdateError {
    UpdateError::Failed {
        message: message.into(),
        steps: Vec::new(),
    }
}

/// The staged payload must look exactly like the release layout: an
/// executable `hiero` plus the three relative argv[0] links.
fn require_independent_doctor_scope(report: &serde_json::Value) -> Result<(), String> {
    if report.get("scope").and_then(|v| v.as_str()) != Some("payload-config-without-registration") {
        return Err("candidate doctor did not report the required independent scope".into());
    }
    Ok(())
}
fn require_offline_doctor(version: &Path, report: &serde_json::Value) -> Result<(), String> {
    if !version.join("assets.json").is_file() {
        return Err("offline doctor warnings require a complete verified semantic payload".into());
    }
    let findings = report
        .get("findings")
        .and_then(|v| v.as_array())
        .ok_or("doctor findings missing")?;
    if findings
        .iter()
        .any(|f| match f.get("level").and_then(|v| v.as_str()) {
            Some("ok") => false,
            Some("warning") => !matches!(
                f.get("code").and_then(|v| v.as_str()),
                Some("config-root-missing")
            ),
            _ => true,
        })
    {
        return Err(
            "offline doctor reported warnings beyond an uninitialized configuration root".into(),
        );
    }
    Ok(())
}

fn validate_payload(staging: &Path) -> Result<(), String> {
    let binary = staging.join(crate::platform::install::executable_name("hiero"));
    if !binary.is_file() {
        return Err(format!(
            "staged release has no hiero binary: {}",
            binary.display()
        ));
    }
    #[cfg(unix)]
    for name in LINK_NAMES.iter().skip(1) {
        let link = staging.join(name);
        match std::fs::read_link(&link) {
            Ok(target) if target == std::path::Path::new("hiero") => {}
            _ => {
                return Err(format!(
                    "staged release link {name} does not point at hiero"
                ));
            }
        }
    }
    #[cfg(windows)]
    if !staging.join("hiero-launcher.exe").is_file() {
        return Err("staged release lacks native launcher".into());
    }
    if staging.join("assets.json").exists() || cfg!(feature = "console-embed") {
        let output = Command::new(&binary)
            .arg("release-assets")
            .arg("--output")
            .arg(staging)
            .output()
            .map_err(|e| format!("native asset verification failed: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "native asset verification failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let expected: serde_json::Value = serde_json::from_slice(
            &std::fs::read(staging.join("assets.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let actual: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
        if actual != expected {
            return Err("bundled asset metadata mismatch".into());
        }
    }

    Ok(())
}

/// Ask the candidate binary what it supports (`hiero version --json`): a
/// read-only probe that never executes downloaded content before the
/// checksum verified it.
pub fn candidate_identity(binary: &Path) -> Result<CandidateIdentity, String> {
    let output = Command::new(binary)
        .arg("version")
        .arg("--json")
        .output()
        .map_err(|error| format!("could not run the candidate: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "candidate version probe failed (exit {})",
            output.status
        ));
    }
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("candidate version output: {error}"))?;
    let version = payload
        .get("version")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "candidate reports no version".to_string())?
        .to_string();
    let protocol_revision = payload
        .get("protocol_revision")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "candidate reports no protocol_revision".to_string())?
        .to_string();
    let supported_schema_version = payload
        .get("supported_schema_version")
        .and_then(|value| value.as_i64())
        .ok_or_else(|| "candidate reports no supported_schema_version".to_string())?;
    Ok(CandidateIdentity {
        version,
        protocol_revision,
        supported_schema_version,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn write_release(payload: &serde_json::Value) -> String {
        serde_json::to_string(payload).unwrap()
    }

    #[test]
    fn release_resolution_reads_release_json_and_refuses_signatures() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("release.json"),
            write_release(&serde_json::json!({
                "version": "1.2.3",
                "archive": "hieronymus-1.2.3-x86_64-unknown-linux-gnu.tar.gz",
                "sha256": format!("{:0>64}", "ab"),
                "signature": serde_json::Value::Null,
            })),
        )
        .unwrap();
        std::fs::write(
            temp.path()
                .join("hieronymus-1.2.3-x86_64-unknown-linux-gnu.tar.gz"),
            b"archive",
        )
        .unwrap();

        let release = resolve_release(temp.path()).unwrap();
        assert_eq!(release.version, "1.2.3");
        assert_eq!(release.sha256, format!("{:0>64}", "ab"));

        // A signature in the metadata is a hard refusal (fail closed).
        std::fs::write(
            temp.path().join("release.json"),
            write_release(&serde_json::json!({
                "version": "1.2.3",
                "archive": "hieronymus-1.2.3-x86_64-unknown-linux-gnu.tar.gz",
                "sha256": format!("{:0>64}", "ab"),
                "signature": "MEUCIQ=",
            })),
        )
        .unwrap();
        let error = resolve_release(temp.path()).unwrap_err();
        assert!(error.to_string().contains("signature"), "{error}");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn release_resolution_discovers_the_release_build_layout() {
        let temp = tempfile::tempdir().unwrap();
        let name = "hieronymus-0.7.0-x86_64-unknown-linux-gnu.tar.gz";
        std::fs::write(temp.path().join(name), b"archive").unwrap();
        std::fs::write(
            temp.path().join(format!("{name}.sha256")),
            format!("{:0>64}  {name}\n", "cd"),
        )
        .unwrap();

        let release = resolve_release(temp.path()).unwrap();
        assert_eq!(release.version, "0.7.0");
        assert_eq!(release.sha256, format!("{:0>64}", "cd"));

        // Two archives without metadata is ambiguous.
        let second = "hieronymus-0.8.0-x86_64-unknown-linux-gnu.tar.gz";
        std::fs::write(temp.path().join(second), b"archive").unwrap();
        std::fs::write(
            temp.path().join(format!("{second}.sha256")),
            format!("{:0>64}  {second}\n", "ee"),
        )
        .unwrap();
        let error = resolve_release(temp.path()).unwrap_err();
        assert!(error.to_string().contains("several archives"), "{error}");
    }

    #[test]
    fn schema_gate_refuses_newer_schemas_and_flags_upgrades() {
        let temp = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(temp.path());
        let candidate = CandidateIdentity {
            version: "9.9.9".to_string(),
            protocol_revision: PROTOCOL_REVISION.to_string(),
            supported_schema_version: 1,
        };

        // Empty: fine.
        assert!(!schema_gate(&config, &candidate).unwrap());

        // Newer schema: hard refusal — never launch an older binary.
        let connection = rusqlite::Connection::open(config.database_path()).unwrap();
        connection
            .execute_batch(
                "create table hieronymus_meta (schema_version integer not null unique);
                 insert into hieronymus_meta values (2);",
            )
            .unwrap();
        drop(connection);
        let error = schema_gate(&config, &candidate).unwrap_err();
        assert!(error.to_string().contains("newer schema"), "{error}");
        assert_eq!(error.exit_code(), 2);
    }

    /// A data root whose database carries `version` as its Rust schema marker.
    fn root_at_schema_version(version: i64) -> (tempfile::TempDir, HieronymusConfig) {
        let temp = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(temp.path());
        let connection = rusqlite::Connection::open(config.database_path()).unwrap();
        connection
            .execute_batch(&format!(
                "create table hieronymus_meta (schema_version integer not null unique);
                 insert into hieronymus_meta values ({version});"
            ))
            .unwrap();
        drop(connection);
        (temp, config)
    }

    fn candidate_supporting(version: i64) -> CandidateIdentity {
        CandidateIdentity {
            version: "9.9.9".to_string(),
            protocol_revision: PROTOCOL_REVISION.to_string(),
            supported_schema_version: version,
        }
    }

    /// The gate compares the candidate against the ACTUAL on-disk version, in
    /// both directions.
    #[test]
    fn schema_gate_is_directional_about_the_on_disk_version() {
        let current = hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION;

        // Candidate supports the current schema, the disk is one behind: a
        // routine upgrade. The install completes and `hiero migrate` is
        // reported — never a refusal.
        let (_temp, config) = root_at_schema_version(current - 1);
        assert!(
            schema_gate(&config, &candidate_supporting(current)).unwrap(),
            "an older database with a newer candidate must require migration"
        );

        // Candidate supports only the older schema, the disk is current:
        // refused outright. The updater never launches an older binary against
        // a newer schema.
        let (_temp, config) = root_at_schema_version(current);
        let error = schema_gate(&config, &candidate_supporting(current - 1)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("database is at Rust schema {current}")),
            "{error}"
        );
        assert_eq!(error.exit_code(), 2);

        // Same version on both sides: nothing to migrate.
        assert!(!schema_gate(&config, &candidate_supporting(current)).unwrap());
    }

    #[test]
    fn schema_gate_refuses_a_database_written_by_a_newer_binary() {
        let current = hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION;
        let (_temp, config) = root_at_schema_version(current + 1);
        // Even a candidate that claims to support it: this binary cannot
        // validate a schema it does not know, so it fails closed.
        let error = schema_gate(&config, &candidate_supporting(current + 1)).unwrap_err();
        assert!(
            error.to_string().contains("written by a newer binary"),
            "{error}"
        );
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn schema_gate_treats_the_python_schema_as_an_explicit_upgrade() {
        let temp = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(temp.path());
        let connection = rusqlite::Connection::open(config.database_path()).unwrap();
        for table in [
            "series",
            "task_sessions",
            "short_term_memories",
            "strict_terms",
        ] {
            connection
                .execute(
                    &format!("create table {table} (id integer primary key)"),
                    [],
                )
                .unwrap();
        }
        drop(connection);
        let candidate = CandidateIdentity {
            version: "9.9.9".to_string(),
            protocol_revision: PROTOCOL_REVISION.to_string(),
            supported_schema_version: 1,
        };
        assert!(schema_gate(&config, &candidate).unwrap());
    }

    // -----------------------------------------------------------------------
    // Doctor exit-code contract (Astra finding 10)
    // -----------------------------------------------------------------------

    #[test]
    fn scoped_candidate_health_rejects_full_scope_and_service_warnings() {
        assert!(
            require_independent_doctor_scope(
                &serde_json::json!({"scope":"full","status":"healthy"})
            )
            .is_err()
        );
        let version = tempfile::tempdir().unwrap();
        std::fs::write(version.path().join("assets.json"), b"verified separately").unwrap();
        for code in ["service-unit-broken", "semantic-model-invalid"] {
            let report = serde_json::json!({"scope":"payload-config-without-registration","findings":[{"level":"warning","code":code}]});
            require_independent_doctor_scope(&report).unwrap();
            assert!(require_offline_doctor(version.path(), &report).is_err());
        }
        assert!(
            require_offline_doctor(
                version.path(),
                &serde_json::json!({"findings":[{"level":"warning","code":"config-root-missing"}]})
            )
            .is_ok()
        );
    }

    #[test]
    fn unexpected_doctor_exit_is_not_healthy() {
        assert!(super::accepted_doctor_exit(Some(42)).is_err());
        assert!(super::accepted_doctor_exit(None).is_err());
        assert!(super::accepted_doctor_exit(Some(-1)).is_err());
        assert!(
            !super::accepted_doctor_exit(Some(0)).unwrap(),
            "exit 0 is healthy"
        );
        assert!(
            super::accepted_doctor_exit(Some(1)).unwrap(),
            "exit 1 is degraded"
        );
    }

    // -----------------------------------------------------------------------
    // Rollback state machine, with a scripted fake ServiceManager
    // -----------------------------------------------------------------------

    #[derive(Default)]
    struct FakeManager {
        calls: std::cell::RefCell<Vec<&'static str>>,
        fail_on: Option<&'static str>,
    }

    impl FakeManager {
        fn failing(call: &'static str) -> Self {
            Self {
                fail_on: Some(call),
                ..Self::default()
            }
        }
        fn calls(&self) -> Vec<&'static str> {
            self.calls.borrow().clone()
        }
        fn record(&self, call: &'static str) -> Result<(), crate::service::ServiceError> {
            self.calls.borrow_mut().push(call);
            if self.fail_on == Some(call) {
                return Err(crate::service::ServiceError::Manager(format!(
                    "scripted {call} failure"
                )));
            }
            Ok(())
        }
    }

    impl crate::service::ServiceManager for FakeManager {
        fn stop(&self) -> Result<(), crate::service::ServiceError> {
            self.record("stop")
        }
        fn reload(&self) -> Result<(), crate::service::ServiceError> {
            self.record("reload")
        }
        fn start(&self) -> Result<(), crate::service::ServiceError> {
            self.record("start")
        }
    }

    /// A managed app layout with two installed versions, links at `current`.
    fn seeded_layout(temp: &Path, versions: &[&str], current: &str) -> AppLayout {
        let app = temp.join("app");
        let layout = AppLayout::new(&app);
        for version in versions {
            let dir = layout.version_dir(version);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("hiero"), b"binary").unwrap();
            for name in LINK_NAMES.iter().skip(1) {
                std::os::unix::fs::symlink("hiero", dir.join(name)).unwrap();
            }
        }
        layout.switch_stable_links(current).unwrap();
        layout
    }

    fn snapshot_for(
        unit_path: PathBuf,
        previous: Option<&str>,
        was_running: bool,
    ) -> RestoreSnapshot {
        RestoreSnapshot {
            previous_version: previous.map(str::to_string),
            unit_before: None,
            unit_path,
            daemon_was_running: was_running,
            candidate_version: "2.0.0".to_string(),
            helper_was_running: false,
            desktop_registration: None,
        }
    }

    #[test]
    fn outer_native_rollback_preserves_unloaded_enabled_state_through_commit() {
        use std::cell::{Cell, RefCell};
        struct NativeState {
            loaded: Cell<bool>,
            enabled: Cell<bool>,
            journal_pending: Cell<bool>,
            events: RefCell<Vec<&'static str>>,
        }
        impl ServiceManager for NativeState {
            fn stop(&self) -> Result<(), service::ServiceError> {
                self.events.borrow_mut().push("stop");
                self.loaded.set(false);
                Ok(())
            }
            fn reload(&self) -> Result<(), service::ServiceError> {
                self.events.borrow_mut().push("reload");
                // macOS Install bootstraps enabled, unloaded Desktop agents.
                self.loaded.set(self.enabled.get());
                Ok(())
            }
            fn start(&self) -> Result<(), service::ServiceError> {
                self.events.borrow_mut().push("start");
                Err(service::ServiceError::Manager("unexpected startup".into()))
            }
        }
        impl RollbackRegistration for NativeState {
            fn restore(&self, _: &LifecycleOperation) -> Result<(), String> {
                self.events.borrow_mut().push("restore");
                self.loaded.set(false);
                self.enabled.set(true);
                Ok(())
            }
            fn restores_manager_state(&self) -> bool {
                true
            }
            fn commit(&self, _: &LifecycleOperation) -> Result<(), String> {
                self.events.borrow_mut().push("commit");
                if self.loaded.get() || !self.enabled.get() {
                    return Err("outer rollback changed restored unloaded/enabled state".into());
                }
                self.journal_pending.set(false);
                Ok(())
            }
        }
        for quit in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let layout = seeded_layout(temp.path(), &["1.0.0", "2.0.0"], "2.0.0");
            let config = HieronymusConfig::new(temp.path().join("data"));
            let operation = LifecycleOperation::acquire(&config).unwrap();
            if quit {
                crate::desktop::control::record_quit(config.data_root()).unwrap();
            }
            let snapshot = snapshot_for(
                temp.path().join("units/hieronymus.service"),
                Some("1.0.0"),
                quit,
            );
            let native = NativeState {
                loaded: Cell::new(true),
                enabled: Cell::new(false),
                journal_pending: Cell::new(true),
                events: RefCell::new(Vec::new()),
            };
            rollback_with_registration(
                &native,
                &layout,
                &config,
                &snapshot,
                &operation,
                Some(&native),
            )
            .unwrap();
            assert_eq!(*native.events.borrow(), ["stop", "restore", "commit"]);
            assert!(!native.loaded.get());
            assert!(native.enabled.get());
            assert!(!native.journal_pending.get());
            assert_eq!(layout.current_version().as_deref(), Some("1.0.0"));
        }
    }

    #[test]
    fn rollback_stops_then_restores_then_reloads_and_preserves_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let layout = seeded_layout(temp.path(), &["1.0.0", "2.0.0"], "2.0.0");
        let config = HieronymusConfig::new(temp.path().join("data"));
        let manager = FakeManager::default();
        let snapshot = snapshot_for(
            temp.path().join("units/hieronymus.service"),
            Some("1.0.0"),
            false,
        );

        rollback(
            &manager,
            &layout,
            &config,
            &snapshot,
            &LifecycleOperation::acquire(&config).unwrap(),
        )
        .unwrap();

        assert_eq!(manager.calls(), vec!["stop", "reload"]);
        assert_eq!(layout.current_version().as_deref(), Some("1.0.0"));
        // Neither the candidate nor the previous version dir was deleted.
        assert!(layout.version_dir("2.0.0").join("hiero").exists());
        assert!(layout.version_dir("1.0.0").join("hiero").exists());
    }

    #[test]
    fn rollback_restarts_the_previous_only_when_a_daemon_was_running() {
        let temp = tempfile::tempdir().unwrap();
        let layout = seeded_layout(temp.path(), &["1.0.0", "2.0.0"], "2.0.0");
        let config = HieronymusConfig::new(temp.path().join("data"));
        let manager = FakeManager::default();
        let snapshot = snapshot_for(
            temp.path().join("units/hieronymus.service"),
            Some("1.0.0"),
            true,
        );

        // No real daemon, so `poll_until_live` times out (short under
        // cfg(test)) and the rollback reports itself as incomplete.
        let error = rollback(
            &manager,
            &layout,
            &config,
            &snapshot,
            &LifecycleOperation::acquire(&config).unwrap(),
        )
        .unwrap_err();
        assert_eq!(manager.calls(), vec!["stop", "reload", "start"]);
        assert!(
            error.contains("restarted the previous version but")
                && error.contains("no live authenticated endpoint at version 1.0.0"),
            "{error}"
        );
    }

    #[test]
    fn a_failed_rollback_step_surfaces_both_causes_and_deletes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let layout = seeded_layout(temp.path(), &["1.0.0", "2.0.0"], "2.0.0");
        let config = HieronymusConfig::new(temp.path().join("data"));
        let manager = FakeManager::failing("reload");
        let snapshot = snapshot_for(
            temp.path().join("units/hieronymus.service"),
            Some("1.0.0"),
            false,
        );

        let error = activation_failed(
            &manager,
            &layout,
            &config,
            &snapshot,
            &["step one".to_string()],
            "health check failed (candidate doctor failed: Some(42))".to_string(),
            &LifecycleOperation::acquire(&config).unwrap(),
        );
        assert_eq!(error.exit_code(), 1);
        match &error {
            UpdateError::FailedAndRollbackFailed {
                original,
                rollback,
                steps,
            } => {
                assert!(original.contains("Some(42)"), "{original}");
                assert!(rollback.contains("reload"), "{rollback}");
                assert_eq!(steps, &vec!["step one".to_string()]);
            }
            other => panic!("expected FailedAndRollbackFailed, got {other:?}"),
        }
        // The message must not claim a restoration that did not happen.
        let rendered = error.to_string();
        assert!(!rendered.contains("rolled back — restored"), "{rendered}");
        assert!(rendered.contains("NOT fully restored"), "{rendered}");
        // The candidate was NOT pruned because the rollback did not verify.
        assert!(layout.version_dir("2.0.0").join("hiero").exists());
        assert!(layout.version_dir("1.0.0").join("hiero").exists());
    }

    #[test]
    fn a_verified_rollback_prunes_the_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let layout = seeded_layout(temp.path(), &["1.0.0", "2.0.0"], "2.0.0");
        let config = HieronymusConfig::new(temp.path().join("data"));
        let manager = FakeManager::default();
        let snapshot = snapshot_for(
            temp.path().join("units/hieronymus.service"),
            Some("1.0.0"),
            false,
        );

        let error = activation_failed(
            &manager,
            &layout,
            &config,
            &snapshot,
            &[],
            "boom".to_string(),
            &LifecycleOperation::acquire(&config).unwrap(),
        );
        assert!(matches!(error, UpdateError::Failed { .. }));
        assert!(
            error.to_string().contains("rolled back — restored 1.0.0"),
            "{error}"
        );
        assert!(
            !layout.version_dir("2.0.0").exists(),
            "candidate pruned after verified rollback"
        );
        assert!(layout.version_dir("1.0.0").join("hiero").exists());
    }

    #[test]
    fn require_semantic_ready_rejects_without_a_live_semantic_lane() {
        let temp = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(temp.path());
        // No daemon at all: nothing authenticated proves a semantic lane, so
        // the gate refuses (never a bare "assume ready" default).
        let error = super::require_semantic_ready(&config).unwrap_err();
        assert!(error.contains("no live authenticated endpoint"), "{error}");
    }

    // -----------------------------------------------------------------------
    // Full `run_update_impl` flow driving a fake ServiceManager so the
    // started-candidate branch (readiness probe, semantic seam, degraded
    // acceptance) is exercised — no test hits it via the real systemd path.
    // -----------------------------------------------------------------------

    use std::io::{Read as _, Write as _};

    fn fake_hiero(path: &Path, version: &str, doctor_exit: i32) {
        let json = format!(
            "{{\"version\":\"{version}\",\"protocol_revision\":\"{PROTOCOL_REVISION}\",\
             \"supported_schema_version\":{}}}",
            hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION
        );
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n  version) printf '%s\\n' '{json}' ;;\n  \
             release-assets) printf '%s\\n' '{{}}' ;;\n  doctor) printf '%s\\n' '{{\"scope\":\"payload-config-without-registration\",\"findings\":[]}}'; exit {doctor_exit} ;;\n  *) exit 0 ;;\nesac\n"
        );
        std::fs::write(path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Install a prior version, build a release archive for `candidate`, and
    /// return options wired to a tempdir unit-dir with a unit file present.
    fn staged_update(
        temp: &Path,
        prior: &str,
        candidate: &str,
        doctor_exit: i32,
    ) -> (UpdateOptions, AppLayout) {
        let app = temp.join("app");
        let layout = AppLayout::new(&app);
        let prior_dir = layout.version_dir(prior);
        std::fs::create_dir_all(&prior_dir).unwrap();
        fake_hiero(&prior_dir.join("hiero"), prior, 0);
        for name in LINK_NAMES.iter().skip(1) {
            std::os::unix::fs::symlink("hiero", prior_dir.join(name)).unwrap();
        }
        layout.switch_stable_links(prior).unwrap();

        let payload = temp.join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        fake_hiero(&payload.join("hiero"), candidate, doctor_exit);
        // Fixture candidate returns this manifest; native inference is covered by the installed package test.
        std::fs::write(payload.join("assets.json"), "{}\n").unwrap();
        for name in LINK_NAMES.iter().skip(1) {
            std::os::unix::fs::symlink("hiero", payload.join(name)).unwrap();
        }
        let release_dir = temp.join("release");
        std::fs::create_dir_all(&release_dir).unwrap();
        let archive_name = format!("hieronymus-{candidate}-{TARGET_TRIPLE}.tar.gz");
        let archive = release_dir.join(&archive_name);
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&payload)
            .args([
                "assets.json",
                "hiero",
                "hieronymus",
                "hieronymus-agent-hook",
                "hieronymus-mcp",
            ])
            .status()
            .unwrap();
        assert!(status.success(), "tar failed");
        let digest = sha256_file(&archive).unwrap();
        std::fs::write(
            release_dir.join(format!("{archive_name}.sha256")),
            format!("{digest}  {archive_name}\n"),
        )
        .unwrap();

        let data_root = temp.join("data");
        std::fs::create_dir_all(&data_root).unwrap();
        let unit_dir = temp.join("units");
        std::fs::create_dir_all(&unit_dir).unwrap();
        std::fs::write(
            unit_dir.join("hieronymus.service"),
            format!(
                "[Service]\nExecStart=\"{}\" daemon --data-root \"{}\"\n",
                prior_dir.join("hiero").display(),
                data_root.display()
            ),
        )
        .unwrap();

        (
            UpdateOptions {
                release_dir,
                app_dir: Some(app),
                data_root: Some(data_root),
                unit_dir: Some(unit_dir),
            },
            layout,
        )
    }

    /// A minimal live daemon for `lifecycle::probe`: an HTTP thread that answers
    /// `GET /status` with the given identity and semantic lane state, plus the
    /// discovery record and token the probe needs. The thread outlives the
    /// test (blocked on `accept`), matching the pattern in
    /// `agent_hook`/`runtime_shutdown` tests.
    fn fake_live_daemon(
        config: &HieronymusConfig,
        instance_id: &str,
        version: &str,
        semantic_state: &str,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = format!(
            "{{\"instance_id\":\"{instance_id}\",\"protocol_revision\":\"{PROTOCOL_REVISION}\",\
             \"version\":\"{version}\",\"semantic\":{{\"state\":\"{semantic_state}\",\
             \"detail\":null}}}}"
        );
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let mut scratch = [0_u8; 2048];
                let _ = stream.read(&mut scratch);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        discovery::write_token(config, &discovery::generate_bearer_token().unwrap()).unwrap();
        discovery::write_discovery(
            config,
            &discovery::DiscoveryRecord {
                discovery_version: discovery::DISCOVERY_VERSION,
                protocol_version: PROTOCOL_REVISION.to_string(),
                host: "127.0.0.1".to_string(),
                port,
                pid: std::process::id(),
                instance_id: instance_id.to_string(),
                started_at: "2026-09-05T00:00:00+00:00".to_string(),
            },
        )
        .unwrap();
    }

    #[test]
    fn update_refuses_a_different_roots_registration_before_staging_or_manager_actions() {
        let temp = tempfile::tempdir().unwrap();
        let (mut options, layout) = staged_update(temp.path(), "1.0.0", "9.9.0", 0);
        options.data_root = Some(temp.path().join("other-data"));
        let manager = FakeManager::default();
        let error = run_update_impl(&options, Some(&manager)).unwrap_err();
        assert!(error.to_string().contains("unit serves data root"));
        assert!(manager.calls().is_empty());
        assert!(all_links_point_at(&layout, "1.0.0").is_ok());
        assert!(!layout.versions_dir().join(".staging-9.9.0").exists());
    }

    #[test]
    fn undiscovered_owner_refuses_before_retirement_or_manager_actions() {
        let temp = tempfile::tempdir().unwrap();
        let (options, layout) = staged_update(temp.path(), "1.0.0", "9.9.0", 0);
        let config = hieronymus::data_root::load_config(options.data_root.as_deref());
        let daemon = crate::daemon::Daemon::start(&crate::daemon::DaemonOptions {
            data_root: options.data_root.clone(),
            port: 0,
            ..Default::default()
        })
        .unwrap();
        std::fs::remove_file(config.daemon_discovery_path()).unwrap();
        assert!(!lifecycle::checked_probe(&config).unwrap().is_live());
        let manager = FakeManager::default();
        let error = run_update_impl(&options, Some(&manager)).unwrap_err();
        assert!(
            manager.calls().is_empty(),
            "refusal called manager: {:?}",
            manager.calls()
        );
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("owns this data root"));
        assert!(
            !config
                .data_root()
                .join(crate::desktop::control::LAUNCH_GATE)
                .exists()
        );
        assert!(all_links_point_at(&layout, "1.0.0").is_ok());
        assert!(!layout.version_dir("9.9.0").exists());
        assert!(!layout.versions_dir().join(".staging-9.9.0").exists());
        assert!(RootOwnership::acquire(&config, "test").is_err());
        daemon.shutdown().unwrap();
    }

    #[test]
    fn update_keeps_a_registered_but_stopped_daemon_stopped() {
        let temp = tempfile::tempdir().unwrap();
        let (options, layout) = staged_update(temp.path(), "1.0.0", "9.9.0", 0);
        let manager = FakeManager::default();
        let report = run_update_impl(&options, Some(&manager)).unwrap();
        assert!(!report.daemon_started);
        assert!(manager.calls().is_empty());
        assert!(all_links_point_at(&layout, "9.9.0").is_ok());
        assert!(layout.version_dir("1.0.0").exists());
    }

    #[test]
    fn running_custom_unit_installation_refuses_before_stopping_or_switching() {
        let temp = tempfile::tempdir().unwrap();
        let (options, layout) = staged_update(temp.path(), "0.9.0", "9.9.0", 0);
        let config = hieronymus::data_root::load_config(options.data_root.as_deref());
        fake_live_daemon(&config, &"ac".repeat(16), "0.9.0", "ready");
        let error = run_update_impl(&options, None).unwrap_err();
        assert!(
            error.to_string().contains("no managed restart capability"),
            "{error}"
        );
        assert_eq!(error.exit_code(), 2);
        assert!(all_links_point_at(&layout, "0.9.0").is_ok());
        assert!(lifecycle::checked_probe(&config).unwrap().is_live());
        assert!(!layout.version_dir("9.9.0").exists());
    }

    #[test]
    fn started_candidate_serving_the_wrong_version_is_rolled_back() {
        let temp = tempfile::tempdir().unwrap();
        let (options, layout) = staged_update(temp.path(), "0.9.0", "9.9.0", 0);
        let config = hieronymus::data_root::load_config(options.data_root.as_deref());
        // A live endpoint that authenticates but still serves the OLD version:
        // the candidate is not proved ready, and the rollback's own readiness
        // check (expecting 0.9.0) then passes against it.
        fake_live_daemon(&config, &"ab".repeat(16), "0.9.0", "ready");
        let manager = FakeManager::default();

        let error = run_update_impl(&options, Some(&manager)).unwrap_err();

        assert!(
            error.to_string().contains("serves version \"0.9.0\"")
                && error.to_string().contains("not the expected 9.9.0"),
            "{error}"
        );
        assert!(
            error.to_string().contains("rolled back — restored 0.9.0"),
            "{error}"
        );
        assert!(all_links_point_at(&layout, "0.9.0").is_ok());
        assert!(!layout.version_dir("9.9.0").exists(), "candidate pruned");
    }

    #[test]
    fn started_candidate_with_a_failed_semantic_lane_is_rolled_back() {
        let temp = tempfile::tempdir().unwrap();
        let (options, layout) = staged_update(temp.path(), "0.9.0", "9.9.0", 0);
        let config = hieronymus::data_root::load_config(options.data_root.as_deref());
        // The candidate publishes a live, authenticated endpoint at the
        // expected version, but its semantic lane reports `failed` (the
        // FTS-only surface): doctor exit 0 and version match are NOT enough —
        // a disarmed lane is never counted as ready (coordinator R4<->S2).
        fake_live_daemon(&config, &"ef".repeat(16), "9.9.0", "failed");
        let manager = FakeManager::default();

        let error = run_update_impl(&options, Some(&manager)).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("candidate semantic lane not ready"),
            "{error}"
        );
        // The fake endpoint keeps serving the candidate version, so the
        // rollback's own readiness verification (expecting 0.9.0) cannot
        // complete — both causes surface and nothing is deleted.
        assert!(matches!(error, UpdateError::FailedAndRollbackFailed { .. }));
        assert_eq!(error.exit_code(), 1);
        assert!(all_links_point_at(&layout, "0.9.0").is_ok());
        assert!(
            layout.version_dir("9.9.0").join("hiero").exists(),
            "candidate preserved for manual recovery"
        );
        // Pre-stop of the fake-running daemon, candidate start, then the
        // rollback sequence (stop, reload, start; its verification poll
        // fails against the still-live candidate endpoint).
        assert_eq!(
            manager.calls(),
            vec!["stop", "start", "stop", "reload", "start"]
        );
    }

    /// Every non-ready semantic verdict the C3 controller can publish refuses
    /// activation, and the two transient ones are *polled* rather than
    /// hard-failed on sight: a freshly started candidate is allowed to finish
    /// arming or indexing within `READY_TIMEOUT` before the gate rules.
    #[test]
    fn transient_semantic_states_are_polled_then_refused() {
        for (state, expected) in [
            ("acquiring", "semantic assets are still being acquired"),
            ("rebuilding", "semantic indexing is still in progress"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let config = HieronymusConfig::new(temp.path());
            fake_live_daemon(&config, &"12".repeat(16), "9.9.0", state);

            let started = Instant::now();
            let error = super::require_semantic_ready(&config).unwrap_err();
            let waited = started.elapsed();

            assert!(error.contains(expected), "{state}: {error}");
            assert!(
                waited >= READY_TIMEOUT,
                "{state} must be retried for the whole readiness window, waited {waited:?}"
            );
        }
    }

    /// A candidate whose semantic lane is `failed` — since C3 that covers an
    /// uninstalled query lane, an invalidated generation, and a cancelled
    /// first rebuild, not only a missing runtime — is refused with the
    /// controller's own actionable detail.
    #[test]
    fn a_failed_semantic_lane_is_refused_with_its_cause() {
        let temp = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(temp.path());
        fake_live_daemon(&config, &"34".repeat(16), "9.9.0", "failed");

        let error = super::require_semantic_ready(&config).unwrap_err();

        assert!(error.contains("semantic retrieval unavailable"), "{error}");
        assert!(error.contains("no failure detail reported"), "{error}");
    }

    #[test]
    fn degraded_candidate_that_confirms_ready_is_kept() {
        let temp = tempfile::tempdir().unwrap();
        let (options, layout) = staged_update(temp.path(), "0.9.0", "9.9.0", 1);
        let config = hieronymus::data_root::load_config(options.data_root.as_deref());
        // Doctor exit 1, but the candidate publishes a live authenticated
        // endpoint at the expected version — the "accepted degraded" path.
        fake_live_daemon(&config, &"cd".repeat(16), "9.9.0", "ready");
        let manager = FakeManager::default();

        let report = run_update_impl(&options, Some(&manager)).unwrap();

        assert_eq!(report.outcome, UpdateOutcome::Updated);
        assert!(report.daemon_started);
        assert!(
            report
                .steps
                .iter()
                .any(|line| line.contains("degraded") && line.contains("update kept")),
            "{:?}",
            report.steps
        );
        assert!(all_links_point_at(&layout, "9.9.0").is_ok());
        assert!(layout.version_dir("9.9.0").join("hiero").exists());
    }

    #[test]
    fn update_requires_ownership_after_a_manager_claims_to_stop_the_daemon() {
        let temp = tempfile::tempdir().unwrap();
        let (options, layout) = staged_update(temp.path(), "0.9.0", "9.9.0", 0);
        let daemon = crate::daemon::Daemon::start(&crate::daemon::DaemonOptions {
            data_root: options.data_root.clone(),
            port: 0,
            ..Default::default()
        })
        .unwrap();
        // This manager acknowledges stop without releasing the daemon's lock.
        let manager = FakeManager::default();
        let error = run_update_impl(&options, Some(&manager)).unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("owns this data root"), "{error}");
        assert_eq!(manager.calls(), vec!["stop", "stop"]);
        assert!(all_links_point_at(&layout, "0.9.0").is_ok());
        assert!(!layout.version_dir("9.9.0").exists());
        daemon.shutdown().unwrap();
    }

    #[test]
    fn rollback_owns_offline_mutations_and_releases_before_managed_restart() {
        struct OwnershipManager<'a>(&'a HieronymusConfig);
        impl ServiceManager for OwnershipManager<'_> {
            fn stop(&self) -> Result<(), service::ServiceError> {
                assert_eq!(
                    LifecycleOperation::acquire(self.0).unwrap_err().kind(),
                    std::io::ErrorKind::WouldBlock
                );
                Ok(())
            }
            fn reload(&self) -> Result<(), service::ServiceError> {
                assert!(RootOwnership::acquire(self.0, "competing-writer").is_err());
                assert_eq!(
                    LifecycleOperation::acquire(self.0).unwrap_err().kind(),
                    std::io::ErrorKind::WouldBlock
                );
                Ok(())
            }
            fn start(&self) -> Result<(), service::ServiceError> {
                assert!(RootOwnership::acquire(self.0, "restarted-daemon").is_ok());
                assert_eq!(
                    LifecycleOperation::acquire(self.0).unwrap_err().kind(),
                    std::io::ErrorKind::WouldBlock
                );
                Ok(())
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let layout = seeded_layout(temp.path(), &["1.0.0", "2.0.0"], "2.0.0");
        let config = HieronymusConfig::new(temp.path().join("data"));
        fake_live_daemon(&config, &"ab".repeat(16), "1.0.0", "ready");
        let snapshot = snapshot_for(
            temp.path().join("units/hieronymus.service"),
            Some("1.0.0"),
            true,
        );
        rollback(
            &OwnershipManager(&config),
            &layout,
            &config,
            &snapshot,
            &LifecycleOperation::acquire(&config).unwrap(),
        )
        .unwrap();
        assert!(all_links_point_at(&layout, "1.0.0").is_ok());
        assert!(RootOwnership::acquire(&config, "next-owner").is_ok());
    }
}
