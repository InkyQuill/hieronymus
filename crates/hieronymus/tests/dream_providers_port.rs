//! Task-4 slice: the configured LLM provider clients behind the dreaming
//! provider seam plus the fail-closed workflow gate (`DreamService::open`),
//! per §Provider Policy of `docs/superpowers/specs/2026-08-31-rust-dreaming-design.md`.
//! Behavior is ported from `src/hieronymus/dream_providers.py` and
//! `src/hieronymus/dream_workflows.py`. Every provider request in this file
//! terminates on an in-process loopback HTTP server (ADR 0012: tests never
//! egress).

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_config::{default_dream_config, save_dream_config};
use hieronymus::dream_providers::{LlmDreamProvider, probe_models, strip_code_fences};
use hieronymus::dreaming::{DeterministicDreamProvider, DreamProvider, DreamService};
use hieronymus::memory_models::{ShortTermMemoryRecord, TranslationContext};
use hieronymus::provider_config::{
    ProviderCatalog, ProviderDefaults, ProviderProfile, save_provider_catalog,
};
use hieronymus::provider_http::{
    BlockingHttpTransport, HttpError, HttpResponse, ProviderTransport,
};
use hieronymus::registry::Registry;
use hieronymus::workspace::{ShortTermMemoryInput, WorkspaceStore};

const SECRET_KEY: &str = "raw-secret-value";

// ---------------------------------------------------------------------------
// Loopback LLM server
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

