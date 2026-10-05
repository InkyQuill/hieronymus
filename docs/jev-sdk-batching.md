# Jev SDK and batching decision (#110)

As of 2026-10-05, use the unpublished workspace crate
[`hiero-decision`](../crates/hiero-decision/UPSTREAM.md), a narrow derivative
of zchee's decision-model-sdk at `c5d4459`. The four-project
[audit](research/2026-10-05-decision-sdk-audit.md) motivated this replacement:
the old registry package stopped resolving on clean release runners.
Rust 1.98 remains the workspace baseline.

The local protocol prepares named questions, retains structured extension
fields and validates independent answers. Authentication and synchronous
bounded HTTP remain in `ProviderTransport`; no SDK HTTP/TLS client, Tower
adapter, async runtime, retry machinery or macro dependency is needed.

Configure the endpoint explicitly, cap responses at 64 KiB, disable retries and
use the caller's timeout (remaining shared deadline for comparisons). Map errors
to fixed diagnostic codes rather than SDK error bodies, which can contain input.
Domain validation remains necessary: SDK parsing does not establish compatible
story scope, authority, calibrated equivalence or transaction-time freshness.
Invalid named answers are omitted while valid siblings remain available for
independent domain validation. Duplicate response keys fail closed; Noul,
confidence, distributions, Choice winners and Score rubrics receive protocol
validation before domain thresholds. The production transport bounds wire
overhead separately and enforces the 64 KiB decoded body limit. This avoids
retrying already valid evidence.

## Batching behavior

The [parallel questions cookbook](https://docs.typesafe.ai/cookbooks/parallel_questions)
asks multiple named questions against one state. Apply that mechanism to bounded
same-scope comparison groups: at most eight pairs and 32 KiB of serialized pair
state per request, with a 16 KiB individual pair cap. Instructions address the
specific named pair; answers are matched by name, never object order. The
cookbook's particular cost/latency measurements are not a performance claim for
our mixed-pair workload.

Both reconsolidation and link processing gather questions before inference.
Link prefetch reads at most eight future pairs without moving the durable cursor;
each mutation still revalidates snapshots and commits its cursor and audit
atomically. No quadratic pair list is materialized. Local vetoes and cached
answers consume no network calls. The existing total pair budget and 60-second
shared drain deadline remain in force. A valid uncertain answer is terminal;
only failed answers enter the explicitly configured fallback, also batched if
it is Jev. Other model providers retain their single-pair context planning.

Prompt relevance already asks literary and technical Noul questions together;
it now uses the same SDK adapter. Separate user messages are not delayed or
pooled across requests.

## Other cookbook suggestions

- [Entity alignment](https://docs.typesafe.ai/cookbooks/entity_alignment): use
  semantic assessment after candidate generation, retaining deterministic
  name/number/polarity vetoes and scope checks.
- [Classifying RAG passages](https://docs.typesafe.ai/cookbooks/classifying_rag_passages):
  relevance and contradiction are distinct signals. This change does not add an
  inference call to recall or let ranking overwrite approved terminology.
- [Citation checks](https://docs.typesafe.ai/cookbooks/citation_check): exact
  evidence and source validation precede model judgment; existing snapshot and
  claim checks remain authoritative.
- [Semantic find](https://docs.typesafe.ai/cookbooks/semantic_find): a forced
  choice does not prove an answer exists. Keep explicit insufficient-context
  outcomes instead of interpreting any winner as equivalence.

Controlled tests cover named mapping, partial failure, terminal uncertainty,
cache reuse, limits and provider envelopes. Native accuracy requires the
separate opt-in synthetic provider test; ordinary unit tests do not qualify it.

## Live synthetic check

In the **batched run** on 2026-10-02, the SDK adapter sent the six existing synthetic calibration
pairs in one batch. All six returned validated assessments: purchased/bought was
equivalent, locked/unlocked contradictory, and closed/shut, before/after,
safe/poisonous and borrowed/lent were insufficient-context. No negative pair was
accepted as equivalent. This is more conservative than the earlier single-pair
run documented in [memory dreaming](memory-dreaming.md); batching is qualified for bounded transport and safe abstention, not equal
semantic recall or general accuracy. The test used a disposable database and
only synthetic text.

The first live adapter attempt exposed duplicate Content-Type headers from SDK
and transport. The adapter now leaves content type, accept and framing to
`ProviderTransport`; the successful rerun and mock regression cover that repair.
