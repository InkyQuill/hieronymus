"""Byte-exact, repository-relative qualification input fingerprints."""

from __future__ import annotations

import hashlib
import os
import stat
from pathlib import Path, PurePosixPath, PureWindowsPath

COMMON_FINGERPRINT_INPUTS = (
    "qualification/prerequisites.json",
    "qualification/rust-toolchain.toml",
    "tools/qualification/model.py",
    "tools/qualification/fingerprint.py",
    "tools/qualification/redaction.py",
    "tools/qualification/validate.py",
    "tools/qualification/render.py",
    "tools/qualification/acquire.py",
    "tools/qualification/process.py",
    "tools/qualification/clean.py",
)

_DOMAIN = b"hieronymus qualification input fingerprint\x00v1\x00"


def fingerprint_inputs(repo_root: Path, paths: tuple[str, ...]) -> str:
    """Hash sorted canonical relative names and raw file bytes without ambiguity."""
    if type(paths) is not tuple:
        raise ValueError("fingerprint paths must be an ordered tuple")
    if not repo_root.is_dir() or repo_root.is_symlink():
        raise ValueError("fingerprint repository root must be a non-symlink directory")

    canonical = tuple(_canonical_relative_path(path) for path in paths)
    if len(canonical) != len(set(canonical)):
        raise ValueError("fingerprint paths must not contain duplicates")

    digest = hashlib.sha256(_DOMAIN)
    for relative in sorted(canonical):
        name = relative.encode("utf-8")
        content = _read_regular_file(repo_root, relative)
        digest.update(len(name).to_bytes(8, "big"))
        digest.update(name)
        digest.update(len(content).to_bytes(16, "big"))
        digest.update(content)
    return digest.hexdigest()


def _canonical_relative_path(value: object) -> str:
    if type(value) is not str or not value:
        raise ValueError("fingerprint paths must be nonempty strings")
    windows = PureWindowsPath(value)
    if windows.is_absolute() or windows.drive or windows.root:
        raise ValueError("fingerprint paths must be relative")
    if "\\" in value:
        raise ValueError("fingerprint paths must use canonical POSIX separators")

    path = PurePosixPath(value)
    if path.is_absolute():
        raise ValueError("fingerprint paths must be relative")
    if path.as_posix() != value or any(part in (".", "..") for part in path.parts):
        raise ValueError("fingerprint paths must be canonical relative paths")
    return value


def _read_regular_file(repo_root: Path, relative: str) -> bytes:
    path = repo_root / relative
    current = repo_root
    try:
        for part in PurePosixPath(relative).parts:
            current = current / part
            metadata = current.lstat()
            if stat.S_ISLNK(metadata.st_mode):
                raise ValueError("fingerprint inputs must be non-symlink regular files")
        if not stat.S_ISREG(metadata.st_mode):
            raise ValueError("fingerprint inputs must be existing regular files")
    except FileNotFoundError as error:
        raise ValueError("fingerprint inputs must be existing regular files") from error

    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except (FileNotFoundError, OSError) as error:
        raise ValueError("fingerprint inputs must be existing regular files") from error
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise ValueError("fingerprint inputs must be existing regular files")
        with os.fdopen(descriptor, "rb", closefd=False) as source:
            return source.read()
    finally:
        os.close(descriptor)