impl RecordedRequest {
    fn header(&self, name: &str) -> &str {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
            .unwrap_or_default()
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// One handler per accepted connection: returns `(status, body)`.
type Handler = Box<dyn Fn(&RecordedRequest) -> (u16, String) + Send + Sync>;

/// An in-process HTTP server standing in for the configured provider. Every
/// connection is served on its own thread so delayed responses (timeout
/// tests) never block later client attempts.
struct LoopbackLlm {
    port: u16,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    accept_thread: Option<std::thread::JoinHandle<()>>,
}

impl LoopbackLlm {
    fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(handler);
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let accept_thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let handler = Arc::clone(&handler);
                        let requests = Arc::clone(&thread_requests);
                        std::thread::spawn(move || {
                            serve_connection(stream, &handler, &requests);
                        });
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

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Drop for LoopbackLlm {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            handle.join().unwrap();
        }
    }
}

fn serve_connection(
    mut stream: TcpStream,
    handler: &Handler,
    requests: &Mutex<Vec<RecordedRequest>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    requests.lock().unwrap().push(request.clone());
    let (status, body) = handler(&request);
    let response = format!(
        "HTTP/1.1 {status} probe\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn read_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
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
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = raw[separator + 4..].to_vec();
    while body.len() < content_length {
        let count = stream.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        body.extend_from_slice(&buffer[..count]);
    }
    Some(RecordedRequest {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Wrap `content` in the OpenAI chat-completions envelope.
fn openai_envelope(content: &str) -> String {
    json!({"choices": [{"message": {"content": content}}]}).to_string()
}

/// One recorded transport call: (url, headers).
type RecordedCall = (String, Vec<(String, String)>);

/// A scripted in-memory transport for retry/classification tests; each call
/// pops the next scripted outcome.
struct FakeTransport {
    outcomes: Mutex<VecDeque<Result<HttpResponse, HttpError>>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl FakeTransport {
    fn new(outcomes: Vec<Result<HttpResponse, HttpError>>) -> Arc<Self> {
        Arc::new(Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl ProviderTransport for FakeTransport {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        _payload: &Value,
        _timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        self.get_json(url, headers, _timeout)
    }

    fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        _timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), headers.to_vec()));
        self.outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected transport call to {url}"))
    }
}

/// A transport that fails the test if the gate or any non-network code path
/// ever tries to reach a provider.
struct NeverTransport;

impl ProviderTransport for NeverTransport {
    fn post_json(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _payload: &Value,
        _timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        panic!("no provider request may leave the process here")
    }

    fn get_json(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        panic!("no provider request may leave the process here")
    }
}

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn temp_config() -> (tempfile::TempDir, HieronymusConfig) {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    (root, config)
}

fn openai_profile(url: &str) -> ProviderProfile {
    ProviderProfile::new("Openai Test", "openai", url, SECRET_KEY, 5.0)
}

fn keyless_openai_profile(url: &str) -> ProviderProfile {
    ProviderProfile::new("Openai Test", "openai", url, "", 5.0)
}

fn save_catalog(config: &HieronymusConfig, profiles: Vec<(&str, ProviderProfile)>) {
    let mut catalog = ProviderCatalog::default();
    for (id, profile) in profiles {
        catalog = catalog.with_provider(id, profile);
    }
    save_provider_catalog(config, &catalog).unwrap();
}

/// Enable exactly one workflow assignment and disable everything else
/// (default configs are all-disabled, so untouched passes stay off).
fn with_enabled_workflow(config: &HieronymusConfig, name: &str, provider: &str, model: &str) {
    let mut dream_config = default_dream_config();
    let workflow = dream_config.workflows.get_mut(name).unwrap();
    workflow.provider = provider.to_string();
    workflow.model = model.to_string();
    workflow.enabled = true;
    save_dream_config(config, &dream_config).unwrap();
}

fn context(series_slug: &str) -> TranslationContext {
    TranslationContext::new(series_slug, "ja", "ru", "translate")
}

fn create_series(config: &HieronymusConfig, slug: &str) {
    Registry::open(config)
        .unwrap()
        .create_series(slug, "Only Sense Online", "ja", "ru", None)
        .unwrap();
}

fn add_memory(workspace: &WorkspaceStore, session_id: i64, kind: &str, text: &str) -> i64 {
    workspace
        .add_short_term_memory(session_id, &ShortTermMemoryInput::new(kind, text))
        .unwrap()
        .id
}

fn completed_session(
    config: &HieronymusConfig,
    slug: &str,
    texts: &[&str],
) -> Vec<ShortTermMemoryRecord> {
    let workspace = WorkspaceStore::open(config).unwrap();
    let session = workspace.start_session(&context(slug)).unwrap();
    for text in texts {
        add_memory(&workspace, session.id, "note", text);
    }
    workspace.complete_session(session.id).unwrap();
    workspace.list_short_term_memories(session.id).unwrap()
}

fn scalar(config: &HieronymusConfig, sql: &str) -> Value {
    let connection = open_migrated(&config.database_path()).unwrap();
    let mut statement = connection.prepare(sql).unwrap();
    let value = statement.query_row([], |row| row.get::<_, rusqlite::types::Value>(0));
    match value.unwrap() {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(value) => json!(value),
        rusqlite::types::Value::Real(value) => json!(value),
        rusqlite::types::Value::Text(value) => json!(value),
        rusqlite::types::Value::Blob(value) => json!(String::from_utf8_lossy(&value).into_owned()),
    }
}

/// Sentinel: the configured key must not appear in any persisted audit, run,
/// or phase text.
fn assert_key_absent_from_records(config: &HieronymusConfig) {
    for (table, column) in [
        ("dream_runs", "error"),
        ("dream_phase_runs", "error"),
        ("dream_audit_entries", "summary"),
        ("dream_audit_entries", "payload_json"),
    ] {
        let matches = scalar(
            config,
            &format!("select count(*) from {table} where {column} like '%{SECRET_KEY}%'"),
        );
        assert_eq!(
            matches,
            json!(0),
            "{table}.{column} must never contain the provider key"
        );
    }
}

fn audit_payloads(config: &HieronymusConfig, run_id: i64, event_type: &str) -> Vec<Value> {
    let connection = open_migrated(&config.database_path()).unwrap();
    let mut statement = connection
        .prepare(
            "select payload_json from dream_audit_entries
             where dream_run_id = ?1 and event_type = ?2 order by id",
        )
        .unwrap();
    let rows = statement
        .query_map(rusqlite::params![run_id, event_type], |row| {
            let payload: String = row.get(0)?;
            Ok(serde_json::from_str(&payload).unwrap())
        })
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

fn sha256_hex(input: &str) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(input.as_bytes()))
}

// ---------------------------------------------------------------------------
// RED: workflow fail-closed gate at DreamService::open
// ---------------------------------------------------------------------------

#[test]
fn gate_open_succeeds_when_no_workflow_is_enabled() {
    let (_root, config) = temp_config();
    // No provider.conf at all and the default all-disabled dream.conf: the
    // synthetic/test world must keep opening with the deterministic provider.
    DreamService::open(&config, DeterministicDreamProvider).unwrap();
}

#[test]
fn gate_fails_closed_on_missing_profile_for_enabled_workflow() {
    let (_root, config) = temp_config();
    save_catalog(
        &config,
        vec![("local-llm", openai_profile("http://127.0.0.1:9/v1"))],
    );
    with_enabled_workflow(
        &config,
        "knowledge_crystals",
        "missing-profile",
        "test-model",
    );
    let provider = LlmDreamProvider::new(
        "local-llm",
        openai_profile("http://127.0.0.1:9/v1"),
        "test-model",
    )
    .unwrap()
    .with_transport(Arc::new(NeverTransport));

    let error = DreamService::open(&config, provider).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("workflow knowledge_crystals: provider profile missing: missing-profile"),
        "{error}"
    );
}

#[test]
fn gate_fails_closed_when_deterministic_provider_faces_llm_workflow() {
    let (_root, config) = temp_config();
    save_catalog(
        &config,
        vec![("local-llm", openai_profile("http://127.0.0.1:9/v1"))],
    );
    with_enabled_workflow(&config, "knowledge_crystals", "local-llm", "test-model");

    let error = DreamService::open(&config, DeterministicDreamProvider).unwrap_err();

    assert!(
        error.to_string().contains(
            "workflow knowledge_crystals requires configured provider local-llm; \
             the deterministic provider never substitutes for a configured LLM workflow"
        ),
        "{error}"
    );
}

#[test]
fn gate_fails_closed_for_llm_provider_on_deterministic_workflow() {
    let (_root, config) = temp_config();
    save_catalog(
        &config,
        vec![("local-llm", openai_profile("http://127.0.0.1:9/v1"))],
    );
    with_enabled_workflow(
        &config,
        "knowledge_crystals",
        "deterministic",
        "unused-model",
    );
    let provider = LlmDreamProvider::new(
        "local-llm",
        openai_profile("http://127.0.0.1:9/v1"),
        "unused-model",
    )
    .unwrap()
    .with_transport(Arc::new(NeverTransport));

    let error = DreamService::open(&config, provider).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("workflow knowledge_crystals is declared deterministic"),
        "{error}"
    );
}

#[test]
fn gate_fails_closed_when_enabled_workflow_profile_has_no_key() {
    let (_root, config) = temp_config();
    save_catalog(
        &config,
        vec![
            ("keyless", keyless_openai_profile("http://127.0.0.1:9/v1")),
            ("local-llm", openai_profile("http://127.0.0.1:9/v1")),
        ],
    );
    with_enabled_workflow(&config, "knowledge_crystals", "keyless", "test-model");
    let provider = LlmDreamProvider::new(
        "local-llm",
        openai_profile("http://127.0.0.1:9/v1"),
        "test-model",
    )
    .unwrap()
    .with_transport(Arc::new(NeverTransport));

    let error = DreamService::open(&config, provider).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("workflow knowledge_crystals: API key missing for provider profile: keyless"),
        "{error}"
    );
}

