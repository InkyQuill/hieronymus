//! `hiero uninstall` (distribution spec §Installer): removes the service
//! unit, the managed application directory (versioned binaries plus stable
//! command links), the owner's PATH links that point into it, and the
//! generated agent-plugin entries — and nothing else. Databases,
//! configuration, models, backups, and audit data are preserved by default;
//! clearing user contents requires the separate explicit `--delete-data`
//! action. The root and its recognized coordination lock files remain.
//!
//! Confirmation is mandatory: either `--yes` or an interactive prompt owned
//! by the CLI layer. The library refuses to run unconfirmed.

use std::path::{Path, PathBuf};

use crate::lifecycle::{
    self,
    operation::{LIFECYCLE_LOCK_FILE, LifecycleOperation},
};
use hieronymus::data_root::load_config;
use hieronymus::ownership::{OWNER_LOCK_FILE, RootOwnership};

use crate::app::AppLayout;
#[cfg(unix)]
use crate::app::LINK_NAMES;
use crate::service::{self, ServiceOptions};

#[derive(Debug, Clone)]
pub struct UninstallOptions {
    /// The managed application root; derived from the running binary when
    /// omitted (developer builds must pass it).
    pub app_dir: Option<PathBuf>,
    pub data_root: Option<PathBuf>,
    /// Service unit directory override (`None` → the systemd user default).
    pub unit_dir: Option<PathBuf>,
    /// The caller confirmed (CLI `--yes` or interactive prompt).
    pub confirmed: bool,
    /// Separate explicit data deletion; never implied by a plain uninstall.
    pub delete_data: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallReport {
    /// What was removed, one entry per item.
    pub removed: Vec<String>,
    /// What was deliberately kept, one entry per item.
    pub preserved: Vec<String>,
    /// The exact data root, present in the report before any deletion.
    pub data_root: PathBuf,
    pub data_deleted: bool,
}

impl UninstallReport {
    pub fn render_human(&self) -> String {
        let mut text = String::new();
        for entry in &self.removed {
            text.push_str(&format!("removed: {entry}\n"));
        }
        for entry in &self.preserved {
            text.push_str(&format!("preserved: {entry}\n"));
        }
        if self.data_deleted {
            text.push_str(&format!(
                "deleted user data from: {} (coordination files retained)\n",
                self.data_root.display()
            ));
        } else {
            text.push_str(&format!(
                "data root preserved: {}\n",
                self.data_root.display()
            ));
        }
        text
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "removed": self.removed,
            "preserved": self.preserved,
            "data_root": self.data_root,
            "data_deleted": self.data_deleted,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UninstallError {
    #[error("uninstall refused (nothing was changed): {0}")]
    Refused(String),
    #[error("service error: {0}")]
    Service(#[from] service::ServiceError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The full uninstall flow. Assumes `options.confirmed` was already checked
/// by the CLI layer.
pub fn run_uninstall(options: &UninstallOptions) -> Result<UninstallReport, UninstallError> {
    if !options.confirmed {
        return Err(UninstallError::Refused(
            "uninstall requires confirmation; rerun with --yes (or answer the prompt)".to_string(),
        ));
    }
    let mut removed = Vec::new();

    let config = load_config(options.data_root.as_deref());
    let operation = LifecycleOperation::acquire(&config)?;

    // Managed application directory (versioned binaries + stable links).
    let root = match &options.app_dir {
        Some(directory) => directory.clone(),
        None => AppLayout::detect_from_exe().map_err(UninstallError::Refused)?,
    };
    let layout = AppLayout::new(&root);
    #[cfg(windows)]
    if layout.root().try_exists()?
        && std::env::current_exe()?
            .canonicalize()?
            .starts_with(layout.root().canonicalize()?)
    {
        return Err(UninstallError::Refused("Windows cannot remove the executing installed CLI. Run install-desktop.ps1 -Uninstall from the verified release directory; it uses an external bootstrap executable".into()));
    }
    if layout.root().exists() {
        if config
            .data_root()
            .canonicalize()?
            .starts_with(layout.root().canonicalize()?)
        {
            return Err(UninstallError::Refused(
                "the data root is inside the application directory; move it outside before uninstalling so coordination locks and user data are preserved".into(),
            ));
        }
        // Safety: only remove a directory that has the managed layout.
        if !layout.versions_dir().is_dir() && !layout.bin_dir().is_dir() {
            return Err(UninstallError::Refused(format!(
                "{} does not look like a managed Hieronymus application directory \
                 (no versions/ or bin/); refusing to remove it",
                layout.root().display()
            )));
        }
    }

    // Service unit first, so nothing can restart the daemon mid-uninstall.
    let service_options = ServiceOptions {
        data_root: config.data_root().to_path_buf(),
        unit_dir: options
            .unit_dir
            .clone()
            .unwrap_or_else(service::default_unit_dir),
        binary: layout.stable_link("hiero"),
        use_manager: true,
    };
    operation.register_unit(&service_options)?;
    let registration_dir = service_options.unit_dir.canonicalize()?;
    if (layout.root().exists() && registration_dir.starts_with(layout.root().canonicalize()?))
        || (config.agent_plugins_root().exists()
            && registration_dir.starts_with(config.agent_plugins_root().canonicalize()?))
        || (options.delete_data && registration_dir.starts_with(config.data_root().canonicalize()?))
    {
        return Err(UninstallError::Refused(
            "the service registration directory is inside a directory being removed; move the registration outside before uninstalling so its coordination lock is preserved".into(),
        ));
    }
    #[cfg(target_os = "linux")]
    let desktop_registration =
        crate::desktop::linux_registration::for_uninstall(&service_options, layout.root())
            .map_err(UninstallError::Refused)?;
    #[cfg(windows)]
    crate::platform::windows_broker::task(
        &service_options,
        crate::platform::windows_broker::TaskAction::Inspect,
        true,
    )
    .map_err(UninstallError::Refused)?;
    #[cfg(target_os = "macos")]
    crate::platform::macos_broker::task(
        &service_options,
        crate::platform::macos_broker::TaskAction::Inspect,
        true,
    )
    .map_err(UninstallError::Refused)?;
    let _retirement = crate::desktop::control::Retirement::begin_owned(
        &config,
        true,
        Some(&layout.stable_link("hiero")),
    )
    .map_err(|e| UninstallError::Refused(e.to_string()))?;
    lifecycle::stop_guarded(&config, &service_options, &operation)
        .map_err(|error| UninstallError::Refused(error.to_string()))?;
    // Keep daemon ownership throughout every offline removal. Neither held
    // coordination inode is ever unlinked, even with --delete-data.
    let _ownership = RootOwnership::acquire(&config, "uninstall")?;
    #[cfg(target_os = "linux")]
    if let Some(registration) = desktop_registration {
        registration
            .uninstall_guarded(&operation)
            .map_err(UninstallError::Refused)?;
        removed.push("owned desktop login entry, launcher, icon and registration record".into());
    }
    #[cfg(windows)]
    crate::platform::windows_broker::task(
        &service_options,
        crate::platform::windows_broker::TaskAction::Remove,
        true,
    )
    .map_err(UninstallError::Refused)?;
    #[cfg(target_os = "macos")]
    crate::platform::macos_broker::task(
        &service_options,
        crate::platform::macos_broker::TaskAction::Remove,
        true,
    )
    .map_err(UninstallError::Refused)?;
    let lines = service::uninstall_guarded(&service_options, &operation)?;
    removed.extend(lines);

    if layout.root().exists() {
        std::fs::remove_dir_all(layout.root())?;
        removed.push(format!(
            "application directory (binaries and command links): {}",
            layout.root().display()
        ));
    } else {
        removed.push("application directory already absent".to_string());
    }

    // PATH links owned by the bootstrap installer, but only when they point
    // into the application directory being removed: a foreign `hiero` on PATH
    // is host configuration and is preserved.
    #[cfg(unix)]
    if let Some(bin_dir) = home::home_dir().map(|home| home.join(".local").join("bin")) {
        for name in LINK_NAMES {
            let link = bin_dir.join(name);
            if link.symlink_metadata().is_err() {
                continue;
            }
            let points_into_app = std::fs::read_link(&link)
                .map(|target| resolve_into(&link, &target, layout.root()))
                .unwrap_or(false);
            if points_into_app {
                std::fs::remove_file(&link)?;
                removed.push(format!("PATH link: {}", link.display()));
            }
        }
    }

    // Generated agent-plugin entries (the installers' generated surface).
    let plugins = config.agent_plugins_root();
    if plugins.is_dir() {
        std::fs::remove_dir_all(&plugins)?;
        removed.push(format!("generated agent plugins: {}", plugins.display()));
    }

    // Preservation ledger: everything under the data root except the
    // generated entries removed above.
    let mut preserved = Vec::new();
    let database = config.database_path();
    if database.exists() {
        preserved.push(format!("database: {}", database.display()));
    }
    for (label, path) in [
        ("backups", config.backups_root()),
        ("configuration", config.config_root().to_path_buf()),
        ("semantic models and index", config.semantic_root()),
    ] {
        if path.exists() {
            preserved.push(format!("{label}: {}", path.display()));
        }
    }

    // Data deletion is a separate explicit action that names the exact root.
    let mut data_deleted = false;
    if options.delete_data {
        preserved = delete_data_contents(config.data_root())?;
        data_deleted = true;
    }

    Ok(UninstallReport {
        removed,
        preserved,
        data_root: config.data_root().to_path_buf(),
        data_deleted,
    })
}

/// Preserve only implementation-defined coordination names, including sessions
/// from other supported platforms when a data root has moved between hosts.
fn is_coordination_name(name: &str) -> bool {
    matches!(
        name,
        OWNER_LOCK_FILE
            | LIFECYCLE_LOCK_FILE
            | ".desktop-launch.lock"
            | "dream-cycle.lock"
            | ".windows-native.lock"
            | ".windows-browser.lock"
            | ".macos-native.lock"
            | ".macos-browser.lock"
    ) || crate::desktop::is_session_lock_name(name)
}

fn delete_data_contents(root: &Path) -> std::io::Result<Vec<String>> {
    let mut retained = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_str().is_some_and(is_coordination_name) {
            retained.push(format!("coordination file: {}", name.to_string_lossy()));
            continue;
        }
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    retained.sort();
    Ok(retained)
}

/// Whether `link` (a symlink at `link_path` with raw `target`) eventually
/// resolves under `root`.
#[cfg(unix)]
fn resolve_into(link_path: &Path, target: &Path, root: &Path) -> bool {
    let base = if target.is_absolute() {
        target.to_path_buf()
    } else {
        link_path.parent().unwrap_or(Path::new("/")).join(target)
    };
    // Lexical containment first, so a link is still recognized when its
    // application directory has already been removed (dangling link).
    if base.starts_with(root) {
        return true;
    }
    base.canonicalize()
        .map(|resolved| resolved.starts_with(root))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hieronymus::data_root::HieronymusConfig;

    fn options(temp: &tempfile::TempDir, confirmed: bool, delete_data: bool) -> UninstallOptions {
        UninstallOptions {
            app_dir: Some(temp.path().join("app")),
            data_root: Some(temp.path().join("data")),
            unit_dir: Some(temp.path().join("units")),
            confirmed,
            delete_data,
        }
    }

    fn seed_install(temp: &tempfile::TempDir) -> HieronymusConfig {
        // Managed application directory with one version and stable links.
        let layout = AppLayout::new(temp.path().join("app"));
        std::fs::create_dir_all(layout.version_dir("1.0.0")).unwrap();
        std::fs::write(layout.version_dir("1.0.0").join("hiero"), b"binary").unwrap();
        layout.switch_stable_links("1.0.0").unwrap();
        // Service unit + generated plugins + user data.
        let config = HieronymusConfig::new(temp.path().join("data"));
        std::fs::create_dir_all(temp.path().join("units")).unwrap();
        let service_options = ServiceOptions {
            data_root: config.data_root().to_path_buf(),
            unit_dir: temp.path().join("units"),
            binary: layout.version_dir("1.0.0").join("hiero"),
            use_manager: false,
        };
        service::install(&service_options).unwrap();
        std::fs::create_dir_all(config.agent_plugins_root()).unwrap();
        std::fs::write(config.database_path(), b"database bytes").unwrap();
        std::fs::create_dir_all(config.backups_root().join("pre-upgrade")).unwrap();
        config
    }

    #[test]
    fn uninstall_requires_confirmation() {
        let temp = tempfile::tempdir().unwrap();
        let error = run_uninstall(&options(&temp, false, false)).unwrap_err();
        assert!(error.to_string().contains("confirmation"), "{error}");
        // Nothing was touched by the refusal.
        assert!(temp.path().exists());
    }

    #[test]
    fn uninstall_removes_software_and_generated_entries_but_preserves_data() {
        let temp = tempfile::tempdir().unwrap();
        let config = seed_install(&temp);

        let report = run_uninstall(&options(&temp, true, false)).unwrap();
        assert!(
            report
                .removed
                .iter()
                .any(|entry| entry.contains("application directory"))
        );
        assert!(report.removed.iter().any(|entry| entry.contains("unit")));
        assert!(report.removed.iter().any(|entry| entry.contains("plugins")));
        assert!(!AppLayout::new(temp.path().join("app")).root().exists());
        assert!(!temp.path().join("units/hieronymus.service").exists());
        assert!(!config.agent_plugins_root().exists());

        // The preservation ledger names the data the spec protects.
        assert!(
            report
                .preserved
                .iter()
                .any(|entry| entry.contains("database"))
        );
        assert!(
            report
                .preserved
                .iter()
                .any(|entry| entry.contains("backups"))
        );
        assert!(config.database_path().exists());
        assert!(config.backups_root().exists());
        assert!(!report.data_deleted);
        assert!(report.render_human().contains("data root preserved"));
    }

    #[test]
    fn delete_data_is_a_separate_explicit_action_naming_the_root() {
        let temp = tempfile::tempdir().unwrap();
        let config = seed_install(&temp);
        let report = run_uninstall(&options(&temp, true, true)).unwrap();
        assert!(report.data_deleted);
        assert_eq!(report.data_root, config.data_root());
        let mut retained: Vec<_> = std::fs::read_dir(config.data_root())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        retained.sort();
        assert_eq!(
            retained,
            [".desktop-launch.lock", ".lifecycle.lock", ".owner.lock"]
        );
        assert!(
            report
                .preserved
                .iter()
                .any(|entry| entry.contains("coordination"))
        );
        assert!(
            report
                .render_human()
                .contains(&config.data_root().display().to_string())
        );
    }

    #[test]
    fn delete_data_removes_unrelated_lock_names() {
        let temp = tempfile::tempdir().unwrap();
        let config = seed_install(&temp);
        for name in [
            "foo.lock",
            ".tray-unknown.lock",
            ".tray-abc.lock",
            ".tray-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.lock",
            ".tray-000000000000000000000000000000000000000000000000000000000000000g.lock",
        ] {
            std::fs::write(config.data_root().join(name), "user data").unwrap();
        }
        std::fs::create_dir(config.data_root().join("notes.lock")).unwrap();
        std::fs::write(config.data_root().join("notes.lock/contents"), "user data").unwrap();
        run_uninstall(&options(&temp, true, true)).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(config.data_root())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [".desktop-launch.lock", ".lifecycle.lock", ".owner.lock"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn data_deletion_preserves_every_coordination_inode_and_held_claim() {
        use std::os::unix::fs::MetadataExt;
        let temp = tempfile::tempdir().unwrap();
        let session = format!(".tray-{}.lock", "0123456789abcdef".repeat(4));
        let names = [
            OWNER_LOCK_FILE,
            LIFECYCLE_LOCK_FILE,
            ".desktop-launch.lock",
            "dream-cycle.lock",
            ".windows-native.lock",
            ".windows-browser.lock",
            ".macos-native.lock",
            ".macos-browser.lock",
            &session,
        ];
        let guards: Vec<_> = names
            .iter()
            .map(|name| {
                let file = crate::platform::native_gate::acquire(temp.path(), name).unwrap();
                let inode = file.metadata().unwrap().ino();
                (file, inode)
            })
            .collect();
        std::fs::write(temp.path().join("unrelated.lock"), "remove me").unwrap();
        let report = delete_data_contents(temp.path()).unwrap();
        assert_eq!(report.len(), names.len());
        assert!(!temp.path().join("unrelated.lock").exists());
        for (name, (_, inode)) in names.iter().zip(&guards) {
            assert_eq!(
                std::fs::metadata(temp.path().join(name)).unwrap().ino(),
                *inode
            );
            assert_eq!(
                crate::platform::native_gate::acquire(temp.path(), name)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
        drop(guards);
        for name in names {
            assert!(crate::platform::native_gate::acquire(temp.path(), name).is_ok());
        }
    }

    #[test]
    fn uninstall_refuses_a_foreign_application_directory() {
        let temp = tempfile::tempdir().unwrap();
        let mut uninstall_options = options(&temp, true, false);
        let foreign = temp.path().join("important-data");
        std::fs::create_dir_all(&foreign).unwrap();
        uninstall_options.app_dir = Some(foreign.clone());
        let error = run_uninstall(&uninstall_options).unwrap_err();
        assert!(error.to_string().contains("does not look like"), "{error}");
        assert!(foreign.exists());
    }

    #[cfg(unix)]
    #[test]
    fn path_links_are_removed_only_when_they_point_into_the_app_dir() {
        let temp = tempfile::tempdir().unwrap();
        seed_install(&temp);
        // Simulate the bootstrap installer's PATH link plus a foreign one.
        let home = temp.path().join("home");
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let app = temp.path().join("app");
        std::os::unix::fs::symlink(app.join("bin/hiero"), bin.join("hiero")).unwrap();
        std::os::unix::fs::symlink("/usr/bin/elsewhere", bin.join("hieronymus")).unwrap();

        // The PATH sweep lives above the home resolution; run the pure
        // predicate instead of faking $HOME for the whole process.
        let target = std::fs::read_link(bin.join("hiero")).unwrap();
        assert!(resolve_into(&bin.join("hiero"), &target, &app));
        let foreign = std::fs::read_link(bin.join("hieronymus")).unwrap();
        assert!(!resolve_into(&bin.join("hieronymus"), &foreign, &app));
    }
}
