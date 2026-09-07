//! Task S3: the REAL semantic qualification suite. Unlike
//! `semantic_execution.rs` (scripted fake providers), everything here runs
//! the pinned all-MiniLM-L6-v2 ONNX model through the production `OnnxArm`
//! and the S1 WordPiece tokenizer, driven only through normal authenticated
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
//! HIERO_TEST_MODEL_DIR=<...>/all-MiniLM-L6-v2 \
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
    BYTE_FOLD_TOKENIZER_ID, EMBEDDING_DIMENSIONS, EmbeddingIdentity, OnnxEmbeddingProvider,
};
use hieronymus::semantic_model::{MODEL_NAME, MODEL_REVISION};
use hieronymus::semantic_store::SemanticStore;
use serde::Deserialize;
use serde_json::{Value, json};

use common::{mcp_headers, send_request, start_daemon, wait_until};

const FIXTURE: &str = include_str!("fixtures/hybrid-relevance.json");

/// `ort` keeps the loaded dylib in a process-global `OnceLock`, so the whole
/// qualification runs as one sequenced test (see the module docs).
const RAG_SEMANTIC_REASON: &str = "rag semantic match";
const SHORT_TERM_REASON: &str = "active session short-term memory match";

// ------------------------------------------------------------- typed fixture

#[derive(Debug, Deserialize)]
struct Fixture {
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
struct QuerySpec {
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
struct ContractExpectation {
    source_text: String,
    canonical_translation: String,
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
    let response = send_request(
        daemon.local_addr().port(),
        "POST",
        "/mcp",
        &mcp_headers(daemon, &[("Mcp-Method", "tools/call"), ("Mcp-Name", name)]),
        &serde_json::to_vec(&tools_call(id, name, arguments)).unwrap(),
    );
    assert_eq!(response.status, 200, "{:?}", response.raw_body);
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
    let enable = semantic_enable(root.path(), &runtime);
    assert_eq!(enable["lane"], json!("armed"), "{enable}");
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

    // The curated relevance assertions.
    for expectation in &fixture.queries {
        let session_id = sessions[&expectation.series_slug];
        let payload = recall(
            &daemon,
            &mut id,
            session_id,
            &expectation.series_slug,
            &expectation.query,
        );

        let rows = rag_rows(&payload);

        // No foreign-series hit, ever: every returned RAG row belongs to a
        // document of the queried series.
        for row in &rows {
            if let Some(doc_id) = doc_id_of(row, &fixture.documents) {
                let series = fixture
                    .documents
                    .iter()
                    .find(|document| document.doc_id == doc_id)
                    .unwrap()
                    .series_slug
                    .clone();
                assert_eq!(
                    series, expectation.series_slug,
                    "query {} leaked a document from series {series}: {payload}",
                    expectation.query_id
                );
            }
        }
        for excluded in &expectation.excluded_series {
            assert!(
                !fixture.series.iter().any(
                    |series| &series.slug == excluded && series.slug == expectation.series_slug
                ),
                "fixture error: {excluded} cannot be both excluded and the queried series"
            );
        }

        // Every curated expected source is in the top 3 RAG rows.
        let observed_top3: Vec<Option<&str>> = rows
            .iter()
            .take(3)
            .map(|row| doc_id_of(row, &fixture.documents))
            .collect();
        eprintln!(
            "query {:?} observed rag top3: {:?}",
            expectation.query_id, observed_top3
        );
        for expected in &expectation.expected_top3_doc_ids {
            assert!(
                observed_top3.contains(&Some(expected.as_str())),
                "query {} ({:?}) must return document {expected} in the top 3 rag rows; got {:?} (rows: {rows:?})",
                expectation.query_id,
                expectation.query,
                observed_top3
            );
        }

        // Semantic provenance for semantic-only queries.
        if expectation.require_semantic_provenance {
            let semantic_hit = rows.iter().any(|row| {
                row["rank_reason"].as_str() == Some(RAG_SEMANTIC_REASON)
                    && expectation.expected_top3_doc_ids.iter().any(|expected| {
                        doc_id_of(row, &fixture.documents) == Some(expected.as_str())
                    })
            });
            assert!(
                semantic_hit,
                "query {} must show actual semantic-lane provenance ({RAG_SEMANTIC_REASON}) for its expected sources: {payload}",
                expectation.query_id
            );
        }

        // The learned memory is returned.
        if let Some(memory_text) = &expectation.expected_memory_text {
            assert!(
                payload["results"].as_array().unwrap().iter().any(|row| {
                    row["text"].as_str() == Some(memory_text.as_str())
                        && row["rank_reason"].as_str() == Some(SHORT_TERM_REASON)
                }),
                "query {} must return the learned memory: {payload}",
                expectation.query_id
            );
        }

        // Active terminology stays deterministic and independent of ranking.
        if let Some(contract) = &expectation.expected_contract_term {
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
        }
    }

