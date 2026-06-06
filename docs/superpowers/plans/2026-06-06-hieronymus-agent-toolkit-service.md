# Hieronymus Agent Toolkit Service Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Python service/toolkit layer for Hieronymus: `hieronymus`/`hiero` aliases, one local daemon, thin lifecycle CLI commands, doctor checks, and a safe installer framework with stubs for future agent integrations.

**Architecture:** Keep memory ownership inside one local Python daemon that exposes a small `127.0.0.1` HTTP JSON API. CLI commands discover daemon state in `~/.config/hieronymus`, start or stop the daemon through a service manager, and render either human-friendly output or stable JSON. Agent installs are represented by a shared target registry and safe plan/install result model; real host-specific integrations remain separate follow-up work.

**Tech Stack:** Python 3.12+, uv, Click, stdlib `http.server`, stdlib `urllib.request`, SQLite-backed existing services, pytest, ruff.

---

## Scope Rules

- Work in `/home/inky/Development/hieronymus/.worktrees/agent-toolkit-service`.
- Focus only on `docs/superpowers/specs/2026-06-06-hieronymus-agent-toolkit-service-design.md`.
- Keep global config and runtime state under `~/.config/hieronymus` by default via the existing `load_config()` behavior.
- Do not implement real Claude/Codex/OpenClaw/opencode/Gemini hooks, skills, lifecycle event mappings, or host-specific config patches in this pass.
- `hiero install <app>` may return honest stubs and dry-run plans for supported targets.
- Do not add systemd, launchd, Windows service integration, Node runtime code, or npm packaging.
- Keep existing memory/dreaming/termbase CLI commands functional.

## Target File Structure

```text
src/hieronymus/
├── cli.py                    # Existing Click entry point; add lifecycle/toolkit commands.
├── config.py                 # Existing global config; add config/runtime helper paths.
├── doctor.py                 # Doctor finding model and checks.
├── install.py                # Agent install target registry, plans, result types, safe helpers.
├── presentation.py           # Human greeting/status rendering and JSON helpers.
├── service_client.py         # Thin HTTP client for the daemon API.
├── service_daemon.py         # `python -m hieronymus.service_daemon` entry point.
├── service_http.py           # ThreadingHTTPServer app and route handlers.
├── service_manager.py        # Ensure/start/stop/restart/status orchestration.
└── service_state.py          # Runtime state files, pid checks, stale cleanup, port selection.
tests/
├── test_cli_service.py       # CLI lifecycle, alias, greeting, JSON behavior.
├── test_doctor.py            # Doctor finding/check behavior.
├── test_install.py           # Installer target registry, dry-run, stubs, safe writes.
├── test_service_http.py      # HTTP route contract.
├── test_service_manager.py   # Idempotent service manager behavior.
└── test_service_state.py     # Runtime state, pid, stale cleanup, port allocation.
docs/
├── service-toolkit.md        # User-facing service/toolkit notes.
└── usage.md                  # Add lifecycle command examples.
```

## Task 1: Add Console Alias and Presentation Helpers

**Files:**
- Modify: `pyproject.toml`
- Create: `src/hieronymus/presentation.py`
- Modify: `tests/test_cli.py`
- Create: `tests/test_cli_service.py`

- [ ] **Step 1: Write failing tests for the `hiero` alias and greeting**

Create `tests/test_cli_service.py`:

```python
from __future__ import annotations

import json
import subprocess
from pathlib import Path

from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus.presentation import GREETING_ICON, render_greeting


def test_render_greeting_contains_identity_and_tagline() -> None:
    rendered = render_greeting("0.1.0")

    assert rendered == f"{GREETING_ICON} Hieronymus v0.1.0\nRemembers things for you."


def test_hiero_console_alias_runs_existing_command(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"

    result = subprocess.run(
        [
            "uv",
            "run",
            "hiero",
            "--data-root",
            str(data_root),
            "init-series",
            "oso",
            "--title",
            "Only Sense Online",
        ],
        check=False,
        cwd=Path.cwd(),
        text=True,
        capture_output=True,
    )

    assert result.returncode == 0
    assert json.loads(result.stdout) == {
        "slug": "oso",
        "database_path": str(data_root / "hieronymus.sqlite"),
    }


def test_cli_help_mentions_service_commands() -> None:
    result = CliRunner().invoke(main, ["help"])

    assert result.exit_code == 0
    assert "hiero status" in result.output
    assert "hiero install codex --dry-run" in result.output
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_render_greeting_contains_identity_and_tagline tests/test_cli_service.py::test_hiero_console_alias_runs_existing_command tests/test_cli_service.py::test_cli_help_mentions_service_commands -v
```

Expected:

```text
FAILED tests/test_cli_service.py::test_render_greeting_contains_identity_and_tagline
FAILED tests/test_cli_service.py::test_hiero_console_alias_runs_existing_command
FAILED tests/test_cli_service.py::test_cli_help_mentions_service_commands
```

- [ ] **Step 3: Add the `hiero` console script**

Modify `[project.scripts]` in `pyproject.toml`:

```toml
[project.scripts]
hieronymus = "hieronymus.cli:main"
hiero = "hieronymus.cli:main"
hieronymus-mcp = "hieronymus.mcp_server:main"
```

- [ ] **Step 4: Add presentation helpers**

Create `src/hieronymus/presentation.py`:

```python
from __future__ import annotations

import json
from importlib.metadata import PackageNotFoundError, version
from typing import Any

GREETING_ICON = "🪶"
STATUS_ICON = "📜"
GUIDE_ICON = "📖"
TAGLINE = "Remembers things for you."


def package_version() -> str:
    try:
        return version("hieronymus")
    except PackageNotFoundError:
        return "0.1.0"


def render_greeting(app_version: str | None = None) -> str:
    resolved_version = app_version if app_version is not None else package_version()
    return f"{GREETING_ICON} Hieronymus v{resolved_version}\n{TAGLINE}"


def render_json(payload: dict[str, Any] | list[Any]) -> str:
    return json.dumps(payload, ensure_ascii=False, sort_keys=True)


def render_pretty_json(payload: dict[str, Any] | list[Any]) -> str:
    return json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True)
```

- [ ] **Step 5: Add `hiero help` command**

In `src/hieronymus/cli.py`, import the presentation helper:

```python
from hieronymus.presentation import GUIDE_ICON, render_greeting
```

Add this command near the top-level Click commands:

