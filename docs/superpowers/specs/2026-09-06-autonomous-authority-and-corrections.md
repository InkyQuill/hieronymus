# Autonomous authority, corrections, and story applicability

Status: proposed implementation design for accepted ADR 0016; documentation only. Baseline: Rust commit `e9fae9b`, schema v4 including C8. This specifies new product work and does not revise historical implementation grades or declare F1/F2 complete.

## Requirements

**Goal:** Establish ADR 0016's autonomous-memory policy and realistic retrieval/host acceptance, then complete F1/F2 using the repaired runtime.

**Architecture:** Keep SQLite structured authority separate from graded retrieval. Corrections and automatic decisions enter validated, transactional domain operations; plugins teach the ordinary autonomous loop. Release acceptance exercises the installed application rather than only in-process stores.

**Tech Stack:** Existing Rust 1.96 / SQLite / LanceDB / ort / tokenizers; Svelte 5; Bun 1.4.0; Linux x86_64. Preserve current dependency pins until a measured retrieval requirement warrants a separately qualified change.

- Both working memory and semantic RAG are mandatory; no FTS-only release alternative.
- ADR 0016 supersedes older plans' human-only rule lifecycle instructions. Preserve deterministic enforcement, explicit-user priority, evidence, revision checks and audit.
- No mandatory human approval inbox, author tagging/scoring, or recurring curation step.
- Linux `x86_64-unknown-linux-gnu`; MCP revision `2026-07-28`; preserve frozen historical fixtures and record intentional Rust deltas separately.
- Configuration, host capabilities and semantic failures must be reported honestly. Do not claim a host or language is supported from synthetic or adjacent tests.
- F1/F2 remain unimplemented. This plan neither credits them as done nor expands them into a new attestation platform.
- Preserve existing dirty ADR/product documents; write new focused artifacts. No live migration, publication or user-host configuration rewrite during tests.

Governing references: [ADR 0016](../../adr/0016-autonomous-story-memory-product-vision.md), [ADR 0010](../../adr/0010-data-locations-schema-ownership-and-upgrade.md), and ADRs 0006–0015 where not amended by 0016. The [implementation review](../../astra-implementation-review-2026-09-06.md), [correctness prerequisites](../plans/2026-09-06-merged-port-correctness.md) and [F1/F2 plan](../plans/2026-09-05-rust-port-release.md) retain their historical scope and grades. Python assets describe historical behavior only.

## Existing entry points and seven acceptance scenarios

| ADR scenario | Current Rust foundation | Missing behavior and observable acceptance |
| --- | --- | --- |
| S1: continuity across chapters/sessions | `RecallService::recall`, `search_series`; `application/memory.rs` short-term add/batch and RAG tools; generated read/recall/orchestrate skills | Ordinary work triggers recall, selective capture and validation without explicit Remember requests. Installed-host transcript spans two sessions and three chapters, retrieves previously captured voice, relationship and rendering, and captures a new consequential observation without author tags or queue actions. |
| S2: autonomous terminology and ambiguity | `Termbase::propose`, `apply_action`; `dream_output::ConceptProposal` | Validated learned decisions activate/revise rules with evidence; two characters sharing a name do not generate a global obligation. Contract contains the supported rule and reports unresolved identity separately. |
| S3: direct rendering correction | `Termbase::contract`, `validate`, `apply_action`; `application/terms.rs` | One correction changes the next contract and validation, survives restart, and resists a contrary Dream decision. No promotion command. |
| S4: wrong recollection | `hieronymus_feedback` currently writes short-term memory; `FeedbackStore::record_recall_outcome` only changes scores | A factual correction durably suppresses or qualifies the exact claim before the next recall, with no invented replacement. Same after restart and consolidation. |
| S5: later revelation and earlier knowledge | `TranslationContext` stores string volume/chapter and story scopes; recall applies context boosts | Structural story order, validity and per-viewpoint knowledge gate retrieval, independent of ranking. Chapter 12 revelation is absent from chapter 3 character-current truth. |
| S6: provider outage | durable Dream retry state and semantic work intent, short-term storage | Correction effects commit without provider calls; leased consolidation retries are bounded and recover automatically while immediate effects remain. |
| S7: host parity | `crates/hiero/src/agent_plugins.rs::render/generate`, Claude and Codex manifests | Actual installed Claude, zCode and Codex sessions exercise S1–S6 with verified ingress capabilities. zCode consumes the identical Claude-format bundle. Manifest generation alone earns no compatibility claim. |

