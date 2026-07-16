from __future__ import annotations

from io import StringIO
from pathlib import Path

import click
import pytest
from click.testing import CliRunner

from hieronymus.cli import main


@pytest.fixture
def runner() -> CliRunner:
    return CliRunner()


class _TtyInput(StringIO):
    def isatty(self) -> bool:
        return True


def _set_tty_input(monkeypatch: pytest.MonkeyPatch) -> None:
    stdin = _TtyInput()
    get_text_stream = click.get_text_stream

    def tty_aware_text_stream(name: str, encoding: str | None = None) -> object:
        if name == "stdin":
            return stdin
        return get_text_stream(name, encoding)

    monkeypatch.setattr("hieronymus.cli.click.get_text_stream", tty_aware_text_stream)


def test_install_accepts_two_explicit_targets(
    runner: CliRunner, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)

    result = runner.invoke(main, ["skills", "install", "--target", "agents", "--target", "claude"])

    assert result.exit_code == 0
    assert Path(".agents/skills/hieronymus-read/SKILL.md").is_file()
    assert Path(".claude/skills/hieronymus-read/SKILL.md").is_file()


def test_noninteractive_install_without_target_is_usage_error(
    runner: CliRunner,
) -> None:
    result = runner.invoke(main, ["skills", "install"])

    assert result.exit_code == 2
    assert "supply at least one --target" in result.output


def test_install_dry_run_does_not_modify_workspace(
    runner: CliRunner, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)

    result = runner.invoke(main, ["skills", "install", "--target", "agents", "--dry-run"])

    assert result.exit_code == 0
    assert "Dry run" in result.output
    assert not (tmp_path / ".agents").exists()


def test_install_replaces_owned_skills_without_confirmation(
    runner: CliRunner, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)
    owned_skill = tmp_path / ".agents/skills/hieronymus-read/SKILL.md"
    owned_skill.parent.mkdir(parents=True)
    owned_skill.write_text("old", encoding="utf-8")

    result = runner.invoke(main, ["skills", "install", "--target", "agents"])

    assert result.exit_code == 0
    assert owned_skill.read_text(encoding="utf-8") != "old"


def test_uninstall_preserves_unrelated_skills_and_parent_directories(
    runner: CliRunner, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)
    custom_skill = tmp_path / ".agents/skills/custom/SKILL.md"
    custom_skill.parent.mkdir(parents=True)
    custom_skill.write_text("custom", encoding="utf-8")
    install = runner.invoke(main, ["skills", "install", "--target", "agents"])

    result = runner.invoke(main, ["skills", "uninstall", "--target", "agents"])

    assert install.exit_code == 0
    assert result.exit_code == 0
    assert not (tmp_path / ".agents/skills/hieronymus-read").exists()
    assert custom_skill.read_text(encoding="utf-8") == "custom"
    assert (tmp_path / ".agents/skills").is_dir()


def test_interactive_install_selects_both_targets_in_canonical_order(
    runner: CliRunner, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)
    _set_tty_input(monkeypatch)

    result = runner.invoke(main, ["skills", "install"], input="y\ny\ny\n")

    assert result.exit_code == 0
    assert (tmp_path / ".agents/skills/hieronymus-read/SKILL.md").is_file()
    assert (tmp_path / ".claude/skills/hieronymus-read/SKILL.md").is_file()
    assert ".agents, .claude" in result.output


def test_interactive_install_requires_at_least_one_selected_target(
    runner: CliRunner, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)
    _set_tty_input(monkeypatch)

    result = runner.invoke(main, ["skills", "install"], input="n\nn\n")

    assert result.exit_code == 2
    assert "supply at least one --target" in result.output


def test_interactive_uninstall_yes_bypasses_final_confirmation(
    runner: CliRunner, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)
    install = runner.invoke(main, ["skills", "install", "--target", "agents"])
    _set_tty_input(monkeypatch)

    result = runner.invoke(main, ["skills", "uninstall", "--yes"], input="y\nn\n")

    assert install.exit_code == 0
    assert result.exit_code == 0
    assert "Continue with" not in result.output
    assert not (tmp_path / ".agents/skills/hieronymus-read").exists()


@pytest.mark.parametrize("command", ["install", "uninstall"])
def test_project_skills_service_errors_render_click_errors(
    runner: CliRunner, monkeypatch: pytest.MonkeyPatch, command: str
) -> None:
    def fail(*args: object, **kwargs: object) -> None:
        raise ValueError("bad destination")

    monkeypatch.setattr(f"hieronymus.cli.{command}_project_skills", fail)

    result = runner.invoke(main, ["skills", command, "--target", "agents"])

    assert result.exit_code == 1
    assert "Error: bad destination" in result.output
    assert "Traceback" not in result.output
