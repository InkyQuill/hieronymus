# Rust Dream execution and durability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run configured Dream workflows through real providers, preserve every accepted mutation with its audit, and drain eligible work without loss or duplicate reinforcement.

**Architecture:** Keep provider resolution and normalized outputs in the domain library. A daemon-owned controller schedules one supervised worker per data root; short SQLite transactions commit domain changes, phase progress, and audit together.

**Tech Stack:** Rust 1.96 / edition 2024, rusqlite/SQLite FTS5, existing blocking daemon workers, Svelte 5, TypeScript, Bun 1.4.0; preserve Cargo.lock native pins.

**Spec:** ADRs 0003, 0005, 0007, 0009–0011; Astra findings 5–6 and 17; Sonnet sections 3.1–3.2. Also read [the remediation coordinator](2026-09-05-rust-port-remediation.md) and [the reconciled review](../../astra-report.md).

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

`dream_workflows.rs` resolves enabled assignments; `dream_output.rs` validates provider output; `dream_link_progress.rs` owns durable pair progress. Existing stores retain graph mutations. `daemon/dream_worker.rs` owns scheduling and manual requests.

- **D1:** Resolve enabled workflows to their configured provider and model — `crates/hieronymus/src/dream_workflows.rs`, `crates/hieronymus/src/lib.rs`, `crates/hieronymus/src/dream_config.rs`, `crates/hieronymus/src/dream_providers.rs`, `crates/hieronymus/src/dreaming.rs`, `crates/hieronymus/tests/dream_workflows.rs`
- **D2:** Normalize all graph outputs and protect approved terminology — `crates/hieronymus/src/dream_output.rs`, `crates/hieronymus/src/lib.rs`, `crates/hieronymus/src/dreaming.rs`, `crates/hieronymus/src/concepts.rs`, `crates/hieronymus/src/crystals.rs`, `crates/hieronymus/tests/dream_output.rs`
- **D3:** Commit phase mutations, completion and audit atomically — `crates/hieronymus/src/dreaming.rs`, `crates/hieronymus/src/dream_audit.rs`, `crates/hieronymus/src/crystals.rs`, `crates/hieronymus/src/concepts.rs`, `crates/hieronymus/tests/dream_transactions.rs`
- **D4:** Persist pair-budget progress across cycles and crashes — `crates/hieronymus/src/dream_link_progress.rs`, `crates/hieronymus/src/lib.rs`, `crates/hieronymus/src/dreaming.rs`, `crates/hieronymus/tests/dream_link_progress.rs`
- **D5:** Run and drain Dream through a supervised daemon controller — `crates/hiero/src/daemon/dream_worker.rs`, `crates/hiero/src/daemon/mod.rs`, `crates/hiero/src/daemon/rest/admin.rs`, `crates/hiero/src/daemon/registry.rs`, `crates/hieronymus/src/dreaming.rs`, `crates/hiero/tests/dream_execution.rs`, `crates/hieronymus/tests/dream_drain.rs`

## Tasks

### Task D1: Resolve enabled workflows to their configured provider and model

**Coverage:** Astra 5–6; Sonnet 2.3; ADR 0007.

**Dependencies:** R1; can precede the other Dream tasks.

**Files:**

- Create: `crates/hieronymus/src/dream_workflows.rs`
- Modify: `crates/hieronymus/src/lib.rs`
- Modify: `crates/hieronymus/src/dream_config.rs`
- Modify: `crates/hieronymus/src/dream_providers.rs`
- Modify: `crates/hieronymus/src/dreaming.rs`
- Test: `crates/hieronymus/tests/dream_workflows.rs`

