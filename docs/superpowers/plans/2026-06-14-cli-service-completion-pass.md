# CLI Service Completion Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the CLI and service roadmap slice by making command boundaries explicit, improving agent-facing service discovery, and tightening human versus machine output behavior.

**Architecture:** Keep Hieronymus local-first: domain stores remain the source of truth, `--data-root` and `HIERONYMUS_DATA_ROOT` continue to select the data root, and the daemon remains the lifecycle/status surface. Add a small command-boundary catalog so direct SQLite access is deliberate and documented, expose service discovery to agent hooks and MCP status without pretending mutation HTTP endpoints exist, and move legacy automation command JSON behind explicit `--json` flags.

**Tech Stack:** Python 3.12, Click, pytest, local HTTP service state/client modules, SQLite-backed Hieronymus domain stores, Markdown docs.

---

## Current Code Map

- `src/hieronymus/cli.py` owns the human CLI, daemon lifecycle commands, OpenTUI launchers, release/install commands, and older agent/debug commands such as `init-series`, `session-start`, `remember-short`, `recall`, and `feedback`.
- `src/hieronymus/service_manager.py`, `src/hieronymus/service_state.py`, `src/hieronymus/service_client.py`, `src/hieronymus/service_daemon.py`, and `src/hieronymus/service_http.py` implement local daemon discovery, lifecycle, runtime files, health, status, and shutdown.
- `src/hieronymus/agent_hooks.py` is the hook entry point packaged into agent integrations. It currently discovers `.hieronymus.json` project context and emits JSON only when `--json` is passed.
- `src/hieronymus/mcp_server.py` is the stdio MCP adapter. It calls domain stores directly because no service mutation RPC surface exists yet.
- `docs/service-toolkit.md` has basic service docs but does not enumerate direct SQLite commands or explain which surfaces are stable for humans versus scripts.
- `docs/roadmap.md` still lists this CLI and service slice as remaining work.
- `tests/test_cli.py`, `tests/test_cli_service.py`, `tests/test_agent_hooks.py`, and `tests/test_mcp_server.py` cover the current command behavior and will need focused updates.

## Target File Map

- Create `src/hieronymus/cli_boundaries.py`: central catalog of direct-store command names, reasons, and intended consumers.
- Create `src/hieronymus/service_discovery.py`: small helper that reads runtime state, verifies local service health through `ServiceClient`, and returns a JSON-safe status object.
- Modify `src/hieronymus/cli.py`: improve `hiero help`, add `--json` to legacy automation commands, and use boundary metadata for help/docs consistency.
- Modify `src/hieronymus/agent_hooks.py`: include local service discovery in JSON hook payloads while keeping human hook output concise.
- Modify `src/hieronymus/mcp_server.py`: expose a lightweight MCP status tool that reports direct-adapter mode and discovered daemon status without routing mutations through human CLI output.
- Modify `docs/service-toolkit.md`: document command groups, direct SQLite boundary, local-first data-root behavior, examples, and alpha status.
- Modify `docs/roadmap.md`: move the CLI and service bullets into completed baseline after implementation is verified.
- Test in `tests/test_cli_service.py`, `tests/test_cli.py`, `tests/test_agent_hooks.py`, `tests/test_mcp_server.py`, and a new `tests/test_cli_boundaries.py`.

---

### Task 1: Add A Testable CLI Boundary Catalog

**Files:**
- Create: `src/hieronymus/cli_boundaries.py`
- Test: `tests/test_cli_boundaries.py`

- [x] **Step 1: Write the failing boundary tests**

Create `tests/test_cli_boundaries.py`:

