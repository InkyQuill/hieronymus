"""Sanitized subprocess discovery and bounded process execution."""

from __future__ import annotations

import ctypes
import hashlib
import json
import os
import resource
import selectors
import signal
import stat
import subprocess
import sys
import tempfile
import time
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

_EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()
_PR_SET_PDEATHSIG = 1
_PR_SET_CHILD_SUBREAPER = 36
_PR_GET_CHILD_SUBREAPER = 37
_REPO_ROOT = Path(__file__).resolve().parents[2]
_ARTIFACT_ROOT = _REPO_ROOT / "qualification/.artifacts"
_UNSHARE_PATH = Path("/usr/bin/unshare")
_MAX_SUPERVISOR_MESSAGE = 4 * 1024 * 1024
_SUPERVISOR_GRACE_SECONDS = 3.0


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


def _validated_unshare_path(path: Path) -> Path:
    """Return a canonical namespace executable or a redacted failure."""
    try:
        resolved = path.resolve(strict=True)
        mode = resolved.stat().st_mode
    except OSError as exc:
        raise RuntimeError("Linux PID namespace isolation is unavailable") from exc
    if (
        not path.is_absolute()
        or resolved != path
        or not stat.S_ISREG(mode)
        or not os.access(resolved, os.X_OK)
    ):
        raise RuntimeError("Linux PID namespace isolation is unavailable")
    return resolved


def _validated_unshare() -> Path:
    """Validate the installed fixed namespace executable."""
    return _validated_unshare_path(_UNSHARE_PATH)


def _arm_namespace_launcher() -> bool:
    """Disable cores and make parent death kill the unshare wrapper after exec."""
    _disable_core_dumps()
    parent_pid = os.getppid()
    if parent_pid <= 1:
        return False
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(_PR_SET_PDEATHSIG, int(signal.SIGKILL), 0, 0, 0) != 0:
        return False
    return os.getppid() == parent_pid


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


@dataclass(frozen=True, slots=True)
class _ProcessIdentity:
    pid: int
    start_time: int


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


def _proc_snapshot() -> dict[int, tuple[_ProcessIdentity, int]]:
    if not sys.platform.startswith("linux"):
        return {}
    states: dict[int, tuple[_ProcessIdentity, int]] = {}
    try:
        entries = os.scandir("/proc")
    except OSError:  # pragma: no cover
        return {}
    with entries:
        for entry in entries:
            if not entry.name.isdigit():
                continue
            state = _proc_state(int(entry.name))
            if state is not None:
                states[state[0].pid] = state
    return states


def _collect_descendants(tracked: dict[int, _ProcessIdentity], *, owner_pid: int) -> None:
    """Collect only descendants of a dedicated process that spawns no unrelated child."""
    states = _proc_snapshot()
    parents = {owner_pid}
    parents.update(pid for pid, identity in tracked.items() if _process_identity(pid) == identity)
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


def _reap_children() -> None:
    while True:
        try:
            pid, _status = os.waitpid(-1, os.WNOHANG)
        except (ChildProcessError, ProcessLookupError):
            return
        if pid == 0:
            return


def _reap_tracked(tracked: Mapping[int, _ProcessIdentity], *, exclude_pid: int | None) -> None:
    for identity in tuple(tracked.values()):
        if identity.pid == exclude_pid:
            continue
        try:
            os.waitpid(identity.pid, os.WNOHANG)
        except (ChildProcessError, ProcessLookupError):
            pass


