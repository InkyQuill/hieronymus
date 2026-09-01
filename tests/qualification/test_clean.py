from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools.qualification.clean import cleanup_targets  # noqa: E402


def test_cleanup_never_targets_repository_or_model_by_default() -> None:
    targets = cleanup_targets(ROOT)
    assert ROOT not in targets
    assert ROOT / "qualification/.artifacts/models" not in targets
    assert all(path.is_relative_to(ROOT / "qualification/.artifacts") for path in targets)
    assert {path.name for path in targets} == {
        "cargo-target",
        "frontend-dist",
        "install",
        "logs",
        "work",
    }


def test_cleanup_model_requires_explicit_include() -> None:
    targets = cleanup_targets(ROOT, include_model=True)
    assert ROOT / "qualification/.artifacts/models" in targets


def test_cleanup_rejects_symlink_targets(tmp_path: Path) -> None:
    root = tmp_path / "repo"
    artifacts = root / "qualification/.artifacts"
    artifacts.mkdir(parents=True)
    outside = tmp_path / "outside"
    outside.mkdir()
    (artifacts / "work").symlink_to(outside, target_is_directory=True)
    with pytest.raises(ValueError, match="symlink"):
        cleanup_targets(root)


def test_cleanup_cli_is_dry_run_by_default_and_apply_is_idempotent(tmp_path: Path) -> None:
    root = tmp_path / "repo"
    artifacts = root / "qualification/.artifacts"
    for name in ("work", "logs", "install", "cargo-target", "frontend-dist", "models"):
        directory = artifacts / name
        directory.mkdir(parents=True)
        (directory / "nested").mkdir()
        (directory / "nested/file").write_text("temporary", encoding="utf-8")

    env = dict(os.environ)
    env["PYTHONPATH"] = str(ROOT)
    dry = subprocess.run(
        (sys.executable, "-m", "tools.qualification.clean", "--repo-root", str(root)),
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    assert (artifacts / "work/nested/file").exists()
    assert "dry-run" in dry.stdout
    assert str(tmp_path) not in dry.stdout

    for _ in range(2):
        subprocess.run(
            (
                sys.executable,
                "-m",
                "tools.qualification.clean",
                "--repo-root",
                str(root),
                "--apply",
            ),
            env=env,
            check=True,
            capture_output=True,
            text=True,
        )
    assert all(not (artifacts / name).exists() for name in cleanup_names())
    assert (artifacts / "models/nested/file").exists()


def cleanup_names() -> tuple[str, ...]:
    return "work", "logs", "install", "cargo-target", "frontend-dist"
