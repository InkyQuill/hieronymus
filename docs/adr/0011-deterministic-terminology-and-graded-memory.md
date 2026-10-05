# 0011 — Deterministic terminology and graded memory

Status: accepted 2026-08-31; autonomous lifecycle amended by ADR 0016 on
2026-09-06. Consolidated 2026-10-05. Supersedes the rule-crystal-only authority
model in ADRs 0003/0005 and the former requirement for human-only approval.

## Context

Terminology validation and memory ranking answer different questions. A useful
recollection may rank above a rule's prose, but that must not weaken the rule.
Treating both as one fuzzy “rule crystal” makes enforcement ambiguous.

## Decision

Keep one structured terminology authority with separate projections:

- `term_rules` and `term_rule_forms` determine lifecycle, source/target forms,
  canonical rendering, variants, language/matching policy, scope and revisions.
- Searchable crystals explain rules and contribute to graded recall. Their prose
  and scores are not the authority.

Compute the applicable deterministic contract before lane fusion and return it
separately from ranked results, even when advisory limits exclude all fuzzy hits.
Validation evaluates that contract before advisory findings. RAG is evidence;
conflicting renderings remain inspectable with rule-conflict markers and cannot
suppress, rewrite or satisfy the contract.

Recall accepts an optional `memory_types` category selection: terms, concepts,
lessons and knowledge. Omission keeps mixed recall including observations and
source passages; selection restricts durable-memory candidates before lexical,
semantic, metadata and graph budgets. It never hides the deterministic contract
or changes claim disposition, authority, chronology or viewpoint. Terms-only
recall requires no semantic lane. This lets agents request the material needed
for a task without treating broad research output as current approved knowledge.
Tests establish filtering and contract preservation, not literary relevance or
live-model accuracy.

Active rules remain enforceable until a validated authority operation changes
them. Agent/Dream learned decisions are permitted under ADR 0016's evidence and
scope policy. Explicit-user direction takes priority within scope. Revision checks,
identity disambiguation, transactional lifecycle audit and origin validation are
required. Passive decay, similarity, confidence and recall misses cannot change
an active rule. Ambiguous output stays tentative.

Migration preserves approved historical obligations with explicit protection;
it does not establish that historical freeform actor labels prove user origin.
Legacy `strict_terms` records and modern authority IDs are separate. Read
[authority ingress](../authority-ingress.md) for operations and receipts, not
historical approval-wrapper examples.

## Recall feedback

Feedback binds one `recall_id`, its returned activations and an idempotency key.
Each event is consumed at most once. Negative usefulness changes graded relevance,
not factual validity or deterministic terminology. Fact corrections and scoped
invalidation use the separate authority path.

## Consequences

The app can maintain memory autonomously while preserving predictable translation
constraints. Ranking and source evidence stay flexible; authority changes are
explicit and explainable. [Business logic](../business-logic.md) explains the
user-visible behavior; live MCP schemas are in `crates/hiero/resources/mcp/`,
and [authority ingress](../authority-ingress.md) explains the public operations.
