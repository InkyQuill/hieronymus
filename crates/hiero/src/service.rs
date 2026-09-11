//! Per-user daemon service management (`hiero service …`, security design
//! spec §Service Installation): one systemd **user** unit rendered from the
//! absolute installed binary and the explicit data root, with a restart/backoff
//! policy and a stable log identifier. Install, upgrade, and uninstall are
//! idempotent and never touch user data.
//!
//! Manager integration rules (this is the test seam too): the systemd user
//! manager is only ever contacted when the unit lives at the default location
//! (`~/.config/systemd/user`) and `systemctl` exists on `PATH`. An explicit
//! `--unit-dir` override means the caller owns the manager interaction, so
//! `install`/`uninstall` only render or remove the unit file and report it —
//! which keeps tests away from the real user manager and makes custom
//! locations usable on non-systemd setups.

use crate::lifecycle::operation::LifecycleOperation;
use hieronymus::data_root::HieronymusConfig;
use hieronymus::ownership::RootOwnership;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Name of the unit this project installs.
pub const SERVICE_UNIT_NAME: &str = "hieronymus.service";

/// Seconds between daemon restart attempts (on-failure backoff policy).
const RESTART_SECONDS: &str = "5s";

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Manager(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Everything one service subcommand needs: the data root to serve, the unit
/// directory, the binary the unit must exec, and whether manager integration
/// is allowed at all (CLI `--no-activate` sets it to `false`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceOptions {
    pub data_root: PathBuf,
    pub unit_dir: PathBuf,
    pub binary: PathBuf,
    pub use_manager: bool,
}

impl ServiceOptions {
    pub fn unit_path(&self) -> PathBuf {
        self.unit_dir.join(SERVICE_UNIT_NAME)
    }
}

/// Default per-user unit directory (systemd user units).
pub fn default_unit_dir() -> PathBuf {
    home::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("systemd")
        .join("user")
}

/// Whether the systemd user manager may be contacted for these options: only
/// when manager use is allowed, the unit lives at the default location, and
/// `systemctl` exists on `PATH`.
pub fn manager_enabled(options: &ServiceOptions) -> bool {
    options.use_manager && options.unit_dir == default_unit_dir() && systemctl_on_path()
}

fn systemctl_on_path() -> bool {
    std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path).any(|directory| directory.join("systemctl").is_file())
        })
        .unwrap_or(false)
}

fn run_systemctl(arguments: &[&str]) -> Result<(), ServiceError> {
    run_manager(
        Path::new("systemctl"),
        arguments,
        std::time::Duration::from_secs(30),
    )
}

// Mutation commands do not consume stdout. Bound the client process without
// killing a daemon or claiming a timed-out manager job was cancelled.
fn run_manager(
    executable: &Path,
    arguments: &[&str],
    timeout: std::time::Duration,
) -> Result<(), ServiceError> {
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut child = Command::new(executable)
        .arg("--user")
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            ServiceError::Manager("could not run systemctl; check the user service manager".into())
        })?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ServiceError::Manager(format!(
                    "systemctl --user {} timed out or could not be observed; inspect the user service before retrying",
                    arguments.join(" ")
                )));
            }
        }
    };
    if status.success() {
        return Ok(());
    }
    Err(ServiceError::Manager(format!(
        "systemctl --user {} failed; inspect the user service manager",
        arguments.join(" ")
    )))
}

/// The unit file content for one binary/data-root pair. Deterministic, so an
/// idempotent reinstall is byte-identical. Both paths are double-quoted;
/// systemd accepts quoted ExecStart arguments and the parser here mirrors it.
pub fn render_unit(binary: &Path, data_root: &Path) -> Result<String, ServiceError> {
    let binary = absolute(binary)?;
    let data_root = absolute(data_root)?;
    let dispatcher = if binary
        .to_str()
        .is_some_and(|p| p.contains(['\'', '"', '\\', '$']))
    {
        if !Path::new("/usr/bin/env").is_file() {
            return Err(ServiceError::Invalid("special executable paths require /usr/bin/env; install coreutils or choose another application directory".into()));
        }
        "\"/usr/bin/env\" -- "
    } else {
        ""
    };
    let binary = encode_unit_path(&binary)?;
    let data_root = encode_unit_path(&data_root)?;
    Ok(format!(
        "# Generated by `hiero service install` — manual edits are overwritten by \
         the next install or update.\n\
         [Unit]\n\
         Description=Hieronymus translation memory daemon\n\
         After=network.target\n\
         \n\
         [Service]\n\
         ExecStart={dispatcher}\"{binary}\" daemon --data-root \"{data_root}\"\n\
         Restart=on-failure\n\
         RestartSec={RESTART_SECONDS}\n\
         SyslogIdentifier=hieronymus-daemon\n\
         NoNewPrivileges=true\n\
         PrivateTmp=true\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        binary = binary,
        data_root = data_root,
    ))
}