#[test]
fn gate_fails_closed_when_injected_provider_does_not_match_wiring() {
    let (_root, config) = temp_config();
    save_catalog(
        &config,
        vec![("local-llm", openai_profile("http://127.0.0.1:9/v1"))],
    );
    with_enabled_workflow(&config, "knowledge_crystals", "local-llm", "model-a");
    let provider = LlmDreamProvider::new(
        "local-llm",
        openai_profile("http://127.0.0.1:9/v1"),
        "model-b",
    )
    .unwrap()
    .with_transport(Arc::new(NeverTransport));

    let error = DreamService::open(&config, provider).unwrap_err();

    let message = error.to_string();
    assert!(
        message.contains(
            "workflow knowledge_crystals is assigned to provider local-llm model model-a"
        ),
        "{message}"
    );
    assert!(
        message.contains("serves local-llm model model-b"),
        "{message}"
    );
}

#[test]
fn gate_resolves_missing_workflow_provider_from_catalog_defaults() {
    let (_root, config) = temp_config();
    let mut catalog = ProviderCatalog::default()
        .with_provider("local-llm", openai_profile("http://127.0.0.1:9/v1"));
    catalog.defaults = ProviderDefaults::new("local-llm", "default-model");
    save_provider_catalog(&config, &catalog).unwrap();
    // ADR 0007 resolution step 2: an empty workflow provider falls back to
    // provider.conf defaults.
    let mut dream_config = default_dream_config();
    let workflow = dream_config
        .workflows
        .get_mut("knowledge_crystals")
        .unwrap();
    workflow.provider = String::new();
    workflow.model = "test-model".to_string();
    workflow.enabled = true;
    save_dream_config(&config, &dream_config).unwrap();

    let provider = LlmDreamProvider::new(
        "local-llm",
        openai_profile("http://127.0.0.1:9/v1"),
        "test-model",
    )
    .unwrap()
    .with_transport(Arc::new(NeverTransport));

    DreamService::open(&config, provider).unwrap();
}

