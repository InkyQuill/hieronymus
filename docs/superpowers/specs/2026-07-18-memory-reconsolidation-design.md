# Memory Reconsolidation Design

**Status:** Approved for the Python baseline on 2026-07-18. Proposed ADR 0011
supersedes this design for Rust wherever it permits active deterministic rules
to decay, archive passively, or be displaced by ranked recall.

## Context

The Rust migration proposal (`docs/rust-migration-proposal/001-006`) originally specified rule
crystals as a mandatory recall lane that "no amount of semantic evidence can override." Verifying
that claim against the current Python implementation found it was never actually true:
`recall.py:268-269` applies a flat `+0.20` additive boost (`_ACTIVE_RULE_BOOST`) to active rule
crystals, not an unbeatable lane — a high-scoring non-rule result can still outrank one. The
"mandatory" framing came from `docs/adr/0005-product-vision.md`'s aspirational language, not the
shipped behavior.

Separately, the schema already carries `rule_intent` and `source_credibility` columns on
`crystals` (`global.sql:59-60,103-104`, both `text`) and existing `crystal_links` and
`crystal_activations` tables — infrastructure for exactly the "graded, associative, gradually
reinforced memory" model this design formalizes, that the current codebase only partially wires
up. **Both fields are freeform text, not numeric weights**: `source_credibility` holds one of a
known small set of labels mapped to confidence via `SOURCE_CREDIBILITY_CONFIDENCE`
(`dreaming.py:26-33`: `rumor=0.15, thought=0.2, observation=0.35, source_text=0.7,
user_suggestion=0.8, expert=0.85, user_rule=0.95`); `rule_intent` is genuinely freeform
(`admin.py:796` defaults it to `"correction"`) but is used everywhere else as a boolean presence
check (`dreaming.py:3675`: `memory.rule_intent.strip()`; `recall.py:504`'s scalar-match boost).
This design reuses both mechanisms as-is rather than changing either column's type or semantics —
no data migration needed for existing rows.

This design corrects the migration proposal's recall model to match reality (rules are
prioritized, not enforced) and extends it with an explicit reconsolidation loop: a crystal
recalled during a session becomes an editable working copy in short-term memory, agents report
back which recalled crystals were useful or missed, and dreaming turns that evidence into
reinforcement, decay, supersession, combination, and associative link strengthening — modeling
recall as an active, revisable process rather than a read-only lookup.

## Goals

- Replace the "rule crystals are a mandatory, unbeatable recall lane" invariant with a boost
  driven by the existing `source_credibility` confidence map and `rule_intent` presence check,
  matching the real current implementation instead of ADR 0005's unimplemented aspiration.
- Let a recalled crystal be copied into session-scoped short-term memory as an editable working
  copy, deduplicated per `(session_id, crystal_id)`, so an agent can revise it during a session
  based on new information without mutating the source crystal directly.
- Let sufficiently-activated crystals pull in linked crystals via `crystal_links` in the same
  recall response (spreading activation), attenuated by link weight.
- Give the agent a cheap, explicit way to report which recalled crystals were actually useful
  versus irrelevant (`useful`/`miss` crystal-id lists), feeding immediate score deltas.
- At dream time, turn accumulated evidence (repeat-recall counts, useful/miss outcomes, edited
  working copies) into: passive reinforcement, decay, supersession of a crystal whose working
  copy diverged meaningfully, combination of highly-similar co-activated crystals, and Hebbian
  strengthening of `crystal_links` between crystals used together.
- Replace rule-crystal archive immunity with a `rule_intent`-driven decay-rate dampener — rules
  fade slower under disuse, but nothing is permanently exempt from decay.

## Non-Goals

- No schema type change for `rule_intent`/`source_credibility` and no migration of existing
  values — both stay `text`, reusing existing label semantics.
