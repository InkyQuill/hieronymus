use assert_cmd::Command;

#[test]
fn version_reports_package_version() {
    let mut command = Command::cargo_bin("hiero").expect("hiero binary should be built");

    command
        .arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains("hiero 0.6.0"));
}