#[test]
fn gate_allows_keyless_ollama_profiles() {
    let (_root, config) = temp_config();
    save_catalog(
        &config,
        vec![(
            "local-ollama",
            ProviderProfile::new("Ollama", "ollama", "http://127.0.0.1:9", "", 5.0),
        )],
    );
    with_enabled_workflow(&config, "knowledge_crystals", "local-ollama", "gemma4-e3b");
    let provider = LlmDreamProvider::new(
        "local-ollama",
        ProviderProfile::new("Ollama", "ollama", "http://127.0.0.1:9", "", 5.0),
        "gemma4-e3b",
    )
    .unwrap()
    .with_transport(Arc::new(NeverTransport));

    DreamService::open(&config, provider).unwrap();
}

#[test]
fn llm_provider_construction_fails_closed_with_python_messages() {
    let error = LlmDreamProvider::new("local-llm", openai_profile("http://127.0.0.1:9/v1"), "   ")
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "model must not be empty for provider profile: local-llm"
    );

    let error = LlmDreamProvider::new(
        "local-llm",
        keyless_openai_profile("http://127.0.0.1:9/v1"),
        "m",
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "API key missing for provider profile: local-llm"
    );

    // Ollama profiles need no key.
    LlmDreamProvider::new(
        "local-ollama",
        ProviderProfile::new("Ollama", "ollama", "http://127.0.0.1:9", "", 5.0),
        "gemma4-e3b",
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// RED: real client pass behavior over loopback HTTP
// ---------------------------------------------------------------------------

#[test]
fn provider_pass_sends_openai_payload_and_parses_fenced_output() {
    let server = LoopbackLlm::start(Box::new(|request| {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/chat/completions");
        assert_eq!(request.header("authorization"), "Bearer raw-secret-value");
        assert!(
            request
                .header("x-hieronymus-request-id")
                .starts_with("dream-"),
            "every request carries a request id: {:?}",
            request.headers
        );
        let payload = request.json();
        assert_eq!(payload["model"], json!("test-model"));
        assert_eq!(payload["temperature"], json!(0.1));
        assert_eq!(payload["response_format"], json!({"type": "json_object"}));
        let prompt = payload["messages"][0]["content"].as_str().unwrap();
        assert!(
            prompt.contains("Dream pass: knowledge_crystals."),
            "{prompt}"
        );
        assert!(
            prompt.contains("The selected memory is important."),
            "{prompt}"
        );
        assert!(
            prompt.contains("Use only provided source memory ids."),
            "{prompt}"
        );
        (
            200,
            openai_envelope(
                "```json\n{\"crystals\": [{\"crystal_type\": \"observation\", \
                 \"title\": \"Loopback\", \"text\": \"Body text.\", \"strength\": 0.5, \
                 \"confidence\": 0.5, \"source_memory_ids\": [1]}]}\n```",
            ),
        )
    }));
    let (_root, config) = temp_config();
    create_series(&config, "book");
    let memories = completed_session(&config, "book", &["The selected memory is important."]);
    assert_eq!(memories.len(), 1);

    let provider = LlmDreamProvider::new(
        "local-llm",
        openai_profile(&server.url("/v1")),
        "test-model",
    )
    .unwrap();

    let payload = provider
        .run_pass("knowledge_crystals", &context("book"), &memories)
        .unwrap();

    assert_eq!(payload["crystals"].as_array().unwrap().len(), 1);
    assert_eq!(server.request_count(), 1);
}

#[test]
fn provider_pass_retries_retryable_failures_then_succeeds() {
    let transport = FakeTransport::new(vec![
        Err(HttpError::Network("connection reset".to_string())),
        Ok(HttpResponse {
            status: 503,
            body: "overloaded".to_string(),
        }),
        Ok(HttpResponse {
            status: 200,
            body: openai_envelope("{}"),
        }),
    ]);
    let provider = LlmDreamProvider::new("local-llm", openai_profile("http://127.0.0.1:9/v1"), "m")
        .unwrap()
        .with_transport(transport.clone())
        .with_retry_backoff(Duration::ZERO);

    let payload = provider
        .run_pass("rule_crystals", &context("book"), &[])
        .unwrap();

    assert_eq!(payload, json!({}));
    assert_eq!(
        transport.call_count(),
        3,
        "two retryable failures then success"
    );
}

#[test]
fn provider_pass_does_not_retry_non_retryable_status() {
    let transport = FakeTransport::new(vec![Ok(HttpResponse {
        status: 400,
        body: "bad request".to_string(),
    })]);
    let provider = LlmDreamProvider::new("local-llm", openai_profile("http://127.0.0.1:9/v1"), "m")
        .unwrap()
        .with_transport(transport.clone())
        .with_retry_backoff(Duration::ZERO);

    let error = provider
        .run_pass("rule_crystals", &context("book"), &[])
        .unwrap_err();

    assert!(
        error.to_string().contains("openai returned HTTP 400"),
        "{error}"
    );
    assert_eq!(transport.call_count(), 1);
}

#[test]
fn provider_pass_fails_after_bounded_retry() {
    let transport = FakeTransport::new(vec![
        Err(HttpError::Network("connection reset".to_string())),
        Err(HttpError::Network("connection reset".to_string())),
        Err(HttpError::Network("connection reset".to_string())),
    ]);
    let provider = LlmDreamProvider::new("local-llm", openai_profile("http://127.0.0.1:9/v1"), "m")
        .unwrap()
        .with_transport(transport.clone())
        .with_retry_backoff(Duration::ZERO);

    let error = provider
        .run_pass("rule_crystals", &context("book"), &[])
        .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("openai request failed"), "{message}");
    assert!(message.contains("connection reset"), "{message}");
    assert!(
        message.contains("request dream-"),
        "the failure must carry the request id: {message}"
    );
    assert_eq!(transport.call_count(), 3, "retry budget is bounded");
}

