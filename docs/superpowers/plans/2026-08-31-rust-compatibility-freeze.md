# Rust Compatibility Freeze Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce the versioned, owner-assigned compatibility manifest and immutable Python/frontend snapshots that gate every later Rust implementation task.

**Architecture:** A small Python-only inventory tool reads public surfaces from the shipping Click, FastMCP, HTTP, config, database, frontend, and installer sources without mutating user data. It writes normalized JSON snapshots; a reviewed manifest assigns each snapshot item a disposition, acceptance owner, technical owner, fixture, and future Rust test target. CI regenerates snapshots in memory and fails on unreviewed drift.

**Tech Stack:** Python 3.12, Click introspection, FastMCP `list_tools()`, `ast`, `tomllib`, SQLite, JSON Schema Draft 2020-12, pytest, Ruff.

**Spec:** `docs/superpowers/specs/2026-08-31-rust-compatibility-contracts-design.md`

## Global Constraints

- Pavel Obruchnikov `<me@inkyquill.net>` is the acceptance owner until a manifest entry explicitly delegates another named owner.
- Every entry has one technical owner from: `data-config`, `database-upgrade`, `terminology-memory`, `dreaming`, `semantic-rag`, `daemon-mcp-security`, or `distribution-cutover`.
- Tool and test counts are derived from snapshots; no count is copied into source or documentation.
- Snapshot generation is read-only and uses only synthetic temporary data roots.
- Secrets in fixtures are synthetic; generated output must never contain values loaded from the developer's real data root.
- MCP contract metadata pins protocol revision `2026-07-28`; `/api/mcp/{operation}` is classified as a removed private Python bridge, not Streamable HTTP.
- Current installed entry points `hiero`, `hieronymus`, `hieronymus-agent-hook`, and `hieronymus-mcp` all receive explicit dispositions.
- This phase does not create Rust production code or change Python runtime behavior.

## File Map

- `compatibility/manifest.schema.json`: machine validation for manifest records.
- `compatibility/manifest.json`: reviewed authority for preservation/change/removal decisions and ownership.
- `compatibility/snapshots/*.json`: normalized current-surface inventories.
- `compatibility/fixtures/`: synthetic canonical inputs and expected outputs referenced by manifest ids.
- `tools/compatibility/model.py`: typed manifest loader and invariant validation.
- `tools/compatibility/inventory_cli.py`: Click and package-script inventory.
- `tools/compatibility/inventory_mcp.py`: FastMCP registry inventory.
- `tools/compatibility/inventory_http.py`: Python router and Svelte-call inventory.
- `tools/compatibility/inventory_state.py`: config/database/installer/test inventory.
- `tools/compatibility/check.py`: aggregate drift and coverage command.
- `tests/compatibility/`: focused tests for the inventory and manifest gates.

---

### Task 1: Manifest Model And Validation Gate

**Files:**
- Create: `compatibility/manifest.schema.json`
- Create: `compatibility/manifest.json`
- Create: `tools/__init__.py`
- Create: `tools/compatibility/__init__.py`
- Create: `tools/compatibility/model.py`
- Create: `tests/compatibility/test_manifest.py`

**Interfaces:**
- Consumes: JSON files rooted at the repository directory.
- Produces: `load_manifest(path: Path) -> Manifest` and `validate_manifest(manifest: Manifest, repo_root: Path) -> list[str]`.

- [ ] **Step 1: Write the failing manifest validation tests**

```python
from pathlib import Path

from tools.compatibility.model import load_manifest, validate_manifest


ROOT = Path(__file__).resolve().parents[2]


def test_checked_in_manifest_is_valid() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    assert validate_manifest(manifest, ROOT) == []


def test_duplicate_contract_id_is_rejected(tmp_path: Path) -> None:
    path = tmp_path / "manifest.json"
    path.write_text(
        '{"manifest_version":1,"python_reference":"0.7.0",'
        '"contracts":[{"id":"cli.hiero","surface":"cli"},'
        '{"id":"cli.hiero","surface":"cli"}]}',
        encoding="utf-8",
    )
    errors = validate_manifest(load_manifest(path), ROOT)
    assert errors == ["duplicate contract id: cli.hiero"]
```

- [ ] **Step 2: Run the tests and verify the missing module failure**

Run: `uv run pytest tests/compatibility/test_manifest.py -v`

Expected: FAIL during collection with `ModuleNotFoundError: No module named 'tools.compatibility'`.

- [ ] **Step 3: Add the JSON schema and typed loader**

