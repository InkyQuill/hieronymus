# Autonomous Authority Implementation Plan

> **For agentic workers:** Use `superpowers:executing-plans` to implement this plan task-by-task when implementation is requested. Steps use checkbox syntax for tracking. This P1 delivery is documentation only; do not execute the generated plan as part of P1.

**Goal:** Establish ADR 0016's autonomous-memory policy and realistic retrieval/host acceptance, then complete F1/F2 using the repaired runtime.

**Architecture:** Keep SQLite structured authority separate from graded retrieval. Corrections and automatic decisions enter validated, transactional domain operations; plugins teach the ordinary autonomous loop. Release acceptance exercises the installed application rather than only in-process stores.

**Tech Stack:** Existing Rust 1.96 / SQLite / LanceDB / ort / tokenizers; Svelte 5; Bun 1.4.0; Linux x86_64. Preserve current dependency pins until a measured retrieval requirement warrants a separately qualified change.

**Spec:** [Autonomous authority, corrections, and story applicability](../specs/2026-09-06-autonomous-authority-and-corrections.md). Read it in full before implementation; it defines every operation, predicate, table and acceptance outcome below. Rust `e9fae9b` and schema v4 after C8 are the design baseline. ADR 0016 is new product scope, not a retroactive downgrade of the completed port.

## Global Constraints

- Both working memory and semantic RAG are mandatory; no FTS-only release alternative.
- ADR 0016 supersedes older plans' human-only rule lifecycle instructions. Preserve deterministic enforcement, explicit-user priority, evidence, revision checks and audit.
- No mandatory human approval inbox, author tagging/scoring, or recurring curation step.
- Linux `x86_64-unknown-linux-gnu`; MCP revision `2026-07-28`; preserve frozen historical fixtures and record intentional Rust deltas separately.
- Configuration, host capabilities and semantic failures must be reported honestly. Do not claim a host or language is supported from synthetic or adjacent tests.
- F1/F2 remain unimplemented. This plan neither credits them as done nor expands them into a new attestation platform.
- Preserve existing dirty ADR/product documents; write new focused artifacts. No live migration, publication or user-host configuration rewrite during tests.

## File boundaries and dependency order

Task 1 owns schema and upgrades. Task 2 owns pure story applicability and context resolution. Task 3 owns typed authority and evidence validation. Task 4 owns immediate correction and coherent reads. Task 5 owns bounded background consolidation and Dream adaptation. Task 6 owns trusted transport ingress. Task 7 owns plugin instructions and real-host evidence. Implement in that order with one focused commit per task after its tests pass. Do not enable new mutation tools before Task 6 has verified origin separation. Tasks 1–5 can be tested with private test-only trusted ingress constructors; never expose those constructors in production tool schemas.

New modules in `crates/hieronymus/src/`: `authority_models.rs` (v1 typed contracts), `authority.rs` (decision transaction/evidence policy), `story_applicability.rs` (ordering/predicate), `corrections.rs` (claim effects), `consolidation.rs` (job lease/result/recovery state machines). Add module exports in `lib.rs` as needed. New `crates/hiero/src/application/authority.rs` adapts public drafts; new `crates/hiero/src/trusted_ingress.rs` alone enriches actor/origin; new `crates/hiero/src/daemon/correction_worker.rs` schedules consolidation. They contain no competing rule authority.

## Task 1: Register the single v5 schema step

**Files:** Create `crates/hieronymus/migrations/005-autonomous-authority.sql`; modify `crates/hieronymus/src/schema_upgrade.rs`, `db.rs`, `registry.rs`, `migrate.rs`, `upgrade.rs`; extend `crates/hieronymus/tests/rust_upgrade.rs` and `upgrade_port.rs`.

**Interfaces:** Input is v4 schema; output is v5 with exactly the tables/columns in the spec. `schema_upgrade::apply_steps` remains caller-transaction-owned; existing upgrade backup/rebuild protocol is unchanged.

