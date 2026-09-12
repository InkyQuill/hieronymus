//! Per-user Task Scheduler backend. No task End/Stop primitive is used: common
//! lifecycle owns authenticated shutdown; suppression disables future recovery.
use super::*;
use crate::platform::{
    windows_broker::{self, TaskAction},
    windows_identity,
};
use ::windows::{
    Win32::System::{
        Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        TaskScheduler::*,
        Variant::VARIANT,
    },
    core::BSTR,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const SERVICE_UNIT_NAME: &str = "hieronymus-task.json";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRecord {
    pub binary: PathBuf,
    pub data_root: PathBuf,
    pub unit_dir: PathBuf,
    pub sid: String,
    pub tray: bool,
    pub enabled: bool,
    pub recovery: bool,
}
impl TaskRecord {
    fn new(options: &ServiceOptions, tray: bool) -> Result<Self, String> {
        let result = Self {
            binary: options.binary.clone(),
            unit_dir: options
                .unit_dir
                .canonicalize()
                .unwrap_or_else(|_| options.unit_dir.clone()),
            data_root: options
                .data_root
                .canonicalize()
                .map_err(|_| "Task root is unavailable")?,
            sid: windows_identity::sid().map_err(|_| "Windows user identity is unavailable")?,
            tray,
            enabled: true,
            recovery: !tray,
        };
        result.xml()?;
        Ok(result)
    }
    pub fn xml(&self) -> Result<String, String> {
        if self.tray {
            return super::windows_task::render_login_in_directory(
                &self.binary,
                &self.data_root,
                &self.unit_dir,
                &self.sid,
                self.enabled,
            );
        }
        super::windows_task::render(
            &self.binary,
            &self.data_root,
            &self.sid,
            self.tray,
            self.enabled,
            self.recovery,
        )
    }
    pub fn name(&self) -> String {
        super::windows_task::name(&self.data_root, &self.sid, self.tray)
    }
    fn owned(&self, expected: &Self) -> Result<(), String> {
        if self.binary != expected.binary
            || self.data_root != expected.data_root
            || self.unit_dir != expected.unit_dir
            || self.sid != expected.sid
            || self.tray != expected.tray
        {
            return Err("Task registration belongs to another installation".into());
        }
        Ok(())
    }
}
pub fn default_unit_dir() -> PathBuf {
    home::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("AppData/Local/Hieronymus/tasks")
}
pub fn manager_enabled(options: &ServiceOptions) -> bool {
    options.use_manager
}
pub fn render_unit(binary: &Path, data_root: &Path) -> Result<String, ServiceError> {
    let record = TaskRecord::new(
        &ServiceOptions {
            binary: binary.into(),
            data_root: data_root.into(),
            unit_dir: default_unit_dir(),
            use_manager: false,
        },
        false,
    )
    .map_err(ServiceError::Invalid)?;
    serde_json::to_string(&record)
        .map_err(|_| ServiceError::Invalid("Could not encode native task".into()))
}
pub fn parse_unit(text: &str) -> Result<UnitDefinition, String> {
    let record: TaskRecord =
        serde_json::from_str(text).map_err(|_| "Invalid native task record")?;
    record.xml()?;
    Ok(UnitDefinition {
        binary: record.binary,
        data_root: record.data_root,
    })
}
fn record_path(options: &ServiceOptions, tray: bool) -> PathBuf {
    options.unit_dir.join(if tray {
        "hieronymus-tray-task.json"
    } else {
        SERVICE_UNIT_NAME
    })
}
fn read_record(options: &ServiceOptions, tray: bool) -> Result<Option<TaskRecord>, String> {
    read_json(&record_path(options, tray))
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not read native registration state".into()),
    };
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read native registration state")?;
    if bytes.len() > 65536 {
        return Err("Native registration state exceeds its bound".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "Invalid native registration state".into())
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
    let record = if options.use_manager {
        serde_json::from_value::<Option<TaskRecord>>(windows_broker::task_guarded(
            options,
            TaskAction::Inspect,
            false,
            operation,
        )?)
        .map_err(|_| "Invalid task readback")?
    } else {
        read_record(options, false)?
    };
    Ok(record.map(|r| UnitDefinition {
        binary: r.binary,
        data_root: r.data_root,
    }))
}
pub(crate) fn validate_unit_root_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<(), ServiceError> {
    operation.register_unit(options)?;
    if let Some(record) = read_record(options, false).map_err(ServiceError::Invalid)? {
        record
            .owned(&TaskRecord::new(options, false).map_err(ServiceError::Invalid)?)
            .map_err(ServiceError::Invalid)?;
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
        windows_broker::task_guarded(options, TaskAction::Install, false, operation)
            .map_err(ServiceError::Manager)?;
    } else {
        hieronymus::atomic::atomic_write_text(
            &options.unit_path(),
            &serde_json::to_string(
                &TaskRecord::new(options, false).map_err(ServiceError::Invalid)?,
            )
            .map_err(|_| ServiceError::Invalid("Could not encode native registration".into()))?,
        )?;
    }
    Ok(vec![
        "Windows daemon task installed for on-demand startup (not started)".into(),
    ])
}
pub(crate) fn uninstall_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Vec<String>, ServiceError> {
    operation.register_unit(options)?;
    validate_unit_root_guarded(options, operation)?;
    if options.use_manager {
        windows_broker::task_guarded(options, TaskAction::Remove, false, operation)
            .map_err(ServiceError::Manager)?;
    } else if options.unit_path().exists() {
        std::fs::remove_file(options.unit_path())?;
    }
    Ok(vec!["Windows daemon task removed".into()])
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
    windows_broker::task_guarded(options, TaskAction::Start, false, operation)
        .map_err(ServiceError::Manager)?;
    Ok(vec!["Windows daemon task started".into()])
}
pub(crate) fn rearm_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<(), ServiceError> {
    operation.register_unit(options)?;
    if options.use_manager {
        windows_broker::task_guarded(options, TaskAction::Rearm, false, operation)
            .map_err(ServiceError::Manager)?;
    }
    Ok(())
}
pub(crate) fn stop_guarded(
    options: &ServiceOptions,
    operation: &LifecycleOperation,
) -> Result<Vec<String>, ServiceError> {
    operation.register_unit(options)?;
    if options.use_manager {
        windows_broker::task_guarded(options, TaskAction::Suppress, false, operation)
            .map_err(ServiceError::Manager)?;
    }
    Ok(vec![
        "Windows daemon recovery disabled until explicit Start".into(),
    ])
}
// Legacy Linux-registration APIs remain callable but never pretend to manage Windows login.
pub(crate) fn owned_login_link(_: &ServiceOptions) -> Result<Option<PathBuf>, ServiceError> {
    Err(ServiceError::Invalid(
        "Windows login uses a separate native tray task".into(),
    ))
}
pub(crate) fn disable_login_guarded(
    _: &ServiceOptions,
    _: &LifecycleOperation,
) -> Result<(), ServiceError> {
    Err(ServiceError::Invalid(
        "Windows login uses a separate native tray task".into(),
    ))
}
/// Compatibility name for the updater's preserved three-operation seam.
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
        self.operation.register_unit(&self.options)?;
        if !self.options.use_manager {
            return Ok(());
        }
        install_guarded(&self.options, self.operation).map(|_| ())
    }
    fn start(&self) -> Result<(), ServiceError> {
        if !self.options.use_manager {
            return Ok(());
        }
        self.operation.register_unit(&self.options)?;
        validate_unit_root_guarded(&self.options, self.operation)?;
        crate::lifecycle::checked_probe(&HieronymusConfig::new(&self.options.data_root))
            .map_err(|e| ServiceError::Manager(e.to_string()))?;
        start_guarded(&self.options, self.operation).map(|_| ())
    }
}
struct Scheduler {
    service: Option<ITaskService>,
    folder: Option<ITaskFolder>,
}
impl Drop for Scheduler {
    fn drop(&mut self) {
        drop(self.folder.take());
        drop(self.service.take());
        unsafe {
            CoUninitialize();
        }
    }
}
impl Scheduler {
    fn connect() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|_| "Could not initialize native COM")?;
            let result = (|| {
                let service: ITaskService =
                    CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
                        .map_err(|_| "Task Scheduler is unavailable")?;
                let empty = VARIANT::default();
                service
                    .Connect(&empty, &empty, &empty, &empty)
                    .map_err(|_| "Could not connect to user Task Scheduler")?;
                let folder = service
                    .GetFolder(&BSTR::from("\\"))
                    .map_err(|_| "Could not open Task Scheduler folder")?;
                Ok::<_, &str>((service, folder))
            })();
            match result {
                Ok((service, folder)) => Ok(Self {
                    service: Some(service),
                    folder: Some(folder),
                }),
                Err(error) => {
                    CoUninitialize();
                    Err(error.into())
                }
            }
        }
    }
    fn folder(&self) -> &ITaskFolder {
        self.folder.as_ref().unwrap()
    }
    fn task(&self, name: &str) -> Result<Option<IRegisteredTask>, String> {
        match unsafe { self.folder().GetTask(&BSTR::from(name)) } {
            Ok(task) => Ok(Some(task)),
            Err(e) if e.code().0 as u32 == 0x80070002 => Ok(None),
            Err(_) => Err("Could not inspect native task".into()),
        }
    }
    fn xml(&self, name: &str) -> Result<Option<String>, String> {
        self.task(name)?
            .map(|t| {
                unsafe { t.Xml() }
                    .map(|s| s.to_string())
                    .map_err(|_| "Could not read native task definition".into())
            })
            .transpose()
    }
    fn put(&self, record: &TaskRecord, exists: bool) -> Result<(), String> {
        unsafe {
            self.folder()
                .RegisterTask(
                    &BSTR::from(record.name()),
                    &BSTR::from(record.xml()?),
                    if exists { TASK_UPDATE.0 } else { TASK_CREATE.0 },
                    &VARIANT::from(record.sid.as_str()),
                    &VARIANT::default(),
                    TASK_LOGON_INTERACTIVE_TOKEN,
                    &VARIANT::default(),
                )
                .map_err(|_| "Could not register native interactive-user task")?;
        }
        Ok(())
    }
    fn remove(&self, name: &str) -> Result<(), String> {
        unsafe { self.folder().DeleteTask(&BSTR::from(name), 0) }
            .map_err(|_| "Could not remove owned native task".into())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    before: Option<TaskRecord>,
    after: Option<TaskRecord>,
}
fn save(path: &Path, record: &Option<TaskRecord>) -> Result<(), String> {
    match record {
        Some(record) => hieronymus::atomic::atomic_write(
            path,
            &serde_json::to_vec(record).map_err(|_| "Could not encode native record")?,
        )
        .map_err(|_| "Could not persist native registration".into()),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not remove native registration record".into()),
        },
    }
}
fn canonical_task_xml(scheduler: &Scheduler, xml: &str) -> Result<String, String> {
    // Both documents pass through the SAME native schema/default serializer.
    // Complete tree comparison below still rejects extra security/execution fields.
    unsafe {
        let service = scheduler.service.as_ref().expect("connected scheduler");
        let definition = service
            .NewTask(0)
            .map_err(|_| "Could not create task definition parser")?;
        definition
            .SetXmlText(&BSTR::from(xml))
            .map_err(|_| "Native task XML is invalid")?;
        let mut canonical = BSTR::new();
        definition
            .XmlText(&mut canonical)
            .map_err(|_| "Could not serialize native task definition")?;
        let canonical = canonical.to_string();
        // Scheduler persistence can spell a LogonTrigger user as DOMAIN\name
        // even when registered with a SID. Normalize only that identity text;
        // preserve every element, attribute and duplicate for full comparison.
        let doc =
            roxmltree::Document::parse(&canonical).map_err(|_| "Invalid normalized task XML")?;
        let mut identities = Vec::new();
        for trigger in doc.descendants().filter(|n| n.has_tag_name("LogonTrigger")) {
            for user in trigger.children().filter(|n| n.has_tag_name("UserId")) {
                for text in user.children().filter(|n| n.is_text()) {
                    let value = text.text().unwrap_or_default();
                    if !value.starts_with("S-1-") {
                        let sid = crate::platform::windows_identity::account_sid(value)
                            .map_err(|_| "Could not resolve native task trigger identity")?;
                        identities.push((text.range(), sid));
                    }
                }
            }
        }
        let mut normalized = canonical.clone();
        for (range, sid) in identities.into_iter().rev() {
            normalized.replace_range(range, &sid);
        }
        Ok(normalized)
    }
}
fn matches(
    scheduler: &Scheduler,
    xml: &Option<String>,
    record: &Option<TaskRecord>,
) -> Result<bool, String> {
    match (xml, record) {
        (None, None) => Ok(true),
        (Some(xml), Some(record)) => {
            // API/schema failures are errors, never evidence of foreign ownership.
            let actual = canonical_task_xml(scheduler, xml)?;
            let expected = canonical_task_xml(scheduler, &record.xml()?)?;
            super::windows_task::same_definition(&actual, &expected)
        }
        _ => Ok(false),
    }
}

