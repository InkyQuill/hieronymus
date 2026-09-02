"""Explicit checksum-verified acquisition of qualification-only artifacts."""

from __future__ import annotations

import argparse
import contextvars
import errno
import fcntl
import hashlib
import json
import os
import shutil
import socket
import stat
import sys
import tarfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Iterator, Sequence
from contextlib import contextmanager
from pathlib import Path, PurePosixPath
from typing import BinaryIO, Final

_SEMANTIC_MODEL_INITIAL_URL: Final = (
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/"
    "9a53d751e60e6dd34f2443711d44d5b09389f89a/onnx/model.onnx"
)
_ALLOWED_MODEL_HOSTS: Final = frozenset(
    {
        "huggingface.co",
        "cdn-lfs.huggingface.co",
        "cas-bridge.xethub.hf.co",
        "us.aws.cdn.hf.co",
    }
)
_ONNX_RUNTIME_INITIAL_URL: Final = (
    "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/"
    "onnxruntime-linux-x64-1.28.0.tgz"
)
_ALLOWED_ONNX_RUNTIME_HOSTS: Final = frozenset(
    {
        "github.com",
        "release-assets.githubusercontent.com",
        "objects.githubusercontent.com",
    }
)
_REDIRECT_STATUSES: Final = frozenset({301, 302, 303, 307, 308})
_MAX_REDIRECTS: Final = 5
_CHUNK_SIZE: Final = 1024 * 1024
_MAX_MODEL_BYTES: Final = 256 * 1024 * 1024
_MAX_ONNX_RUNTIME_ARCHIVE_BYTES: Final = 512 * 1024 * 1024
_MAX_ONNX_RUNTIME_EXTRACTED_BYTES: Final = 2 * 1024 * 1024 * 1024
_TRANSPORT_TIMEOUT_SECONDS: Final = 120
_ACQUISITION_TIMEOUT_SECONDS: Final = 300
_LOCK_RETRY_SECONDS: Final = 0.01
_MODEL_DESTINATION: Final = Path("qualification/.artifacts/models/all-MiniLM-L6-v2/model.onnx")
_MODEL_DIRECTORY_COMPONENTS: Final = ("qualification", ".artifacts", "models", "all-MiniLM-L6-v2")
_DESTINATION_NAME: Final = "model.onnx"
_PARTIAL_NAME: Final = "model.onnx.part"
# Persistent advisory evidence; the abstract socket remains the unlink-proof mutex.
_LOCK_NAME: Final = "model.onnx.lock"
_DIRECTORY_FLAGS: Final = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
_RUNTIME_PARENT_COMPONENTS: Final = ("qualification", ".artifacts", "models")
_RUNTIME_TOP_DIRECTORY: Final = "onnxruntime-linux-x64-1.28.0"
_RUNTIME_ARCHIVE_PART_NAME: Final = f"{_RUNTIME_TOP_DIRECTORY}.tgz.part"
_RUNTIME_DIRECTORY_PART_NAME: Final = f"{_RUNTIME_TOP_DIRECTORY}.part"
_RUNTIME_LIBRARY: Final = Path("lib/libonnxruntime.so")
_monotonic = time.monotonic
_sleep = time.sleep


class _Deadline:
    """One monotonic budget shared by every acquisition phase."""

    def __init__(self, expires_at: float) -> None:
        self._expires_at = expires_at

    @classmethod
    def after(cls, seconds: float) -> _Deadline:
        return cls(_monotonic() + seconds)

    def remaining(self) -> float:
        remaining = self._expires_at - _monotonic()
        if remaining <= 0:
            raise AcquisitionError("semantic model acquisition deadline exceeded")
        return remaining

    def check(self) -> None:
        self.remaining()


_CURRENT_DEADLINE: contextvars.ContextVar[_Deadline | None] = contextvars.ContextVar(
    "qualification_acquisition_deadline",
    default=None,
)
_MUTEX_TRACKING_LOCK = threading.Lock()
_ACTIVE_MUTEX_SOCKETS: set[socket.socket] = set()
_ACTIVE_ADVISORY_LOCK_FDS: set[int] = set()


def _before_fork() -> None:
    _MUTEX_TRACKING_LOCK.acquire()


def _after_fork_parent() -> None:
    _MUTEX_TRACKING_LOCK.release()


def _after_fork_child() -> None:
    try:
        for mutex_socket in _ACTIVE_MUTEX_SOCKETS:
            try:
                mutex_socket.close()
            except OSError:
                pass
        _ACTIVE_MUTEX_SOCKETS.clear()
        for lock_fd in _ACTIVE_ADVISORY_LOCK_FDS:
            try:
                os.close(lock_fd)
            except OSError:
                pass
        _ACTIVE_ADVISORY_LOCK_FDS.clear()
    finally:
        _MUTEX_TRACKING_LOCK.release()


os.register_at_fork(
    before=_before_fork,
    after_in_parent=_after_fork_parent,
    after_in_child=_after_fork_child,
)


class AcquisitionError(RuntimeError):
    """A non-sensitive deterministic acquisition failure."""


def _require_private_regular(metadata: os.stat_result, subject: str) -> None:
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o600
        or metadata.st_nlink != 1
    ):
        raise AcquisitionError(f"{subject} must be a private regular file")


def _require_trusted_directory(metadata: os.stat_result) -> None:
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) & 0o022
    ):
        raise AcquisitionError("artifact directory is not a trusted directory")


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(  # type: ignore[override]
        self,
        req: urllib.request.Request,
        fp: BinaryIO,
        code: int,
        msg: str,
        headers: object,
        newurl: str,
    ) -> None:
        return None


def _open_no_redirect(request: urllib.request.Request) -> BinaryIO:
    """Open one HTTPS request without proxying or automatic redirects."""
    opener = urllib.request.build_opener(
        urllib.request.ProxyHandler({}),
        _NoRedirect(),
        urllib.request.HTTPSHandler(),
    )
    try:
        timeout = getattr(request, "_hieronymus_timeout", _TRANSPORT_TIMEOUT_SECONDS)
        return opener.open(request, timeout=timeout)
    except urllib.error.HTTPError as error:
        return error


