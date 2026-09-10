# Autonomous Story Memory For Agents

## Status

Accepted on 2026-09-06 from the owner's product direction.

Supersedes [ADR 0005](0005-product-vision.md) in full as the product vision.
Amends [ADR 0011](0011-deterministic-terminology-and-graded-memory.md): its
structured terminology authority and deterministic enforcement remain; its
human-only approval and lifecycle policy is superseded by this decision.
The corresponding human-only lifecycle requirements in the
[Rust terminology design](../superpowers/specs/2026-08-31-rust-terminology-memory-design.md)
and candidate-only agent workflow instructions are superseded as well.

This is the target product contract, not a claim that the implementation
already satisfies it. Runtime, transport, storage, provider, release, and
cutover decisions in ADRs 0006–0015 remain in force except for the explicit
ADR 0011 amendment above. Where migration plans preserve mandatory human
memory review, this ADR takes precedence; implementation changes still need
focused designs and verification.

## Context

Hieronymus exists to help AI agents remember the vital parts of a story,
its terminology, and its flow across sessions and long works. The author
should be able to work on the book without managing a memory database.
Agents and the app decide what matters, preserve it, retrieve it when useful,
and revise it as understanding develops.

ADR 0005 already described selective learning, autonomous dreaming, and agent
skills. However, it also centered extensive manual memory administration.
ADR 0011 subsequently required a human to approve every active terminology
rule and every change to it. Generated agent skills now explicitly defer
terminology approval to a human. That turns the author into a required memory
curator and contradicts the intended product.

The owner's interaction should be as simple as “this is remembered wrong” or
“translate this as …”. Such input is a correction signal for the system to
interpret and apply, not the start of a review queue.

## Decision

Hieronymus is a local-first, autonomous story-memory application delivered to
agents through plugins, workflow skills, and MCP. Literary translation is a
primary use case; story understanding and continuity are core responsibilities.

“Brain-like” describes the behavior: selective attention, association,
consolidation, contextual recall, revision, and forgetting. It does not require
a biological simulation, a particular database, or an autonomous agent that
writes the book itself.

### Responsibility And Ordinary Workflow

The host agent understands the current task and material. Skills teach it to
recall relevant context before working, notice significant information while
reading or translating, record useful observations, and check its output
against applicable terminology. This happens as part of ordinary work without
requiring the author to invoke Learn or Remember for every passage.

The application owns durable memory, consolidation, applicability, conflict
handling, and validated mutations. Dreaming selects, connects, reinforces,
compacts, revises, or retires observations automatically. MCP exposes explicit
operations; it does not replace the host agent's contextual judgment.

The normal loop is:

1. Recall relevant story context and the applicable terminology contract.
2. Read, write, or translate with that context.
3. Record significant observations and uncertainty in compact form.
4. Consolidate and maintain memory automatically within resource limits.
5. Use corrections and subsequent evidence to improve future recall and work.

There is no mandatory human review, approval inbox, or recurring curation step
in this loop. Candidate states are internal uncertainty states, not assignments
for the author. Installation, provider configuration, and optional diagnostics
are operational tasks; they do not justify content-review gates.

### What The System Remembers

Memory should preserve information likely to affect future understanding or
writing, including:

- characters, identities, relationships, motivations, and changes over time;
- events, their order and causes, consequences, and unresolved story threads;
- world rules, abilities, places, items, and recurring concepts;
- terminology, names, renderings, aliases, and context-dependent usage;
- narrator and character voice, register, tone, and recurring stylistic choices;
- scene continuity, promises and revelations, and who knows what at a given
  point in the story;
- translation decisions, explicit corrections, uncertainty, and contradictions.

Importance is inferred by agents and the app from recurrence, consequence,
connections, task relevance, and explicit user direction. The author does not
assign importance scores, tags, links, or memory lifetimes.