```python
from __future__ import annotations

from pathlib import Path

from hieronymus.cli_boundaries import DIRECT_STORE_COMMANDS, DIRECT_STORE_MCP_ADAPTER


def test_direct_store_cli_commands_are_documented() -> None:
    names = {entry.name for entry in DIRECT_STORE_COMMANDS}

    assert names == {
        "init-series",
        "propose-term",
        "validate",
        "remember",
        "session-start",
        "session-complete",
        "remember-short",
        "recall",
        "feedback",
        "dream",
    }
    assert all(entry.reason for entry in DIRECT_STORE_COMMANDS)
    assert all(entry.consumer in {"human-debug", "agent-automation", "maintenance"} for entry in DIRECT_STORE_COMMANDS)


def test_mcp_direct_store_adapter_boundary_is_explicit() -> None:
    assert DIRECT_STORE_MCP_ADAPTER.name == "hieronymus-mcp"
    assert DIRECT_STORE_MCP_ADAPTER.consumer == "agent-automation"
    assert "stdio MCP adapter" in DIRECT_STORE_MCP_ADAPTER.reason
    assert "human CLI output" not in DIRECT_STORE_MCP_ADAPTER.reason


def test_service_toolkit_mentions_every_direct_store_command() -> None:
    docs = Path("docs/service-toolkit.md").read_text(encoding="utf-8")

    for entry in DIRECT_STORE_COMMANDS:
        assert f"`hiero {entry.name}`" in docs
        assert entry.reason in docs
```

- [x] **Step 2: Run tests and confirm the catalog is missing**

Run:

```bash
uv run pytest tests/test_cli_boundaries.py -v
```

Expected: FAIL with `ModuleNotFoundError: No module named 'hieronymus.cli_boundaries'`.

- [x] **Step 3: Implement the boundary catalog**

Create `src/hieronymus/cli_boundaries.py`:

```python
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class DirectStoreCommand:
    name: str
    consumer: str
    reason: str


DIRECT_STORE_COMMANDS: tuple[DirectStoreCommand, ...] = (
    DirectStoreCommand(
        name="init-series",
        consumer="human-debug",
        reason="bootstrap command that creates registry rows before a service mutation API exists",
    ),
    DirectStoreCommand(
        name="propose-term",
        consumer="human-debug",
        reason="legacy termbase helper retained for local debugging of deterministic terminology storage",
    ),
    DirectStoreCommand(
        name="validate",
        consumer="human-debug",
        reason="legacy termbase validator that reads files locally and checks deterministic terminology rules",
    ),
    DirectStoreCommand(
        name="remember",
        consumer="human-debug",
        reason="legacy long-memory helper retained until old memory primitives are fully retired",
    ),
    DirectStoreCommand(
        name="session-start",
        consumer="agent-automation",
        reason="agent workflow primitive that starts local workspace sessions through the domain store",
    ),
    DirectStoreCommand(
        name="session-complete",
        consumer="agent-automation",
        reason="agent workflow primitive that completes local workspace sessions through the domain store",
    ),
    DirectStoreCommand(
        name="remember-short",
        consumer="agent-automation",
        reason="agent workflow primitive that writes short-term observations through the domain store",
    ),
    DirectStoreCommand(
        name="recall",
        consumer="agent-automation",
        reason="agent workflow primitive that combines recall service output without parsing human CLI text",
    ),
    DirectStoreCommand(
        name="feedback",
        consumer="agent-automation",
        reason="agent workflow primitive that records correction events through the feedback store",
    ),
    DirectStoreCommand(
        name="dream",
        consumer="maintenance",
        reason="maintenance command that invokes DreamService directly so local dreaming works without a daemon",
    ),
)

DIRECT_STORE_MCP_ADAPTER = DirectStoreCommand(
    name="hieronymus-mcp",
    consumer="agent-automation",
    reason=(
        "stdio MCP adapter uses Python domain stores directly because the local daemon currently exposes "
        "lifecycle and status HTTP only; it does not parse human CLI output"
    ),
)
```

- [x] **Step 4: Add temporary docs section so the catalog test can pass later**

Do not update `docs/service-toolkit.md` in this task. Leave the docs assertion failing until Task 5 updates the service documentation with the exact command list and reasons.

- [x] **Step 5: Run catalog tests and record expected partial failure**

Run:

```bash
uv run pytest tests/test_cli_boundaries.py -v
```

