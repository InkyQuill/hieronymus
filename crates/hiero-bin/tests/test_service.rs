use std::{
    future::Future,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    response::Response,
};
use futures::{SinkExt, StreamExt};
use hiero_bin::{
    api::{
        admin::{AdminApi, FeedbackRecorder},
        contracts::AdminActionRequest,
        events::{AdminEvent, admin_event_channel, next_admin_event},
    },
    assets::AssetSource,
    daemon::{
        AppState, WorkerSupervisor, bind_listener, build_router, load_or_create_auth_token, serve,
    },
};
use hiero_core::{
    config::HieronymusConfig,
    db,
    domain::{ConceptStore, CreateConceptInput, FeedbackError, FeedbackEvent},
    dreaming::{DreamConfig, PhaseProfile},
    provider::{
        HttpMethod, PassName, ProviderCatalog, ProviderDefaults, ProviderProfile, ProviderRequest,
        ProviderResponse, ProviderTransport,
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::oneshot,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use tower::ServiceExt;

#[derive(Default)]
struct FakeProviderTransport {
    requests: Mutex<Vec<ProviderRequest>>,
    responses: Mutex<std::collections::VecDeque<hiero_core::provider::Result<ProviderResponse>>>,
}

impl FakeProviderTransport {
    fn with_ollama_models(models: &[&[&str]]) -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(
                models
                    .iter()
                    .map(|models| {
                        Ok(ProviderResponse {
                            status: 200,
                            body: serde_json::to_vec(&json!({
                                "models": models
                                    .iter()
                                    .map(|name| json!({"model": name}))
                                    .collect::<Vec<_>>()
                            }))
                            .expect("fake response should serialize"),
                        })
                    })
                    .collect(),
            ),
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(
                [Err(hiero_core::provider::ProviderError::Transport)]
                    .into_iter()
                    .collect(),
            ),
        })
    }
}

#[async_trait]
impl ProviderTransport for FakeProviderTransport {
    async fn execute(
        &self,
        request: ProviderRequest,
    ) -> hiero_core::provider::Result<ProviderResponse> {
        assert_eq!(request.method(), HttpMethod::Get);
        self.requests
            .lock()
            .expect("request lock should work")
            .push(request);
        self.responses
            .lock()
            .expect("response lock should work")
            .pop_front()
            .expect("fake response should exist")
    }
}

const LOCAL_HOST: &str = "127.0.0.1:9768";
const BODY_LIMIT: usize = 1_000_000;
const AUTH_TOKEN: &str = "test-auth-token";

async fn test_state() -> (AppState, broadcast::Receiver<()>) {
    let pool = db::connect_url("sqlite::memory:?cache=shared")
        .await
        .expect("test database should connect");
    let root = TempDir::new()
        .expect("temporary directory should be created")
        .keep();
    let config = HieronymusConfig::with_roots(root.join("data"), root.join("config"));
    let (shutdown, receiver) = broadcast::channel(4);
    let (events, _) = admin_event_channel(4);
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
        receiver,
    )
}

async fn websocket_server(
    mut state: AppState,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("test WebSocket listener should bind");
    let port = listener
        .local_addr()
        .expect("test listener should have an address")
        .port();
    state.port = port;
    let router = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("test WebSocket server should run");
    });
    let mut request = format!("ws://127.0.0.1:{port}/ws/admin")
        .into_client_request()
        .expect("WebSocket URL should produce a request");
    request.headers_mut().insert(
        header::ORIGIN,
        format!("http://127.0.0.1:{port}")
            .parse()
            .expect("test Origin should be valid"),
    );
    let (socket, response) = connect_async(request)
        .await
        .expect("authorized WebSocket should connect");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    (socket, server)
}

async fn wait_for_no_workers(workers: &WorkerSupervisor) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while workers.active_count().await != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("WebSocket worker should finish");
}

async fn response_body(response: Response) -> Vec<u8> {
    to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable")
        .to_vec()
}

fn override_assets(root: &Path) -> AssetSource {
    AssetSource::override_root(root).expect("fixture asset root should be accepted")
}

fn json_request(method: Method, path: &str, value: Value) -> Request<Body> {
    let mut request = request(
        method,
        path,
        Body::from(serde_json::to_vec(&value).expect("request JSON should serialize")),
    );
    request
        .headers_mut()
        .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    request
}

fn request(method: Method, path: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, LOCAL_HOST)
        .header("x-hieronymus-token", AUTH_TOKEN)
        .body(body)
        .expect("test request should build")
}

#[derive(Clone, Copy)]
struct SecurityCase {
    name: &'static str,
    host: Option<&'static str>,
    origin: Option<&'static str>,
    token: Option<&'static str>,
    status: StatusCode,
    error_code: Option<&'static str>,
}

fn security_request(method: Method, path: &str, case: SecurityCase) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(host) = case.host {
        builder = builder.header(header::HOST, host);
    }
    if let Some(origin) = case.origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    if let Some(token) = case.token {
        builder = builder.header("x-hieronymus-token", token);
    }
    builder
        .body(Body::empty())
        .expect("security test request should build")
}

async fn assert_security_response(response: Response, case: SecurityCase) {
    assert_eq!(response.status(), case.status, "{}", case.name);
    if let Some(error_code) = case.error_code {
        let payload = response_json(response).await;
        assert_eq!(payload["code"], error_code, "{}", case.name);
        assert!(payload["error"].is_string(), "{}", case.name);
        assert!(payload["request_id"].is_string(), "{}", case.name);
    }
}

async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&body).expect("response should contain JSON")
}