- [x] Add failing upgrade cases: fresh v5 and upgraded v4 have identical schema; v4 active learned-looking provenance is preserved as protected legacy authority; candidate remains learned; newer/partial schema is refused; failure before marker rolls back new tables and rule authority; existing C8 link rows survive unchanged.
- [x] Run `cargo test -p hieronymus --test rust_upgrade` and record the intended missing-v5 failure.
- [x] Implement table inventory and backfill in `005-autonomous-authority.sql`. Apply the 2026-09-07 legacy-global schema clarification: preserve actual v4 global reach with migration-only nullable-series applicability and post-backfill creation/update/reuse guards; new decisions remain series-bound. Retained Python imports convert through v4 before applying v5 exactly once. Extend upgrade row accounting for exact intended backfill counts, preserving all prior rows. Use literal SQLite PK/FK/CHECK constraints described in the spec, append both version marker writes last, and register one step:

```rust
Step {
    from: 4,
    to: 5,
    sql: include_str!("../migrations/005-autonomous-authority.sql"),
    converter: None,
}
```

- [x] Set supported version to 5, add all new load-bearing tables to mandatory schema checks, and assert backfill count equals pre-upgrade `term_rules` count. Add series initialization of `authority_state` to the existing series creation transaction. Never use prose interpretation or provider calls in upgrade.
- [x] Run `cargo test -p hieronymus --test rust_upgrade --test upgrade_port`; inspect FK and fresh/upgraded equality outcomes; commit `feat: add autonomous authority schema v5` with only Task 1 files. If v5 has been allocated elsewhere, coordinate a contiguous renumbering before editing migration history.

## Task 2: Resolve story order and viewpoint applicability

**Files:** Create `crates/hieronymus/src/story_applicability.rs`, `crates/hieronymus/tests/story_applicability.rs`; modify `memory_models.rs`, `lib.rs`.

**Interfaces:** Define `StoryQueryV1 { series_id, timeline_id, position_id, viewpoint, scope_predicates, mode }`, `Viewpoint::{Narrator, Character(i64), Unspecified}`, `QueryMode::{Current, OmniscientResearch}` and `ApplicabilityV1` exactly as the spec. `StoryApplicability::resolve_context(&Connection, &TranslationContext) -> Result<StoryQueryV1, ApplicabilityError>`; `evaluate(&Connection, &ApplicabilityV1, &StoryQueryV1) -> Result<Eligibility, ApplicabilityError>`. `Eligibility` is `Current`, `FutureOrOutsideViewpoint`, `Unknown`, or `Excluded`. `ApplicabilityError` distinguishes wrong series/timeline, invalid interval and ambiguous position. Add `register_order_tx(&Transaction, manifest, evidence_id, expected_timeline_revision)` with compare-and-swap revision and no commit; it bumps series revision too.

- [ ] Add failing table cases, asserting these exact outcomes:

```text
manifest [Book I/Prologue, Book I/The Orchard, Book I/Interlude α, Book II/III]
compare(Prologue, III) = before; compare(III, missing Appendix) = unknown
reveal p12 + character query p3 = FutureOrOutsideViewpoint
narrator knows p12 + character has no gate = FutureOrOutsideViewpoint
Mira knows p15 + query Mira p15 = Current
valid [p2,p8) + query p8 = Excluded
two volumes both chapter III + no volume = AmbiguousPosition
```

- [ ] Run `cargo test -p hieronymus --test story_applicability` and observe missing predicate behavior.
- [ ] Implement ID-based order lookup and three-valued comparisons; preserve string chapter identifiers and scope spelling. Require exact scope containment before time and knowledge gates. Return unknown for unplaced metadata. For research mode classify future material separately; never change `Current` eligibility to bypass gates. Persist order changes with evidence and revision checking.
- [ ] Run that test target including flashback narrative-order, relationship evolution and no implicit narrator omniscience cases; commit `feat: resolve story position and viewpoint applicability`.

## Task 3: Introduce validated authority decisions

**Files:** Create `authority_models.rs`, `authority.rs`, `crates/hieronymus/tests/autonomous_authority.rs`; modify `terminology.rs`, `lib.rs`.

