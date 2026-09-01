"""Explicit checksum-verified acquisition of qualification-only artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
import sys
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Sequence
from contextlib import closing
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
_MODEL_DESTINATION: Final = Path("qualification/.artifacts/models/all-MiniLM-L6-v2/model.onnx")


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
        return opener.open(request, timeout=60)
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


def _remove_partial(partial: Path) -> None:
    try:
        partial.unlink(missing_ok=True)
    except OSError:
        raise AcquisitionError("could not remove the semantic model partial file") from None


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(_CHUNK_SIZE), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _existing_destination_is_verified(destination: Path, expected_sha256: str) -> bool:
    try:
        metadata = destination.lstat()
    except FileNotFoundError:
        return False
    except OSError:
        raise AcquisitionError("could not inspect the semantic model destination") from None
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise AcquisitionError("semantic model destination must be a regular file")
    try:
        return _sha256_file(destination) == expected_sha256
    except OSError:
        raise AcquisitionError("could not verify the existing semantic model") from None


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


def _stream_model(initial_url: str, partial: Path, expected_sha256: str) -> None:
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

            digest = hashlib.sha256()
            with partial.open("xb") as destination:
                while True:
                    chunk = response.read(_CHUNK_SIZE)
                    if not chunk:
                        break
                    if not isinstance(chunk, bytes):
                        raise AcquisitionError("semantic model response was not binary")
                    destination.write(chunk)
                    digest.update(chunk)
            if digest.hexdigest() != expected_sha256:
                raise AcquisitionError("semantic model checksum mismatch")
            return


def acquire_semantic_model(repo_root: Path) -> Path:
    """Acquire and atomically promote the exact approved semantic model."""
    destination = repo_root / _MODEL_DESTINATION
    partial = destination.with_name("model.onnx.part")
    try:
        _remove_partial(partial)
        initial_url, expected_sha256 = _load_model_spec(repo_root)
        if _existing_destination_is_verified(destination, expected_sha256):
            return destination
        destination.parent.mkdir(parents=True, exist_ok=True)
        _stream_model(initial_url, partial, expected_sha256)
        os.replace(partial, destination)
        return destination
    except AcquisitionError:
        _remove_partial(partial)
        raise
    except Exception:
        try:
            _remove_partial(partial)
        except AcquisitionError:
            pass
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
    print(destination.relative_to(root).as_posix())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