#[tokio::test]
async fn router_registers_every_phase_005_section_2_route() {
    let (state, _) = test_state().await;
    let router = build_router(state);
    let routes = [
        (Method::GET, "/", StatusCode::OK),
        (Method::GET, "/admin", StatusCode::OK),
        (Method::GET, "/config", StatusCode::OK),
        (Method::GET, "/assets/app.js", StatusCode::NOT_FOUND),
        (Method::GET, "/api/providers", StatusCode::OK),
        (Method::GET, "/ws/admin", StatusCode::UNAUTHORIZED),
        (Method::GET, "/health", StatusCode::OK),
        (Method::GET, "/status", StatusCode::OK),
        (Method::POST, "/shutdown", StatusCode::OK),
        (Method::POST, "/mcp", StatusCode::NOT_ACCEPTABLE),
        (
            Method::POST,
            "/api/mcp/series_create",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
    ];

    for (method, path, expected) in routes {
        let response = router
            .clone()
            .oneshot(request(method, path, Body::empty()))
            .await
            .expect("router should answer");
        assert_eq!(response.status(), expected, "{path}");
    }
}

#[tokio::test]
async fn api_provider_contract_matches_the_current_typescript_client() {
    let (mut state, _) = test_state().await;
    state.provider_transport = Some(FakeProviderTransport::failing());
    let router = build_router(state);
    let draft = json!({
        "provider": {
            "id": "local-ollama",
            "name": "Local Ollama",
            "type": "ollama",
            "url": "http://127.0.0.1:9",
            "key": "",
            "timeout_seconds": "30"
        }
    });

    let saved = router
        .clone()
        .oneshot(json_request(Method::POST, "/api/providers", draft))
        .await
        .expect("provider save should answer");
    assert_eq!(saved.status(), StatusCode::OK);
    assert_eq!(
        response_json(saved).await,
        json!({
            "provider": {
                "id": "local-ollama",
                "name": "Local Ollama",
                "type": "ollama",
                "url": "http://127.0.0.1:9",
                "key_configured": false,
                "model": "",
                "timeout_seconds": 30.0
            }
        })
    );

    let listed = router
        .clone()
        .oneshot(request(Method::GET, "/api/providers", Body::empty()))
        .await
        .expect("provider list should answer");
    assert_eq!(
        response_json(listed).await,
        json!({
            "providers": [{
                "id": "local-ollama",
                "name": "Local Ollama",
                "type": "ollama",
                "url": "http://127.0.0.1:9",
                "key_configured": false,
                "model": "",
                "timeout_seconds": 30.0
            }]
        })
    );

    let check = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/providers/local-ollama/check",
            json!({}),
        ))
        .await
        .expect("provider check should answer");
    let check = response_json(check).await;
    assert_eq!(check["check"]["ok"], false);
    assert_eq!(check["check"]["models"], json!([]));
    assert_eq!(check["check"]["source"], "live");
    assert!(
        check["check"]["error"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );

    let models = router
        .clone()
        .oneshot(request(
            Method::GET,
            "/api/providers/missing/models",
            Body::empty(),
        ))
        .await
        .expect("model refresh should answer");
    assert_eq!(models.status(), StatusCode::NOT_FOUND);
    let error = response_json(models).await;
    assert_eq!(error["error"], "provider not found");
    assert_eq!(error["code"], "not_found");
    assert!(error["request_id"].is_string());

    let deleted = router
        .oneshot(request(
            Method::DELETE,
            "/api/providers/local-ollama",
            Body::empty(),
        ))
        .await
        .expect("provider delete should answer");
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(response_json(deleted).await, json!({}));
}

#[tokio::test]
async fn api_provider_delete_rejects_workflow_references_and_clears_only_matching_default() {
    let (state, _) = test_state().await;
    let path = state.config.provider_config_path();
    let mut catalog = ProviderCatalog::load(&path).expect("empty catalog should load");
    for id in ["workflow-provider", "default-provider", "free-provider"] {
        catalog
            .upsert(ProviderProfile::new(
                id,
                id,
                "ollama",
                "http://127.0.0.1:11434",
            ))
            .expect("provider should be valid");
    }
    catalog.set_defaults(ProviderDefaults {
        provider: "default-provider".into(),
        model: "default-model".into(),
    });
    catalog.save(&path).expect("catalog should save");
    let mut dream = DreamConfig::default();
    dream.workflows.insert(
        PassName::Concepts,
        PhaseProfile {
            provider: "workflow-provider".into(),
            model: "workflow-model".into(),
            enabled: true,
            max_records_per_pass: 10,
        },
    );
    dream.save(&state.config).expect("dream config should save");
    let router = build_router(state.clone());

    let referenced = router
        .clone()
        .oneshot(request(
            Method::DELETE,
            "/api/providers/workflow-provider",
            Body::empty(),
        ))
        .await
        .expect("referenced delete should answer");
    assert_eq!(referenced.status(), StatusCode::BAD_REQUEST);
    assert!(response_json(referenced).await["error"].is_string());

    let default = router
        .clone()
        .oneshot(request(
            Method::DELETE,
            "/api/providers/default-provider",
            Body::empty(),
        ))
        .await
        .expect("default delete should answer");
    assert_eq!(default.status(), StatusCode::OK);
    let catalog = ProviderCatalog::load(&path).expect("updated catalog should load");
    assert!(catalog.get("workflow-provider").is_some());
    assert!(catalog.get("default-provider").is_none());
    assert_eq!(catalog.defaults(), &ProviderDefaults::default());

    let free = router
        .oneshot(request(
            Method::DELETE,
            "/api/providers/free-provider",
            Body::empty(),
        ))
        .await
        .expect("unreferenced delete should answer");
    assert_eq!(free.status(), StatusCode::OK);
}

fn configure_local_provider(state: &AppState) {
    let mut catalog = ProviderCatalog::load(state.config.provider_config_path())
        .expect("empty catalog should load");
    catalog
        .upsert(ProviderProfile::new(
            "local",
            "Local",
            "ollama",
            "http://127.0.0.1:11434",
        ))
        .expect("provider should be valid");
    catalog
        .save(state.config.provider_config_path())
        .expect("catalog should save");
}

#[tokio::test]
async fn api_provider_check_persists_its_own_live_discovery() {
    let (mut state, _) = test_state().await;
    let transport = FakeProviderTransport::with_ollama_models(&[&["check-live"]]);
    state.provider_transport = Some(transport.clone());
    configure_local_provider(&state);
    let router = build_router(state);

    let check = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/providers/local/check",
            json!({}),
        ))
        .await
        .expect("live check should answer");
    assert_eq!(
        response_json(check).await["check"]["models"],
        json!(["check-live"])
    );

    let dream = router
        .oneshot(request(Method::GET, "/api/settings/dream", Body::empty()))
        .await
        .expect("dream settings should answer");
    assert_eq!(
        response_json(dream).await["model_cache"]["providers"]["local"]["models"],
        json!(["check-live"])
    );
    assert_eq!(
        transport
            .requests
            .lock()
            .expect("request lock should work")
            .len(),
        1
    );
}

#[tokio::test]
async fn api_provider_models_refresh_bypasses_its_own_cached_discovery() {
    let (mut state, _) = test_state().await;
    let transport =
        FakeProviderTransport::with_ollama_models(&[&["models-first"], &["models-refreshed"]]);
    state.provider_transport = Some(transport.clone());
    configure_local_provider(&state);
    let router = build_router(state);

    for expected in ["models-first", "models-refreshed"] {
        let response = router
            .clone()
            .oneshot(request(
                Method::GET,
                "/api/providers/local/models",
                Body::empty(),
            ))
            .await
            .expect("model refresh should answer");
        assert_eq!(response_json(response).await["models"], json!([expected]));
    }

    let dream = router
        .clone()
        .oneshot(request(Method::GET, "/api/settings/dream", Body::empty()))
        .await
        .expect("dream settings should answer");
    assert_eq!(
        response_json(dream).await["model_cache"]["providers"]["local"]["models"],
        json!(["models-refreshed"])
    );
    assert_eq!(
        transport
            .requests
            .lock()
            .expect("request lock should work")
            .len(),
        2
    );
}

