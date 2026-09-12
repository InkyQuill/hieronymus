use std::process::{Command, Output};

use serde_json::{Value, json};

use hiero::project_context::exit_code;

fn write_project(root: &std::path::Path, schema: u64) {
    let manifest = if schema == 1 {
        "---\nschema-version: 1\ntitle: Example\nlanguage: ru\nstatus: drafting\n---\nManuscript body must stay private.\n"
            .to_string()
    } else {
        format!(
            "---\nschema-version: {schema}\ntitle: Example\nlanguage: ru\nstatus: drafting\nproject-kind: authoring\nwork-kind: book\ntranslation-enabled: true\n---\n"
        )
    };
    std::fs::write(root.join("project.md"), manifest).unwrap();
    std::fs::write(
        root.join("AGENTS.md"),
        "Agreement text must be read separately and never printed.\n",
    )
    .unwrap();
}

fn cli(root: &std::path::Path, data_root: &std::path::Path, extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hiero"));
    command
        .env("PATH", "")
        .arg("project-context")
        .arg("--cwd")
        .arg(root)
        .arg("--data-root")
        .arg(data_root)
        .args(extra)
        .output()
        .unwrap()
}

fn json_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON stdout ({error}): {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn v1_project_context_is_unbound_and_does_not_bootstrap_runtime_state() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    std::fs::create_dir(&project).unwrap();
    write_project(&project, 1);
    let data_root = fixture.path().join("must-not-be-created");

    let output = cli(
        &project,
        &data_root,
        &["--args", "{\"direction_id\":null}", "--json"],
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        json_stdout(&output),
        json!({
            "version": 1,
            "status": "unbound",
            "root": project.canonicalize().unwrap(),
            "schema_version": 1,
            "instructions_path": project.canonicalize().unwrap().join("AGENTS.md"),
            "binding": null,
            "direction_id": null,
            "source_language": "ru",
            "target_language": null,
            "diagnostics": [],
        })
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("Manuscript body must stay private"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("Agreement text must be read separately"),
        "{stdout}"
    );
    assert!(
        !data_root.exists(),
        "project inspection created {data_root:?}"
    );
}

#[test]
fn project_context_human_output_is_the_same_complete_envelope() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    std::fs::create_dir(&project).unwrap();
    write_project(&project, 1);
    let data_root = fixture.path().join("unused");
    let root = project.canonicalize().unwrap();

    let output = cli(&project, &data_root, &[]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "version: 1\nstatus: unbound\nroot: {}\nschema_version: 1\ninstructions_path: {}\nbinding: none\ndirection_id: none\nsource_language: ru\ntarget_language: none\ndiagnostics: none\n",
            root.display(),
            root.join("AGENTS.md").display()
        )
    );
}

#[test]
fn ready_binding_is_reported_without_legacy_language_defaults() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    std::fs::create_dir(&project).unwrap();
    write_project(&project, 1);
    std::fs::write(
        project.join(".hieronymus.json"),
        r#"{
            "series_slug": "actual-work",
            "source_language": "ja",
            "target_language": "en",
            "cws": {
                "binding_version": 1,
                "project_contract_version": 1,
                "directions": {}
            }
        }"#,
    )
    .unwrap();

    let output = cli(&project, &fixture.path().join("unused"), &["--json"]);

    assert_eq!(output.status.code(), Some(0));
    let report = json_stdout(&output);
    assert_eq!(report["status"], "ready");
    assert_eq!(report["binding"], json!({"series_slug": "actual-work"}));
    assert_eq!(report["source_language"], "ru");
    assert_eq!(report["target_language"], Value::Null);
}

#[test]
fn schema_two_authoring_is_supported_without_direction_context() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    std::fs::create_dir(&project).unwrap();
    write_project(&project, 2);

    let output = cli(&project, &fixture.path().join("unused"), &["--json"]);

    assert_eq!(output.status.code(), Some(0));
    let report = json_stdout(&output);
    assert_eq!(report["status"], "unbound");
    assert_eq!(report["schema_version"], 2);
    assert_eq!(report["source_language"], "ru");
}