fn rollback(
    scheduler: &Scheduler,
    expected: &TaskRecord,
    path: &Path,
    journal: &Journal,
) -> Result<(), String> {
    for record in [&journal.before, &journal.after].into_iter().flatten() {
        record.owned(expected)?;
    }
    let actual = scheduler.xml(&expected.name())?;
    if !matches(scheduler, &actual, &journal.before)?
        && !matches(scheduler, &actual, &journal.after)?
    {
        return Err(
            "Pending task transaction conflicts with external changes; no task was overwritten"
                .into(),
        );
    }
    if !matches(scheduler, &actual, &journal.before)? {
        match &journal.before {
            Some(record) => scheduler.put(record, actual.is_some())?,
            None => scheduler.remove(&expected.name())?,
        }
    }
    if !matches(
        scheduler,
        &scheduler.xml(&expected.name())?,
        &journal.before,
    )? {
        return Err("Native registration rollback readback failed".into());
    }
    save(path, &journal.before)
}
/// Called only by the committed broker while both continuation gates are held.
pub(crate) fn execute(
    options: &ServiceOptions,
    action: TaskAction,
    tray: bool,
) -> Result<Value, String> {
    let package = options.unit_dir.join(".hieronymus-package-pending.json");
    match action {
        TaskAction::PackageCapture => {
            if package.exists() {
                return Err("A prior package registration rollback is pending".into());
            }
            super::package_preflight::require_settled([
                record_path(options, false).with_extension("pending.json"),
                record_path(options, true).with_extension("pending.json"),
            ])?;
            let before = [
                execute(options, TaskAction::Inspect, false)?,
                execute(options, TaskAction::Inspect, true)?,
            ];
            hieronymus::private_file::create_private_new(
                &package,
                &serde_json::to_vec(&before).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            return Ok(Value::Null);
        }
        TaskAction::PackageRestore => {
            let before: [Option<TaskRecord>; 2] = serde_json::from_slice(
                &hieronymus::private_file::read_private(&package).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let scheduler = Scheduler::connect()?;
            for (tray, before) in [false, true].into_iter().zip(before) {
                let expected = TaskRecord::new(options, tray)?;
                let after: Option<TaskRecord> =
                    serde_json::from_value(execute(options, TaskAction::Inspect, tray)?)
                        .map_err(|e| e.to_string())?;
                rollback(
                    &scheduler,
                    &expected,
                    &record_path(options, tray),
                    &Journal { before, after },
                )?;
            }
            // Outer rollback commits only after explicit prior-active restart.
            return Ok(Value::Null);
        }
        TaskAction::PackageCommit => {
            std::fs::remove_file(package).map_err(|e| e.to_string())?;
            return Ok(Value::Null);
        }
        _ => {}
    }
    let expected = TaskRecord::new(options, tray)?;
    let scheduler = Scheduler::connect()?;
    let path = record_path(options, tray);
    let pending = path.with_extension("pending.json");
    if let Some(journal) = read_json::<Journal>(&pending)? {
        if matches!(action, TaskAction::Inspect) {
            // Passive readback must never repair/mutate before caller authentication.
            for record in [&journal.before, &journal.after].into_iter().flatten() {
                record.owned(&expected)?;
            }
            let actual = scheduler.xml(&expected.name())?;
            let active = if matches(&scheduler, &actual, &journal.before)? {
                &journal.before
            } else if matches(&scheduler, &actual, &journal.after)? {
                &journal.after
            } else {
                return Err("Pending task transaction conflicts with external changes".into());
            };
            return serde_json::to_value(active)
                .map_err(|_| "Could not encode pending task readback".into());
        }
        rollback(&scheduler, &expected, &path, &journal)?;
        std::fs::remove_file(&pending).map_err(|_| "Could not clear recovered task transaction")?;
    }
    let mut before = read_record(options, tray)?;
    if let Some(record) = &before {
        record.owned(&expected)?;
    }
    let xml = scheduler.xml(&expected.name())?;
    // Native enabled state is the login preference authority, including an OS UI toggle.
    if let (Some(record), Some(xml)) = (&mut before, &xml) {
        // Registered XML may omit the default Enabled=true. Let the connected
        // native parser materialize defaults before reading the enabled state;
        // complete definition/ownership comparison still follows below.
        let normalized = canonical_task_xml(&scheduler, xml)?;
        let doc = roxmltree::Document::parse(&normalized).map_err(|_| "Invalid task readback")?;
        let enabled = doc
            .descendants()
            .find(|n| n.has_tag_name("Settings"))
            .and_then(|n| n.children().find(|n| n.has_tag_name("Enabled")))
            .and_then(|n| n.text());
        record.enabled = match enabled {
            Some("true") => true,
            Some("false") => false,
            _ => return Err("Native task has invalid enabled state".into()),
        };
    }
    if !matches(&scheduler, &xml, &before)? {
        return Err(
            "Native task is foreign, missing or modified; registration was not changed".into(),
        );
    }
    if matches!(action, TaskAction::Inspect) {
        return serde_json::to_value(before).map_err(|_| "Could not encode task readback".into());
    }
    let mut after = before.clone().unwrap_or(expected.clone());
    match action {
        TaskAction::Install => {}
        TaskAction::Start | TaskAction::Rearm => {
            after.enabled = true;
            after.recovery = !tray;
        }
        TaskAction::Suppress => {
            after.enabled = false;
            after.recovery = false;
        }
        TaskAction::EnableLogin => {
            if !tray {
                return Err("Login operation requires a tray task".into());
            }
            after.enabled = true;
        }
        TaskAction::DisableLogin => {
            if !tray {
                return Err("Login operation requires a tray task".into());
            }
            after.enabled = false;
        }
        TaskAction::Remove | TaskAction::Inspect => {}
        TaskAction::PackageCapture | TaskAction::PackageRestore | TaskAction::PackageCommit => {
            unreachable!()
        }
    }
    if matches!(action, TaskAction::Remove | TaskAction::Suppress) && before.is_none() {
        return Ok(Value::Null);
    }
    let after = if matches!(action, TaskAction::Remove) {
        None
    } else {
        Some(after)
    };
    let journal = Journal { before, after };
    hieronymus::atomic::atomic_write(
        &pending,
        &serde_json::to_vec(&journal).map_err(|_| "Could not encode task transaction")?,
    )
    .map_err(|_| "Could not persist task transaction")?;
    let result = (|| {
        match &journal.after {
            Some(record) => scheduler.put(record, xml.is_some())?,
            None => scheduler.remove(&expected.name())?,
        }
        if !matches(
            &scheduler,
            &scheduler.xml(&expected.name())?,
            &journal.after,
        )? {
            return Err("Native task registration readback failed".into());
        }
        save(&path, &journal.after)
    })();
    if let Err(error) = result {
        rollback(&scheduler, &expected, &path, &journal).map_err(|_| "Native task mutation failed and rollback is pending; retry only after inspecting registration")?;
        std::fs::remove_file(&pending)
            .map_err(|_| "Native task rollback completed but its journal remains")?;
        return Err(error);
    }
    std::fs::remove_file(&pending)
        .map_err(|_| "Native task registered but its transaction journal remains")?;
    if matches!(action, TaskAction::Start) {
        let task = scheduler
            .task(&expected.name())?
            .ok_or("Registered daemon task disappeared")?;
        unsafe { task.Run(&VARIANT::default()) }.map_err(
            |_| "Native task could not start; recovery remains armed for this explicit Start",
        )?;
    }
    serde_json::to_value(journal.after).map_err(|_| "Could not encode native result".into())
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
        let assert_blocked = |reason: &str| {
            let attempts = windows_broker::SPAWN_ATTEMPTS.get();
            assert!(read_unit(&options).unwrap_err().contains(reason));
            assert!(
                super::super::status(&options)
                    .unwrap_err()
                    .to_string()
                    .contains(reason)
            );
            assert!(
                matches!(super::super::check_unit(&options, &options.binary),
                UnitVerdict::Broken(message) if message.contains(reason))
            );
            assert!(
                windows_broker::inspect(&options, false)
                    .unwrap_err()
                    .contains(reason)
            );
            assert_eq!(windows_broker::SPAWN_ATTEMPTS.get(), attempts);
        };
        let held = LifecycleOperation::acquire(&HieronymusConfig::new(&options.data_root)).unwrap();
        assert_blocked("another lifecycle operation");
        drop(held);
        // A different root can still contend for the same registration directory.
        let other = ServiceOptions {
            data_root: temp.path().join("other-root"),
            ..options.clone()
        };
        let held = LifecycleOperation::acquire(&HieronymusConfig::new(&other.data_root)).unwrap();
        held.register_unit(&other).unwrap();
        assert_blocked("another operation holds this service registration");
        drop(held);
        let local = ServiceOptions {
            use_manager: false,
            ..options
        };
        let held = LifecycleOperation::acquire(&HieronymusConfig::new(&local.data_root)).unwrap();
        assert_eq!(read_unit_guarded(&local, &held).unwrap(), None);
        assert_eq!(read_unit_guarded(&local, &held).unwrap(), None);
        validate_unit_root_guarded(&local, &held).unwrap();
    }
    #[test]
    fn native_connected_normalization_distinguishes_invalid_xml_from_foreign_definition() {
        let temp = tempfile::tempdir().unwrap();
        let options = ServiceOptions {
            data_root: temp.path().to_path_buf(),
            unit_dir: temp.path().to_path_buf(),
            binary: std::env::current_exe().unwrap(),
            use_manager: true,
        };
        let record = TaskRecord::new(&options, false).unwrap();
        let scheduler = Scheduler::connect().unwrap();
        assert!(
            matches(
                &scheduler,
                &Some(record.xml().unwrap()),
                &Some(record.clone())
            )
            .unwrap()
        );
        let mut changed = record.clone();
        changed.recovery = false;
        assert!(
            !matches(
                &scheduler,
                &Some(changed.xml().unwrap()),
                &Some(record.clone())
            )
            .unwrap()
        );
        assert_eq!(
            matches(&scheduler, &Some("<invalid".into()), &Some(record)).unwrap_err(),
            "Native task XML is invalid"
        );
    }
    struct DisposableTask {
        record: TaskRecord,
    }
    impl Drop for DisposableTask {
        fn drop(&mut self) {
            // This name was created exclusively from this test's fresh TempDir.
            if let Ok(scheduler) = Scheduler::connect()
                && scheduler.task(&self.record.name()).ok().flatten().is_some()
            {
                let _ = scheduler.remove(&self.record.name());
            }
        }
    }
    #[test]
    fn native_package_capture_refuses_pending_before_after_and_absence_then_accepts_recovery() {
        for actual_side in [0, 1, 2] {
            for tray in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let options = ServiceOptions {
                    data_root: temp.path().join("root"),
                    unit_dir: temp.path().join("units"),
                    binary: std::env::current_exe().unwrap(),
                    use_manager: true,
                };
                std::fs::create_dir_all(&options.data_root).unwrap();
                std::fs::create_dir_all(&options.unit_dir).unwrap();
                let before = TaskRecord::new(&options, tray).unwrap();
                let _cleanup = DisposableTask {
                    record: before.clone(),
                };
                let scheduler = Scheduler::connect().unwrap();
                assert!(scheduler.task(&before.name()).unwrap().is_none());
                let mut after = before.clone();
                after.enabled = false;
                after.recovery = false;
                let actual = match actual_side {
                    0 => Some(before.clone()),
                    1 => Some(after.clone()),
                    _ => None,
                };
                if let Some(actual) = &actual {
                    scheduler.put(actual, false).unwrap();
                }
                let path = record_path(&options, tray);
                save(&path, &actual).unwrap();
                let pending = path.with_extension("pending.json");
                let journal = Journal {
                    before: Some(before.clone()),
                    after: if actual_side == 2 { None } else { Some(after) },
                };
                let bytes = serde_json::to_vec(&journal).unwrap();
                hieronymus::atomic::atomic_write(&pending, &bytes).unwrap();
                let record_bytes = std::fs::read(&path).ok();
                let xml = scheduler.xml(&before.name()).unwrap();
                execute(&options, TaskAction::Inspect, tray).unwrap();
                assert!(
                    execute(&options, TaskAction::PackageCapture, true)
                        .unwrap_err()
                        .contains("recovery is pending")
                );
                assert!(
                    !options
                        .unit_dir
                        .join(".hieronymus-package-pending.json")
                        .exists()
                );
                assert_eq!(std::fs::read(&pending).unwrap(), bytes);
                assert_eq!(std::fs::read(&path).ok(), record_bytes);
                assert_eq!(scheduler.xml(&before.name()).unwrap(), xml);
                execute(&options, TaskAction::Install, tray).unwrap();
                assert!(!pending.exists());
                execute(&options, TaskAction::PackageCapture, true).unwrap();
                let captured: [Option<TaskRecord>; 2] = serde_json::from_slice(
                    &hieronymus::private_file::read_private(
                        &options.unit_dir.join(".hieronymus-package-pending.json"),
                    )
                    .unwrap(),
                )
                .unwrap();
                assert_eq!(captured[usize::from(tray)], Some(before));
                execute(&options, TaskAction::PackageCommit, true).unwrap();
            }
        }
    }

    #[test]
    fn native_pending_registration_recovers_exact_prior_definition_and_rejects_foreign_task() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let units = temp.path().join("units");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&units).unwrap();
        let options = ServiceOptions {
            data_root: root,
            unit_dir: units,
            binary: std::env::current_exe().unwrap(),
            use_manager: true,
        };
        let before = TaskRecord::new(&options, false).unwrap();
        let _cleanup = DisposableTask {
            record: before.clone(),
        };
        let scheduler = Scheduler::connect().unwrap();
        assert!(scheduler.task(&before.name()).unwrap().is_none());
        scheduler.put(&before, false).unwrap();
        let path = record_path(&options, false);
        save(&path, &Some(before.clone())).unwrap();
        {
            let mut read_options = options.clone();
            read_options.use_manager = false;
            let parent =
                LifecycleOperation::acquire(&HieronymusConfig::new(&options.data_root)).unwrap();
            assert!(
                read_unit(&read_options).is_err(),
                "public path must not reacquire the held parent"
            );
            assert!(read_unit_guarded(&read_options, &parent).unwrap().is_some());
        }
        // Successful offline replacement never suppresses an already stopped task.
        // Native Install preserves its enabled/recovery state without starting it.
        execute(&options, TaskAction::PackageCapture, true).unwrap();
        execute(&options, TaskAction::Install, false).unwrap();
        execute(&options, TaskAction::PackageCommit, true).unwrap();
        assert_eq!(read_record(&options, false).unwrap().unwrap(), before);
        assert_ne!(
            unsafe {
                scheduler
                    .task(&before.name())
                    .unwrap()
                    .unwrap()
                    .State()
                    .unwrap()
            },
            TASK_STATE_RUNNING
        );
        // Package rollback starts from an idle but enabled authoritative task.
        // Suppress mutates both native XML and the record; capture must precede it.
        assert_ne!(
            unsafe {
                scheduler
                    .task(&before.name())
                    .unwrap()
                    .unwrap()
                    .State()
                    .unwrap()
            },
            TASK_STATE_RUNNING
        );
        execute(&options, TaskAction::PackageCapture, true).unwrap();
        execute(&options, TaskAction::Suppress, false).unwrap();
        assert!(!read_record(&options, false).unwrap().unwrap().enabled);
        execute(&options, TaskAction::PackageRestore, true).unwrap();
        assert_eq!(read_record(&options, false).unwrap().unwrap(), before);
        assert!(
            matches(
                &scheduler,
                &scheduler.xml(&before.name()).unwrap(),
                &Some(before.clone())
            )
            .unwrap()
        );
        assert_ne!(
            unsafe {
                scheduler
                    .task(&before.name())
                    .unwrap()
                    .unwrap()
                    .State()
                    .unwrap()
            },
            TASK_STATE_RUNNING
        );
        execute(&options, TaskAction::PackageCommit, true).unwrap();
        let mut after = before.clone();
        after.enabled = false;
        after.recovery = false;
        let journal = Journal {
            before: Some(before.clone()),
            after: Some(after.clone()),
        };
        hieronymus::atomic::atomic_write(
            &path.with_extension("pending.json"),
            &serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        scheduler.put(&after, true).unwrap();
        assert!(
            matches(
                &scheduler,
                &scheduler.xml(&before.name()).unwrap(),
                &Some(after)
            )
            .unwrap()
        );
        // Production interrupted-registration recovery performs real scheduler rollback.
        let pending_readback = execute(&options, TaskAction::Inspect, false).unwrap();
        assert_eq!(pending_readback["enabled"], false);
        assert!(path.with_extension("pending.json").exists());
        let readback = execute(&options, TaskAction::Install, false).unwrap();
        assert_eq!(
            serde_json::from_value::<TaskRecord>(readback).unwrap(),
            before
        );
        assert!(
            matches(
                &scheduler,
                &scheduler.xml(&before.name()).unwrap(),
                &Some(before.clone())
            )
            .unwrap()
        );
        assert!(!path.with_extension("pending.json").exists());
        let mut foreign = before.clone();
        foreign.binary = temp.path().join("foreign.exe");
        std::fs::write(&foreign.binary, b"fixture").unwrap();
        scheduler.put(&foreign, true).unwrap();
        assert!(execute(&options, TaskAction::Install, false).is_err());
        assert!(
            matches(
                &scheduler,
                &scheduler.xml(&before.name()).unwrap(),
                &Some(foreign)
            )
            .unwrap()
        );
    }
}