```python
@main.command("help")
def help_command() -> None:
    click.echo(render_greeting())
    click.echo()
    click.echo(f"{GUIDE_ICON} Common commands")
    click.echo("  hiero                  Start or connect to the local service")
    click.echo("  hiero status           Show daemon and provider status")
    click.echo("  hiero doctor           Check configuration and service health")
    click.echo("  hiero restart          Restart the local daemon")
    click.echo("  hiero admin            Open the memory management TUI")
    click.echo("  hiero config           Open the configuration TUI")
    click.echo("  hiero install codex --dry-run")
```

- [ ] **Step 6: Run targeted tests and verify they pass**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_render_greeting_contains_identity_and_tagline tests/test_cli_service.py::test_hiero_console_alias_runs_existing_command tests/test_cli_service.py::test_cli_help_mentions_service_commands -v
```

Expected:

```text
3 passed
```

- [ ] **Step 7: Commit**

```bash
git add pyproject.toml src/hieronymus/presentation.py src/hieronymus/cli.py tests/test_cli_service.py
git commit -m "feat: add hiero alias and CLI presentation"
```

## Task 2: Add Runtime State and Single-Instance Primitives

**Files:**
- Modify: `src/hieronymus/config.py`
- Create: `src/hieronymus/service_state.py`
- Create: `tests/test_service_state.py`

- [ ] **Step 1: Write failing runtime state tests**

Create `tests/test_service_state.py`:

```python
from __future__ import annotations

import os
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.service_state import (
    ServerState,
    allocate_loopback_port,
    cleanup_stale_state,
    is_pid_running,
    read_server_state,
    runtime_paths,
    write_server_state,
)


def test_runtime_paths_stay_under_config_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    paths = runtime_paths(config)

    assert paths.config_root == tmp_path / "hieronymus"
    assert paths.server_json == tmp_path / "hieronymus" / "server.json"
    assert paths.server_pid == tmp_path / "hieronymus" / "server.pid"
    assert paths.server_lock == tmp_path / "hieronymus" / "server.lock"


def test_server_state_round_trips_as_json(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=12345,
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )

    write_server_state(config, state)

    assert read_server_state(config) == state


def test_cleanup_stale_state_removes_dead_pid_files(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=99999999,
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )
    paths = runtime_paths(config)
    write_server_state(config, state)
    paths.server_lock.write_text("99999999", encoding="utf-8")

    removed = cleanup_stale_state(config)

    assert removed is True
    assert read_server_state(config) is None
    assert not paths.server_pid.exists()
    assert not paths.server_lock.exists()


def test_current_process_pid_is_running() -> None:
    assert is_pid_running(os.getpid()) is True


def test_allocate_loopback_port_returns_connectable_port_number() -> None:
    port = allocate_loopback_port()

    assert 1024 < port < 65536
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_service_state.py -v
```

Expected:

```text
FAILED tests/test_service_state.py
```

- [ ] **Step 3: Add config runtime helpers**

Modify `src/hieronymus/config.py`:

```python
@dataclass(frozen=True)
class HieronymusConfig:
    data_root: Path

    @property
    def database_path(self) -> Path:
        return self.data_root / "hieronymus.sqlite"

    @property
    def config_root(self) -> Path:
        return self.data_root

    @property
    def backups_root(self) -> Path:
        return self.config_root / "backups"
```

- [ ] **Step 4: Implement runtime state helpers**

Create `src/hieronymus/service_state.py`:

```python
from __future__ import annotations

import json
import os
import socket
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

from hieronymus.config import HieronymusConfig


@dataclass(frozen=True)
class RuntimePaths:
    config_root: Path
    server_json: Path
    server_pid: Path
    server_lock: Path


@dataclass(frozen=True)
class ServerState:
    pid: int
    host: str
    port: int
    version: str
    started_at: str
    data_root: str
    database_path: str
    token: str

    @property
    def base_url(self) -> str:
        return f"http://{self.host}:{self.port}"

    def to_json_dict(self) -> dict[str, Any]:
        return asdict(self)

    @classmethod
    def from_json_dict(cls, payload: dict[str, Any]) -> ServerState:
        return cls(
            pid=int(payload["pid"]),
            host=str(payload["host"]),
            port=int(payload["port"]),
            version=str(payload["version"]),
            started_at=str(payload["started_at"]),
            data_root=str(payload["data_root"]),
            database_path=str(payload["database_path"]),
            token=str(payload["token"]),
        )


def runtime_paths(config: HieronymusConfig) -> RuntimePaths:
    root = config.config_root
    return RuntimePaths(
        config_root=root,
        server_json=root / "server.json",
        server_pid=root / "server.pid",
        server_lock=root / "server.lock",
    )


def is_pid_running(pid: int) -> bool:
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def read_server_state(config: HieronymusConfig) -> ServerState | None:
    path = runtime_paths(config).server_json
    if not path.exists():
        return None
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    try:
        return ServerState.from_json_dict(payload)
    except (KeyError, TypeError, ValueError):
        return None


def write_server_state(config: HieronymusConfig, state: ServerState) -> None:
    paths = runtime_paths(config)
    paths.config_root.mkdir(parents=True, exist_ok=True)
    tmp = paths.server_json.with_name(f"{paths.server_json.name}.tmp-{os.getpid()}")
    tmp.write_text(json.dumps(state.to_json_dict(), ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    tmp.replace(paths.server_json)
    paths.server_pid.write_text(f"{state.pid}\n", encoding="utf-8")


def remove_server_state(config: HieronymusConfig) -> None:
    paths = runtime_paths(config)
    for path in (paths.server_json, paths.server_pid, paths.server_lock):
        try:
            path.unlink()
        except FileNotFoundError:
            pass


def cleanup_stale_state(config: HieronymusConfig) -> bool:
    state = read_server_state(config)
    if state is None:
        return False
    if is_pid_running(state.pid):
        return False
    remove_server_state(config)
    return True


def allocate_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])
```

- [ ] **Step 5: Run targeted tests**

Run:

```bash
uv run pytest tests/test_service_state.py -v
```

Expected:

```text
5 passed
```

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/config.py src/hieronymus/service_state.py tests/test_service_state.py
git commit -m "feat: add service runtime state helpers"
```

## Task 3: Add Local HTTP Daemon Contract

**Files:**
- Create: `src/hieronymus/service_http.py`
- Create: `src/hieronymus/service_daemon.py`
- Create: `tests/test_service_http.py`

- [ ] **Step 1: Write failing HTTP contract tests**

Create `tests/test_service_http.py`:

```python
from __future__ import annotations

import json
import threading
import urllib.request
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.service_http import HieronymusHTTPServer, build_server
from hieronymus.service_state import ServerState, allocate_loopback_port


def _read_json(url: str) -> dict[str, object]:
    with urllib.request.urlopen(url, timeout=2) as response:
        return json.loads(response.read().decode("utf-8"))


def test_health_endpoint_returns_daemon_identity(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=12345,
        host="127.0.0.1",
        port=allocate_loopback_port(),
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )
    server = build_server(config, state)
    assert isinstance(server, HieronymusHTTPServer)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        payload = _read_json(f"http://127.0.0.1:{state.port}/health")
    finally:
        server.shutdown()
        thread.join(timeout=2)
        server.server_close()

    assert payload["ok"] is True
    assert payload["service"] == "hieronymus"
    assert payload["version"] == "0.1.0"


def test_status_endpoint_returns_paths_and_pid(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=12345,
        host="127.0.0.1",
        port=allocate_loopback_port(),
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )
    server = build_server(config, state)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        payload = _read_json(f"http://127.0.0.1:{state.port}/status")
    finally:
        server.shutdown()
        thread.join(timeout=2)
        server.server_close()

    assert payload["running"] is True
    assert payload["pid"] == 12345
    assert payload["data_root"] == str(config.data_root)
    assert payload["database_path"] == str(config.database_path)
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_service_http.py -v
```

Expected:

```text
FAILED tests/test_service_http.py
```

- [ ] **Step 3: Implement HTTP server**

Create `src/hieronymus/service_http.py`:

```python
from __future__ import annotations

import json
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.service_state import ServerState


class HieronymusHTTPServer(ThreadingHTTPServer):
    def __init__(
        self,
        server_address: tuple[str, int],
        config: HieronymusConfig,
        state: ServerState,
    ) -> None:
        super().__init__(server_address, HieronymusRequestHandler)
        self.config = config
        self.state = state


class HieronymusRequestHandler(BaseHTTPRequestHandler):
    server: HieronymusHTTPServer

    def log_message(self, format: str, *args: object) -> None:
        return

    def do_GET(self) -> None:
        if self.path == "/health":
            self._send_json(
                {
                    "ok": True,
                    "service": "hieronymus",
                    "version": self.server.state.version,
                }
            )
            return
        if self.path == "/status":
            self._send_json(status_payload(self.server.config, self.server.state))
            return
        self._send_json({"error": "not_found", "path": self.path}, status=HTTPStatus.NOT_FOUND)

    def do_POST(self) -> None:
        if self.path == "/shutdown":
            self._send_json({"ok": True, "stopping": True})
            self.server.shutdown()
            return
        self._send_json({"error": "not_found", "path": self.path}, status=HTTPStatus.NOT_FOUND)

    def _send_json(self, payload: dict[str, Any], status: HTTPStatus = HTTPStatus.OK) -> None:
        body = json.dumps(payload, ensure_ascii=False, sort_keys=True).encode("utf-8")
        self.send_response(status.value)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def status_payload(config: HieronymusConfig, state: ServerState) -> dict[str, Any]:
    return {
        "running": True,
        "pid": state.pid,
        "host": state.host,
        "port": state.port,
        "version": state.version,
        "started_at": state.started_at,
        "data_root": str(config.data_root),
        "database_path": str(config.database_path),
        "config_path": str(config.config_root),
        "providers": [],
        "mcp_adapter": {"available": True, "mode": "local-http"},
        "housekeeping": {"last_cycle": None, "pending": False},
    }


def build_server(config: HieronymusConfig, state: ServerState) -> HieronymusHTTPServer:
    return HieronymusHTTPServer((state.host, state.port), config, state)
```

- [ ] **Step 4: Implement daemon module**

Create `src/hieronymus/service_daemon.py`:

```python
from __future__ import annotations

import argparse
import os
import secrets
from datetime import UTC, datetime

from hieronymus.config import load_config
from hieronymus.presentation import package_version
from hieronymus.service_http import build_server
from hieronymus.service_state import ServerState, allocate_loopback_port, remove_server_state, write_server_state


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="python -m hieronymus.service_daemon")
    parser.add_argument("--data-root", default=None)
    parser.add_argument("--port", type=int, default=0)
    return parser


def main(argv: list[str] | None = None) -> None:
    args = build_parser().parse_args(argv)
    config = load_config(args.data_root)
    config.data_root.mkdir(parents=True, exist_ok=True)
    port = args.port if args.port > 0 else allocate_loopback_port()
    state = ServerState(
        pid=os.getpid(),
        host="127.0.0.1",
        port=port,
        version=package_version(),
        started_at=datetime.now(UTC).isoformat(),
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token=secrets.token_hex(16),
    )
    write_server_state(config, state)
    server = build_server(config, state)
    try:
        server.serve_forever()
    finally:
        server.server_close()
        remove_server_state(config)


if __name__ == "__main__":
    main()
```

During implementation, replace the intentionally direct PID expression with this cleaner import at the top:

```python
import os
```

and set:

```python
pid=os.getpid(),
```

- [ ] **Step 5: Run HTTP tests**

Run:

```bash
uv run pytest tests/test_service_http.py -v
```

Expected:

```text
2 passed
```

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/service_http.py src/hieronymus/service_daemon.py tests/test_service_http.py
git commit -m "feat: add local service HTTP daemon"
```

## Task 4: Add Service Client and Manager

**Files:**
- Create: `src/hieronymus/service_client.py`
- Create: `src/hieronymus/service_manager.py`
- Create: `tests/test_service_manager.py`

- [ ] **Step 1: Write failing manager tests**

Create `tests/test_service_manager.py`:

```python
from __future__ import annotations

import json
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.service_manager import ServiceManager
from hieronymus.service_state import ServerState, read_server_state, write_server_state


class FakeClient:
    def __init__(self, healthy: bool) -> None:
        self.healthy = healthy
        self.shutdown_called = False

    def health(self, state: ServerState) -> dict[str, object]:
        if not self.healthy:
            raise OSError("connection refused")
        return {"ok": True, "service": "hieronymus"}

    def status(self, state: ServerState) -> dict[str, object]:
        return {"running": True, "pid": state.pid}

    def shutdown(self, state: ServerState) -> dict[str, object]:
        self.shutdown_called = True
        return {"ok": True, "stopping": True}


def test_status_reports_not_running_without_state(tmp_path: Path) -> None:
    manager = ServiceManager(HieronymusConfig(data_root=tmp_path / "hieronymus"))

    status = manager.status()

    assert status["running"] is False
    assert status["reason"] == "no-state"