**Interfaces:** Implement spec's `DecisionRequestV1`, `OperationV1`, `CorrectionIntentV1`, receipts/results/errors, `EvidenceRef`, and `OriginReceiptId`. UUID may be a validated string newtype using existing dependencies; no new UUID dependency is required. `DecisionStore::apply(&DecisionRequestV1) -> Result<DecisionResultV1, DecisionErrorV1>` owns one immediate transaction. `apply_decision_tx(&Transaction, &DecisionRequestV1)` owns ingestion receipts and job intent but no commit. Extract internal `apply_mutations_tx(&Transaction, validated_mutations, audit_owner)` for policy-checked domain mutations only: no receipt, enqueue or commit. `audit_owner` distinguishes ingestion decision ID from consolidation result ID; Task 5 shares this primitive, never the ingestion wrapper. Refactor `Termbase::apply_action` to delegate to an internal transaction-taking lifecycle primitive while maintaining legacy replay receipts. Its new use accepts the resolved authority/applicability and cannot bypass it via legacy approve/archive tools.

- [ ] Add failing behavior cases: one anchor stays tentative; duplicated anchor counted once; two disjoint paragraphs with matching aligned rendering activate learned rule; contradiction prevents activation; learned replacement lacks contradiction and stays tentative; user B overrides learned A in one chapter while A remains elsewhere; Dream A cannot overwrite user B; stale series/rule revision rejects; exact replay returns stored receipt; changed payload with same ID rejects; failure inserting audit leaves rule/projection unchanged.
- [ ] Run `cargo test -p hieronymus --test autonomous_authority` and confirm failures concern missing policy rather than broken fixtures.
- [ ] Implement the spec's evidence-count/identity checks and applicability intersection/exclusion projection. Decision code follows this order:

```text
BEGIN IMMEDIATE
lookup receipt by decision_id; compare canonical request and origin; replay or conflict
resolve receipt, immutable evidence spans, concept, languages, applicability
compare authority_state and object revisions
evaluate learned/explicit_user policy; persist tentative or apply lifecycle + forms + projection
write decision evidence and lifecycle audit; increment authority_state
insert consolidation intent; write exact result; COMMIT
```

- [ ] Add source-name collision test: Alex-person-1 and Alex-person-2 with the same facet surface never resolve to one global rule. Test `Scope` moving learned authority into explicit scope is rejected. Preserve generic active-crystal mutation guards.
- [ ] Run `cargo test -p hieronymus --test autonomous_authority --test terminology_port`; commit `feat: validate autonomous terminology decisions`.

## Task 4: Commit immediate corrections and coherent reads

**Files:** Create `corrections.rs`, `crates/hieronymus/tests/correction_transactions.rs`; modify `authority.rs`, `feedback.rs`, `recall.rs`, `terminology.rs`, `lib.rs` and the concrete retrieval stores used by `RecallService`.

**Interfaces:** `apply_correction_tx(&Transaction, &DecisionRequestV1, &CorrectionIntentV1) -> Result<CorrectionEffect, DecisionErrorV1>` writes no independent commit. `CorrectionEffect` contains affected claim/rule IDs, revisions and effective applicability. Expose the existing feedback transaction helper internally and use it for relevance intent. Add `rehydrate_claims(&Connection, hits, &StoryQueryV1)` using claim bindings/effects. Contract/validation receive optional `required_decision_id` and return the observed series revision; stable-read response logic retries once or returns `StaleContext`.

- [ ] Write failing transaction tests for all rows in the minimum acceptance table below. Inject failure separately before effect, after effect, after job insert and before receipt: all rows roll back. Drop a successful response and replay: no second deltas/job. Concurrent corrections sharing revision: one succeeds, the other gets conflict. A scoped invalidation preserves disjoint earlier facts.
- [ ] Run `cargo test -p hieronymus --test correction_transactions`.
- [ ] Implement claim invalidation/qualification and typed rendering through the authority transaction. Do not invent replacement text for invalidation. Bind compound legacy hits conservatively. Persist job and existing semantic work intent with the correction. Apply SQLite eligibility before ranking/limit across short-term, crystal, facet and semantic candidates; refill bounded pages, then rehydrate before returning.
- [ ] Exercise a deliberately stale vector candidate and stale in-flight Dream/recall snapshot: known-invalid text must not return as current truth; required-decision validation cannot report success before the rendering is applied. Relevance-only negative feedback may change scores but must leave claim validity and `term_rules` byte-equivalent.
- [ ] Run correction, authority and existing recall/feedback targets discovered under `crates/hieronymus/tests`; commit `feat: persist immediate correction effects`.

## Task 5: Consolidate without losing immediate authority

