from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path
from unittest.mock import patch

from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus.config import load_config
from hieronymus.presentation import GREETING_ICON, display_version, render_greeting
from hieronymus.settings import DreamingSettings, load_settings, save_settings


@dataclass(frozen=True)
class CliUpdateStatus:
    current_version: str = "0.1.0"
    latest_version: str | None = "0.1.0"
    latest_tag: str | None = "v0.1.0"
    current_revision: str | None = None
    latest_revision: str | None = None
    update_available: bool = False
    managed_checkout: Path = Path("/tmp/hieronymus-managed")
    managed_install: bool = True
    target: str = "latest"

    def as_dict(self) -> dict[str, object]:
        return {
            "current_version": self.current_version,
            "latest_version": self.latest_version,
            "latest_tag": self.latest_tag,
            "current_revision": self.current_revision,
            "latest_revision": self.latest_revision,
            "update_available": self.update_available,
            "managed_checkout": str(self.managed_checkout),
            "managed_install": self.managed_install,
            "target": self.target,
        }


def test_render_greeting_formats_prerelease_identity_for_humans() -> None:
    rendered = render_greeting("0.2.0")

    assert rendered == f"{GREETING_ICON} Hieronymus v0.2.0α\nRemembers things for you."


def test_display_version_marks_zero_major_versions_as_alpha() -> None:
    assert display_version("0.2.0") == "v0.2.0α"


def test_display_version_leaves_stable_versions_without_alpha() -> None:
    assert display_version("1.0.0") == "v1.0.0"


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
    assert "Open the memory management TUI" not in result.output
    assert "Open the configuration TUI" in result.output
    assert "Open the local management TUI" in result.output
    assert "Show management counts and available views" in result.output
    assert "Show config paths" not in result.output
    assert "hiero update           Update managed installs in place" in result.output


def test_click_help_describes_config_command() -> None:
    result = CliRunner().invoke(main, ["--help"])

    assert result.exit_code == 0
    assert "config" in result.output
    assert "Open the configuration TUI." in result.output


def test_readme_documents_production_install_update_and_uninstall() -> None:
    readme = Path("README.md").read_text(encoding="utf-8")
    normalized_readme = " ".join(readme.split())

    assert "https://github.com/InkyQuill/hieronymus" in readme
    assert (
        "curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/install.sh | sh"
        in readme
    )
    assert "hiero update" in readme
    assert "uninstall.sh" in readme
    assert "--keep-data" in readme
    assert "--purge-data" in readme
    assert (
        "The non-interactive uninstall one-liner removes the app and keeps "
        "settings/data by default." in normalized_readme
    )
    assert "HIERONYMUS_DATA_ROOT" in readme
    assert (
        "The uninstall script only removes Hieronymus-owned install and "
        "config/data paths. It does not remove translation workspace directories."
        in normalized_readme
    )
    assert (
        "The prompt uses ~/.config/hieronymus unless HIERONYMUS_DATA_ROOT is set."
        in normalized_readme
    )
    assert "--purge-data removes the configured data root." in normalized_readme
    assert "If HIERONYMUS_DATA_ROOT is set, check it before purging." in normalized_readme


def test_usage_documents_uninstall_data_modes_and_workspace_warning() -> None:
    usage = Path("docs/usage.md").read_text(encoding="utf-8")
    normalized_usage = " ".join(usage.split())

    assert "--keep-data" in usage
    assert "--purge-data" in usage
    assert (
        "The non-interactive uninstall one-liner removes the app and keeps "
        "settings/data by default." in normalized_usage
    )
    assert "HIERONYMUS_DATA_ROOT" in usage
    assert (
        "The uninstall script only removes Hieronymus-owned install and "
        "config/data paths. It does not remove translation workspace directories."
        in normalized_usage
    )
    assert "--purge-data removes the configured data root." in normalized_usage
    assert "If HIERONYMUS_DATA_ROOT is set, check it before purging." in normalized_usage


def test_docs_describe_real_config_tui_and_llm_providers() -> None:
    combined = "\n".join(
        Path(path).read_text(encoding="utf-8")
        for path in [
            "README.md",
            "docs/usage.md",
            "docs/memory-dreaming.md",
            "docs/service-toolkit.md",
        ]
    )

    forbidden = [
        "not-available-in-this-pass",
        "config TUI is separate work",
        "only provider implemented now is the deterministic provider",
        "External LLM providers are a later extension",
        "external LLM providers are deferred",
    ]
    for phrase in forbidden:
        assert phrase not in combined

    assert "hiero config" in combined
    assert "OPENAI_API_KEY" in combined
    assert "GEMINI_API_KEY" in combined
    assert "ANTHROPIC_API_KEY" in combined
    assert "API key values may be stored locally" in combined
    assert "new_short_term_memory_threshold" in combined
    assert "TypeScript React/OpenTUI terminal UI" in combined
    assert "Bun >=1.3" in combined
    assert "bun install --cwd frontend --frozen-lockfile" in combined
    assert "React/Ink" not in combined
    assert "Node.js >=22" not in combined
    assert "pnpm --dir frontend" not in combined


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


