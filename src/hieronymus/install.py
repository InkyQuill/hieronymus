from __future__ import annotations

import shutil
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from hieronymus.config import HieronymusConfig

AGENT_WORKFLOW_SPEC = "docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md"


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

    def to_json_dict(self) -> dict[str, object]:
        return {
            "target": self.target,
            "display_name": self.display_name,
            "protocol_note": self.protocol_note,
            "docs": self.docs,
            "result_kind": self.result_kind,
            "steps": [step.to_json_dict() for step in self.steps],
        }


@dataclass(frozen=True)
class InstallTarget:
    name: str
    display_name: str
    detect_path: str
    config_path: str
    protocol_note: str
    docs: str


TARGETS = [
    InstallTarget(
        name="claude",
        display_name="Claude Code",
        detect_path="~/.claude.json",
        config_path="~/.claude.json",
        protocol_note=(
            "Claude Code integration uses MCP; host-specific hooks are deferred to a later pass."
        ),
        docs=AGENT_WORKFLOW_SPEC,
    ),
    InstallTarget(
        name="codex",
        display_name="Codex",
        detect_path="~/.codex",
        config_path="~/.codex/config.toml",
        protocol_note="Codex integration uses MCP; real hooks and skills are a follow-up.",
        docs=AGENT_WORKFLOW_SPEC,
    ),
    InstallTarget(
        name="openclaw",
        display_name="OpenClaw",
        detect_path="~/.openclaw",
        config_path="~/.openclaw/openclaw.json",
        protocol_note="OpenClaw integration is reserved for future MCP configuration support.",
        docs=AGENT_WORKFLOW_SPEC,
    ),
    InstallTarget(
        name="opencode",
        display_name="opencode",
        detect_path="~/.config/opencode",
        config_path="~/.config/opencode/plugin.json",
        protocol_note=(
            "opencode integration is reserved for future MCP plugin configuration support."
        ),
        docs=AGENT_WORKFLOW_SPEC,
    ),
    InstallTarget(
        name="gemini",
        display_name="Gemini CLI",
        detect_path="~/.gemini",
        config_path="~/.gemini/settings.json",
        protocol_note="Gemini CLI integration is reserved for future MCP configuration support.",
        docs=AGENT_WORKFLOW_SPEC,
    ),
    InstallTarget(
        name="pi",
        display_name="Pi",
        detect_path="~/.pi",
        config_path="~/.pi/config.json",
        protocol_note="Pi is a reserved future integration target.",
        docs=AGENT_WORKFLOW_SPEC,
    ),
    InstallTarget(
        name="hermes",
        display_name="Hermes",
        detect_path="~/.hermes",
        config_path="~/.hermes/config.json",
        protocol_note="Hermes is a reserved future integration target.",
        docs=AGENT_WORKFLOW_SPEC,
    ),
]


def known_targets() -> list[str]:
    return [target.name for target in TARGETS]


def resolve_target(name: str) -> InstallTarget:
    normalized = name.lower()
    for target in TARGETS:
        if target.name == normalized:
            return target
    supported = ", ".join(known_targets())
    raise ValueError(f"unknown install target: {name}; supported targets: {supported}")


def plan_install(config: HieronymusConfig, target_name: str) -> InstallPlan:
    _ = config
    target = resolve_target(target_name)
    return InstallPlan(
        target=target.name,
        display_name=target.display_name,
        protocol_note=target.protocol_note,
        docs=target.docs,
        result_kind="stub",
        steps=[
            InstallStep(
                action="inspect",
                path=target.config_path,
                description=f"Detect existing {target.display_name} MCP configuration.",
            ),
            InstallStep(
                action="defer",
                path=AGENT_WORKFLOW_SPEC,
                description=(
                    f"Real {target.display_name} hooks and skills are specified separately."
                ),
            ),
        ],
    )


def atomic_write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    temporary.write_text(text, encoding="utf-8")
    temporary.replace(path)


def backup_file(config: HieronymusConfig, source: Path, *, agent: str, extension: str) -> Path:
    config.backups_root.mkdir(parents=True, exist_ok=True)
    timestamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%S%fZ")
    suffix = extension.removeprefix(".")
    backup = config.backups_root / f"{agent}-{source.stem}-{timestamp}.{suffix}"
    shutil.copy2(source, backup)
    return backup
