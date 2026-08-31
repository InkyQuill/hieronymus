# Keep Semantic Retrieval Derived And Gate Platform Support Empirically

## Status

Proposed.

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
embedding. Filtering by series occurs inside vector search or through an
over-fetch strategy whose recall loss is measured and accepted explicitly.

Durable jobs record kind, status, generation, cursor, total and completed
counts, attempt count, last error, cancellation request, lease owner/expiry,
created/started/completed timestamps, and model identity. Jobs resume safely
after daemon crashes and never hold a SQLite write transaction during model
inference or LanceDB writes.

Release targets are supported only after native build, install, model-load,
index, search, fallback, and uninstall tests pass on that target. Initially this
means Linux x86_64 and macOS targets proven by the spike. Windows remains
experimental until daemon lifecycle and native semantic dependencies pass the
same gates.

## Consequences

The first Rust release may ship fewer supported platforms than the aspirational
proposal. It will have a reliable FTS-only mode and a reproducible path to
enable semantic retrieval rather than silently depending on unverified native
libraries.
