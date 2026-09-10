//! Task S3: the REAL semantic qualification suite. Unlike
//! `semantic_execution.rs` (scripted fake providers), everything here runs
//! the pinned multilingual MiniLM ONNX model through the production `OnnxArm`
//! and its Unigram tokenizer, driven only through normal authenticated
//! MCP operations and the real `hiero semantic enable` CLI.
//!
//! `#[ignore]`d because it needs two explicit filesystem assets:
//!
//! - `HIERO_TEST_ONNX_RUNTIME`: the onnxruntime shared library
//!   (`libonnxruntime.so`, qualified identity in
//!   `qualification/prerequisites.json`).
//! - `HIERO_TEST_MODEL_DIR`: a directory holding the pinned
//!   `model.onnx` and `tokenizer.json` (SHA-256 pins in
//!   `hieronymus::semantic_model`).
//!
//! When either asset is requested but absent the test FAILS (it never skips
//! and never passes vacuously). Run:
//!
//! ```text
//! HIERO_TEST_ONNX_RUNTIME=<...>/libonnxruntime.so \
//! HIERO_TEST_MODEL_DIR=<...>/paraphrase-multilingual-MiniLM-L12-v2 \
//! cargo test -p hiero --test semantic_real -- --ignored --nocapture
//! ```
//!
//! Everything corpus-driven runs as one sequenced `#[test]` on purpose: ort
//! keeps the loaded ONNX Runtime dylib in a process-global `OnceLock`, and a
//! FAILED dynamic load poisons every later ort use in the same process. The
//! healthy daemon therefore performs the single successful in-process load;
//! the corrupt-model daemon fails at the model checksum before any dylib is
//! touched; and the corrupt-runtime daemon runs as a real `hiero daemon`
//! SUBPROCESS so its failed load can never be masked.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use hiero::daemon::semantic_worker::{RequiredSemanticState, require_semantic_ready};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::recall::WARNING_SEMANTIC_UNAVAILABLE;
use hieronymus::semantic_embeddings::{
    EMBEDDING_DIMENSIONS, EmbeddingIdentity, EmbeddingProvider, OnnxEmbeddingProvider,
};
use hieronymus::semantic_store::SemanticStore;
use serde::Deserialize;
use serde_json::{Value, json};

use common::{mcp_headers, send_request, start_daemon, wait_until};

const FIXTURE: &str = include_str!("fixtures/hybrid-relevance.json");

/// `ort` keeps the loaded dylib in a process-global `OnceLock`, so the whole
/// qualification runs as one sequenced test (see the module docs).
const RAG_SEMANTIC_REASON: &str = "rag semantic match";
const LEGACY_ENGLISH_TOKENIZER_ID: &str = "wordpiece-minilm-l6-v2@sha256:be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037:max-256:longest-first:bert-normalize:cls-sep";
const SHORT_TERM_REASON: &str = "active session short-term memory match";

// ------------------------------------------------------------- typed fixture

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    adr: String,
    purpose: String,
    series: Vec<SeriesSpec>,
    documents: Vec<DocumentSpec>,
    memories: Vec<MemorySpec>,
    terms: Vec<TermSpec>,
    queries: Vec<QuerySpec>,
}

#[derive(Debug, Deserialize)]
struct SeriesSpec {
    slug: String,
    title: String,
    source_language: String,
    target_language: String,
}

