# Memory Reconsolidation Design

**Status:** Approved for implementation planning on 2026-07-18

## Context

The Rust migration proposal (`docs/rust-migration-proposal/001-006`) originally specified rule
crystals as a mandatory recall lane that "no amount of semantic evidence can override." Verifying
that claim against the current Python implementation found it was never actually true:
`recall.py:268-269` applies a flat `+0.20` additive boost (`_ACTIVE_RULE_BOOST`) to active rule
crystals, not an unbeatable lane — a high-scoring non-rule result can still outrank one. The
"mandatory" framing came from `docs/adr/0005-product-vision.md`'s aspirational language, not the
shipped behavior.

Separately, the schema already carries `rule_intent` and `source_credibility` columns on
`crystals` (`src/hieronymus/migrations/global.sql:90-115`) and an existing `crystal_links` table
and `crystal_activations` table — infrastructure for exactly the "graded, associative, gradually
reinforced memory" model this design formalizes, that the current codebase only partially wires
up (rule crystals get a boost; nothing today creates working copies on recall, propagates
activation across `crystal_links`, or lets an agent report which recalled crystals it actually
used).

This design corrects the migration proposal's recall model to match reality (rules are
prioritized, not enforced) and extends it with an explicit reconsolidation loop: a crystal
recalled during a session becomes an editable working copy in short-term memory, agents report
back which recalled crystals were useful or missed, and dreaming turns that evidence into
reinforcement, decay, supersession, and associative link strengthening — modeling recall as an
active, revisable process rather than a read-only lookup.

## Goals

- Replace the "rule crystals are a mandatory, unbeatable recall lane" invariant with a
  `rule_intent`-weighted score boost, matching the real current implementation.
- Let a recalled crystal be copied into session-scoped short-term memory as an editable working
  copy, deduplicated per `(session_id, crystal_id)`, so an agent can revise it during a session
  based on new information without mutating the source crystal directly.
- Let sufficiently-activated crystals pull in linked crystals via `crystal_links` in the same
  recall response (spreading activation), attenuated by link weight.
- Give the agent a cheap, explicit way to report which recalled crystals were actually useful
  versus irrelevant (`useful`/`miss` crystal-id lists), feeding immediate score deltas.
- At dream time, turn accumulated evidence (repeat-recall counts, useful/miss outcomes, edited
  working copies) into: passive reinforcement, decay, supersession of a crystal whose working
  copy diverged meaningfully, and Hebbian strengthening/creation of `crystal_links` between
  crystals used together.
- Replace rule-crystal archive immunity with a `rule_intent`-driven decay-rate dampener — rules
  fade slower under disuse, but nothing is permanently exempt from decay.

## Non-Goals

- No embedding-based link discovery. `crystal_links` strengthen from usage co-occurrence
  evidence at dream time, not a separate similarity/clustering job — semantic association is
  already RAG's job (vector search over `rag_chunks`), not a second graph on top of crystals.
- No live (synchronous, in-recall) link creation or strengthening — only lookup of existing
  links happens during `recall()`; writing new/stronger links is a batched dreaming operation.
- No change to the RAG pipeline's own retrieval model (003 §3-§5 of the migration proposal) —
  this design is scoped to crystals/short-term memory, not RAG chunks.
- No UI/admin-console changes are specified here; the admin console's crystal/concept views
  (005 §5 of the migration proposal) are updated separately if this design changes what they
  need to display.

## Recall-Time Behavior

`RecallService::recall(session_id, ctx, query, limit)`, for each crystal candidate:

1. **Score boost, not a mandatory lane.** Apply a boost proportional to `rule_intent` (a
   continuous field, not a boolean `crystal_type == 'rule'` gate) and `source_credibility`. A
   crystal with high `rule_intent` typically ranks first, but a strong enough non-rule match can
   still outrank it — this is the "prioritized, not enforced" behavior the current
   `_ACTIVE_RULE_BOOST` already approximates; this design makes the boost a function of the
   existing graded fields instead of a binary type check.
