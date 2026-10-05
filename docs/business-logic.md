# How Hieronymus memory works

Your agent does the writing and decides what is worth remembering. Hieronymus
keeps those observations across tasks, finds useful context and consolidates it.
The console lets you inspect memory, supply pointers and correct mistakes.
Ordinary use does not require approving a queue or manually tagging every fact.

## What is stored

| Term | Meaning |
| --- | --- |
| Series | A work or group of related books; its identity is language-neutral. |
| Task session | A bounded piece of work with a series, languages and task/story context. |
| Short-term memory | A compact observation from an active session, usable before consolidation. |
| Crystal | A long-term memory: a fact, lesson, preference or background knowledge. |
| Concept | A durable identity, such as a character, place or ability. |
| Facet | A name, rendering or other language/context-specific detail of a concept. |
| Claim | A statement bound to a memory or source, with validity and applicability. |
| Terminology rule | Structured source/target forms and matching policy enforced by validation. |
| RAG source | Imported source text used as evidence during retrieval. |
| Dreaming | Background processing of observations into durable, evidence-linked memory. |

A concept rename preserves identity; replacing a rendering does not merge two
characters. Tags describe context but do not establish identity. A source locator
helps find material; immutable captured bytes and their hash establish evidence.
Strength describes usefulness in recall; confidence describes evidential certainty.
Neither score grants authority.

## The normal task

1. The agent resolves the actual project, languages and applicable context.
2. It reuses a compatible active session or starts one for this task.
3. It recalls memory and the applicable deterministic terminology contract.
4. It reads, writes or translates, recording significant observations and uncertainty.
5. It validates applicable terminology and completes the session it owns.
6. Dreaming processes observations across sessions in continuing batches.

A nested skill leaves its caller's session active. Finishing an assistant turn,
switching chats or pausing prompt capture does not finish the task. SessionEnd
hooks provide a fallback for one explicitly bound session; hosts without working
hooks need explicit completion. Completed does not mean dreamed.

Project instructions determine which memory to use and which writes are allowed.
By default, project files are primary and Hieronymus is additional memory.
A project can choose another arrangement; that does not forge authority or
silently change the status of a stored rule. See [translation integration](translation-workspace-integration.md).

## Recall and story context

Ordinary recall combines recent observations, crystals and source passages. Its
default `OmniscientResearch` mode inspects broadly and returns uncertainty and
non-current evidence with labels. It is useful for research, not a declaration
that every returned statement is true in the current scene. Read `non_current`
as well as `results`. Explicit `Current` mode applies strict scene filtering.

Story labels such as book/chapter can boost relevance. Resolved applicability,
ordered positions and knowledge gates govern current truth separately. Chapter
numbers alone do not establish chronology. A narrator's knowledge does not imply
that a character knows the same revelation. Unknown context stays unknown.

For example, a chapter-12 revelation may be useful in research but must not
become the chapter-3 character's knowledge. A later change in level can coexist
with the earlier level; similarity is not permission to replace it.

The deterministic contract is separate from ranked hits. Applicable active
terminology stays binding even when another passage ranks higher. Source passages
are advisory; conflicting renderings can remain visible with conflict markers.
Semantic failure is reported explicitly. Lexical results do not establish that
semantic retrieval succeeded.

## Corrections and authority

| Signal | Effect |
| --- | --- |
| “Use this rendering” with verified identity and scope | Changes the applicable terminology contract through the validated authority path. |
| “This memory is wrong” with an exact selected claim | Invalidates that claim in its scope; invents no replacement. |
| “This is only a suspicion” | Qualifies the selected claim without declaring it false everywhere. |
| “This result was not useful” | Changes relevance feedback, not factual validity or terminology authority. |
| Ambiguous or unsupported input | Remains tentative with an explanation. |

The app distinguishes learned decisions from explicit user instructions. Agents
and Dreaming can establish learned rules through structured evidence checks.
Explicit user direction has priority within its scope; inference, popularity,
negative recall feedback and passive decay cannot undo it. A later verified user
correction can replace it. Legacy approved rules retain migration protection.

An ordinary MCP call remains agent input even if it says `source_role=user` or
quotes the author. Explicit-user corrections use the local console or an
independently delivered supported host prompt. Hooks are local-user automation,
not cryptographic proof of native-host provenance. See [authority ingress](authority-ingress.md)
and [hook context](agent-hook-context.md) for the exact boundary.

Once a clear correction is applied, the next dependent recall/validation must
respect it without waiting for Dreaming. A receipt records the result and
revision. Exact retries replay that result; changed inputs conflict rather than
rebasing silently. An accepted/tentative signal is not an applied rule, and
an enqueued consolidation job is not completed processing.

The current natural-language hook parser accepts a limited single-line English
command grammar with multilingual quoted payloads; the console offers structured
corrections. See [supported input](../crates/hiero/resources/correction-input.md).
Free conversation is the product direction, not a claim that arbitrary phrasing
is already understood.

## Consolidation, forgetting and failure

Dreaming combines pending observations across sessions within one project and
language scope. Session completion is not an eligibility gate. Each observation
retains its own task/story context; a batch does not give every input the first
task's context. Later clarifications can revise earlier assumptions without an
explicit correction marker; the model must distinguish corrections from story
changes. Capture order supplies recency, not authority.
Every consumed observation needs a committed successor or an authorized discard
reason; unsupported outputs leave it pending. Merely listing an input as covered
cannot archive it. Valid output is preserved in full; resource budgets split
work into further batches rather than rejecting it by record count.

Provider failures preserve observations and already applied corrections. Runs
and phase audits explain what committed and what failed. Maintenance can report
a warning after persistence without pretending that the failed phase passed.

Passive decay lowers eligible advisory strength after meaningful completed work,
with a floor; it deletes no records and does not change confidence. Rules,
explicit user authority, recent use and uncertain applicability are protected.
Manual combining first asks the configured knowledge Dreaming model for an
editable proposal over all selected crystals. Preparation is read-only. User
confirmation creates the new memory and archives its sources atomically, retaining
lineage; changed source snapshots require a new proposal. Rules and incompatible
project/language/type scopes cannot be merged through this action.

Comparison may consolidate compatible duplicates only after evidence, scope and
snapshot checks. An uncertain answer preserves both records. See
[Memory and Dreaming](memory-dreaming.md) for budgets and settings.

SQLite owns authoritative data. Search indexes are derived and rebuildable;
corrupt indexes must not destroy memories. Uninstall keeps user data by default,
and upgrades preserve backups and ownership checks.

These rules are accepted product contracts. Exact runtime behavior, host support
and model quality require their own evidence; see [ADRs](README.md#why-these-rules-exist)
and [verification](rust-cutover-rehearsal.md).
