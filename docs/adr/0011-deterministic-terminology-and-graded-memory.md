# Separate Deterministic Terminology From Graded Memory Ranking

## Status

Accepted on 2026-08-31. This ADR supersedes ADR 0005's statement under
`### Rule Crystals` that Hieronymus has no separate structured terminology
authority, and ADR 0003 Decision 4's equivalent rule-crystal-only storage
decision. It also supersedes ADR 0005 `### Rule Crystals` where that section
prohibits manual promotion and gives dreaming authority to activate rules.
Rule crystals remain the searchable/advisory projection of the structured
authority.

Amended on 2026-09-06 by
[ADR 0016](0016-autonomous-story-memory-product-vision.md). The human-only
approval and lifecycle restrictions below are superseded: the system maintains
rules autonomously through validated, audited operations, while explicit user
corrections retain priority. Structured authority, separate deterministic
contracts, and protection from passive scoring remain in force. The original
text below is retained to explain the earlier decision.

## Context

Hieronymus has two related but different responsibilities:

- enforce approved renderings and forbidden variants deterministically;
- rank potentially useful memories for an agent.

Earlier documents sometimes describe both as “rule crystals.” This creates a
dangerous ambiguity: a fuzzy result may outrank a rule-intent memory in recall,
while approved terminology must never be silently overridden by fuzzy or
semantic evidence.

## Decision

Model one authoritative rule record with two explicit projections:

- a deterministic contract projection used by termbase contract and validation;
- a graded memory projection used by recall, explanation, and dreaming.

An approved active rule remains enforceable until it is explicitly archived,
superseded, or replaced by an authorized user action. Passive decay, recall
misses, semantic similarity, dreaming, and confidence scoring cannot deactivate
or alter its deterministic projection.

Recall remains a flat scored result set. A rule-intent memory receives a boost
but need not be the first fuzzy recall result. Every recall response also
includes a separate deterministic-contract section when applicable, so callers
cannot mistake ranking order for enforcement order. Validation evaluates that
section before advisory findings.

Rule lifecycle transitions are transactional and audited. Only an
authenticated user acting through the explicit rule-approval operation may
transition `candidate` to `active`, replace an active rule, archive it, or
supersede it. The operation records actor identity, prior and resulting
revision ids, timestamp, and reason, and succeeds only after deterministic
validation of the structured rule. Provider output, dreaming, imports of new
unapproved material, and passive scoring may create or update candidates but
cannot activate them. Malformed or ambiguous provider output never becomes
active. Existing approved legacy rules keep their active status through ADR
0010's audited migration; migration is preservation, not autonomous approval.

Structured rule data includes concept identity, source forms, canonical target
rendering, approved and forbidden variants, language tags, story scope where
applicable, matching policy, status, provenance, and revision relationship.
Rendered crystal prose is derived display/search content.

The Rust schema stores this authority in `term_rules` and `term_rule_forms`;
`term_rules` owns lifecycle, scope, concept, canonical rendering, matching
policy, provenance, and revision linkage, while `term_rule_forms` owns source,
approved, and forbidden forms with language and case sensitivity. A rule may
reference its advisory crystal projection, but that projection is not the
authority. ADR 0010's typed database converter owns migration from
`strict_terms`, aliases, tags, and existing rule crystals into these tables.

RAG ingestion and retrieval are advisory, but their output is contract-aware.
Applicable active rules are computed from the user query/source context before
RAG fusion and returned in the separate deterministic contract. RAG chunks that
contain conflicting target renderings are retained as evidence but carry a
`conflicts_with_rule_ids` marker and cannot suppress, rewrite, or satisfy the
active contract. Any generated answer or validation path evaluates the contract
after retrieval and before returning success.

## Recall Feedback

Recall feedback addresses a specific recall invocation, not every activation in
a session. Each recall creates a `recall_id`; returned crystal results expose an
activation id. Feedback carries the `recall_id`, activation ids, and an
idempotency key. Dreaming consumes each feedback event at most once.

Negative recall feedback affects only graded memory scores. It cannot weaken an
active deterministic contract.

## Consequences

This resolves the apparent conflict between graded reconsolidation and strict
terminology. The memory-reconsolidation design is retained for advisory memory
but is superseded where it allows active deterministic rules to decay or be
silently displaced.
