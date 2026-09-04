//! The provider `check`/`models` REST routes in real mode: configured
//! profiles with reachable (loopback) endpoints are probed through the real
//! provider client, while the frozen synthetic fixture world
//! (`https://provider.invalid/v1`) keeps answering `source: "fixture"`.
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
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 4096];
    let separator = loop {
        if let Some(position) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
        let count = stream.read(&mut buffer).ok()?;
        if count == 0 {
            return None;
        }
        raw.extend_from_slice(&buffer[..count]);
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
fn synthetic_fixture_world_keeps_the_frozen_semantics() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    // The frozen oracle world: provider.invalid can never resolve, so the
    // routes must answer with fixture semantics, not network attempts.
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
    assert_eq!(
        models,
        json!({"models": ["synthetic-model"], "source": "fixture", "error": ""})
    );
    let check = post_check(&fixture, "synthetic-provider");
    assert_eq!(
        check["check"],
        json!({"ok": true, "models": ["synthetic-model"], "source": "fixture", "error": ""})
    );
}
