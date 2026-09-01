"""Descriptor-safe cleanup for bounded Rust qualification artifacts."""

from __future__ import annotations

import argparse
import os
import stat
from dataclasses import dataclass
from pathlib import Path

_DEFAULT_TARGETS = ("cargo-target", "frontend-dist", "install", "logs", "work")
_MODEL_TARGET = "models"
_ROOT = Path(__file__).resolve().parents[2]


def _reject_symlink(path: Path, *, label: str) -> None:
    if path.is_symlink():
        raise ValueError(f"{label} must not be a symlink")


def _artifact_root(repo_root: Path) -> Path:
    lexical_repo = repo_root.absolute()
    _reject_symlink(lexical_repo, label="repository root")
    if not lexical_repo.is_dir():
        raise ValueError("repository root must be a directory")
    if lexical_repo != lexical_repo.resolve(strict=True):
        raise ValueError("repository root must be canonical")
    qualification = lexical_repo / "qualification"
    _reject_symlink(qualification, label="qualification root")
    artifacts = qualification / ".artifacts"
    _reject_symlink(artifacts, label="artifact root")
    return artifacts


def cleanup_targets(repo_root: Path, include_model: bool = False) -> tuple[Path, ...]:
    """Return the finite cleanup allowlist after rejecting unsafe path shapes."""
    artifacts = _artifact_root(repo_root)
    names = (*_DEFAULT_TARGETS, *((_MODEL_TARGET,) if include_model else ()))
    targets = tuple(artifacts / name for name in names)
    home = Path.home().resolve(strict=True)
    translation = home / "Yandex.Disk/Translation"
    for target in targets:
        if target in (repo_root.absolute(), artifacts, home):
            raise ValueError("cleanup target is an unsafe root")
        if target.is_relative_to(translation):
            raise ValueError("cleanup target is inside the translation workspace")
        if not target.is_relative_to(artifacts):  # pragma: no cover
            raise ValueError("cleanup target escaped the artifact root")
        _reject_symlink(target, label=f"cleanup target {target.name}")
    return targets


def _open_directory(name: str, parent_fd: int) -> int:
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    return os.open(name, flags, dir_fd=parent_fd)


def _mount_id(fd: int) -> int:
    """Read the Linux mount identity so same-device bind mounts cannot be crossed."""
    try:
        lines = Path(f"/proc/self/fdinfo/{fd}").read_text(encoding="ascii").splitlines()
    except OSError as exc:  # pragma: no cover - qualification target is Linux procfs
        raise ValueError("cleanup requires Linux mount identity data") from exc
    for line in lines:
        if line.startswith("mnt_id:"):
            try:
                return int(line.split(":", 1)[1].strip())
            except ValueError as exc:  # pragma: no cover
                raise ValueError("cleanup found malformed mount identity data") from exc
    raise ValueError("cleanup requires a mount identity for every opened directory")


def _identity(info: os.stat_result) -> tuple[int, int, int, int]:
    return info.st_mode, info.st_dev, info.st_ino, info.st_nlink


def _entry_identity_at(parent_fd: int, name: str) -> tuple[int, int, int, int]:
    return _identity(os.stat(name, dir_fd=parent_fd, follow_symlinks=False))


def _same_identity(
    current: tuple[int, int, int, int],
    expected: tuple[int, int, int, int],
    *,
    children_removed: bool = False,
) -> bool:
    if children_removed and stat.S_ISDIR(expected[0]):
        return current[:3] == expected[:3]
    return current == expected


@dataclass(frozen=True, slots=True)
class _TreeNode:
    name: str
    identity: tuple[int, int, int, int]
    mount_id: int | None
    children: tuple[_TreeNode, ...]


def _validate_open_tree(directory_fd: int, *, mount_id: int) -> tuple[_TreeNode, ...]:
    nodes: list[_TreeNode] = []
    for entry in sorted(os.scandir(directory_fd), key=lambda item: item.name):
        identity = _entry_identity_at(directory_fd, entry.name)
        mode, _, _, links = identity
        if stat.S_ISLNK(mode):
            raise ValueError(f"cleanup refuses symlink entry {entry.name}")
        if stat.S_ISDIR(mode):
            child_fd = _open_directory(entry.name, directory_fd)
            try:
                if _identity(os.fstat(child_fd)) != identity:
                    raise ValueError(f"cleanup entry {entry.name} changed during validation")
                child_mount = _mount_id(child_fd)
                if child_mount != mount_id:
                    raise ValueError(f"cleanup refuses mounted directory {entry.name}")
                children = _validate_open_tree(child_fd, mount_id=mount_id)
            finally:
                os.close(child_fd)
            nodes.append(_TreeNode(entry.name, identity, child_mount, children))
        elif stat.S_ISREG(mode):
            if links != 1:
                raise ValueError(f"cleanup refuses hard-linked file {entry.name}")
            nodes.append(_TreeNode(entry.name, identity, None, ()))
        else:
            raise ValueError(f"cleanup refuses special entry {entry.name}")
    return tuple(nodes)


