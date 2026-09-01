# Rust Qualification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce reproducible, reviewed qualification evidence for MCP transport, semantic native dependencies, frontend embedding, and legacy database import before any production Rust workspace or dependent implementation plan is started.

**Architecture:** Four standalone Rust harness crates live under `qualification/harnesses/` and exercise only synthetic or frozen compatibility inputs in disposable work directories. Python tooling validates a common evidence schema, redacts and renders canonical JSON into human-readable Markdown, fingerprints every input, and computes one aggregate gate without rerunning heavy or network-dependent probes during ordinary checks. The harnesses and their lockfiles are qualification artifacts, not the production workspace or reusable product modules.

**Tech Stack:** Python 3.12 standard library, pytest, Ruff, Rust 1.96.0 on `x86_64-unknown-linux-gnu`, Cargo lockfiles, Bun 1.3.14, rmcp 3.1.4 candidate, LanceDB 0.37.1 candidate, ort 2.0.0-rc.13 candidate, rust-embed 8.12.0 candidate, rusqlite 0.40.2 candidate, SQLite FTS5.

**Spec:** `docs/superpowers/specs/2026-08-31-rust-migration-program-design.md`

## Global Constraints

- The documents under `docs/rust-migration-proposal/` remain useful analysis but are not normative.
- Produce qualification records for MCP transport, semantic native dependencies, frontend embedding, and legacy database import before writing the dependent implementation plan.
- This stage creates no production Rust workspace, production crate, daemon, migration runner, semantic index, or embedded-asset server; every Rust source file is a disposable qualification harness under `qualification/harnesses/`.
- The qualification target is exactly `x86_64-unknown-linux-gnu` with Rust `1.96.0 (ac68faa20 2026-05-25)`; a later target requires its own ADR and measured record.
- macOS and Windows are not supported by the initial cutover.
- FTS5-only operation is the required baseline for the initial Linux cutover.
- The MCP protocol revision is exactly `2026-07-28`; stdio is newline-delimited JSON-RPC, Streamable HTTP is `POST /mcp` with JSON or request-scoped SSE, and `/api/mcp/{operation}` remains an intentionally removed private Python bridge.
- SQLite remains authoritative for RAG sources, chunks, metadata, and semantic job state; embeddings and LanceDB tables are disposable derived artifacts.
- The semantic qualification must record exact crate versions/features, binary size, checksum-verified model load, a 10,000-chunk filtered index, zero cross-series hits, insert/search/delete, generation isolation, crash/cancel recovery, a complete 50-query run, and FTS fallback.
- Any failed semantic criterion selects `fts-only` for the initial Linux release; it never blocks the Rust workspace plan or the `x86_64-unknown-linux-gnu` release by itself.
- MCP, frontend embedding, or legacy database import failure blocks the Rust workspace/dependent plan named by the record. A failure cannot change protocol revision, remove a required transport, introduce filesystem `ServeDir`, require Bun at runtime, discard a supported database, or create a fresh sibling database beside legacy data.
- Release artifacts embed the built Svelte assets through `rust-embed`; Bun `1.3.14` is a build-time prerequisite only and is not an end-user runtime dependency.
- Existing supported databases are never opened for mutation without a successful preflight and recoverable backup.
- A failed upgrade does not leave a database marked as upgraded.
- Approved active terminology cannot be weakened by fuzzy recall or passive scoring.
- Normal mutations use one daemon-owned domain path across CLI, MCP, and web.
- Secret values do not appear in URLs, logs, discovery records, diagnostics, or frontend JSON; the `Secret<T>` type and redacted projections enforce this.
- Semantic retrieval failure degrades to FTS5 rather than failing recall.
- Every bounded background operation is resumable, auditable, or safely repeatable.
- Rust does not write source code into translation workspaces.
- Qualification inputs are checked-in synthetic fixtures or deterministic generated corpora. Harnesses must never enumerate `$HOME`, read the developer's real data root, dump the environment, or access `/home/inky/Yandex.Disk/Translation`; Rust/Cargo/Bun toolchain and package caches are the only permitted home-scoped reads and their absolute paths are normalized out of evidence.
- Canonical records contain no secrets, source-row text, hostnames, usernames, absolute home paths, bearer headers, cookies, launch grants, provider keys, or raw process logs.
- Network access is permitted only in the explicit dependency-fetch, Bun-install, and checksum-verified model-acquisition steps. Ordinary record validation and every replay after acquisition run with Cargo offline and make no network request.
- Every live run writes beneath `qualification/.artifacts/`, hashes frozen inputs before and after use, removes transient work/log/install directories on success or failure, and leaves only ignored caches plus reviewed records.
- Each harness has its own `Cargo.toml` and committed `Cargo.lock`; direct risk dependencies are exact pins and all resolved versions/features are copied from `cargo metadata --locked` and `cargo tree -e features --locked` into the record.
- Each record's input digest covers `qualification/prerequisites.json`, `tools/qualification/model.py`, `tools/qualification/render.py`, its own Python runner, Cargo manifest/lockfile/source/tests, every consumed compatibility manifest/snapshot/fixture, and risk-specific frontend/corpus inputs; changing any of them makes the record stale.
- Pavel Obruchnikov `<me@inkyquill.net>` is the acceptance owner for all four records and the aggregate gate unless a compatibility-manifest entry explicitly delegates another named owner.

## Normative Inputs

- `docs/adr/0013-semantic-index-and-platform-support.md`
- `docs/adr/0015-mcp-protocol-and-transport.md`
- `docs/adr/0010-data-locations-schema-ownership-and-upgrade.md`
- `docs/adr/0014-web-console-replaces-terminal-ui.md`
- `docs/superpowers/specs/2026-08-31-rust-semantic-rag-design.md`
- `docs/superpowers/specs/2026-08-31-rust-daemon-mcp-security-design.md`
- `docs/superpowers/specs/2026-08-31-rust-database-upgrade-design.md`
- `docs/superpowers/specs/2026-08-31-rust-distribution-cutover-design.md`
- `docs/superpowers/specs/2026-08-31-rust-compatibility-contracts-design.md`
- `compatibility/README.md`, `compatibility/manifest.json`, and the frozen fixtures named by each task.

## Outcome Rules

| Risk | Passing decision | Failing decision | Aggregate consequence |
|---|---|---|---|
| MCP transport | `qualified` | `blocked` | Blocks `rust-workspace-and-contract-harness` and `rust-daemon-mcp-security`; ADR 0015 remains unchanged. |
| Semantic native dependencies | `semantic-enabled` | `fts-only` | Never blocks the aggregate gate; selects semantic-enabled or FTS5-only Linux mode. |
| Frontend embedding | `qualified` | `blocked` | Blocks `rust-workspace-and-contract-harness`, `rust-frontend`, and `rust-distribution-cutover`; ADR 0014 remains unchanged. |
| Legacy database import | `qualified` | `blocked` | Blocks `rust-workspace-and-contract-harness`, `rust-database-upgrade`, and `rust-distribution-cutover`; ADR 0010 and the database-upgrade spec remain unchanged. |

Missing, stale, malformed, partially executed, or unreviewed evidence is blocking. A blocking result is a valid qualification record but is not permission to write its dependent implementation plan.

## File Map

- `qualification/README.md`: acquisition, offline replay, record refresh, cleanup, and decision rules.
- `qualification/prerequisites.json`: exact target/toolchain/Bun/model identity and allowed network acquisition sources.
- `qualification/rust-toolchain.toml`: qualification-only Rust 1.96.0 pin.
- `qualification/schemas/record.schema.json`: machine-readable contract for one risk record.
- `qualification/schemas/gate.schema.json`: machine-readable aggregate-gate contract.
- `qualification/fixtures/semantic-corpus.json`: deterministic 10,000-chunk/50-query corpus recipe.
- `qualification/harnesses/mcp-transport/`: standalone rmcp transport probe and lockfile.
- `qualification/harnesses/semantic-native/`: standalone ONNX/LanceDB/FTS probe and lockfile.
- `qualification/harnesses/frontend-embedding/`: standalone rust-embed probe and lockfile.
- `qualification/harnesses/legacy-database-import/`: standalone read-only SQLite import probe and lockfile.
- `qualification/records/*.json`: canonical machine evidence for four risks plus the aggregate gate.
- `docs/qualification/rust/*.md`: generated human-readable records; never hand-edited independently of JSON.
- `tools/qualification/model.py`: typed records, required-criterion sets, consequence constants, input fingerprints, and redaction validation.
- `tools/qualification/render.py`: deterministic Markdown renderer and drift checker.
- `tools/qualification/acquire.py`: explicit atomic semantic-model acquisition with checksum verification.
- `tools/qualification/clean.py`: bounded cleanup for ignored qualification artifacts only.
- `tools/qualification/run_mcp.py`: MCP harness orchestration and record production.
- `tools/qualification/run_semantic.py`: semantic build/recovery/fallback orchestration and record production.
- `tools/qualification/run_frontend.py`: Bun build/rust-embed orchestration and record production.
- `tools/qualification/run_database.py`: read-only fixture import orchestration and record production.
- `tools/qualification/run.py`: one CLI dispatcher for individual or all live qualifications.
- `tools/qualification/check.py`: network-free schema, fingerprint, Markdown, redaction, and aggregate-gate validation.
- `tests/qualification/factories.py`: deterministic valid/failed/stale/review-state record factories and bounded fake executable writer used only by tests.
- `tests/qualification/`: Python tests for record invariants, runners, cleanup safety, and gate truth table.
- `.github/workflows/pr.yml`: ordinary network-free qualification-record drift gate.
- `.gitignore`: ignores model, work, install, log, and Cargo target artifacts under `qualification/.artifacts/`.

---

### Task 1: Qualification Record Boundary And Safe Artifact Lifecycle

**Complexity:** Medium, 3–4 hours.