Define `Contract` with required fields `id`, `surface`, `acceptance_owner`, `technical_owner`, `python_entry_point`, `tests`, `fixture`, `rust_test_target`, and `disposition`. Restrict `surface`, `technical_owner`, and `disposition` to the exact values in the accepted spec. Implement `validate_manifest` so it reports duplicate ids, missing referenced fixture/test paths, blank owners, and `intentionally-change`/`remove` entries without an `adr`.

```python
@dataclass(frozen=True)
class Contract:
    id: str
    surface: str
    acceptance_owner: str
    technical_owner: str
    python_entry_point: str
    tests: tuple[str, ...]
    fixture: str
    rust_test_target: str
    disposition: str
    adr: str | None = None


@dataclass(frozen=True)
class Manifest:
    manifest_version: int
    python_reference: str
    contracts: tuple[Contract, ...]
```

Seed `compatibility/manifest.json` with a valid empty contract list; later tasks populate it only from reviewed snapshots.

- [ ] **Step 4: Run the manifest tests**

Run: `uv run pytest tests/compatibility/test_manifest.py -v`

Expected: PASS.

- [ ] **Step 5: Commit the validation boundary**

```bash
git add compatibility/manifest.schema.json compatibility/manifest.json tools/__init__.py tools/compatibility/__init__.py tools/compatibility/model.py tests/compatibility/test_manifest.py
git commit -m "test: add compatibility manifest validation"
```

### Task 2: CLI And Installed Entrypoint Snapshot

**Files:**
- Create: `tools/compatibility/inventory_cli.py`
- Create: `compatibility/snapshots/cli.json`
- Create: `compatibility/fixtures/cli/help-root.txt`
- Create: `compatibility/fixtures/cli/agent-hook-help.txt`
- Create: `tests/compatibility/test_cli_inventory.py`
- Modify: `compatibility/manifest.json`

**Interfaces:**
- Consumes: `pyproject.toml`, `hieronymus.cli:main`, and `hieronymus.agent_hooks:main`.
- Produces: `snapshot_cli(repo_root: Path) -> dict[str, object]` with sorted `scripts` and recursively sorted Click `commands`.

- [ ] **Step 1: Write the failing CLI inventory test**

```python
def test_cli_snapshot_matches_shipping_commands() -> None:
    actual = snapshot_cli(ROOT)
    expected = json.loads((ROOT / "compatibility/snapshots/cli.json").read_text())
    assert actual == expected
    assert set(actual["scripts"]) == {
        "hiero",
        "hieronymus",
        "hieronymus-agent-hook",
        "hieronymus-mcp",
    }
    assert "session-start" in actual["command_paths"]["hieronymus-agent-hook"]
    assert "session-end" in actual["command_paths"]["hieronymus-agent-hook"]
```

- [ ] **Step 2: Run the focused test and confirm the missing exporter failure**

Run: `uv run pytest tests/compatibility/test_cli_inventory.py -v`

Expected: FAIL importing `tools.compatibility.inventory_cli`.

- [ ] **Step 3: Implement deterministic Click traversal**

Use `tomllib` for `[project.scripts]`. Traverse `click.Group.commands` recursively without invoking command callbacks. Record command path, option names, parameter kinds, defaults after replacing filesystem paths with `<PATH>`, required flags, help text, and declared exit behavior. Render root/agent-hook help with `click.testing.CliRunner` and store the exact normalized text fixtures.

```python
def walk(group: click.Group, prefix: tuple[str, ...] = ()) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for name, command in sorted(group.commands.items()):
        path = (*prefix, name)
        rows.append(command_record(path, command))
        if isinstance(command, click.Group):
            rows.extend(walk(command, path))
    return rows
```

- [ ] **Step 4: Generate the snapshot and add manifest entries**

Run: `uv run python -m tools.compatibility.inventory_cli --write`

Add one manifest entry per installed script and public command path. Give the legacy scripts `preserve` dispositions implemented by the command-link routing in the distribution spec; set canonical Rust targets to `crates/hiero-cli/tests/cli_contract.rs::<contract_id_with_underscores>`.

- [ ] **Step 5: Re-run the CLI and manifest tests**

Run: `uv run pytest tests/compatibility/test_cli_inventory.py tests/compatibility/test_manifest.py -v`

Expected: PASS.

- [ ] **Step 6: Commit the CLI freeze**

```bash
git add tools/compatibility/inventory_cli.py compatibility/snapshots/cli.json compatibility/fixtures/cli compatibility/manifest.json tests/compatibility/test_cli_inventory.py
git commit -m "test: freeze CLI compatibility surface"
```

### Task 3: MCP Registry Snapshot