def _terminate_owned_tree(
    tracked: dict[int, _ProcessIdentity],
    *,
    owner_pid: int,
    root_process: subprocess.Popen[bytes] | None = None,
) -> bool:
    """Terminate PID/starttime identities; raw process-group signals are never used."""
    _collect_descendants(tracked, owner_pid=owner_pid)
    for identity in tuple(tracked.values()):
        _signal_identity(identity, signal.SIGTERM)
    deadline = time.monotonic() + 0.25
    while time.monotonic() < deadline:
        _collect_descendants(tracked, owner_pid=owner_pid)
        for identity in tuple(tracked.values()):
            _signal_identity(identity, signal.SIGTERM)
        if owner_pid == os.getpid():
            _reap_tracked(
                tracked,
                exclude_pid=None if root_process is None else root_process.pid,
            )
        if not any(_identity_alive(identity) for identity in tracked.values()):
            break
        time.sleep(0.01)
    for identity in tuple(tracked.values()):
        _signal_identity(identity, signal.SIGKILL)
    if root_process is not None:
        try:
            root_process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            root = tracked.get(root_process.pid)
            if root is not None:
                _signal_identity(root, signal.SIGKILL)
            root_process.wait(timeout=1)
    deadline = time.monotonic() + 1
    while time.monotonic() < deadline:
        _collect_descendants(tracked, owner_pid=owner_pid)
        for identity in tuple(tracked.values()):
            _signal_identity(identity, signal.SIGKILL)
        if owner_pid == os.getpid():
            _reap_tracked(
                tracked,
                exclude_pid=None if root_process is None else root_process.pid,
            )
        if not any(_identity_alive(identity) for identity in tracked.values()):
            return True
        time.sleep(0.01)
    return not any(_identity_alive(identity) for identity in tracked.values())


def _empty_spawn_receipt(started: float) -> ProcessReceipt:
    return ProcessReceipt(
        exit_code=None,
        timed_out=False,
        stdout_sha256=_EMPTY_SHA256,
        stderr_sha256=_EMPTY_SHA256,
        duration_ms=max(0, int((time.monotonic() - started) * 1000)),
        process_group_reaped=True,
        core_dumps_disabled=False,
    )


def _supervise_target(config: Mapping[str, object]) -> ProcessReceipt:
    """Run one target inside the single-purpose, single-threaded supervisor."""
    if resource.getrlimit(resource.RLIMIT_CORE) != (0, 0):
        raise RuntimeError("qualification supervisor core-dump suppression is unavailable")
    argv_value = config.get("argv")
    cwd_value = config.get("cwd")
    env_value = config.get("env")
    timeout_value = config.get("timeout_seconds")
    progress_value = config.get("no_progress_seconds")
    if (
        type(argv_value) is not list
        or not argv_value
        or any(type(item) is not str or not item or "\x00" in item for item in argv_value)
        or type(cwd_value) is not str
        or not cwd_value
        or "\x00" in cwd_value
        or type(env_value) is not dict
        or any(
            type(key) is not str or type(value) is not str or "\x00" in key or "\x00" in value
            for key, value in env_value.items()
        )
        or type(timeout_value) is not int
        or timeout_value <= 0
        or type(progress_value) is not int
        or progress_value <= 0
    ):
        raise RuntimeError("invalid supervisor configuration")
    if _subreaper_state(True) is None:
        raise RuntimeError("Linux child-subreaper support is required")

    started = time.monotonic()
    stdout_hash = hashlib.sha256()
    stderr_hash = hashlib.sha256()
    timed_out = False
    process: subprocess.Popen[bytes] | None = None
    selector = selectors.DefaultSelector()
    tracked: dict[int, _ProcessIdentity] = {}
    reaped = True
    try:
        try:
            process = subprocess.Popen(
                tuple(argv_value),
                cwd=cwd_value,
                env=env_value,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
                close_fds=True,
            )
        except OSError:
            return _empty_spawn_receipt(started)
        identity = _process_identity(process.pid)
        if identity is not None:
            tracked[process.pid] = identity
        assert process.stdout is not None and process.stderr is not None
        for pipe, digest in ((process.stdout, stdout_hash), (process.stderr, stderr_hash)):
            os.set_blocking(pipe.fileno(), False)
            selector.register(pipe, selectors.EVENT_READ, digest)
        last_progress = time.monotonic()
        terminated = False
        while selector.get_map() or process.poll() is None:
            _collect_descendants(tracked, owner_pid=os.getpid())
            now = time.monotonic()
            if not terminated and (
                now - started >= timeout_value or now - last_progress >= progress_value
            ):
                timed_out = True
                reaped = _terminate_owned_tree(tracked, owner_pid=os.getpid(), root_process=process)
                terminated = True
            elif process.poll() is not None and not terminated:
                reaped = _terminate_owned_tree(tracked, owner_pid=os.getpid(), root_process=process)
                terminated = True
            events = selector.select(timeout=0.05) if selector.get_map() else ()
            if not events and not selector.get_map():
                time.sleep(0.01)
            for key, _mask in events:
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
        if process.poll() is None:
            reaped = _terminate_owned_tree(tracked, owner_pid=os.getpid(), root_process=process)
        exit_code = process.wait(timeout=1)
        _collect_descendants(tracked, owner_pid=os.getpid())
        if any(_identity_alive(item) for item in tracked.values()):
            reaped = (
                _terminate_owned_tree(tracked, owner_pid=os.getpid(), root_process=process)
                and reaped
            )
        _reap_children()
        return ProcessReceipt(
            exit_code=exit_code,
            timed_out=timed_out,
            stdout_sha256=stdout_hash.hexdigest(),
            stderr_sha256=stderr_hash.hexdigest(),
            duration_ms=max(0, int((time.monotonic() - started) * 1000)),
            process_group_reaped=reaped
            and not any(_identity_alive(item) for item in tracked.values()),
            core_dumps_disabled=True,
        )
    finally:
        selector.close()
        if process is not None:
            for pipe in (process.stdout, process.stderr):
                if pipe is not None and not pipe.closed:
                    pipe.close()
            _collect_descendants(tracked, owner_pid=os.getpid())
            if any(_identity_alive(item) for item in tracked.values()):
                _terminate_owned_tree(tracked, owner_pid=os.getpid(), root_process=process)


