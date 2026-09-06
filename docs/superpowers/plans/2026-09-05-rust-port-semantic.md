# Rust memory and semantic RAG completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship working hybrid recall over learned memory and imported RAG, using real model tokenization and durable production indexing.

**Architecture:** SQLite remains authoritative; LanceDB generations are rebuildable derived data. One supervised indexing worker and the daemon's recall service load the same pinned model/tokenizer identity; readiness is explicit and semantic failure cannot silently satisfy the owner's completion requirement.

**Tech Stack:** Rust 1.96 / edition 2024, rusqlite/SQLite FTS5, existing blocking daemon workers, Svelte 5, TypeScript, Bun 1.4.0; preserve Cargo.lock native pins.

**Spec:** ADR 0013; ADR 0011 deterministic contract; Astra 7–8; Sonnet predicate-hardening note; owner's 2026-09-05 requirement rejecting an FTS-only release. Also read [the remediation coordinator](2026-09-05-rust-port-remediation.md) and [the reconciled review](../../astra-report.md).

## Global Constraints

- “All domain mutations used by normal CLI, MCP, and frontend workflows go through the daemon.” (ADR 0009)
- “The first Rust cutover supports `x86_64-unknown-linux-gnu`.” (ADR 0013)
- Owner requirement (2026-09-05): working memory and semantic RAG are mandatory. ADR 0013’s FTS-only allowance is not an acceptable completion or release alternative for this program.
- “Only an authenticated user acting through the explicit rule-approval operation may transition `candidate` to `active`” (ADR 0011).
- “the separate CSRF token layer is waived.” (ADR 0012, 2026-09-03 amendment)
- MCP revision remains `2026-07-28`. Preserve frozen Python inputs; new ADR-backed Rust expectations are separate versioned fixtures.
- Preserve unrelated work, immutable backups, local plaintext credentials, and the single data-root layout. No Python runtime rollback, dependency-pin refresh, publication, or user-service changes during unit tests.
- Steps below specify planned code, not code already implemented. Existing types are referenced by source module; new cross-task interfaces are declared explicitly.

---

## File structure and execution boundary

`semantic_tokenizer.rs` owns actual WordPiece preprocessing. `daemon/semantic_worker.rs` owns jobs and reload notifications. Existing semantic modules retain generation leases, reconciliation and retrieval. Relevance fixtures test the shipped path rather than synthetic vectors.

- **S1:** Replace byte-fold preprocessing with a pinned real tokenizer — `crates/hieronymus/src/semantic_tokenizer.rs`, `crates/hieronymus/src/lib.rs`, `crates/hieronymus/src/semantic_model.rs`, `crates/hieronymus/src/semantic_embeddings.rs`, `crates/hieronymus/src/semantic_recall.rs`, `crates/hieronymus/src/semantic_arming.rs`, `crates/hieronymus/Cargo.toml`, `Cargo.lock`, `crates/hieronymus/tests/fixtures/minilm-tokenizer.json`, `crates/hieronymus/tests/semantic_tokenizer.rs`
- **S2:** Supervise rebuilds and arm actual daemon recall — `crates/hiero/src/daemon/semantic_worker.rs`, `crates/hiero/src/daemon/mod.rs`, `crates/hiero/src/application/mod.rs`, `crates/hiero/src/application/memory.rs`, `crates/hiero/src/main.rs`, `crates/hiero/src/daemon/rest/status.rs`, `crates/hieronymus/src/semantic_jobs.rs`, `crates/hieronymus/src/semantic_arming.rs`, `crates/hieronymus/src/semantic_recall.rs`, `crates/hiero/tests/semantic_execution.rs`
- **S3:** Validate real hybrid relevance and centralize ANN predicates — `crates/hieronymus/src/semantic_index.rs`, `crates/hiero/tests/fixtures/hybrid-relevance.json`, `crates/hieronymus/tests/semantic_predicates.rs`, `crates/hiero/tests/semantic_real.rs`, `docs/semantic-validation.md`, `qualification/records/semantic-native.json`

## Tasks

### Task S1: Replace byte-fold preprocessing with a pinned real tokenizer

**Coverage:** Astra 7; document/query identity and old-generation invalidation.

**Dependencies:** R1; precedes S2.

**Files:**

