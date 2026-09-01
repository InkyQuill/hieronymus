from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import tools.qualification.clean as clean_module  # noqa: E402
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


@pytest.mark.parametrize("kind", ["symlink", "fifo", "hardlink"])
def test_cleanup_dry_run_deeply_rejects_every_apply_hazard(tmp_path: Path, kind: str) -> None:
    root = tmp_path / "repo"
    nested = root / "qualification/.artifacts/work/nested"
    nested.mkdir(parents=True)
    outside = tmp_path / "outside"
    outside.write_text("keep", encoding="utf-8")
    if kind == "symlink":
        (nested / "hazard").symlink_to(outside)
    elif kind == "fifo":
        os.mkfifo(nested / "hazard")
    else:
        os.link(outside, nested / "hazard")

    with pytest.raises(ValueError):
        clean_module.main(["--repo-root", str(root)])
    assert outside.read_text(encoding="utf-8") == "keep"
    assert nested.exists()


def test_cleanup_rejects_same_device_bind_mount_id_before_mutation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = tmp_path / "repo"
    nested = root / "qualification/.artifacts/work/nested"
    nested.mkdir(parents=True)
    (nested / "file").write_text("keep", encoding="utf-8")
    real_mount_id = clean_module._mount_id

    def injected(fd: int) -> int:
        path = Path(os.readlink(f"/proc/self/fd/{fd}"))
        value = real_mount_id(fd)
        return value + 1 if path.name == "nested" else value

    monkeypatch.setattr(clean_module, "_mount_id", injected)
    with pytest.raises(ValueError, match="mounted"):
        clean_module.main(["--repo-root", str(root), "--apply"])
    assert (nested / "file").read_text(encoding="utf-8") == "keep"


def test_cleanup_revalidates_identity_before_deletion(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = tmp_path / "repo"
    target = root / "qualification/.artifacts/work"
    target.mkdir(parents=True)
    victim = target / "file"
    victim.write_text("keep", encoding="utf-8")
    original = clean_module._entry_identity_at
    calls = 0

    def swapped(parent_fd: int, name: str) -> object:
        nonlocal calls
        identity = original(parent_fd, name)
        if name == "file":
            calls += 1
            if calls > 1:
                return (*identity[:-1], identity[-1] + 1)
        return identity

    monkeypatch.setattr(clean_module, "_entry_identity_at", swapped)
    with pytest.raises(ValueError, match="changed"):
        clean_module.main(["--repo-root", str(root), "--apply"])
    assert victim.read_text(encoding="utf-8") == "keep"


def cleanup_names() -> tuple[str, ...]:
    return "work", "logs", "install", "cargo-target", "frontend-dist"