def _write_all(fd: int, payload: bytes) -> None:
    view = memoryview(payload)
    while view:
        written = os.write(fd, view)
        view = view[written:]


def _write_limited(fd: int, payload: bytes, *, deadline: float) -> None:
    selector = selectors.DefaultSelector()
    view = memoryview(payload)
    try:
        os.set_blocking(fd, False)
        selector.register(fd, selectors.EVENT_WRITE)
        while view and time.monotonic() < deadline:
            events = selector.select(timeout=min(0.05, max(0.0, deadline - time.monotonic())))
            if not events:
                continue
            try:
                written = os.write(fd, view)
            except BlockingIOError:
                continue
            view = view[written:]
        if view:
            raise TimeoutError
    finally:
        selector.close()


def _read_limited(fd: int, *, deadline: float) -> bytes:
    selector = selectors.DefaultSelector()
    payload = bytearray()
    try:
        selector.register(fd, selectors.EVENT_READ)
        while time.monotonic() < deadline:
            events = selector.select(timeout=min(0.05, max(0.0, deadline - time.monotonic())))
            if not events:
                continue
            chunk = os.read(fd, 64 * 1024)
            if not chunk:
                return bytes(payload)
            payload.extend(chunk)
            if len(payload) > _MAX_SUPERVISOR_MESSAGE:
                raise RuntimeError("qualification supervisor returned an invalid receipt")
        raise TimeoutError
    finally:
        selector.close()


