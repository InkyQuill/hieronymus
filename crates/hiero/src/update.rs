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
//! something it cannot verify. Production feed transport is out of scope
//! here; the directory works over any mount/sync the owner provides.
//!
//! Compatibility gates, in order, all before anything is changed:
//! candidate protocol revision must equal this binary's (a protocol change
//! needs an explicit bootstrap install), the candidate must support the
//! database schema on disk (a newer-schema target is refused — the updater
//! never launches an older binary against a newer schema), and no daemon may
//! be running unless it is the managed service (which the update stops).

use std::path::{Path, PathBuf};
use std::process::Command;

use hieronymus::data_root::{HieronymusConfig, load_config};

use crate::app::{AppLayout, LINK_NAMES, TARGET_TRIPLE, compare_versions};
use crate::daemon::discovery;
use crate::daemon::registry::PROTOCOL_REVISION;
use crate::service::{self, ServiceOptions};

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
    /// The update was applied but failed afterwards; rollback ran. The steps
    /// recorded before the failure are carried for the CLI to report.
    #[error("update failed: {message}")]
    Failed { message: String, steps: Vec<String> },
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
            UpdateError::Failed { .. } | UpdateError::Io(_) => 1,
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

/// The full update flow. On refusal nothing on disk has changed; on a health
/// failure after the switch the prior links (and service unit) are restored.
pub fn run_update(options: &UpdateOptions) -> Result<UpdateReport, UpdateError> {
    let mut lines: Vec<String> = Vec::new();

    let root = match &options.app_dir {
        Some(directory) => directory.clone(),
        None => AppLayout::detect_from_exe().map_err(UpdateError::Refused)?,
    };
    let layout = AppLayout::new(root);
    let previous_version = layout.current_version();

    let release = resolve_release(&options.release_dir)?;
    lines.push(format!(
        "release resolved: {} (archive {})",
        release.version,
        release.archive.display()
    ));

    if previous_version.as_deref() == Some(release.version.as_str()) {
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

    let actual = sha256_file(&release.archive)?;
    if actual != release.sha256 {
        return Err(UpdateError::Refused(format!(
            "archive checksum mismatch: expected {}, got {actual}",
            release.sha256
        )));
    }
    lines.push("archive checksum verified".to_string());

    // Stage alongside the current version; never in-place. Every gate below
    // runs against the staged copy, and any refusal discards it first.
    std::fs::create_dir_all(layout.versions_dir())?;
    let staging = layout
        .versions_dir()
        .join(format!(".staging-{}", release.version));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let refused = |message: String| {
        let _ = std::fs::remove_dir_all(&staging);
        UpdateError::Refused(message)
    };

    if let Err(error) = extract_archive(&release.archive, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(reason) = validate_payload(&staging) {
        return Err(refused(reason));
    }

    let candidate = match candidate_identity(&staging.join("hiero")) {
        Ok(identity) => identity,
        Err(error) => return Err(refused(error.to_string())),
    };
    if candidate.version != release.version {
        return Err(refused(format!(
            "archive binary reports version {} but the release metadata says {}",
            candidate.version, release.version
        )));
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

    let config = load_config(options.data_root.as_deref());
    let migration_required = match schema_gate(&config, &candidate) {
        Ok(migration_required) => migration_required,
        Err(error) => return Err(refused(error.to_string())),
    };

    let mut service_options = ServiceOptions {
        data_root: config.data_root().to_path_buf(),
        unit_dir: options
            .unit_dir
            .clone()
            .unwrap_or_else(service::default_unit_dir),
        binary: layout.stable_link("hiero"),
        use_manager: true,
    };
    let unit_installed = service_options.unit_path().exists();
    let manager_engaged = unit_installed && service::manager_enabled(&service_options);
    // The exact unit content before this run: rollback restores it verbatim
    // instead of guessing what a pre-existing unit pointed at.
    let unit_before = if unit_installed {
        std::fs::read_to_string(service_options.unit_path()).ok()
    } else {
        None
    };

    let daemon_was_running = discovery::daemon_is_active(&config);
    if daemon_was_running && !manager_engaged {
        return Err(refused(
            "a daemon is currently running and the updater cannot stop it (no service \
             unit, or a custom --unit-dir); stop the daemon first"
                .to_string(),
        ));
    }
    if daemon_was_running {
        service::stop(&service_options).map_err(|error| UpdateError::Failed {
            message: format!("could not stop the running service: {error}"),
            steps: lines.clone(),
        })?;
        lines.push("running daemon stopped".to_string());
    }

    // Promote the staged directory and switch the stable links.
    let version_dir = layout.version_dir(&release.version);
    // The daemon is already down here, so an I/O failure in the promote or
    // switch must not strand the user half-switched: it goes through the
    // same rollback machinery as the health gate.
    let promote = (|| -> std::io::Result<()> {
        if version_dir.exists() {
            // A leftover from an interrupted attempt — the up-to-date check
            // above guarantees this is never the currently linked version.
            std::fs::remove_dir_all(&version_dir)?;
        }
        std::fs::rename(&staging, &version_dir)?;
        layout.switch_stable_links(&release.version)
    })();
    if let Err(error) = promote {
        rollback(
            &layout,
            &release.version,
            previous_version.as_deref(),
            &service_options,
            daemon_was_running,
            unit_before.as_deref(),
        );
        return Err(UpdateError::Failed {
            message: format!("install/link switch failed ({error}); rolled back"),
            steps: lines,
        });
    }
    lines.push(format!(
        "installed into {} and switched the stable links",
        version_dir.display()
    ));

    // The unit execs an absolute path: re-render it so a start launches the
    // new binary, never the old one.
    if unit_installed {
        service_options.binary = version_dir.join("hiero");
        if let Err(error) = service::install(&service_options) {
            rollback(
                &layout,
                &release.version,
                previous_version.as_deref(),
                &service_options,
                daemon_was_running,
                unit_before.as_deref(),
            );
            return Err(UpdateError::Failed {
                message: format!("service unit update failed ({error}); rolled back"),
                steps: lines,
            });
        }
        lines.push("service unit updated to the new binary".to_string());
    }

    if migration_required {
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

    let daemon_started = if manager_engaged {
        if let Err(error) = service::start(&service_options) {
            rollback(
                &layout,
                &release.version,
                previous_version.as_deref(),
                &service_options,
                daemon_was_running,
                unit_before.as_deref(),
            );
            return Err(UpdateError::Failed {
                message: format!("service start failed; rolled back: {error}"),
                steps: lines,
            });
        }
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

    // Health gate: the candidate binary's non-mutating doctor. Exit 0/1
    // (healthy/degraded) keeps the update; exit 2 (unhealthy) rolls back.
    let health = Command::new(version_dir.join("hiero"))
        .arg("doctor")
        .arg("--data-root")
        .arg(config.data_root())
        .output()?;
    let health_code = health.status.code().unwrap_or(2);
    if health_code == 2 {
        rollback(
            &layout,
            &release.version,
            previous_version.as_deref(),
            &service_options,
            daemon_was_running,
            unit_before.as_deref(),
        );
        let previous = previous_version.as_deref().unwrap_or("no previous version");
        return Err(UpdateError::Failed {
            message: format!("health check failed (doctor exited 2); restored {previous}"),
            steps: lines,
        });
    }
    lines.push(if health_code == 1 {
        "health check: degraded (doctor warnings); update kept".to_string()
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
/// support what is on disk. Returns whether a migration is required before
/// the daemon may start; refusals abort the update before any change.
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

/// Restore the previous version after a failed apply: links, unit, and (when
/// the update stopped a running daemon) the daemon itself. Best effort — the
/// caller reports the original failure either way.
fn rollback(
    layout: &AppLayout,
    new_version: &str,
    previous_version: Option<&str>,
    service_options: &ServiceOptions,
    daemon_was_running: bool,
    unit_before: Option<&str>,
) {
    match previous_version {
        Some(previous) => {
            let _ = layout.switch_stable_links(previous);
            let restore = ServiceOptions {
                binary: layout.version_dir(previous).join("hiero"),
                ..service_options.clone()
            };
            match unit_before {
                Some(content) => {
                    let _ = hieronymus::atomic::atomic_write_text(
                        &service_options.unit_path(),
                        content,
                    );
                }
                None => {
                    let _ = std::fs::remove_file(service_options.unit_path());
                }
            }
            if daemon_was_running {
                let _ = service::start(&restore);
            }
        }
        None => {
            for name in LINK_NAMES {
                let _ = std::fs::remove_file(layout.stable_link(name));
            }
            match unit_before {
                Some(content) => {
                    let _ = hieronymus::atomic::atomic_write_text(
                        &service_options.unit_path(),
                        content,
                    );
                }
                None => {
                    let _ = std::fs::remove_file(service_options.unit_path());
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(layout.version_dir(new_version));
    // A promote-stage failure leaves the staged copy behind; after a rename
    // this path no longer exists and the removal is a no-op.
    let _ = std::fs::remove_dir_all(
        layout
            .versions_dir()
            .join(format!(".staging-{new_version}")),
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
    let metadata_path = directory.join("release.json");
    if metadata_path.exists() {
        let text = std::fs::read_to_string(&metadata_path)?;
        let payload: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
            UpdateError::Source(format!("release.json is not valid JSON: {error}"))
        })?;
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

/// Extract a `.tar.gz` archive with the system `tar` (the same tool the
/// bootstrap installer and release build use).
fn extract_archive(archive: &Path, destination: &Path) -> Result<(), UpdateError> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .status()
        .map_err(|error| failed(format!("could not run tar: {error}")))?;
    if !status.success() {
        return Err(failed(format!(
            "archive extraction failed (tar exited {status})"
        )));
    }
    Ok(())
}

fn failed(message: impl Into<String>) -> UpdateError {
    UpdateError::Failed {
        message: message.into(),
        steps: Vec::new(),
    }
}

/// The staged payload must look exactly like the release layout: an
/// executable `hiero` plus the three relative argv[0] links.
fn validate_payload(staging: &Path) -> Result<(), String> {
    let binary = staging.join("hiero");
    if !binary.is_file() {
        return Err(format!(
            "staged release has no hiero binary: {}",
            binary.display()
        ));
    }
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

#[cfg(test)]
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
}
