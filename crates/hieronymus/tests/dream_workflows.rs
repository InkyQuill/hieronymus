//! Task D1: enabled dream workflow assignments resolve to their own
//! configured provider and model (ADR 0007); disabled assignments never
//! require a provider and never receive provider requests. The provider-side
//! behavior is verified over in-process loopback HTTP (ADR 0012: tests never
//! egress).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_config::{default_dream_config, save_dream_config};
use hieronymus::dream_workflows::{WorkflowChoice, WorkflowResolver, enabled_choices};
use hieronymus::dreaming::DreamService;
use hieronymus::provider_config::{
    ProviderCatalog, ProviderProfile, load_provider_catalog, save_provider_catalog,
};
use hieronymus::registry::Registry;
use hieronymus::workspace::{ShortTermMemoryInput, WorkspaceStore};

// ---------------------------------------------------------------------------
// enabled_choices: the pure resolution gate
// ---------------------------------------------------------------------------

#[test]
fn disabled_workflow_never_requires_a_provider() {
    let choices = vec![
        WorkflowChoice {
            name: "consolidation".into(),
            enabled: false,
            provider: "".into(),
            model: "".into(),
        },
        WorkflowChoice {
            name: "coverage_audit".into(),
            enabled: true,
            provider: "local".into(),
            model: "audit".into(),
        },
    ];
    let selected = enabled_choices(choices).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].model, "audit");
}

#[test]
fn enabled_choices_rejects_a_run_without_the_required_coverage_audit() {
    let choices = vec![
        WorkflowChoice {
            name: "knowledge_crystals".into(),
            enabled: true,
            provider: "local".into(),
            model: "crystals".into(),
        },
        WorkflowChoice {
            name: "coverage_audit".into(),
            enabled: false,
            provider: "".into(),
            model: "".into(),
        },
    ];
    let error = enabled_choices(choices).unwrap_err();
    assert_eq!(
        error,
        "coverage_audit must be enabled before processing memories"
    );
}

#[test]
fn enabled_choices_rejects_enabled_workflow_without_provider_or_model() {
    let choices = vec![WorkflowChoice {
        name: "coverage_audit".into(),
        enabled: true,
        provider: "  ".into(),
        model: "".into(),
    }];
    let error = enabled_choices(choices).unwrap_err();
    assert_eq!(error, "enabled workflows require a provider and model");
}

// ---------------------------------------------------------------------------
// Loopback LLM server
// ---------------------------------------------------------------------------

/// One recorded chat request: the model it asked for and its auth header.
struct RecordedRequest {
    model: String,
    authorization: String,
}

/// An in-process HTTP server standing in for the configured providers (ADR
/// 0012). Every chat request is recorded and answered with `status`; a 200
/// answer carries an OpenAI envelope whose content covers the dreaming pass:
/// coverage_audit accounts for the selection, knowledge_crystals
/// crystallizes one observation, everything else returns empty.
struct LoopbackLlm {
    url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    accept_thread: Option<std::thread::JoinHandle<()>>,
}