Expected: the first two tests PASS, and `test_service_toolkit_mentions_every_direct_store_command` FAILS because the docs do not yet mention every command.

- [x] **Step 6: Commit the catalog and tests**

Commit only after Task 5 updates docs and the boundary tests pass. Keep this task's file changes staged for the later docs commit.

---

### Task 2: Add Service Discovery For Agent Adapters

**Files:**
- Create: `src/hieronymus/service_discovery.py`
- Modify: `src/hieronymus/agent_hooks.py`
- Modify: `src/hieronymus/mcp_server.py`
- Test: `tests/test_agent_hooks.py`
- Test: `tests/test_mcp_server.py`

- [x] **Step 1: Write service discovery unit tests through agent hooks**

Append to `tests/test_agent_hooks.py`:

```python
from dataclasses import replace

from hieronymus.config import HieronymusConfig
from hieronymus.service_state import ServerState, write_server_state


def test_hook_session_start_json_includes_missing_service_discovery(tmp_path: Path) -> None:
    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path), "--json"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["service"] == {
        "available": False,
        "mode": "direct-local",
        "reason": "no running local service discovered",
    }


def test_hook_session_start_json_includes_discovered_service(
    tmp_path: Path,
    monkeypatch,
) -> None:
    data_root = tmp_path / "hieronymus"
    config = HieronymusConfig(data_root=data_root)
    state = ServerState(
        pid=123,
        host="127.0.0.1",
        port=8765,
        version="0.2.0",
        started_at="2026-06-14T00:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="secret",
    )
    write_server_state(config, state)
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(data_root))

    def fake_health(self, received_state):
        assert received_state == state
        return {"ok": True, "service": "hieronymus", "version": "0.2.0"}

    monkeypatch.setattr("hieronymus.service_client.ServiceClient.health", fake_health)

    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path), "--json"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["service"] == {
        "available": True,
        "mode": "local-http",
        "base_url": "http://127.0.0.1:8765",
        "pid": 123,
        "version": "0.2.0",
        "data_root": str(data_root),
        "database_path": str(data_root / "hieronymus.sqlite"),
    }
```

- [x] **Step 2: Write MCP status tool test**

Append to `tests/test_mcp_server.py`:

```python
def test_mcp_status_reports_direct_adapter_and_service_discovery(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    monkeypatch.setattr(
        "hieronymus.mcp_server.discover_local_service",
        lambda config: {
            "available": False,
            "mode": "direct-local",
            "reason": "no running local service discovered",
        },
    )

    from hieronymus.mcp_server import hieronymus_status

    assert hieronymus_status() == {
        "adapter": {
            "name": "hieronymus-mcp",
            "mode": "stdio-direct-store",
            "reason": (
                "stdio MCP adapter uses Python domain stores directly because the local daemon currently exposes "
                "lifecycle and status HTTP only; it does not parse human CLI output"
            ),
        },
        "service": {
            "available": False,
            "mode": "direct-local",
            "reason": "no running local service discovered",
        },
        "data_root": str(tmp_path / "hieronymus"),
        "database_path": str(tmp_path / "hieronymus" / "hieronymus.sqlite"),
    }
```

- [x] **Step 3: Run targeted tests and confirm failures**

Run:

```bash
uv run pytest tests/test_agent_hooks.py tests/test_mcp_server.py::test_mcp_status_reports_direct_adapter_and_service_discovery -v
```

Expected: FAIL because `service` is absent from hook payloads and `hieronymus_status` does not exist.

- [x] **Step 4: Implement service discovery helper**

Create `src/hieronymus/service_discovery.py`:

