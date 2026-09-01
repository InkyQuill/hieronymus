from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from dataclasses import asdict
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.qualification.factories import fake_child_and_grandchild  # noqa: E402
from tools.qualification.process import (  # noqa: E402
    ToolRoots,
    discover_tool_roots,
    run_owned_process,
    safe_subprocess_env,
)


def _fake_tool_roots(tmp_path: Path) -> ToolRoots:
    cargo_home = tmp_path / "fake-cargo-home"
    rustup_home = tmp_path / "fake-rustup-home"
    bin_dir = cargo_home / "bin"
    bin_dir.mkdir(parents=True)
    rustup_home.mkdir()
    resolved_target = Path(sys.executable).resolve()
    cargo_invocation = bin_dir / "cargo"
    rustup_invocation = bin_dir / "rustup"
    cargo_invocation.symlink_to(resolved_target)
    rustup_invocation.symlink_to(resolved_target)
    return ToolRoots(
        cargo_home=cargo_home.resolve(),
        rustup_home=rustup_home.resolve(),
        cargo_invocation=cargo_invocation.absolute(),
        cargo_resolved_target=resolved_target,
        rustup_invocation=rustup_invocation.absolute(),
        rustup_resolved_target=resolved_target,
    )


def test_discovery_preserves_shim_paths_without_resolving_them(tmp_path: Path) -> None:
    expected = _fake_tool_roots(tmp_path)
    original_env = {
        "HOME": str(tmp_path / "original-home"),
        "CARGO_HOME": str(expected.cargo_home),
        "RUSTUP_HOME": str(expected.rustup_home),
        "PATH": str(expected.cargo_invocation.parent),
    }
    actual = discover_tool_roots(original_env)
    assert actual == expected
    assert actual.cargo_invocation.name == "cargo"
    assert actual.cargo_invocation != actual.cargo_resolved_target


def test_discovery_uses_original_path_and_rejects_invalid_targets(tmp_path: Path) -> None:
    home = tmp_path / "home"
    cargo_home = home / ".cargo"
    rustup_home = home / ".rustup"
    path_bin = tmp_path / "original-bin"
    cargo_home.mkdir(parents=True)
    rustup_home.mkdir()
    path_bin.mkdir()
    for name in ("cargo", "rustup"):
        (path_bin / name).symlink_to(Path(sys.executable).resolve())

    roots = discover_tool_roots({"HOME": str(home), "PATH": str(path_bin)})
    assert roots.cargo_home == cargo_home.resolve()
    assert roots.cargo_invocation == (path_bin / "cargo").absolute()

    (path_bin / "cargo").unlink()
    (path_bin / "cargo").mkdir()
    with pytest.raises(ValueError, match="cargo"):
        discover_tool_roots({"HOME": str(home), "PATH": str(path_bin)})


def test_safe_environment_is_private_offline_and_scrubbed(tmp_path: Path) -> None:
    roots = _fake_tool_roots(tmp_path)
    env = safe_subprocess_env(
        tmp_path / "work",
        cargo_offline=True,
        tool_roots=roots,
        cargo_target_dir=tmp_path / "work/cargo-target",
    )

    assert env["CARGO_HOME"] == str(roots.cargo_home)
    assert env["RUSTUP_HOME"] == str(roots.rustup_home)
    assert env["CARGO_NET_OFFLINE"] == "true"
    assert env["RUSTUP_AUTO_INSTALL"] == "0"
    shim_parents = list(
        dict.fromkeys((str(roots.cargo_invocation.parent), str(roots.rustup_invocation.parent)))
    )
    assert env["PATH"].split(os.pathsep)[: len(shim_parents)] == shim_parents
    assert env["PATH"].split(os.pathsep)[len(shim_parents) :] == ["/usr/bin", "/bin"]
    for key in ("HOME", "TMPDIR", "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME"):
        path = Path(env[key])
        assert path.is_relative_to(tmp_path / "work")
        assert path.stat().st_mode & 0o077 == 0
    forbidden = (
        "TOKEN",
        "KEY",
        "SECRET",
        "PASSWORD",
        "PROXY",
        "COOKIE",
        "AUTH",
        "OPENAI",
        "ANTHROPIC",
        "OLLAMA",
    )
    assert not any(any(marker in key.upper() for marker in forbidden) for key in env)


def test_safe_environment_rejects_unbounded_or_symlinked_paths(tmp_path: Path) -> None:
    roots = _fake_tool_roots(tmp_path)
    work = tmp_path / "work"
    work.mkdir()
    outside = tmp_path / "outside"
    outside.mkdir()
    with pytest.raises(ValueError, match="cargo target"):
        safe_subprocess_env(
            work,
            cargo_offline=True,
            tool_roots=roots,
            cargo_target_dir=outside,
        )
    linked = tmp_path / "linked-work"
    linked.symlink_to(work, target_is_directory=True)
    with pytest.raises(ValueError, match="symlink"):
        safe_subprocess_env(
            linked,
            cargo_offline=True,
            tool_roots=roots,
            cargo_target_dir=linked / "cargo-target",
        )