impl LoopbackLlm {
    fn start(status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let accept_thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let requests = Arc::clone(&thread_requests);
                        std::thread::spawn(move || {
                            serve_connection(stream, status, &requests);
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
            url,
            requests,
            stop,
            accept_thread: Some(accept_thread),
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.url)
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

fn serve_connection(mut stream: TcpStream, status: u16, requests: &Mutex<Vec<RecordedRequest>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let body = if status == 200 {
        let payload: Value = serde_json::from_str(&request.body).unwrap_or(Value::Null);
        let prompt = payload["messages"][0]["content"]
            .as_str()
            .unwrap_or_default();
        let instruction: Value = serde_json::from_str(prompt).unwrap_or(Value::Null);
        let memory_ids: Vec<Value> = instruction["memories"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|memory| memory["id"].clone())
            .collect();
        let content = if instruction["instruction"]
            .as_str()
            .unwrap_or_default()
            .contains("coverage_audit")
        {
            json!({"covered_memory_ids": memory_ids}).to_string()
        } else if instruction["instruction"]
            .as_str()
            .unwrap_or_default()
            .contains("Dream pass: knowledge_crystals.")
        {
            json!({"crystals": [{
                "crystal_type": "observation",
                "title": "Two lanes",
                "text": "The two-lane memory is important.",
                "strength": 0.6,
                "confidence": 0.8,
                "source_memory_ids": memory_ids,
            }]})
            .to_string()
        } else {
            "{}".to_string()
        };
        json!({"choices": [{"message": {"content": content}}]}).to_string()
    } else {
        "bad request".to_string()
    };
    let authorization = request
        .headers
        .get("authorization")
        .cloned()
        .unwrap_or_default();
    let model = serde_json::from_str::<Value>(&request.body).unwrap_or(Value::Null)["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    requests.lock().unwrap().push(RecordedRequest {
        model,
        authorization,
    });
    let response = format!(
        "HTTP/1.1 {status} two-lane\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

struct RawRequest {
    body: String,
    headers: std::collections::BTreeMap<String, String>,
}

fn read_request(stream: &mut TcpStream) -> Option<RawRequest> {
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
    lines.next()?;
    let mut headers = std::collections::BTreeMap::new();
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
    Some(RawRequest {
        body: String::from_utf8_lossy(&body).into_owned(),
        headers,
    })
}

// ---------------------------------------------------------------------------
// Two configured provider lanes over loopback HTTP
// ---------------------------------------------------------------------------

const KEY_A: &str = "lane-a-key";
const KEY_B: &str = "lane-b-key";

fn temp_config() -> (tempfile::TempDir, HieronymusConfig) {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    (root, config)
}

fn context(series_slug: &str) -> hieronymus::memory_models::TranslationContext {
    hieronymus::memory_models::TranslationContext::new(series_slug, "ja", "ru", "translate")
}

fn create_series(config: &HieronymusConfig, slug: &str) {
    Registry::open(config)
        .unwrap()
        .create_series(slug, "Only Sense Online", "ja", "ru", None)
        .unwrap();
}

fn completed_session(config: &HieronymusConfig, slug: &str, texts: &[&str]) {
    let workspace = WorkspaceStore::open(config).unwrap();
    let session = workspace.start_session(&context(slug)).unwrap();
    for text in texts {
        workspace
            .add_short_term_memory(session.id, &ShortTermMemoryInput::new("note", *text))
            .unwrap();
    }
    workspace.complete_session(session.id).unwrap();
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

/// knowledge_crystals on lane-a/model-knowledge, coverage_audit on
/// lane-b/model-audit; relations explicitly disabled while still carrying
/// wiring it must never use.
fn save_two_lane_wiring(config: &HieronymusConfig, endpoint: &str) {
    let catalog = ProviderCatalog::default()
        .with_provider(
            "lane-a",
            ProviderProfile::new("Lane A", "openai", endpoint, KEY_A, 5.0),
        )
        .with_provider(
            "lane-b",
            ProviderProfile::new("Lane B", "openai", endpoint, KEY_B, 5.0),
        );
    save_provider_catalog(config, &catalog).unwrap();

    let mut dream_config = default_dream_config();
    let knowledge = dream_config
        .workflows
        .get_mut("knowledge_crystals")
        .unwrap();
    knowledge.provider = "lane-a".to_string();
    knowledge.model = "model-knowledge".to_string();
    knowledge.enabled = true;
    let coverage = dream_config.workflows.get_mut("coverage_audit").unwrap();
    coverage.provider = "lane-b".to_string();
    coverage.model = "model-audit".to_string();
    coverage.enabled = true;
    let relations = dream_config.workflows.get_mut("relations").unwrap();
    relations.provider = "missing-lane".to_string();
    relations.model = "unused-model".to_string();
    relations.enabled = false;
    save_dream_config(config, &dream_config).unwrap();
}

#[test]
fn enabled_workflows_run_on_their_own_provider_and_model() {
    let server = LoopbackLlm::start(200);
    let (_root, config) = temp_config();
    create_series(&config, "book");
    completed_session(&config, "book", &["The two-lane memory is important."]);
    save_two_lane_wiring(&config, &server.endpoint("/v1"));

    let resolver = WorkflowResolver::from_catalog(load_provider_catalog(&config).unwrap());
    let service = DreamService::open(&config, resolver).unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(run.provider, "openai");
    assert_eq!(run.created_crystal_count, 1);

    // Only the two enabled passes reached a provider: the disabled
    // assignments made zero requests.
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].model, "model-knowledge");
    assert_eq!(requests[0].authorization, format!("Bearer {KEY_A}"));
    assert_eq!(requests[1].model, "model-audit");
    assert_eq!(requests[1].authorization, format!("Bearer {KEY_B}"));
    drop(requests);

    // Phase records carry each pass's actual resolved profile id and model,
    // not one provider's identity for every phase.
    let connection = open_migrated(&config.database_path()).unwrap();
    let mut statement = connection
        .prepare(
            "select phase, provider_profile, model from dream_phase_runs
             order by id",
        )
        .unwrap();
    let rows: Vec<(String, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "knowledge_crystals".to_string(),
                "lane-a".to_string(),
                "model-knowledge".to_string()
            ),
            (
                "coverage_audit".to_string(),
                "lane-b".to_string(),
                "model-audit".to_string()
            ),
            (
                "persistence".to_string(),
                "lane-a".to_string(),
                "model-knowledge".to_string()
            ),
        ]
    );
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(1));
}

#[test]
fn provider_failure_preserves_pending_memory() {
    let server = LoopbackLlm::start(400);
    let (_root, config) = temp_config();
    create_series(&config, "book");
    completed_session(&config, "book", &["Valid input."]);
    save_two_lane_wiring(&config, &server.endpoint("/v1"));

    let resolver = WorkflowResolver::from_catalog(load_provider_catalog(&config).unwrap());
    let service = DreamService::open(&config, resolver).unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();

    assert!(
        error.to_string().contains("openai returned HTTP 400"),
        "{error}"
    );
    assert_eq!(
        scalar(&config, "select status from dream_runs"),
        json!("failed")
    );
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
    assert_eq!(
        scalar(
            &config,
            "select count(*) from short_term_memories where archived_at is null"
        ),
        json!(1),
        "a failed provider pass must leave the memory pending"
    );
    assert_eq!(
        server.request_count(),
        1,
        "a non-retryable failure stops the run before the next pass"
    );
}

#[test]
fn a_run_without_enabled_workflows_is_rejected_before_any_provider_call() {
    let server = LoopbackLlm::start(200);
    let (_root, config) = temp_config();
    create_series(&config, "book");
    completed_session(&config, "book", &["Valid input."]);
    // The default dream.conf disables every workflow, including the required
    // coverage audit; the gate only needs the catalog to say so.
    save_provider_catalog(&config, &ProviderCatalog::default()).unwrap();

    let resolver = WorkflowResolver::from_catalog(load_provider_catalog(&config).unwrap());
    let service = DreamService::open(&config, resolver).unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("coverage_audit must be enabled before processing memories"),
        "{error}"
    );
    assert_eq!(
        scalar(&config, "select status from dream_runs"),
        json!("failed")
    );
    assert_eq!(
        server.request_count(),
        0,
        "no provider is required, so no provider is contacted"
    );
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
}