Existing `RuleActionRequest` trusts its actor from the transport, checks a rule revision and stores canonical replay receipts. It does not distinguish learned authority from explicit user direction. `dream_output.rs` deliberately rejects active-rule supersede targets; retain this protection for generic crystal operations. `feedback.rs` cannot reach hard rules. Those boundaries are useful, not defects to remove wholesale.

## Trust and typed protocol v1

The daemon owns trusted ingress. A configured daemon credential identifies a client, not the human author of arbitrary client text. `source_role="user"`, `source_credibility="user_rule"`, “User told me …”, provider confidence, and a supplied `actor_kind` never confer user authority.

Introduce server-issued, immutable `OriginReceipt` records. A direct authenticated console correction route may mint a `console_user` receipt only from its dedicated user interaction, with the submitted text and session context. Agent tools cannot invoke that route using the ordinary MCP client credential. A host bridge may mint a `host_user_event` receipt only from a host-owned user-message event channel whose role, event ID, text and session binding the bridge obtains independently of model tool arguments. Bridge credentials are separate from model-accessible MCP credentials. Receipt bytes are stored locally; a hash binds text/context, but a hash alone is not provenance verification. No copied transcript supplied by a model qualifies.

The daemon assigns `agent` to normal MCP submissions and `dream` to its internal provider worker. Provider output supplies only a decision draft; the worker attaches its own selected-context origin, permitted evidence IDs and actor. Host adapters refer to an already minted receipt, not an arbitrary claimed user event. The receipt authorizes only an interpretation grounded in its stored utterance/context, never an unrelated operation. Direct UI structured rendering fields or an exact, context-bound rendering command can be resolved without an LLM; other interpretations must name stored spans and remain tentative when the intent/target is not clear. A bridge must constrain calls to its event/session and cannot grant a blanket user authority token. In v1, explicit rendering activation requires the canonical target to equal a stored structured user field or an exact quoted target span in a supported rendering-command grammar; source selection, language pair and scope must match the receipt context. Factual invalidation/qualification similarly requires a supported intent pattern and a uniquely selected claim/recall-item binding; qualification text is preserved verbatim. Provider paraphrase is not accepted as proof that a different rendering or wider scope was requested. Unsupported linguistic forms are durable tentative signals until trusted structured context resolves them; supported input grammar/language coverage must be recorded in host acceptance.

Current host support for an independent user-event channel has **not been established**. Native hook names or host authentication do not prove it. For a host without this capability, ordinary MCP supports learned work and stores unverified correction signals as tentative; it must report `UnverifiedOrigin` rather than promise immediate explicit-user authority. Direct console input remains possible but is not claimed as satisfaction of the normal host workflow. Verifying a supported trusted channel in each advertised host is an external release requirement, not a requirement that authors approve memories or a reason to weaken this boundary.

Public JSON uses a tagged version-1 draft; unknown versions, unknown operation fields and unknown enum variants are rejected. IDs are positive SQLite integers except UUID `decision_id` and receipt IDs. Language values must resolve to the series' registered normalized language identifiers; terminology requires both nonempty languages. Story facts use `source_language` and nullable `target_language` (language-neutral fact identity). No wildcard language inferred from an empty string.