```python
from __future__ import annotations

from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClient, ServiceClientError
from hieronymus.service_state import cleanup_stale_state, read_server_state


def discover_local_service(config: HieronymusConfig) -> dict[str, Any]:
    cleanup_stale_state(config)
    state = read_server_state(config)
    if state is None:
        return {
            "available": False,
            "mode": "direct-local",
            "reason": "no running local service discovered",
        }

    try:
        health = ServiceClient().health(state)
    except ServiceClientError as error:
        return {
            "available": False,
            "mode": "direct-local",
            "reason": f"local service state exists but health check failed: {error}",
        }

    return {
        "available": True,
        "mode": "local-http",
        "base_url": state.base_url,
        "pid": state.pid,
        "version": str(health.get("version", state.version)),
        "data_root": state.data_root,
        "database_path": state.database_path,
    }
```

- [x] **Step 5: Add discovery to agent hook JSON payloads**

Modify `src/hieronymus/agent_hooks.py` imports:

```python
from hieronymus.config import load_config
from hieronymus.service_discovery import discover_local_service
```

Then add discovery in `session_start` after the project context payload is built:

```python
    payload["service"] = discover_local_service(load_config())
```

Do not add this field to non-JSON human output. The existing human output remains:

```python
    click.echo("Hieronymus context loaded" if payload["handled"] else payload["reason"])
```

- [x] **Step 6: Add MCP status tool**

Modify `src/hieronymus/mcp_server.py` imports:

```python
from hieronymus.cli_boundaries import DIRECT_STORE_MCP_ADAPTER
from hieronymus.service_discovery import discover_local_service
```

Add this tool near the first MCP tools:

```python
@server.tool()
def hieronymus_status() -> dict[str, Any]:
    """Report MCP adapter mode and discovered local service status."""
    config = _load_validated_config()
    return {
        "adapter": {
            "name": DIRECT_STORE_MCP_ADAPTER.name,
            "mode": "stdio-direct-store",
            "reason": DIRECT_STORE_MCP_ADAPTER.reason,
        },
        "service": discover_local_service(config),
        "data_root": str(config.data_root),
        "database_path": str(config.database_path),
    }
```

- [x] **Step 7: Run targeted tests**

Run:

```bash
uv run pytest tests/test_agent_hooks.py tests/test_mcp_server.py::test_mcp_status_reports_direct_adapter_and_service_discovery tests/test_mcp_server.py::test_mcp_server_registers_expected_tool_names -v
```

Expected: PASS after adding `hieronymus_status` to the expected MCP tool-name list if that test enumerates every tool.

- [x] **Step 8: Commit service discovery changes**

Run:

```bash
git add src/hieronymus/service_discovery.py src/hieronymus/agent_hooks.py src/hieronymus/mcp_server.py tests/test_agent_hooks.py tests/test_mcp_server.py
git commit -m "feat: expose local service discovery to agent adapters"
```

---

### Task 3: Put Legacy Automation JSON Behind `--json`

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `tests/test_cli.py`

- [x] **Step 1: Update CLI tests to request JSON explicitly**

In `tests/test_cli.py`, update invocations that parse JSON from these commands to include `--json`:

```python
"init-series", "only-sense-online", "--title", "Only Sense Online", "--json"
```

```python
"session-start", "only-sense-online", "--task-type", "translation", "--json"
```

```python
"session-complete", str(session_id), "--json"
```

```python
"remember-short", str(session_id), "--role", "user", "--kind", "correction", "--text", "Use honorifics.", "--json"
```

```python
"recall", str(session_id), "--series", "only-sense-online", "--query", "honorifics", "--source-language", "ja", "--target-language", "en", "--task-type", "translation", "--json"
```

```python
"feedback", str(crystal_id), "--event", "helpful", "--role", "user", "--json"
```

Apply the same explicit `--json` pattern to `propose-term`, `validate`, and `remember` tests if they parse JSON.

- [x] **Step 2: Add human-output tests for the same commands**

Append focused tests to `tests/test_cli.py`:

