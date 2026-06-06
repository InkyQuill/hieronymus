from __future__ import annotations

import json
import os
import shutil
import tempfile
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

import tomli_w

from hieronymus.config import HieronymusConfig

AGENT_WORKFLOW_SPEC = "docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md"


@dataclass(frozen=True)
class AgentAvailability:
    target: str
    display_name: str
    available: bool
    installed: bool
    detect_paths: tuple[str, ...]
    config_paths: tuple[str, ...]
    install_path: str
    reason: str

    def __post_init__(self) -> None:
        object.__setattr__(self, "detect_paths", tuple(self.detect_paths))
        object.__setattr__(self, "config_paths", tuple(self.config_paths))

    def to_json_dict(self) -> dict[str, object]:
        return {
            "target": self.target,
            "display_name": self.display_name,
            "available": self.available,
            "installed": self.installed,
            "detect_paths": list(self.detect_paths),
            "config_paths": list(self.config_paths),
            "install_path": self.install_path,
            "reason": self.reason,
        }


@dataclass(frozen=True)
class InstallStep:
    action: str
    path: str
    description: str

    def to_json_dict(self) -> dict[str, str]:
        return {
            "action": self.action,
            "path": self.path,
            "description": self.description,
        }


@dataclass(frozen=True)
class InstallPlan:
    target: str
    display_name: str
    protocol_note: str
    docs: str
    result_kind: str
    steps: list[InstallStep]
    availability: AgentAvailability

    def to_json_dict(self) -> dict[str, object]:
        return {
            "target": self.target,
            "display_name": self.display_name,
            "protocol_note": self.protocol_note,
            "docs": self.docs,
            "result_kind": self.result_kind,
            "steps": [step.to_json_dict() for step in self.steps],
            "availability": self.availability.to_json_dict(),
        }


class AgentPlugin(Protocol):
    name: str
    display_name: str
    detect_paths: tuple[str, ...]
    config_paths: tuple[str, ...]
    docs: str
    protocol_note: str

    def availability(self, config: HieronymusConfig) -> AgentAvailability:
        raise NotImplementedError

    def plan(self, config: HieronymusConfig) -> InstallPlan:
        raise NotImplementedError

    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        raise NotImplementedError


def expand_user(path: str) -> Path:
    return Path(path).expanduser()