**Files:** Create `consolidation.rs`, `crates/hieronymus/tests/correction_consolidation.rs`, `crates/hiero/src/daemon/correction_worker.rs`; modify `dream_output.rs`, `dreaming.rs`, `dream_workflows.rs`, `lib.rs`, and `crates/hiero/src/daemon/dream_worker.rs` to schedule the new worker within existing budgets.

**Interfaces:** `ConsolidationStore::lease_next(now, series_id) -> Result<Option<ConsolidationLease>, ConsolidationError>`; `prepare_result(lease_token, result_id, canonical_output)`; `finish_result_tx(&Transaction, result_id, lease_token)`; `fail(lease_token, error_code, now)`; `recovery_tick(now)`. Lease contains original job decision ID, separate result UUID/generation, token, expiry, attempts and observed series revision. Implement `ConsolidationResultV1`/`DerivedMutationV1` and the exact result-state protocol from the spec. Errors distinguish storage, expired lease, canonical idempotency conflict and revision conflict. Inject clock and provider call function for tests. `dream_output` accepts a separate versioned `decisions` draft section; generic `supersede_actions` retains active-rule guards. The worker attaches trusted Dream origin and selected evidence bounds. `recovery_tick` consumes persisted `provider_recovery_state`; no health-generation producer or synthetic transition is assumed.

- [ ] Add failing fake-clock cases for 120-second lease expiry, restart recovery, 30s/2m/10m/1h/6h initial retry delays and six failures total before parking. With only one parked job, unchanged configuration and no intervening provider requests, move clock six hours, make provider available, run the worker tick and assert completion. Assert a second tick/restart/config-fingerprint change cannot bypass the per-provider six-hour recovery cap; two parked jobs alternate by last-attempt ordering. Malformed provider output follows transient bounded retry; a local invariant/schema failure stays failed with diagnostics.
- [ ] Add result-identity tests: prepared result restart does not call provider; identical prepare replays and changed canonical bytes conflict; expired old lease cannot prepare. Inject failures between derived effect/audit/completion writes and assert total rollback. Drop a successful completion reply, restart, replay the result ID and assert identical receipt, exactly one derived effect, unchanged relevance deltas, original job complete and **zero self-generated pending jobs**. A stale series/object revision marks result stale and allocates a new generation/result ID for reevaluation, without rewriting its canonical payload. Test stale Dream output cannot resurrect invalidated claims or explicit-user A→B through new crystal/facet lineage.
- [ ] Run `cargo test -p hieronymus --test correction_consolidation`.
- [ ] Implement reserved/prepared/stale/complete result persistence and bounded recovery tick as specified. Persist normalized provider output before applying it; provider call remains outside SQLite transaction. Complete through the shared no-enqueue mutation primitive, not `apply_decision_tx`:

```text
BEGIN IMMEDIATE
if result complete: return exact stored completion receipt
verify prepared bytes + current lease/generation + evidence + revisions
if stale: mark result stale; advance job generation; schedule bounded retry; COMMIT
otherwise: apply_mutations_tx(..., audit_owner = result_id)
write derived audit/authority/lineage + semantic intent + exact result receipt
mark result and original job complete; clear lease; COMMIT
// No ingestion receipt, repeated correction delta, or new consolidation job.
```

- [ ] Schedule `recovery_tick` every 60 seconds even without ordinary work, plus once at startup. Lease oldest due parked job and advance job/provider deadlines atomically before the actual budgeted recovery call. Keep provider-slot identity stable across configuration changes and persisted deadlines across restart. Run new tests plus `dream_output`, `dream_transactions`, `dream_drain` and `dream_link_progress` integration targets; commit `feat: consolidate corrections with durable bounded retries`.

## Task 6: Enforce trusted ingress across transports

**Files:** Create `crates/hiero/src/trusted_ingress.rs`, `application/correction_parser.rs`, `application/authority.rs`, `crates/hiero/tests/application_authority.rs`; modify `application/mod.rs`, `application/memory.rs`, `application/terms.rs`, `daemon/protocol.rs` and actual daemon/MCP tool registration used by `tool_completeness.rs`. Extend `daemon_mcp_contract.rs`, `daemon_rest_routes.rs` and `tool_completeness.rs`.