```python
def test_init_series_human_output_is_not_json(tmp_path):
    data_root = tmp_path / "hieronymus"

    result = CliRunner().invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "init-series",
            "oso",
            "--title",
            "Only Sense Online",
        ],
    )

    assert result.exit_code == 0
    assert result.output == f"Series oso initialized at {data_root / 'hieronymus.sqlite'}\n"


def test_session_start_human_output_is_not_json(tmp_path):
    data_root = tmp_path / "hieronymus"
    runner = CliRunner()
    _create_series(data_root)

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "session-start",
            "only-sense-online",
            "--task-type",
            "translation",
        ],
    )

    assert result.exit_code == 0
    assert result.output.startswith("Session ")
    assert result.output.endswith(" started\n")
    assert result.output.strip().startswith("{") is False


def test_recall_human_output_is_not_json(tmp_path):
    data_root = tmp_path / "hieronymus"
    runner = CliRunner()
    session_id = _start_session(runner, data_root)

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "recall",
            str(session_id),
            "--series",
            "only-sense-online",
            "--query",
            "anything",
            "--source-language",
            "ja",
            "--target-language",
            "en",
            "--task-type",
            "translation",
        ],
    )

    assert result.exit_code == 0
    assert result.output == "No recall results.\n"
```

- [x] **Step 3: Run tests and confirm failures**

Run:

```bash
uv run pytest tests/test_cli.py -v
```

Expected: FAIL because the legacy commands do not accept `--json` yet and still print JSON by default.

- [x] **Step 4: Add helper output functions in `src/hieronymus/cli.py`**

Near the existing payload helpers, add:

```python
def _echo_json_or_line(payload: object, *, json_output: bool, line: str) -> None:
    if json_output:
        click.echo(render_json(payload))
        return
    click.echo(line)
```

Use `render_json` rather than `json.dumps` so all machine output uses the same formatting path.

- [x] **Step 5: Add `--json` flags and human output**

For each command below, add `@click.option("--json", "json_output", is_flag=True)` before `@click.pass_context`, add `json_output: bool` to the function signature, build the existing payload, and call `_echo_json_or_line`.

`init-series`:

```python
    payload = {"slug": series.slug, "database_path": str(ctx.obj["config"].database_path)}
    _echo_json_or_line(
        payload,
        json_output=json_output,
        line=f"Series {series.slug} initialized at {ctx.obj['config'].database_path}",
    )
```

`propose-term`:

```python
    payload = {"term_id": term_id}
    _echo_json_or_line(payload, json_output=json_output, line=f"Term proposal {term_id} created")
```

`validate`:

```python
    payload = [asdict(finding) for finding in findings]
    _echo_json_or_line(
        payload,
        json_output=json_output,
        line="No terminology findings." if not payload else f"{len(payload)} terminology finding(s).",
    )
```

`remember`:

```python
    payload = {"memory_id": memory_id}
    _echo_json_or_line(payload, json_output=json_output, line=f"Memory {memory_id} stored")
```

`session-start`:

```python
    payload = {"session_id": session.id}
    _echo_json_or_line(payload, json_output=json_output, line=f"Session {session.id} started")
```

`session-complete`:

```python
    payload = {"session_id": session_id, "completed": True}
    _echo_json_or_line(payload, json_output=json_output, line=f"Session {session_id} completed")
```

`remember-short`:

```python
    payload = {"memory_id": memory_id}
    _echo_json_or_line(payload, json_output=json_output, line=f"Short-term memory {memory_id} stored")
```

`recall`:

```python
    payload = [
        {
            "source": result.source,
            "rank": result.rank,
            "score": result.score,
            "reason": result.reason,
            "crystal": _crystal_payload(result.crystal),
            "short_term_memory": _short_term_memory_payload(result.short_term_memory),
        }
        for result in results
    ]
    _echo_json_or_line(
        payload,
        json_output=json_output,
        line="No recall results." if not payload else f"{len(payload)} recall result(s).",
    )
```

`feedback`:

```python
    payload = {"event_id": event_id}
    _echo_json_or_line(payload, json_output=json_output, line=f"Feedback event {event_id} recorded")
```

- [x] **Step 6: Preserve console alias behavior**

Update `tests/test_cli_service.py::test_hiero_console_alias_runs_existing_command` to pass `--json` to `init-series`, because that test parses stdout:

