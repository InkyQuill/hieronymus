use hieronymus::data_root::HieronymusConfig;
use hieronymus::ollama_embeddings::OllamaEmbeddingProvider;
use hieronymus::provider_http::{HttpError, HttpResponse, ProviderTransport};
use hieronymus::semantic_arming::{SemanticConfiguration, load_configuration, save_configuration};
use hieronymus::semantic_embeddings::EmbeddingProvider;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct FixtureTransport {
    requests: Mutex<Vec<Value>>,
    vector: Mutex<Value>,
    digest: Mutex<String>,
}
impl ProviderTransport for FixtureTransport {
    fn get_json(
        &self,
        _: &str,
        _: &[(String, String)],
        _: Duration,
    ) -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse {status:200,body:json!({"models":[{"name":"nomic:latest","model":"nomic:latest","digest":*self.digest.lock().unwrap()}]}).to_string()})
    }
    fn post_json(
        &self,
        _: &str,
        _: &[(String, String)],
        payload: &Value,
        _: Duration,
    ) -> Result<HttpResponse, HttpError> {
        self.requests.lock().unwrap().push(payload.clone());
        Ok(HttpResponse {
            status: 200,
            body: json!({"embeddings":*self.vector.lock().unwrap()}).to_string(),
        })
    }
}
fn transport(vector: Value) -> Arc<FixtureTransport> {
    Arc::new(FixtureTransport {
        requests: Mutex::new(vec![]),
        vector: Mutex::new(vector),
        digest: Mutex::new("a".repeat(64)),
    })
}
#[test]
fn exact_text_and_immutable_identity() {
    let wire = transport(json!([[0.6, 0.8]]));
    let mut provider = OllamaEmbeddingProvider::with_transport(
        "http://127.0.0.1:11434",
        "nomic:latest",
        wire.clone(),
    )
    .unwrap();
    provider.embed_document_text("雪 — Ёж\n", &[1, 2]).unwrap();
    provider.embed_query_text("¿Dónde?", &[3]).unwrap();
    let requests = wire.requests.lock().unwrap();
    assert_eq!(
        requests[1],
        json!({"model":"nomic:latest","input":"雪 — Ёж\n","truncate":false})
    );
    assert_eq!(requests[2]["input"], "¿Dónde?");
    assert_eq!(provider.identity().dimensions(), 2);
    drop(requests);
    *wire.digest.lock().unwrap() = "b".repeat(64);
    assert!(provider.embed_query_text("changed", &[3]).is_err());
}
#[test]
fn invalid_vectors_never_arm() {
    for vector in [
        json!([]),
        json!([[]]),
        json!([[1, 2], [1, 2]]),
        json!([[0, 0]]),
        json!([[1e300, 1]]),
    ] {
        assert!(
            OllamaEmbeddingProvider::with_transport(
                "http://localhost:11434",
                "nomic:latest",
                transport(vector)
            )
            .is_err()
        );
    }
}
#[test]
fn configuration_is_backward_compatible_and_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(dir.path().join("root"));
    std::fs::create_dir_all(config.config_root()).unwrap();
    std::fs::write(
        hieronymus::semantic_arming::semantic_config_path(&config),
        toml::to_string(&std::collections::BTreeMap::from([(
            "runtime_library",
            dir.path().join("old.so").to_str().unwrap(),
        )]))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        load_configuration(&config).unwrap().unwrap().provider,
        "onnx"
    );
    let settings = SemanticConfiguration::ollama("http://localhost:11434", "nomic:latest");
    save_configuration(&config, &settings).unwrap();
    let loaded = load_configuration(&config).unwrap().unwrap();
    assert_eq!(loaded.provider, "ollama");
    assert_eq!(loaded.configuration_revision, 1);
    for url in [
        "ftp://localhost",
        "http://user:secret@localhost",
        "http://localhost/?key=secret",
    ] {
        assert!(
            save_configuration(&config, &SemanticConfiguration::ollama(url, "nomic:latest"))
                .is_err()
        );
        assert_eq!(load_configuration(&config).unwrap().unwrap(), loaded);
    }
}

