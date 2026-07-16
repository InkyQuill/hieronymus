"""Install the bundled Hieronymus skills into a project workspace."""

from __future__ import annotations

import shutil
import tempfile
from dataclasses import dataclass
from pathlib import Path

from hieronymus.agent_assets import asset_map

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
        staged = _stage_skill_directories(roots, assets)
        _replace_staged_skill_directories(staged)
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
        paths = tuple(root / relative_path for root in roots for relative_path in skill_assets())
    else:
        paths = tuple(
            skill_directory for root in roots for skill_directory in _owned_skill_directories(root)
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


def _validate_install_destinations(roots: tuple[Path, ...], assets: dict[str, str]) -> None:
    """Validate every destination before staging or replacing any skill."""
    for root in roots:
        for relative_path in assets:
            destination = root
            for component in Path(relative_path).parts:
                destination /= component
                if destination.is_symlink():
                    raise ValueError(
                        f"project skill destination must not contain a symlink: {destination}"
                    )
        for directory_name in _skill_assets_by_directory(assets):
            destination = root / directory_name
            if destination.exists() and not _is_owned_skill_directory(destination):
                raise ValueError(f"project skill destination is not an owned skill: {destination}")


def _skill_assets_by_directory(assets: dict[str, str]) -> dict[str, dict[Path, str]]:
    """Group bundled assets by their direct child skill directory."""
    grouped: dict[str, dict[Path, str]] = {}
    for relative_path, text in assets.items():
        path = Path(relative_path)
        if path.is_absolute() or len(path.parts) < 2 or path.parts[0].startswith("."):
            raise ValueError(f"invalid bundled project skill path: {relative_path}")
        if any(part in {".", ".."} for part in path.parts):
            raise ValueError(f"invalid bundled project skill path: {relative_path}")
        grouped.setdefault(path.parts[0], {})[Path(*path.parts[1:])] = text
    for directory_name, files in grouped.items():
        if not directory_name.startswith("hieronymus-") or Path("SKILL.md") not in files:
            raise ValueError(f"invalid bundled project skill directory: {directory_name}")
    return grouped


def _stage_skill_directories(
    roots: tuple[Path, ...], assets: dict[str, str]
) -> tuple[tuple[Path, Path], ...]:
    """Stage complete skill directories as siblings beneath each skills root."""
    grouped_assets = _skill_assets_by_directory(assets)
    staged: list[tuple[Path, Path]] = []
    try:
        for root in roots:
            root.mkdir(parents=True, exist_ok=True)
            for directory_name, files in grouped_assets.items():
                stage = Path(tempfile.mkdtemp(prefix=f".{directory_name}.stage-", dir=root))
                staged.append((root / directory_name, stage))
                for relative_path, text in files.items():
                    destination = stage / relative_path
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_text(text, encoding="utf-8")
    except OSError:
        _remove_staged_directories(staged)
        raise
    return tuple(staged)


def _replace_staged_skill_directories(staged: tuple[tuple[Path, Path], ...]) -> None:
    """Replace owned directories by rename, retaining backups until all swaps succeed."""
    replaced: list[tuple[Path, Path, Path | None]] = []
    try:
        for destination, stage in staged:
            backup: Path | None = None
            if destination.exists():
                backup = Path(
                    tempfile.mkdtemp(prefix=f".{destination.name}.backup-", dir=destination.parent)
                )
                backup.rmdir()
                destination.replace(backup)
            replaced.append((destination, stage, backup))
            stage.replace(destination)
    except OSError:
        for destination, stage, backup in reversed(replaced):
            if destination.exists():
                destination.replace(stage)
            if backup is not None and backup.exists():
                backup.replace(destination)
        _remove_staged_directories(staged)
        raise
    for _, _, backup in replaced:
        if backup is not None:
            shutil.rmtree(backup)


def _remove_staged_directories(
    staged: tuple[tuple[Path, Path], ...] | list[tuple[Path, Path]],
) -> None:
    for _, stage in staged:
        if stage.exists():
            shutil.rmtree(stage)


def _is_owned_skill_directory(path: Path) -> bool:
    return (
        path.name.startswith("hieronymus-")
        and path.is_dir()
        and not path.is_symlink()
        and (path / "SKILL.md").is_file()
        and not (path / "SKILL.md").is_symlink()
    )


def _owned_skill_directories(root: Path) -> tuple[Path, ...]:
    if not root.exists():
        return ()
    return tuple(child for child in sorted(root.iterdir()) if _is_owned_skill_directory(child))
