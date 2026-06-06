from __future__ import annotations

import json

import pytest

from hieronymus.agent_assets import asset_map, render_agent_plugin_assets


def test_asset_map_contains_required_skills() -> None:
    assets = asset_map()

    assert "skills/hieronymus-recall/SKILL.md" in assets
    assert "skills/hieronymus-learn/SKILL.md" in assets
    assert "skills/hieronymus-read/SKILL.md" in assets
    assert "skills/hieronymus-translate/SKILL.md" in assets
    assert "skills/hieronymus-review/SKILL.md" in assets
    assert "skills/hieronymus-orchestrate/SKILL.md" in assets
    assert "mcp/hieronymus.mcp.json" in assets
    assert "hooks/hooks.codex.json" in assets


def test_skill_text_enforces_strict_vs_advisory_contract() -> None:
    assets = asset_map()
    translate = assets["skills/hieronymus-translate/SKILL.md"]

    assert "Strict concept contracts are mandatory" in translate
    assert "Crystals and lessons are advisory" in translate
    assert "Do not approve terminology proposals yourself" in translate


def test_all_skills_include_boundary_contract() -> None:
    assets = asset_map()
    skill_paths = [path for path in assets if path.startswith("skills/")]

    for path in skill_paths:
        skill = assets[path]
        assert "Strict concept contracts are mandatory" in skill
        assert "Crystals and lessons are advisory" in skill
        assert "Do not approve terminology proposals yourself" in skill


def test_mcp_config_references_hieronymus_mcp() -> None:
    config = json.loads(asset_map()["mcp/hieronymus.mcp.json"])

    assert config["mcpServers"]["hieronymus"]["command"] == "hieronymus-mcp"


def test_codex_hooks_call_session_start_and_end_modules() -> None:
    hooks = json.loads(asset_map()["hooks/hooks.codex.json"])

    assert hooks["hooks"] == [
        {
            "args": ["-m", "hieronymus.agent_hooks", "session-start"],
            "command": "python",
            "event": "SessionStart",
        },
        {
            "args": ["-m", "hieronymus.agent_hooks", "session-end"],
            "command": "python",
            "event": "Stop",
        },
    ]


def test_render_agent_plugin_assets_includes_agent_name() -> None:
    assets = render_agent_plugin_assets("codex")

    assert assets[".codex-plugin/plugin.json"].startswith("{")
    assert '"name": "hieronymus"' in assets[".codex-plugin/plugin.json"]
    assert "hieronymus-mcp" in assets["mcp/hieronymus.mcp.json"]


@pytest.mark.parametrize(
    ("target", "manifest_path"),
    [
        ("codex", ".codex-plugin/plugin.json"),
        ("claude", ".claude-plugin/plugin.json"),
        ("gemini", "gemini-extension.json"),
        ("opencode", "opencode/plugin.json"),
    ],
)
def test_render_agent_plugin_assets_adds_target_manifest(
    target: str,
    manifest_path: str,
) -> None:
    assets = render_agent_plugin_assets(target)
    manifest = json.loads(assets[manifest_path])

    assert manifest["name"] == "hieronymus"
    assert "mcp/hieronymus.mcp.json" in assets


def test_codex_manifest_includes_hooks_entry() -> None:
    assets = render_agent_plugin_assets("codex")
    manifest = json.loads(assets[".codex-plugin/plugin.json"])

    assert manifest["hooks"] == "./hooks/hooks.codex.json"


def test_render_agent_plugin_assets_rejects_unknown_target() -> None:
    with pytest.raises(ValueError, match="Unsupported agent plugin target"):
        render_agent_plugin_assets("unknown")
