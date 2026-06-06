from __future__ import annotations

from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.install import (
    TARGETS,
    InstallPlan,
    InstallStep,
    InstallTarget,
    atomic_write_text,
    backup_file,
    known_targets,
    plan_install,
    resolve_target,
)


def test_public_install_targets_are_facades() -> None:
    assert all(isinstance(target, InstallTarget) for target in TARGETS)
    assert [target.name for target in TARGETS] == known_targets()


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
    assert isinstance(target, InstallTarget)
    assert target.detect_path == "~/.codex"
    assert target.config_path == "~/.codex/config.toml"
    assert "MCP" in target.protocol_note


def test_resolve_target_has_complete_metadata_for_all_targets() -> None:
    expected = {
        "claude": ("Claude Code / Claude Desktop", "~/.claude", "~/.claude.json"),
        "codex": ("Codex", "~/.codex", "~/.codex/config.toml"),
        "openclaw": ("OpenClaw", "~/.openclaw", "~/.openclaw/openclaw.json"),
        "opencode": (
            "opencode",
            "~/.config/opencode",
            "~/.config/opencode/plugin.json",
        ),
        "gemini": ("Gemini CLI", "~/.gemini", "~/.gemini/settings.json"),
        "pi": ("Pi", "~/.pi", "~/.pi/config.json"),
        "hermes": ("Hermes", "~/.hermes", "~/.hermes/config.json"),
    }

    for name, (display_name, detect_path, config_path) in expected.items():
        target = resolve_target(name)

        assert target.display_name == display_name
        assert target.detect_path == detect_path
        assert target.config_path == config_path
        assert target.docs == "docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md"
        assert target.protocol_note


def test_plan_install_returns_provider_plan_for_codex(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    plan = plan_install(config, "codex")

    assert isinstance(plan, InstallPlan)
    assert plan.target == "codex"
    assert plan.result_kind == "installable"
    assert plan.steps == [
        InstallStep(
            action="write-assets",
            path=str(config.agent_plugins_root / "codex"),
            description="Write plugin assets.",
        ),
        InstallStep(
            action="patch-config",
            path="~/.codex/config.toml",
            description="Register the Hieronymus plugin.",
        ),
    ]


def test_plan_install_json_includes_docs_for_codex(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    payload = plan_install(config, "codex").to_json_dict()

    assert payload["docs"] == "docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md"
    assert payload["availability"]["target"] == "codex"


def test_resolve_target_has_reserved_pi_paths() -> None:
    target = resolve_target("pi")

    assert target.detect_path == "~/.pi"
    assert target.config_path == "~/.pi/config.json"


def test_resolve_target_has_reserved_hermes_paths() -> None:
    target = resolve_target("hermes")

    assert target.detect_path == "~/.hermes"
    assert target.config_path == "~/.hermes/config.json"


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


def test_backup_file_returns_unique_existing_paths_for_same_source(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    source = tmp_path / "agent.json"
    source.write_text('{"old": true}\n', encoding="utf-8")

    first = backup_file(config, source, agent="codex", extension="json")
    second = backup_file(config, source, agent="codex", extension="json")

    assert first != second
    assert first.exists()
    assert second.exists()
