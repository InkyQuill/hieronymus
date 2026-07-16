from __future__ import annotations

from pathlib import Path

import pytest
from click.testing import CliRunner

from hieronymus.cli import main


@pytest.fixture
def runner() -> CliRunner:
    return CliRunner()


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
