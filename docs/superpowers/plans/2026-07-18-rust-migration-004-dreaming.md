# Rust Migration Phase 004 Dreaming and LLM Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port provider configuration, all dreaming phases, reconsolidation, link reinforcement, bounded decay, audit, locking, and scheduled execution.

**Architecture:** `DreamService` is a thin orchestrator over phase objects and Phase 003 stores. Provider adapters implement one object-safe async trait; pure parsing/scoring/reconsolidation decisions remain synchronous. A cross-process lock coordinates daemon and direct CLI invocations.

**Tech Stack:** Rust 2024, Tokio, reqwest/rustls, serde, fs2, SHA-256, SQLx.

## Global Constraints

- Preserve every current dream phase and public `DreamService` behavior.
- `provider.conf` owns provider profiles; `dream.conf` owns workflows and scheduling only.
- Never send secrets to logs, diagnostics, audit payloads, or error displays.
- Both daemon and CLI dream runs contend on the same cross-process lock.
- Decay and maintenance queries are bounded, indexed, cancellable, and auditable.
- LLM output is untrusted input: parse, validate, penalize malformed recoveries, and never panic.

---

## File Map

- `provider/`: catalog, cache, registry, and Anthropic/OpenAI/Google/Ollama adapters.
- `dreaming/`: lock, config, workflows, phases, reconsolidation, reinforcement, decay, audit, service, scheduler.
- `tests/test_dreaming.rs`: end-to-end fake-provider cycles and lock contention.

**Focused commands:** Tasks 1-6 respectively use `cargo test -p hiero-core --test test_providers`, `--test test_dream_config`, `--test test_dream_lock`, `--test test_dream_phases`, `--test test_reconsolidation --test test_link_reinforcement`, and `--test test_dreaming`. RED means the named contract fails for the absent behavior; GREEN means exit 0 with all named tests passed.

### Task 1: Provider Catalog and Adapters

**Files:** Create `provider/{mod.rs,catalog.rs,cache.rs,anthropic.rs,openai.rs,google.rs,ollama.rs}`, `tests/test_providers.rs`.

**Interfaces:** Produce `DreamProvider::{crystallize,run_pass}`, `DreamOutput`, `CandidateCrystal`, `ConceptCandidate`, `ProviderProfile`, `ProviderCatalog::{load,save,get,upsert,delete}`, `ProviderRegistry::{resolve,check,suggest_models}`, and bounded `ModelCache` from proposal 004 §2.

- [ ] Port provider config/cache/provider tests with fake HTTP responses: auth header, request shape, response extraction, timeout, HTTP error, malformed JSON, redaction, model suggestion cache, and Ollama local URL.
- [ ] Run focused tests; expect RED.
- [ ] Implement adapters over injected `reqwest::Client`, typed provider errors, rustls, explicit timeouts, and secrecy-aware credentials. Keep the trait object-safe because registry contents are heterogeneous.
- [ ] Run focused tests; expect GREEN. Commit `feat: port dream providers`.

### Task 2: Dream Configuration and Workflow Resolution

**Files:** Create `dreaming/{config.rs,workflows.rs}`, `tests/test_dream_config.rs`.

**Interfaces:** Produce `DreamConfig`, `WorkflowProfile`, `PhaseProfile`, `DreamPhase`, atomic `load/save/validate`, and deterministic workflow resolution from proposal 004 §8.

- [ ] Port dream config/workflow cases: defaults, profile override, missing provider/model, disabled phase, invalid interval, unknown phase, and provider separation.
- [ ] Run focused tests; expect RED. Implement serde config plus pure resolver; expect GREEN.
- [ ] Commit `feat: port dreaming configuration`.

### Task 3: Cross-Process Lock and Audit Lifecycle

**Files:** Create `dreaming/{lock.rs,audit.rs}`, `tests/test_dream_lock.rs`.

**Interfaces:** Produce `DreamCyclePaths`, `dream_cycle_paths`, `DreamCycleState`, RAII `DreamCycleGuard`, `acquire_dream_cycle_lock(config, owner, wait) -> Result<DreamCycleGuard, DreamCycleAlreadyRunning>`, and audit/run/phase start-complete-fail APIs.

