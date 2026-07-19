use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use hiero_core::provider::{
    AnthropicProvider, CredentialSource, DreamProvider, GoogleProvider, HttpMethod, ModelCache,
    ModelCacheEntry, OllamaProvider, OpenAiProvider, PassName, ProviderCatalog, ProviderDefaults,
    ProviderError, ProviderProfile, ProviderRegistry, ProviderRequest, ProviderResponse,
    ProviderTransport, ReqwestTransport, ReqwestTransportOptions,
};
use secrecy::SecretString;

fn profile(id: &str, kind: &str, url: &str) -> ProviderProfile {
    ProviderProfile::new(id, id, kind, url).with_timeout(Duration::from_secs(2))
}

fn keyed(profile: ProviderProfile, key: &str) -> ProviderProfile {
    profile.with_credential_source(CredentialSource::Inline(SecretString::from(key.to_owned())))
}

#[derive(Default)]
struct FakeTransport {
    requests: Mutex<Vec<ProviderRequest>>,
    responses: Mutex<VecDeque<hiero_core::provider::Result<ProviderResponse>>>,
}

impl FakeTransport {
    fn with_json(values: impl IntoIterator<Item = serde_json::Value>) -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(
                values
                    .into_iter()
                    .map(|value| {
                        Ok(ProviderResponse {
                            status: 200,
                            body: serde_json::to_vec(&value).unwrap(),
                        })
                    })
                    .collect(),
            ),
        })
    }
    fn requests(&self) -> Vec<ProviderRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ProviderTransport for FakeTransport {
    async fn execute(
        &self,
        request: ProviderRequest,
    ) -> hiero_core::provider::Result<ProviderResponse> {
        self.requests.lock().unwrap().push(request);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(ProviderError::Transport))
    }
}

fn context() -> hiero_core::domain::TranslationContext {
    hiero_core::domain::TranslationContext::new("series", "en", "ru")
}

#[test]
fn catalog_loads_legacy_top_level_fixture_and_round_trips_defaults() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("provider.conf");
    std::fs::write(
        &path,
        r#"
[deepseek-api]
name = "Deepseek"
type = "openai"
url = "https://api.deepseek.com"
key = "raw-secret"
timeout_seconds = 45

[local]
type = "ollama"
url = "http://127.0.0.1:11434"

[defaults]
provider = "deepseek-api"
model = "deepseek-v4-flash"
"#,
    )
    .unwrap();

    let catalog = ProviderCatalog::load(&path).unwrap();
    assert_eq!(catalog.get("deepseek-api").unwrap().name(), "Deepseek");
    assert_eq!(catalog.get("local").unwrap().name(), "local");
    assert_eq!(
        catalog.defaults(),
        &ProviderDefaults {
            provider: "deepseek-api".into(),
            model: "deepseek-v4-flash".into()
        }
    );
    assert!(!format!("{:?}", catalog.get("deepseek-api").unwrap()).contains("raw-secret"));

    catalog.save(&path).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("[deepseek-api]"));
    assert!(raw.contains("[defaults]"));
    assert!(!raw.contains("[providers."));
    assert!(raw.contains("key = \"raw-secret\""));
    assert_eq!(
        ProviderCatalog::load(&path).unwrap().defaults(),
        catalog.defaults()
    );
}

#[test]
fn catalog_migrates_api_key_alias_and_gemini_to_canonical_key_and_google() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("provider.conf");
    std::fs::write(&path, "[gemini]\ntype='gemini'\nurl='https://generativelanguage.googleapis.com'\napi_key='legacy-secret'\n").unwrap();
    let catalog = ProviderCatalog::load(&path).unwrap();
    assert_eq!(catalog.get("gemini").unwrap().provider_type(), "google");
    catalog.save(&path).unwrap();
    let raw = std::fs::read_to_string(path).unwrap();
    assert!(raw.contains("type = \"google\""));
    assert!(raw.contains("key = \"legacy-secret\""));
    assert!(!raw.contains("api_key"));
}