**Files:**
- Create: `tools/compatibility/inventory_mcp.py`
- Create: `compatibility/snapshots/mcp.json`
- Create: `compatibility/fixtures/mcp/protocol.json`
- Create: `tests/compatibility/test_mcp_inventory.py`
- Modify: `compatibility/manifest.json`

**Interfaces:**
- Consumes: `asyncio.run(hieronymus.mcp_server.server.list_tools())` and ADR 0015.
- Produces: `snapshot_mcp() -> dict[str, object]` containing the derived registry and exact protocol/transport declarations.

- [ ] **Step 1: Write the failing registry test**

```python
def test_mcp_snapshot_matches_fastmcp_registry() -> None:
    snapshot = snapshot_mcp()
    expected = json.loads((ROOT / "compatibility/snapshots/mcp.json").read_text())
    assert snapshot == expected
    assert snapshot["protocol_revision"] == "2026-07-28"
    assert snapshot["transports"] == ["stdio", "streamable-http"]
    assert len(snapshot["tools"]) == snapshot["derived_tool_count"]
```

- [ ] **Step 2: Run the focused test and verify it fails before implementation**

Run: `uv run pytest tests/compatibility/test_mcp_inventory.py -v`

Expected: FAIL importing `tools.compatibility.inventory_mcp`.

- [ ] **Step 3: Implement FastMCP schema normalization**

Serialize each returned tool's `name`, `description`, and `inputSchema`; sort object keys and sort tools by name, but do not normalize descriptions, required fields, enum order, or types. Add transport metadata explicitly from ADR 0015. Record `/api/mcp/{operation}` separately as `private_python_bridge` with `disposition: remove` and `adr: docs/adr/0015-mcp-protocol-and-transport.md`.

```python
async def registered_tools() -> list[dict[str, object]]:
    tools = await mcp_server.server.list_tools()
    return sorted(
        (
            {
                "name": tool.name,
                "description": tool.description or "",
                "input_schema": tool.inputSchema,
            }
            for tool in tools
        ),
        key=lambda item: str(item["name"]),
    )
```

- [ ] **Step 4: Generate snapshot and populate one manifest entry per tool**

Run: `uv run python -m tools.compatibility.inventory_mcp --write`

Each tool entry references a synthetic success/error fixture directory `compatibility/fixtures/mcp/<tool-name>/` and a future Rust target `crates/hiero-mcp/tests/registry_contract.rs::<tool_name>`. Create the directories with `success.input.json`, `success.output.json`, and `error.output.json` only when the current Python tests already establish those results; otherwise make manifest validation fail with `missing_fixture` until the fixture is supplied in this task.

- [ ] **Step 5: Run MCP inventory and manifest tests**

Run: `uv run pytest tests/compatibility/test_mcp_inventory.py tests/compatibility/test_manifest.py -v`

Expected: PASS with the tool count derived from the snapshot.

- [ ] **Step 6: Commit the MCP freeze**

```bash
git add tools/compatibility/inventory_mcp.py compatibility/snapshots/mcp.json compatibility/fixtures/mcp compatibility/manifest.json tests/compatibility/test_mcp_inventory.py
git commit -m "test: freeze MCP registry contracts"
```

### Task 4: HTTP, WebSocket, And Frontend Request Snapshot

**Files:**
- Create: `tools/compatibility/inventory_http.py`
- Create: `compatibility/snapshots/http.json`
- Create: `compatibility/fixtures/http/route-cases.json`
- Create: `tests/compatibility/test_http_inventory.py`
- Modify: `compatibility/manifest.json`

**Interfaces:**
- Consumes: `src/hieronymus/service_http.py` AST and literal `request()` calls in `frontend/src/web/lib/api.ts`.
- Produces: `snapshot_http(repo_root: Path) -> dict[str, object]` with concrete method/path/auth/frontend-consumer records.

- [ ] **Step 1: Write the failing route coverage test**

```python
def test_every_frontend_request_has_a_route_contract() -> None:
    snapshot = snapshot_http(ROOT)
    route_keys = {(row["method"], row["path_template"]) for row in snapshot["routes"]}
    for call in snapshot["frontend_calls"]:
        assert (call["method"], call["path_template"]) in route_keys


def test_required_route_families_are_explicit() -> None:
    paths = {row["path_template"] for row in snapshot_http(ROOT)["routes"]}
    assert {"/health", "/status", "/shutdown", "/mcp", "/ws/admin"} <= paths
    assert "/api/mcp/{operation}" in paths
```

- [ ] **Step 2: Run the focused test and verify the exporter is absent**

Run: `uv run pytest tests/compatibility/test_http_inventory.py -v`

Expected: FAIL importing `tools.compatibility.inventory_http`.