```python
"--title",
"Only Sense Online",
"--json",
```

- [x] **Step 7: Run targeted tests**

Run:

```bash
uv run pytest tests/test_cli.py tests/test_cli_service.py::test_hiero_console_alias_runs_existing_command -v
```

Expected: PASS.

- [x] **Step 8: Commit JSON gating changes**

Run:

```bash
git add src/hieronymus/cli.py tests/test_cli.py tests/test_cli_service.py
git commit -m "feat: require json flag for automation cli payloads"
```

---

### Task 4: Improve CLI Help Grouping And Alpha Language

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `tests/test_cli_service.py`

- [x] **Step 1: Replace brittle help assertions with grouped-help expectations**

Update `tests/test_cli_service.py::test_cli_help_mentions_service_commands`:

```python
def test_cli_help_mentions_service_commands() -> None:
    result = CliRunner().invoke(main, ["help"])

    assert result.exit_code == 0
    assert "Hieronymus v0.2.0α" in result.output
    assert "Alpha software: local-first, usable at your own risk." in result.output
    assert "Service" in result.output
    assert "Management" in result.output
    assert "Agent and automation" in result.output
    assert "Maintenance" in result.output
    assert "Examples" in result.output
    assert "hiero status --json" in result.output
    assert "hiero session-start oso --task-type translation --json" in result.output
    assert "hiero recall 1 --series oso --query \"style\" --source-language ja --target-language en --task-type translation --json" in result.output
    assert "Open the memory management TUI" not in result.output
    assert "Show config paths" not in result.output
```

- [x] **Step 2: Run the help test and confirm failure**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_cli_help_mentions_service_commands -v
```

Expected: FAIL because the current custom help lacks alpha risk language, groups, and examples.

- [x] **Step 3: Implement grouped custom help**

Replace `help_command()` in `src/hieronymus/cli.py` with:

```python
@main.command("help")
def help_command() -> None:
    click.echo(render_greeting())
    click.echo("Alpha software: local-first, usable at your own risk.")
    click.echo()
    click.echo(f"{GUIDE_ICON} Service")
    click.echo("  hiero                  Start or connect to the local service")
    click.echo("  hiero status           Show daemon and provider status")
    click.echo("  hiero status --json    Emit daemon and provider status for scripts")
    click.echo("  hiero stop             Request graceful daemon shutdown")
    click.echo("  hiero restart          Restart the local daemon")
    click.echo()
    click.echo(f"{GUIDE_ICON} Management")
    click.echo("  hiero config           Open the configuration TUI")
    click.echo("  hiero config --json    Emit config, provider, dreaming, ingest, and release state")
    click.echo("  hiero admin            Open the local management TUI")
    click.echo("  hiero admin --json     Emit management counts and available views")
    click.echo("  hiero doctor           Check configuration and service health")
    click.echo()
    click.echo(f"{GUIDE_ICON} Agent and automation")
    click.echo("  hiero session-start <series> --task-type <type> --json")
    click.echo("  hiero remember-short <session-id> --role user --kind correction --text <text> --json")
    click.echo("  hiero recall <session-id> --series <series> --query <query> --source-language <src> --target-language <dst> --task-type <type> --json")
    click.echo("  hiero feedback <crystal-id> --event helpful --role user --json")
    click.echo()
    click.echo(f"{GUIDE_ICON} Maintenance")
    click.echo("  hiero install codex --dry-run")
    click.echo("  hiero update           Update managed installs in place")
    click.echo("  hiero dream --json     Run local dreaming and emit machine-readable status")
    click.echo()
    click.echo(f"{GUIDE_ICON} Examples")
    click.echo("  hiero status --json")
    click.echo("  hiero session-start oso --task-type translation --json")
    click.echo('  hiero recall 1 --series oso --query "style" --source-language ja --target-language en --task-type translation --json')
```

- [x] **Step 4: Run the help test**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_cli_help_mentions_service_commands -v
```

