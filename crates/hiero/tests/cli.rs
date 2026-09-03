//! Integration tests for the `hiero` binary skeleton: version output and the
//! classify command in human and JSON forms.

use std::process::Command;

fn hiero(arguments: &[&str]) -> (String, String, std::process::ExitStatus) {
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(arguments)
        .output()
        .unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
        output.status,
    )
}

#[test]
fn version_prints_human_alpha_marker() {
    let (stdout, stderr, status) = hiero(&["version"]);
    assert!(status.success());
    assert!(stderr.is_empty());
    assert!(stdout.contains("hiero v"), "{stdout}");
    assert!(stdout.contains('\u{03b1}'), "{stdout}");
}

#[test]
fn version_prints_json_when_requested() {
    let (stdout, _stderr, status) = hiero(&["version", "--json"]);
    assert!(status.success());
    assert!(stdout.starts_with("{\"version\": \""), "{stdout}");
}

#[test]
fn classify_reports_empty_database_as_json() {
    let root = tempfile::tempdir().unwrap();
    let (stdout, _stderr, status) = hiero(&[
        "classify",
        "--json",
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert!(status.success(), "{stdout}");
    assert!(stdout.contains("\"state\": \"empty\""), "{stdout}");
    assert!(stdout.contains("hieronymus.sqlite"), "{stdout}");
}

#[test]
fn classify_reports_python_schema_in_human_form() {
    let root = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open(root.path().join("hieronymus.sqlite")).unwrap();
    for table in [
        "series",
        "task_sessions",
        "short_term_memories",
        "strict_terms",
    ] {
        connection
            .execute(
                &format!("create table {table} (id integer primary key)"),
                [],
            )
            .unwrap();
    }
    drop(connection);

    let (stdout, _stderr, status) =
        hiero(&["classify", "--data-root", root.path().to_str().unwrap()]);
    assert!(status.success(), "{stdout}");
    assert!(stdout.contains("database state: python-schema"), "{stdout}");
}

#[test]
fn unknown_command_exits_two_with_usage_hint() {
    let (_stdout, stderr, status) = hiero(&["make-chaos"]);
    assert_eq!(status.code(), Some(2));
    assert!(stderr.contains("unknown command: make-chaos"), "{stderr}");
    assert!(stderr.contains("usage: hiero"), "{stderr}");
}
