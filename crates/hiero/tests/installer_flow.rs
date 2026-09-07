//! The bootstrap installer (`scripts/install.sh`) executed end-to-end against
//! local fixture releases built from the real binary: the 8 installer steps
//! with the release-build archive layout. Tests override `HOME` so nothing
//! touches the developer's real home, pass `--unit-dir`/`--no-activate` so
//! the systemd user manager is never contacted, and only use local
//! directories (no network).

#[path = "common/multilingual.rs"]
mod multilingual;

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Test-only corroboration of the installed controller's cached readiness.
/// An authority commit can record newer work before the next controller poll.
fn installed_corpus_is_covered(database: &Path) -> rusqlite::Result<bool> {
    let db = rusqlite::Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.query_row(
        "select not exists(select 1 from semantic_jobs where status in ('queued','running'))
         and (not exists(select 1 from rag_chunks) or exists(
           select 1 from semantic_generations where active=1 and status='active'
           and corpus_revision=coalesce((select revision from corpus_revision where singleton=1),0)))",
        [], |row| row.get(0),
    )
}

#[test]
fn installed_readiness_requires_current_coverage_and_drained_work() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("readiness.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "create table corpus_revision(singleton integer,revision integer);
        create table rag_chunks(id integer);
        create table semantic_jobs(status text);
        create table semantic_generations(active integer,status text,corpus_revision integer);
        insert into corpus_revision values(1,1);
        insert into rag_chunks values(1);
        insert into semantic_generations values(1,'active',1);",
    )
    .unwrap();
    assert!(installed_corpus_is_covered(&path).unwrap());
    db.execute_batch("update corpus_revision set revision=2;")
        .unwrap();
    assert!(
        !installed_corpus_is_covered(&path).unwrap(),
        "a cached Ready cannot cover a newer authority intent"
    );
    db.execute_batch("update semantic_generations set corpus_revision=2; insert into semantic_jobs values('queued');").unwrap();
    assert!(!installed_corpus_is_covered(&path).unwrap());
    db.execute_batch("update semantic_jobs set status='running';")
        .unwrap();
    assert!(!installed_corpus_is_covered(&path).unwrap());
    db.execute_batch("update semantic_jobs set status='complete';")
        .unwrap();
    assert!(installed_corpus_is_covered(&path).unwrap());
    db.execute_batch("delete from rag_chunks; delete from semantic_generations;")
        .unwrap();
    assert!(installed_corpus_is_covered(&path).unwrap());
}

fn real_version() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["version", "--json"])
        .output()
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    payload["version"].as_str().unwrap().to_string()
}

fn sha256_hex(path: &Path) -> String {
    use sha2::Digest;
    let mut file = std::fs::File::open(path).unwrap();
    let mut digest = sha2::Sha256::new();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();
    digest.update(&buffer);
    format!("{:x}", digest.finalize())
}

/// Build a local fixture release in the exact layout
/// `scripts/release-build.sh` produces: archive with the binary plus relative
/// argv[0] links, and the `.sha256` sibling.
fn build_release(root: &Path, binary: &str) -> PathBuf {
    let release_dir = root.join("release");
    let payload = root.join("payload");
    std::fs::create_dir_all(&payload).unwrap();
    std::fs::copy(binary, payload.join("hiero")).unwrap();
    assert!(
        Command::new("strip")
            .arg("--strip-debug")
            .arg(payload.join("hiero"))
            .status()
            .unwrap()
            .success()
    );
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(payload.join("hiero"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(payload.join("hiero"), permissions).unwrap();
    for name in ["hieronymus", "hieronymus-agent-hook", "hieronymus-mcp"] {
        std::os::unix::fs::symlink("hiero", payload.join(name)).unwrap();
    }
    std::fs::create_dir_all(&release_dir).unwrap();
    let version = real_version();
    let name = format!("hieronymus-{version}-x86_64-unknown-linux-gnu.tar.gz");
    let archive = release_dir.join(&name);
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&payload)
        .args([
            "hiero",
            "hieronymus",
            "hieronymus-agent-hook",
            "hieronymus-mcp",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "tar failed");
    std::fs::write(
        release_dir.join(format!("{name}.sha256")),
        format!("{}  {name}\n", sha256_hex(&archive)),
    )
    .unwrap();
    release_dir
}

fn installer_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/install.sh")
        .canonicalize()
        .unwrap()
}

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
        }
    }

    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    fn app(&self) -> PathBuf {
        self.root.path().join("app")
    }

    fn data_root(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn unit_dir(&self) -> PathBuf {
        self.root.path().join("units")
    }

    fn run(&self, extra: &[&str]) -> (String, String, std::process::ExitStatus) {
        let output = Command::new(installer_path())
            .arg("--release-dir")
            .arg(self.root.path().join("release"))
            .arg("--app-dir")
            .arg(self.app())
            .arg("--data-root")
            .arg(self.data_root())
            .arg("--unit-dir")
            .arg(self.unit_dir())
            .arg("--no-activate")
            .args(extra)
            .env("HOME", self.home())
            .output()
            .expect("installer could not be executed");
        (
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
            output.status,
        )
    }
}

