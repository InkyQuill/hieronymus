//! Explicit installed-artifact checks; fixtures are never inferred from target/debug.
use std::path::PathBuf;
use std::process::Command;

#[ignore = "requires disposable installed release"]
#[test]
fn shipped_aliases_resolve_in_one_version_directory() {
    let app = std::env::var_os("HIERO_TEST_INSTALLED_APP")
        .expect("set HIERO_TEST_INSTALLED_APP to a disposable installed application");
    let bin = PathBuf::from(app).join("bin");
    let executable = std::fs::canonicalize(bin.join("hiero")).unwrap();
    for name in ["hieronymus", "hieronymus-agent-hook", "hieronymus-mcp"] {
        assert_eq!(std::fs::canonicalize(bin.join(name)).unwrap(), executable);
    }
}

mod common;

#[ignore = "requires disposable installed release"]
#[test]
fn installed_candidate_is_executable_and_releases_ownership_on_stop_and_sigterm() {
    use common::installed::{InstalledDaemon, binary, command};
    use hieronymus::data_root::HieronymusConfig;
    let binary = binary();
    let root = tempfile::tempdir().unwrap();
    let version = command(&binary, root.path())
        .args(["version", "--json"])
        .output()
        .unwrap();
    assert!(version.status.success());
    let version: serde_json::Value = serde_json::from_slice(&version.stdout).unwrap();
    assert_eq!(version["protocol_revision"], common::PROTOCOL_REVISION);
    let config = HieronymusConfig::new(root.path());
    let mut daemon = InstalledDaemon::start(&binary, root.path());
    let token = std::fs::read(config.daemon_token_path()).unwrap();
    let status_tool = daemon
        .client
        .call_tool("hieronymus_status", &serde_json::json!({}))
        .unwrap();
    assert_eq!(
        status_tool["result"]["structuredContent"]["service"]["mode"],
        "local-http"
    );
    for action in ["daemon", "migrate", "recover"] {
        let refused = command(&binary, root.path()).arg(action).output().unwrap();
        assert!(
            !refused.status.success(),
            "concurrent {action} must be refused"
        );
        assert!(daemon.ready());
    }
    let stopped = command(&binary, root.path()).arg("stop").output().unwrap();
    assert!(stopped.status.success());
    daemon.wait_stopped();
    assert!(!config.daemon_discovery_path().exists());
    let mut daemon = InstalledDaemon::start(&binary, root.path());
    assert_eq!(std::fs::read(config.daemon_token_path()).unwrap(), token);
    assert!(
        Command::new("/bin/kill")
            .args(["-TERM", &daemon.pid().to_string()])
            .status()
            .unwrap()
            .success()
    );
    daemon.wait_stopped();
    assert!(!config.daemon_discovery_path().exists());
    let mut daemon = InstalledDaemon::start(&binary, root.path());
    daemon
        .client
        .post("/shutdown", &serde_json::json!({}))
        .unwrap();
    daemon.wait_stopped();
}

