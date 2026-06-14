from __future__ import annotations

from pathlib import Path

import pytest

from hieronymus.agent_plugins import available_plugins, resolve_plugin
from hieronymus.agent_plugins.base import AgentAvailability, BaseAgentPlugin, InstallPlan
from hieronymus.config import HieronymusConfig


@pytest.fixture
def isolated_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    return home


def test_available_plugins_lists_canonical_targets_in_order() -> None:
    assert [plugin.name for plugin in available_plugins()] == [
        "claude",
        "codex",
        "openclaw",
        "opencode",
        "gemini",
        "mimo",
        "pi",
        "hermes",
    ]


def test_resolve_plugin_returns_provider() -> None:
    assert resolve_plugin("codex").display_name == "Codex"


def test_resolve_plugin_normalizes_lower_case_name() -> None:
    assert resolve_plugin("CoDeX").name == "codex"


def test_resolve_plugin_supports_aliases() -> None:
    assert resolve_plugin("xiaomi-mimo").name == "mimo"
    assert resolve_plugin("xiaomi_mimo").name == "mimo"
    assert resolve_plugin("mimocode").name == "mimo"
    assert resolve_plugin("MiMoCode").name == "mimo"


def test_resolve_plugin_reports_supported_targets() -> None:
    with pytest.raises(
        ValueError,
        match=(
            "Unsupported agent target 'unknown'; supported targets: "
            "claude, codex, openclaw, opencode, gemini, mimo, pi, hermes"
        ),
    ):
        resolve_plugin("unknown")


def test_config_has_agent_plugins_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert config.agent_plugins_root == tmp_path / "hieronymus" / "agent-plugins"


def test_codex_availability_requires_host_marker_for_install(
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
        installed=False,
        detect_paths=("~/.codex",),
        config_paths=("~/.codex/config.toml",),
        install_path=str(config.agent_plugins_root / "codex"),
        reason="host detected",
    )