def test_status_uses_existing_healthy_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=12345,
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )
    write_server_state(config, state)
    manager = ServiceManager(config, client=FakeClient(healthy=True))

    status = manager.status()

    assert status["running"] is True
    assert status["pid"] == 12345


def test_stop_without_state_is_clean_result(tmp_path: Path) -> None:
    manager = ServiceManager(HieronymusConfig(data_root=tmp_path / "hieronymus"))

    result = manager.stop()

    assert result == {"running": False, "stopped": False, "reason": "not-running"}


def test_stop_calls_shutdown_for_existing_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=12345,
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )
    write_server_state(config, state)
    client = FakeClient(healthy=True)
    manager = ServiceManager(config, client=client)

    result = manager.stop()

    assert client.shutdown_called is True
    assert result["stopped"] is True
    assert read_server_state(config) is None
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_service_manager.py -v
```

Expected:

```text
FAILED tests/test_service_manager.py
```

- [ ] **Step 3: Implement HTTP client**

Create `src/hieronymus/service_client.py`:

```python
from __future__ import annotations

import json
import urllib.request
from typing import Any

from hieronymus.service_state import ServerState


class ServiceClient:
    def __init__(self, timeout: float = 2.0) -> None:
        self.timeout = timeout

    def health(self, state: ServerState) -> dict[str, Any]:
        return self._json("GET", state, "/health")

    def status(self, state: ServerState) -> dict[str, Any]:
        return self._json("GET", state, "/status")

    def shutdown(self, state: ServerState) -> dict[str, Any]:
        return self._json("POST", state, "/shutdown")

    def _json(self, method: str, state: ServerState, path: str) -> dict[str, Any]:
        request = urllib.request.Request(f"{state.base_url}{path}", method=method)
        with urllib.request.urlopen(request, timeout=self.timeout) as response:
            payload = json.loads(response.read().decode("utf-8"))
        if not isinstance(payload, dict):
            raise ValueError(f"expected JSON object from {path}")
        return payload
```

- [ ] **Step 4: Implement service manager**

Create `src/hieronymus/service_manager.py`:

```python
from __future__ import annotations

import subprocess
import sys
import time
from typing import Any, Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClient
from hieronymus.service_state import (
    ServerState,
    cleanup_stale_state,
    read_server_state,
    remove_server_state,
)


class ClientProtocol(Protocol):
    def health(self, state: ServerState) -> dict[str, Any]:
        raise NotImplementedError

    def status(self, state: ServerState) -> dict[str, Any]:
        raise NotImplementedError

    def shutdown(self, state: ServerState) -> dict[str, Any]:
        raise NotImplementedError


