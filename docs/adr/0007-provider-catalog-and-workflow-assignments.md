# 0007 — Provider profiles and bounded model decisions

Status: accepted; context budgeting amended 2026-09-13. Consolidated 2026-10-05,
including the accepted Jev research limits and local protocol replacement.

## Context

Endpoints and credentials are reusable access configuration; Dream passes are
consumers with their own models and budgets. Mixing both into `dream.conf` made
routing unclear and duplicated secrets. Optional model judgments also need a
boundary that preserves evidence, authority and predictable failure behavior.

## Decision

`provider.conf` owns named profiles, protocol type, endpoint, private key, timeout,
optional context window and default provider/model. `dream.conf` owns Dream prompts,
scheduling, budgets and per-workflow provider/model/enabled assignments. Profiles
support OpenAI-compatible, Gemini, Anthropic and Ollama protocols. Book files and
the runtime database do not own provider credentials.

Resolve the workflow's explicit assignment, then catalog defaults where supported;
fail with a clear configuration error if unresolved. Saved workflow assignments
should be explicit. Defaults simplify initial setup, not conceal routing.
Provider checks and model-list hints describe a profile, not successful Dreaming.

Settings keep profile editing separate from workflow assignment. Redacted secret
markers preserve the saved key; a new key changes only the private catalog.
Legacy `dream.conf.providers` conversion preserves exact keys and workflow references,
using stable IDs. Conflicts fail before overwriting different profiles; remove old
blocks only after successful conversion. ADRs 0001/0010 govern private-file handling,
atomic promotion and recovery.

### Context and time budgets

A configured `context_window` is at least 1024 tokens and includes input/output.
Fit complete records, instructions, source snapshots and reserved output within all
enabled pass budgets; omitted work stays pending. A record that cannot fit fails
explicitly without being consumed. Valid output record counts are organization
guidance, not a reason to reject useful provider output.

Native Ollama discovers context through `/api/show`, respects lower profile/Modelfile
bounds, requests explicit `num_ctx`/`num_predict` and disables truncation/context
shifting. Cloud models use advertised capacities rather than arbitrary local caps;
structured extraction disables thinking where supported.

Timeouts are positive configurable request budgets, without an arbitrary fixed upper
cap. Memory comparison has a separate timeout and optional explicit backup; a valid
uncertain result is terminal. No implicit cloud backup or retry-until-accepted loop.
See [Memory and Dreaming](../memory-dreaming.md) for current operational limits.

### Local decision protocol — accepted 2026-10-05

Use the unpublished workspace `hiero-decision` crate, a narrow derivative of zchee's
audited protocol, replacing the unavailable registry dependency. Preserve donor
licenses and exact provenance in [UPSTREAM.md](../../crates/hiero-decision/UPSTREAM.md).
This is not a full SDK fork or another provider runtime.

The protocol validates named Noul/Choice/Score questions and answers, distributions
and response identity. Valid siblings survive malformed independent answers;
duplicate JSON keys and oversized responses fail closed. Existing bounded synchronous
`ProviderTransport` owns HTTP/TLS, credentials and deadlines. No SDK async runtime,
second client, implicit retry, environment-selected endpoint or payload logging.
Domain consumers retain scope, evidence, vetoes, thresholds and mutation authority.
[Decision adapter](../jev-sdk-batching.md) owns the batching/transport details.

Reason: a small protocol boundary avoids clean-runner registry resolution failures
and unnecessary transport dependencies while retaining validated answers. It does
not establish model accuracy or native installation success.

### Jev expansion limits — accepted research outcomes, 2026-10-03

Keep conservative comparison validation and deterministic provenance/name/number/
polarity vetoes. Do not lower thresholds or widen destructive consolidation based
on a small synthetic pilot. Keep prompt relevance's independent rubric/thresholds;
its Japanese false negative is a measured limitation, not permission to retune around
one case.

| Proposed feature | Decision and reason |
| --- | --- |
| Support enforcement in Dream | Offline fixture only; a small support study does not establish safe archival or factual truth. |
| Passage classification/reranking | No runtime addition; an ambiguous pronoun was confidently resolved without identifying evidence. |
| Automatic entity alignment | Keep advisory; aliases and matching labels do not establish same identity/context. |
| Remote source-span selection | Keep the deterministic offline prototype; validate exact original bytes/IDs/hashes, without claiming agent productivity gains. |
| Layout recovery | No runtime addition; near-total abstention does not justify a network pass or changed chunking. |

Revisit with representative independently judged inputs and a concrete product
benefit. Source/scope/revision checks stay in code. Unavailable or uncertain judgments
preserve source records and pending work. Historical raw measurements belong in
local archives/Git history; opt-in evaluation fixtures remain under `scripts/fixtures/`.

### PDF extraction — retained 2026-10-03

Keep the current `pdf-extract` implementation. The audited default-feature graph
has no unused bundle whose disabling simplifies the native app. A subprocess adds
installed tooling; a replacement parser makes extraction quality our responsibility.
Revisit only with representative documents and a measured improvement in extraction
or deployment complexity. Empty/scanned documents must fail explicitly.

## Consequences

Endpoint configuration is reusable, processing assignments remain understandable,
and secrets have one owner. Model judgments remain optional, bounded and subordinate
to domain validation. Small studies can justify rejecting a proposed runtime feature;
they do not qualify production accuracy or authorize new mandatory cloud steps.
