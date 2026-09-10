# Merged Rust Port Correctness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repair the confirmed data-loss, ownership, semantic readiness/recovery, audit, scheduling and integration defects before release work.

**Architecture:** Preserve existing domain stores and controllers. Strengthen transactional boundaries and make readiness/retry state reflect durable evidence; consolidate duplicated client behavior.

**Tech Stack:** Rust 1.96 / edition 2024, existing SQLite/rusqlite, LanceDB/ort/tokenizers pins, Svelte 5 and Bun 1.4.0.

**Spec:** [Merged implementation review](../../astra-implementation-review-2026-09-06.md); [ADR 0016](../../adr/0016-autonomous-story-memory-product-vision.md), and ADRs 0008–0015 as amended.

## Global Constraints

- Owner: both working memory and semantic RAG are mandatory. No FTS-only completion or release alternative.
- ADR 0016: “There is no mandatory human review, approval inbox, or recurring curation step in this loop.” Keep structured validation, revision checks, audit and priority of explicit user corrections. Do not merely remove existing guards without a replacement authority design.
- Ordinary mutations go through the daemon; one data-root owner outlives every writer. Offline upgrade/recovery remains exclusive.
- Linux target `x86_64-unknown-linux-gnu`; MCP revision `2026-07-28`. Preserve frozen historical fixtures and use versioned Rust expectations for accepted deltas.
- Retain plaintext local credential policy, Host/Origin/session checks and the separate-CSRF waiver.
- Preserve unrelated dirty documentation, original plans and reports. No publication, live data migration or user-service replacement is authorized by execution of unit/process tests.
- Keep current native dependency pins unless measured P2 acceptance requires a separately qualified model change. No extra fixed-count or attestation infrastructure.

---

## File structure and execution order

Execute C1 → C2 → C3 → C4 → C5 → C6 → C7 → C8 → C9. C1 owns export; C2 worker lifetime; C3–C6 semantic state and API integration; C7 graph transactions/projections; C8 Dream bounds; C9 client consolidation. C4 introduces schema v3 for its corpus revision/outbox and C8 retry state together, through ordered migrations. The accepted ADR 0016 policy design is P1 in the companion plan; these correctness repairs do not invent a new rule authority.

### Task C1: Protect export destinations and take one coherent snapshot

**Coverage:** A1 (High, new M5 data loss).

**Dependencies:** None.

**Files:**

- Modify: `crates/hiero/src/export.rs`
- Create: `crates/hiero/tests/export_safety.rs`

**Interfaces:** Existing export::run signature stays unchanged; add ExportError::UnsafeDestination(PathBuf). Export remains an explicitly read-only diagnostic operation, with no daemon write authority.

- [ ] **Step 1: Write the failing regression.**