```rust
// Proposed APIs/types; not current implementation.
struct DecisionRequestV1 {
    version: u8,                    // exactly 1
    decision_id: Uuid,              // root-wide idempotency key
    expected_revision: u64,         // authority_state revision for this series
    actor_kind: ActorKind,          // daemon-attached, not deserialized from draft
    origin: OriginReceiptId,        // verified by trusted_ingress
    evidence_refs: Vec<EvidenceRef>,
    series_id: i64,
    concept_id: Option<i64>,        // Some required for term/fact application
    source_language: String,
    target_language: Option<String>,
    applicability: ApplicabilityV1,
    operation: OperationV1,
}
enum ActorKind { ExplicitUser, Agent, Dream }
enum Authority { Learned, ExplicitUser }
enum OperationV1 {
    Activate { candidate_id: i64, candidate_revision: u64 },
    Replace { rule_id: i64, rule_revision: u64, rendering: RenderingV1 },
    Scope { rule_id: i64, rule_revision: u64, new_applicability: ApplicabilityV1 },
    Archive { rule_id: i64, rule_revision: u64 },
    Correct { intent: CorrectionIntentV1 },
}
enum CorrectionIntentV1 {
    Fact { claim_id: i64, claim_revision: u64, effect: FactEffect },
    Relevance { recall_id: String, useful: Vec<i64>, missed: Vec<i64> },
    Rendering { replaces: Option<(i64, u64)>, value: RenderingV1 },
}
enum FactEffect { Invalidate, Qualify { qualification: String } }
struct RenderingV1 {
    source_forms: Vec<String>, canonical: String,
    approved_variants: Vec<String>, forbidden_variants: Vec<String>,
    case_sensitive: bool,
}
enum DecisionResultV1 {
    Applied { receipt: DecisionReceiptV1 },
    Replayed { receipt: DecisionReceiptV1 },
    Tentative { receipt: DecisionReceiptV1, reasons: Vec<TentativeReason> },
}
// Receipt: decision_id, resulting_revision, affected rule/claim IDs and
// their revisions, effective applicability, effect, consolidation job ID,
// origin receipt ID and commit timestamp; persisted exact serialized value.
enum DecisionErrorV1 {
    UnsupportedVersion, InvalidRequest, UnverifiedOrigin, OriginMismatch,
    EvidenceMismatch, UnknownTarget, LanguageMismatch, ApplicabilityConflict,
    AuthorityConflict, RevisionConflict { current_revision: u64 },
    IdempotencyConflict, StorageUnavailable,
}
```

`EvidenceRef` is a typed pointer `(kind, id, content_hash, span_start, span_end)`, where kind is `source_passage`, `aligned_rendering`, `observation`, or `user_event`. The application resolves immutable records, checks same series/concept/context and valid UTF-8 byte boundaries, and never accepts an inlined provider passage as a preexisting source. `TentativeReason` is `AmbiguousIdentity`, `AmbiguousIntent`, `InsufficientEvidence`, `UnknownOrder`, or `ConflictingEvidence`. Valid but unresolved drafts persist the raw signal and reasons and create context-gathering work; malformed, forged or stale requests are errors and do not mutate authority. Receipt replay precedes revision checking and returns the original result, not today's projection. Reusing a decision ID with different canonical content, ingress principal or origin returns `IdempotencyConflict`.

## Evidence policy and decision table

Evidence policy v1 is deterministic in what it measures; semantic interpretation is explicitly fallible. An eligible learned rendering needs at least **two distinct source passage anchors** with an aligned, identical target rendering, from distinct paragraph ranges; copies, overlapping spans, Dream summaries and repeat recalls count once by source document identity/hash and range. Both anchors must explicitly bind the same concept identity, languages and applicability; at least one names the identity through an existing unambiguous facet or a source identity anchor (not surface name alone). There must be zero unresolved contradictory anchors in the selected concept/applicability evidence set and no applicable explicit-user override. Activation covers only the demonstrated scope; two chapters do not prove series-wide universality. Scope widening requires this same evidence test in each newly covered volume/chapter segment.

For learned replacement/archive, require those two supporting anchors **and** one source-grounded contradiction or qualification of the old rule, with old evidence re-evaluated and conflict explained structurally (`erroneous_mapping`, `scope_mismatch`, or `story_evolution`). A correction of an erroneous mapping replaces within its resolved scope; evolution closes a validity interval and opens another. A provider confidence of 1.0 cannot substitute for any criterion. These are conservative versioned initial criteria, to be evaluated on the story corpus; they do not imply that recurrence alone establishes linguistic truth. Factual inferred revisions use two distinct supporting source anchors and a direct contradiction of the prior claim; without those they remain qualified uncertainty, not destructive replacement.

A verified explicit user instruction needs one stored user event with clear intent, exact concept and language resolution and explicit or session-bound applicability; it need not recur. An ordinary “translate this as B” resolves “this” against a unique selected source occurrence/concept and the current language pair/scope. Missing selection or two matching people stays tentative; do not apply a global replacement or ask for routine adjudication. Agents continue useful work and gather context. Facts with no concept anchor may be captured as observations until linked; they cannot mutate another claim by surface match.

