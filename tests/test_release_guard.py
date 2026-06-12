from __future__ import annotations

from pathlib import Path

import pytest

from hieronymus.release_guard import ReleaseGuardError, validate_alpha_release_metadata


def write_project(root: Path, *, pyproject_version: str, module_version: str) -> None:
    (root / "src" / "hieronymus").mkdir(parents=True)
    (root / "pyproject.toml").write_text(
        f'[project]\nname = "hieronymus"\nversion = "{pyproject_version}"\n',
        encoding="utf-8",
    )
    (root / "src" / "hieronymus" / "__init__.py").write_text(
        f'"""Hieronymus translation memory."""\n\n__version__ = "{module_version}"\n',
        encoding="utf-8",
    )


def test_validate_alpha_release_metadata_accepts_zero_major_version(tmp_path: Path) -> None:
    write_project(tmp_path, pyproject_version="0.2.0", module_version="0.2.0")

    assert validate_alpha_release_metadata(tmp_path) == "0.2.0"


def test_validate_alpha_release_metadata_rejects_one_major_version(tmp_path: Path) -> None:
    write_project(tmp_path, pyproject_version="1.0.0", module_version="1.0.0")

    with pytest.raises(ReleaseGuardError, match="alpha releases must stay on 0.x"):
        validate_alpha_release_metadata(tmp_path)


def test_validate_alpha_release_metadata_rejects_mismatched_versions(tmp_path: Path) -> None:
    write_project(tmp_path, pyproject_version="0.2.0", module_version="0.3.0")

    with pytest.raises(ReleaseGuardError, match="version mismatch"):
        validate_alpha_release_metadata(tmp_path)
