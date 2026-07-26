# Task 6 Report: Dream Service and Background Scheduler

## Status

Complete. Task 6 composes the existing Phase 004 lock, audit, provider-pass,
reconsolidation, reinforcement, and decay collaborators into a bounded dream
service and a serial background scheduler. No proposal 005 work was started.

## Implementation

- Added proposal 004 `CycleOptions`, `MaintenancePayload`, and
  `MaintenanceResult`, plus the borrowed `DreamService<'a>` service surface.
- Added `run_cycle`, `run_all`, and scheduler-only due-cycle execution. Explicit
  cycles acquire the cross-process data-root lock; a due cycle runs under the
  scheduler-owned guard without reacquiring it.
- Added terminal skipped audit records for non-waiting lock denial and complete
  run/phase failure cleanup for provider, persistence, maintenance, and
  cancellation paths.
- Added cancellation guards that retain the OS lock until asynchronous audit
  cleanup finishes, preventing another process from observing a released lock
  while the cancelled run is still open.
- Reused `execute_provider_passes` and the existing by-value `DreamPhase::run`
  contract. Configured passes remain ordered and resolve only through the
  injected provider resolver.
- Persisted provider outputs through the existing dream stores, recorded
  coverage audit entries, archived selected short-term memories, and composed
  reconsolidation, reinforcement, link combination, and decay.
- Bounded and deterministically ordered activations and maintenance inputs by
  `max_changed_crystals_per_cycle` before handing them to Task 5
  reinforcement collaborators.
- Added `decay_candidates` and deterministic, deduplicated
  `apply_maintenance` entry points.
- Added `run_background_loop` using one `tokio::select!` loop, skipped missed
  ticks, a broadcast shutdown signal, and no spawned or overlapping cycles.
- Added file-backed end-to-end tests with injected fake providers only. The
  suite performs no provider network I/O and uses no API keys.

## Files

- `crates/hiero-core/src/dreaming/mod.rs`
- `crates/hiero-core/src/dreaming/service.rs`
- `crates/hiero-core/src/dreaming/scheduler.rs`
- `crates/hiero-core/tests/test_dreaming.rs`

## TDD Evidence

### RED

The end-to-end test surface was written first. The corrected initial focused
run failed only because the Task 6 public API did not exist:

```text
cargo test -p hiero-core --test test_dreaming --locked

error[E0432]: unresolved imports
`hiero_core::dreaming::CycleOptions`,
`hiero_core::dreaming::DreamService`,
`hiero_core::dreaming::run_background_loop`
error: could not compile `hiero-core`
```

### GREEN

```text
cargo test -p hiero-core --test test_dreaming --locked

test result: ok. 9 passed; 0 failed; 0 ignored
```

The focused suite covers successful persistence and exact audit counts,
provider failure, in-flight cancellation, lock denial, empty input, bounded
`run_all`, bounded deterministic activations, bounded deterministic
maintenance, and graceful serial scheduler shutdown.

All Phase 004 suites:

```text
cargo test -p hiero-core \
  --test test_dream_config \
  --test test_dream_lock \
  --test test_dream_phases \
  --test test_reconsolidation \
  --test test_link_reinforcement \
  --test test_dreaming \
  --locked --no-fail-fast

98 passed; 0 failed; 2 ignored
```

The two ignored cases are the existing subprocess helper entry points in
`test_dream_lock`; their parent process tests execute them.

## Verification

Fresh verification against the final source:

```text
cargo fmt --all -- --check
passed

cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
passed

cargo test --workspace --all-targets --all-features --locked --no-fail-fast
passed; no failed suite

cargo doc --workspace --no-deps --all-features --locked
passed

git diff --check
passed
```

The existing Phase 004 decay integration test also confirms through
`EXPLAIN QUERY PLAN` that decay uses `idx_crystals_maintenance`.

## Self-Review

- The scheduler owns the lock across the complete due cycle and never
  reacquires it through public `run_due`.
- No service or test bypasses the provider resolver with network access or
  credentials.
- All terminal service outcomes close the run audit, and cancellation cleanup
  retains the lock until that close is durable.
- Provider phases retain their Task 4 order and by-value execution contract.
- Changed-crystal inputs are sorted, deduplicated, and truncated before Task 5
  reinforcement.
- The scheduler has a single in-flight cycle, no unbounded task creation, and a
  direct shutdown branch.
- Existing Task 1-5 implementations were composed without refactoring their
  public contracts.

## Concerns

None.

## Fix Round 1

### Findings addressed

- A dream run now selects exactly one `TranslationContext`; sessions from a
  different series, source language, target language, or normalized metadata
  context remain pending for a later run.
- Waiting and non-waiting lock acquisition now runs through
  `spawn_blocking`. Current-thread runtime coverage proves a waiting OS lock
  cannot stall unrelated async work, and cancellation of a waiting request
  cannot leave a detached lock owner.
- Public `run_due` acquires its own lock. The scheduler alone uses the
  crate-private guard-taking variant, so scheduler execution does not
  double-acquire.
