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

    atomic_write_text(target, '{\n  "ok": true\n}\n')

    assert target.read_text(encoding="utf-8") == '{\n  "ok": true\n}\n'


def test_backup_file_writes_under_hieronymus_backups(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    source = tmp_path / "agent.json"
    source.write_text('{"old": true}\n', encoding="utf-8")

    backup = backup_file(config, source, agent="codex", extension="json")

    assert backup.parent == config.backups_root
    assert backup.read_text(encoding="utf-8") == '{"old": true}\n'