#[tokio::test]
async fn api_settings_contract_uses_the_current_wrappers_and_shapes() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    let dream = router
        .clone()
        .oneshot(request(Method::GET, "/api/settings/dream", Body::empty()))
        .await
        .expect("dream settings should answer");
    assert_eq!(dream.status(), StatusCode::OK);
    let dream = response_json(dream).await;
    assert_eq!(
        dream["dream"],
        json!({
            "dreaming": {
                "enabled": false,
                "schedule_interval_minutes": 30,
                "min_pending_short_term_memories": 20,
                "max_pending_short_term_memories": 200,
                "max_short_term_memories_per_cycle": 50,
                "not_enough_memories_cycle_threshold": 5,
                "max_changed_crystals_per_cycle": 200,
                "max_related_concepts_per_cycle": 80,
                "max_related_crystals_per_concept": 20,
                "max_total_affected_crystals": 500,
                "max_short_term_memories_per_run": 500,
                "max_long_term_records_affected_per_run": 1000,
                "max_relation_records_per_pass": 1000,
                "general_prompt": "Use English as the primary searchable memory language. Preserve Japanese, Russian, and other languages only as terms, names, renderings, quoted evidence, or metadata. Long-term crystals must be 1-2 sentences. Short-term memories must be 1-6 sentences."
            },
            "workflows": {
                "concepts": {"provider": "", "model": "", "enabled": false, "max_records_per_pass": 500},
                "coverage_audit": {"provider": "", "model": "", "enabled": false, "max_records_per_pass": 500},
                "knowledge_crystals": {"provider": "", "model": "", "enabled": false, "max_records_per_pass": 500},
                "reinforcement": {"provider": "", "model": "", "enabled": false, "max_records_per_pass": 500},
                "relations": {"provider": "", "model": "", "enabled": false, "max_records_per_pass": 500},
                "rule_crystals": {"provider": "", "model": "", "enabled": false, "max_records_per_pass": 500},
                "terminology_candidates": {"provider": "", "model": "", "enabled": false, "max_records_per_pass": 500}
            }
        })
    );
    assert_eq!(dream["providers"], json!([]));
    assert_eq!(dream["model_cache"], json!({"providers": {}}));
    assert!(
        dream["dream"]["dreaming"]
            .get("reconsolidation_diff_threshold")
            .is_none()
    );

    let ingest = json!({
        "ingest": {
            "short_memory": {
                "warning_sentence_count": 7,
                "rejection_sentence_count": 31,
                "warning_symbol_count": 100,
                "rejection_symbol_count": 200
            },
            "learn": {"max_block_chars": 1400}
        }
    });
    let saved_ingest = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/settings/ingest",
            ingest.clone(),
        ))
        .await
        .expect("ingest settings save should answer");
    assert_eq!(response_json(saved_ingest).await, ingest);

    let release = json!({"release": {"update_channel": "dev"}});
    let saved_release = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/settings/release",
            release.clone(),
        ))
        .await
        .expect("release settings save should answer");
    assert_eq!(response_json(saved_release).await, release);
    let loaded_release = router
        .oneshot(request(Method::GET, "/api/settings/release", Body::empty()))
        .await
        .expect("release settings load should answer");
    assert_eq!(response_json(loaded_release).await, release);
}

#[tokio::test]
async fn api_admin_snapshot_action_and_manual_dream_shapes_match_the_client() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    let dashboard = router
        .clone()
        .oneshot(request(Method::GET, "/api/admin/dashboard", Body::empty()))
        .await
        .expect("dashboard should answer");
    let dashboard = response_json(dashboard).await;
    assert_eq!(dashboard["header"]["product"], "Hieronymus");
    assert!(dashboard["header"]["version"].is_string());
    assert!(dashboard["header"]["tagline"].is_string());
    assert!(dashboard["stats"].is_object());
    assert!(dashboard["views"].is_array());
    assert!(dashboard["short_term_status"].is_object());
    assert!(dashboard["dream_status"].is_object());

    let snapshot = router
        .clone()
        .oneshot(request(
            Method::GET,
            "/api/admin/snapshot?view=Concepts&selected_id=7",
            Body::empty(),
        ))
        .await
        .expect("snapshot should answer");
    assert_eq!(
        response_json(snapshot).await,
        json!({
            "snapshot": {
                "view": "Concepts",
                "rows": [],
                "selected": null,
                "detail": {
                    "title": "Concepts",
                    "subtitle": "No record selected",
                    "body": "",
                    "fields": []
                }
            }
        })
    );

    let unconfirmed = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/admin/actions/delete_crystal",
            json!({"id": 7}),
        ))
        .await
        .expect("confirmation rejection should answer");
    assert_eq!(unconfirmed.status(), StatusCode::BAD_REQUEST);

    let manual = router
        .oneshot(json_request(
            Method::POST,
            "/api/admin/actions/run_manual_dreaming",
            json!({}),
        ))
        .await
        .expect("manual dreaming should answer");
    assert_eq!(
        response_json(manual).await,
        json!({"started": true, "status": "running"})
    );
}

#[tokio::test]
async fn api_admin_snapshot_reads_seeded_concept_and_resolves_selected_id() {
    let (state, _) = test_state().await;
    let concept = ConceptStore::new(&state.pool)
        .create(CreateConceptInput {
            canonical_name: "Guild Ledger".into(),
            scope_type: "series".into(),
            scope_key: "merchant-guild".into(),
        })
        .await
        .expect("concept fixture should be created");
    let router = build_router(state);

    let response = router
        .oneshot(request(
            Method::GET,
            &format!(
                "/api/admin/snapshot?view=Concepts&selected_id={}",
                concept.id
            ),
            Body::empty(),
        ))
        .await
        .expect("snapshot should answer");
    let snapshot = response_json(response).await["snapshot"].clone();

    assert_eq!(snapshot["rows"].as_array().map(Vec::len), Some(1));
    assert_eq!(snapshot["rows"][0]["id"], concept.id);
    assert_eq!(snapshot["rows"][0]["label"], "Guild Ledger");
    assert_eq!(snapshot["selected"]["id"], concept.id);
    assert_eq!(snapshot["detail"]["title"], "Guild Ledger");
    assert_eq!(snapshot["detail"]["body"], "");
}