fn absolute(path: &Path) -> Result<PathBuf, ServiceError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Err(ServiceError::Invalid(format!(
            "path must be absolute (the unit may not rely on the daemon's working \
             directory): {}",
            path.display()
        )))
    }
}

/// The parsed execution definition of one unit file, as far as this project
/// renders and reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitDefinition {
    pub binary: PathBuf,
    pub data_root: PathBuf,
}

/// Parse `ExecStart="…" daemon --data-root "…"`, optionally behind the exact
/// fixed `/usr/bin/env --` dispatcher. Returns
/// `Err` with a reason when the unit is not one of ours or is broken.
pub fn parse_unit(text: &str) -> Result<UnitDefinition, String> {
    let line = text
        .lines()
        .find(|line| line.starts_with("ExecStart="))
        .ok_or("unit file has no ExecStart entry")?;
    let tokens = tokenize(&line["ExecStart=".len()..])?;
    let arguments = match tokens.as_slice() {
        [dispatcher, separator, rest @ ..] if dispatcher == "/usr/bin/env" && separator == "--" => {
            rest
        }
        [dispatcher, ..] if dispatcher == "/usr/bin/env" => {
            return Err("unsupported env dispatcher in unit".into());
        }
        arguments => arguments,
    };
    match arguments {
        [binary, command, flag, data_root]
            if command == "daemon"
                && flag == "--data-root"
                && Path::new(binary).is_absolute()
                && Path::new(data_root).is_absolute() =>
        {
            Ok(UnitDefinition {
                binary: PathBuf::from(binary),
                data_root: PathBuf::from(data_root),
            })
        }
        _ => Err("unit ExecStart is not `\"<binary>\" daemon --data-root \"<root>\"`".to_string()),
    }
}

