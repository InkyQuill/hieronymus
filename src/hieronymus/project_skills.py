"""Install the bundled Hieronymus skills into a project workspace."""

from __future__ import annotations

import shutil
from dataclasses import dataclass
from pathlib import Path

from hieronymus.agent_assets import asset_map
from hieronymus.agent_plugins.base import atomic_write_text

_TARGET_DIRECTORIES = {
    "agents": ".agents/skills",
    "claude": ".claude/skills",
}
_TARGET_ORDER = tuple(_TARGET_DIRECTORIES)


@dataclass(frozen=True)
class ProjectSkillPlan:
    """The filesystem paths a project skill operation will affect."""

    action: str
    targets: tuple[str, ...]
    dry_run: bool
    paths: tuple[Path, ...]


def skill_assets() -> dict[str, str]:
    """Return bundled skill paths relative to a target's ``skills`` directory."""
    return {
        path.removeprefix("skills/"): text
        for path, text in asset_map().items()
        if path.startswith("skills/") and path.endswith("/SKILL.md")
    }


def install_project_skills(
    workspace: Path, targets: tuple[str, ...], *, dry_run: bool = False
) -> ProjectSkillPlan:
    """Install every bundled skill into each selected project target."""
    plan = plan_project_skills(workspace, "install", targets, dry_run=dry_run)
    if not dry_run:
        assets = skill_assets()
        roots = target_roots(workspace, plan.targets)
        _validate_install_destinations(roots, assets)
        for root in roots:
            for relative_path, text in assets.items():
                atomic_write_text(root / relative_path, text)
    return plan


def uninstall_project_skills(
    workspace: Path, targets: tuple[str, ...], *, dry_run: bool = False
) -> ProjectSkillPlan:
    """Remove only owned skill directories from each selected project target."""
    plan = plan_project_skills(workspace, "uninstall", targets, dry_run=dry_run)
    if not dry_run:
        for path in plan.paths:
            shutil.rmtree(path)
    return plan


def plan_project_skills(
    workspace: Path, action: str, targets: tuple[str, ...], *, dry_run: bool = False
) -> ProjectSkillPlan:
    """Validate an operation and describe the paths it will modify."""
    if action not in {"install", "uninstall"}:
        raise ValueError(f"unsupported project skill action: {action}")

    normalized_targets = _normalize_targets(targets)
    roots = target_roots(workspace, normalized_targets)
    if action == "install":
        paths = tuple(
            root / relative_path
            for root in roots
            for relative_path in skill_assets()
        )
    else:
        paths = tuple(
            skill_directory
            for root in roots
            for skill_directory in _owned_skill_directories(root)
        )
    return ProjectSkillPlan(action, normalized_targets, dry_run, paths)


def target_roots(workspace: Path, targets: tuple[str, ...]) -> tuple[Path, ...]:
    """Return validated project skill target directories."""
    if workspace.is_symlink():
        raise ValueError(f"workspace must not be a symlink: {workspace}")
    if not workspace.is_dir():
        raise ValueError(f"workspace must be a directory: {workspace}")

    roots = tuple(workspace / _TARGET_DIRECTORIES[target] for target in targets)
    for root in roots:
        _validate_target_root(root)
    return roots


def _normalize_targets(targets: tuple[str, ...]) -> tuple[str, ...]:
    unknown_targets = set(targets).difference(_TARGET_DIRECTORIES)
    if unknown_targets:
        unknown = ", ".join(sorted(unknown_targets))
        raise ValueError(f"unsupported project skill target: {unknown}")
    return tuple(target for target in _TARGET_ORDER if target in targets)


def _validate_target_root(root: Path) -> None:
    if root.parent.is_symlink() or root.is_symlink():
        raise ValueError(f"target root must not be a symlink: {root}")
    if root.parent.exists() and not root.parent.is_dir():
        raise ValueError(f"target parent must be a directory: {root.parent}")
    if root.exists() and not root.is_dir():
        raise ValueError(f"target root must be a directory: {root}")


def _validate_install_destinations(
    roots: tuple[Path, ...], assets: dict[str, str]
) -> None:
    """Reject symlinks in every bundled destination before writing anything."""
    for root in roots:
        for relative_path in assets:
            destination = root
            for component in Path(relative_path).parts:
                destination /= component
                if destination.is_symlink():
                    raise ValueError(
                        "project skill destination must not contain a symlink: "
                        f"{destination}"
                    )


def _owned_skill_directories(root: Path) -> tuple[Path, ...]:
    if not root.exists():
        return ()
    return tuple(
        child
        for child in sorted(root.iterdir())
        if (
            child.name.startswith("hieronymus-")
            and child.is_dir()
            and not child.is_symlink()
            and (child / "SKILL.md").is_file()
            and not (child / "SKILL.md").is_symlink()
        )
    )
