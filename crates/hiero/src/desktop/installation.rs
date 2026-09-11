//! Package-scoped registration snapshot. Native rollback restores manager state, not just files.
use crate::{lifecycle::operation::LifecycleOperation, service::ServiceOptions};
#[cfg(target_os = "linux")]
use std::path::PathBuf;
pub(crate) struct Snapshot {
    service: ServiceOptions,
    #[cfg(target_os = "linux")]
    files: Vec<(PathBuf, Option<Vec<u8>>, Option<PathBuf>)>,
}
impl Snapshot {
    pub(crate) fn capture(
        service: &ServiceOptions,
        op: &LifecycleOperation,
    ) -> Result<Self, String> {
        op.register_unit(service).map_err(|e| e.to_string())?;
        #[cfg(target_os = "linux")]
        {
            let r = super::linux_registration::LinuxRegistration::new(service.clone());
            let paths = [
                service.unit_path(),
                service
                    .unit_dir
                    .join(super::linux_registration::RECORD_NAME),
                service
                    .unit_dir
                    .join("default.target.wants/hieronymus.service"),
                r.autostart_dir.join("hieronymus.desktop"),
                r.applications_dir.join("hieronymus.desktop"),
                r.icons_dir.join("hieronymus.svg"),
            ];
            let mut files = Vec::new();
            for p in paths {
                match std::fs::symlink_metadata(&p) {
                    Ok(m) if m.file_type().is_symlink() => files.push((
                        p.clone(),
                        None,
                        Some(std::fs::read_link(&p).map_err(|e| e.to_string())?),
                    )),
                    Ok(m) if m.is_file() && m.len() <= 65536 => files.push((
                        p.clone(),
                        Some(std::fs::read(&p).map_err(|e| e.to_string())?),
                        None,
                    )),
                    Ok(_) => return Err("Registration must be a bounded owned file".into()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        files.push((p, None, None))
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            Ok(Self {
                service: service.clone(),
                files,
            })
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            native(service, NativeAction::Capture)?;
            Ok(Self {
                service: service.clone(),
            })
        }
    }
    pub(crate) fn install(&self, op: &LifecycleOperation) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            super::linux_registration::LinuxRegistration::new(self.service.clone())
                .install_guarded(op)
        }
        #[cfg(target_os = "macos")]
        {
            op.register_unit(&self.service).map_err(|e| e.to_string())?;
            crate::platform::macos_broker::task(
                &self.service,
                crate::platform::macos_broker::TaskAction::InstallDesktop,
                true,
            )
            .map(|_| ())
        }
        #[cfg(windows)]
        {
            op.register_unit(&self.service).map_err(|e| e.to_string())?;
            crate::service::install_guarded(&self.service, op).map_err(|e| e.to_string())?;
            crate::platform::windows_broker::task(
                &self.service,
                crate::platform::windows_broker::TaskAction::Install,
                true,
            )
            .map(|_| ())
        }
    }
    pub(crate) fn restore(&self) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            for (path, bytes, link) in &self.files {
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
                if let Some(bytes) = bytes {
                    hieronymus::atomic::atomic_write(path, bytes).map_err(|e| e.to_string())?;
                }
                if let Some(target) = link {
                    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
                    std::os::unix::fs::symlink(target, path).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            native(&self.service, NativeAction::Restore)
        }
    }
    pub(crate) fn commit(&self) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            Ok(())
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            native(&self.service, NativeAction::Commit)
        }
    }
}
#[cfg(any(windows, target_os = "macos"))]
enum NativeAction {
    Capture,
    Restore,
    Commit,
}
#[cfg(any(windows, target_os = "macos"))]
fn native(options: &ServiceOptions, action: NativeAction) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    use crate::platform::macos_broker as broker;
    #[cfg(windows)]
    use crate::platform::windows_broker as broker;
    broker::task(
        options,
        match action {
            NativeAction::Capture => broker::TaskAction::PackageCapture,
            NativeAction::Restore => broker::TaskAction::PackageRestore,
            NativeAction::Commit => broker::TaskAction::PackageCommit,
        },
        true,
    )
    .map(|_| ())
}
