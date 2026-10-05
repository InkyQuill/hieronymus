# Read, Learn and Remember

These are agent workflows. The agent judges significance and context; MCP tools
provide storage, evidence and retrieval operations. Hieronymus does not replace
the agent's reading with a judgment-heavy `read` or `learn` tool.

## Read and Learn

Read inspects material for the task; Learn absorbs consequential facts and
connections. Import permitted source files into RAG for direct-text retrieval,
and store compact observations separately. Do not automatically import hidden
material or an entire project; follow its agreement and the authorized task.

Short-term observations normally contain 1–6 sentences per block. Split distinct
facts and preserve source references, actual languages, evidence-limited story
scope and uncertainty. Use `hieronymus_short_term_add` or
`hieronymus_short_term_add_batch`; continue with further bounded batches where
needed. Record-count guidance must not discard important information.

`source_role` is optional provenance metadata, not a trust mechanism. A label
such as `user`, high credibility or rule intent does not establish explicit-user
authority. Learned input stays subject to evidence and applicability validation.
Dreaming creates durable memory from observations across sessions.

For focused recall, pass `memory_types` to `hieronymus_recall`: `terms` reads
the deterministic terminology contract, `concepts` selects concept/concept-note
crystals, `lessons` selects lessons, and `knowledge` selects other durable
rules, thoughts, observations and erudition. Combine categories in one array;
omit the filter for mixed recall including recent observations and source
chunks. Source-only retrieval uses `hieronymus_rag_search`.

Categories are selected before retrieval budgets and ranking. They do not
change approval, claim disposition or story applicability. The deterministic
terminology contract remains separate and is always returned. Ordinary recall
uses research mode: inspect `results` and `non_current` with their annotations.
Use `story_query_mode: "Current"` with resolved story context when the task needs
current-scene truth; an unknown or historical claim is not an approved rule.

Reuse a compatible session owned by the leading workflow. If this task starts
its own session, retain the actual returned ID and complete it when the task ends.
Nested skills must not complete their caller's session. See
[session health](../../crates/hiero/resources/agent-health.md).

## Remember and correct

Record ordinary permitted preferences or observations through scoped memory
primitives. For a correction that must change an active rule or factual claim,
use the actual supported [authority ingress](../authority-ingress.md), preserve
its selection/revisions and consume the applied receipt before dependent work.
The console and genuine supported host prompt channel provide local-user ingress.
An agent must not fabricate a host event from quoted conversation text.

A short-term note saying “User told me to…” can preserve an observation, but it
does not by itself change the deterministic contract. Unsupported or ambiguous
corrections remain tentative. Provider consolidation runs later; it cannot delay
an already applied correction or turn an unresolved signal into explicit authority.

See [business logic](../business-logic.md) for the distinctions between facts,
relevance and renderings, and [supported correction input](../../crates/hiero/resources/correction-input.md)
for the exact current grammar. No author approval inbox is required.