#[derive(Debug, Deserialize)]
struct DocumentSpec {
    series_slug: String,
    doc_id: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct MemorySpec {
    series_slug: String,
    kind: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct TermSpec {
    series_slug: String,
    category: String,
    source_text: String,
    canonical_translation: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuerySpec {
    language: String,
    context: QueryContext,
    excluded_doc_ids: Vec<String>,
    query_id: String,
    query: String,
    series_slug: String,
    #[serde(default)]
    expected_top3_doc_ids: Vec<String>,
    #[serde(default)]
    require_semantic_provenance: bool,
    #[serde(default)]
    excluded_series: Vec<String>,
    #[serde(default)]
    expected_memory_text: Option<String>,
    #[serde(default)]
    expected_contract_term: Option<ContractExpectation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryContext {
    viewpoint: Option<String>,
    story_position: Option<String>,
    acceptance: String,
}

#[derive(Debug, Deserialize)]
struct ContractExpectation {
    source_text: String,
    canonical_translation: String,
}

#[test]
fn relevance_context_is_explicit_and_unknown_fields_are_rejected() {
    let mut fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let parsed: Fixture = serde_json::from_value(fixture.clone()).unwrap();
    for language in ["en", "ja", "ru"] {
        assert!(
            parsed
                .queries
                .iter()
                .any(|query| query.language == language)
        );
    }
    fixture["queries"][0]["context"]["unknown_viewpoint_hint"] = json!("future");
    assert!(serde_json::from_value::<Fixture>(fixture).is_err());
}

// ------------------------------------------------------------------- assets

/// A required asset path from the environment; absent assets fail the test.
fn required_asset(env_name: &str) -> PathBuf {
    let value = std::env::var(env_name).unwrap_or_else(|_| {
        panic!(
            "{env_name} must point at the real asset for the real semantic \
             suite; this test fails rather than skipping"
        )
    });
    let path = PathBuf::from(value);
    assert!(
        path.exists(),
        "{env_name}={} does not exist on disk",
        path.display()
    );
    path
}

fn onnx_runtime_path() -> PathBuf {
    required_asset("HIERO_TEST_ONNX_RUNTIME")
}

fn model_dir() -> PathBuf {
    let dir = required_asset("HIERO_TEST_MODEL_DIR");
    for file in ["model.onnx", "tokenizer.json"] {
        assert!(
            dir.join(file).is_file(),
            "HIERO_TEST_MODEL_DIR={} is missing {file}",
            dir.display()
        );
    }
    dir
}

/// Stages the pinned model and tokenizer assets into an isolated data root
/// exactly the way a caller managing its own artifacts would; every later
/// step (CLI enable, daemon arming) still verifies the SHA-256 pins.
fn stage_assets(root: &Path, model_source: &Path) {
    let config = HieronymusConfig::new(root);
    let model_destination = SemanticStore::model_path_for(&config);
    std::fs::create_dir_all(model_destination.parent().unwrap()).unwrap();
    std::fs::copy(model_source.join("model.onnx"), &model_destination)
        .unwrap_or_else(|_| panic!("model copy from {}", model_source.display()));
    std::fs::copy(
        model_source.join("tokenizer.json"),
        SemanticStore::tokenizer_path_for(&config),
    )
    .unwrap_or_else(|_| panic!("tokenizer copy from {}", model_source.display()));
}

/// The real CLI: `hiero semantic enable --json --data-root <root> --runtime
/// <lib>`. A separate process, so the daemon's in-process load verdicts are
/// never masked by this run.
fn semantic_enable(root: &Path, runtime: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "semantic",
            "enable",
            "--json",
            "--data-root",
            &root.display().to_string(),
            "--runtime",
            &runtime.display().to_string(),
        ])
        .output()
        .expect("the built hiero binary must run");
    assert!(
        output.status.success(),
        "hiero semantic enable failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str(&String::from_utf8(output.stdout).unwrap())
        .expect("enable --json prints one JSON payload")
}

// ------------------------------------------------------------ HTTP helpers

fn tools_call(id: i64, name: &str, arguments: Value) -> Value {
    json!({
        "id": id,
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {
                    "name": "semantic-real",
                    "version": "1.0.0"
                },
                "io.modelcontextprotocol/protocolVersion": common::PROTOCOL_REVISION
            },
            "arguments": arguments,
            "name": name
        }
    })
}

fn call_tool(daemon: &hiero::daemon::Daemon, id: i64, name: &str, arguments: Value) -> Value {
    let started = std::time::Instant::now();
    let response = send_request(
        daemon.local_addr().port(),
        "POST",
        "/mcp",
        &mcp_headers(daemon, &[("Mcp-Method", "tools/call"), ("Mcp-Name", name)]),
        &serde_json::to_vec(&tools_call(id, name, arguments)).unwrap(),
    );
    assert_eq!(response.status, 200, "{:?}", response.raw_body);
    eprintln!(
        "P2 latency tool={name} ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let body = response.body();
    assert!(
        body["result"].get("isError").and_then(Value::as_bool) != Some(true),
        "tool {name} failed: {body}"
    );
    serde_json::from_str::<Value>(body["result"]["content"][0]["text"].as_str().unwrap())
        .expect("tool payload is JSON")
}

fn status_body(daemon: &hiero::daemon::Daemon) -> Value {
    let response = send_request(
        daemon.local_addr().port(),
        "GET",
        "/status",
        &[(
            "Authorization".to_string(),
            format!("Bearer {}", daemon.bearer().expose_secret()),
        )],
        b"",
    );
    assert_eq!(response.status, 200, "{:?}", response.raw_body);
    response.body()
}

#[derive(Debug, Deserialize)]
struct StatusDto {
    semantic: SemanticStateDto,
}

#[derive(Debug, Deserialize)]
struct SemanticStateDto {
    state: String,
    detail: Option<String>,
}

fn semantic_state(daemon: &hiero::daemon::Daemon) -> SemanticStateDto {
    serde_json::from_value::<StatusDto>(status_body(daemon))
        .unwrap()
        .semantic
}

fn wait_for_state(daemon: &hiero::daemon::Daemon, wanted: &str) -> SemanticStateDto {
    assert!(
        wait_until(
            || semantic_state(daemon).state == wanted,
            Duration::from_secs(120)
        ),
        "semantic state never reached {wanted:?}: last seen {:?}",
        semantic_state(daemon)
    );
    semantic_state(daemon)
}

fn recall(
    daemon: &hiero::daemon::Daemon,
    id: &mut i64,
    session_id: i64,
    series_slug: &str,
    query: &str,
) -> Value {
    *id += 1;
    call_tool(
        daemon,
        *id,
        "hieronymus_recall",
        json!({
            "session_id": session_id,
            "series_slug": series_slug,
            "query": query,
            "limit": 8,
        }),
    )
}

/// A tool call whose failure is the assertion: returns the tool-error text.
fn call_tool_error(
    daemon: &hiero::daemon::Daemon,
    id: &mut i64,
    name: &str,
    arguments: Value,
) -> String {
    *id += 1;
    let response = send_request(
        daemon.local_addr().port(),
        "POST",
        "/mcp",
        &mcp_headers(daemon, &[("Mcp-Method", "tools/call"), ("Mcp-Name", name)]),
        &serde_json::to_vec(&tools_call(*id, name, arguments)).unwrap(),
    );
    assert_eq!(response.status, 200, "{:?}", response.raw_body);
    let body = response.body();
    assert_eq!(
        body["result"]["isError"],
        json!(true),
        "tool {name} was expected to fail: {body}"
    );
    body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The public semantic RAG search (task C5): a bare row array, no session.
fn rag_search(
    daemon: &hiero::daemon::Daemon,
    id: &mut i64,
    series_slug: &str,
    query: &str,
) -> Vec<Value> {
    *id += 1;
    let payload = call_tool(
        daemon,
        *id,
        "hieronymus_rag_search",
        json!({"series_slug": series_slug, "query": query, "limit": 8}),
    );
    payload
        .as_array()
        .unwrap_or_else(|| panic!("rag search answers a bare row array: {payload}"))
        .clone()
}

/// The RAG rows of a recall payload, in ranked order (rank 1 first).
fn rag_rows(payload: &Value) -> Vec<&Value> {
    let mut rows: Vec<&Value> = payload["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row.get("chunk_kind").is_some())
        .collect();
    rows.sort_by_key(|row| row["rank"].as_i64().unwrap_or(i64::MAX));
    rows
}

fn doc_id_of<'a>(row: &'a Value, documents: &'a [DocumentSpec]) -> Option<&'a str> {
    let source_ref = row["source_ref"].as_str().unwrap_or_default();
    documents
        .iter()
        .find(|document| source_ref.contains(&format!("{}.txt", document.doc_id)))
        .map(|document| document.doc_id.as_str())
}

// ------------------------------------------------------------------ the test

#[test]
#[ignore = "needs the real pinned ONNX assets; see the module docs"]
fn real_semantic_qualification() {
    let runtime = onnx_runtime_path();
    let models = model_dir();
    let fixture: Fixture = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture.adr, "0011");
    assert!(!fixture.purpose.is_empty());

    // -- 1. A corrupt runtime fails readiness, honestly ------------------
    // This scenario's daemon runs as a real `hiero daemon` SUBPROCESS: ort
    // keeps the loaded dylib in a process-global `OnceLock`, and a FAILED
    // dynamic load poisons every later ort use in the same process (the
    // healthy daemon below must load the real runtime in-process). The
    // subprocess keeps both verdicts honest.
    {
        let root = tempfile::tempdir().unwrap();
        stage_assets(root.path(), &models);
        let junk = root.path().join("libonnxruntime.so");
        std::fs::write(&junk, b"this is not an ELF shared object").unwrap();
        let port = free_port();
        let mut child = Command::new(env!("CARGO_BIN_EXE_hiero"))
            .args([
                "daemon",
                "--data-root",
                &root.path().display().to_string(),
                "--port",
                &port.to_string(),
            ])
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the built hiero binary must run");
        let token_path = root.path().join("daemon.token");
        assert!(
            wait_until(|| token_path.is_file(), Duration::from_secs(30)),
            "the daemon subprocess never published its bearer token"
        );
        let bearer = std::fs::read_to_string(&token_path)
            .unwrap()
            .trim()
            .to_string();
        let enable = semantic_enable(root.path(), &junk);
        assert_eq!(enable["lane"], json!("disarmed"), "{enable}");
        assert!(
            enable["reason"]
                .as_str()
                .unwrap()
                .contains("restart-required"),
            "{enable}"
        );
        // A valid path after the failed native load cannot silently reuse the
        // process-global poisoned library. The same daemon must request restart.
        let repaired = semantic_enable(root.path(), &runtime);
        assert_eq!(repaired["lane"], json!("disarmed"), "{repaired}");
        assert!(
            repaired["reason"]
                .as_str()
                .unwrap()
                .contains("restart-required"),
            "{repaired}"
        );
        let failed = wait_until(
            || {
                let response = send_request(
                    port,
                    "GET",
                    "/status",
                    &[("Authorization".to_string(), format!("Bearer {bearer}"))],
                    b"",
                );
                response.status == 200
                    && serde_json::from_value::<StatusDto>(response.body())
                        .map(|dto| dto.semantic.state == "failed")
                        .unwrap_or(false)
            },
            Duration::from_secs(120),
        );
        assert!(
            failed,
            "a corrupt runtime must drive the daemon's semantic state to failed"
        );
        let response = send_request(
            port,
            "GET",
            "/status",
            &[("Authorization".to_string(), format!("Bearer {bearer}"))],
            b"",
        );
        let dto: StatusDto = serde_json::from_value(response.body()).unwrap();
        let error = require_semantic_ready(&required_state(&dto.semantic)).unwrap_err();
        assert!(error.contains("semantic retrieval unavailable"), "{error}");
        child.kill().unwrap();
        let _ = child.wait();
    }

    // -- 2. A corrupt model fails readiness (checksum, before any load) ---
    {
        let root = tempfile::tempdir().unwrap();
        stage_assets(root.path(), &models);
        let model_path = SemanticStore::model_path_for(&HieronymusConfig::new(root.path()));
        corrupt_file_keep_size(&model_path);
        let daemon = start_daemon(root.path());
        let enable = semantic_enable(root.path(), &runtime);
        assert_eq!(enable["lane"], json!("disarmed"), "{enable}");
        wait_for_state(&daemon, "failed");
        let dto: StatusDto = serde_json::from_value(status_body(&daemon)).unwrap();
        assert!(
            require_semantic_ready(&required_state(&dto.semantic)).is_err(),
            "a corrupt model must never pass the readiness gate"
        );
        daemon.shutdown().unwrap();
    }

    // -- 2b. Before the runtime is configured, the answers say so ---------
    // Task C5 (review finding A5). The assets are staged but no runtime is
    // enabled, so the daemon has model bytes and no way to run them: the
    // exact state a fresh install sits in. Both required lanes are mandatory,
    // so this state may not answer like a complete one — mixed recall serves
    // what it has and reports the gap, while the strict semantic search
    // refuses instead of quietly serving the lexical half. Nothing here loads
    // ort (an unconfigured arm never reaches a dynamic load), so the healthy
    // daemon below still performs the single successful in-process load.
    {
        let root = tempfile::tempdir().unwrap();
        stage_assets(root.path(), &models);
        let daemon = start_daemon(root.path());
        let mut cold = 0_i64;
        wait_for_state(&daemon, "failed");

        let series = &fixture.series[0];
        cold += 1;
        call_tool(
            &daemon,
            cold,
            "hieronymus_series_create",
            json!({
                "slug": series.slug,
                "title": series.title,
                "source_language": series.source_language,
                "target_language": series.target_language,
            }),
        );
        cold += 1;
        let session = call_tool(
            &daemon,
            cold,
            "hieronymus_session_start",
            json!({"series_slug": series.slug}),
        );
        let session_id = session["session_id"].as_i64().unwrap();

        let physician = fixture
            .documents
            .iter()
            .find(|document| document.doc_id == "physician")
            .unwrap();
        let cold_sources = root.path().join("sources");
        std::fs::create_dir_all(&cold_sources).unwrap();
        let path = cold_sources.join("physician.txt");
        std::fs::write(&path, &physician.text).unwrap();
        cold += 1;
        call_tool(
            &daemon,
            cold,
            "hieronymus_rag_import",
            json!({"series_slug": series.slug, "path": path.to_str().unwrap()}),
        );
        let memory = &fixture.memories[0];
        cold += 1;
        call_tool(
            &daemon,
            cold,
            "hieronymus_short_term_add",
            json!({"session_id": session_id, "kind": memory.kind, "text": memory.text}),
        );

        // Memory retrieval is lexical: use the fixture's learned-memory
        // query, not an unrelated physician paraphrase. The strict RAG probe
        // below independently exercises unavailable semantic retrieval.
        let memory_query = fixture
            .queries
            .iter()
            .find(|query| query.expected_memory_text.as_deref() == Some(memory.text.as_str()))
            .expect("the fixture supplies a query for its learned memory");
        let payload = recall(
            &daemon,
            &mut cold,
            session_id,
            &series.slug,
            &memory_query.query,
        );
        assert!(
            payload["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|warning| warning["kind"].as_str() == Some(WARNING_SEMANTIC_UNAVAILABLE)),
            "an unconfigured semantic runtime must be reported on recall: {payload}"
        );
        assert!(
            payload["results"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["text"].as_str() == Some(memory.text.as_str())),
            "memory must keep serving while semantics is down: {payload}"
        );

        // The strict semantic search refuses, actionably.
        let error = call_tool_error(
            &daemon,
            &mut cold,
            "hieronymus_rag_search",
            json!({
                "series_slug": series.slug,
                "query": "Which doctor cared for the injured seaman on the ship?",
                "limit": 8,
            }),
        );
        assert!(
            error.contains("semantic retrieval unavailable"),
            "the refusal must name the unavailable service: {error}"
        );
        assert!(
            error.contains("hiero semantic enable --runtime"),
            "the refusal must stay actionable: {error}"
        );
        daemon.shutdown().unwrap();
    }

    // -- 3. The healthy flow: real model, real daemon, curated corpus ------
    let root = tempfile::tempdir().unwrap();
    stage_assets(root.path(), &models);
    let daemon = start_daemon(root.path());
    wait_for_state(&daemon, "failed");
    let instance_before = status_body(&daemon)["instance_id"].clone();
    let arming_started = std::time::Instant::now();
    let enable = semantic_enable(root.path(), &runtime);
    eprintln!(
        "P2 enable initial verdict ms={:.3}: {enable}",
        arming_started.elapsed().as_secs_f64() * 1000.0
    );
    // The CLI's bounded wait can return acquiring for a larger verified model.
    // That is not readiness: require the daemon's actual terminal state here.
    if enable["state"] == "acquiring" {
        assert_eq!(enable["lane"], json!("disarmed"), "{enable}");
        wait_for_state(&daemon, "ready");
    } else {
        assert_eq!(enable["lane"], json!("armed"), "{enable}");
    }
    eprintln!(
        "P2 enable actual-ready ms={:.3}",
        arming_started.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(status_body(&daemon)["instance_id"], instance_before);
    assert_eq!(enable["model_status"], json!("available"), "{enable}");
    assert_eq!(enable["downloaded"], json!(false), "{enable}");
    assert_eq!(enable["tokenizer_downloaded"], json!(false), "{enable}");

    let mut id = 0_i64;

    // Series and sessions (normal authenticated MCP operations only).
    let mut sessions = std::collections::HashMap::<String, i64>::new();
    for series in &fixture.series {
        id += 1;
        call_tool(
            &daemon,
            id,
            "hieronymus_series_create",
            json!({
                "slug": series.slug,
                "title": series.title,
                "source_language": series.source_language,
                "target_language": series.target_language,
            }),
        );
        id += 1;
        let session = call_tool(
            &daemon,
            id,
            "hieronymus_session_start",
            json!({"series_slug": series.slug}),
        );
        sessions.insert(series.slug.clone(), session["session_id"].as_i64().unwrap());
    }

    // Documents: one imported .txt per fixture document.
    let sources = root.path().join("sources");
    std::fs::create_dir_all(&sources).unwrap();
    for document in &fixture.documents {
        let path = sources.join(format!("{}.txt", document.doc_id));
        std::fs::write(&path, &document.text).unwrap();
        id += 1;
        let imported = call_tool(
            &daemon,
            id,
            "hieronymus_rag_import",
            json!({"series_slug": document.series_slug, "path": path.to_str().unwrap()}),
        );
        assert!(
            imported["semantic_rebuild_job"].is_string(),
            "import must queue a durable rebuild: {imported}"
        );
    }

    // Learned memories (short-term, through the session).
    for memory in &fixture.memories {
        id += 1;
        call_tool(
            &daemon,
            id,
            "hieronymus_short_term_add",
            json!({
                "session_id": sessions[&memory.series_slug],
                "kind": memory.kind,
                "text": memory.text,
            }),
        );
    }

    // Active terminology: proposed then approved through the termbase tools.
    for term in &fixture.terms {
        id += 1;
        let draft = call_tool(
            &daemon,
            id,
            "hieronymus_termbase_propose",
            json!({
                "series_slug": term.series_slug,
                "category": term.category,
                "source_text": term.source_text,
                "canonical_translation": term.canonical_translation,
            }),
        );
        id += 1;
        call_tool(
            &daemon,
            id,
            "hieronymus_termbase_approve",
            json!({
                "series_slug": term.series_slug,
                "term_id": draft["id"],
            }),
        );
    }

    // The supervised worker rebuilds the corpus with the real model.
    wait_for_state(&daemon, "ready");
    let store_config = HieronymusConfig::new(root.path());
    let store = SemanticStore::open(&store_config).unwrap();
    let active = store
        .active_generation()
        .unwrap()
        .expect("active generation");
    assert_eq!(active.identity, OnnxEmbeddingProvider::static_identity());
    assert_eq!(active.written_count, active.expected_count);
    assert!(
        active.expected_count >= fixture.documents.len() as u64,
        "every fixture document must be embedded: {active:?}"
    );

    // Ready passes the typed gate.
    let dto: StatusDto = serde_json::from_value(status_body(&daemon)).unwrap();
    require_semantic_ready(&required_state(&dto.semantic)).expect("ready must pass the gate");

    // Keep measuring later cases after a failure: a language failure must not
    // hide the other languages or public tool outcomes.
    let mut failures = Vec::new();
    for expectation in &fixture.queries {
        assert!(["en", "ja", "ru"].contains(&expectation.language.as_str()));
        assert_eq!(expectation.context.acceptance, "retrieval");
        assert!(
            expectation.context.viewpoint.is_none() && expectation.context.story_position.is_none(),
            "viewpoint applicability requires P1 runtime; never silently ignore context"
        );
        let session_id = sessions[&expectation.series_slug];
        let payload = recall(
            &daemon,
            &mut id,
            session_id,
            &expectation.series_slug,
            &expectation.query,
        );
        for (tool, rows) in [
            (
                "recall",
                rag_rows(&payload).into_iter().cloned().collect::<Vec<_>>(),
            ),
            (
                "rag_search",
                rag_search(
                    &daemon,
                    &mut id,
                    &expectation.series_slug,
                    &expectation.query,
                ),
            ),
        ] {
            record_case_failure(&mut failures, &expectation.query_id, tool, || {
                let top_three_source_ids: Vec<&str> = rows
                    .iter()
                    .take(3)
                    .map(|row| {
                        doc_id_of(row, &fixture.documents)
                            .expect("every returned source must resolve")
                    })
                    .collect();
                let returned_series: Vec<&str> = rows
                    .iter()
                    .map(|row| {
                        let doc_id = doc_id_of(row, &fixture.documents).expect("known source");
                        fixture
                            .documents
                            .iter()
                            .find(|doc| doc.doc_id == doc_id)
                            .unwrap()
                            .series_slug
                            .as_str()
                    })
                    .collect();
                let requested_series = expectation.series_slug.as_str();
                eprintln!(
                    "P2 {} {} language={} top3={:?} rows={:?}",
                    expectation.query_id, tool, expectation.language, top_three_source_ids, rows
                );
                assert!(
                    returned_series
                        .iter()
                        .all(|series| series == &requested_series)
                );
                assert!(returned_series.iter().all(|series| {
                    !expectation
                        .excluded_series
                        .iter()
                        .any(|excluded| excluded == series)
                }));
                for row in &rows {
                    assert!(!expectation.excluded_doc_ids.iter().any(|excluded| Some(
                        excluded.as_str()
                    ) == doc_id_of(
                        row,
                        &fixture.documents
                    )));
                }
                for expected in &expectation.expected_top3_doc_ids {
                    let expected_source_id = expected.as_str();
                    assert!(top_three_source_ids.contains(&expected_source_id));
                    if expectation.require_semantic_provenance {
                        let semantic_only_result_reasons: Vec<&str> = rows
                            .iter()
                            .filter(|row| {
                                doc_id_of(row, &fixture.documents) == Some(expected_source_id)
                            })
                            .filter_map(|row| row["rank_reason"].as_str())
                            .collect();
                        assert!(
                            semantic_only_result_reasons
                                .iter()
                                .any(|reason| reason == &"rag semantic match")
                        );
                    }
                }
            });
        }
        collect_recall_expectation_failures(expectation, &payload, &mut failures);
    }
    eprintln!("P2 corpus failures: {failures:?}");
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status
            .lines()
            .filter(|line| line.starts_with("VmHWM:") || line.starts_with("VmRSS:"))
        {
            eprintln!("P2 resource {line}");
        }
    }

    // Repeated sessions use the same durable RAG corpus. Session-local notes
    // are not silently promoted into long-term authority by session completion.
    for memory in &fixture.memories {
        id += 1;
        call_tool(
            &daemon,
            id,
            "hieronymus_session_complete",
            json!({"session_id": sessions[&memory.series_slug]}),
        );
        id += 1;
        let next = call_tool(
            &daemon,
            id,
            "hieronymus_session_start",
            json!({"series_slug": memory.series_slug}),
        );
        let next_id = next["session_id"].as_i64().unwrap();
        sessions.insert(memory.series_slug.clone(), next_id);
        let response = recall(&daemon, &mut id, next_id, &memory.series_slug, &memory.text);
        assert!(
            !response["results"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["rank_reason"].as_str() == Some(SHORT_TERM_REASON)
                    && row["text"].as_str() == Some(memory.text.as_str()))
        );
        let query = fixture
            .queries
            .iter()
            .find(|q| q.series_slug == memory.series_slug && q.require_semantic_provenance)
            .unwrap();
        let response = recall(&daemon, &mut id, next_id, &memory.series_slug, &query.query);
        let rows = rag_rows(&response);
        for expected in &query.expected_top3_doc_ids {
            assert!(
                rows.iter()
                    .take(3)
                    .any(|row| doc_id_of(row, &fixture.documents) == Some(expected.as_str()))
            );
        }
        eprintln!("P2 repeated-session durable RAG retained; unpromoted session-local note absent");
    }

    // -- 4. English-model generations are rejected, then healed by a rebuild ---
    // A legacy manifest persisted under the retired English-model tokenization
    // (with its original model, revision and tokenizer id) is seeded
    // the way a legacy database would carry it. The armed multilingual lane can
    // never serve it: the per-recall identity check degrades the lane, and
    // the supervised worker rebuilds under the real identity.
    let legacy_identity = EmbeddingIdentity::new(
        "onnx",
        "all-MiniLM-L6-v2",
        "9a53d751e60e6dd34f2443711d44d5b09389f89a",
        EMBEDDING_DIMENSIONS,
        "l2",
        LEGACY_ENGLISH_TOKENIZER_ID,
        512,
        32,
    )
    .unwrap();
    {
        let connection = rusqlite::Connection::open(store_config.database_path()).unwrap();
        connection
            .execute(
                "update semantic_generations set status = 'superseded', active = 0 where active = 1",
                [],
            )
            .unwrap();
        connection
            .execute(
                "insert into semantic_generations(
                   generation_id, status, provider, model, model_revision, dimensions,
                   normalization, tokenizer, max_input_tokens, max_batch_inputs,
                   expected_count, written_count, last_chunk_id, active, created_at, updated_at
                 ) values ('legacy-english-model', 'active', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                           (select count(*) from rag_chunks),
                           (select count(*) from rag_chunks), 0, 1, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                rusqlite::params![
                    legacy_identity.provider(),
                    legacy_identity.model(),
                    legacy_identity.revision(),
                    legacy_identity.dimensions() as i64,
                    legacy_identity.normalization(),
                    legacy_identity.tokenizer(),
                    legacy_identity.max_input_tokens() as i64,
                    legacy_identity.max_batch_inputs() as i64,
                ],
            )
            .unwrap();
    }
    // The very next recall must not serve the English-model generation...
    {
        let session_id = sessions[&fixture.queries[0].series_slug];
        let payload = recall(
            &daemon,
            &mut id,
            session_id,
            &fixture.queries[0].series_slug,
            &fixture.queries[0].query,
        );
        assert!(
            payload["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|warning| warning["kind"].as_str() == Some(WARNING_SEMANTIC_UNAVAILABLE)),
            "a persisted English-model generation must degrade the semantic lane: {payload}"
        );
        // ...and the strict search refuses outright rather than answering
        // with the lexical half over a corpus whose vectors it cannot query
        // (task C5): whether the controller has noticed yet or not, the
        // outcome is an error, never a plausible-looking result set.
        let error = call_tool_error(
            &daemon,
            &mut id,
            "hieronymus_rag_search",
            json!({
                "series_slug": fixture.queries[0].series_slug,
                "query": fixture.queries[0].query,
                "limit": 8,
            }),
        );
        assert!(!error.is_empty(), "the refusal must carry a reason");
    }
    // ...and the next normal import heals it: the durable whole-corpus
    // rebuild runs under the pinned identity and supersedes the legacy row
    // (the English-model generation is never relabeled, never served).
    {
        let healing = root.path().join("sources").join("healing.txt");
        std::fs::write(
            &healing,
            "The harbour log recorded the healing of the fleet.",
        )
        .unwrap();
        id += 1;
        let imported = call_tool(
            &daemon,
            id,
            "hieronymus_rag_import",
            json!({"series_slug": fixture.queries[0].series_slug, "path": healing.to_str().unwrap()}),
        );
        assert!(
            imported["semantic_rebuild_job"].is_string(),
            "the healing import must queue a rebuild: {imported}"
        );
    }
    assert!(
        wait_until(
            || {
                SemanticStore::open(&store_config)
                    .unwrap()
                    .active_generation()
                    .unwrap()
                    .is_some_and(|manifest| {
                        manifest.identity == OnnxEmbeddingProvider::static_identity()
                    })
            },
            Duration::from_secs(180)
        ),
        "the English-model generation must be replaced by a pinned-identity rebuild"
    );
    wait_for_state(&daemon, "ready");
    let healed = SemanticStore::open(&store_config)
        .unwrap()
        .active_generation()
        .unwrap()
        .unwrap();
    assert_eq!(healed.identity, OnnxEmbeddingProvider::static_identity());
    assert_ne!(healed.identity.tokenizer(), LEGACY_ENGLISH_TOKENIZER_ID);
    {
        let connection = rusqlite::Connection::open(store_config.database_path()).unwrap();
        let legacy: i64 = connection
            .query_row(
                "select count(*) from semantic_generations
                 where generation_id = 'legacy-english-model'
                   and tokenizer = ?1 and status in ('failed', 'superseded') and active = 0",
                rusqlite::params![LEGACY_ENGLISH_TOKENIZER_ID],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            legacy, 1,
            "the legacy row keeps its English-model identity and is terminal/inactive"
        );
    }
    // The healed lane serves real semantic recall again.
    {
        let query = fixture
            .queries
            .iter()
            .find(|q| q.require_semantic_provenance)
            .unwrap();
        let session_id = sessions[&query.series_slug];
        let payload = recall(
            &daemon,
            &mut id,
            session_id,
            &query.series_slug,
            &query.query,
        );
        assert!(
            rag_rows(&payload)
                .iter()
                .any(|row| row["rank_reason"].as_str() == Some(RAG_SEMANTIC_REASON)),
            "the healed lane must serve semantic recall again: {payload}"
        );
    }

    let mut provider = OnnxEmbeddingProvider::load(&runtime, &models.join("model.onnx")).unwrap();
    for invalid in [250_037, u32::MAX] {
        assert!(provider.embed_document(&[invalid]).is_err());
        assert!(provider.embed_query(&[invalid]).is_err());
    }
    daemon.shutdown().unwrap();
    assert_corpus_success(&failures);
}

fn record_case_failure(
    failures: &mut Vec<String>,
    query_id: &str,
    check: &str,
    assertion: impl FnOnce() + std::panic::UnwindSafe,
) {
    if std::panic::catch_unwind(assertion).is_err() {
        failures.push(format!("{query_id}:{check}"));
    }
}

fn assert_corpus_success(failures: &[String]) {
    assert!(failures.is_empty(), "corpus failures: {failures:?}");
}

fn collect_recall_expectation_failures(
    expectation: &QuerySpec,
    payload: &Value,
    failures: &mut Vec<String>,
) {
    // The learned memory is returned.
    if let Some(memory_text) = &expectation.expected_memory_text {
        record_case_failure(failures, &expectation.query_id, "memory", || {
            assert!(
                payload["results"].as_array().unwrap().iter().any(|row| {
                    row["text"].as_str() == Some(memory_text.as_str())
                        && row["rank_reason"].as_str() == Some(SHORT_TERM_REASON)
                }),
                "query {} must return the learned memory: {payload}",
                expectation.query_id
            );
        });
    }

    // Active terminology stays deterministic and independent of ranking.
    if let Some(contract) = &expectation.expected_contract_term {
        record_case_failure(failures, &expectation.query_id, "contract", || {
            let terms = payload["deterministic_contract"].as_array().unwrap();
            assert!(
                terms.iter().any(|term| {
                    term["source_text"].as_str() == Some(contract.source_text.as_str())
                        && term["canonical_translation"].as_str()
                            == Some(contract.canonical_translation.as_str())
                }),
                "query {} must carry the approved term in the deterministic contract: {payload}",
                expectation.query_id
            );
            // The term has no ranked hit requirement: ranking never depends
            // on the contract (and the contract never silently overrides
            // ranked text — the rows are untouched by the termbase).
            let ranked_has_term = payload["results"].as_array().unwrap().iter().any(|row| {
                row["text"]
                    .as_str()
                    .is_some_and(|text| text.contains(&contract.canonical_translation))
            });
            assert!(
                !ranked_has_term,
                "query {} ranked rows must stay independent of the deterministic contract",
                expectation.query_id
            );
        });
    }
}

#[test]
fn memory_and_contract_failures_preserve_later_language_outcomes() {
    let fixture: Fixture = serde_json::from_str(FIXTURE).unwrap();
    // Missing learned memory and contract deliberately reproduce both early
    // recall regressions using the same checks as the live corpus loop.
    let missing = json!({"results": [], "deterministic_contract": []});
    let mut failures = Vec::new();
    for expectation in &fixture.queries {
        collect_recall_expectation_failures(expectation, &missing, &mut failures);
        if matches!(expectation.language.as_str(), "ja" | "ru") {
            record_case_failure(&mut failures, &expectation.query_id, "recall", || {
                panic!("injected later-language retrieval failure");
            });
        }
    }
    assert_eq!(
        failures,
        [
            "learned-memory-electrification:memory",
            "terminology-independence:contract",
            "ship-physician-ja-paraphrase:recall",
            "cartographer-ja-paraphrase:recall",
            "ship-physician-ru-paraphrase:recall",
            "cartographer-ru-paraphrase:recall",
        ]
    );
    assert!(std::panic::catch_unwind(|| assert_corpus_success(&failures)).is_err());

    // A present contract with canonical text leaking into ranked evidence
    // must also be aggregated rather than mistaken for a passing contract.
    let expectation = fixture
        .queries
        .iter()
        .find(|query| query.expected_contract_term.is_some())
        .unwrap();
    let polluted = json!({
        "results": [{"text": "пенициллин"}],
        "deterministic_contract": [{"source_text": "penicillin", "canonical_translation": "пенициллин"}],
    });
    let mut failures = Vec::new();
    collect_recall_expectation_failures(expectation, &polluted, &mut failures);
    assert_eq!(failures, ["terminology-independence:contract"]);
}

// ------------------------------------------------------------------ helpers

fn required_state(dto: &SemanticStateDto) -> RequiredSemanticState {
    match dto.state.as_str() {
        "acquiring" => RequiredSemanticState::Acquiring,
        "rebuilding" => RequiredSemanticState::Rebuilding,
        "ready" => RequiredSemanticState::Ready,
        other => {
            RequiredSemanticState::Failed(dto.detail.clone().unwrap_or_else(|| other.to_string()))
        }
    }
}

/// Flips bytes in the middle of the file, preserving its size: the cheap
/// size pre-check still passes, so the failure must come from the
/// cryptographic verification at provider load.
fn corrupt_file_keep_size(path: &Path) {
    let mut bytes = std::fs::read(path).unwrap();
    assert!(bytes.len() > 16);
    let middle = bytes.len() / 2;
    for byte in &mut bytes[middle..middle + 8] {
        *byte = byte.wrapping_add(1);
    }
    std::fs::write(path, bytes).unwrap();
}

/// An OS-assigned free TCP port for the daemon subprocess.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