```rust
use hieronymus::data_root::HieronymusConfig;
#[test]
fn export_refuses_authoritative_database() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let _app = hiero::application::Application::open(&config).unwrap();
    let before = std::fs::read(config.database_path()).unwrap();
    assert!(hiero::export::run(&config, &config.database_path()).is_err());
    assert_eq!(std::fs::read(config.database_path()).unwrap(), before);
}
```

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p hiero --test export_safety --locked`. Expected before implementation: export returns success and replaces the SQLite file with JSON.

- [ ] **Step 3: Implement the change.**

Reject authoritative/runtime destinations before creating parents or writing: database, WAL/SHM, configuration, token/discovery, ownership lock, managed model/index/backup/plugin assets. Resolve existing file identity to catch symlink/hard-link aliases; deny writing through symlink components. Prefer a new destination and require a deliberate separate overwrite option for existing exports. Create a sibling temporary file, flush and sync, then atomically publish without clobbering an existing destination. Hold one read transaction for all table queries; do not serialize different committed moments. Keep the documented content-export scope explicit rather than advertising a complete backup.

```rust
// Start one read snapshot before iterating EXPORTED_TABLES.
connection.execute_batch("BEGIN DEFERRED")?;
// Run all export_table calls on this same connection before:
connection.execute_batch("COMMIT")?;
// Publish with tempfile::NamedTempFile::persist_noclobber only after
// validating the destination and syncing the temporary file.
```

- [ ] **Step 4: Verify behavior, including the real boundary.**

Run: `cargo test -p hiero --test export_safety --locked`. Expected: database path is rejected without byte changes; ordinary export succeeds.

Add symlink and hard-link aliases, existing configuration/lock paths, failed publication and concurrent writer cases. Independently query foreign-key-related exported tables to establish one snapshot. Confirm failure never truncates an existing file; metadata checks alone must not be followed by a clobbering write vulnerable to replacement.

- [ ] **Step 5: Review and commit only this task.**

```bash
git add crates/hiero/src/export.rs crates/hiero/tests/export_safety.rs
git commit -m "fix: protect authoritative files from export"
```

### Task C2: Own startup failures and reap completed workers

**Coverage:** A2 High integration regression; A9 Medium retention.

**Dependencies:** C1; independent of semantic policy.

**Files:**

- Modify: `crates/hiero/src/daemon/mod.rs`
- Modify: `crates/hiero/src/daemon/workers.rs`
- Create: `crates/hiero/tests/worker_lifetime.rs`

**Interfaces:** Add WorkerGroup::reap_finished(&self) -> Result<usize,String>, returning joined finished handles; keep live handles admitted until joined. Add internal StartupGuard owning stop, WorkerGroup and RootOwnership until transferred to Daemon. Guard drop signals and joins before releasing ownership.

- [ ] **Step 1: Write the failing regression.**

```rust
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;
use hiero::daemon::workers::WorkerGroup;
#[test]
fn finished_work_is_reaped_without_stopping_admission() {
    let workers = WorkerGroup::new(Arc::new(AtomicBool::new(false)));
    for _ in 0..100 { workers.spawn(Box::new(|_| {})).unwrap(); }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(workers.reap_finished().unwrap(), 100);
    assert_eq!(workers.live_count(), 0);
    workers.spawn(Box::new(|_| {})).unwrap();
    workers.stop_and_join().unwrap();
}
```

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p hiero --test worker_lifetime --locked`. Expected before implementation: reaping API is absent; current handles remain until shutdown.

- [ ] **Step 3: Implement the change.**

Perform bind and credential validation before starting workers where practical. Protect every remaining post-spawn fallible step with StartupGuard, including discovery publication and controller setup. Reap finished connection handles periodically without stopping admission; join outside the handle mutex, preserve panic diagnostics, and prevent concurrent shutdown callers from observing an empty list while another caller still joins. Keep root ownership until all writers finish even on panic/error.

```rust
// Core partition inside reap_finished; collect handles under the mutex,
// then drop the mutex guard before joining.
let mut handles = self.handles.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
let mut finished = Vec::new();
let mut index = 0;
while index < handles.len() {
    if handles[index].is_finished() {
        finished.push(handles.swap_remove(index));
    } else {
        index += 1;
    }
}
```

- [ ] **Step 4: Verify behavior, including the real boundary.**

Run: `cargo test -p hiero --test worker_lifetime --locked`. Expected: completed work is removed and new admission still works; failed startup leaves no surviving writer.

Port the review's occupied-port/counted SemanticArm reproduction into a negative regression: after Daemon::start fails, no arm work may occur after ownership is reacquired. Inject token/discovery/controller failures too. Test join panic, simultaneous stop requests, long-running writer and nested spawn. Count retained handles over repeated real HTTP requests instead of benchmarking only synthetic jobs.

- [ ] **Step 5: Review and commit only this task.**

```bash
git add crates/hiero/src/daemon/mod.rs crates/hiero/src/daemon/workers.rs crates/hiero/tests/worker_lifetime.rs
git commit -m "fix: retain ownership through failed startup cleanup"
```

### Task C3: Make semantic readiness depend on installed, verified service state

**Coverage:** A3 High; R4 gate integration.

**Dependencies:** C2.

**Files:**

- Modify: `crates/hiero/src/daemon/semantic_worker.rs`
- Modify: `crates/hiero/src/application/mod.rs`
- Modify: `crates/hiero/src/update.rs`
- Modify: `crates/hiero/tests/semantic_execution.rs`

**Interfaces:** Add public ReadinessEvidence with public fields { query_installed: bool, corpus_empty: bool, generation_valid: bool, rebuild_pending: bool } and readiness_from_evidence(&ReadinessEvidence) -> RequiredSemanticState. Change install callback to Fn(SemanticLane) -> Result<(),String>. Generation validation consumes identity, index integrity and C4 corpus revision; never accept only manifest existence.

