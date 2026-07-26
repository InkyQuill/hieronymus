use std::collections::BTreeSet;
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

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
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{broadcast, oneshot},
};
use tower::ServiceExt;

struct FailingBackend(&'static str);
struct FakeDreamRunner;

#[async_trait]
impl McpBackend for FailingBackend {
    async fn call(&self, _name: &str, _arguments: Value) -> anyhow::Result<Value> {
        anyhow::bail!("{}", self.0)
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
fn python_description_compatibility_strings_are_exact() {
    let descriptions = tool_catalog()
        .into_iter()
        .map(|tool| (tool.name, tool.description))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        descriptions["hieronymus_concept_semantic_tags_set"],
        "Replace semantic tags for a concept."
    );
    assert_eq!(
        descriptions["hieronymus_crystal_story_scopes_set"],
        "Replace story scopes for a crystal."
    );
    assert_eq!(
        descriptions["hieronymus_crystal_semantic_tags_set"],
        "Replace semantic tags for a crystal."
    );
}

#[test]
fn every_registered_tool_has_a_stable_object_schema() {
    let schemas = tool_catalog()
        .into_iter()
        .map(|tool| {
            (
                tool.name,
                json!({
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                }),
            )
        })
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
async fn concept_list_without_series_filter_returns_mixed_scopes_flat_and_stable() {
    let (state, _) = test_state().await;
    let backend = StoreMcpBackend::new(state.pool, state.config);
    backend
        .call(
            "hieronymus_series_create",
            json!({"slug": "oso", "title": "OSO", "source_language": "ja", "target_language": "ru"}),
        )
        .await
        .unwrap();
    for arguments in [
        json!({"canonical_name": "Global", "semantic_tags": ["shared"]}),
        json!({"canonical_name": "Series", "series_slug": "oso", "semantic_tags": ["scoped"]}),
    ] {
        backend
            .call("hieronymus_concept_create", arguments)
            .await
            .unwrap();
    }

    let rows = backend
        .call("hieronymus_concept_list", json!({}))
        .await
        .unwrap();
    assert_eq!(
        rows.as_array()
            .unwrap()
            .iter()
            .map(|row| row["canonical_name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Global", "Series"]
    );
    assert!(rows[0].get("record").is_none());
    assert_eq!(rows[1]["semantic_tags"], json!(["scoped"]));
}

#[tokio::test]
async fn concept_facet_and_crystal_results_match_the_flat_python_contract() {
    let (state, _) = test_state().await;
    let backend = StoreMcpBackend::new(state.pool.clone(), state.config);
    backend
        .call(
            "hieronymus_series_create",
            json!({"slug": "oso", "title": "OSO", "source_language": "ja", "target_language": "ru"}),
        )
        .await
        .unwrap();
    let concept = backend
        .call(
            "hieronymus_concept_create",
            json!({
                "canonical_name": "Holo",
                "description": "Wise wolf",
                "status": "established",
                "confidence": 0.9,
                "semantic_tags": ["character"],
                "series_slug": "oso"
            }),
        )
        .await
        .unwrap();
    let concept_id = concept["id"].as_i64().unwrap();
    assert_eq!(
        concept,
        json!({
            "id": concept_id,
            "canonical_name": "Holo",
            "description": "Wise wolf",
            "status": "established",
            "confidence": 0.9,
            "scope_type": "series",
            "scope_key": "series:oso",
            "semantic_tags": ["character"],
            "merged_into_concept_id": null
        })
    );
    assert_eq!(
        backend
            .call("hieronymus_concept_get", json!({"concept_id": concept_id}),)
            .await
            .unwrap(),
        concept
    );
    let updated = backend
        .call(
            "hieronymus_concept_update",
            json!({"concept_id": concept_id, "description": "Wolf deity"}),
        )
        .await
        .unwrap();
    assert_eq!(updated["semantic_tags"], json!(["character"]));
    assert!(updated.get("record").is_none());

    let facet = backend
        .call(
            "hieronymus_concept_facet_add",
            json!({
                "concept_id": concept_id,
                "value": "Холо",
                "language": "ru",
                "facet_type": "rendering",
                "confidence": 0.8,
                "is_canonical": true,
                "story_scopes": ["volume:1"],
                "semantic_tags": ["name"]
            }),
        )
        .await
        .unwrap();
    let facet_id = facet["id"].as_i64().unwrap();
    assert_eq!(
        facet,
        json!({
            "id": facet_id,
            "concept_id": concept_id,
            "language": "ru",
            "facet_type": "rendering",
            "kind": "rendering",
            "value": "Холо",
            "confidence": 0.8,
            "source_crystal_id": null,
            "language_tags": ["ru"],
            "story_scopes": ["volume:1"],
            "semantic_tags": ["name"],
            "is_canonical": true
        })
    );
    let facet = backend
        .call(
            "hieronymus_concept_facet_update",
            json!({"facet_id": facet_id, "value": "Хоро", "semantic_tags": ["alias"]}),
        )
        .await
        .unwrap();
    assert_eq!(facet["value"], "Хоро");
    assert_eq!(facet["semantic_tags"], json!(["alias"]));
    assert!(facet.get("record").is_none());

    let alias = backend
        .call(
            "hieronymus_concept_facet_add",
            json!({
                "concept_id": concept_id,
                "value": "Horo",
                "language": "ja-Latn",
                "facet_type": "alias"
            }),
        )
        .await
        .unwrap();
    let alias_id = alias["id"].as_i64().unwrap();
    assert_eq!(alias["facet_type"], "alias");
    assert_eq!(alias["kind"], "name");
    let alias = backend
        .call(
            "hieronymus_concept_facet_update",
            json!({"facet_id": alias_id, "value": "Hōro"}),
        )
        .await
        .unwrap();
    assert_eq!(alias["kind"], "name");
    let listed = backend
        .call(
            "hieronymus_concept_facet_list",
            json!({"concept_id": concept_id}),
        )
        .await
        .unwrap();
    assert_eq!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .find(|facet| facet["id"] == alias_id)
            .unwrap()["kind"],
        "name"
    );
    backend
        .call(
            "hieronymus_concept_rename",
            json!({"concept_id": concept_id, "new_label": "Horo"}),
        )
        .await
        .unwrap();
    let listed = backend
        .call(
            "hieronymus_concept_facet_list",
            json!({"concept_id": concept_id}),
        )
        .await
        .unwrap();
    let former = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|facet| facet["facet_type"] == "former_label")
        .unwrap();
    assert_eq!(former["kind"], "name");

    let now = chrono::Utc::now();
    let crystal_id = sqlx::query("INSERT INTO crystals(crystal_type,text,title,scope_type,scope_key,series_slug,source_language,target_language,tags_json,strength,confidence,source_credibility,rule_intent,status,created_at,updated_at) VALUES('lesson','Holo memory','Holo','series','series:oso','oso','ja','ru','[]',0.8,0.9,'observation','','active',?,?)")
        .bind(now).bind(now).execute(&state.pool).await.unwrap().last_insert_rowid();
    let crystal = backend
        .call(
            "hieronymus_crystal_story_scopes_set",
            json!({"crystal_id": crystal_id, "story_scopes": ["volume:1"], "confidence": 0.7}),
        )
        .await
        .unwrap();
    assert_eq!(
        crystal,
        json!({
            "id": crystal_id,
            "crystal_type": "lesson",
            "text": "Holo memory",
            "title": "Holo",
            "confidence": 0.9,
            "strength": 0.8,
            "status": "active",
            "source_credibility": "observation",
            "rule_intent": "",
            "story_scopes": ["volume:1"],
            "semantic_tags": [],
            "concept_ids": []
        })
    );
    let crystal = backend
        .call(
            "hieronymus_crystal_semantic_tags_set",
            json!({"crystal_id": crystal_id, "semantic_tags": ["character"], "confidence": 0.8}),
        )
        .await
        .unwrap();
    assert_eq!(crystal["semantic_tags"], json!(["character"]));
    assert!(crystal.get("record").is_none());
    let crystal = backend
        .call(
            "hieronymus_crystal_link_concept",
            json!({"crystal_id": crystal_id, "concept_id": concept_id}),
        )
        .await
        .unwrap();
    assert_eq!(crystal["concept_ids"], json!([concept_id]));
}

#[tokio::test]
async fn term_approval_uses_candidate_languages_and_default_memory_session_is_reused() {
    let (state, _) = test_state().await;
    let backend = StoreMcpBackend::new(state.pool.clone(), state.config);
    backend
        .call(
            "hieronymus_series_create",
            json!({"slug": "oso", "title": "OSO", "source_language": "ja", "target_language": "ru"}),
        )
        .await
        .unwrap();
    let term = backend
        .call(
            "hieronymus_termbase_propose",
            json!({
                "series_slug": "oso",
                "source_language": "ko",
                "target_language": "de",
                "category": "name",
                "source_text": "호로",
                "canonical_translation": "Holo"
            }),
        )
        .await
        .unwrap();
    backend
        .call(
            "hieronymus_termbase_approve",
            json!({"series_slug": "oso", "term_id": term["term_id"]}),
        )
        .await
        .expect("candidate context should be authoritative");

    for text in ["first", "second"] {
        backend
            .call(
                "hieronymus_memory_add",
                json!({"series_slug": "oso", "kind": "note", "text": text}),
            )
            .await
            .unwrap();
    }
    backend
        .call(
            "hieronymus_memory_search",
            json!({"series_slug": "oso", "query": "first"}),
        )
        .await
        .unwrap();
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM task_sessions")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(sessions, 1);
    let memories: i64 = sqlx::query_scalar("SELECT count(*) FROM short_term_memories")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(memories, 2);
}

#[tokio::test]
async fn status_uses_shared_doctor_report_instead_of_hard_coded_ok() {
    let (state, root) = test_state().await;
    state.config.ensure_directories().unwrap();
    std::fs::write(
        root.path().join("config").join("providers.toml"),
        "invalid = [",
    )
    .unwrap();
    let backend = StoreMcpBackend::new(state.pool, state.config);

    let status = backend.call("hieronymus_status", json!({})).await.unwrap();
    assert_eq!(status["running"], true);
    assert!(status["doctor"]["checks"].is_array());
    assert!(
        status["doctor"]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["status"] != "ok")
    );
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
async fn http_redacts_secret_path_sql_and_provider_body_from_backend_errors() {
    for leak in [
        "secret API key sk-super-secret",
        "/home/inky/private/providers.toml",
        "SQLITE_CONSTRAINT: raw SQL statement",
        "provider body: account balance and prompt",
    ] {
        let router =
            axum::Router::new().nest_service("/mcp", http::service(Arc::new(FailingBackend(leak))));
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
        let text = result["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(result["result"]["isError"], true);
        assert_eq!(text, "The tool could not complete the request.");
        assert!(!text.contains(leak));
    }
}

#[tokio::test]
async fn http_preserves_classified_payload_validation_but_sanitizes_real_database_failure() {
    let (state, _) = test_state().await;
    let pool = state.pool.clone();
    let router = axum::Router::new().nest_service(
        "/mcp",
        http::service(Arc::new(StoreMcpBackend::new(pool.clone(), state.config))),
    );
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
        .unwrap();
    let session_id = initialize.headers()["mcp-session-id"].clone();
    let mut invalid = mcp_request(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "hieronymus_series_create", "arguments": {"slug": 42}}
    }));
    invalid
        .headers_mut()
        .insert("mcp-session-id", session_id.clone());
    let invalid = response_json(router.clone().oneshot(invalid).await.unwrap()).await;
    assert!(
        invalid["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("invalid payload for MCP tool")
    );

    pool.close().await;
    let mut status = mcp_request(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "hieronymus_status", "arguments": {}}
    }));
    status.headers_mut().insert("mcp-session-id", session_id);
    let status = response_json(router.oneshot(status).await.unwrap()).await;
    assert_eq!(
        status["result"]["content"][0]["text"],
        "The tool could not complete the request."
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

async fn random_port() -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("random loopback port should bind");
    listener
        .local_addr()
        .expect("listener should have an address")
        .port()
}