def test_restart_json_returns_manager_payload(tmp_path: Path) -> None:
    runner = CliRunner()

    with patch("hieronymus.cli.ServiceManager") as manager_class:
        manager_class.return_value.restart.return_value = {
            "stopped": {"running": False, "stopped": True},
            "status": {"running": True, "pid": 1000, "port": 32199},
        }
        result = runner.invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "restart", "--json"],
        )

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "status": {"pid": 1000, "port": 32199, "running": True},
        "stopped": {"running": False, "stopped": True},
    }


def test_config_json_returns_real_settings_and_paths(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "config", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["config_root"] == str(data_root)
    assert payload["database_path"] == str(data_root / "hieronymus.sqlite")
    assert payload["settings_path"] == str(data_root / "settings.toml")
    assert payload["tui"] == "available"
    assert payload["settings"]["dreaming"]["active_provider"] == "deterministic"
    assert payload["providers"][0]["name"] == "deterministic"


def test_config_launch_invokes_opentui(tmp_path: Path, monkeypatch) -> None:
    data_root = tmp_path / "hieronymus"
    launched = {}

    def fake_launch_opentui(mode, *, data_root):
        launched["mode"] = mode
        launched["data_root"] = data_root

    monkeypatch.setattr("hieronymus.cli._launch_opentui", fake_launch_opentui)

    result = CliRunner().invoke(main, ["--data-root", str(data_root), "config"])

    assert result.exit_code == 0
    assert launched == {
        "mode": "config",
        "data_root": data_root,
    }


def test_dream_json_uses_configured_active_provider(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    config = load_config(str(data_root))
    save_settings(
        config,
        load_settings(config).with_dreaming(DreamingSettings(active_provider="deterministic")),
    )

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "dream", "--json"],
    )

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "cycle_id": 1,
        "status": "completed",
        "provider": "deterministic",
        "input_count": 0,
        "created_crystal_count": 0,
        "proposal_count": 0,
        "error": "",
    }


def test_dream_rejects_disabled_provider(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "dream", "--provider", "openai"],
    )

    assert result.exit_code != 0
    assert "dream provider is disabled: openai" in result.output


def test_admin_json_returns_available_tui_status(tmp_path: Path) -> None:
    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "admin", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["tui"] == "available"
    assert payload["counts"] == {
        "series": 0,
        "crystals": 0,
        "lessons": 0,
        "short_term_memories": 0,
        "sessions": 0,
        "dream_runs": 0,
        "pending_proposals": 0,
        "audit_events": 0,
    }
    assert payload["service"]["running"] is False


def test_install_json_returns_dry_run_plan(tmp_path: Path) -> None:
    runner = CliRunner()

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(tmp_path / "hieronymus"),
            "install",
            "codex",
            "--dry-run",
            "--json",
        ],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["target"] == "codex"
    assert payload["result_kind"] == "installable"
    assert payload["dry_run"] is True
    assert payload["steps"][1]["path"] == "~/.codex/config.toml"


def test_install_human_output_is_honest_dry_run_plan(tmp_path: Path) -> None:
    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "install", "codex", "--dry-run"],
    )

    assert result.exit_code == 0
    assert "Planning Codex integration" in result.output
    assert "Installed Codex integration" not in result.output
    assert "Planned changes:" in result.output


def test_install_unknown_target_returns_clean_error(tmp_path: Path) -> None:
    runner = CliRunner()

    result = runner.invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "install", "unknown-agent"],
    )

    assert result.exit_code == 1
    assert "unknown install target: unknown-agent" in result.output
    assert "Traceback" not in result.output


def test_update_check_json_returns_status_from_release(tmp_path: Path) -> None:
    status = CliUpdateStatus(latest_version="0.2.0", latest_tag="v0.2.0", update_available=True)

    with patch("hieronymus.cli.check_update", return_value=status) as check_update:
        result = CliRunner().invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "update", "--check", "--json"],
        )

    assert result.exit_code == 0
    check_update.assert_called_once_with(target="latest", allow_dev=False)
    assert json.loads(result.output) == status.as_dict()


def test_update_rejects_main_without_dev_flag(tmp_path: Path) -> None:
    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "update", "--check", "--target", "main"],
    )

    assert result.exit_code == 1
    assert "--dev" in result.output
    assert "Traceback" not in result.output


def test_update_check_human_prints_main_revision_with_dev_flag(tmp_path: Path) -> None:
    status = CliUpdateStatus(
        current_version="0.2.0",
        latest_version=None,
        latest_tag="main",
        current_revision="abc1234",
        latest_revision="def5678",
        update_available=True,
        target="main",
    )

    with patch("hieronymus.cli.check_update", return_value=status) as check_update:
        result = CliRunner().invoke(
            main,
            [
                "--data-root",
                str(tmp_path / "hieronymus"),
                "update",
                "--check",
                "--target",
                "main",
                "--dev",
            ],
        )

    assert result.exit_code == 0
    check_update.assert_called_once_with(target="main", allow_dev=True)
    assert "Update available: abc1234 -> def5678" in result.output
    assert "vmain" not in result.output