- [ ] **Step 1: Write the failing regression.**

```rust
use hiero::daemon::semantic_worker::{ReadinessEvidence, readiness_from_evidence, RequiredSemanticState};
#[test]
fn ready_requires_an_installed_query_lane() {
    let evidence = ReadinessEvidence {
        query_installed: false, corpus_empty: true,
        generation_valid: false, rebuild_pending: false,
    };
    assert_ne!(readiness_from_evidence(&evidence), RequiredSemanticState::Ready);
}
```

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p hiero --test semantic_execution --locked`. Expected before implementation: the evidence API is absent; the counted-arm failure currently reaches Ready with no installed query lane.

- [ ] **Step 3: Implement the change.**

Install the query lane successfully before advertising Ready. Propagate second/third arm failures; retain previous valid lane only with explicit generation/identity evidence. On cancellation verify an older generation or empty corpus; otherwise report rebuilding/failed. Surface reconcile/query/sample errors rather than defaulting to empty jobs or ready. Make query capability/readiness derive from one synchronized state so update sees the same service that requests use.

```rust
pub struct ReadinessEvidence {
    pub query_installed: bool, pub corpus_empty: bool,
    pub generation_valid: bool, pub rebuild_pending: bool,
}

pub fn readiness_from_evidence(e: &ReadinessEvidence) -> RequiredSemanticState {
    if !e.query_installed {
        return RequiredSemanticState::Failed("query lane is not installed".into());
    }
    if e.rebuild_pending { return RequiredSemanticState::Rebuilding; }
    if e.corpus_empty || e.generation_valid { RequiredSemanticState::Ready }
    else { RequiredSemanticState::Failed("no valid current semantic generation".into()) }
}
```

- [ ] **Step 4: Verify behavior, including the real boundary.**

Run: `cargo test -p hiero --test semantic_execution --locked`. Expected: missing query installation never reports Ready; cancellation and stale manifests produce truthful state.

Extend the existing SemanticArm seam: first arm succeeds, second fails, callback count stays zero, controller must fail. Test first-generation cancellation, missing index directory, byte-fold manifest at startup, corrupt state, and failed query-lane replacement. Run update_flow tests proving all these states reject activation; preserve transient acquiring/rebuilding polling.

- [ ] **Step 5: Review and commit only this task.**

```bash
git add crates/hiero/src/daemon/semantic_worker.rs crates/hiero/src/application/mod.rs crates/hiero/src/update.rs crates/hiero/tests/semantic_execution.rs
git commit -m "fix: derive semantic readiness from service evidence"
```

### Task C4: Persist import indexing intent and reconcile corpus identity

**Coverage:** A4 High; equal-count queue dedup, stale startup and missed commit/enqueue window.

**Dependencies:** C3.

**Files:**

- Create: `crates/hieronymus/migrations/003-runtime-recovery.sql`
- Modify: `crates/hieronymus/src/schema_upgrade.rs`
- Modify: `crates/hieronymus/src/db.rs`
- Modify: `crates/hieronymus/src/rag.rs`
- Modify: `crates/hieronymus/src/semantic_recall.rs`
- Modify: `crates/hieronymus/src/semantic_jobs.rs`
- Modify: `crates/hiero/src/daemon/semantic_worker.rs`
- Modify: `crates/hiero/src/application/memory.rs`
- Create: `crates/hiero/tests/semantic_recovery.rs`

**Interfaces:** Introduce monotonic corpus revision stored in SQLite and captured by semantic jobs/generations. New same_build_request(current_revision:i64, queued_revision:i64, current_identity:&EmbeddingIdentity, queued_identity:&EmbeddingIdentity)->bool replaces count-only equivalence. Import writes durable semantic_work_intent inside its authoritative transaction. C8 consumes dream_retry_state in this same v3 migration.

- [ ] **Step 1: Write the failing regression.**

```rust
use hieronymus::semantic_recall::same_build_request;
use hieronymus::semantic_embeddings::{EmbeddingProvider, FakeEmbeddingProvider};
#[test]
fn equal_chunk_count_does_not_equate_different_corpus_revisions() {
    let identity = FakeEmbeddingProvider::new(384).identity().clone();
    assert!(!same_build_request(9, 8, &identity, &identity));
    assert!(same_build_request(9, 9, &identity, &identity));
}
```

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p hiero --test semantic_recovery --locked`. Expected before implementation: queue comparison uses expected_count rather than corpus revision/identity.

