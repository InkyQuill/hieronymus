# Semantic validation: real hybrid relevance

> Historical record: Python source and commands below belong to the
> [archived Python line](archive/python-v0.7.0.md). Current release acquisition uses
> `bun scripts/stage-release-assets.ts`; these records are not active release gates.

The first record below is historical S3 English qualification. P2 multilingual
qualification and its release limits are recorded in the subsequent section;
the historical result does not establish current multilingual or host support.

Date: 2026-09-06. Branch `agent/rust-port-semantic`. This document records
the REAL model run qualifying hybrid (memory + RAG semantic) retrieval with
the pinned ONNX assets — no synthetic harness vectors — through normal
authenticated MCP operations and the real `hiero` CLI.

## Historical S3 invocation

This is the invocation recorded for the historical S3 source, not a command
for the current head. The committed S3 source and its nine-document corpus
are retained together at `ec7465597ecd43b30d60287e6a06b22f16e683c7`
(`test: qualify real memory and semantic rag retrieval`). That commit contains
`crates/hiero/tests/semantic_real.rs`, the old model/tokenizer pins, and
`crates/hiero/tests/fixtures/hybrid-relevance.json` (SHA-256
`a7898e8e8d6c8a634fc4a1b094a99696a3c88fcb9a533d60a0f86ef6d32e7142`).
Replaying S3 requires that historical checkout and the matching qualified
assets below; changing only the model directory at current head does not
restore the old implementation. This snapshot covers S3's nine-document run,
not P2's later 35-document comparison.

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

## P2 multilingual qualification — 2026-09-07

The required languages are Japanese source and Russian target, with an English
compact-memory baseline. The fixed corpus has **35 documents, 6 series and 11
queries**, plus one lexical session note and one deterministic approved term.
Each language pool includes topic-near distractors; foreign-series near-verbatim
queries cannot leak into results. Expected source IDs were manually specified.

Corpus SHA-256: `0f901aa7db1e60493d6e2261aeeacfd99ef48a38f645c9e608a0ac5dc4d77ccf`.
The corpus was frozen before candidate replacement and used unchanged for both
models. Both normal MCP tools are exercised for every case, and provenance is
required on each expected source rather than an unrelated semantic row. Query
and context DTOs reject unknown fields. Viewpoint/story-position fields are
explicitly empty for retrieval acceptance; filling them currently fails instead
of pretending the pending P1 runtime used them.

### Measured decision and assets

The old model failed the Russian cartographer paraphrase in both tools. Expected
`cartographer-ru`; observed `nurse-ru, ship-physician-ru, captain-ru`. Japanese
physician ranked only third after carpenter and gardener. This is the measured
reason for qualifying a replacement; Japanese/Russian scope was not dropped and
no query was changed into an exact lexical match.

The candidate is `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2`,
immutable revision `e8f8c211226b894fcb81acc59f3b34ba3efd5f42`. Its upstream
[immutable model card](https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/blob/e8f8c211226b894fcb81acc59f3b34ba3efd5f42/README.md)
describes multilingual sentence embeddings. The actual native public-MCP run,
not that description, determines acceptance here.