#[test]
fn provider_pass_times_out_within_the_configured_timeout() {
    let server = LoopbackLlm::start(Box::new(|_request| {
        std::thread::sleep(Duration::from_millis(1500));
        (200, openai_envelope("{}"))
    }));
    let profile = ProviderProfile::new("Openai Test", "openai", server.url("/v1"), SECRET_KEY, 0.2);
    let provider = LlmDreamProvider::new("local-llm", profile, "test-model").unwrap();

    let error = provider
        .run_pass("rule_crystals", &context("book"), &[])
        .unwrap_err();

    assert!(error.to_string().contains("timed out"), "{error}");
}

#[test]
fn provider_pass_rejects_oversized_response_before_parse() {
    let server = LoopbackLlm::start(Box::new(|_request| (200, "x".repeat(8192))));
    let profile = ProviderProfile::new("Openai Test", "openai", server.url("/v1"), SECRET_KEY, 5.0);
    let provider = LlmDreamProvider::new("local-llm", profile, "test-model")
        .unwrap()
        .with_transport(Arc::new(BlockingHttpTransport::new(1024)));

    let error = provider
        .run_pass("rule_crystals", &context("book"), &[])
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("provider response exceeded the 1024 byte limit"),
        "{error}"
    );
    assert_eq!(server.request_count(), 1, "oversized responses never retry");
}

#[test]
fn provider_pass_reports_envelope_and_json_failures_with_python_messages() {
    let cases: Vec<(String, &str)> = vec![
        (
            "not json at all".to_string(),
            "openai response did not match provider envelope",
        ),
        (
            openai_envelope("null"),
            "openai returned a non-object rule_crystals response",
        ),
        (
            openai_envelope("\"just text\""),
            "openai returned a non-object rule_crystals response",
        ),
        (
            openai_envelope("{not json}"),
            "openai returned invalid JSON for rule_crystals",
        ),
    ];
    for (content, expected) in cases {
        let transport = FakeTransport::new(vec![Ok(HttpResponse {
            status: 200,
            body: content.clone(),
        })]);
        let provider =
            LlmDreamProvider::new("local-llm", openai_profile("http://127.0.0.1:9/v1"), "m")
                .unwrap()
                .with_transport(transport.clone());
        let error = provider
            .run_pass("rule_crystals", &context("book"), &[])
            .unwrap_err();
        assert_eq!(error.to_string(), expected, "body: {content}");
    }
}

