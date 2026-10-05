# Decision adapter and comparison batching

The workspace uses unpublished `hiero-decision`, a narrow named-question protocol
rather than a full external SDK. [ADR 0007](adr/0007-provider-catalog-and-workflow-assignments.md)
records the decision and rejected runtime expansions;
[UPSTREAM.md](../crates/hiero-decision/UPSTREAM.md) retains donor/license provenance.

## Protocol and transport boundary

Prepare named Noul/Choice/Score questions with structured extension fields and
match answers by requested name, never object order. Validate probabilities,
distributions, winners and score rubrics. Omit invalid independent answers while
preserving valid siblings; duplicate JSON keys and oversized bodies fail closed.
Protocol validity does not prove story equivalence, authority or current evidence.

Synchronous `ProviderTransport` owns endpoint/authentication/TLS/deadlines. Jev uses
an explicit endpoint, no redirects or automatic retries, a 64 KiB decoded-body cap
and bounded wire overhead. Fixed diagnostic codes replace unsafe echoed error bodies.
No async SDK runtime, second HTTP client, environment-selected endpoint or payload
logging is added. Discovery and inference share the configured request budget.

## Comparison batches

After deterministic scope/provenance/name/number/polarity checks, gather up to eight
same-scope pairs in one Jev request: 16 KiB per individual pair, 32 KiB serialized
combined state. Question instructions bind their specific pair. Local vetoes and
cached answers consume no network calls. Valid uncertainty is terminal; only failed
answers enter an explicitly configured fallback. Other providers retain single-pair
context planning. Separate user prompts are not pooled or delayed for batching.

Reconsolidation and linking gather questions before inference. Link prefetch reads
at most eight future pairs without advancing the durable cursor. Each mutation
rechecks snapshots and commits cursor/audit atomically. No quadratic pair list is
materialized. Pair/run budgets and configurable timeouts are owned by
[Memory and Dreaming](memory-dreaming.md#working-copies-and-comparison), not a fixed
shared one-minute drain deadline.

Cache identity includes pair versions, routing and rubric/parser versions. Change
identity when those contracts change. A cached accepted answer still requires
transaction-time evidence/applicability checks before destructive consolidation.
Prompt relevance asks its separate literary/technical questions through the same
protocol adapter; comparison thresholds do not calibrate message capture.

## Verification limits

Controlled tests cover named mapping, partial failure, caps, timeouts, credential
redaction, no retry/redirect, fallback and terminal uncertainty. They establish the
adapter boundary, not model accuracy.

`real_jev_synthetic_pair_calibration` is explicitly ignored and requires
`HIERO_TEST_COMPARISON_CREDENTIAL_ROOT` with a private `relevance.conf`. It sends
synthetic pairs in a disposable fixture, not manuscript data. Follow its current
input requirements and the [runtime-check guidance](rust-cutover-rehearsal.md).
Historical single/batch pilots differed in abstention; neither is a general
calibration guarantee or qualification of the new local protocol by inheritance.
