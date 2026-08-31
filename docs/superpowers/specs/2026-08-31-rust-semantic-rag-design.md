# Rust Semantic RAG Design

**Status:** Proposed for review on 2026-08-31.

## Goal

Provide hybrid FTS5 and semantic RAG retrieval while keeping all source data
recoverable from SQLite and making native-index failures non-fatal.

## Authoritative Data

SQLite owns RAG sources, chunks, display text, location, metadata, language
tags, story scopes, semantic tags, checksums, and job/generation manifests.
External-content FTS5 indexes are maintained by column-specific insert, delete,
and update triggers and can be rebuilt from ordinary tables.

Import commits authoritative SQLite rows before semantic indexing. Re-import of
the same `(series_slug, source_ref)` is checksum-aware and transactional. Removed
chunks delete or invalidate derived semantic state through explicit cleanup and
foreign-key cascades.

## Embeddings

The embedding-provider interface reports provider, model, immutable revision,
dimensions, normalization, and maximum batch/input limits. Document and query
embeddings from a provider must have the same identity. Model download happens
only after an explicit semantic operation or configured background job, never
during config load or a non-mutating doctor check.

Downloads use a temporary file, checksum verification, and atomic promotion.
An incomplete or incompatible model leaves FTS retrieval available.

## Generations

Each vector record includes chunk id, series slug, chunk checksum, generation
id, model identity, and embedding. A generation manifest records expected and
written counts plus validation status. Rebuild writes an isolated generation;
search continues using the previous active generation. Activation is an atomic
metadata transition after count, checksum, dimensions, and sample-query checks.

Stale and cancelled generations are garbage-collected only when no active
reader can reference them. Losing the complete LanceDB directory causes a
rebuild, not data loss.

## Durable Jobs

The SQLite job record contains job id/type, status, generation id, model
identity, cursor, total/completed/failed counts, attempts, last error,
cancellation flag, lease owner and expiry, and timestamps. Batch claim uses a
transactional lease. Inference and LanceDB writes occur outside SQLite write
transactions. Completion updates chunk state and progress transactionally.

Cancellation stops after the current bounded batch and never activates its
generation. Expired jobs are recoverable without duplicating active generation
state.

## Search And Fusion

FTS and semantic lanes apply the same series/context eligibility rules before
fusion. Vector filtering uses stored `series_slug`; if the selected LanceDB API
cannot filter before ANN, the dependency spike must quantify and approve an
over-fetch policy before implementation.

Reciprocal rank fusion combines ranks, never incomparable raw BM25 and distance
scores. Constants and tie-breaking are stable and tested. Missing semantic
state returns FTS results with a structured degraded-mode warning. Corrupt or
dimension-mismatched hits are excluded and schedule repair.

## Dependency Spike

Before implementation, compile and run the chosen LanceDB and ONNX stack on
each candidate target. The spike must prove model loading, batch embedding,
filtered insert/search/delete, generation isolation, binary size, first-run
download, and FTS fallback. Exact crate versions and features are selected from
the spike and locked; proposal versions are not reused blindly.

## Acceptance Criteria

- Deleting semantic artifacts and rebuilding yields equivalent eligible chunk
  ids for deterministic fixtures.
- Search never mixes model identities, dimensions, or generations.
- Crash/retry/cancel tests preserve the prior active generation.
- FTS-only mode supports import and search without model download.
- Series filtering cannot return cross-series chunks.
- No SQLite write transaction spans inference or LanceDB I/O.