- [ ] Port lock and audit tests, spawning a second `hiero` process against the same temp root; assert one winner, stale metadata recovery only when OS lock is free, token-checked cleanup, and failure audit completion.
- [ ] Run focused tests; expect RED.
- [ ] Use an OS advisory lock held by the guard for its full lifetime; state JSON is diagnostic only. Cleanup in `Drop` must never remove another token's state.
- [ ] Run focused tests; expect GREEN. Commit `feat: add safe dream-cycle locking and audit`.

### Task 4: Phase Parsing and Execution

**Files:** Create `dreaming/{phases.rs,parsing.rs,crystallize.rs,concepts.rs,evidence.rs}`, `tests/test_dream_phases.rs`.

**Interfaces:** Produce the `DreamPhase` trait and typed phase outputs in proposal 004 §3 plus `strip_code_fences(&str) -> &str` and `parse_dream_output(&str) -> Result<DreamOutput, MalformedPayloadError>`.

- [ ] Port dreaming/evidence/malformed-output tests for every phase, fenced JSON, surrounding prose, truncated/invalid output, schema mismatch, empty output, duplicate proposals, and audit penalties.
- [ ] Run focused tests; expect RED.
- [ ] Implement ordered recovery: direct JSON, one fenced JSON block, one balanced object/array extraction; reject ambiguity. Mark recovered output and apply the specified malformed penalty through Phase 003 scoring.
- [ ] Execute phases sequentially according to resolved workflow; provider HTTP is async, parsing is sync, large parsing uses `spawn_blocking`. Run tests; expect GREEN.
- [ ] Commit `feat: port dreaming phases and parsing`.

### Task 5: Reconsolidation, Reinforcement, and Decay

**Files:** Create `dreaming/{reconsolidation.rs,reinforcement.rs,decay.rs}`, `tests/test_reconsolidation.rs`, `tests/test_link_reinforcement.rs`.

**Interfaces:** Produce proposal 004 §4 collaborators and pure decision functions for reinforce-vs-supersede, pair generation, and decay deltas.

- [ ] Encode exact fixtures from the July 18 reconsolidation design: similarity threshold boundaries, inherited concept links, working-copy archive, pairwise Hebbian strengthening, combined events, rule-intent dampening, confidence-zero archive, stable pagination, and cancellation.
- [ ] Run focused tests; expect RED.
- [ ] Keep decisions pure and execute each accepted result through store transactions. Query maintenance by `idx_crystals_maintenance` cursor `(last_reinforced_cycle,last_activated_cycle,id)` with a fixed batch size; never offset-scan the whole table.
- [ ] Run focused tests; expect GREEN. Commit `feat: implement memory reconsolidation and decay`.

### Task 6: Dream Service and Background Scheduler

**Files:** Create `dreaming/{service.rs,scheduler.rs}`, `tests/test_dreaming.rs`; export from `lib.rs`.

**Interfaces:** Produce proposal 004 §7 `CycleOptions`, `MaintenancePayload`, `MaintenanceResult`, `DreamService::{run_cycle,run_all,run_due,decay_candidates,apply_maintenance}`, and `run_background_loop(service, interval, shutdown)`.

- [ ] Write end-to-end fake-provider tests for success, phase failure, cancellation, concurrent lock denial, empty input, audit counts, and graceful scheduler shutdown.
- [ ] Run focused test; expect RED.
- [ ] Compose lock → run record → configured phases → reconsolidation/reinforcement/decay → completion. Use `tokio::select!` with interval and broadcast shutdown; do not spawn unbounded cycles or overlap runs.
- [ ] Run all Phase 004 and workspace verification; expect GREEN. Commit `feat: complete Rust dreaming service`.

## Phase Acceptance

- [ ] No provider test performs network I/O and no captured output contains fixture API keys.
- [ ] Two OS processes cannot run a dream cycle simultaneously against one data root.
- [ ] Every error path closes run/phase audit records and releases the lock.
- [ ] `EXPLAIN QUERY PLAN` confirms decay uses `idx_crystals_maintenance`.