def _receipt_payload(receipt: ProcessReceipt) -> bytes:
    return json.dumps(
        {
            "core_dumps_disabled": receipt.core_dumps_disabled,
            "duration_ms": receipt.duration_ms,
            "exit_code": receipt.exit_code,
            "process_group_reaped": receipt.process_group_reaped,
            "stderr_sha256": receipt.stderr_sha256,
            "stdout_sha256": receipt.stdout_sha256,
            "timed_out": receipt.timed_out,
        },
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def _parse_receipt(payload: bytes) -> ProcessReceipt:
    try:
        value = json.loads(payload)
        if type(value) is not dict or set(value) != {
            "core_dumps_disabled",
            "duration_ms",
            "exit_code",
            "process_group_reaped",
            "stderr_sha256",
            "stdout_sha256",
            "timed_out",
        }:
            raise ValueError
        receipt = ProcessReceipt(**value)
        if (
            (receipt.exit_code is not None and type(receipt.exit_code) is not int)
            or type(receipt.timed_out) is not bool
            or type(receipt.duration_ms) is not int
            or receipt.duration_ms < 0
            or type(receipt.process_group_reaped) is not bool
            or type(receipt.core_dumps_disabled) is not bool
            or any(
                type(digest) is not str
                or len(digest) != 64
                or any(character not in "0123456789abcdef" for character in digest)
                for digest in (receipt.stdout_sha256, receipt.stderr_sha256)
            )
        ):
            raise ValueError
        return receipt
    except (TypeError, ValueError, json.JSONDecodeError) as exc:
        raise RuntimeError("qualification supervisor returned an invalid receipt") from exc


def _supervisor_command(config_fd: int, result_fd: int) -> tuple[str, ...]:
    return (
        sys.executable,
        str(Path(__file__).resolve()),
        "--supervisor",
        str(config_fd),
        str(result_fd),
    )


def _namespace_launcher_command(
    unshare: Path,
    config_fd: int,
    result_fd: int,
    supervisor_command: tuple[str, ...],
) -> tuple[str, ...]:
    return (
        sys.executable,
        str(Path(__file__).resolve()),
        "--namespace-launcher",
        str(unshare),
        str(config_fd),
        str(result_fd),
        *supervisor_command,
    )


def _pipe() -> tuple[int, int]:
    if hasattr(os, "pipe2"):
        return os.pipe2(os.O_CLOEXEC)
    read_fd, write_fd = os.pipe()  # pragma: no cover - Linux has pipe2
    os.set_inheritable(read_fd, False)
    os.set_inheritable(write_fd, False)
    return read_fd, write_fd


def _terminate_supervisor(process: subprocess.Popen[bytes]) -> None:
    identity = _process_identity(process.pid)
    if identity is None:
        return
    # The tracked process is the unshare wrapper. Its --kill-child contract and
    # the kernel's PID-namespace-init semantics terminate every namespace task.
    _signal_identity(identity, signal.SIGKILL)
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        _signal_identity(identity, signal.SIGKILL)
        process.wait(timeout=1)


def _validated_public_process_config(
    argv: tuple[str, ...],
    *,
    cwd: Path,
    env: Mapping[str, str],
    timeout_seconds: int,
    no_progress_seconds: int,
) -> tuple[tuple[str, ...], Path, dict[str, str]]:
    if (
        type(argv) is not tuple
        or not argv
        or any(type(arg) is not str or not arg or "\x00" in arg for arg in argv)
    ):
        raise ValueError("argv must be a tuple of nonempty exact strings")
    if not isinstance(cwd, Path) or not cwd.is_absolute():
        raise ValueError("cwd must be an absolute canonical directory Path")
    try:
        canonical_cwd = cwd.resolve(strict=True)
    except OSError as exc:
        raise ValueError("cwd must be an absolute canonical directory Path") from exc
    if canonical_cwd != cwd or not canonical_cwd.is_dir():
        raise ValueError("cwd must be an absolute canonical directory Path")
    if not isinstance(env, Mapping):
        raise TypeError("env must be a Mapping with exact string keys and values")
    try:
        env_items = tuple(env.items())
    except (AttributeError, RuntimeError) as exc:
        raise TypeError("env must be a Mapping with exact string keys and values") from exc
    if any(
        type(key) is not str
        or type(value) is not str
        or not key
        or "\x00" in key
        or "\x00" in value
        for key, value in env_items
    ):
        raise ValueError("env must have exact string keys and values")
    if (
        type(timeout_seconds) is not int
        or timeout_seconds <= 0
        or type(no_progress_seconds) is not int
        or no_progress_seconds <= 0
    ):
        raise ValueError("process timeouts must be positive exact integers")
    return argv, canonical_cwd, dict(env_items)


def run_owned_process(
    argv: tuple[str, ...],
    *,
    cwd: Path,
    env: Mapping[str, str],
    timeout_seconds: int,
    no_progress_seconds: int,
) -> ProcessReceipt:
    """Run inside a kernel PID lifetime boundary using inherited anonymous pipes."""
    argv, cwd, env_snapshot = _validated_public_process_config(
        argv,
        cwd=cwd,
        env=env,
        timeout_seconds=timeout_seconds,
        no_progress_seconds=no_progress_seconds,
    )
    unshare = _validated_unshare()
    config_read, config_write = _pipe()
    result_read, result_write = _pipe()
    supervisor: subprocess.Popen[bytes] | None = None
    try:
        supervisor = subprocess.Popen(
            _namespace_launcher_command(
                unshare,
                config_read,
                result_write,
                _supervisor_command(config_read, result_write),
            ),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC"},
            close_fds=True,
            pass_fds=(config_read, result_write),
        )
        _close_fd(config_read)
        config_read = -1
        _close_fd(result_write)
        result_write = -1
        config = json.dumps(
            {
                "argv": list(argv),
                "cwd": os.fspath(cwd),
                "env": env_snapshot,
                "no_progress_seconds": no_progress_seconds,
                "timeout_seconds": timeout_seconds,
            },
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        if len(config) > _MAX_SUPERVISOR_MESSAGE:
            raise ValueError("supervisor configuration is too large")
        deadline = time.monotonic() + timeout_seconds + _SUPERVISOR_GRACE_SECONDS
        try:
            _write_limited(config_write, config, deadline=deadline)
            _close_fd(config_write)
            config_write = -1
            payload = _read_limited(result_read, deadline=deadline)
        except TimeoutError as exc:
            raise RuntimeError("qualification supervisor did not return a receipt") from exc
        if supervisor.wait(timeout=max(0.1, deadline - time.monotonic())) != 0:
            raise RuntimeError("qualification supervisor failed")
        return _parse_receipt(payload)
    except (BrokenPipeError, OSError, subprocess.TimeoutExpired) as exc:
        raise RuntimeError("qualification supervisor failed") from exc
    finally:
        for descriptor in (config_read, config_write, result_read, result_write):
            _close_fd(descriptor)
        if supervisor is not None and supervisor.poll() is None:
            _terminate_supervisor(supervisor)


def _supervisor_entry(config_fd: int, result_fd: int) -> int:
    try:
        payload = bytearray()
        while True:
            chunk = os.read(config_fd, 64 * 1024)
            if not chunk:
                break
            payload.extend(chunk)
            if len(payload) > _MAX_SUPERVISOR_MESSAGE:
                raise RuntimeError("invalid supervisor configuration")
        value = json.loads(payload)
        if type(value) is not dict:
            raise RuntimeError("invalid supervisor configuration")
        receipt = _supervise_target(value)
        _write_all(result_fd, _receipt_payload(receipt))
        return 0
    except BaseException:
        return 1
    finally:
        _close_fd(config_fd)
        _close_fd(result_fd)


def _namespace_launcher_entry(unshare: Path, supervisor_command: tuple[str, ...]) -> int:
    """Become the fixed unshare wrapper before any configuration is read."""
    try:
        unshare = _validated_unshare_path(unshare)
        if not _arm_namespace_launcher():
            return 1
        os.execv(
            str(unshare),
            (
                str(unshare),
                "--user",
                "--map-root-user",
                "--pid",
                "--fork",
                "--mount-proc",
                "--kill-child=SIGKILL",
                "--",
                *supervisor_command,
            ),
        )
    except BaseException:
        return 1
    return 1


if __name__ == "__main__":  # pragma: no cover - exercised through run_owned_process
    if len(sys.argv) == 4 and sys.argv[1] == "--supervisor":
        raise SystemExit(_supervisor_entry(int(sys.argv[2]), int(sys.argv[3])))
    if len(sys.argv) >= 6 and sys.argv[1] == "--namespace-launcher":
        raise SystemExit(_namespace_launcher_entry(Path(sys.argv[2]), tuple(sys.argv[5:])))
    raise SystemExit(2)