def test_codex_availability_detects_assets_and_managed_marker(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    codex = home / ".codex"
    codex.mkdir(parents=True)
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    (codex / "config.toml").write_text(
        "\n".join(
            [
                "[hieronymus]",
                "managed = true",
                "[mcp_servers.hieronymus]",
                'command = "hieronymus-mcp"',
                "args = []",
                "[plugins.hieronymus]",
                f'path = "{config.agent_plugins_root / "codex"}"',
                "",
            ]
        ),
        encoding="utf-8",
    )
    (config.agent_plugins_root / "codex" / ".codex-plugin").mkdir(parents=True)
    (config.agent_plugins_root / "codex" / "skills" / "hieronymus-recall").mkdir(parents=True)
    (config.agent_plugins_root / "codex" / ".codex-plugin" / "plugin.json").write_text(
        "{}\n",
        encoding="utf-8",
    )
    (config.agent_plugins_root / "codex" / ".mcp.json").write_text("{}\n", encoding="utf-8")
    (config.agent_plugins_root / "codex" / "skills" / "hieronymus-recall" / "SKILL.md").write_text(
        "recall\n", encoding="utf-8"
    )
    monkeypatch.setenv("HOME", str(home))

    availability = resolve_plugin("codex").availability(config)

    assert availability.installed is True


def test_codex_availability_rejects_stale_marker_without_entries(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    codex = home / ".codex"
    codex.mkdir(parents=True)
    (codex / "config.toml").write_text("[hieronymus]\nmanaged = true\n", encoding="utf-8")
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    (config.agent_plugins_root / "codex" / ".codex-plugin").mkdir(parents=True)
    (config.agent_plugins_root / "codex" / "skills" / "hieronymus-recall").mkdir(parents=True)
    (config.agent_plugins_root / "codex" / ".codex-plugin" / "plugin.json").write_text(
        "{}\n",
        encoding="utf-8",
    )
    (config.agent_plugins_root / "codex" / ".mcp.json").write_text("{}\n", encoding="utf-8")
    (config.agent_plugins_root / "codex" / "skills" / "hieronymus-recall" / "SKILL.md").write_text(
        "recall\n", encoding="utf-8"
    )
    monkeypatch.setenv("HOME", str(home))

    availability = resolve_plugin("codex").availability(config)

    assert availability.installed is False


def test_codex_availability_rejects_incomplete_asset_directory(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    codex = home / ".codex"
    codex.mkdir(parents=True)
    (codex / "config.toml").write_text("[hieronymus]\nmanaged = true\n", encoding="utf-8")
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    (config.agent_plugins_root / "codex").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))

    availability = resolve_plugin("codex").availability(config)

    assert availability.installed is False


def test_codex_availability_rejects_symlink_required_asset(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    codex = home / ".codex"
    codex.mkdir(parents=True)
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    (codex / "config.toml").write_text(
        "\n".join(
            [
                "[hieronymus]",
                "managed = true",
                "[mcp_servers.hieronymus]",
                'command = "hieronymus-mcp"',
                "args = []",
                "[plugins.hieronymus]",
                f'path = "{config.agent_plugins_root / "codex"}"',
                "",
            ]
        ),
        encoding="utf-8",
    )
    plugin_root = config.agent_plugins_root / "codex"
    (plugin_root / ".codex-plugin").mkdir(parents=True)
    (plugin_root / "skills" / "hieronymus-recall").mkdir(parents=True)
    outside = tmp_path / "outside-plugin.json"
    outside.write_text("{}\n", encoding="utf-8")
    (plugin_root / ".codex-plugin" / "plugin.json").symlink_to(outside)
    (plugin_root / ".mcp.json").write_text("{}\n", encoding="utf-8")
    (plugin_root / "skills" / "hieronymus-recall" / "SKILL.md").write_text(
        "recall\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("HOME", str(home))

    availability = resolve_plugin("codex").availability(config)

    assert availability.installed is False


def test_codex_availability_rejects_symlink_asset_directory(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    codex = home / ".codex"
    codex.mkdir(parents=True)
    (codex / "config.toml").write_text("[hieronymus]\nmanaged = true\n", encoding="utf-8")
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.agent_plugins_root.mkdir(parents=True)
    escaped = tmp_path / "escaped-codex"
    escaped.mkdir()
    (config.agent_plugins_root / "codex").symlink_to(escaped, target_is_directory=True)
    monkeypatch.setenv("HOME", str(home))

    availability = resolve_plugin("codex").availability(config)

    assert availability.installed is False


def test_reserved_provider_ignores_stale_managed_marker(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    pi = home / ".pi"
    pi.mkdir(parents=True)
    (pi / "config.json").write_text(
        '{"hieronymus": {"managed": true}}\n',
        encoding="utf-8",
    )
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    (config.agent_plugins_root / "pi").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))

    availability = resolve_plugin("pi").availability(config)

    assert availability.available is True
    assert availability.installed is False
    assert availability.reason == "reserved target"


def test_mimo_availability_detects_mimocode_home(
    isolated_home: Path,
    tmp_path: Path,
) -> None:
    (isolated_home / ".mimocode").mkdir()
    config = HieronymusConfig(data_root=tmp_path)

    availability = resolve_plugin("mimo").availability(config)

    assert availability.available is True
    assert availability.installed is False
    assert availability.reason == "reserved target"


def test_reserved_plugins_report_reserved_install_plan(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path)
    plugin = resolve_plugin("pi")

    plan = plugin.install(config)

    assert plan.result_kind == "reserved"
    assert plan.steps == []
    assert "protocol" in plan.protocol_note.lower()
    assert not (config.agent_plugins_root / "pi").exists()


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
    assert availability.detect_paths == ("~/.claude", "~/.claude.json")
    assert availability.config_paths == ("~/.claude.json",)


def test_availability_json_paths_are_fresh_lists() -> None:
    availability = AgentAvailability(
        target="codex",
        display_name="Codex",
        available=True,
        installed=False,
        detect_paths=["~/.codex"],
        config_paths=["~/.codex/config.toml"],
        install_path="/tmp/hieronymus/agent-plugins/codex",
        reason="host detected",
    )

    payload = availability.to_json_dict()
    assert payload["detect_paths"] == ["~/.codex"]
    assert payload["config_paths"] == ["~/.codex/config.toml"]

    detect_paths = payload["detect_paths"]
    config_paths = payload["config_paths"]
    assert isinstance(detect_paths, list)
    assert isinstance(config_paths, list)

    detect_paths.append("mutated")
    config_paths.append("mutated")

    assert availability.detect_paths == ("~/.codex",)
    assert availability.config_paths == ("~/.codex/config.toml",)


def test_invalid_plugin_reports_empty_detect_paths(tmp_path: Path) -> None:
    class InvalidPlugin(BaseAgentPlugin):
        name = "invalid"
        display_name = "Invalid"
        detect_paths = ()
        config_paths = ("~/invalid.json",)
        protocol_note = "Invalid test plugin."

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(
        ValueError,
        match="invalid plugin must define at least one detect path",
    ):
        InvalidPlugin().plan(config)


def test_invalid_plugin_reports_empty_config_paths(tmp_path: Path) -> None:
    class InvalidPlugin(BaseAgentPlugin):
        name = "invalid"
        display_name = "Invalid"
        detect_paths = ("~/.invalid",)
        config_paths = ()
        protocol_note = "Invalid test plugin."

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(
        ValueError,
        match="invalid plugin must define at least one config path",
    ):
        InvalidPlugin().plan(config)


def test_plugin_plan_includes_availability(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    plan = resolve_plugin("codex").plan(config)

    assert isinstance(plan, InstallPlan)
    assert plan.availability.target == "codex"
    assert plan.to_json_dict()["availability"] == plan.availability.to_json_dict()