def _safe_url(url: str) -> str:
    """Validate one URL without returning or reporting its sensitive query."""
    try:
        parsed = urllib.parse.urlsplit(url)
        port = parsed.port
    except (TypeError, ValueError):
        raise AcquisitionError("invalid acquisition URL") from None
    if parsed.scheme != "https":
        raise AcquisitionError("acquisition URL must use HTTPS")
    if parsed.username is not None or parsed.password is not None:
        raise AcquisitionError("acquisition URL must not contain user information")
    if parsed.fragment:
        raise AcquisitionError("acquisition URL must not contain a fragment")
    if parsed.hostname not in _ALLOWED_MODEL_HOSTS or port is not None:
        raise AcquisitionError("acquisition URL host is not allowed")
    if parsed.netloc != parsed.hostname:
        raise AcquisitionError("acquisition URL host is not canonical")
    return urllib.parse.urlunsplit(parsed)


def _safe_runtime_url(url: str) -> str:
    """Validate one runtime URL without reflecting its signed query."""
    try:
        parsed = urllib.parse.urlsplit(url)
        port = parsed.port
    except (TypeError, ValueError):
        raise AcquisitionError("invalid ONNX Runtime acquisition URL") from None
    if parsed.scheme != "https":
        raise AcquisitionError("ONNX Runtime acquisition URL must use HTTPS")
    if parsed.username is not None or parsed.password is not None:
        raise AcquisitionError("ONNX Runtime acquisition URL must not contain user information")
    if parsed.fragment:
        raise AcquisitionError("ONNX Runtime acquisition URL must not contain a fragment")
    if parsed.hostname not in _ALLOWED_ONNX_RUNTIME_HOSTS or port is not None:
        raise AcquisitionError("ONNX Runtime acquisition URL host is not allowed")
    if parsed.netloc != parsed.hostname:
        raise AcquisitionError("ONNX Runtime acquisition URL host is not canonical")
    return urllib.parse.urlunsplit(parsed)


def _load_model_spec(repo_root: Path) -> tuple[str, str]:
    try:
        raw = json.loads(
            (repo_root / "qualification/prerequisites.json").read_text(encoding="utf-8")
        )
        model = raw["semantic_model"]
        url = model["url"]
        expected_sha256 = model["sha256"]
    except (
        FileNotFoundError,
        OSError,
        UnicodeError,
        json.JSONDecodeError,
        KeyError,
        TypeError,
    ):
        raise AcquisitionError("invalid qualification prerequisites") from None
    if not isinstance(url, str) or url != _SEMANTIC_MODEL_INITIAL_URL:
        raise AcquisitionError(
            "semantic model initial URL does not match the approved prerequisite"
        )
    if (
        not isinstance(expected_sha256, str)
        or len(expected_sha256) != 64
        or any(character not in "0123456789abcdef" for character in expected_sha256)
    ):
        raise AcquisitionError("semantic model checksum is invalid")
    return _safe_url(url), expected_sha256


def _load_runtime_spec(repo_root: Path) -> tuple[str, str, Path]:
    try:
        raw = json.loads(
            (repo_root / "qualification/prerequisites.json").read_text(encoding="utf-8")
        )
        runtime = raw["onnx_runtime"]
    except (
        FileNotFoundError,
        OSError,
        UnicodeError,
        json.JSONDecodeError,
        KeyError,
        TypeError,
    ):
        raise AcquisitionError("invalid qualification prerequisites") from None
    expected = {
        "version": "1.28.0",
        "target": "linux-x64",
        "url": _ONNX_RUNTIME_INITIAL_URL,
        "library": _RUNTIME_LIBRARY.as_posix(),
    }
    if not isinstance(runtime, dict) or any(
        runtime.get(key) != value for key, value in expected.items()
    ):
        raise AcquisitionError("ONNX Runtime prerequisite does not match the approved artifact")
    expected_sha256 = runtime.get("sha256")
    if (
        not isinstance(expected_sha256, str)
        or len(expected_sha256) != 64
        or any(character not in "0123456789abcdef" for character in expected_sha256)
    ):
        raise AcquisitionError("ONNX Runtime checksum is invalid")
    return _safe_runtime_url(runtime["url"]), expected_sha256, _RUNTIME_LIBRARY


def _remove_stale_partial(model_dir_fd: int) -> None:
    try:
        metadata = os.stat(_PARTIAL_NAME, dir_fd=model_dir_fd, follow_symlinks=False)
    except FileNotFoundError:
        return
    except OSError:
        raise AcquisitionError("could not inspect the semantic model partial file") from None
    _require_private_regular(metadata, "semantic model partial")
    try:
        os.unlink(_PARTIAL_NAME, dir_fd=model_dir_fd)
        os.fsync(model_dir_fd)
    except OSError:
        raise AcquisitionError("could not remove the semantic model partial file") from None


def _sha256_fd(fd: int, deadline: _Deadline) -> str:
    metadata = os.fstat(fd)
    if metadata.st_size > _MAX_MODEL_BYTES:
        raise AcquisitionError("semantic model destination exceeds the byte limit")
    digest = hashlib.sha256()
    offset = 0
    while True:
        deadline.check()
        chunk = os.pread(fd, min(_CHUNK_SIZE, _MAX_MODEL_BYTES - offset + 1), offset)
        deadline.check()
        if not chunk:
            break
        offset += len(chunk)
        if offset > _MAX_MODEL_BYTES:
            raise AcquisitionError("semantic model destination exceeds the byte limit")
        digest.update(chunk)
    return digest.hexdigest()


def _existing_destination_is_verified(
    model_dir_fd: int,
    expected_sha256: str,
    deadline: _Deadline,
) -> bool:
    try:
        fd = os.open(
            _DESTINATION_NAME,
            os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
            dir_fd=model_dir_fd,
        )
    except FileNotFoundError:
        return False
    except OSError:
        raise AcquisitionError("could not inspect the semantic model destination") from None
    try:
        metadata = os.fstat(fd)
        _require_private_regular(metadata, "semantic model destination")
        try:
            return _sha256_fd(fd, deadline) == expected_sha256
        except OSError:
            raise AcquisitionError("could not verify the existing semantic model") from None
    finally:
        os.close(fd)