// ---------------------------------------------------------------------------
// Bootstrap refusal paths (full package success is the explicit live test)
// ---------------------------------------------------------------------------

#[test]
fn install_refuses_a_checksum_mismatch_and_installs_nothing() {
    let sandbox = Sandbox::new();
    let release_dir = build_release(sandbox.root.path(), env!("CARGO_BIN_EXE_hiero"));
    let version = real_version();
    let checksum = release_dir.join(format!(
        "hieronymus-{version}-x86_64-unknown-linux-gnu.tar.gz.sha256"
    ));
    std::fs::write(&checksum, format!("{:0>64}  tampered\n", "dead")).unwrap();

    let (stdout, stderr, status) = sandbox.run(&[]);
    assert_eq!(status.code(), Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("checksum mismatch"), "{stderr}");
    // Nothing was installed: the app directory was never created.
    assert!(!sandbox.app().exists());
    assert!(!sandbox.unit_dir().join("hieronymus.service").exists());
}

#[test]
fn install_refuses_a_signed_release_in_the_waived_first_line() {
    let sandbox = Sandbox::new();
    let release_dir = build_release(sandbox.root.path(), env!("CARGO_BIN_EXE_hiero"));
    let version = real_version();
    std::fs::write(
        release_dir.join("release.json"),
        format!(
            "{{\"version\": \"{version}\", \
             \"archive\": \"hieronymus-{version}-x86_64-unknown-linux-gnu.tar.gz\", \
             \"sha256\": \"{}\", \
             \"signature\": \"MEUCIQ==\"}}",
            "ab".repeat(32)
        ),
    )
    .unwrap();

    let (stdout, stderr, status) = sandbox.run(&[]);
    assert_eq!(status.code(), Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("signature"), "{stderr}");
    assert!(!sandbox.app().join("bin/hiero").exists());
}

#[test]
fn install_requires_a_release_source() {
    let sandbox = Sandbox::new();
    let output = Command::new(installer_path())
        .env("HOME", sandbox.home())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--release-dir"), "{stderr}");
}

#[test]
fn install_refuses_a_non_https_release_url() {
    let sandbox = Sandbox::new();
    // A loopback port that refuses connections: the installer must reject the
    // scheme before any fetch is attempted.
    let output = Command::new(installer_path())
        .args(["--release-url", "http://127.0.0.1:1/releases"])
        .arg("--app-dir")
        .arg(sandbox.app())
        .arg("--data-root")
        .arg(sandbox.data_root())
        .env("HOME", sandbox.home())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("https://"),
        "the refusal must name the https requirement: {stderr}"
    );
    // Nothing was fetched or installed.
    assert!(!sandbox.app().exists());
}

#[test]
fn install_refuses_missing_semantic_assets_before_activation() {
    let sandbox = Sandbox::new();
    build_release(sandbox.root.path(), env!("CARGO_BIN_EXE_hiero"));
    let (stdout, stderr, status) = sandbox.run(&[]);
    assert!(!status.success(), "{stdout}{stderr}");
    assert!(
        stderr.contains("required semantic assets"),
        "{stdout}{stderr}"
    );
    assert!(!sandbox.app().exists());
    assert!(!sandbox.unit_dir().exists());
}

