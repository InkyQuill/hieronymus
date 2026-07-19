use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use chrono::{TimeZone, Utc};
use hiero_core::provider::{
    AnthropicProvider, CandidateCrystal, CredentialSource, DreamOutput, DreamProvider,
    GoogleProvider, ModelCache, ModelCacheEntry, OllamaProvider, OpenAiProvider, ProviderCatalog,
    ProviderError, ProviderProfile, ProviderRegistry,
};
use secrecy::ExposeSecret;

fn profile(id: &str, provider: &str) -> ProviderProfile {
    ProviderProfile::new(id, provider, "test-model")
        .with_base_url("http://127.0.0.1:11434")
        .with_timeout(Duration::from_secs(2))
}

#[test]
fn profile_debug_and_serialization_never_disclose_credentials() {
    let profile =
        profile("openai", "openai").with_credential_source(CredentialSource::Environment {
            variable: "HIERONYMUS_TEST_KEY".into(),
        });
    let debug = format!("{profile:?}");
    assert!(!debug.contains("secret-value"));
    assert!(!toml::to_string(&profile).unwrap().contains("secret-value"));
}

#[test]
fn catalog_round_trips_strict_profiles_and_supports_mutation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("provider.conf");
    let mut catalog = ProviderCatalog::default();
    catalog.upsert(profile("local", "ollama")).unwrap();
    catalog.save(&path).unwrap();

    let mut loaded = ProviderCatalog::load(&path).unwrap();
    assert_eq!(loaded.get("local").unwrap().model(), "test-model");
    assert!(loaded.delete("local"));
    assert!(loaded.get("local").is_none());
}

#[test]
fn catalog_rejects_unknown_fields_and_symlink_destinations() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("provider.conf");
    std::fs::write(
        &path,
        "[providers.bad]\nid='bad'\nprovider='openai'\nmodel='m'\nsurprise=true\n",
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
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "keep");
    }
}

#[test]
fn credential_sources_resolve_only_explicit_environment_or_files() {
    let root = tempfile::tempdir().unwrap();
    let key_file = root.path().join("key");
    std::fs::write(&key_file, " file-secret \n").unwrap();
    let secret = CredentialSource::File { path: key_file }.resolve().unwrap();
    assert_eq!(secret.expose_secret(), "file-secret");
    assert!(!format!("{secret:?}").contains("file-secret"));
}

#[test]
fn model_cache_is_deterministic_bounded_and_ttl_aware() {
    let now = Utc.with_ymd_and_hms(2026, 7, 19, 1, 0, 0).unwrap();
    let cache = ModelCache::new(2, 64, Duration::from_secs(60));
    cache
        .insert("b", vec!["z".into(), "a".into()], now)
        .unwrap();
    cache.insert("a", vec!["m".into()], now).unwrap();
    cache.insert("c", vec!["n".into()], now).unwrap();
    assert_eq!(cache.get("a", now), None);
    assert_eq!(cache.get("b", now).unwrap(), &["a", "z"]);
    assert_eq!(cache.get("c", now + chrono::Duration::seconds(60)), None);
}

#[test]
fn model_cache_load_discards_oversized_and_expired_entries() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("llm-cache.json");
    let now = Utc.with_ymd_and_hms(2026, 7, 19, 2, 0, 0).unwrap();
    let entries = BTreeMap::from([(
        "one".into(),
        ModelCacheEntry {
            models: vec!["model".into()],
            cached_at: now,
        },
    )]);
    let cache = ModelCache::from_entries(2, 1024, Duration::from_secs(60), entries).unwrap();
    cache.save(&path).unwrap();
    let loaded = ModelCache::load(&path, 2, 1024, Duration::from_secs(60), now).unwrap();
    assert_eq!(loaded.get("one", now).unwrap(), &["model"]);
}

#[test]
fn candidate_output_schema_is_strict() {
    let output = DreamOutput {
        crystals: vec![CandidateCrystal {
            crystal_type: "rule".into(),
            title: "title".into(),
            text: "text".into(),
            source_credibility: "explicit_rule".into(),
            rule_intent: "must".into(),
            confidence: 0.9,
        }],
        concepts: vec![],
    };
    assert_eq!(
        serde_json::to_value(output).unwrap()["crystals"][0]["title"],
        "title"
    );
}