- [ ] **Step 3: Implement the change.**

Increment corpus revision and upsert indexing intent in the same transaction as text/identity-affecting RAG changes. Worker may consume/coalesce intents after commit; crashes cannot lose them. Capture revision and identity in each candidate; activation checks both against current authoritative state. Startup/periodic bounded checks compare active coverage with the current revision, queue missing work, and invalidate old byte-fold or missing-index generations. Failed enqueue is visible as pending/error, not silently dropped. Add v2→v3 upgrade while keeping original SQL immutable and extending required-schema classification.

```sql
CREATE TABLE corpus_revision (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  revision INTEGER NOT NULL
);
INSERT INTO corpus_revision VALUES (1, 0);
CREATE TABLE semantic_work_intent (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  revision INTEGER NOT NULL,
  requested_at TEXT NOT NULL
);
ALTER TABLE semantic_generations ADD COLUMN corpus_revision INTEGER NOT NULL DEFAULT -1;
ALTER TABLE dream_link_batches ADD COLUMN next_left_offset INTEGER NOT NULL DEFAULT 0;
ALTER TABLE dream_link_batches ADD COLUMN next_right_offset INTEGER NOT NULL DEFAULT 1;
CREATE TABLE dream_retry_state (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  failures INTEGER NOT NULL,
  next_attempt_at TEXT,
  config_fingerprint TEXT NOT NULL
);
INSERT INTO dream_retry_state VALUES (1, 0, NULL, '');
UPDATE hieronymus_meta SET schema_version = 3;
PRAGMA user_version = 3;
```

- [ ] **Step 4: Verify behavior, including the real boundary.**

Run: `cargo test -p hiero --test semantic_recovery --locked`. Expected: same-count changed content/identity queues new work; startup repairs missed indexing intent.

Fault-inject immediately after import commit before in-memory notification, restart with an older active index, and prove new text becomes semantically retrievable without another import. Test replace/remove/add with unchanged count, changed identity, deleted index, concurrent imports, and v2→v3 upgrade with real rows. Existing jobs with revision -1 must rebuild, never relabel. Implement same_build_request as `current_revision == queued_revision && current_identity == queued_identity`. Keep inference outside write transactions.

- [ ] **Step 5: Review and commit only this task.**

```bash
git add crates/hieronymus/migrations/003-runtime-recovery.sql crates/hieronymus/src/schema_upgrade.rs crates/hieronymus/src/db.rs crates/hieronymus/src/rag.rs crates/hieronymus/src/semantic_recall.rs crates/hieronymus/src/semantic_jobs.rs crates/hiero/src/daemon/semantic_worker.rs crates/hiero/src/application/memory.rs crates/hiero/tests/semantic_recovery.rs
git commit -m "fix: persist semantic indexing intent with imports"
```

### Task C5: Connect public RAG search and propagate unavailable semantics

**Coverage:** A5 High; M2/S2 completion contract.

**Dependencies:** C3–C4.

**Files:**

- Modify: `crates/hiero/src/application/memory.rs`
- Modify: `crates/hiero/src/application/mod.rs`
- Modify: `crates/hieronymus/src/recall.rs`
- Modify: `crates/hieronymus/src/semantic_recall.rs`
- Modify: `crates/hiero/tests/semantic_execution.rs`
- Modify: `crates/hiero/tests/semantic_real.rs`
- Create: `compatibility/rust/rag-search-v2.json`
- Modify: `compatibility/README.md`

**Interfaces:** Add Application::search_rag(&self, series:&str, query:&str, limit:usize)->Result<Value,AppError>; uses actual semantic/hybrid retrieval without requiring a synthetic task session. Mixed recall consumes shared semantic availability and emits WARNING_SEMANTIC_UNAVAILABLE whenever required semantics did not execute.

- [ ] **Step 1: Write the failing regression.**

