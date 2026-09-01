"""Sanitized subprocess discovery and bounded process execution."""

from __future__ import annotations

import ctypes
import hashlib
import os
import resource
import selectors
import signal
import stat
import subprocess
import sys
import threading
import time
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

_EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()
_PROCESS_LOCK = threading.Lock()
_PR_SET_CHILD_SUBREAPER = 36
_PR_GET_CHILD_SUBREAPER = 37


@dataclass(frozen=True, slots=True)
class ToolRoots:
    """Canonical caches plus preserved lexical shim and resolved executable paths."""

    cargo_home: Path
    rustup_home: Path
    cargo_invocation: Path
    cargo_resolved_target: Path
    rustup_invocation: Path
    rustup_resolved_target: Path


@dataclass(frozen=True, slots=True)
class ProcessReceipt:
    """Non-sensitive bounded result of an owned subprocess."""

    exit_code: int | None
    timed_out: bool
    stdout_sha256: str
    stderr_sha256: str
    duration_ms: int
    process_group_reaped: bool
    core_dumps_disabled: bool


def _absolute_lexical(value: str, *, label: str) -> Path:
    if not value or "\x00" in value:
        raise ValueError(f"{label} must be a nonempty path")
    return Path(os.path.abspath(value))


def _canonical_directory(path: Path, *, label: str) -> Path:
    try:
        canonical = path.resolve(strict=True)
    except OSError as exc:
        raise ValueError(f"{label} does not exist") from exc
    if not canonical.is_dir():
        raise ValueError(f"{label} is not a directory")
    return canonical


def _find_invocation(name: str, cargo_home: Path, original_path: str) -> Path:
    candidates = [cargo_home / "bin" / name]
    for part in original_path.split(os.pathsep):
        directory = part or os.curdir
        candidates.append(_absolute_lexical(os.path.join(directory, name), label=name))
    seen: set[Path] = set()
    for candidate in candidates:
        lexical = candidate.absolute()
        if lexical in seen:
            continue
        seen.add(lexical)
        if os.path.lexists(lexical):
            return lexical
    raise ValueError(f"{name} invocation was not found in the original environment")


def _resolved_executable(invocation: Path, *, label: str) -> Path:
    try:
        target = invocation.resolve(strict=True)
    except OSError as exc:
        raise ValueError(f"{label} invocation has no resolved target") from exc
    mode = target.stat().st_mode
    if not stat.S_ISREG(mode) or not os.access(target, os.X_OK):
        raise ValueError(f"{label} target is not an executable regular file")
    return target


def discover_tool_roots(original_env: Mapping[str, str]) -> ToolRoots:
    """Discover Rust shims from the unsanitized caller mapping without replacing them."""
    home_value = original_env.get("HOME")
    cargo_value = original_env.get("CARGO_HOME")
    rustup_value = original_env.get("RUSTUP_HOME")
    if cargo_value is None or rustup_value is None:
        if not home_value:
            raise ValueError("original HOME is required when tool cache roots are omitted")
        home = _absolute_lexical(home_value, label="HOME")
        cargo_value = cargo_value or str(home / ".cargo")
        rustup_value = rustup_value or str(home / ".rustup")

    cargo_lexical = _absolute_lexical(cargo_value, label="CARGO_HOME")
    rustup_lexical = _absolute_lexical(rustup_value, label="RUSTUP_HOME")
    original_path = original_env.get("PATH", "")
    cargo_invocation = _find_invocation("cargo", cargo_lexical, original_path)
    rustup_invocation = _find_invocation("rustup", cargo_lexical, original_path)
    return ToolRoots(
        cargo_home=_canonical_directory(cargo_lexical, label="CARGO_HOME"),
        rustup_home=_canonical_directory(rustup_lexical, label="RUSTUP_HOME"),
        cargo_invocation=cargo_invocation,
        cargo_resolved_target=_resolved_executable(cargo_invocation, label="cargo"),
        rustup_invocation=rustup_invocation,
        rustup_resolved_target=_resolved_executable(rustup_invocation, label="rustup"),
    )


def _reject_symlink_components(path: Path) -> None:
    current = Path(path.anchor)
    for part in path.parts[1:]:
        current /= part
        if current.is_symlink():
            raise ValueError(f"work root contains a symlink: {current.name}")
        if not current.exists():
            break


def _private_directory(path: Path) -> None:
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    _reject_symlink_components(path)
    if path.is_symlink() or not path.is_dir():
        raise ValueError("private subprocess directory is not a real directory")
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(path, flags)
    try:
        os.fchmod(descriptor, 0o700)
    finally:
        os.close(descriptor)