def _canonical_repo_root(repo_root: Path) -> Path:
    try:
        canonical = repo_root.resolve(strict=True)
        if not canonical.is_dir():
            raise AcquisitionError("repository root must be a directory")
        return canonical
    except AcquisitionError:
        raise
    except (OSError, RuntimeError):
        raise AcquisitionError("repository root is invalid") from None


@contextmanager
def _open_model_directory(repo_root: Path) -> Iterator[tuple[Path, tuple[int, ...]]]:
    canonical_root = _canonical_repo_root(repo_root)
    opened: list[int] = []
    try:
        current_fd = os.open(canonical_root, _DIRECTORY_FLAGS)
        opened.append(current_fd)
        for component in _MODEL_DIRECTORY_COMPONENTS:
            _current_deadline().check()
            created = False
            try:
                os.mkdir(component, mode=0o700, dir_fd=current_fd)
                created = True
            except FileExistsError:
                pass
            if created:
                os.fsync(current_fd)
            try:
                child_fd = os.open(component, _DIRECTORY_FLAGS, dir_fd=current_fd)
            except OSError:
                raise AcquisitionError("artifact directory is not a trusted directory") from None
            opened.append(child_fd)
            if created:
                try:
                    os.fchmod(child_fd, 0o700)
                    os.fsync(child_fd)
                except OSError:
                    raise AcquisitionError(
                        "could not secure the semantic model directory"
                    ) from None
            _require_trusted_directory(os.fstat(child_fd))
            current_fd = child_fd
            _current_deadline().check()
    except AcquisitionError:
        for fd in reversed(opened):
            os.close(fd)
        raise
    except OSError:
        for fd in reversed(opened):
            os.close(fd)
        raise AcquisitionError("could not prepare the semantic model directory") from None
    try:
        yield canonical_root, tuple(opened)
    finally:
        for fd in reversed(opened):
            os.close(fd)


@contextmanager
def _open_runtime_parent(repo_root: Path) -> Iterator[tuple[Path, tuple[int, ...]]]:
    canonical_root = _canonical_repo_root(repo_root)
    opened: list[int] = []
    try:
        current_fd = os.open(canonical_root, _DIRECTORY_FLAGS)
        opened.append(current_fd)
        for component in _RUNTIME_PARENT_COMPONENTS:
            _current_deadline().check()
            created = False
            try:
                os.mkdir(component, mode=0o700, dir_fd=current_fd)
                created = True
            except FileExistsError:
                pass
            if created:
                os.fsync(current_fd)
            try:
                child_fd = os.open(component, _DIRECTORY_FLAGS, dir_fd=current_fd)
            except OSError:
                raise AcquisitionError("ONNX Runtime artifact directory is not trusted") from None
            opened.append(child_fd)
            if created:
                os.fchmod(child_fd, 0o700)
                os.fsync(child_fd)
            _require_trusted_directory(os.fstat(child_fd))
            current_fd = child_fd
    except AcquisitionError:
        for fd in reversed(opened):
            os.close(fd)
        raise
    except OSError:
        for fd in reversed(opened):
            os.close(fd)
        raise AcquisitionError("could not prepare the ONNX Runtime directory") from None
    try:
        yield canonical_root, tuple(opened)
    finally:
        for fd in reversed(opened):
            os.close(fd)


def _validate_directory_chain(
    canonical_root: Path,
    directory_fds: tuple[int, ...],
) -> None:
    try:
        lexical_root = os.stat(canonical_root, follow_symlinks=False)
        held_root = os.fstat(directory_fds[0])
    except OSError:
        raise AcquisitionError("artifact directory identity changed") from None
    if (lexical_root.st_dev, lexical_root.st_ino) != (held_root.st_dev, held_root.st_ino):
        raise AcquisitionError("artifact directory identity changed")
    for parent_fd, expected_fd, component in zip(
        directory_fds[:-1],
        directory_fds[1:],
        _MODEL_DIRECTORY_COMPONENTS,
        strict=True,
    ):
        try:
            current_fd = os.open(component, _DIRECTORY_FLAGS, dir_fd=parent_fd)
        except OSError:
            raise AcquisitionError("artifact directory identity changed") from None
        try:
            current = os.fstat(current_fd)
            expected = os.fstat(expected_fd)
            _require_trusted_directory(current)
            _require_trusted_directory(expected)
            if (current.st_dev, current.st_ino) != (expected.st_dev, expected.st_ino):
                raise AcquisitionError("artifact directory identity changed")
        finally:
            os.close(current_fd)


def _current_deadline() -> _Deadline:
    deadline = _CURRENT_DEADLINE.get()
    if deadline is None:
        raise AcquisitionError("semantic model acquisition deadline is unavailable")
    return deadline


def _mutex_address(canonical_root: Path) -> bytes:
    identity = os.fsencode(canonical_root) + b"\0" + os.fsencode(_MODEL_DESTINATION)
    digest = hashlib.sha256(identity).hexdigest().encode("ascii")
    return b"\0hieronymus-acquire-" + digest


@contextmanager
def _kernel_mutex(canonical_root: Path, deadline: _Deadline) -> Iterator[None]:
    """Hold an unlink-proof Linux kernel mutex for this repository/model pair."""
    address = _mutex_address(canonical_root)
    held: socket.socket | None = None
    while held is None:
        deadline.check()
        candidate = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM | socket.SOCK_CLOEXEC)
        try:
            with _MUTEX_TRACKING_LOCK:
                try:
                    candidate.bind(address)
                except OSError as error:
                    if error.errno != errno.EADDRINUSE:
                        raise AcquisitionError(
                            "could not lock semantic model acquisition"
                        ) from None
                else:
                    _ACTIVE_MUTEX_SOCKETS.add(candidate)
                    held = candidate
        finally:
            if held is not candidate:
                candidate.close()
        if held is None:
            _sleep(min(_LOCK_RETRY_SECONDS, deadline.remaining()))
    owner_pid = os.getpid()
    try:
        yield
    finally:
        if os.getpid() == owner_pid:
            with _MUTEX_TRACKING_LOCK:
                _ACTIVE_MUTEX_SOCKETS.discard(held)
                held.close()