- No embedding-based link discovery. `crystal_links` strengthen from usage co-occurrence
  evidence at dream time, not a separate similarity/clustering job — semantic association is
  already RAG's job (vector search over `rag_chunks`), not a second graph on top of crystals.
- No live (synchronous, in-recall) link creation, strengthening, or crystal combination — only
  lookup of existing links happens during `recall()`; writing new/stronger links and combining
  crystals are batched dreaming operations.
- No change to the RAG pipeline's own retrieval model (003 §3-§5 of the migration proposal) —
  this design is scoped to crystals/short-term memory, not RAG chunks.
- No UI/admin-console changes are specified here; the admin console's crystal/concept views
  (005 §5 of the migration proposal) are updated separately if this design changes what they
  need to display.
- No N-way crystal combination in one step — combination is pairwise only (see "Dream-Time
  Integration"); an N-way cluster resolves over several dream cycles, one pair at a time.

## Recall-Time Behavior

`RecallService::recall(session_id, ctx, query, limit)`, for each crystal candidate:

1. **Score boost, not a mandatory lane.** Apply `source_credibility_weight(crystal) =
   SOURCE_CREDIBILITY_CONFIDENCE.get(crystal.source_credibility).unwrap_or(0.35)` (the map ported
   to Rust verbatim, `0.35` = `observation`'s weight as the schema default) as a multiplicative
   or additive scoring factor, plus a flat `RULE_INTENT_BOOST` (named constant, not derived from
   the text content) applied only when `!crystal.rule_intent.trim().is_empty()`. A high-credibility
   rule-intent crystal typically ranks first, but a strong enough non-rule match can still outrank
   it — matching the real current `_ACTIVE_RULE_BOOST`/`_add_short_term_scalar_boosts` behavior,
   just fed by the existing label semantics instead of a `crystal_type == 'rule'` gate.
2. **Session-scoped working-copy dedup.** Look up an existing `short_term_memories` row with
   `source_crystal_id = crystal.id` for this `session_id`. If none exists, insert one (text
   seeded from the crystal's current text) — this is the crystal's editable working copy for the
   session. If one already exists, don't duplicate it; instead insert a `memory_events` row
   (`event_type = "recalled_again"`) so repeat-recall pressure is captured through the existing
   event-sourced scoring pattern rather than a new mutable counter column.
3. **`crystal_activations` logging.** Every candidate returned (deduped or not) gets one
   `crystal_activations` row: `(session_id, crystal_id, activated_at, outcome = NULL)`. This is
   the per-activation ledger `record_recall_outcome` (below) and dreaming's `LinkReinforcer`
   phase both read — a full log, unlike the deduped short-term working copy.
4. **Live spreading activation.** For any crystal whose post-boost score exceeds
   `SPREADING_ACTIVATION_THRESHOLD`, look up its 1-hop neighbors in `crystal_links` and fold them
   into the result set at `linked_score = source_score * link_weight * SPREADING_ATTENUATION`.
   Both constants live as named `const`s in `hiero-core::domain::recall` (default `0.55` and
   `0.5`) — matching how `_ACTIVE_RULE_BOOST` is a hardcoded module constant today, not a config
   file field, since nothing in the current system makes these user-tunable. This is a single
   bounded query per triggering crystal, not a graph traversal — one hop only.

Active-rule-first ordering is dropped as an invariant; result ordering is purely score-based
after the boost in step 1 is applied.

## Explicit Feedback Signal

New store method, exposed as a CLI command, MCP tool, and HTTP endpoint (specified concretely in
the migration proposal's 003/005 updates):

```
FeedbackStore::record_recall_outcome(session_id: i64, useful: &[i64], miss: &[i64]) -> Result<()>
```

Behavior:
- Each id in `useful` gets an `IMMEDIATE_EVENT_DELTAS["recalled_useful"] = (0.06, 0.04)` delta —
  one notch below `confirmed_by_user`'s `(0.15, 0.20)`, since the agent finding a memory useful
  isn't the same strength of signal as a human explicitly confirming it's correct.
- Each id in `miss` gets `IMMEDIATE_EVENT_DELTAS["recalled_miss"] = (-0.05, -0.03)` — deliberately
  softer than `contradicted_by_user`'s `(-0.20, -0.25)`, since "not relevant this time" isn't the
  same claim as "this is wrong."
- Both id lists additionally set `outcome = 'useful' | 'miss'` on the matching `crystal_activations`
  rows for that `session_id` (§Recall-Time Behavior step 3), which is what `LinkReinforcer`
  reads at dream time.

The MCP tool's schema description instructs the calling agent to send this after acting on
recall results — the instruction lives in the tool definition itself so it travels with the tool
rather than depending on a separately maintained agent skill document staying in sync.

## Dream-Time Integration

Three new/changed phases, each implementing the `DreamPhase` trait from 004 §2:

- **`ReinforcementManager` (existing phase, extended).** Reads `recalled_again` event counts
  (`PASSIVE_EVENT_DELTAS["recalled_again"] = (0.02, 0.0)` — strength-only, since mere
  resurfacing implies nothing about correctness) accumulated since the last cycle as additional
  passive-delta evidence, alongside whatever direct feedback already existed. `recalled_useful`/
  `recalled_miss` are already applied immediately (§Explicit Feedback Signal) so this phase does
  not reapply them.
- **`Reconsolidator` (new phase, purely algorithmic — no `DreamProvider`/LLM dependency, unlike
  `Crystallizer`).** For every non-archived working-copy short-term memory
  (`source_crystal_id IS NOT NULL`), computes a token-level diff ratio against the source
  crystal's current text.
  - Below `DreamConfig::reconsolidation_diff_threshold` (default: edit distance under 20% of
    token count): reinforce the original crystal in place, no new row.
  - At or above the threshold: crystallize a *new* crystal with `supersedes_crystal_id` set to
    the original's id (text taken directly from the working copy — no LLM call, this is a
    mechanical carry-over), and flip the original to `status = 'superseded'`. The new crystal
    inherits the original's `crystal_concepts` rows (copied, not moved — the superseded row's
    links are left in place for audit history; only active crystals are queried in practice, so
    the leftover rows on a superseded crystal are inert).
  - Either way, the working-copy `short_term_memories` row gets `archived_at` set (the existing
    field already used to exclude processed rows from dreaming input — `workspace.py:498`,
    `dreaming.py:1278` etc. — no new lifecycle field needed).
- **`LinkReinforcer` (new phase, replaces the earlier draft's overloaded `Consolidator`
  extension — kept separate because `Consolidator`'s existing input is `Vec<ConceptRecord>`, a
  different type from the crystal-activation data this phase needs).** Reads the cycle's
  `crystal_activations` rows.
  - **Hebbian strengthening**: crystals with `outcome = 'useful'` in the same session are
    co-activation evidence; strengthens the existing `crystal_links` row between them, or creates
    one if none exists.
  - **Pairwise combination**: among co-activated `useful` pairs, if a similarity check (shared
    `crystal_concepts`, or a text-similarity score above a threshold) indicates they're
    near-duplicates, propose a combination: pick a survivor (higher `source_credibility` weight,
    tie-broken by higher `strength`), union the other's `crystal_concepts` and `crystal_links`
    rows onto the survivor, set the absorbed crystal's `status = 'superseded'`, and record a
    `memory_events` row (`event_type = "combined_into"`, `evidence = survivor_id`) on the
    absorbed crystal — `supersedes_crystal_id` isn't reused here since that slot is already the
    `Reconsolidator`'s "new crystal replaces its own prior working-copy revision" relationship;
    combination is a different relationship (two independently-existing crystals merging) and
    doesn't need a new column, just an event-sourced record consistent with how
    `recalled_again`/`recalled_useful`/`recalled_miss` are already handled. Combination is
    pairwise only (Non-Goals) — a 3-way-similar cluster resolves over multiple dream cycles.

## Rule-Intent Crystals

Archive immunity for `crystal_type == 'rule' && status == 'active'` is removed. In its place,
`apply_score_delta` (003 §2.5) dampens decay deltas by a flat factor (default `0.5`, i.e. half
the normal decay rate) when `!crystal.rule_intent.trim().is_empty()`, optionally further scaled
by `source_credibility_weight(crystal)` for graded effect (a `user_rule`-credibility crystal
decays slower than an `observation`-credibility one, even if both have non-empty `rule_intent`).
A well-established rule fades gradually under sustained disuse instead of snapping directly to
archived, but nothing is permanently exempt — consistent with "prioritized, not enforced."

## Data Model Changes

- `short_term_memories`: add nullable `source_crystal_id INTEGER REFERENCES crystals(id)` —
  marks a row as a working copy; `NULL` for organically-added short-term memories.
- `crystal_activations`: gains (or already has, to be confirmed against `global.sql` when 002 is
  updated) a nullable `outcome TEXT` column (`'useful' | 'miss' | NULL`) written by
  `record_recall_outcome`.
- `crystal_links`: reused as-is; exact columns pinned from `global.sql` when 002 is updated.
- No new tables. `memory_events` already supports arbitrary `event_type` strings, so
  `recalled_again`/`recalled_useful`/`recalled_miss`/`combined_into` require no schema change
  there.

## Interaction With The Migration Proposal

This design changes the following sections of `docs/rust-migration-proposal/`:

- **003** (`RecallService::recall`, §3): rewrite per "Recall-Time Behavior" above; drop the
  "active rules always first" invariant; port `SOURCE_CREDIBILITY_CONFIDENCE` into
  `hiero-core::domain::crystal` or `values`. `FeedbackStore` (§2.5): add
  `record_recall_outcome`, the three new event-delta entries, and the `rule_intent`/
  `source_credibility` decay dampener in `apply_score_delta`.
- **004** (§2): `ReinforcementManager` extended per above; add `Reconsolidator` and
  `LinkReinforcer` as new phases; `Consolidator` is explicitly left unchanged (concepts only).
  `DreamConfig` (004 §1) gains `reconsolidation_diff_threshold`.
- **005** (§3 MCP tools, §5 REST API): add the `useful`/`miss` feedback tool/endpoint with
  agent-facing instruction text in its schema.
- **002**: add `source_crystal_id` to `short_term_memories`; confirm/add `outcome` on
  `crystal_activations`; pin real `crystal_links` columns from `global.sql`.

## Testing Considerations

- Dedup correctness: two recalls of the same crystal in one session produce exactly one
  short-term working copy, two `crystal_activations` rows, and one `recalled_again` memory event
  (from the second recall's dedup hit).
- Spreading activation: a crystal above `SPREADING_ACTIVATION_THRESHOLD` with a known
  `crystal_links` neighbor returns that neighbor in the same `recall()` call, at the expected
  attenuated score, and does not recurse past one hop.
- Reconsolidation: a working copy edited beyond `reconsolidation_diff_threshold` produces a new
  crystal with `supersedes_crystal_id` set, flips the original to `superseded`, copies concept
  links, and archives the working copy — all on the next dream cycle. An unedited or trivially
  edited working copy only reinforces the original and still gets archived.
- Combination: two co-activated `useful` crystals with high similarity produce exactly one
  `combined_into` event, one survivor with the union of both crystals' concept/links, and one
  absorbed crystal marked `superseded` — never a schema change to `supersedes_crystal_id`.
- Rule decay: two crystals with identical event histories but one empty and one non-empty
  `rule_intent` decay at different rates; both remain archivable given enough negative evidence
  (no immunity). Two non-empty-`rule_intent` crystals with different `source_credibility` labels
  decay at different rates from each other too.