| Existing authority / signal | Permitted action |
| --- | --- |
| None / eligible learned evidence | Activate learned rule at exact demonstrated applicability. |
| Learned / qualifying new learned evidence | Revision-checked replace, rescope or archive; audit old/new evidence and state. |
| Learned / verified clear user B | Immediately apply explicit-user B within intersection; preserve learned A outside it. |
| Explicit user / learned preference or passive decay | No authority mutation. Retain evidence as advisory disagreement. |
| Any hard rule / negative usefulness | Apply relevance event only; no archive, revoke or silent replacement. |
| Explicit user / later verified explicit correction | Revision-checked replacement within corrected scope. Commit order alone does not guess ambiguous conversational intent. |
| Any / unclear identity, order or interpretation | Tentative with reasons; no new hard obligation. |
| Fact / clear user invalidation without replacement | Suppress claim in specified scope immediately; no invented fact. |
| Fact / explicit qualification | Return only qualified status and explanation; never plain current truth. |

Applicability is a set, not a ranking boost. Explicit-user rules dominate learned rules only in their intersection. Two incompatible learned obligations in an overlapping region are rejected, not arbitrarily sorted. A later explicit correction of an explicit rule must identify the prior revision; narrower correction preserves old authority outside the intersection. Implement overrides as retained historical rules plus applicability exclusion rows, not whole-rule archival that loses other scopes. `Scope` cannot evade authority priority by moving a competing rule into an explicit region. Contract projection resolves identity first, then applicability and authority, then rendering; it never consumes graded recall scores. Ambiguous source occurrences produce an unresolved finding and cannot earn a clean applicable validation result by omission.

## Story order, validity and knowledge

`ApplicabilityV1` contains `series_id`, exact volume/chapter keys (optional), normalized additional scope predicates (all required), `timeline_id`, optional `[valid_from, valid_until)` position IDs, and viewpoint knowledge gates. Volume/chapter strings retain their current `TranslationContext` values and `volume:` / `chapter:` scope spelling. They are identity keys, not ordinals. Resolve legacy chapter scopes inside the selected volume; repeated chapter labels without volume resolve as unknown, never by first match. Existing other story scopes retain exact-match meaning and cannot imply chronology.

A timeline is a versioned ordered sequence of positions within one series. Positions have stable IDs, volume key, chapter key, optional scene key and an explicit unique integer ordinal **assigned from source manifest order**, never parsed from names. Import can use a book's table of contents or an explicit ordered manifest; its persisted evidence is required for reordering. Ordinals are implementation order values, not numeric requirements for chapter identifiers. Example manifest: `Book I/Prologue`, `Book I/The Orchard`, `Book I/Interlude α`, `Book II/III` maps to positions p1–p4 in that order. `chapter:III` alone is not the third chapter. An unplaced `Appendix: Letters` remains unordered; comparisons return `unknown` and current-truth recall omits position-sensitive claims with `unknown_story_order` warning. Scenes refine chapters; unresolved within-chapter ordering similarly yields unknown. A flashback has its narrative reading position here; event chronology is a separate optional relation between claims and never overwrites revelation order.

World validity and knowledge are separate: a character may have died at p2, narrator reveal at p12, and character Mira learn at p15. Each claim has world-valid `[from, until)` and one or more `knowledge_gate(viewpoint_kind, viewpoint_concept_id, known_from, known_until)` rows. Query viewpoint is `narrator`, `character(concept_id)`, or `unspecified`; narrator is not implicitly omniscient. Narrator gates follow revelation/read position. An explicitly requested omniscient research mode may include future material, but only in a separately labelled `future_or_outside_viewpoint` section, never the current-context list or terminology contract. Character queries require a positive character knowledge gate; narrator evidence cannot satisfy it. Unspecified viewpoint returns only claims marked safe for all viewpoints in the relevant interval, otherwise uncertain/outside-viewpoint metadata without assertion text. Unknown position/viewpoint never defaults to latest knowledge.

Revelation does not change earlier knowledge gates. Relationship evolution closes old world validity and creates a new claim linked by `evolves_from`, preserving earlier recall. Correction marks an old claim erroneous in correction applicability regardless of when the correction arrived; it does not recast the error as historical story truth. Capture observation time, event validity and knowledge time separately. Two characters named Alex retain separate concept IDs, facets and gates; unresolved Alex references remain observations with candidate identities, not mandatory aliases.