#[test]
fn ollama_profile_rejects_non_loopback_urls() {
    let error = profile("bad", "ollama")
        .with_base_url("http://example.com:11434")
        .validate()
        .unwrap_err();
    assert!(matches!(error, ProviderError::UnsafeEndpoint(_)));
}

#[test]
fn registry_holds_heterogeneous_object_safe_providers() {
    fn assert_send_sync(_: Arc<dyn hiero_core::provider::DreamProvider>) {}
    let _ = assert_send_sync;
}

fn fake_http(status: u16, body: &str, delay: Duration) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    let body = body.to_owned();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            request.extend_from_slice(&buffer[..count]);
            let header_end = request.windows(4).position(|window| window == b"\r\n\r\n");
            if let Some(header_end) = header_end {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        let _ = sender.send(String::from_utf8_lossy(&request).into_owned());
        thread::sleep(delay);
        let reason = if status == 200 { "OK" } else { "Error" };
        write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    (format!("http://{address}"), receiver)
}

fn fake_redirect(location: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let location = location.to_owned();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 4096];
        let _ = stream.read(&mut buffer);
        write!(stream, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    format!("http://{address}")
}

fn network_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn test_context() -> hiero_core::domain::TranslationContext {
    hiero_core::domain::TranslationContext::new("series", "en", "ru")
}

fn file_credential(root: &tempfile::TempDir) -> CredentialSource {
    let path = root.path().join("credential");
    std::fs::write(&path, "fixture-secret").unwrap();
    CredentialSource::File { path }
}

