# Separate Deterministic Terminology From Graded Memory Ranking

## Status

Proposed.

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

Rule lifecycle transitions are transactional and audited. Provider output and
dreaming may propose rules but cannot activate them unless the existing product
policy explicitly permits that transition and deterministic validation of the
structured rule succeeds. Malformed or ambiguous provider output never becomes
active.

Structured rule data includes concept identity, source forms, canonical target
rendering, approved and forbidden variants, language tags, story scope where
applicable, matching policy, status, provenance, and revision relationship.
Rendered crystal prose is derived display/search content.

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
