# Rust Qualification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce reproducible, reviewed qualification evidence for MCP transport, semantic native dependencies, frontend embedding, and legacy database import before any production Rust workspace or dependent implementation plan is started.

**Architecture:** First correct and re-review the frozen MCP 2026-07-28 authority against the official stateless request model, then commit that oracle before any candidate is qualified. Four standalone Rust harness crates live under `qualification/harnesses/` and exercise only synthetic or frozen compatibility inputs in disposable work directories; native executions are explicit opt-in qualification jobs, while ordinary Python tests use fakes and the record gate is network-free. Python tooling validates a common evidence schema, renders canonical JSON into Markdown, fingerprints every input, and computes one aggregate gate without rerunning heavy probes.

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
- MCP 2026-07-28 is stateless: there is no `initialize`, `notifications/initialized`, or transport session; every request carries protocol revision and client identity metadata, every HTTP request also carries `Mcp-Method` and `Mcp-Name`, and every successful tool result contains required `resultType`.
- SQLite remains authoritative for RAG sources, chunks, metadata, and semantic job state; embeddings and LanceDB tables are disposable derived artifacts.
- The semantic qualification must record exact crate versions/features, binary size, checksum-verified model load, a 10,000-chunk actual ANN index, checked series pre-filter-before-ANN plan/cardinality proof, zero cross-series hits, insert/search/delete, generation isolation, SQLite-durable lease/counter/cancellation recovery, zero SQLite-write-transaction spans across ONNX/LanceDB I/O, a complete 50-query run, and nonempty/isolation/rebuild-equivalent FTS fallback.
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
- Network access is permitted only for Task 1's named official-spec authority review and the explicit dependency-fetch, Bun-install, and checksum-verified model/runtime-acquisition steps. Ordinary record validation and every replay after acquisition run offline and make no network request.
- Ordinary `uv run pytest` tests inject bounded fake executables and never require Cargo, Rust artifacts, Bun packages, ONNX Runtime, a model, or a native-library cache. Live Rust/native qualification is explicit through `HIERONYMUS_QUALIFICATION_LIVE=1` commands and its dedicated workflow only.
- Every live run writes beneath `qualification/.artifacts/`, hashes frozen inputs before and after use, removes transient work/log/install directories on success or failure, and leaves only ignored caches plus reviewed records.
- Every Cargo build/test/Clippy command sets a risk-specific `CARGO_TARGET_DIR` beneath `qualification/.artifacts/cargo-target/`; crash probes disable core dumps and run in an owned process group that the runner terminates and reaps in `finally`.
- Each harness has its own `Cargo.toml` and committed `Cargo.lock`; direct risk dependencies are exact pins and all resolved versions/features are copied from `cargo metadata --locked` and `cargo tree -e features --locked` into the record.
- Each record's input digest covers `qualification/prerequisites.json`, `qualification/rust-toolchain.toml`, `tools/qualification/model.py`, `tools/qualification/render.py`, `tools/qualification/acquire.py`, its own Python runner, Cargo manifest/lockfile/source/tests, every consumed compatibility manifest/snapshot/fixture including every actual HTTP route case, and risk-specific frontend/corpus inputs; changing any of them makes the record stale.
- Pavel Obruchnikov `<me@inkyquill.net>` is the acceptance owner for all four records and the aggregate gate unless a compatibility-manifest entry explicitly delegates another named owner.

## Normative Inputs

- `https://modelcontextprotocol.io/specification/2026-07-28` (used only for Task 1 authority correction/review; ordinary replay consumes the corrected checked-in oracle)
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

## Execution And File Ownership Order

- Execute Tasks 1–16 in numeric order. Task 1's corrected compatibility oracle must be accepted as its own commit before Task 3 starts.
- Task 2 exclusively establishes common schemas/model/render/process/cleanup files. Task 6 is the only risk task allowed to extend the common acquisition/prerequisite files, and it starts only after Task 2 is accepted.
- MCP Tasks 3–5, semantic Tasks 6–9, frontend Tasks 10–12, and database Tasks 13–15 each use manifest/build, behavior/recovery, then runner/evidence commits. Within a risk, later tasks modify only files explicitly handed off by the prior task.
- Task 16 is the only task that revisits accepted records, gate schema, common README, dispatcher/checker, or CI. Do not implement risk tasks concurrently when they name the same file; never fold an earlier review boundary into a later commit.

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
- `tools/compatibility/inventory_mcp.py`: generator for the corrected official MCP 2026-07-28 protocol oracle.
- `tools/compatibility/inventory_http.py`: generator for MCP Host/auth/version/method/name HTTP route cases.
- `qualification/records/*.json`: canonical machine evidence for four risks plus the aggregate gate.
- `docs/qualification/rust/*.md`: generated human-readable records; never hand-edited independently of JSON.
- `tools/qualification/model.py`: typed records, required-criterion sets, consequence constants, input fingerprints, and redaction validation.
- `tools/qualification/render.py`: deterministic Markdown renderer and drift checker.
- `tools/qualification/acquire.py`: explicit atomic semantic-model acquisition with checksum verification.
- `tools/qualification/clean.py`: bounded cleanup for ignored qualification artifacts only.
- `tools/qualification/process.py`: owned process-group execution, core-dump suppression, timeouts, and bounded termination/reaping.
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

### Task 1: Correct And Re-Review The Official MCP 2026-07-28 Authority

**Complexity:** Medium, 2–3 hours.

**Files:**
- Modify: `tools/compatibility/inventory_mcp.py`
- Modify: `tools/compatibility/inventory_http.py`
- Modify: `tools/compatibility/check.py`
- Modify: `tests/compatibility/test_mcp_inventory.py`
- Modify: `tests/compatibility/test_http_inventory.py`
- Modify: `compatibility/fixtures/mcp/protocol.json`
- Modify: `compatibility/fixtures/http/route-cases.json`
- Modify if its MCP transport metadata changes: `compatibility/manifest.json`

**Interfaces:**
- Consumes: official MCP 2026-07-28 request/response and Streamable HTTP rules plus ADR 0015's transport/auth/Host requirements.
- Produces: `_protocol_fixture(snapshot: dict[str, object]) -> dict[str, object]` whose `target` contains stateless `tools/list` and `tools/call` requests, per-request `protocolVersion` and `clientInfo`, required `resultType`, stdio framing, and HTTP JSON/SSE variants without initialize/session fields.
- Produces: the `http.route.post.mcp` target cases in `_route_cases(snapshot: dict[str, object])`, covering valid request, invalid Host, missing/invalid bearer, missing/wrong protocol metadata, missing/wrong `Mcp-Method`, and missing/wrong `Mcp-Name`.
- Produces: a separately reviewed compatibility commit that is an immutable prerequisite of Tasks 3–5; candidate qualification must not edit or normalize this oracle.

- [ ] **Step 1: Replace obsolete fixture assertions with failing official-wire assertions**

```python
def test_protocol_fixture_is_stateless_2026_07_28() -> None:
    protocol = json.loads(
        (ROOT / "compatibility/fixtures/mcp/protocol.json").read_text(encoding="utf-8")
    )
    target = protocol["target"]
    compact = json.dumps(protocol, sort_keys=True, separators=(",", ":"))
    assert '"initialize":' not in compact
    assert "notifications/initialized" not in compact
    assert "Mcp-Session-Id" not in compact
    assert {"initialize", "initialized", "session"}.isdisjoint(target)
    for request in target["requests"]:
        assert request["params"]["protocolVersion"] == "2026-07-28"
        assert request["params"]["clientInfo"] == {
            "name": "compatibility-replay",
            "version": "1.0.0",
        }
    assert "Mcp-Session-Id" not in target["streamable_http"]["request"]["headers"]
    result_type = target["tool_call"]["response"]["result"]["resultType"]
    assert isinstance(result_type, str) and result_type


def test_http_mcp_cases_cover_official_metadata_and_local_security() -> None:
    case = _route_cases_by_id()["http.route.post.mcp"]["target"]
    success = case["success"]["request"]
    assert success["headers"]["Mcp-Method"] == "tools/call"
    assert success["headers"]["Mcp-Name"] == "hieronymus_status"
    assert success["body"]["params"]["protocolVersion"] == "2026-07-28"
    assert {failure["id"] for failure in case["failures"]} == {
        "invalid-host",
        "missing-bearer",
        "invalid-bearer",
        "missing-version",
        "unsupported-version",
        "missing-mcp-method",
        "wrong-mcp-method",
        "missing-mcp-name",
        "wrong-mcp-name",
    }
```

- [ ] **Step 2: Run the focused tests and verify RED against the obsolete fixture**

Run: `uv run pytest tests/compatibility/test_mcp_inventory.py::test_protocol_fixture_is_stateless_2026_07_28 tests/compatibility/test_http_inventory.py::test_http_mcp_cases_cover_official_metadata_and_local_security -v`

Expected: FAIL because the current target fixture still contains `initialize`, lacks per-request metadata and `resultType`, and omits the method/name HTTP failures.

- [ ] **Step 3: Implement the corrected generators and frozen target shapes**

Define target request params exactly as:

```python
REQUEST_METADATA = {
    "protocolVersion": "2026-07-28",
    "clientInfo": {"name": "compatibility-replay", "version": "1.0.0"},
}

tools_list_request = {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/list",
    "params": REQUEST_METADATA,
}
tool_call_request = {
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
        **REQUEST_METADATA,
        "name": "hieronymus_status",
        "arguments": {},
    },
}
```