- [ ] **Step 3: Implement extraction plus an explicit route table**

Parse `do_GET`, `do_POST`, and `do_DELETE` string comparisons/prefixes from the Python AST. Parse literal and template-string paths passed to `request()` in `api.ts`, normalizing interpolations to `{id}`, `{action}`, and query fields. Merge these discovered records with an explicit reviewed table for `/mcp`, SPA routes/assets, WebSocket upgrade, auth changes, and private-bridge removal. Fail if a discovered route is absent from the reviewed table.

```python
REVIEWED_ROUTES = (
    Route("GET", "/health", "intentionally-change", "ADR-0012"),
    Route("GET", "/status", "intentionally-change", "ADR-0012"),
    Route("POST", "/shutdown", "intentionally-change", "ADR-0012"),
    Route("POST", "/mcp", "intentionally-change", "ADR-0015"),
    Route("POST", "/api/mcp/{operation}", "remove", "ADR-0015"),
    Route("GET", "/ws/admin", "intentionally-change", "ADR-0012"),
)
```

The final explicit table also contains every providers/settings/admin/static route enumerated in the accepted compatibility spec, with the actual current method from `service_http.py` and the body/response shape from `api.ts`.

- [ ] **Step 4: Add request/response/auth fixture cases and manifest records**

Run: `uv run python -m tools.compatibility.inventory_http --write`

`route-cases.json` stores one success and one relevant auth/validation failure per concrete route. Use sentinel credential `compat-secret-do-not-log`; assert it is absent from expected response and normalized log fields.

- [ ] **Step 5: Run Python and frontend contract checks**

Run: `uv run pytest tests/compatibility/test_http_inventory.py tests/test_service_http.py -v`

Run: `cd frontend && bun run test --run frontend/src/web/app.test.ts frontend/src/web/lib`

Expected: both commands PASS.

- [ ] **Step 6: Commit the network/frontend freeze**

```bash
git add tools/compatibility/inventory_http.py compatibility/snapshots/http.json compatibility/fixtures/http compatibility/manifest.json tests/compatibility/test_http_inventory.py
git commit -m "test: freeze HTTP and frontend contracts"
```

### Task 5: Config, Database, Installer, And Test Ownership Snapshot

**Files:**
- Create: `tools/compatibility/inventory_state.py`
- Create: `compatibility/snapshots/state.json`
- Create: `compatibility/fixtures/config/current/`
- Create: `compatibility/fixtures/database/minimal-python.sqlite`
- Create: `tests/compatibility/test_state_inventory.py`
- Modify: `compatibility/manifest.json`

**Interfaces:**
- Consumes: config dataclasses/loaders, `src/hieronymus/migrations/*.sql`, synthetic SQLite roots, `pyproject.toml`, `install.sh`, `uninstall.sh`, and collected pytest node ids.
- Produces: `snapshot_state(repo_root: Path) -> dict[str, object]` and `collect_test_nodeids(repo_root: Path) -> list[str]`.

- [ ] **Step 1: Write failing isolation and ownership tests**

```python
def test_state_snapshot_uses_only_explicit_synthetic_root(tmp_path: Path) -> None:
    root = tmp_path / "data-root"
    snapshot = snapshot_state(ROOT, root)
    assert snapshot["data_root"] == "<DATA_ROOT>"
    assert {item["path"] for item in snapshot["owned_paths"]} >= {
        "hieronymus.sqlite",
        "provider.conf",
        "dream.conf",
        "ingest.conf",
        "release.conf",
        "llmcache.tmp",
        "backups/",
        "agent-plugins/",
    }


def test_every_python_test_has_manifest_disposition() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    covered = {test for contract in manifest.contracts for test in contract.tests}
    assert set(collect_test_nodeids(ROOT)) <= covered | internal_test_nodeids(manifest)
```

- [ ] **Step 2: Run the tests and verify failure before the collector exists**

Run: `uv run pytest tests/compatibility/test_state_inventory.py -v`

Expected: FAIL importing `tools.compatibility.inventory_state`.

- [ ] **Step 3: Implement synthetic config/database collection**

Set `HIERONYMUS_DATA_ROOT` to the provided temporary root before importing config-owning modules. Build the minimal fixture only through current public stores/migrations. Inventory SQLite tables, columns, indexes, triggers, foreign keys, and application migration ledgers from `sqlite_master`/PRAGMA queries. Inventory config keys from typed redacted payloads; represent secrets only as `configured: true|false`.

```python
def sqlite_contract(connection: sqlite3.Connection) -> dict[str, object]:
    return {
        "tables": query_tables(connection),
        "columns": query_columns(connection),
        "indexes": query_indexes(connection),
        "triggers": query_triggers(connection),
        "foreign_keys": query_foreign_keys(connection),
    }
```