#[tokio::test(flavor = "current_thread")]
async fn adapters_preserve_provider_specific_headers_requests_and_envelopes() {
    let _network = network_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    let cases = [
        (
            "openai",
            "/v1/chat/completions",
            "authorization: Bearer fixture-secret",
            r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
        ),
        (
            "anthropic",
            "/v1/messages",
            "anthropic-version: 2023-06-01",
            r#"{"content":[{"text":"{\"ok\":true}"}]}"#,
        ),
        (
            "google",
            "/v1beta/models/test-model:generateContent",
            "x-goog-api-key: fixture-secret",
            r#"{"candidates":[{"content":{"parts":[{"text":"{\"ok\":true}"}]}}]}"#,
        ),
        (
            "ollama",
            "/api/chat",
            "content-type: application/json",
            r#"{"message":{"content":"{\"ok\":true}"}}"#,
        ),
    ];
    for (kind, path, header, response) in cases {
        let (endpoint, captured) = fake_http(200, response, Duration::ZERO);
        let base = if kind == "openai" {
            format!("{endpoint}/v1")
        } else {
            endpoint
        };
        let mut configured = ProviderProfile::new(kind, kind, "test-model").with_base_url(base);
        if kind != "ollama" {
            configured = configured.with_credential_source(file_credential(&root));
        }
        let provider: Box<dyn DreamProvider> = match kind {
            "openai" => Box::new(OpenAiProvider::new(reqwest::Client::new(), configured).unwrap()),
            "anthropic" => {
                Box::new(AnthropicProvider::new(reqwest::Client::new(), configured).unwrap())
            }
            "google" => Box::new(GoogleProvider::new(reqwest::Client::new(), configured).unwrap()),
            "ollama" => Box::new(OllamaProvider::new(reqwest::Client::new(), configured).unwrap()),
            _ => unreachable!(),
        };
        assert_eq!(
            provider
                .run_pass("coverage", &test_context(), &[])
                .await
                .unwrap(),
            serde_json::json!({"ok":true})
        );
        let request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            request.starts_with(&format!("POST {path} HTTP/1.1")),
            "{request}"
        );
        assert!(request.contains(header), "{request}");
        assert!(
            kind == "google" || request.contains("\"model\":\"test-model\""),
            "{request}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn adapter_classifies_http_malformed_json_and_timeout_without_leaking_body() {
    let _network = network_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    for (status, body, delay, expected) in [
        (
            429,
            "fixture-secret upstream",
            Duration::ZERO,
            "provider returned HTTP 429",
        ),
        (
            200,
            "not-json",
            Duration::ZERO,
            "provider returned malformed JSON",
        ),
        (
            200,
            "{}",
            Duration::from_millis(100),
            "provider request timed out",
        ),
    ] {
        let (endpoint, _) = fake_http(status, body, delay);
        let timeout = if delay.is_zero() {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(20)
        };
        let profile = ProviderProfile::new("openai", "openai", "m")
            .with_base_url(format!("{endpoint}/v1"))
            .with_credential_source(file_credential(&root))
            .with_timeout(timeout);
        let provider = OpenAiProvider::new(reqwest::Client::new(), profile).unwrap();
        let error = provider
            .run_pass("pass", &test_context(), &[])
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), expected);
        assert!(!format!("{error:?}").contains("fixture-secret"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn registry_model_suggestions_are_sorted_and_cached_without_second_request() {
    let _network = network_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    let (endpoint, captured) =
        fake_http(200, r#"{"data":[{"id":"z"},{"id":"a"}]}"#, Duration::ZERO);
    let profile = ProviderProfile::new("openai", "openai", "m")
        .with_base_url(format!("{endpoint}/v1"))
        .with_credential_source(file_credential(&root));
    let now = Utc::now();
    let registry = ProviderRegistry::new(
        reqwest::Client::new(),
        ModelCache::new(8, 1024, Duration::from_secs(60)),
    );
    assert_eq!(
        registry.suggest_models(&profile, now).await.unwrap(),
        vec!["a", "z"]
    );
    captured.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(
        registry.suggest_models(&profile, now).await.unwrap(),
        vec!["a", "z"]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn oversized_response_is_rejected_before_json_parsing() {
    let _network = network_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    let oversized = format!("{{\"padding\":\"{}\"}}", "x".repeat(2 * 1024 * 1024));
    let (endpoint, _) = fake_http(200, &oversized, Duration::ZERO);
    let profile = ProviderProfile::new("openai", "openai", "m")
        .with_base_url(format!("{endpoint}/v1"))
        .with_credential_source(file_credential(&root));
    let error = OpenAiProvider::new(reqwest::Client::new(), profile)
        .unwrap()
        .run_pass("pass", &test_context(), &[])
        .await
        .unwrap_err();
    assert!(matches!(error, ProviderError::ResponseTooLarge { .. }));
}

#[tokio::test(flavor = "current_thread")]
async fn cross_host_redirect_is_not_followed_with_custom_api_key_header() {
    let _network = network_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    let (target, target_capture) = fake_http(200, r#"{"candidates":[]}"#, Duration::ZERO);
    let source = fake_redirect(&target);
    let profile = ProviderProfile::new("google", "google", "m")
        .with_base_url(source)
        .with_credential_source(file_credential(&root));
    let error = GoogleProvider::new(reqwest::Client::new(), profile)
        .unwrap()
        .run_pass("pass", &test_context(), &[])
        .await
        .unwrap_err();
    assert!(matches!(error, ProviderError::Http { status: 307 }));
    assert!(
        target_capture
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn ollama_v1_endpoint_uses_openai_compatibility_shape_without_configured_key() {
    let _network = network_test_lock().lock().await;
    let (endpoint, captured) = fake_http(
        200,
        r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
        Duration::ZERO,
    );
    let profile = ProviderProfile::new("ollama-openai", "ollama", "local-model")
        .with_base_url(format!("{endpoint}/v1"));
    let provider = OllamaProvider::new(reqwest::Client::new(), profile).unwrap();
    assert_eq!(
        provider
            .run_pass("pass", &test_context(), &[])
            .await
            .unwrap(),
        serde_json::json!({"ok": true})
    );
    let request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(request.contains("authorization: Bearer ollama"));
}

#[cfg(unix)]
#[test]
fn catalog_rejects_fifo_without_blocking() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("provider.conf");
    let status = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let started = std::time::Instant::now();
    assert!(ProviderCatalog::load(&path).is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn catalog_source_contains_both_unix_and_windows_anchored_backends() {
    let source = include_str!("../src/provider/catalog.rs");
    assert!(source.contains("struct UnixAnchor"));
    assert!(source.contains("struct WindowsAnchor"));
    assert!(source.contains("WINDOWS_SHARE_WITHOUT_DELETE"));
    assert!(source.contains("WINDOWS_OPEN_REPARSE_POINT"));
}
