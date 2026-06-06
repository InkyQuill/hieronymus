from __future__ import annotations

import os
import shutil
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from hieronymus.config import HieronymusConfig

AGENT_WORKFLOW_SPEC = "docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md"


@dataclass(frozen=True)
class AgentAvailability:
    target: str
    display_name: str
    available: bool
    installed: bool
    detect_paths: list[str]
    config_paths: list[str]
    install_path: str
    reason: str

    def to_json_dict(self) -> dict[str, object]:
        return {
            "target": self.target,
            "display_name": self.display_name,
            "available": self.available,
            "installed": self.installed,
            "detect_paths": self.detect_paths,
            "config_paths": self.config_paths,
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


def any_path_exists(paths: tuple[str, ...]) -> bool:
    return any(expand_user(path).exists() for path in paths)


class BaseAgentPlugin:
    name: str
    display_name: str
    detect_paths: tuple[str, ...]
    config_paths: tuple[str, ...]
    docs = AGENT_WORKFLOW_SPEC
    protocol_note: str

    @property
    def detect_path(self) -> str:
        return self.detect_paths[0]

    @property
    def config_path(self) -> str:
        return self.config_paths[0]

    def availability(self, config: HieronymusConfig) -> AgentAvailability:
        install_path = config.agent_plugins_root / self.name
        available = any_path_exists(self.detect_paths)
        installed = install_path.exists()
        return AgentAvailability(
            target=self.name,
            display_name=self.display_name,
            available=available,
            installed=installed,
            detect_paths=list(self.detect_paths),
            config_paths=list(self.config_paths),
            install_path=str(install_path),
            reason="host detected" if available else "host config path not found",
        )

    def plan(self, config: HieronymusConfig) -> InstallPlan:
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
        return self.plan(config)


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