**Interfaces:** `TrustedIngress::bind(principal, DecisionDraftV1) -> Result<DecisionRequestV1, IngressError>` is the only production constructor of actor-bearing domain requests. `DecisionDraftV1` contains the public version/IDs/evidence/concept/languages/applicability/operation and optional server receipt reference; it rejects actor/origin fabrication. `IngressError` maps to the spec's origin/request errors. Implement `parse_correction_v1(&OriginReceipt) -> Result<ParsedCorrectionV1, TentativeReason>` and `bind_selection(parsed, receipt_context) -> Result<CorrectionIntentV1, TentativeReason>` using the spec's complete English grammar, JSON token rules, UTF-8 spans and exact single occurrence/claim bindings. Separate direct-user submission validates UI interaction and stores an origin receipt; verified bridge ingress validates independent host-owned event text/session binding with credentials separate from ordinary MCP tools. Per the user's 2026-09-07 decision, same-account local shell access is trusted: protect MCP ingress against fabricated authorship and cross-context replay, without an isolated broker or claims against local credential reads, bridge invocation or direct database edits. Tools: `hieronymus_decide`, `hieronymus_correct`. `hieronymus_feedback` compatibility adapter reports tentative for unresolved prose and uses typed correction handling when available.

- [ ] Add failing tests: provider JSON claims `actor_kind=explicit_user`; ordinary MCP client supplies `source_role=user`; copied transcript supplies invented event ID; correct receipt is reused with another concept/session/operation; legacy approve/archive attempts to bypass the new policy. All must fail or stay tentative without authority change. A valid console event and a test bridge-owned event each yield explicit-user receipts. Synthetic bridge success proves server enforcement only.
- [ ] Add table-driven parser tests for every spec example: `translate this as X` binds one occurrence; quoted Unicode source matches exact source bytes; escaped quote/backslash target decodes once with correct byte spans; unquoted `and`/`or`, semicolon-separated commands, malformed quotes, unsupported language, multiple selections and ambiguous Alex stay tentative with zero mutation. `that memory is wrong` invalidates exactly one selected claim without a replacement; three unselected recall hits remain tentative. Qualification preserves decoded text. Test quoted source mismatch, missing language/scope, stale receipt source hash and changed revision separately. Assert command matching is anchored, keyword folding does not change payload case, bare-target punctuation is preserved and quoted trailing prose cannot be ignored.
- [ ] Run `cargo test -p hiero --test application_authority`.
- [ ] Implement the deterministic parser and selection binding before trusted request construction; store/recheck event/token byte spans and decoded values, with no provider extraction or silent revision refresh. Implement separation and tool registration, route all legacy hard-rule mutations through policy, expose observed revision/receipt/error variants, and reject unknown operation fields. Add accepted-receipt dependency to validation arguments. Never infer human authorship from the existing shared bearer actor label.
- [ ] Run `cargo test -p hiero --test application_authority --test daemon_mcp_contract --test daemon_rest_routes --test tool_completeness --test application_terms --test application_memory`; preserve MCP revision `2026-07-28` and frozen historical fixture files. Record changed Rust expectations in new tests. Commit `feat: bind correction authority to trusted ingress`.

## Task 7: Deliver the ordinary workflow and verify installed hosts

**Files:** Modify `crates/hiero/src/agent_plugins.rs` and its tests; create `crates/hiero/tests/autonomous_plugin_workflow.rs` and `docs/superpowers/reports/2026-09-06-autonomous-host-acceptance.md` when evidence is actually collected. Generated outputs are the exact Claude/Codex manifests, `.mcp.json`, eight skill paths and optional hook path listed in the spec. Do not edit Python assets or frozen fixture outputs to erase historical behavior.

**Interfaces:** `render/generate` signatures remain unchanged. Skills consume the v1 tools, typed receipts, story query and semantic availability. zCode installation uses the same Claude directory and content hash; no new target format. Codex installation uses its supported plugin mechanism. No claim that generated hooks alone establish trusted ingress. Qualify actual independent host event text/session binding and ordinary MCP credential separation under the trusted-local-shell threat model; do not invent a stronger shell-isolation acceptance requirement.

