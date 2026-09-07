# Rust Domain and MCP Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every advertised tool perform its domain operation and deliver deterministic terminology independently of ranked recall.

**Architecture:** A daemon-owned Application composes existing domain stores. Typed argument adapters preserve the frozen registry while transport serialization and actor identity remain at the authenticated boundary; ADR-backed response/lifecycle deltas get versioned Rust expectations.

**Tech Stack:** Rust 1.96 / edition 2024, rusqlite/SQLite FTS5, existing blocking daemon workers, Svelte 5, TypeScript, Bun 1.4.0; preserve Cargo.lock native pins.

**Spec:** [ADR 0011](../../adr/0011-deterministic-terminology-and-graded-memory.md), [ADR 0015](../../adr/0015-mcp-protocol-and-transport.md), [terminology design](../specs/2026-08-31-rust-terminology-memory-design.md), [compatibility design](../specs/2026-08-31-rust-compatibility-contracts-design.md). Also read [the remediation coordinator](2026-09-05-rust-port-remediation.md) and [the reconciled review](../../astra-report.md).

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

M1 owns Application and common dispatch; M2 owns recall/RAG/workspace adapters; M3 owns structured rule lifecycle; M4 owns graph adapters; M5 owns all CLI/host clients and registry completeness. Domain methods missing from the Rust stores are ported in their adapter task, never replaced by direct projection-table writes. D5 supplies the dream dispatch.

- **M1:** Introduce the application dispatcher and series/session tools — `crates/hiero/src/application/mod.rs`, `crates/hiero/src/application/series_sessions.rs`, `crates/hiero/src/lib.rs`, `crates/hiero/src/daemon/mod.rs`, `crates/hiero/src/daemon/protocol.rs`, `crates/hiero/src/daemon/registry.rs`, `crates/hiero/src/daemon/server.rs`, `crates/hiero/tests/application_series.rs`
- **M2:** Wire memory, RAG, feedback, and the full recall response — `crates/hiero/src/application/memory.rs`, `crates/hiero/src/application/mod.rs`, `crates/hieronymus/src/recall.rs`, `crates/hieronymus/src/terminology.rs`, `compatibility/rust/recall-v2.json`, `compatibility/README.md`, `crates/hiero/tests/application_memory.rs`
- **M3:** Complete explicit structured-rule lifecycle and term tools — `crates/hiero/src/application/terms.rs`, `crates/hiero/src/application/mod.rs`, `crates/hieronymus/src/terminology.rs`, `crates/hieronymus/src/crystals.rs`, `compatibility/rust/rule-lifecycle-v2.json`, `crates/hiero/tests/application_terms.rs`
- **M4:** Complete concept/facet and crystal metadata tool adapters — `crates/hiero/src/application/graph.rs`, `crates/hiero/src/application/mod.rs`, `crates/hieronymus/src/concepts.rs`, `crates/hieronymus/src/crystals.rs`, `crates/hiero/tests/application_graph.rs`
- **M5:** Route CLI and agent integrations through complete daemon capabilities — `crates/hiero/src/main.rs`, `crates/hiero/src/agent_hook.rs`, `crates/hiero/src/stdio.rs`, `crates/hiero/src/application/mod.rs`, `crates/hiero/src/agent_plugins.rs`, `crates/hiero/src/lib.rs`, `compatibility/rust/tool-cases-v1.json`, `crates/hiero/tests/tool_completeness.rs`, `crates/hiero/tests/recall_feedback_surfaces.rs`, `docs/usage.md`, `docs/agent-workflows.md`

## Tasks

### Task M1: Introduce the application dispatcher and series/session tools

**Coverage:** Astra 1; Sonnet 2.1; tools hieronymus_status, hieronymus_series_create, hieronymus_series_init, hieronymus_series_list, hieronymus_series_set_language_tags, hieronymus_session_start, hieronymus_session_complete.

**Dependencies:** R1–R2, R5.

**Files:**

