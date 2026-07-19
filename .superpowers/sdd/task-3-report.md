# Plan 4 Task 3 Report: Semantic Index Job Queue

## Result

Implemented durable SQLite semantic-index state, authoritative RAG enqueueing, bounded semantic work, and daemon lifecycle integration.

## RED evidence

The required command was run before production implementation:

```text
uv run pytest tests/test_semantic_jobs.py tests/test_rag_store.py tests/test_service_daemon.py -q
```

It failed during collection with two errors because `hieronymus.semantic_jobs` did not exist. After the first minimal implementation, the same slice failed 30 tests because migration `0004` had no fresh-schema baseline verifier. Both failures were caused by missing Task 3 behavior rather than unrelated test defects.

Additional self-review regressions were observed RED before their fixes:

- a crash-left `running` job was not reclaimed;
- a crash between Lance generation creation and the SQLite checkpoint could not resume;
- a pending cancellation left by a crashed worker was not completed;
- a cancelled job could still win the SQLite generation CAS;
- an import racing activation left the obsolete generation failed instead of cancelled.

## Schema and state design

- Migration `0004_semantic_index_state.sql` and `global.sql` create the singleton active/desired generation state, durable jobs, deterministic job items, and per-generation indexed checksums.
- `SQLiteGenerationPointer` implements the accepted `GenerationPointer` protocol with `BEGIN IMMEDIATE` compare-and-set semantics.
- Generation activation also verifies the matching job is running, uncancelled, and fully processed in the same SQLite transaction. This closes the import-versus-activation race.
- Job rows retain identity, previous generation, manifest checksum, status, expected/processed counts, redacted error, retryability, cancellation, index checkpoint, and timestamps across daemon restarts.

## Authoritative-first RAG behavior

- RAG source/chunk/tag/FTS changes and semantic job enqueueing commit in one SQLite transaction.
- Embedding and LanceDB work happens only after that transaction commits and never holds an SQLite write transaction.
- Replacement and explicit source deletion capture obsolete chunk IDs and enqueue delete operations.
- Imports wake registered workers only after commit; lexical search remains available when embedding or index writes fail.

## Worker and lifecycle behavior

- Pending work is selected in bounded deterministic ID order, capped by both the configured semantic batch and provider batch size.
- The worker rechecks every authoritative checksum immediately before the corresponding Lance write.
- Cancellation is checked between batches and during activation; incomplete generations are discarded.
- Interrupted running jobs, processed batches, generation-creation checkpoints, and activation-before-final-status crashes resume safely.
- Model identity changes supersede unfinished work and recreate the provider for the new durable identity.
- Errors are persisted as retryable and redacted against configured provider secrets.
- One daemon worker starts from the ASGI lifespan, wakes by event with a 30-second fallback poll, and stops with the shared coordinator's remaining deadline. Worker failures are logged without taking down the daemon or lexical RAG.

## Verification

- Focused required tests: `42 passed in 5.39s`.
- Focused required Ruff command: `All checks passed!`.
- Full suite: `1555 passed, 3 skipped, 3 warnings in 161.68s`; warnings are the existing `pty.forkpty()` deprecation warnings in release-script tests.
- Global Ruff: `All checks passed!`.
- Global format check: `195 files already formatted`.
- `git diff --check`: clean.

## Self-review

- Confirmed SQLite is authoritative and no embedding/Lance calls occur inside a SQLite write transaction.
- Confirmed manifest verification and mutation work are bounded; no dummy full-corpus vector allocation remains.
- Confirmed pointer CAS serializes against concurrent imports and validates job cancellation/count state.
- Confirmed every worker failure path preserves lexical data and records a redacted retryable error.
- Confirmed restart, cancellation, stale checksum, deletion, identity change, index failure, event wake, activation, and coordinator shutdown have deterministic fake-backed tests with no network or model download.

## Concerns

None.
