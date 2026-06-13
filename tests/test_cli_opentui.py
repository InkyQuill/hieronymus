import json
import os
import re
import select
import shutil
import subprocess
import sys
import time
from pathlib import Path

import pytest
from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus.config import HieronymusConfig
from hieronymus.tui_bridge.server import dispatch


def test_bridge_dispatch_config_bootstrap(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    response = dispatch(
        config,
        {"id": "1", "method": "config.bootstrap", "params": {}},
    )

    assert response["id"] == "1"
    assert response["ok"] is True
    assert response["result"]["selected_provider"] == "anthropic"


def test_bridge_dispatch_normalizes_numeric_id(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    response = dispatch(
        config,
        {"id": 1, "method": "config.bootstrap", "params": {}},
    )

    assert response["id"] == "1"
    assert response["ok"] is True
    assert response["result"]["selected_provider"] == "anthropic"


def test_bridge_dispatch_unknown_method_returns_error(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    response = dispatch(
        config,
        {"id": "1", "method": "missing.method", "params": {}},
    )

    assert response == {
        "id": "1",
        "ok": False,
        "error": {"code": "method_not_found", "message": "unknown method: missing.method"},
    }


def test_cli_bridge_command_reads_one_request(tmp_path, monkeypatch) -> None:
    runner = CliRunner()
    request = json.dumps({"id": "1", "method": "config.bootstrap", "params": {}})

    result = runner.invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "tui-bridge"],
        input=request + "\n",
    )

    assert result.exit_code == 0
    response = json.loads(result.output)
    assert response["id"] == "1"
    assert response["ok"] is True


def test_cli_bridge_command_preserves_id_for_invalid_params(tmp_path) -> None:
    request = json.dumps({"id": "bad-params", "method": "config.bootstrap", "params": []})

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "tui-bridge"],
        input=request + "\n",
    )

    assert result.exit_code == 0
    response = json.loads(result.output)
    assert response == {
        "id": "bad-params",
        "ok": False,
        "error": {"code": "invalid_request", "message": "params must be an object"},
    }


def test_frontend_entrypoint_searches_from_module_parents(
    tmp_path: Path,
    monkeypatch,
) -> None:
    from hieronymus import cli

    repo = tmp_path / "repo"
    module_file = repo / "src" / "hieronymus" / "cli.py"
    bundle = repo / "frontend" / "dist" / "main.js"
    module_file.parent.mkdir(parents=True)
    module_file.write_text("", encoding="utf-8")
    bundle.parent.mkdir(parents=True)
    bundle.write_text("", encoding="utf-8")
    monkeypatch.setattr(cli, "__file__", str(module_file))
    monkeypatch.chdir(tmp_path)

    assert cli._frontend_entrypoint() == str(bundle)


def test_frontend_entrypoint_fails_when_bundle_is_missing(
    tmp_path: Path,
    monkeypatch,
) -> None:
    from hieronymus import cli

    module_file = tmp_path / "repo" / "src" / "hieronymus" / "cli.py"
    module_file.parent.mkdir(parents=True)
    module_file.write_text("", encoding="utf-8")
    monkeypatch.setattr(cli, "__file__", str(module_file))

    with pytest.raises(FileNotFoundError, match="OpenTUI frontend bundle not found; looked for:"):
        cli._frontend_entrypoint()


ANSI_RE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def _built_frontend_bundle() -> Path:
    return _repo_root() / "frontend" / "dist" / "main.js"


def _require_opentui_smoke_runtime() -> Path:
    if os.name != "posix":
        pytest.skip("OpenTUI smoke tests require a POSIX PTY")
    if shutil.which("bun") is None:
        pytest.skip("OpenTUI smoke tests require Bun")
    bundle = _built_frontend_bundle()
    if not bundle.exists():
        pytest.skip("OpenTUI smoke tests require frontend/dist/main.js")
    return bundle


def _strip_ansi(text: str) -> str:
    return ANSI_RE.sub("", text)


def _read_pty_until(fd: int, expected: str, *, timeout: float = 8.0) -> str:
    deadline = time.monotonic() + timeout
    chunks: list[bytes] = []
    expected_compact = "".join(expected.split())
    while time.monotonic() < deadline:
        readable, _, _ = select.select([fd], [], [], 0.1)
        if not readable:
            continue
        try:
            chunk = os.read(fd, 4096)
        except OSError:
            break
        if not chunk:
            break
        chunks.append(chunk)
        text = _strip_ansi(b"".join(chunks).decode(errors="replace"))
        if expected in text or expected_compact in "".join(text.split()):
            return text
    text = _strip_ansi(b"".join(chunks).decode(errors="replace"))
    raise AssertionError(f"did not see {expected!r} in OpenTUI output:\n{text}")


def _set_pty_size(fd: int, *, rows: int, columns: int) -> None:
    import fcntl
    import struct
    import termios

    size = struct.pack("HHHH", rows, columns, 0, 0)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, size)