def _remove_validated_tree(
    directory_fd: int, nodes: tuple[_TreeNode, ...], *, mount_id: int
) -> None:
    for node in nodes:
        if not _same_identity(_entry_identity_at(directory_fd, node.name), node.identity):
            raise ValueError(f"cleanup entry {node.name} changed before deletion")
        mode = node.identity[0]
        if stat.S_ISDIR(mode):
            child_fd = _open_directory(node.name, directory_fd)
            try:
                if _identity(os.fstat(child_fd)) != node.identity:
                    raise ValueError(f"cleanup entry {node.name} changed before deletion")
                if _mount_id(child_fd) != mount_id:
                    raise ValueError(f"cleanup refuses mounted directory {node.name}")
                _remove_validated_tree(child_fd, node.children, mount_id=mount_id)
            finally:
                os.close(child_fd)
            if not _same_identity(
                _entry_identity_at(directory_fd, node.name),
                node.identity,
                children_removed=True,
            ):
                raise ValueError(f"cleanup entry {node.name} changed before deletion")
            os.rmdir(node.name, dir_fd=directory_fd)
        else:
            os.unlink(node.name, dir_fd=directory_fd)


def _open_artifacts(repo_root: Path) -> int | None:
    artifacts = _artifact_root(repo_root)
    if not artifacts.exists():
        return None
    root_fd = os.open(repo_root.absolute(), _open_flags())
    try:
        qualification_fd = _open_directory("qualification", root_fd)
    finally:
        os.close(root_fd)
    try:
        return _open_directory(".artifacts", qualification_fd)
    finally:
        os.close(qualification_fd)


def _open_flags() -> int:
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    return flags


def _validated_targets(
    repo_root: Path, *, include_model: bool
) -> tuple[tuple[str, ...], int | None, tuple[_TreeNode, ...]]:
    targets = cleanup_targets(repo_root, include_model=include_model)
    names = tuple(path.name for path in targets)
    artifacts_fd = _open_artifacts(repo_root)
    if artifacts_fd is None:
        return names, None, ()
    artifact_device = os.fstat(artifacts_fd).st_dev
    artifact_mount = _mount_id(artifacts_fd)
    plans: list[_TreeNode] = []
    try:
        for name in names:
            try:
                identity = _entry_identity_at(artifacts_fd, name)
            except FileNotFoundError:
                continue
            if not stat.S_ISDIR(identity[0]):
                raise ValueError(f"cleanup target {name} is not a real directory")
            target_fd = _open_directory(name, artifacts_fd)
            try:
                if _identity(os.fstat(target_fd)) != identity:
                    raise ValueError(f"cleanup target {name} changed during validation")
                if (
                    os.fstat(target_fd).st_dev != artifact_device
                    or _mount_id(target_fd) != artifact_mount
                ):
                    raise ValueError(f"cleanup refuses mounted target {name}")
                children = _validate_open_tree(target_fd, mount_id=artifact_mount)
            finally:
                os.close(target_fd)
            plans.append(_TreeNode(name, identity, artifact_mount, children))
    except BaseException:
        os.close(artifacts_fd)
        raise
    return names, artifacts_fd, tuple(plans)


def _apply_cleanup(repo_root: Path, *, include_model: bool) -> tuple[str, ...]:
    names, artifacts_fd, plans = _validated_targets(repo_root, include_model=include_model)
    if artifacts_fd is None:
        return names
    try:
        artifact_mount = _mount_id(artifacts_fd)
        for plan in plans:
            if not _same_identity(_entry_identity_at(artifacts_fd, plan.name), plan.identity):
                raise ValueError(f"cleanup target {plan.name} changed before deletion")
            target_fd = _open_directory(plan.name, artifacts_fd)
            try:
                if _identity(os.fstat(target_fd)) != plan.identity:
                    raise ValueError(f"cleanup target {plan.name} changed before deletion")
                if _mount_id(target_fd) != artifact_mount:
                    raise ValueError(f"cleanup refuses mounted target {plan.name}")
                _remove_validated_tree(target_fd, plan.children, mount_id=artifact_mount)
            finally:
                os.close(target_fd)
            if not _same_identity(
                _entry_identity_at(artifacts_fd, plan.name),
                plan.identity,
                children_removed=True,
            ):
                raise ValueError(f"cleanup target {plan.name} changed before deletion")
            os.rmdir(plan.name, dir_fd=artifacts_fd)
    finally:
        os.close(artifacts_fd)
    return names


def _dry_run_cleanup(repo_root: Path, *, include_model: bool) -> tuple[str, ...]:
    names, artifacts_fd, _ = _validated_targets(repo_root, include_model=include_model)
    if artifacts_fd is not None:
        os.close(artifacts_fd)
    return names


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="remove allowlisted transient data")
    parser.add_argument("--include-model", action="store_true", help="also remove acquired models")
    parser.add_argument("--repo-root", type=Path, default=_ROOT, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if args.apply:
        names = _apply_cleanup(args.repo_root, include_model=args.include_model)
        mode = "applied"
    else:
        names = _dry_run_cleanup(args.repo_root, include_model=args.include_model)
        mode = "dry-run"
    print(f"{mode}: {', '.join(names)}")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