Story position and viewpoint matter. A later revelation must not silently
rewrite an earlier character's knowledge; a changed relationship must not erase
its previous state. Recall distinguishes an evolving story fact from an error
in memory. Scope-aware applicability must prevent later information from being
presented as already true or known at an earlier point. Exact representation
and ranking policies belong in focused designs.

Series remain language-neutral. Concepts provide identity anchors; facets retain
language- and scope-specific names and renderings. Compact memory explanations
remain English-first, preserving exact source forms, target renderings, and
necessary evidence in their original languages. Source retrieval can supply
passages as evidence; a source archive alone is not story memory.

### Autonomous Memory And Terminology

Keep the separation established by ADR 0011: graded memories guide reasoning;
structured active terminology rules determine validation. Fuzzy ranking cannot
silently override an applicable active rule.

Remove the human-only lifecycle restriction. Agents and dreaming may submit
rule decisions, and the application may activate, replace, scope, supersede,
or archive rules automatically through validated, transactional, audited
operations. Provider prose alone is not an authoritative mutation. Structured
validation, concept disambiguation, evidence, and revision checks still apply.

Distinguish rules learned by the system from explicit user instructions:

- The system may establish and revise learned rules when supporting evidence
  and applicability justify the decision. Automatic revision is explicit and
  audited; confidence decay or recall ranking cannot change an active contract.
- An explicit user rendering instruction has priority within its scope. Once
  its meaning and concept are clear, the app applies it through the same
  validated authority path without asking for a separate approval. Subsequent
  inference, popularity, or negative recall feedback cannot overrule it. A
  later explicit user correction can replace it.
- Ambiguous or unsupported interpretations remain tentative and recallable as
  uncertainty. They do not become hard rules merely to empty a backlog. The
  system seeks context or later evidence and continues useful work without
  requiring the author to adjudicate candidates.

Do not resolve ambiguity by guessing a global string replacement. Automatic
activation criteria and conflict policies require implementation specifications
and tests; neither human approval nor unvalidated provider output is the
fallback policy.

### Corrections Are Signals, Not Curation

The user may correct a remembered fact, point out a bad recall, or prescribe a
translation in ordinary conversation or through the console. The system links
the signal to the relevant concept, memory, passage, language, and scope. The
user should not need to find database IDs or edit crystal records.

Clear corrections must affect the next relevant operation, even if background
consolidation has not run. A superseded fact must not keep appearing as current
truth while waiting for dreaming. A clear rendering instruction must reach the
validated terminology contract before dependent validation reports success.
Durable correction ingestion and later consolidation must preserve that effect
across restarts and retries.

“This recollection is wrong” can invalidate or qualify a claim without supplying
its replacement. “This result was not useful” affects relevance and does not
prove the underlying fact false. “Use this translation” expresses terminology
intent. The app must distinguish these signals rather than treating all
negative feedback as deletion or all corrections as global rules.

Ambiguous corrections remain visible as uncertainty and must not silently
alter unrelated memory. The system may explain what context is missing, but
routine memory maintenance does not stop for human review.

### Plugin And Skill Delivery

Agent plugins are a first-class distribution surface. Target hosts include
Claude, Codex, zCode, and other compatible agent environments. Each integration
packages the shared workflow skills and MCP connection configuration using the
host's supported plugin or extension mechanism. Hooks may improve automatic
session recall and capture where supported; correctness must not depend on
hooks that a host does not offer.

Skills teach agents when and how to recall, read, learn, record corrections,
validate terminology, preserve story position and uncertainty, and recover from
unavailable services. They must make selective memory use habitual and must
not instruct agents to wait for human terminology approval.

Host adapters share one behavioral contract. Integration files contain no
copied backend or competing memory authority. Runtime and managed integration
assets remain separate from book content. Optional workspace skill installation
must not become a requirement for every book.