async fn spawn_stdio_shim(root: &TempDir, port: u16) -> Child {
    Command::new(assert_cmd::cargo::cargo_bin!("hiero"))
        .arg("--data-root")
        .arg(root.path().join("data"))
        .arg("mcp")
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("HIERONYMUS_PORT", port.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("stdio shim should spawn")
}

async fn write_mcp(child: &mut Child, value: Value) {
    let stdin = child.stdin.as_mut().expect("shim stdin should be piped");
    stdin
        .write_all(serde_json::to_string(&value).unwrap().as_bytes())
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();
}

async fn read_mcp(reader: &mut BufReader<tokio::process::ChildStdout>) -> Value {
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(6), reader.read_line(&mut line))
        .await
        .expect("shim should answer promptly")
        .expect("shim stdout should remain readable");
    serde_json::from_str(&line)
        .unwrap_or_else(|error| panic!("invalid MCP response: {error}: {line}"))
}

async fn initialize_stdio(
    child: &mut Child,
    reader: &mut BufReader<tokio::process::ChildStdout>,
) -> Value {
    write_mcp(
        child,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "spawned-test", "version": "1"}
            }
        }),
    )
    .await;
    read_mcp(reader).await
}

#[tokio::test]
async fn stdio_spawned_binary_keeps_one_session_and_forwards_tool_calls() {
    let root = TempDir::new().unwrap();
    let port = random_port().await;
    let config = HieronymusConfig::with_roots(
        root.path().join("data"),
        root.path().join("config/hieronymus"),
    );
    let token_path = config.auth_token_path();
    let (stop, stopped) = oneshot::channel();
    let daemon = tokio::spawn(hiero_bin::daemon::serve(config, port, async move {
        let _ = stopped.await;
    }));
    tokio::time::timeout(Duration::from_secs(3), async {
        while !token_path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("daemon should create its token");

    let mut child = spawn_stdio_shim(&root, port).await;
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let initialized = initialize_stdio(&mut child, &mut reader).await;
    assert_eq!(initialized["result"]["serverInfo"]["name"], "hieronymus");
    write_mcp(
        &mut child,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    write_mcp(
        &mut child,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(
        read_mcp(&mut reader).await["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        40
    );
    write_mcp(
        &mut child,
        json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{"name":"hieronymus_status","arguments":{}}
        }),
    )
    .await;
    assert_ne!(read_mcp(&mut reader).await["result"]["isError"], true);

    drop(child.stdin.take());
    tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .expect("stdio EOF should stop the shim")
        .unwrap();
    stop.send(()).unwrap();
    daemon.await.unwrap().unwrap();
}

#[tokio::test]
async fn stdio_reports_unavailable_daemon_as_a_tool_error() {
    let root = TempDir::new().unwrap();
    let config = HieronymusConfig::with_roots(
        root.path().join("data"),
        root.path().join("config/hieronymus"),
    );
    std::fs::create_dir_all(config.auth_token_path().parent().unwrap()).unwrap();
    std::fs::write(config.auth_token_path(), "test-token\n").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        config.auth_token_path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    let mut child = spawn_stdio_shim(&root, random_port().await).await;
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    initialize_stdio(&mut child, &mut reader).await;
    write_mcp(
        &mut child,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    write_mcp(
        &mut child,
        json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"hieronymus_status","arguments":{}}
        }),
    )
    .await;
    let response = read_mcp(&mut reader).await;
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("daemon is unavailable")
    );
}

