use assert_cmd::Command;
use predicates::prelude::*;

fn isolated_command(temporary: &tempfile::TempDir) -> Command {
    let mut command = Command::cargo_bin("hiero").expect("hiero binary should build");
    command
        .env("HOME", temporary.path().join("home"))
        .env("XDG_CONFIG_HOME", temporary.path().join("config"))
        .env_remove("HIERONYMUS_PORT");
    command
}

#[test]
fn doctor_json_is_the_only_stdout_payload() {
    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let assert = isolated_command(&temporary)
        .args([
            "--data-root",
            temporary
                .path()
                .join("data")
                .to_str()
                .expect("temporary path should be Unicode"),
            "doctor",
            "--json",
        ])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());

    let payload: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout)
        .expect("stdout should be one JSON value");
    assert!(payload["checks"].is_array());
}

#[test]
fn doctor_human_output_labels_every_status() {
    let temporary = tempfile::tempdir().expect("temporary root should be created");
    isolated_command(&temporary)
        .args([
            "--data-root",
            temporary
                .path()
                .join("data")
                .to_str()
                .expect("temporary path should be Unicode"),
            "doctor",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("System diagnostics"))
        .stdout(predicate::str::contains("WARN database"));
}

#[test]
fn phase_dependent_commands_fail_without_fake_success_output() {
    let temporary = tempfile::tempdir().expect("temporary root should be created");
    isolated_command(&temporary)
        .args(["--json", "search", "series", "query"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "search command is unavailable until its Rust service is implemented",
        ));
}
