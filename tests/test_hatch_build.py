from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from subprocess import CalledProcessError
from types import ModuleType, SimpleNamespace
from typing import Any

import pytest

ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture
def build_hook_module(monkeypatch: pytest.MonkeyPatch) -> ModuleType:
    interface = ModuleType("hatchling.builders.hooks.plugin.interface")
    interface.BuildHookInterface = object  # type: ignore[attr-defined]
    for package in (
        "hatchling",
        "hatchling.builders",
        "hatchling.builders.hooks",
        "hatchling.builders.hooks.plugin",
    ):
        monkeypatch.setitem(sys.modules, package, ModuleType(package))
    monkeypatch.setitem(sys.modules, interface.__name__, interface)

    spec = importlib.util.spec_from_file_location("tested_hatch_build", ROOT / "hatch_build.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def build_hook(module: ModuleType, root: Path) -> Any:
    hook = module.CustomBuildHook()
    hook.root = str(root)
    return hook


def test_build_hook_keeps_actionable_missing_bun_error(
    build_hook_module: ModuleType,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(build_hook_module, "which", lambda command: None)

    with pytest.raises(RuntimeError, match="install it and ensure `bun` is on PATH"):
        build_hook(build_hook_module, tmp_path).initialize("editable", {})


@pytest.mark.parametrize("version", ["1.3.0-alpha", "1.2.99", "0.99.0"])
def test_build_hook_rejects_unsupported_bun(
    build_hook_module: ModuleType,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    version: str,
) -> None:
    commands: list[list[str]] = []
    monkeypatch.setattr(build_hook_module, "which", lambda command: f"/bin/{command}")

    def fake_run(command: list[str], **kwargs: object) -> SimpleNamespace:
        commands.append(command)
        return SimpleNamespace(stdout=version)

    monkeypatch.setattr(build_hook_module, "run", fake_run)

    with pytest.raises(RuntimeError, match=f"installed Bun version {version} is unsupported"):
        build_hook(build_hook_module, tmp_path).initialize("editable", {})

    assert commands == [["bun", "--version"]]


@pytest.mark.parametrize("version", ["unknown", "1.3", ""])
def test_build_hook_rejects_unparseable_bun_versions(
    build_hook_module: ModuleType,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    version: str,
) -> None:
    monkeypatch.setattr(build_hook_module, "which", lambda command: f"/bin/{command}")
    monkeypatch.setattr(
        build_hook_module,
        "run",
        lambda command, **kwargs: SimpleNamespace(stdout=version),
    )

    with pytest.raises(RuntimeError, match="could not validate the installed Bun version"):
        build_hook(build_hook_module, tmp_path).initialize("editable", {})


def test_build_hook_rejects_failed_bun_version_command(
    build_hook_module: ModuleType,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(build_hook_module, "which", lambda command: f"/bin/{command}")

    def fail_version(command: list[str], **kwargs: object) -> None:
        raise CalledProcessError(1, command)

    monkeypatch.setattr(build_hook_module, "run", fail_version)

    with pytest.raises(RuntimeError, match="could not validate the installed Bun version"):
        build_hook(build_hook_module, tmp_path).initialize("editable", {})


def test_build_hook_validates_supported_bun_before_building(
    build_hook_module: ModuleType,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    calls: list[tuple[list[str], dict[str, object]]] = []
    monkeypatch.setattr(build_hook_module, "which", lambda command: f"/bin/{command}")

    def fake_run(command: list[str], **kwargs: object) -> SimpleNamespace:
        calls.append((command, kwargs))
        return SimpleNamespace(stdout="1.3.14")

    monkeypatch.setattr(build_hook_module, "run", fake_run)

    build_hook(build_hook_module, tmp_path).initialize("editable", {})

    assert [command for command, _ in calls] == [
        ["bun", "--version"],
        ["bun", "install", "--frozen-lockfile"],
        ["bun", "run", "build"],
    ]
    assert calls[0][1] == {"capture_output": True, "check": True, "text": True}