def _validate_tool_roots(tool_roots: ToolRoots) -> None:
    for label, root in (
        ("CARGO_HOME", tool_roots.cargo_home),
        ("RUSTUP_HOME", tool_roots.rustup_home),
    ):
        if root != root.resolve(strict=True) or not root.is_dir():
            raise ValueError(f"{label} is not a canonical directory")
    for label, invocation, target in (
        ("cargo", tool_roots.cargo_invocation, tool_roots.cargo_resolved_target),
        ("rustup", tool_roots.rustup_invocation, tool_roots.rustup_resolved_target),
    ):
        if not invocation.is_absolute() or invocation.resolve(strict=True) != target:
            raise ValueError(f"{label} invocation and resolved target disagree")
        if target != _resolved_executable(invocation, label=label):
            raise ValueError(f"{label} resolved target is not canonical")


def _artifact_cargo_root(work_root: Path) -> Path | None:
    parts = work_root.parts
    for index in range(len(parts) - 1):
        if parts[index : index + 2] == ("qualification", ".artifacts"):
            return Path(*parts[: index + 2]) / "cargo-target"
    return None


def _bounded_target(work_root: Path, cargo_target_dir: Path) -> Path:
    target = cargo_target_dir.absolute()
    _reject_symlink_components(target)
    artifact_cargo_root = _artifact_cargo_root(work_root)
    if artifact_cargo_root is not None:
        if not target.is_relative_to(artifact_cargo_root):
            raise ValueError("cargo target must be beneath qualification/.artifacts/cargo-target")
    elif not target.is_relative_to(work_root):
        raise ValueError("cargo target must be beneath the supplied test work root")
    if target == work_root:
        raise ValueError("cargo target cannot equal the work root")
    _private_directory(target)
    return target.resolve(strict=True)


def safe_subprocess_env(
    work_root: Path,
    *,
    cargo_offline: bool,
    tool_roots: ToolRoots,
    cargo_target_dir: Path,
) -> dict[str, str]:
    """Create a private, credential-free environment for qualification subprocesses."""
    lexical_work = work_root.absolute()
    _reject_symlink_components(lexical_work)
    _private_directory(lexical_work)
    canonical_work = lexical_work.resolve(strict=True)
    _validate_tool_roots(tool_roots)
    target = _bounded_target(canonical_work, cargo_target_dir)

    private = {
        "HOME": canonical_work / "home",
        "TMPDIR": canonical_work / "tmp",
        "XDG_CACHE_HOME": canonical_work / "xdg/cache",
        "XDG_CONFIG_HOME": canonical_work / "xdg/config",
        "XDG_DATA_HOME": canonical_work / "xdg/data",
    }
    for directory in private.values():
        _private_directory(directory)

    path_parts: list[str] = []
    for path in (
        tool_roots.cargo_invocation.parent,
        tool_roots.rustup_invocation.parent,
        Path("/usr/bin"),
        Path("/bin"),
    ):
        value = str(path)
        if value not in path_parts:
            path_parts.append(value)
    env = {name: str(path) for name, path in private.items()}
    env.update(
        {
            "CARGO_HOME": str(tool_roots.cargo_home),
            "RUSTUP_HOME": str(tool_roots.rustup_home),
            "CARGO_TARGET_DIR": str(target),
            "CARGO_NET_OFFLINE": "true" if cargo_offline else "false",
            "RUSTUP_AUTO_INSTALL": "0",
            "PATH": os.pathsep.join(path_parts),
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
            "TZ": "UTC",
        }
    )
    return env


def _disable_core_dumps() -> None:
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def _subreaper_state(enable: bool) -> int | None:
    if not sys.platform.startswith("linux"):
        return None
    libc = ctypes.CDLL(None, use_errno=True)
    previous = ctypes.c_int()
    if libc.prctl(_PR_GET_CHILD_SUBREAPER, ctypes.byref(previous), 0, 0, 0) != 0:
        return None
    if libc.prctl(_PR_SET_CHILD_SUBREAPER, int(enable), 0, 0, 0) != 0:
        return None
    return previous.value


def _restore_subreaper(previous: int | None) -> None:
    if previous is None or not sys.platform.startswith("linux"):
        return
    ctypes.CDLL(None, use_errno=True).prctl(_PR_SET_CHILD_SUBREAPER, previous, 0, 0, 0)


def _signal_group(process_group: int, sig: signal.Signals) -> None:
    try:
        os.killpg(process_group, sig)
    except ProcessLookupError:
        pass