def _validate_lock_file(model_dir_fd: int, lock_fd: int) -> None:
    try:
        held = os.fstat(lock_fd)
        named = os.stat(_LOCK_NAME, dir_fd=model_dir_fd, follow_symlinks=False)
    except OSError:
        raise AcquisitionError("semantic model acquisition lock identity changed") from None
    identity_matches = (held.st_dev, held.st_ino) == (named.st_dev, named.st_ino)
    _require_private_regular(held, "semantic model acquisition lock")
    _require_private_regular(named, "semantic model acquisition lock")
    if not identity_matches:
        raise AcquisitionError("semantic model acquisition lock must be a private regular file")


@contextmanager
def _acquisition_lock(model_dir_fd: int) -> Iterator[int]:
    deadline = _current_deadline()
    created = False
    lock_fd: int
    with _MUTEX_TRACKING_LOCK:
        try:
            try:
                lock_fd = os.open(
                    _LOCK_NAME,
                    os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                    0o600,
                    dir_fd=model_dir_fd,
                )
                created = True
            except FileExistsError:
                lock_fd = os.open(
                    _LOCK_NAME,
                    os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC,
                    dir_fd=model_dir_fd,
                )
        except OSError:
            raise AcquisitionError("could not open the semantic model acquisition lock") from None
        _ACTIVE_ADVISORY_LOCK_FDS.add(lock_fd)
    owner_pid = os.getpid()
    try:
        if created:
            os.fsync(model_dir_fd)
        _validate_lock_file(model_dir_fd, lock_fd)
        while True:
            deadline.check()
            try:
                fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                _sleep(min(_LOCK_RETRY_SECONDS, deadline.remaining()))
            except OSError:
                raise AcquisitionError("could not lock semantic model acquisition") from None
        _validate_lock_file(model_dir_fd, lock_fd)
        yield lock_fd
        if os.getpid() != owner_pid:
            return
        _validate_lock_file(model_dir_fd, lock_fd)
    finally:
        if os.getpid() == owner_pid:
            with _MUTEX_TRACKING_LOCK:
                _ACTIVE_ADVISORY_LOCK_FDS.discard(lock_fd)
                os.close(lock_fd)


def _response_status(response: object) -> int:
    status = getattr(response, "status", None)
    if status is None:
        getcode = getattr(response, "getcode", None)
        status = getcode() if callable(getcode) else None
    if not isinstance(status, int):
        raise AcquisitionError("acquisition response has no valid status")
    return status


def _redirect_location(response: object) -> str:
    headers = getattr(response, "headers", None)
    location = headers.get("Location") if headers is not None else None
    if not isinstance(location, str) or not location:
        raise AcquisitionError("acquisition redirect has no valid location")
    return location


def _request(url: str, deadline: _Deadline | None = None) -> urllib.request.Request:
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/octet-stream",
            "Accept-Encoding": "identity",
            "User-Agent": "hieronymus-qualification-acquire/1",
        },
        method="GET",
    )
    forbidden = {"authorization", "proxy-authorization", "cookie"}
    if forbidden & {key.lower() for key, _value in request.header_items()}:
        raise AcquisitionError("credential-bearing acquisition request rejected")
    if deadline is not None:
        request._hieronymus_timeout = min(  # type: ignore[attr-defined]
            _TRANSPORT_TIMEOUT_SECONDS,
            deadline.remaining(),
        )
    return request


def _content_length(response: object) -> int | None:
    headers = getattr(response, "headers", None)
    raw = headers.get("Content-Length") if headers is not None else None
    if raw is None:
        return None
    try:
        length = int(raw, 10)
    except (TypeError, ValueError):
        raise AcquisitionError("semantic model response has an invalid content length") from None
    if length < 0:
        raise AcquisitionError("semantic model response has an invalid content length")
    return length


def _set_response_read_timeout(response: object, timeout: float) -> None:
    """Rearm a fake or urllib HTTPS response's underlying socket for one read."""
    is_closed = getattr(response, "isclosed", None)
    if getattr(response, "fp", object()) is None and callable(is_closed) and is_closed():
        # http.client closes zero-length responses before the first caller read;
        # their subsequent read is a non-blocking EOF and needs no socket timeout.
        return
    candidates = [response]
    seen: set[int] = set()
    while candidates and len(seen) < 12:
        candidate = candidates.pop(0)
        identity = id(candidate)
        if identity in seen:
            continue
        seen.add(identity)
        setter = getattr(candidate, "settimeout", None)
        if callable(setter):
            try:
                setter(timeout)
            except Exception:
                raise AcquisitionError("could not set semantic model response timeout") from None
            return
        for attribute in ("fp", "raw", "_sock"):
            nested = getattr(candidate, attribute, None)
            if nested is not None:
                candidates.append(nested)
    raise AcquisitionError("could not set semantic model response timeout")