#[test]
fn provider_pass_builds_each_provider_types_payload() {
    // google → gemini wire shape.
    let transport = FakeTransport::new(vec![Ok(HttpResponse {
        status: 200,
        body: json!({"candidates": [{"content": {"parts": [{"text": "{}"}]}}]}).to_string(),
    })]);
    let provider = LlmDreamProvider::new(
        "google-api",
        ProviderProfile::new("Google", "google", "http://gemini.invalid", SECRET_KEY, 5.0),
        "gemini-model",
    )
    .unwrap()
    .with_transport(transport.clone());
    provider
        .run_pass("rule_crystals", &context("book"), &[])
        .unwrap();
    let (url, headers) = &transport.calls.lock().unwrap()[0];
    assert_eq!(
        url,
        "http://gemini.invalid/v1beta/models/gemini-model:generateContent"
    );
    assert_eq!(headers[0].0, "x-goog-api-key");
    assert_eq!(headers[0].1, SECRET_KEY);

    // anthropic wire shape.
    let transport = FakeTransport::new(vec![Ok(HttpResponse {
        status: 200,
        body: json!({"content": [{"text": "{}"}]}).to_string(),
    })]);
    let provider = LlmDreamProvider::new(
        "anthropic-api",
        ProviderProfile::new(
            "Anthropic",
            "anthropic",
            "http://anth.invalid",
            SECRET_KEY,
            5.0,
        ),
        "claude-model",
    )
    .unwrap()
    .with_transport(transport.clone());
    provider
        .run_pass("rule_crystals", &context("book"), &[])
        .unwrap();
    let (url, headers) = &transport.calls.lock().unwrap()[0];
    assert_eq!(url, "http://anth.invalid/v1/messages");
    assert!(headers.contains(&("x-api-key".to_string(), SECRET_KEY.to_string())));
    assert!(headers.contains(&("anthropic-version".to_string(), "2023-06-01".to_string())));

    // native ollama wire shape: no auth header, chat payload with json format.
    let transport = FakeTransport::new(vec![Ok(HttpResponse {
        status: 200,
        body: json!({"message": {"content": "{}"}}).to_string(),
    })]);
    let provider = LlmDreamProvider::new(
        "local-ollama",
        ProviderProfile::new("Ollama", "ollama", "http://ollama.invalid:11434", "", 5.0),
        "gemma4-e3b",
    )
    .unwrap()
    .with_transport(transport.clone());
    provider
        .run_pass("rule_crystals", &context("book"), &[])
        .unwrap();
    let (url, headers) = &transport.calls.lock().unwrap()[0];
    assert_eq!(url, "http://ollama.invalid:11434/api/chat");
    assert!(
        !headers.iter().any(|(name, _)| name == "authorization"),
        "native ollama sends no auth header"
    );

    // ollama behind an OpenAI-compatible endpoint uses the openai wire with
    // the `api_key or "ollama"` bearer fallback.
    let transport = FakeTransport::new(vec![Ok(HttpResponse {
        status: 200,
        body: openai_envelope("{}"),
    })]);
    let provider = LlmDreamProvider::new(
        "local-ollama",
        ProviderProfile::new(
            "Ollama",
            "ollama",
            "http://ollama.invalid:11434/v1",
            "",
            5.0,
        ),
        "gemma4-e3b",
    )
    .unwrap()
    .with_transport(transport.clone());
    provider
        .run_pass("rule_crystals", &context("book"), &[])
        .unwrap();
    let (url, headers) = &transport.calls.lock().unwrap()[0];
    assert_eq!(url, "http://ollama.invalid:11434/v1/chat/completions");
    assert!(headers.contains(&("Authorization".to_string(), "Bearer ollama".to_string())));
}

#[test]
fn strip_code_fences_only_unwraps_complete_fences() {
    assert_eq!(
        strip_code_fences("  ```json\n{\"a\": 1}\n```  "),
        "{\"a\": 1}"
    );
    assert_eq!(strip_code_fences("```\n{\"a\": 1}\n```"), "{\"a\": 1}");
    assert_eq!(strip_code_fences("{\"a\": 1}"), "{\"a\": 1}");
    assert_eq!(strip_code_fences("{\"a\": `x`}"), "{\"a\": `x`}");
    // An unterminated fence is data, not a wrapper: never stripped.
    assert_eq!(
        strip_code_fences("```json\n{\"a\": 1}"),
        "```json\n{\"a\": 1}"
    );
}

// ---------------------------------------------------------------------------
// RED: end-to-end dream run over the real client + sentinel redaction
// ---------------------------------------------------------------------------

/// Scripted dream pass handler: answers every pass from the recorded
/// selection, wrapped in the OpenAI envelope.
/// Parse the dream prompt payload out of the chat request body.
fn dream_prompt_of(request: &RecordedRequest) -> Value {
    let content = request.json()["messages"][0]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    serde_json::from_str(&content).unwrap_or(Value::Null)
}

fn dream_pass_handler() -> Handler {
    Box::new(|request| {
        let payload = dream_prompt_of(request);
        let instruction = payload["instruction"].as_str().unwrap_or_default();
        let memory_ids: Vec<Value> = payload["memories"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|memory| memory["id"].clone())
            .collect();
        let content = if instruction.contains("coverage_audit") {
            json!({"covered_memory_ids": memory_ids}).to_string()
        } else if instruction.contains("Dream pass: knowledge_crystals.") {
            json!({"crystals": [{
                "crystal_type": "observation",
                "title": "Loopback",
                "text": "The loopback memory is important.",
                "strength": 0.6,
                "confidence": 0.8,
                "source_memory_ids": memory_ids,
            }]})
            .to_string()
        } else {
            "{}".to_string()
        };
        (200, openai_envelope(&content))
    })
}