/// Escape systemd command arguments and disable specifier/environment expansion.
fn encode_unit_path(path: &Path) -> Result<String, ServiceError> {
    let value = path
        .to_str()
        .ok_or_else(|| ServiceError::Invalid("service paths must be UTF-8".into()))?;
    if value.chars().any(char::is_control) {
        return Err(ServiceError::Invalid(
            "service paths cannot contain control characters or newlines".into(),
        ));
    }
    Ok(value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$"))
}

/// Decode the renderer's bounded subset; reject expansions or malformed quoting.
fn tokenize(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' => match chars.next() {
                Some(c @ ('\\' | '"')) => current.push(c),
                _ => return Err("unsupported unit escape".into()),
            },
            '%' | '$' => {
                if chars.next() == Some(c) {
                    current.push(c);
                } else {
                    return Err("unsupported unit expansion".into());
                }
            }
            ' ' if !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c if c.is_control() => return Err("unsupported unit control character".into()),
            c => current.push(c),
        }
    }
    if quoted {
        return Err("unterminated unit quote".into());
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Read and parse the unit file for `options`, if it exists.
pub fn read_unit(options: &ServiceOptions) -> Result<Option<UnitDefinition>, String> {
    let path = options.unit_path();
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    parse_unit(&text)
        .map(Some)
        .map_err(|reason| format!("{}: {reason}", path.display()))
}

/// Install (or idempotently reinstall) the unit. The unit file is written
/// atomically; the manager, when engaged, is reloaded and the unit enabled
/// in headless mode (desktop mode retains on-demand startup), **without starting it** — the start decision belongs to the caller after
/// the schema check (bootstrap installer step 8).
pub fn install(options: &ServiceOptions) -> Result<Vec<String>, ServiceError> {
    let config = HieronymusConfig::new(&options.data_root);
    let operation = LifecycleOperation::acquire(&config)?;
    operation.register_unit(options)?;
    let health = crate::lifecycle::checked_probe(&config)
        .map_err(|error| ServiceError::Invalid(error.to_string()))?;
    // Missing/unreachable discovery is not proof that an owner is absent.
    // Keep the offline claim through both unit publication and reload/enable;
    // an authenticated live owner already supplies the required identity.
    let _ownership = if health.is_live() {
        None
    } else {
        Some(RootOwnership::acquire(&config, "service-install")?)
    };
    install_guarded(options, &operation)
}

// The caller retains offline RootOwnership through this call, or has just
// authenticated the live owner. Do not reacquire ownership here: startup and
// update already hold it while installing the unit.
pub(crate) fn install_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Vec<String>, ServiceError> {
    operation.register_unit(options)?;
    validate_unit_root(options)?;
    let desktop_mode =
        crate::desktop::linux_registration::desktop_mode(options).map_err(ServiceError::Invalid)?;
    let mut lines = Vec::new();
    if !options.binary.is_file() {
        return Err(ServiceError::Invalid(format!(
            "service binary does not exist: {}",
            options.binary.display()
        )));
    }
    let unit = render_unit(&options.binary, &options.data_root)?;
    hieronymus::atomic::atomic_write_text(&options.unit_path(), &unit)?;
    lines.push(format!(
        "service unit written: {}",
        options.unit_path().display()
    ));
    if manager_enabled(options) {
        run_systemctl(&["daemon-reload"])?;
        if !desktop_mode {
            run_systemctl(&["enable", SERVICE_UNIT_NAME])?;
            lines.push(format!(
                "service enabled (not started): {SERVICE_UNIT_NAME}"
            ));
        } else {
            lines.push("desktop mode: service remains startable on demand".into());
        }
    } else {
        lines.push(
            "manager integration skipped (custom --unit-dir or no systemctl on PATH); \
             enable/start it manually if needed"
                .to_string(),
        );
    }
    Ok(lines)
}

/// Gracefully stop the daemon, then remove its unit; databases, configuration,
/// models, backups, and audit data are never touched. Idempotent: a missing unit is a no-op.
pub fn uninstall(options: &ServiceOptions) -> Result<Vec<String>, ServiceError> {
    let config = HieronymusConfig::new(&options.data_root);
    let operation = LifecycleOperation::acquire(&config)?;
    crate::lifecycle::stop_guarded(&config, options, &operation)
        .map_err(|error| ServiceError::Invalid(error.to_string()))?;
    uninstall_guarded(options, &operation)
}

pub(crate) fn uninstall_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Vec<String>, ServiceError> {
    operation.register_unit(options)?;
    validate_unit_root(options)?;
    let mut lines = Vec::new();
    let path = options.unit_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
        lines.push(format!("service unit removed: {}", path.display()));
    } else {
        lines.push("service unit already absent".to_string());
    }
    if manager_enabled(options) {
        run_systemctl(&["daemon-reload"])?;
        lines.push("manager reloaded".to_string());
    }
    Ok(lines)
}