def _stream_model(
    initial_url: str,
    model_dir_fd: int,
    expected_sha256: str,
    deadline: _Deadline,
) -> int:
    current_url = initial_url
    redirects = 0
    while True:
        deadline.check()
        try:
            response = _open_no_redirect(_request(current_url, deadline))
        except Exception as error:
            if isinstance(error, AcquisitionError):
                raise
            raise AcquisitionError("semantic model request failed") from None
        partial_fd: int | None = None
        completed = False
        try:
            deadline.check()
            status = _response_status(response)
            if status in _REDIRECT_STATUSES:
                if redirects >= _MAX_REDIRECTS:
                    raise AcquisitionError("semantic model redirect limit exceeded")
                location = _redirect_location(response)
                current_url = _safe_url(urllib.parse.urljoin(current_url, location))
                redirects += 1
                continue
            if status != 200:
                raise AcquisitionError("semantic model request returned an unexpected status")

            declared_length = _content_length(response)
            if declared_length is not None and declared_length > _MAX_MODEL_BYTES:
                raise AcquisitionError("semantic model response exceeds the byte limit")

            digest = hashlib.sha256()
            total = 0
            try:
                partial_fd = os.open(
                    _PARTIAL_NAME,
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                    0o600,
                    dir_fd=model_dir_fd,
                )
            except OSError:
                raise AcquisitionError("could not create the semantic model partial file") from None
            _require_private_regular(
                os.fstat(partial_fd),
                "semantic model partial",
            )
            try:
                os.fsync(model_dir_fd)
            except OSError:
                raise AcquisitionError(
                    "could not persist the semantic model partial entry"
                ) from None
            try:
                destination = os.fdopen(os.dup(partial_fd), "wb")
            except OSError:
                raise AcquisitionError("could not open the semantic model partial file") from None
            with destination:
                while True:
                    timeout = min(_TRANSPORT_TIMEOUT_SECONDS, deadline.remaining())
                    _set_response_read_timeout(response, timeout)
                    deadline.check()
                    try:
                        chunk = response.read(_CHUNK_SIZE)
                    except Exception as error:
                        if isinstance(error, AcquisitionError):
                            raise
                        raise AcquisitionError("semantic model response read failed") from None
                    deadline.check()
                    if not chunk:
                        break
                    if not isinstance(chunk, bytes):
                        raise AcquisitionError("semantic model response was not binary")
                    total += len(chunk)
                    if total > _MAX_MODEL_BYTES:
                        raise AcquisitionError("semantic model response exceeds the byte limit")
                    destination.write(chunk)
                    digest.update(chunk)
                destination.flush()
            os.fsync(partial_fd)
            if declared_length is not None and total != declared_length:
                raise AcquisitionError(
                    "semantic model response length did not match its declaration"
                )
            if digest.hexdigest() != expected_sha256:
                raise AcquisitionError("semantic model checksum mismatch")
            completed = True
        except Exception:
            if partial_fd is not None:
                _remove_owned_partial(model_dir_fd, partial_fd)
                os.close(partial_fd)
                partial_fd = None
            raise
        finally:
            try:
                response.close()
            except Exception:
                if partial_fd is not None:
                    _remove_owned_partial(model_dir_fd, partial_fd)
                    os.close(partial_fd)
                    partial_fd = None
                raise AcquisitionError("could not close semantic model response") from None
        if completed and partial_fd is not None:
            return partial_fd


def _promote_open_partial(model_dir_fd: int, partial_fd: int) -> None:
    written = os.fstat(partial_fd)
    _require_private_regular(written, "semantic model partial")
    try:
        named = os.stat(_PARTIAL_NAME, dir_fd=model_dir_fd, follow_symlinks=False)
    except OSError:
        raise AcquisitionError("semantic model partial identity changed before promotion") from None
    _require_private_regular(named, "semantic model partial")
    if named.st_dev != written.st_dev or named.st_ino != written.st_ino:
        raise AcquisitionError("semantic model partial identity changed before promotion")
    try:
        os.replace(
            _PARTIAL_NAME,
            _DESTINATION_NAME,
            src_dir_fd=model_dir_fd,
            dst_dir_fd=model_dir_fd,
        )
        os.fsync(model_dir_fd)
    except OSError:
        raise AcquisitionError("could not promote the semantic model") from None


def _remove_owned_partial(model_dir_fd: int, partial_fd: int) -> None:
    try:
        owned = os.fstat(partial_fd)
        named = os.stat(_PARTIAL_NAME, dir_fd=model_dir_fd, follow_symlinks=False)
    except (FileNotFoundError, OSError):
        return
    if stat.S_ISREG(named.st_mode) and (named.st_dev, named.st_ino) == (
        owned.st_dev,
        owned.st_ino,
    ):
        try:
            os.unlink(_PARTIAL_NAME, dir_fd=model_dir_fd)
            os.fsync(model_dir_fd)
        except OSError:
            return


def _final_validation_hook(_stage: str) -> None:
    """A deterministic race-injection seam; production calls leave state unchanged."""


def _same_file_version(left: os.stat_result, right: os.stat_result) -> bool:
    return (
        left.st_dev,
        left.st_ino,
        left.st_size,
        left.st_mtime_ns,
        left.st_ctime_ns,
    ) == (
        right.st_dev,
        right.st_ino,
        right.st_size,
        right.st_mtime_ns,
        right.st_ctime_ns,
    )


def _validate_lexical_destination(
    canonical_root: Path,
    directory_fds: tuple[int, ...],
    expected: os.stat_result,
) -> Path:
    """Resolve and lstat the complete lexical path at the success boundary."""
    destination = canonical_root / _MODEL_DESTINATION
    try:
        resolved = destination.resolve(strict=True)
        resolved.relative_to(canonical_root)
    except (OSError, RuntimeError, ValueError):
        raise AcquisitionError("semantic model destination escaped the repository") from None
    if resolved != destination:
        raise AcquisitionError("semantic model destination escaped the repository")

    current = canonical_root
    try:
        root_metadata = os.lstat(current)
        held_root = os.fstat(directory_fds[0])
    except OSError:
        raise AcquisitionError("artifact directory identity changed") from None
    if stat.S_ISLNK(root_metadata.st_mode) or (
        root_metadata.st_dev,
        root_metadata.st_ino,
    ) != (held_root.st_dev, held_root.st_ino):
        raise AcquisitionError("artifact directory identity changed")

    for component, held_fd in zip(
        _MODEL_DIRECTORY_COMPONENTS,
        directory_fds[1:],
        strict=True,
    ):
        current /= component
        try:
            lexical = os.lstat(current)
            held = os.fstat(held_fd)
        except OSError:
            raise AcquisitionError("artifact directory identity changed") from None
        if stat.S_ISLNK(lexical.st_mode):
            raise AcquisitionError("artifact directory identity changed")
        _require_trusted_directory(lexical)
        _require_trusted_directory(held)
        if (lexical.st_dev, lexical.st_ino) != (held.st_dev, held.st_ino):
            raise AcquisitionError("artifact directory identity changed")

    try:
        lexical_destination = os.lstat(destination)
    except OSError:
        raise AcquisitionError("semantic model destination identity changed") from None
    if stat.S_ISLNK(lexical_destination.st_mode):
        raise AcquisitionError("semantic model destination identity changed")
    _require_private_regular(lexical_destination, "semantic model destination")
    if not _same_file_version(expected, lexical_destination):
        raise AcquisitionError("semantic model destination identity changed")
    return destination