    // -- 3b. The public semantic search over the real model ---------------
    // The same curated paraphrase, through `hieronymus_rag_search` rather
    // than recall: a session-less series+query search that must run the real
    // semantic lane (task C5). The query shares no content word with the
    // document, so a lexical-only implementation cannot answer it at all —
    // which is precisely what made the pre-C5 lexical path invisible.
    for query_id in ["physician-paraphrase", "cartographer-paraphrase"] {
        let expectation = fixture
            .queries
            .iter()
            .find(|query| query.query_id == query_id)
            .unwrap();
        let rows = rag_search(
            &daemon,
            &mut id,
            &expectation.series_slug,
            &expectation.query,
        );
        assert!(
            !rows.is_empty(),
            "query {query_id} must return hybrid rows: {rows:?}"
        );
        for row in &rows {
            assert_eq!(row["source"], json!("rag"), "{row}");
            // Foreign-series exclusion: `harbour-records` holds a near
            // duplicate of the physician document, so a leak would show up
            // here as a top hit rather than as a subtle ordering change.
            if let Some(doc_id) = doc_id_of(row, &fixture.documents) {
                let owner = fixture
                    .documents
                    .iter()
                    .find(|document| document.doc_id == doc_id)
                    .unwrap();
                assert_eq!(
                    owner.series_slug, expectation.series_slug,
                    "query {query_id} leaked {doc_id} from {}: {rows:?}",
                    owner.series_slug
                );
            }
        }
        // Real semantic provenance for the expected sources.
        assert!(
            rows.iter().any(|row| {
                row["rank_reason"].as_str() == Some(RAG_SEMANTIC_REASON)
                    && expectation.expected_top3_doc_ids.iter().any(|expected| {
                        doc_id_of(row, &fixture.documents) == Some(expected.as_str())
                    })
            }),
            "query {query_id} must show semantic-lane provenance ({RAG_SEMANTIC_REASON}): {rows:?}"
        );
    }

    // -- 4. Byte-fold generations are rejected, then healed by a rebuild ---
    // A legacy manifest persisted under the retired byte-fold tokenization
    // (identical to the pinned identity except the tokenizer id) is seeded
    // the way a legacy database would carry it. The armed WordPiece lane can
    // never serve it: the per-recall identity check degrades the lane, and
    // the supervised worker rebuilds under the real identity.
    let byte_fold_identity = EmbeddingIdentity::new(
        "onnx",
        MODEL_NAME,
        MODEL_REVISION,
        EMBEDDING_DIMENSIONS,
        "l2",
        BYTE_FOLD_TOKENIZER_ID,
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
                 ) values ('legacy-byte-fold', 'active', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                           (select count(*) from rag_chunks),
                           (select count(*) from rag_chunks), 0, 1, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                rusqlite::params![
                    byte_fold_identity.provider(),
                    byte_fold_identity.model(),
                    byte_fold_identity.revision(),
                    byte_fold_identity.dimensions() as i64,
                    byte_fold_identity.normalization(),
                    byte_fold_identity.tokenizer(),
                    byte_fold_identity.max_input_tokens() as i64,
                    byte_fold_identity.max_batch_inputs() as i64,
                ],
            )
            .unwrap();
    }
    // The very next recall must not serve the byte-fold generation...
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
            "a persisted byte-fold generation must degrade the semantic lane: {payload}"
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
    // (the byte-fold generation is never relabeled, never served).
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
        "the byte-fold generation must be replaced by a pinned-identity rebuild"
    );
    wait_for_state(&daemon, "ready");
    let healed = SemanticStore::open(&store_config)
        .unwrap()
        .active_generation()
        .unwrap()
        .unwrap();
    assert_eq!(healed.identity, OnnxEmbeddingProvider::static_identity());
    assert_ne!(healed.identity.tokenizer(), BYTE_FOLD_TOKENIZER_ID);
    {
        let connection = rusqlite::Connection::open(store_config.database_path()).unwrap();
        let legacy: i64 = connection
            .query_row(
                "select count(*) from semantic_generations
                 where generation_id = 'legacy-byte-fold'
                   and tokenizer = ?1 and status in ('failed', 'superseded') and active = 0",
                rusqlite::params![BYTE_FOLD_TOKENIZER_ID],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            legacy, 1,
            "the legacy row keeps its byte-fold identity and is terminal/inactive"
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

    daemon.shutdown().unwrap();
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
