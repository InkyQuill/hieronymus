use hiero::daemon::discovery::{ensure_installation_token, read_token};
use hieronymus::data_root::HieronymusConfig;

#[cfg(unix)]
#[test]
fn credential_reader_rejects_permissive_file() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    ensure_installation_token(&config).unwrap();
    std::fs::set_permissions(
        config.daemon_token_path(),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(read_token(&config).is_err());
}

#[cfg(unix)]
#[test]
fn credential_reader_rejects_symbolic_alias() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    ensure_installation_token(&config).unwrap();
    let original = root.path().join("original");
    std::fs::rename(config.daemon_token_path(), &original).unwrap();
    std::os::unix::fs::symlink(&original, config.daemon_token_path()).unwrap();
    assert!(read_token(&config).is_err());
    assert!(ensure_installation_token(&config).is_err());
}

#[test]
fn credential_creation_never_overwrites_an_existing_secret() {
    use hiero::platform::credentials::{create_private_new, read_private};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("token");
    create_private_new(&path, b"first").unwrap();
    assert!(create_private_new(&path, b"second").is_err());
    assert_eq!(read_private(&path).unwrap(), b"first");
}

#[test]
fn competing_credential_creators_return_only_the_published_token() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let barrier = std::sync::Barrier::new(12);
    std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..12)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    ensure_installation_token(&config).unwrap()
                })
            })
            .collect();
        for task in tasks {
            let token = task.join().unwrap();
            assert_eq!(
                token.expose_secret(),
                read_token(&config).unwrap().expose_secret()
            );
        }
    });
}

#[test]
fn installed_selection_rejects_versions_that_escape_the_immutable_directory() {
    let root = tempfile::tempdir().unwrap();
    let layout = hiero::app::AppLayout::new(root.path());
    assert!(layout.switch_stable_links("../other").is_err());
    assert_eq!(layout.current_version(), None);
}

#[test]
fn credential_reader_rejects_hard_link_aliases() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    ensure_installation_token(&config).unwrap();
    std::fs::hard_link(config.daemon_token_path(), root.path().join("alias")).unwrap();
    assert!(read_token(&config).is_err());
}

#[cfg(windows)]
#[test]
fn credential_reader_rejects_inherited_or_permissive_acl() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("token");
    // Native default creation intentionally inherits the test directory DACL.
    std::fs::write(&path, b"unprotected").unwrap();
    assert!(hiero::platform::credentials::read_private(&path).is_err());
}

#[cfg(windows)]
#[test]
fn windows_selection_switches_one_record_and_keeps_launchers_immutable() {
    let root = tempfile::tempdir().unwrap();
    let layout = hiero::app::AppLayout::new(root.path());
    for version in ["1.0.0", "2.0.0"] {
        std::fs::create_dir_all(layout.version_dir(version)).unwrap();
        std::fs::write(layout.version_dir(version).join("hiero.exe"), version).unwrap();
        std::fs::write(
            layout.version_dir(version).join("hiero-launcher.exe"),
            b"launcher fixture",
        )
        .unwrap();
    }
    layout.switch_stable_links("1.0.0").unwrap();
    layout.switch_stable_links("2.0.0").unwrap();
    assert_eq!(layout.current_version().as_deref(), Some("2.0.0"));
    for name in hiero::app::LINK_NAMES {
        let launcher = layout.stable_link(name);
        assert_eq!(std::fs::read(&launcher).unwrap(), b"launcher fixture");
        assert_eq!(
            hiero::platform::install::selected_executable(&launcher).unwrap(),
            layout.version_dir("2.0.0").join("hiero.exe")
        );
    }
    layout.switch_stable_links("1.0.0").unwrap();
    assert_eq!(layout.current_version().as_deref(), Some("1.0.0"));
}

#[cfg(windows)]
#[test]
fn export_rejects_a_native_junction_parent() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("data"));
    let _application = hiero::application::Application::open(&config).unwrap();
    let junction = root.path().join("alias");
    let status = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(config.data_root())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "native junction fixture creation must succeed"
    );
    assert!(hiero::export::run(&config, &junction.join("export.json")).is_err());
    assert!(!config.data_root().join("export.json").exists());
    std::fs::remove_dir(junction).unwrap();
}

#[cfg(windows)]
#[test]
fn credential_reader_rejects_an_explicit_everyone_read_ace() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("token");
    hiero::platform::credentials::create_private_new(&path, b"secret").unwrap();
    let status = std::process::Command::new("icacls.exe")
        .arg(&path)
        .args(["/grant", "*S-1-1-0:(R)", "/Q"])
        .status()
        .unwrap();
    assert!(status.success(), "native ACL fixture must succeed");
    assert!(hiero::platform::credentials::read_private(&path).is_err());
}