zCode consumes Claude plugins and uses the same Claude-format bundle. Its
integration work is shared packaging, installation guidance, and compatibility
checks; a separate plugin format or dedicated adapter is not required. A host
without native plugins should receive a documented supported MCP-and-skills
setup where possible, with capability differences stated plainly.

### Local Application And Reliability

Retain the local daemon, SQLite-backed memory, bounded provider-backed dreaming,
and rebuildable search indexes. Local-first means local state ownership;
configured remote model calls remain possible. The Rust runtime and Svelte web
console direction remains governed by the existing technical ADRs.

The console primarily supports setup, status, inspection, explanations, and
correction signals. Deep maintenance tools may exist for diagnosis and recovery,
but ordinary success must not depend on manual merge, split, promotion, tagging,
reinforcement, or dream review.

Consolidation is bounded, retryable, and auditable. Failed or unavailable
providers leave observations and corrections durable and usable; the app reports
degraded operation without fabricating successful consolidation. It should
resume automatically when configuration and service availability permit.

Auditability supports explanation and recovery after automated decisions. It
is not a requirement for a human to inspect or approve those decisions.

## Alternatives Considered

1. **Human-curated memory with agent proposals:** offers explicit control but
   requires continuing author labor. Rejected as the ordinary product model.
2. **An unrestricted model that rewrites memory and terminology directly:**
   removes manual work but makes corrections and deterministic contracts
   unreliable. Rejected.
3. **Autonomous maintenance with structured authority and correction signals:**
   chosen. Agents judge relevance; the app validates and applies decisions;
   explicit user direction remains authoritative without a review ceremony.

## Consequences And Implementation Gaps

The current repository contains reusable memory, dreaming, terminology, and
plugin foundations. The following observed gaps must be addressed by later
implementation work:

- `src/hieronymus/agent_assets.py` and `crates/hiero/src/agent_plugins.rs`
  instruct agents to leave terminology for human approval; Rust asset tests
  also protect that obsolete instruction.
- ADR 0011 and the Rust terminology design impose human-only lifecycle
  transitions. The terminology and application mutation paths need an explicit
  autonomous authority design, preserving user corrections and auditability.
- The Rust plugin generator already produces the Claude-format bundle consumed
  by zCode. Document that shared installation path and check compatibility;
  absence of a separate zCode generator target is not a missing adapter.
- Existing Read/Learn/Remember skills and manual administration surfaces need
  alignment with automatic capture, immediate correction effects, and optional
  inspection. Plugin generation alone does not prove that normal agent work
  reliably remembers important information.
- Story-flow coverage, temporal applicability, and correction propagation need
  end-to-end verification; storage and recall primitives alone do not prove
  that the product remembers a story usefully.

This ADR does not implement those changes, authorize a runtime cutover, or
claim host compatibility that has not been tested. Historical baselines remain
records of previous behavior, not requirements to reproduce human review.

## Product Acceptance Scenarios

- An agent works across chapters and sessions, retrieves important prior facts,
  voice, and terminology, and records new significant information without the
  author labeling facts or opening a memory queue.
- Consistent evidence establishes usable terminology automatically; ambiguous
  names remain contextual uncertainty rather than incorrect mandatory rules.
- The user says “translate this as X”; the next applicable validation uses X
  without a promotion action, and later dreaming cannot silently undo it.
- The user says “that memory is wrong”; the next relevant recall no longer
  presents it as unquestioned truth, including after restart and consolidation.
- A revelation changes what is known later in the story while recall for an
  earlier scene preserves that scene's knowledge and continuity.
- A provider outage delays consolidation without losing observations or
  corrections, and recovery resumes work without author curation.
- Each advertised host installation supplies working MCP access and skills
  that exercise the same autonomous loop. Verify the shared Claude bundle in
  Claude and zCode, and the Codex bundle in Codex.

Success is useful continuity and fewer repeated corrections during real story
work, with no required human memory-review steps. Counts of stored memories,
accepted proposals, or console features are not substitutes for that outcome.