Expected: PASS.

- [x] **Step 5: Commit help changes**

Run:

```bash
git add src/hieronymus/cli.py tests/test_cli_service.py
git commit -m "docs: improve cli help groups and alpha warning"
```

---

### Task 5: Update Service Documentation And Roadmap

**Files:**
- Modify: `docs/service-toolkit.md`
- Modify: `docs/roadmap.md`
- Test: `tests/test_cli_boundaries.py`
- Test: `tests/test_cli_service.py`

- [ ] **Step 1: Replace `docs/service-toolkit.md` with complete service docs**

Rewrite the relevant sections of `docs/service-toolkit.md` so it contains this structure and exact command boundary lines:

```markdown
# Hieronymus Service Toolkit

Hieronymus is alpha local-first software. It can be used, but users should treat data and workflows as pre-release and keep their own backups.

Hieronymus installs two equivalent console commands:

- `hieronymus`
- `hiero`

Every subcommand works through either command. For example, `hieronymus status --json` and `hiero status --json` call the same CLI entry point.

Running `hiero` with no subcommand starts the local daemon if it is not already running, then prints a short status surface.

The daemon is the normal lifecycle and status surface for the global SQLite store. It exposes local HTTP JSON endpoints on `127.0.0.1` for health, status, and shutdown. It does not yet expose mutation endpoints for every domain primitive, so some CLI commands and the stdio MCP adapter intentionally call Python domain stores directly.

## Data Root And Runtime Files

Hieronymus remains local-first. `--data-root` and `HIERONYMUS_DATA_ROOT` select the configured root. The default root is `~/.config/hieronymus`.

The selected root contains:

- `hieronymus.sqlite`
- `dream.conf`
- `ingest.conf`
- `release.conf`
- `server.json`
- `server.pid`
- `server.lock`

Components must not discover service state by parsing human CLI output. Use runtime files and JSON APIs instead.

## Human Output And JSON Output

Human commands may print prose, grouped help, and the Hieronymus identity line. Machine-readable payloads are behind `--json`.

Automation should prefer:

- `hiero status --json`
- `hiero config --json`
- `hiero admin --json`
- `hiero session-start <series> --task-type <type> --json`
- `hiero remember-short <session-id> --role user --kind correction --text <text> --json`
- `hiero recall <session-id> --series <series> --query <query> --source-language <src> --target-language <dst> --task-type <type> --json`
- `hiero feedback <crystal-id> --event helpful --role user --json`
- `hiero dream --json`

## Direct SQLite Command Boundary

These commands intentionally access SQLite through Python domain stores in this pass:

- `hiero init-series`: bootstrap command that creates registry rows before a service mutation API exists
- `hiero propose-term`: legacy termbase helper retained for local debugging of deterministic terminology storage
- `hiero validate`: legacy termbase validator that reads files locally and checks deterministic terminology rules
- `hiero remember`: legacy long-memory helper retained until old memory primitives are fully retired
- `hiero session-start`: agent workflow primitive that starts local workspace sessions through the domain store
- `hiero session-complete`: agent workflow primitive that completes local workspace sessions through the domain store
- `hiero remember-short`: agent workflow primitive that writes short-term observations through the domain store
- `hiero recall`: agent workflow primitive that combines recall service output without parsing human CLI text
- `hiero feedback`: agent workflow primitive that records correction events through the feedback store
- `hiero dream`: maintenance command that invokes DreamService directly so local dreaming works without a daemon

The `hieronymus-mcp` stdio MCP adapter uses Python domain stores directly because the local daemon currently exposes lifecycle and status HTTP only; it does not parse human CLI output.

## Service Discovery

Agent adapters should discover the local daemon from the configured data root. The runtime state is stored in `server.json` and guarded by `server.pid` and `server.lock`.

When a daemon is healthy, discovery reports `mode: local-http`. When no healthy daemon is found, adapters may continue in `mode: direct-local` if the operation is local-first and uses the configured data root.

## Command Groups

Service:

- `hiero`
- `hiero status`
- `hiero status --json`
- `hiero stop`
- `hiero restart`

Management:

- `hiero config`
- `hiero config --json`
- `hiero admin`
- `hiero admin --json`
- `hiero doctor`

Agent and automation:

- `hiero session-start <series> --task-type <type> --json`
- `hiero session-complete <session-id> --json`
- `hiero remember-short <session-id> --role <role> --kind <kind> --text <text> --json`
- `hiero recall <session-id> --series <series> --query <query> --source-language <src> --target-language <dst> --task-type <type> --json`
- `hiero feedback <crystal-id> --event <event> --role <role> --json`

Maintenance:

- `hiero install <app> --dry-run`
- `hiero update`
- `hiero dream --json`
```

