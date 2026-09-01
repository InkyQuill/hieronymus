"""Explicit checksum-verified acquisition of qualification-only artifacts."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import stat
import sys
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Iterator, Sequence
from contextlib import closing, contextmanager
from pathlib import Path
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
_REDIRECT_STATUSES: Final = frozenset({301, 302, 303, 307, 308})
_MAX_REDIRECTS: Final = 5
_CHUNK_SIZE: Final = 1024 * 1024
_MAX_MODEL_BYTES: Final = 256 * 1024 * 1024
_TRANSPORT_TIMEOUT_SECONDS: Final = 120
_MODEL_DESTINATION: Final = Path("qualification/.artifacts/models/all-MiniLM-L6-v2/model.onnx")
_MODEL_DIRECTORY_COMPONENTS: Final = ("qualification", ".artifacts", "models", "all-MiniLM-L6-v2")
_DESTINATION_NAME: Final = "model.onnx"
_PARTIAL_NAME: Final = "model.onnx.part"
# This inode is deliberately persistent: unlinking it would permit split flock domains.
_LOCK_NAME: Final = "model.onnx.lock"
_DIRECTORY_FLAGS: Final = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC


class AcquisitionError(RuntimeError):
    """A non-sensitive deterministic acquisition failure."""


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
        return opener.open(request, timeout=_TRANSPORT_TIMEOUT_SECONDS)
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


def _remove_stale_partial(model_dir_fd: int) -> None:
    try:
        metadata = os.stat(_PARTIAL_NAME, dir_fd=model_dir_fd, follow_symlinks=False)
    except FileNotFoundError:
        return
    except OSError:
        raise AcquisitionError("could not inspect the semantic model partial file") from None
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise AcquisitionError("semantic model partial must be a regular file")
    try:
        os.unlink(_PARTIAL_NAME, dir_fd=model_dir_fd)
    except OSError:
        raise AcquisitionError("could not remove the semantic model partial file") from None


def _sha256_fd(fd: int) -> str:
    digest = hashlib.sha256()
    with os.fdopen(os.dup(fd), "rb") as source:
        for chunk in iter(lambda: source.read(_CHUNK_SIZE), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _existing_destination_is_verified(model_dir_fd: int, expected_sha256: str) -> bool:
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
        if not stat.S_ISREG(metadata.st_mode):
            raise AcquisitionError("semantic model destination must be a regular file")
        try:
            return _sha256_fd(fd) == expected_sha256
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
            try:
                os.mkdir(component, mode=0o700, dir_fd=current_fd)
            except FileExistsError:
                pass
            try:
                child_fd = os.open(component, _DIRECTORY_FLAGS, dir_fd=current_fd)
            except OSError:
                raise AcquisitionError("artifact directory is not a trusted directory") from None
            opened.append(child_fd)
            current_fd = child_fd
        yield canonical_root, tuple(opened)
    except AcquisitionError:
        raise
    except OSError:
        raise AcquisitionError("could not prepare the semantic model directory") from None
    finally:
        for fd in reversed(opened):
            os.close(fd)


def _validate_directory_chain(directory_fds: tuple[int, ...]) -> None:
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
            if (current.st_dev, current.st_ino) != (expected.st_dev, expected.st_ino):
                raise AcquisitionError("artifact directory identity changed")
        finally:
            os.close(current_fd)


@contextmanager
def _acquisition_lock(model_dir_fd: int) -> Iterator[None]:
    try:
        lock_fd = os.open(
            _LOCK_NAME,
            os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC,
            0o600,
            dir_fd=model_dir_fd,
        )
    except OSError:
        raise AcquisitionError("could not open the semantic model acquisition lock") from None
    try:
        metadata = os.fstat(lock_fd)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
            raise AcquisitionError("semantic model acquisition lock must be a private regular file")
        try:
            fcntl.flock(lock_fd, fcntl.LOCK_EX)
        except OSError:
            raise AcquisitionError("could not lock semantic model acquisition") from None
        yield
    finally:
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


def _request(url: str) -> urllib.request.Request:
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


def _stream_model(
    initial_url: str,
    model_dir_fd: int,
    expected_sha256: str,
) -> int:
    current_url = initial_url
    redirects = 0
    while True:
        try:
            response = _open_no_redirect(_request(current_url))
        except Exception as error:
            if isinstance(error, AcquisitionError):
                raise
            raise AcquisitionError("semantic model request failed") from None
        with closing(response):
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
            try:
                destination = os.fdopen(os.dup(partial_fd), "wb")
            except OSError:
                os.close(partial_fd)
                raise AcquisitionError("could not open the semantic model partial file") from None
            try:
                with destination:
                    while True:
                        chunk = response.read(_CHUNK_SIZE)
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
            except Exception:
                _remove_owned_partial(model_dir_fd, partial_fd)
                os.close(partial_fd)
                raise
            return partial_fd


def _promote_open_partial(model_dir_fd: int, partial_fd: int) -> None:
    written = os.fstat(partial_fd)
    if not stat.S_ISREG(written.st_mode) or written.st_nlink != 1:
        raise AcquisitionError("semantic model partial identity is invalid")
    try:
        named = os.stat(_PARTIAL_NAME, dir_fd=model_dir_fd, follow_symlinks=False)
    except OSError:
        raise AcquisitionError("semantic model partial identity changed before promotion") from None
    if (
        not stat.S_ISREG(named.st_mode)
        or named.st_dev != written.st_dev
        or named.st_ino != written.st_ino
    ):
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
        except OSError:
            return


def acquire_semantic_model(repo_root: Path) -> Path:
    """Acquire and atomically promote the exact approved semantic model."""
    try:
        with _open_model_directory(repo_root) as (canonical_root, directory_fds):
            model_dir_fd = directory_fds[-1]
            initial_url, expected_sha256 = _load_model_spec(canonical_root)
            with _acquisition_lock(model_dir_fd):
                _remove_stale_partial(model_dir_fd)
                if _existing_destination_is_verified(model_dir_fd, expected_sha256):
                    _validate_directory_chain(directory_fds)
                    return canonical_root / _MODEL_DESTINATION
                partial_fd = _stream_model(initial_url, model_dir_fd, expected_sha256)
                try:
                    _validate_directory_chain(directory_fds)
                    _promote_open_partial(model_dir_fd, partial_fd)
                except Exception:
                    _remove_owned_partial(model_dir_fd, partial_fd)
                    os.close(partial_fd)
                    raise
                os.close(partial_fd)
                return canonical_root / _MODEL_DESTINATION
    except AcquisitionError:
        raise
    except Exception:
        raise AcquisitionError("semantic model acquisition failed") from None


def main(argv: Sequence[str] | None = None, *, repo_root: Path | None = None) -> int:
    """Run an explicit qualification acquisition command."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", choices=("semantic-model",))
    parser.parse_args(argv)
    root = Path(__file__).resolve().parents[2] if repo_root is None else repo_root
    try:
        destination = acquire_semantic_model(root)
    except AcquisitionError as error:
        print(f"Acquisition failed: {error}", file=sys.stderr)
        return 1
    print(destination.relative_to(root.resolve(strict=True)).as_posix())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
