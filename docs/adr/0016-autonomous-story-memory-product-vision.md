# 0016 — Autonomous story memory

Status: accepted 2026-09-06; consolidated 2026-10-05 without changing the decision.
Supersedes ADR 0005's product vision and the human-only lifecycle in ADR 0011.
This is accepted policy, not blanket evidence of implementation or host support.

## Context

Authors need continuity across chapters and sessions without maintaining a memory
database. Requiring them to approve every rendering or curate memories defeats
that purpose. Letting model prose rewrite authority directly makes continuity and
explicit corrections unreliable.

## Decision

The host agent understands the task, recalls context and captures consequential
observations as part of ordinary reading/writing/translation. Hieronymus owns durable
storage, contextual retrieval, consolidation and validated mutations. Plugins deliver
shared skills and MCP configuration; they do not contain a competing backend.

Remember terminology, characters and relationships, voice, chronology, knowledge,
world rules, unresolved threads and useful uncertainty. Keep observations atomic
and evidence-linked; concepts anchor identity and facets retain multilingual forms.
A source archive alone is not story understanding. Compact searchable explanations
can be English-first while exact forms and quotations retain their languages.

The author supplies ordinary corrections and context, not a required promotion,
merge, tagging or scoring ceremony. The console primarily supports connection,
inspection, explanations, correction and configuration. Deep maintenance tools are
optional. Success means useful continuity and fewer repeated corrections, not
stored-record counts or the number of administration screens.

### Authority and correction

Structured terminology remains deterministic under ADR 0011. Agents and Dreaming
may establish, revise, scope or retire learned rules through validated transactional
operations with evidence, concept disambiguation, revisions and audit. Provider prose
or a high confidence score alone is never authority.

An explicit-user instruction has priority within its scope. A later explicit-user
correction can replace it; inference, popularity and negative relevance feedback
cannot. Ambiguity stays tentative and recallable with reasons. Neither human approval
nor unvalidated model output is a fallback for missing evidence.

Clear applied corrections affect the next dependent recall/validation, including
during provider outage. Fact invalidation does not invent a replacement; qualification
does not invalidate all scopes; relevance feedback does not prove a fact false.
Durable ingestion and retry preserve immediate effects. Consolidation cannot silently
resurrect a corrected claim or overrule explicit terminology.

Ordinary agent submissions cannot mint explicit-user origin. Supported console/host
routes and exact selection are described in [authority ingress](../authority-ingress.md).
The present hook parser has a limited grammar; arbitrary conversational phrasing is
not claimed as implemented. Hooks are local-user automation, with the trust limits
in [hook context](../agent-hook-context.md).

### Story applicability

Chronology and viewpoint govern current truth separately from relevance ranking.
A later revelation must not become an earlier character's knowledge. Resolved order
comes from evidence/manifests, not guessed chapter numbers. Unknown context remains
labeled uncertainty. Broad research may return historical/non-current evidence;
strict current-scene validation remains a separate operation.

### Consolidation across tasks

Sessions are storage and agent-lifecycle handles, not boundaries of learned
meaning. Dreaming selects observations across active/completed sessions within
one project/language scope, with a bounded rotating observation cursor. Every
input retains its own source/story context instead of inheriting another task's
chapter or viewpoint. Session IDs do not enter model prompts; observation capture
order is a recency signal, never authority or narrative order.

Later clarification need not explicitly identify an earlier record to correct.
The model may learn the supported current understanding from both observations.
This trades a hard session boundary for gradual model-driven consolidation;
implicit correction accuracy is not guaranteed by batching or synthetic tests.
Deterministic terminology, user authority, evidence disposition and applicability
checks still govern persistence. Co-activation processing spans compatible
project/language scopes; passive decay counts drained project work rather than
individual sessions. Existing session identifiers remain compatible storage/API
handles; this change does not migrate or delete source history.

Manual combining is optional author maintenance. It first sends all selected
compatible crystals to the knowledge Dreaming model and offers an editable title
and memory text. Preview does not alter records; a confirmed merge checks source
snapshots, preserves lineage and archives the originals in one transaction.
Model failure leaves the selection intact and offers retry. This avoids asking
authors to write the combined memory from an empty form. Concept identity merging
remains a separate direct operation into the first selected concept.

### Reliability and integrations

Keep local state ownership, rebuildable indexes and bounded auditable processing.
Configured remote models are allowed; local-first is not a promise of offline inference.
Outages preserve observations/corrections and report degraded work honestly. Restart
and retry must not duplicate effects or fabricate completion.

Shared skills teach context, recall, selective capture, correction, validation,
session completion and recovery. Correctness cannot depend on hooks a host lacks.
Claude and zCode share the Claude-format bundle; there is no separate zCode backend.
The current required native workflow matrix is Claude/Codex/Pi. Pi's passive package
supplies skills/MCP context, not trusted correction ingress. Packaging is distinct
from actual host acceptance; see [host checks](../agent-host-acceptance.md).

## Alternatives and consequences

Human-curated proposals were rejected as the ordinary workflow because they create
recurring author labor. Unrestricted autonomous rewrites were rejected because they
weaken evidence and deterministic authority. Validated autonomous maintenance is the
chosen tradeoff: more explicit context/revision handling, less routine human curation.

The Rust runtime contains authority, applicability and correction paths; their
existence does not qualify every story scenario, model or host. Current integration
contracts live in maintained guides, and unresolved qualification belongs in the
[roadmap](../roadmap.md). Old Python paths and historical gap lists are not current
implementation requirements.

## Acceptance scenarios

- Important facts, voice and terminology survive work across chapters without
  manual fact labeling or a review queue.
- Consistent evidence establishes usable learned terminology; ambiguous identity
  never becomes an incorrect mandatory rule.
- An applied user rendering affects the next validation and later Dreaming preserves it.
- An applied invalidation no longer appears as unquestioned current truth after restart.
- Earlier/later scenes retain their own chronology and character knowledge.
- Provider failure delays consolidation without losing observations or immediate effects.
- Advertised hosts load skills and MCP and perform the actual workflow; generated
  files, isolated transport tests and another host's evidence do not substitute.
