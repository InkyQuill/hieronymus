//! Current-user Aqua LaunchAgents. No hard-stop or kickstart -k is used.
use super::*;
use crate::platform::{
    macos_broker::{self, TaskAction},
    macos_identity,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::Read,
    process::{Command, Stdio},
};
pub const SERVICE_UNIT_NAME: &str = "net.inkyquill.hieronymus.daemon.plist";
pub fn default_unit_dir() -> PathBuf {
    home::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/LaunchAgents")
}
pub fn manager_enabled(options: &ServiceOptions) -> bool {
    options.use_manager
}
pub fn render_unit(binary: &Path, root: &Path) -> Result<String, ServiceError> {
    macos_agent::render(binary, root, &default_unit_dir(), false).map_err(ServiceError::Invalid)
}
pub fn parse_unit(text: &str) -> Result<UnitDefinition, String> {
    let (binary, data_root) = macos_agent::parse(text)?;
    Ok(UnitDefinition { binary, data_root })
}
pub fn read_unit(options: &ServiceOptions) -> Result<Option<UnitDefinition>, String> {
    let operation = LifecycleOperation::acquire(&HieronymusConfig::new(&options.data_root))
        .map_err(|e| e.to_string())?;
    read_unit_guarded(options, &operation)
}
pub(crate) fn read_unit_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Option<UnitDefinition>, String> {
    operation
        .register_unit(options)
        .map_err(|e| e.to_string())?;
    if options.use_manager {
        let result = macos_broker::task(options, TaskAction::Inspect, false)?;
        if result.is_null() {
            return Ok(None);
        }
    }
    read_file(&options.unit_path())?
        .map(|text| {
            if text != definition(options, false)? {
                return Err("LaunchAgent definition is foreign or modified".into());
            }
            parse_unit(&text)
        })
        .transpose()
}
pub(crate) fn validate_unit_root_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<(), ServiceError> {
    operation.register_unit(options)?;
    if let Some(text) = read_file(&options.unit_path()).map_err(ServiceError::Invalid)? {
        let expected = definition(options, false).map_err(ServiceError::Invalid)?;
        if text != expected {
            return Err(ServiceError::Invalid(
                "LaunchAgent is foreign or modified".into(),
            ));
        }
    }
    if options.use_manager {
        read_unit_guarded(options, operation).map_err(ServiceError::Invalid)?;
    }
    Ok(())
}
pub(crate) fn install_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Vec<String>, ServiceError> {
    operation.register_unit(options)?;
    validate_unit_root_guarded(options, operation)?;
    if !options.binary.is_file() {
        return Err(ServiceError::Invalid(
            "Service binary does not exist".into(),
        ));
    }
    if options.use_manager {
        macos_broker::task(options, TaskAction::Install, false).map_err(ServiceError::Manager)?;
    } else {
        hieronymus::atomic::atomic_write_text(
            &options.unit_path(),
            &definition(options, false).map_err(ServiceError::Invalid)?,
        )?;
    }
    Ok(vec![
        "macOS daemon LaunchAgent installed for on-demand startup".into(),
    ])
}
pub(crate) fn uninstall_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Vec<String>, ServiceError> {
    operation.register_unit(options)?;
    validate_unit_root_guarded(options, operation)?;
    if options.use_manager {
        macos_broker::task(options, TaskAction::Remove, false).map_err(ServiceError::Manager)?;
    } else {
        macos_agent::restore_definition(&options.unit_path(), None)
            .map_err(ServiceError::Invalid)?;
    }
    Ok(vec!["macOS daemon LaunchAgent removed".into()])
}
pub(crate) fn start_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Vec<String>, ServiceError> {
    operation.register_unit(options)?;
    if !options.use_manager {
        return Err(ServiceError::Manager(
            "Native manager integration is disabled".into(),
        ));
    }
    macos_broker::task(options, TaskAction::Start, false).map_err(ServiceError::Manager)?;
    Ok(vec!["macOS daemon LaunchAgent started".into()])
}
pub(crate) fn rearm_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<(), ServiceError> {
    operation.register_unit(options)?;
    if options.use_manager {
        macos_broker::task(options, TaskAction::Rearm, false).map_err(ServiceError::Manager)?;
    }
    Ok(())
}
pub(crate) fn stop_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Vec<String>, ServiceError> {
    operation.register_unit(options)?;
    if options.use_manager {
        macos_broker::task(options, TaskAction::Suppress, false).map_err(ServiceError::Manager)?;
    }
    Ok(vec![
        "macOS daemon disabled until explicit Start; tray login preference preserved".into(),
    ])
}
pub(crate) fn owned_login_link(_: &ServiceOptions) -> Result<Option<PathBuf>, ServiceError> {
    Err(ServiceError::Invalid(
        "macOS login uses a separate tray LaunchAgent".into(),
    ))
}
pub(crate) fn disable_login_guarded(
    _: &ServiceOptions,
    _: &LifecycleOperation,
) -> Result<(), ServiceError> {
    Err(ServiceError::Invalid(
        "macOS login uses a separate tray LaunchAgent".into(),
    ))
}
/// Compatibility name for the updater's existing guarded manager interface.
pub struct SystemdManager<'a> {
    options: ServiceOptions,
    operation: &'a LifecycleOperation,
}
impl<'a> SystemdManager<'a> {
    pub fn new(options: ServiceOptions, operation: &'a LifecycleOperation) -> Self {
        Self { options, operation }
    }
}
impl ServiceManager for SystemdManager<'_> {
    fn stop(&self) -> Result<(), ServiceError> {
        if !self.options.use_manager {
            return Ok(());
        }
        crate::lifecycle::stop_guarded(
            &HieronymusConfig::new(&self.options.data_root),
            &self.options,
            self.operation,
        )
        .map(|_| ())
        .map_err(|e| ServiceError::Manager(e.to_string()))
    }
    fn reload(&self) -> Result<(), ServiceError> {
        install_guarded(&self.options, self.operation).map(|_| ())
    }
    fn start(&self) -> Result<(), ServiceError> {
        if !self.options.use_manager {
            return Ok(());
        }
        validate_unit_root_guarded(&self.options, self.operation)?;
        crate::lifecycle::checked_probe(&HieronymusConfig::new(&self.options.data_root))
            .map_err(|e| ServiceError::Manager(e.to_string()))?;
        start_guarded(&self.options, self.operation).map(|_| ())
    }
}
fn definition(options: &ServiceOptions, tray: bool) -> Result<String, String> {
    let root = options
        .data_root
        .canonicalize()
        .map_err(|_| "LaunchAgent root is unavailable")?;
    let directory = options
        .unit_dir
        .canonicalize()
        .map_err(|_| "LaunchAgent directory is unavailable")?;
    macos_agent::render(&options.binary, &root, &directory, tray)
}
fn path(options: &ServiceOptions, tray: bool) -> PathBuf {
    options.unit_dir.join(format!(
        "{}.plist",
        macos_agent::label(&options.data_root, tray)
    ))
}
fn read_file(path: &Path) -> Result<Option<String>, String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not read LaunchAgent state".into()),
    };
    let mut text = String::new();
    file.take(65537)
        .read_to_string(&mut text)
        .map_err(|_| "Could not read LaunchAgent state")?;
    if text.len() > 65536 {
        return Err("LaunchAgent state exceeds its bound".into());
    }
    Ok(Some(text))
}
// Broker owns continuation gates for this entire call, even if launchctl stalls.
// Output is drained concurrently with a hard memory bound; no native diagnostics escape.
fn command(args: &[&str]) -> Result<(std::process::ExitStatus, String), String> {
    let mut child = Command::new("/bin/launchctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start launchctl")?;
    let mut output = child.stdout.take().ok_or("Missing launchctl output")?;
    let reader = std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut overflow = false;
        loop {
            let n = output
                .read(&mut chunk)
                .map_err(|_| "Could not read launchctl output")?;
            if n == 0 {
                break;
            }
            if all.len() + n <= 65536 {
                all.extend_from_slice(&chunk[..n]);
            } else {
                overflow = true;
            }
        }
        if overflow {
            return Err("Launchctl output exceeds its bound");
        }
        String::from_utf8(all).map_err(|_| "Invalid launchctl output")
    });
    let status = child.wait().map_err(|_| "Could not observe launchctl")?;
    let output = reader.join().map_err(|_| "Launchctl reader failed")??;
    Ok((status, output))
}
fn run(args: &[&str]) -> Result<(), String> {
    if command(args)?.0.success() {
        Ok(())
    } else {
        Err("LaunchAgent manager operation failed".into())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    definition: Option<String>,
    disabled: bool,
    loaded: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    before: Snapshot,
    after: Option<String>,
}
struct Agent<'a> {
    options: &'a ServiceOptions,
    tray: bool,
    domain: String,
    label: String,
    path: PathBuf,
    expected: String,
}
impl<'a> Agent<'a> {
    fn new(options: &'a ServiceOptions, tray: bool) -> Result<Self, String> {
        Ok(Self {
            options,
            tray,
            domain: macos_identity::domain()
                .map_err(|_| "No current-user graphical launchd domain")?,
            label: macos_agent::label(&options.data_root, tray),
            path: path(options, tray),
            expected: definition(options, tray)?,
        })
    }
    fn target(&self) -> String {
        format!("{}/{}", self.domain, self.label)
    }
    fn loaded(&self) -> Result<Option<String>, String> {
        let (status, text) = command(&["print", &self.target()])?;
        if status.success() {
            self.validate_loaded(&text)?;
            return Ok(Some(text));
        }
        // ESRCH=3 is the documented underlying missing-service result; current
        // launchctl also reports 113 for an absent service. Domain existence is
        // independently checked so absent GUI/login cannot masquerade as absence.
        if matches!(status.code(), Some(3 | 113)) {
            run(&["print", &self.domain])?;
            Ok(None)
        } else {
            Err("Could not inspect LaunchAgent".into())
        }
    }
    fn validate_loaded(&self, text: &str) -> Result<(), String> {
        macos_agent::validate_loaded(
            text,
            &self.path,
            &macos_agent::arguments(
                &self.options.binary,
                &self.options.data_root,
                &self.options.unit_dir,
                self.tray,
            )?,
        )
    }
    fn disabled(&self) -> Result<bool, String> {
        let (status, text) = command(&["print-disabled", &self.domain])?;
        if !status.success() {
            return Err("Could not read LaunchAgent login state".into());
        }
        macos_agent::disabled_state(&text, &self.label)
    }

