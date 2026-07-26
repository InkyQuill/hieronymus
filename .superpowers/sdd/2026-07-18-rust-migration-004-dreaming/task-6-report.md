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

## Fix round 2

### RED

Three integration regressions were added before changing production code:

```text
cargo test -p hiero-core --test test_dreaming \
  cancelling_many_waiters_does_not_occupy_blocking_workers -- --exact --nocapture

FAILED: the sentinel blocking task timed out while two cancelled lock waiters
remained parked in lock_exclusive.

cargo test -p hiero-core --test test_dreaming \
  failed_audit_cleanup_poison_is_visible_and_recovers_before_unlock \
  -- --exact --nocapture

FAILED: injected fail_run failure never exposed audit-recovery state and the
forgotten guard could not recover.

cargo test -p hiero-core --test test_dreaming \
  late_persistence_failure_retries_algorithmic_work_exactly_once \
  -- --exact --nocapture

FAILED: retry used a new maintenance cycle and applied cycle_decay again after
the first cycle had already committed reconsolidation, activation claims, link
reinforcement, and decay.
```

The late-failure fixture was deliberately strengthened to use a real working
copy. That exposed the additional requirement that the durable resume record
must reload the exact original input IDs: reconsolidation may archive the only
working-copy input before provider output commits.

### GREEN

- The service records durable `algorithm_batch_started`,
  `algorithm_batch_completed`, and `algorithm_batch_committed` audit stages.
  The batch identifier is derived from the exact ordered memory IDs, the start
  stage retains the original maintenance cycle and input IDs, and the committed
  marker shares the provider-output/source-archive transaction.
- An unfinished stage is resumed before ordinary pending selection. The service
  reloads archived working copies from the original input set, reuses the
  original maintenance cycle, and skips algorithms after the completed marker.
  Same-cycle activation claims are reselected so cancellation between an
  algorithm commit and its marker converges through the existing
  reconsolidation, link, and decay idempotency.
- Wait-capable lock acquisition now performs short non-waiting OS lock attempts
  in the blocking pool with cancellation-friendly asynchronous retry sleeps.
  Cancelling many waiters leaves blocking capacity available while the holder
  still owns the lock.
- Audit cleanup failure changes the owned lock state to `audit-recovery`, keeps
  the supervisor and guard alive, and retries durable `fail_run` closure. The
  guard is released only after closure succeeds; no descriptor is forgotten.
- The persistence-cancellation regression now waits for a real decay commit
  before cancellation and verifies identical scores and event count after
  retry.

### Verification

Fresh verification against the final fix-round-2 source:

```text
cargo test -p hiero-core --test test_dreaming --test test_dream_lock \
  --no-fail-fast
52 passed; 0 failed; 3 ignored helper entry points

cargo clippy --workspace --all-targets --all-features -- -D warnings
passed

cargo test --workspace --all-features --no-fail-fast
passed; no failed suite

cargo doc --workspace --all-features --no-deps
passed

cargo fmt --all -- --check
passed

git diff --check
passed
```

The first full-workspace run had one unrelated `test_memory` contention timeout
under suite load. Its exact rerun passed, and the complete workspace rerun then
passed.

### Self-review

- Provider passes still precede algorithms, and provider persistence still
  follows algorithms.
- A retry cannot silently select a smaller input set after reconsolidation
  archives a working copy.
- Algorithm mutations use one durable maintenance-cycle identity across
  failure and cancellation retries.
- Poison state is externally observable through the existing typed lock
  contention diagnostic, and owned-state cleanup remains token-safe.
- No schema migration or unrelated deferred-minor work was introduced.

### Concerns

None.

## Final review fix wave

### RED

The seven final-review findings were reproduced with focused regression
coverage before their production fixes:

- production workflow integration could not resolve blank provider/model
  defaults or deliver the exact phase prompt to a production-shaped transport;
- the registry could not execute the configured `deterministic` provider;
- the consolidator type was absent, and a late persistence retry left no merge
  proposal or applied passive reinforcement event;
