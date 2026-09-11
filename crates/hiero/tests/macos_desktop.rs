use hiero::service::macos_agent;
#[test]
fn definitions_preserve_literal_arguments_and_separate_login_from_daemon() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("hiero & <quoted> \" '");
    let dir = root.path().join("agents");
    let daemon = macos_agent::render(&binary, root.path(), &dir, false).unwrap();
    let tray = macos_agent::render(&binary, root.path(), &dir, true).unwrap();
    assert_eq!(
        macos_agent::parse(&daemon).unwrap(),
        (binary.clone(), root.path().to_path_buf())
    );
    assert!(daemon.contains("&amp; &lt;quoted&gt; &quot; &apos;"));
    assert!(daemon.contains("<key>RunAtLoad</key><false/>"));
    assert!(tray.contains("<key>RunAtLoad</key><true/>"));
    assert!(daemon.contains("<key>KeepAlive</key><false/>"));
    assert!(tray.contains("<key>KeepAlive</key><false/>"));
    assert_ne!(
        macos_agent::label(root.path(), false),
        macos_agent::label(root.path(), true)
    );
    assert_eq!(macos_agent::domain(501), "gui/501");
    let other = tempfile::tempdir().unwrap();
    assert_ne!(
        macos_agent::label(root.path(), false),
        macos_agent::label(other.path(), false)
    );
    assert!(tray.contains("<string>Aqua</string>"));
    let doc = roxmltree::Document::parse_with_options(
        &tray,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .unwrap();
    let args = doc
        .descendants()
        .find(|n| n.has_tag_name("array"))
        .unwrap()
        .children()
        .filter(|n| n.is_element())
        .map(|n| n.text().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        args,
        macos_agent::arguments(&binary, root.path(), &dir, true).unwrap()
    );
}
#[test]
fn failed_registration_restores_exact_prior_definition_or_removes_new_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fixture.plist");
    std::fs::write(&path, b"new failed registration").unwrap();
    macos_agent::restore_definition(&path, Some(b"previous owned definition")).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"previous owned definition");
    macos_agent::restore_definition(&path, None).unwrap();
    assert!(!path.exists());
}

#[test]
fn daemon_registration_uses_actual_cli_command() {
    let root = tempfile::tempdir().unwrap();
    let args = macos_agent::arguments(&root.path().join("hiero"), root.path(), root.path(), false)
        .unwrap();
    assert_eq!(&args[1..3], ["daemon", "--data-root"]);
}

#[test]
fn publication_failure_rolls_back_and_failed_rollback_remains_indeterminate() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("owned.plist");
    std::fs::write(&path, b"prior").unwrap();
    let result: Result<(), String> = macos_agent::publish_with_rollback(
        || {
            std::fs::write(&path, b"candidate").unwrap();
            Err("bootstrap failed".into())
        },
        || macos_agent::restore_definition(&path, Some(b"prior")),
    );
    assert_eq!(result.unwrap_err(), "bootstrap failed");
    assert_eq!(std::fs::read(&path).unwrap(), b"prior");
    let result: Result<(), String> = macos_agent::publish_with_rollback(
        || Err("bootstrap failed".into()),
        || Err("native rollback pending".into()),
    );
    assert!(result.unwrap_err().contains("rollback remains pending"));
    let result = macos_agent::publish_with_rollback(
        || Ok(7),
        || panic!("successful registration must not roll back"),
    );
    assert_eq!(result.unwrap(), 7);
}
#[cfg(target_os = "macos")]
#[test]
#[ignore = "Requires a real current-user Aqua login; uses only a fresh disposable root and agent directory"]
fn native_disposable_launchagents_register_toggle_readback_and_remove() {
    use hiero::desktop::{AutostartRegistration, macos_registration::MacosRegistration};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let directory = temp.path().join("agents");
    std::fs::create_dir_all(&root).unwrap();
    let binary = temp.path().join("hiero");
    std::fs::copy(env!("CARGO_BIN_EXE_hiero"), &binary).unwrap();
    let options = hiero::service::ServiceOptions {
        data_root: root,
        unit_dir: directory,
        binary,
        use_manager: true,
    };
    let mut registration = MacosRegistration::new(options.clone());
    registration.install().unwrap();
    assert!(hiero::service::status(&options).unwrap().consistent());
    assert!(registration.is_enabled().unwrap());
    registration.set_enabled(false).unwrap();
    assert!(!registration.is_enabled().unwrap());
    registration.set_enabled(true).unwrap();
    assert!(registration.is_enabled().unwrap());
    hiero::service::stop(&options).unwrap();
    assert!(registration.is_enabled().unwrap());
    registration.uninstall().unwrap();
    assert!(!registration.is_enabled().unwrap());
    assert!(!options.unit_path().exists());
}

#[test]
fn loaded_readback_refuses_foreign_paths_arguments_recovery_and_unknown_formats() {
    let path = std::path::Path::new("/tmp/owned.plist");
    let args = vec![
        "/app/bin/hiero".to_string(),
        "daemon".into(),
        "--data-root".into(),
        "/tmp/root".into(),
    ];
    let text = "gui/501/fixture = {\npath = /tmp/owned.plist\nprogram = /app/bin/hiero\narguments = {\n/app/bin/hiero\ndaemon\n--data-root\n/tmp/root\n}\nproperties = inferred program\n}";
    macos_agent::validate_loaded(text, path, &args).unwrap();
    let mut named_args = args.clone();
    named_args[3] = "/tmp/keepalive-book".into();
    macos_agent::validate_loaded(
        &text.replace("/tmp/root", "/tmp/keepalive-book"),
        path,
        &named_args,
    )
    .unwrap();
    for changed in [
        text.replace("path = /tmp/owned.plist", "path = /tmp/foreign.plist"),
        text.replace("/tmp/root", "/tmp/foreign"),
        text.replace("inferred program", "keepalive | inferred program"),
        text.replace(
            "program = /app/bin/hiero",
            "program = /app/bin/hiero\nprogram = /foreign",
        ),
    ] {
        assert!(macos_agent::validate_loaded(&changed, path, &args).is_err());
    }
    assert!(!macos_agent::disabled_state("disabled services = {\n}", "fixture").unwrap());
    assert!(
        macos_agent::disabled_state("disabled services = {\n\"fixture\" => true\n}", "fixture")
            .unwrap()
    );
    assert!(macos_agent::disabled_state("unknown output", "fixture").is_err());
    assert!(
        macos_agent::disabled_state(
            "disabled services = {\n\"fixture\" => true\n\"fixture\" => false\n}",
            "fixture"
        )
        .is_err()
    );
}