- [ ] Add failing behavioral integration tests with a fake host transcript driver: normal chapter work invokes recall/capture/validation; a rendering correction calls the correction tool and awaits an applied receipt; relevance feedback does not use factual invalidation; ambiguous identity stays tentative; provider outage does not cause a false consolidation claim. Assertions inspect tool calls and returned state, not Markdown wording.
- [ ] Run `cargo test -p hiero --test autonomous_plugin_workflow`.
- [ ] Update all eight skill bodies to explain the automatic loop, source evidence and separate correction intents. Remove human approval instructions, self-assigned authority guidance and unsolicited book-file reports. Keep generated integration in the data root and hooks optional.
- [ ] Run generator tests and new workflow tests. Install release bits into a disposable root and execute S1–S7 separately in actual Claude, zCode and Codex, using actual working-memory and semantic RAG retrieval. Capture host versions, supported ingress mechanism, selected languages, source corpus identity, model/provider configuration without secrets, actual MCP negotiation, receipt IDs and observed outcomes. If a host cannot supply a verified user-event channel, mark its explicit-user workflow unsupported and leave S7 unsatisfied; synthetic tests and console fallback cannot close it.
- [ ] Commit `feat: teach autonomous memory workflow in agent plugins` with only delivered code and factual evidence. Missing hosts or provider/model access are external requirements to report; do not fabricate a passing report or skip semantics to obtain a green release.

## Minimum acceptance matrix

Each case is a required executable domain/application test in Tasks 3–6 and a corresponding installed workflow trace in Task 7 where the host supports trusted ingress. A test pass must assert resulting contract/claim state and job/audit state, not just an HTTP 200.

| Case | Test sequence and durable assertion | ADR mapping |
| --- | --- | --- |
| Learned A, user B | Seed eligible A; ingest verified scoped B; validate B succeeds and A fails; restart; repeat; submit contrary Dream A and verify B remains. Outside corrected scope A still applies. | S2, S3 |
| Wrong event, no replacement | Recall claim; ingest invalidation; next recall omits or qualifies it in every lane; restart and consolidate; no replacement claim invented. | S4 |
| Unhelpful result | Record missed activation for its recall ID; score changes exactly once; underlying claim and rule remain valid; replay does nothing. | S4 distinction |
| Shared surface name | Two Alex concepts and ambiguous mention; capture tentative observation; no global replacement, wrong-person hard rule or required review action. | S2 |
| Learned revision | Two independent aligned anchors plus scoped contradiction; matching revisions commit replacement/evidence/audit; stale request and duplicate-evidence request cannot do so. | S2 |
| Chapter 12 revelation | Store narrative manifest with string IDs as well as a numbered fixture; narrator p12/character p15 gates; query character p3 excludes current truth; explicit research result is separately marked. | S5 |
| Provider unavailable | Commit clear correction; disable provider; restart; correction remains effective through six failures and parking; recover with unchanged config/no other requests at the six-hour recovery deadline; complete once with no recursive job or repeated delta. | S6 |
| Ordinary work across sessions | Three chapters/two sessions, prior voice/relationship/rendering retrieved and significant new observation captured without author labels or Remember invocation. | S1 |
| Three installed hosts | Claude and zCode use identical Claude-format bundle; Codex uses its supported bundle; all exercise receipt/recall/validation with working memory and semantic RAG. | S7 |

## Final verification and remaining release requirements

At implementation completion run `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`; also run the repository-required `uv run pytest`, `uv run ruff check .`, `uv run ruff format --check .`. Report unavailable toolchains/dependencies separately from failures; do not change pins to bypass qualification. P1 documentation delivery itself does not require implementation tests or Markdown wording tests.

Self-review coverage: Tasks 1–3 implement authority/evidence/migration and S2; Task 4 implements immediate S3/S4 and correct retrieval; Task 2 plus Task 4 implement S5; Task 5 implements S6; Task 6 establishes the trustworthy origin needed by S3/S4; Task 7 exercises S1/S7 and all minimum examples. Implementation must preserve old migration and Dream link/recovery behavior throughout.

External requirements remaining after this design: verified user-event ingestion in each actual host; installed-host/runtime access; real configured semantic model/provider and corpus/language acceptance evidence. These constrain release claims, not the independently executable domain design. Learned evidence thresholds and conservative unknown-position recall must be evaluated against a realistic story corpus; changing them is a versioned policy decision with measured evidence. F1/F2 packaging, retrieval acceptance and cutover remain unimplemented and separately owned. No live migration, publication, release promotion or user-host rewrite is part of this plan's P1 authoring scope.