#[test]
fn missing_project_and_unsafe_filesystem_input_use_public_diagnostics() {
    let fixture = tempfile::tempdir().unwrap();
    let data_root = fixture.path().join("unused");

    let missing = cli(fixture.path(), &data_root, &["--json"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        json_stdout(&missing),
        json!({
            "version": 1,
            "status": "not_found",
            "root": null,
            "schema_version": null,
            "instructions_path": null,
            "binding": null,
            "direction_id": null,
            "source_language": null,
            "target_language": null,
            "diagnostics": ["project_not_found"],
        })
    );

    let unsafe_path = fixture.path().join("child").join("..");
    std::fs::create_dir(fixture.path().join("child")).unwrap();
    let unsafe_output = cli(&unsafe_path, &data_root, &["--json"]);
    assert_eq!(unsafe_output.status.code(), Some(2));
    let report = json_stdout(&unsafe_output);
    assert_eq!(report["status"], "invalid");
    assert_eq!(report["diagnostics"], json!(["unsafe_path"]));
    assert!(!String::from_utf8_lossy(&unsafe_output.stdout).contains("CWS project I/O error"));
}

#[test]
fn invalid_args_unrelated_flags_and_direction_selection_are_rejected() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    std::fs::create_dir(&project).unwrap();
    write_project(&project, 1);
    let data_root = fixture.path().join("unused");

    for args in [
        vec!["--args", "not-json", "--json"],
        vec!["--args", "{\"direction_id\":null,\"extra\":true}", "--json"],
        vec!["--start-daemon", "--json"],
        vec!["--output", "elsewhere.json", "--json"],
        vec!["--host", "codex", "--json"],
        vec![
            "--delivery-id",
            "550e8400-e29b-41d4-a716-446655440000",
            "--json",
        ],
    ] {
        let output = cli(&project, &data_root, &args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "args {args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let selected = cli(
        &project,
        &data_root,
        &["--args", "{\"direction_id\":\"unknown\"}", "--json"],
    );
    assert_eq!(selected.status.code(), Some(1));
    let report = json_stdout(&selected);
    assert_eq!(report["status"], "not_found");
    assert_eq!(report["direction_id"], "unknown");
    assert_eq!(report["diagnostics"], json!(["unknown_direction"]));
    assert!(!data_root.exists());
}

#[test]
fn independent_schema_binding_and_contract_versions_have_stable_statuses() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    std::fs::create_dir(&project).unwrap();
    write_project(&project, 1);
    let data_root = fixture.path().join("unused");

    for (binding_version, contract_version, diagnostic) in [
        (2, 1, "unsupported_binding_version"),
        (1, 2, "unsupported_contract_version"),
    ] {
        std::fs::write(
            project.join(".hieronymus.json"),
            format!(
                r#"{{
                    "series_slug": "actual-work",
                    "cws": {{
                        "binding_version": {binding_version},
                        "project_contract_version": {contract_version},
                        "directions": {{}}
                    }}
                }}"#
            ),
        )
        .unwrap();
        let output = cli(&project, &data_root, &["--json"]);
        assert_eq!(output.status.code(), Some(1));
        let report = json_stdout(&output);
        assert_eq!(report["status"], "unsupported");
        assert_eq!(report["diagnostics"], json!([diagnostic]));
    }

    std::fs::write(
        project.join("project.md"),
        "---\nschema-version: 99\ntitle: Future\nlanguage: ru\nstatus: planning\n---\n",
    )
    .unwrap();
    let output = cli(&project, &data_root, &["--json"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json_stdout(&output);
    assert_eq!(report["status"], "unsupported");
    assert_eq!(report["schema_version"], 99);
    assert_eq!(report["diagnostics"], json!(["unsupported_schema"]));
}

#[test]
fn legacy_hook_directs_cws_projects_to_explicit_inspection() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    let descendant = project.join("story");
    std::fs::create_dir_all(&descendant).unwrap();
    write_project(&project, 1);
    std::fs::write(
        descendant.join(".hieronymus.json"),
        r#"{"series_slug":"guessed","source_language":"ja","target_language":"en"}"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .env("PATH", "")
        .args(["agent-hook", "session-start", "--cwd"])
        .arg(&descendant)
        .arg("--data-root")
        .arg(fixture.path().join("unused"))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("hiero project-context"), "{stdout}");
    assert!(stdout.contains("AGENTS.md"), "{stdout}");
    assert!(!stdout.contains("guessed"), "{stdout}");
    assert!(!stdout.contains("ja"), "{stdout}");
    assert!(!stdout.contains("en"), "{stdout}");
}

#[test]
fn malformed_inspection_status_fails_closed() {
    assert_eq!(exit_code(&json!({"status": "unexpected"})), 2);
    assert_eq!(exit_code(&json!({})), 2);
}

#[cfg(unix)]
#[test]
fn deleted_implicit_cwd_returns_safe_json_and_human_envelopes() {
    let fixture = tempfile::tempdir().unwrap();
    let deleted_cwd = fixture.path().join("deleted-cwd");
    std::fs::create_dir(&deleted_cwd).unwrap();
    let data_root = fixture.path().join("unused");
    let output = Command::new("/bin/sh")
        .env("PATH", "")
        .args([
            "-c",
            "cd \"$1\" && /bin/rmdir \"$1\" && exec \"$2\" project-context --json --data-root \"$3\"",
            "sh",
        ])
        .arg(&deleted_cwd)
        .arg(env!("CARGO_BIN_EXE_hiero"))
        .arg(&data_root)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        json_stdout(&output),
        json!({
            "version": 1,
            "status": "invalid",
            "root": null,
            "schema_version": null,
            "instructions_path": null,
            "binding": null,
            "direction_id": null,
            "source_language": null,
            "target_language": null,
            "diagnostics": ["filesystem_error"],
        })
    );
    assert!(!data_root.exists());

    let deleted_human_cwd = fixture.path().join("deleted-human-cwd");
    std::fs::create_dir(&deleted_human_cwd).unwrap();
    let human = Command::new("/bin/sh")
        .env("PATH", "")
        .args([
            "-c",
            "cd \"$1\" && /bin/rmdir \"$1\" && exec \"$2\" project-context --data-root \"$3\"",
            "sh",
        ])
        .arg(&deleted_human_cwd)
        .arg(env!("CARGO_BIN_EXE_hiero"))
        .arg(&data_root)
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(2));
    assert!(
        human.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert_eq!(
        String::from_utf8(human.stdout).unwrap(),
        "version: 1\nstatus: invalid\nroot: none\nschema_version: none\ninstructions_path: none\nbinding: none\ndirection_id: none\nsource_language: none\ntarget_language: none\ndiagnostics: filesystem_error\n"
    );
}

