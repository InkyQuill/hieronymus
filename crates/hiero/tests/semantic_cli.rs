//! `hiero semantic status|enable`: the semantic lane arming surface. Status is
//! report-only (never downloads, never loads the ONNX runtime); enable is the
//! explicit acquisition operation over the TLS-capable model transport, with
//! the loopback URL/checksum/size overrides keeping the tests offline.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::rag::{RagImport, RagStore};
use hieronymus::registry::Registry;
use hieronymus::semantic_embeddings::{
    EMBEDDING_DIMENSIONS, EmbeddingProvider as _, FakeEmbeddingProvider,
};
use hieronymus::semantic_store::{SemanticChunk, SemanticSample, SemanticStore};
use hieronymus::semantic_tokenizer::ModelTokenizer;
use hieronymus::workspace::WorkspaceStore;
use sha2::Digest;

fn hiero(arguments: &[&str]) -> (String, String, std::process::ExitStatus) {
    let _daemon = if arguments.get(1) == Some(&"enable") {
        let root = arguments
            .windows(2)
            .find(|args| args[0] == "--data-root")
            .unwrap()[1];
        Some(
            hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
                data_root: Some(root.into()),
                port: 0,
                ..Default::default()
            })
            .unwrap(),
        )
    } else {
        None
    };
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

/// Serves `body` once per connection over plain loopback HTTP with an exact
/// Content-Length (the acquisition path verifies the checksum itself).
struct LoopbackFile {
    url: String,
    requests: Arc<Mutex<usize>>,
    stop: Arc<AtomicBool>,
    #[allow(dead_code)] // joined on drop
    accept_thread: Option<std::thread::JoinHandle<()>>,
}

impl LoopbackFile {
    fn start(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let accept_thread = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            loop {
                if thread_stop.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                *thread_requests.lock().unwrap() += 1;
                let mut buffer = [0_u8; 4096];
                let _ = socket.read(&mut buffer);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes());
                let _ = socket.write_all(&body);
                let _ = socket.flush();
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}/model.onnx"),
            requests,
            stop,
            accept_thread: Some(accept_thread),
        }
    }

    fn request_count(&self) -> usize {
        *self.requests.lock().unwrap()
    }
}

impl Drop for LoopbackFile {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

fn artifact(body: &[u8]) -> (String, String) {
    (
        format!("{:x}", sha2::Sha256::digest(body)),
        body.len().to_string(),
    )
}

// ---------------------------------------------------------------------------
// status: report-only
// ---------------------------------------------------------------------------

#[test]
fn status_on_a_fresh_root_reports_the_missing_model() {
    let root = tempfile::tempdir().unwrap();
    let data_root = root.path().to_str().unwrap();
    let (stdout, stderr, status) = hiero(&["semantic", "status", "--data-root", data_root]);
    assert!(status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("missing"), "{stdout}");
    assert!(stdout.contains("fts-only"), "{stdout}");
    assert!(stdout.contains("wordpiece"), "{stdout}");

    let (stdout, _, status) = hiero(&["semantic", "status", "--json", "--data-root", data_root]);
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["model"], "missing");
    assert_eq!(payload["generation"], serde_json::Value::Null);
    assert_eq!(
        payload["tokenizer"],
        hieronymus::semantic_tokenizer::MINILM_TOKENIZER_ID
    );
    assert_eq!(payload["intact"], true);
}

#[test]
fn status_reports_an_active_generation_without_touching_the_network() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let registry = Registry::open(&config).unwrap();
    registry
        .create_series("demo", "demo", "ja", "en", None)
        .unwrap();
    let workspace = WorkspaceStore::open(&config).unwrap();
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    workspace.start_session(&context).unwrap();

    let source = root.path().join("a.txt");
    std::fs::write(&source, "Cooking Talent appears here.").unwrap();
    RagStore::open(&config)
        .unwrap()
        .import_file("demo", &source, &RagImport::new())
        .unwrap();

    let store = SemanticStore::open(&config).unwrap();
    let tokenizer = ModelTokenizer::from_bytes(include_bytes!(
        "../../hieronymus/tests/fixtures/minilm-tokenizer.json"
    ))
    .unwrap();
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    loop {
        let pending = store.pending_chunk_ids("gen-a", 8).unwrap();
        if pending.is_empty() {
            break;
        }
        let chunks: Vec<SemanticChunk> = pending
            .iter()
            .map(|chunk_id| {
                let (series_slug, text) = store.chunk_row(*chunk_id).unwrap().unwrap();
                SemanticChunk {
                    chunk_id: *chunk_id,
                    series_slug,
                    token_ids: tokenizer.encode(&text).unwrap(),
                }
            })
            .collect();
        store.write_batch("gen-a", &mut provider, &chunks).unwrap();
    }
    store
        .activate_generation(
            "gen-a",
            &mut provider,
            &SemanticSample {
                series_slug: "demo".to_string(),
                token_ids: tokenizer.encode("probe").unwrap(),
            },
        )
        .unwrap();

    let data_root = config.data_root().to_str().unwrap();
    let (stdout, _, status) = hiero(&["semantic", "status", "--json", "--data-root", data_root]);
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["model"], "missing");
    assert_eq!(payload["generation"]["generation_id"], "gen-a");
    assert_eq!(payload["generation"]["model"], "fake-model");
    assert_eq!(payload["intact"], true);
}