**Files:**
- Create: `qualification/README.md`
- Create: `qualification/prerequisites.json`
- Create: `qualification/rust-toolchain.toml`
- Create: `qualification/schemas/record.schema.json`
- Create: `qualification/schemas/gate.schema.json`
- Create: `tools/qualification/__init__.py`
- Create: `tools/qualification/model.py`
- Create: `tools/qualification/render.py`
- Create: `tools/qualification/acquire.py`
- Create: `tools/qualification/clean.py`
- Create: `tests/qualification/factories.py`
- Create: `tests/qualification/test_model.py`
- Create: `tests/qualification/test_render.py`
- Create: `tests/qualification/test_clean.py`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: checked-in JSON/SQLite/text inputs rooted at `repo_root`; never implicit environment or user data.
- Produces: `load_record(path: Path) -> QualificationRecord`.
- Produces: `validate_record(record: QualificationRecord, repo_root: Path) -> list[str]`.
- Produces: `fingerprint_inputs(repo_root: Path, paths: tuple[str, ...]) -> str`, SHA-256 over sorted relative paths plus bytes.
- Produces: `render_record(record: QualificationRecord) -> str` with stable headings/table order.
- Produces: `acquire_semantic_model(repo_root: Path) -> Path`, explicit networked atomic download only.
- Produces: `cleanup_targets(repo_root: Path, include_model: bool = False) -> tuple[Path, ...]` and CLI `python -m tools.qualification.clean [--apply] [--include-model]`.
- Produces: `safe_subprocess_env(work_root: Path, *, cargo_offline: bool) -> dict[str, str]`, retaining only toolchain/package-cache paths and `PATH`, setting task-local `HOME`/temporary paths, and removing every `HIERONYMUS_*`, credential, proxy, token, and provider variable.
- Test-only factory: `make_record(repo_root: Path, risk: Risk, *, failed: tuple[str, ...] = (), review_status: ReviewStatus = "pending") -> QualificationRecord`; it emits every required criterion and a current input digest.
- Test-only factory: `accepted_records(repo_root: Path, *, semantic: Literal["semantic-enabled", "fts-only"]) -> dict[Risk, QualificationRecord]`, `accepted_failure(repo_root: Path, risk: Risk) -> QualificationRecord`, `pending_review(record: QualificationRecord) -> QualificationRecord`, and `write_fake_executable(tmp_path: Path, *, failed_criteria: tuple[str, ...] = ()) -> Path`.

- [ ] **Step 1: Write failing record, redaction, rendering, and cleanup tests**

```python
from dataclasses import replace
from pathlib import Path

from tests.qualification.factories import make_record
from tools.qualification.clean import cleanup_targets
from tools.qualification.model import (
    fingerprint_inputs,
    validate_record,
)
from tools.qualification.render import render_record


ROOT = Path(__file__).resolve().parents[2]


def test_semantic_failure_is_complete_and_selects_fts_only() -> None:
    record = make_record(ROOT, "semantic-native", failed=("locked-native-build",))
    assert record.decision == "fts-only"
    assert validate_record(record, ROOT) == []


def test_blocking_failure_cannot_change_the_contract() -> None:
    record = make_record(ROOT, "mcp-transport", failed=("locked-native-build",))
    changed = replace(record, consequence="downgrade the MCP revision")
    assert validate_record(changed, ROOT) == [
        "mcp-transport consequence does not match the accepted blocking consequence"
    ]


def test_records_reject_secret_and_user_path_literals() -> None:
    record = make_record(ROOT, "frontend-embedding")
    leaked = replace(
        record,
        commands=(*record.commands, "compat-secret-do-not-log /home/alice/private"),
    )
    assert validate_record(leaked, ROOT) == [
        "record contains forbidden literal: compat-secret-do-not-log",
        "record contains an absolute home path",
    ]


def test_render_is_deterministic() -> None:
    record = make_record(ROOT, "legacy-database-import")
    assert render_record(record) == render_record(record)
    assert "Legacy Database Import Qualification Record" in render_record(record)


def test_cleanup_never_targets_repository_or_model_by_default() -> None:
    targets = cleanup_targets(ROOT)
    assert ROOT not in targets
    assert ROOT / "qualification/.artifacts/models" not in targets
    assert all(path.is_relative_to(ROOT / "qualification/.artifacts") for path in targets)


def test_input_fingerprint_changes_with_fixture_bytes(tmp_path: Path) -> None:
    fixture = tmp_path / "fixture.txt"
    fixture.write_text("before", encoding="utf-8")
    before = fingerprint_inputs(tmp_path, ("fixture.txt",))
    fixture.write_text("after", encoding="utf-8")
    assert fingerprint_inputs(tmp_path, ("fixture.txt",)) != before
```

- [ ] **Step 2: Run the tests and verify the missing-package failure**

Run: `uv run pytest tests/qualification/test_model.py tests/qualification/test_render.py tests/qualification/test_clean.py -v`

Expected: FAIL during collection with `ModuleNotFoundError: No module named 'tools.qualification'`.

- [ ] **Step 3: Define the exact prerequisite file and qualification-only toolchain pin**

Create `qualification/rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.96.0"
profile = "minimal"
targets = ["x86_64-unknown-linux-gnu"]
components = ["clippy", "rustfmt"]
```

Create `qualification/prerequisites.json`:

```json
{
  "schema_version": 1,
  "target": "x86_64-unknown-linux-gnu",
  "rust": {
    "version": "1.96.0",
    "commit": "ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96"
  },
  "bun": {
    "version": "1.3.14",
    "authority": "frontend/package.json"
  },
  "semantic_model": {
    "provider": "onnx-runtime",
    "repository": "sentence-transformers/all-MiniLM-L6-v2",
    "revision": "9a53d751e60e6dd34f2443711d44d5b09389f89a",
    "file": "onnx/model.onnx",
    "url": "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/9a53d751e60e6dd34f2443711d44d5b09389f89a/onnx/model.onnx",
    "sha256": "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452",
    "dimensions": 384,
    "normalization": "l2"
  },
  "network_policy": {
    "allowed_only_for": [
      "cargo fetch --locked",
      "bun install --frozen-lockfile",
      "python -m tools.qualification.acquire semantic-model"
    ],
    "ordinary_replay": "offline"
  }
}
```

The model file is stored at `qualification/.artifacts/models/all-MiniLM-L6-v2/model.onnx`, is never committed, and is promoted from `.part` only after the exact SHA-256 matches.

- [ ] **Step 4: Implement typed records and immutable risk rules**

Use these exact public types and constants in `tools/qualification/model.py`:

```python
from dataclasses import dataclass
from pathlib import Path
from typing import Literal


Risk = Literal[
    "mcp-transport",
    "semantic-native",
    "frontend-embedding",
    "legacy-database-import",
]
EvidenceStatus = Literal["pass", "fail", "not-run"]
Decision = Literal["qualified", "blocked", "semantic-enabled", "fts-only"]
ReviewStatus = Literal["pending", "accepted", "rejected"]
JsonScalar = str | int | float | bool | None
MeasurementValue = JsonScalar | tuple[JsonScalar, ...]


REQUIRED_CRITERIA: dict[Risk, tuple[str, ...]] = {
    "mcp-transport": (
        "locked-native-build",
        "protocol-2026-07-28",
        "unsupported-version-rejected",
        "stdio-newline-jsonrpc",
        "streamable-http-json",
        "streamable-http-sse",
        "registry-identity",
        "result-error-identity",
        "required-auth-metadata",
        "private-bridge-absent",
    ),
    "semantic-native": (
        "locked-native-build",
        "binary-size-recorded",
        "install-uninstall",
        "model-checksum-load",
        "ten-thousand-chunk-build",
        "zero-cross-series-hits",
        "insert-search-delete",
        "generation-isolation",
        "crash-recovery",
        "cancel-recovery",
        "fts-fallback",
        "fifty-query-run",
    ),
    "frontend-embedding": (
        "bun-version-and-frozen-build",
        "missing-bundle-rejected",
        "release-assets-embedded",
        "index-and-spa-fallback",
        "hashed-asset-and-mime",
        "missing-asset-404",
        "empty-directory-execution",
        "no-runtime-bun-node-python",
        "no-source-map-secret",
        "binary-size-recorded",
    ),
    "legacy-database-import": (
        "bundled-sqlite-fts5",
        "fixture-classification",
        "supported-current-read",
        "supported-legacy-read",
        "typed-row-accounting",
        "fts-query-equivalence",
        "ledger-preserved",
        "unsupported-fail-closed",
        "source-byte-identity",
        "no-sensitive-row-output",
    ),
}


FAILURE_CONSEQUENCES: dict[Risk, str] = {
    "mcp-transport": (
        "Block rust-workspace-and-contract-harness and rust-daemon-mcp-security; "
        "preserve MCP revision 2026-07-28, stdio, POST /mcp JSON/SSE, and removal "
        "of /api/mcp/{operation}."
    ),
    "semantic-native": (
        "Select FTS5-only for the initial x86_64-unknown-linux-gnu release; do not "
        "block rust-workspace-and-contract-harness or the Linux release."
    ),
    "frontend-embedding": (
        "Block rust-workspace-and-contract-harness, rust-frontend, and "
        "rust-distribution-cutover; preserve embedded Svelte assets and no Bun, "
        "Node, or Python runtime dependency."
    ),
    "legacy-database-import": (
        "Block rust-workspace-and-contract-harness, rust-database-upgrade, and "
        "rust-distribution-cutover; preserve every supported source schema and "
        "never create a fresh sibling database beside legacy data."
    ),
}


@dataclass(frozen=True)
class Evidence:
    criterion: str
    status: EvidenceStatus
    summary: str
    measurements: dict[str, MeasurementValue]
    not_run_reason: str | None = None


@dataclass(frozen=True)
class Environment:
    rustc: str
    cargo: str
    target: str
    os: str
    kernel: str
    architecture: str
    bun: str | None
    native_libraries: tuple[str, ...]


@dataclass(frozen=True)
class LockedDependency:
    name: str
    version: str
    source: str
    checksum: str | None
    features: tuple[str, ...]


@dataclass(frozen=True)
class CleanupEvidence:
    work_dir_removed: bool
    raw_logs_removed: bool
    install_dir_removed: bool
    source_inputs_unchanged: bool
    user_data_opened: bool


@dataclass(frozen=True)
class Review:
    owner: str
    status: ReviewStatus
    objective_evidence_reviewed: bool
    normative_constraints_preserved: bool


@dataclass(frozen=True)
class QualificationRecord:
    schema_version: int
    risk: Risk
    target: str
    status: Literal["pass", "fail"]
    decision: Decision
    acceptance_owner: str
    specs: tuple[str, ...]
    contract_ids: tuple[str, ...]
    input_paths: tuple[str, ...]
    input_digest: str
    commands: tuple[str, ...]
    environment: Environment
    dependencies: tuple[LockedDependency, ...]
    evidence: tuple[Evidence, ...]
    consequence: str
    cleanup: CleanupEvidence
    review: Review
```

