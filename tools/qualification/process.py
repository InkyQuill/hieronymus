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
import tempfile
import threading
import time
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

_EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()
_PROCESS_LOCK = threading.Lock()
_PROCESS_LOCK_TIMEOUT_SECONDS = 5.0
_PR_SET_CHILD_SUBREAPER = 36
_PR_GET_CHILD_SUBREAPER = 37
_REPO_ROOT = Path(__file__).resolve().parents[2]
_ARTIFACT_ROOT = _REPO_ROOT / "qualification/.artifacts"
_PARENT_MARKER_FDS: set[int] = set()


class ProcessCoordinationTimeout(RuntimeError):
    """Raised before spawn when the process runner cannot be acquired safely."""


def _reset_after_fork() -> None:
    global _PROCESS_LOCK
    _PROCESS_LOCK = threading.Lock()
    for descriptor in tuple(_PARENT_MARKER_FDS):
        try:
            os.close(descriptor)
        except OSError:
            pass
    _PARENT_MARKER_FDS.clear()


if hasattr(os, "register_at_fork"):
    os.register_at_fork(after_in_child=_reset_after_fork)


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


def _directory_flags() -> int:
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    return flags


def _open_existing_directory(path: Path, *, label: str) -> int:
    """Open an absolute directory one component at a time without following links."""
    if not path.is_absolute():
        raise ValueError(f"{label} must be absolute")
    descriptor = os.open(path.anchor, _directory_flags())
    try:
        for part in path.parts[1:]:
            child = os.open(part, _directory_flags(), dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        return descriptor
    except OSError as exc:
        os.close(descriptor)
        raise ValueError(f"{label} must be an existing nonsymlink directory") from exc


def _require_private_owned(fd: int, *, label: str) -> None:
    info = os.fstat(fd)
    if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError(f"{label} must be a current-uid private 0700 directory")


def _create_private_descendant(anchor: Path, target: Path, *, label: str) -> Path:
    """Create only target's missing descendants through an already safe anchor fd."""
    _preflight_private_descendant(anchor, target, label=label)
    anchor_fd = _open_existing_directory(anchor, label=f"{label} root")
    descriptor = anchor_fd
    try:
        for part in target.relative_to(anchor).parts:
            try:
                os.mkdir(part, mode=0o700, dir_fd=descriptor)
            except FileExistsError:
                pass
            child = os.open(part, _directory_flags(), dir_fd=descriptor)
            _require_private_owned(child, label=label)
            if descriptor != anchor_fd:
                os.close(descriptor)
            descriptor = child
        return Path(os.readlink(f"/proc/self/fd/{descriptor}"))
    except OSError as exc:
        raise ValueError(f"{label} contains an unsafe path component") from exc
    finally:
        if descriptor != anchor_fd:
            os.close(descriptor)
        os.close(anchor_fd)


def _preflight_private_descendant(anchor: Path, target: Path, *, label: str) -> None:
    """Validate every existing component before any directory is created."""
    if target == anchor or not target.is_relative_to(anchor):
        raise ValueError(f"{label} must be a strict descendant of its safe root")
    anchor_fd = _open_existing_directory(anchor, label=f"{label} root")
    descriptor = anchor_fd
    try:
        for part in target.relative_to(anchor).parts:
            try:
                child = os.open(part, _directory_flags(), dir_fd=descriptor)
            except FileNotFoundError:
                return
            _require_private_owned(child, label=label)
            if descriptor != anchor_fd:
                os.close(descriptor)
            descriptor = child
    except OSError as exc:
        raise ValueError(f"{label} contains an unsafe path component") from exc
    finally:
        if descriptor != anchor_fd:
            os.close(descriptor)
        os.close(anchor_fd)


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


def _validated_work_root(work_root: Path) -> tuple[Path, bool]:
    lexical = work_root.absolute()
    forbidden = {
        Path("/"),
        _REPO_ROOT,
        _ARTIFACT_ROOT,
        Path.home().resolve(strict=True),
    }
    translation = Path.home().resolve(strict=True) / "Yandex.Disk/Translation"
    if lexical in forbidden or lexical.is_relative_to(translation):
        raise ValueError("work root is an unsafe root")
    live = lexical.is_relative_to(_ARTIFACT_ROOT) and lexical != _ARTIFACT_ROOT
    temporary = Path(tempfile.gettempdir()).resolve(strict=True)
    if not live and (lexical == temporary or not lexical.is_relative_to(temporary)):
        raise ValueError("test work root must be beneath the canonical system temporary root")
    fd = _open_existing_directory(lexical, label="work root")
    try:
        _require_private_owned(fd, label="work root")
        canonical = Path(os.readlink(f"/proc/self/fd/{fd}"))
    finally:
        os.close(fd)
    if canonical != lexical:
        raise ValueError("work root must be canonical")
    return canonical, live


def _target_boundary(work_root: Path, cargo_target_dir: Path, *, live: bool) -> tuple[Path, Path]:
    target = cargo_target_dir.absolute()
    if live:
        cargo_root = _ARTIFACT_ROOT / "cargo-target"
        if target == cargo_root or not target.is_relative_to(cargo_root):
            raise ValueError("cargo target must be beneath qualification/.artifacts/cargo-target")
        return cargo_root, target
    if target == work_root or not target.is_relative_to(work_root):
        raise ValueError("cargo target must be beneath the supplied test work root")
    return work_root, target


def safe_subprocess_env(
    work_root: Path,
    *,
    cargo_offline: bool,
    tool_roots: ToolRoots,
    cargo_target_dir: Path,
) -> dict[str, str]:
    """Create a private, credential-free environment for qualification subprocesses."""
    canonical_work, live = _validated_work_root(work_root)
    _validate_tool_roots(tool_roots)
    target_anchor, target_path = _target_boundary(canonical_work, cargo_target_dir, live=live)

    private = {
        "HOME": canonical_work / "home",
        "TMPDIR": canonical_work / "tmp",
        "XDG_CACHE_HOME": canonical_work / "xdg/cache",
        "XDG_CONFIG_HOME": canonical_work / "xdg/config",
        "XDG_DATA_HOME": canonical_work / "xdg/data",
    }
    if live:
        _preflight_private_descendant(_ARTIFACT_ROOT, target_anchor, label="cargo target root")
        if target_anchor.exists():
            _preflight_private_descendant(target_anchor, target_path, label="cargo target")
    else:
        _preflight_private_descendant(target_anchor, target_path, label="cargo target")
    for directory in private.values():
        _preflight_private_descendant(
            canonical_work, directory, label="private subprocess directory"
        )

    if live:
        _create_private_descendant(_ARTIFACT_ROOT, target_anchor, label="cargo target root")
    target = _create_private_descendant(target_anchor, target_path, label="cargo target")
    for directory in private.values():
        _create_private_descendant(canonical_work, directory, label="private subprocess directory")

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
    if ctypes.CDLL(None, use_errno=True).prctl(_PR_SET_CHILD_SUBREAPER, previous, 0, 0, 0) != 0:
        raise RuntimeError("failed to restore Linux child-subreaper state")


@dataclass(frozen=True, slots=True)
class _ProcessIdentity:
    pid: int
    start_time: int


@dataclass(frozen=True, slots=True)
class _OwnedMarker:
    """In-memory identity of the pipe inherited by cooperative descendants."""

    device: int
    inode: int


def _create_owned_marker() -> tuple[int, int, _OwnedMarker]:
    flags = getattr(os, "O_CLOEXEC", 0)
    if hasattr(os, "pipe2"):
        read_fd, write_fd = os.pipe2(flags)
    else:  # pragma: no cover - Linux qualification hosts provide pipe2
        read_fd, write_fd = os.pipe()
        os.set_inheritable(read_fd, False)
        os.set_inheritable(write_fd, False)
    info = os.fstat(read_fd)
    return read_fd, write_fd, _OwnedMarker(info.st_dev, info.st_ino)


def _close_fd(descriptor: int | None) -> None:
    if descriptor is None:
        return
    try:
        os.close(descriptor)
    except OSError:
        pass


def _proc_state(pid: int) -> tuple[_ProcessIdentity, int] | None:
    """Return a PID-reuse-safe identity and parent PID from Linux procfs."""
    if not sys.platform.startswith("linux"):
        return None
    try:
        raw = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
    except (FileNotFoundError, PermissionError, ProcessLookupError):
        return None
    end = raw.rfind(")")
    if end < 0:
        return None
    fields = raw[end + 2 :].split()
    try:
        return _ProcessIdentity(pid=pid, start_time=int(fields[19])), int(fields[1])
    except (IndexError, ValueError):  # pragma: no cover - malformed procfs is not actionable
        return None


def _process_identity(pid: int) -> _ProcessIdentity | None:
    state = _proc_state(pid)
    return None if state is None else state[0]


def _has_owned_marker(pid: int, marker: _OwnedMarker) -> bool:
    try:
        descriptors = os.scandir(f"/proc/{pid}/fd")
    except OSError:
        return False
    with descriptors:
        for descriptor in descriptors:
            try:
                if os.readlink(descriptor.path) != f"pipe:[{marker.inode}]":
                    continue
                info = os.stat(descriptor.path)
            except OSError:
                continue
            if (
                stat.S_ISFIFO(info.st_mode)
                and info.st_dev == marker.device
                and info.st_ino == marker.inode
            ):
                return True
    return False


def _collect_descendants(
    tracked: dict[int, _ProcessIdentity], marker: _OwnedMarker | None = None
) -> None:
    """Extend tracked by inherited marker or established PID-safe ancestry.

    The marker is a cooperative containment boundary. A descendant that closes it
    before first observation cannot safely be distinguished from an unrelated process.
    Once observed, the PID/start-time identity remains tracked after marker close.
    """
    if not sys.platform.startswith("linux"):
        return
    states: dict[int, tuple[_ProcessIdentity, int]] = {}
    try:
        entries = os.scandir("/proc")
    except OSError:  # pragma: no cover
        return
    with entries:
        for entry in entries:
            if not entry.name.isdigit():
                continue
            state = _proc_state(int(entry.name))
            if state is not None:
                states[state[0].pid] = state
    if marker is not None:
        for identity, _parent in states.values():
            if identity.pid != os.getpid() and _has_owned_marker(identity.pid, marker):
                tracked.setdefault(identity.pid, identity)
    parents = {pid for pid, identity in tracked.items() if _process_identity(pid) == identity}
    changed = True
    while changed:
        changed = False
        for identity, parent in states.values():
            if identity.pid not in tracked and parent in parents:
                tracked[identity.pid] = identity
                parents.add(identity.pid)
                changed = True


def _identity_alive(identity: _ProcessIdentity) -> bool:
    return _process_identity(identity.pid) == identity


def _signal_identity(identity: _ProcessIdentity, sig: signal.Signals) -> bool:
    """Signal only when PID and procfs start time still match the owned identity."""
    if not _identity_alive(identity):
        return False
    pidfd: int | None = None
    try:
        if hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal"):
            pidfd = os.pidfd_open(identity.pid)
            if not _identity_alive(identity):
                return False
            signal.pidfd_send_signal(pidfd, sig)
        else:  # pragma: no cover - Linux target provides pidfds
            os.kill(identity.pid, sig)
    except ProcessLookupError:
        return False
    finally:
        if pidfd is not None:
            os.close(pidfd)
    return True


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


def _reap_tracked(tracked: Mapping[int, _ProcessIdentity], *, root_pid: int) -> None:
    for identity in tuple(tracked.values()):
        if identity.pid == root_pid:
            continue
        try:
            os.waitpid(identity.pid, os.WNOHANG)
        except (ChildProcessError, ProcessLookupError):
            pass


def _terminate_owned_tree(
    process: subprocess.Popen[bytes],
    process_group: int,
    tracked: dict[int, _ProcessIdentity],
    marker: _OwnedMarker,
) -> bool:
    """Terminate the group plus tracked setsid descendants, then reap only owned PIDs."""
    _collect_descendants(tracked, marker)
    _signal_group(process_group, signal.SIGTERM)
    for identity in tuple(tracked.values()):
        _signal_identity(identity, signal.SIGTERM)
    deadline = time.monotonic() + 0.25
    while time.monotonic() < deadline:
        _collect_descendants(tracked, marker)
        _reap_tracked(tracked, root_pid=process.pid)
        if not any(_identity_alive(identity) for identity in tracked.values()):
            break
        time.sleep(0.01)
    _signal_group(process_group, signal.SIGKILL)
    for identity in tuple(tracked.values()):
        _signal_identity(identity, signal.SIGKILL)
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:  # pragma: no cover
        root = tracked.get(process.pid)
        if root is not None:
            _signal_identity(root, signal.SIGKILL)
        process.wait(timeout=1)
    deadline = time.monotonic() + 1
    while time.monotonic() < deadline:
        _collect_descendants(tracked, marker)
        _reap_tracked(tracked, root_pid=process.pid)
        if not any(_identity_alive(identity) for identity in tracked.values()):
            return True
        time.sleep(0.01)
    return not any(_identity_alive(identity) for identity in tracked.values())


def run_owned_process(
    argv: tuple[str, ...],
    *,
    cwd: Path,
    env: Mapping[str, str],
    timeout_seconds: int,
    no_progress_seconds: int,
) -> ProcessReceipt:
    """Run one owned process group with bounded output, time, cleanup, and receipts."""
    if not argv or any(type(arg) is not str or not arg or "\x00" in arg for arg in argv):
        raise ValueError("argv must contain nonempty safe strings")
    if timeout_seconds <= 0 or no_progress_seconds <= 0:
        raise ValueError("process timeouts must be positive")
    if not _PROCESS_LOCK.acquire(timeout=_PROCESS_LOCK_TIMEOUT_SECONDS):
        raise ProcessCoordinationTimeout("qualification process runner is busy")
    started = time.monotonic()
    stdout_hash = hashlib.sha256()
    stderr_hash = hashlib.sha256()
    timed_out = False
    exit_code: int | None = None
    process_group_reaped = True
    marker_read: int | None = None
    marker_write: int | None = None
    operation_error: BaseException | None = None

    try:
        previous_subreaper = _subreaper_state(True)
        if previous_subreaper is None:
            raise RuntimeError("Linux child-subreaper support is required")
        process: subprocess.Popen[bytes] | None = None
        selector = selectors.DefaultSelector()
        try:
            marker_read, marker_write, marker = _create_owned_marker()
            _PARENT_MARKER_FDS.add(marker_write)
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
                        pass_fds=(marker_read,),
                    )
                finally:
                    _close_fd(marker_read)
                    marker_read = None
            except OSError:
                return ProcessReceipt(
                    exit_code=None,
                    timed_out=False,
                    stdout_sha256=_EMPTY_SHA256,
                    stderr_sha256=_EMPTY_SHA256,
                    duration_ms=max(0, int((time.monotonic() - started) * 1000)),
                    process_group_reaped=True,
                    core_dumps_disabled=False,
                )

            process_group = process.pid
            root_identity = _process_identity(process.pid)
            if root_identity is None:  # pragma: no cover - Linux procfs is required by target
                raise RuntimeError("owned process identity is unavailable")
            tracked = {process.pid: root_identity}
            assert process.stdout is not None and process.stderr is not None
            for pipe, digest in ((process.stdout, stdout_hash), (process.stderr, stderr_hash)):
                os.set_blocking(pipe.fileno(), False)
                selector.register(pipe, selectors.EVENT_READ, digest)
            last_progress = time.monotonic()
            terminated = False
            termination_started: float | None = None
            while selector.get_map() or process.poll() is None:
                _collect_descendants(tracked, marker)
                now = time.monotonic()
                if not terminated and (
                    now - started >= timeout_seconds or now - last_progress >= no_progress_seconds
                ):
                    timed_out = True
                    process_group_reaped = _terminate_owned_tree(
                        process, process_group, tracked, marker
                    )
                    terminated = True
                    termination_started = time.monotonic()
                elif process.poll() is not None and not terminated:
                    process_group_reaped = _terminate_owned_tree(
                        process, process_group, tracked, marker
                    )
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
                process_group_reaped = _terminate_owned_tree(
                    process, process_group, tracked, marker
                )
            exit_code = process.wait(timeout=1)
            _collect_descendants(tracked, marker)
            if any(_identity_alive(identity) for identity in tracked.values()):
                process_group_reaped = _terminate_owned_tree(
                    process, process_group, tracked, marker
                )
            process_group_reaped = process_group_reaped and not _group_exists(process_group)
        except BaseException as error:
            operation_error = error
            if process is not None:
                identity = _process_identity(process.pid)
                tracked = {} if identity is None else {process.pid: identity}
                try:
                    _terminate_owned_tree(process, process.pid, tracked, marker)
                except BaseException:
                    error.add_note("failed to terminate the owned process tree")
            raise
        finally:
            selector.close()
            if process is not None:
                for pipe in (process.stdout, process.stderr):
                    if pipe is not None and not pipe.closed:
                        pipe.close()
            _close_fd(marker_read)
            if marker_write is not None:
                _PARENT_MARKER_FDS.discard(marker_write)
                _close_fd(marker_write)
            try:
                _restore_subreaper(previous_subreaper)
            except RuntimeError as restore_error:
                if operation_error is None:
                    raise
                operation_error.add_note(str(restore_error))
    finally:
        _PROCESS_LOCK.release()

    return ProcessReceipt(
        exit_code=exit_code,
        timed_out=timed_out,
        stdout_sha256=stdout_hash.hexdigest(),
        stderr_sha256=stderr_hash.hexdigest(),
        duration_ms=max(0, int((time.monotonic() - started) * 1000)),
        process_group_reaped=process_group_reaped,
        core_dumps_disabled=True,
    )