#[test]
fn actionable_translation_fixtures_project_the_existing_envelope() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../compatibility/rust/cws-project-v1.json"
    ))
    .unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let Some(selections) = case["expect"]["selections"].as_array() else {
            continue;
        };
        let root = tempfile::tempdir().unwrap();
        for (relative, contents) in case["files"].as_object().unwrap() {
            let path = root.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents.as_str().unwrap()).unwrap();
        }
        let data_root = root.path().join("unused-data");
        for selection in selections {
            let cwd = root.path().join(selection["cwd"].as_str().unwrap());
            let args = json!({"direction_id":selection["direction_id"]}).to_string();
            let output = cli(&cwd, &data_root, &["--args", &args, "--json"]);
            let report = json_stdout(&output);
            assert_eq!(
                report["status"], selection["status"],
                "{} {selection}: {report}",
                case["name"]
            );
            assert_eq!(report.as_object().unwrap().len(), 10);
            assert_eq!(output.status.code(), Some(i32::from(exit_code(&report))));
            if let Some(diagnostic) = selection.get("diagnostic") {
                assert_eq!(report["diagnostics"], json!([diagnostic]));
            } else {
                assert_eq!(
                    &report["direction_id"],
                    selection
                        .get("selected_direction")
                        .unwrap_or(&selection["direction_id"])
                );
                assert_eq!(report["source_language"], selection["source_language"]);
                assert_eq!(report["target_language"], selection["target_language"]);
                if selection["status"] == "ready" {
                    assert_eq!(report["binding"]["series_slug"], "work");
                } else {
                    assert_eq!(report["binding"], Value::Null);
                }
            }
            assert!(!data_root.exists());
        }
    }
}

#[test]
fn series_documents_without_volume_metadata_project_actionable_context() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../compatibility/rust/cws-project-v1.json"
    ))
    .unwrap();
    let case = fixtures["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "translation-series")
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    for (relative, contents) in case["files"].as_object().unwrap() {
        let path = root.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents.as_str().unwrap()).unwrap();
    }
    // Recorded from the canonical CWS draft -> reviewed -> accepted producer.
    let accepted = "---\ndirection-id: ru-main\ndraft-id: u002\nsource-units:\n  - ja:u002\npacket-transaction: 210549e157b84af0b55a9db1b2eca549\nbase-revision: absent\nstatus: accepted\nreview-hash: 23216bae071f0aa39799ab476268d46f780eefb1f4454df208899cbc44fdc197\n---\nПеревод второго тома.\n\n";
    let target = root
        .path()
        .join("translations/ru-main/volumes/v002/accepted/u002.md");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, accepted).unwrap();
    let data_root = root.path().join("unused-data");
    for kind in ["drafts", "accepted"] {
        let path = root
            .path()
            .join(format!("translations/ru-main/volumes/v002/{kind}/u002.md"));
        let output = cli(&path, &data_root, &["--json"]);
        assert!(output.stderr.is_empty());
        assert_eq!(output.status.code(), Some(0), "{}", json_stdout(&output));
        assert_eq!(
            json_stdout(&output),
            json!({
                "version": 1,
                "status": "unbound",
                "root": root.path().canonicalize().unwrap(),
                "schema_version": 2,
                "instructions_path": root.path().canonicalize().unwrap().join("AGENTS.md"),
                "binding": null,
                "direction_id": "ru-main",
                "source_language": "ja",
                "target_language": "ru",
                "diagnostics": [],
            })
        );
        assert!(!data_root.exists());
    }
}

#[test]
fn relative_cwd_selects_the_same_direction_as_an_absolute_path() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let cases: Value = serde_json::from_str(include_str!(
        "../../../compatibility/rust/cws-project-v1.json"
    ))
    .unwrap();
    let case = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "direction-context-series")
        .unwrap();
    for (relative, contents) in case["files"].as_object().unwrap() {
        let path = project.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents.as_str().unwrap()).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .current_dir(fixture.path())
        .env("PATH", "")
        .args([
            "project-context",
            "--cwd",
            "project/translations/ru-main/volumes/v002",
            "--json",
        ])
        .output()
        .unwrap();
    let report = json_stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(report["direction_id"], "ru-main");
    assert_eq!(report["source_language"], "en");
}