- [ ] **Step 2: Move roadmap CLI and service items to completed baseline**

In `docs/roadmap.md`, replace the CLI And Service section with:

```markdown
### CLI And Service

The CLI is both a human tool and an automation surface. Human output should be
clear, while JSON output remains stable for scripts.

Completed baseline:

- Commands that intentionally access SQLite through Python domain stores are
  documented with command-level reasons.
- Agent-facing hook and MCP adapter status paths report local service discovery
  from configured runtime files instead of parsing human CLI output.
- Local-first operation and existing `--data-root` / `HIERONYMUS_DATA_ROOT`
  behavior remain the data-root selection contract.
- CLI help has clearer command grouping, examples, and alpha status language.
- Machine-readable legacy automation command output is behind `--json`.

Remaining work:

- No active roadmap items in this section.
```

- [ ] **Step 3: Run docs and boundary tests**

Run:

```bash
uv run pytest tests/test_cli_boundaries.py tests/test_cli_service.py::test_cli_help_mentions_service_commands -v
```

Expected: PASS.

- [ ] **Step 4: Commit docs**

Run:

```bash
git add docs/service-toolkit.md docs/roadmap.md tests/test_cli_boundaries.py src/hieronymus/cli_boundaries.py
git commit -m "docs: document cli service boundaries"
```

---

### Task 6: Verify Full Backend Quality

**Files:**
- No new files unless verification reveals a still-valid issue.

- [ ] **Step 1: Run backend tests**

Run:

```bash
uv run pytest
```

Expected: PASS.

- [ ] **Step 2: Run Ruff lint**

Run:

```bash
uv run ruff check .
```

Expected: PASS.

- [ ] **Step 3: Run Ruff formatting check**

Run:

```bash
uv run ruff format --check .
```

Expected: PASS.

- [ ] **Step 4: Check git status**

Run:

```bash
git status --short --branch
```

Expected: branch `plan/cli-service-completion-pass` with no unstaged changes after commits.

---

### Task 7: Push And Open PR

**Files:**
- No source changes.

- [ ] **Step 1: Push the branch**

Run:

```bash
git push -u origin plan/cli-service-completion-pass
```

Expected: branch pushed to GitHub.

- [ ] **Step 2: Open PR**

Run:

```bash
gh pr create \
  --title "Complete CLI and service roadmap slice" \
  --body "## Summary
- documents direct SQLite command boundaries and service discovery behavior
- adds local service discovery to agent-facing status paths
- gates legacy automation CLI JSON behind --json and improves help grouping
- updates roadmap after verification

## Tests
- uv run pytest
- uv run ruff check .
- uv run ruff format --check ."
```

Expected: PR URL printed.

---

## Self-Review

- Spec coverage: direct SQLite commands are cataloged and documented in Tasks 1 and 5; service discovery is added in Task 2; `--data-root` remains unchanged and is documented in Task 5; CLI help is improved in Task 4; JSON-only machine output is handled in Task 3.
- Placeholder scan: this plan contains concrete steps, commands, and test bodies without open-ended filler.
- Type consistency: `DirectStoreCommand`, `discover_local_service`, and `hieronymus_status` are defined before later tasks use them, and all payload keys match the tests shown in the plan.