def test_owned_process_streams_digests_without_retaining_output(tmp_path: Path) -> None:
    stdout = b"public-output\n"
    stderr = b"private-stderr\n"
    command = (
        sys.executable,
        "-c",
        f"import os; os.write(1, {stdout!r}); os.write(2, {stderr!r})",
    )
    receipt = run_owned_process(
        command,
        cwd=tmp_path,
        env={"PATH": "/usr/bin:/bin"},
        timeout_seconds=5,
        no_progress_seconds=2,
    )
    assert receipt.exit_code == 0
    assert not receipt.timed_out
    assert receipt.stdout_sha256 == hashlib.sha256(stdout).hexdigest()
    assert receipt.stderr_sha256 == hashlib.sha256(stderr).hexdigest()
    assert receipt.process_group_reaped and receipt.core_dumps_disabled
    serialized = json.dumps(asdict(receipt), sort_keys=True)
    assert "public-output" not in serialized
    assert "private-stderr" not in serialized
    assert str(tmp_path) not in serialized


def test_owned_process_disables_core_and_reaps_group(tmp_path: Path) -> None:
    roots = _fake_tool_roots(tmp_path)
    receipt = run_owned_process(
        fake_child_and_grandchild(tmp_path),
        cwd=tmp_path,
        env=safe_subprocess_env(
            tmp_path / "process-work",
            cargo_offline=True,
            tool_roots=roots,
            cargo_target_dir=tmp_path / "process-work/cargo-target",
        ),
        timeout_seconds=1,
        no_progress_seconds=1,
    )
    assert receipt.timed_out and receipt.core_dumps_disabled
    assert receipt.process_group_reaped and list(tmp_path.glob("core*")) == []
    serialized = json.dumps(asdict(receipt), sort_keys=True)
    assert all(str(path) not in serialized for path in asdict(roots).values())


def test_owned_process_no_progress_timeout_and_spawn_failure_are_bounded(tmp_path: Path) -> None:
    stalled = run_owned_process(
        (sys.executable, "-c", "import time; time.sleep(60)"),
        cwd=tmp_path,
        env={"PATH": "/usr/bin:/bin"},
        timeout_seconds=10,
        no_progress_seconds=1,
    )
    assert stalled.timed_out and stalled.duration_ms < 5_000
    assert stalled.process_group_reaped

    missing = run_owned_process(
        (str(tmp_path / "missing"),),
        cwd=tmp_path,
        env={"PATH": "/usr/bin:/bin"},
        timeout_seconds=1,
        no_progress_seconds=1,
    )
    assert missing.exit_code is None and not missing.timed_out
    assert missing.process_group_reaped and missing.core_dumps_disabled


def test_owned_process_times_out_when_child_closes_pipes_but_keeps_running(
    tmp_path: Path,
) -> None:
    receipt = run_owned_process(
        (
            sys.executable,
            "-c",
            "import os,time; os.close(1); os.close(2); time.sleep(60)",
        ),
        cwd=tmp_path,
        env={"PATH": "/usr/bin:/bin"},
        timeout_seconds=10,
        no_progress_seconds=1,
    )
    assert receipt.timed_out and receipt.duration_ms < 5_000
    assert receipt.process_group_reaped


@pytest.mark.skipif(
    os.environ.get("HIERONYMUS_QUALIFICATION_LIVE") != "1",
    reason="requires the explicitly acquired Rust 1.96.0 toolchain",
)
def test_sanitized_env_keeps_discovered_rust_toolchain(tmp_path: Path) -> None:
    original_env = dict(os.environ)
    lexical_cargo_home = Path(
        original_env.get("CARGO_HOME", str(Path(original_env["HOME"]) / ".cargo"))
    ).absolute()
    roots = discover_tool_roots(original_env)
    env = safe_subprocess_env(
        tmp_path,
        cargo_offline=True,
        tool_roots=roots,
        cargo_target_dir=tmp_path / "cargo-target",
    )
    assert Path(env["CARGO_HOME"]).resolve() == roots.cargo_home
    assert Path(env["RUSTUP_HOME"]).resolve() == roots.rustup_home
    assert Path(env["HOME"]).is_relative_to(tmp_path)
    assert roots.cargo_invocation == lexical_cargo_home / "bin/cargo"
    assert roots.cargo_invocation.name == "cargo"
    assert roots.cargo_resolved_target == roots.cargo_invocation.resolve(strict=True)
    assert roots.rustup_invocation == lexical_cargo_home / "bin/rustup"
    assert roots.rustup_invocation.name == "rustup"
    assert roots.rustup_resolved_target == roots.rustup_invocation.resolve(strict=True)
    assert all(
        target.is_file() and os.access(target, os.X_OK)
        for target in (roots.cargo_resolved_target, roots.rustup_resolved_target)
    )
    version = subprocess.run(
        (str(roots.cargo_invocation), "+1.96.0", "--version"),
        env=env,
        check=True,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert version.stdout.startswith("cargo 1.96.0")
    assert "syncing" not in version.stderr.lower()
    assert "downloading" not in version.stderr.lower()
    metadata = subprocess.run(
        (
            str(roots.cargo_invocation),
            "+1.96.0",
            "metadata",
            "--offline",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
            str(ROOT / "tests/qualification/fixtures/process-smoke/Cargo.toml"),
        ),
        env=env,
        check=True,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert json.loads(metadata.stdout)["packages"][0]["name"] == "qualification-process-smoke"
    assert "updating" not in metadata.stderr.lower()