/// Read the login link created by our deterministic default.target unit. Refuse
/// foreign links before asking the manager to change the shared registration.
pub(crate) fn owned_login_link(options: &ServiceOptions) -> Result<Option<PathBuf>, ServiceError> {
    let path = options
        .unit_dir
        .join("default.target.wants")
        .join(SERVICE_UNIT_NAME);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let target = std::fs::read_link(&path)?;
            let resolved = if target.is_absolute() {
                target.clone()
            } else {
                path.parent().unwrap().join(&target)
            };
            if resolved.canonicalize()? != options.unit_path().canonicalize()? {
                return Err(ServiceError::Invalid(
                    "daemon login link belongs to another installation".into(),
                ));
            }
            Ok(Some(target))
        }
        Ok(_) => Err(ServiceError::Invalid(
            "daemon login registration is not an owned symbolic link".into(),
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Disable future daemon logins without --now: the current daemon keeps running.
/// Offline/custom installations reconcile only the positively owned local link.
pub(crate) fn disable_login_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<(), ServiceError> {
    operation.register_unit(options)?;
    validate_unit_root(options)?;
    let link = owned_login_link(options)?;
    if manager_enabled(options) {
        run_systemctl(&["disable", SERVICE_UNIT_NAME])?;
    }
    if link.is_some() {
        let path = options
            .unit_dir
            .join("default.target.wants")
            .join(SERVICE_UNIT_NAME);
        if path.symlink_metadata().is_ok() {
            owned_login_link(options)?;
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// What `hiero service status` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub unit_path: PathBuf,
    pub definition: Option<UnitDefinition>,
    /// Why the definition is not consistent with these options, if it is not.
    pub problems: Vec<String>,
}

impl ServiceStatus {
    pub fn consistent(&self) -> bool {
        self.definition.is_some() && self.problems.is_empty()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "unit": self.unit_path,
            "installed": self.definition.is_some(),
            "consistent": self.consistent(),
            "binary": self.definition.as_ref().map(|definition| definition.binary.clone()),
            "data_root": self.definition.as_ref().map(|definition| definition.data_root.clone()),
            "problems": self.problems,
        })
    }

    pub fn render_human(&self) -> String {
        match &self.definition {
            None => format!(
                "service unit: not installed ({} does not exist)",
                self.unit_path.display()
            ),
            Some(definition) => {
                let mut text = format!(
                    "service unit: {}\n  binary: {}\n  data root: {}",
                    self.unit_path.display(),
                    definition.binary.display(),
                    definition.data_root.display()
                );
                for problem in &self.problems {
                    text.push_str(&format!("\n  problem: {problem}"));
                }
                text
            }
        }
    }
}

/// Compare the installed unit against the expected definition.
pub fn status(options: &ServiceOptions) -> Result<ServiceStatus, ServiceError> {
    let definition = read_unit(options).map_err(ServiceError::Invalid)?;
    let mut problems = Vec::new();
    if let Some(definition) = &definition {
        if !definition.binary.is_file() {
            problems.push(format!(
                "unit binary does not exist: {}",
                definition.binary.display()
            ));
        }
        if definition.data_root != options.data_root {
            problems.push(format!(
                "unit serves data root {} but this root is {}",
                definition.data_root.display(),
                options.data_root.display()
            ));
        }
    }
    Ok(ServiceStatus {
        unit_path: options.unit_path(),
        definition,
        problems,
    })
}

/// Start or stop the daemon through the manager. Refuses (instead of
/// degrading) because an explicit lifecycle request that cannot be executed
/// must fail loudly, with the manual command in the message.
pub fn start(options: &ServiceOptions) -> Result<Vec<String>, ServiceError> {
    let config = HieronymusConfig::new(&options.data_root);
    let operation = LifecycleOperation::acquire(&config)?;
    operation.register_unit(options)?;
    if !options.unit_path().exists() {
        return Err(ServiceError::Invalid(
            "no service unit; install one with `hiero service install`".into(),
        ));
    }
    crate::lifecycle::start_guarded(&config, options, &operation)
        .map_err(|error| ServiceError::Manager(error.to_string()))
}

pub fn stop(options: &ServiceOptions) -> Result<Vec<String>, ServiceError> {
    crate::lifecycle::stop(&HieronymusConfig::new(&options.data_root), options)
        .map_err(|error| ServiceError::Manager(error.to_string()))
}

pub(crate) fn start_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Vec<String>, ServiceError> {
    operation.register_unit(options)?;
    lifecycle(options, "start", &["start", SERVICE_UNIT_NAME])
}

pub(crate) fn stop_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Vec<String>, ServiceError> {
    operation.register_unit(options)?;
    lifecycle(options, "stop", &["stop", SERVICE_UNIT_NAME])
}

/// Refuse foreign or unparseable registration before any manager/file change.
/// A same-root obsolete binary may be repaired by install or update.
pub(crate) fn validate_unit_root(options: &ServiceOptions) -> Result<(), ServiceError> {
    if let Some(definition) = read_unit(options).map_err(ServiceError::Invalid)? {
        let unit_root = definition
            .data_root
            .canonicalize()
            .unwrap_or(definition.data_root.clone());
        let expected = options
            .data_root
            .canonicalize()
            .unwrap_or(options.data_root.clone());
        if unit_root != expected {
            return Err(ServiceError::Invalid(format!(
                "unit serves data root {} but this root is {}",
                definition.data_root.display(),
                options.data_root.display(),
            )));
        }
    }
    Ok(())
}

/// The three manager lifecycle operations `hiero update`'s rollback state
/// machine drives, behind a trait so tests can assert the exact call sequence
/// and script a chosen call to fail. Each operation is all-or-nothing: it
/// either completes or returns [`ServiceError`], never a partial success the
/// caller has to interpret.
pub trait ServiceManager {
    /// Stop the managed unit (the candidate the failed activation may have
    /// started). A unit that is already stopped is still `Ok`.
    fn stop(&self) -> Result<(), ServiceError>;
    /// Re-read unit files after the on-disk unit was restored.
    fn reload(&self) -> Result<(), ServiceError>;
    /// Start the managed unit (the restored previous version).
    fn start(&self) -> Result<(), ServiceError>;
}

/// The production [`ServiceManager`]: the systemd **user** manager, contacted
/// only when [`manager_enabled`] holds for its options. A custom `--unit-dir`
/// or a host without `systemctl` skips manager contact after ownership checks,
/// so the updater's rollback path does
/// link/unit restoration without ever touching a manager it must not touch.
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
        self.operation.register_unit(&self.options)?;
        validate_unit_root(&self.options)?;
        if !manager_enabled(&self.options) {
            return Ok(());
        }
        crate::lifecycle::stop_guarded(
            &HieronymusConfig::new(&self.options.data_root),
            &self.options,
            self.operation,
        )
        .map(|_| ())
        .map_err(|error| ServiceError::Manager(error.to_string()))
    }

    fn reload(&self) -> Result<(), ServiceError> {
        self.operation.register_unit(&self.options)?;
        validate_unit_root(&self.options)?;
        if !manager_enabled(&self.options) {
            return Ok(());
        }
        run_systemctl(&["daemon-reload"])
    }

    fn start(&self) -> Result<(), ServiceError> {
        self.operation.register_unit(&self.options)?;
        validate_unit_root(&self.options)?;
        if !manager_enabled(&self.options) {
            return Ok(());
        }
        crate::lifecycle::checked_probe(&HieronymusConfig::new(&self.options.data_root))
            .map_err(|error| ServiceError::Manager(error.to_string()))?;
        run_systemctl(&["start", SERVICE_UNIT_NAME])
    }
}