def _verified_return_path(
    canonical_root: Path,
    directory_fds: tuple[int, ...],
    lock_fd: int,
    expected_sha256: str,
    deadline: _Deadline,
) -> Path:
    """Validate the named model at the final in-operation linearization point.

    The final lexical-path identity check is the success linearization point. A
    same-user mutation after this function returns is outside the guarantee of a
    Path-returning API; every mutation before that check is rejected.
    """
    _validate_directory_chain(canonical_root, directory_fds)
    _validate_lock_file(directory_fds[-1], lock_fd)
    _final_validation_hook("after-directory-check")
    model_dir_fd = directory_fds[-1]
    try:
        destination_fd = os.open(
            _DESTINATION_NAME,
            os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
            dir_fd=model_dir_fd,
        )
    except OSError:
        raise AcquisitionError("could not inspect the semantic model destination") from None
    try:
        held = os.fstat(destination_fd)
        _require_private_regular(held, "semantic model destination")
        if held.st_size > _MAX_MODEL_BYTES:
            raise AcquisitionError("semantic model destination exceeds the byte limit")
        _final_validation_hook("after-destination-open")
        try:
            digest = _sha256_fd(destination_fd, deadline)
        except OSError:
            raise AcquisitionError("could not verify the semantic model destination") from None
        if digest != expected_sha256:
            raise AcquisitionError("semantic model destination checksum mismatch")
        _final_validation_hook("after-destination-hash")
        current = os.fstat(destination_fd)
        if not _same_file_version(held, current):
            raise AcquisitionError("semantic model destination identity changed")
        try:
            named = os.stat(_DESTINATION_NAME, dir_fd=model_dir_fd, follow_symlinks=False)
        except OSError:
            raise AcquisitionError("semantic model destination identity changed") from None
        _require_private_regular(named, "semantic model destination")
        if not _same_file_version(current, named):
            raise AcquisitionError("semantic model destination identity changed")
        _final_validation_hook("after-named-destination-check")
        deadline.check()
        _validate_directory_chain(canonical_root, directory_fds)
        _validate_lock_file(model_dir_fd, lock_fd)
        destination = _validate_lexical_destination(
            canonical_root,
            directory_fds,
            current,
        )
        return destination
    finally:
        os.close(destination_fd)


def _validate_runtime_parent(
    canonical_root: Path,
    directory_fds: tuple[int, ...],
) -> None:
    try:
        lexical_root = os.stat(canonical_root, follow_symlinks=False)
        held_root = os.fstat(directory_fds[0])
    except OSError:
        raise AcquisitionError("ONNX Runtime artifact directory identity changed") from None
    if (lexical_root.st_dev, lexical_root.st_ino) != (held_root.st_dev, held_root.st_ino):
        raise AcquisitionError("ONNX Runtime artifact directory identity changed")
    for parent_fd, expected_fd, component in zip(
        directory_fds[:-1],
        directory_fds[1:],
        _RUNTIME_PARENT_COMPONENTS,
        strict=True,
    ):
        try:
            current_fd = os.open(component, _DIRECTORY_FLAGS, dir_fd=parent_fd)
        except OSError:
            raise AcquisitionError("ONNX Runtime artifact directory identity changed") from None
        try:
            current = os.fstat(current_fd)
            expected = os.fstat(expected_fd)
            _require_trusted_directory(current)
            _require_trusted_directory(expected)
            if (current.st_dev, current.st_ino) != (expected.st_dev, expected.st_ino):
                raise AcquisitionError("ONNX Runtime artifact directory identity changed")
        finally:
            os.close(current_fd)


def _remove_runtime_entry(path: Path) -> None:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return
    except OSError:
        raise AcquisitionError("could not inspect ONNX Runtime temporary state") from None
    try:
        if stat.S_ISDIR(metadata.st_mode):
            shutil.rmtree(path)
        else:
            path.unlink()
    except OSError:
        raise AcquisitionError("could not remove ONNX Runtime temporary state") from None