#[test]
fn dream_service_runs_configured_llm_workflow_over_loopback() {
    let server = LoopbackLlm::start(dream_pass_handler());
    let (_root, config) = temp_config();
    create_series(&config, "book");
    completed_session(&config, "book", &["The loopback memory is important."]);
    save_catalog(
        &config,
        vec![("local-llm", openai_profile(&server.url("/v1")))],
    );
    with_enabled_workflow(&config, "knowledge_crystals", "local-llm", "test-model");

    let provider = LlmDreamProvider::new(
        "local-llm",
        openai_profile(&server.url("/v1")),
        "test-model",
    )
    .unwrap();
    let service = DreamService::open(&config, provider).unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(run.provider, "openai");
    assert_eq!(run.created_crystal_count, 1);
    let crystal = scalar(&config, "select text from crystals");
    assert_eq!(crystal, json!("The loopback memory is important."));
    assert!(
        server.request_count() >= 7,
        "all seven passes hit the provider"
    );
    // Sentinel: the key traveled only in the outbound Authorization header.
    let sent_key = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|request| request.header("authorization") == "Bearer raw-secret-value");
    assert!(
        sent_key,
        "the configured key authenticates the outbound call"
    );
    assert_key_absent_from_records(&config);
}

#[test]
fn audit_prompt_hash_and_endpoint_match_the_request_sent_over_the_wire() {
    let server = LoopbackLlm::start(dream_pass_handler());
    let (_root, config) = temp_config();
    create_series(&config, "book");
    completed_session(&config, "book", &["The loopback memory is important."]);
    save_catalog(
        &config,
        vec![("local-llm", openai_profile(&server.url("/v1")))],
    );
    with_enabled_workflow(&config, "knowledge_crystals", "local-llm", "test-model");

    let provider = LlmDreamProvider::new(
        "local-llm",
        openai_profile(&server.url("/v1")),
        "test-model",
    )
    .unwrap();
    let service = DreamService::open(&config, provider).unwrap();
    let run = service.run_cycle("manual", false).unwrap();
    assert_eq!(run.status, "completed");

    // The prompt that actually left the process, one request per pass, in
    // pass order (each pass completes before the next starts).
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 7, "one wire request per pass");
    let wire_hashes: Vec<String> = requests
        .iter()
        .map(|request| {
            let body = request.json();
            let content = body["messages"][0]["content"].as_str().unwrap();
            sha256_hex(content)
        })
        .collect();
    drop(requests);

    let request_payloads = audit_payloads(&config, run.id, "provider_request");
    let response_payloads = audit_payloads(&config, run.id, "provider_response");
    assert_eq!(request_payloads.len(), 7);
    assert_eq!(response_payloads.len(), 7);
    for index in 0..7 {
        let expected_hash = &wire_hashes[index];
        assert_eq!(
            request_payloads[index]["prompt_sha256"],
            json!(expected_hash),
            "request audit entry {index} must hash the prompt sent on the wire"
        );
        assert_eq!(
            response_payloads[index]["prompt_sha256"],
            json!(expected_hash),
            "response audit entry {index} must hash the same prompt"
        );
        assert_eq!(
            request_payloads[index]["endpoint"],
            json!(server.url("/v1")),
            "the audited endpoint is the redacted profile endpoint"
        );
    }
    assert_key_absent_from_records(&config);
}

#[test]
fn failed_llm_run_fails_closed_and_redacts_records() {
    let server = LoopbackLlm::start(Box::new(|_request| (500, "boom".to_string())));
    let (_root, config) = temp_config();
    create_series(&config, "book");
    completed_session(&config, "book", &["Valid input."]);
    save_catalog(
        &config,
        vec![("local-llm", openai_profile(&server.url("/v1")))],
    );
    with_enabled_workflow(&config, "knowledge_crystals", "local-llm", "test-model");

    let provider = LlmDreamProvider::new(
        "local-llm",
        openai_profile(&server.url("/v1")),
        "test-model",
    )
    .unwrap()
    .with_retry_backoff(Duration::ZERO);
    let service = DreamService::open(&config, provider).unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();

    assert!(
        error.to_string().contains("openai returned HTTP 500"),
        "{error}"
    );
    let status = scalar(&config, "select status from dream_runs");
    assert_eq!(status, json!("failed"));
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
    let audited_events = scalar(
        &config,
        "select count(*) from dream_audit_entries
         where dream_run_id = (select max(id) from dream_runs)",
    );
    assert!(
        audited_events.as_i64().unwrap() > 0,
        "the failed run must keep its audit entries"
    );
    assert_key_absent_from_records(&config);
}