Store those two objects in `target["requests"]` in list/call order and store the call request/response pair at `target["tool_call"]`. The target response for `tools/call` keeps the frozen content/error envelope and includes the official nonempty `resultType` field inside `result`; the focused test rejects absence or an empty value. Stdio stores one request/response JSON line per operation. HTTP success includes `Host: 127.0.0.1:<PORT>`, synthetic bearer, `MCP-Protocol-Version: 2026-07-28`, `Mcp-Method: tools/call`, and `Mcp-Name: hieronymus_status`; failure cases change or omit exactly one field and retain deterministic status/error bodies. Delete initialize, initialized-notification, `Mcp-Session-Id`, and transport-session lifecycle shapes from the entire protocol fixture, including `current`; retain only current registry/tool-response observation as explicitly non-authoritative historical evidence. Qualification consumes only `target`.

- [ ] **Step 4: Regenerate and prove the authority is complete and deterministic**

Run: `uv run python -m tools.compatibility.inventory_mcp --write`

Run: `uv run python -m tools.compatibility.inventory_http --write`

Run: `uv run pytest tests/compatibility/test_mcp_inventory.py tests/compatibility/test_http_inventory.py tests/compatibility/test_check.py -v`

Expected: PASS; regenerated target fixtures have no obsolete handshake/session shape, every target request has exact per-request metadata, HTTP cases cover Host/auth/version/method/name, and successful tool results require `resultType`.

- [ ] **Step 5: Run the compatibility gate before any candidate work**

Run: `uv run --no-cache --no-sync python -B -m tools.compatibility.check`

Expected: exit `0` with no snapshot, fixture, manifest, ownership, or route-case drift.

- [ ] **Step 6: Commit the corrected oracle as the candidate-qualification prerequisite**

```bash
git add tools/compatibility/inventory_mcp.py tools/compatibility/inventory_http.py tools/compatibility/check.py tests/compatibility/test_mcp_inventory.py tests/compatibility/test_http_inventory.py compatibility/fixtures/mcp/protocol.json compatibility/fixtures/http/route-cases.json compatibility/manifest.json
git commit -m "fix: align MCP compatibility oracle with 2026-07-28"
```

Stop if this commit is not accepted. Tasks 3–5 fingerprint and consume this corrected commit; they must never preserve or replay the obsolete target fixture.

### Task 2: Qualification Record Boundary And Safe Artifact Lifecycle

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
- Create: `tools/qualification/process.py`
- Create: `tests/qualification/factories.py`
- Create: `tests/qualification/test_model.py`
- Create: `tests/qualification/test_render.py`
- Create: `tests/qualification/test_clean.py`
- Create: `tests/qualification/test_process.py`
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
- Produces: `run_owned_process(argv: tuple[str, ...], *, cwd: Path, env: Mapping[str, str], timeout_seconds: int, no_progress_seconds: int) -> ProcessReceipt`; on Linux it uses a new session/process group, sets core size to zero before exec, terminates then kills the owned group on timeout/failure, reaps the leader, and returns only bounded digests/status/timings.
- Produces: `ProcessReceipt(exit_code: int | None, timed_out: bool, stdout_sha256: str, stderr_sha256: str, duration_ms: int, process_group_reaped: bool, core_dumps_disabled: bool)`.
- Test-only factory: `make_record(repo_root: Path, risk: Risk, *, failed: tuple[str, ...] = (), review_status: ReviewStatus = "pending") -> QualificationRecord`; it emits every required criterion and a current input digest.
- Test-only factory: `accepted_records(repo_root: Path, *, semantic: Literal["semantic-enabled", "fts-only"]) -> dict[Risk, QualificationRecord]`, `accepted_failure(repo_root: Path, risk: Risk) -> QualificationRecord`, `pending_review(record: QualificationRecord) -> QualificationRecord`, `write_fake_executable(tmp_path: Path, *, failed_criteria: tuple[str, ...] = ()) -> Path`, and `fake_child_and_grandchild(tmp_path: Path) -> tuple[str, ...]`.

- [ ] **Step 1: Write failing record, redaction, rendering, and cleanup tests**

```python
from dataclasses import replace
from pathlib import Path

from tests.qualification.factories import fake_child_and_grandchild, make_record
from tools.qualification.clean import cleanup_targets
from tools.qualification.model import (
    fingerprint_inputs,
    validate_record,
)
from tools.qualification.render import render_record
from tools.qualification.process import run_owned_process, safe_subprocess_env


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


def test_owned_process_disables_core_and_reaps_group(tmp_path: Path) -> None:
    receipt = run_owned_process(
        fake_child_and_grandchild(tmp_path),
        cwd=tmp_path,
        env=safe_subprocess_env(tmp_path, cargo_offline=True),
        timeout_seconds=1,
        no_progress_seconds=1,
    )
    assert receipt.timed_out
    assert receipt.core_dumps_disabled
    assert receipt.process_group_reaped
    assert list(tmp_path.glob("core*")) == []
```

- [ ] **Step 2: Run the tests and verify the missing-package failure**