def _group_exists(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
    except ProcessLookupError:
        return False
    except PermissionError:  # pragma: no cover - an owned group remains signalable
        return True
    return True


def _terminate_group(process: subprocess.Popen[bytes], process_group: int) -> None:
    _signal_group(process_group, signal.SIGTERM)
    deadline = time.monotonic() + 0.25
    while _group_exists(process_group) and time.monotonic() < deadline:
        time.sleep(0.01)
    if _group_exists(process_group):
        _signal_group(process_group, signal.SIGKILL)
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:  # pragma: no cover
        _signal_group(process_group, signal.SIGKILL)
        process.wait(timeout=1)


def _reap_adopted_group(process_group: int) -> None:
    deadline = time.monotonic() + 1
    while time.monotonic() < deadline:
        reaped = False
        while True:
            try:
                pid, _ = os.waitpid(-process_group, os.WNOHANG)
            except ChildProcessError:
                return
            if pid == 0:
                break
            reaped = True
        if not _group_exists(process_group):
            return
        if not reaped:
            time.sleep(0.01)


def run_owned_process(
    argv: tuple[str, ...],
    *,
    cwd: Path,
    env: Mapping[str, str],
    timeout_seconds: int,
    no_progress_seconds: int,
) -> ProcessReceipt:
    """Run one owned process group with bounded output, time, cleanup, and receipts."""
    if not argv or any(not isinstance(arg, str) or "\x00" in arg for arg in argv):
        raise ValueError("argv must contain nonempty safe strings")
    if timeout_seconds <= 0 or no_progress_seconds <= 0:
        raise ValueError("process timeouts must be positive")
    started = time.monotonic()
    stdout_hash = hashlib.sha256()
    stderr_hash = hashlib.sha256()
    timed_out = False
    exit_code: int | None = None
    process_group_reaped = True

    with _PROCESS_LOCK:
        previous_subreaper = _subreaper_state(True)
        process: subprocess.Popen[bytes] | None = None
        selector = selectors.DefaultSelector()
        try:
            try:
                process = subprocess.Popen(
                    argv,
                    cwd=cwd,
                    env=dict(env),
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    start_new_session=True,
                    preexec_fn=_disable_core_dumps,
                    close_fds=True,
                )
            except OSError:
                return ProcessReceipt(
                    exit_code=None,
                    timed_out=False,
                    stdout_sha256=_EMPTY_SHA256,
                    stderr_sha256=_EMPTY_SHA256,
                    duration_ms=max(0, int((time.monotonic() - started) * 1000)),
                    process_group_reaped=True,
                    core_dumps_disabled=True,
                )

            process_group = process.pid
            assert process.stdout is not None and process.stderr is not None
            for pipe, digest in ((process.stdout, stdout_hash), (process.stderr, stderr_hash)):
                os.set_blocking(pipe.fileno(), False)
                selector.register(pipe, selectors.EVENT_READ, digest)
            last_progress = time.monotonic()
            terminated = False
            termination_started: float | None = None
            while selector.get_map() or process.poll() is None:
                now = time.monotonic()
                if not terminated and (
                    now - started >= timeout_seconds or now - last_progress >= no_progress_seconds
                ):
                    timed_out = True
                    _terminate_group(process, process_group)
                    terminated = True
                    termination_started = time.monotonic()
                elif process.poll() is not None and not terminated:
                    _terminate_group(process, process_group)
                    terminated = True
                    termination_started = time.monotonic()

                events = selector.select(timeout=0.05) if selector.get_map() else ()
                if not events and not selector.get_map():
                    time.sleep(0.05)
                for key, _ in events:
                    try:
                        chunk = os.read(key.fileobj.fileno(), 64 * 1024)
                    except BlockingIOError:
                        continue
                    if chunk:
                        key.data.update(chunk)
                        last_progress = time.monotonic()
                    else:
                        selector.unregister(key.fileobj)
                        key.fileobj.close()
                if termination_started is not None and time.monotonic() - termination_started > 1:
                    break

            if process.poll() is None:
                _terminate_group(process, process_group)
            exit_code = process.wait(timeout=1)
            _reap_adopted_group(process_group)
            process_group_reaped = not _group_exists(process_group)
        except BaseException:
            if process is not None:
                _terminate_group(process, process.pid)
                _reap_adopted_group(process.pid)
            raise
        finally:
            selector.close()
            if process is not None:
                for pipe in (process.stdout, process.stderr):
                    if pipe is not None and not pipe.closed:
                        pipe.close()
            _restore_subreaper(previous_subreaper)

    return ProcessReceipt(
        exit_code=exit_code,
        timed_out=timed_out,
        stdout_sha256=stdout_hash.hexdigest(),
        stderr_sha256=stderr_hash.hexdigest(),
        duration_ms=max(0, int((time.monotonic() - started) * 1000)),
        process_group_reaped=process_group_reaped,
        core_dumps_disabled=True,
    )