```rust
use hiero::application::Application;
use hieronymus::data_root::HieronymusConfig;
use serde_json::json;
#[test]
fn rag_search_rejects_an_unavailable_required_semantic_service() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call("hieronymus_series_create", &json!({"slug":"book","title":"Book"}), "test").unwrap();
    assert!(app.call("hieronymus_rag_search",
        &json!({"series_slug":"book","query":"physician","limit":3}), "test").is_err());
}
```

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p hiero --test semantic_execution --locked`. Expected before implementation: rag_search returns a successful lexical array without checking readiness.

- [ ] **Step 3: Implement the change.**

Move standalone RAG search onto the shared semantic/hybrid service; preserve source provenance and series filtering. Keep result rows compatible or version a deliberate envelope change in the new fixture. Empty corpus with loaded assets is valid; unavailable assets are an explicit error for strict search. Mixed recall retains memory and deterministic terminology but emits a structured incomplete-semantic warning. Do not make documentation or a fixture substitute for runtime behavior.

```rust
// Inside the public tool adapter, before invoking the search service:
application.search_rag(&args.series_slug, &args.query, args.limit as usize)
// Inside mixed recall, semantic absence produces WARNING_SEMANTIC_UNAVAILABLE;
// it is never interpreted as every required lane having run cleanly.
```

- [ ] **Step 4: Verify behavior, including the real boundary.**

Run: `cargo test -p hiero --test semantic_execution --locked`. Expected: strict search refuses disarmed service; ready search contributes real semantic results.

Extend the actual ONNX corpus to call hieronymus_rag_search for the physician/seaman paraphrase, not only recall. Assert semantic provenance and foreign-series exclusion. Start without runtime and verify mixed recall warnings plus independent contract, then arm and verify the warning disappears. Run real suite explicitly with verified assets.

- [ ] **Step 5: Review and commit only this task.**

```bash
git add crates/hiero/src/application/memory.rs crates/hiero/src/application/mod.rs crates/hieronymus/src/recall.rs crates/hieronymus/src/semantic_recall.rs crates/hiero/tests/semantic_execution.rs crates/hiero/tests/semantic_real.rs compatibility/rust/rag-search-v2.json compatibility/README.md
git commit -m "fix: connect public rag search to required semantics"
```

### Task C6: Make semantic configuration reload the running daemon truthfully

**Coverage:** A6 High; semantic.conf validation and mutation ownership.

**Dependencies:** C3–C5.

**Files:**

- Modify: `crates/hiero/src/main.rs`
- Modify: `crates/hiero/src/daemon/rest/mod.rs`
- Modify: `crates/hiero/src/daemon/semantic_worker.rs`
- Modify: `crates/hieronymus/src/semantic_arming.rs`
- Modify: `crates/hieronymus/src/state_classifier.rs`
- Modify: `crates/hiero/tests/semantic_cli.rs`
- Modify: `crates/hiero/tests/semantic_execution.rs`

**Interfaces:** Add authenticated native POST /semantic/configure with {runtime_library:String}; response {state,detail,configuration_revision}. SemanticController::reload_configuration(&self)->Result<(),String> sends a worker control message; worker re-resolves the current configuration. load_runtime_library becomes Result<Option<PathBuf>,SemanticConfigError> with Missing distinct from malformed.

- [ ] **Step 1: Write the failing regression.**

```rust
use hieronymus::{data_root::HieronymusConfig, semantic_arming::load_runtime_library};
#[test]
fn malformed_semantic_configuration_is_not_absence() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    std::fs::write(root.path().join("semantic.conf"), "runtime_library = [").unwrap();
    assert!(load_runtime_library(&config).is_err());
}
```

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p hiero --test semantic_cli --test semantic_execution --locked`. Expected before implementation: loader returns Option and collapses parse failure; live enable cannot replace captured UnconfiguredArm.

- [ ] **Step 3: Implement the change.**

Route ordinary configuration changes through the owner daemon and acknowledge its actual state. Re-resolve the arm after a configuration change; invalidate installation state during reload and retry boundedly. Asset acquisition can stage checksum-verified files, but final promotion/configuration is owner-controlled. Resolve relative runtime paths against an explicit stable base or persist canonical absolute paths. If ort's process-global failed load prevents repair, report restart-required and perform only an explicit supervised restart path; do not endlessly retry a poisoned runtime or claim success from an independent CLI load.

