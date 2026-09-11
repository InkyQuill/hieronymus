//! Current-user Aqua LaunchAgents. No hard-stop or kickstart -k is used.
use super::*;
use crate::platform::{
    macos_broker::{self, TaskAction},
    macos_identity,
};
use macos_agent::{DaemonMode, LoadedState};
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
    macos_agent::render_mode(
        binary,
        root,
        &default_unit_dir(),
        false,
        DaemonMode::Headless,
    )
    .map_err(ServiceError::Invalid)
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
        "macOS daemon LaunchAgent installed; owned login mode retained".into(),
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
        "macOS daemon has no automatic recovery; login preference preserved".into(),
    ])
}
/// The owned plist is the persisted daemon-login declaration, not an XDG link.
pub(crate) fn owned_login_link(options: &ServiceOptions) -> Result<Option<PathBuf>, ServiceError> {
    match read_file(&options.unit_path()).map_err(ServiceError::Invalid)? {
        Some(text)
            if owned_mode(options, &text).map_err(ServiceError::Invalid)?
                == DaemonMode::Headless =>
        {
            Ok(Some(options.unit_path()))
        }
        _ => Ok(None),
    }
}
pub(crate) fn disable_login_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<(), ServiceError> {
    operation.register_unit(options)?;
    if options.use_manager {
        macos_broker::task(options, TaskAction::DesktopMode, false)
            .map_err(ServiceError::Manager)?;
    } else {
        validate_unit_root_guarded(options, operation)?;
        hieronymus::atomic::atomic_write_text(
            &options.unit_path(),
            &definition_mode(options, false, DaemonMode::Desktop).map_err(ServiceError::Invalid)?,
        )?;
    }
    Ok(())
}
/// Caller retains offline root ownership after authenticated shutdown/release.
pub(crate) fn finish_stop_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<(), ServiceError> {
    operation.register_unit(options)?;
    if options.use_manager {
        macos_broker::task(options, TaskAction::FinishStop, false)
            .map_err(ServiceError::Manager)?;
    }
    Ok(())
}
/// Actual future-login preference, read with the ordinary operation guards.
pub fn daemon_login_enabled(options: &ServiceOptions) -> Result<bool, String> {
    let state = macos_broker::inspect(options, false)?;
    if state.is_null() {
        return Ok(false);
    }
    state
        .get("login_enabled")
        .and_then(Value::as_bool)
        .ok_or("Invalid daemon login readback".into())
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
fn owned_mode(options: &ServiceOptions, text: &str) -> Result<DaemonMode, String> {
    let root = options
        .data_root
        .canonicalize()
        .map_err(|_| "LaunchAgent root is unavailable")?;
    let directory = options
        .unit_dir
        .canonicalize()
        .map_err(|_| "LaunchAgent directory is unavailable")?;
    macos_agent::owned_mode(text, &options.binary, &root, &directory)
}
fn definition(options: &ServiceOptions, tray: bool) -> Result<String, String> {
    let mode = if tray {
        DaemonMode::Desktop
    } else {
        read_file(&path(options, false))?
            .map(|text| owned_mode(options, &text))
            .transpose()?
            .unwrap_or(DaemonMode::Headless)
    };
    definition_mode(options, tray, mode)
}
fn definition_mode(
    options: &ServiceOptions,
    tray: bool,
    mode: DaemonMode,
) -> Result<String, String> {
    let root = options
        .data_root
        .canonicalize()
        .map_err(|_| "LaunchAgent root is unavailable")?;
    let directory = options
        .unit_dir
        .canonicalize()
        .map_err(|_| "LaunchAgent directory is unavailable")?;
    macos_agent::render_mode(&options.binary, &root, &directory, tray, mode)
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
    fn loaded(&self) -> Result<Option<LoadedState>, String> {
        let (status, text) = command(&["print", &self.target()])?;
        if status.success() {
            return self.validate_loaded(&text).map(Some);
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
    fn validate_loaded(&self, text: &str) -> Result<LoadedState, String> {
        let loaded = macos_agent::loaded_state(
            text,
            &self.path,
            &macos_agent::arguments(
                &self.options.binary,
                &self.options.data_root,
                &self.options.unit_dir,
                self.tray,
            )?,
        )?;
        if loaded.target != self.target() {
            return Err("Loaded LaunchAgent target differs".into());
        }
        let current = read_file(&self.path)?.ok_or("Loaded LaunchAgent has no owned definition")?;
        self.validate_definition(&current)?;
        let run_at_load = self.tray || owned_mode(self.options, &current)? == DaemonMode::Headless;
        if loaded.run_at_load != run_at_load {
            return Err("Loaded LaunchAgent login mode differs".into());
        }
        Ok(loaded)
    }
    fn validate_definition(&self, text: &str) -> Result<(), String> {
        if self.tray {
            if text != definition_mode(self.options, true, DaemonMode::Desktop)? {
                return Err("Foreign tray LaunchAgent".into());
            }
        } else {
            owned_mode(self.options, text)?;
        }
        Ok(())
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
        if let Some(loaded) = self.loaded()? {
            if loaded.pid.is_some() || !loaded.idle {
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
        if let Some(text) = &definition {
            self.validate_definition(text)?;
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
            self.validate_definition(text)?;
        }
        let actual = read_file(&self.path)?;
        if actual != journal.before.definition && actual != journal.after {
            return Err("Pending LaunchAgent conflicts with external changes".into());
        }
        if !journal.before.loaded {
            self.bootout_idle()?;
        } else if actual != journal.before.definition {
            return Err(
                "Cannot restore changed login mode of a previously loaded LaunchAgent".into(),
            );
        }
        macos_agent::restore_definition(
            &self.path,
            journal.before.definition.as_deref().map(str::as_bytes),
        )?;
        self.toggle(journal.before.disabled)?;
        if journal.before.loaded && self.loaded()?.is_none() {
            if self.tray
                || journal
                    .before
                    .definition
                    .as_deref()
                    .map(|text| owned_mode(self.options, text))
                    .transpose()?
                    == Some(DaemonMode::Headless)
            {
                return Err("Restoring this loaded login agent would start a process; rollback remains pending".into());
            }
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
fn execute_agent(
    options: &ServiceOptions,
    action: TaskAction,
    tray: bool,
) -> Result<Value, String> {
    let mut agent = Agent::new(options, tray)?;
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
                agent.validate_definition(text)?;
            }
            let actual = agent.snapshot()?;
            return Ok(if actual.definition.is_none() {
                Value::Null
            } else {
                readback(&agent, &actual, true)?
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
            readback(&agent, &before, false)?
        });
    }
    if matches!(action, TaskAction::Reconcile) {
        return readback(&agent, &before, false);
    }
    if matches!(action, TaskAction::FinishStop) {
        agent.bootout_idle()?;
        return readback(&agent, &agent.snapshot()?, false);
    }
    if matches!(action, TaskAction::Suppress) {
        // The owned no-KeepAlive policy already suppresses recovery. Preserve
        // native enablement so a future headless login still starts its daemon.
        return readback(&agent, &before, false);
    }
    if matches!(action, TaskAction::DesktopMode) {
        if tray {
            return Err("Daemon mode operation requires daemon registration".into());
        }
        let desired = definition_mode(options, false, DaemonMode::Desktop)?;
        macos_agent::check_mode_change(
            before
                .definition
                .as_deref()
                .map(|text| owned_mode(options, text))
                .transpose()?,
            before.loaded,
            DaemonMode::Desktop,
        )?;
        agent.expected = desired;
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
            #[cfg(test)]
            if tray && matches!(action, TaskAction::Install) && FAIL_TRAY_PUBLICATION.replace(false)
            {
                return Err("Injected tray publication failure".into());
            }
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
            // Login publication never starts a process while the caller retains
            // offline ownership. Only Desktop mode can bootstrap here (RunAtLoad
            // false); Headless startup is deferred until explicit Start/login.
            if !tray
                && !disabled
                && !matches!(action, TaskAction::Start)
                && owned_mode(options, &agent.expected)? == DaemonMode::Desktop
                && agent.loaded()?.is_none()
            {
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
            readback(&agent, &actual, false)
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
        // Launch only after registration commit: a startup error must not leave
        // a journal whose repair requires stopping the just-started daemon.
        if agent.loaded()?.is_none() {
            run(&[
                "bootstrap",
                &agent.domain,
                agent.path.to_str().ok_or("Invalid LaunchAgent path")?,
            ])?;
        }
        if agent.loaded()?.is_some_and(|loaded| loaded.idle) {
            run(&["kickstart", &agent.target()])?;
        }
    }
    Ok(value)
}

fn readback(agent: &Agent<'_>, snapshot: &Snapshot, pending: bool) -> Result<Value, String> {
    let Some(text) = &snapshot.definition else {
        return Ok(Value::Null);
    };
    let mode = if agent.tray {
        DaemonMode::Desktop
    } else {
        owned_mode(agent.options, text)?
    };
    Ok(
        json!({"enabled":!snapshot.disabled,"loaded":snapshot.loaded,"pending":pending,"mode":mode,"login_enabled":!snapshot.disabled&&(agent.tray||mode==DaemonMode::Headless)}),
    )
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DesktopJournal {
    daemon: Snapshot,
    tray: Snapshot,
}
fn restore_desktop(options: &ServiceOptions, journal: &DesktopJournal) -> Result<(), String> {
    // Finish any interrupted inner publication before restoring the complete
    // prior pair. No nested pending journal may replay a later partial mode.
    execute_agent(options, TaskAction::Reconcile, true)?;
    execute_agent(options, TaskAction::Reconcile, false)?;
    let tray = Agent::new(options, true)?;
    tray.restore(&Journal {
        before: journal.tray.clone(),
        after: Some(definition_mode(options, true, DaemonMode::Desktop)?),
    })?;
    let daemon = Agent::new(options, false)?;
    daemon.restore(&Journal {
        before: journal.daemon.clone(),
        after: Some(definition_mode(options, false, DaemonMode::Desktop)?),
    })?;
    Ok(())
}
/// Composite desktop installation keeps prior daemon mode/native preference in
/// one continuing broker transaction, including failure after daemon conversion.
pub(crate) fn execute(
    options: &ServiceOptions,
    action: TaskAction,
    tray: bool,
) -> Result<Value, String> {
    let pending = path(options, false).with_extension("desktop-pending.json");
    if let Some(text) = read_file(&pending)? {
        let journal: DesktopJournal =
            serde_json::from_str(&text).map_err(|_| "Invalid desktop transaction")?;
        for (is_tray, snapshot) in [(false, &journal.daemon), (true, &journal.tray)] {
            if let Some(text) = &snapshot.definition {
                Agent::new(options, is_tray)?.validate_definition(text)?;
            }
        }
        if matches!(action, TaskAction::Inspect) {
            let mut result = execute_agent(options, action, tray)?;
            if let Some(object) = result.as_object_mut() {
                object.insert("pending".into(), Value::Bool(true));
            }
            return Ok(result);
        }
        restore_desktop(options, &journal)?;
        std::fs::remove_file(&pending)
            .map_err(|_| "Could not clear recovered desktop transaction")?;
    }
    if !matches!(action, TaskAction::InstallDesktop) {
        return execute_agent(options, action, tray);
    }
    if !tray {
        return Err("Desktop installation requires tray registration".into());
    }
    execute_agent(options, TaskAction::Reconcile, true)?;
    execute_agent(options, TaskAction::Reconcile, false)?;
    let daemon = Agent::new(options, false)?;
    let tray = Agent::new(options, true)?;
    let before = DesktopJournal {
        daemon: daemon.snapshot()?,
        tray: tray.snapshot()?,
    };
    macos_agent::check_mode_change(
        before
            .daemon
            .definition
            .as_deref()
            .map(|text| owned_mode(options, text))
            .transpose()?,
        before.daemon.loaded,
        DaemonMode::Desktop,
    )?;
    hieronymus::atomic::atomic_write(
        &pending,
        &serde_json::to_vec(&before).map_err(|_| "Could not encode desktop transaction")?,
    )
    .map_err(|_| "Could not persist desktop transaction")?;
    let result = macos_agent::publish_with_rollback(
        || {
            execute_agent(options, TaskAction::DesktopMode, false)?;
            execute_agent(options, TaskAction::Install, true)
        },
        || restore_desktop(options, &before),
    );
    if result
        .as_ref()
        .is_err_and(|error| error == "LaunchAgent registration failed; rollback remains pending")
    {
        return result;
    }
    std::fs::remove_file(&pending)
        .map_err(|_| "Desktop transaction completed but journal remains")?;
    result
}

#[cfg(test)]
thread_local! { static FAIL_TRAY_PUBLICATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

#[cfg(test)]
mod tests {
    use super::*;
    use macos_agent::DaemonMode;
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
        let mut agent = Agent::new(&options, false).unwrap();
        agent.expected = definition_mode(&options, false, DaemonMode::Desktop).unwrap();
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
    #[test]
    #[ignore = "Requires an Aqua login; only unique disposable jobs running /usr/bin/true"]
    fn native_headless_stop_conversion_and_failed_tray_restores_prior_mode() {
        let temp = tempfile::tempdir().unwrap();
        let options = ServiceOptions {
            data_root: temp.path().join("root"),
            unit_dir: temp.path().join("agents"),
            binary: PathBuf::from("/usr/bin/true"),
            use_manager: true,
        };
        std::fs::create_dir_all(&options.data_root).unwrap();
        std::fs::create_dir_all(&options.unit_dir).unwrap();
        execute(&options, TaskAction::Install, false).unwrap();
        let agent = Agent::new(&options, false).unwrap();
        let headless = agent.snapshot().unwrap();
        assert!(!headless.loaded);
        assert!(!headless.disabled);
        assert_eq!(
            owned_mode(&options, headless.definition.as_deref().unwrap()).unwrap(),
            DaemonMode::Headless
        );
        // A benign native fixture process executes once and exits; no daemon,
        // model, browser or real user service is started by this test.
        run(&["bootstrap", &agent.domain, agent.path.to_str().unwrap()]).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !agent.loaded().unwrap().is_some_and(|job| job.idle)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(agent.loaded().unwrap().unwrap().idle);
        assert!(
            execute(&options, TaskAction::InstallDesktop, true)
                .unwrap_err()
                .contains("Stop the daemon")
        );
        assert_eq!(read_file(&agent.path).unwrap(), headless.definition);
        assert!(!agent.disabled().unwrap());
        assert!(agent.loaded().unwrap().is_some());
        let operation =
            LifecycleOperation::acquire(&HieronymusConfig::new(&options.data_root)).unwrap();
        operation.register_unit(&options).unwrap();
        let _owner = RootOwnership::acquire(
            &HieronymusConfig::new(&options.data_root),
            "native-fixture-post-stop",
        )
        .unwrap();
        execute(&options, TaskAction::Suppress, false).unwrap();
        execute(&options, TaskAction::FinishStop, false).unwrap();
        let stopped = agent.snapshot().unwrap();
        assert!(!stopped.loaded);
        assert_eq!(stopped.disabled, headless.disabled);
        assert_eq!(stopped.definition, headless.definition);
        FAIL_TRAY_PUBLICATION.set(true);
        assert!(
            execute(&options, TaskAction::InstallDesktop, true)
                .unwrap_err()
                .contains("Injected tray publication")
        );
        let restored = agent.snapshot().unwrap();
        assert_eq!(restored.definition, headless.definition);
        assert_eq!(restored.disabled, headless.disabled);
        assert!(!restored.loaded);
        assert!(read_file(&path(&options, true)).unwrap().is_none());
        execute(&options, TaskAction::InstallDesktop, true).unwrap();
        assert_eq!(
            owned_mode(&options, &read_file(&agent.path).unwrap().unwrap()).unwrap(),
            DaemonMode::Desktop
        );
        execute(&options, TaskAction::Install, false).unwrap();
        assert_eq!(
            owned_mode(&options, &read_file(&agent.path).unwrap().unwrap()).unwrap(),
            DaemonMode::Desktop
        );
        execute(&options, TaskAction::Remove, true).unwrap();
        execute(&options, TaskAction::Remove, false).unwrap();
    }
}