Collect tests with `uv run pytest --collect-only -q` and parse only final node-id lines beginning with `tests/`. Do not copy `.pytest_cache`.

- [ ] **Step 4: Generate state snapshot and complete manifest ownership**

Run: `uv run python -m tools.compatibility.inventory_state --write`

Every config field, database object group, installer behavior, generated integration target, and collected test node id receives either a public contract mapping or an `implementation_internal` record with a non-empty reason. Assign the seven accepted technical-owner values; do not use team names or unnamed “migration” ownership.

- [ ] **Step 5: Run state, manifest, and database compatibility tests**

Run: `uv run pytest tests/compatibility/test_state_inventory.py tests/compatibility/test_manifest.py tests/test_db_compatibility.py tests/test_memory_graph_migration.py -v`

Expected: PASS.

- [ ] **Step 6: Commit the state and ownership freeze**

```bash
git add tools/compatibility/inventory_state.py compatibility/snapshots/state.json compatibility/fixtures/config compatibility/fixtures/database compatibility/manifest.json tests/compatibility/test_state_inventory.py
git commit -m "test: freeze data and install contracts"
```

### Task 6: Aggregate Drift Check And Parity Report

**Files:**
- Create: `tools/compatibility/check.py`
- Create: `tests/compatibility/test_check.py`
- Modify: `pyproject.toml`
- Modify: `.github/workflows/pr.yml`
- Create: `compatibility/README.md`

**Interfaces:**
- Consumes: all four snapshot functions and `compatibility/manifest.json`.
- Produces: `uv run python -m tools.compatibility.check`, exit `0` on exact match and exit `1` with a sorted diff/coverage report on drift.

- [ ] **Step 1: Write the failing aggregate-command test**

```python
def test_check_reports_snapshot_drift(tmp_path: Path) -> None:
    repo = copy_contract_tree(tmp_path)
    snapshot = repo / "compatibility/snapshots/cli.json"
    snapshot.write_text('{"scripts":{}}', encoding="utf-8")
    result = run_check(repo)
    assert result.exit_code == 1
    assert "snapshot drift: compatibility/snapshots/cli.json" in result.output
```

- [ ] **Step 2: Run the test and verify failure before command implementation**

Run: `uv run pytest tests/compatibility/test_check.py -v`

Expected: FAIL importing `tools.compatibility.check`.

- [ ] **Step 3: Implement in-memory regeneration and readable reporting**

The check command calls each snapshot function without `--write`, compares canonical `json.dumps(..., ensure_ascii=False, indent=2, sort_keys=True) + "\n"`, validates the manifest, and emits counts derived by surface/disposition/owner. It must not update expected files in check mode.

```python
def main() -> int:
    failures = snapshot_diffs(ROOT) + validate_manifest(load_manifest(MANIFEST), ROOT)
    if failures:
        for failure in sorted(failures):
            print(failure)
        return 1
    print(render_parity_summary(load_manifest(MANIFEST)))
    return 0
```

- [ ] **Step 4: Add the CI gate and contributor instructions**

Document exactly two commands:

```bash
uv run python -m tools.compatibility.check
uv run python -m tools.compatibility.<inventory_module> --write
```

The second command is allowed only together with reviewed manifest/fixture changes and an ADR for changed/removed contracts. CI uploads the textual parity summary as an artifact.

- [ ] **Step 5: Run the complete phase verification**

Run: `uv run python -m tools.compatibility.check`

Run: `uv run pytest`

Run: `uv run ruff check .`

Run: `uv run ruff format --check .`

Expected: compatibility check exits `0`; all tests and Ruff commands pass.

- [ ] **Step 6: Commit the compatibility gate**

```bash
git add tools/compatibility/check.py tests/compatibility/test_check.py pyproject.toml .github compatibility/README.md
git commit -m "ci: gate Rust migration on frozen contracts"
```

## Self-Review Record

- Spec coverage: manifest fields/owners, all named surfaces, immutable fixtures, differential normalization boundary, change control, test mapping, and readable CI reporting each map to Tasks 1–6.
- Placeholder scan: the plan contains no deferred implementation markers; each generated inventory has an exact source, output, and failure gate.
- Type consistency: all tasks use `Manifest`/`Contract` from Task 1; snapshot functions return JSON-compatible dictionaries and check mode never writes.
- Deferred by design: Rust differential execution is not part of this freeze. It begins only after this manifest is reviewed and is owned by the subsequent Rust workspace/contract-harness plan.