- Create: `crates/hiero/src/application/mod.rs`
- Create: `crates/hiero/src/application/series_sessions.rs`
- Modify: `crates/hiero/src/lib.rs`
- Modify: `crates/hiero/src/daemon/mod.rs`
- Modify: `crates/hiero/src/daemon/protocol.rs`
- Modify: `crates/hiero/src/daemon/registry.rs`
- Modify: `crates/hiero/src/daemon/server.rs`
- Test: `crates/hiero/tests/application_series.rs`

**Interfaces:** Application::open(&HieronymusConfig) -> Result<Application, AppError>; Application::call(&self, &str, &serde_json::Value, &str) -> Result<Value,AppError>, last parameter is authenticated actor. AppError variants Invalid(String), Domain(String), NotImplemented(String). McpRegistry retains schemas; its call now receives &Application and actor. Internal family dispatchers return Option<Result<Value,AppError>> so None means not their tool.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hiero::application::Application;
use hieronymus::data_root::HieronymusConfig;
use serde_json::json;
#[test]
fn series_create_is_visible_to_list() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call("hieronymus_series_create", &json!({"slug":"book","title":"Book"}), "local-user").unwrap();
    let rows = app.call("hieronymus_series_list", &json!({}), "local-user").unwrap();
    assert!(rows.as_array().unwrap().iter().any(|row| row["slug"] == "book"));
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test application_series --locked`
Expected before the change: Application is absent; current registry returns NotPorted.

- [ ] **Step 3: Implement the bounded change**

Application holds the selected config, RecallService, and later a DreamController handle. Each request opens the existing short-lived store APIs, not another daemon or direct CLI path. Decode exactly the input_schema defaults/null rules from mcp.json using serde argument structs, and preserve Python mcp_server.py wrappers where ADRs do not amend them. Implement series_init's project setup behavior from the Python function as well as series_create; do not alias away its side effects. Registry returns MCP content/structuredContent envelopes centrally; map malformed arguments to protocol invalid-params and domain failures to tool error results according to existing protocol fixtures. Status remains fixture-compatible.

```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")] Invalid(String),
    #[error("{0}")] Domain(String),
    #[error("tool is not implemented: {0}")] NotImplemented(String),
}
#[derive(serde::Deserialize)]
struct CreateSeries {
    slug: String,
    title: String,
    #[serde(default)] source_language: String,
    #[serde(default)] target_language: String,
    #[serde(default)] language_tags: Option<Vec<String>>,
}
// In series_sessions::dispatch, decode CreateSeries and call:
// Registry::open(config)?.create_series(&args.slug, &args.title,
//     &args.source_language, &args.target_language, args.language_tags.as_deref()).
// Project each Series field explicitly; do not serialize database connections.
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test application_series --locked`
Expected after the change: create/list/session start/complete perform persisted work; missing required fields fail without writes.

Use Registry::create_series(&str, &str, &str, &str, Option<&[String]>) from registry.rs:57. Add explicit default/null/missing/wrong-type tests; duplicate slug and unknown session cases; test real POST /mcp plus stdio for one series/session workflow using existing independent raw HTTP helpers.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/application/mod.rs crates/hiero/src/application/series_sessions.rs crates/hiero/src/lib.rs crates/hiero/src/daemon/mod.rs crates/hiero/src/daemon/protocol.rs crates/hiero/src/daemon/registry.rs crates/hiero/src/daemon/server.rs crates/hiero/tests/application_series.rs
git commit -m "feat: wire application series and session tools"
```

### Task M2: Wire memory, RAG, feedback, and the full recall response

**Coverage:** Astra 1/13; Sonnet 2.2; tools hieronymus_memory_add, hieronymus_memory_search, hieronymus_short_term_add, hieronymus_short_term_add_batch, hieronymus_feedback, hieronymus_recall, hieronymus_rag_import, hieronymus_rag_search.

**Dependencies:** M1.

**Files:**

- Create: `crates/hiero/src/application/memory.rs`
- Modify: `crates/hiero/src/application/mod.rs`
- Modify: `crates/hieronymus/src/recall.rs`
- Modify: `crates/hieronymus/src/terminology.rs`
- Create: `compatibility/rust/recall-v2.json`
- Modify: `compatibility/README.md`
- Test: `crates/hiero/tests/application_memory.rs`

