"""Descriptor-safe cleanup for bounded Rust qualification artifacts."""

from __future__ import annotations

import argparse
import os
import stat
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


def _remove_open_tree(directory_fd: int, *, device: int) -> None:
    for entry in os.scandir(directory_fd):
        info = entry.stat(follow_symlinks=False)
        if stat.S_ISLNK(info.st_mode):
            raise ValueError(f"cleanup refuses symlink entry {entry.name}")
        if stat.S_ISDIR(info.st_mode):
            child_fd = _open_directory(entry.name, directory_fd)
            try:
                if os.fstat(child_fd).st_dev != device:
                    raise ValueError(f"cleanup refuses mounted directory {entry.name}")
                _remove_open_tree(child_fd, device=device)
            finally:
                os.close(child_fd)
            os.rmdir(entry.name, dir_fd=directory_fd)
        elif stat.S_ISREG(info.st_mode):
            os.unlink(entry.name, dir_fd=directory_fd)
        else:
            raise ValueError(f"cleanup refuses special entry {entry.name}")


def _apply_cleanup(repo_root: Path, *, include_model: bool) -> tuple[str, ...]:
    targets = cleanup_targets(repo_root, include_model=include_model)
    artifacts = _artifact_root(repo_root)
    if not artifacts.exists():
        return tuple(path.name for path in targets)
    root_flags = os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        root_flags |= os.O_NOFOLLOW
    repo_fd = os.open(repo_root.absolute(), root_flags)
    try:
        qualification_fd = _open_directory("qualification", repo_fd)
    finally:
        os.close(repo_fd)
    try:
        artifacts_fd = _open_directory(".artifacts", qualification_fd)
    finally:
        os.close(qualification_fd)
    try:
        artifact_device = os.fstat(artifacts_fd).st_dev
        for target in targets:
            try:
                info = os.stat(target.name, dir_fd=artifacts_fd, follow_symlinks=False)
            except FileNotFoundError:
                continue
            if not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode):
                raise ValueError(f"cleanup target {target.name} is not a real directory")
            target_fd = _open_directory(target.name, artifacts_fd)
            try:
                if os.fstat(target_fd).st_dev != artifact_device:
                    raise ValueError(f"cleanup refuses mounted target {target.name}")
                _remove_open_tree(target_fd, device=artifact_device)
            finally:
                os.close(target_fd)
            os.rmdir(target.name, dir_fd=artifacts_fd)
    finally:
        os.close(artifacts_fd)
    return tuple(path.name for path in targets)


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
        names = tuple(
            path.name for path in cleanup_targets(args.repo_root, include_model=args.include_model)
        )
        mode = "dry-run"
    print(f"{mode}: {', '.join(names)}")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
