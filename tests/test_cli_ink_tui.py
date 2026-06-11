import json
import subprocess
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
    assert response["result"]["selected_provider"] == "openai"


def test_bridge_dispatch_normalizes_numeric_id(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    response = dispatch(
        config,
        {"id": 1, "method": "config.bootstrap", "params": {}},
    )

    assert response["id"] == "1"
    assert response["ok"] is True
    assert response["result"]["selected_provider"] == "openai"


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

    with pytest.raises(FileNotFoundError, match="Ink frontend bundle not found; looked for:"):
        cli._frontend_entrypoint()


def test_cli_config_launches_ink_when_tui_env_unset(tmp_path, monkeypatch) -> None:
    calls = []

    def fake_launch_ink(mode, *, data_root):
        calls.append((mode, data_root))

    monkeypatch.delenv("HIERONYMUS_TUI", raising=False)
    monkeypatch.setattr("hieronymus.cli._launch_ink", fake_launch_ink)

    data_root = tmp_path / "hieronymus"
    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "config"],
    )

    assert result.exit_code == 0
    assert calls == [("config", data_root)]


def test_cli_admin_launches_ink_when_tui_env_unset(tmp_path, monkeypatch) -> None:
    calls = []

    def fake_launch_ink(mode, *, data_root):
        calls.append((mode, data_root))

    monkeypatch.delenv("HIERONYMUS_TUI", raising=False)
    monkeypatch.setattr("hieronymus.cli._launch_ink", fake_launch_ink)

    data_root = tmp_path / "hieronymus"
    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "admin"],
    )

    assert result.exit_code == 0
    assert calls == [("admin", data_root)]


def test_cli_ink_config_launches_frontend_with_data_root(tmp_path, monkeypatch) -> None:
    calls = []
    data_root = tmp_path / "hieronymus"

    def fake_run(command, check, env):
        calls.append((command, env))

    monkeypatch.setattr("hieronymus.cli.subprocess.run", fake_run)
    monkeypatch.setattr("hieronymus.cli._frontend_entrypoint", lambda: "/tmp/hiero-ink.js")

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "config"],
    )

    assert result.exit_code == 0
    command, env = calls[0]
    assert command[0] == "node"
    assert command[1] == "/tmp/hiero-ink.js"
    assert command[2:] == [
        "config",
        "--bridge-command",
        "hiero",
    ]
    assert env["HIERONYMUS_DATA_ROOT"] == str(data_root)


def test_cli_ink_admin_launches_frontend_when_requested(tmp_path, monkeypatch) -> None:
    calls = []
    data_root = tmp_path / "hieronymus"

    def fake_run(command, check, env):
        calls.append((command, env))

    monkeypatch.setattr("hieronymus.cli.subprocess.run", fake_run)
    monkeypatch.setattr("hieronymus.cli._frontend_entrypoint", lambda: "/tmp/hiero-ink.js")

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "admin"],
    )

    assert result.exit_code == 0
    command, env = calls[0]
    assert command[2:] == [
        "admin",
        "--bridge-command",
        "hiero",
    ]
    assert env["HIERONYMUS_DATA_ROOT"] == str(data_root)


def test_cli_ink_launch_failure_returns_clean_error(tmp_path, monkeypatch) -> None:
    def fail_run(command, check, env):
        raise FileNotFoundError(command[0])

    monkeypatch.setattr("hieronymus.cli.subprocess.run", fail_run)
    monkeypatch.setattr("hieronymus.cli._frontend_entrypoint", lambda: "/tmp/hiero-ink.js")

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "config"],
    )

    assert result.exit_code == 1
    assert "Error: Ink TUI launch failed: node executable not found" in result.output
    assert "Traceback" not in result.output


def test_cli_ink_nonzero_exit_returns_clean_error(tmp_path, monkeypatch) -> None:
    def fail_run(command, check, env):
        raise subprocess.CalledProcessError(7, command)

    monkeypatch.setattr("hieronymus.cli.subprocess.run", fail_run)
    monkeypatch.setattr("hieronymus.cli._frontend_entrypoint", lambda: "/tmp/hiero-ink.js")

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "config"],
    )

    assert result.exit_code == 1
    assert "Error: Ink TUI exited with code 7" in result.output
    assert "Traceback" not in result.output


def test_cli_json_config_bypasses_ink_launcher(tmp_path, monkeypatch) -> None:
    def fail_run(*args, **kwargs):
        raise AssertionError("JSON output must not launch frontend")

    monkeypatch.setattr("hieronymus.cli.subprocess.run", fail_run)

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "config", "--json"],
    )

    assert result.exit_code == 0
    assert '"settings_path"' in result.output