#[tokio::test]
async fn stdio_reports_malformed_daemon_response_as_a_tool_error() {
    let root = TempDir::new().unwrap();
    let config = HieronymusConfig::with_roots(
        root.path().join("data"),
        root.path().join("config/hieronymus"),
    );
    std::fs::create_dir_all(config.auth_token_path().parent().unwrap()).unwrap();
    std::fs::write(config.auth_token_path(), "test-token\n").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        config.auth_token_path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 4096];
        let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut request)
            .await
            .unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot-json",
            )
            .await
            .unwrap();
    });
    let mut child = spawn_stdio_shim(&root, port).await;
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    initialize_stdio(&mut child, &mut reader).await;
    write_mcp(
        &mut child,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    write_mcp(
        &mut child,
        json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"hieronymus_status","arguments":{}}
        }),
    )
    .await;
    let response = read_mcp(&mut reader).await;
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("malformed response")
    );
    fake.await.unwrap();
}

#[tokio::test]
async fn stdio_never_follows_a_daemon_redirect_with_the_token() {
    let root = TempDir::new().unwrap();
    let config = HieronymusConfig::with_roots(
        root.path().join("data"),
        root.path().join("config/hieronymus"),
    );
    std::fs::create_dir_all(config.auth_token_path().parent().unwrap()).unwrap();
    std::fs::write(config.auth_token_path(), "redirect-secret\n").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        config.auth_token_path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    let collector = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let collector_port = collector.local_addr().unwrap().port();
    let redirector = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = redirector.local_addr().unwrap().port();
    let redirect = tokio::spawn(async move {
        let (mut socket, _) = redirector.accept().await.unwrap();
        let mut request = vec![0; 4096];
        let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut request)
            .await
            .unwrap();
        let response = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:{collector_port}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let stolen = tokio::spawn(async move {
        match tokio::time::timeout(Duration::from_secs(1), collector.accept()).await {
            Ok(Ok((mut socket, _))) => {
                let mut request = vec![0; 4096];
                let read = tokio::io::AsyncReadExt::read(&mut socket, &mut request)
                    .await
                    .unwrap();
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 13\r\nConnection: close\r\n\r\n{\"result\":{}}",
                    )
                    .await
                    .unwrap();
                String::from_utf8_lossy(&request[..read]).contains("redirect-secret")
            }
            _ => false,
        }
    });

    let mut child = spawn_stdio_shim(&root, port).await;
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    initialize_stdio(&mut child, &mut reader).await;
    write_mcp(
        &mut child,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    write_mcp(
        &mut child,
        json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"hieronymus_status","arguments":{}}
        }),
    )
    .await;
    let response = read_mcp(&mut reader).await;
    assert_eq!(response["result"]["isError"], true);
    assert!(
        !stolen.await.unwrap(),
        "redirect target received the daemon token"
    );
    redirect.await.unwrap();
}