**Interfaces:** RecallResponse adds deterministic_contract: Vec<ContractTerm>; hits/warnings remain library fields. Serialized recall DTO is {recall_id,deterministic_contract,results,warnings}. memory::dispatch uses Application::call's family signature. Existing WorkspaceStore, RagStore, TranslationContext, ProposeFields and Termbase APIs stay canonical.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hiero::application::Application;
use hieronymus::data_root::HieronymusConfig;
use serde_json::json;
#[test]
fn recall_exposes_contract_even_when_results_are_empty() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call("hieronymus_series_create", &json!({"slug":"book","title":"Book"}), "local-user").unwrap();
    let session = app.call("hieronymus_session_start", &json!({"series_slug":"book"}), "local-user").unwrap();
    let recall = app.call("hieronymus_recall",
        &json!({"session_id":session["id"],"series_slug":"book","query":"absent","limit":1}),
        "local-user").unwrap();
    assert_eq!(recall["deterministic_contract"], json!([]));
    assert!(recall["results"].is_array());
    assert!(recall["recall_id"].is_string());
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test application_memory --locked`
Expected before the change: recall is unimplemented or its DTO lacks deterministic_contract.

- [ ] **Step 3: Implement the bounded change**

Return the already-computed contract, never recompute from selected hits. Add Serde derives or explicit safe DTOs for ContractTerm/RecallWarning only; flatten RecallHit variants to the agreed results rows with tier, IDs, activation ID, score/reasons, tags/scopes and rule markers. Resolve series/session context and reject cross-series session arguments. Port legacy memory_add into a short-term session wrapper, not CrystalStore::add_crystal. Keep correction-text hieronymus_feedback distinct from correlated recall-outcome feedback. RAG import resolves actual file parser/type and persists chunks before notifying S2's index worker; semantic RAG search must use S2’s required semantic lane; before S2 is integrated, report semantic-not-ready explicitly rather than treating lexical-only output as completed RAG. Version the Rust recall expectation and explain ADR 0011 precedence rather than regenerating Python fixtures.

```rust
// End of RecallService::recall:
Ok(RecallResponse {
    recall_id,
    deterministic_contract: contract,
    hits: selected,
    warnings: lane_warnings,
})
// The transport DTO uses the normative results key; the internal field stays hits.
// Serialize every contract term, even if limit=1 removed all advisory hits.
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test application_memory --locked`
Expected after the change: all eight tools work; empty/ranked-limited/degraded recall includes the separate contract.

After M3, seed an approved rule and assert its structured canonical rendering survives limit=1, no fuzzy match, and a conflicting RAG hit. Test batch atomic rejection, wrong session ownership, oversize input, unsupported import type, repeated recall working-copy dedup and activation IDs. Do not silently change default/null semantics.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/application/memory.rs crates/hiero/src/application/mod.rs crates/hieronymus/src/recall.rs crates/hieronymus/src/terminology.rs compatibility/rust/recall-v2.json compatibility/README.md crates/hiero/tests/application_memory.rs
git commit -m "feat: expose deterministic recall contracts and wire memory tools"
```

### Task M3: Complete explicit structured-rule lifecycle and term tools

**Coverage:** Astra 13 and ADR 0011 end-to-end approval; tools hieronymus_termbase_propose, hieronymus_termbase_approve, hieronymus_termbase_contract, hieronymus_termbase_validate, hieronymus_rule_crystal_archive, hieronymus_rule_crystal_validate, hieronymus_rule_crystals_list.

**Dependencies:** R3, M1–M2.

**Files:**

- Create: `crates/hiero/src/application/terms.rs`
- Modify: `crates/hiero/src/application/mod.rs`
- Modify: `crates/hieronymus/src/terminology.rs`
- Modify: `crates/hieronymus/src/crystals.rs`
- Create: `compatibility/rust/rule-lifecycle-v2.json`
- Test: `crates/hiero/tests/application_terms.rs`

