//! The provider `check`/`models` REST routes in real mode: every configured
//! profile — a `.invalid` host included — is probed through the real provider
//! client over the blocking transport. The old `source: "fixture"` shortcut
//! for `*.invalid` hosts was removed per Astra finding 12 / plan W3; the
//! reconciled contract is the current provider route tests.
//! Task-4 slice; the frozen contracts themselves live in
//! `daemon_rest_routes.rs`. All traffic here stays on loopback (ADR 0012).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

mod common;

use common::{browser_headers, send_request, start_daemon_with_browser_session, wait_until};

const SECRET_KEY: &str = "route-test-key-123";
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";

/// One recorded request: (path, headers).
type RecordedRequest = (String, BTreeMap<String, String>);

/// Minimal loopback stand-in for an OpenAI-compatible provider.
struct LoopbackModels {
    port: u16,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    accept_thread: Option<std::thread::JoinHandle<()>>,
}

impl LoopbackModels {
    /// `status_and_body` decides per request; `hit_count` counts accepted
    /// requests so tests can assert how often the daemon probed.
    fn start(status_and_body: impl Fn(usize) -> (u16, String) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(status_and_body);
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let accept_thread = std::thread::spawn(move || {
            let mut hits = 0_usize;
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let Some((path, headers)) = read_request(&mut stream) else {
                            continue;
                        };
                        thread_requests.lock().unwrap().push((path, headers));
                        hits += 1;
                        let (status, body) = handler(hits);
                        let response = format!(
                            "HTTP/1.1 {status} probe\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            requests,
            stop,
            accept_thread: Some(accept_thread),
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn authorizations(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(_, headers)| headers.get("authorization").cloned().unwrap_or_default())
            .collect()
    }
}

impl Drop for LoopbackModels {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            handle.join().unwrap();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<(String, BTreeMap<String, String>)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 4096];
    let separator = loop {
        if let Some(position) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
        let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
        stream.set_read_timeout(Some(remaining)).ok()?;
        let count = stream.read(&mut buffer).ok()?;
        if count == 0 {
            return None;
        }
        raw.extend_from_slice(&buffer[..count]);
        if raw.len() > 65_536 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&raw[..separator]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let path = request_line.split_whitespace().nth(1)?.to_string();
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    Some((path, headers))
}

fn save_profile(fixture: &common::RouteFixture, id: &str, url: &str) -> Value {
    let body = json!({
        "provider": {
            "id": id,
            "name": "Local LLM",
            "type": "openai",
            "url": url,
            "key": SECRET_KEY,
            "timeout_seconds": "5",
        }
    });
    let response = send_request(
        fixture.port,
        "POST",
        "/api/providers",
        &browser_headers(fixture, &[("Origin", common::same_origin(fixture.port))]),
        body.to_string().as_bytes(),
    );
    assert_eq!(response.status, 200, "{:?}", response.raw_body);
    response.body()
}

fn get_models(fixture: &common::RouteFixture, id: &str) -> Value {
    let response = send_request(
        fixture.port,
        "GET",
        &format!("/api/providers/{id}/models"),
        &browser_headers(fixture, &[("Origin", common::same_origin(fixture.port))]),
        b"",
    );
    assert_eq!(response.status, 200, "{:?}", response.raw_body);
    assert_eq!(response.content_type, JSON_CONTENT_TYPE);
    response.body()
}

fn post_check(fixture: &common::RouteFixture, id: &str) -> Value {
    let response = send_request(
        fixture.port,
        "POST",
        &format!("/api/providers/{id}/check"),
        &browser_headers(fixture, &[("Origin", common::same_origin(fixture.port))]),
        b"",
    );
    assert_eq!(response.status, 200, "{:?}", response.raw_body);
    response.body()
}

#[test]
fn provider_models_and_check_probe_the_configured_endpoint() {
    let llm = LoopbackModels::start(|hits| {
        if hits == 1 {
            // One retryable failure first: the probe must retry through it.
            (503, "overloaded".to_string())
        } else {
            (
                200,
                json!({"data": [{"id": "model-b"}, {"id": "model-a"}]}).to_string(),
            )
        }
    });
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    save_profile(&fixture, "local-llm", &llm.url());

    let models = get_models(&fixture, "local-llm");
    assert_eq!(
        models,
        json!({"models": ["model-a", "model-b"], "source": "api", "error": ""})
    );

    let check = post_check(&fixture, "local-llm");
    assert_eq!(
        check,
        json!({
            "check": {
                "ok": true,
                "models": ["model-a", "model-b"],
                "source": "api",
                "error": "",
            },
            "error": "",
        })
    );
    assert!(
        wait_until(|| llm.request_count() >= 3, Duration::from_secs(5)),
        "the daemon must probe the real endpoint (with one retry), saw {} requests",
        llm.request_count()
    );
    assert!(
        llm.authorizations()
            .iter()
            .all(|value| value == &format!("Bearer {SECRET_KEY}")),
        "the configured key authenticates the outbound probe"
    );
    // Sentinel: the key never echoes back through the route payloads.
    for payload in [models, check] {
        assert!(
            !payload.to_string().contains(SECRET_KEY),
            "route payload leaked the key: {payload}"
        );
    }
}

#[test]
fn provider_models_fall_back_to_defaults_when_the_endpoint_fails() {
    let llm = LoopbackModels::start(|_hits| (500, "boom".to_string()));
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    save_profile(&fixture, "local-llm", &llm.url());

    let models = get_models(&fixture, "local-llm");
    assert_eq!(
        models,
        json!({
            "models": ["gpt-4.1-mini", "gpt-4.1", "o4-mini"],
            "source": "defaults",
            "error": "model suggestions unavailable",
        })
    );

    let check = post_check(&fixture, "local-llm");
    assert_eq!(check["check"]["ok"], json!(false));
    assert_eq!(check["check"]["error"], "model suggestions unavailable");
    assert_eq!(check["error"], "");
}

#[test]
fn provider_routes_without_a_configured_key_report_the_python_error() {
    let llm = LoopbackModels::start(|_hits| (200, "{}".to_string()));
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    // Save with an empty key: the stored profile has no credential.
    let body = json!({
        "provider": {
            "id": "local-llm",
            "name": "Local LLM",
            "type": "openai",
            "url": llm.url(),
            "key": "",
            "timeout_seconds": "5",
        }
    });
    let saved = send_request(
        fixture.port,
        "POST",
        "/api/providers",
        &browser_headers(&fixture, &[("Origin", common::same_origin(fixture.port))]),
        body.to_string().as_bytes(),
    );
    assert_eq!(saved.status, 200, "{:?}", saved.raw_body);

    let models = get_models(&fixture, "local-llm");
    assert_eq!(models["source"], json!("defaults"));
    assert_eq!(
        models["error"],
        json!("API key missing for provider profile")
    );
    assert_eq!(
        llm.request_count(),
        0,
        "keyless profiles never hit the wire"
    );
}

#[test]
fn a_dot_invalid_profile_is_probed_for_real_never_faked() {
    // Astra finding 12 / plan W3: the `.invalid` host shortcut that answered
    // `{"source": "fixture", "models": ["synthetic-model"]}` was removed.
    // `provider.invalid` can never resolve, so the real probe now returns a
    // genuine transport failure. The reconciled contract per ADR 0012 /
    // Astra 12 is recorded in the current provider route tests; the
    // frozen fixture bytes are untouched.
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let saved = save_profile(
        &fixture,
        "synthetic-provider",
        "https://provider.invalid/v1",
    );
    assert_eq!(
        saved["provider"]["url"],
        json!("https://provider.invalid/v1")
    );

    let models = get_models(&fixture, "synthetic-provider");
    assert_ne!(models["source"], json!("fixture"), "{models}");
    assert!(
        models["models"]
            .as_array()
            .is_some_and(|list| list.iter().all(|model| model != "synthetic-model")),
        "no synthetic model may be fabricated: {models}"
    );
    assert_eq!(models["source"], json!("defaults"));
    assert_eq!(models["error"], json!("model suggestions unavailable"));

    let check = post_check(&fixture, "synthetic-provider");
    assert_ne!(check["check"]["source"], json!("fixture"), "{check}");
    assert_eq!(
        check["check"]["ok"],
        json!(false),
        "a .invalid host never actually connects: {check}"
    );
}

#[test]
fn merge_preview_is_read_only_and_confirmation_rejects_changed_sources() {
    let model = LoopbackModels::start(|_| {
        (200, json!({"choices":[{"message":{"content":r#"{"title":"Names","text":"Alto lives in Verel."}"#}}]}).to_string())
    });
    let (fixture, root, daemon) = start_daemon_with_browser_session();
    save_profile(&fixture, "merge-model", &model.url());
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    let mut settings = hieronymus::dream_config::default_dream_config();
    let assignment = settings.workflows.get_mut("knowledge_crystals").unwrap();
    assignment.provider = "merge-model".into();
    assignment.model = "merge-test".into();
    assignment.enabled = false;
    hieronymus::dream_config::save_dream_config(&config, &settings).unwrap();
    hieronymus::registry::Registry::open(&config)
        .unwrap()
        .create_series("book", "Book", "ja", "ru", None)
        .unwrap();
    let store = hieronymus::crystals::CrystalStore::open(&config).unwrap();
    let context =
        hieronymus::memory_models::TranslationContext::new("book", "ja", "ru", "translate");
    let first = store
        .add_crystal(
            &context,
            "concept",
            &hieronymus::crystals::NewCrystal::new("concept", "The hero is Alto."),
        )
        .unwrap();
    let second = store
        .add_crystal(
            &context,
            "concept",
            &hieronymus::crystals::NewCrystal::new("concept", "He lives in Verel."),
        )
        .unwrap();
    let post = |path: &str, body: Value| {
        send_request(
            fixture.port,
            "POST",
            path,
            &browser_headers(&fixture, &[("Origin", common::same_origin(fixture.port))]),
            body.to_string().as_bytes(),
        )
    };
    let preview = post(
        "/api/admin/merge-preview",
        json!({"ids":[first,second],"view":"Crystals"}),
    );
    assert_eq!(preview.status, 200, "{:?}", preview.raw_body);
    let proposal = preview.body();
    assert_eq!(proposal["text"], "Alto lives in Verel.");
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    let active = || {
        db.query_row(
            "select count(*) from crystals where status='active'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert_eq!(active(), 2);
    let mut body = json!({"view":"Crystals","ids":[first,second],"confirmed":true,"title":proposal["title"],"text":proposal["text"],"source_snapshots":proposal["source_snapshots"]});
    db.execute(
        "update crystals set text='Alto lives in another city.' where id=?1",
        [second],
    )
    .unwrap();
    let stale = post("/api/admin/actions/merge_selected", body.clone());
    assert_eq!(stale.status, 400);
    assert!(stale.body()["error"].as_str().unwrap().contains("changed"));
    assert_eq!(active(), 2);
    let fresh = post(
        "/api/admin/merge-preview",
        json!({"ids":[first,second],"view":"Crystals"}),
    );
    assert_eq!(fresh.status, 200, "{:?}", fresh.raw_body);
    body["source_snapshots"] = fresh.body()["source_snapshots"].clone();
    body["text"] = json!("Alto lives in another city.");
    let saved = post("/api/admin/actions/merge_selected", body);
    assert_eq!(saved.status, 200, "{:?}", saved.raw_body);
    assert_eq!(active(), 1);
    assert_eq!(model.request_count(), 2);
    daemon.shutdown().unwrap();
}