class ServiceManager:
    def __init__(
        self,
        config: HieronymusConfig,
        *,
        client: ClientProtocol | None = None,
        startup_timeout: float = 5.0,
    ) -> None:
        self.config = config
        self.client = client if client is not None else ServiceClient()
        self.startup_timeout = startup_timeout

    def status(self) -> dict[str, Any]:
        cleanup_stale_state(self.config)
        state = read_server_state(self.config)
        if state is None:
            return {"running": False, "reason": "no-state"}
        try:
            payload = self.client.status(state)
        except OSError:
            remove_server_state(self.config)
            return {"running": False, "reason": "unreachable"}
        return payload

    def ensure_running(self) -> dict[str, Any]:
        current = self.status()
        if current.get("running") is True:
            return {"started": False, "status": current}
        self.start()
        return {"started": True, "status": self.status()}

    def start(self) -> None:
        self.config.data_root.mkdir(parents=True, exist_ok=True)
        subprocess.Popen(
            [
                sys.executable,
                "-m",
                "hieronymus.service_daemon",
                "--data-root",
                str(self.config.data_root),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        deadline = time.monotonic() + self.startup_timeout
        while time.monotonic() < deadline:
            state = read_server_state(self.config)
            if state is not None:
                try:
                    self.client.health(state)
                    return
                except OSError:
                    time.sleep(0.05)
            else:
                time.sleep(0.05)
        raise RuntimeError("Hieronymus daemon did not become healthy")

    def stop(self) -> dict[str, Any]:
        state = read_server_state(self.config)
        if state is None:
            return {"running": False, "stopped": False, "reason": "not-running"}
        try:
            self.client.shutdown(state)
        except OSError:
            remove_server_state(self.config)
            return {"running": False, "stopped": False, "reason": "unreachable"}
        remove_server_state(self.config)
        return {"running": False, "stopped": True}

    def restart(self) -> dict[str, Any]:
        stopped = self.stop()
        self.start()
        return {"stopped": stopped, "status": self.status()}
```

- [ ] **Step 5: Run manager tests**

Run:

```bash
uv run pytest tests/test_service_manager.py -v
```

Expected:

```text
4 passed
```

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/service_client.py src/hieronymus/service_manager.py tests/test_service_manager.py
git commit -m "feat: add thin service client and manager"
```

## Task 5: Wire CLI Lifecycle Commands

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `tests/test_cli_service.py`

- [ ] **Step 1: Add failing CLI lifecycle tests**

Append to `tests/test_cli_service.py`:

```python
from unittest.mock import patch


def test_status_json_returns_manager_payload(tmp_path: Path) -> None:
    runner = CliRunner()

    with patch("hieronymus.cli.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        result = runner.invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "status", "--json"],
        )

    assert result.exit_code == 0
    assert json.loads(result.output) == {"reason": "no-state", "running": False}


def test_stop_json_returns_manager_payload(tmp_path: Path) -> None:
    runner = CliRunner()

    with patch("hieronymus.cli.ServiceManager") as manager_class:
        manager_class.return_value.stop.return_value = {
            "running": False,
            "stopped": False,
            "reason": "not-running",
        }
        result = runner.invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "stop", "--json"],
        )

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "reason": "not-running",
        "running": False,
        "stopped": False,
    }


def test_no_subcommand_ensures_service_and_prints_greeting(tmp_path: Path) -> None:
    runner = CliRunner()

    with patch("hieronymus.cli.ServiceManager") as manager_class:
        manager_class.return_value.ensure_running.return_value = {
            "started": True,
            "status": {"running": True, "pid": 1000, "port": 32199},
        }
        result = runner.invoke(main, ["--data-root", str(tmp_path / "hieronymus")])

    assert result.exit_code == 0
    assert "🪶 Hieronymus v" in result.output
    assert "running: yes" in result.output
    assert "port: 32199" in result.output
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_status_json_returns_manager_payload tests/test_cli_service.py::test_stop_json_returns_manager_payload tests/test_cli_service.py::test_no_subcommand_ensures_service_and_prints_greeting -v
```

Expected:

```text
FAILED tests/test_cli_service.py::test_status_json_returns_manager_payload
FAILED tests/test_cli_service.py::test_stop_json_returns_manager_payload
FAILED tests/test_cli_service.py::test_no_subcommand_ensures_service_and_prints_greeting
```

- [ ] **Step 3: Import service helpers in CLI**

Add to `src/hieronymus/cli.py`:

```python
from hieronymus.presentation import render_greeting, render_json
from hieronymus.service_manager import ServiceManager
```

- [ ] **Step 4: Make the Click group invoke without subcommand**

Replace the group decorator and function header in `src/hieronymus/cli.py`:

```python
@click.group(invoke_without_command=True)
@click.option("--data-root", type=click.Path(file_okay=False, dir_okay=True), default=None)
@click.pass_context
def main(ctx: click.Context, data_root: str | None) -> None:
    config = load_config(data_root)
    if config.data_root.exists() and not config.data_root.is_dir():
        raise click.ClickException(f"data root is not a directory: {config.data_root}")
    ctx.obj = {"config": config}
    if ctx.invoked_subcommand is None:
        result = ServiceManager(config).ensure_running()
        status = result["status"]
        click.echo(render_greeting())
        click.echo()
        click.echo("running: yes" if status.get("running") else "running: no")
        if "pid" in status:
            click.echo(f"pid: {status['pid']}")
        if "port" in status:
            click.echo(f"port: {status['port']}")
```

- [ ] **Step 5: Add status/stop/restart commands**

Add to `src/hieronymus/cli.py`:

```python
@main.command("status")
@click.option("--json", "as_json", is_flag=True)
@click.pass_context
def status_command(ctx: click.Context, as_json: bool) -> None:
    payload = ServiceManager(ctx.obj["config"]).status()
    if as_json:
        click.echo(render_json(payload))
        return
    click.echo(render_greeting())
    click.echo()
    click.echo("running: yes" if payload.get("running") else "running: no")
    if "reason" in payload:
        click.echo(f"reason: {payload['reason']}")
    if "pid" in payload:
        click.echo(f"pid: {payload['pid']}")
    if "port" in payload:
        click.echo(f"port: {payload['port']}")
    if "database_path" in payload:
        click.echo(f"database: {payload['database_path']}")


@main.command("stop")
@click.option("--json", "as_json", is_flag=True)
@click.pass_context
def stop_command(ctx: click.Context, as_json: bool) -> None:
    payload = ServiceManager(ctx.obj["config"]).stop()
    if as_json:
        click.echo(render_json(payload))
        return
    click.echo(render_greeting())
    click.echo()
    if payload.get("stopped"):
        click.echo("stopped: yes")
    else:
        click.echo("stopped: no")
        if "reason" in payload:
            click.echo(f"reason: {payload['reason']}")


@main.command("restart")
@click.option("--json", "as_json", is_flag=True)
@click.pass_context
def restart_command(ctx: click.Context, as_json: bool) -> None:
    payload = ServiceManager(ctx.obj["config"]).restart()
    if as_json:
        click.echo(render_json(payload))
        return
    click.echo(render_greeting())
    click.echo()
    click.echo("restarted: yes")
    status = payload["status"]
    if "pid" in status:
        click.echo(f"pid: {status['pid']}")
    if "port" in status:
        click.echo(f"port: {status['port']}")
```

- [ ] **Step 6: Add admin/config placeholders**

Add to `src/hieronymus/cli.py`:

```python
@main.command("config")
@click.option("--json", "as_json", is_flag=True)
@click.pass_context
def config_command(ctx: click.Context, as_json: bool) -> None:
    payload = {
        "config_root": str(ctx.obj["config"].config_root),
        "database_path": str(ctx.obj["config"].database_path),
        "tui": "not-available-in-this-pass",
    }
    if as_json:
        click.echo(render_json(payload))
        return
    click.echo(render_greeting())
    click.echo()
    click.echo(f"config: {payload['config_root']}")
    click.echo(f"database: {payload['database_path']}")
    click.echo("configuration TUI: planned")


@main.command("admin")
@click.option("--json", "as_json", is_flag=True)
def admin_command(as_json: bool) -> None:
    payload = {"tui": "not-available-in-this-pass"}
    if as_json:
        click.echo(render_json(payload))
        return
    click.echo(render_greeting())
    click.echo()
    click.echo("admin TUI: planned")
```

- [ ] **Step 7: Run CLI service tests**

Run:

```bash
uv run pytest tests/test_cli_service.py -v
```

Expected:

```text
7 passed
```

- [ ] **Step 8: Commit**

```bash
git add src/hieronymus/cli.py tests/test_cli_service.py
git commit -m "feat: add service lifecycle CLI commands"
```

## Task 6: Add Doctor Checks

**Files:**
- Create: `src/hieronymus/doctor.py`
- Modify: `src/hieronymus/cli.py`
- Create: `tests/test_doctor.py`
- Modify: `tests/test_cli_service.py`

- [ ] **Step 1: Write failing doctor tests**

Create `tests/test_doctor.py`:

```python
from __future__ import annotations

from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.doctor import Doctor, DoctorFinding


def test_doctor_reports_missing_config_root_as_autofixable(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    report = Doctor(config).run(autofix=False)

    assert DoctorFinding(
        level="warning",
        code="config-root-missing",
        message=f"Config root does not exist: {config.config_root}",
        autofixed=False,
    ) in report["warnings"]


def test_doctor_autofix_creates_config_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    report = Doctor(config).run(autofix=True)

    assert config.config_root.is_dir()
    assert report["autofixed"][0].code == "config-root-created"


def test_doctor_reports_database_file_when_present(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.database_path.write_text("not sqlite", encoding="utf-8")

    report = Doctor(config).run(autofix=False)

    assert report["errors"][0].code == "database-unreadable"
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_doctor.py -v
```

Expected:

```text
FAILED tests/test_doctor.py
```

- [ ] **Step 3: Implement doctor checks**

Create `src/hieronymus/doctor.py`:

```python
from __future__ import annotations

import sqlite3
from dataclasses import asdict, dataclass
from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.service_manager import ServiceManager


@dataclass(frozen=True)
class DoctorFinding:
    level: str
    code: str
    message: str
    autofixed: bool = False

    def to_json_dict(self) -> dict[str, Any]:
        return asdict(self)


class Doctor:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config

    def run(self, *, autofix: bool = False) -> dict[str, list[DoctorFinding]]:
        autofixed: list[DoctorFinding] = []
        warnings: list[DoctorFinding] = []
        errors: list[DoctorFinding] = []

        if not self.config.config_root.exists():
            if autofix:
                self.config.config_root.mkdir(parents=True, exist_ok=True)
                autofixed.append(
                    DoctorFinding(
                        level="info",
                        code="config-root-created",
                        message=f"Created config root: {self.config.config_root}",
                        autofixed=True,
                    )
                )
            else:
                warnings.append(
                    DoctorFinding(
                        level="warning",
                        code="config-root-missing",
                        message=f"Config root does not exist: {self.config.config_root}",
                    )
                )
        elif not self.config.config_root.is_dir():
            errors.append(
                DoctorFinding(
                    level="error",
                    code="config-root-not-directory",
                    message=f"Config root is not a directory: {self.config.config_root}",
                )
            )

        if self.config.database_path.exists():
            try:
                with sqlite3.connect(self.config.database_path) as conn:
                    conn.execute("select 1").fetchone()
            except sqlite3.DatabaseError:
                errors.append(
                    DoctorFinding(
                        level="error",
                        code="database-unreadable",
                        message=f"Database is not readable as SQLite: {self.config.database_path}",
                    )
                )

        status = ServiceManager(self.config).status()
        if status.get("running") is True:
            autofixed.append(
                DoctorFinding(
                    level="info",
                    code="daemon-running",
                    message="Hieronymus daemon is reachable.",
                )
            )
        else:
            warnings.append(
                DoctorFinding(
                    level="warning",
                    code="daemon-not-running",
                    message="Hieronymus daemon is not running.",
                )
            )

        return {"autofixed": autofixed, "warnings": warnings, "errors": errors}


def report_to_json(report: dict[str, list[DoctorFinding]]) -> dict[str, list[dict[str, Any]]]:
    return {
        "autofixed": [finding.to_json_dict() for finding in report["autofixed"]],
        "warnings": [finding.to_json_dict() for finding in report["warnings"]],
        "errors": [finding.to_json_dict() for finding in report["errors"]],
    }
```

- [ ] **Step 4: Add CLI doctor command**

Add imports to `src/hieronymus/cli.py`:

```python
from hieronymus.doctor import Doctor, report_to_json
```

Add command:

```python
@main.command("doctor")
@click.option("--fix", "autofix", is_flag=True)
@click.option("--json", "as_json", is_flag=True)
@click.pass_context
def doctor_command(ctx: click.Context, autofix: bool, as_json: bool) -> None:
    report = Doctor(ctx.obj["config"]).run(autofix=autofix)
    payload = report_to_json(report)
    if as_json:
        click.echo(render_json(payload))
        return
    click.echo(render_greeting())
    click.echo()
    for title, key in (("Autofixed", "autofixed"), ("Doctor warnings", "warnings"), ("Doctor errors", "errors")):
        click.echo(f"{title}:")
        findings = payload[key]
        if not findings:
            click.echo("  none")
        for finding in findings:
            click.echo(f"  - {finding['message']}")
```

- [ ] **Step 5: Add CLI doctor JSON test**

Append to `tests/test_cli_service.py`:

```python
def test_doctor_json_has_expected_sections(tmp_path: Path) -> None:
    runner = CliRunner()

    result = runner.invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "doctor", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert sorted(payload.keys()) == ["autofixed", "errors", "warnings"]
```

- [ ] **Step 6: Run doctor tests**

Run:

```bash
uv run pytest tests/test_doctor.py tests/test_cli_service.py::test_doctor_json_has_expected_sections -v
```

Expected:

```text
4 passed
```

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/doctor.py src/hieronymus/cli.py tests/test_doctor.py tests/test_cli_service.py
git commit -m "feat: add service doctor checks"
```

## Task 7: Add Installer Framework and Stub Targets

**Files:**
- Create: `src/hieronymus/install.py`
- Modify: `src/hieronymus/cli.py`
- Create: `tests/test_install.py`
- Modify: `tests/test_cli_service.py`

- [ ] **Step 1: Write failing installer framework tests**

Create `tests/test_install.py`:

```python
from __future__ import annotations

from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.install import (
    InstallPlan,
    InstallStep,
    atomic_write_text,
    backup_file,
    known_targets,
    plan_install,
    resolve_target,
)


def test_known_targets_include_initial_and_future_names() -> None:
    assert known_targets() == [
        "claude",
        "codex",
        "openclaw",
        "opencode",
        "gemini",
        "pi",
        "hermes",
    ]


def test_resolve_target_has_metadata_for_codex() -> None:
    target = resolve_target("codex")

    assert target.name == "codex"
    assert target.display_name == "Codex"
    assert "MCP" in target.protocol_note


def test_plan_install_returns_honest_stub_for_codex(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    plan = plan_install(config, "codex")

    assert isinstance(plan, InstallPlan)
    assert plan.target == "codex"
    assert plan.result_kind == "stub"
    assert plan.steps == [
        InstallStep(
            action="inspect",
            path="~/.codex/config.toml",
            description="Detect existing Codex MCP configuration.",
        ),
        InstallStep(
            action="defer",
            path="docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md",
            description="Real Codex hooks and skills are specified separately.",
        ),
    ]


def test_atomic_write_text_creates_parent_and_replaces_file(tmp_path: Path) -> None:
    target = tmp_path / "nested" / "config.json"

    atomic_write_text(target, "{\n  \"ok\": true\n}\n")

    assert target.read_text(encoding="utf-8") == "{\n  \"ok\": true\n}\n"


def test_backup_file_writes_under_hieronymus_backups(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    source = tmp_path / "agent.json"
    source.write_text("{\"old\": true}\n", encoding="utf-8")

    backup = backup_file(config, source, agent="codex", extension="json")

    assert backup.parent == config.backups_root
    assert backup.read_text(encoding="utf-8") == "{\"old\": true}\n"
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
uv run pytest tests/test_install.py -v
```

Expected:

```text
FAILED tests/test_install.py
```

- [ ] **Step 3: Implement installer framework**

Create `src/hieronymus/install.py`:

```python
from __future__ import annotations

import shutil
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

from hieronymus.config import HieronymusConfig

AGENT_WORKFLOW_SPEC = "docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md"


@dataclass(frozen=True)
class InstallStep:
    action: str
    path: str
    description: str

    def to_json_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class InstallPlan:
    target: str
    display_name: str
    result_kind: str
    protocol_note: str
    docs: str
    steps: list[InstallStep]

    def to_json_dict(self) -> dict[str, Any]:
        return {
            "target": self.target,
            "display_name": self.display_name,
            "result_kind": self.result_kind,
            "protocol_note": self.protocol_note,
            "docs": self.docs,
            "steps": [step.to_json_dict() for step in self.steps],
        }


@dataclass(frozen=True)
class InstallTarget:
    name: str
    display_name: str
    detect_path: str
    config_path: str
    protocol_note: str
    docs: str


TARGETS: list[InstallTarget] = [
    InstallTarget("claude", "Claude Code", "~/.claude.json", "~/.claude.json", "Uses MCP plus host-specific hooks in a later pass.", AGENT_WORKFLOW_SPEC),
    InstallTarget("codex", "Codex", "~/.codex", "~/.codex/config.toml", "Uses MCP. Real hooks and skills are separate follow-up work.", AGENT_WORKFLOW_SPEC),
    InstallTarget("openclaw", "OpenClaw", "~/.openclaw", "~/.openclaw/openclaw.json", "Uses OpenClaw plugin/MCP surfaces in a later pass.", AGENT_WORKFLOW_SPEC),
    InstallTarget("opencode", "opencode", "~/.config/opencode", "~/.config/opencode/plugin.json", "Uses opencode plugin and commands in a later pass.", AGENT_WORKFLOW_SPEC),
    InstallTarget("gemini", "Gemini CLI", "~/.gemini", "~/.gemini/settings.json", "Uses MCP, because Gemini CLI integration is MCP-oriented.", AGENT_WORKFLOW_SPEC),
    InstallTarget("pi", "Pi", "~/.pi", "~/.pi/config.json", "Future target reserved for a later integration spec.", AGENT_WORKFLOW_SPEC),
    InstallTarget("hermes", "Hermes", "~/.hermes", "~/.hermes/config.json", "Future target reserved for a later integration spec.", AGENT_WORKFLOW_SPEC),
]


def known_targets() -> list[str]:
    return [target.name for target in TARGETS]


def resolve_target(name: str) -> InstallTarget:
    normalized = name.lower()
    for target in TARGETS:
        if target.name == normalized:
            return target
    supported = ", ".join(known_targets())
    raise ValueError(f"unknown install target: {name}; supported targets: {supported}")


def plan_install(config: HieronymusConfig, target_name: str) -> InstallPlan:
    target = resolve_target(target_name)
    return InstallPlan(
        target=target.name,
        display_name=target.display_name,
        result_kind="stub",
        protocol_note=target.protocol_note,
        docs=target.docs,
        steps=[
            InstallStep(
                action="inspect",
                path=target.config_path,
                description=f"Detect existing {target.display_name} MCP configuration.",
            ),
            InstallStep(
                action="defer",
                path=AGENT_WORKFLOW_SPEC,
                description=f"Real {target.display_name} hooks and skills are specified separately.",
            ),
        ],
    )


def atomic_write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(f"{path.name}.tmp-{time.time_ns()}")
    tmp.write_text(text, encoding="utf-8")
    tmp.replace(path)


def backup_file(config: HieronymusConfig, source: Path, *, agent: str, extension: str) -> Path:
    config.backups_root.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    target = config.backups_root / f"{agent}-{stamp}.{extension}"
    shutil.copy2(source, target)
    return target
```

- [ ] **Step 4: Add install CLI command**

Add import to `src/hieronymus/cli.py`:

```python
from hieronymus.install import plan_install
```

Add command:

```python
@main.command("install")
@click.argument("app")
@click.option("--dry-run", is_flag=True)
@click.option("--force", is_flag=True)
@click.option("--json", "as_json", is_flag=True)
@click.pass_context
def install_command(ctx: click.Context, app: str, dry_run: bool, force: bool, as_json: bool) -> None:
    try:
        plan = plan_install(ctx.obj["config"], app)
    except ValueError as error:
        raise click.ClickException(str(error)) from error

    payload = plan.to_json_dict()
    payload["dry_run"] = dry_run
    payload["force"] = force
    if as_json:
        click.echo(render_json(payload))
        return

    click.echo(render_greeting())
    click.echo()
    click.echo(f"Installing {plan.display_name} integration")
    click.echo(plan.protocol_note)
    click.echo("Planned changes:")
    for step in plan.steps:
        click.echo(f"- {step.action}: {step.path}")
        click.echo(f"  {step.description}")
    if plan.result_kind == "stub":
        click.echo("Result: stub; real integration is deferred to the agent workflow spec.")
```

- [ ] **Step 5: Add CLI install tests**

Append to `tests/test_cli_service.py`:

```python
def test_install_json_returns_stub_plan(tmp_path: Path) -> None:
    runner = CliRunner()

    result = runner.invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "install", "codex", "--dry-run", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["target"] == "codex"
    assert payload["result_kind"] == "stub"
    assert payload["dry_run"] is True
    assert payload["steps"][1]["path"] == "docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md"


def test_install_unknown_target_returns_clean_error(tmp_path: Path) -> None:
    runner = CliRunner()

    result = runner.invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "install", "unknown-agent"],
    )

    assert result.exit_code == 1
    assert "unknown install target: unknown-agent" in result.output
    assert "Traceback" not in result.output
```

- [ ] **Step 6: Run installer tests**

Run:

```bash
uv run pytest tests/test_install.py tests/test_cli_service.py::test_install_json_returns_stub_plan tests/test_cli_service.py::test_install_unknown_target_returns_clean_error -v
```

Expected:

```text
7 passed
```

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/install.py src/hieronymus/cli.py tests/test_install.py tests/test_cli_service.py
git commit -m "feat: add agent installer framework stubs"
```

## Task 8: Add End-to-End Service Smoke Tests

**Files:**
- Modify: `tests/test_cli_service.py`
- Modify: `src/hieronymus/service_daemon.py` if smoke test exposes daemon startup issues.
- Modify: `src/hieronymus/service_manager.py` if smoke test exposes lifecycle timing issues.

- [ ] **Step 1: Add real subprocess lifecycle smoke test**

Append to `tests/test_cli_service.py`:

```python
def test_status_start_stop_lifecycle_with_real_daemon(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"

    start_result = subprocess.run(
        ["uv", "run", "hiero", "--data-root", str(data_root)],
        check=False,
        cwd=Path.cwd(),
        text=True,
        capture_output=True,
        timeout=10,
    )
    try:
        assert start_result.returncode == 0
        assert "🪶 Hieronymus v" in start_result.stdout

        status_result = subprocess.run(
            ["uv", "run", "hiero", "--data-root", str(data_root), "status", "--json"],
            check=False,
            cwd=Path.cwd(),
            text=True,
            capture_output=True,
            timeout=10,
        )
        assert status_result.returncode == 0
        status_payload = json.loads(status_result.stdout)
        assert status_payload["running"] is True
        assert status_payload["host"] == "127.0.0.1"
        assert status_payload["database_path"] == str(data_root / "hieronymus.sqlite")
    finally:
        stop_result = subprocess.run(
            ["uv", "run", "hiero", "--data-root", str(data_root), "stop", "--json"],
            check=False,
            cwd=Path.cwd(),
            text=True,
            capture_output=True,
            timeout=10,
        )
        assert stop_result.returncode == 0
```

- [ ] **Step 2: Run smoke test and verify behavior**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_status_start_stop_lifecycle_with_real_daemon -v
```

Expected:

```text
1 passed
```

- [ ] **Step 3: If the daemon does not stop reliably, adjust shutdown timing**

In `src/hieronymus/service_manager.py`, after `self.client.shutdown(state)`, poll briefly before removing state:

```python
        deadline = time.monotonic() + 2.0
        while time.monotonic() < deadline:
            try:
                self.client.health(state)
            except OSError:
                break
            time.sleep(0.05)
```

- [ ] **Step 4: Re-run smoke test**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_status_start_stop_lifecycle_with_real_daemon -v
```

Expected:

```text
1 passed
```

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/service_manager.py src/hieronymus/service_daemon.py tests/test_cli_service.py
git commit -m "test: cover real service lifecycle"
```

## Task 9: Add Documentation

**Files:**
- Create: `docs/service-toolkit.md`
- Modify: `docs/usage.md`

- [ ] **Step 1: Write service toolkit docs**

Create `docs/service-toolkit.md`:

```markdown
# Hieronymus Service Toolkit

Hieronymus installs two equivalent console commands:

- `hieronymus`
- `hiero`

Every subcommand works through either command. For example, `hieronymus status` and `hiero status`
call the same CLI entry point.

Running `hiero` with no subcommand starts the local daemon if it is not already running, then prints a
short status surface:

```text
🪶 Hieronymus v0.1.0
Remembers things for you.
```

The daemon is the only normal owner of the global SQLite store. Thin CLI commands and future agent
adapters discover it through runtime files under `~/.config/hieronymus`:

- `server.json`
- `server.pid`
- `server.lock`

The daemon exposes a local HTTP JSON API on `127.0.0.1`. Human CLI output may use the Hieronymus
identity line, while automation should use `--json`.

## Commands

- `hiero status --json` shows daemon status.
- `hiero doctor --json` checks config, store, and service health.
- `hiero stop` requests graceful shutdown.
- `hiero restart` restarts the daemon.
- `hiero config` shows config paths for now; the TUI is separate work.
- `hiero admin` reports that the management TUI is separate work.
- `hiero install <app> --dry-run` shows the safe installer plan for an agent.

## Agent Install Boundary

This pass provides the installer framework and stubs. Real Claude, Codex, OpenClaw, opencode, Gemini
CLI, Pi, and Hermes integrations belong to follow-up specs based on:

`docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md`
```

- [ ] **Step 2: Add usage examples**

Append to `docs/usage.md`:

```markdown
## Service Commands

```bash
hiero
hiero status --json
hiero doctor
hiero install codex --dry-run
hiero stop
```

`hiero` is an alias for `hieronymus`; all subcommands work with either command.
```

- [ ] **Step 3: Commit docs**

```bash
git add docs/service-toolkit.md docs/usage.md
git commit -m "docs: explain service toolkit commands"
```

## Task 10: Final Verification

**Files:**
- Modify only files needed to fix verification failures.

- [ ] **Step 1: Run full test suite**

Run:

```bash
uv run pytest
```

Expected:

```text
all tests pass
```

- [ ] **Step 2: Run lint**

Run:

```bash
uv run ruff check .
```

Expected:

```text
All checks passed!
```

- [ ] **Step 3: Run format check**

Run:

```bash
uv run ruff format --check .
```

Expected:

```text
All done!
```

- [ ] **Step 4: Check service spec boundary**

Run:

```bash
rg -n "real .*hook|global hook fallback|Claude hook|Codex hook|OpenClaw hook|opencode hook|Gemini hook" src tests docs/service-toolkit.md
```

Expected:

```text
docs/service-toolkit.md contains only boundary/deferred wording; src/ and tests do not contain real host hook implementations.
```

- [ ] **Step 5: Check worktree status**

Run:

```bash
git status --short
```

Expected:

```text
no uncommitted changes
```

If verification fixes were required:

```bash
git add .
git commit -m "test: verify service toolkit pass"
```

If no fixes were required, do not create an empty commit.

## Self-Review

Spec coverage:

- Python core, not Node: covered by using only Python stdlib modules and no new Node code.
- `hieronymus` and `hiero` aliases: Task 1.
- One local daemon exposing HTTP JSON: Tasks 2, 3, 4, and 8.
- Runtime state under `~/.config/hieronymus`: Tasks 2 and 5 preserve `load_config()` defaults and use `config.config_root`.
- Single-instance/idempotent behavior: Tasks 2 and 4 cover stale state cleanup, existing healthy state, and manager `ensure_running()`.
- Thin lifecycle commands: Task 5 adds no-subcommand startup, `status`, `stop`, `restart`, `config`, `admin`, and `help`.
- Doctor checks: Task 6.
- Installer framework: Task 7.
- Real integrations deferred: Tasks 7, 9, and 10 keep hooks/skills/lifecycle mappings out of source.
- Human presentation identity: Tasks 1 and 5.
- JSON automation output: Tasks 5, 6, and 7.
- Tests with fake home/config paths: Tasks 2, 4, 6, and 7 use `tmp_path`.

Quality checks:

- Every implementation task starts with failing tests.
- Every task has a targeted verification command and commit.
- File boundaries are focused: state, HTTP, client, manager, doctor, install, presentation.
- No external network, service manager, systemd, launchd, or host-agent config mutation is required.

Known sequencing risk:

- Task 8 starts a real daemon process. If it is flaky on the first implementation pass, fix process shutdown in `ServiceManager.stop()` before broadening tests.