Apply the same predicate to short-term observations, crystals, concept facets and semantic/FTS RAG candidates **before fusion and final limit**, and rehydrate hits through current SQLite state before response serialization. Refill from bounded candidate pages when filtered hits shrink results; report exhaustion rather than pad with ineligible hits. Raw source inspection can expose a later passage only as clearly labelled source evidence, not remembered current truth. For compound legacy crystals, qualifying/suppressing the whole item is safer than extracting a supposedly safe sentence. New capture splits atomic claims. Unknown legacy temporal metadata is `unspecified`, not “always true”. This conservatism may reduce recall until evidence supplies position/gates; measure it rather than silently remove filtering.

## Immediate correction transaction and consolidation

Expose `hieronymus_decide` for structured learned decisions and `hieronymus_correct` for correction drafts; trusted origin enrichment is shared by MCP, CLI and console adapters. Keep `hieronymus_feedback` as an explicit compatibility adapter: clear typed corrections enter the new path; old prose-only input returns `tentative` with an explanation instead of claiming it changed a contract. Existing recall-outcome feedback remains relevance-only.

`DecisionStore::apply(request)` opens `BEGIN IMMEDIATE`, validates canonical replay, origin binding, target revisions, evidence, scope and series authority revision, and then performs one commit containing:

1. Immutable decision/input/evidence/audit records.
2. Immediate structured rule/form/projection changes for rendering, or claim status/applicability suppression for factual correction, or existing event-scored relevance deltas for relevance.
3. Updated per-object revisions and incremented series `authority_state.revision` (including tentative signal storage; exact replay changes nothing).
4. A durable `consolidation_jobs` intent plus existing semantic invalidation/work intent and corpus revision when retrieval content changes.
5. Exact serialized result receipt.

Refactor existing terminology and feedback primitives to accept the caller's transaction; do not call public functions which open and commit separate connections. Projection crystals follow authoritative rule changes in that transaction. Provider calls and vector index writes never occur inside it. Database failure rolls everything back and returns no success. Lost reply after commit is recovered by decision-ID replay.

“Next operation” means one admitted after the correction commit. Recall/validation pin a SQLite snapshot and include the series authority revision. Before successful response publication, compare that revision with current state and retry once on a changed revision; continued churn yields a typed stale-context response rather than stale success. A dependent validation carries `required_decision_id`; reject pending/tentative/missing decisions, and use a snapshot at least as new as its receipt, respecting later corrections. Concurrent corrections based on the same revision yield one commit and one `RevisionConflict`; adapters reload state and re-resolve the later user instruction with a new decision ID and current prior-rule revision. Never silently rebase a learned decision or use client timestamps as last-writer authority.

Suppression is enforced by SQLite rehydration even when semantic indexes or caches are stale. Every derived object carries claim lineage; Dream merging/copying must inherit invalidations and explicit-user authority masks or reject the output. A pre-correction Dream batch rechecks series revision on commit and reschedules on conflict. Raw archive passages remain source evidence with correction annotation; a known-invalid claim must not reappear as truth through source snippets, facets or duplicates linked to that claim. Unlinked paraphrase matching may propose a lineage link but cannot justify suppressing unrelated memories; this residual identity limitation must be included in acceptance corpus evaluation.

Consolidation leases one job for 120 seconds, uses `decision_id` as stable work identity, and rechecks revisions before commit. Expired leases recover on restart. Retry schedule: 30 seconds, 2 minutes, 10 minutes, 1 hour, 6 hours after successive transient failures; after the initial attempt and five retries (six failed attempts total), park as `degraded`. Provider health/configuration transition automatically re-arms parked jobs, once per transition; no tight loop or curation queue. Permanent schema/invalid-output errors are `failed` with diagnostics; configuration or code recovery may re-arm them, never clear the immediate correction. Success atomically records consolidated lineage and job completion; score deltas already applied at ingestion are never applied again. Existing Dream budgets bound provider work; a worker may process at most one correction job per series at a time. These jobs supplement current `dream_retry_state`, not replace C8 link cursors.

## Exact schema ownership: one v4 → v5 step