**Interfaces:** Add RuleActionRequest {rule_id:i64, action:RuleAction, actor:String, reason:String, expected_revision:i64, idempotency_key:String}; RuleAction is Approve, Archive, Replace {replacement_id:i64}. Termbase::apply_action(&RuleActionRequest) -> Result<TermRule,TermbaseError>. Existing approve becomes a compatibility wrapper; protected projection operations use the structured authority.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hiero::application::Application;
use hieronymus::data_root::HieronymusConfig;
use serde_json::json;
#[test]
fn proposed_term_does_not_enforce_until_explicit_approval() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call("hieronymus_series_create",&json!({"slug":"book","title":"Book",
        "source_language":"ja","target_language":"ru"}),"local-user").unwrap();
    let draft = app.call("hieronymus_termbase_propose",&json!({"series_slug":"book",
        "category":"name","source_text":"猫","canonical_translation":"Кот"}),"local-user").unwrap();
    let args = json!({"series_slug":"book","raw_text":"猫"});
    assert_eq!(app.call("hieronymus_termbase_contract",&args,"local-user").unwrap(), json!([]));
    app.call("hieronymus_termbase_approve",&json!({"series_slug":"book","term_id":draft["id"]}),"local-user").unwrap();
    assert_eq!(app.call("hieronymus_termbase_contract",&args,"local-user").unwrap()[0]["canonical_translation"],"Кот");
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test application_terms --locked`
Expected before the change: term tools are unimplemented; projection-only archive can leave authority active.

- [ ] **Step 3: Implement the bounded change**

Use one SQLite transaction to check expected revision, validate structured forms, mutate lifecycle/projection, and insert term_rule_actions/audit. Same idempotency key with the same canonical request returns its stored result; differing payload is a conflict. Transport supplies local authenticated actor, never accepts an actor override from raw arguments. Calling the explicitly named approval/archive operation is the explicit user operation under the local-owner credential policy; do not let dream/provider code call the approval API. Preserve legacy tool arguments with deterministic internal request keys based on action/rule/current transition for retries, and add optional reason/revision/idempotency parameters only through versioned registry changes supported by ADR 0011. Rule-crystal archive resolves term_rules.rule_crystal_id; it must archive authority and projection together. Unknown or ambiguous links fail, not guess.

```rust
#[derive(Debug, Clone)]
pub enum RuleAction { Approve, Archive, Replace { replacement_id: i64 } }
#[derive(Debug, Clone)]
pub struct RuleActionRequest {
    pub rule_id: i64, pub action: RuleAction,
    pub actor: String, pub reason: String,
    pub expected_revision: i64, pub idempotency_key: String,
}
// In apply_action, canonicalize the request before lookup in term_rule_actions.
// Compare revision before mutation. Persist the action row inside the same
// transaction as term_rules, term_rule_forms, and the crystal projection.
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test application_terms --locked`
Expected after the change: candidate remains advisory; explicit approval enforces; archive/replacement atomically update both projections.

Add revise/retry/conflicting-idempotency and crash-before-commit tests; active rules survive feedback/decay; a dream-generated candidate cannot activate. Check deterministic validation/ambiguity before advisory findings. Keep proposals as candidates; never route approve_proposal to unaudited crystal status writes.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/application/terms.rs crates/hiero/src/application/mod.rs crates/hieronymus/src/terminology.rs crates/hieronymus/src/crystals.rs compatibility/rust/rule-lifecycle-v2.json crates/hiero/tests/application_terms.rs
git commit -m "feat: wire audited structured rule lifecycle"
```

### Task M4: Complete concept/facet and crystal metadata tool adapters