- a deliberately unshared `run_all` budget exceeded the configured aggregate
  long-term-record limit;
- blocked-provider scheduler shutdown timed out while cleanup was still in
  flight;
- an injected audit cleanup/publication failure returned only the provider
  failure;
- a source-linked non-working-copy row was accepted by reconsolidation and
  returned `ReinforcedInPlace`.

### GREEN

- Production `DreamService` now resolves every enabled workflow against the
  provider catalog, applies catalog defaults, builds the phase-specific prompt
  including `general_prompt`, and passes that exact prompt through each provider
  transport.
- The registered deterministic provider emits rule crystals only for explicit
  rule-pattern memories and performs no network I/O.
- Additive migration 0011 adds reviewable, idempotent directed concept merge
  proposals. `Consolidator` applies the confirmed scope, Unicode casefold,
  target-order, and stable source-order contract without changing terminology
  proposals. Normal durable algorithm batches now run consolidation and
  bounded passive reinforcement with exact-once retry behavior.
- Related-concept, per-concept related-crystal, changed-crystal,
  total-affected-crystal, and long-term-record limits now bound provider and
  algorithmic work. `run_all` shares one aggregate long-term budget.
- Scheduler shutdown cancels the in-flight cycle and waits for the supervised
  lock/audit cleanup acknowledgement before returning.
- Audit recovery retains a state-publication error alongside the original
  cleanup failure.
- Reconsolidation's locked reread now requires `kind = 'working_copy'` before
  any mutation.
- Proposal 004, user-facing dreaming documentation, migration history tests,
  schema fixtures, and public row decoding coverage now describe migration 0011
  and the dedicated merge-proposal contract.

### Verification

```text
cargo test -p hiero-core \
  --test test_providers \
  --test test_dream_config \
  --test test_dream_lock \
  --test test_dream_phases \
  --test test_reconsolidation \
  --test test_link_reinforcement \
  --test test_dreaming \
  --locked --no-fail-fast
151 passed; 0 failed; 3 ignored helper entry points

cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
passed

cargo test --workspace --all-targets --all-features --locked --no-fail-fast
passed; no failed target

cargo doc --workspace --no-deps --all-features --locked
passed

cargo fmt --all -- --check
passed

git diff --check
passed
```

The first full-workspace verification exposed stale ten-migration expectations
in the FTS and legacy-upgrade suites after additive migration 0011. Those
manifests were updated, all three exact failed targets passed, and the complete
workspace rerun passed.

### Self-review

- Provider resolution remains fail-closed, enabled phase order is stable, and
  all test providers are injected without credentials or external I/O.
- Consolidation never auto-merges, does not reuse the terminology proposal DTO,
  and its pending directed-pair uniqueness makes durable retries converge.
- Every newly composed algorithm uses the original resumable maintenance cycle;
  provider persistence and source archival remain one transaction.
- Aggregate budgets are owned by the supervised run, while cycle-local affected
  limits are recreated only at cycle boundaries.
- Shutdown does not release the data-root lock before audit cleanup is durable,
  and publication failures are no longer discarded.
- The reconsolidation kind check occurs inside the locked reread transaction
  before archive or score mutation.
- Proposal 005 and the recorded deferred minors remain untouched.

### Concerns

None.

## Exceptional final fix wave

### RED

- A consolidator that received stale `ConceptRecord` snapshots still produced a
  merge proposal after the authoritative rows had changed.
- An injected failure on the second proposal insert left one committed proposal,
  proving that the batch did not share one transaction. Cancellation coverage
  exposed the same missing batch rollback boundary.
- The durable paged scan API and its schema were absent, so the split-page
  starvation regression did not compile.
- Limit-one reconsolidation and link regressions did not compile because the
  affected-crystal admission API did not exist.
- A duplicate group larger than two scan pages selected a lower-id temporary
  target instead of the authoritative high-id established target.