Run: `uv run pytest tests/qualification/test_model.py tests/qualification/test_render.py tests/qualification/test_clean.py tests/qualification/test_process.py -v`

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
      "manual review of https://modelcontextprotocol.io/specification/2026-07-28",
      "cargo fetch --locked",
      "bun install --frozen-lockfile",
      "python -m tools.qualification.acquire semantic-model",
      "python -m tools.qualification.acquire onnx-runtime"
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
        "no-handshake-or-session",
        "per-request-client-metadata",
        "unsupported-version-rejected",
        "stdio-newline-jsonrpc",
        "streamable-http-json",
        "streamable-http-sse",
        "http-method-name-headers",
        "http-host-auth-version-cases",
        "registry-identity",
        "result-error-identity",
        "result-type-required",
        "required-auth-metadata",
        "private-bridge-absent",
    ),
    "semantic-native": (
        "locked-native-build",
        "binary-size-recorded",
        "install-uninstall",
        "model-checksum-load",
        "ten-thousand-chunk-build",
        "ann-index-created",
        "series-prefilter-before-ann",
        "zero-cross-series-hits",
        "insert-search-delete",
        "generation-isolation",
        "durable-sqlite-job-state",
        "no-sqlite-write-across-native-io",
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
        "runtime-asset-root-inaccessible",
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


COMMON_FINGERPRINT_INPUTS = (
    "qualification/prerequisites.json",
    "qualification/rust-toolchain.toml",
    "tools/qualification/model.py",
    "tools/qualification/render.py",
    "tools/qualification/acquire.py",
    "tools/qualification/process.py",
    "tools/qualification/clean.py",
)


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
    core_dumps_disabled: bool
    owned_process_groups_reaped: bool


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

Validation requires every criterion exactly once, forbids unknown criteria, requires a reason for `not-run`, derives record status from evidence, enforces the exact decision/consequence mapping, verifies every input path/digest, and requires `COMMON_FINGERPRINT_INPUTS`. Each runner appends its own runner, Cargo manifest/lock/source/tests, and every concrete consumed fixture path: MCP and frontend both include the full `compatibility/fixtures/http/route-cases.json`; MCP also includes every concrete tool input/wire file and the corrected protocol fixture. A nonempty ordered command list uses only repository-relative/artifact-relative arguments. All cleanup booleans except `user_data_opened` must be true and `user_data_opened` false. The serialized record is scanned for every forbidden data class in Global Constraints. Live runners create `Review(owner="Pavel Obruchnikov <me@inkyquill.net>", status="pending", objective_evidence_reviewed=False, normative_constraints_preserved=False)`; only Task 16 changes review fields. JSON schemas set `additionalProperties: false` on every fixed object; `measurements` permits only named scalar or scalar-array values, never nested logs or payloads.

- [ ] **Step 5: Implement deterministic rendering, acquisition, and bounded cleanup**

`render_record` prints title, decision, exact replay commands, environment/dependency tables, one row per required criterion, consumed compatibility ids, input digest, cleanup assertions, and the immutable consequence. It sorts mappings and dependency rows; it never includes raw stdout/stderr.

Expose deterministic CLIs `python -m tools.qualification.model validate <record.json>` and `python -m tools.qualification.render --check <record.json> <record.md>` so individual tasks can validate a record before the aggregate checker exists.

`acquire_semantic_model` uses `urllib.request`, writes only `model.onnx.part`, hashes while streaming, deletes a mismatched partial file, calls `os.replace` after success, and returns the verified final path. The initial URL must exactly match `prerequisites.json`; at most five redirect hops may use HTTPS with no userinfo/fragment and exact host in `huggingface.co`, `cdn-lfs.huggingface.co`, `cas-bridge.xethub.hf.co`, or the observed `us.aws.cdn.hf.co`. Redirect query strings may carry CDN signatures but are never logged or persisted; caller credentials/cookies are never forwarded. Any other hop fails before body download, and only the pinned final SHA-256 authorizes promotion.

`safe_subprocess_env` allowlists `PATH`, `CARGO_HOME`, `RUSTUP_HOME`, and `BUN_INSTALL_CACHE_DIR` when present; sets `HOME`, `TMPDIR`, `XDG_CACHE_HOME`, and `XDG_CONFIG_HOME` beneath the risk work root; sets `CARGO_NET_OFFLINE=true` for replay; and removes every other inherited variable. Tests set sentinel `HIERONYMUS_DATA_ROOT`, `HOME`, `HTTP_PROXY`, `OPENAI_API_KEY`, `Authorization`, and `COOKIE` values and assert none reaches a child probe or record.

`run_owned_process` is the only live-run subprocess primitive. Tests launch a fake child and grandchild, assert the process group is gone after timeout, assert `RLIMIT_CORE` is `(0, 0)` inside the child, and assert no `core` file exists. Every Cargo invocation receives an explicit `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/<risk>` through `safe_subprocess_env`.

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

Run: `uv run pytest tests/qualification/test_model.py tests/qualification/test_render.py tests/qualification/test_clean.py tests/qualification/test_process.py -v`

Run: `uv run ruff check tools/qualification tests/qualification`

Run: `uv run ruff format --check tools/qualification tests/qualification`

Expected: all focused tests pass; Ruff reports no errors or formatting drift.

- [ ] **Step 8: Commit the record boundary**

```bash
git add .gitignore qualification/README.md qualification/prerequisites.json qualification/rust-toolchain.toml qualification/schemas tools/qualification tests/qualification
git commit -m "test: define Rust qualification record contract"
```

### Task 3: MCP Candidate Manifest And Locked Build

**Complexity:** Low, 1–2 hours.

**Files:**
- Create: `qualification/harnesses/mcp-transport/Cargo.toml`
- Create: `qualification/harnesses/mcp-transport/Cargo.lock`
- Create: `qualification/harnesses/mcp-transport/src/main.rs` containing only `fn main() {}` until Task 4.

**Interfaces:**
- Consumes: accepted Task 1 oracle commit and Rust 1.96.0 qualification toolchain.
- Produces: standalone exact candidate manifest and committed lockfile; no transport behavior or record.
- Build invariant: every command sets `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport` and never creates a repository-local `target/`.

- [ ] **Step 1: Verify RED before the standalone manifest exists**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu`

Expected: FAIL because `qualification/harnesses/mcp-transport/Cargo.toml` does not exist.

- [ ] **Step 2: Create the standalone candidate manifest**

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

- [ ] **Step 3: Acquire, lock, and prove the empty candidate builds offline**

Networked prerequisite:

```bash
cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/mcp-transport/Cargo.toml
cargo +1.96.0 fetch --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked
```

Offline build:

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu
```

Expected: PASS without network access; `git status --short` shows no unbounded `target/` tree.

- [ ] **Step 4: Commit the independently reviewable candidate boundary**

```bash
git add qualification/harnesses/mcp-transport/Cargo.toml qualification/harnesses/mcp-transport/Cargo.lock qualification/harnesses/mcp-transport/src/main.rs
git commit -m "test: lock MCP qualification candidate"
```

### Task 4: MCP Stateless Transport Behavior

**Complexity:** High, 3–4 hours.

**Files:**
- Modify: `qualification/harnesses/mcp-transport/src/main.rs`
- Create: `qualification/harnesses/mcp-transport/src/registry.rs`
- Create: `qualification/harnesses/mcp-transport/src/report.rs`
- Create: `qualification/harnesses/mcp-transport/tests/transport.rs`

**Interfaces:**
- Consumes: Task 1's corrected `compatibility/fixtures/mcp/protocol.json`, `compatibility/fixtures/http/route-cases.json`, `compatibility/snapshots/mcp.json`, and Task 3 lockfile.
- Produces Rust CLI: `mcp-transport stdio --registry <path> --protocol <path>`.
- Produces Rust CLI: `mcp-transport http --registry <path> --protocol <path> --route-cases <path> --bind 127.0.0.1:0 --ready-file <path>`.
- Produces bounded JSON evidence for every MCP criterion; it never performs or accepts initialize/session behavior.

- [ ] **Step 1: Write failing stateless stdio/HTTP behavior tests**

In `tests/transport.rs`, load only `protocol["target"]`; assert `tools/list` and `tools/call` work without a preceding handshake, reject absent/wrong per-request protocol/client metadata, require `resultType`, and reject any `initialize`, `notifications/initialized`, or session header. Replay both HTTP response modes and every Task 1 `http.route.post.mcp` target case, including `Mcp-Method`/`Mcp-Name`, Host, bearer, and version failures.

- [ ] **Step 2: Run behavior tests and verify RED**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu --test transport`

Expected: FAIL because the empty binary exposes no transport.

- [ ] **Step 3: Implement the frozen registry and stateless transports**

`registry.rs` loads immutable `ToolDefinition` values and canned success/error fixture results only. It has no domain store or SQLite access. If rmcp cannot represent the official shape, report failure; do not introduce handshake state or alter fixtures.

```rust
pub const PROTOCOL_REVISION: &str = "2026-07-28";

pub trait RegistryProbe: Sized {
    fn load(snapshot: &Path) -> anyhow::Result<Self>;
    fn list_tools(&self) -> &[ToolDefinition];
    fn fixture_result(&self, name: &str, arguments: &Value) -> anyhow::Result<Value>;
}
```

Every request validates `protocolVersion` and `clientInfo`; HTTP additionally validates `MCP-Protocol-Version`, `Mcp-Method`, and `Mcp-Name` before dispatch. Every successful result includes `resultType`. Stdio emits one response JSON object plus `\n`; HTTP binds only the supplied loopback address and serves only `POST /mcp` as JSON or request-scoped SSE.

- [ ] **Step 4: Implement bounded reports and pass the offline behavior suite**

`report.rs` emits one bounded JSON object containing protocol version, transport, request/response SHA-256 values, content types, registry digest, stdout framing counts, and exit status. It never emits the bearer literal, headers, raw payload text, or absolute paths.

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu
```

Expected: the test command completes without network access and proves both exact-match and mismatch-reporting paths. Candidate protocol/transport support is decided only by the live fixture replay and becomes an honest qualified or blocking record.

- [ ] **Step 5: Commit transport behavior without records**

```bash
git add qualification/harnesses/mcp-transport/src qualification/harnesses/mcp-transport/tests/transport.rs
git commit -m "test: prove stateless MCP transport behavior"
```

### Task 5: MCP Runner And Qualification Evidence

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `tools/qualification/run_mcp.py`
- Create: `tests/qualification/test_run_mcp.py`
- Create: `qualification/records/mcp-transport.json`
- Create: `docs/qualification/rust/mcp-transport.md`

**Interfaces:**
- Consumes: Tasks 1, 3, and 4 plus every MCP tool input/wire fixture and manifest ids `cli.script.hieronymus-mcp`, `http.route.post.mcp`, `http.route.post.api.mcp.operation`, and `mcp.tool.*`.
- Produces Python: `run(repo_root: Path, work_root: Path, *, executable: Path) -> QualificationRecord` for fake-injected unit tests and `run_live(repo_root: Path, work_root: Path) -> QualificationRecord` for the opt-in CLI; `run_live` rejects missing `HIERONYMUS_QUALIFICATION_LIVE=1` before Cargo execution.
- Produces canonical `qualified` only when every MCP criterion passes; otherwise exact Task 2 `blocked` consequence.

- [ ] **Step 1: Write failing fake-only runner tests**

```python
def test_mcp_runner_consumes_corrected_oracle(tmp_path: Path) -> None:
    executable = write_fake_executable(tmp_path)
    record = run(ROOT, tmp_path, executable=executable)
    assert "compatibility/fixtures/mcp/protocol.json" in record.input_paths
    assert "compatibility/fixtures/http/route-cases.json" in record.input_paths
    assert {item.criterion for item in record.evidence} == set(
        REQUIRED_CRITERIA["mcp-transport"]
    )


def test_mcp_failure_preserves_adr_0015(tmp_path: Path) -> None:
    executable = write_fake_executable(tmp_path, failed_criteria=("result-type-required",))
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "blocked"
    assert record.consequence == FAILURE_CONSEQUENCES["mcp-transport"]
```

- [ ] **Step 2: Run the unit tests and verify RED without invoking Cargo**

Run: `uv run pytest tests/qualification/test_run_mcp.py -v`

Expected: FAIL importing `tools.qualification.run_mcp`; no Cargo command runs.

- [ ] **Step 3: Implement the fake-injectable live runner**

`run_mcp.run` uses `qualification/.artifacts/work/mcp-transport`, starts each transport with a 20-second ready/response timeout, replays every target transport case, compares tool lists by canonical JSON digest, compares one success and one error tool call across transports, verifies `/api/mcp/fixture` is absent, and gathers locked dependencies with:

```bash
cargo +1.96.0 metadata --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --format-version 1
cargo +1.96.0 tree --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked -e features
```

The runner uses `run_owned_process`, deletes ready files/logs/target output in `finally`, verifies all compatibility inputs are byte-identical, and records the corrected oracle commit/digest. It compares exact route-case status/body digests and never claims generic Host/auth routing beyond the frozen `POST /mcp` cases.

Run: `HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_mcp --write`

Expected: exit `0` means a complete, sanitized record was written. The record itself says `qualified` or `blocked`; a blocking candidate is retained rather than rewritten to pass.

- [ ] **Step 4: Verify unit tests, opt-in live evidence, and formatting**

Run: `uv run pytest tests/qualification/test_run_mcp.py tests/qualification/test_model.py tests/qualification/test_render.py -v`

Run: `uv run ruff check tools/qualification/run_mcp.py tests/qualification/test_run_mcp.py`

Run: `cargo +1.96.0 fmt --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --check`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: tests, Ruff, rustfmt, and Clippy pass; record validation reports no missing criterion, stale input, secret, or path leak.

- [ ] **Step 5: Commit the MCP qualification record**

```bash
git add tools/qualification/run_mcp.py tests/qualification/test_run_mcp.py qualification/records/mcp-transport.json docs/qualification/rust/mcp-transport.md
git commit -m "test: qualify MCP transport candidate"
```

### Task 6: Semantic Acquisition, Corpus, And Locked Candidate Build

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/fixtures/semantic-corpus.json`
- Create: `qualification/harnesses/semantic-native/Cargo.toml`
- Create: `qualification/harnesses/semantic-native/Cargo.lock`
- Create: `qualification/harnesses/semantic-native/src/lib.rs`
- Create: `qualification/harnesses/semantic-native/src/corpus.rs`
- Create: `qualification/harnesses/semantic-native/tests/corpus.rs`
- Modify: `qualification/prerequisites.json`
- Modify: `tools/qualification/acquire.py`
- Create: `tests/qualification/test_acquire.py`

**Interfaces:**
- Consumes: corrected Task 1 MCP RAG/recall success inputs as fixed query seeds and Task 2 acquisition boundary.
- Produces: `generate_corpus(spec: &CorpusSpec) -> anyhow::Result<(Vec<Chunk>, Vec<Query>)>` with exactly 10,000 chunks across 100 series and 50 queries.
- Produces: verified model/runtime directories plus standalone exact candidate manifest/lockfile; no semantic decision or record.

- [ ] **Step 1: Write failing deterministic-corpus and safe-acquisition tests**

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

```

In `tests/qualification/test_acquire.py`, use an in-memory ONNX archive with the official top directory and relative library symlink; assert the validated chain resolves within the extraction root. Add RED cases for `../` traversal, absolute links, link cycles, links escaping the top directory, and checksum mismatch. Add one accepted signed redirect to exact host `us.aws.cdn.hf.co` and one rejected redirect to `us.aws.cdn.hf.co.attacker.invalid`.

- [ ] **Step 2: Run the tests and confirm the missing manifest failure**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --no-default-features`

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

Extend `tools.qualification.acquire` with `onnx-runtime`. It downloads to `qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0.tgz.part`, permits only HTTPS redirects through `github.com`, `release-assets.githubusercontent.com`, and `objects.githubusercontent.com`, and verifies SHA-256 before extraction. Require exactly one archive top directory named `onnxruntime-linux-x64-1.28.0`; strip only that component. Extract regular files/directories first, then create relative symlinks only after lexically resolving every hop, rejecting absolute targets, `..` escape, cycles, dangling links, device entries, hard links, and any final target outside the extraction root. Validate the required `lib/libonnxruntime.so` chain and atomically rename the completed directory.

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
  "dep:ort",
]

[dependencies]
anyhow = "1"
arrow-array = { version = "=58.0.0", optional = true }
arrow-schema = { version = "=58.0.0", optional = true }
futures = "0.3"
lancedb = { version = "=0.37.1", default-features = false, optional = true }
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

`--no-default-features` compiles the FTS path without LanceDB, Arrow, ndarray, or ort references. Avoid a direct `ndarray` dependency: use the `ort` rc.13 re-export/input macros so no cross-version array values cross the public boundary. The native runtime is loaded explicitly from the verified acquisition path; no build script downloads ONNX Runtime.

- [ ] **Step 6: Acquire dependencies once and prove the candidate builds offline**

Networked acquisition prerequisites:

```bash
uv run python -m tools.qualification.acquire semantic-model
uv run python -m tools.qualification.acquire onnx-runtime
cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/semantic-native/Cargo.toml
cargo +1.96.0 fetch --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked
```

Offline replay:

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --no-default-features --target x86_64-unknown-linux-gnu
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu
```

Expected: both feature boundaries compile without network access and acquisition tests prove the exact redirect/extraction policy.

- [ ] **Step 7: Commit the acquisition and candidate boundary**

```bash
git add qualification/fixtures/semantic-corpus.json qualification/harnesses/semantic-native/Cargo.toml qualification/harnesses/semantic-native/Cargo.lock qualification/harnesses/semantic-native/src/lib.rs qualification/harnesses/semantic-native/src/corpus.rs qualification/harnesses/semantic-native/tests/corpus.rs qualification/prerequisites.json tools/qualification/acquire.py tests/qualification/test_acquire.py
git commit -m "test: lock semantic native qualification candidate"
```

### Task 7: Semantic ANN Index And Pre-Filter Proof

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/harnesses/semantic-native/src/model.rs`
- Create: `qualification/harnesses/semantic-native/src/index.rs`
- Create: `qualification/harnesses/semantic-native/tests/index.rs`

**Interfaces:**
- Consumes: Task 6 model/runtime, deterministic corpus, candidate lockfile, and LanceDB candidate.
- Produces: `OnnxEmbeddingProvider::load(runtime: &Path, model: &Path, expected_sha256: &str) -> anyhow::Result<Self>`.
- Produces: `async fn GenerationIndex::create(root: &Path, identity: ModelIdentity, generation: &str) -> anyhow::Result<Self>`, `create_ann_index(&mut self)`, `explain_prefiltered_search(&self, series_slug: &str, vector: &[f32], limit: usize)`, and append/search/delete/verify/activate methods.
- Proof rule: `series-prefilter-before-ann` passes only with a created ANN index, an adversarial cardinality/top-k result, and checked explain/analyze output showing the `series_slug` predicate below ANN nearest-neighbor execution; flat scan or post-filter/over-fetch is a failure.
- Test helpers in `tests/index.rs`: `populated_adversarial_index() -> anyhow::Result<GenerationIndex>` creates eligible ids `1..=10` plus 100 closer ineligible decoys; `needle() -> Vec<f32>` returns the fixed 384-float query; `expected_eligible_ids() -> Vec<i64>` returns `1..=10`.

- [ ] **Step 1: Write failing ANN creation and adversarial pre-filter tests**

```rust
#[tokio::test]
async fn search_uses_series_prefilter_before_ann() -> anyhow::Result<()> {
    let mut index = populated_adversarial_index().await?;
    index.create_ann_index().await?;
    let plan = index.explain_prefiltered_search("eligible", &needle(), 10).await?;
    assert!(plan.ann_index_used);
    assert!(plan.series_predicate_below_ann);
    assert_eq!(plan.eligible_cardinality, 10);
    let hits = index.search("eligible", &needle(), 10).await?;
    assert_eq!(hits.len(), 10);
    assert_eq!(hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(), expected_eligible_ids());
    Ok(())
}
```

Populate 10 eligible vectors whose distances rank below 100 closer ineligible decoys. Set `limit=10` and the candidate's ANN probe parameter low enough that post-filtering a global top-k returns fewer than 10; require all 10 exact eligible ids. The checked explain/analyze plan stores normalized operator/predicate/index names and cardinalities, never raw vectors.

- [ ] **Step 2: Run the ANN test and verify RED**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu --test index`

Expected: FAIL because the ANN index and explainable pre-filter path do not exist.

- [ ] **Step 3: Implement the model and ANN boundary**

`OnnxEmbeddingProvider` verifies checksum, loads the verified runtime, mean-pools, L2-normalizes, and returns an owned `Vec<f32>`; all ndarray values remain internal to ort's re-export. `GenerationIndex` stores chunk id, series slug, checksum, generation, and 384-float vector, creates a cosine IVF-PQ ANN index on the vector column with 16 partitions, waits until index statistics report all 10,000 rows indexed, and issues the series predicate as part of the ANN query. Evidence records index type/name, indexed-row count, distance metric, normalized plan operators, predicate placement, eligible cardinality, requested top-k, and returned count. It rejects any plan lacking both ANN index use and predicate pushdown.

- [ ] **Step 4: Prove insert/search/delete and generation isolation**

Add tests that insert a synthetic eligible chunk, find it through ANN, delete it, and prove absence after index refresh; create an incomplete second generation and prove all searches stay on the active first generation. Run the complete 10,000-chunk/50-query corpus twice and require identical corpus/index-input digests.

- [ ] **Step 5: Run offline tests and commit ANN behavior**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu --test index`

Expected: PASS with an actual ANN index, exact adversarial eligible ids/cardinality, and normalized explain/analyze proof; disabling index creation or moving the predicate to post-filter makes the test fail.

```bash
git add qualification/harnesses/semantic-native/src/model.rs qualification/harnesses/semantic-native/src/index.rs qualification/harnesses/semantic-native/tests/index.rs
git commit -m "test: prove semantic ANN prefiltering"
```

### Task 8: Semantic Durable Recovery And FTS Fallback

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/harnesses/semantic-native/src/main.rs`
- Create: `qualification/harnesses/semantic-native/src/scenario.rs`
- Create: `qualification/harnesses/semantic-native/src/fts.rs`
- Create: `qualification/harnesses/semantic-native/tests/recovery.rs`
- Create: `qualification/harnesses/semantic-native/tests/fts.rs`

**Interfaces:**
- Consumes: Tasks 6–7 model/runtime, corpus, lockfile, and ANN index behavior.
- Produces Rust CLI: `semantic-native scenario --work-dir <path> --state-db <path> --mode complete|crash|cancel --generation <id> --stop-after <count>`.
- Produces Rust CLI: `semantic-native query --work-dir <path> --series <slug> --query-index <0..49>`.
- Produces Rust CLI without native features: `semantic-native fts-fallback --work-dir <path> --series <slug> --query-index <0..49>`.
- Produces: durable SQLite job/generation state with leases, counters, cancellation, and recovery; filesystem progress is forbidden.
- Test helpers in `tests/recovery.rs`: `run_complete`, `run_crash`, `run_cancel`, `resume`, `read_active_generation`, `written_count`, `generation_status`, and `run_complete_with_probe` operate only beneath a supplied temp directory; crash uses a core-disabled owned child process group. `TransactionProbe::native_io_while_write_transaction() -> usize` exposes the violation counter.
- Test helpers in `tests/fts.rs`: `expected_fts_id_digests`, `run_all_fts_queries`, `digest_receipts`, and `delete_and_rebuild_fts` consume the deterministic corpus recipe and operate only on a temporary SQLite database.

- [ ] **Step 1: Write failing durable recovery, transaction-boundary, and FTS tests**

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
#[tokio::test]
async fn native_io_never_runs_inside_sqlite_write_transaction() -> anyhow::Result<()> {
    let probe = TransactionProbe::new();
    run_complete_with_probe(&probe).await?;
    assert_eq!(probe.native_io_while_write_transaction(), 0);
    Ok(())
}

#[test]
fn fts_fallback_is_nonempty_isolated_and_rebuild_equivalent() -> anyhow::Result<()> {
    let expected = expected_fts_id_digests();
    let before = run_all_fts_queries()?;
    assert!(before.iter().all(|receipt| receipt.eligible_count > 0));
    assert!(before.iter().all(|receipt| receipt.cross_series_count == 0));
    assert_eq!(digest_receipts(&before), expected);
    delete_and_rebuild_fts()?;
    assert_eq!(digest_receipts(&run_all_fts_queries()?), expected);
    Ok(())
}
```

- [ ] **Step 2: Run the focused tests and verify missing scenario/runner failures**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu --test recovery --test fts`

Expected: FAIL because the recovery test target and scenario module do not exist.

- [ ] **Step 3: Implement SQLite-owned job/generation state and transaction-free native I/O**

Create these qualification-only SQLite tables in `state-db`:

```sql
create table semantic_generations (
    generation_id text primary key,
    status text not null check (status in ('building','ready','active','superseded','cancelled','failed')),
    model_digest text not null,
    expected_count integer not null,
    written_count integer not null default 0,
    active integer not null default 0 check (active in (0,1))
);
create unique index one_active_semantic_generation
    on semantic_generations(active) where active = 1;
create table semantic_jobs (
    job_id text primary key,
    generation_id text not null references semantic_generations(generation_id),
    status text not null check (status in ('queued','running','cancel_requested','cancelled','complete','failed')),
    next_batch integer not null,
    completed_batches integer not null,
    lease_owner text,
    lease_expires_unix_ms integer,
    cancel_requested integer not null default 0 check (cancel_requested in (0,1))
);
```

For each batch: claim/renew lease and read cursor in a short committed SQLite write transaction; close the transaction; run ONNX and LanceDB I/O; then open a new short transaction to compare lease owner, persist written counters/checksums, and advance `next_batch`. `TransactionProbe` wraps every SQLite write transaction and every native call; entering ONNX/LanceDB with write depth nonzero is a hard failure and increments a recorded violation counter. No filesystem progress/manifest controls recovery or activation.

`--mode crash --stop-after 4200` aborts only after the batch receipt is committed. Resume takes an expired lease, verifies persisted counters against LanceDB ids/checksums, and continues exactly once. Cancellation is a committed request flag observed between batches; it closes handles, commits `cancelled`, clears the lease, and never activates that generation. Activation is one SQLite transaction after 10,000 rows, ANN/index verification, and sample queries succeed.

The CLI rejects work directories outside `qualification/.artifacts/work/semantic-native`, symlinks, model/runtime checksum mismatch, mixed dimensions, mixed generation ids, and non-loopback acquisition URLs before writing.

- [ ] **Step 4: Implement and prove the strengthened FTS fallback**

Create a disposable external-content FTS5 table over generated SQLite rows. `expected_fts_id_digests()` independently derives each query's eligible ids from the deterministic corpus recipe before opening SQLite, requires at least one expected id for every query, and returns 50 ordered SHA-256 digests. FTS search includes `series_slug` in the SQLite query, requires the same nonempty digests with zero cross-series ids, then deletes/rebuilds the FTS index and requires identical digests again. Compile this path with `--no-default-features`; model/runtime/index paths are absent and a download probe must remain untouched.

- [ ] **Step 5: Run offline recovery/fallback tests and commit behavior**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu --test recovery`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --no-default-features --target x86_64-unknown-linux-gnu --test fts`

Expected: durable lease/counter recovery, cancellation, active-generation isolation, zero native-I/O transaction violations, exact nonempty FTS id digests, series isolation, and rebuild equivalence all pass.

```bash
git add qualification/harnesses/semantic-native/src/main.rs qualification/harnesses/semantic-native/src/scenario.rs qualification/harnesses/semantic-native/src/fts.rs qualification/harnesses/semantic-native/tests/recovery.rs qualification/harnesses/semantic-native/tests/fts.rs
git commit -m "test: prove semantic recovery and FTS fallback"
```

### Task 9: Semantic Runner And Measured Evidence

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `tools/qualification/run_semantic.py`
- Create: `tests/qualification/test_run_semantic.py`
- Create: `qualification/records/semantic-native.json`
- Create: `docs/qualification/rust/semantic-native.md`

**Interfaces:**
- Consumes: Tasks 6–8 and frozen RAG/recall seed fixtures.
- Produces: `run(repo_root: Path, work_root: Path, *, executable: Path) -> QualificationRecord` for fake-injected pytest and `run_live(repo_root: Path, work_root: Path) -> QualificationRecord` for the opt-in CLI; `run_live` rejects missing `HIERONYMUS_QUALIFICATION_LIVE=1`.
- Produces: `semantic-enabled` only when all sixteen semantic criteria pass; any other complete result is `fts-only` and never blocks Linux release.

- [ ] **Step 1: Write failing fake-only decision and evidence tests**

```python
def test_any_semantic_failure_selects_fts_only(tmp_path: Path) -> None:
    executable = write_fake_executable(
        tmp_path, failed_criteria=("series-prefilter-before-ann",)
    )
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "fts-only"
    assert "do not block" in record.consequence


def test_prefilter_and_recovery_evidence_are_required(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=write_fake_executable(tmp_path))
    criteria = {item.criterion for item in record.evidence}
    assert "series-prefilter-before-ann" in criteria
    assert "no-sqlite-write-across-native-io" in criteria
```

- [ ] **Step 2: Run Python tests and verify RED without native prerequisites**

Run: `uv run pytest tests/qualification/test_run_semantic.py -v`

Expected: FAIL importing `tools.qualification.run_semantic`; no Rust/native process runs.

- [ ] **Step 3: Implement the full measured runner**

`run_semantic.run` first removes only `qualification/.artifacts/cargo-target/semantic-native`, recreates it empty, and sets `CARGO_TARGET_DIR` to that exact path so `locked-native-build` is a clean build. It then executes the following exact evidence sequence against fresh disposable paths:

1. Build the semantic binary with `--release --locked --features semantic-native` and record binary/library sizes plus `ldd` basenames.
2. Copy the binary, `libonnxruntime.so`, model, and an empty index root beneath `qualification/.artifacts/install/semantic-native`; run one query; remove the install directory; assert it no longer exists.
3. Verify model checksum before load and record 384 dimensions plus normalized output digest.
4. Build 10,000 chunks for `generation-a`, create and await the ANN index, and record normalized explain/analyze evidence.
5. Run the adversarial top-k/cardinality case and all 50 series-scoped queries; require exact eligible ids, pre-filter-before-ANN proof, and zero cross-series hits.
6. Insert one synthetic chunk, find it through ANN, delete it, and prove it is absent after refresh.
7. Create incomplete `generation-b`; prove searches remain on `generation-a`.
8. Record SQLite job/generation rows, leases, batch counters, and exactly zero native-I/O-inside-write-transaction violations.
9. With core dumps disabled and an owned process group, abort `generation-b` at 4,200, expire/take its lease, resume to 10,000, verify, activate once, and prove no duplicate ids.
10. Cancel a new generation at 3,200 through durable SQLite state and prove `generation-b` remains active.
11. Build the FTS binary with `--release --locked --no-default-features`, omit model/index paths, and prove all 50 nonempty expected eligible-id digests, series isolation, delete/rebuild equivalence, and no download attempt.

Every subprocess uses `run_owned_process`, has a 20-minute total timeout and a 2-minute no-progress timeout, has `RLIMIT_CORE=0`, and is terminated/reaped by owned process group in `finally`. A build or prerequisite failure marks itself `fail`; causally dependent criteria become `not-run` with the exact failing criterion in `not_run_reason`. The record remains complete and selects `fts-only`.

- [ ] **Step 4: Run the opt-in semantic qualification and write both records**

Run:

```bash
HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_semantic --write
```

Expected: exit `0` after atomically writing a complete `qualification/records/semantic-native.json` and matching Markdown. The JSON decision is exactly `semantic-enabled` when all criteria pass or `fts-only` when any criterion fails. No outcome from this command is `blocked`.

- [ ] **Step 5: Verify cleanup and offline reproducibility**

Run: `uv run python -m tools.qualification.clean`

Expected: dry-run lists only work/log/install/Cargo-target/frontend-dist descendants and does not list the acquired model/runtime directory.

Run: `uv run python -m tools.qualification.clean --apply`

Expected: transient paths are absent; model/runtime acquisitions remain.

Run: `uv run python -m tools.qualification.model validate qualification/records/semantic-native.json`

Run: `uv run python -m tools.qualification.render --check qualification/records/semantic-native.json docs/qualification/rust/semantic-native.md`

Expected: both commands pass without running Cargo, opening a network connection, or requiring transient artifacts; input fingerprints, redaction, evidence completeness, and generated Markdown match.

- [ ] **Step 6: Run focused code-quality checks**

Run: `uv run pytest tests/qualification/test_run_semantic.py tests/qualification/test_model.py tests/qualification/test_clean.py -v`

Run: `uv run ruff check tools/qualification/run_semantic.py tests/qualification/test_run_semantic.py`

Run: `cargo +1.96.0 fmt --manifest-path qualification/harnesses/semantic-native/Cargo.toml --check`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --all-targets --features semantic-native --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: Python and Rust checks pass. Record validation accepts either measured semantic decision and rejects missing or leaked evidence.

- [ ] **Step 7: Commit the semantic qualification record**

```bash
git add tools/qualification/run_semantic.py tests/qualification/test_run_semantic.py qualification/records/semantic-native.json docs/qualification/rust/semantic-native.md
git commit -m "test: record semantic native qualification"
```

### Task 10: Frontend Candidate Manifest And Reproducible Bundle Build

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `qualification/harnesses/frontend-embedding/Cargo.toml`
- Create: `qualification/harnesses/frontend-embedding/Cargo.lock`
- Create: `qualification/harnesses/frontend-embedding/build.rs`
- Create: `qualification/harnesses/frontend-embedding/src/main.rs` containing only `fn main() {}` until Task 11.

**Interfaces:**
- Consumes: `frontend/package.json`, `frontend/bun.lock`, `frontend/vite.config.ts`, and all `frontend/src/**`.
- Produces: a Svelte bundle at `qualification/.artifacts/frontend-dist/current` and standalone rust-embed manifest/lockfile.
- Build invariant: the RustEmbed folder is manifest-relative `../../.artifacts/frontend-dist/current/`; `build.rs` resolves it from `CARGO_MANIFEST_DIR`, and every Cargo command sets `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding`.

- [ ] **Step 1: Verify RED for the missing candidate manifest and bundle**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked --release --target x86_64-unknown-linux-gnu`

Expected: FAIL because the frontend-embedding manifest does not exist.

- [ ] **Step 2: Create the standalone rust-embed candidate manifest**

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

`build.rs` resolves `Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.artifacts/frontend-dist/current")`, canonicalizes it, requires `index.html` plus at least one `assets/` file, rejects symlinks escaping that root, and emits only exact `cargo:rerun-if-changed` paths. The runner, not the build script, builds Svelte before Cargo; no environment interpolation selects another asset root.

- [ ] **Step 3: Acquire Bun/Cargo dependencies and build without replay network**

One-time networked acquisition:

```bash
bun install --cwd frontend --frozen-lockfile
cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml
cargo +1.96.0 fetch --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked
```

Do not invoke a nonexistent Bun offline-install flag. For replay, reuse the lock-matched hydrated `frontend/node_modules`, clear `qualification/.artifacts/frontend-dist/current`, and execute the build in a Linux network namespace:

```bash
unshare --user --map-root-user --net -- bun run --cwd frontend build -- --outDir ../qualification/.artifacts/frontend-dist/current --emptyOutDir
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 build --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --target x86_64-unknown-linux-gnu
```

Expected: Bun reports `1.3.14`; `bun.lock` is unchanged; the build succeeds with no network namespace interface; exact file/byte digests are stable across two clean builds. If unprivileged network namespaces are unavailable, record the frontend build criterion failed—do not claim offline replay.

- [ ] **Step 4: Commit the independently reviewable build boundary**

```bash
git add qualification/harnesses/frontend-embedding/Cargo.toml qualification/harnesses/frontend-embedding/Cargo.lock qualification/harnesses/frontend-embedding/build.rs qualification/harnesses/frontend-embedding/src/main.rs
git commit -m "test: lock frontend embedding candidate"
```

### Task 11: Embedded Asset Resolution And Runtime Independence

**Complexity:** Medium, 2–3 hours.

**Files:**
- Modify: `qualification/harnesses/frontend-embedding/src/main.rs`
- Create: `qualification/harnesses/frontend-embedding/src/assets.rs`
- Create: `qualification/harnesses/frontend-embedding/tests/assets.rs`

**Interfaces:**
- Consumes: Task 10's canonical bundle and locked candidate.
- Produces: `AssetResponse resolve_asset(request_path: &str)` and CLI `frontend-embedding manifest|get --path <request-path>`.
- Ownership boundary: proves embedded path resolution, MIME/digests, SPA fallback, and filesystem independence only; it does not claim Host validation, bearer/session auth, CSRF, or full HTTP routing.

- [ ] **Step 1: Write failing embedded-resolution tests**

```rust
#[test]
fn embedded_index_and_spa_paths_resolve() {
    assert_eq!(resolve_asset("/").status, 200);
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

- [ ] **Step 2: Run the asset tests and verify RED**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked --release --target x86_64-unknown-linux-gnu --test assets`

Expected: FAIL because the resolver does not exist.

- [ ] **Step 3: Implement the embedded resolver with exact fallback rules**

```rust
#[derive(rust_embed::RustEmbed)]
#[folder = "../../.artifacts/frontend-dist/current/"]
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

The exact rules are: normalize one leading slash; reject `..`, backslash, percent-decoded separators, and NUL with 400; serve a present embedded file with its MIME type; return 404 for a missing path beneath `/assets/`; otherwise serve embedded `index.html`. The CLI returns only status, MIME, body digest, byte length, and fallback; it never prints bodies or claims HTTP security behavior.

- [ ] **Step 4: Prove filesystem/runtime independence under inaccessible asset roots**

After building, copy only the release binary into `qualification/.artifacts/install/frontend-embedding/bin`; move `qualification/.artifacts/frontend-dist/current` to `qualification/.artifacts/frontend-dist/quarantine/current`, then set the runner-owned quarantine directory to mode `000`. Execute every path probe under `strace -f -e trace=%file`; require the same response digests and no attempted open/stat/readlink beneath the original or quarantined asset root. Restore permissions and the asset root in `finally`, then remove the install/quarantine directories. Also require executable dependencies/process tree to contain no Bun, Node, or Python.

- [ ] **Step 5: Run behavior tests and commit**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked --release --target x86_64-unknown-linux-gnu --test assets`

Expected: PASS; missing bundle at compile time fails, embedded path behavior matches, and runtime asset-root access is unnecessary.

```bash
git add qualification/harnesses/frontend-embedding/src qualification/harnesses/frontend-embedding/tests/assets.rs
git commit -m "test: prove embedded frontend asset behavior"
```

### Task 12: Frontend Runner And Qualification Evidence

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `tools/qualification/run_frontend.py`
- Create: `tests/qualification/test_run_frontend.py`
- Create: `qualification/records/frontend-embedding.json`
- Create: `docs/qualification/rust/frontend-embedding.md`

**Interfaces:**
- Consumes: Tasks 10–11, frontend source/lock/build config, and only the path/status/MIME/body shape from manifest ids `frontend.route.get.root`, `frontend.route.get.admin`, `frontend.route.get.admin.path`, `frontend.route.get.assets.path`, `frontend.route.get.config`, and `frontend.route.get.config.path` in `route-cases.json`.
- Produces: `run(repo_root: Path, work_root: Path, *, executable: Path) -> QualificationRecord` for fake-injected pytest and `run_live(repo_root: Path, work_root: Path) -> QualificationRecord` for the opt-in CLI; `run_live` rejects missing `HIERONYMUS_QUALIFICATION_LIVE=1`.
- Produces: `qualified` only when every frontend criterion passes; otherwise exact Task 2 blocking consequence.

- [ ] **Step 1: Write failing fake-only runner tests**

```python
def test_frontend_runner_owns_embedded_path_contracts_only(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=write_fake_executable(tmp_path))
    assert set(record.contract_ids) == {
        "frontend.route.get.root",
        "frontend.route.get.admin",
        "frontend.route.get.admin.path",
        "frontend.route.get.assets.path",
        "frontend.route.get.config",
        "frontend.route.get.config.path",
    }
    assert "compatibility/fixtures/http/route-cases.json" in record.input_paths
    assert not {"http-host-validation", "browser-auth", "csrf"} & {
        item.criterion for item in record.evidence
    }


def test_frontend_failure_does_not_select_serve_dir(tmp_path: Path) -> None:
    executable = write_fake_executable(
        tmp_path, failed_criteria=("runtime-asset-root-inaccessible",)
    )
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "blocked"
    assert "embedded Svelte assets" in record.consequence
```

- [ ] **Step 2: Run Python tests and verify RED without Bun or Cargo**

Run: `uv run pytest tests/qualification/test_run_frontend.py -v`

Expected: FAIL importing `tools.qualification.run_frontend`; no Bun/Cargo/native command runs.

- [ ] **Step 3: Implement the live embedding runner and objective criteria**

`run_frontend.run` runs Task 10's network-isolated Bun build directly into the canonical artifact directory, fingerprints it, and builds release mode with:

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 build --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --target x86_64-unknown-linux-gnu
```

It then:

1. Records Bun version, frozen lock digest, bundle digest, and successful build.
2. Renames the canonical asset directory away, builds in fresh bounded `qualification/.artifacts/cargo-target/frontend-embedding-missing`, requires a stable compile-time missing-bundle failure, removes that target, then restores the asset root in `finally`.
3. Compares the embedded manifest's relative files, byte lengths, and SHA-256 values to the copied Vite output.
4. Replays only the six routes' embedded path/status/MIME/body expectations; excludes Host/auth/CSRF/router ownership from the record.
5. Performs Task 11's rename/permission-denial plus `strace` open proof with only the release binary present.
6. Requires `ldd` basenames and process tree to show no Bun, Node, or Python runtime.
7. Rejects any `.map` file and scans embedded bytes for `compat-secret-do-not-log`, `Authorization: Bearer`, `provider_key`, `/home/`, and `Yandex.Disk`.
8. Records release binary byte size without imposing an unapproved size threshold.
9. Runs Clippy with `-D warnings`, the canonical asset root, Cargo offline, and the bounded target before removing the asset root.

The runner uses `run_owned_process`, restores all renamed/permission-denied paths, then removes copied bundle, install directory, traces, and Cargo target in `finally`; it preserves only the canonical record and ignored Bun cache/node_modules.

- [ ] **Step 4: Run opt-in qualification and focused verification**

Run: `HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_frontend --write`

Expected: writes a complete `frontend-embedding.json` and generated Markdown. It records `qualified` or an honest `blocked` result; it never records filesystem serving or a runtime language dependency as an alternative.

Run: `uv run pytest tests/qualification/test_run_frontend.py tests/qualification/test_model.py tests/qualification/test_render.py -v`

Run: `uv run ruff check tools/qualification/run_frontend.py tests/qualification/test_run_frontend.py`

Run: `cargo +1.96.0 fmt --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --check`

Expected: Python tests and rustfmt pass; the live runner's bounded Clippy step passes before cleanup; record validation reports exact path-contract ownership, complete criteria, and no sensitive data.

- [ ] **Step 5: Commit the frontend qualification record**

```bash
git add tools/qualification/run_frontend.py tests/qualification/test_run_frontend.py qualification/records/frontend-embedding.json docs/qualification/rust/frontend-embedding.md
git commit -m "test: qualify frontend asset embedding"
```

### Task 13: Legacy Database Candidate Manifest And Locked Build

**Complexity:** Low, 1–2 hours.

**Files:**
- Create: `qualification/harnesses/legacy-database-import/Cargo.toml`
- Create: `qualification/harnesses/legacy-database-import/Cargo.lock`
- Create: `qualification/harnesses/legacy-database-import/src/main.rs` containing only `fn main() {}` until Task 14.

**Interfaces:**
- Consumes: Rust 1.96.0 and rusqlite 0.40.2 candidate.
- Produces: standalone bundled-SQLite manifest and committed lockfile; no database behavior or evidence.
- Build invariant: every Cargo command sets `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import`.

- [ ] **Step 1: Verify RED before the candidate manifest exists**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --target x86_64-unknown-linux-gnu`

Expected: FAIL because the legacy-database-import manifest does not exist.

- [ ] **Step 2: Create the minimal bundled-SQLite candidate manifest**

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

- [ ] **Step 3: Acquire, lock, and prove offline build**

```bash
cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml
cargo +1.96.0 fetch --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --target x86_64-unknown-linux-gnu
```

Expected: PASS without network access and no repository-local target directory.

- [ ] **Step 4: Commit the candidate boundary**

```bash
git add qualification/harnesses/legacy-database-import/Cargo.toml qualification/harnesses/legacy-database-import/Cargo.lock qualification/harnesses/legacy-database-import/src/main.rs
git commit -m "test: lock database import qualification candidate"
```

### Task 14: Read-Only Database Fixture Behavior

**Complexity:** High, 3–4 hours.

**Files:**
- Modify: `qualification/harnesses/legacy-database-import/src/main.rs`
- Create: `qualification/harnesses/legacy-database-import/src/classify.rs`
- Create: `qualification/harnesses/legacy-database-import/src/probe_import.rs`
- Create: `qualification/harnesses/legacy-database-import/src/report.rs`
- Create: `qualification/harnesses/legacy-database-import/tests/fixtures.rs`

**Interfaces:**
- Consumes: `compatibility/snapshots/state.json` and exactly the six frozen SQLite files under `compatibility/fixtures/database/`.
- Produces: `classify_read_only(source: &Path, fixture_root: &Path, state_contract: &Path) -> anyhow::Result<Classification>` and `probe_import(source: &Path, target: &Path, fixture_root: &Path, work_root: &Path, expected: &DatabaseContract) -> anyhow::Result<ProbeReceipt>`.
- Produces CLI: `legacy-database-import classify --fixture-root compatibility/fixtures/database --source-name <basename> --contract compatibility/snapshots/state.json` and `probe-import ... --work-root <risk-work-root> --target-name <basename>`; arbitrary source/target paths are not accepted.
- Test helpers in `tests/fixtures.rs`: `expected_cases()` returns `minimal-python.sqlite/supported-python/true`, `legacy-python.sqlite/supported-legacy-python/true`, `empty.sqlite/empty/false`, `partial-python.sqlite/partial-python/false`, `corrupt.sqlite/corrupt/false`, and `unknown-schema.sqlite/unknown-schema/false`; `fixture`, `fixture_root`, and `state_contract` resolve checked-in inputs; `assert_rejected_source` and `assert_rejected_target` invoke the CLI and require exit code `2` before SQLite opens.

- [ ] **Step 1: Write failing fixture-matrix and path-boundary tests**

```rust
#[test]
fn frozen_fixture_matrix_is_read_only() -> anyhow::Result<()> {
    for (name, classification, safe) in expected_cases() {
        let before = sha256(fixture(name))?;
        let actual = classify_read_only(&fixture(name), &fixture_root(), &state_contract())?;
        assert_eq!((actual.name.as_str(), actual.safe_to_convert), (classification, safe));
        assert_eq!(sha256(fixture(name))?, before);
    }
    Ok(())
}

#[test]
fn cli_rejects_source_or_target_outside_owned_roots() {
    assert_rejected_source("../../user.sqlite");
    assert_rejected_target("../../sibling.sqlite");
}
```

- [ ] **Step 2: Run fixture tests and verify RED**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --target x86_64-unknown-linux-gnu --test fixtures`

Expected: FAIL because classification/import and CLI root enforcement do not exist.

- [ ] **Step 3: Implement bounded classification and neutral probe import**

Open source fixtures with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`, immediately set `PRAGMA query_only=ON`, and never issue a source transaction or write pragma. Classification uses only integrity result, exact tables/columns/features from `state.json`, application migration-ledger presence, and the frozen variant expectations. It is a qualification classifier, not the production `StateClassifier`.

Canonicalize `fixture_root` and require it equals `repo_root/compatibility/fixtures/database`; accept `source-name` only when it is one of the six frozen basenames and its resolved path is a non-symlink direct child. Canonicalize `work_root` and require it is a non-symlink descendant of `qualification/.artifacts/work/legacy-database-import`; construct `target-name` beneath it and reject separators, `..`, existing symlinks, or any target outside that root.

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

- [ ] **Step 4: Implement objective FTS, ledger, and byte-identity checks**

For `minimal-python.sqlite`, compare table/column/index/trigger/foreign-key inventories and representative-row digests with `state.json`; run `PRAGMA integrity_check`, `PRAGMA foreign_key_check`, and one exact FTS query for strict terms, memories, concepts, crystals, and RAG chunks. For `legacy-python.sqlite`, require the legacy identity/fingerprint and full typed ledger accounting even when fewer tables are present.

Before and after every classify/import attempt, hash the source fixture and require equality. Hash every file in the fixture directory to prove no journal, WAL, sibling database, backup, or target appeared next to it. Unsupported/corrupt/partial/empty cases must fail before target creation with their frozen classification. Reports include only classification, counts, schema/object digests, FTS result id digests, ledger outcome counts, error code, and source byte-identity boolean.

- [ ] **Step 5: Run all fixture tests offline and commit behavior**

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --target x86_64-unknown-linux-gnu
```

Expected: bundled SQLite reports FTS5 enabled; the supported current and legacy cases produce exact neutral receipts; four non-convertible cases fail closed; all source and sibling-directory digests remain unchanged.

```bash
git add qualification/harnesses/legacy-database-import/src qualification/harnesses/legacy-database-import/tests/fixtures.rs
git commit -m "test: prove read-only database fixture import"
```

### Task 15: Database Runner And Qualification Evidence

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `tools/qualification/run_database.py`
- Create: `tests/qualification/test_run_database.py`
- Create: `qualification/records/legacy-database-import.json`
- Create: `docs/qualification/rust/legacy-database-import.md`

**Interfaces:**
- Consumes: Tasks 13–14, `compatibility/snapshots/state.json`, all six frozen database fixtures, and manifest ids `database.schema.current`, `database.migrations.current`, and `database.upgrade.preflight`.
- Produces: `run(repo_root: Path, work_root: Path, *, executable: Path) -> QualificationRecord` for fake-injected unit tests and `run_live(repo_root: Path, work_root: Path) -> QualificationRecord` for the opt-in CLI; `run_live` rejects missing `HIERONYMUS_QUALIFICATION_LIVE=1`.
- Produces: `qualified` only when every database criterion passes; otherwise exact Task 2 blocking consequence.

- [ ] **Step 1: Write failing fake-only runner tests**

```python
def test_database_runner_uses_only_frozen_fixture_root(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=write_fake_executable(tmp_path))
    assert all(
        not path.endswith(".sqlite")
        or path.startswith("compatibility/fixtures/database/")
        for path in record.input_paths
    )


def test_database_failure_preserves_data_disposition(tmp_path: Path) -> None:
    executable = write_fake_executable(
        tmp_path, failed_criteria=("source-byte-identity",)
    )
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "blocked"
    assert "fresh sibling database" in record.consequence
```

- [ ] **Step 2: Run Python tests and verify RED without Cargo**

Run: `uv run pytest tests/qualification/test_run_database.py -v`

Expected: FAIL importing `tools.qualification.run_database`; no Rust process runs.

- [ ] **Step 3: Implement the bounded runner and write the live record**

The runner passes the exact frozen fixture root and a risk work root to Task 14's CLI, never an arbitrary source/target path. It uses `run_owned_process`, hashes every fixture/input before and after, removes target databases/traces/Cargo target in `finally`, and records only classifications/counts/digests/error codes.

Run: `HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_database --write`

Expected: writes a complete JSON/Markdown pair with `qualified` or `blocked`. No source-row value, memory text, note, provider value, absolute path, or raw SQLite error dump enters either record.

Run: `uv run pytest tests/qualification/test_run_database.py tests/qualification/test_model.py tests/qualification/test_render.py -v`

Run: `uv run ruff check tools/qualification/run_database.py tests/qualification/test_run_database.py`

Run: `cargo +1.96.0 fmt --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --check`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: all checks pass; record validation proves exact fixture/manifest ownership, complete criteria, accepted consequence, and source-byte identity.

- [ ] **Step 4: Commit the database qualification record**

```bash
git add tools/qualification/run_database.py tests/qualification/test_run_database.py qualification/records/legacy-database-import.json docs/qualification/rust/legacy-database-import.md
git commit -m "test: qualify legacy database import"
```

### Task 16: Reviewed Aggregate Gate And Network-Free Replay

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
- Create: `.github/workflows/rust-qualification-live.yml`
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
- Produces: `run_one_live(risk: Risk, repo_root: Path, work_root: Path) -> QualificationRecord` and opt-in CLI `python -m tools.qualification.run <risk|all> --write`.
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
    monkeypatch.setattr("subprocess.run", fail_if_called)
    result = invoke_check("--records-only")
    assert result.exit_code == 0
```

- [ ] **Step 2: Run the tests and verify the missing aggregate modules**

Run: `uv run pytest tests/qualification/test_run.py tests/qualification/test_gate.py -v`

Expected: FAIL importing `tools.qualification.run` and `tools.qualification.check`.

- [ ] **Step 3: Implement one dispatcher and non-short-circuiting live execution**

```python
LIVE_RUNNERS: dict[Risk, Callable[[Path, Path], QualificationRecord]] = {
    "mcp-transport": run_mcp.run_live,
    "semantic-native": run_semantic.run_live,
    "frontend-embedding": run_frontend.run_live,
    "legacy-database-import": run_database.run_live,
}


def run_one_live(risk: Risk, repo_root: Path, work_root: Path) -> QualificationRecord:
    if os.environ.get("HIERONYMUS_QUALIFICATION_LIVE") != "1":
        raise RuntimeError("live qualification requires HIERONYMUS_QUALIFICATION_LIVE=1")
    return LIVE_RUNNERS[risk](repo_root, work_root / risk)
```

`all` executes risks in the dictionary order above, gives each a distinct work root, writes every complete result even after a failure, then recomputes/writes the blocked pending-review aggregate JSON/Markdown so the record set stays internally consistent. It returns success when all four records are complete and valid. Risk decisions are evaluated only by `check --require-qualified`; this prevents a valid blocking record from being discarded.

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
HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run all --write

# Ordinary network-free validation
uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only

# Required before any dependent Rust implementation plan
uv run --no-cache --no-sync python -B -m tools.qualification.check --require-qualified

# Bounded cleanup; model/runtime deletion is separately explicit
uv run python -m tools.qualification.clean
uv run python -m tools.qualification.clean --apply
uv run python -m tools.qualification.clean --apply --include-model
```

Append this step to the existing Python PR job in `.github/workflows/pr.yml` without adding Rust/Bun/cache setup:

```yaml
- name: Validate Rust qualification records
  env:
    HIERONYMUS_QUALIFICATION_LIVE: "0"
  run: |
    uv run --no-cache --no-sync pytest tests/qualification
    uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only
```

Every runner test injects a fake executable, so this job needs no Cargo cache, Bun install, ONNX Runtime, model, or network. It validates durable records and allows an honest blocking record to merge. Any workflow that generates a dependent Rust plan must run `--require-qualified` first.

Create `.github/workflows/rust-qualification-live.yml` exactly as an explicit manual job:

```yaml
name: Rust qualification live
on:
  workflow_dispatch:

jobs:
  qualify:
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@v4
      - uses: astral-sh/setup-uv@v6
      - uses: oven-sh/setup-bun@v2
        with:
          bun-version: 1.3.14
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.96.0
          targets: x86_64-unknown-linux-gnu
          components: clippy,rustfmt
      - name: Acquire locked prerequisites
        run: |
          uv sync --frozen
          uv run python -m tools.qualification.acquire semantic-model
          uv run python -m tools.qualification.acquire onnx-runtime
          bun install --cwd frontend --frozen-lockfile
          cargo +1.96.0 fetch --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked
          cargo +1.96.0 fetch --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked
          cargo +1.96.0 fetch --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked
          cargo +1.96.0 fetch --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked
      - name: Run isolated live qualification
        env:
          HIERONYMUS_QUALIFICATION_LIVE: "1"
          CARGO_NET_OFFLINE: "true"
        run: uv run python -m tools.qualification.run all --write
      - name: Validate sanitized records
        run: uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only
      - uses: actions/upload-artifact@v4
        with:
          name: rust-qualification-records
          path: |
            qualification/records/*.json
            docs/qualification/rust/*.md
```

The workflow never commits records. Each runner supplies its bounded risk-specific `CARGO_TARGET_DIR`; `run_owned_process` disables cores/reaps groups. A scheduled or pull-request trigger is forbidden.

- [ ] **Step 8: Run complete qualification-stage verification**

Run: `uv run --no-cache --no-sync python -B -m tools.compatibility.check`

Expected: exit `0`; the frozen compatibility boundary has not drifted.

Run: `uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only`

Expected: exit `0`; records, fingerprints, redaction, reviews, Markdown, and checked-in aggregate gate are internally consistent without network access.

Run: `uv run --no-cache --no-sync python -B -m tools.qualification.check --require-qualified`

Expected for permission to write the next Rust workspace/dependent plan: exit `0` with exactly one qualified mode line. If it exits `1`, preserve the blocking aggregate record and stop before writing a dependent plan.

Run: `env -u HIERONYMUS_QUALIFICATION_LIVE uv run --no-cache --no-sync pytest`

Run: `uv run --no-cache --no-sync ruff check .`

Run: `uv run --no-cache --no-sync ruff format --check .`

Expected: the full Python suite and required project checks pass with live qualification disabled; runner tests prove fake injection and fail if Cargo/Bun/native execution is attempted.

Run: `git diff --exit-code -- uv.lock frontend/bun.lock`

Expected: ordinary verification changed neither dependency lockfile. Rust/Bun/native harness and Clippy commands run only in the explicit live workflow or the live commands in Tasks 3–15, always with the risk-specific bounded Cargo target directory.

- [ ] **Step 9: Verify cleanup, repository scope, and final diff**

Run: `uv run python -m tools.qualification.clean --apply`

Expected: transient qualification work/install/log/target/bundle directories are removed; verified model/runtime acquisitions remain ignored and no tracked record changes.

Run: `git diff --check`

Run: `git status --short`

Expected: no whitespace errors; only files enumerated by this plan plus any pre-existing unrelated user changes appear. No production Rust path, translation workspace, runtime database, model binary, raw log, node_modules, frontend dist, or user-owned untracked file is staged.

- [ ] **Step 10: Commit the accepted aggregate gate**

```bash
git add tools/qualification/run.py tools/qualification/check.py tests/qualification/test_run.py tests/qualification/test_gate.py qualification/README.md qualification/schemas/gate.schema.json qualification/records docs/qualification/rust .github/workflows/pr.yml .github/workflows/rust-qualification-live.yml
git commit -m "ci: gate Rust plans on qualification records"
```

## Self-Review Record

- Spec coverage: Task 1 corrects and separately commits the official stateless MCP oracle; Tasks 3–5 qualify its exact metadata/headers/result type/transports/registry/error parity and private-bridge absence. Tasks 6–9 own actual ANN creation, checked pre-filter plan/cardinality proof, SQLite-durable recovery/no-write-transaction-native-I/O proof, strengthened FTS, and FTS-only selection. Tasks 10–12 own manifest-correct Svelte embedding and traced runtime independence without claiming HTTP security ownership. Tasks 13–15 own frozen-root database classification/import, typed accounting, FTS/ledger proof, fail-closed behavior, and source immutability. Task 16 owns review, reproducibility, accepted consequences, and the aggregate program-sequence gate.
- Normative consequence coverage: semantic failure has exactly one accepted non-blocking result, `fts-only`; MCP/frontend/database failure blocks named dependent plans and never edits fixtures or specifications to turn a failure into a pass.
- Network coverage: acquisition commands are named and checksum/frozen-lock constrained; Hugging Face redirects include the observed exact CDN host under hop validation; Bun replay uses an isolated network namespace rather than a nonexistent offline-install flag; ordinary pytest/check/aggregate replay uses fakes, no subprocesses, and no native caches.
- Sensitive-data coverage: inputs are synthetic/frozen, work roots are bounded, reports contain only digests/counts/basenames, source database bytes are verified unchanged, and canonical records reject secrets, user paths, row text, raw headers, and logs.
- Cleanup coverage: all transient output and every Cargo target are under one ignored exact root; core dumps are disabled; owned process groups are reaped; cleanup targets are enumerated/tested; model/runtime removal needs a separate flag; and no recursive operation can target the repository, home, translation workspace, or user data.
- Ownership coverage: Task 2 establishes common files; Task 6 alone extends acquisition inputs; every risk is split into separately committed manifest/build, behavior/recovery, and runner/evidence reviews; only Task 16 revisits common schemas/records/CI after measurements exist.
- Type/signature consistency: fake-injected `run(..., executable: Path)` and guarded `run_live(...)` are distinct for all four runners; risk/criterion ids come from `REQUIRED_CRITERIA`; every record uses Task 2's exact types; Task 16 dispatches only `run_live` and its records-only path imports no runner.
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

Execute Tasks 1–16 only through the required sub-skill named in the header. Task 1's corrected compatibility commit is a hard prerequisite and must be accepted before candidate qualification. After Task 16, a qualified aggregate permits the separate Rust workspace/contract-harness plan; a blocked aggregate is the durable stage result and requires a new candidate qualification run or an ADR-backed specification change before dependent planning.