#[test]
fn catalog_crud_has_no_duplicate_id_or_model_in_profile_payload() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("provider.conf");
    let mut catalog = ProviderCatalog::default();
    catalog
        .upsert(profile("openai", "openai", "https://api.openai.com/v1"))
        .unwrap();
    catalog.set_defaults(ProviderDefaults {
        provider: "openai".into(),
        model: "gpt-4.1".into(),
    });
    catalog.save(&path).unwrap();
    let raw = std::fs::read_to_string(path).unwrap();
    let table = raw.parse::<toml::Table>().unwrap();
    assert!(!table["openai"].as_table().unwrap().contains_key("id"));
    assert!(!table["openai"].as_table().unwrap().contains_key("model"));
}

#[test]
fn catalog_rejects_unknown_symlink_and_fifo_inputs() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("provider.conf");
    std::fs::write(
        &path,
        "[bad]\ntype='openai'\nurl='https://example.test'\nsurprise=true\n",
    )
    .unwrap();
    assert!(matches!(
        ProviderCatalog::load(&path),
        Err(ProviderError::Config(_))
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = root.path().join("outside");
        std::fs::write(&outside, "keep").unwrap();
        std::fs::remove_file(&path).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(ProviderCatalog::default().save(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        assert!(ProviderCatalog::load(&path).is_err());
    }
}

#[test]
fn pass_name_is_exhaustive_and_unknown_values_fail() {
    assert_eq!(PassName::ALL.len(), 7);
    assert_eq!(
        serde_json::to_string(&PassName::CoverageAudit).unwrap(),
        "\"coverage_audit\""
    );
    assert!(serde_json::from_str::<PassName>("\"made_up\"").is_err());
}

#[tokio::test]
async fn injected_transport_is_used_and_provider_shapes_are_exact() {
    let cases = [
        (
            "openai",
            serde_json::json!({"choices":[{"message":{"content":"{\"ok\":true}"}}]}),
        ),
        (
            "anthropic",
            serde_json::json!({"content":[{"text":"{\"ok\":true}"}]}),
        ),
        (
            "google",
            serde_json::json!({"candidates":[{"content":{"parts":[{"text":"{\"ok\":true}"}]}}]}),
        ),
        (
            "ollama",
            serde_json::json!({"message":{"content":"{\"ok\":true}"}}),
        ),
    ];
    for (kind, response) in cases {
        let transport = FakeTransport::with_json([response]);
        let base = if kind == "ollama" {
            "http://127.0.0.1:11434"
        } else {
            "https://example.test"
        };
        let mut configured = profile(kind, kind, base);
        if kind != "ollama" {
            configured = keyed(configured, "fixture-secret");
        }
        let provider: Box<dyn DreamProvider> = match kind {
            "openai" => {
                Box::new(OpenAiProvider::new(transport.clone(), configured, "test-model").unwrap())
            }
            "anthropic" => Box::new(
                AnthropicProvider::new(transport.clone(), configured, "test-model").unwrap(),
            ),
            "google" => Box::new(
                GoogleProvider::new(transport.clone(), configured, "models/test model").unwrap(),
            ),
            "ollama" => {
                Box::new(OllamaProvider::new(transport.clone(), configured, "test-model").unwrap())
            }
            _ => unreachable!(),
        };
        assert_eq!(
            provider
                .run_pass(PassName::CoverageAudit, &context(), &[])
                .await
                .unwrap(),
            serde_json::json!({"ok":true})
        );
        let request = transport.requests().pop().unwrap();
        assert_eq!(request.method(), HttpMethod::Post);
        assert_eq!(
            request.payload().unwrap()["contents"].is_array(),
            kind == "google"
        );
        if kind == "google" {
            assert!(
                request
                    .url()
                    .as_str()
                    .contains("test%20model:generateContent")
            );
        }
        if kind != "ollama" {
            assert!(
                request
                    .headers()
                    .values()
                    .any(|value| value.contains("fixture-secret"))
            );
        }
        assert!(!format!("{request:?}").contains("fixture-secret"));
    }
}

#[tokio::test]
async fn health_requests_apply_google_and_ollama_output_caps_and_ollama_bearer() {
    let google_transport = FakeTransport::with_json([serde_json::json!({})]);
    let google = keyed(profile("google", "google", "https://example.test"), "g-key");
    ProviderRegistry::with_transport(
        google_transport.clone(),
        ModelCache::new(4, 4096, Duration::from_secs(60)),
    )
    .check(&google, "gemini")
    .await
    .unwrap();
    assert_eq!(
        google_transport.requests()[0].payload().unwrap()["generationConfig"]["maxOutputTokens"],
        1
    );

    let ollama_transport = FakeTransport::with_json([serde_json::json!({})]);
    let ollama = keyed(
        profile("ollama", "ollama", "http://127.0.0.1:11434"),
        "local-key",
    );
    ProviderRegistry::with_transport(
        ollama_transport.clone(),
        ModelCache::new(4, 4096, Duration::from_secs(60)),
    )
    .check(&ollama, "llama")
    .await
    .unwrap();
    let request = &ollama_transport.requests()[0];
    assert_eq!(request.payload().unwrap()["options"]["num_predict"], 1);
    assert_eq!(request.headers()["authorization"], "Bearer local-key");
}

#[test]
fn ollama_rejects_dns_names_and_production_transport_ignores_proxy() {
    assert!(matches!(
        profile("bad", "ollama", "http://localhost:11434").validate(),
        Err(ProviderError::UnsafeEndpoint(_))
    ));
    let local = profile("local", "ollama", "http://127.0.0.1:11434");
    assert!(
        ReqwestTransport::for_profile(
            &local,
            &ReqwestTransportOptions {
                trusted_proxy: Some("not a url".into()),
                custom_ca_pem: vec![]
            }
        )
        .is_ok()
    );
    let remote = profile("remote", "openai", "https://example.test");
    assert!(
        ReqwestTransport::for_profile(
            &remote,
            &ReqwestTransportOptions {
                trusted_proxy: Some("not a url".into()),
                custom_ca_pem: vec![]
            }
        )
        .is_err()
    );
}

#[tokio::test]
async fn cache_identity_changes_when_inline_or_file_credential_rotates() {
    let now = Utc::now();
    let transport = FakeTransport::with_json([
        serde_json::json!({"data":[{"id":"one"}]}),
        serde_json::json!({"data":[{"id":"two"}]}),
    ]);
    let registry = ProviderRegistry::with_transport(
        transport.clone(),
        ModelCache::new(8, 4096, Duration::from_secs(60)),
    );
    let first = keyed(
        profile("openai", "openai", "https://example.test/v1"),
        "secret-a",
    );
    assert_eq!(registry.suggest_models(&first, now).await.unwrap(), ["one"]);
    assert_eq!(registry.suggest_models(&first, now).await.unwrap(), ["one"]);
    let second = keyed(
        profile("openai", "openai", "https://example.test/v1"),
        "secret-b",
    );
    assert_eq!(
        registry.suggest_models(&second, now).await.unwrap(),
        ["two"]
    );
    assert_eq!(transport.requests().len(), 2);
}

#[test]
fn model_cache_enforces_serialized_bound_without_poisoning() {
    let now = Utc.with_ymd_and_hms(2026, 7, 19, 1, 0, 0).unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cache.json");
    let cache = ModelCache::new(4, 220, Duration::from_secs(60));
    cache.insert("a", vec!["small".into()], now).unwrap();
    assert!(
        cache
            .insert("oversized", vec!["x".repeat(500)], now)
            .is_err()
    );
    assert_eq!(cache.get("a", now).unwrap(), ["small"]);
    cache.save(&path).unwrap();
    assert!(std::fs::metadata(path).unwrap().len() <= 220);
}

#[test]
fn model_cache_hostile_malformed_and_legacy_files_fail_soft_empty() {
    let now = Utc::now();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cache.json");
    for contents in [
        "x".repeat(10_000),
        "{broken".into(),
        r#"{"providers":{"openai":{"provider":"openai","models":["old"],"fetched_at":"2026-01-01T00:00:00Z","error":"","identity":"old"}}}"#.into(),
    ] {
        std::fs::write(&path, contents).unwrap();
        let cache = ModelCache::load(&path, 2, 256, Duration::from_secs(60), now).unwrap();
        assert!(cache.get("anything", now).is_none());
    }
}

#[tokio::test]
async fn openai_and_anthropic_discovery_paginate_with_auth() {
    for kind in ["openai", "anthropic"] {
        let transport = FakeTransport::with_json([
            serde_json::json!({"data":[{"id":"b"}],"has_more":true,"last_id":"b"}),
            serde_json::json!({"data":[{"id":"a"}],"has_more":false}),
        ]);
        let configured = keyed(profile(kind, kind, "https://example.test/v1"), "secret");
        let models = ProviderRegistry::with_transport(
            transport.clone(),
            ModelCache::new(8, 4096, Duration::from_secs(60)),
        )
        .suggest_models(&configured, Utc::now())
        .await
        .unwrap();
        assert_eq!(models, ["a", "b"]);
        let requests = transport.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url().path(), "/v1/models");
        assert!(requests[1].url().query().unwrap().contains("after=b"));
        assert!(
            requests[0]
                .headers()
                .values()
                .any(|value| value == "secret" || value == "Bearer secret")
        );
    }
}

#[tokio::test]
async fn ollama_discovery_uses_tags_and_optional_bearer() {
    let transport = FakeTransport::with_json([
        serde_json::json!({"models":[{"model":"qwen:latest"},{"model":"llama:latest"}]}),
    ]);
    let configured = keyed(
        profile("ollama", "ollama", "http://127.0.0.1:11434"),
        "local-token",
    );
    let models = ProviderRegistry::with_transport(
        transport.clone(),
        ModelCache::new(8, 4096, Duration::from_secs(60)),
    )
    .suggest_models(&configured, Utc::now())
    .await
    .unwrap();
    assert_eq!(models, ["llama:latest", "qwen:latest"]);
    let requests = transport.requests();
    assert_eq!(requests[0].url().path(), "/api/tags");
    assert_eq!(
        requests[0].headers().get("authorization").unwrap(),
        "Bearer local-token"
    );
}

#[tokio::test]
async fn discovery_stops_at_the_finite_page_cap() {
    let pages = (0..8).map(|page| {
        serde_json::json!({
            "data": [{"id": format!("model-{page}")}],
            "has_more": true,
            "last_id": format!("cursor-{page}")
        })
    });
    let transport = FakeTransport::with_json(pages);
    let configured = keyed(
        profile("openai", "openai", "https://example.test/v1"),
        "secret",
    );
    let result = ProviderRegistry::with_transport(
        transport.clone(),
        ModelCache::new(8, 4096, Duration::from_secs(60)),
    )
    .suggest_models(&configured, Utc::now())
    .await;
    assert!(matches!(result, Err(ProviderError::PaginationLimit)));
    assert_eq!(transport.requests().len(), 8);
}

#[tokio::test]
async fn google_discovery_filters_non_generative_models_and_paginates() {
    let transport = FakeTransport::with_json([
        serde_json::json!({"models":[{"name":"models/embed","supportedGenerationMethods":["embedContent"]},{"name":"models/gemini","supportedGenerationMethods":["generateContent"]}],"nextPageToken":"next token"}),
        serde_json::json!({"models":[{"name":"models/gemini-2","supportedGenerationMethods":["generateContent"]}]}),
    ]);
    let configured = keyed(
        profile("google", "google", "https://example.test"),
        "secret",
    );
    let models = ProviderRegistry::with_transport(
        transport.clone(),
        ModelCache::new(8, 4096, Duration::from_secs(60)),
    )
    .suggest_models(&configured, Utc::now())
    .await
    .unwrap();
    assert_eq!(models, ["gemini", "gemini-2"]);
    assert!(
        transport.requests()[1]
            .url()
            .query()
            .unwrap()
            .contains("pageToken=next+token")
    );
}

#[tokio::test]
async fn discovery_detects_pagination_loop_and_does_not_cache_errors() {
    let transport = FakeTransport::with_json([
        serde_json::json!({"data":[{"id":"a"}],"has_more":true,"last_id":"a"}),
        serde_json::json!({"data":[{"id":"a"}],"has_more":true,"last_id":"a"}),
        serde_json::json!({"data":[{"id":"fresh"}]}),
    ]);
    let configured = keyed(
        profile("openai", "openai", "https://example.test/v1"),
        "secret",
    );
    let registry = ProviderRegistry::with_transport(
        transport.clone(),
        ModelCache::new(8, 4096, Duration::from_secs(60)),
    );
    assert!(matches!(
        registry.suggest_models(&configured, Utc::now()).await,
        Err(ProviderError::PaginationLoop)
    ));
    assert_eq!(
        registry
            .suggest_models(&configured, Utc::now())
            .await
            .unwrap(),
        ["fresh"]
    );
}

#[test]
fn google_model_normalization_rejects_ambiguous_segments() {
    let transport = FakeTransport::with_json([]);
    let configured = keyed(
        profile("google", "google", "https://example.test"),
        "secret",
    );
    for invalid in ["", "models/a/b", "a?b", "a#b"] {
        assert!(GoogleProvider::new(transport.clone(), configured.clone(), invalid).is_err());
    }
}

#[test]
fn catalog_source_contains_unix_and_windows_anchored_backends() {
    let source = include_str!("../src/provider/catalog.rs");
    assert!(source.contains("struct UnixAnchor"));
    assert!(source.contains("struct WindowsAnchor"));
    assert!(source.contains("WINDOWS_SHARE_WITHOUT_DELETE"));
}

fn fake_http(status: u16, body: String, extra_headers: String) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 8192];
        let count = stream.read(&mut buffer).unwrap();
        let _ = sender.send(String::from_utf8_lossy(&buffer[..count]).into_owned());
        let _ = write!(
            stream,
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    });
    (format!("http://{address}"), receiver)
}

