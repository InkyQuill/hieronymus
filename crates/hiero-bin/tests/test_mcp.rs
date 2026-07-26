use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use hiero_bin::{
    assets::AssetSource,
    daemon::{AppState, WorkerSupervisor, build_router},
    mcp::{DreamRunner, McpBackend, StoreDreamRunner, StoreMcpBackend, http, tool_catalog},
};
use hiero_core::{
    config::HieronymusConfig,
    db,
    domain::{AddMemoryInput, TranslationContext, WorkspaceStore},
    dreaming::{DreamConfig, DreamPhaseError, DreamProviderResolver, WorkflowProfile},
    provider::{DeterministicProvider, DreamProvider, ProviderCatalog},
    registry::SeriesRegistry,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tower::ServiceExt;

struct FailingBackend;
struct FakeDreamRunner;

#[async_trait]
impl McpBackend for FailingBackend {
    async fn call(&self, _name: &str, _arguments: Value) -> anyhow::Result<Value> {
        anyhow::bail!("injected backend failure")
    }
}

#[async_trait]
impl DreamRunner for FakeDreamRunner {
    async fn run(&self, provider: Option<&str>, wait: bool) -> anyhow::Result<Value> {
        Ok(json!({"provider": provider, "wait": wait, "started": true}))
    }
}

const AUTH_TOKEN: &str = "test-auth-token";
const LOCAL_HOST: &str = "127.0.0.1:9768";

const EXPECTED_TOOLS: [&str; 40] = [
    "hieronymus_concept_archive",
    "hieronymus_concept_create",
    "hieronymus_concept_facet_add",
    "hieronymus_concept_facet_list",
    "hieronymus_concept_facet_set_canonical",
    "hieronymus_concept_facet_update",
    "hieronymus_concept_get",
    "hieronymus_concept_list",
    "hieronymus_concept_merge",
    "hieronymus_concept_proposals_list",
    "hieronymus_concept_rename",
    "hieronymus_concept_semantic_tags_set",
    "hieronymus_concept_update",
    "hieronymus_crystal_link_concept",
    "hieronymus_crystal_semantic_tags_set",
    "hieronymus_crystal_story_scopes_set",
    "hieronymus_dream",
    "hieronymus_feedback",
    "hieronymus_memory_add",
    "hieronymus_memory_search",
    "hieronymus_rag_import",
    "hieronymus_rag_search",
    "hieronymus_recall",
    "hieronymus_recall_feedback",
    "hieronymus_rule_crystal_archive",
    "hieronymus_rule_crystal_validate",
    "hieronymus_rule_crystals_list",
    "hieronymus_series_create",
    "hieronymus_series_init",
    "hieronymus_series_list",
    "hieronymus_series_set_language_tags",
    "hieronymus_session_complete",
    "hieronymus_session_start",
    "hieronymus_short_term_add",
    "hieronymus_short_term_add_batch",
    "hieronymus_status",
    "hieronymus_termbase_approve",
    "hieronymus_termbase_contract",
    "hieronymus_termbase_propose",
    "hieronymus_termbase_validate",
];

async fn test_state() -> (AppState, TempDir) {
    let pool = db::connect_url("sqlite::memory:?cache=shared")
        .await
        .expect("test database should connect");
    let root = TempDir::new().expect("temporary directory should be created");
    let config = HieronymusConfig::with_roots(root.path().join("data"), root.path().join("config"));
    let (shutdown, _) = broadcast::channel(4);
    let (events, _) = broadcast::channel(4);
    (
        AppState {
            pool,
            config: Arc::new(config),
            shutdown,
            auth_token: Arc::from(AUTH_TOKEN),
            port: 9768,
            dream_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            provider_transport: None,
            workers: WorkerSupervisor::default(),
            events,
            assets: AssetSource::embedded(),
        },
        root,
    )
}

fn mcp_request(body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::HOST, LOCAL_HOST)
        .header("x-hieronymus-token", AUTH_TOKEN)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .body(Body::from(
            serde_json::to_vec(&body).expect("MCP request should serialize"),
        ))
        .expect("MCP request should build")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("MCP response body should be readable");
    let text = String::from_utf8_lossy(&body);
    let payload = text
        .lines()
        .find_map(|line| line.strip_prefix("data: {"))
        .map_or_else(
            || text.as_bytes().to_vec(),
            |rest| format!("{{{rest}").into_bytes(),
        );
    serde_json::from_slice(&payload)
        .unwrap_or_else(|error| panic!("MCP response should be JSON: {error}; body={}", text))
}

