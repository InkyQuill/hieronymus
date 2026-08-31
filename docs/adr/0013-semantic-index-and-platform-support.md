# Keep Semantic Retrieval Derived And Gate Platform Support Empirically

## Status

Accepted on 2026-08-31.

## Context

Local embeddings and LanceDB can improve RAG recall, but ONNX Runtime, Arrow,
and LanceDB add large native dependencies and platform-specific build risk. The
proposal pins illustrative crate versions and assumes cross-platform behavior
without a verified compatibility matrix.

## Decision

SQLite remains authoritative for RAG sources, chunks, metadata, and semantic
index job state. Embeddings and LanceDB tables are disposable derived artifacts.
FTS5-only retrieval is a supported degraded mode and must remain available when
the model or vector index is absent, incompatible, corrupt, or rebuilding.

Before selecting crate versions, run a dependency spike against current stable
Rust and the intended release targets. Record exact versions and features in
`Cargo.lock`; proposal version numbers are not normative.

Semantic index identity includes provider, model, model revision, dimensions,
normalization policy, chunk checksum, schema version, and generation id. A
generation becomes active only after all expected chunks are indexed and its
manifest verifies. Search never mixes generations or dimensions.

Vector records carry `chunk_id`, `series_slug`, checksum, generation id, and
embedding. Series isolation is correctness-critical: filtering occurs before
ANN ranking. If the selected backend cannot pre-filter reliably, Hieronymus uses
one physical vector table/index per series. Over-fetch followed by post-filtering
is rejected because it can silently lose eligible results.

Durable jobs record kind, status, generation, cursor, total and completed
counts, attempt count, last error, cancellation request, lease owner/expiry,
created/started/completed timestamps, and model identity. Jobs resume safely
after daemon crashes and never hold a SQLite write transaction during model
inference or LanceDB writes.

The first Rust cutover supports `x86_64-unknown-linux-gnu`. FTS5-only operation
is the required baseline; semantic retrieval is enabled in the release only if
the qualification record shows: clean native build, install/uninstall, checksum
verified model load, 10,000-chunk filtered index build, zero cross-series hits
across the contract corpus, crash/cancel recovery, FTS fallback, and a complete
50-query run without panic or corruption. Failure of any item ships the same
Linux release in FTS5-only mode.

macOS and Windows are not supported by the initial cutover. Promoting another
target requires a later ADR with the same measured qualification record plus its
daemon-service lifecycle tests. The dependency spike selects versions and
records measurements; it does not decide series-isolation semantics or weaken
the promotion criteria above.

## Consequences

The first Rust release may ship fewer supported platforms than the aspirational
proposal. It will have a reliable FTS-only mode and a reproducible path to
enable semantic retrieval rather than silently depending on unverified native
libraries.