/// A deliberately executable marker makes pre-execution refusal observable.
fn marker_release(sandbox: &Sandbox, expanded: bool, many_entries: bool) -> (PathBuf, String) {
    let release = sandbox.root.path().join("release");
    std::fs::create_dir_all(&release).unwrap();
    let name = "hieronymus-0.7.0-x86_64-unknown-linux-gnu.tar.gz";
    let archive = release.join(name);
    let encoder = flate2::write::GzEncoder::new(
        std::fs::File::create(&archive).unwrap(),
        flate2::Compression::fast(),
    );
    let mut builder = tar::Builder::new(encoder);
    let marker = b"#!/bin/sh\necho BOOTSTRAP_EXECUTED\nexit 1\n";
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o755);
    header.set_size(marker.len() as u64);
    header.set_cksum();
    builder
        .append_data(&mut header, "hiero", &marker[..])
        .unwrap();
    if expanded {
        // Exercise the production 3 GiB bound with a small compressed fixture.
        let size = 3 * 1024 * 1024 * 1024_u64;
        header.set_size(size);
        header.set_cksum();
        builder
            .append_data(&mut header, "oversized", std::io::repeat(0).take(size))
            .unwrap();
    }
    if many_entries {
        header.set_size(0);
        header.set_cksum();
        for index in 0..20_000 {
            builder
                .append_data(
                    &mut header,
                    format!("entry-{index:05}-{}", "x".repeat(80)),
                    std::io::empty(),
                )
                .unwrap();
        }
    }
    builder.into_inner().unwrap().finish().unwrap();
    let digest = sha256_hex(&archive);
    std::fs::write(release.join(format!("{name}.sha256")), &digest).unwrap();
    (
        release,
        format!(r#""version":"0.7.0","archive":"{name}","sha256":"{digest}""#),
    )
}

#[test]
fn bootstrap_rejects_ambiguous_metadata_before_executing_archive_bytes() {
    let sandbox = Sandbox::new();
    let (release, fields) = marker_release(&sandbox, false, false);
    for extra in [
        r#", "signatu\u0072e":"not-verified""#,
        r#", "signature":null,"signature":null"#,
        r#", "nested":{"signature":"not-verified"}"#,
        r#", "target":"aarch64-unknown-linux-gnu""#,
        r#", "signature":{"value":null}"#,
        r#", "signature":null,"version":"0.7.0""#,
        r#", "signature":null} {"signature":null"#,
    ] {
        std::fs::write(release.join("release.json"), format!("{{{fields}{extra}}}")).unwrap();
        let (stdout, stderr, status) = sandbox.run(&[]);
        assert!(!status.success(), "{extra}: {stdout}{stderr}");
        assert!(stderr.contains("metadata"), "{extra}: {stdout}{stderr}");
        assert!(
            !stdout.contains("BOOTSTRAP_EXECUTED"),
            "{extra}: marker executed"
        );
        assert!(!sandbox.app().exists());
    }
    // The marker is live with an accepted envelope; failures above cannot be
    // attributed to an invalid archive or non-executable fixture.
    std::fs::write(
        release.join("release.json"),
        format!("{{{fields},\"signature\":null}}"),
    )
    .unwrap();
    let (stdout, _, _) = sandbox.run(&[]);
    assert!(stdout.contains("BOOTSTRAP_EXECUTED"), "{stdout}");
}

#[test]
fn bootstrap_bounds_expansion_before_executing_archive_bytes() {
    let sandbox = Sandbox::new();
    marker_release(&sandbox, true, false);
    let (stdout, stderr, status) = sandbox.run(&[]);
    assert!(!status.success(), "{stdout}{stderr}");
    assert!(stderr.contains("expanded size limit"), "{stderr}");
    assert!(!stdout.contains("BOOTSTRAP_EXECUTED"));
    assert!(!sandbox.app().exists());
}