fn isolated_config() -> Arc<HieronymusConfig> {
    let root = TempDir::new()
        .expect("temporary directory should be created")
        .keep();
    Arc::new(HieronymusConfig::with_roots(
        root.join("data"),
        root.join("config"),
    ))
}

#[test]
fn catalog_registers_the_exact_forty_unique_tool_names() {
    let catalog = tool_catalog();
    let actual = catalog
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<BTreeSet<_>>();
    let expected = EXPECTED_TOOLS.into_iter().collect::<BTreeSet<_>>();

    assert_eq!(catalog.len(), 40);
    assert_eq!(actual, expected);
}

#[test]
fn every_registered_tool_has_a_stable_object_schema() {
    let schemas = tool_catalog()
        .into_iter()
        .map(|tool| (tool.name, tool.input_schema))
        .collect::<std::collections::BTreeMap<_, _>>();

    insta::assert_json_snapshot!("all_mcp_tool_schemas", schemas);
}

#[tokio::test]
async fn store_backend_dispatches_registry_concept_rule_workspace_termbase_rag_and_feedback() {
    let (state, root) = test_state().await;
    let backend = StoreMcpBackend::new(state.pool.clone(), state.config.clone())
        .with_dream_runner(Arc::new(FakeDreamRunner));

    backend
        .call(
            "hieronymus_series_create",
            json!({
                "slug": "oso",
                "title": "Spice and Wolf",
                "source_language": "ja",
                "target_language": "ru"
            }),
        )
        .await
        .expect("registry dispatch should succeed");
    let concept = backend
        .call(
            "hieronymus_concept_create",
            json!({"canonical_name": "Holo", "series_slug": "oso"}),
        )
        .await
        .expect("concept dispatch should succeed");
    assert_eq!(concept["canonical_name"], "Holo");

    let rules = backend
        .call(
            "hieronymus_rule_crystals_list",
            json!({"series_slug": "oso"}),
        )
        .await
        .expect("rule dispatch should succeed");
    assert_eq!(rules, json!([]));

    let session = backend
        .call(
            "hieronymus_session_start",
            json!({"series_slug": "oso", "task_type": "translation"}),
        )
        .await
        .expect("workspace dispatch should succeed");
    let session_id = session["session_id"]
        .as_i64()
        .expect("session id should be numeric");
    backend
        .call(
            "hieronymus_feedback",
            json!({"session_id": session_id, "correction_text": "Use Волчица."}),
        )
        .await
        .expect("feedback dispatch should succeed");

    let crystal_id: i64 = sqlx::query(
        "INSERT INTO crystals(crystal_type,text,title,scope_type,scope_key,series_slug,source_language,target_language,tags_json,strength,confidence,source_credibility,rule_intent,status,created_at,updated_at) VALUES('lesson','Always render ホロ as Холо','Holo rule','series','series:oso','oso','ja','ru','[]',0.8,0.9,'user_rule','termbase','active',?,?)",
    )
    .bind(chrono::Utc::now())
    .bind(chrono::Utc::now())
    .execute(&state.pool)
    .await
    .expect("rule fixture should insert")
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO crystals(crystal_type,text,title,scope_type,scope_key,series_slug,source_language,target_language,tags_json,strength,confidence,source_credibility,rule_intent,status,created_at,updated_at) VALUES('lesson','ordinary memory','Note','series','series:oso','oso','ja','ru','[]',0.8,0.9,'observation','','active',?,?)",
    )
    .bind(chrono::Utc::now())
    .bind(chrono::Utc::now())
    .execute(&state.pool)
    .await
    .expect("non-rule fixture should insert");
    let rules = backend
        .call(
            "hieronymus_rule_crystals_list",
            json!({"series_slug": "oso"}),
        )
        .await
        .expect("rule-intent dispatch should succeed");
    assert_eq!(
        rules
            .as_array()
            .expect("rule list should be an array")
            .iter()
            .map(|rule| rule["record"]["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [crystal_id]
    );
    backend
        .call(
            "hieronymus_recall",
            json!({
                "session_id": session_id,
                "series_slug": "oso",
                "query": "Холо",
                "limit": 10
            }),
        )
        .await
        .expect("recall should create activation rows");
    backend
        .call(
            "hieronymus_recall_feedback",
            json!({"session_id": session_id, "useful": [crystal_id], "miss": []}),
        )
        .await
        .expect("recall feedback should record useful outcome");
    let outcome: String = sqlx::query_scalar(
        "SELECT outcome FROM crystal_activations WHERE session_id = ? AND crystal_id = ?",
    )
    .bind(session_id)
    .bind(crystal_id)
    .fetch_one(&state.pool)
    .await
    .expect("activation outcome should persist");
    assert_eq!(outcome, "useful");

    let term = backend
        .call(
            "hieronymus_termbase_propose",
            json!({
                "series_slug": "oso",
                "category": "name",
                "source_text": "ホロ",
                "canonical_translation": "Холо",
                "notes": "approved reader-facing spelling"
            }),
        )
        .await
        .expect("termbase dispatch should succeed");
    assert!(term["term_id"].is_i64());
    let notes: String = sqlx::query_scalar("SELECT soft_origin FROM crystals WHERE id = ?")
        .bind(term["term_id"].as_i64().unwrap())
        .fetch_one(&state.pool)
        .await
        .expect("term notes should be persisted");
    assert_eq!(notes, "approved reader-facing spelling");

    let dream = backend
        .call(
            "hieronymus_dream",
            json!({"provider": "fake-provider", "wait": true}),
        )
        .await
        .expect("dream dispatch should use the injected runner");
    assert_eq!(
        dream,
        json!({"provider": "fake-provider", "wait": true, "started": true})
    );

    let source = root.path().join("rag.md");
    std::fs::write(&source, "# Holo\nA wise wolf.").expect("RAG fixture should be written");
    backend
        .call(
            "hieronymus_rag_import",
            json!({"series_slug": "oso", "path": source}),
        )
        .await
        .expect("RAG dispatch should succeed");
}

#[tokio::test]
async fn production_dream_runner_executes_dream_service_with_an_injected_provider() {
    let (state, _) = test_state().await;
    state.config.ensure_directories().unwrap();
    SeriesRegistry::new(&state.pool)
        .create("oso", "Spice and Wolf", "ja", "ru")
        .await
        .unwrap();
    let workspace = WorkspaceStore::new(&state.pool);
    let session = workspace
        .start_session(
            &TranslationContext::new("oso", "ja", "ru"),
            "translation",
            "",
            "",
        )
        .await
        .unwrap();
    workspace
        .add_short_term(
            session.id,
            AddMemoryInput {
                text: "Use Холо for ホロ.".into(),
                ..AddMemoryInput::default()
            },
        )
        .await
        .unwrap();
    workspace.complete_session(session.id).await.unwrap();
    let resolved = Arc::new(AtomicBool::new(false));
    let resolver_observation = resolved.clone();
    let resolver: Arc<dyn DreamProviderResolver> = Arc::new(move |_: &WorkflowProfile| {
        resolver_observation.store(true, Ordering::Release);
        Ok::<Arc<dyn DreamProvider>, DreamPhaseError>(Arc::new(DeterministicProvider))
    });
    let mut dream_config = DreamConfig {
        enabled: true,
        min_pending_short_term_memories: 1,
        ..DreamConfig::default()
    };
    for profile in dream_config.workflows.values_mut() {
        profile.enabled = true;
        profile.provider = "deterministic".into();
        profile.model = "deterministic".into();
    }
    let runner = StoreDreamRunner::new(state.pool, state.config).with_components(
        dream_config,
        ProviderCatalog::default(),
        resolver,
    );

    let result = runner
        .run(None, false)
        .await
        .expect("the production runner should execute the real DreamService");
    assert!(result["cycle_id"].is_i64());
    assert!(result["status"].is_string());
    assert!(resolved.load(Ordering::Acquire), "dream result: {result}");
}

#[tokio::test]
async fn production_dream_runner_reports_invalid_configuration() {
    let (state, root) = test_state().await;
    std::fs::create_dir_all(root.path().join("config")).unwrap();
    std::fs::write(state.config.dream_config_path(), "invalid = [").unwrap();
    let error = StoreDreamRunner::new(state.pool, state.config)
        .run(None, false)
        .await
        .expect_err("invalid configuration must remain a tool error");
    assert!(error.to_string().contains("dream"));
}

#[tokio::test]
async fn http_surfaces_backend_errors_as_tool_errors() {
    let router = axum::Router::new().nest_service("/mcp", http::service(Arc::new(FailingBackend)));
    let initialize = router
        .clone()
        .oneshot(mcp_request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "hiero-test", "version": "1"}
            }
        })))
        .await
        .expect("initialize should complete");
    let session_id = initialize.headers()["mcp-session-id"].clone();

    let mut call = mcp_request(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "hieronymus_status", "arguments": {}}
    }));
    call.headers_mut().insert("mcp-session-id", session_id);
    let result = response_json(router.oneshot(call).await.unwrap()).await;

    assert_eq!(result["result"]["isError"], true);
    assert!(
        result["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("injected backend failure")
    );
}