| Asset | Bytes | SHA-256 |
| --- | ---: | --- |
| `onnx/model.onnx` | 470,301,610 | `10f7a088420252b26caf819236ca2c9d2987afd0fc06fec7553b542a5655a05a` |
| `tokenizer.json` | 9,081,518 | `2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8` |
| Immutable `README.md`, includes `license: apache-2.0` | 3,888 | `1e98ea05b0de579fcaad3d625b62ea55647142ed674d5f5ebf1440e4bbbb6f23` |
| Standard `LICENSE` from [Apache](https://www.apache.org/licenses/LICENSE-2.0.txt) | 11,358 | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |

The repository contains no standalone license file: the pinned model card's
Apache-2.0 declaration and separately fetched standard license text are retained
as distinct evidence. ONNX Runtime remains 1.28.0 with the 24,268,848-byte library
and SHA-256 recorded above. No dependency pin or runtime binary changed.

The model plus tokenizer footprint grows from 90,871,461 bytes to 479,383,128
bytes (5.28×); including the same runtime library, from 115,140,309 to 503,651,976
bytes. These are uncompressed embedding artifacts, not whole-app package sizes.
The candidate retains 384 dimensions and mean-pooling/L2 normalization. Its
configuration declares vocabulary size 250,037, maximum position embeddings
512, and sentence-transformer input limit 128. The Unigram asset uses a
precompiled normalizer, whitespace/Metaspace splitting and `<s>`/`</s>` tokens.
Production truncation is 128 tokens including specials; the tokenizer identity
includes its hash and these policies. Out-of-vocabulary numeric IDs are rejected
rather than modulo-folded into unrelated vocabulary entries.

Current acquisition metadata is updated in `qualification/prerequisites.json`
and `tools/qualification/acquire.py`, with a 512 MiB bounded artifact ceiling.
Historical qualification records, tokenizer fixture and standalone historical
`run_semantic.py` harness retain the old identity. That historical harness is not
a current-model gate. The new tokenizer fixture is stored separately, with the unmodified Apache-2.0
license beside it as `multilingual-minilm-tokenizer.LICENSE`. Its source is the
Sentence Transformers model repository and immutable revision named above.

### Commands and unsuccessful attempts

All Rust commands used `CARGO_BUILD_JOBS=4`, Rust 1.96.0, existing lockfile, and
explicit asset paths below; no simultaneous builds replaced the process suite's
`target/debug/hiero` binary. The current worktree was
`/home/inky/Development/hieronymus/.worktrees/product-release-readiness`.

**Historical measured old-model invocation, not a current-head replay recipe:**

The exact source snapshot/test-and-pin patch used for the 35-document
old-model measurements was not retained. The logs and measured outcomes below
remain evidence of that run, but do not provide an exact replay source. The
frozen corpus is recoverable at
`807c0c7f872500b242b78be8032ae1f6b548200e` as
`crates/hiero/tests/fixtures/hybrid-relevance.json`, with the SHA-256 recorded
above; that commit already includes the replacement model/tokenizer and is not
the old-model executable. The earlier S3 commit has only nine documents and
lacks the later measured test instrumentation. Neither that base checkout nor
an old model directory alone reconstructs the measured comparison. No inferred
patch is presented as the original source.

```text
CARGO_BUILD_JOBS=4 \
HIERO_TEST_ONNX_RUNTIME="$PWD/qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so" \
HIERO_TEST_MODEL_DIR="$PWD/qualification/.artifacts/models/all-MiniLM-L6-v2" \
cargo test -p hiero --test semantic_real --locked -- --ignored --nocapture
```

**Current-model invocation and supporting checks** (with the matching qualified
assets acquired at the stated paths):

```sh
CARGO_BUILD_JOBS=4 \
HIERO_TEST_ONNX_RUNTIME="$PWD/qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so" \
HIERO_TEST_MODEL_DIR="$PWD/qualification/.artifacts/models/paraphrase-multilingual-MiniLM-L12-v2" \
cargo test -p hiero --test semantic_real --locked -- --ignored --nocapture

CARGO_BUILD_JOBS=4 cargo test -p hieronymus --test semantic_tokenizer
uv run pytest tests/qualification/test_acquire.py -q
uv run python -m tools.qualification.acquire semantic-model
```

The initial `/usr/bin/time -v` attempt never ran: this host lacks that binary.
Resource measurement therefore reads Linux `/proc/self/status` (`VmHWM`, `VmRSS`)
in the live test after the corpus; per-tool wall latency uses monotonic time.
An initial original-model run timed out before the corpus during daemon enable
(27.28s). A retry passed the preliminary 21-document corpus (64.19s), but weak
rankings motivated adding 14 near-topic distractors. The frozen 35-document old
model runs both failed the same Russian case (72.86s and instrumented 74.93s).

The first candidate run (92.09s) stopped before the corpus: the CLI reached its
30-second polling budget and honestly returned `acquiring`, `disarmed`,
`runtime_verified:false`. The test formerly assumed immediate arming. It now
accepts that transitional report only while separately waiting for true daemon
`ready`; `acquiring` never passes readiness. This slower startup matters to
installed updater/readiness budgets and must be exercised by F1/F2.

Candidate files were acquired from immutable Hugging Face `resolve/<revision>`
URLs, checked against the repository API's LFS SHA-256 values, then verified by
current acquisition tooling and native load. The first acquisition-tool check
rejected mode 0644 on the manually staged model; after `chmod 600`, the private
regular-file verification succeeded. The first Apache license request hit a
connection reset; `curl --fail --location --max-time 25` succeeded on retry.

### Remaining release acceptance

[Host acceptance](agent-host-acceptance.md) records installed generated bundles
but blocked real Claude/Codex workflows, and unverified zCode. P1 immediate
correction and earlier/later viewpoint behavior are explicitly unresolved: its
runtime has not been implemented. Passing retrieval does not satisfy those
separate product requirements or constitute F1/F2 installed acceptance.

### All fixed-corpus outcomes

Top-three IDs below were identical between recall and rag_search for each
model/query. Both endpoints were called and asserted independently.

| Case | Original English model | Multilingual candidate | Verdict |
| --- | --- | --- | --- |
| `physician-paraphrase` | ["physician", "physician-duplicate", "nurse"] | ["physician", "physician-duplicate", "nurse"] | both PASS |
| `cartographer-paraphrase` | ["cartographer", "storm", "physician-duplicate"] | ["cartographer", "storm", "physician-duplicate"] | both PASS |
| `lexical-keeper` | ["lighthouse-keeper", "carpenter", "nurse"] | ["lighthouse-keeper", "carpenter", "nurse"] | both PASS |
| `distractor-control-carpenter` | ["carpenter", "physician", "physician-duplicate"] | ["carpenter", "physician-duplicate", "lighthouse-keeper"] | both PASS |
| `foreign-series-isolation` | ["physician", "physician-duplicate", "nurse"] | ["physician", "physician-duplicate", "nurse"] | both PASS |
| `learned-memory-electrification` | ["lighthouse-keeper", "storm", "nurse"] | ["lighthouse-keeper", "carpenter", "storm"] | both PASS |
| `terminology-independence` | ["nurse", "physician", "physician-duplicate"] | ["physician", "physician-duplicate", "nurse"] | both PASS |
| `ship-physician-ja-paraphrase` | ["carpenter-ja", "gardener-ja", "ship-physician-ja"] | ["ship-physician-ja", "nurse-ja", "carpenter-ja"] | both PASS |
| `cartographer-ja-paraphrase` | ["cartographer-ja", "carpenter-ja", "healer-ja"] | ["cartographer-ja", "scribe-ja", "captain-ja"] | both PASS |
| `ship-physician-ru-paraphrase` | ["nurse-ru", "keeper-ru", "ship-physician-ru"] | ["ship-physician-ru", "nurse-ru", "captain-ru"] | both PASS |
| `cartographer-ru-paraphrase` | ["nurse-ru", "ship-physician-ru", "captain-ru"] | ["cartographer-ru", "cook-ru", "captain-ru"] | original FAIL; candidate PASS |

All expected multilingual sources rank first with the candidate. Every returned
source resolves to the requested series; foreign distractors are excluded.
Expected semantic-only sources carry `rag semantic match`. The learned note
uses lexical `active session short-term memory match`; approved terminology
remains in the independent deterministic contract. No correction/viewpoint
claim is inferred from these results.

### Comparable resource observations

These are debug-suite process observations, not a release-binary memory
benchmark. The process includes failed-load probes, corpus imports, rebuilds
and native ONNX sessions; Linux high-water RSS persists after allocations are
freed. Query latencies include loopback MCP transport and real retrieval.

| Metric | Original | Candidate |
| --- | ---: | ---: |
| hieronymus_recall corpus median / max (ms) | 485.774 / 691.749 | 482.925 / 607.430 |
| hieronymus_rag_search corpus median / max (ms) | 313.769 / 713.721 | 362.400 / 661.975 |
| Process VmHWM at corpus end (KiB) | 571,200 | 2,778,612 |
| Process VmRSS at corpus end (KiB) | 495,360 | 2,042,980 |

The candidate's complete live suite passed in **206.44 seconds**:
`/tmp/hieronymus-p2-multilingual-final.log`. The old comparison log is
`/tmp/hieronymus-p2-minilm-measured.log`. The generation test seeds the old
English model/revision/tokenizer manifest, requires immediate recall degradation
and strict-search refusal, imports a normal source to queue rebuilding, then
asserts the new identity becomes active and the old identity stays inactive and
unchanged. Both document/query providers reject numeric IDs 250037 and
`u32::MAX`. Completing and restarting a session retains durable RAG retrieval;
the unpromoted session-local note is absent in the new session. That boundary
is not an assertion of successful autonomous consolidation.

A separate fresh-daemon stopwatch probe with staged candidate assets measured
`semantic enable` returning `acquiring`/`disarmed` after **30.907 s**, followed
by true `/status` readiness after **43.279 s** from command invocation. This is
process-cold initialization in a debug build, not an OS page-cache-cold benchmark.
Command: `python /tmp/hieronymus-p2-cold-probe.py`; the script starts the actual
binary with a disposable root, waits for the initial failed/no-runtime daemon
state, invokes normal CLI enable, then polls normal CLI status's nested daemon
payload. Log `/tmp/hieronymus-p2-cold-final.log`, structured timing
`/tmp/hieronymus-p2-cold-measurement.json`. The temporary daemon/root were removed.
Two preceding supplemental probes failed (first misread the CLI status wrapper;
second exited nonzero during enable and outlasted its 20-second cleanup wait).
They supply no successful timing. The corrected probe retained stderr (empty on
success), waited for actual initial daemon state and completed successfully.

Final focused verification: native semantic arming/store/recall/tokenizer tests
**67 passed, 1 optional live ignored**; application semantic execution plus DTO
schema guard **33 passed, 1 required-assets live ignored**; current acquisition
**141 passed**; Cargo formatting, full Ruff checks and formatting (217 files),
and diff whitespace checks passed. Ignored default tests do not establish live
support; the separately requested 206.44-second live run above does.

## Ollama transport and selection checks

Offline fixtures in `ollama_embeddings` verify exact Unicode text over the real
HTTP transport, `truncate:false`, identity/count/dimension/finite-value bounds,
error propagation, and coherent SQLite/LanceDB index-to-query behavior. These
fixtures do not establish embedding quality.

A separate explicitly ignored `semantic_cli` test exercises production arming,
CLI/REST persistence, configuration rejection and daemon restart using a loopback
Ollama API and a real pinned tokenizer. It requires an explicit verified fixture
and fails if that input is absent:

```bash
HIERO_OLLAMA_TEST_TOKENIZER=/absolute/path/to/pinned/tokenizer.json \
  cargo test -p hiero --test semantic_cli \
  ollama_cli_configures_arms_and_restarts_without_onnx_assets -- --ignored
```

A real installed Ollama model and public MCP query must be exercised separately
for practical integration acceptance. Native Claude/Codex/Pi host-matrix
qualification is deferred and is not implied by either fixture suite.