**Interfaces:** Produce `WorkflowChoice { pub name: String, pub enabled: bool, pub provider: String, pub model: String }`, `enabled_choices(Vec<WorkflowChoice>) -> Result<Vec<WorkflowChoice>, String>`, and `WorkflowResolver::provider(&self, choice: &WorkflowChoice) -> Result<Box<dyn DreamProvider>, DreamError>`. Resolver owns a snapshot of the existing provider catalog. Existing `LlmDreamProvider::new(profile_id, ProviderProfile, model)` constructs each selected provider; deterministic providers remain explicit test injections.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hieronymus::dream_workflows::{enabled_choices, WorkflowChoice};
#[test]
fn disabled_workflow_never_requires_a_provider() {
    let choices = vec![
        WorkflowChoice { name: "consolidation".into(), enabled: false, provider: "".into(), model: "".into() },
        WorkflowChoice { name: "coverage_audit".into(), enabled: true, provider: "local".into(), model: "audit".into() },
    ];
    let selected = enabled_choices(choices).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].model, "audit");
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hieronymus --test dream_workflows`
Expected before the change: new resolver is absent; existing execution iterates disabled workflows.

- [ ] **Step 3: Implement the bounded change**

Translate the configured ordered workflow assignments into choices, applying the configured default only where an assignment is absent. Filter disabled workflows before provider validation and execution. Reject an enabled choice with an unknown provider, blank model, or missing required key. Reject a run whose required coverage audit is disabled before processing any input. Resolve a fresh provider per selected workflow and write its actual profile/model into phase records. Remove the production deterministic-provider construction from admin execution in D5. Keep providers worker-local so existing non-Send test implementations need not be forced across threads.

```rust
pub fn enabled_choices(choices: Vec<WorkflowChoice>) -> Result<Vec<WorkflowChoice>, String> {
    let selected: Vec<_> = choices.into_iter().filter(|choice| choice.enabled).collect();
    if !selected.iter().any(|choice| choice.name == "coverage_audit") {
        return Err("coverage_audit must be enabled before processing memories".into());
    }
    if selected.iter().any(|choice| choice.provider.trim().is_empty() || choice.model.trim().is_empty()) {
        return Err("enabled workflows require a provider and model".into());
    }
    Ok(selected)
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hieronymus --test dream_workflows`
Expected after the change: disabled assignments produce no provider calls; enabled assignments use their own model.

Add a two-profile loopback provider test using the existing ProviderTransport injection: consolidation and coverage_audit return different recorded model names, disabled passes receive zero requests, provider failure preserves pending memory. Use exact configured workflow names from dream_config.rs for production mapping; do not rename the schema to match the small regression.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hieronymus/src/dream_workflows.rs crates/hieronymus/src/lib.rs crates/hieronymus/src/dream_config.rs crates/hieronymus/src/dream_providers.rs crates/hieronymus/src/dreaming.rs crates/hieronymus/tests/dream_workflows.rs
git commit -m "fix: resolve enabled dream workflow assignments"
```

### Task D2: Normalize all graph outputs and protect approved terminology

**Coverage:** Astra Dream output follow-up; ADR 0003 and 0011.

**Dependencies:** D1, M3, M4.

**Files:**

- Create: `crates/hieronymus/src/dream_output.rs`
- Modify: `crates/hieronymus/src/lib.rs`
- Modify: `crates/hieronymus/src/dreaming.rs`
- Modify: `crates/hieronymus/src/concepts.rs`
- Modify: `crates/hieronymus/src/crystals.rs`
- Test: `crates/hieronymus/tests/dream_output.rs`

**Interfaces:** Produce `validate_action_targets(value: &serde_json::Value, allowed: &std::collections::BTreeSet<i64>, active_rules: &std::collections::BTreeSet<i64>) -> Result<(), String>`. Internal typed normalized records mirror the Python `_NormalizedDreamOutput` fields (listed below), with IDs checked against the selected context before store calls. Consume M3's explicit rule lifecycle; Dream has no approval authority.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use std::collections::BTreeSet;
use hieronymus::dream_output::validate_action_targets;
use serde_json::json;
#[test]
fn dream_cannot_supersede_an_active_rule() {
    let allowed = BTreeSet::from([11, 12]);
    let protected = BTreeSet::from([11]);
    let output = json!({"supersede_actions": [
        {"old_crystal_id": 11, "new_crystal_id": 12, "reason": "model suggestion"}
    ]});
    assert!(validate_action_targets(&output, &allowed, &protected).is_err());
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hieronymus --test dream_output`
Expected before the change: normalizer/target validation is absent and output sections are ignored.

- [ ] **Step 3: Implement the bounded change**

Port the complete Python normalized output contract: crystals with concept_names; concept_proposals; concepts(canonical_name,description,tags,confidence_delta); facets(concept_name,value,kind,language_tags,story_scopes,semantic_tags,confidence,is_canonical); supersede_actions(old_crystal_id,new_crystal_id,reason); reinforce_actions(id,confidence_delta,salience_delta); warnings/rejected_entries/skipped_candidates. Validate finite bounded numeric inputs, required names, same-context IDs, and referenced concepts. Reject malformed entries with durable audit details rather than silently dropping entire sections. Model output may propose rules but cannot activate, replace, or archive an active rule. Resolve concept names within the group's context; never use the first group's context for another series. Apply accepted outputs through transaction-aware store methods introduced in D3.

```rust
pub fn validate_action_targets(
    value: &serde_json::Value,
    allowed: &std::collections::BTreeSet<i64>,
    active_rules: &std::collections::BTreeSet<i64>,
) -> Result<(), String> {
    if let Some(actions) = value.get("supersede_actions") {
        let actions = actions.as_array().ok_or("supersede_actions must be an array")?;
        for action in actions {
            for field in ["old_crystal_id", "new_crystal_id"] {
                let id = action.get(field).and_then(serde_json::Value::as_i64)
                    .ok_or_else(|| format!("{field} must be an integer"))?;
                if !allowed.contains(&id) || active_rules.contains(&id) {
                    return Err(format!("dream action is not authorized for crystal {id}"));
                }
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hieronymus --test dream_output`
Expected after the change: protected/out-of-context actions are rejected; valid concepts, facets, links and graded-memory actions persist.

Add one fixture per previously ignored output section and one mixed valid/invalid output. Assert public store projections, canonical facet uniqueness, context isolation, and no implicit rule activation. Preserve warnings and rejected entry reasons without leaking provider credentials. The helper above illustrates target checks; typed normalization must cover reinforcement and all other mutation paths too.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hieronymus/src/dream_output.rs crates/hieronymus/src/lib.rs crates/hieronymus/src/dreaming.rs crates/hieronymus/src/concepts.rs crates/hieronymus/src/crystals.rs crates/hieronymus/tests/dream_output.rs
git commit -m "fix: apply validated dream graph outputs"
```

### Task D3: Commit phase mutations, completion and audit atomically

**Coverage:** Sonnet 3.1; Astra audit follow-up; ADR 0005/0010.

**Dependencies:** R3, D2.

**Files:**

- Modify: `crates/hieronymus/src/dreaming.rs`
- Modify: `crates/hieronymus/src/dream_audit.rs`
- Modify: `crates/hieronymus/src/crystals.rs`
- Modify: `crates/hieronymus/src/concepts.rs`
- Test: `crates/hieronymus/tests/dream_transactions.rs`

**Interfaces:** Produce `dream_audit::commit_audited<T>(conn: &mut rusqlite::Connection, operation: impl FnOnce(&rusqlite::Transaction<'_>) -> rusqlite::Result<T>) -> rusqlite::Result<T>`. Add transaction-taking variants for the existing domain writes and `DreamAuditStore::append_in_transaction` with the existing append arguments plus `&Transaction`; it returns the inserted audit ID and applies existing payload redaction. Transport/service ownership remains unchanged.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hieronymus::dream_audit::commit_audited;
use rusqlite::Connection;
#[test]
fn audit_failure_rolls_back_mutation() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE domain(value INTEGER);
        CREATE TABLE audit(value INTEGER CHECK(value > 0));").unwrap();
    let result = commit_audited(&mut db, |tx| {
        tx.execute("INSERT INTO domain VALUES (1)", [])?;
        tx.execute("INSERT INTO audit VALUES (0)", [])?;
        Ok(())
    });
    assert!(result.is_err());
    let count: i64 = db.query_row("SELECT count(*) FROM domain", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hieronymus --test dream_transactions`
Expected before the change: new primitive is absent; current Dream phases commit domain state before completion/audit.

- [ ] **Step 3: Implement the bounded change**

For deterministic phases, open one immediate write transaction, apply domain updates, mark the phase completed, append the redacted audit, then commit. For LLM phases, perform network calls and parsing outside transactions; revalidate current state inside the application transaction before applying accepted output and its phase/audit. Never call a store method that opens another connection from inside this transaction. Preserve failure audit as a separate failed-phase record after rollback, without claiming domain success. Existing approval transactions are independently governed by M3.

```rust
pub fn commit_audited<T>(
    conn: &mut rusqlite::Connection,
    operation: impl FnOnce(&rusqlite::Transaction<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let result = operation(&tx)?;
    tx.commit()?;
    Ok(result)
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hieronymus --test dream_transactions`
Expected after the change: audit/phase errors leave domain state unchanged; successful writes expose all three records together.

The small transaction test is not sufficient alone. Run a real deterministic phase against a temporary domain DB with a SQLite trigger that aborts INSERT into dream_audit_entries; assert crystal/link changes and phase completion rolled back. Repeat failure injection before phase status and immediately before commit, then retry and verify exactly one semantic effect.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hieronymus/src/dreaming.rs crates/hieronymus/src/dream_audit.rs crates/hieronymus/src/crystals.rs crates/hieronymus/src/concepts.rs crates/hieronymus/tests/dream_transactions.rs
git commit -m "fix: commit dream phases with their audit"
```

### Task D4: Persist pair-budget progress across cycles and crashes

**Coverage:** Sonnet 3.2; activation loss and replay prevention.

**Dependencies:** R3, D3.

**Files:**

- Create: `crates/hieronymus/src/dream_link_progress.rs`
- Modify: `crates/hieronymus/src/lib.rs`
- Modify: `crates/hieronymus/src/dreaming.rs`
- Test: `crates/hieronymus/tests/dream_link_progress.rs`

**Interfaces:** Produce `canonical_pairs(ids: &[i64]) -> Vec<(i64,i64)>`; `LinkProgress::open(&HieronymusConfig) -> Result<Self,DreamError>`; `process(&mut self, cycle: i64, pair_budget: usize) -> Result<usize,DreamError>`. It consumes R3's dream_link_batches/members/pairs tables. The returned count is pairs terminalized in this call, not selected activations.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hieronymus::dream_link_progress::canonical_pairs;
#[test]
fn pairs_are_unique_and_resume_in_stable_order() {
    assert_eq!(canonical_pairs(&[3, 1, 2, 1]), vec![(1, 2), (1, 3), (2, 3)]);
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hieronymus --test dream_link_progress`
Expected before the change: durable pair progress does not exist; current implementation marks every activation consumed after a partial budget.

- [ ] **Step 3: Implement the bounded change**

Snapshot eligible activations into one durable batch per session with UNIQUE activation membership. Materialize unique unordered crystal pairs in deterministic order. Each budgeted pair's reinforcement plus pair status/applied_cycle/result_json plus audit commits atomically using D3. Leave unprocessed pairs queued. Mark a batch's activations consumed only after all pairs are applied or explicitly skipped (deleted/ineligible IDs produce audited skip reasons). On restart resume queued pairs; never recreate completed work from still-unconsumed activations. A zero pair budget consumes nothing. Keep session and context isolation in batch creation.

```rust
pub fn canonical_pairs(ids: &[i64]) -> Vec<(i64, i64)> {
    let unique: Vec<_> = ids.iter().copied().collect::<std::collections::BTreeSet<_>>()
        .into_iter().collect();
    let mut pairs = Vec::new();
    for (index, left) in unique.iter().enumerate() {
        for right in unique.iter().skip(index + 1) {
            pairs.push((*left, *right));
        }
    }
    pairs
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hieronymus --test dream_link_progress`
Expected after the change: all three pairs eventually execute once with budget=1; activations are not consumed after the first pair.

Add a persisted three-activation test: process budget1, reopen store, process budget1 twice, assert three distinct pair effects and no further effect on a fourth call. Inject rollback between link mutation and pair status; retry must apply once. Cover two sessions, deleted crystal, duplicate activation, empty batch and budget0. The canonicalization test is only the first RED cycle.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hieronymus/src/dream_link_progress.rs crates/hieronymus/src/lib.rs crates/hieronymus/src/dreaming.rs crates/hieronymus/tests/dream_link_progress.rs
git commit -m "fix: persist dream reinforcement progress"
```

### Task D5: Run and drain Dream through a supervised daemon controller

**Coverage:** Astra 5/17; scheduler, manual admin, MCP dream and shutdown.

**Dependencies:** R5, D1–D4, M1.

**Files:**

- Create: `crates/hiero/src/daemon/dream_worker.rs`
- Modify: `crates/hiero/src/daemon/mod.rs`
- Modify: `crates/hiero/src/daemon/rest/admin.rs`
- Modify: `crates/hiero/src/daemon/registry.rs`
- Modify: `crates/hieronymus/src/dreaming.rs`
- Test: `crates/hiero/tests/dream_execution.rs`
- Test: `crates/hieronymus/tests/dream_drain.rs`

**Interfaces:** Produce public `DreamRequest { pub all: bool, pub manual: bool }`, `DreamTicket { pub run_id: String }`, and cloneable `DreamController::request(&self, DreamRequest) -> Result<DreamTicket,String>`. `DreamController::start(config: HieronymusConfig, workers: &mut WorkerGroup) -> Result<Self,String>` consumes R5 supervision. Core `drain_batches(next: impl FnMut() -> Result<usize,DreamError>, cancelled: &AtomicBool) -> Result<usize,DreamError>` stops on zero progress. Route handlers share one controller; providers are created inside its worker.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hieronymus::dreaming::drain_batches;
use std::sync::atomic::AtomicBool;
#[test]
fn dream_all_drains_more_than_one_batch() {
    let mut batches = [2_usize, 2, 1, 0].into_iter();
    let count = drain_batches(|| Ok(batches.next().unwrap_or(0)), &AtomicBool::new(false)).unwrap();
    assert_eq!(count, 5);
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hieronymus --test dream_drain && cargo test -p hiero --test dream_execution`
Expected before the change: run_all performs one capped selection, and production manual Dream constructs a deterministic provider.

- [ ] **Step 3: Implement the bounded change**

Place the pure regression in crates/hieronymus/tests/dream_drain.rs (included below in the file list). Count completed/archived eligible inputs as progress, not merely selected rows. Stop with a durable failed/blocked result on provider/audit failure or zero progress while eligible work remains; do not spin or claim success. Re-select after each bounded batch so newly eligible work is considered, checking cancellation between batches. Schedule from actual config enabled/interval state; manual requests may bypass the global automatic-scheduling switch but never disabled per-workflow assignments. Coalesce concurrent triggers into one active run and expose tickets/status. Persist the selected run's progress; only completed sessions are eligible. Connect MCP dream, CLI dream through M5, and admin manual action to this controller. Join it on SIGTERM before releasing root ownership.

```rust
pub fn drain_batches(
    mut next: impl FnMut() -> Result<usize, DreamError>,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<usize, DreamError> {
    let mut total = 0;
    while !cancelled.load(std::sync::atomic::Ordering::Acquire) {
        let completed = next()?;
        if completed == 0 { break; }
        total += completed;
    }
    Ok(total)
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hieronymus --test dream_drain && cargo test -p hiero --test dream_execution`
Expected after the change: multiple capped batches drain; all production entry points use configured providers and share concurrency control.

Use a loopback provider and completed sessions exceeding 2×batch cap in the process test. Verify each input is archived exactly once, disabled automatic scheduling makes no calls, manual execution works, mid-run failure leaves remaining inputs pending, and SIGTERM joins the worker. Pure tests of drain_batches do not establish production wiring.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/daemon/dream_worker.rs crates/hiero/src/daemon/mod.rs crates/hiero/src/daemon/rest/admin.rs crates/hiero/src/daemon/registry.rs crates/hieronymus/src/dreaming.rs crates/hiero/tests/dream_execution.rs crates/hieronymus/tests/dream_drain.rs
git commit -m "feat: supervise and drain production dream runs"
```

## Plan self-review

Coverage is mapped in the coordinator. Every task above has a regression, implementation sketch, explicit interfaces, and a focused verification command. Execute prerequisites first; snippets using newly introduced APIs intentionally fail to compile before those APIs land. Do not interpret a passing compile as the behavioral green step.