Validation requires every criterion exactly once, forbids unknown criteria, requires a reason for `not-run`, derives record status from evidence, enforces the exact decision/consequence mapping, verifies every input path and digest, requires a nonempty ordered command list using repository-relative/artifact-relative arguments, requires the first four cleanup booleans true and `user_data_opened` false, and scans the serialized record for the forbidden data classes in Global Constraints. Live runners create `Review(owner="Pavel Obruchnikov <me@inkyquill.net>", status="pending", objective_evidence_reviewed=False, normative_constraints_preserved=False)`; only Task 7 changes review fields. JSON schemas set `additionalProperties: false` on every fixed object; `measurements` permits only named scalar or scalar-array values, never nested logs or payloads.

- [ ] **Step 5: Implement deterministic rendering, acquisition, and bounded cleanup**

`render_record` prints title, decision, exact replay commands, environment/dependency tables, one row per required criterion, consumed compatibility ids, input digest, cleanup assertions, and the immutable consequence. It sorts mappings and dependency rows; it never includes raw stdout/stderr.

Expose deterministic CLIs `python -m tools.qualification.model validate <record.json>` and `python -m tools.qualification.render --check <record.json> <record.md>` so individual tasks can validate a record before the aggregate checker exists.

`acquire_semantic_model` uses `urllib.request`, writes only `model.onnx.part`, hashes while streaming, deletes a mismatched partial file, calls `os.replace` after success, and returns the verified final path. Redirects are limited to `huggingface.co`, `cdn-lfs.huggingface.co`, and `cas-bridge.xethub.hf.co`; the command reports expected/actual checksum without printing headers.

`safe_subprocess_env` allowlists `PATH`, `CARGO_HOME`, `RUSTUP_HOME`, and `BUN_INSTALL_CACHE_DIR` when present; sets `HOME`, `TMPDIR`, `XDG_CACHE_HOME`, and `XDG_CONFIG_HOME` beneath the risk work root; sets `CARGO_NET_OFFLINE=true` for replay; and removes every other inherited variable. Tests set sentinel `HIERONYMUS_DATA_ROOT`, `HOME`, `HTTP_PROXY`, `OPENAI_API_KEY`, `Authorization`, and `COOKIE` values and assert none reaches a child probe or record.

`cleanup_targets` returns only these resolved paths:

```python
ARTIFACT_TARGETS = (
    "qualification/.artifacts/work",
    "qualification/.artifacts/logs",
    "qualification/.artifacts/install",
    "qualification/.artifacts/cargo-target",
    "qualification/.artifacts/frontend-dist",
)
MODEL_TARGET = "qualification/.artifacts/models"
```

Before deletion, require each target to be a non-symlink descendant of `qualification/.artifacts`, reject the artifacts root itself, and print exact paths in dry-run mode. `--include-model` adds only `MODEL_TARGET`.

- [ ] **Step 6: Add ignored artifact boundaries and initial contributor instructions**

Append exactly these entries to `.gitignore`:

```gitignore
# Disposable Rust qualification artifacts
qualification/.artifacts/
qualification/harnesses/*/target/
```

Document that acquisition is explicit, live results are checked in only after sanitization, ordinary `tools.qualification.check` never acquires dependencies/models, and Markdown records are generated from JSON.

- [ ] **Step 7: Run focused verification**

Run: `uv run pytest tests/qualification/test_model.py tests/qualification/test_render.py tests/qualification/test_clean.py -v`

Run: `uv run ruff check tools/qualification tests/qualification`

Run: `uv run ruff format --check tools/qualification tests/qualification`

Expected: all focused tests pass; Ruff reports no errors or formatting drift.

- [ ] **Step 8: Commit the record boundary**

```bash
git add .gitignore qualification/README.md qualification/prerequisites.json qualification/rust-toolchain.toml qualification/schemas tools/qualification tests/qualification
git commit -m "test: define Rust qualification record contract"
```

