# Memory and Dreaming

Dreaming turns observations from completed tasks into durable, evidence-linked
memory, then maintains the affected records. It runs in the background; authors
inspect problems and correct mistakes instead of curating a required queue.
[Business logic](business-logic.md) introduces the memory terms and authority rules.

## Sessions and pending work

A series is language-neutral. Sessions retain the actual languages, task and story
context. Short-term memories belong to an active session; recall requires one,
uses its context and rejects conflicting explicit arguments.

Complete the session after the task's writes and validation. Completion makes its
observations eligible for Dreaming; it does not prove processing has finished.
Recalled crystals create deduplicated working copies with original claim bindings.
Invalidated memories are not reactivated. A RAG-only read does not create those copies.

Each provider batch selects from one completed session, oldest-first with a
restart-safe rotating cursor. A deferred old task cannot indefinitely starve newer
work. Large sessions continue through further batches. No selected task's context
is applied to another task's observations. Source roles are freeform provenance
metadata; they do not authenticate a user or grant rule authority.

Memory prose is compact and primarily English for searchability, with exact
source forms, renderings, quotations and relevant non-English text preserved.
Facets cite the selected observations supporting them; concepts retain identity
through renames and ordinary decay. Inferred thought memories remain labeled and
rank below comparable source-backed memory.

## What commits

Seven passes cover concepts, terminology candidates, rule crystals, knowledge
crystals, relations, reinforcement and coverage. Outputs cite selected input IDs.
The app stages, validates and deduplicates output before its persistence transaction.
Evidence, context isolation, authority and snapshot checks remain necessary even
when the JSON is valid. Provider output alone cannot override an active rule.

For each selected input, the persistence audit records an `input_dispositions` outcome:

| Outcome | Consequence |
| --- | --- |
| Represented by specific committed crystal/facet IDs | Input can be archived with those successors. |
| Intentionally discarded with an accepted reason | Input can be archived under the discard policy. |
| Deferred / `no_committed_successor` | Input remains pending; coverage enumeration is insufficient. |

A discard reason is nonempty and at most 512 characters. Explicit-user evidence
and rule-intent inputs cannot be discarded through the provider path. Accepted
successors take precedence over discard requests. Dispositions, successor links,
archive changes and persistence completion commit together; rollback leaves inputs
retryable. A session becomes dreamed only when no pending input remains.

Valid provider output is persisted in full even when it exceeds configured output
record guidance. Input/context and deterministic work budgets split processing
into batches; they are not arbitrary output rejection ceilings. Unsupported or
empty outputs leave work pending. A zero-progress drain stops and reports pending
rather than retrying forever.

## Scheduling, providers and concurrency

Configure profiles in `provider.conf` and Dream assignments/prompts in `dream.conf`
through Settings. [ADR 0007](adr/0007-provider-catalog-and-workflow-assignments.md)
explains configuration ownership. Cloud models use advertised context capacities;
Ollama needs a bounded plan and must not silently truncate input. Structured
extraction disables thinking where supported.

Scheduled runs respect the minimum pending-memory threshold. The urgent cap or
backlog escape after repeated low-count skips can run earlier. Thresholds count
short-term memories, not crystals. Manual runs drain eligible pending work,
including the last small batch:

```sh
hiero tool-call hieronymus_dream --args '{}' --json
```

Cycles serialize per data root through `dream-cycle.lock` and an in-process guard.
The daemon coalesces manual requests into its active or pending run; the MCP
call waits for that run's drain result. Scheduled cycles use nonblocking admission. `dream-cycle.json`
is informational; a live OS lock is never broken based on stale state.

Provider profile timeouts are positive, configurable request budgets. Comparison
uses its own timeout. There is no fixed one-minute Dream drain deadline or arbitrary
upper timeout; long-thinking providers may need 300 seconds or more. Changed
settings take effect on the next run.

## Audits and recovery

Audits retain affected input/record IDs, evidence and applicability, provider/model
and configuration identity, prompts and bounded redacted payloads where available,
parse warnings, accepted/rejected outputs and actual mutations. They are append-only;
corrections append later events. Completion of one phase does not prove later
maintenance or a provider succeeded.

A failed provider or coverage pass applies no staged persistence outputs and leaves
work retryable. Already committed immediate corrections remain effective during
provider outages. Correction gathering and source-index recovery have separate
supervised workers; pending/parked jobs are not successful consolidation.

Maintenance operates on a bounded affected set, not the whole store. Failed
salience decay after persistence can complete with committed counts and a visible
failed-phase warning. If the warning cannot be recorded, the run fails honestly
while retaining the committed persistence counts.