```rust
#[derive(Debug, thiserror::Error)]
pub enum SemanticConfigError {
    #[error("cannot read semantic configuration: {0}")]
    Read(#[from] std::io::Error),
    #[error("invalid semantic configuration: {0}")]
    Invalid(String),
}
```

- [ ] **Step 4: Verify behavior, including the real boundary.**

Run: `cargo test -p hiero --test semantic_cli --test semantic_execution --locked`. Expected: malformed config is explicit; enable changes the running service or truthfully reports restart-required.

Repeat the review's real sequence: start daemon without runtime, enable with verified assets, query that same daemon after acknowledgement; it must reach Ready or explicitly complete a supervised restart. Test changed valid path, invalid path, CLI cancellation, restart persistence and failed prior ort load. Do not return lane=armed solely because the CLI can load it.

- [ ] **Step 5: Review and commit only this task.**

```bash
git add crates/hiero/src/main.rs crates/hiero/src/daemon/rest/mod.rs crates/hiero/src/daemon/semantic_worker.rs crates/hieronymus/src/semantic_arming.rs crates/hieronymus/src/state_classifier.rs crates/hiero/tests/semantic_cli.rs crates/hiero/tests/semantic_execution.rs
git commit -m "fix: reload semantic configuration through the daemon"
```

### Task C7: Commit complete concept merges and preserve proposal evidence

**Coverage:** A7 High; A13 projection/variant gap.

**Dependencies:** C4; autonomous authority semantics are specified by P1.

**Files:**

- Modify: `crates/hieronymus/src/concepts.rs`
- Modify: `crates/hiero/src/application/admin/actions.rs`
- Modify: `crates/hiero/src/application/admin/views.rs`
- Modify: `crates/hiero/tests/admin_actions.rs`
- Modify: `crates/hiero/tests/admin_views.rs`