#[path = "support/current_story.rs"]
mod current_story;

#[test]
fn index_query_restart_and_dimension_identity_are_coherent() {
    use hieronymus::{
        rag::{RagImport, RagStore},
        registry::Registry,
        semantic_jobs::{JobOutcome, RebuildConfig, RebuildInputs, SemanticJobStore},
        semantic_recall::SemanticLane,
        semantic_store::{SemanticSample, SemanticStore},
        semantic_tokenizer::ModelTokenizer,
    };
    let dir = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(dir.path().join("root"));
    Registry::open(&config)
        .unwrap()
        .create_series("demo", "Demo", "ja", "en", None)
        .unwrap();
    current_story::register(&config, "demo");
    let source = "雪の城 — The snow castle remembers. Ёж";
    let path = dir.path().join("source.txt");
    std::fs::write(&path, source).unwrap();
    RagStore::open(&config)
        .unwrap()
        .import_file("demo", &path, &RagImport::new())
        .unwrap();
    let second = dir.path().join("second.txt");
    std::fs::write(&second, "The distant river thaws.").unwrap();
    RagStore::open(&config)
        .unwrap()
        .import_file("demo", &second, &RagImport::new())
        .unwrap();
    current_story::capture_chunks(&config, "demo");
    let tokenizer =
        || ModelTokenizer::from_bytes(include_bytes!("fixtures/minilm-tokenizer.json")).unwrap();
    let wire = transport(json!([[0.6, 0.8]]));
    let make_provider = || {
        OllamaEmbeddingProvider::with_transport(
            "http://localhost:11434",
            "nomic:latest",
            wire.clone(),
        )
        .unwrap()
    };
    let mut provider = make_provider();
    let store = SemanticStore::open(&config).unwrap();
    store
        .begin_generation("ollama-a", provider.identity())
        .unwrap();
    assert_eq!(store.pending_chunk_ids("ollama-a", 10).unwrap().len(), 2);
    let jobs = SemanticJobStore::open(&config).unwrap();
    let job = jobs
        .enqueue_rebuild("ollama-a", provider.identity())
        .unwrap();
    let outcome = jobs
        .run_rebuild(
            &job.job_id,
            RebuildInputs {
                provider: &mut provider,
                tokenizer: &mut tokenizer(),
                sample: SemanticSample {
                    text: source.into(),
                    series_slug: "demo".into(),
                    token_ids: tokenizer().encode(source).unwrap(),
                },
            },
            &RebuildConfig::default(),
        )
        .unwrap();
    assert!(matches!(outcome, JobOutcome::Completed { .. }));
    let context = current_story::context("demo", "ja", "en", "translation");
    for _ in 0..2 {
        let lane = SemanticLane::new(Box::new(make_provider()), Box::new(tokenizer()));
        let result = lane.run(&config, &context, "Forgotten winter fortress?", 5);
        assert!(!result.degraded, "{:?}", result.warnings);
        assert!(!result.records.is_empty());
    }
    assert!(
        wire.requests
            .lock()
            .unwrap()
            .iter()
            .any(|p| p["input"] == source)
    );
    assert!(
        wire.requests
            .lock()
            .unwrap()
            .iter()
            .any(|p| p["input"] == "Forgotten winter fortress?")
    );
    let other = OllamaEmbeddingProvider::with_transport(
        "http://localhost:11434",
        "nomic:latest",
        transport(json!([[0.6, 0.8, 0.1]])),
    )
    .unwrap();
    assert_ne!(provider.identity(), other.identity());
    let lane = SemanticLane::new(Box::new(other), Box::new(tokenizer()));
    assert!(lane.run(&config, &context, "winter", 5).degraded);
    *wire.digest.lock().unwrap() = "b".repeat(64);
    let lane = SemanticLane::new(Box::new(make_provider()), Box::new(tokenizer()));
    assert!(lane.run(&config, &context, "winter", 5).degraded);
}

