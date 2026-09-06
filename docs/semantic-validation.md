# Semantic validation: real hybrid relevance (Task S3)

Date: 2026-09-06. Branch `agent/rust-port-semantic`. This document records
the REAL model run qualifying hybrid (memory + RAG semantic) retrieval with
the pinned ONNX assets — no synthetic harness vectors — through normal
authenticated MCP operations and the real `hiero` CLI.

## Command

```text
HIERO_TEST_ONNX_RUNTIME=<repo>/qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so \
HIERO_TEST_MODEL_DIR=<repo>/qualification/.artifacts/models/all-MiniLM-L6-v2 \
cargo test -p hiero --test semantic_real --locked -- --ignored --nocapture
```

Result: `test result: ok. 1 passed; 0 failed` (17.8 s wall clock, debug
build). Without the two environment variables the test FAILS (`panicked`:
`HIERO_TEST_ONNX_RUNTIME must point at the real asset…`), never skips.

## Artifact identities (verified before and during the run)

| Artifact | Identity |
| --- | --- |
| ONNX model | `sentence-transformers/all-MiniLM-L6-v2` @ `9a53d751e60e6dd34f2443711d44d5b09389f89a`, `onnx/model.onnx`, 90,405,214 bytes, SHA-256 `6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452` (matches `qualification/records/semantic-native.json` `model-checksum-load`) |
| Tokenizer | `tokenizer.json`, same revision, 466,247 bytes, SHA-256 `be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037` (the S1 pin; identity `wordpiece-minilm-l6-v2@sha256:be50…`) |
| ONNX runtime | `onnxruntime-linux-x64-1.28.0.tgz` (SHA-256 `a3e1b79d7bb1bf09696ce675f49e4064e6c81f6202b8225624fff0e93f8d6407`, per `qualification/prerequisites.json`), extracted `lib/libonnxruntime.so.1.28.0` = 24,268,848 bytes (SHA-256 `1461ef7cc3d9e49982591721683cc3e3a55580aeca9a5254e7aac47b75ee4bab`; byte size matches the record's `runtime_library_bytes`). Acquired with `uv run python -m tools.qualification.acquire onnx-runtime` / `semantic-model` (pinned URL + SHA-256 verification, no unverified download). |

## Corpus

`crates/hiero/tests/fixtures/hybrid-relevance.json`: 2 series
(`lighthouse-chronicles`, foreign `harbour-records`), 9 documents
(including the mandated example — RAG `The physician treated the wounded
sailor aboard the vessel.` vs distractor `The carpenter repaired the cabin
door.` — a near-duplicate physician source, near-topic distractors, and two
foreign-series physician/doctor documents), 1 learned short-term memory,
1 approved term (`penicillin` → `пенициллин`), 7 queries.

## Observed outcomes (actual top-3 RAG documents per query)

| Query | Observed rag top-3 | Assertions |
| --- | --- | --- |
| `Which doctor cared for the injured seaman on the ship?` | physician, physician-duplicate, nurse | expected sources in top 3, `rag semantic match` provenance, no foreign-series hit |
| `Who made maps of lands no vessel had reached?` | cartographer, storm, physician-duplicate | semantic-only paraphrase hit with semantic provenance |
| `the lighthouse keeper trimmed the lamp at dusk` | lighthouse-keeper, carpenter, nurse | lexical hit |
| `Who fixed the door of the cabin?` | carpenter, physician, physician-duplicate | distractor is retrievable for its own topic |
| `A doctor cared for an injured seaman on a ship.` | physician, physician-duplicate, nurse | the foreign near-verbatim document never leaks |
| `the lighthouse lamp was electrified` | (rag top-3: lighthouse-keeper, storm, nurse) | learned memory returned as `active session short-term memory match` |
| `The physician stockpiled penicillin before the storm.` | nurse, physician, physician-duplicate | `deterministic_contract` carries `penicillin` → `пенициллин`; no ranked row contains the canonical translation (terminology independent of ranking) |

Also asserted in the same run:

- Import → durable rebuild → activation: active generation identity equals
  the pinned `OnnxEmbeddingProvider::static_identity()`
  (WordPiece tokenizer id, not byte-fold), `written == expected`.
- Byte-fold rejection: a legacy manifest persisted under
  `byte-fold-v1` degrades the very next recall
  (`semantic_lane_unavailable` warning); a subsequent normal import's
  rebuild supersedes it under the pinned identity; the legacy row keeps its
  byte-fold identity and status `superseded` (never relabeled, never
  served); the healed lane serves `rag semantic match` hits again.
- Corrupt runtime (non-ELF `libonnxruntime.so`, real `hiero daemon`
  subprocess): `semantic enable` reports `disarmed`, daemon `/status`
  reaches `failed`, and `require_semantic_ready` refuses it.
- Corrupt model (same size, flipped bytes): fails at the SHA-256 check at
  provider load — daemon `failed`, gate refuses.

## Observed limits

- The pinned model is English MiniLM. This corpus is English; the mandated
  example's semantics resolve cleanly, but the model is NOT multilingual —
  extending the corpus to the project's actual source/target-language
  examples (and any model replacement) is a separately qualified identity
  change and is not covered by this run.
- Short-term memory recall is FTS (implicit AND over query tokens), not
  semantic: the memory query is lexical by design; the semantic lane
  contributes the RAG rows.
- `ort` keeps the loaded dylib in a process-global `OnceLock`, and a FAILED
  dynamic load poisons the process for later ort use — hence the
  corrupt-runtime daemon runs as a real `hiero daemon` subprocess (see the
  module docs of `crates/hiero/tests/semantic_real.rs`).

## Qualification record

`qualification/records/semantic-native.json` already pins the artifact
identities used here (`model_sha256`, `runtime_library_bytes`, model bytes);
its schema and `tools.qualification.validate` close the evidence list and
replay-command grammar, so no additive evidence entry is possible without a
separate schema change. The run re-verified those pinned identities; the
outcomes above are this document's record.