def load_json_object(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    text = path.read_text(encoding="utf-8")
    if text.strip() == "":
        return {}
    payload = json.loads(text)
    if not isinstance(payload, dict):
        raise ValueError(f"expected JSON object in {path}")
    return payload


def patch_json_config(
    config: HieronymusConfig,
    path: Path,
    *,
    agent: str,
    payload: dict[str, Any],
) -> None:
    if path.exists():
        backup_file(config, path, agent=agent, extension=".json")
    atomic_write_text(
        path,
        json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
    )


def load_toml_object(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    text = path.read_text(encoding="utf-8")
    if text.strip() == "":
        return {}
    return tomllib.loads(text)


def patch_toml_config(
    config: HieronymusConfig,
    path: Path,
    *,
    agent: str,
    payload: dict[str, Any],
) -> None:
    if path.exists():
        backup_file(config, path, agent=agent, extension=".toml")
    atomic_write_text(path, tomli_w.dumps(payload))


def any_path_exists(paths: tuple[str, ...]) -> bool:
    return any(expand_user(path).exists() for path in paths)


def has_hieronymus_marker(path: Path) -> bool:
    if not path.exists():
        return False
    try:
        if path.suffix == ".toml":
            payload = load_toml_object(path)
        elif path.suffix == ".json":
            payload = load_json_object(path)
        else:
            text = path.read_text(encoding="utf-8")
            return "BEGIN HIERONYMUS MANAGED BLOCK" in text
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError):
        return False

    marker = payload.get("hieronymus")
    if isinstance(marker, dict) and marker.get("managed") is True:
        return True
    return False


def write_plugin_assets(
    config: HieronymusConfig,
    target: str,
    assets: dict[str, str],
) -> list[Path]:
    if (
        not target
        or Path(target).is_absolute()
        or ".." in target
        or "/" in target
        or "\\" in target
    ):
        raise ValueError(f"plugin target must be a simple name: {target}")

    plugin_root = config.agent_plugins_root / target
    if plugin_root.is_symlink():
        raise ValueError(f"plugin root must not be a symlink: {plugin_root}")

    root = plugin_root.resolve()
    written: list[Path] = []
    for relative, text in assets.items():
        destination = (root / relative).resolve()
        if destination != root and root not in destination.parents:
            raise ValueError(f"asset path escapes plugin root: {relative}")
        atomic_write_text(destination, text)
        written.append(destination)
    return written


class BaseAgentPlugin:
    name: str
    display_name: str
    detect_paths: tuple[str, ...]
    config_paths: tuple[str, ...]
    docs = AGENT_WORKFLOW_SPEC
    protocol_note: str

    def _require_non_empty_paths(self) -> None:
        if not self.detect_paths:
            raise ValueError(f"{self.name} plugin must define at least one detect path")
        if not self.config_paths:
            raise ValueError(f"{self.name} plugin must define at least one config path")

    @property
    def detect_path(self) -> str:
        self._require_non_empty_paths()
        return self.detect_paths[0]

    @property
    def config_path(self) -> str:
        self._require_non_empty_paths()
        return self.config_paths[0]

    def availability(self, config: HieronymusConfig) -> AgentAvailability:
        self._require_non_empty_paths()
        install_path = config.agent_plugins_root / self.name
        available = any_path_exists(self.detect_paths)
        installed = install_path.exists() and any(
            has_hieronymus_marker(expand_user(path)) for path in self.config_paths
        )
        return AgentAvailability(
            target=self.name,
            display_name=self.display_name,
            available=available,
            installed=installed,
            detect_paths=self.detect_paths,
            config_paths=self.config_paths,
            install_path=str(install_path),
            reason="host detected" if available else "host config path not found",
        )

    def plan(self, config: HieronymusConfig) -> InstallPlan:
        self._require_non_empty_paths()
        return InstallPlan(
            target=self.name,
            display_name=self.display_name,
            protocol_note=self.protocol_note,
            docs=self.docs,
            result_kind="installable",
            availability=self.availability(config),
            steps=[
                InstallStep(
                    "write-assets",
                    str(config.agent_plugins_root / self.name),
                    "Write plugin assets.",
                ),
                InstallStep(
                    "patch-config",
                    self.config_paths[0],
                    "Register the Hieronymus plugin.",
                ),
            ],
        )

    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        _ = force
        plan = self.plan(config)
        return InstallPlan(
            target=plan.target,
            display_name=plan.display_name,
            protocol_note=plan.protocol_note,
            docs=plan.docs,
            result_kind="stub",
            steps=plan.steps,
            availability=plan.availability,
        )


def atomic_write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            "w",
            delete=False,
            dir=path.parent,
            encoding="utf-8",
            prefix=f".{path.name}.",
            suffix=".tmp",
        ) as temporary:
            temporary_path = Path(temporary.name)
            temporary.write(text)
            temporary.flush()
            os.fsync(temporary.fileno())
        temporary_path.replace(path)
    except Exception:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)
        raise


def backup_file(config: HieronymusConfig, source: Path, *, agent: str, extension: str) -> Path:
    config.backups_root.mkdir(parents=True, exist_ok=True)
    suffix = extension.removeprefix(".")
    for attempt in range(100):
        unique = f"{time.time_ns()}-{os.getpid()}-{attempt}"
        backup = config.backups_root / f"{agent}-{source.stem}-{unique}.{suffix}"
        try:
            descriptor = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL)
        except FileExistsError:
            continue
        else:
            os.close(descriptor)
            try:
                shutil.copy2(source, backup)
            except Exception:
                backup.unlink(missing_ok=True)
                raise
            return backup
    raise FileExistsError(f"could not create unique backup path for {source}")
