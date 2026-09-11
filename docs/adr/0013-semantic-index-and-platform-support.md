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
## Amendment — 2026-09-10: explicit Ollama embeddings

The current authority and semantic specifications require a working semantic
lane. They supersede this ADR's original FTS-only release acceptance language:
lexical fallback remains diagnostic degraded behavior and cannot satisfy semantic
readiness or strict RAG requests.

ONNX remains the default embedding provider. Explicitly configured Ollama
`/api/embed` is an alternative that satisfies the semantic lane when its verified
generation and query lane are ready. Chat/Dream profiles remain separate.
Ollama receives exact original chunk/query text with `truncate:false`; the pinned
MiniLM tokenizer remains the existing local segmentation policy, not a claim
about Ollama's tokenizer. Ollama arming needs that tokenizer asset but no ONNX
model or runtime library.

Generations record provider, model, discovered immutable SHA-256 model digest,
actual dimensions, client normalization, exact-text/truncation/input-bound policy
and pinned segmentation identity. Discovery is checked when arming and before
and after inference; an identity change requires reconfiguration and rebuilding.
Provider/model switches cannot reuse incompatible generations or pending jobs.
No model is pulled implicitly. Native Claude/Codex/Pi acceptance matrix testing
remains a separate deferred qualification; a local Ollama smoke establishes
integration, not model quality or host-matrix acceptance.

### 2026-09-11 split desktop release packaging

Version-2 exact-target metadata binds one platform archive and one canonical common model archive. The common model/tokenizer/notices are packaged once and are absent from every platform archive; their semantic pins are unchanged. A content-addressed acquisition cache is reverified on every reuse. Installation assembles both archives into a complete immutable per-version tree, including its own model files, before activation. Whole-version rollback therefore has no mutable dependency on the cache.

New metadata is named `release-<triple>.json`; there is no split `release.json` alias. Legacy monolithic input remains supported, while 0.8.0 users bootstrap split-release support with the current standalone installer. Final receipts bind platform/model archive hashes, exact-target metadata and assembled asset hashes. Linux release symbol stripping changes only Hieronymus executables; pinned upstream runtime bytes remain unchanged. macOS bundle contents are bound by an outside manifest, avoiding sealed-resource self-reference. Current candidates are unsigned and unnotarized.

Linux x86_64, Windows x86_64 and Apple Silicon retain their separately pinned native runtime contracts. Intel macOS fails packaging until the source-built runtime is promoted. Native Windows/macOS install/update/rollback and in-use-image acceptance remain external qualification; cross-compilation and portable transaction tests cannot replace it. See `docs/desktop-tray.md`, `docs/desktop-platforms.md`, and the final target qualification records.