- Create: `crates/hieronymus/src/semantic_tokenizer.rs`
- Modify: `crates/hieronymus/src/lib.rs`
- Modify: `crates/hieronymus/src/semantic_model.rs`
- Modify: `crates/hieronymus/src/semantic_embeddings.rs`
- Modify: `crates/hieronymus/src/semantic_recall.rs`
- Modify: `crates/hieronymus/src/semantic_arming.rs`
- Modify: `crates/hieronymus/Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/hieronymus/tests/fixtures/minilm-tokenizer.json`
- Test: `crates/hieronymus/tests/semantic_tokenizer.rs`

**Interfaces:** Produce `ModelTokenizer::from_bytes(&[u8]) -> Result<Self,SemanticError>` and `ModelTokenizer::encode(&self,&str) -> Result<Vec<u32>,SemanticError>`, implementing existing ChunkTokenizer. Tokenizer identity includes asset hash, max_length=256, special-token policy, truncation and normalization. Both query/document paths consume this type, not ByteFoldTokenizer.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hieronymus::semantic_tokenizer::ModelTokenizer;
#[test]
fn minilm_uses_wordpiece_ids_and_special_tokens() {
    let tokenizer = ModelTokenizer::from_bytes(
        include_bytes!("fixtures/minilm-tokenizer.json")
    ).unwrap();
    assert_eq!(tokenizer.encode("Hello world").unwrap(), vec![101, 7592, 2088, 102]);
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hieronymus --test semantic_tokenizer --locked`
Expected before the change: current byte-fold tokens differ from the known vocabulary IDs.

- [ ] **Step 3: Implement the bounded change**

Add tokenizers = { version = "=0.23.2", default-features = false, features = ["onig"] } to the domain crate and update the lockfile only for this dependency closure. Obtain tokenizer.json from the same pinned model revision and require SHA-256 be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037 (466247 bytes; verified during planning). Source: https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/9a53d751e60e6dd34f2443711d44d5b09389f89a/tokenizer.json . Preserve its BertNormalizer and postprocessor, explicitly set longest-first truncation to 256 tokens including special tokens, and disable tokenizer padding so existing batch inference builds masks. Download/verify/persist tokenizer alongside the model atomically; no unverified executable download is involved. Identity must change from byte-fold-v1; reject and rebuild old generations, never relabel them. Retain byte-fold only in explicitly synthetic tests if still useful.

```rust
pub struct ModelTokenizer(tokenizers::Tokenizer);

impl ModelTokenizer {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SemanticError> {
        let mut tokenizer = tokenizers::Tokenizer::from_bytes(bytes)
            .map_err(|error| SemanticError::InvalidEmbedding(error.to_string()))?;
        tokenizer.with_truncation(Some(tokenizers::TruncationParams {
            max_length: 256,
            ..Default::default()
        })).map_err(|error| SemanticError::InvalidEmbedding(error.to_string()))?;
        tokenizer.with_padding(None);
        Ok(Self(tokenizer))
    }

    pub fn encode(&self, text: &str) -> Result<Vec<u32>, SemanticError> {
        self.0.encode(text, true)
            .map(|encoding| encoding.get_ids().to_vec())
            .map_err(|error| SemanticError::InvalidEmbedding(error.to_string()))
    }
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hieronymus --test semantic_tokenizer --locked`
Expected after the change: known IDs match; production query and rebuild use identical tokenizer identity.

Add casing, accents, punctuation, CJK, Cyrillic, empty string, long input and invalid asset/hash cases. Check [CLS]/[SEP] retained under truncation, masks exclude padding, pooled outputs are finite/384-dimensional/L2-normalized. Reopen a persisted byte-fold generation and prove it cannot serve new queries. Keep existing ort/LanceDB/Arrow pins; onig becomes a release dependency that F1 must qualify.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hieronymus/src/semantic_tokenizer.rs crates/hieronymus/src/lib.rs crates/hieronymus/src/semantic_model.rs crates/hieronymus/src/semantic_embeddings.rs crates/hieronymus/src/semantic_recall.rs crates/hieronymus/src/semantic_arming.rs crates/hieronymus/Cargo.toml Cargo.lock crates/hieronymus/tests/fixtures/minilm-tokenizer.json crates/hieronymus/tests/semantic_tokenizer.rs
git commit -m "fix: use pinned wordpiece tokenizer for semantic recall"
```

### Task S2: Supervise rebuilds and arm actual daemon recall

**Coverage:** Astra 8; Sonnet 2.1; mandatory production semantic RAG.

**Dependencies:** R3, R5, M2, S1.

**Files:**

- Create: `crates/hiero/src/daemon/semantic_worker.rs`
- Modify: `crates/hiero/src/daemon/mod.rs`
- Modify: `crates/hiero/src/application/mod.rs`
- Modify: `crates/hiero/src/application/memory.rs`
- Modify: `crates/hiero/src/main.rs`
- Modify: `crates/hiero/src/daemon/rest/status.rs`
- Modify: `crates/hieronymus/src/semantic_jobs.rs`
- Modify: `crates/hieronymus/src/semantic_arming.rs`
- Modify: `crates/hieronymus/src/semantic_recall.rs`
- Test: `crates/hiero/tests/semantic_execution.rs`

**Interfaces:** Produce `SemanticController::start(config: HieronymusConfig, runtime: PathBuf, workers: &mut WorkerGroup) -> Result<Self,String>` and `request_rebuild(&self, series: &str) -> Result<String,String>` returning durable job ID. Produce `RequiredSemanticState { Acquiring, Rebuilding, Ready, Failed(String) }` and `require_semantic_ready(&RequiredSemanticState) -> Result<(),String>`. Existing SemanticJobStore::run_rebuild consumes RebuildInputs and RebuildConfig; arm_recall_service installs its resulting lane in Application instead of dropping ArmedRecall.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hiero::daemon::semantic_worker::{require_semantic_ready, RequiredSemanticState};
#[test]
fn required_semantics_cannot_be_reported_ready_when_disarmed() {
    assert!(require_semantic_ready(&RequiredSemanticState::Failed("runtime missing".into())).is_err());
    assert!(require_semantic_ready(&RequiredSemanticState::Rebuilding).is_err());
    assert!(require_semantic_ready(&RequiredSemanticState::Ready).is_ok());
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test semantic_execution`
Expected before the change: there is no production job executor and arming results are not connected to recall.

- [ ] **Step 3: Implement the bounded change**

Start the worker under R5 ownership, reconcile interrupted jobs before claiming new work, and call run_rebuild with S1 tokenizer plus the real ONNX provider. Read authoritative bounded SQLite chunks; perform inference/LanceDB work outside SQLite transactions; use existing lease/generation checks for activation. RAG import queues a rebuild after its authoritative commit, with durable recovery of a missed in-memory wakeup by startup/periodic reconciliation. Refresh Application's recall lane on verified generation activation; queries remain on a coherent identity and generation. Persist configured runtime location in the existing semantic configuration/settings ownership, validate on startup, and retain it across restarts. Expose acquiring/rebuilding/ready/failed state in /status and console. A missing runtime/model/tokenizer is an actionable setup failure, not 'semantic enabled'. Mixed recall may return useful memory plus explicit incomplete warnings during transient rebuilds, but strict RAG calls must not pretend successful semantic retrieval and release/update health cannot count a disarmed lane as ready. Do not turn an empty RAG corpus into an error: loaded model/runtime with no documents is ready-for-ingest.

```rust
pub enum RequiredSemanticState {
    Acquiring,
    Rebuilding,
    Ready,
    Failed(String),
}

pub fn require_semantic_ready(state: &RequiredSemanticState) -> Result<(), String> {
    match state {
        RequiredSemanticState::Ready => Ok(()),
        RequiredSemanticState::Acquiring => Err("semantic assets are still being acquired".into()),
        RequiredSemanticState::Rebuilding => Err("semantic indexing is still in progress".into()),
        RequiredSemanticState::Failed(reason) => Err(format!("semantic retrieval unavailable: {reason}")),
    }
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test semantic_execution`
Expected after the change: queued jobs actually finish in the daemon; imported documents are retrievable after activation and restart.

Add a real daemon test with a test-injected embedding provider to cover import→queue→active generation→recall, cancel/retry, stale lease, worker failure, concurrent import, restart reconciliation, and SIGTERM join. The real model version of the same flow is required in S3/F2; the fake-provider test is not final semantic evidence. Reuse the existing service's synchronization boundary; do not share a non-Send ONNX session unsafely. Add a typed status deserializer test proving an FTS-only lane is not healthy for the required semantic readiness gate.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/daemon/semantic_worker.rs crates/hiero/src/daemon/mod.rs crates/hiero/src/application/mod.rs crates/hiero/src/application/memory.rs crates/hiero/src/main.rs crates/hiero/src/daemon/rest/status.rs crates/hieronymus/src/semantic_jobs.rs crates/hieronymus/src/semantic_arming.rs crates/hieronymus/src/semantic_recall.rs crates/hiero/tests/semantic_execution.rs
git commit -m "feat: execute semantic jobs and arm daemon recall"
```

### Task S3: Validate real hybrid relevance and centralize ANN predicates

**Coverage:** Astra 7–8 evidence gap; Sonnet 3.4 hardening; owner requires memory and RAG.

**Dependencies:** S2, M3, D5.

**Files:**

- Modify: `crates/hieronymus/src/semantic_index.rs`
- Create: `crates/hiero/tests/fixtures/hybrid-relevance.json`
- Test: `crates/hieronymus/tests/semantic_predicates.rs`
- Test: `crates/hiero/tests/semantic_real.rs`
- Create: `docs/semantic-validation.md`
- Modify: `qualification/records/semantic-native.json`

**Interfaces:** Produce `semantic_index::series_predicate(&str) -> Result<String,SemanticError>` using existing validate_slug. Real test accepts HIERO_TEST_ONNX_RUNTIME and HIERO_TEST_MODEL_DIR filesystem paths, starts an isolated built daemon, and uses normal authenticated MCP/CLI operations. `hybrid-relevance.json` stores documents/memories, queries, expected top-3 IDs, excluded series and terminology expectations; no production bypass routes.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hieronymus::semantic_index::series_predicate;
#[test]
fn predicate_rejects_control_and_quote_injection() {
    assert_eq!(series_predicate("book-one").unwrap(), "series_slug = 'book-one'");
    for slug in ["x' OR true", "x;drop", "x\ny", ""] {
        assert!(series_predicate(slug).is_err(), "{slug:?}");
    }
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hieronymus --test semantic_predicates`
Expected before the change: predicate construction is duplicated; this is hardening, not a demonstrated current injection vulnerability.

- [ ] **Step 3: Implement the bounded change**

Use one validated predicate builder for both ANN search and affected reads/deletes. Preserve strict slug validation; do not introduce a nonexistent parameter-binding API. Build a relevance corpus with semantic-only paraphrases, lexical hits, near-topic distractors, duplicated sources and foreign-series documents. Initial exact example: RAG 'The physician treated the wounded sailor aboard the vessel.'; distractor 'The carpenter repaired the cabin door.'; query 'Which doctor cared for the injured seaman on the ship?' must return the physician source in top3 with semantic provenance. Add learned-memory recall and an active term with no ranked hit; the deterministic_contract still contains that term. Extend to the project's actual source/target-language examples before release, recording observed limits instead of assuming this English MiniLM is multilingual. Use real model inference and normal daemon wiring; synthetic harness vectors cannot meet this gate. Update the qualification record with artifact identities and actual results only after the run.

```rust
pub fn series_predicate(series_slug: &str) -> Result<String, SemanticError> {
    validate_slug(series_slug)?;
    Ok(format!("series_slug = '{series_slug}'"))
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hieronymus --test semantic_predicates`
Expected after the change: all predicate rejection cases pass; separately, the real semantic suite establishes actual retrieval.

Required run: `cargo test -p hiero --test semantic_real -- --ignored --nocapture` with the two explicit asset-path environment variables. This test must fail, not skip/pass, if requested assets are absent. Assertions: every curated expected source is top3; no foreign-series hit; provenance includes actual semantic contribution for semantic-only queries; learned memory is returned; active terminology remains independent; byte-fold generations are rejected; corrupt model/runtime fails readiness. Record model/tokenizer hashes and complete corpus outcomes in docs/semantic-validation.md. Do not weaken expected relevance or remove failing languages merely to turn the gate green; any model replacement is a separately qualified identity change.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hieronymus/src/semantic_index.rs crates/hiero/tests/fixtures/hybrid-relevance.json crates/hieronymus/tests/semantic_predicates.rs crates/hiero/tests/semantic_real.rs docs/semantic-validation.md qualification/records/semantic-native.json
git commit -m "test: qualify real memory and semantic rag retrieval"
```

## Plan self-review

Coverage is mapped in the coordinator. Every task above has a regression, implementation sketch, explicit interfaces, and a focused verification command. Execute prerequisites first; snippets using newly introduced APIs intentionally fail to compile before those APIs land. Do not interpret a passing compile as the behavioral green step.