// ---------------------------------------------------------------------------
// enable: explicit acquisition (never runs at daemon start or config load)
// ---------------------------------------------------------------------------

#[test]
fn enable_acquires_the_model_from_an_explicit_loopback_url() {
    let body = b"fake-onnx-artifact-bytes".to_vec();
    let server = LoopbackFile::start(body.clone());
    let (sha, size) = artifact(&body);
    let root = tempfile::tempdir().unwrap();
    let data_root = root.path().to_str().unwrap();

    let (stdout, stderr, status) = hiero(&[
        "semantic",
        "enable",
        "--url",
        &server.url,
        "--sha256",
        &sha,
        "--bytes",
        &size,
        "--data-root",
        data_root,
    ]);
    assert!(status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("acquired"), "{stdout}");
    assert_eq!(server.request_count(), 1);

    // A second enable on an already-acquired model is a no-op: no download.
    let (stdout, _, status) = hiero(&[
        "semantic",
        "enable",
        "--url",
        &server.url,
        "--sha256",
        &sha,
        "--bytes",
        &size,
        "--data-root",
        data_root,
    ]);
    assert!(status.success(), "{stdout}");
    assert!(stdout.contains("already acquired"), "{stdout}");
    assert_eq!(server.request_count(), 1);
}

#[test]
fn enable_json_reports_acquisition_and_lane_verdict() {
    let body = b"fake-onnx-artifact-bytes".to_vec();
    let server = LoopbackFile::start(body.clone());
    let (sha, size) = artifact(&body);
    let root = tempfile::tempdir().unwrap();

    let (stdout, _, status) = hiero(&[
        "semantic",
        "enable",
        "--json",
        "--url",
        &server.url,
        "--sha256",
        &sha,
        "--bytes",
        &size,
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["model_status"], "available");
    assert_eq!(payload["downloaded"], true);
    assert_eq!(payload["runtime_verified"], false);
    assert_eq!(payload["lane"], "disarmed");
    assert!(
        payload["reason"].as_str().unwrap().contains("runtime"),
        "{payload}"
    );
}

#[test]
fn enable_rejects_a_checksum_mismatch_without_promoting() {
    let body = b"fake-onnx-artifact-bytes".to_vec();
    let server = LoopbackFile::start(body.clone());
    let root = tempfile::tempdir().unwrap();
    let wrong_sha = format!("{:x}", sha2::Sha256::digest(b"something else"));

    let (_, stderr, status) = hiero(&[
        "semantic",
        "enable",
        "--url",
        &server.url,
        "--sha256",
        &wrong_sha,
        "--bytes",
        &body.len().to_string(),
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("checksum"), "{stderr}");

    let config = HieronymusConfig::new(root.path());
    let store = SemanticStore::open(&config).unwrap();
    assert!(
        !store.model_path().exists(),
        "a failed acquisition must not promote anything"
    );
}

#[test]
fn enable_reacquires_an_invalid_model_file() {
    let body = b"fake-onnx-artifact-bytes".to_vec();
    let server = LoopbackFile::start(body.clone());
    let (sha, size) = artifact(&body);
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let store = SemanticStore::open(&config).unwrap();
    std::fs::create_dir_all(store.model_path().parent().unwrap()).unwrap();
    std::fs::write(store.model_path(), b"stale").unwrap();

    let (stdout, _, status) = hiero(&[
        "semantic",
        "enable",
        "--url",
        &server.url,
        "--sha256",
        &sha,
        "--bytes",
        &size,
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert!(status.success(), "{stdout}");
    assert_eq!(server.request_count(), 1);
    assert_eq!(std::fs::read(store.model_path()).unwrap(), body);
}

#[test]
fn enable_rejects_unsupported_url_schemes_without_dialing() {
    let root = tempfile::tempdir().unwrap();
    let (_, stderr, status) = hiero(&[
        "semantic",
        "enable",
        "--url",
        "ftp://example.invalid/model.onnx",
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(status.code(), Some(2));
    assert!(stderr.contains("unsupported"), "{stderr}");
}

#[test]
fn enable_with_bytes_override_never_trusts_a_pinned_size_file() {
    use hieronymus::semantic_model::MODEL_BYTES;

    // Regression: a file that matches the PINNED size says nothing about the
    // requested override artifact. It must not count as "available" — the
    // user's --sha256 has never been verified against it.
    let body = b"fake-onnx-artifact-bytes".to_vec();
    let server = LoopbackFile::start(body.clone());
    let (sha, size) = artifact(&body);
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let store = SemanticStore::open(&config).unwrap();
    std::fs::create_dir_all(store.model_path().parent().unwrap()).unwrap();
    let pinned_size_placeholder = std::fs::File::create(store.model_path()).unwrap();
    pinned_size_placeholder.set_len(MODEL_BYTES).unwrap();

    let (stdout, _, status) = hiero(&[
        "semantic",
        "enable",
        "--json",
        "--url",
        &server.url,
        "--sha256",
        &sha,
        "--bytes",
        &size,
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["downloaded"], true, "{payload}");
    assert_eq!(payload["model_status"], "available", "{payload}");
    assert_eq!(server.request_count(), 1);
    assert_eq!(std::fs::read(store.model_path()).unwrap(), body);
}

#[test]
fn enable_with_bytes_override_detects_a_checksum_mismatched_file() {
    // Same size as the requested artifact, wrong content: size alone must
    // never pass the --bytes availability check — the checksum is verified
    // (and the repair re-downloads the correct bytes).
    let right = b"fake-onnx-artifact-bytes";
    let wrong = b"fake-onnx-artifact-WRONG";
    let server = LoopbackFile::start(right.to_vec());
    let (sha, size) = artifact(right);
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let store = SemanticStore::open(&config).unwrap();
    std::fs::create_dir_all(store.model_path().parent().unwrap()).unwrap();
    std::fs::write(store.model_path(), wrong).unwrap();

    let (stdout, _, status) = hiero(&[
        "semantic",
        "enable",
        "--json",
        "--url",
        &server.url,
        "--sha256",
        &sha,
        "--bytes",
        &size,
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["downloaded"], true, "{payload}");
    assert_eq!(payload["model_status"], "available", "{payload}");
    assert_eq!(server.request_count(), 1);
    assert_eq!(std::fs::read(store.model_path()).unwrap(), right);
}

#[test]
fn malformed_semantic_configuration_is_not_absence() {
    use hieronymus::semantic_arming::load_runtime_library;
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    assert!(load_runtime_library(&config).unwrap().is_none());
    for text in [
        "runtime_library = [",
        "runtime_library = 3",
        "",
        "runtime_library = 'relative.so'",
    ] {
        std::fs::write(root.path().join("semantic.conf"), text).unwrap();
        assert!(load_runtime_library(&config).is_err(), "{text}");
        assert!(
            hieronymus::state_classifier::classify(&config).is_err(),
            "{text}"
        );
    }
}

#[test]
fn enable_without_an_owner_does_not_create_state() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "semantic",
            "enable",
            "--data-root",
            root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn cancelled_cli_leaves_acquisition_under_daemon_ownership() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let daemon = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.path().into()),
        port: 0,
        ..Default::default()
    })
    .unwrap();
    let body = b"owner completes this verified staged model";
    let (sha, size) = artifact(body);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/model.onnx", listener.local_addr().unwrap());
    let (started, receive_started) = std::sync::mpsc::channel();
    let (release, receive_release) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut received = 0;
        while !request[..received].windows(4).any(|end| end == b"\r\n\r\n") {
            assert!(
                received < request.len(),
                "request headers exceed the test server bound"
            );
            let count = socket.read(&mut request[received..]).unwrap();
            assert!(
                count > 0,
                "connection closed before complete request headers"
            );
            received += count;
        }
        assert!(request[..received].starts_with(b"GET /model.onnx HTTP/1.1\r\n"));
        started.send(()).unwrap();
        receive_release
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        socket.write_all(body).unwrap();
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "semantic",
            "enable",
            "--json",
            "--url",
            &url,
            "--sha256",
            &sha,
            "--bytes",
            &size,
            "--data-root",
            root.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    receive_started
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
            data_root: Some(root.path().into()),
            port: 0,
            ..Default::default()
        })
        .is_err()
    );
    release.send(()).unwrap();
    server.join().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let model = SemanticStore::model_path_for(&config);
    while !model.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(std::fs::read(model).unwrap(), body);
    assert!(!root.path().join("semantic.conf").exists());
    daemon.shutdown().unwrap();
}
