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
    let text = "gui/501/fixture = {\nstate = not running\npath = /tmp/owned.plist\nprogram = /app/bin/hiero\narguments = {\n/app/bin/hiero\ndaemon\n--data-root\n/tmp/root\n}\nproperties = inferred program\n}";
    macos_agent::validate_loaded(text, path, &args).unwrap();
    let mut named_args = args.clone();
    named_args[3] = "/tmp/keepalive-book".into();
    macos_agent::validate_loaded(
        &text.replace("/tmp/root", "/tmp/keepalive-book"),
        path,
        &named_args,
    )
    .unwrap();
    let tahoe = text
        .replace(
            "arguments = {",
            "stdout path = /dev/null\nstderr path = /dev/null\nproxy started suspended = false\nextension alive = false\ntrial factors memory limit = 0 MB\nchecked allocations = false (queried = true)\nchecked allocations reason = none\nchecked allocations flags = 0x0\npended spawn = false\npended nondemand spawn = false\nspawn reason filter = none\narguments = {",
        )
        .replace(
            "properties = inferred program",
            "properties = inferred program | system service | tle system",
        );
    macos_agent::validate_loaded(&tahoe, path, &args).unwrap();
    for changed in [
        text.replace("path = /tmp/owned.plist", "path = /tmp/foreign.plist"),
        text.replace("/tmp/root", "/tmp/foreign"),
        text.replace("inferred program", "keepalive | inferred program"),
        tahoe.replace("stdout path = /dev/null", "stdout path = /tmp/output"),
        text.replace(
            "program = /app/bin/hiero",
            "program = /app/bin/hiero\nprogram = /foreign",
        ),
    ] {
        assert!(macos_agent::validate_loaded(&changed, path, &args).is_err());
    }
    assert!(!macos_agent::disabled_state("disabled services = {\n}", "fixture").unwrap());
    assert!(!macos_agent::disabled_state("\n\tdisabled services = {\n}\n", "fixture").unwrap());
    assert!(
        macos_agent::disabled_state("disabled services = {\n\"fixture\" => true\n}", "fixture")
            .unwrap()
    );
    assert!(
        macos_agent::disabled_state(
            "\n\tdisabled services = {\n\t\t\"fixture\" => disabled\n\t}\n",
            "fixture"
        )
        .unwrap()
    );
    assert!(
        !macos_agent::disabled_state(
            "\n\tdisabled services = {\n\t\t\"fixture\" => enabled\n\t}\n",
            "fixture"
        )
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

#[test]
fn readback_never_confuses_malformed_authority_with_absence() {
    for text in [
        "disabled services = {\n\"fixture\"=> true\n}",
        "disabled services = {\n\"fixture\" => true\n\"fixture\"=> false\n}",
        "disabled services = {\n\"fixture\" : true\n}",
        "disabled services = {\n\"fixture\" => true\n}\n}",
        "disabled services = {\nother malformed entry\n}",
    ] {
        assert!(
            macos_agent::disabled_state(text, "fixture").is_err(),
            "accepted {text}"
        );
    }
    let args = vec![
        "/app/hiero".into(),
        "daemon".into(),
        "--data-root".into(),
        "/tmp/root".into(),
    ];
    let valid = "gui/501/fixture = {\nstate = not running\npath = /tmp/owned.plist\nprogram = /app/hiero\narguments = {\n/app/hiero\ndaemon\n--data-root\n/tmp/root\n}\nproperties = inferred program\n}";
    for text in [
        valid.replace("\n}\nproperties = inferred program\n}", ""),
        valid.trim_end_matches('}').to_owned(),
        valid.replace(
            "properties = inferred program",
            "pid=> 123\nproperties = inferred program",
        ),
        valid.replace(
            "properties = inferred program",
            "pid = invalid\nproperties = inferred program",
        ),
        valid.replace(
            "properties = inferred program",
            "pid = 123\npid= 456\nproperties = inferred program",
        ),
        valid.replace(
            "properties = inferred program",
            "keepalive=> true\nproperties = inferred program",
        ),
    ] {
        assert!(
            macos_agent::validate_loaded(&text, std::path::Path::new("/tmp/owned.plist"), &args)
                .is_err(),
            "accepted {text}"
        );
    }
}

#[test]
fn headless_login_mode_is_owned_persisted_and_restored_after_failed_transition() {
    use macos_agent::DaemonMode::{Desktop, Headless};
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("hiero");
    let directory = root.path().join("agents");
    let headless =
        macos_agent::render_mode(&binary, root.path(), &directory, false, Headless).unwrap();
    assert!(headless.contains("<key>RunAtLoad</key><true/>"));
    let desktop =
        macos_agent::render_mode(&binary, root.path(), &directory, false, Desktop).unwrap();
    assert!(desktop.contains("<key>RunAtLoad</key><false/>"));
    for text in [&headless, &desktop] {
        assert!(text.contains("<key>KeepAlive</key><false/>"));
    }
    let path = root.path().join("mode.plist");
    std::fs::write(&path, &headless).unwrap();
    let identify = || {
        macos_agent::owned_mode(
            &std::fs::read_to_string(&path).unwrap(),
            &binary,
            root.path(),
            &directory,
        )
        .unwrap()
    };
    assert_eq!(identify(), Headless);
    let result: Result<(), String> = macos_agent::publish_with_rollback(
        || {
            std::fs::write(&path, &desktop).unwrap();
            assert_eq!(identify(), Desktop);
            Err("tray registration failed".into())
        },
        || macos_agent::restore_definition(&path, Some(headless.as_bytes())),
    );
    assert_eq!(result.unwrap_err(), "tray registration failed");
    assert_eq!(identify(), Headless);
    let foreign = headless.replace(
        "<key>KeepAlive</key><false/>",
        "<key>KeepAlive</key><true/>",
    );
    assert!(macos_agent::owned_mode(&foreign, &binary, root.path(), &directory).is_err());
}

#[test]
fn loaded_process_state_requires_explicit_idle_and_preserves_argument_spaces() {
    let path = std::path::Path::new("/tmp/owned.plist");
    let args = vec![
        "/app/hiero".to_string(),
        "daemon".into(),
        "--data-root".into(),
        "/tmp/root  ".into(),
    ];
    let text = "gui/501/fixture = {\n\tstate = not running\n\tpath = /tmp/owned.plist\n\tprogram = /app/hiero\n\targuments = {\n\t\t/app/hiero\n\t\tdaemon\n\t\t--data-root\n\t\t/tmp/root  \n\t}\n\tproperties = inferred program | runatload\n}";
    let loaded = macos_agent::loaded_state(text, path, &args).unwrap();
    assert!(loaded.idle);
    assert_eq!(loaded.pid, None);
    assert!(loaded.run_at_load);
    assert!(
        macos_agent::loaded_state(&text.replace("/tmp/root  ", "/tmp/root"), path, &args).is_err()
    );
    let live = text.replace("state = not running", "state = running\n\tpid = 42");
    let loaded = macos_agent::loaded_state(&live, path, &args).unwrap();
    assert_eq!(loaded.pid, Some(42));
    assert!(!loaded.idle);
    let proxy = text.replace("state = not running", "state = xpcproxy\n\tpid = 42");
    let loaded = macos_agent::loaded_state(&proxy, path, &args).unwrap();
    assert_eq!(loaded.pid, Some(42));
    assert!(!loaded.idle);
    for malformed in [
        text.replace("state = not running", "state = running"),
        text.replace("state = not running", "state = not running\n\tpid = 42"),
        text.replace("state = not running", "state = not running\n\tpid = 0"),
        text.replace(
            "state = not running",
            "state = not running\n\tprocess id = 42",
        ),
        text.replace(
            "inferred program | runatload",
            "inferred program | runatload | runatload",
        ),
    ] {
        assert!(macos_agent::loaded_state(&malformed, path, &args).is_err());
    }
}

#[test]
fn mode_conversion_requires_unloaded_prior_job_and_preserves_explicit_mode() {
    use macos_agent::{
        DaemonMode::{Desktop, Headless},
        check_mode_change,
    };
    assert!(check_mode_change(Some(Headless), true, Desktop).is_err());
    check_mode_change(Some(Headless), false, Desktop).unwrap();
    check_mode_change(Some(Desktop), true, Desktop).unwrap();
    check_mode_change(None, false, Desktop).unwrap();
    assert!(check_mode_change(None, true, Desktop).is_err());
}