#[tokio::test]
async fn api_admin_snapshot_resolves_selected_id_older_than_the_bounded_row_list() {
    let (state, _) = test_state().await;
    sqlx::query(
        "WITH RECURSIVE sequence(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value < 201) INSERT INTO concepts(canonical_name, scope_type, scope_key, status, confidence, created_at, updated_at) SELECT printf('Concept %03d', value), 'global', '', 'candidate', 0.2, datetime('now'), datetime('now') FROM sequence",
    )
    .execute(&state.pool)
    .await
    .expect("concept fixtures should be created");
    let oldest_id = sqlx::query_scalar::<_, i64>("SELECT min(id) FROM concepts")
        .fetch_one(&state.pool)
        .await
        .expect("oldest concept should exist");
    let router = build_router(state);

    let response = router
        .oneshot(request(
            Method::GET,
            &format!("/api/admin/snapshot?view=Concepts&selected_id={oldest_id}"),
            Body::empty(),
        ))
        .await
        .expect("snapshot should answer");
    let snapshot = response_json(response).await["snapshot"].clone();

    assert_eq!(snapshot["rows"].as_array().map(Vec::len), Some(200));
    assert!(
        snapshot["rows"]
            .as_array()
            .is_some_and(|rows| rows.iter().all(|row| row["id"] != oldest_id))
    );
    assert_eq!(snapshot["selected"]["id"], oldest_id);
    assert_eq!(snapshot["detail"]["title"], "Concept 001");
}

#[tokio::test]
async fn api_admin_snapshot_reads_every_supported_store_view() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    for view in [
        "Crystals",
        "Lessons",
        "Concepts",
        "Proposals",
        "Short-Term%20Memories",
        "Short-Term%20Sessions",
        "Dream%20Runs",
    ] {
        let response = router
            .clone()
            .oneshot(request(
                Method::GET,
                &format!("/api/admin/snapshot?view={view}"),
                Body::empty(),
            ))
            .await
            .expect("snapshot should answer");

        assert_eq!(response.status(), StatusCode::OK, "view {view}");
        assert!(
            response_json(response).await["snapshot"]["rows"].is_array(),
            "view {view}"
        );
    }
}

#[derive(Clone, Default)]
struct FakeFeedback {
    events: Arc<Mutex<Vec<FeedbackEvent>>>,
}

impl FeedbackRecorder for FakeFeedback {
    fn record(
        &self,
        event: FeedbackEvent,
    ) -> impl Future<Output = Result<(), FeedbackError>> + Send {
        self.events
            .lock()
            .expect("fake lock should work")
            .push(event);
        std::future::ready(Ok(()))
    }

    fn delete_by_user(
        &self,
        crystal_id: i64,
    ) -> impl Future<Output = Result<(), FeedbackError>> + Send {
        self.record(FeedbackEvent {
            crystal_id,
            event_type: "deleted_by_user".into(),
            source_role: "web_admin".into(),
            evidence: Some("Deleted from web admin".into()),
            session_id: None,
        })
    }
}

#[tokio::test]
async fn api_admin_score_actions_delegate_the_event_without_local_arithmetic() {
    let feedback = FakeFeedback::default();
    let calls = feedback.events.clone();
    let (state, _) = test_state().await;
    let api = AdminApi::new(feedback, state.pool);

    let result = api
        .run_action(
            "reinforce_crystal",
            AdminActionRequest {
                id: 42.into(),
                confirmed: None,
            },
        )
        .await
        .expect("delegated action should succeed");

    assert_eq!(result.message, "Crystal reinforced");
    assert_eq!(
        calls.lock().expect("fake lock should work").as_slice(),
        &[FeedbackEvent {
            crystal_id: 42,
            event_type: "confirmed_by_user".into(),
            source_role: "web_admin".into(),
            evidence: Some("Reinforced from web admin".into()),
            session_id: None,
        }]
    );
}

#[tokio::test]
async fn api_admin_concept_score_actions_delegate_to_the_concept_store() {
    let (state, _) = test_state().await;
    let concept = ConceptStore::new(&state.pool)
        .create(CreateConceptInput {
            canonical_name: "Guild Ledger".into(),
            scope_type: "global".into(),
            scope_key: String::new(),
        })
        .await
        .expect("concept fixture should be created");
    let router = build_router(state.clone());

    let response = router
        .oneshot(json_request(
            Method::POST,
            "/api/admin/actions/reinforce_concept",
            json!({"id": concept.id}),
        ))
        .await
        .expect("concept action should answer");

    assert_eq!(response.status(), StatusCode::OK);
    let response = response_json(response).await;
    assert_eq!(response["snapshot"]["rows"][0]["id"], concept.id);
    assert_eq!(response["snapshot"]["selected"]["id"], concept.id);
    assert_eq!(
        ConceptStore::new(&state.pool)
            .get(concept.id)
            .await
            .expect("concept should remain")
            .confidence,
        0.35
    );
}

#[tokio::test]
async fn api_admin_action_accepts_numeric_strings_and_rejects_invalid_ids_stably() {
    let (state, _) = test_state().await;
    let concept = ConceptStore::new(&state.pool)
        .create(CreateConceptInput {
            canonical_name: "Numeric String".into(),
            scope_type: "global".into(),
            scope_key: String::new(),
        })
        .await
        .expect("concept fixture should be created");
    let router = build_router(state);

    let accepted = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/admin/actions/reinforce_concept",
            json!({"id": concept.id.to_string()}),
        ))
        .await
        .expect("numeric-string action should answer");
    assert_eq!(accepted.status(), StatusCode::OK);

    for id in [
        json!("not-a-number"),
        json!("9223372036854775808"),
        json!(0),
        json!(-1),
    ] {
        let rejected = router
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/admin/actions/reinforce_concept",
                json!({"id": id}),
            ))
            .await
            .expect("invalid action should answer");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(rejected).await;
        assert!(payload["error"].is_string());
        assert_eq!(payload["code"], "invalid_request");
        assert!(payload["request_id"].is_string());
    }
}

#[tokio::test]
async fn manual_dream_worker_supervisor_cancels_and_joins_in_flight_work() {
    struct RunningGuard(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for RunningGuard {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::Release);
        }
    }

    let workers = WorkerSupervisor::default();
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let entered = Arc::new(tokio::sync::Notify::new());
    let task_running = running.clone();
    let task_entered = entered.clone();
    assert!(
        workers
            .spawn(async move {
                let _guard = RunningGuard(task_running);
                task_entered.notify_one();
                std::future::pending::<()>().await;
            })
            .await
    );
    entered.notified().await;

    let shutdown_workers = workers.clone();
    let joined = tokio::spawn(async move {
        shutdown_workers.shutdown().await.unwrap();
    });
    joined.await.expect("shutdown join should complete");

    assert!(!running.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(workers.active_count().await, 0);
    assert!(!workers.spawn(async {}).await);
}

#[tokio::test]
async fn manual_worker_supervisor_reaps_completion_without_spawn_poll_or_shutdown() {
    let workers = WorkerSupervisor::default();
    let (completed, observed) = tokio::sync::oneshot::channel();
    assert!(
        workers
            .spawn(async move {
                let _ = completed.send(());
            })
            .await
    );
    observed.await.expect("worker should complete");

    tokio::time::timeout(Duration::from_secs(1), workers.wait_idle())
        .await
        .expect("continuous reaper should observe completion");
    assert_eq!(workers.active_count().await, 0);
}

