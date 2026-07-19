# Phase 003 Task 4 Report: Centralize Feedback and Scoring

## Result

DONE

- Base SHA: `c80cfd83dc9778729a1c985e27e37cb57ca38b22`
- RED: `cargo test -p hiero-core --test test_scoring --locked` failed on the missing `FeedbackError`, `FeedbackEvent`, `FeedbackStore`, `ScoreDelta`, and `apply_score_delta` API.
- GREEN: the focused scoring suite passes all 15 tests.
- Commit SHA: `ae71b9c` (`feat: centralize memory feedback scoring`)

## API map

| Public API | Implementation |
|---|---|
| `FeedbackEvent` | Raw feedback contract with crystal, event label, source role, optional evidence, and optional session. |
| `ScoreDelta` | Strength/confidence delta pair shared by all scoring consumers. |
| `apply_score_delta` | Sole pure score path: finite clamping, confidence archival, archived-state preservation, non-finite delta handling, and rule-intent decay dampening. |
| `FeedbackStore::record` | SQLx tracked `BEGIN IMMEDIATE`; validates context, writes `memory_events`, and applies immediate scores atomically. |
| `FeedbackStore::record_recall_outcome` | Deduplicates inputs, validates session/activation ownership, updates activation outcomes, and applies each crystal delta once. |

## Delta matrix

| Immediate event | Strength | Confidence |
|---|---:|---:|
| `confirmed_by_user` | +0.15 | +0.20 |
| `contradicted_by_user` | -0.20 | -0.25 |
| `deleted_by_user` | -0.50 | -0.35 |
| `recalled_useful` | +0.06 | +0.04 |
| `recalled_miss` | -0.05 | -0.03 |

| Passive event | Strength | Confidence |
|---|---:|---:|
| `cited` | +0.03 | +0.02 |
| `used_in_translation` | +0.05 | +0.02 |
| `passed_review` | +0.07 | +0.05 |
| `caused_correction` | -0.10 | -0.12 |
| `superseded` | -0.12 | -0.05 |
| `recalled_again` | +0.02 | 0.00 |

## State and idempotence map

- Scores clamp to `[0, 1]`; non-finite deltas are ignored before persistence.
- Confidence reaching zero archives every lifecycle type. Existing archived status never reactivates implicitly.
- `deleted_by_user` additionally archives when post-delta strength is below `0.05`.
- Candidate and other forward-compatible persisted statuses remain unchanged unless an archive rule applies.
- Non-empty `rule_intent` dampens each negative delta by `1.0 - 0.5 * credibility.clamp(0.0, 1.0)`, with the canonical observation fallback `0.35`.
- Activation outcomes use `(session_id, crystal_id)` as their logical ledger key across all matching activation rows. One report writes one audit event and delta per crystal, repeated identical reports are no-ops, and opposite reports return `OutcomeConflict` atomically.
- Multiple pending activations for the same session/crystal receive the same outcome together; duplicated request IDs never double-apply.
- `memory_events` is the audit schema; source role and evidence remain raw, absent evidence deterministically persists as an empty string, and no metadata/secret payload is accepted by the API.
- Score-only crystal updates do not touch FTS-owned text or IDs; the FTS shadow integrity check remains clean.

## Verification

- `cargo test -p hiero-core --test test_scoring --locked` — 15 passed.
- Targeted `database`, `fts`, `schema`, `test_concepts`, `test_crystals`, `test_memory`, `test_rule_crystals`, and `test_termbase` regressions — passed.
- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` — passed.
- `cargo test --workspace --all-features --locked` — passed.
- `cargo doc --workspace --no-deps --all-features --locked` — passed.
- `uv run --frozen pytest` — 1193 collected, exit 0.
- `uv run --frozen ruff check .` — passed.
- `uv run --frozen ruff format --check .` — 167 files already formatted.
- `uv.lock` was restored after the local editable-package refresh side effect.

## Review remediation

- Follow-up SHA: `5428856` (`fix: align feedback scoring contracts`).
- RED: the expanded focused suite failed four contracts: credibility ordering, late-activation idempotence, global-scope session ownership, and the exact fallback boundary.
- GREEN: `cargo test -p hiero-core --test test_scoring --locked` passes 19 tests.
- The exact negative-delta rule-intent factor is now documented and implemented as `1.0 - 0.5 * credibility.clamp(0.0, 1.0)`, with the observation fallback `0.35`. Rumor, observation, expert, and user-rule fixtures prove that higher credibility loses less; positive deltas and non-rule-intent crystals remain unchanged.
- Activation ledger states are explicit: all pending applies once; all matching is a no-op; matching plus pending fills only pending; any conflicting persisted outcome fails atomically. A concurrent late activation/report test proves `BEGIN IMMEDIATE` serialization without another audit event or score delta.
- Coherent global crystals with empty scope key/series slug can report outcomes from any session series. Coherent series crystals require `scope_key == series:{series_slug}` and exact session ownership. Malformed/other scopes and cross-series series activations fail typed without mutation.
- Recall-adjacent database, FTS, schema, crystal, workspace-memory, and termbase suites pass.
- Full Rust fmt, Clippy, workspace test, and docs pass. Frozen Python pytest exits 0; Ruff lint and format pass. `uv.lock` was restored again after the editable-package refresh.