#[test]
fn malformed_llm_output_is_audited_with_parse_warnings() {
    let server = LoopbackLlm::start(Box::new(|request| {
        let payload = dream_prompt_of(request);
        let instruction = payload["instruction"].as_str().unwrap_or_default();
        let memory_ids: Vec<Value> = payload["memories"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|memory| memory["id"].clone())
            .collect();
        let content = if instruction.contains("coverage_audit") {
            json!({"covered_memory_ids": memory_ids}).to_string()
        } else if instruction.contains("Dream pass: knowledge_crystals.") {
            // The `body` fallback is a recoverable malformation: accepted with
            // a confidence penalty and an audited parse warning.
            json!({"crystals": [{
                "body": "Recovered from a malformed body field.",
                "kind": "rule_crystal",
                "source_memory_ids": memory_ids,
            }]})
            .to_string()
        } else {
            "{}".to_string()
        };
        (200, openai_envelope(&content))
    }));
    let (_root, config) = temp_config();
    create_series(&config, "book");
    completed_session(&config, "book", &["Valid input."]);
    save_catalog(
        &config,
        vec![("local-llm", openai_profile(&server.url("/v1")))],
    );
    with_enabled_workflow(&config, "knowledge_crystals", "local-llm", "test-model");

    let provider = LlmDreamProvider::new(
        "local-llm",
        openai_profile(&server.url("/v1")),
        "test-model",
    )
    .unwrap();
    let service = DreamService::open(&config, provider).unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    assert_eq!(run.status, "completed");
    let parse_warnings = scalar(
        &config,
        "select count(*) from dream_audit_entries
         where event_type = 'parse_warnings'",
    );
    assert!(
        parse_warnings.as_i64().unwrap() > 0,
        "recoverable malformation must be audited"
    );
    let penalty = scalar(&config, "select malformed_penalty from crystals");
    assert!(penalty.as_f64().unwrap() > 0.0);
    assert_key_absent_from_records(&config);
}

// ---------------------------------------------------------------------------
// RED: provider probe (REST check/models seam behavior)
// ---------------------------------------------------------------------------

#[test]
fn probe_models_lists_real_models_over_loopback() {
    let server = LoopbackLlm::start(Box::new(|request| {
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/v1/models");
        assert_eq!(request.header("authorization"), "Bearer raw-secret-value");
        (
            200,
            json!({"data": [{"id": "model-b"}, {"id": "model-a"}]}).to_string(),
        )
    }));
    let probe = probe_models(
        &openai_profile(&server.url("/v1")),
        Arc::new(BlockingHttpTransport::default()),
    );
    assert!(probe.ok, "{:?}", probe.error);
    assert_eq!(probe.models, vec!["model-a", "model-b"]);
    assert_eq!(probe.source, "api");
    assert_eq!(probe.error, "");
}

#[test]
fn probe_models_retries_retryable_failures() {
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler_hits = Arc::clone(&hits);
    let server = LoopbackLlm::start(Box::new(move |_request| {
        let seen = handler_hits.fetch_add(1, Ordering::SeqCst);
        if seen <= 1 {
            (503, "overloaded".to_string())
        } else {
            (200, json!({"data": [{"id": "model-a"}]}).to_string())
        }
    }));
    let probe = probe_models(
        &openai_profile(&server.url("/v1")),
        Arc::new(BlockingHttpTransport::default()),
    );
    assert!(probe.ok, "{:?}", probe.error);
    assert_eq!(probe.models, vec!["model-a"]);
    assert_eq!(server.request_count(), 3, "bounded retry budget");
}

#[test]
fn probe_models_missing_key_falls_back_to_defaults() {
    let server = LoopbackLlm::start(Box::new(|_request| (200, "{}".to_string())));
    let probe = probe_models(
        &keyless_openai_profile(&server.url("/v1")),
        Arc::new(BlockingHttpTransport::default()),
    );
    assert!(!probe.ok);
    assert_eq!(probe.models, vec!["gpt-4.1-mini", "gpt-4.1", "o4-mini"]);
    assert_eq!(probe.source, "defaults");
    assert_eq!(probe.error, "API key missing for provider profile");
    assert_eq!(
        server.request_count(),
        0,
        "keyless profiles never hit the wire"
    );
}

#[test]
fn probe_models_unavailable_endpoint_falls_back_to_defaults() {
    let server = LoopbackLlm::start(Box::new(|_request| (403, "denied".to_string())));
    let probe = probe_models(
        &openai_profile(&server.url("/v1")),
        Arc::new(BlockingHttpTransport::default()),
    );
    assert!(!probe.ok);
    assert_eq!(probe.models, vec!["gpt-4.1-mini", "gpt-4.1", "o4-mini"]);
    assert_eq!(probe.source, "defaults");
    assert_eq!(probe.error, "model suggestions unavailable");
}
