from __future__ import annotations

import hashlib
import json
import os
import signal
import stat
import subprocess
import sys
import threading
import time
import uuid
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


def _private_test_root(path: Path) -> Path:
    path.mkdir(mode=0o700)
    path.chmod(0o700)
    return path


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


def _processes_with_token(token: str) -> tuple[int, ...]:
    needle = token.encode("utf-8")
    matches: list[int] = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            argv = (entry / "cmdline").read_bytes().split(b"\0")
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        if needle in argv:
            matches.append(int(entry.name))
    return tuple(matches)


def _wait_for_no_token(token: str, *, timeout: float = 3.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not _processes_with_token(token):
            return
        time.sleep(0.02)
    assert _processes_with_token(token) == ()


def _direct_namespace_wrappers() -> tuple[int, ...]:
    matches: list[int] = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            status = (entry / "status").read_text(encoding="ascii")
            argv = (entry / "cmdline").read_bytes().split(b"\0")
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        parent_line = next((line for line in status.splitlines() if line.startswith("PPid:")), "")
        if parent_line.split() == ["PPid:", str(os.getpid())] and (
            b"--namespace-launcher" in argv or b"--kill-child=SIGKILL" in argv
        ):
            matches.append(int(entry.name))
    return tuple(matches)


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
    work = _private_test_root(tmp_path / "work")
    env = safe_subprocess_env(
        work,
        cargo_offline=True,
        tool_roots=roots,
        cargo_target_dir=work / "cargo-target",
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
        assert path.is_relative_to(work)
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
    work = _private_test_root(tmp_path / "work")
    outside = tmp_path / "outside"
    outside.mkdir()
    with pytest.raises(ValueError, match="cargo target"):
        safe_subprocess_env(
            work,
            cargo_offline=True,
            tool_roots=roots,
            cargo_target_dir=outside,
        )


def test_safe_environment_rejects_unsafe_roots_without_mutating_them(tmp_path: Path) -> None:
    roots = _fake_tool_roots(tmp_path)
    missing = tmp_path / "caller-controlled-missing"
    unsafe = (
        Path("/"),
        ROOT,
        ROOT / "qualification/.artifacts",
        Path.home().resolve(),
        Path.home().resolve() / "Yandex.Disk/Translation",
        missing,
    )
    before = {
        path: (path.exists(), path.stat().st_mode if path.exists() else None) for path in unsafe
    }
    for path in unsafe:
        with pytest.raises(ValueError):
            safe_subprocess_env(
                path,
                cargo_offline=True,
                tool_roots=roots,
                cargo_target_dir=path / "cargo-target",
            )
    after = {
        path: (path.exists(), path.stat().st_mode if path.exists() else None) for path in unsafe
    }
    assert after == before
    assert not missing.exists()


def test_safe_environment_requires_existing_private_current_uid_test_root(
    tmp_path: Path,
) -> None:
    roots = _fake_tool_roots(tmp_path)
    work = tmp_path / "work"
    work.mkdir(mode=0o755)
    work.chmod(0o755)
    with pytest.raises(ValueError, match="private"):
        safe_subprocess_env(
            work,
            cargo_offline=True,
            tool_roots=roots,
            cargo_target_dir=work / "cargo-target",
        )
    assert stat.S_IMODE(work.stat().st_mode) == 0o755
    assert not (work / "cargo-target").exists()
    linked = tmp_path / "linked-work"
    linked.symlink_to(work, target_is_directory=True)
    with pytest.raises(ValueError, match="symlink"):
        safe_subprocess_env(
            linked,
            cargo_offline=True,
            tool_roots=roots,
            cargo_target_dir=linked / "cargo-target",
        )


def test_safe_environment_live_root_and_cargo_target_are_strictly_bounded(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import tools.qualification.process as process_module

    roots = _fake_tool_roots(tmp_path)
    repo = tmp_path / "repo"
    artifacts = repo / "qualification/.artifacts"
    artifacts.mkdir(parents=True)
    work = _private_test_root(artifacts / "work-run")
    monkeypatch.setattr(process_module, "_REPO_ROOT", repo)
    monkeypatch.setattr(process_module, "_ARTIFACT_ROOT", artifacts)
    env = safe_subprocess_env(
        work,
        cargo_offline=True,
        tool_roots=roots,
        cargo_target_dir=artifacts / "cargo-target/mcp-run",
    )
    assert Path(env["CARGO_TARGET_DIR"]) == artifacts / "cargo-target/mcp-run"
    assert stat.S_IMODE(work.stat().st_mode) == 0o700
    with pytest.raises(ValueError, match="cargo target"):
        safe_subprocess_env(
            work,
            cargo_offline=True,
            tool_roots=roots,
            cargo_target_dir=artifacts / "cargo-target",
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
    work = _private_test_root(tmp_path / "process-work")
    receipt = run_owned_process(
        fake_child_and_grandchild(tmp_path),
        cwd=tmp_path,
        env=safe_subprocess_env(
            work,
            cargo_offline=True,
            tool_roots=roots,
            cargo_target_dir=work / "cargo-target",
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
    assert missing.process_group_reaped and not missing.core_dumps_disabled


@pytest.mark.parametrize("bad", [("",), (sys.executable, ""), (sys.executable, 1)])
def test_owned_process_rejects_nonempty_exact_string_argv(
    tmp_path: Path, bad: tuple[object, ...]
) -> None:
    with pytest.raises(ValueError, match="argv"):
        run_owned_process(  # type: ignore[arg-type]
            bad,
            cwd=tmp_path,
            env={"PATH": "/usr/bin:/bin"},
            timeout_seconds=1,
            no_progress_seconds=1,
        )


@pytest.mark.parametrize(
    ("field", "bad"),
    [
        ("argv", [sys.executable]),
        ("env", {"PATH": 1}),
        ("env", {1: "/usr/bin:/bin"}),
        ("timeout_seconds", True),
        ("timeout_seconds", 1.0),
        ("no_progress_seconds", False),
        ("no_progress_seconds", 1.0),
    ],
)
def test_owned_process_rejects_public_values_before_json_coercion(
    tmp_path: Path, field: str, bad: object
) -> None:
    arguments: dict[str, object] = {
        "argv": (sys.executable, "-c", "pass"),
        "cwd": tmp_path.resolve(),
        "env": {"PATH": "/usr/bin:/bin"},
        "timeout_seconds": 1,
        "no_progress_seconds": 1,
    }
    arguments[field] = bad
    with pytest.raises((TypeError, ValueError)):
        run_owned_process(**arguments)  # type: ignore[arg-type]


def test_owned_process_requires_absolute_canonical_directory_cwd(tmp_path: Path) -> None:
    canonical = tmp_path.resolve()
    linked = tmp_path.parent / f"qualification-cwd-link-{uuid.uuid4().hex}"
    linked.symlink_to(canonical, target_is_directory=True)
    try:
        for bad in (Path("."), linked, canonical / "missing"):
            with pytest.raises((TypeError, ValueError), match="cwd"):
                run_owned_process(
                    (sys.executable, "-c", "pass"),
                    cwd=bad,
                    env={"PATH": "/usr/bin:/bin"},
                    timeout_seconds=1,
                    no_progress_seconds=1,
                )
    finally:
        linked.unlink()


def test_owned_process_fails_before_target_spawn_when_unshare_is_unavailable(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import tools.qualification.process as process_module

    marker = tmp_path / "target-spawned"
    monkeypatch.setattr(process_module, "_UNSHARE_PATH", tmp_path / "missing-unshare")
    with pytest.raises(RuntimeError) as raised:
        run_owned_process(
            (sys.executable, "-c", f"open({str(marker)!r}, 'w').close()"),
            cwd=tmp_path.resolve(),
            env={"PATH": "/usr/bin:/bin"},
            timeout_seconds=1,
            no_progress_seconds=1,
        )
    assert str(raised.value) == "Linux PID namespace isolation is unavailable"
    assert str(tmp_path) not in str(raised.value)
    assert not marker.exists()


def test_owned_process_fails_before_target_spawn_when_unshare_is_unsupported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import tools.qualification.process as process_module

    marker = tmp_path / "unsupported-target-spawned"
    fake_unshare = tmp_path / "unshare"
    fake_unshare.write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
    fake_unshare.chmod(0o700)
    monkeypatch.setattr(process_module, "_UNSHARE_PATH", fake_unshare)
    with pytest.raises(RuntimeError) as raised:
        run_owned_process(
            (sys.executable, "-c", f"open({str(marker)!r}, 'w').close()"),
            cwd=tmp_path.resolve(),
            env={"PATH": "/usr/bin:/bin"},
            timeout_seconds=1,
            no_progress_seconds=1,
        )
    assert str(raised.value) == "qualification supervisor failed"
    assert str(tmp_path) not in str(raised.value)
    assert not marker.exists()


def test_namespace_supervisor_crash_kills_setsid_double_fork_descendant(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import tools.qualification.process as process_module

    token = f"qualification-crash-{uuid.uuid4().hex}"
    ready = tmp_path / "crash-ready"
    script = tmp_path / "crashing-supervisor.py"
    write_limit = (
        f"pathlib.Path({str(ready)!r}).write_text(str(resource.getrlimit(resource.RLIMIT_CORE)))"
    )
    exec_sleeper = (
        "os.execv(sys.executable, "
        f"[sys.executable, '-c', 'import time; time.sleep(60)', {token!r}])"
    )
    script.write_text(
        "import os,pathlib,resource,sys,time\n"
        "if os.fork() == 0:\n"
        " os.setsid()\n"
        " if os.fork() == 0:\n"
        f"  {write_limit}\n"
        f"  {exec_sleeper}\n"
        " os._exit(0)\n"
        "deadline = time.monotonic() + 2\n"
        f"ready = pathlib.Path({str(ready)!r})\n"
        "while not ready.exists() and time.monotonic() < deadline:\n"
        " time.sleep(0.01)\n"
        "os._exit(17)\n",
        encoding="utf-8",
    )
    monkeypatch.setattr(
        process_module,
        "_supervisor_command",
        lambda config_fd, result_fd: (
            sys.executable,
            str(script),
            str(config_fd),
            str(result_fd),
        ),
    )
    with pytest.raises(RuntimeError, match="qualification supervisor failed"):
        run_owned_process(
            (sys.executable, "-c", "pass"),
            cwd=tmp_path.resolve(),
            env={"PATH": "/usr/bin:/bin"},
            timeout_seconds=1,
            no_progress_seconds=1,
        )
    assert ready.read_text(encoding="utf-8") == "(0, 0)"
    _wait_for_no_token(token)


def test_namespace_shutdown_catches_descendants_forked_during_termination(
    tmp_path: Path,
) -> None:
    token = f"qualification-fork-race-{uuid.uuid4().hex}"
    script = tmp_path / "fork-during-shutdown.py"
    exec_sleeper = (
        "os.execv(sys.executable, "
        f"[sys.executable, '-c', 'import time; time.sleep(60)', {token!r}])"
    )
    script.write_text(
        "import os,signal,sys,time\n"
        "signal.signal(signal.SIGTERM, lambda *_: None)\n"
        "while True:\n"
        " pid = os.fork()\n"
        " if pid == 0:\n"
        f"  {exec_sleeper}\n"
        " time.sleep(0.02)\n",
        encoding="utf-8",
    )
    receipt = run_owned_process(
        (sys.executable, str(script)),
        cwd=tmp_path.resolve(),
        env={"PATH": "/usr/bin:/bin"},
        timeout_seconds=1,
        no_progress_seconds=1,
    )
    assert receipt.timed_out and receipt.process_group_reaped
    _wait_for_no_token(token)


def test_parent_death_kills_namespace_wrapper_and_target(tmp_path: Path) -> None:
    token = f"qualification-parent-death-{uuid.uuid4().hex}"
    ready = tmp_path / "parent-death-ready"
    worker = tmp_path / "parent-worker.py"
    target_code = (
        f"import pathlib,time; pathlib.Path({str(ready)!r}).write_text('ready'); time.sleep(60)"
    )
    worker.write_text(
        "import sys\n"
        f"sys.path.insert(0, {str(ROOT)!r})\n"
        "from pathlib import Path\n"
        "from tools.qualification.process import run_owned_process\n"
        "run_owned_process(\n"
        f" (sys.executable, '-c', {target_code!r}, {token!r}),\n"
        f" cwd=Path({str(tmp_path)!r}),\n"
        " env={'PATH':'/usr/bin:/bin'},\n"
        " timeout_seconds=60,\n"
        " no_progress_seconds=60,\n"
        ")\n",
        encoding="utf-8",
    )
    parent = subprocess.Popen((sys.executable, str(worker)))
    try:
        deadline = time.monotonic() + 5
        while not ready.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        assert ready.exists()
        assert _processes_with_token(token)
        parent.kill()
        parent.wait(timeout=3)
        _wait_for_no_token(token)
    finally:
        if parent.poll() is None:
            parent.kill()
            parent.wait(timeout=3)
        for pid in _processes_with_token(token):
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


def test_wrapper_crash_kills_namespace_target_without_signalling_others(tmp_path: Path) -> None:
    token = f"qualification-wrapper-crash-{uuid.uuid4().hex}"
    ready = tmp_path / "wrapper-crash-ready"
    errors: list[BaseException] = []

    def run() -> None:
        try:
            run_owned_process(
                (
                    sys.executable,
                    "-c",
                    (
                        f"import pathlib,time; pathlib.Path({str(ready)!r})"
                        ".write_text('ready'); time.sleep(60)"
                    ),
                    token,
                ),
                cwd=tmp_path.resolve(),
                env={"PATH": "/usr/bin:/bin"},
                timeout_seconds=60,
                no_progress_seconds=60,
            )
        except BaseException as exc:
            errors.append(exc)

    thread = threading.Thread(target=run)
    thread.start()
    try:
        deadline = time.monotonic() + 5
        launchers: tuple[int, ...] = ()
        while time.monotonic() < deadline:
            launchers = _direct_namespace_wrappers()
            if ready.exists() and len(launchers) == 1:
                break
            time.sleep(0.02)
        assert ready.exists() and len(launchers) == 1
        assert _processes_with_token(token)
        os.kill(launchers[0], signal.SIGKILL)
        thread.join(timeout=5)
        assert not thread.is_alive()
        assert len(errors) == 1 and isinstance(errors[0], RuntimeError)
        _wait_for_no_token(token)
    finally:
        for pid in _processes_with_token(token):
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


def test_core_limit_is_zero_before_supervisor_spawns_target(tmp_path: Path) -> None:
    expected = b"(0, 0)\n"
    receipt = run_owned_process(
        (
            sys.executable,
            "-c",
            "import resource; print(resource.getrlimit(resource.RLIMIT_CORE))",
        ),
        cwd=tmp_path.resolve(),
        env={"PATH": "/usr/bin:/bin"},
        timeout_seconds=2,
        no_progress_seconds=1,
    )
    assert receipt.exit_code == 0
    assert receipt.stdout_sha256 == hashlib.sha256(expected).hexdigest()
    assert receipt.core_dumps_disabled


def test_owned_process_reaps_descendant_that_escapes_group_with_setsid(tmp_path: Path) -> None:
    token = f"qualification-setsid-{uuid.uuid4().hex}"
    marker = tmp_path / "escaped.started"
    child_code = (
        "import os,pathlib,time; os.setsid(); "
        f"pathlib.Path({str(marker)!r}).write_text('started'); time.sleep(60)"
    )
    parent = tmp_path / "setsid-parent.py"
    parent.write_text(
        "import subprocess,sys,time\n"
        f"subprocess.Popen([sys.executable, '-c', {child_code!r}, {token!r}])\n"
        "time.sleep(60)\n",
        encoding="utf-8",
    )
    receipt = run_owned_process(
        (sys.executable, str(parent)),
        cwd=tmp_path,
        env={"PATH": "/usr/bin:/bin"},
        timeout_seconds=2,
        no_progress_seconds=1,
    )
    assert receipt.timed_out and receipt.process_group_reaped
    assert marker.exists()
    _wait_for_no_token(token)


def test_owned_process_reaps_immediate_double_fork_orphan(tmp_path: Path) -> None:
    """Catch loss of ancestry before the runner's first procfs scan."""
    marker = tmp_path / "double-fork.started"
    script = tmp_path / "double-fork.py"
    script.write_text(
        "import os,pathlib,time\n"
        "if os.fork(): os._exit(0)\n"
        "os.setsid()\n"
        "if os.fork(): os._exit(0)\n"
        f"pathlib.Path({str(marker)!r}).write_text('started')\n"
        "os.close(1); os.close(2); time.sleep(60)\n",
        encoding="utf-8",
    )
    receipt = run_owned_process(
        (sys.executable, str(script)),
        cwd=tmp_path,
        env={"PATH": "/usr/bin:/bin"},
        timeout_seconds=2,
        no_progress_seconds=1,
    )
    assert marker.exists() and receipt.process_group_reaped
    _wait_for_no_token(str(script))


def test_owned_process_reaps_nondumpable_immediate_double_fork_orphan(
    tmp_path: Path,
) -> None:
    """Catch ownership schemes that depend on reading descendant file descriptors."""
    marker = tmp_path / "nondumpable-double-fork.started"
    script = tmp_path / "nondumpable-double-fork.py"
    script.write_text(
        "import ctypes,os,pathlib,time\n"
        "if os.fork(): os._exit(0)\n"
        "os.setsid()\n"
        "if os.fork(): os._exit(0)\n"
        "ctypes.CDLL(None).prctl(4, 0, 0, 0, 0)\n"
        "for fd in range(3, 256):\n"
        " try: os.close(fd)\n"
        " except OSError: pass\n"
        f"pathlib.Path({str(marker)!r}).write_text('started')\n"
        "time.sleep(60)\n",
        encoding="utf-8",
    )
    receipt = run_owned_process(
        (sys.executable, str(script)),
        cwd=tmp_path,
        env={"PATH": "/usr/bin:/bin"},
        timeout_seconds=2,
        no_progress_seconds=1,
    )
    assert marker.exists() and receipt.process_group_reaped
    _wait_for_no_token(str(script))


def test_owned_process_never_signals_unrelated_child_forked_during_spawn(
    tmp_path: Path,
) -> None:
    start = threading.Event()
    unrelated: list[subprocess.Popen[bytes]] = []

    def fork_unrelated() -> None:
        start.wait(timeout=2)
        unrelated.append(subprocess.Popen((sys.executable, "-c", "import time; time.sleep(60)")))

    thread = threading.Thread(target=fork_unrelated)
    thread.start()
    try:
        start.set()
        receipt = run_owned_process(
            (sys.executable, "-c", "import time; time.sleep(60)"),
            cwd=tmp_path,
            env={"PATH": "/usr/bin:/bin"},
            timeout_seconds=1,
            no_progress_seconds=1,
        )
        thread.join(timeout=3)
        assert receipt.process_group_reaped
        assert unrelated and unrelated[0].poll() is None
    finally:
        thread.join(timeout=3)
        for process in unrelated:
            process.terminate()
            process.wait(timeout=5)


def test_pid_reuse_seam_never_signals_replacement(monkeypatch: pytest.MonkeyPatch) -> None:
    import signal

    import tools.qualification.process as process_module

    owned = process_module._ProcessIdentity(pid=12345, start_time=10)
    replacement = process_module._ProcessIdentity(pid=12345, start_time=11)
    calls: list[tuple[int, object]] = []
    monkeypatch.setattr(process_module, "_process_identity", lambda _pid: replacement)
    monkeypatch.setattr(process_module.os, "kill", lambda pid, sig: calls.append((pid, sig)))
    assert not process_module._signal_identity(owned, signal.SIGTERM)
    assert calls == []


def test_owned_process_never_uses_raw_process_group_signals(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import tools.qualification.process as process_module

    monkeypatch.setattr(
        os,
        "killpg",
        lambda *_args: (_ for _ in ()).throw(AssertionError("raw PGID signal")),
    )
    monkeypatch.setattr(process_module, "_collect_descendants", lambda *_args, **_kwargs: None)
    assert process_module._terminate_owned_tree({}, owner_pid=os.getpid())


def test_concurrent_parent_calls_use_isolated_supervisors(
    tmp_path: Path,
) -> None:
    receipts: list[object] = []

    def run(value: str) -> None:
        receipts.append(
            run_owned_process(
                (sys.executable, "-c", f"print({value!r})"),
                cwd=tmp_path,
                env={"PATH": "/usr/bin:/bin"},
                timeout_seconds=2,
                no_progress_seconds=1,
            )
        )

    threads = [threading.Thread(target=run, args=(str(index),)) for index in range(8)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=5)
    assert len(receipts) == 8
    assert all(receipt.exit_code == 0 for receipt in receipts)  # type: ignore[union-attr]


@pytest.mark.skipif(not hasattr(os, "fork"), reason="requires POSIX fork")
def test_forked_parent_call_uses_its_own_supervisor(tmp_path: Path) -> None:
    read_fd, write_fd = os.pipe()
    try:
        pid = os.fork()
        if pid == 0:
            os.close(read_fd)
            try:
                receipt = run_owned_process(
                    (sys.executable, "-c", "print('forked-parent')"),
                    cwd=tmp_path,
                    env={"PATH": "/usr/bin:/bin"},
                    timeout_seconds=2,
                    no_progress_seconds=1,
                )
                os.write(write_fd, b"ok" if receipt.exit_code == 0 else b"bad")
            finally:
                os._exit(0)
        os.close(write_fd)
        assert os.read(read_fd, 8) == b"ok"
        os.waitpid(pid, 0)
    finally:
        for descriptor in (read_fd, write_fd):
            try:
                os.close(descriptor)
            except OSError:
                pass


@pytest.mark.parametrize("mode", ["crash", "invalid", "hang"])
def test_supervisor_failure_is_bounded_redacted_and_closes_parent_fds(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: str
) -> None:
    import tools.qualification.process as process_module

    script = tmp_path / "bad-supervisor.py"
    behavior = {
        "crash": "raise SystemExit(3)",
        "invalid": "import os; os.write(int(__import__('sys').argv[2]), b'not-json')",
        "hang": "import time; time.sleep(60)",
    }[mode]
    script.write_text(f"{behavior}\n", encoding="utf-8")
    monkeypatch.setattr(
        process_module,
        "_supervisor_command",
        lambda config_fd, result_fd: (
            sys.executable,
            str(script),
            str(config_fd),
            str(result_fd),
        ),
    )
    before = len(os.listdir("/proc/self/fd"))
    secret = tmp_path / "must-not-leak"
    started = time.monotonic()
    with pytest.raises(RuntimeError) as raised:
        run_owned_process(
            (sys.executable, "-c", "print('never')", str(secret)),
            cwd=tmp_path,
            env={
                "PATH": "/usr/bin:/bin",
                "QUALIFICATION_PADDING": "x" * (200_000 if mode == "hang" else 1),
            },
            timeout_seconds=1,
            no_progress_seconds=1,
        )
    assert time.monotonic() - started < 5
    assert str(secret) not in str(raised.value)
    assert len(os.listdir("/proc/self/fd")) == before


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
    work = _private_test_root(tmp_path / "live-work")
    env = safe_subprocess_env(
        work,
        cargo_offline=True,
        tool_roots=roots,
        cargo_target_dir=work / "cargo-target",
    )
    assert Path(env["CARGO_HOME"]).resolve() == roots.cargo_home
    assert Path(env["RUSTUP_HOME"]).resolve() == roots.rustup_home
    assert Path(env["HOME"]).is_relative_to(work)
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