#[tokio::test]
async fn stdio_ignores_environment_proxies_that_could_receive_the_token() {
    let root = TempDir::new().unwrap();
    let port = random_port().await;
    let config = HieronymusConfig::with_roots(
        root.path().join("data"),
        root.path().join("config/hieronymus"),
    );
    let token_path = config.auth_token_path();
    let (stop, stopped) = oneshot::channel();
    let daemon = tokio::spawn(hiero_bin::daemon::serve(config, port, async move {
        let _ = stopped.await;
    }));
    tokio::time::timeout(Duration::from_secs(3), async {
        while !token_path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let proxy = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let proxy_port = proxy.local_addr().unwrap().port();
    let intercepted = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(1), proxy.accept())
            .await
            .is_ok()
    });
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("hiero"))
        .arg("--data-root")
        .arg(root.path().join("data"))
        .arg("mcp")
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("HIERONYMUS_PORT", port.to_string())
        .env("HTTP_PROXY", format!("http://127.0.0.1:{proxy_port}"))
        .env("NO_PROXY", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    initialize_stdio(&mut child, &mut reader).await;
    write_mcp(
        &mut child,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    write_mcp(
        &mut child,
        json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"hieronymus_status","arguments":{}}
        }),
    )
    .await;
    assert_ne!(read_mcp(&mut reader).await["result"]["isError"], true);
    assert!(!intercepted.await.unwrap());
    drop(child.stdin.take());
    child.wait().await.unwrap();
    stop.send(()).unwrap();
    daemon.await.unwrap().unwrap();
}

#[tokio::test]
async fn stdio_times_out_when_a_daemon_accepts_but_never_answers() {
    let root = TempDir::new().unwrap();
    let config = HieronymusConfig::with_roots(
        root.path().join("data"),
        root.path().join("config/hieronymus"),
    );
    std::fs::create_dir_all(config.auth_token_path().parent().unwrap()).unwrap();
    std::fs::write(config.auth_token_path(), "test-token\n").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        config.auth_token_path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let stalled = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let mut child = spawn_stdio_shim(&root, port).await;
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    initialize_stdio(&mut child, &mut reader).await;
    write_mcp(
        &mut child,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    write_mcp(
        &mut child,
        json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"hieronymus_status","arguments":{}}
        }),
    )
    .await;
    let response = tokio::time::timeout(Duration::from_secs(7), read_mcp(&mut reader))
        .await
        .expect("loopback MCP request must have a deadline");
    assert_eq!(response["result"]["isError"], true);
    stalled.abort();
}