**Interfaces:** Add ConceptStore::merge_concepts_in_transaction(transaction:&rusqlite::Transaction<'_>, source_id:i64, target_id:i64, reason:&str)->Result<ConceptRecord,ConceptError>. Existing merge_concepts owns a transaction and delegates to it. Keep the existing strict_concept_proposals identity: D already writes its normalized proposals there. No second proposal store or identity discriminator is required by current evidence.

- [ ] **Step 1: Write the failing regression.**

```rust
use hieronymus::data_root::HieronymusConfig;
use serde_json::json;
#[test]
fn concept_merge_rolls_back_when_audit_fails() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let app = hiero::application::Application::open(&config).unwrap();
    let mut ids = Vec::new();
    for name in ["Target", "Source"] {
        ids.push(app.call("hieronymus_concept_create",
            &json!({"canonical_name":name}), "test").unwrap()["id"].as_i64().unwrap());
    }
    let db = rusqlite::Connection::open(config.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER reject_audit BEFORE INSERT ON audit_log
        BEGIN SELECT RAISE(ABORT, 'audit failure'); END;").unwrap();
    let before: String = db.query_row("SELECT status FROM concepts WHERE id=?1",
        [ids[1]], |r| r.get(0)).unwrap();
    assert!(hiero::application::admin::run_action(&app, "test", "merge_selected",
        &json!({"view":"Concepts","ids":ids,"confirmed":true,"text":"merged"})).is_err());
    let after: String = db.query_row("SELECT status FROM concepts WHERE id=?1",
        [ids[1]], |r| r.get(0)).unwrap();
    assert_eq!(after, before);
}
```

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p hiero --test admin_actions --test admin_views --locked`. Expected before implementation: source remains merged after audit failure.

- [ ] **Step 3: Implement the change.**

Move the existing domain merge logic into a transaction-taking primitive, preserving aliases/facets, links, scopes and FTS. Admin preflight and every selected source merge execute inside one immediate transaction, followed by one audit insert and commit. Reject duplicate IDs before writes. Preserve approved/forbidden variants and rationale from strict_concept_proposals when materializing the concept/facets; D already writes to this same authoritative proposal store. Any automatic rule decision must later use P1's structured authority, not these projection tables.

```rust
// Within the admin merge transaction, using prevalidated source IDs:
for source_id in sources {
    ConceptStore::merge_concepts_in_transaction(
        &transaction, *source_id, target, "merged from admin contract"
    ).map_err(domain)?;
}
// Existing write_audit executes on this same transaction; commit only after it succeeds.
```

- [ ] **Step 4: Verify behavior, including the real boundary.**

Run: `cargo test -p hiero --test admin_actions --test admin_views --locked`. Expected: audit failure or a late source failure rolls back all domain/projection changes.

Inject failure on second merge and audit append; assert source statuses, target facets, redirects, links and audit are unchanged. Exercise conflicting canonical facets and repeated IDs. Seed a Dream-produced strict proposal and verify it is listed and its variant evidence survives materialization. Do not make this optional diagnostic UI a required human queue under ADR 0016.

- [ ] **Step 5: Review and commit only this task.**

```bash
git add crates/hieronymus/src/concepts.rs crates/hiero/src/application/admin/actions.rs crates/hiero/src/application/admin/views.rs crates/hiero/tests/admin_actions.rs crates/hiero/tests/admin_views.rs
git commit -m "fix: make concept merges atomic with their audit"
```

### Task C8: Bound Dream retries, deterministic draining and pair generation

**Coverage:** A8 High; A10/A11 Medium.

**Dependencies:** C4's v3 retry/cursor schema.

**Files:**

- Modify: `crates/hiero/src/daemon/dream_worker.rs`
- Modify: `crates/hieronymus/src/dreaming.rs`
- Modify: `crates/hieronymus/src/dream_link_progress.rs`
- Modify: `crates/hiero/tests/dream_execution.rs`
- Modify: `crates/hieronymus/tests/dream_drain.rs`
- Modify: `crates/hieronymus/tests/dream_link_progress.rs`

**Interfaces:** Add dream_worker::retry_delay(failures:u32)->Duration; retry state persisted in dream_retry_state. Extend DrainRecord with work counts or internal DrainProgress {archived_inputs,feedback_events,terminalized_pairs}; total progress sums semantic effects, not selected rows. Persist pair cursor offsets in dream_link_batches rather than materializing every queued pair.

- [ ] **Step 1: Write the failing regression.**

```rust
use hiero::daemon::dream_worker::retry_delay;
use std::time::Duration;
#[test]
fn retry_backoff_is_bounded_and_not_the_two_second_urgent_gate() {
    assert_eq!(retry_delay(0), Duration::ZERO);
    assert_eq!(retry_delay(1), Duration::from_secs(30));
    assert_eq!(retry_delay(2), Duration::from_secs(60));
    assert_eq!(retry_delay(100), Duration::from_secs(1800));
}
```

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p hiero --test dream_execution --locked && cargo test -p hieronymus --test dream_drain --test dream_link_progress --locked`. Expected before implementation: urgent failures have no backoff and progress ignores deterministic-only work.

- [ ] **Step 3: Implement the change.**

Before urgent/interval submission, check persisted next-attempt eligibility keyed by relevant provider/workflow config fingerprint. Use bounded jitter around the deterministic delay; reset after success or configuration repair. Manual retry is explicit/coalesced and does not erase durable failure history. Make drains continue while any eligible work advances, including link-only/feedback-only cycles; when resource budget/cancellation ends work, return pending/interrupted rather than completed-with-hidden-work. Lazily enumerate pairs over a stable activation snapshot with persisted offsets, bounded memory and transaction work. Record terminal effects/audit and cursor advancement atomically. Upgrade existing materialized batches by draining them first; do not reapply terminal pairs.

```rust
pub fn retry_delay(failures: u32) -> std::time::Duration {
    if failures == 0 { return std::time::Duration::ZERO; }
    let seconds = 30_u64.saturating_mul(1_u64 << failures.saturating_sub(1).min(6));
    std::time::Duration::from_secs(seconds.min(1800))
}
```

- [ ] **Step 4: Verify behavior, including the real boundary.**

Run: `cargo test -p hiero --test dream_execution --locked && cargo test -p hieronymus --test dream_drain --test dream_link_progress --locked`. Expected: persistent outages respect retry limits; deterministic-only drain reports truthfully; pair budget also bounds enumeration work.

Use an injected clock, not minute-long sleeps: simulate repeated urgent failures, restart, config change and success. Pair-only backlog with budget1 must advance all eligible pairs without replay and report pending when bounded execution ends. Test 10,000 snapshot members with budget1 without constructing 49,995,000 rows/vectors. Cover zero budget, deleted sources and crash between mutation/cursor updates. Keep observations and corrections usable through outages.

- [ ] **Step 5: Review and commit only this task.**

```bash
git add crates/hiero/src/daemon/dream_worker.rs crates/hieronymus/src/dreaming.rs crates/hieronymus/src/dream_link_progress.rs crates/hiero/tests/dream_execution.rs crates/hieronymus/tests/dream_drain.rs crates/hieronymus/tests/dream_link_progress.rs
git commit -m "fix: bound dream retries and resumable work"
```

### Task C9: Use one authenticated discovery client across normal entry points

**Coverage:** A12 Medium, deferred M/R integration.

**Dependencies:** C2–C6.

**Files:**

- Modify: `crates/hiero/src/daemon_client.rs`
- Modify: `crates/hiero/src/lifecycle.rs`
- Modify: `crates/hiero/src/main.rs`
- Modify: `crates/hiero/src/stdio.rs`
- Modify: `crates/hiero/src/agent_hook.rs`
- Modify: `crates/hiero/tests/tool_completeness.rs`
- Modify: `crates/hiero/tests/runtime_shutdown.rs`

**Interfaces:** Extend lifecycle::DaemonClient with existing call_tool(&self,name:&str,args:&Value)->Result<Value,ClientError> behavior. daemon_client becomes a thin compatibility wrapper or is removed after every caller migrates; one lifecycle::probe governs instance/protocol validation and opt-in service startup.

- [ ] **Step 1: Write the failing regression.**

```rust
use hieronymus::data_root::HieronymusConfig;
#[test]
fn stale_discovery_is_rejected_by_the_tool_client() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let daemon = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.path().into()), port: 0, ..Default::default()
    }).unwrap();
    let saved = std::fs::read(config.daemon_discovery_path()).unwrap();
    daemon.shutdown().unwrap();
    std::fs::write(config.daemon_discovery_path(), saved).unwrap();
    assert!(hiero::daemon_client::DaemonClient::connect(&config).is_err());
}
```

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p hiero --test tool_completeness --test runtime_shutdown --locked`. Expected before implementation: the old client returns Ok after reading stale discovery/token without probing the endpoint.

- [ ] **Step 3: Implement the change.**

Reuse authenticated lifecycle probe and expected instance/protocol checks. Preserve the documented explicit autostart policy for tool-call while implementing it through managed service integration; do not fork an unrelated unsupervised owner. Route tool-call, feedback, stdio and hooks through the same client. Keep stdout clean JSON-RPC and credentials out of errors. Preserve generic tool-call as a valid CLI design; dedicated convenience commands are not required merely to inflate the command list.

```rust
// The compatibility client must delegate discovery/health:
let client = crate::lifecycle::connect(config, start_daemon)?;
// Move the existing call_tool request/envelope code onto this authenticated
// client instead of re-reading discovery and assuming it is live.
```

- [ ] **Step 4: Verify behavior, including the real boundary.**

Run: `cargo test -p hiero --test tool_completeness --test runtime_shutdown --locked`. Expected: stale discovery is rejected or repaired through the permitted managed-start path; all transport cases still pass.

Test a dummy listener reusing the port, a valid credential with mismatching instance ID, protocol mismatch, missing discovery and opt-in versus default behavior through actual tool-call and stdio processes. Re-run the complete 39-tool matrix after the client consolidation.

- [ ] **Step 5: Review and commit only this task.**

```bash
git add crates/hiero/src/daemon_client.rs crates/hiero/src/lifecycle.rs crates/hiero/src/main.rs crates/hiero/src/stdio.rs crates/hiero/src/agent_hook.rs crates/hiero/tests/tool_completeness.rs crates/hiero/tests/runtime_shutdown.rs
git commit -m "fix: unify authenticated daemon client discovery"
```

## Completion review

Check every mapped finding against persisted/process evidence, not only helper tests. Keep test-only code isolated; a pure helper regression is the first RED cycle, not a replacement for the integration assertions. Validate links and cross-task interfaces before handoff.