def test_update_human_runs_update_and_prints_up_to_date(tmp_path: Path) -> None:
    status = CliUpdateStatus(current_version="0.2.0", latest_version="0.2.0")

    with (
        patch("hieronymus.cli.check_update", return_value=status) as check_update,
        patch("hieronymus.cli.run_update") as run_update,
    ):
        result = CliRunner().invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "update"],
        )

    assert result.exit_code == 0
    check_update.assert_called_once_with(target="latest", allow_dev=False)
    run_update.assert_not_called()
    assert "Hieronymus is up to date: v0.2.0α" in result.output
    assert "managed checkout: /tmp/hieronymus-managed" in result.output


def test_update_check_human_prints_no_update_available(tmp_path: Path) -> None:
    status = CliUpdateStatus(current_version="0.2.0", latest_version="0.2.0")

    with patch("hieronymus.cli.check_update", return_value=status):
        result = CliRunner().invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "update", "--check"],
        )

    assert result.exit_code == 0
    assert "No update available: v0.2.0α" in result.output
    assert "Hieronymus is up to date" not in result.output


def test_update_check_human_prints_update_available(tmp_path: Path) -> None:
    status = CliUpdateStatus(latest_version="0.2.0", latest_tag="v0.2.0", update_available=True)

    with patch("hieronymus.cli.check_update", return_value=status):
        result = CliRunner().invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "update", "--check"],
        )

    assert result.exit_code == 0
    assert "Update available: v0.1.0α -> v0.2.0α" in result.output
    assert "Updated Hieronymus" not in result.output


def test_update_human_prints_updated_after_applied_update(tmp_path: Path) -> None:
    before = CliUpdateStatus(
        current_version="0.1.0",
        latest_version="0.2.0",
        latest_tag="v0.2.0",
        update_available=True,
    )
    after = CliUpdateStatus(current_version="0.2.0", latest_version="0.2.0")

    with (
        patch("hieronymus.cli.check_update", return_value=before) as check_update,
        patch("hieronymus.cli.run_update", return_value=after) as run_update,
    ):
        result = CliRunner().invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "update"],
        )

    assert result.exit_code == 0
    check_update.assert_called_once_with(target="latest", allow_dev=False)
    run_update.assert_called_once_with(target="latest", allow_dev=False)
    assert "Updated Hieronymus: v0.1.0α -> v0.2.0α" in result.output
    assert "Update available" not in result.output


def test_update_human_prints_available_if_update_remains_available(tmp_path: Path) -> None:
    before = CliUpdateStatus(
        current_version="0.1.0",
        latest_version="0.2.0",
        latest_tag="v0.2.0",
        update_available=True,
    )
    after = CliUpdateStatus(
        current_version="0.1.0",
        latest_version="0.2.0",
        latest_tag="v0.2.0",
        update_available=True,
    )

    with (
        patch("hieronymus.cli.check_update", return_value=before),
        patch("hieronymus.cli.run_update", return_value=after),
    ):
        result = CliRunner().invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "update"],
        )

    assert result.exit_code == 0
    assert "Update available: v0.1.0α -> v0.2.0α" in result.output
    assert "Updated Hieronymus" not in result.output


def test_update_unmanaged_runtime_error_returns_clean_error(tmp_path: Path) -> None:
    status = CliUpdateStatus(latest_version="0.2.0", latest_tag="v0.2.0", update_available=True)

    with (
        patch("hieronymus.cli.check_update", return_value=status),
        patch(
            "hieronymus.cli.run_update",
            side_effect=RuntimeError("Updates require installation through the managed installer."),
        ),
    ):
        result = CliRunner().invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "update"],
        )

    assert result.exit_code == 1
    assert "Updates require installation through the managed installer." in result.output
    assert "Traceback" not in result.output


def test_update_subprocess_error_returns_clean_error(tmp_path: Path) -> None:
    error = subprocess.CalledProcessError(returncode=128, cmd=["git", "fetch", "--tags"])

    with patch("hieronymus.cli.check_update", side_effect=error):
        result = CliRunner().invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "update", "--check"],
        )

    assert result.exit_code == 1
    assert "Update command failed: git fetch --tags exited with code 128" in result.output
    assert "Traceback" not in result.output


def test_doctor_json_has_expected_sections(tmp_path: Path) -> None:
    runner = CliRunner()

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        result = runner.invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "doctor", "--json"],
        )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert sorted(payload.keys()) == ["autofixed", "errors", "warnings"]


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


def test_status_start_stop_lifecycle_with_real_daemon(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"

    try:
        start_result = subprocess.run(
            ["uv", "run", "hiero", "--data-root", str(data_root)],
            check=False,
            cwd=Path.cwd(),
            text=True,
            capture_output=True,
            timeout=10,
        )
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