2. **Session-scoped working-copy dedup.** Look up an existing `short_term_memories` row with
   `source_crystal_id = crystal.id` for this `session_id`. If none exists, insert one (text
   seeded from the crystal's current text) — this is the crystal's editable working copy for the
   session. If one already exists, don't duplicate it; instead insert a `memory_events` row
   (`event_type = "recalled_again"`) so repeat-recall pressure is captured through the existing
   event-sourced scoring pattern rather than a new mutable counter column.
3. **Live spreading activation.** For any crystal whose score (after step 1) exceeds a
   configurable threshold, look up its 1-hop neighbors in `crystal_links` and fold them into the
   result set at `linked_score = source_score * link_weight * attenuation_factor` (attenuation
   configurable, default e.g. `0.5`). This is a single bounded query per triggering crystal, not
   a graph traversal — one hop only.

Active-rule-first ordering is dropped as an invariant; result ordering is purely score-based
after the boost in step 1 is applied.

## Explicit Feedback Signal

New store method, exposed as a CLI command, MCP tool, and HTTP endpoint (specified concretely in
the migration proposal's 003/005 updates):

```
FeedbackStore::record_recall_outcome(session_id: i64, useful: &[i64], miss: &[i64]) -> Result<()>
```

Each id in `useful` receives an immediate positive delta via the existing
`IMMEDIATE_EVENT_DELTAS` mechanism (`event_type = "recalled_useful"`); each id in `miss` receives
an immediate negative delta (`event_type = "recalled_miss"`). The MCP tool's schema description
instructs the calling agent to send this after acting on recall results — the instruction lives
in the tool definition itself so it travels with the tool rather than depending on a separately
maintained agent skill document staying in sync.

## Dream-Time Integration

- **Reinforcement.** `ReinforcementManager` reads `recalled_again`/`recalled_useful`/
  `recalled_miss` event counts accumulated since the last cycle as passive-delta evidence, in
  addition to whatever direct feedback already existed.
- **Reconsolidation vs. supersession.** For every working-copy short-term memory whose text has
  diverged from its source crystal's current text (the agent edited it mid-session), the
  `Crystallizer` phase computes a token-level diff ratio between the working copy and the source
  crystal's text. Below a default threshold (edit distance under 20% of token count), it
  reinforces the original crystal in place — no new row. At or above the threshold, it
  crystallizes a *new* crystal with `supersedes_crystal_id` pointing at the original, and the
  original transitions to `status = 'superseded'`. The threshold is a `DreamConfig` field (004
  §1's config struct), not hardcoded, so it can be tuned without a schema change. History is
  preserved via a chain of superseded crystals, never overwritten in place.
- **Hebbian link strengthening.** Crystals marked `useful` together within the same session are
  co-activation evidence. `Consolidator` strengthens the existing `crystal_links` row between
  them, or creates one if none exists, during its batch pass over the cycle's session data —
  this is deliberately async/batched (Non-Goals), not computed per-feedback-call.

## Rule-Intent Crystals

Archive immunity for `crystal_type == 'rule' && status == 'active'` is removed. In its place,
`rule_intent` acts as a decay-rate multiplier inside `apply_score_delta` (e.g. decay deltas are
scaled by `1.0 - (rule_intent * 0.5)`, so a crystal with `rule_intent = 1.0` decays at half the
normal rate). A well-established rule fades gradually under sustained disuse instead of snapping
directly to archived, but nothing is permanently exempt — consistent with "prioritized, not
enforced."

## Data Model Changes

- `short_term_memories`: add nullable `source_crystal_id INTEGER REFERENCES crystals(id)` —
  marks a row as a working copy; `NULL` for organically-added short-term memories.
- `crystal_links`, `crystal_activations`: reuse the existing tables (`global.sql`); exact column
  sets are pinned when the migration proposal's schema doc (002) is updated from a direct read
  of `global.sql`, not re-specified here.
- No new tables. `memory_events` already supports arbitrary `event_type` strings, so
  `recalled_again`/`recalled_useful`/`recalled_miss` require no schema change there.

## Interaction With The Migration Proposal

This design changes the following sections of `docs/rust-migration-proposal/`:

- **003** (`RecallService::recall`, §3): rewrite per "Recall-Time Behavior" above; drop the
  "active rules always first" invariant. `FeedbackStore` (§2.5): add
  `record_recall_outcome`; `apply_score_delta` gains the `rule_intent` decay multiplier.
- **004** (`ReinforcementManager`, `Crystallizer`, `Consolidator`, §2): each gains the
  responsibilities described in "Dream-Time Integration" above.
- **005** (§3 MCP tools, §5 REST API): add the `useful`/`miss` feedback tool/endpoint with
  agent-facing instruction text in its schema.
- **002**: add `source_crystal_id` to `short_term_memories`; pin real `crystal_links` /
  `crystal_activations` columns from `global.sql`.

## Testing Considerations

- Dedup correctness: two recalls of the same crystal in one session produce exactly one
  short-term working copy and two `memory_events` rows (one `recalled_again` after the first
  dedup hit).
- Spreading activation: a crystal above threshold with a known `crystal_links` neighbor returns
  that neighbor in the same `recall()` call, at the expected attenuated score, and does not
  recurse past one hop.
- Reconsolidation: a working copy edited beyond the "minor edit" threshold produces a new
  crystal with `supersedes_crystal_id` set and flips the original to `superseded` on the next
  dream cycle; an unedited or trivially-edited working copy only reinforces the original.
- Rule decay: two crystals with identical event histories but different `rule_intent` values
  decay at different rates, and both are archivable given enough negative evidence (no
  immunity).