/// Verdict of comparing the installed unit against the expected definition and
/// the currently running binary — doctor's service-definition check (security
/// spec §Service Installation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitVerdict {
    Absent,
    Consistent,
    /// The unit is unreadable, is not one of ours, or its binary is gone.
    Broken(String),
    /// The unit serves a different data root than the one being checked.
    DataRootMismatch {
        unit_root: PathBuf,
    },
    /// The unit still execs a different binary than the running one (typical
    /// after an update that did not reinstall the unit).
    StaleBinary {
        unit_binary: PathBuf,
    },
}

/// Read the unit for `options` and compare it with `current_binary`.
pub fn check_unit(options: &ServiceOptions, current_binary: &Path) -> UnitVerdict {
    let definition = match read_unit(options) {
        Ok(Some(definition)) => definition,
        Ok(None) => return UnitVerdict::Absent,
        Err(reason) => return UnitVerdict::Broken(reason),
    };
    if !definition.binary.is_file() {
        return UnitVerdict::Broken(format!(
            "unit binary does not exist: {}",
            definition.binary.display()
        ));
    }
    if definition.data_root != options.data_root {
        return UnitVerdict::DataRootMismatch {
            unit_root: definition.data_root.clone(),
        };
    }
    let same_binary = match (
        definition.binary.canonicalize(),
        current_binary.canonicalize(),
    ) {
        (Ok(unit), Ok(current)) => unit == current,
        _ => definition.binary == current_binary,
    };
    if !same_binary {
        return UnitVerdict::StaleBinary {
            unit_binary: definition.binary.clone(),
        };
    }
    UnitVerdict::Consistent
}