#[ignore = "requires disposable installed release"]
#[test]
fn installed_migration_preserves_terms_and_provenance_and_recovers_corruption() {
    use common::installed::{binary, command};
    let binary = binary();
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("hieronymus.sqlite");
    seed_legacy(&database);
    for args in [vec!["migrate", "--dry-run"], vec!["migrate"]] {
        let output = command(&binary, root.path()).args(&args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let verify = || {
        let db = rusqlite::Connection::open_with_flags(
            &database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let term: (String, String) = db
            .query_row(
                "select canonical_translation,notes from term_rules where source_text='センス'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(term, ("Sense".into(), "chapter 1 evidence".into()));
        let source: String = db
            .query_row(
                "select source_ref from short_term_memories where id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(source, "chapter-1:paragraph-3");
        let concepts: i64 = db
            .query_row("select count(*) from concepts where canonical_name='Harbour Keeper' and description='chapter 1 character'", [], |r| r.get(0))
            .unwrap();
        assert!(concepts > 0, "migration must preserve existing concepts");
    };
    verify();
    std::fs::write(&database, b"deliberately corrupted disposable database").unwrap();
    let refused = command(&binary, root.path())
        .arg("daemon")
        .output()
        .unwrap();
    assert!(!refused.status.success());
    let recovered = command(&binary, root.path())
        .arg("recover")
        .output()
        .unwrap();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    verify();
    for malformed in [
        "update hieronymus_meta set schema_version=999; pragma user_version=999;",
        "drop table dream_retry_state;",
    ] {
        let broken = tempfile::tempdir().unwrap();
        std::fs::copy(&database, broken.path().join("hieronymus.sqlite")).unwrap();
        let db = rusqlite::Connection::open(broken.path().join("hieronymus.sqlite")).unwrap();
        db.execute_batch(malformed).unwrap();
        drop(db);
        let output = command(&binary, broken.path())
            .arg("daemon")
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "newer/partial schema must fail closed"
        );
    }
}

fn seed_legacy(database: &std::path::Path) {
    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .execute_batch(include_str!("../../hieronymus/migrations/global.sql"))
        .unwrap();
    connection.execute_batch("insert into series(slug,title,default_source_language,default_target_language,created_at,updated_at) values ('demo','Demo','ja','en','2026-09-07','2026-09-07');
        insert into concepts(id,canonical_name,description,scope_type,scope_key,status,confidence,created_at,updated_at) values (1,'Harbour Keeper','chapter 1 character','series','series:demo','active',.8,'2026-09-07','2026-09-07');
        insert into strict_terms(series_slug,source_language,target_language,category,source_text,canonical_translation,status,notes,created_at,updated_at) values ('demo','ja','en','name','センス','Sense','approved','chapter 1 evidence','2026-09-07','2026-09-07');
        insert into strict_terms_fts(rowid,source_text,canonical_translation,notes) values (1,'センス','Sense','chapter 1 evidence');
        insert into task_sessions(id,series_slug,source_language,target_language,task_type,status,created_at,last_activity_at) values (1,'demo','ja','en','translation','completed','2026-09-07','2026-09-07');
        insert into short_term_memories(id,session_id,source_role,kind,text,source_ref,created_at) values (1,1,'user','note','The hero remembers the harbour.','chapter-1:paragraph-3','2026-09-07');").unwrap();
    drop(connection);
}

#[ignore = "requires disposable installed release and distinct compiled upgrade release"]
#[test]
fn actual_distinct_versions_upgrade_after_completed_cutover() {
    rehearse_distinct_versions(false);
}

#[ignore = "requires compiled rollback baseline and distinct compiled upgrade release"]
#[test]
fn actual_compiled_candidate_health_failure_restores_ready_previous_version() {
    rehearse_distinct_versions(true);
}

fn rehearse_distinct_versions(inject_health_failure: bool) {
    use common::installed::{InstalledDaemon, command};
    let base = PathBuf::from(
        std::env::var_os(if inject_health_failure {
            "HIERO_TEST_ROLLBACK_BASE_RELEASE_DIR"
        } else {
            "HIERO_TEST_BASE_RELEASE_DIR"
        })
        .expect("explicit base release directory is required"),
    );
    let upgrade = PathBuf::from(
        std::env::var_os("HIERO_TEST_UPGRADE_RELEASE_DIR")
            .expect("HIERO_TEST_UPGRADE_RELEASE_DIR is required"),
    );
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    let root = temp.path().join("data");
    let home = temp.path().join("home");
    let unit = temp.path().join("unit");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let install = Command::new(common::repo_root().join("scripts/install.sh"))
        .args(["--no-activate", "--release-dir"])
        .arg(base)
        .arg("--app-dir")
        .arg(&app)
        .arg("--data-root")
        .arg(&root)
        .arg("--unit-dir")
        .arg(&unit)
        .env("HOME", &home)
        .env_remove("HIERONYMUS_RELEASE_URL")
        .env_remove("HIERONYMUS_RELEASE_DIR")
        .output()
        .unwrap();
    assert!(
        install.status.success(),
        "{}",
        String::from_utf8_lossy(&install.stderr)
    );
    let binary = app.join("bin/hiero");
    let version = || -> serde_json::Value {
        let output = command(&binary, &root)
            .args(["version", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let previous = version();
    seed_legacy(&root.join("hieronymus.sqlite"));
    let migration = command(&binary, &root).arg("migrate").output().unwrap();
    assert!(
        migration.status.success(),
        "{}",
        String::from_utf8_lossy(&migration.stderr)
    );
    let mut update_command = command(&binary, &root);
    if inject_health_failure {
        update_command.env(
            "HIERO_SEMANTIC_MODEL_DIR",
            temp.path().join("missing-model"),
        );
    }
    let update = update_command
        .arg("update")
        .arg("--release-dir")
        .arg(upgrade)
        .arg("--app-dir")
        .arg(&app)
        .arg("--unit-dir")
        .arg(&unit)
        .arg("--json")
        .output()
        .unwrap();
    if inject_health_failure {
        assert!(!update.status.success());
        let detail = format!(
            "{}{}",
            String::from_utf8_lossy(&update.stdout),
            String::from_utf8_lossy(&update.stderr)
        );
        assert!(detail.contains("health check failed"), "{detail}");
        assert!(detail.contains("rolled back"), "{detail}");
        assert_eq!(version()["version"], previous["version"]);
        for alias in [
            "hiero",
            "hieronymus",
            "hieronymus-mcp",
            "hieronymus-agent-hook",
        ] {
            assert_eq!(
                std::fs::canonicalize(app.join("bin").join(alias)).unwrap(),
                std::fs::canonicalize(&binary).unwrap()
            );
        }
        let mut restored = InstalledDaemon::start(&binary, &root);
        assert_eq!(
            restored.client.get("/status").unwrap()["version"],
            previous["version"]
        );
        restored
            .client
            .post("/shutdown", &serde_json::json!({}))
            .unwrap();
        restored.wait_stopped();
        return;
    }
    assert!(
        update.status.success(),
        "{}{}",
        String::from_utf8_lossy(&update.stdout),
        String::from_utf8_lossy(&update.stderr)
    );
    let current = version();
    assert_ne!(
        previous["version"], current["version"],
        "same-version reinstall is not an upgrade"
    );
    let mut daemon = InstalledDaemon::start(&binary, &root);
    let response = daemon
        .client
        .call_tool("hieronymus_series_list", &serde_json::json!({}))
        .unwrap();
    assert!(
        response["result"]["structuredContent"]
            .to_string()
            .contains("demo")
    );
    let status = daemon.client.get("/status").unwrap();
    assert_eq!(status["version"], current["version"]);
    daemon
        .client
        .post("/shutdown", &serde_json::json!({}))
        .unwrap();
    daemon.wait_stopped();
    eprintln!(
        "actual compiled release upgrade {} -> {} after completed Python cutover PASS",
        previous["version"], current["version"]
    );
}