**Coverage:** Astra 1 and incomplete graph access; tools hieronymus_concept_create, hieronymus_concept_get, hieronymus_concept_list, hieronymus_concept_update, hieronymus_concept_archive, hieronymus_concept_merge, hieronymus_concept_rename, hieronymus_concept_semantic_tags_set, hieronymus_concept_facet_add, hieronymus_concept_facet_update, hieronymus_concept_facet_list, hieronymus_concept_facet_set_canonical, hieronymus_crystal_link_concept, hieronymus_crystal_story_scopes_set, hieronymus_crystal_semantic_tags_set, hieronymus_concept_proposals_list.

**Dependencies:** M1, M3 for protected lifecycle boundaries.

**Files:**

- Create: `crates/hiero/src/application/graph.rs`
- Modify: `crates/hiero/src/application/mod.rs`
- Modify: `crates/hieronymus/src/concepts.rs`
- Modify: `crates/hieronymus/src/crystals.rs`
- Test: `crates/hiero/tests/application_graph.rs`

**Interfaces:** graph::dispatch has the M1 family signature. Add ConceptStore::update_facet(i64,&FacetPatch) -> Result<ConceptFacetRecord,ConceptError>; FacetPatch fields mirror optional update schema preserving omitted versus explicit null. Add ConceptStore::list_proposals() -> Result<Vec<Value>,ConceptError> with explicit safe DTO projection from strict_concept_proposals.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hiero::application::Application;
use hieronymus::data_root::HieronymusConfig;
use serde_json::json;
#[test]
fn concept_rename_keeps_identity() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    let concept = app.call("hieronymus_concept_create",&json!({"canonical_name":"Cat"}),"local-user").unwrap();
    app.call("hieronymus_concept_rename",&json!({"concept_id":concept["id"],"new_label":"House Cat"}),"local-user").unwrap();
    let after = app.call("hieronymus_concept_get",&json!({"concept_id":concept["id"]}),"local-user").unwrap();
    assert_eq!(after["id"],concept["id"]);
    assert_eq!(after["canonical_name"],"House Cat");
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test application_graph --locked`
Expected before the change: graph tools return NotPorted.

- [ ] **Step 3: Implement the bounded change**

Map the 16 named tools to ConceptStore/CrystalStore methods listed in the coordinator, using typed input defaults from the frozen schemas. Implement missing update_facet and proposal projection in the domain store rather than SQL in transports. Preserve canonical facet uniqueness, merged redirect behavior, tag/scope side tables and FTS updates transactionally. Port Python mcp_server.py's facade results and ConceptStore behavior as the fallback reference, excluding superseded rule activation. Facet type/kind compatibility gives explicit facet_type precedence over legacy kind as the Python wrapper does.

```rust
#[derive(Default, serde::Deserialize)]
pub struct FacetPatch {
    pub facet_type: Option<String>, pub kind: Option<String>,
    pub value: Option<String>, pub language: Option<String>,
    pub language_tags: Option<Vec<String>>, pub story_scopes: Option<Vec<String>>,
    pub semantic_tags: Option<Vec<String>>, pub confidence: Option<f64>,
    pub source_crystal_id: Option<i64>, pub is_canonical: Option<bool>,
}
// Fields permitting explicit null resets must use a presence-aware wrapper
// at deserialization: Option<Option<T>> with a custom deserialize_with that
// returns Some(Option::<T>::deserialize(deserializer)?). Omitted defaults to None.
// Tests pin which fields Python treats null as unchanged versus cleared.
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test application_graph --locked`
Expected after the change: all 16 adapters return real graph data and preserve identity/canonical/tag constraints.

Add create→facet→canonical→rename→merge→list/get, unknown IDs, foreign facet ownership, invalid confidence, null-versus-omitted updates, archived/merged links and transactional FTS projection tests. Every advertised graph tool must have one successful behavioral case and one domain error case.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/application/graph.rs crates/hiero/src/application/mod.rs crates/hieronymus/src/concepts.rs crates/hieronymus/src/crystals.rs crates/hiero/tests/application_graph.rs
git commit -m "feat: wire concept facet and crystal metadata tools"
```

### Task M5: Route CLI and agent integrations through complete daemon capabilities

**Coverage:** Astra 1/14; headless CLI and generated integration follow-ups; Sonnet 2.1.