def _smoke_opentui_bundle(mode: str, expected_title: str, tmp_path: Path) -> str:
    bundle = _require_opentui_smoke_runtime()
    import pty

    try:
        master_fd, slave_fd = pty.openpty()
    except OSError as error:
        pytest.skip(f"OpenTUI smoke tests could not allocate PTY: {error}")
    try:
        _set_pty_size(slave_fd, rows=48, columns=200)
    except OSError as error:
        os.close(master_fd)
        os.close(slave_fd)
        pytest.skip(f"OpenTUI smoke tests could not size PTY: {error}")
    data_root = tmp_path / "hieronymus"
    env = {
        **os.environ,
        "HIERONYMUS_DATA_ROOT": str(data_root),
        "TERM": os.environ.get("TERM", "xterm-256color"),
    }
    command = [
        "bun",
        str(bundle),
        mode,
        "--bridge-command",
        sys.executable,
        "--bridge-arg",
        "-m",
        "--bridge-arg",
        "hieronymus",
    ]
    process = subprocess.Popen(
        command,
        stdin=slave_fd,
        stdout=slave_fd,
        stderr=slave_fd,
        cwd=_repo_root(),
        env=env,
        close_fds=True,
    )
    os.close(slave_fd)
    try:
        output = _read_pty_until(master_fd, expected_title)
        os.write(master_fd, b"q")
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=5)
            raise AssertionError(f"OpenTUI {mode} did not exit after q")
        assert process.returncode == 0
        return output
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        os.close(master_fd)


def test_packaged_opentui_config_starts_from_built_bundle(tmp_path: Path) -> None:
    output = _smoke_opentui_bundle("config", "Hieronymus Config", tmp_path)

    assert "Providers" in output
    assert "dream.conf" in output


def test_packaged_opentui_admin_starts_from_built_bundle(tmp_path: Path) -> None:
    output = _smoke_opentui_bundle("admin", "Hieronymus Admin", tmp_path)

    assert "crystals0" in output
    assert "DreamDISABLED" in output


def test_cli_config_launches_opentui_when_tui_env_unset(tmp_path, monkeypatch) -> None:
    calls = []

    def fake_launch_opentui(mode, *, data_root):
        calls.append((mode, data_root))

    monkeypatch.delenv("HIERONYMUS_TUI", raising=False)
    monkeypatch.setattr("hieronymus.cli._launch_opentui", fake_launch_opentui)

    data_root = tmp_path / "hieronymus"
    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "config"],
    )

    assert result.exit_code == 0
    assert calls == [("config", data_root)]


def test_cli_admin_launches_opentui_when_tui_env_unset(tmp_path, monkeypatch) -> None:
    calls = []

    def fake_launch_opentui(mode, *, data_root):
        calls.append((mode, data_root))

    monkeypatch.delenv("HIERONYMUS_TUI", raising=False)
    monkeypatch.setattr("hieronymus.cli._launch_opentui", fake_launch_opentui)

    data_root = tmp_path / "hieronymus"
    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "admin"],
    )

    assert result.exit_code == 0
    assert calls == [("admin", data_root)]


def test_cli_opentui_config_launches_frontend_with_data_root(tmp_path, monkeypatch) -> None:
    calls = []
    data_root = tmp_path / "hieronymus"

    def fake_run(command, check, env):
        calls.append((command, env))

    monkeypatch.setattr("hieronymus.cli.subprocess.run", fake_run)
    monkeypatch.setattr("hieronymus.cli._frontend_entrypoint", lambda: "/tmp/hiero-opentui.js")

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "config"],
    )

    assert result.exit_code == 0
    command, env = calls[0]
    assert command[0] == "bun"
    assert command[1] == "/tmp/hiero-opentui.js"
    assert command[2:] == [
        "config",
        "--bridge-command",
        sys.executable,
        "--bridge-arg",
        "-m",
        "--bridge-arg",
        "hieronymus",
    ]
    assert env["HIERONYMUS_DATA_ROOT"] == str(data_root)


def test_cli_opentui_admin_launches_frontend_when_requested(tmp_path, monkeypatch) -> None:
    calls = []
    data_root = tmp_path / "hieronymus"

    def fake_run(command, check, env):
        calls.append((command, env))

    monkeypatch.setattr("hieronymus.cli.subprocess.run", fake_run)
    monkeypatch.setattr("hieronymus.cli._frontend_entrypoint", lambda: "/tmp/hiero-opentui.js")

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "admin"],
    )

    assert result.exit_code == 0
    command, env = calls[0]
    assert command[2:] == [
        "admin",
        "--bridge-command",
        sys.executable,
        "--bridge-arg",
        "-m",
        "--bridge-arg",
        "hieronymus",
    ]
    assert env["HIERONYMUS_DATA_ROOT"] == str(data_root)


def test_cli_opentui_launch_failure_returns_clean_error(tmp_path, monkeypatch) -> None:
    def fail_run(command, check, env):
        raise FileNotFoundError(command[0])

    monkeypatch.setattr("hieronymus.cli.subprocess.run", fail_run)
    monkeypatch.setattr("hieronymus.cli._frontend_entrypoint", lambda: "/tmp/hiero-opentui.js")

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "config"],
    )

    assert result.exit_code == 1
    assert "Error: OpenTUI launch failed: bun executable not found" in result.output
    assert "Traceback" not in result.output


def test_cli_opentui_nonzero_exit_returns_clean_error(tmp_path, monkeypatch) -> None:
    def fail_run(command, check, env):
        raise subprocess.CalledProcessError(7, command)

    monkeypatch.setattr("hieronymus.cli.subprocess.run", fail_run)
    monkeypatch.setattr("hieronymus.cli._frontend_entrypoint", lambda: "/tmp/hiero-opentui.js")

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "config"],
    )

    assert result.exit_code == 1
    assert "Error: OpenTUI exited with code 7" in result.output
    assert "Traceback" not in result.output


def test_cli_json_config_bypasses_opentui_launcher(tmp_path, monkeypatch) -> None:
    def fail_run(*args, **kwargs):
        raise AssertionError("JSON output must not launch frontend")

    monkeypatch.setattr("hieronymus.cli.subprocess.run", fail_run)

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "config", "--json"],
    )

    assert result.exit_code == 0
    assert '"dream_config_path"' in result.output
