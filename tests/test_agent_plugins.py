from __future__ import annotations

from pathlib import Path

import pytest

from hieronymus.agent_plugins import available_plugins, resolve_plugin
from hieronymus.agent_plugins.base import AgentAvailability, InstallPlan
from hieronymus.config import HieronymusConfig


def test_available_plugins_include_supported_targets() -> None:
    assert [plugin.name for plugin in available_plugins()] == [
        "claude",
        "codex",
        "openclaw",
        "opencode",
        "gemini",
        "pi",
        "hermes",
    ]


def test_resolve_plugin_returns_provider() -> None:
    assert resolve_plugin("codex").display_name == "Codex"


def test_resolve_plugin_normalizes_lower_case_name() -> None:
    assert resolve_plugin("CoDeX").name == "codex"


def test_resolve_plugin_reports_supported_targets() -> None:
    with pytest.raises(
        ValueError,
        match=(
            "unknown install target: unknown; supported targets: "
            "claude, codex, openclaw, opencode, gemini, pi, hermes"
        ),
    ):
        resolve_plugin("unknown")


def test_config_has_agent_plugins_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert config.agent_plugins_root == tmp_path / "hieronymus" / "agent-plugins"


def test_codex_availability_detects_host_and_plugin_install(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / ".codex").mkdir(parents=True)
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    (config.agent_plugins_root / "codex").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))

    availability = resolve_plugin("codex").availability(config)

    assert availability == AgentAvailability(
        target="codex",
        display_name="Codex",
        available=True,
        installed=True,
        detect_paths=["~/.codex"],
        config_paths=["~/.codex/config.toml"],
        install_path=str(config.agent_plugins_root / "codex"),
        reason="host detected",
    )


def test_claude_availability_checks_all_detect_paths(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    home.mkdir()
    (home / ".claude.json").write_text("{}\n", encoding="utf-8")
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    monkeypatch.setenv("HOME", str(home))

    availability = resolve_plugin("claude").availability(config)

    assert availability.available is True
    assert availability.installed is False
    assert availability.detect_paths == ["~/.claude", "~/.claude.json"]
    assert availability.config_paths == ["~/.claude.json"]


def test_plugin_plan_includes_availability(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    plan = resolve_plugin("codex").plan(config)

    assert isinstance(plan, InstallPlan)
    assert plan.availability.target == "codex"
    assert plan.to_json_dict()["availability"] == plan.availability.to_json_dict()