**Dependencies:** M1–M4, D5, R5.

**Files:**

- Modify: `crates/hiero/src/main.rs`
- Modify: `crates/hiero/src/agent_hook.rs`
- Modify: `crates/hiero/src/stdio.rs`
- Modify: `crates/hiero/src/application/mod.rs`
- Create: `crates/hiero/src/agent_plugins.rs`
- Modify: `crates/hiero/src/lib.rs`
- Create: `compatibility/rust/tool-cases-v1.json`
- Test: `crates/hiero/tests/tool_completeness.rs`
- Modify: `crates/hiero/tests/recall_feedback_surfaces.rs`
- Modify: `docs/usage.md`
- Modify: `docs/agent-workflows.md`

**Interfaces:** Application::implemented_tools() -> &'static [&'static str] lists concrete handlers, not the snapshot. lifecycle::DaemonClient from R5 drives recall-feedback and headless commands. agent_plugins::render(&HieronymusConfig) -> Result<Vec<(PathBuf,String)>,String> builds installation-owned plugin files using the stable hiero mcp entry point.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hiero::{application::Application, daemon::McpRegistry};
#[test]
fn every_advertised_tool_has_a_concrete_handler() {
    let registry = McpRegistry::embedded();
    let mut expected: Vec<_> = registry.list_tools().iter().map(|t| t.name.as_str()).collect();
    let mut actual = Application::implemented_tools().to_vec();
    expected.sort_unstable(); actual.sort_unstable();
    assert_eq!(actual, expected);
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test tool_completeness --locked`
Expected before the change: handler list incomplete; runtime behavioral matrix exposes NotPorted.

- [ ] **Step 3: Implement the bounded change**

Replace direct FeedbackStore access in run_recall_feedback with DaemonClient::post('/recall/feedback'). Add headless CLI adapters for series/session, recall, dream, RAG import/search, term validation and JSON export using Application over authenticated /mcp or named daemon routes. Export is a read operation with explicit destination; do not copy a live SQLite file. Generated Read/Learn/Remember skills retain English-first guidance, candidate-only ingestion, active-rule validation and stable CLI paths; no source code goes into book workspaces. Provide hiero plugins generate --data-root and hiero export --output with deterministic documented outputs. Generated host configuration uses stdio discovery, not a fixed port or baked-in bearer. Do not rewrite user host configuration automatically.

```rust
let client = hiero::lifecycle::connect(&config, true)?;
let payload = serde_json::json!({
    "recall_id": recall_id,
    "useful": useful,
    "miss": miss,
    "idempotency_key": idempotency_key,
});
let result = client.post("/recall/feedback", &payload)?;
// Render result to the existing CLI JSON/human envelope.
// This path contains no FeedbackStore::open or database connection.
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test tool_completeness --locked`
Expected after the change: all advertised tools execute successfully against seeded real domain state; CLI feedback produces the same daemon audit/results.

tool-cases-v1.json entries contain {name,arguments,expected_subset} and complete setup operations, not canned responses. Execute every entry through real HTTP and stdio; assert persisted mutations by an independent read-only SQLite connection. Verify no /api/mcp calls, stdout-only JSON-RPC, default/null/error contracts and generated plugin byte output. Registry name equality is supplementary, never the sole completion test.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/main.rs crates/hiero/src/agent_hook.rs crates/hiero/src/stdio.rs crates/hiero/src/application/mod.rs crates/hiero/src/agent_plugins.rs crates/hiero/src/lib.rs compatibility/rust/tool-cases-v1.json crates/hiero/tests/tool_completeness.rs crates/hiero/tests/recall_feedback_surfaces.rs docs/usage.md docs/agent-workflows.md
git commit -m "feat: finish headless clients and verify full MCP behavior"
```

## Plan self-review

Coverage is mapped in the coordinator. Every task above has a regression, implementation sketch, explicit interfaces, and a focused verification command. Execute prerequisites first; snippets using newly introduced APIs intentionally fail to compile before those APIs land. Do not interpret a passing compile as the behavioral green step.