### Task 2: MCP 2026-07-28 Transport Qualification

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/harnesses/mcp-transport/Cargo.toml`
- Create: `qualification/harnesses/mcp-transport/Cargo.lock`
- Create: `qualification/harnesses/mcp-transport/src/main.rs`
- Create: `qualification/harnesses/mcp-transport/src/registry.rs`
- Create: `qualification/harnesses/mcp-transport/src/report.rs`
- Create: `qualification/harnesses/mcp-transport/tests/transport.rs`
- Create: `tools/qualification/run_mcp.py`
- Create: `tests/qualification/test_run_mcp.py`
- Create: `qualification/records/mcp-transport.json`
- Create: `docs/qualification/rust/mcp-transport.md`

**Interfaces:**
- Consumes: `compatibility/snapshots/mcp.json`, `compatibility/fixtures/mcp/protocol.json`, every `compatibility/fixtures/mcp/*/{success,error}.input.json`, every corresponding `wire.{success,error}.json`, and manifest ids `cli.script.hieronymus-mcp`, `http.route.post.mcp`, `http.route.post.api.mcp.operation`, and `mcp.tool.*`.
- Produces Rust CLI: `mcp-transport stdio --registry <path> --protocol <path>`.
- Produces Rust CLI: `mcp-transport http --registry <path> --protocol <path> --bind 127.0.0.1:0 --ready-file <path>`.
- Produces Python: `run(repo_root: Path, work_root: Path, *, executable: Path | None = None) -> QualificationRecord`; `None` builds the locked candidate, while tests inject a bounded fake executable.
- Produces canonical record decision `qualified` only when all ten MCP criteria pass; otherwise `blocked` with the exact Task 1 consequence.

- [ ] **Step 1: Write failing runner and transport tests**

```python
def test_mcp_runner_consumes_the_frozen_protocol_and_registry(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path)
    assert record.risk == "mcp-transport"
    assert "compatibility/fixtures/mcp/protocol.json" in record.input_paths
    assert "compatibility/snapshots/mcp.json" in record.input_paths
    assert [item.criterion for item in record.evidence] == list(
        REQUIRED_CRITERIA["mcp-transport"]
    )


def test_mcp_failure_preserves_adr_0015(tmp_path: Path, failing_harness: Path) -> None:
    record = run(ROOT, tmp_path, executable=failing_harness)
    assert record.status == "fail"
    assert record.decision == "blocked"
    assert record.consequence == FAILURE_CONSEQUENCES["mcp-transport"]
```

In Rust integration tests, feed the target `initialize`, unsupported-version, stdio, Streamable HTTP JSON, and SSE cases from `protocol.json`; assert an exact match produces `pass`, an exact mismatch produces bounded `fail` evidence without normalization, and stdio stdout framing counts one JSON object followed by `\n` per response.

- [ ] **Step 2: Run tests and confirm both missing implementations**

Run: `uv run pytest tests/qualification/test_run_mcp.py -v`

Expected: FAIL importing `tools.qualification.run_mcp`.

Run: `cargo +1.96.0 test --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked`

Expected: FAIL because `qualification/harnesses/mcp-transport/Cargo.toml` does not exist.

- [ ] **Step 3: Create the standalone candidate manifest and registry adapter**

Use this direct candidate boundary; the generated lockfile records every transitive version:

```toml
[package]
name = "hieronymus-mcp-transport-qualification"
version = "0.0.0"
edition = "2024"
rust-version = "1.96"
publish = false

[[bin]]
name = "mcp-transport"
path = "src/main.rs"

[dependencies]
anyhow = "1"
rmcp = { version = "=3.1.4", default-features = false, features = [
  "client",
  "server",
  "macros",
  "schemars",
  "transport-child-process",
  "transport-io",
  "transport-streamable-http-client-reqwest",
  "transport-streamable-http-server",
] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
tokio = { version = "1", features = ["io-util", "macros", "net", "process", "rt-multi-thread", "sync", "time"] }

[dev-dependencies]
tempfile = "3"
```

`registry.rs` loads the frozen registry JSON into immutable `ToolDefinition` values and returns canned fixture results only; it has no domain store or SQLite access:

```rust
pub const PROTOCOL_REVISION: &str = "2026-07-28";

pub struct FrozenRegistry {
    pub tools: Vec<ToolDefinition>,
    pub digest: String,
}

pub trait RegistryProbe: Sized {
    fn load(snapshot: &Path) -> anyhow::Result<Self>;
    fn list_tools(&self) -> &[ToolDefinition];
    fn fixture_result(&self, name: &str, arguments: &Value) -> anyhow::Result<Value>;
}
```

If rmcp cannot represent the exact revision or a required transport behavior, the harness records that criterion as `fail`; do not patch fixture expectations, alias another revision, or implement a private operation URL.

- [ ] **Step 4: Implement stdio and loopback HTTP probes with bounded lifecycle**

`main.rs` exposes exactly two subcommands. Stdio reads one JSON-RPC object per input line, writes responses only to stdout, and writes diagnostics only to stderr. HTTP binds only the supplied loopback address, writes `{ "address": "127.0.0.1:<PORT>" }` atomically to the ready file, requires `Authorization: Bearer qualification-synthetic-token` and `MCP-Protocol-Version: 2026-07-28`, serves only `POST /mcp`, and supports both JSON and request-scoped SSE responses.

`report.rs` emits one bounded JSON object containing protocol version, transport, request/response SHA-256 values, content types, registry digest, stdout framing counts, and exit status. It never emits the bearer literal, headers, raw payload text, or absolute paths.

- [ ] **Step 5: Acquire crates explicitly, lock them, and run offline tests**

Networked acquisition prerequisite:

```bash
cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/mcp-transport/Cargo.toml
cargo +1.96.0 fetch --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked
```

Offline replay:

```bash
CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu
```

Expected: the test command completes without network access and proves both exact-match and mismatch-reporting paths. Candidate protocol/transport support is decided only by the live fixture replay and becomes an honest qualified or blocking record.

- [ ] **Step 6: Implement the Python live runner and write both record formats**

`run_mcp.run` uses `qualification/.artifacts/work/mcp-transport`, starts each transport with a 20-second ready/response timeout, replays every target transport case, compares tool lists by canonical JSON digest, compares one success and one error tool call across transports, verifies `/api/mcp/fixture` is absent, and gathers locked dependencies with:

```bash
cargo +1.96.0 metadata --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --format-version 1
cargo +1.96.0 tree --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked -e features
```

The runner kills child processes in `finally`, deletes ready files and logs, verifies all compatibility inputs are byte-identical, then atomically writes `qualification/records/mcp-transport.json` and renders `docs/qualification/rust/mcp-transport.md`.

Run: `uv run python -m tools.qualification.run_mcp --write`

Expected: exit `0` means a complete, sanitized record was written. The record itself says `qualified` or `blocked`; a blocking candidate is retained rather than rewritten to pass.

- [ ] **Step 7: Verify focused record and formatting checks**

Run: `uv run pytest tests/qualification/test_run_mcp.py tests/qualification/test_model.py tests/qualification/test_render.py -v`

Run: `uv run ruff check tools/qualification/run_mcp.py tests/qualification/test_run_mcp.py`

Run: `cargo +1.96.0 fmt --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --check`

Run: `CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: tests, Ruff, rustfmt, and Clippy pass; record validation reports no missing criterion, stale input, secret, or path leak.

- [ ] **Step 8: Commit the MCP qualification**

```bash
git add qualification/harnesses/mcp-transport tools/qualification/run_mcp.py tests/qualification/test_run_mcp.py qualification/records/mcp-transport.json docs/qualification/rust/mcp-transport.md
git commit -m "test: qualify MCP transport candidate"
```

### Task 3: Deterministic Semantic Corpus And Native Candidate Core

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/fixtures/semantic-corpus.json`
- Create: `qualification/harnesses/semantic-native/Cargo.toml`
- Create: `qualification/harnesses/semantic-native/Cargo.lock`
- Create: `qualification/harnesses/semantic-native/src/lib.rs`
- Create: `qualification/harnesses/semantic-native/src/corpus.rs`
- Create: `qualification/harnesses/semantic-native/src/model.rs`
- Create: `qualification/harnesses/semantic-native/src/index.rs`
- Create: `qualification/harnesses/semantic-native/src/fts.rs`
- Create: `qualification/harnesses/semantic-native/tests/corpus.rs`
- Create: `qualification/harnesses/semantic-native/tests/index.rs`
- Modify: `qualification/prerequisites.json`
- Modify: `tools/qualification/acquire.py`
- Modify: `tests/qualification/test_model.py`

**Interfaces:**
- Consumes: deterministic corpus recipe plus the success inputs from `compatibility/fixtures/mcp/hieronymus_rag_search/` and `compatibility/fixtures/mcp/hieronymus_recall/` as fixed semantic-query seeds.
- Produces: `generate_corpus(spec: &CorpusSpec) -> anyhow::Result<(Vec<Chunk>, Vec<Query>)>` with exactly 10,000 chunks across 100 series and 50 queries.
- Produces: trait `EmbeddingProvider::embed(&mut self, token_ids: &[i64], attention: &[i64]) -> anyhow::Result<Vec<f32>>`.
- Produces: `OnnxEmbeddingProvider::load(runtime: &Path, model: &Path, expected_sha256: &str) -> anyhow::Result<Self>`.
- Produces: `async fn GenerationIndex::create(root: &Path, identity: ModelIdentity, generation: &str) -> anyhow::Result<Self>` plus async append/search/delete/verify/activate methods.
- Produces: `fts_search(connection: &rusqlite::Connection, series_slug: &str, query: &str, limit: usize) -> anyhow::Result<Vec<i64>>` with no native semantic dependency when built without the `semantic-native` feature.

- [ ] **Step 1: Write failing deterministic-corpus and index-boundary tests**

```rust
#[test]
fn corpus_is_exactly_ten_thousand_chunks_and_fifty_queries() -> anyhow::Result<()> {
    let spec = CorpusSpec::load(fixture("qualification/fixtures/semantic-corpus.json"))?;
    let (chunks, queries) = generate_corpus(&spec)?;
    assert_eq!(chunks.len(), 10_000);
    assert_eq!(queries.len(), 50);
    assert_eq!(chunks.iter().map(|c| &c.series_slug).collect::<BTreeSet<_>>().len(), 100);
    let (chunks_again, queries_again) = generate_corpus(&spec)?;
    assert_eq!(
        corpus_digest(&chunks, &queries),
        corpus_digest(&chunks_again, &queries_again),
    );
    Ok(())
}

#[tokio::test]
async fn filter_is_applied_before_vector_ranking() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let mut index = GenerationIndex::create(root.path(), fake_identity(), "generation-a").await?;
    index.append(fake_chunks_for_two_series()).await?;
    let hits = index.search("series-a", &fake_embedding(7), 10).await?;
    assert!(hits.iter().all(|hit| hit.series_slug == "series-a"));
    Ok(())
}

#[tokio::test]
async fn incomplete_generation_never_becomes_active() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    activate_complete_generation(root.path(), "generation-a").await?;
    write_incomplete_generation(root.path(), "generation-b").await?;
    assert_eq!(read_active_generation(root.path())?, "generation-a");
    Ok(())
}
```

- [ ] **Step 2: Run the tests and confirm the missing manifest failure**

Run: `cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --no-default-features`

Expected: FAIL because `qualification/harnesses/semantic-native/Cargo.toml` does not exist.

- [ ] **Step 3: Define the exact deterministic corpus recipe**

Create `qualification/fixtures/semantic-corpus.json`:

```json
{
  "schema_version": 1,
  "seed": "hieronymus-semantic-qualification-v1",
  "series_count": 100,
  "chunks_per_series": 100,
  "query_count": 50,
  "embedding_dimensions": 384,
  "generation_ids": ["generation-a", "generation-b"],
  "series_slug_format": "qualification-series-{index:03}",
  "chunk_text_format": "synthetic literary memory {series_index:03}/{chunk_index:03}",
  "query_selection": "queries 0..49 target series 0..49 and chunk index (query_index * 17) % 100"
}
```

Corpus generation uses SHA-256 of `seed || series_index_be || chunk_index_be` for deterministic synthetic token ids, checksum, and fake vector values. Every query carries an explicit eligible `series_slug`; generated decoys reuse token patterns in other series so post-filter over-fetch loses results and the test detects it. The semantic record stores the resulting corpus digest; input fingerprinting includes both this recipe and `corpus.rs`, so a generator change makes the record stale.

- [ ] **Step 4: Add the exact native runtime prerequisite and atomic acquisition**

Extend `qualification/prerequisites.json` with:

```json
"onnx_runtime": {
  "version": "1.28.0",
  "target": "linux-x64",
  "url": "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-linux-x64-1.28.0.tgz",
  "sha256": "a3e1b79d7bb1bf09696ce675f49e4064e6c81f6202b8225624fff0e93f8d6407",
  "library": "lib/libonnxruntime.so"
}
```

Extend `tools.qualification.acquire` with `onnx-runtime`. It downloads to `qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0.tgz.part`, permits redirects only through `github.com`, `release-assets.githubusercontent.com`, and `objects.githubusercontent.com`, verifies SHA-256, safely extracts only archive members beneath `qualification/.artifacts/models/onnxruntime-1.28.0/`, rejects symlinks and path traversal, verifies `lib/libonnxruntime.so` exists, and atomically renames the completed directory. Add tests with an in-memory safe archive, a `../escape` member, a symlink member, a disallowed redirect host, and a checksum mismatch.

- [ ] **Step 5: Create the feature-separated candidate manifest**

```toml
[package]
name = "hieronymus-semantic-native-qualification"
version = "0.0.0"
edition = "2024"
rust-version = "1.96"
publish = false

[features]
default = ["semantic-native"]
semantic-native = [
  "dep:arrow-array",
  "dep:arrow-schema",
  "dep:lancedb",
  "dep:ndarray",
  "dep:ort",
]

[dependencies]
anyhow = "1"
arrow-array = { version = "=58.0.0", optional = true }
arrow-schema = { version = "=58.0.0", optional = true }
futures = "0.3"
lancedb = { version = "=0.37.1", default-features = false, optional = true }
ndarray = { version = "0.16", optional = true }
ort = { version = "=2.0.0-rc.13", default-features = false, features = ["api-28", "load-dynamic", "ndarray", "std", "tracing"], optional = true }
rusqlite = { version = "=0.40.2", default-features = false, features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
tempfile = "3"
tokio = { version = "1", features = ["macros", "process", "rt-multi-thread", "sync", "time"] }

[dev-dependencies]
proptest = "1"
```

`--no-default-features` compiles the FTS path without LanceDB, Arrow, ndarray, or ort references. The native runtime is loaded explicitly from the verified acquisition path; no build script downloads ONNX Runtime.

- [ ] **Step 6: Implement the model, generation, and FTS boundaries**

Use these exact types:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelIdentity {
    pub provider: String,
    pub model: String,
    pub revision: String,
    pub dimensions: usize,
    pub normalization: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Chunk {
    pub chunk_id: i64,
    pub series_slug: String,
    pub checksum: String,
    pub generation_id: String,
    pub token_ids: Vec<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchHit {
    pub chunk_id: i64,
    pub series_slug: String,
    pub generation_id: String,
    pub distance: f32,
}
```

`OnnxEmbeddingProvider` verifies the model checksum before `ort::init_from`, validates 384 output dimensions, mean-pools with the attention mask, and L2-normalizes. It records only identity, dimensions, load duration, and output digest.

`GenerationIndex` stores `chunk_id`, `series_slug`, checksum, generation id, and a fixed-size 384-float vector. `search` applies the exact series predicate before ANN ranking. If the candidate cannot prove pre-filtering through returned execution metadata and adversarial decoys, the live criterion fails; do not substitute over-fetch/post-filter behavior. Activation writes a small manifest only after expected/written counts, identity, dimensions, checksums, and sample queries verify.

`fts.rs` creates a disposable external-content FTS5 table over generated SQLite rows and searches by `series_slug`; it is compiled and tested with `--no-default-features`.

- [ ] **Step 7: Acquire dependencies once and run offline unit tests**

Networked acquisition prerequisites:

```bash
uv run python -m tools.qualification.acquire semantic-model
uv run python -m tools.qualification.acquire onnx-runtime
cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/semantic-native/Cargo.toml
cargo +1.96.0 fetch --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked
```

Offline replay:

```bash
CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --no-default-features --target x86_64-unknown-linux-gnu
CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu
```

Expected: corpus, adversarial filtering, insert/search/delete, generation isolation, and FTS tests pass without a network request. Two generations of the complete corpus have an identical digest.

- [ ] **Step 8: Commit the semantic candidate core**

```bash
git add qualification/fixtures/semantic-corpus.json qualification/harnesses/semantic-native qualification/prerequisites.json tools/qualification/acquire.py tests/qualification/test_model.py
git commit -m "test: add semantic native qualification core"
```

### Task 4: Semantic Recovery, Fallback, And Measured Record

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/harnesses/semantic-native/src/main.rs`
- Create: `qualification/harnesses/semantic-native/src/scenario.rs`
- Create: `qualification/harnesses/semantic-native/tests/recovery.rs`
- Create: `tools/qualification/run_semantic.py`
- Create: `tests/qualification/test_run_semantic.py`
- Create: `qualification/records/semantic-native.json`
- Create: `docs/qualification/rust/semantic-native.md`

**Interfaces:**
- Consumes: Task 3 model/runtime acquisitions, corpus recipe, semantic Cargo lockfile, and the frozen RAG/recall seed fixtures.
- Produces Rust CLI: `semantic-native scenario --work-dir <path> --mode complete|crash|cancel --generation <id> --stop-after <count>`.
- Produces Rust CLI: `semantic-native query --work-dir <path> --series <slug> --query-index <0..49>`.
- Produces Rust CLI without native features: `semantic-native fts-fallback --work-dir <path> --series <slug> --query-index <0..49>`.
- Produces Python: `run(repo_root: Path, work_root: Path, *, executable: Path | None = None) -> QualificationRecord`; `None` builds the locked candidate and tests may inject an executable that emits bounded failure evidence.
- Produces decision `semantic-enabled` only when all twelve criteria pass; every other complete result is `fts-only` and never aggregate-blocking.

- [ ] **Step 1: Write failing recovery and decision tests**

```rust
#[tokio::test]
async fn crash_leaves_previous_generation_active_and_resume_is_exact() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    run_complete(root.path(), "generation-a").await?;
    let crash = run_crash(root.path(), "generation-b", 4_200).await;
    assert!(crash.is_err());
    assert_eq!(read_active_generation(root.path())?, "generation-a");
    resume(root.path(), "generation-b").await?;
    assert_eq!(written_count(root.path(), "generation-b")?, 10_000);
    Ok(())
}

#[tokio::test]
async fn cancellation_never_activates_cancelled_generation() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    run_complete(root.path(), "generation-a").await?;
    run_cancel(root.path(), "generation-b", 3_200).await?;
    assert_eq!(read_active_generation(root.path())?, "generation-a");
    assert_eq!(generation_status(root.path(), "generation-b")?, "cancelled");
    Ok(())
}
```

```python
def test_any_semantic_failure_selects_fts_only_without_blocking(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=failing_semantic_harness())
    assert record.status == "fail"
    assert record.decision == "fts-only"
    assert "do not block" in record.consequence
```

- [ ] **Step 2: Run the focused tests and verify missing scenario/runner failures**

Run: `uv run pytest tests/qualification/test_run_semantic.py -v`

Expected: FAIL importing `tools.qualification.run_semantic`.

Run: `CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --test recovery`

Expected: FAIL because the recovery test target and scenario module do not exist.

- [ ] **Step 3: Implement bounded crash, cancel, resume, and activation scenarios**

`scenario` processes the corpus in batches of 200. After each batch it closes LanceDB handles, atomically writes a sanitized progress file containing generation, cursor, expected/written counts, model identity, and checksum accumulator, then continues. `--mode crash --stop-after 4200` calls `std::process::abort()` only after the 4,200-row checkpoint is durable. `--mode cancel --stop-after 3200` exits normally with generation status `cancelled`. Resume begins at the exact durable cursor, verifies existing record checksums, and never changes the active generation until all 10,000 rows and sample queries verify.

The CLI rejects work directories outside `qualification/.artifacts/work/semantic-native`, symlinks, model/runtime checksum mismatch, mixed dimensions, mixed generation ids, and non-loopback acquisition URLs before writing.

- [ ] **Step 4: Implement the full measured runner**

`run_semantic.run` first removes only `qualification/.artifacts/cargo-target/semantic-native`, recreates it empty, and sets `CARGO_TARGET_DIR` to that exact path so `locked-native-build` is a clean build. It then executes the following exact evidence sequence against fresh disposable paths:

1. Build the semantic binary with `--release --locked --features semantic-native` and record binary/library sizes plus `ldd` basenames.
2. Copy the binary, `libonnxruntime.so`, model, and an empty index root beneath `qualification/.artifacts/install/semantic-native`; run one query; remove the install directory; assert it no longer exists.
3. Verify model checksum before load and record 384 dimensions plus normalized output digest.
4. Build 10,000 chunks for `generation-a`, assert expected/written counts match, and activate it.
5. Run all 50 series-scoped queries; require no panic/error/corrupt row and zero hit whose `series_slug` differs from the query.
6. Insert one synthetic chunk, find it, delete it, and prove it is absent.
7. Create incomplete `generation-b`; prove searches remain on `generation-a`.
8. Abort `generation-b` at 4,200, resume to 10,000, verify, activate once, and prove no duplicate chunk ids.
9. Cancel a new generation at 3,200 and prove `generation-b` remains active.
10. Build the FTS binary with `--release --locked --no-default-features`, temporarily omit model/index paths, and prove all 50 queries complete with structured `fts-only` mode and no model download attempt.

Every subprocess has a 20-minute total timeout and a 2-minute no-progress timeout. A build or prerequisite failure marks itself `fail`; causally dependent criteria become `not-run` with the exact failing criterion in `not_run_reason`. The record remains complete and selects `fts-only`.

- [ ] **Step 5: Run the semantic qualification and write both records**

Run:

```bash
CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_semantic --write
```

Expected: exit `0` after atomically writing a complete `qualification/records/semantic-native.json` and matching Markdown. The JSON decision is exactly `semantic-enabled` when all criteria pass or `fts-only` when any criterion fails. No outcome from this command is `blocked`.

- [ ] **Step 6: Verify cleanup and offline reproducibility**

Run: `uv run python -m tools.qualification.clean`

Expected: dry-run lists only work/log/install/Cargo-target/frontend-dist descendants and does not list the acquired model/runtime directory.

Run: `uv run python -m tools.qualification.clean --apply`

Expected: transient paths are absent; model/runtime acquisitions remain.

Run: `uv run python -m tools.qualification.model validate qualification/records/semantic-native.json`

Run: `uv run python -m tools.qualification.render --check qualification/records/semantic-native.json docs/qualification/rust/semantic-native.md`

Expected: both commands pass without running Cargo, opening a network connection, or requiring transient artifacts; input fingerprints, redaction, evidence completeness, and generated Markdown match.

- [ ] **Step 7: Run focused code-quality checks**

Run: `uv run pytest tests/qualification/test_run_semantic.py tests/qualification/test_model.py tests/qualification/test_clean.py -v`

Run: `uv run ruff check tools/qualification/run_semantic.py tests/qualification/test_run_semantic.py`

Run: `cargo +1.96.0 fmt --manifest-path qualification/harnesses/semantic-native/Cargo.toml --check`

Run: `CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --all-targets --features semantic-native --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: Python and Rust checks pass. Record validation accepts either measured semantic decision and rejects missing or leaked evidence.

- [ ] **Step 8: Commit the semantic qualification record**

```bash
git add qualification/harnesses/semantic-native/src/main.rs qualification/harnesses/semantic-native/src/scenario.rs qualification/harnesses/semantic-native/tests/recovery.rs tools/qualification/run_semantic.py tests/qualification/test_run_semantic.py qualification/records/semantic-native.json docs/qualification/rust/semantic-native.md
git commit -m "test: record semantic native qualification"
```

### Task 5: Svelte Frontend Embedding Qualification

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/harnesses/frontend-embedding/Cargo.toml`
- Create: `qualification/harnesses/frontend-embedding/Cargo.lock`
- Create: `qualification/harnesses/frontend-embedding/build.rs`
- Create: `qualification/harnesses/frontend-embedding/src/main.rs`
- Create: `qualification/harnesses/frontend-embedding/src/assets.rs`
- Create: `qualification/harnesses/frontend-embedding/tests/assets.rs`
- Create: `tools/qualification/run_frontend.py`
- Create: `tests/qualification/test_run_frontend.py`
- Create: `qualification/records/frontend-embedding.json`
- Create: `docs/qualification/rust/frontend-embedding.md`

**Interfaces:**
- Consumes: `frontend/package.json`, `frontend/bun.lock`, `frontend/vite.config.ts`, all `frontend/src/**`, `compatibility/fixtures/http/route-cases.json`, and manifest ids `frontend.route.get.root`, `frontend.route.get.admin`, `frontend.route.get.admin.path`, `frontend.route.get.assets.path`, `frontend.route.get.config`, and `frontend.route.get.config.path`.
- Produces Rust: `AssetResponse resolve_asset(request_path: &str)`, where `AssetResponse` contains `status: u16`, `content_type: String`, `body_sha256: String`, and `fallback: bool`.
- Produces Rust CLI: `frontend-embedding manifest` and `frontend-embedding get --path <request-path>`.
- Produces Python: `run(repo_root: Path, work_root: Path, *, executable: Path | None = None) -> QualificationRecord`; `None` builds the locked candidate and tests may inject an executable that emits bounded failure evidence.
- Produces decision `qualified` only when all ten frontend criteria pass; otherwise `blocked` with the exact Task 1 consequence.

- [ ] **Step 1: Write failing runner and embedded-asset tests**

```rust
#[test]
fn embedded_index_and_spa_routes_resolve_without_filesystem_access() {
    assert_eq!(resolve_asset("/").status, 200);
    assert_eq!(resolve_asset("/").content_type, "text/html; charset=utf-8");
    assert!(resolve_asset("/admin/fixture").fallback);
    assert!(resolve_asset("/config/fixture").fallback);
}

#[test]
fn missing_real_asset_is_not_an_index_fallback() {
    let response = resolve_asset("/assets/qualification-missing.js");
    assert_eq!(response.status, 404);
    assert!(!response.fallback);
}
```

```python
def test_frontend_runner_owns_exact_frozen_route_contracts(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path)
    assert record.contract_ids == (
        "frontend.route.get.admin",
        "frontend.route.get.admin.path",
        "frontend.route.get.assets.path",
        "frontend.route.get.config",
        "frontend.route.get.config.path",
        "frontend.route.get.root",
    )


def test_frontend_failure_blocks_without_selecting_serve_dir(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=failing_frontend_harness())
    assert record.decision == "blocked"
    assert "embedded Svelte assets" in record.consequence
    assert "Bun, Node, or Python runtime" in record.consequence
```

- [ ] **Step 2: Run tests and confirm missing harness/runner failures**

Run: `uv run pytest tests/qualification/test_run_frontend.py -v`

Expected: FAIL importing `tools.qualification.run_frontend`.

Run: `cargo +1.96.0 test --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked --release`

Expected: FAIL because the frontend-embedding manifest does not exist.

- [ ] **Step 3: Create the standalone rust-embed candidate manifest**

```toml
[package]
name = "hieronymus-frontend-embedding-qualification"
version = "0.0.0"
edition = "2024"
rust-version = "1.96"
publish = false
build = "build.rs"

[[bin]]
name = "frontend-embedding"
path = "src/main.rs"

[dependencies]
anyhow = "1"
mime_guess = "2"
percent-encoding = "2"
rust-embed = { version = "=8.12.0", features = ["debug-embed", "deterministic-timestamps", "interpolate-folder-path"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"

[dev-dependencies]
tempfile = "3"
```

`build.rs` requires `HIERONYMUS_QUALIFICATION_ASSET_DIR`, canonicalizes it, requires both `index.html` and at least one `assets/` file, rejects symlinks escaping that root, and emits only `cargo:rerun-if-env-changed` plus `cargo:rerun-if-changed` lines. The runner, not the build script, builds the Svelte frontend before Cargo.

- [ ] **Step 4: Implement the embedded resolver with exact fallback rules**

```rust
#[derive(rust_embed::RustEmbed)]
#[folder = "$HIERONYMUS_QUALIFICATION_ASSET_DIR/"]
struct FrontendAssets;

use sha2::Digest;

#[derive(Debug, Serialize)]
pub struct AssetResponse {
    pub status: u16,
    pub content_type: String,
    pub body_sha256: String,
    pub fallback: bool,
}

fn response(status: u16, key: &str, bytes: &[u8], fallback: bool) -> AssetResponse {
    AssetResponse {
        status,
        content_type: if key.ends_with(".html") {
            "text/html; charset=utf-8".to_owned()
        } else {
            mime_guess::from_path(key).first_or_octet_stream().to_string()
        },
        body_sha256: format!("{:x}", sha2::Sha256::digest(bytes)),
        fallback,
    }
}

fn empty_response(status: u16) -> AssetResponse {
    response(status, "response.json", b"", false)
}

pub fn resolve_asset(request_path: &str) -> AssetResponse {
    let lowercase = request_path.to_ascii_lowercase();
    if lowercase.contains("%2f") || lowercase.contains("%5c") || lowercase.contains("%00") {
        return empty_response(400);
    }
    let decoded = match percent_encoding::percent_decode_str(request_path).decode_utf8() {
        Ok(path) => path,
        Err(_) => return empty_response(400),
    };
    if decoded.contains('\\')
        || decoded.contains('\0')
        || decoded.split('/').any(|part| part == "..")
    {
        return empty_response(400);
    }
    let key = decoded.trim_start_matches('/');
    let key = if key.is_empty() { "index.html" } else { key };
    if let Some(asset) = FrontendAssets::get(key) {
        return response(200, key, asset.data.as_ref(), false);
    }
    if key.starts_with("assets/") {
        return empty_response(404);
    }
    let index = FrontendAssets::get("index.html").expect("build.rs verified index.html");
    response(200, "index.html", index.data.as_ref(), true)
}
```

The exact rules are: normalize one leading slash; reject `..`, backslash, percent-decoded separators, and NUL with 400; serve a present embedded file with its MIME type; return 404 for a missing path beneath `/assets/`; otherwise serve embedded `index.html` as a client-side route fallback. The CLI returns only status, MIME, body digest, byte length, and fallback; it never prints asset bodies.

- [ ] **Step 5: Acquire Bun/Cargo dependencies explicitly and build the actual bundle**

Networked acquisition prerequisites:

```bash
bun install --cwd frontend --frozen-lockfile
cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml
cargo +1.96.0 fetch --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked
```

Offline frontend build:

```bash
bun install --cwd frontend --frozen-lockfile --offline
bun run --cwd frontend build
```

Expected: Bun reports version `1.3.14`; the frozen install and Vite build pass without a network request; output exists only at ignored `frontend/dist` until copied into the disposable qualification asset directory.

- [ ] **Step 6: Implement the live embedding runner and objective criteria**

`run_frontend.run` copies `frontend/dist` to `qualification/.artifacts/frontend-dist/current`, fingerprints it, and builds release mode with:

```bash
HIERONYMUS_QUALIFICATION_ASSET_DIR=qualification/.artifacts/frontend-dist/current CARGO_NET_OFFLINE=true cargo +1.96.0 build --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --target x86_64-unknown-linux-gnu
```

It then:

1. Records Bun version, frozen lock digest, bundle digest, and successful build.
2. Builds once with an explicit nonexistent asset directory and requires a stable missing-bundle failure before returning to the valid directory.
3. Compares the embedded manifest's relative files, byte lengths, and SHA-256 values to the copied Vite output.
4. Replays the six frontend route contracts from `route-cases.json`, including root/admin/config fallbacks and actual/missing asset behavior.
5. Copies only the release binary into an otherwise empty temporary directory and reruns every route probe there.
6. Requires `ldd` basenames and process execution to show no Bun, Node, Python, frontend directory, or filesystem asset dependency.
7. Rejects any `.map` file and scans embedded bytes for `compat-secret-do-not-log`, `Authorization: Bearer`, `provider_key`, `/home/`, and `Yandex.Disk`.
8. Records release binary byte size without imposing an unapproved size threshold.

The runner removes the copied bundle, temporary binary directory, and Cargo target in `finally`; it preserves only the canonical record and ignored Bun package cache/node_modules.

- [ ] **Step 7: Run qualification and focused verification**

Run: `CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_frontend --write`

Expected: writes a complete `frontend-embedding.json` and generated Markdown. It records `qualified` or an honest `blocked` result; it never records filesystem serving or a runtime language dependency as an alternative.

Run: `uv run pytest tests/qualification/test_run_frontend.py tests/qualification/test_model.py tests/qualification/test_render.py -v`

Run: `uv run ruff check tools/qualification/run_frontend.py tests/qualification/test_run_frontend.py`

Run: `cargo +1.96.0 fmt --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --check`

Run: `HIERONYMUS_QUALIFICATION_ASSET_DIR=frontend/dist CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: tests and format/lint checks pass; record validation reports exact route ownership, complete criteria, and no sensitive data.

- [ ] **Step 8: Commit the frontend qualification**

```bash
git add qualification/harnesses/frontend-embedding tools/qualification/run_frontend.py tests/qualification/test_run_frontend.py qualification/records/frontend-embedding.json docs/qualification/rust/frontend-embedding.md
git commit -m "test: qualify frontend asset embedding"
```

### Task 6: Read-Only Legacy Database Import Qualification

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/harnesses/legacy-database-import/Cargo.toml`
- Create: `qualification/harnesses/legacy-database-import/Cargo.lock`
- Create: `qualification/harnesses/legacy-database-import/src/main.rs`
- Create: `qualification/harnesses/legacy-database-import/src/classify.rs`
- Create: `qualification/harnesses/legacy-database-import/src/probe_import.rs`
- Create: `qualification/harnesses/legacy-database-import/src/report.rs`
- Create: `qualification/harnesses/legacy-database-import/tests/fixtures.rs`
- Create: `tools/qualification/run_database.py`
- Create: `tests/qualification/test_run_database.py`
- Create: `qualification/records/legacy-database-import.json`
- Create: `docs/qualification/rust/legacy-database-import.md`

**Interfaces:**
- Consumes: `compatibility/snapshots/state.json`; all six `compatibility/fixtures/database/{minimal-python,legacy-python,empty,partial-python,corrupt,unknown-schema}.sqlite`; manifest ids `database.schema.current`, `database.migrations.current`, and `database.upgrade.preflight`.
- Produces Rust: `classify_read_only(source: &Path, state_contract: &Path) -> anyhow::Result<Classification>`.
- Produces Rust: `probe_import(source: &Path, target: &Path, expected: &DatabaseContract) -> anyhow::Result<ProbeReceipt>`.
- Produces Rust CLI: `legacy-database-import classify --source <fixture> --contract compatibility/snapshots/state.json` and `probe-import --source <fixture> --target <disposable-path> --contract compatibility/snapshots/state.json`.
- Produces Python: `run(repo_root: Path, work_root: Path, *, executable: Path | None = None) -> QualificationRecord`; `None` builds the locked candidate and tests may inject an executable that emits bounded failure evidence.
- Produces decision `qualified` only when all ten database criteria pass; otherwise `blocked` with the exact Task 1 consequence.

- [ ] **Step 1: Write failing fixture matrix and runner tests**

```rust
#[test]
fn frozen_fixture_matrix_matches_expected_classification_without_writes() -> anyhow::Result<()> {
    let cases = [
        ("minimal-python.sqlite", "supported-python", true),
        ("legacy-python.sqlite", "supported-legacy-python", true),
        ("empty.sqlite", "empty", false),
        ("partial-python.sqlite", "partial-python", false),
        ("corrupt.sqlite", "corrupt", false),
        ("unknown-schema.sqlite", "unknown-schema", false),
    ];
    for (name, classification, safe_to_convert) in cases {
        let before = sha256(fixture(name))?;
        let actual = classify_read_only(fixture(name), state_contract())?;
        assert_eq!(actual.name, classification);
        assert_eq!(actual.safe_to_convert, safe_to_convert);
        assert_eq!(sha256(fixture(name))?, before);
    }
    Ok(())
}

#[test]
fn unsupported_sources_do_not_create_probe_target() -> anyhow::Result<()> {
    for name in ["empty.sqlite", "partial-python.sqlite", "corrupt.sqlite", "unknown-schema.sqlite"] {
        let target = temp_target(name)?;
        assert!(probe_import(fixture(name), &target, contract()).is_err());
        assert!(!target.exists());
    }
    Ok(())
}
```

```python
def test_database_failure_blocks_without_data_disposition_change(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=failing_database_harness())
    assert record.decision == "blocked"
    assert "preserve every supported source schema" in record.consequence
    assert "fresh sibling database" in record.consequence
```

- [ ] **Step 2: Run tests and confirm missing implementations**

Run: `uv run pytest tests/qualification/test_run_database.py -v`

Expected: FAIL importing `tools.qualification.run_database`.

Run: `cargo +1.96.0 test --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked`

Expected: FAIL because the legacy-database-import manifest does not exist.

- [ ] **Step 3: Create the minimal bundled-SQLite candidate manifest**

```toml
[package]
name = "hieronymus-legacy-database-import-qualification"
version = "0.0.0"
edition = "2024"
rust-version = "1.96"
publish = false

[[bin]]
name = "legacy-database-import"
path = "src/main.rs"

[dependencies]
anyhow = "1"
rusqlite = { version = "=0.40.2", default-features = false, features = ["bundled", "backup", "serde_json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"

[dev-dependencies]
tempfile = "3"
```

The candidate deliberately uses bundled SQLite so the probe measures FTS5/file compatibility without silently depending on the workstation's SQLite library.

- [ ] **Step 4: Implement bounded classification and neutral probe import**

Open source fixtures with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`, immediately set `PRAGMA query_only=ON`, and never issue a source transaction or write pragma. Classification uses only integrity result, exact tables/columns/features from `state.json`, application migration-ledger presence, and the frozen variant expectations. It is a qualification classifier, not the production `StateClassifier`.

For the two supported fixtures only, `probe_import` creates a new disposable target with this neutral schema:

```sql
create table probe_rows (
    source_table text not null,
    source_id text not null,
    payload_sha256 text not null,
    field_count integer not null,
    primary key (source_table, source_id)
);
create table probe_ledger (
    source_table text not null,
    source_id text not null,
    outcome text not null check (outcome in ('read', 'skipped', 'blocking')),
    reason_code text not null,
    primary key (source_table, source_id)
);
```

Read typed integers, reals, text, blobs, booleans, JSON, and documented timestamps; canonicalize each row in memory; store only its digest and field count. Every source row has exactly one ledger outcome. The probe explicitly covers series, sessions, strict terms/aliases/tags, concepts/facets, memories/crystals/links, RAG sources/chunks/tags/scopes, events/audit, and `memory_graph_migration_ledger`. It never attempts the production target rule schema or one-way cutover protocol.

- [ ] **Step 5: Implement objective FTS, ledger, and byte-identity checks**

For `minimal-python.sqlite`, compare table/column/index/trigger/foreign-key inventories and representative-row digests with `state.json`; run `PRAGMA integrity_check`, `PRAGMA foreign_key_check`, and one exact FTS query for strict terms, memories, concepts, crystals, and RAG chunks. For `legacy-python.sqlite`, require the legacy identity/fingerprint and full typed ledger accounting even when fewer tables are present.

Before and after every classify/import attempt, hash the source fixture and require equality. Hash every file in the fixture directory to prove no journal, WAL, sibling database, backup, or target appeared next to it. Unsupported/corrupt/partial/empty cases must fail before target creation with their frozen classification. Reports include only classification, counts, schema/object digests, FTS result id digests, ledger outcome counts, error code, and source byte-identity boolean.

- [ ] **Step 6: Acquire dependencies and run all fixture tests offline**

Networked acquisition prerequisite:

```bash
cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml
cargo +1.96.0 fetch --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked
```

Offline replay:

```bash
CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --target x86_64-unknown-linux-gnu
```

Expected: bundled SQLite reports FTS5 enabled; the supported current and legacy cases produce exact neutral receipts; four non-convertible cases fail closed; all source and sibling-directory digests remain unchanged.

- [ ] **Step 7: Run the live record writer and focused verification**

Run: `CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_database --write`

Expected: writes a complete JSON/Markdown pair with `qualified` or `blocked`. No source-row value, memory text, note, provider value, absolute path, or raw SQLite error dump enters either record.

Run: `uv run pytest tests/qualification/test_run_database.py tests/qualification/test_model.py tests/qualification/test_render.py -v`

Run: `uv run ruff check tools/qualification/run_database.py tests/qualification/test_run_database.py`

Run: `cargo +1.96.0 fmt --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --check`

Run: `CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: all checks pass; record validation proves exact fixture/manifest ownership, complete criteria, accepted consequence, and source-byte identity.

- [ ] **Step 8: Commit the database qualification**

```bash
git add qualification/harnesses/legacy-database-import tools/qualification/run_database.py tests/qualification/test_run_database.py qualification/records/legacy-database-import.json docs/qualification/rust/legacy-database-import.md
git commit -m "test: qualify legacy database import"
```

### Task 7: Reviewed Aggregate Gate And Network-Free Replay

**Complexity:** Medium, 3–4 hours.

**Files:**
- Create: `tools/qualification/run.py`
- Create: `tools/qualification/check.py`
- Create: `tests/qualification/test_run.py`
- Create: `tests/qualification/test_gate.py`
- Create: `qualification/records/aggregate.json`
- Create: `docs/qualification/rust/gate.md`
- Modify: `qualification/README.md`
- Modify: `qualification/schemas/gate.schema.json`
- Modify: `.github/workflows/pr.yml`
- Modify after acceptance review: `qualification/records/mcp-transport.json`
- Modify after acceptance review: `qualification/records/semantic-native.json`
- Modify after acceptance review: `qualification/records/frontend-embedding.json`
- Modify after acceptance review: `qualification/records/legacy-database-import.json`
- Regenerate after acceptance review: `docs/qualification/rust/mcp-transport.md`
- Regenerate after acceptance review: `docs/qualification/rust/semantic-native.md`
- Regenerate after acceptance review: `docs/qualification/rust/frontend-embedding.md`
- Regenerate after acceptance review: `docs/qualification/rust/legacy-database-import.md`

**Interfaces:**
- Consumes: all four canonical records, their rendered Markdown, schemas, prerequisites, harness sources/lockfiles, and frozen input fingerprints.
- Produces: `run_one(risk: Risk, repo_root: Path, work_root: Path) -> QualificationRecord` and CLI `python -m tools.qualification.run <risk|all> --write`.
- Produces: `compute_gate(records: Mapping[Risk, QualificationRecord]) -> GateRecord`.
- Produces: `validation_and_review_issues(records: Mapping[Risk, QualificationRecord]) -> dict[Risk, tuple[str, ...]]` and `record_digests(records: Mapping[Risk, QualificationRecord]) -> dict[Risk, str]`.
- Produces: CLI `python -m tools.qualification.check [--record <risk>] [--records-only] [--require-qualified]`.
- Produces: canonical aggregate JSON and generated `docs/qualification/rust/gate.md`.

- [ ] **Step 1: Write the failing aggregate truth-table and offline-check tests**

```python
def test_all_pass_enables_semantic() -> None:
    gate = compute_gate(accepted_records(ROOT, semantic="semantic-enabled"))
    assert gate.status == "qualified"
    assert gate.release_mode == "semantic-enabled"
    assert gate.blocked_plans == ()


def test_semantic_failure_selects_fts_only_and_still_qualifies() -> None:
    gate = compute_gate(accepted_records(ROOT, semantic="fts-only"))
    assert gate.status == "qualified"
    assert gate.release_mode == "fts-only"
    assert gate.blocked_plans == ()


@pytest.mark.parametrize(
    ("risk", "blocked_plan"),
    [
        ("mcp-transport", "rust-daemon-mcp-security"),
        ("frontend-embedding", "rust-frontend"),
        ("legacy-database-import", "rust-database-upgrade"),
    ],
)
def test_non_semantic_failure_blocks_without_changing_specs(
    risk: Risk, blocked_plan: str
) -> None:
    records = accepted_records(ROOT, semantic="semantic-enabled")
    records[risk] = accepted_failure(ROOT, risk)
    gate = compute_gate(records)
    assert gate.status == "blocked"
    assert "rust-workspace-and-contract-harness" in gate.blocked_plans
    assert blocked_plan in gate.blocked_plans


def test_pending_review_is_blocking() -> None:
    pending = accepted_records(ROOT, semantic="fts-only")
    pending["mcp-transport"] = pending_review(pending["mcp-transport"])
    assert compute_gate(pending).status == "blocked"


def test_stale_input_is_blocking() -> None:
    stale = accepted_records(ROOT, semantic="fts-only")
    stale["frontend-embedding"] = replace(
        stale["frontend-embedding"], input_digest="0" * 64
    )
    assert compute_gate(stale).status == "blocked"


def test_records_only_check_never_invokes_live_runners(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("socket.create_connection", fail_if_called)
    result = invoke_check("--records-only")
    assert result.exit_code == 0
```

- [ ] **Step 2: Run the tests and verify the missing aggregate modules**

Run: `uv run pytest tests/qualification/test_run.py tests/qualification/test_gate.py -v`

Expected: FAIL importing `tools.qualification.run` and `tools.qualification.check`.

- [ ] **Step 3: Implement one dispatcher and non-short-circuiting live execution**

```python
RUNNERS: dict[Risk, Callable[[Path, Path], QualificationRecord]] = {
    "mcp-transport": run_mcp.run,
    "semantic-native": run_semantic.run,
    "frontend-embedding": run_frontend.run,
    "legacy-database-import": run_database.run,
}


def run_one(risk: Risk, repo_root: Path, work_root: Path) -> QualificationRecord:
    return RUNNERS[risk](repo_root, work_root / risk)
```

`all` executes risks in the dictionary order above, gives each a distinct work root, writes every complete result even after a failure, and returns success when all four records are complete and valid. Risk decisions are evaluated only by `check --require-qualified`; this prevents a valid blocking record from being discarded.

- [ ] **Step 4: Implement the exact aggregate decision function**

```python
BLOCKING_PLANS: dict[Risk, tuple[str, ...]] = {
    "mcp-transport": (
        "rust-workspace-and-contract-harness",
        "rust-daemon-mcp-security",
    ),
    "semantic-native": (),
    "frontend-embedding": (
        "rust-workspace-and-contract-harness",
        "rust-frontend",
        "rust-distribution-cutover",
    ),
    "legacy-database-import": (
        "rust-workspace-and-contract-harness",
        "rust-database-upgrade",
        "rust-distribution-cutover",
    ),
}


@dataclass(frozen=True)
class GateRecord:
    schema_version: int
    target: str
    status: Literal["qualified", "blocked"]
    release_mode: Literal["semantic-enabled", "fts-only"] | None
    blocked_plans: tuple[str, ...]
    issues: tuple[str, ...]
    record_digests: dict[Risk, str]
    acceptance_owner: str


def compute_gate(records: Mapping[Risk, QualificationRecord]) -> GateRecord:
    risk_issues = validation_and_review_issues(records)
    issues = tuple(
        sorted(
            f"{risk}: {issue}"
            for risk, found in risk_issues.items()
            for issue in found
        )
    )
    blocked: set[str] = set()
    for risk in REQUIRED_CRITERIA:
        record = records.get(risk)
        if risk_issues[risk]:
            if risk == "semantic-native":
                blocked.add("rust-workspace-and-contract-harness")
            else:
                blocked.update(BLOCKING_PLANS[risk])
        elif risk != "semantic-native" and record is not None and record.decision != "qualified":
            blocked.update(BLOCKING_PLANS[risk])
    qualified = not issues and not blocked
    semantic = records.get("semantic-native")
    if qualified:
        assert semantic is not None
        assert semantic.decision in ("semantic-enabled", "fts-only")
        release_mode = semantic.decision
    else:
        release_mode = None
    return GateRecord(
        schema_version=1,
        target="x86_64-unknown-linux-gnu",
        status="qualified" if qualified else "blocked",
        release_mode=release_mode,
        blocked_plans=tuple(sorted(blocked)),
        issues=issues,
        record_digests=record_digests(records),
        acceptance_owner="Pavel Obruchnikov <me@inkyquill.net>",
    )
```

`validation_and_review_issues` treats missing/extra risk, schema error, stale input digest, Markdown drift, redaction failure, incomplete evidence, consequence mismatch, `review.status != "accepted"`, or false review assertions as blocking. A reviewed semantic `fts-only` record is not an issue.

- [ ] **Step 5: Perform the named acceptance-owner review without altering evidence**

Every live runner writes this exact review block initially:

```json
{
  "owner": "Pavel Obruchnikov <me@inkyquill.net>",
  "status": "pending",
  "objective_evidence_reviewed": false,
  "normative_constraints_preserved": false
}
```

Pavel reviews the canonical evidence, input/lockfile digests, human rendering, and consequence. If the record accurately reflects the observed pass or failure without weakening a normative requirement, change only that record's review block to:

```json
{
  "owner": "Pavel Obruchnikov <me@inkyquill.net>",
  "status": "accepted",
  "objective_evidence_reviewed": true,
  "normative_constraints_preserved": true
}
```

If either assertion is false, set `status` to `rejected`, keep the false assertion false, regenerate Markdown, and leave the aggregate gate blocked. Never edit evidence status, measurements, input digest, or consequence during review; rerun the owning harness to change measured evidence.

- [ ] **Step 6: Implement network-free drift validation and aggregate rendering**

`check` loads schemas and records, recomputes every input fingerprint, validates redaction/review/consequence rules, regenerates four Markdown strings in memory, computes the gate, and compares canonical JSON/Markdown bytes. It imports no runner module in `--records-only` mode and opens no subprocess, socket, SQLite connection, Cargo cache, model, or frontend bundle; SQLite fixture files are read only as bytes for SHA-256.

`--record semantic-native` validates one risk but does not claim aggregate qualification. `--records-only` exits `0` for an internally consistent blocked gate. `--require-qualified` exits `1` and prints sorted issue/blocked-plan ids unless the gate is exactly `qualified`; when qualified, it prints exactly one of:

```text
Rust qualification gate: qualified (semantic-enabled)
Rust qualification gate: qualified (fts-only)
```

Run the accepted records through the renderer, write `qualification/records/aggregate.json`, and generate `docs/qualification/rust/gate.md`.

- [ ] **Step 7: Add contributor commands and the ordinary CI record gate**

Document exactly these workflows in `qualification/README.md`:

```bash
# One-time networked acquisitions
uv run python -m tools.qualification.acquire semantic-model
uv run python -m tools.qualification.acquire onnx-runtime
bun install --cwd frontend --frozen-lockfile
cargo +1.96.0 fetch --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked
cargo +1.96.0 fetch --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked
cargo +1.96.0 fetch --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked
cargo +1.96.0 fetch --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked

# Explicit live qualification; writes reviewed inputs but starts with pending review
CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run all --write

# Ordinary network-free validation
uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only

# Required before any dependent Rust implementation plan
uv run --no-cache --no-sync python -B -m tools.qualification.check --require-qualified

# Bounded cleanup; model/runtime deletion is separately explicit
uv run python -m tools.qualification.clean
uv run python -m tools.qualification.clean --apply
uv run python -m tools.qualification.clean --apply --include-model
```

Add only the `--records-only` command to normal PR CI. It validates durable records without network/native model/Bun prerequisites and allows an honest blocking record to merge. Any workflow that generates a dependent Rust plan must run `--require-qualified` first.

- [ ] **Step 8: Run complete qualification-stage verification**

Run: `uv run --no-cache --no-sync python -B -m tools.compatibility.check`

Expected: exit `0`; the frozen compatibility boundary has not drifted.

Run: `uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only`

Expected: exit `0`; records, fingerprints, redaction, reviews, Markdown, and checked-in aggregate gate are internally consistent without network access.

Run: `uv run --no-cache --no-sync python -B -m tools.qualification.check --require-qualified`

Expected for permission to write the next Rust workspace/dependent plan: exit `0` with exactly one qualified mode line. If it exits `1`, preserve the blocking aggregate record and stop before writing a dependent plan.

Run: `uv run pytest`

Run: `uv run ruff check .`

Run: `uv run ruff format --check .`

Expected: the full Python suite and required project checks pass.

Run: `bun run --cwd frontend format`

Run: `bun run --cwd frontend typecheck`

Run: `bun run --cwd frontend test`

Run: `bun run --cwd frontend build`

Expected: frontend format/type/test/build checks pass with the frozen lock and produce no source maps containing sensitive literals.

Run `cargo fmt --check` for all four manifests. Then attempt each harness test and Clippy command with `CARGO_NET_OFFLINE=true`, Rust 1.96.0, `--locked`, and target `x86_64-unknown-linux-gnu`; use `-D warnings` for Clippy. For frontend-embedding commands, set `HIERONYMUS_QUALIFICATION_ASSET_DIR=frontend/dist` after the successful Bun build.

Expected: rustfmt passes for every harness. A candidate that builds has passing harness unit/integration and Clippy checks offline; a native dependency that prevents build/test/Clippy has that exact command failure captured under `locked-native-build` (or the owning build criterion), dependent evidence marked `not-run`, and the required blocked/FTS-only decision. An uncaptured Rust failure is a stage-verification failure.

- [ ] **Step 9: Verify cleanup, repository scope, and final diff**

Run: `uv run python -m tools.qualification.clean --apply`

Expected: transient qualification work/install/log/target/bundle directories are removed; verified model/runtime acquisitions remain ignored and no tracked record changes.

Run: `git diff --check`

Run: `git status --short`

Expected: no whitespace errors; only files enumerated by this plan plus any pre-existing unrelated user changes appear. No production Rust path, translation workspace, runtime database, model binary, raw log, node_modules, frontend dist, or user-owned untracked file is staged.

- [ ] **Step 10: Commit the accepted aggregate gate**

```bash
git add tools/qualification/run.py tools/qualification/check.py tests/qualification/test_run.py tests/qualification/test_gate.py qualification/README.md qualification/schemas/gate.schema.json qualification/records docs/qualification/rust .github/workflows/pr.yml
git commit -m "ci: gate Rust plans on qualification records"
```

## Self-Review Record

- Spec coverage: Task 2 owns exact MCP revision/transports/registry/error parity and private-bridge absence; Tasks 3–4 own every ADR 0013 measured criterion and FTS-only selection; Task 5 owns actual Svelte release embedding and runtime independence; Task 6 owns every frozen database variant, typed read/accounting, FTS/ledger proof, fail-closed behavior, and source immutability; Task 7 owns review, reproducibility, accepted consequences, and the aggregate program-sequence gate.
- Normative consequence coverage: semantic failure has exactly one accepted non-blocking result, `fts-only`; MCP/frontend/database failure blocks named dependent plans and never edits fixtures or specifications to turn a failure into a pass.
- Network coverage: acquisition commands are named and checksum/frozen-lock constrained; record validation, aggregate calculation, and post-acquisition replay are explicitly offline.
- Sensitive-data coverage: inputs are synthetic/frozen, work roots are bounded, reports contain only digests/counts/basenames, source database bytes are verified unchanged, and canonical records reject secrets, user paths, row text, raw headers, and logs.
- Cleanup coverage: all transient output is under one ignored exact root, cleanup targets are enumerated and tested, model/runtime removal needs a separate flag, and no recursive operation can target the repository, home, translation workspace, or user data.
- Ownership coverage: common record code is completed before risk tasks; MCP, frontend, and database tasks own disjoint files; semantic Tasks 3–4 are deliberately sequential; only the final aggregate task revisits common schemas/records after all measurements exist.
- Type/signature consistency: every runner returns the Task 1 `QualificationRecord`; risk names and criterion ids come from `REQUIRED_CRITERIA`; every record uses the same `Evidence`, review, decision, consequence, fingerprint, and renderer contracts; Task 7 consumes those exact types without aliases.
- Production-scope check: the file map contains no production Rust workspace or crate path, and no task changes Python runtime behavior or starts a dependent implementation plan.

Before accepting this plan, run:

```bash
rg -n '\b(T[B]D|T[O]DO)\b|implement la[t]er|fill in deta[i]ls|Similar to Tas[k]' docs/superpowers/plans/2026-09-01-rust-qualification.md
git diff --check -- docs/superpowers/plans/2026-09-01-rust-qualification.md
git diff -- docs/superpowers/plans/2026-09-01-rust-qualification.md
git status --short
```

Expected: the red-flag scan prints nothing; diff check passes; the diff contains only this plan; the pre-existing unrelated `uv.lock` modification remains unstaged and untouched.

## Execution Handoff

Execute Tasks 1–7 only through the required sub-skill named in the header. After Task 7, a qualified aggregate permits the separate Rust workspace/contract-harness plan; a blocked aggregate is the durable stage result and requires a new candidate qualification run or an ADR-backed specification change before dependent planning.