The future implementation owns **only** new `crates/hieronymus/migrations/005-autonomous-authority.sql` plus registration `{from:4,to:5}` in `schema_upgrade.rs` and `SUPPORTED_RUST_SCHEMA_VERSION = 5`/mandatory-table checks in `db.rs`. Never edit `004-lazy-dream-links.sql`, earlier SQL or the v1 baseline. Fresh creation replays the same registered step; no opportunistic table creation by domain stores. If another accepted step consumes v5 before implementation, coordinate and renumber this entire step before any code ships; there is no second owner of version 5.

The following is the complete logical table/column inventory for the step. All referenced existing object IDs use foreign keys; receipts/audit are retained, with no cascade deletion of history. JSON columns require `json_valid` and typed domain validation in the same transaction. UTC timestamps are text; booleans are checked 0/1; revisions are nonnegative integers. UUIDs are text primary keys.

| Table/change | Columns, constraints and ownership |
| --- | --- |
| `authority_state` | `series_id PK FK series`, `revision NOT NULL DEFAULT 0`. One row per series, inserted also on series creation. |
| `origin_receipts` | `id PK`, `kind CHECK(console_user,host_user_event,agent,dream,legacy_import)`, `principal`, `session_id nullable FK task_sessions`, `event_id`, `text`, `context_json`, `content_hash`, `created_at`; UNIQUE(kind,principal,event_id). Receipt minting belongs to trusted ingress. |
| `decision_records` | `decision_id PK`, `series_id FK`, `origin_id FK`, `actor_kind`, `expected_revision`, `resulting_revision`, `canonical_request`, `result_json`, `status CHECK(applied,tentative)`, `created_at`. Index(series_id,resulting_revision). Canonical request includes all typed operation fields and evidence refs. |
| `decision_evidence` | `decision_id FK`, `ordinal`, `kind`, `source_id`, `hash`, `span_start`, `span_end`; PK(decision_id,ordinal). Polymorphic source integrity verified transactionally against immutable evidence records, not assumed from a numeric ID. |
| `evidence_records` | `id PK INTEGER`, `series_id FK`, `kind`, `source_identity`, `source_hash`, `span_start`, `span_end`, `content`, `binding_json`, `created_at`; UNIQUE(kind,source_identity,source_hash,span_start,span_end). Bindings include exact concept, languages, position, alignment and originating record; append-only snapshots preserve deleted/changed sources. |
| `story_timelines` | `id PK INTEGER`, `series_id FK`, `name`, `revision`, UNIQUE(series_id,name). |
| `story_positions` | `id PK INTEGER`, `timeline_id FK`, `volume_key`, `chapter_key`, `scene_key NOT NULL DEFAULT ''`, `ordinal`, `evidence_id FK evidence_records`; UNIQUE(timeline_id,ordinal), UNIQUE(timeline_id,volume_key,chapter_key,scene_key). |
| `applicabilities` | `id PK INTEGER`, `series_id FK`, `timeline_id nullable FK`, `volume_key nullable`, `chapter_key nullable`, `scope_predicates_json`, `valid_from nullable FK story_positions`, `valid_until nullable FK story_positions`, `metadata_state CHECK(resolved,unspecified)`. Domain checks same timeline, interval ordering and chapter/volume containment. |
| `knowledge_gates` | `id PK INTEGER`, `applicability_id FK`, `viewpoint_kind CHECK(narrator,character,all)`, `viewpoint_concept_id nullable FK concepts`, `known_from nullable FK story_positions`, `known_until nullable FK`; require character iff concept nonnull. Domain verifies series/timeline and ordered intervals. |
| `memory_claims` | `id PK INTEGER`, `series_id FK`, `concept_id nullable FK`, `text`, `revision`, `status CHECK(current,invalid,qualified,tentative)`, `qualification`, `applicability_id FK`, `evolves_from nullable FK memory_claims`, `created_at`, `updated_at`. |
| `claim_bindings` | `id PK INTEGER`, `claim_id FK`, exactly one nonnull FK of `short_term_id`, `crystal_id`, `facet_id`, `rag_chunk_id`; UNIQUE per (claim_id,target column). Multiple atomic claims may bind an item; return it only if all asserted claims are eligible, otherwise qualify/omit whole item. |
| `claim_effects` | `id PK INTEGER`, `claim_id FK`, `decision_id FK`, `applicability_id FK`, `effect CHECK(invalid,qualified)`, `qualification`, `supersedes_effect_id nullable FK`; index(claim_id). Scoped effects override base claim state; disjoint scope remains intact. |
| `rule_authority` | `rule_id PK FK term_rules`, `authority CHECK(learned,explicit_user)`, `origin_id FK`, `decision_id nullable FK`, `applicability_id FK`, `legacy_protected CHECK 0/1`. Existing `term_rules.revision` remains rule revision. |
| `rule_exclusions` | `id PK INTEGER`, `rule_id FK`, `applicability_id FK`, `decision_id FK`; exclusions represent replaced intersection while retaining the original rule elsewhere. |
| `consolidation_jobs` | `decision_id PK FK`, `state CHECK(pending,leased,retry,degraded,failed,complete)`, `attempts`, `next_attempt_at nullable`, `lease_until nullable`, `lease_token nullable`, `last_error_code nullable`, `last_rearm_generation`, `updated_at`; index(state,next_attempt_at). |

