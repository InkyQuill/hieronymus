# Merged port correctness continuation — 2026-09-07

This work resumes `origin/agent/merged-port-correctness` at `f11aa51` and targets
`feat/rust-rewrite-proposals`. The current correctness plan and accepted ADRs
are the behavioral authority. Python is a historical baseline, not a requirement
to preserve obsolete behavior. Frozen historical compatibility fixtures remain intact.

## Independent C1–C5 reassessment

Claude delivered substantial repairs: a coherent export read snapshot, startup
cleanup that retains root ownership through worker joins, normal worker reaping,
durable import revision/indexing intent, revision-aware generation activation,
and public semantic RAG integration. Independent review nevertheless found four
correctness defects that required follow-up fixes:

| Area | Defect | Repair and evidence |
| --- | --- | --- |
| C1 export | Lexical normalization hid symlink/parent traversal while publication used the original spelling. | Reject raw symlinks and bind temporary creation and publication to an opened directory descriptor. Tests cover protected database aliases and ancestor replacement after validation. |
| C3 readiness | Index validity meant only that a directory existed. | Open the actual table; validate schema, counts, identity and vector query usability. Tests cover empty/malformed directories, identity/count mismatch and damaged ANN files. |
| C5 result bounds | Fusion could return both lane limits, exceeding requested and global limits. | Bound the fused result by the requested limit and global cap. A regression reproduced 73 rows against a cap of 50. |
| C5 incomplete results | Strict search discarded corrupt-hit and repair-failure warnings. | Return an explicit semantic-unavailable error for incomplete strict results; preserve mixed-recall warnings. Regression reproduced a successful empty result on corrupt semantic evidence. |

The real ONNX suite also contained two stale assertions: its cold-memory query
was unrelated to the inserted memory, and its old-tokenizer assertion accepted
only `superseded`, although rejected generations are marked `failed`. Corrected
assertions still require useful memory during outages and unchanged rejected
identity with the generation inactive.

## Final integration review

The cross-task review found an additional C3/C4/C5 race: a worker could gather
valid readiness evidence, then overwrite a concurrent import’s `Rebuilding`
state with the older `Ready` verdict. Existing recovery tests proved eventual
convergence but did not cover that publication interval. Both publication paths and an added-source strict query reproduced the
defect in deterministic regressions. Readiness publication now checks an
invalidation epoch under the same synchronization as rebuild queueing. Corpus
and queue inputs come from one short SQLite snapshot. Revision checks bracket
semantic execution, refusing stale strict results while retaining old mixed
recall hits with an explicit warning. Final passing evidence is recorded below.

## Validation scope

Real semantic tests use the pinned model/tokenizer and checksum-verified ONNX
Runtime in disposable data roots. The ordinary runtime acquisition command still
fails its known extraction-tree validation (a P3 release-readiness prerequisite).
The extracted library matches the previously qualified SHA-256
`1461ef7cc3d9e49982591721683cc3e3a55580aeca9a5254e7aac47b75ee4bab`;
using that exact library for tests does not certify the acquisition workflow.

## C6: owner-controlled semantic configuration

Configuration loading distinguishes absence from malformed settings. Native
configuration and asset promotion execute under daemon ownership; the CLI waits
for that same daemon's state instead of independently loading a library and
claiming success. Failed native initialization or an incompatible runtime change
reports `restart-required` explicitly. Readiness and configuration revision are
published together, preventing a concurrent status request from attributing old
readiness to a new, unarmed configuration.

The deterministic regression first reproduced `{state: ready,
configuration_revision: 1}` while reload was paused before arming; it passes with
the synchronized snapshot. The focused suite passes 12 CLI and 30 semantic
execution tests. The real ONNX suite passes the unconfigured daemon → CLI enable
→ same instance → public RAG and mixed recall sequence.

## C7: atomic concept maintenance and proposal evidence

Admin concept merging now uses one immediate transaction for preflight, all
selected sources, domain projections and the audit record. Tests reproduce and
then prevent partial commits after audit failure, later-source failure and
repeated IDs, including FTS and relationship state. Proposal materialization
retains rationale and approved/forbidden variant evidence. Approved variants are
noncanonical rendering facets; forbidden variants are explicitly tagged note
facets, preserving the distinction without introducing a new authority policy.

## C8: durable retry and bounded Dream work

Automatic retry eligibility persists across restart with bounded backoff and
jitter; relevant configuration repair and successful completion reset it. Manual
retries remain coalesced and retain failure history. Drain outcomes distinguish
completed, pending and interrupted work, counting deterministic effects as well
as crystallization inputs.

Schema v4 adds stable crystal snapshots and distinguishes lazy from legacy pair
batches. This extends the plan because v3 activation-only membership cannot
preserve cursor meaning after source deletion. Original migrations remain
unchanged. Legacy materialized work drains first; each new pair's effect, audit
and cursor update commit together. A 10,000-member regression with budget one
creates one pair row. Snapshot creation is linear in membership; pair enumeration
is bounded by the configured budget. This adds a migration to maintain, but
avoids deletion-driven pair loss or replay.

## C9: one authenticated discovery client

The compatibility client delegates to lifecycle discovery and MCP forwarding.
Stdio requires a successful authenticated instance/protocol probe before
forwarding. Explicit autostart uses managed-service integration. Tests exercise
stale discovery, reused ports, mismatching instance/protocol, default refusal,
opt-in startup and clean JSON-RPC output; post-connection route errors remain
separately covered. Migration/update diagnostics use matching authenticated
daemon evidence rather than treating any listener as the daemon.

Independent review additionally required updater ownership to remain separate
from health authentication: rejected credentials or missing discovery cannot
prove the root is unowned. A regression reproduced link activation and a false
migration-pending result while a real daemon held the root. Update and rollback
now hold exclusive ownership through offline mutation and release it before
managed startup. Tests also reject a manager's no-op stop.

## Integrated verification

The final scoped review approved `48138b1` without remaining actionable findings.
The full checks run on that production tree, with four Cargo build jobs and
crate-by-crate execution to bound native build memory.

- `cargo test -p hieronymus --all-targets --all-features --locked --no-fail-fast`:
  508 passed, zero failed, one ignored.
- `cargo test -p hiero --all-targets --all-features --locked --no-fail-fast`:
  522 passed, one ignored, one mock HTTP test failed. Its responder read only
  once before closing although the client sends headers and body separately.
  Test-only follow-up `f191e65` reads complete bounded headers and bodies,
  checks fragmented input and EOF, and keeps the authenticated unauthorized
  response assertion with a 32 KiB valid request. The entire corrected target
  passed all six tests; the affected case passed 30 consecutive reruns.
  Exact cause of the original intermittent response is unconfirmed: diagnostic
  attempts with the old fixture did not reproduce it. The incomplete-request
  fixture defect is established; a TCP reset was not directly observed.
  All final application targets are covered by this full run plus the corrected
  target rerun (524 passing tests, one ignored). Production code is unchanged
  after `48138b1`; only this test fixture changed.
- `cargo test -p hiero --all-features --test semantic_real --locked -- --ignored --nocapture`:
  the pinned real ONNX qualification passed, including same-daemon enable,
  public semantic search, mixed recall and recovery.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`:
  passed; the later test-only target also passed Clippy with all features.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

The frontend production bundle was built using Bun 1.4.0 and the frozen lockfile
for the all-feature embedded-console build. Frozen compatibility snapshots and
fixtures are unchanged; existing migrations through v3 are unchanged.