def _stream_runtime_archive(
    initial_url: str,
    parent_fd: int,
    expected_sha256: str,
    deadline: _Deadline,
) -> None:
    current_url = initial_url
    redirects = 0
    while True:
        deadline.check()
        try:
            response = _open_no_redirect(_request(current_url, deadline))
        except Exception as error:
            if isinstance(error, AcquisitionError):
                raise
            raise AcquisitionError("ONNX Runtime request failed") from None
        archive_fd: int | None = None
        try:
            status = _response_status(response)
            if status in _REDIRECT_STATUSES:
                if redirects >= _MAX_REDIRECTS:
                    raise AcquisitionError("ONNX Runtime redirect limit exceeded")
                current_url = _safe_runtime_url(
                    urllib.parse.urljoin(current_url, _redirect_location(response))
                )
                redirects += 1
                continue
            if status != 200:
                raise AcquisitionError("ONNX Runtime request returned an unexpected status")
            declared_length = _content_length(response)
            if declared_length is not None and declared_length > _MAX_ONNX_RUNTIME_ARCHIVE_BYTES:
                raise AcquisitionError("ONNX Runtime archive exceeds the byte limit")
            try:
                archive_fd = os.open(
                    _RUNTIME_ARCHIVE_PART_NAME,
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                    0o600,
                    dir_fd=parent_fd,
                )
            except OSError:
                raise AcquisitionError("could not create ONNX Runtime archive") from None
            digest = hashlib.sha256()
            total = 0
            with os.fdopen(os.dup(archive_fd), "wb") as destination:
                while True:
                    _set_response_read_timeout(response, deadline.remaining())
                    try:
                        chunk = response.read(_CHUNK_SIZE)
                    except Exception:
                        raise AcquisitionError("ONNX Runtime response read failed") from None
                    deadline.check()
                    if not chunk:
                        break
                    if not isinstance(chunk, bytes):
                        raise AcquisitionError("ONNX Runtime response was not binary")
                    total += len(chunk)
                    if total > _MAX_ONNX_RUNTIME_ARCHIVE_BYTES:
                        raise AcquisitionError("ONNX Runtime archive exceeds the byte limit")
                    destination.write(chunk)
                    digest.update(chunk)
                destination.flush()
            os.fsync(archive_fd)
            if declared_length is not None and declared_length != total:
                raise AcquisitionError("ONNX Runtime response length did not match its declaration")
            if digest.hexdigest() != expected_sha256:
                raise AcquisitionError("ONNX Runtime archive checksum mismatch")
            os.close(archive_fd)
            archive_fd = None
            os.fsync(parent_fd)
            return
        except Exception:
            if archive_fd is not None:
                os.close(archive_fd)
            try:
                os.unlink(_RUNTIME_ARCHIVE_PART_NAME, dir_fd=parent_fd)
                os.fsync(parent_fd)
            except FileNotFoundError:
                pass
            except OSError:
                pass
            raise
        finally:
            try:
                response.close()
            except Exception:
                try:
                    os.unlink(_RUNTIME_ARCHIVE_PART_NAME, dir_fd=parent_fd)
                except OSError:
                    pass
                raise AcquisitionError("could not close ONNX Runtime response") from None


def _stripped_archive_path(name: str) -> PurePosixPath:
    path = PurePosixPath(name)
    if path.is_absolute() or not path.parts or path.parts[0] != _RUNTIME_TOP_DIRECTORY:
        raise AcquisitionError("ONNX Runtime archive has an unexpected top directory")
    if any(part in ("", ".", "..") for part in path.parts):
        raise AcquisitionError("ONNX Runtime archive member escapes its top directory")
    return PurePosixPath(*path.parts[1:])


def _relative_link_target(link_path: PurePosixPath, target: str) -> PurePosixPath:
    raw = PurePosixPath(target)
    if raw.is_absolute() or not target:
        raise AcquisitionError("ONNX Runtime archive contains an absolute link")
    parts = list(link_path.parent.parts)
    for part in raw.parts:
        if part in ("", "."):
            continue
        if part == "..":
            if not parts:
                raise AcquisitionError("ONNX Runtime archive link escapes extraction root")
            parts.pop()
            continue
        parts.append(part)
    if not parts:
        raise AcquisitionError("ONNX Runtime archive link has no target")
    return PurePosixPath(*parts)


def _resolve_archive_link(
    link_path: PurePosixPath,
    links: dict[PurePosixPath, str],
    kinds: dict[PurePosixPath, str],
) -> PurePosixPath:
    pending = list(_relative_link_target(link_path, links[link_path]).parts)
    resolved: list[str] = []
    visited: set[PurePosixPath] = {link_path}
    while pending:
        candidate = PurePosixPath(*resolved, pending.pop(0))
        if candidate in links:
            if candidate in visited:
                raise AcquisitionError("ONNX Runtime archive contains a link cycle")
            visited.add(candidate)
            expanded = _relative_link_target(candidate, links[candidate])
            pending = [*expanded.parts, *pending]
            resolved = []
            continue
        kind = kinds.get(candidate)
        if kind is None or (pending and kind != "directory"):
            raise AcquisitionError("ONNX Runtime archive contains a dangling link")
        resolved.append(candidate.name)
    target = PurePosixPath(*resolved)
    if target not in kinds:
        raise AcquisitionError("ONNX Runtime archive contains a dangling link")
    return target


def _archive_members(
    bundle: tarfile.TarFile,
) -> tuple[
    dict[PurePosixPath, tarfile.TarInfo],
    dict[PurePosixPath, str],
    dict[PurePosixPath, str],
]:
    members: dict[PurePosixPath, tarfile.TarInfo] = {}
    kinds: dict[PurePosixPath, str] = {}
    links: dict[PurePosixPath, str] = {}
    total_size = 0
    for member in bundle.getmembers():
        path = _stripped_archive_path(member.name)
        if not path.parts:
            if not member.isdir():
                raise AcquisitionError("ONNX Runtime archive top entry is not a directory")
            continue
        if path in members:
            raise AcquisitionError("ONNX Runtime archive contains duplicate members")
        if member.islnk():
            raise AcquisitionError("ONNX Runtime archive contains a hard link")
        if member.isdir():
            kind = "directory"
        elif member.isreg():
            kind = "file"
            total_size += member.size
            if total_size > _MAX_ONNX_RUNTIME_EXTRACTED_BYTES:
                raise AcquisitionError("ONNX Runtime extraction exceeds the byte limit")
        elif member.issym():
            kind = "symlink"
            links[path] = member.linkname
        else:
            raise AcquisitionError("ONNX Runtime archive contains a special entry")
        members[path] = member
        kinds[path] = kind

    for path in tuple(kinds):
        for parent in path.parents:
            if not parent.parts:
                continue
            parent_kind = kinds.setdefault(parent, "directory")
            if parent_kind != "directory":
                raise AcquisitionError("ONNX Runtime archive member has a non-directory parent")
    for link_path in links:
        _resolve_archive_link(link_path, links, kinds)
    return members, kinds, links