#[tokio::test]
async fn http_initialize_creates_session_then_lists_and_calls_tools() {
    let (state, _root) = test_state().await;
    let router = build_router(state);
    let initialize = router
        .clone()
        .oneshot(mcp_request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "hiero-test", "version": "1"}
            }
        })))
        .await
        .expect("initialize request should complete");
    assert_eq!(initialize.status(), StatusCode::OK);
    let session_id = initialize
        .headers()
        .get("mcp-session-id")
        .expect("stateful initialize should create an MCP session")
        .clone();
    let initialized = response_json(initialize).await;
    assert_eq!(initialized["result"]["serverInfo"]["name"], "hieronymus");

    let mut list = mcp_request(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    }));
    list.headers_mut()
        .insert("mcp-session-id", session_id.clone());
    let listed = router
        .clone()
        .oneshot(list)
        .await
        .expect("tools/list should complete");
    assert_eq!(listed.status(), StatusCode::OK);
    assert_eq!(
        response_json(listed).await["result"]["tools"]
            .as_array()
            .expect("tools/list should return an array")
            .len(),
        40
    );

    let mut call = mcp_request(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "hieronymus_status", "arguments": {}}
    }));
    call.headers_mut().insert("mcp-session-id", session_id);
    let called = router
        .oneshot(call)
        .await
        .expect("tools/call should complete");
    assert_eq!(called.status(), StatusCode::OK);
    assert!(response_json(called).await["result"]["isError"] != json!(true));
}

#[tokio::test]
async fn http_rejects_unknown_tool_invalid_payload_and_foreign_origin() {
    let (state, _root) = test_state().await;
    let router = build_router(state);

    let mut foreign = mcp_request(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "hiero-test", "version": "1"}
        }
    }));
    foreign.headers_mut().insert(
        header::ORIGIN,
        "https://attacker.invalid".parse().expect("valid header"),
    );
    assert_eq!(
        router.clone().oneshot(foreign).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    let unknown = StoreMcpBackend::new(
        db::connect_url("sqlite::memory:")
            .await
            .expect("test database should connect"),
        isolated_config(),
    )
    .call("hieronymus_not_a_tool", json!({}))
    .await
    .expect_err("unknown tool should fail");
    assert!(unknown.to_string().contains("unknown MCP tool"));

    let invalid = StoreMcpBackend::new(
        db::connect_url("sqlite::memory:")
            .await
            .expect("test database should connect"),
        isolated_config(),
    )
    .call("hieronymus_series_create", json!({"slug": 42}))
    .await
    .expect_err("invalid payload should fail");
    assert!(invalid.to_string().contains("invalid payload"));
}
