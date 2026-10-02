# Jev SDK and batching decision (#110)

As of 2026-10-02, use `typesafe-sdk-rust` **0.2.0**, pinned in Cargo.lock,
with default features disabled. Raise the workspace, development toolchain and
CI pin to Rust **1.98.0**. Both the published 0.2.0 package metadata and the
[0.1.1 manifest](https://github.com/zchee/typesafe-sdk-rust/blob/v0.1.1/Cargo.toml)
require Rust 1.98; downgrading the SDK does not preserve the previous toolchain.

The [SDK](https://github.com/zchee/typesafe-sdk-rust) provides prepared named
questions, authentication, response decoding, response limits and explicit retry
configuration. Its custom service interface fits the existing mockable
`ProviderTransport`. Disable its Hyper, macro and tracing features: reuse our
bounded blocking transport and avoid another TLS stack or body logging. The
adapter runs a time-enabled Tokio runtime on a scoped thread, making synchronous
Dream and prompt-delivery callers safe even inside an existing runtime.

Configure the endpoint explicitly, cap responses at 64 KiB, disable retries and
use the caller's timeout (remaining shared deadline for comparisons). Map errors
to fixed diagnostic codes rather than SDK error bodies, which can contain input.
Domain validation remains necessary: SDK parsing does not establish compatible
story scope, authority, calibrated equivalence or transaction-time freshness.
When SDK response decoding rejects one answer, bounded JSON remains available
for independent domain validation of the other answers; no unvalidated field
can authorize a merge. This avoids retrying already valid evidence.

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
