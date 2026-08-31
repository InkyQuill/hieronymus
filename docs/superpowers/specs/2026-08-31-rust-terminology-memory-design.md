# Rust Terminology And Memory Design

**Status:** Proposed for review on 2026-08-31.

## Goal

Preserve deterministic translation rules while allowing fuzzy memory,
reconsolidation, feedback, and semantic evidence to remain graded and revisable.

## Rule Authority

An active approved rule has structured match and rendering data. Its lifecycle
is `candidate`, `active`, `superseded`, or `archived`. Only an audited explicit
authenticated user action can transition `candidate` to `active`, replace an
active rule, or make an active rule leave `active`. The approve/change request
records actor, reason, prior revision, and idempotency key and must pass
deterministic validation transactionally. Dreaming and provider output can
only create or refine candidates. Passive decay and recall feedback never
change that lifecycle. Supported legacy active rules retain status during the
audited database conversion.

Termbase contract returns the applicable active rules for source text and
translation context. Validation checks canonical and forbidden renderings and
returns deterministic findings before advisory findings. Ambiguous concept
identity produces an explicit warning instead of a guessed hard violation.

Rule-crystal text, FTS tokens, and recall metadata are projections of the
structured rule. Updating the authoritative rule refreshes its projections in
the same transaction or queues a rebuildable projection repair.

ADR 0010's database-upgrade track owns the `term_rules`/`term_rule_forms`
schema and typed migration from legacy strict terms, aliases, tags, concepts,
and existing rule crystals. This terminology track owns validation and runtime
lifecycle only after conversion; it performs no migration-on-read.

## Recall Result

A recall response contains:

- `recall_id`;
- `deterministic_contract`, a list of applicable active rule ids and structured
  rendering requirements;
- `results`, a flat ranked list of short-term memories, crystals, and RAG
  chunks;
- degraded-mode warnings, such as semantic index unavailable.

Ranked results include stable source kind/id, activation id where relevant,
score, rank, reason components, text, scopes/tags, concept references,
credibility, and rule/thought markers. The deterministic contract is not mixed
into score ordering.

## Ranking

Candidate generation is bounded by context and source lane. Each lane produces
a ranked list; fusion uses deterministic constants and tie-breaking. Rule
intent and source credibility may boost a memory result. They cannot modify the
deterministic contract.

Spreading activation is one hop and bounded by the remaining result budget. It
uses persisted links plus activation evidence; it does not perform writes while
ranking. Duplicate entities are merged with traceable reason components.

RAG and semantic hits are advisory evidence. Before fusion, the service computes
the applicable active contract from query/source context. Each conflicting RAG
hit is annotated with `conflicts_with_rule_ids`; it remains inspectable but
cannot remove, satisfy, rewrite, or outrank the separate contract section.
Answer/validation success is impossible until the post-retrieval contract check
passes or returns an explicit ambiguity warning.

## Working Copies And Feedback

Returning a crystal may create at most one working copy per
`(session_id, crystal_id)`. Repeated recall records another activation and a
repeat-recall event without duplicating the working copy.

Feedback submits `recall_id`, an idempotency key, and disjoint useful/miss
activation-id lists. The service verifies ownership and rejects unknown,
duplicate-across-lists, or already-finalized feedback. Applying the same
idempotency key again returns the prior result without applying score deltas
twice.

Feedback records which activation it addresses. Dreaming claims unprocessed
feedback events with a durable consumption marker so link reinforcement does
not replay them in later cycles.

## Reconsolidation

An edited working copy can reinforce or supersede its advisory source crystal
according to a deterministic diff policy. Supersession preserves provenance
and concept links. It cannot mechanically rewrite or deactivate an active
deterministic rule; a conflicting edit becomes a rule-change proposal requiring
the authorized lifecycle path.

Pairwise combination and link reinforcement operate only on advisory crystal
state. Combination is transactional, audited, idempotent, and bounded per
cycle. Concepts do not decay as crystals.

## Acceptance Criteria

- Fuzzy and semantic results cannot alter or hide applicable active contracts.
- Conflicting RAG chunks remain evidence but are identified and cannot satisfy
  deterministic validation.
- Active deterministic rules survive arbitrary passive decay and miss events.
- Recall/feedback is correlated and idempotent across repeated recalls in one
  session.
- Working-copy dedup and feedback event consumption do not lose activations.
- Reconsolidation cannot bypass the rule lifecycle.
- Candidate activation and active-rule changes require an authenticated,
  audited user operation; dreaming has no activation capability.
- Python compatibility fixtures yield equivalent deterministic findings.