#[tokio::test(flavor = "current_thread")]
async fn reqwest_transport_caps_bodies_and_does_not_follow_redirects() {
    let _network = network_test_lock().lock().await;
    let oversized = format!("{{\"x\":\"{}\"}}", "x".repeat(2 * 1024 * 1024));
    let (endpoint, _) = fake_http(200, oversized, String::new());
    let configured = keyed(
        profile("openai", "openai", &format!("{endpoint}/v1")),
        "secret",
    );
    let transport = Arc::new(
        ReqwestTransport::for_profile(&configured, &ReqwestTransportOptions::default()).unwrap(),
    );
    let error = OpenAiProvider::new(transport, configured, "m")
        .unwrap()
        .run_pass(PassName::Concepts, &context(), &[])
        .await
        .unwrap_err();
    assert!(matches!(error, ProviderError::ResponseTooLarge { .. }));

    let (target, target_capture) = fake_http(200, "{}".into(), String::new());
    let (source, _) = fake_http(307, String::new(), format!("Location: {target}\r\n"));
    let configured = keyed(profile("google", "google", &source), "secret");
    let transport = Arc::new(
        ReqwestTransport::for_profile(&configured, &ReqwestTransportOptions::default()).unwrap(),
    );
    let error = GoogleProvider::new(transport, configured, "m")
        .unwrap()
        .run_pass(PassName::Concepts, &context(), &[])
        .await
        .unwrap_err();
    assert!(matches!(error, ProviderError::Http { status: 307 }));
    assert!(
        target_capture
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
}

fn network_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[test]
fn model_cache_round_trip_current_schema() {
    let now = Utc.with_ymd_and_hms(2026, 7, 19, 2, 0, 0).unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cache.json");
    let entries = BTreeMap::from([(
        "identity".into(),
        ModelCacheEntry {
            models: vec!["model".into()],
            cached_at: now,
        },
    )]);
    let cache = ModelCache::from_entries(2, 1024, Duration::from_secs(60), entries).unwrap();
    cache.save(&path).unwrap();
    assert_eq!(
        ModelCache::load(&path, 2, 1024, Duration::from_secs(60), now)
            .unwrap()
            .get("identity", now)
            .unwrap(),
        ["model"]
    );
}