- Detached drop-time audit tasks were removed. An owned blocking supervisor
  retains the guard, uses its own current-thread runtime, observes caller
  cancellation, closes the run and any running phase, and only then releases
  the lock. Runtime shutdown is covered. If durable cleanup itself fails, the
  supervisor deliberately leaks the guard and fails closed instead of
  releasing a lock with an open audit row.
- Provider output persistence and source-memory archival now share one
  immediate transaction. Crystal insertion reuses the existing validated
  transaction helper. Injected failures at concepts, facets, proposals,
  crystals, provenance, relations, reinforcement, and archive boundaries all
  roll back; retry produces exactly one durable output set. In-flight
  cancellation of a large persistence batch also rolls back and converges on
  retry.
- Activation claiming now selects the exact deterministic bounded activation
  IDs and updates only those IDs in the same immediate transaction. Overflow
  keeps `cycle_id IS NULL` and is selected by the next cycle.
- Algorithmic decay is skipped when the remaining change budget is zero, so
  Task 5's standalone zero-means-default convention cannot expand a service
  cycle.
- Provider reinforcement applies the validated candidate strength and
  confidence deltas directly, persists those exact deltas as an applied event,
  and updates `last_reinforced_cycle` for positive strength.
- Supersession uses exact directed `(old_id, new_id)` deduplication; symmetric
  canonicalization remains limited to combination pairs.
- `run_all` uses `max_short_term_memories_per_cycle` for each constituent cycle
  and stops when its aggregate reaches
  `max_short_term_memories_per_run`.
- Maintenance uses one unique-crystal budget across canonical submitted
  reinforcement, decay, combination, and supersession inputs. A floor/no-op
  input still consumes its unique-ID budget.
- A true two-subprocess service test holds the data-root lock inside a blocking
  fake provider in process one and proves process two receives a terminal
  skipped run rather than overlapping it.
- The by-value `DreamPhase::run` contract remains unchanged. Task 5 algorithms
  are consumed through their public interfaces and were not modified.

### TDD evidence

The new review tests were added before the correction set. The first combined
RED was:

```text
cargo test -p hiero-core --test test_dreaming --locked --no-fail-fast

directed_supersede_preserves_old_to_new_when_old_id_is_greater ... FAILED
each_run_uses_one_exact_series_and_language_context ... FAILED
activation_overflow_remains_unclaimed_until_the_next_cycle ... FAILED
no_op_inputs_still_consume_the_aggregate_maintenance_budget ... FAILED
provider_reinforcement_applies_the_exact_validated_deltas ... FAILED
public_run_due_acquires_the_cycle_lock_itself ... FAILED
run_all_obeys_per_cycle_and_aggregate_per_run_limits ... FAILED
output_persistence_and_source_archive_are_one_retry_safe_transaction ... FAILED

test result: FAILED. 11 passed; 8 failed; 0 ignored
```

Representative exact failures included:

```text
each_run_uses_one_exact_series_and_language_context:
left: 2
right: 1

activation_overflow_remains_unclaimed_until_the_next_cycle:
left: 0
right: 1

public_run_due_acquires_the_cycle_lock_itself:
called `Result::unwrap_err()` on an `Ok` value: Some(DreamRunRecord ...)

directed_supersede_preserves_old_to_new_when_old_id_is_greater:
left: ["superseded", "active"]
right: ["active", "superseded"]
```

The final focused GREEN, including structured cancellation, runtime shutdown,
atomic persistence cancellation, exact delta, bounded overflow, and subprocess
coverage, is:

```text
cargo test -p hiero-core --test test_dreaming --locked --no-fail-fast

test result: ok. 24 passed; 0 failed; 1 ignored
```

The ignored test is the subprocess helper entry point; the ordinary
`two_process_services_cannot_overlap_one_data_root_cycle` test executes it in
both holder and contender processes.

### Verification

Fresh verification against the final fix-round source:

```text
cargo test -p hiero-core \
  --test test_dream_config \
  --test test_dream_lock \
  --test test_dream_phases \
  --test test_reconsolidation \
  --test test_link_reinforcement \
  --test test_dreaming \
  --locked --no-fail-fast

113 passed; 0 failed; 3 ignored helper entry points

cargo test --workspace --all-targets --all-features --locked --no-fail-fast
passed; no failed suite

cargo fmt --all -- --check
passed

cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
passed

cargo doc --workspace --no-deps --all-features --locked
passed

git diff --check
passed
```

### Self-review

- Every wait-capable filesystem lock path is outside a Tokio async worker.
- The supervisor, not a `Drop` implementation, owns cancellation cleanup and
  the OS guard.
- Output writes and source archive cannot be observed as a partial commit.
- Context, memory, activation, and change limits use stable deterministic
  ordering.
- Public `run_due` is safe without an undocumented caller precondition.
- No provider fixture performs network I/O or contains credentials.
- Unrelated deferred minors were not implemented; cancellation assertions were
  strengthened because they directly verify the blocking lifecycle fix.

### Concerns

None.