SQL backfills authority_state and an immutable `legacy_import` receipt for existing rules. Preserve all active v4 obligations as `explicit_user` with `legacy_protected=1`; this is a conservative compatibility protection, **not a claim that their freeform actor strings prove a human instruction**. Legacy candidates become learned, unevidenced candidates. Existing rule scopes map to exact applicability with temporal metadata `unspecified`; preserve their existing contract scope behavior as protected legacy rules until a validated scope correction, while surfacing that temporal applicability is unknown. New temporal rules cannot inherit this exception. Migration records affected counts and this intentional policy separately from frozen fixtures. A later verified explicit correction can replace protected legacy rules; Dream cannot. Existing memory items acquire atomic/compound claim bindings only where structurally available; otherwise lazily capture a legacy compound claim through a normal audited domain write, never schema migration on reads. Unbound legacy items are classified unknown by retrieval until linked. Do not infer facts from prose during schema upgrade.

`term_rule_actions` and `term_rule_revisions` remain lifecycle audits; populate them from the shared transactional primitive and link the new decision ID in the canonical action payload. Add no duplicate rule truth columns to crystals. Upgrade backup, exclusive transaction, verification, rollback and semantic rebuild use the existing ADR 0010 runner. Verify new mandatory tables, FK integrity, interval constraints, authority backfill count and fresh/upgraded schema equality before publishing markers; the SQL marker statements come last as in existing steps.

## Plugin delivery and acceptance limits

Update only the active Rust generator and its behavioral tests; keep `src/hieronymus/agent_assets.py` and frozen Python fixtures historical, recording intentional Rust deltas. All eight shared skills (bootstrap, recall, read, learn, remember, translate, review, orchestrate) teach the automatic loop: obtain context/position/viewpoint, recall contract and graded memories, capture consequential observations with evidence, submit corrections, wait for applied receipt before dependent validation, then record relevance separately. Selectivity prioritizes recurrence, consequences, voice, continuity and unresolved threads; no author tagging or scores. Remember handles ordinary conversational signals, not a special mandatory command.

Generated `agent-plugins/claude/.claude-plugin/plugin.json`, `.mcp.json`, and `skills/*/SKILL.md` are also the zCode bundle. Codex uses `agent-plugins/codex/.codex-plugin/plugin.json`, `.mcp.json`, `skills/*/SKILL.md` through its supported integration. `hooks/hooks.codex.json` is optional and cannot supply correctness or trusted origins merely by existing. Retain stable installed `hieronymus-mcp` entry point and daemon discovery; no baked endpoint/credential or copied backend. Generate assets under the configured data root; never write book content, automatic reports, or workspace configuration. Optional reports go to application diagnostics or an explicitly selected user path. No automatic host configuration rewrite.

Run S1–S7 against installed Linux release bits, actual semantic model/provider configuration and host versions, preserving source/target languages and data-root identity in the evidence. Host transcripts must show receipt, restart, recall and validation outcomes; internal unit tests prove domain behavior, not host support. Keep retrieval quality evaluation and packaging/cutover F1/F2 as separately unfinished work. No dependency upgrade, model replacement, host authentication capability, language coverage or release publication is authorized by this design.