def _extract_runtime_archive(archive_path: Path, extraction_root: Path) -> None:
    try:
        with tarfile.open(archive_path, mode="r:gz") as bundle:
            members, kinds, links = _archive_members(bundle)
            for path, kind in sorted(kinds.items(), key=lambda item: len(item[0].parts)):
                destination = extraction_root.joinpath(*path.parts)
                if kind == "directory":
                    destination.mkdir(mode=0o700, parents=True, exist_ok=True)
            for path, member in members.items():
                if kinds[path] != "file":
                    continue
                source = bundle.extractfile(member)
                if source is None:
                    raise AcquisitionError("could not read ONNX Runtime archive member")
                destination = extraction_root.joinpath(*path.parts)
                with source, destination.open("xb") as output:
                    shutil.copyfileobj(source, output, _CHUNK_SIZE)
                destination.chmod(0o700 if member.mode & 0o111 else 0o600)
            for path, target in links.items():
                extraction_root.joinpath(*path.parts).symlink_to(target)
    except AcquisitionError:
        raise
    except (OSError, tarfile.TarError, EOFError):
        raise AcquisitionError("invalid ONNX Runtime archive") from None


def _validate_runtime_directory(destination: Path, library: Path) -> Path:
    try:
        metadata = destination.lstat()
        if not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
            raise AcquisitionError("ONNX Runtime destination is not a directory")
        resolved_root = destination.resolve(strict=True)
        resolved_library = (destination / library).resolve(strict=True)
        resolved_library.relative_to(resolved_root)
        if not resolved_library.is_file():
            raise AcquisitionError("ONNX Runtime library is not a regular file")
    except AcquisitionError:
        raise
    except (FileNotFoundError, OSError, RuntimeError, ValueError):
        raise AcquisitionError("ONNX Runtime library chain is invalid") from None
    return destination


def _validated_runtime_destination(root: Path, library: Path) -> Path:
    return _validate_runtime_directory(root / _RUNTIME_TOP_DIRECTORY, library)


def acquire_onnx_runtime(repo_root: Path) -> Path:
    """Acquire, validate, and atomically publish the approved native runtime."""
    deadline = _Deadline.after(_ACQUISITION_TIMEOUT_SECONDS)
    token = _CURRENT_DEADLINE.set(deadline)
    try:
        with _open_runtime_parent(repo_root) as (canonical_root, directory_fds):
            parent_fd = directory_fds[-1]
            parent = canonical_root.joinpath(*_RUNTIME_PARENT_COMPONENTS)
            initial_url, expected_sha256, library = _load_runtime_spec(canonical_root)
            destination = parent / _RUNTIME_TOP_DIRECTORY
            if destination.exists() or destination.is_symlink():
                return _validated_runtime_destination(parent, library)

            archive_path = parent / _RUNTIME_ARCHIVE_PART_NAME
            extraction_root = parent / _RUNTIME_DIRECTORY_PART_NAME
            _remove_runtime_entry(archive_path)
            _remove_runtime_entry(extraction_root)
            try:
                _stream_runtime_archive(
                    initial_url,
                    parent_fd,
                    expected_sha256,
                    deadline,
                )
                _validate_runtime_parent(canonical_root, directory_fds)
                extraction_root.mkdir(mode=0o700)
                _extract_runtime_archive(archive_path, extraction_root)
                validated = _validate_runtime_directory(extraction_root, library)
                if validated != extraction_root:
                    raise AcquisitionError("ONNX Runtime extraction root is invalid")
                _validate_runtime_parent(canonical_root, directory_fds)
                os.rename(
                    _RUNTIME_DIRECTORY_PART_NAME,
                    _RUNTIME_TOP_DIRECTORY,
                    src_dir_fd=parent_fd,
                    dst_dir_fd=parent_fd,
                )
                os.fsync(parent_fd)
            finally:
                _remove_runtime_entry(archive_path)
                _remove_runtime_entry(extraction_root)
            return _validated_runtime_destination(parent, library)
    except AcquisitionError:
        raise
    except Exception:
        raise AcquisitionError("ONNX Runtime acquisition failed") from None
    finally:
        _CURRENT_DEADLINE.reset(token)


def acquire_semantic_model(repo_root: Path) -> Path:
    """Acquire and atomically promote the exact approved semantic model.

    The returned path was verified at the final lexical identity check while the
    unlink-proof mutex was held. Mutations after return are the caller's boundary.
    """
    deadline = _Deadline.after(_ACQUISITION_TIMEOUT_SECONDS)
    token = _CURRENT_DEADLINE.set(deadline)
    try:
        with _open_model_directory(repo_root) as (canonical_root, directory_fds):
            model_dir_fd = directory_fds[-1]
            initial_url, expected_sha256 = _load_model_spec(canonical_root)
            with _kernel_mutex(canonical_root, deadline):
                with _acquisition_lock(model_dir_fd) as lock_fd:
                    _remove_stale_partial(model_dir_fd)
                    if not _existing_destination_is_verified(
                        model_dir_fd,
                        expected_sha256,
                        deadline,
                    ):
                        partial_fd = _stream_model(
                            initial_url,
                            model_dir_fd,
                            expected_sha256,
                            deadline,
                        )
                        try:
                            _validate_directory_chain(canonical_root, directory_fds)
                            _promote_open_partial(model_dir_fd, partial_fd)
                        except Exception:
                            _remove_owned_partial(model_dir_fd, partial_fd)
                            os.close(partial_fd)
                            raise
                        os.close(partial_fd)
                    return _verified_return_path(
                        canonical_root,
                        directory_fds,
                        lock_fd,
                        expected_sha256,
                        deadline,
                    )
    except AcquisitionError:
        raise
    except Exception:
        raise AcquisitionError("semantic model acquisition failed") from None
    finally:
        _CURRENT_DEADLINE.reset(token)


def main(argv: Sequence[str] | None = None, *, repo_root: Path | None = None) -> int:
    """Run an explicit qualification acquisition command."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", choices=("semantic-model", "onnx-runtime"))
    arguments = parser.parse_args(argv)
    root = Path(__file__).resolve().parents[2] if repo_root is None else repo_root
    try:
        if arguments.artifact == "semantic-model":
            destination = acquire_semantic_model(root)
        else:
            destination = acquire_onnx_runtime(root)
    except AcquisitionError as error:
        print(f"Acquisition failed: {error}", file=sys.stderr)
        return 1
    print(destination.relative_to(root.resolve(strict=True)).as_posix())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