## Passive decay

`completed_session_v1` gives one opportunity per fully consumed session whose
fresh evidence persisted as a crystal or facet. Idle time, empty/failed runs,
partial batches and another series' work create no opportunity. Unknown story
order cannot establish non-use; stored session context governs eligibility.

Eligible unused advisory crystals lose 0.02 strength with a 0.20 floor. Confidence,
text and status stay unchanged; nothing is deleted. Rules, explicit-user authority
or confirmation, recall/working copies, recent concurrent use, newly created/edited
records and unknown/qualified/invalid claims are protected.

The phase scans at most `max_total_affected_crystals` and uses only the remaining
crystal-change budgets after earlier phases. A capped opportunity is terminal;
restart cannot apply catch-up decay. Opportunity markers, affected IDs, deltas
and successful audit commit together. Passive decay does not change the content-edit
timestamp. Evidence-based reinforcement can later restore strength.

## Semantic memory indexing

The server indexes crystals and pending observations in continuing background
batches, independently of recall. SQLite `memory_vectors` is derived cache data;
missing, changed-text or model-incompatible rows form the backlog. Inference runs
outside write transactions and publication rechecks the source. Restart resumes
from authoritative rows. Maintenance continues throughout the server lifetime,
including after idle periods: additions and edits are indexed, while deleted or
archived sources leave the index. Progress totals reflect current memory; a
shrinking total is normal database activity, not an indexing failure.

Overview exposes indexed/remaining counts and failures; the tray indicates work.
Recall reads the index and distinguishes unfinished indexing from a bounded
candidate scan. `memory_semantic_pending` and `memory_semantic_unavailable` are
explicit warnings; lexical matches do not establish semantic success. See
[semantic storage](adr/0013-semantic-index-and-platform-support.md).

## Working copies and comparison

Dream input separates fresh `memories` from `activated_memories`. At most eight
compatible copies from up to 512 recent candidates augment a request. Fresh
observations retain at least one complete record and three quarters of their fitted
allowance. Final rendered instructions, snapshots and reserved output must fit.
Copies cannot satisfy fresh coverage or justify double reinforcement.

Exact unchanged copies can reinforce strength once without increasing confidence.
Changed names, numbers or polarity trigger conservative vetoes. Other changes
require verified equivalence before absorption. Series, languages, applicability,
concept identity, evidence and current snapshots must agree. Source rows, lineage,
metadata and links are retained. Uncertainty or incompatibility preserves copies;
coactivation can remain an advisory association.

**Memory comparison** is unassigned by default. Configure a primary and optional
explicit backup privately in `comparison.conf` or Settings. Jev reuses the Prompt
relevance credential; other model IDs use provider profiles. There is no implicit
cloud backup. A valid uncertain answer is terminal; only unavailable configuration,
transport/timeout or malformed output can use the configured fallback.

```toml
max_pairs_per_run = 8
timeout_seconds = 300

[primary]
provider = "my-provider"
model = "my-model"
```

The per-run model pair budget is 1–32. Jev batches up to eight eligible pairs,
16 KiB per pair and 32 KiB combined state. Other providers receive one pair per
request. Discovery and inference share that request's configurable timeout;
see [decision adapter](jev-sdk-batching.md).

Cache identity includes exact pair versions, routing and rubric/parser versions.
Transaction-time validation remains necessary on cache hits. Outages have a
five-minute cooldown; credential, configuration or content changes invalidate
identity. Corrupt entries are discarded. Unchanged unresolved copies can be parked
with durable snapshot markers instead of repeatedly scheduling Dream; evidence,
claim, applicability or routing changes reopen them. Budget exhaustion never parks
unassessed work. Invalid optional comparison configuration disables comparison
with a diagnostic, while independent Dream phases remain available.

## Inspecting sources and results

Capture `source_ref` when available; session book/chapter metadata adds context.
Recall projects source locations from retained observations. Citations group the
same file revision/context into exact line ranges, preserving gaps and all contributing
memory IDs. No location is invented. A locator is not an immutable evidence receipt
or chronological ordering. Archival preserves locators; hard deletion of their source
rows removes them.

Console search uses Unicode lowercase substring matching over full projected text
and claims before pagination; `%` and `_` are literal. Corrections select one exact
statement with its authority revisions. Changing the selected memory closes the old
correction. Optional merge selection persists across pages/searches in the same
book/view and clears on a book/view change.

Model assignment and transport tests establish routing and safeguards, not literary
accuracy. Live synthetic probes, representative text judgments and installed-host
checks are separate opt-in evidence; see [runtime checks](rust-cutover-rehearsal.md).