#[tokio::test]
async fn worker_supervisor_surfaces_task_failures_at_shutdown() {
    let workers = WorkerSupervisor::default();
    assert!(
        workers
            .spawn_result(async { anyhow::bail!("recurring worker failed") })
            .await
    );
    workers.wait_idle().await;

    let error = workers
        .shutdown()
        .await
        .expect_err("worker failure must reach the daemon lifecycle");
    assert!(error.to_string().contains("recurring worker failed"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_supervisor_bounds_abort_and_reap_for_a_stalled_worker() {
    struct SlowDrop;
    impl Drop for SlowDrop {
        fn drop(&mut self) {
            std::thread::sleep(Duration::from_secs(5));
        }
    }

    let workers = WorkerSupervisor::default();
    let entered = Arc::new(tokio::sync::Notify::new());
    let task_entered = entered.clone();
    assert!(
        workers
            .spawn(async move {
                let _slow_drop = SlowDrop;
                task_entered.notify_one();
                std::future::pending::<()>().await;
            })
            .await
    );
    entered.notified().await;

    tokio::time::timeout(Duration::from_secs(4), workers.shutdown())
        .await
        .expect("supervisor shutdown itself must be bounded")
        .expect("forced cancellation is a clean shutdown");
    assert_eq!(workers.active_count().await, 0);
}

#[tokio::test]
async fn worker_supervisor_bounds_retained_failure_summaries() {
    let workers = WorkerSupervisor::default();
    for index in 0..20 {
        assert!(
            workers
                .spawn_result(
                    async move { anyhow::bail!("failure-{index:02}-{}", "x".repeat(2_000)) }
                )
                .await
        );
    }
    workers.wait_idle().await;

    let error = workers
        .shutdown()
        .await
        .expect_err("worker failures must reach the daemon lifecycle");
    let summary = error.to_string();
    assert!(summary.contains("failure-19-"));
    assert!(!summary.contains("failure-00-"));
    assert!(
        summary.len() < 17_000,
        "retained failure summary should stay bounded"
    );
}

#[tokio::test]
async fn router_mcp_security_matrix_is_route_complete() {
    let (state, _) = test_state().await;
    let router = build_router(state);
    let cases = [
        SecurityCase {
            name: "MCP correct token and missing Origin",
            host: Some(LOCAL_HOST),
            origin: None,
            token: Some(AUTH_TOKEN),
            status: StatusCode::NOT_ACCEPTABLE,
            error_code: None,
        },
        SecurityCase {
            name: "MCP missing token",
            host: Some(LOCAL_HOST),
            origin: None,
            token: None,
            status: StatusCode::UNAUTHORIZED,
            error_code: Some("unauthorized"),
        },
        SecurityCase {
            name: "MCP wrong token",
            host: Some(LOCAL_HOST),
            origin: None,
            token: Some("wrong-token"),
            status: StatusCode::UNAUTHORIZED,
            error_code: Some("unauthorized"),
        },
        SecurityCase {
            name: "MCP missing Host",
            host: None,
            origin: None,
            token: Some(AUTH_TOKEN),
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "MCP malformed Host authority",
            host: Some("127.0.0.1:not-a-port"),
            origin: None,
            token: Some(AUTH_TOKEN),
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "MCP wrong loopback Host port",
            host: Some("127.0.0.1:9999"),
            origin: None,
            token: Some(AUTH_TOKEN),
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "MCP same Origin",
            host: Some(LOCAL_HOST),
            origin: Some("http://127.0.0.1:9768"),
            token: Some(AUTH_TOKEN),
            status: StatusCode::NOT_ACCEPTABLE,
            error_code: None,
        },
        SecurityCase {
            name: "MCP foreign Origin",
            host: Some(LOCAL_HOST),
            origin: Some("https://evil.example"),
            token: Some(AUTH_TOKEN),
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
    ];

    for case in cases {
        let response = router
            .clone()
            .oneshot(security_request(Method::POST, "/mcp", case))
            .await
            .expect("router should answer");
        assert_security_response(response, case).await;
    }
}

#[tokio::test]
async fn router_browser_admin_security_matrix_is_route_complete() {
    let (state, _) = test_state().await;
    let router = build_router(state);
    let cases = [
        SecurityCase {
            name: "admin correct token and missing Origin",
            host: Some(LOCAL_HOST),
            origin: None,
            token: Some(AUTH_TOKEN),
            status: StatusCode::OK,
            error_code: None,
        },
        SecurityCase {
            name: "admin missing token and missing Origin",
            host: Some(LOCAL_HOST),
            origin: None,
            token: None,
            status: StatusCode::UNAUTHORIZED,
            error_code: Some("unauthorized"),
        },
        SecurityCase {
            name: "admin wrong token and missing Origin",
            host: Some(LOCAL_HOST),
            origin: None,
            token: Some("wrong-token"),
            status: StatusCode::UNAUTHORIZED,
            error_code: Some("unauthorized"),
        },
        SecurityCase {
            name: "admin missing Host",
            host: None,
            origin: Some("http://127.0.0.1:9768"),
            token: None,
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "admin malformed Host authority",
            host: Some("127.0.0.1:not-a-port"),
            origin: Some("http://127.0.0.1:9768"),
            token: None,
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "admin wrong loopback Host port",
            host: Some("127.0.0.1:9999"),
            origin: Some("http://127.0.0.1:9768"),
            token: None,
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "admin same Origin without token",
            host: Some(LOCAL_HOST),
            origin: Some("http://127.0.0.1:9768"),
            token: None,
            status: StatusCode::OK,
            error_code: None,
        },
        SecurityCase {
            name: "admin foreign Origin with correct token",
            host: Some(LOCAL_HOST),
            origin: Some("https://evil.example"),
            token: Some(AUTH_TOKEN),
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
    ];

    for case in cases {
        let response = router
            .clone()
            .oneshot(security_request(Method::GET, "/api/providers", case))
            .await
            .expect("router should answer");
        assert_security_response(response, case).await;
    }
}

#[tokio::test]
async fn router_rejects_a_request_body_larger_than_one_megabyte() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    let accepted = router
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/mcp/probe",
            Body::from(vec![b'a'; BODY_LIMIT]),
        ))
        .await
        .expect("router should answer");
    assert_eq!(accepted.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let rejected = router
        .oneshot(request(
            Method::POST,
            "/api/mcp/probe",
            Body::from(vec![b'a'; BODY_LIMIT + 1]),
        ))
        .await
        .expect("router should answer");
    assert_eq!(rejected.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(response_json(rejected).await["code"], "payload_too_large");
}

#[tokio::test]
async fn router_preserves_or_generates_request_ids_on_every_response() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    let supplied = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/missing.json")
                .header(header::HOST, LOCAL_HOST)
                .header("x-hieronymus-token", AUTH_TOKEN)
                .header("x-request-id", "test-request-123")
                .body(Body::empty())
                .expect("test request should build"),
        )
        .await
        .expect("router should answer");
    assert_eq!(
        supplied.headers()["x-request-id"],
        "test-request-123",
        "the caller's correlation ID should be preserved"
    );
    assert_eq!(
        response_json(supplied).await["request_id"],
        "test-request-123"
    );

    let generated = router
        .oneshot(request(Method::GET, "/health", Body::empty()))
        .await
        .expect("router should answer");
    let generated_id = generated.headers()["x-request-id"]
        .to_str()
        .expect("request ID should be ASCII");
    assert!(
        uuid::Uuid::parse_str(generated_id).is_ok(),
        "generated request ID should be a UUID"
    );
}

#[tokio::test]
async fn router_health_is_cheap_and_status_reports_doctor_checks() {
    let (state, _) = test_state().await;
    let router = build_router(state.clone());

    let health = router
        .clone()
        .oneshot(request(Method::GET, "/health", Body::empty()))
        .await
        .expect("router should answer");
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(
        response_json(health).await,
        json!({
            "ok": true,
            "service": "hieronymus",
            "version": env!("CARGO_PKG_VERSION"),
        })
    );

    let status = router
        .clone()
        .oneshot(request(Method::GET, "/status", Body::empty()))
        .await
        .expect("router should answer");
    assert_eq!(status.status(), StatusCode::OK);
    let payload = response_json(status).await;
    assert_eq!(payload["running"], true);
    assert!(payload["doctor"]["checks"].is_array());

    state.pool.close().await;
    let health_after_close = router
        .oneshot(request(Method::GET, "/health", Body::empty()))
        .await
        .expect("router should answer");
    assert_eq!(health_after_close.status(), StatusCode::OK);
}

#[tokio::test]
async fn router_shutdown_security_matrix_is_route_complete_and_side_effect_safe() {
    let cases = [
        (
            SecurityCase {
                name: "shutdown correct token and missing Origin",
                host: Some(LOCAL_HOST),
                origin: None,
                token: Some(AUTH_TOKEN),
                status: StatusCode::OK,
                error_code: None,
            },
            true,
        ),
        (
            SecurityCase {
                name: "shutdown missing token",
                host: Some(LOCAL_HOST),
                origin: None,
                token: None,
                status: StatusCode::UNAUTHORIZED,
                error_code: Some("unauthorized"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown wrong token",
                host: Some(LOCAL_HOST),
                origin: None,
                token: Some("wrong-token"),
                status: StatusCode::UNAUTHORIZED,
                error_code: Some("unauthorized"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown missing Host",
                host: None,
                origin: None,
                token: Some(AUTH_TOKEN),
                status: StatusCode::FORBIDDEN,
                error_code: Some("forbidden"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown malformed Host authority",
                host: Some("127.0.0.1:not-a-port"),
                origin: None,
                token: Some(AUTH_TOKEN),
                status: StatusCode::FORBIDDEN,
                error_code: Some("forbidden"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown wrong loopback Host port",
                host: Some("127.0.0.1:9999"),
                origin: None,
                token: Some(AUTH_TOKEN),
                status: StatusCode::FORBIDDEN,
                error_code: Some("forbidden"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown same Origin with correct token",
                host: Some(LOCAL_HOST),
                origin: Some("http://127.0.0.1:9768"),
                token: Some(AUTH_TOKEN),
                status: StatusCode::OK,
                error_code: None,
            },
            true,
        ),
        (
            SecurityCase {
                name: "shutdown same Origin without token",
                host: Some(LOCAL_HOST),
                origin: Some("http://127.0.0.1:9768"),
                token: None,
                status: StatusCode::UNAUTHORIZED,
                error_code: Some("unauthorized"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown foreign Origin with correct token",
                host: Some(LOCAL_HOST),
                origin: Some("https://evil.example"),
                token: Some(AUTH_TOKEN),
                status: StatusCode::FORBIDDEN,
                error_code: Some("forbidden"),
            },
            false,
        ),
    ];

    for (case, should_signal) in cases {
        let (state, mut shutdown) = test_state().await;
        let response = build_router(state)
            .oneshot(security_request(Method::POST, "/shutdown", case))
            .await
            .expect("router should answer");
        if case.error_code.is_some() {
            assert_security_response(response, case).await;
        } else {
            assert_eq!(response.status(), case.status, "{}", case.name);
            assert_eq!(
                response_json(response).await,
                json!({"ok": true, "stopping": true}),
                "{}",
                case.name
            );
        }

        let signal = tokio::time::timeout(Duration::from_millis(20), shutdown.recv()).await;
        let received_shutdown = matches!(signal, Ok(Ok(())));
        assert_eq!(received_shutdown, should_signal, "{}", case.name);
    }
}

#[tokio::test]
async fn router_internal_errors_are_sanitized_and_correlated() {
    let (state, _) = test_state().await;
    state.pool.close().await;
    let router = build_router(state);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/status")
                .header(header::HOST, LOCAL_HOST)
                .header("x-hieronymus-token", AUTH_TOKEN)
                .header("x-request-id", "closed-pool-request")
                .body(Body::empty())
                .expect("test request should build"),
        )
        .await
        .expect("router should answer");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response_json(response).await;
    assert_eq!(
        body,
        json!({
            "error": "internal server error",
            "code": "internal_error",
            "request_id": "closed-pool-request",
        })
    );
    assert!(!body.to_string().contains("pool closed"));
}

#[tokio::test]
async fn router_method_errors_use_the_stable_json_envelope() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/health")
                .header(header::HOST, LOCAL_HOST)
                .header("x-hieronymus-token", AUTH_TOKEN)
                .header("x-request-id", "wrong-method-request")
                .body(Body::empty())
                .expect("test request should build"),
        )
        .await
        .expect("router should answer");
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response_json(response).await,
        json!({
            "error": "method not allowed",
            "code": "method_not_allowed",
            "request_id": "wrong-method-request",
        })
    );
}

#[tokio::test]
async fn router_binding_helper_exposes_the_exact_ipv4_loopback_address() {
    let reservation = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("test port should be reservable");
    let port = reservation
        .local_addr()
        .expect("listener should have an address")
        .port();
    drop(reservation);

    let listener = bind_listener(port)
        .await
        .expect("daemon listener should bind");
    assert_eq!(
        listener.local_addr().expect("listener has an address"),
        format!("127.0.0.1:{port}")
            .parse()
            .expect("expected address should parse")
    );
}

#[test]
fn router_auth_token_file_is_created_once_with_private_permissions() {
    let temp = TempDir::new().expect("temporary directory should be created");
    let path = temp.path().join("config").join("auth-token");

    let first = load_or_create_auth_token(&path).expect("token should be created");
    let second = load_or_create_auth_token(&path).expect("token should be reused");

    assert_eq!(first, second);
    assert!(first.len() >= 64);
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(path)
            .expect("token metadata should be readable")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn router_auth_token_file_rejects_non_private_existing_permissions() {
    let temp = TempDir::new().expect("temporary directory should be created");
    let path = temp.path().join("auth-token");
    std::fs::write(&path, AUTH_TOKEN).expect("fixture token should be written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("fixture permissions should be set");

    let error = load_or_create_auth_token(&path).expect_err("public token file must be rejected");

    assert!(error.to_string().contains("0600"));
}

#[tokio::test]
async fn router_serve_fails_fast_when_the_requested_port_is_occupied() {
    let occupied = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("test port should be reservable");
    let port = occupied
        .local_addr()
        .expect("listener should have an address")
        .port();
    let temp = TempDir::new().expect("temporary directory should be created");
    let config =
        HieronymusConfig::load(Some(temp.path().join("data"))).expect("test config should load");

    let error = serve(config, port, std::future::pending())
        .await
        .expect_err("occupied port must fail startup");
    let message = format!("{error:#}");
    assert!(message.contains(&format!("127.0.0.1:{port}")));
    assert!(
        message.contains("in use") || message.contains("Address already in use"),
        "{message}"
    );
}

#[tokio::test]
async fn assets_embedded_bundle_serves_index_and_hashed_files_with_cache_contracts() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    let index = router
        .clone()
        .oneshot(request(Method::GET, "/", Body::empty()))
        .await
        .expect("router should serve the embedded index");
    assert_eq!(index.status(), StatusCode::OK);
    assert_eq!(
        index.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(index.headers()[header::CACHE_CONTROL], "no-cache");
    assert!(
        String::from_utf8(response_body(index).await)
            .expect("embedded index should be UTF-8")
            .contains("<div id=\"app\"></div>")
    );

    let script = router
        .clone()
        .oneshot(request(
            Method::GET,
            "/assets/index-DnP9ckhr.js",
            Body::empty(),
        ))
        .await
        .expect("router should serve an embedded hashed asset");
    assert_eq!(script.status(), StatusCode::OK);
    assert_eq!(
        script.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        script.headers()[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );

    let stylesheet = router
        .oneshot(request(
            Method::GET,
            "/assets/index-BdYtw-xD.css",
            Body::empty(),
        ))
        .await
        .expect("router should serve an embedded hashed stylesheet");
    assert_eq!(stylesheet.status(), StatusCode::OK);
    assert_eq!(
        stylesheet.headers()[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn assets_override_serves_mime_cache_and_spa_fallback_without_frontend_changes() {
    let temp = TempDir::new().expect("temporary directory should be created");
    std::fs::create_dir(temp.path().join("assets")).expect("asset directory should be created");
    std::fs::write(
        temp.path().join("index.html"),
        b"<!doctype html><main>override console</main>",
    )
    .expect("override index should be written");
    std::fs::write(
        temp.path().join("assets").join("app-01234567.css"),
        b"body{color:teal}",
    )
    .expect("override stylesheet should be written");
    std::fs::write(
        temp.path().join("assets").join("guide-reference.css"),
        b"body{color:purple}",
    )
    .expect("non-hashed stylesheet should be written");
    let (mut state, _) = test_state().await;
    state.assets = override_assets(temp.path());
    let router = build_router(state);

    let stylesheet = router
        .clone()
        .oneshot(request(
            Method::GET,
            "/assets/app-01234567.css",
            Body::empty(),
        ))
        .await
        .expect("router should serve an override asset");
    assert_eq!(stylesheet.status(), StatusCode::OK);
    assert_eq!(
        stylesheet.headers()[header::CONTENT_TYPE],
        "text/css; charset=utf-8"
    );
    assert_eq!(
        stylesheet.headers()[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(response_body(stylesheet).await, b"body{color:teal}");

    let non_hashed = router
        .clone()
        .oneshot(request(
            Method::GET,
            "/assets/guide-reference.css",
            Body::empty(),
        ))
        .await
        .expect("router should serve a non-hashed override asset");
    assert_eq!(non_hashed.status(), StatusCode::OK);
    assert_eq!(
        non_hashed.headers()[header::CACHE_CONTROL],
        "public, max-age=3600"
    );

    let client_route = router
        .oneshot(request(Method::GET, "/memories/selected", Body::empty()))
        .await
        .expect("router should serve the SPA shell for a client route");
    assert_eq!(client_route.status(), StatusCode::OK);
    assert_eq!(client_route.headers()[header::CACHE_CONTROL], "no-cache");
    assert_eq!(
        response_body(client_route).await,
        b"<!doctype html><main>override console</main>"
    );
}

#[tokio::test]
async fn assets_override_rejects_traversal_symlink_escape_and_special_files_without_path_leaks() {
    let temp = TempDir::new().expect("temporary directory should be created");
    let root = temp.path().join("dist");
    std::fs::create_dir(&root).expect("asset directory should be created");
    std::fs::write(root.join("index.html"), b"safe index")
        .expect("fixture index should be written");
    let outside = temp.path().join("outside-secret.txt");
    std::fs::write(&outside, b"do not serve").expect("outside fixture should be written");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("escaped.txt"))
        .expect("fixture symlink should be created");
    #[cfg(unix)]
    let _socket = std::os::unix::net::UnixListener::bind(root.join("special.sock"))
        .expect("fixture socket should be created");

    let (mut state, _) = test_state().await;
    state.assets = override_assets(&root);
    let router = build_router(state);
    let paths = [
        "/assets/%2e%2e/outside-secret.txt",
        "/assets/escaped.txt",
        "/assets/special.sock",
        "/assets/missing.js",
    ];
    for path in paths {
        let response = router
            .clone()
            .oneshot(request(Method::GET, path, Body::empty()))
            .await
            .expect("router should reject an unsafe asset path");
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        let body = String::from_utf8(response_body(response).await)
            .expect("sanitized error body should be UTF-8");
        assert!(
            !body.contains(temp.path().to_string_lossy().as_ref()),
            "{path}"
        );
        assert!(!body.contains("outside-secret"), "{path}");
    }
}

#[tokio::test]
async fn events_websocket_subscribes_streams_events_and_cleans_up_after_disconnect() {
    let (state, _) = test_state().await;
    let events = state.events.clone();
    let workers = state.workers.clone();
    let (mut socket, server) = websocket_server(state).await;

    events
        .send(AdminEvent::Refresh)
        .expect("connected client should subscribe");
    let message = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("event should arrive promptly")
        .expect("WebSocket should remain connected")
        .expect("event frame should be valid");
    assert_eq!(
        serde_json::from_str::<Value>(
            message
                .to_text()
                .expect("admin event should be sent as text")
        )
        .expect("admin event should be JSON"),
        json!({"type": "refresh"})
    );

    socket
        .send(Message::Close(None))
        .await
        .expect("client close should be sent");
    drop(socket);
    wait_for_no_workers(&workers).await;
    server.abort();
}

#[tokio::test]
async fn events_lag_emits_the_stable_resync_required_event() {
    let (events, mut receiver) = admin_event_channel(1);
    events
        .send(AdminEvent::Refresh)
        .expect("test receiver should be present");
    events
        .send(AdminEvent::Refresh)
        .expect("test receiver should remain present");

    assert_eq!(
        next_admin_event(&mut receiver).await,
        Some(AdminEvent::ResyncRequired)
    );
}

#[tokio::test]
async fn events_shutdown_closes_the_websocket_and_joins_its_worker() {
    let (state, _) = test_state().await;
    let shutdown = state.shutdown.clone();
    let workers = state.workers.clone();
    let (mut socket, server) = websocket_server(state).await;

    shutdown
        .send(())
        .expect("connected WebSocket should observe shutdown");
    let close = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("WebSocket should close promptly");
    assert!(
        close.is_none() || matches!(close, Some(Ok(Message::Close(_)))),
        "{close:?}"
    );
    wait_for_no_workers(&workers).await;
    server.abort();
}

#[tokio::test]
async fn events_serve_external_shutdown_closes_websocket_then_drains_in_flight_http() {
    let reservation = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("test port should be reservable");
    let port = reservation
        .local_addr()
        .expect("listener should have an address")
        .port();
    drop(reservation);
    let temp = TempDir::new().expect("temporary directory should be created");
    let config = HieronymusConfig::with_roots(temp.path().join("data"), temp.path().join("config"));
    let token_path = config.auth_token_path();
    let (signal, shutdown) = oneshot::channel();
    let server = tokio::spawn(serve(config, port, async move {
        let _ = shutdown.await;
    }));
    let token = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(token) = tokio::fs::read_to_string(&token_path).await {
                break token.trim().to_owned();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("daemon should publish its token");

    let mut websocket_request = format!("ws://127.0.0.1:{port}/ws/admin")
        .into_client_request()
        .expect("WebSocket URL should produce a request");
    websocket_request.headers_mut().insert(
        header::ORIGIN,
        format!("http://127.0.0.1:{port}")
            .parse()
            .expect("test Origin should be valid"),
    );
    websocket_request.headers_mut().insert(
        "x-hieronymus-token",
        token.parse().expect("test token should be a valid header"),
    );
    let (mut socket, _) = connect_async(websocket_request)
        .await
        .expect("same-origin WebSocket should connect through serve");

    let mut in_flight = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("in-flight request should connect");
    in_flight
        .write_all(
            format!(
                "POST /api/mcp/probe HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                 X-Hieronymus-Token: {token}\r\nContent-Length: 1\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("partial request headers should be sent");

    let mut accepted_after_partial = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("health probe should connect");
    accepted_after_partial
        .write_all(
            format!(
                "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                 X-Hieronymus-Token: {token}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("health probe should be sent");
    let mut health = Vec::new();
    accepted_after_partial
        .read_to_end(&mut health)
        .await
        .expect("health response should be read");
    assert!(health.starts_with(b"HTTP/1.1 200"));

    signal
        .send(())
        .expect("external shutdown should be delivered");
    let close = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("external shutdown should close WebSocket promptly")
        .expect("server should send a close frame")
        .expect("close frame should be valid");
    assert!(matches!(
        close,
        Message::Close(Some(frame))
            if frame.code
                == tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Away
    ));
    assert!(
        !server.is_finished(),
        "Axum should still be draining the partial HTTP request"
    );

    in_flight
        .write_all(b"x")
        .await
        .expect("in-flight request should remain writable during drain");
    let mut response = Vec::new();
    in_flight
        .read_to_end(&mut response)
        .await
        .expect("drained response should be readable");
    assert!(response.starts_with(b"HTTP/1.1 415"));
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .expect("daemon should finish after in-flight request drains")
        .expect("daemon task should not panic")
        .expect("daemon should stop cleanly");
}

#[tokio::test]
async fn shutdown_aborts_an_in_flight_request_after_the_bounded_grace_period() {
    let reservation = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let temp = TempDir::new().unwrap();
    let config = HieronymusConfig::with_roots(temp.path().join("data"), temp.path().join("config"));
    let token_path = config.auth_token_path();
    let (signal, shutdown) = oneshot::channel();
    let server = tokio::spawn(serve(config, port, async move {
        let _ = shutdown.await;
    }));
    let token = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(token) = tokio::fs::read_to_string(&token_path).await {
                break token.trim().to_owned();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut request = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    request
        .write_all(
            format!(
                "POST /api/mcp/status HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                 X-Hieronymus-Token: {token}\r\nContent-Type: application/json\r\n\
                 Content-Length: 1\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut accepted = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    accepted
        .write_all(
            format!(
                "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                 X-Hieronymus-Token: {token}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut health = Vec::new();
    accepted.read_to_end(&mut health).await.unwrap();
    assert!(health.starts_with(b"HTTP/1.1 200"));
    signal.send(()).unwrap();

    let error = tokio::time::timeout(Duration::from_secs(7), server)
        .await
        .expect("bounded graceful shutdown should finish")
        .unwrap()
        .expect_err("an exhausted grace period should be surfaced");
    assert!(error.to_string().contains("graceful shutdown timed out"));
    drop(request);
}

#[tokio::test]
async fn events_websocket_requires_same_origin_even_with_a_valid_daemon_token() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    let missing_origin = router
        .clone()
        .oneshot(request(Method::GET, "/ws/admin", Body::empty()))
        .await
        .expect("router should reject an origin-less WebSocket request");
    assert_eq!(missing_origin.status(), StatusCode::UNAUTHORIZED);

    let same_origin = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ws/admin")
                .header(header::HOST, LOCAL_HOST)
                .header(header::ORIGIN, format!("http://{LOCAL_HOST}"))
                .body(Body::empty())
                .expect("same-origin request should build"),
        )
        .await
        .expect("router should authorize a same-origin WebSocket request");
    assert_eq!(same_origin.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn assets_spa_fallback_never_claims_reserved_transport_namespaces() {
    let (state, _) = test_state().await;
    let router = build_router(state);
    let reserved = [
        "/api",
        "/api/unknown",
        "/mcp",
        "/mcp/unknown",
        "/ws",
        "/ws/unknown",
        "/health",
        "/health/unknown",
        "/status",
        "/status/unknown",
        "/shutdown",
        "/shutdown/unknown",
    ];

    for path in reserved {
        let response = router
            .clone()
            .oneshot(request(Method::GET, path, Body::empty()))
            .await
            .expect("reserved namespace should answer");
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = response_body(response).await;
        assert_ne!(content_type, "text/html; charset=utf-8", "{path}");
        assert!(
            !body
                .windows(b"<div id=\"app\"></div>".len())
                .any(|window| window == b"<div id=\"app\"></div>"),
            "{path}"
        );
    }
}
