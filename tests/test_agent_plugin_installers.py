from __future__ import annotations

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