- After a passive-reinforcement failure committed one crystal, retry recreated
  an empty cycle budget and applied a third unique crystal beyond a limit of two.
- Immediate cursor rewind on every low-id concept update prevented the scan from
  ever indexing a higher-id concept.

### GREEN

- `AffectedCrystalIds` admits mutations by unique long-term crystal id.
  In-place reconsolidation and reinforcement count one id, supersession counts
  old and new ids, links and combinations count both participants, decay counts
  the decayed id, and provider-created crystals count their returned ids.
  Reusing an id across phases or cycles in the same run does not charge it twice.
- Every actual algorithmic or provider mutation records its affected ids in
  `dream_affected_crystals` in the same SQLite transaction. Resumed algorithm
  batches reconstruct both cycle and run admission state from that ledger, so a
  partial per-item phase cannot reset the hard bound.
- Limit one allows in-place reconsolidation but blocks supersession, linking, and
  combination before mutation. The cross-phase regression proves that one
  crystal can be reconsolidated and provider-reinforced in one run.
- `Consolidator` owns one `BEGIN IMMEDIATE` transaction for authoritative input
  rereads, group/target recomputation, the complete bounded proposal batch, and
  scan cursor advancement. SQL failure and task cancellation roll back the
  complete batch, while pending directed-pair retries remain idempotent.
- The consolidator refreshes normalized keys through a durable page cursor and
  defers proposals until a complete clean sweep. Writes during a sweep mark a
  subsequent restart without rewinding the active cursor, so higher ids remain
  reachable; the dirty sweep suppresses proposals before restarting.
- Complete clean sweeps read every member of each duplicate group, so groups
  split across pages and groups larger than two pages choose one authoritative
  target. Existing pending pairs no longer starve later candidates. Terminal or
  merged concepts are removed from the normalized-key index and remain
  ineligible.

### Migration and query plan

- Additive migration `0012_concept_consolidation_scan.sql` adds
  `concept_consolidation_keys`, `concept_consolidation_scan`,
  `dream_affected_crystals`, the normalized lookup index, and scan-reset/key
  invalidation triggers. Migrations 0001-0011 remain unchanged.
- The production duplicate-group `EXPLAIN QUERY PLAN` regression observes
  `concept_consolidation_keys_lookup_idx`.
- Migration history, legacy upgrade, FTS object, exact schema, and explicit
  index suites pass: 93 passed, 0 failed.

### Verification

```text
cargo test -p hiero-core \
  --test test_dreaming \
  --test test_link_reinforcement \
  --test test_reconsolidation \
  --locked --no-fail-fast
67 passed; 0 failed; 1 ignored helper entry point

cargo test -p hiero-core \
  --test test_providers \
  --test test_dream_config \
  --test test_dream_lock \
  --test test_dream_phases \
  --test test_reconsolidation \
  --test test_link_reinforcement \
  --test test_dreaming \
  --locked --no-fail-fast
162 passed; 0 failed; 3 ignored helper entry points

cargo fmt --all -- --check
passed

cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
passed

cargo test --workspace --all-targets --all-features --locked --no-fail-fast
passed; no failed target

cargo doc --workspace --no-deps --all-features --locked
passed

git diff --check
passed
```

### Self-review

- The budget precheck occurs before each mutation; durable ledger insertion
  occurs before the same transaction commits.
- New-id admission reserves capacity before insertion and records the returned
  id, avoiding guessed row ids.
- Consolidation output remains bounded even though target selection evaluates
  the complete authoritative group.
- The durable restart marker lets an active sweep reach high ids while ensuring
  that a sweep dirtied by relevant concept changes cannot emit proposals.
- The Rust practices companion audit identified the oversized-group target,
  retry-reset budget, and cursor-rewind starvation defects; all now have
  observed RED and exact GREEN regressions.
- User-facing dreaming documentation and proposal 004 describe unique-id
  accounting, retry durability, atomic consolidation, and paged discovery.

### Concerns

None.
