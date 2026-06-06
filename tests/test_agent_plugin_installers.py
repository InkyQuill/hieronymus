from __future__ import annotations

import json
import tomllib
from pathlib import Path

import pytest

from hieronymus.agent_plugins import resolve_plugin
from hieronymus.agent_plugins.base import write_plugin_assets
from hieronymus.config import HieronymusConfig


def test_write_plugin_assets_creates_expected_files(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    written = write_plugin_assets(
        config,
        "codex",
        {
            "skills/hieronymus-recall/SKILL.md": "recall",
            "mcp/hieronymus.mcp.json": "{}\n",
        },
    )

    assert written == [
        config.agent_plugins_root / "codex" / "skills" / "hieronymus-recall" / "SKILL.md",
        config.agent_plugins_root / "codex" / "mcp" / "hieronymus.mcp.json",
    ]
    assert written[0].read_text(encoding="utf-8") == "recall"
    assert written[1].read_text(encoding="utf-8") == "{}\n"


def test_write_plugin_assets_rejects_path_escape(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(ValueError, match="asset path escapes plugin root"):
        write_plugin_assets(config, "codex", {"../escape": "bad"})


def test_write_plugin_assets_rejects_target_traversal(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    escaped = config.agent_plugins_root.parent / "escaped" / "asset.txt"

    with pytest.raises(ValueError, match="plugin target must be a simple name"):
        write_plugin_assets(config, "../escaped", {"asset.txt": "bad"})

    assert not escaped.exists()


def test_write_plugin_assets_rejects_absolute_target(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    escaped = tmp_path / "escaped" / "asset.txt"

    with pytest.raises(ValueError, match="plugin target must be a simple name"):
        write_plugin_assets(config, str(tmp_path / "escaped"), {"asset.txt": "bad"})

    assert not escaped.exists()


def test_write_plugin_assets_rejects_symlink_plugin_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    escaped_root = tmp_path / "escaped"
    escaped_root.mkdir()
    config.agent_plugins_root.mkdir(parents=True)
    (config.agent_plugins_root / "codex").symlink_to(escaped_root, target_is_directory=True)

    with pytest.raises(ValueError, match="plugin root must not be a symlink"):
        write_plugin_assets(config, "codex", {"asset.txt": "bad"})

    assert not (escaped_root / "asset.txt").exists()


def test_write_plugin_assets_rejects_symlink_agent_plugins_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    escaped_root = tmp_path / "escaped"
    escaped_root.mkdir()
    config.config_root.mkdir(parents=True)
    config.agent_plugins_root.symlink_to(escaped_root, target_is_directory=True)

    with pytest.raises(ValueError, match="agent plugins root must not be a symlink"):
        write_plugin_assets(config, "codex", {"asset.txt": "bad"})

    assert not (escaped_root / "codex" / "asset.txt").exists()


def test_codex_install_writes_assets_and_reports_installed(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / ".codex").mkdir(parents=True)
    (home / ".codex" / "config.toml").write_text("", encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    plan = resolve_plugin("codex").install(config, force=False)

    assert plan.result_kind == "installed"
    assert plan.availability.installed is True
    assert (
        config.agent_plugins_root / "codex" / "skills" / "hieronymus-recall" / "SKILL.md"
    ).exists()
    assert (config.agent_plugins_root / "codex" / ".codex-plugin" / "plugin.json").exists()
    assert (config.agent_plugins_root / "codex" / ".mcp.json").exists()
    assert resolve_plugin("codex").availability(config).installed is True


def test_codex_install_is_idempotent_for_identical_assets(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / ".codex").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    plugin = resolve_plugin("codex")

    first = plugin.install(config, force=False)
    manifest = config.agent_plugins_root / "codex" / ".codex-plugin" / "plugin.json"
    first_text = manifest.read_text(encoding="utf-8")
    second = plugin.install(config, force=False)

    assert first.result_kind == "installed"
    assert second.result_kind == "installed"
    assert manifest.read_text(encoding="utf-8") == first_text


def test_codex_install_patches_toml_mcp_and_plugin(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    codex = home / ".codex"
    codex.mkdir(parents=True)
    config_path = codex / "config.toml"
    config_path.write_text('[profile]\nname = "default"\n', encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    resolve_plugin("codex").install(config)

    payload = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert payload["profile"]["name"] == "default"
    assert payload["mcp_servers"]["hieronymus"]["command"] == "hieronymus-mcp"
    assert payload["mcp_servers"]["hieronymus"]["args"] == []
    assert payload["plugins"]["hieronymus"]["path"] == str(config.agent_plugins_root / "codex")
    assert payload["hieronymus"] == {"managed": True, "version": "0.1.0"}
    backups = list(config.backups_root.glob("codex-config-*.toml"))
    assert len(backups) == 1
    assert backups[0].read_text(encoding="utf-8") == '[profile]\nname = "default"\n'


def test_codex_install_keeps_toml_config_valid_on_repeated_install(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / ".codex").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    plugin = resolve_plugin("codex")

    plugin.install(config)
    plugin.install(config)

    payload = tomllib.loads((home / ".codex" / "config.toml").read_text(encoding="utf-8"))
    assert sorted(payload["mcp_servers"]) == ["hieronymus"]
    assert sorted(payload["plugins"]) == ["hieronymus"]


def test_claude_install_patches_json_config(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / ".claude").mkdir(parents=True)
    claude_json = home / ".claude.json"
    claude_json.write_text('{"theme": "dark"}\n', encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    plan = resolve_plugin("claude").install(config)

    payload = json.loads(claude_json.read_text(encoding="utf-8"))
    assert plan.result_kind == "installed"
    assert plan.availability.installed is True
    assert payload["theme"] == "dark"
    assert payload["mcpServers"]["hieronymus"] == {
        "args": [],
        "command": "hieronymus-mcp",
    }
    assert payload["hieronymus"] == {
        "managed": True,
        "pluginPath": str(config.agent_plugins_root / "claude"),
        "version": "0.1.0",
    }
    assert (config.agent_plugins_root / "claude" / ".claude-plugin" / "plugin.json").exists()
    backups = list(config.backups_root.glob("claude-.claude-*.json"))
    assert len(backups) == 1
    assert backups[0].read_text(encoding="utf-8") == '{"theme": "dark"}\n'


def test_claude_install_creates_missing_json_config(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / ".claude").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    resolve_plugin("claude").install(config)

    payload = json.loads((home / ".claude.json").read_text(encoding="utf-8"))
    assert payload["mcpServers"]["hieronymus"]["command"] == "hieronymus-mcp"
    assert payload["hieronymus"]["managed"] is True
    assert not config.backups_root.exists()


def test_claude_install_handles_empty_json_config(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / ".claude").mkdir(parents=True)
    claude_json = home / ".claude.json"
    claude_json.write_text("\n", encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    resolve_plugin("claude").install(config)

    payload = json.loads(claude_json.read_text(encoding="utf-8"))
    assert payload["mcpServers"]["hieronymus"]["command"] == "hieronymus-mcp"
    backups = list(config.backups_root.glob("claude-.claude-*.json"))
    assert len(backups) == 1
    assert backups[0].read_text(encoding="utf-8") == "\n"


def test_claude_install_rejects_non_object_json_without_writing_assets(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / ".claude").mkdir(parents=True)
    (home / ".claude.json").write_text("[]\n", encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(ValueError, match="expected JSON object"):
        resolve_plugin("claude").install(config)

    assert not (config.agent_plugins_root / "claude").exists()
    assert json.loads((home / ".claude.json").read_text(encoding="utf-8")) == []


def test_claude_install_is_idempotent_for_config_entries(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / ".claude").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    plugin = resolve_plugin("claude")

    plugin.install(config)
    plugin.install(config)

    payload = json.loads((home / ".claude.json").read_text(encoding="utf-8"))
    assert sorted(payload["mcpServers"]) == ["hieronymus"]
    assert payload["hieronymus"]["pluginPath"] == str(config.agent_plugins_root / "claude")


@pytest.mark.parametrize(
    (
        "target",
        "host_dir",
        "config_path",
        "existing_payload",
        "backup_glob",
        "manifest_path",
        "registration_key",
    ),
    [
        (
            "openclaw",
            ".openclaw",
            ".openclaw/openclaw.json",
            {"theme": "dark"},
            "openclaw-openclaw-*.json",
            "openclaw/plugin.json",
            "plugins",
        ),
        (
            "opencode",
            ".config/opencode",
            ".config/opencode/plugin.json",
            {"mode": "fast"},
            "opencode-plugin-*.json",
            "opencode/plugin.json",
            "plugins",
        ),
        (
            "gemini",
            ".gemini",
            ".gemini/settings.json",
            {"ui": {"theme": "dark"}},
            "gemini-settings-*.json",
            "gemini-extension.json",
            "extensions",
        ),
    ],
)
def test_json_agent_install_patches_config_and_reports_installed(
    target: str,
    host_dir: str,
    config_path: str,
    existing_payload: dict[str, object],
    backup_glob: str,
    manifest_path: str,
    registration_key: str,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / host_dir).mkdir(parents=True)
    absolute_config_path = home / config_path
    existing_text = json.dumps(existing_payload, sort_keys=True) + "\n"
    absolute_config_path.write_text(existing_text, encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    plan = resolve_plugin(target).install(config)

    payload = json.loads(absolute_config_path.read_text(encoding="utf-8"))
    assert plan.result_kind == "installed"
    assert plan.availability.installed is True
    for key, value in existing_payload.items():
        assert payload[key] == value
    assert payload["mcpServers"]["hieronymus"] == {
        "args": [],
        "command": "hieronymus-mcp",
    }
    assert payload[registration_key]["hieronymus"] == {
        "path": str(config.agent_plugins_root / target),
    }
    assert payload["hieronymus"] == {"managed": True, "version": "0.1.0"}
    assert (config.agent_plugins_root / target / manifest_path).exists()
    assert resolve_plugin(target).availability(config).installed is True
    backups = list(config.backups_root.glob(backup_glob))
    assert len(backups) == 1
    assert backups[0].read_text(encoding="utf-8") == existing_text


@pytest.mark.parametrize(
    ("target", "host_dir", "config_path", "registration_key"),
    [
        ("openclaw", ".openclaw", ".openclaw/openclaw.json", "plugins"),
        ("opencode", ".config/opencode", ".config/opencode/plugin.json", "plugins"),
        ("gemini", ".gemini", ".gemini/settings.json", "extensions"),
    ],
)
def test_json_agent_install_is_idempotent_for_config_entries(
    target: str,
    host_dir: str,
    config_path: str,
    registration_key: str,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / host_dir).mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    plugin = resolve_plugin(target)

    first = plugin.install(config)
    second = plugin.install(config)

    payload = json.loads((home / config_path).read_text(encoding="utf-8"))
    assert first.result_kind == "installed"
    assert second.result_kind == "installed"
    assert sorted(payload["mcpServers"]) == ["hieronymus"]
    assert sorted(payload[registration_key]) == ["hieronymus"]
    assert payload["hieronymus"]["managed"] is True


def test_codex_install_rejects_conflicting_mcp_without_force(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    codex = home / ".codex"
    codex.mkdir(parents=True)
    config_path = codex / "config.toml"
    config_path.write_text(
        '[mcp_servers.hieronymus]\ncommand = "custom-hiero"\n',
        encoding="utf-8",
    )
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(ValueError, match="refusing to overwrite existing hieronymus entry"):
        resolve_plugin("codex").install(config, force=False)

    payload = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert payload["mcp_servers"]["hieronymus"]["command"] == "custom-hiero"
    assert not (config.agent_plugins_root / "codex").exists()


def test_codex_install_force_overwrites_conflicting_mcp(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    codex = home / ".codex"
    codex.mkdir(parents=True)
    config_path = codex / "config.toml"
    config_path.write_text(
        '[mcp_servers.hieronymus]\ncommand = "custom-hiero"\n',
        encoding="utf-8",
    )
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    plan = resolve_plugin("codex").install(config, force=True)

    payload = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert plan.availability.installed is True
    assert payload["mcp_servers"]["hieronymus"]["command"] == "hieronymus-mcp"


def test_json_agent_install_rejects_malformed_section_without_traceback(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    (home / ".gemini").mkdir(parents=True)
    settings = home / ".gemini" / "settings.json"
    settings.write_text('{"mcpServers": []}\n', encoding="utf-8")
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(ValueError, match="expected object at mcpServers"):
        resolve_plugin("gemini").install(config)

    assert json.loads(settings.read_text(encoding="utf-8")) == {"mcpServers": []}
    assert not (config.agent_plugins_root / "gemini").exists()