#[test]
fn production_http_sends_exact_unicode_and_truncate_false() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut inputs = vec![];
        for _ in 0..9 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = vec![];
            let mut byte = [0];
            while !bytes.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
            }
            let head = String::from_utf8(bytes).unwrap();
            let body = if head.starts_with("GET /api/tags ") {
                json!({"models":[{"name":"nomic:latest","digest":"a".repeat(64)}]})
            } else {
                assert!(head.starts_with("POST /api/embed "));
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                let mut bytes = vec![0; length];
                socket.read_exact(&mut bytes).unwrap();
                let payload: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(payload["model"], "nomic:latest");
                assert_eq!(payload["truncate"], false);
                inputs.push(payload["input"].as_str().unwrap().to_owned());
                json!({"embeddings":[[0.6,0.8]]})
            }
            .to_string();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
        inputs
    });
    let mut provider = OllamaEmbeddingProvider::load(&url, "nomic:latest").unwrap();
    provider.embed_document_text("雪 — Ёж\n", &[1]).unwrap();
    provider.embed_query_text("¿Dónde?", &[2]).unwrap();
    let inputs = server.join().unwrap();
    assert_eq!(&inputs[1..], ["雪 — Ёж\n", "¿Dónde?"]);
}

struct InvalidResponse {
    status: u16,
    body: String,
    timeout: bool,
}
impl ProviderTransport for InvalidResponse {
    fn get_json(
        &self,
        _: &str,
        _: &[(String, String)],
        _: Duration,
    ) -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse {
            status: 200,
            body: json!({"models":[{"name":"nomic:latest","digest":"a".repeat(64)}]}).to_string(),
        })
    }
    fn post_json(
        &self,
        _: &str,
        _: &[(String, String)],
        _: &Value,
        _: Duration,
    ) -> Result<HttpResponse, HttpError> {
        if self.timeout {
            return Err(HttpError::Timeout { millis: 30_000 });
        }
        Ok(HttpResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
}
#[test]
fn malformed_oversized_failed_and_timed_out_responses_fail_closed() {
    for response in [
        InvalidResponse {
            status: 500,
            body: "server failed".into(),
            timeout: false,
        },
        InvalidResponse {
            status: 200,
            body: "{".into(),
            timeout: false,
        },
        InvalidResponse {
            status: 200,
            body: "{\"embeddings\":[[NaN,Infinity]]}".into(),
            timeout: false,
        },
        InvalidResponse {
            status: 200,
            body: " ".repeat(1024 * 1024 + 1),
            timeout: false,
        },
        InvalidResponse {
            status: 200,
            body: "{}".into(),
            timeout: false,
        },
        InvalidResponse {
            status: 200,
            body: "{\"embeddings\":[[null,true]]}".into(),
            timeout: false,
        },
        InvalidResponse {
            status: 200,
            body: "".into(),
            timeout: true,
        },
    ] {
        assert!(
            OllamaEmbeddingProvider::with_transport(
                "http://localhost:11434",
                "nomic:latest",
                Arc::new(response)
            )
            .is_err()
        );
    }
}
#[test]
fn input_limits_and_live_width_changes_are_errors() {
    let wire = transport(json!([[0.6, 0.8]]));
    let mut provider = OllamaEmbeddingProvider::with_transport(
        "http://localhost:11434",
        "nomic:latest",
        wire.clone(),
    )
    .unwrap();
    assert!(provider.embed_document(&[123]).is_err());
    assert!(provider.embed_query_text("", &[]).is_err());
    assert!(
        provider
            .embed_document_text(&"x".repeat(65537), &[123])
            .is_err()
    );
    assert_eq!(wire.requests.lock().unwrap().len(), 1);
    *wire.vector.lock().unwrap() = json!([[0.6, 0.8, 0.1]]);
    let error = provider
        .embed_query_text("changed", &[1])
        .unwrap_err()
        .to_string();
    assert!(error.contains("dimensions changed"), "{error}");
}