#[test]
fn bootstrap_bounds_many_entry_listing_before_executing_archive_bytes() {
    let sandbox = Sandbox::new();
    marker_release(&sandbox, false, true);
    let (stdout, stderr, status) = sandbox.run(&[]);
    assert!(!status.success(), "{stdout}{stderr}");
    assert!(
        stderr.contains("listing") && stderr.contains("size limit"),
        "{stderr}"
    );
    assert!(!stdout.contains("BOOTSTRAP_EXECUTED"));
    assert!(!sandbox.app().exists());
}

/// The real packaged acceptance path: no native assets are copied into the
/// data root and no semantic-enable/runtime override is sent. Every daemon
/// and CLI subprocess runs with an empty PATH and no loader override.
#[test]
#[ignore = "requires HIERO_TEST_RELEASE_DIR produced by scripts/release-build.sh"]
fn installed_release_boots_bundled_assets_and_runs_multilingual_retrieval() {
    use serde_json::{Value, json};
    use std::time::{Duration, Instant};
    let release =
        PathBuf::from(std::env::var_os("HIERO_TEST_RELEASE_DIR").expect("built release directory"));
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.home()).unwrap();
    let install = || {
        Command::new(installer_path())
            .arg("--release-dir")
            .arg(&release)
            .arg("--app-dir")
            .arg(sandbox.app())
            .arg("--data-root")
            .arg(sandbox.data_root())
            .arg("--unit-dir")
            .arg(sandbox.unit_dir())
            .arg("--no-activate")
            .env("HOME", sandbox.home())
            .env_remove("HIERONYMUS_RELEASE_URL")
            .env_remove("HIERONYMUS_RELEASE_DIR")
            .output()
            .unwrap()
    };
    let output = install();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let binary = sandbox.app().join("bin/hiero");
    let empty_path = sandbox.root.path().join("empty-path");
    std::fs::create_dir(&empty_path).unwrap();
    let command = || {
        let mut cmd = Command::new(&binary);
        cmd.env("HOME", sandbox.home())
            .env("PATH", &empty_path)
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("HIERO_SEMANTIC_MODEL_DIR")
            .env_remove("ORT_DYLIB_PATH");
        cmd
    };
    let cli = |args: &[&str]| -> Value {
        let output = command()
            .args(args)
            .arg("--data-root")
            .arg(sandbox.data_root())
            .arg("--json")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let doctor = cli(&["doctor"]);
    assert!(
        doctor
            .to_string()
            .contains("qualified ONNX runtime selected"),
        "{doctor}"
    );
    assert!(
        doctor.to_string().contains("models/minilm/model.onnx"),
        "{doctor}"
    );
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let log = std::fs::File::create(sandbox.root.path().join("daemon.log")).unwrap();
    let start = Instant::now();
    let _daemon = Child(
        command()
            .arg("daemon")
            .arg("--data-root")
            .arg(sandbox.data_root())
            .stdout(std::process::Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let mut states = Vec::new();
    let wait_ready = || -> Value {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let output = command()
                .args(["status", "--json", "--data-root"])
                .arg(sandbox.data_root())
                .output()
                .unwrap();
            if let Ok(status) = serde_json::from_slice::<Value>(&output.stdout) {
                let semantic = &status["status"]["semantic"];
                if semantic["state"] == "ready"
                    && installed_corpus_is_covered(&sandbox.data_root().join("hieronymus.sqlite"))
                        .unwrap()
                {
                    return status;
                }
                assert_ne!(semantic["state"], json!("failed"), "{status}");
            }
            assert!(
                Instant::now() < deadline,
                "installed daemon readiness timed out"
            );
            std::thread::sleep(Duration::from_millis(250));
        }
    };
    let ready = wait_ready();
    states.push(ready);
    eprintln!(
        "F1 installed cold-ready seconds={:.3} (pre-staged assets, no OS cache purge)",
        start.elapsed().as_secs_f64()
    );
    assert!(
        !sandbox.data_root().join("semantic.conf").exists(),
        "bundled startup must not persist another version's runtime path"
    );
    let tool = |name: &str, args: Value| -> Value {
        let reply = cli(&["tool-call", name, "--args", &args.to_string()]);
        assert_ne!(reply["result"]["isError"], json!(true), "{reply}");
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/hybrid-relevance.json")).unwrap();
    let contexts = multilingual::seed(&tool, &fixture, &sandbox.root.path().join("sources"), true);
    wait_ready();
    let mut failures = Vec::new();
    for query in fixture["queries"].as_array().unwrap() {
        for endpoint in ["hieronymus_recall", "hieronymus_rag_search"] {
            wait_ready();
            let context = &contexts[query["series_slug"].as_str().unwrap()];
            let mut args =
                multilingual::query_context(context, json!({"query":query["query"],"limit":8}));
            if endpoint == "hieronymus_recall" {
                args["session_id"] = context["session_id"].clone();
            }
            let payload = tool(endpoint, args);
            if endpoint == "hieronymus_recall" {
                if let Some(text) = query["expected_memory_text"].as_str()
                    && !payload["results"].as_array().unwrap().iter().any(|row| {
                        row["text"] == text
                            && row["rank_reason"] == "active session short-term memory match"
                    })
                {
                    failures.push(format!("{} missing learned memory", query["query_id"]));
                }
                if let Some(term) = query.get("expected_contract_term") {
                    let matched = payload["deterministic_contract"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|row| {
                            row["source_text"] == term["source_text"]
                                && row["canonical_translation"] == term["canonical_translation"]
                        });
                    if !matched {
                        failures.push(format!(
                            "{} missing deterministic contract",
                            query["query_id"]
                        ));
                    }
                }
            }
            let rows: Vec<&Value> = if endpoint == "hieronymus_recall" {
                payload["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|row| row.get("chunk_kind").is_some())
                    .collect()
            } else {
                payload["results"].as_array().unwrap().iter().collect()
            };
            let docs = fixture["documents"].as_array().unwrap();
            let resolve = |row: &&Value| {
                docs.iter().find(|doc| {
                    row["source_ref"]
                        .as_str()
                        .unwrap_or("")
                        .contains(&format!("{}.txt", doc["doc_id"].as_str().unwrap()))
                })
            };
            let top: Vec<&str> = rows
                .iter()
                .take(3)
                .filter_map(resolve)
                .map(|doc| doc["doc_id"].as_str().unwrap())
                .collect();
            for expected in query["expected_top3_doc_ids"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if !top.contains(&expected.as_str().unwrap()) {
                    failures.push(format!(
                        "{} {endpoint} missing {expected}, top={top:?}",
                        query["query_id"]
                    ));
                }
            }
            for row in &rows {
                match resolve(row) {
                    Some(doc) if doc["series_slug"] == query["series_slug"] => {}
                    _ => failures.push(format!(
                        "{} {endpoint} invalid series/source",
                        query["query_id"]
                    )),
                }
                if query["require_semantic_provenance"] == true
                    && !row.to_string().contains("rag semantic match")
                {
                    failures.push(format!(
                        "{} {endpoint} missing semantic provenance",
                        query["query_id"]
                    ));
                }
            }
            eprintln!("F1 installed {} {endpoint} top3={top:?}", query["query_id"]);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let daemon_pid = _daemon.0.id();
    let process_status = std::fs::read_to_string(format!("/proc/{daemon_pid}/status")).unwrap();
    let memory: Vec<_> = process_status
        .lines()
        .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
        .collect();
    eprintln!("F1 installed daemon pid={daemon_pid} corpus-end memory={memory:?}");
    let mappings = std::fs::read_to_string(format!("/proc/{daemon_pid}/maps")).unwrap();
    let runtime_mapping = mappings
        .lines()
        .find(|line| line.contains("libonnxruntime.so"))
        .expect("actual installed native runtime mapping");
    assert!(
        runtime_mapping.contains("/app/versions/"),
        "{runtime_mapping}"
    );
    eprintln!("F1 installed runtime mapping={runtime_mapping}");
    drop(_daemon);
    let rerun = install();
    assert!(
        rerun.status.success(),
        "{}{}",
        String::from_utf8_lossy(&rerun.stdout),
        String::from_utf8_lossy(&rerun.stderr)
    );
    eprintln!("F1 installed package clean install, bundled runtime, corpus, offline rerun passed");
}
