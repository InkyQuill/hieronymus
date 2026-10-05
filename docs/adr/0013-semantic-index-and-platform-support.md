# 0013 — Derived semantic indexes and native support

Status: accepted 2026-08-31; amended for required semantics/Ollama (2026-09-10),
split desktop artifacts (2026-09-11), exact SQLite vectors (2026-10-02) and
background memory indexing (2026-10-05). Consolidated current policy 2026-10-05.
The original LanceDB and FTS-only release baseline is superseded.

## Context

Semantic retrieval is necessary for useful multilingual memory, but its derived
indexes must not become authority or risk source data. LanceDB/Arrow/DataFusion
added substantial build complexity without a current need for ANN. The accepted
priority is simple exact retrieval with fewer dependencies, retaining inference quality.

## Decision

SQLite owns sources, chunks, memory, manifests, corpus revisions and durable job
state. Derived RAG vectors live in `semantic/sqlite-vectors/generation_<id>.sqlite3`.
Rust performs exhaustive cosine ranking with F64 accumulation over stored F32
vectors, stable chunk-ID ties and bound series filtering before ranking. Reject
zero, nonfinite or mis-sized vectors. No ANN, quantization, sqlite-vec extension,
Lance dependency or additional database runtime is required.

Generation identity includes provider/model/revision, dimensions, normalization,
tokenizer/segmentation identity, corpus checksums, schema and generation. Activate
only a complete verified generation matching the authoritative corpus revision.
Never mix dimensions/generations. Batch writes are atomic; inference occurs outside
write transactions. Jobs retain progress/leases and resume or recover after crashes.

Semantic retrieval is required. Lexical fallback is visible degraded behavior;
it cannot satisfy strict RAG search or semantic readiness. Missing, acquiring,
rebuilding, incompatible or corrupt indexes report their actual state. A ready
lane over an empty corpus may return an empty success.

ONNX with the pinned multilingual model is the default. Explicit Ollama `/api/embed`
is an alternative, separate from chat/Dream profiles. It receives exact original
text with `truncate:false`; no model is pulled implicitly. Arming needs the pinned
segmentation tokenizer, not the ONNX model/runtime. Discovered model digest and
inference identity are checked around requests; changes require reconfiguration
and a new generation, not reuse of incompatible jobs. The local tokenizer does not
claim to be Ollama's tokenizer.

### Upgrade and diagnostics

Old `lancedb` artifacts remain intact during 0.x and cannot satisfy SQLite readiness.
Rebuild derived vectors from authoritative text with the unchanged inference model;
do not import old Lance vectors or migrate authoritative memories for this change.
An older binary likewise uses its own normal recovery on rollback. Cleanup requires
a separately defined upgrade policy before 1.0.

Integrity diagnostics are read-only. Validate SQLite integrity, full identity and
stored vectors; slow checking alone must not label a healthy index corrupt.
Measure full serving-path performance before introducing validation caches or
relaxing checks. Synthetic neighbor agreement does not measure literary relevance.

### Memory indexing

The supervised semantic worker also maintains `memory_vectors` in continuing
batches independent of recall throughout the server lifetime, including after
idle periods. Deleted or archived sources leave the derived index and progress
totals reflect current memory. Authoritative rows define the missing/changed/model-
incompatible backlog. Publish only after rechecking the source. Overview/status
expose progress and failures; the tray indicates ordinary work. Recall reads the
index and distinguishes unfinished indexing from its bounded candidate scan.

### Native distribution

Each target's format-2 metadata binds a platform archive and one canonical common
model archive. Produce model bytes once, reverify cached acquisitions and assemble
complete immutable versions before activation. Rollback never depends on mutable
cache contents. Preserve legacy monolithic input compatibility; do not emit a
misleading split `release.json` alias.

Exact runtime/model pins live in acquisition metadata, not duplicated prose.
Current targets are Linux x86_64, Windows x86_64 and Apple Silicon/Intel macOS.
Intel uses reviewed source-built runtime pins. Runtime acquisition, native inference,
interactive desktop acceptance and host workflows are separate claims. Missing
optional evidence is advisory under AGENTS.md; corrupt artifacts and unsafe
ownership remain runtime refusals. Unsigned/unnotarized status is disclosed.

## Consequences

Exact search favors straightforward validation and retrieval over ANN latency.
Indexes remain disposable while source/memory data stays authoritative. Native
inference/tokenizer assets still contribute to build and distribution size;
this decision does not replace them or claim a full-app benchmark speedup.

[Semantic validation](../semantic-validation.md) owns current checks;
[platform artifacts](../desktop-platforms.md) owns packaging detail. Historical
Lance harnesses qualify their old inputs only, not the current backend.