fn lifecycle(
    options: &ServiceOptions,
    action: &str,
    arguments: &[&str],
) -> Result<Vec<String>, ServiceError> {
    validate_unit_root(options)?;
    if !options.unit_path().exists() {
        return Err(ServiceError::Manager(format!(
            "no service unit at {}; install one with `hiero service install`",
            options.unit_path().display()
        )));
    }
    if !manager_enabled(options) {
        return Err(ServiceError::Manager(format!(
            "manager integration is disabled for --unit-dir {}; {action} the daemon \
             manually: {} daemon --data-root {}",
            options.unit_dir.display(),
            options.binary.display(),
            options.data_root.display()
        )));
    }
    run_systemctl(arguments)?;
    Ok(vec![format!("service {action}ed: {SERVICE_UNIT_NAME}")])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(temp: &tempfile::TempDir) -> ServiceOptions {
        ServiceOptions {
            data_root: temp.path().join("data"),
            unit_dir: temp.path().join("units"),
            binary: temp.path().join("bin/hiero"),
            use_manager: false,
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn manager_timeout_is_bounded_reaped_and_output_is_not_a_diagnostic() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("manager");
        let pid_file = root.path().join("pid");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nprintf 'SECRET' >&2\nexec sleep 30\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let began = std::time::Instant::now();
        let error = run_manager(
            &executable,
            &["start", SERVICE_UNIT_NAME],
            std::time::Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(began.elapsed() < std::time::Duration::from_secs(1));
        assert!(!error.to_string().contains("SECRET"));
        let pid = std::fs::read_to_string(pid_file).unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }

    #[test]
    fn unit_renders_absolute_paths_and_the_restart_policy() {
        let unit = render_unit(
            Path::new("/opt/app/versions/1.0.0/hiero"),
            Path::new("/home/u/root"),
        )
        .unwrap();
        assert!(
            unit.contains(
                "ExecStart=\"/opt/app/versions/1.0.0/hiero\" daemon --data-root \"/home/u/root\""
            ),
            "{unit}"
        );
        assert!(unit.contains("Restart=on-failure"), "{unit}");
        assert!(unit.contains("RestartSec=5s"), "{unit}");
        assert!(unit.contains("WantedBy=default.target"), "{unit}");
        assert!(unit.contains("After=network.target"), "{unit}");
        assert!(
            unit.contains("SyslogIdentifier=hieronymus-daemon"),
            "{unit}"
        );
        assert!(unit.contains("NoNewPrivileges=true"), "{unit}");
    }

    #[test]
    fn unit_rendering_requires_absolute_supported_paths() {
        let error = render_unit(Path::new("relative/hiero"), Path::new("/root")).unwrap_err();
        assert!(error.to_string().contains("absolute"), "{error}");
        let error = render_unit(Path::new("/a\nb"), Path::new("/root")).unwrap_err();
        assert!(error.to_string().contains("newline"), "{error}");
    }

    #[test]
    fn unit_parses_back_into_its_definition() {
        let text = render_unit(Path::new("/opt/hiero"), Path::new("/root with space")).unwrap();
        let definition = parse_unit(&text).unwrap();
        assert_eq!(definition.binary, PathBuf::from("/opt/hiero"));
        assert_eq!(definition.data_root, PathBuf::from("/root with space"));
    }

    #[test]
    fn parse_rejects_foreign_units() {
        assert!(parse_unit("[Unit]\nDescription=x\n").is_err());
        assert!(parse_unit("ExecStart=/usr/bin/other --serve\n").is_err());
    }

    #[test]
    fn install_is_idempotent_and_never_contacts_the_manager_for_custom_unit_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let options = options(&temp);
        std::fs::create_dir_all(options.binary.parent().unwrap()).unwrap();
        std::fs::write(&options.binary, b"fake binary").unwrap();

        let first = install(&options).unwrap();
        let second = install(&options).unwrap();
        assert!(
            first
                .iter()
                .any(|line| line.contains("manager integration skipped")),
            "{first:?}"
        );
        assert_eq!(
            std::fs::read_to_string(options.unit_path()).unwrap(),
            render_unit(&options.binary, &options.data_root).unwrap()
        );
        let _: Vec<String> = second;
    }

    #[test]
    fn install_refuses_a_missing_binary() {
        let temp = tempfile::tempdir().unwrap();
        let options = options(&temp);
        let error = install(&options).unwrap_err();
        assert!(error.to_string().contains("does not exist"), "{error}");
    }

    #[test]
    fn uninstall_removes_the_unit_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let options = options(&temp);
        std::fs::create_dir_all(&options.unit_dir).unwrap();
        std::fs::write(
            options.unit_path(),
            render_unit(&options.binary, &options.data_root).unwrap(),
        )
        .unwrap();

        let lines = uninstall(&options).unwrap();
        assert!(lines[0].contains("removed"), "{lines:?}");
        assert!(!options.unit_path().exists());

        let lines = uninstall(&options).unwrap();
        assert!(lines[0].contains("already absent"), "{lines:?}");
    }

    #[test]
    fn status_reports_absent_broken_and_consistent_units() {
        let temp = tempfile::tempdir().unwrap();
        let options = options(&temp);
        std::fs::create_dir_all(options.binary.parent().unwrap()).unwrap();
        std::fs::write(&options.binary, b"binary").unwrap();

        // Absent.
        let absent = status(&options).unwrap();
        assert!(!absent.consistent());
        assert!(absent.render_human().contains("not installed"));

        // Consistent.
        install(&options).unwrap();
        let consistent = status(&options).unwrap();
        assert!(consistent.consistent(), "{consistent:?}");
        assert!(consistent.to_json()["installed"].as_bool().unwrap());

        // Broken: binary removed underneath the unit.
        std::fs::remove_file(&options.binary).unwrap();
        let broken = status(&options).unwrap();
        assert!(!broken.consistent());
        assert!(broken.problems[0].contains("does not exist"));

        // Mismatched data root.
        std::fs::write(&options.binary, b"binary").unwrap();
        let mut other = options.clone();
        other.data_root = temp.path().join("elsewhere");
        let mismatched = status(&other).unwrap();
        assert!(!mismatched.consistent());
        assert!(mismatched.problems[0].contains("data root"));
    }

    #[test]
    fn lifecycle_refuses_without_a_unit_and_with_an_unverifiable_unit() {
        let temp = tempfile::tempdir().unwrap();
        let options = options(&temp);
        let error = start(&options).unwrap_err();
        assert!(error.to_string().contains("no service unit"), "{error}");

        std::fs::create_dir_all(&options.unit_dir).unwrap();
        std::fs::write(options.unit_path(), "[Unit]\n").unwrap();
        let error = stop(&options).unwrap_err();
        // Custom unit dir: the manager is never contacted.
        assert!(
            error.to_string().contains("unit file has no ExecStart"),
            "{error}"
        );
    }

    #[test]
    fn systemd_manager_is_a_silent_noop_when_manager_integration_is_disabled() {
        // A custom `--unit-dir` (as every update test uses) means the caller
        // owns the manager, so `SystemdManager` never shells out to systemctl
        // and every lifecycle call is `Ok`.
        let temp = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(options(&temp).data_root);
        let operation = LifecycleOperation::acquire(&config).unwrap();
        let manager = SystemdManager::new(options(&temp), &operation);
        assert!(manager.stop().is_ok());
        assert!(manager.reload().is_ok());
        assert!(manager.start().is_ok());
    }

    #[test]
    fn unit_check_verdicts_cover_absent_consistent_broken_mismatch_and_stale() {
        let temp = tempfile::tempdir().unwrap();
        let options = options(&temp);
        std::fs::create_dir_all(options.binary.parent().unwrap()).unwrap();
        std::fs::write(&options.binary, b"binary").unwrap();
        let current_binary = options.binary.clone();

        // Absent.
        assert_eq!(check_unit(&options, &current_binary), UnitVerdict::Absent);

        // Consistent.
        install(&options).unwrap();
        assert_eq!(
            check_unit(&options, &current_binary),
            UnitVerdict::Consistent
        );

        // Broken: the unit's binary disappeared.
        std::fs::remove_file(&options.binary).unwrap();
        assert!(matches!(
            check_unit(&options, &current_binary),
            UnitVerdict::Broken(_)
        ));

        // Stale: a different (existing) binary than the running one.
        std::fs::write(&options.binary, b"binary").unwrap();
        let foreign = temp.path().join("foreign-hiero");
        std::fs::write(&foreign, b"foreign").unwrap();
        assert_eq!(
            check_unit(&options, &foreign),
            UnitVerdict::StaleBinary {
                unit_binary: options.binary.clone()
            }
        );

        // Mismatched data root.
        let mut other = options.clone();
        other.data_root = temp.path().join("elsewhere");
        assert_eq!(
            check_unit(&other, &current_binary),
            UnitVerdict::DataRootMismatch {
                unit_root: options.data_root.clone()
            }
        );
    }
}