    fn toggle(&self, disabled: bool) -> Result<(), String> {
        run(&[if disabled { "disable" } else { "enable" }, &self.target()])?;
        if self.disabled()? != disabled {
            return Err("LaunchAgent enabled-state readback failed".into());
        }
        Ok(())
    }
    fn bootout_idle(&self) -> Result<(), String> {
        if let Some(text) = self.loaded()? {
            if text.lines().any(|l| l.trim().starts_with("pid = ")) {
                return Err(
                    "LaunchAgent still has a process; graceful exit must complete before bootout"
                        .into(),
                );
            }
            run(&["bootout", &self.target()])?;
            if self.loaded()?.is_some() {
                return Err("LaunchAgent remains loaded after bootout".into());
            }
        }
        Ok(())
    }
    fn snapshot(&self) -> Result<Snapshot, String> {
        let definition = read_file(&self.path)?;
        if definition.as_ref().is_some_and(|s| s != &self.expected) {
            return Err("LaunchAgent definition is foreign or modified".into());
        }
        let loaded = self.loaded()?.is_some();
        if loaded && definition.is_none() {
            return Err("Loaded LaunchAgent has no owned definition".into());
        }
        Ok(Snapshot {
            definition,
            disabled: self.disabled()?,
            loaded,
        })
    }
    fn restore(&self, journal: &Journal) -> Result<(), String> {
        for text in [&journal.before.definition, &journal.after]
            .into_iter()
            .flatten()
        {
            if text != &self.expected {
                return Err("Pending LaunchAgent belongs to another installation".into());
            }
        }
        let actual = read_file(&self.path)?;
        if actual != journal.before.definition && actual != journal.after {
            return Err("Pending LaunchAgent conflicts with external changes".into());
        }
        if !journal.before.loaded {
            self.bootout_idle()?;
        }
        macos_agent::restore_definition(
            &self.path,
            journal.before.definition.as_deref().map(str::as_bytes),
        )?;
        self.toggle(journal.before.disabled)?;
        if journal.before.loaded && self.loaded()?.is_none() {
            run(&[
                "bootstrap",
                &self.domain,
                self.path.to_str().ok_or("Invalid LaunchAgent path")?,
            ])?;
        }
        let actual = self.snapshot()?;
        if actual.definition != journal.before.definition
            || actual.disabled != journal.before.disabled
            || actual.loaded != journal.before.loaded
        {
            return Err("LaunchAgent rollback readback failed".into());
        }
        Ok(())
    }
}
/// Called only by the committed broker with both continuation gates held.
pub(crate) fn execute(
    options: &ServiceOptions,
    action: TaskAction,
    tray: bool,
) -> Result<Value, String> {
    let agent = Agent::new(options, tray)?;
    let pending = agent.path.with_extension("pending.json");
    if let Some(text) = read_file(&pending)? {
        let journal: Journal =
            serde_json::from_str(&text).map_err(|_| "Invalid LaunchAgent transaction")?;
        if matches!(action, TaskAction::Inspect) {
            // Passive inspection never repairs registration. It checks the
            // journal's ownership and reports the actual file/native state so a
            // later authenticated mutation can reach recovery under its guards.
            for text in [&journal.before.definition, &journal.after]
                .into_iter()
                .flatten()
            {
                if text != &agent.expected {
                    return Err("Pending LaunchAgent belongs to another installation".into());
                }
            }
            let actual = agent.snapshot()?;
            return Ok(if actual.definition.is_none() {
                Value::Null
            } else {
                json!({"enabled": !actual.disabled,"loaded":actual.loaded,"pending":true})
            });
        }
        agent.restore(&journal)?;
        std::fs::remove_file(&pending)
            .map_err(|_| "Could not clear recovered LaunchAgent transaction")?;
    }
    let before = agent.snapshot()?;
    if matches!(action, TaskAction::Inspect) {
        return Ok(if before.definition.is_none() {
            Value::Null
        } else {
            json!({"enabled":!before.disabled,"loaded":before.loaded})
        });
    }
    if matches!(action, TaskAction::Remove | TaskAction::Suppress) && before.definition.is_none() {
        return Ok(Value::Null);
    }
    if matches!(action, TaskAction::EnableLogin | TaskAction::DisableLogin) && !tray {
        return Err("Login action requires tray LaunchAgent".into());
    }
    let after = if matches!(action, TaskAction::Remove) {
        None
    } else {
        Some(agent.expected.clone())
    };
    let journal = Journal { before, after };
    hieronymus::atomic::atomic_write(
        &pending,
        &serde_json::to_vec(&journal).map_err(|_| "Could not encode LaunchAgent transaction")?,
    )
    .map_err(|_| "Could not persist LaunchAgent transaction")?;
    let result = macos_agent::publish_with_rollback(
        || {
            if matches!(action, TaskAction::Remove) {
                agent.toggle(true)?;
                agent.bootout_idle()?;
                macos_agent::restore_definition(&agent.path, None)?;
                return Ok(Value::Null);
            }
            hieronymus::atomic::atomic_write_text(&agent.path, &agent.expected)
                .map_err(|_| "Could not publish LaunchAgent")?;
            let disabled = match action {
                TaskAction::Suppress | TaskAction::DisableLogin => true,
                TaskAction::EnableLogin | TaskAction::Rearm | TaskAction::Start => false,
                _ => journal.before.disabled,
            };
            agent.toggle(disabled)?;
            // Publishing tray preference never starts a second helper in this login.
            // launchd loads its RunAtLoad plist at the next Aqua login. Daemon is
            // bootstrapped without RunAtLoad and explicitly kickstarted on Start only.
            if !tray && !disabled && agent.loaded()?.is_none() {
                run(&[
                    "bootstrap",
                    &agent.domain,
                    agent.path.to_str().ok_or("Invalid LaunchAgent path")?,
                ])?;
            }
            let actual = agent.snapshot()?;
            if actual.definition != journal.after || actual.disabled != disabled {
                return Err("LaunchAgent registration readback failed".into());
            }
            Ok(json!({"enabled":!actual.disabled,"loaded":actual.loaded}))
        },
        || agent.restore(&journal),
    );
    let value = match result {
        Ok(v) => v,
        Err(error) => {
            if error == "LaunchAgent registration failed; rollback remains pending" {
                return Err(error);
            }
            std::fs::remove_file(&pending)
                .map_err(|_| "LaunchAgent rollback completed but journal remains")?;
            return Err(error);
        }
    };
    std::fs::remove_file(&pending)
        .map_err(|_| "LaunchAgent registered but transaction journal remains")?;
    if matches!(action, TaskAction::Start) {
        run(&["kickstart", &agent.target()])?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_reads_reject_common_guard_contention_before_broker_spawn() {
        let temp = tempfile::tempdir().unwrap();
        let options = ServiceOptions {
            data_root: temp.path().join("root"),
            unit_dir: temp.path().join("units"),
            binary: std::env::current_exe().unwrap(),
            use_manager: true,
        };
        let held = LifecycleOperation::acquire(&HieronymusConfig::new(&options.data_root)).unwrap();
        let attempts = macos_broker::SPAWN_ATTEMPTS.get();
        assert!(
            read_unit(&options)
                .unwrap_err()
                .contains("another lifecycle operation")
        );
        assert!(super::super::status(&options).is_err());
        assert!(macos_broker::inspect(&options, false).is_err());
        assert_eq!(attempts, macos_broker::SPAWN_ATTEMPTS.get());
        drop(held);
        let other = ServiceOptions {
            data_root: temp.path().join("other"),
            ..options.clone()
        };
        let held = LifecycleOperation::acquire(&HieronymusConfig::new(&other.data_root)).unwrap();
        held.register_unit(&other).unwrap();
        assert!(
            read_unit(&options)
                .unwrap_err()
                .contains("another operation holds this service registration")
        );
        assert_eq!(attempts, macos_broker::SPAWN_ATTEMPTS.get());
    }
    #[test]
    #[ignore = "Requires current-user Aqua launchd; only fresh disposable on-demand agents"]
    fn native_interrupted_registration_restores_prior_absence_and_rejects_foreign_file() {
        let temp = tempfile::tempdir().unwrap();
        let options = ServiceOptions {
            data_root: temp.path().join("root"),
            unit_dir: temp.path().join("agents"),
            binary: std::env::current_exe().unwrap(),
            use_manager: true,
        };
        std::fs::create_dir_all(&options.data_root).unwrap();
        std::fs::create_dir_all(&options.unit_dir).unwrap();
        let agent = Agent::new(&options, false).unwrap();
        let before = agent.snapshot().unwrap();
        assert!(before.definition.is_none());
        assert!(!before.loaded);
        let journal = Journal {
            before,
            after: Some(agent.expected.clone()),
        };
        hieronymus::atomic::atomic_write_text(&agent.path, &agent.expected).unwrap();
        run(&["bootstrap", &agent.domain, agent.path.to_str().unwrap()]).unwrap();
        assert!(agent.snapshot().unwrap().loaded);
        // Actual bootstrap occurred, but RunAtLoad=false never ran the test binary.
        agent.restore(&journal).unwrap();
        let restored = agent.snapshot().unwrap();
        assert!(restored.definition.is_none());
        assert!(!restored.loaded);
        std::fs::write(&agent.path, "foreign").unwrap();
        assert!(agent.restore(&journal).is_err());
        assert_eq!(std::fs::read_to_string(&agent.path).unwrap(), "foreign");
        std::fs::remove_file(&agent.path).unwrap();
    }
}
