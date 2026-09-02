"""Deterministic tests for checksum-verified qualification acquisitions."""

from __future__ import annotations

import hashlib
import http.client
import io
import json
import multiprocessing
import os
import stat
import sys
import tarfile
import threading
import time
from collections.abc import Callable, Mapping
from pathlib import Path

import pytest

_ROOT = Path(__file__).resolve().parents[2]
if str(_ROOT) not in sys.path:
    sys.path.insert(0, str(_ROOT))

from tools.qualification import acquire  # noqa: E402, I001


_INITIAL_URL = (
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/"
    "9a53d751e60e6dd34f2443711d44d5b09389f89a/onnx/model.onnx"
)
_DESTINATION = Path("qualification/.artifacts/models/all-MiniLM-L6-v2/model.onnx")
_ONNX_RUNTIME_URL = (
    "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/"
    "onnxruntime-linux-x64-1.28.0.tgz"
)
_ONNX_RUNTIME_DESTINATION = Path("qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0")
_ONNX_RUNTIME_TOP = "onnxruntime-linux-x64-1.28.0"


class _Response:
    def __init__(
        self,
        status: int,
        *,
        headers: Mapping[str, str] | None = None,
        body: bytes = b"",
        read_error: OSError | None = None,
        before_read: Callable[[], None] | None = None,
        close_error: OSError | None = None,
    ) -> None:
        self.status = status
        self.headers = dict(headers or {})
        self._body = body
        self._offset = 0
        self._read_error = read_error
        self._before_read = before_read
        self._close_error = close_error
        self.read_timeouts: list[float] = []

    def settimeout(self, timeout: float) -> None:
        self.read_timeouts.append(timeout)

    def read(self, size: int = -1) -> bytes:
        if self._before_read is not None:
            self._before_read()
            self._before_read = None
        if self._read_error is not None:
            error = self._read_error
            self._read_error = None
            raise error
        if size < 0:
            size = len(self._body) - self._offset
        chunk = self._body[self._offset : self._offset + size]
        self._offset += len(chunk)
        return chunk

    def close(self) -> None:
        if self._close_error is not None:
            raise self._close_error
        return None


def _repo(tmp_path: Path, *, body: bytes, url: str = _INITIAL_URL) -> Path:
    prerequisites = {
        "semantic_model": {
            "url": url,
            "sha256": hashlib.sha256(body).hexdigest(),
        }
    }
    path = tmp_path / "qualification/prerequisites.json"
    path.parent.mkdir(parents=True)
    path.write_text(json.dumps(prerequisites), encoding="utf-8")
    return tmp_path


def _request_headers(request: object) -> dict[str, str]:
    return {
        key.lower(): value
        for key, value in request.header_items()  # type: ignore[attr-defined]
    }


def _write_private(path: Path, body: bytes) -> None:
    path.write_bytes(body)
    path.chmod(0o600)


def _runtime_archive(
    *,
    invalid: str | None = None,
    library_body: bytes = b"verified runtime library",
) -> bytes:
    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode="w:gz") as bundle:
        for directory in (_ONNX_RUNTIME_TOP, f"{_ONNX_RUNTIME_TOP}/lib"):
            entry = tarfile.TarInfo(directory)
            entry.type = tarfile.DIRTYPE
            entry.mode = 0o755
            bundle.addfile(entry)

        library_name = f"{_ONNX_RUNTIME_TOP}/lib/libonnxruntime.so.1.28.0"
        library = tarfile.TarInfo(library_name)
        library.size = len(library_body)
        library.mode = 0o755
        bundle.addfile(library, io.BytesIO(library_body))

        if invalid == "traversal":
            traversal = tarfile.TarInfo(f"{_ONNX_RUNTIME_TOP}/../escaped")
            traversal.size = 7
            bundle.addfile(traversal, io.BytesIO(b"escaped"))
        elif invalid == "device":
            device = tarfile.TarInfo(f"{_ONNX_RUNTIME_TOP}/lib/device")
            device.type = tarfile.CHRTYPE
            bundle.addfile(device)
        elif invalid == "hard-link":
            hard_link = tarfile.TarInfo(f"{_ONNX_RUNTIME_TOP}/lib/hard-link")
            hard_link.type = tarfile.LNKTYPE
            hard_link.linkname = library_name
            bundle.addfile(hard_link)
        elif invalid == "wrong-top-directory":
            wrong_top = tarfile.TarInfo("unexpected-top/file")
            wrong_top.size = 5
            bundle.addfile(wrong_top, io.BytesIO(b"wrong"))

        first_link = tarfile.TarInfo(f"{_ONNX_RUNTIME_TOP}/lib/libonnxruntime.so.1")
        first_link.type = tarfile.SYMTYPE
        first_link.linkname = (
            "/etc/passwd"
            if invalid == "absolute-link"
            else "libonnxruntime.so"
            if invalid == "cycle"
            else "../../../outside"
            if invalid == "escaping-link"
            else "missing-library"
            if invalid == "dangling-link"
            else "libonnxruntime.so.1.28.0"
        )
        bundle.addfile(first_link)

        required_link = tarfile.TarInfo(f"{_ONNX_RUNTIME_TOP}/lib/libonnxruntime.so")
        required_link.type = tarfile.SYMTYPE
        required_link.linkname = "libonnxruntime.so.1"
        bundle.addfile(required_link)
    return archive.getvalue()


def _runtime_repo(
    tmp_path: Path,
    *,
    archive: bytes,
    expected_archive: bytes | None = None,
) -> Path:
    prerequisites = {
        "onnx_runtime": {
            "version": "1.28.0",
            "target": "linux-x64",
            "url": _ONNX_RUNTIME_URL,
            "sha256": hashlib.sha256(
                archive if expected_archive is None else expected_archive
            ).hexdigest(),
            "library": "lib/libonnxruntime.so",
        }
    }
    path = tmp_path / "qualification/prerequisites.json"
    path.parent.mkdir(parents=True)
    path.write_text(json.dumps(prerequisites), encoding="utf-8")
    return tmp_path


def test_prerequisites_and_toolchain_are_exact() -> None:
    assert (_ROOT / "qualification/rust-toolchain.toml").read_bytes() == (
        b'[toolchain]\nchannel = "1.96.0"\nprofile = "minimal"\n'
        b'targets = ["x86_64-unknown-linux-gnu"]\n'
        b'components = ["clippy", "rustfmt"]\n'
    )
    assert json.loads((_ROOT / "qualification/prerequisites.json").read_bytes()) == {
        "schema_version": 1,
        "target": "x86_64-unknown-linux-gnu",
        "rust": {
            "version": "1.96.0",
            "commit": "ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96",
        },
        "bun": {"version": "1.3.14", "authority": "frontend/package.json"},
        "semantic_model": {
            "provider": "onnx-runtime",
            "repository": "sentence-transformers/all-MiniLM-L6-v2",
            "revision": "9a53d751e60e6dd34f2443711d44d5b09389f89a",
            "file": "onnx/model.onnx",
            "url": _INITIAL_URL,
            "sha256": "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452",
            "dimensions": 384,
            "normalization": "l2",
        },
        "onnx_runtime": {
            "version": "1.28.0",
            "target": "linux-x64",
            "url": _ONNX_RUNTIME_URL,
            "sha256": "a3e1b79d7bb1bf09696ce675f49e4064e6c81f6202b8225624fff0e93f8d6407",
            "library": "lib/libonnxruntime.so",
        },
        "network_policy": {
            "allowed_only_for": [
                "manual review of https://modelcontextprotocol.io/specification/2026-07-28",
                "cargo fetch --locked",
                "bun install --frozen-lockfile",
                "python -m tools.qualification.acquire semantic-model",
                "python -m tools.qualification.acquire onnx-runtime",
            ],
            "ordinary_replay": "offline",
        },
    }


def test_acquisition_follows_allowed_https_redirects_without_forwarding_credentials(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"verified model bytes" * 100_000
    repo_root = _repo(tmp_path, body=body)
    redirect_urls = (
        "https://cdn-lfs.huggingface.co/model.onnx?X-Amz-Signature=top-secret",
        "https://cas-bridge.xethub.hf.co/model.onnx?X-Amz-Signature=top-secret",
        "https://us.aws.cdn.hf.co/model.onnx?X-Amz-Signature=top-secret",
    )
    requests: list[object] = []
    replace_calls: list[tuple[str, str]] = []
    real_replace = os.replace

    def atomic_replace(
        source: str,
        destination: str,
        *,
        src_dir_fd: int,
        dst_dir_fd: int,
    ) -> None:
        replace_calls.append((source, destination))
        real_replace(
            source,
            destination,
            src_dir_fd=src_dir_fd,
            dst_dir_fd=dst_dir_fd,
        )

    def fake_open(request: object) -> _Response:
        requests.append(request)
        assert not (
            {"authorization", "proxy-authorization", "cookie"} & _request_headers(request).keys()
        )
        if request.full_url == _INITIAL_URL:  # type: ignore[attr-defined]
            return _Response(
                302,
                headers={
                    "Location": redirect_urls[0],
                    "Set-Cookie": "session=do-not-forward",
                },
            )
        if request.full_url == redirect_urls[0]:  # type: ignore[attr-defined]
            return _Response(303, headers={"Location": redirect_urls[1]})
        if request.full_url == redirect_urls[1]:  # type: ignore[attr-defined]
            return _Response(308, headers={"Location": redirect_urls[2]})
        assert request.full_url == redirect_urls[2]  # type: ignore[attr-defined]
        return _Response(
            200,
            body=body,
            before_read=lambda: assert_destination_absent(repo_root),
        )

    monkeypatch.setattr(acquire, "_open_no_redirect", fake_open)
    monkeypatch.setattr(acquire.os, "replace", atomic_replace)

    destination = acquire.acquire_semantic_model(repo_root)

    assert destination == repo_root / _DESTINATION
    assert destination.read_bytes() == body
    assert not destination.with_name("model.onnx.part").exists()
    assert [request.full_url for request in requests] == [  # type: ignore[attr-defined]
        _INITIAL_URL,
        *redirect_urls,
    ]
    assert replace_calls == [("model.onnx.part", "model.onnx")]


def assert_destination_absent(repo_root: Path) -> None:
    assert not (repo_root / _DESTINATION).exists()


@pytest.mark.parametrize(
    "redirect",
    [
        "http://cdn-lfs.huggingface.co/model.onnx",
        "https://user:password@cdn-lfs.huggingface.co/model.onnx",
        "https://cdn-lfs.huggingface.co/model.onnx#secret-fragment",
        "https://evil.example/model.onnx?token=redirect-secret",
        "https://huggingface.co.evil.example/model.onnx",
        "https://us.aws.cdn.hf.co.attacker.invalid/model.onnx?token=redirect-secret",
        "https://huggingface.co:8443/model.onnx",
    ],
)
def test_acquisition_rejects_unsafe_redirects_without_echoing_or_leaving_partial(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    redirect: str,
) -> None:
    repo_root = _repo(tmp_path, body=b"model")
    part = repo_root / _DESTINATION.with_name("model.onnx.part")
    part.parent.mkdir(parents=True)
    _write_private(part, b"stale partial")
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(302, headers={"Location": redirect}),
    )

    with pytest.raises(acquire.AcquisitionError) as caught:
        acquire.acquire_semantic_model(repo_root)

    assert not part.exists()
    assert "redirect-secret" not in str(caught.value)
    assert "secret-fragment" not in str(caught.value)
    assert "password" not in str(caught.value)


def test_acquisition_rejects_more_than_five_redirect_hops(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = _repo(tmp_path, body=b"model")
    opened: list[str] = []

    def fake_open(request: object) -> _Response:
        opened.append(request.full_url)  # type: ignore[attr-defined]
        return _Response(
            307,
            headers={"Location": f"https://cdn-lfs.huggingface.co/hop-{len(opened)}"},
        )

    monkeypatch.setattr(acquire, "_open_no_redirect", fake_open)

    with pytest.raises(acquire.AcquisitionError, match="redirect limit"):
        acquire.acquire_semantic_model(repo_root)

    assert len(opened) == 6
    assert not (repo_root / _DESTINATION.with_name("model.onnx.part")).exists()


def test_acquisition_rejects_checksum_mismatch_without_promoting_partial(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = _repo(tmp_path, body=b"expected bytes")
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=b"different bytes"),
    )
    replace_calls: list[tuple[object, object]] = []
    monkeypatch.setattr(
        acquire.os,
        "replace",
        lambda source, target: replace_calls.append((source, target)),
    )

    with pytest.raises(acquire.AcquisitionError, match="checksum"):
        acquire.acquire_semantic_model(repo_root)

    assert replace_calls == []
    assert not (repo_root / _DESTINATION).exists()
    assert not (repo_root / _DESTINATION.with_name("model.onnx.part")).exists()


@pytest.mark.parametrize("failure", ["http", "read", "replace"])
def test_acquisition_removes_partial_after_transport_or_filesystem_failure(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    failure: str,
) -> None:
    body = b"model bytes"
    repo_root = _repo(tmp_path, body=body)
    if failure == "http":
        response = _Response(503)
    elif failure == "read":
        response = _Response(200, read_error=OSError("signed-query-secret"))
    else:
        response = _Response(200, body=body)
        monkeypatch.setattr(
            acquire.os,
            "replace",
            lambda _source, _target: (_ for _ in ()).throw(OSError("replace failed")),
        )
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: response)

    with pytest.raises(acquire.AcquisitionError) as caught:
        acquire.acquire_semantic_model(repo_root)

    assert "signed-query-secret" not in str(caught.value)
    assert caught.value.__cause__ is None
    assert not (repo_root / _DESTINATION.with_name("model.onnx.part")).exists()
    assert not (repo_root / _DESTINATION).exists()


def test_acquisition_removes_partial_when_partial_stream_cannot_be_opened(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"model bytes"
    repo_root = _repo(tmp_path, body=body)
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))
    monkeypatch.setattr(
        acquire.os,
        "fdopen",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(OSError("fdopen failed")),
    )

    with pytest.raises(acquire.AcquisitionError, match="partial file"):
        acquire.acquire_semantic_model(repo_root)

    assert not (repo_root / _DESTINATION.with_name("model.onnx.part")).exists()


def test_existing_verified_destination_skips_network_and_stale_partial_is_removed(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"already verified"
    repo_root = _repo(tmp_path, body=body)
    destination = repo_root / _DESTINATION
    destination.parent.mkdir(parents=True)
    _write_private(destination, body)
    part = destination.with_name("model.onnx.part")
    _write_private(part, b"stale")
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("verified destination must not open the network"),
    )

    assert acquire.acquire_semantic_model(repo_root) == destination
    assert destination.read_bytes() == body
    assert not part.exists()


def test_existing_unverified_destination_is_replaced_only_after_verification(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"new verified model"
    repo_root = _repo(tmp_path, body=body)
    destination = repo_root / _DESTINATION
    destination.parent.mkdir(parents=True)
    _write_private(destination, b"old invalid model")
    observed_existing: list[bytes] = []

    def fake_open(_request: object) -> _Response:
        return _Response(
            200,
            body=body,
            before_read=lambda: observed_existing.append(destination.read_bytes()),
        )

    monkeypatch.setattr(acquire, "_open_no_redirect", fake_open)

    assert acquire.acquire_semantic_model(repo_root) == destination
    assert observed_existing == [b"old invalid model"]
    assert destination.read_bytes() == body


def test_invalid_initial_url_is_rejected_before_transport(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = _repo(
        tmp_path,
        body=b"model",
        url="https://evil.example/model.onnx?api_key=initial-secret",
    )
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("invalid initial URL must be rejected before transport"),
    )

    with pytest.raises(acquire.AcquisitionError) as caught:
        acquire.acquire_semantic_model(repo_root)

    assert "initial-secret" not in str(caught.value)


@pytest.mark.parametrize(
    ("artifact", "acquirer", "destination"),
    [
        ("semantic-model", "acquire_semantic_model", _DESTINATION),
        ("onnx-runtime", "acquire_onnx_runtime", _ONNX_RUNTIME_DESTINATION),
    ],
)
def test_cli_accepts_only_explicit_qualification_acquisitions(
    monkeypatch: pytest.MonkeyPatch,
    artifact: str,
    acquirer: str,
    destination: Path,
) -> None:
    monkeypatch.setattr(acquire, acquirer, lambda _repo_root: _ROOT / destination)

    assert acquire.main([artifact], repo_root=_ROOT) == 0


def test_onnx_runtime_extracts_validated_relative_library_symlink_chain(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=archive),
    )

    destination = acquire.acquire_onnx_runtime(repo_root)

    library = destination / "lib/libonnxruntime.so"
    assert destination == repo_root / _ONNX_RUNTIME_DESTINATION
    assert library.is_symlink()
    assert library.readlink() == Path("libonnxruntime.so.1")
    assert library.resolve(strict=True) == (destination / "lib/libonnxruntime.so.1.28.0")
    assert library.read_bytes() == b"verified runtime library"
    assert not destination.with_name(f"{destination.name}.part").exists()
    assert not destination.with_name(f"{destination.name}.tgz.part").exists()


@pytest.mark.parametrize(
    "invalid",
    [
        "traversal",
        "absolute-link",
        "cycle",
        "escaping-link",
        "dangling-link",
        "device",
        "hard-link",
        "wrong-top-directory",
    ],
)
def test_onnx_runtime_rejects_unsafe_archive_members_without_external_writes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    invalid: str,
) -> None:
    archive = _runtime_archive(invalid=invalid)
    repo_root = _runtime_repo(tmp_path, archive=archive)
    outside = tmp_path / "outside"
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=archive),
    )

    with pytest.raises(acquire.AcquisitionError):
        acquire.acquire_onnx_runtime(repo_root)

    assert not outside.exists()
    assert not (repo_root / _ONNX_RUNTIME_DESTINATION).exists()
    assert (
        not (repo_root / _ONNX_RUNTIME_DESTINATION)
        .with_name(f"{_ONNX_RUNTIME_DESTINATION.name}.part")
        .exists()
    )
    assert (
        not (repo_root / _ONNX_RUNTIME_DESTINATION)
        .with_name(f"{_ONNX_RUNTIME_DESTINATION.name}.tgz.part")
        .exists()
    )


def test_onnx_runtime_rejects_checksum_mismatch_before_extraction(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(
        tmp_path,
        archive=archive,
        expected_archive=b"different approved archive",
    )
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=archive),
    )

    with pytest.raises(acquire.AcquisitionError, match="checksum"):
        acquire.acquire_onnx_runtime(repo_root)

    assert not (repo_root / _ONNX_RUNTIME_DESTINATION).exists()


def test_onnx_runtime_accepts_approved_signed_redirect_host(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    signed = "https://release-assets.githubusercontent.com/runtime.tgz?signature=runtime-secret"
    requests: list[str] = []

    def fake_open(request: object) -> _Response:
        requests.append(request.full_url)  # type: ignore[attr-defined]
        if request.full_url == _ONNX_RUNTIME_URL:  # type: ignore[attr-defined]
            return _Response(302, headers={"Location": signed})
        return _Response(200, body=archive)

    monkeypatch.setattr(acquire, "_open_no_redirect", fake_open)

    assert acquire.acquire_onnx_runtime(repo_root) == (repo_root / _ONNX_RUNTIME_DESTINATION)
    assert requests == [_ONNX_RUNTIME_URL, signed]


def test_onnx_runtime_rejects_archive_path_replacement_after_checksum(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    approved = _runtime_archive()
    replacement = _runtime_archive(library_body=b"unapproved replacement runtime")
    repo_root = _runtime_repo(tmp_path, archive=approved)
    archive_part = (repo_root / _ONNX_RUNTIME_DESTINATION).with_name(
        f"{_ONNX_RUNTIME_TOP}.tgz.part"
    )
    replacement_path = tmp_path / "replacement.tgz"
    replacement_path.write_bytes(replacement)
    real_extract = acquire._extract_runtime_archive

    def replace_before_extract(*args: object) -> None:
        os.replace(replacement_path, archive_part)
        real_extract(*args)  # type: ignore[arg-type]

    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=approved),
    )
    monkeypatch.setattr(acquire, "_extract_runtime_archive", replace_before_extract)

    with pytest.raises(acquire.AcquisitionError):
        acquire.acquire_onnx_runtime(repo_root)

    assert not (repo_root / _ONNX_RUNTIME_DESTINATION).exists()
    assert archive_part.read_bytes() == replacement


def test_onnx_runtime_rejects_same_inode_archive_mutation_after_checksum(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    approved = _runtime_archive()
    replacement = _runtime_archive(library_body=b"unapproved same-inode runtime")
    repo_root = _runtime_repo(tmp_path, archive=approved)
    real_extract = acquire._extract_runtime_archive
    mutated_identity: tuple[int, int] | None = None

    def mutate_before_extract(
        archive_fd: int,
        extraction_fd: int,
        deadline: acquire._Deadline,
    ) -> None:
        nonlocal mutated_identity
        before = os.fstat(archive_fd)
        os.ftruncate(archive_fd, 0)
        assert os.pwrite(archive_fd, replacement, 0) == len(replacement)
        after = os.fstat(archive_fd)
        mutated_identity = (after.st_dev, after.st_ino)
        assert mutated_identity == (before.st_dev, before.st_ino)
        real_extract(archive_fd, extraction_fd, deadline)

    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=approved),
    )
    monkeypatch.setattr(acquire, "_extract_runtime_archive", mutate_before_extract)

    with pytest.raises(acquire.AcquisitionError, match="checksum"):
        acquire.acquire_onnx_runtime(repo_root)

    assert mutated_identity is not None
    assert not (repo_root / _ONNX_RUNTIME_DESTINATION).exists()


def test_onnx_runtime_rejects_preexisting_runtime_without_approved_provenance(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    destination = repo_root / _ONNX_RUNTIME_DESTINATION
    library = destination / "lib/libonnxruntime.so"
    library.parent.mkdir(parents=True)
    library.write_bytes(b"plausible but unapproved runtime")
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("unproven runtime must fail before transport"),
    )

    with pytest.raises(acquire.AcquisitionError, match="provenance"):
        acquire.acquire_onnx_runtime(repo_root)

    assert library.read_bytes() == b"plausible but unapproved runtime"


def test_onnx_runtime_rejects_tampered_reused_runtime(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=archive),
    )
    destination = acquire.acquire_onnx_runtime(repo_root)
    library_target = (destination / "lib/libonnxruntime.so").resolve(strict=True)
    library_target.write_bytes(b"tampered runtime")
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("tampered runtime must fail before transport"),
    )

    with pytest.raises(acquire.AcquisitionError, match="provenance"):
        acquire.acquire_onnx_runtime(repo_root)

    assert library_target.read_bytes() == b"tampered runtime"


def test_onnx_runtime_reuses_valid_runtime_after_fresh_archive_verification(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)

    requests = 0

    def fresh_archive(_request: object) -> _Response:
        nonlocal requests
        requests += 1
        return _Response(200, body=archive)

    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        fresh_archive,
    )
    destination = acquire.acquire_onnx_runtime(repo_root)

    assert acquire.acquire_onnx_runtime(repo_root) == destination
    assert requests == 2


def test_onnx_runtime_rejects_forged_self_signed_provenance(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    destination = repo_root / _ONNX_RUNTIME_DESTINATION
    library = destination / "lib/libonnxruntime.so"
    library.parent.mkdir(parents=True)
    destination.chmod(0o700)
    library.write_bytes(b"attacker-provided runtime")
    library.chmod(0o600)
    directory_fd = os.open(destination, os.O_RDONLY | os.O_DIRECTORY)
    try:
        tree_sha256 = acquire._runtime_tree_digest(
            directory_fd,
            acquire._Deadline.after(5),
        )
    finally:
        os.close(directory_fd)
    provenance = destination / ".hieronymus-acquisition.json"
    provenance.write_text(
        json.dumps(
            {
                "archive_sha256": hashlib.sha256(archive).hexdigest(),
                "schema_version": 1,
                "tree_sha256": tree_sha256,
            },
            sort_keys=True,
            separators=(",", ":"),
        )
        + "\n",
        encoding="utf-8",
    )
    provenance.chmod(0o600)
    requests = 0

    def fresh_archive(_request: object) -> _Response:
        nonlocal requests
        requests += 1
        return _Response(200, body=archive)

    monkeypatch.setattr(acquire, "_open_no_redirect", fresh_archive)

    with pytest.raises(acquire.AcquisitionError):
        acquire.acquire_onnx_runtime(repo_root)

    assert requests == 1
    assert library.read_bytes() == b"attacker-provided runtime"


def test_onnx_runtime_cleanup_stays_on_held_parent_after_ancestor_swap(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    artifacts = repo_root / "qualification/.artifacts"
    models = artifacts / "models"
    held_models = artifacts / "held-models"
    victim = tmp_path / "external-victim"
    victim.mkdir()
    victim_archive = victim / f"{_ONNX_RUNTIME_TOP}.tgz.part"
    victim_runtime = victim / f"{_ONNX_RUNTIME_TOP}.part"
    sentinel = victim_runtime / "do-not-delete"
    real_extract = acquire._extract_runtime_archive
    swapped = False

    def swap_after_extract(*args: object) -> None:
        nonlocal swapped
        real_extract(*args)  # type: ignore[arg-type]
        models.rename(held_models)
        victim_archive.write_bytes(b"external archive")
        (victim_runtime / "lib").mkdir(parents=True)
        (victim_runtime / "lib/libonnxruntime.so").write_bytes(b"external runtime")
        sentinel.write_text("preserve me", encoding="utf-8")
        models.symlink_to(victim, target_is_directory=True)
        swapped = True

    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=archive),
    )
    monkeypatch.setattr(acquire, "_extract_runtime_archive", swap_after_extract)

    try:
        with pytest.raises(acquire.AcquisitionError):
            acquire.acquire_onnx_runtime(repo_root)
    finally:
        if swapped:
            models.unlink()
            held_models.rename(models)

    assert victim_archive.read_bytes() == b"external archive"
    assert sentinel.read_text(encoding="utf-8") == "preserve me"


def test_concurrent_onnx_runtime_acquisitions_share_owned_temporary_state(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    transport_barrier = threading.Barrier(2)
    start_barrier = threading.Barrier(3)
    results: list[Path] = []
    errors: list[Exception] = []

    def fake_open(_request: object) -> _Response:
        try:
            transport_barrier.wait(timeout=0.2)
        except threading.BrokenBarrierError:
            pass
        return _Response(200, body=archive)

    def worker() -> None:
        start_barrier.wait()
        try:
            results.append(acquire.acquire_onnx_runtime(repo_root))
        except Exception as error:  # pragma: no cover - asserted below
            errors.append(error)

    monkeypatch.setattr(acquire, "_open_no_redirect", fake_open)
    threads = [threading.Thread(target=worker) for _ in range(2)]
    for thread in threads:
        thread.start()
    start_barrier.wait()
    for thread in threads:
        thread.join(timeout=5)

    assert all(not thread.is_alive() for thread in threads)
    assert errors == []
    assert results == [repo_root / _ONNX_RUNTIME_DESTINATION] * 2
    assert (repo_root / _ONNX_RUNTIME_DESTINATION / "lib/libonnxruntime.so").is_file()
    assert (
        not (repo_root / _ONNX_RUNTIME_DESTINATION)
        .with_name(f"{_ONNX_RUNTIME_TOP}.tgz.part")
        .exists()
    )


def test_onnx_runtime_deadline_covers_extraction_and_prevents_promotion(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    real_extract = acquire._extract_runtime_archive

    def delayed_extract(*args: object) -> None:
        real_extract(*args)  # type: ignore[arg-type]
        time.sleep(0.05)

    monkeypatch.setattr(acquire, "_ACQUISITION_TIMEOUT_SECONDS", 0.01)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=archive),
    )
    monkeypatch.setattr(acquire, "_extract_runtime_archive", delayed_extract)

    with pytest.raises(acquire.AcquisitionError, match="deadline"):
        acquire.acquire_onnx_runtime(repo_root)

    assert not (repo_root / _ONNX_RUNTIME_DESTINATION).exists()


def test_onnx_runtime_deadline_after_final_validation_prevents_promotion(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    real_validate_parent = acquire._validate_runtime_parent
    validations = 0

    def delay_final_validation(*args: object) -> None:
        nonlocal validations
        validations += 1
        real_validate_parent(*args)  # type: ignore[arg-type]
        if validations == 2:
            time.sleep(0.05)

    monkeypatch.setattr(acquire, "_ACQUISITION_TIMEOUT_SECONDS", 0.02)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=archive),
    )
    monkeypatch.setattr(acquire, "_validate_runtime_parent", delay_final_validation)

    with pytest.raises(acquire.AcquisitionError, match="deadline"):
        acquire.acquire_onnx_runtime(repo_root)

    assert validations == 2
    assert not (repo_root / _ONNX_RUNTIME_DESTINATION).exists()


def test_onnx_runtime_reclaims_private_stale_archive_after_interrupted_run(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    destination = repo_root / _ONNX_RUNTIME_DESTINATION
    stale_archive = destination.with_name(f"{_ONNX_RUNTIME_TOP}.tgz.part")
    stale_archive.parent.mkdir(parents=True)
    stale_archive.write_bytes(b"stale interrupted archive")
    stale_archive.chmod(0o600)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=archive),
    )

    assert acquire.acquire_onnx_runtime(repo_root) == destination
    assert not stale_archive.exists()
    assert (destination / "lib/libonnxruntime.so").is_file()


def test_onnx_runtime_reclaims_private_stale_extraction_after_interrupted_run(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    archive = _runtime_archive()
    repo_root = _runtime_repo(tmp_path, archive=archive)
    destination = repo_root / _ONNX_RUNTIME_DESTINATION
    stale_extraction = destination.with_name(f"{_ONNX_RUNTIME_TOP}.part")
    stale_extraction.parent.mkdir(parents=True)
    stale_extraction.mkdir(mode=0o700)
    stale_file = stale_extraction / "interrupted"
    stale_file.write_bytes(b"stale extraction")
    stale_file.chmod(0o600)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _Response(200, body=archive),
    )

    assert acquire.acquire_onnx_runtime(repo_root) == destination
    assert not stale_extraction.exists()
    assert (destination / "lib/libonnxruntime.so").is_file()


def test_cli_prints_a_relative_destination_for_a_canonicalized_root_symlink(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    canonical_root = tmp_path / "canonical"
    canonical_root.mkdir()
    linked_root = tmp_path / "linked"
    linked_root.symlink_to(canonical_root, target_is_directory=True)
    destination = canonical_root / _DESTINATION
    monkeypatch.setattr(acquire, "acquire_semantic_model", lambda _root: destination)

    assert acquire.main(["semantic-model"], repo_root=linked_root) == 0
    assert capsys.readouterr().out == f"{_DESTINATION.as_posix()}\n"


def test_concurrent_acquisitions_cannot_replace_a_different_partial_inode(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Removing the process lock must make one caller publish another inode."""
    body = b"concurrent verified model"
    repo_root = _repo(tmp_path, body=body)
    first_read_started = threading.Event()
    release_first_read = threading.Event()
    open_lock = threading.Lock()
    open_count = 0

    def fake_open(_request: object) -> _Response:
        nonlocal open_count
        with open_lock:
            open_count += 1
            call = open_count
        if call == 1:
            return _Response(
                200,
                body=body,
                before_read=lambda: (
                    first_read_started.set(),
                    release_first_read.wait(timeout=5),
                ),
            )
        return _Response(200, body=body)

    monkeypatch.setattr(acquire, "_open_no_redirect", fake_open)
    results: list[Path] = []
    errors: list[BaseException] = []

    def run() -> None:
        try:
            results.append(acquire.acquire_semantic_model(repo_root))
        except BaseException as error:  # pragma: no cover - asserted below
            errors.append(error)

    first = threading.Thread(target=run)
    second = threading.Thread(target=run)
    first.start()
    assert first_read_started.wait(timeout=5)
    second.start()
    time.sleep(0.05)
    release_first_read.set()
    first.join(timeout=5)
    second.join(timeout=5)

    assert not first.is_alive() and not second.is_alive()
    assert errors == []
    assert results == [repo_root / _DESTINATION, repo_root / _DESTINATION]
    assert open_count == 1
    assert (repo_root / _DESTINATION).read_bytes() == body
    lock_path = (repo_root / _DESTINATION).with_name("model.onnx.lock")
    assert lock_path.is_file()
    assert lock_path.stat().st_nlink == 1


def test_processes_contend_on_one_unlink_proof_kernel_mutex(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"cross-process verified model"
    repo_root = _repo(tmp_path, body=body)
    context = multiprocessing.get_context("fork")
    first_read_started = context.Event()
    release_first_read = context.Event()
    open_count = context.Value("i", 0)
    results = context.Queue()

    def fake_open(_request: object) -> _Response:
        with open_count.get_lock():
            open_count.value += 1

        def replace_lock_while_held() -> None:
            lock_path = (repo_root / _DESTINATION).with_name("model.onnx.lock")
            lock_path.unlink()
            lock_path.write_bytes(b"")
            lock_path.chmod(0o600)
            first_read_started.set()
            assert release_first_read.wait(timeout=10)

        return _Response(
            200,
            body=body,
            before_read=replace_lock_while_held,
        )

    def run() -> None:
        try:
            results.put(("ok", str(acquire.acquire_semantic_model(repo_root))))
        except BaseException as error:  # pragma: no cover - asserted in parent
            results.put(("error", type(error).__name__))

    monkeypatch.setattr(acquire, "_open_no_redirect", fake_open)
    first = context.Process(target=run)
    second = context.Process(target=run)
    first.start()
    assert first_read_started.wait(timeout=5)
    second.start()
    time.sleep(0.05)
    assert open_count.value == 1
    release_first_read.set()
    first.join(timeout=10)
    second.join(timeout=10)

    assert first.exitcode == 0 and second.exitcode == 0
    received = [results.get(timeout=2), results.get(timeout=2)]
    assert received == [
        ("error", "AcquisitionError"),
        ("ok", str(repo_root / _DESTINATION)),
    ]
    assert open_count.value == 1
    lock_path = (repo_root / _DESTINATION).with_name("model.onnx.lock")
    assert lock_path.is_file()
    assert lock_path.stat().st_nlink == 1


@pytest.mark.parametrize(
    "ancestor",
    ["qualification", ".artifacts", "models", "all-MiniLM-L6-v2"],
)
def test_acquisition_rejects_symlinked_artifact_ancestor_without_external_writes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    ancestor: str,
) -> None:
    body = b"model"
    repo_root = tmp_path / "repo"
    outside = tmp_path / "outside"
    repo_root.mkdir()
    outside.mkdir()
    prerequisites = {
        "semantic_model": {"url": _INITIAL_URL, "sha256": hashlib.sha256(body).hexdigest()}
    }
    if ancestor == "qualification":
        (outside / "prerequisites.json").write_text(json.dumps(prerequisites), encoding="utf-8")
        (repo_root / "qualification").symlink_to(outside, target_is_directory=True)
    else:
        prerequisite_path = repo_root / "qualification/prerequisites.json"
        prerequisite_path.parent.mkdir()
        prerequisite_path.write_text(json.dumps(prerequisites), encoding="utf-8")
        components = [".artifacts", "models", "all-MiniLM-L6-v2"]
        parent = repo_root / "qualification"
        for component in components:
            path = parent / component
            if component == ancestor:
                path.symlink_to(outside, target_is_directory=True)
                break
            path.mkdir()
            parent = path
    sentinel = outside / "sentinel"
    sentinel.write_bytes(b"do not change")
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))

    with pytest.raises(acquire.AcquisitionError):
        acquire.acquire_semantic_model(repo_root)

    assert sentinel.read_bytes() == b"do not change"
    assert not (outside / "model.onnx").exists()
    assert not (outside / "model.onnx.part").exists()
    assert not (outside / "model.onnx.lock").exists()


@pytest.mark.parametrize(
    "ancestor",
    ["qualification", ".artifacts", "models", "all-MiniLM-L6-v2"],
)
def test_acquisition_detects_artifact_ancestor_swapped_to_symlink_during_transfer(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    ancestor: str,
) -> None:
    """Removing the pre-promotion chain check must allow a swapped tree to succeed."""
    body = b"model"
    repo_root = _repo(tmp_path / "repo", body=body)
    model_dir = (repo_root / _DESTINATION).parent
    model_dir.mkdir(parents=True)
    ancestor_path = {
        "qualification": repo_root / "qualification",
        ".artifacts": repo_root / "qualification/.artifacts",
        "models": repo_root / "qualification/.artifacts/models",
        "all-MiniLM-L6-v2": model_dir,
    }[ancestor]
    moved = ancestor_path.with_name(f"{ancestor_path.name}.moved")
    outside = tmp_path / f"outside-{ancestor}"
    outside.mkdir()
    sentinel = outside / "sentinel"
    sentinel.write_bytes(b"do not change")
    swapped = False

    class _SwappingResponse(_Response):
        @property
        def status(self) -> int:  # type: ignore[override]
            nonlocal swapped
            if not swapped:
                ancestor_path.rename(moved)
                ancestor_path.symlink_to(outside, target_is_directory=True)
                swapped = True
            return 200

        @status.setter
        def status(self, value: int) -> None:
            self._status = value

    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: _SwappingResponse(200, body=body),
    )

    with pytest.raises(acquire.AcquisitionError):
        acquire.acquire_semantic_model(repo_root)

    assert swapped
    assert sentinel.read_bytes() == b"do not change"
    assert not (outside / "model.onnx").exists()
    assert not (outside / "model.onnx.part").exists()


@pytest.mark.parametrize("leaf", ["model.onnx", "model.onnx.part", "model.onnx.lock"])
def test_acquisition_rejects_symlinked_artifact_leaf_without_touching_target(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    leaf: str,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path / "repo", body=body)
    model_dir = (repo_root / _DESTINATION).parent
    model_dir.mkdir(parents=True)
    outside = tmp_path / "outside"
    outside.write_bytes(b"do not change")
    (model_dir / leaf).symlink_to(outside)
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))

    with pytest.raises(acquire.AcquisitionError):
        acquire.acquire_semantic_model(repo_root)

    assert outside.read_bytes() == b"do not change"
    assert (model_dir / leaf).is_symlink()


def test_acquisition_rejects_declared_or_streamed_oversize_without_secret_echo(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path, body=body)
    monkeypatch.setattr(acquire, "_MAX_MODEL_BYTES", 8)
    too_large = 9
    responses = iter(
        (
            _Response(200, headers={"Content-Length": str(too_large)}, body=body),
            _Response(200, body=b"x" * too_large),
        )
    )
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: next(responses))

    for _case in range(2):
        with pytest.raises(acquire.AcquisitionError) as caught:
            acquire.acquire_semantic_model(repo_root)
        assert str(too_large) not in str(caught.value)
        assert not (repo_root / _DESTINATION.with_name("model.onnx.part")).exists()


def test_transport_open_uses_the_bounded_acquisition_timeout(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    observed: list[float] = []

    class _Opener:
        def open(self, _request: object, *, timeout: float) -> _Response:
            observed.append(timeout)
            return _Response(200)

    monkeypatch.setattr(acquire.urllib.request, "build_opener", lambda *_handlers: _Opener())

    response = acquire._open_no_redirect(acquire._request(_INITIAL_URL))
    response.close()

    assert observed == [acquire._TRANSPORT_TIMEOUT_SECONDS]
    assert 0 < observed[0] <= 300


def test_acquisition_fsyncs_the_written_inode_before_descriptor_relative_replace(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path, body=body)
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))
    events: list[str] = []
    real_fsync = os.fsync
    real_replace = os.replace

    def record_fsync(fd: int) -> None:
        events.append("fsync")
        real_fsync(fd)

    def record_replace(
        source: str,
        destination: str,
        *,
        src_dir_fd: int,
        dst_dir_fd: int,
    ) -> None:
        assert source == "model.onnx.part"
        assert destination == "model.onnx"
        assert src_dir_fd == dst_dir_fd
        events.append("replace")
        real_replace(
            source,
            destination,
            src_dir_fd=src_dir_fd,
            dst_dir_fd=dst_dir_fd,
        )

    monkeypatch.setattr(acquire.os, "fsync", record_fsync)
    monkeypatch.setattr(acquire.os, "replace", record_replace)

    acquire.acquire_semantic_model(repo_root)

    assert "fsync" in events
    assert events.index("fsync") < events.index("replace")


def test_unlinked_lock_file_cannot_split_the_thread_mutex_domain(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Replacing the advisory lock inode must not permit a second transport open."""
    body = b"unlink-proof model"
    repo_root = _repo(tmp_path, body=body)
    first_read_started = threading.Event()
    release_first_read = threading.Event()
    calls_lock = threading.Lock()
    open_count = 0

    def fake_open(_request: object) -> _Response:
        nonlocal open_count
        with calls_lock:
            open_count += 1
            call = open_count
        if call == 1:

            def replace_lock() -> None:
                lock_path = (repo_root / _DESTINATION).with_name("model.onnx.lock")
                lock_path.unlink()
                lock_path.write_bytes(b"")
                lock_path.chmod(0o600)
                first_read_started.set()
                assert release_first_read.wait(timeout=5)

            return _Response(200, body=body, before_read=replace_lock)
        return _Response(200, body=body)

    monkeypatch.setattr(acquire, "_open_no_redirect", fake_open)
    outcomes: list[str] = []

    def run() -> None:
        try:
            acquire.acquire_semantic_model(repo_root)
        except acquire.AcquisitionError:
            outcomes.append("rejected")
        else:
            outcomes.append("ok")

    first = threading.Thread(target=run)
    second = threading.Thread(target=run)
    first.start()
    assert first_read_started.wait(timeout=5)
    second.start()
    time.sleep(0.05)
    assert open_count == 1
    release_first_read.set()
    first.join(timeout=5)
    second.join(timeout=5)

    assert not first.is_alive() and not second.is_alive()
    assert outcomes.count("rejected") == 1
    assert outcomes.count("ok") == 1
    assert open_count == 1


def test_acquisition_rejects_permissive_lock_file(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path, body=body)
    model_dir = (repo_root / _DESTINATION).parent
    model_dir.mkdir(parents=True)
    lock_path = model_dir / "model.onnx.lock"
    lock_path.write_bytes(b"")
    lock_path.chmod(0o644)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("invalid lock metadata must fail before transport"),
    )

    with pytest.raises(acquire.AcquisitionError, match="private regular file"):
        acquire.acquire_semantic_model(repo_root)


def test_acquisition_rejects_lock_file_not_owned_by_current_identity(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path, body=body)
    model_dir = (repo_root / _DESTINATION).parent
    model_dir.mkdir(parents=True)
    lock_path = model_dir / "model.onnx.lock"
    lock_path.write_bytes(b"")
    lock_path.chmod(0o600)
    current_uid = os.geteuid()
    monkeypatch.setattr(
        acquire,
        "_require_trusted_directory",
        lambda metadata: (
            None
            if stat.S_ISDIR(metadata.st_mode)
            and metadata.st_uid == current_uid
            and not stat.S_IMODE(metadata.st_mode) & 0o022
            else pytest.fail("test directory unexpectedly became untrusted")
        ),
    )
    monkeypatch.setattr(acquire.os, "geteuid", lambda: current_uid + 1)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("foreign lock must fail before transport"),
    )

    with pytest.raises(acquire.AcquisitionError, match="private regular file"):
        acquire.acquire_semantic_model(repo_root)


def test_forked_child_drops_inherited_kernel_mutex_before_acquiring(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Without at-fork cleanup, the child keeps its own inherited bind alive forever."""
    body = b"fork-safe model"
    repo_root = _repo(tmp_path, body=body)
    context = multiprocessing.get_context("fork")
    result = context.Queue()
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))
    canonical = repo_root.resolve(strict=True)
    deadline = acquire._Deadline.after(5)

    def child() -> None:
        try:
            result.put(("ok", str(acquire.acquire_semantic_model(repo_root))))
        except BaseException as error:  # pragma: no cover - asserted in parent
            result.put(("error", type(error).__name__))

    with acquire._kernel_mutex(canonical, deadline):
        process = context.Process(target=child)
        process.start()
        time.sleep(0.05)
    process.join(timeout=5)

    assert process.exitcode == 0
    assert result.get(timeout=2) == ("ok", str(repo_root / _DESTINATION))


def test_forked_child_drops_every_inherited_lock_before_reacquiring(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A child must not retain either half of any inherited lock stack."""
    first_body = b"first fork-safe model"
    second_body = b"second fork-safe model"
    first_root = _repo(tmp_path / "first", body=first_body)
    second_root = _repo(tmp_path / "second", body=second_body)
    bodies = {
        first_root.resolve(strict=True): first_body,
        second_root.resolve(strict=True): second_body,
    }
    context = multiprocessing.get_context("fork")
    result = context.Queue()
    monkeypatch.setattr(acquire, "_ACQUISITION_TIMEOUT_SECONDS", 0.5)

    def child() -> None:
        try:
            acquired = []
            for repo_root in (first_root, second_root):
                expected = bodies[repo_root.resolve(strict=True)]
                acquire._open_no_redirect = lambda _request, body=expected: _Response(
                    200,
                    body=body,
                )
                acquired.append(str(acquire.acquire_semantic_model(repo_root)))
            result.put(("ok", acquired))
        except BaseException as error:  # pragma: no cover - asserted in parent
            result.put(("error", type(error).__name__))

    deadline = acquire._Deadline.after(5)
    token = acquire._CURRENT_DEADLINE.set(deadline)
    try:
        with acquire._open_model_directory(first_root) as (first_canonical, first_fds):
            with acquire._open_model_directory(second_root) as (second_canonical, second_fds):
                with acquire._kernel_mutex(first_canonical, deadline):
                    with acquire._acquisition_lock(first_fds[-1]):
                        with acquire._kernel_mutex(second_canonical, deadline):
                            with acquire._acquisition_lock(second_fds[-1]):
                                process = context.Process(target=child)
                                process.start()
                                time.sleep(0.05)
    finally:
        acquire._CURRENT_DEADLINE.reset(token)
    process.join(timeout=5)

    assert process.exitcode == 0
    assert result.get(timeout=2) == (
        "ok",
        [str(first_root / _DESTINATION), str(second_root / _DESTINATION)],
    )
    assert acquire._ACTIVE_MUTEX_SOCKETS == set()
    assert acquire._ACTIVE_ADVISORY_LOCK_FDS == set()


def test_fork_without_active_locks_does_not_poison_child_acquisition(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"no inherited locks"
    repo_root = _repo(tmp_path, body=body)
    context = multiprocessing.get_context("fork")
    result = context.Queue()
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))

    def child() -> None:
        try:
            result.put(("ok", str(acquire.acquire_semantic_model(repo_root))))
        except BaseException as error:  # pragma: no cover - asserted in parent
            result.put(("error", type(error).__name__))

    process = context.Process(target=child)
    process.start()
    process.join(timeout=5)

    assert process.exitcode == 0
    assert result.get(timeout=2) == ("ok", str(repo_root / _DESTINATION))


def test_forked_lock_context_does_not_close_reused_child_descriptor(tmp_path: Path) -> None:
    """Inherited context finalizers must not act on child fd-number reuse."""
    repo_root = _repo(tmp_path, body=b"fd reuse")
    read_fd, write_fd = os.pipe()
    deadline = acquire._Deadline.after(5)
    token = acquire._CURRENT_DEADLINE.set(deadline)
    child = False
    reused_fd = -1
    pid = -1
    try:
        with acquire._open_model_directory(repo_root) as (_canonical, directory_fds):
            with acquire._acquisition_lock(directory_fds[-1]) as lock_fd:
                pid = os.fork()
                if pid == 0:
                    child = True
                    os.close(read_fd)
                    replacement_fd = os.open(os.devnull, os.O_RDONLY | os.O_CLOEXEC)
                    if replacement_fd != lock_fd:
                        os.dup2(replacement_fd, lock_fd)
                        os.close(replacement_fd)
                    reused_fd = lock_fd
    finally:
        acquire._CURRENT_DEADLINE.reset(token)

    if child:  # pragma: no cover - result is asserted through the pipe
        try:
            os.fstat(reused_fd)
            os.write(write_fd, b"open")
        except OSError:
            os.write(write_fd, b"closed")
        finally:
            os._exit(0)

    os.close(write_fd)
    outcome = os.read(read_fd, 16)
    os.close(read_fd)
    waited_pid, status = os.waitpid(pid, 0)
    assert waited_pid == pid
    assert os.waitstatus_to_exitcode(status) == 0
    assert outcome == b"open"


def test_nested_lock_exception_releases_parent_tracking_and_locks(tmp_path: Path) -> None:
    repo_root = _repo(tmp_path, body=b"exception cleanup")
    canonical = repo_root.resolve(strict=True)
    deadline = acquire._Deadline.after(5)
    token = acquire._CURRENT_DEADLINE.set(deadline)
    try:
        with acquire._open_model_directory(repo_root) as (_canonical, directory_fds):
            with pytest.raises(RuntimeError, match="stop inside lock stack"):
                with acquire._kernel_mutex(canonical, deadline):
                    with acquire._acquisition_lock(directory_fds[-1]):
                        assert len(acquire._ACTIVE_MUTEX_SOCKETS) == 1
                        assert len(acquire._ACTIVE_ADVISORY_LOCK_FDS) == 1
                        raise RuntimeError("stop inside lock stack")

            assert acquire._ACTIVE_MUTEX_SOCKETS == set()
            assert acquire._ACTIVE_ADVISORY_LOCK_FDS == set()
            with acquire._kernel_mutex(canonical, deadline):
                with acquire._acquisition_lock(directory_fds[-1]):
                    pass
    finally:
        acquire._CURRENT_DEADLINE.reset(token)

    assert acquire._ACTIVE_MUTEX_SOCKETS == set()
    assert acquire._ACTIVE_ADVISORY_LOCK_FDS == set()


def test_process_exit_releases_full_lock_stack(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"lock owner exited"
    repo_root = _repo(tmp_path, body=body)
    context = multiprocessing.get_context("fork")
    ready = context.Event()

    def exit_while_holding_locks() -> None:
        deadline = acquire._Deadline.after(5)
        token = acquire._CURRENT_DEADLINE.set(deadline)
        try:
            with acquire._open_model_directory(repo_root) as (canonical, directory_fds):
                with acquire._kernel_mutex(canonical, deadline):
                    with acquire._acquisition_lock(directory_fds[-1]):
                        ready.set()
                        os._exit(0)
        finally:  # pragma: no cover - os._exit intentionally skips finalizers
            acquire._CURRENT_DEADLINE.reset(token)

    process = context.Process(target=exit_while_holding_locks)
    process.start()
    assert ready.wait(timeout=5)
    process.join(timeout=5)
    assert process.exitcode == 0

    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))
    assert acquire.acquire_semantic_model(repo_root) == repo_root / _DESTINATION


@pytest.mark.parametrize("existing", [False, True], ids=["promoted", "existing"])
@pytest.mark.parametrize(
    "stage",
    [
        "after-directory-check",
        "after-destination-open",
        "after-destination-hash",
        "after-named-destination-check",
    ],
)
def test_final_validation_rejects_leaf_swap_at_every_in_operation_hook(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    existing: bool,
    stage: str,
) -> None:
    body = b"final verified bytes"
    repo_root = _repo(tmp_path, body=body)
    destination = repo_root / _DESTINATION
    if existing:
        destination.parent.mkdir(parents=True)
        _write_private(destination, body)
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))
    swapped = False

    def hook(current: str) -> None:
        nonlocal swapped
        if current != stage or swapped:
            return
        displaced = destination.with_name("model.onnx.displaced")
        destination.rename(displaced)
        destination.write_bytes(b"attacker replacement")
        swapped = True

    monkeypatch.setattr(acquire, "_final_validation_hook", hook)

    with pytest.raises(acquire.AcquisitionError):
        acquire.acquire_semantic_model(repo_root)

    assert swapped
    assert destination.read_bytes() == b"attacker replacement"


@pytest.mark.parametrize("existing", [False, True], ids=["promoted", "existing"])
@pytest.mark.parametrize(
    "stage",
    [
        "after-directory-check",
        "after-destination-open",
        "after-destination-hash",
        "after-named-destination-check",
    ],
)
@pytest.mark.parametrize(
    "ancestor",
    ["qualification", ".artifacts", "models", "all-MiniLM-L6-v2"],
)
def test_final_validation_rejects_ancestor_swap_at_every_in_operation_hook(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    existing: bool,
    stage: str,
    ancestor: str,
) -> None:
    body = b"final verified bytes"
    repo_root = _repo(tmp_path / "repo", body=body)
    destination = repo_root / _DESTINATION
    destination.parent.mkdir(parents=True)
    if existing:
        _write_private(destination, body)
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))
    ancestor_path = {
        "qualification": repo_root / "qualification",
        ".artifacts": repo_root / "qualification/.artifacts",
        "models": repo_root / "qualification/.artifacts/models",
        "all-MiniLM-L6-v2": destination.parent,
    }[ancestor]
    outside = tmp_path / f"outside-{ancestor}-{existing}-{stage}"
    outside.mkdir()
    swapped = False

    def hook(current: str) -> None:
        nonlocal swapped
        if current != stage or swapped:
            return
        ancestor_path.rename(ancestor_path.with_name(f"{ancestor_path.name}.displaced"))
        ancestor_path.symlink_to(outside, target_is_directory=True)
        swapped = True

    monkeypatch.setattr(acquire, "_final_validation_hook", hook)

    with pytest.raises(acquire.AcquisitionError):
        acquire.acquire_semantic_model(repo_root)

    assert swapped


def test_final_validation_rejects_in_place_mutation_after_hash(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"trusted model bytes"
    replacement = b"changed model bytes"
    assert len(body) == len(replacement)
    repo_root = _repo(tmp_path, body=body)
    destination = repo_root / _DESTINATION
    destination.parent.mkdir(parents=True)
    _write_private(destination, body)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("verified fast path must not open transport"),
    )

    def hook(stage: str) -> None:
        if stage == "after-destination-hash":
            destination.write_bytes(replacement)

    monkeypatch.setattr(acquire, "_final_validation_hook", hook)

    with pytest.raises(acquire.AcquisitionError, match="identity changed"):
        acquire.acquire_semantic_model(repo_root)


def test_total_deadline_bounds_redirect_sequence(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path, body=body)
    now = 100.0
    opened: list[_Response] = []
    closed: list[_Response] = []

    class _TrackedResponse(_Response):
        def close(self) -> None:
            closed.append(self)

    def monotonic() -> float:
        return now

    def fake_open(_request: object) -> _Response:
        nonlocal now
        now += 0.6
        response = _TrackedResponse(302, headers={"Location": _INITIAL_URL})
        opened.append(response)
        return response

    monkeypatch.setattr(acquire, "_monotonic", monotonic)
    monkeypatch.setattr(acquire, "_ACQUISITION_TIMEOUT_SECONDS", 1.0)
    monkeypatch.setattr(acquire, "_open_no_redirect", fake_open)

    with pytest.raises(acquire.AcquisitionError, match="deadline"):
        acquire.acquire_semantic_model(repo_root)

    assert closed == opened


def test_existing_destination_is_rejected_when_it_exceeds_the_byte_limit(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"123456789"
    repo_root = _repo(tmp_path, body=body)
    destination = repo_root / _DESTINATION
    destination.parent.mkdir(parents=True)
    _write_private(destination, body)
    monkeypatch.setattr(acquire, "_MAX_MODEL_BYTES", 8)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("oversized existing destination must fail closed"),
    )

    with pytest.raises(acquire.AcquisitionError, match="byte limit"):
        acquire.acquire_semantic_model(repo_root)


def test_each_new_directory_entry_is_fsynced_in_its_parent(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path, body=body)
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))
    created_parent_inodes: list[tuple[int, int]] = []
    synced_inodes: list[tuple[int, int]] = []
    real_mkdir = os.mkdir
    real_fsync = os.fsync

    def record_mkdir(path: str, mode: int = 0o777, *, dir_fd: int | None = None) -> None:
        assert dir_fd is not None
        real_mkdir(path, mode=mode, dir_fd=dir_fd)
        metadata = os.fstat(dir_fd)
        created_parent_inodes.append((metadata.st_dev, metadata.st_ino))

    def record_fsync(fd: int) -> None:
        metadata = os.fstat(fd)
        if stat.S_ISDIR(metadata.st_mode):
            synced_inodes.append((metadata.st_dev, metadata.st_ino))
        real_fsync(fd)

    monkeypatch.setattr(acquire.os, "mkdir", record_mkdir)
    monkeypatch.setattr(acquire.os, "fsync", record_fsync)

    acquire.acquire_semantic_model(repo_root)

    assert created_parent_inodes
    assert all(inode in synced_inodes for inode in created_parent_inodes)


@pytest.mark.parametrize("leaf", ["model.onnx", "model.onnx.part", "model.onnx.lock"])
def test_acquisition_rejects_external_hardlinked_artifact_leaf(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    leaf: str,
) -> None:
    """Dropping the nlink check must let an external inode enter acquisition state."""
    body = b"model"
    repo_root = _repo(tmp_path / "repo", body=body)
    model_dir = (repo_root / _DESTINATION).parent
    model_dir.mkdir(parents=True)
    external = tmp_path / f"external-{leaf}"
    external.write_bytes(body if leaf == "model.onnx" else b"")
    external.chmod(0o600)
    os.link(external, model_dir / leaf)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("hardlinked state must fail before transport"),
    )

    with pytest.raises(acquire.AcquisitionError, match="private regular file"):
        acquire.acquire_semantic_model(repo_root)

    assert external.read_bytes() == (body if leaf == "model.onnx" else b"")
    assert external.stat().st_nlink == 2


@pytest.mark.parametrize("leaf", ["model.onnx", "model.onnx.part", "model.onnx.lock"])
def test_acquisition_rejects_permissive_artifact_leaf(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    leaf: str,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path, body=body)
    model_dir = (repo_root / _DESTINATION).parent
    model_dir.mkdir(parents=True)
    artifact = model_dir / leaf
    artifact.write_bytes(body if leaf == "model.onnx" else b"")
    artifact.chmod(0o640)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("permissive state must fail before transport"),
    )

    with pytest.raises(acquire.AcquisitionError, match="private regular file"):
        acquire.acquire_semantic_model(repo_root)


@pytest.mark.parametrize(
    "ancestor",
    ["qualification", ".artifacts", "models", "all-MiniLM-L6-v2"],
)
def test_acquisition_rejects_group_writable_artifact_directory(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    ancestor: str,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path / "repo", body=body)
    model_dir = (repo_root / _DESTINATION).parent
    model_dir.mkdir(parents=True)
    selected = {
        "qualification": repo_root / "qualification",
        ".artifacts": repo_root / "qualification/.artifacts",
        "models": repo_root / "qualification/.artifacts/models",
        "all-MiniLM-L6-v2": model_dir,
    }[ancestor]
    selected.chmod(0o770)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("untrusted directory must fail before transport"),
    )

    with pytest.raises(acquire.AcquisitionError, match="trusted directory"):
        acquire.acquire_semantic_model(repo_root)


def test_acquisition_rejects_artifact_directory_not_owned_by_current_identity(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = _repo(tmp_path, body=b"model")
    current_uid = os.geteuid()
    monkeypatch.setattr(acquire.os, "geteuid", lambda: current_uid + 1)
    monkeypatch.setattr(
        acquire,
        "_open_no_redirect",
        lambda _request: pytest.fail("foreign directory must fail before transport"),
    )

    with pytest.raises(acquire.AcquisitionError, match="trusted directory"):
        acquire.acquire_semantic_model(repo_root)


def test_acquisition_creates_private_artifact_state(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path, body=body)
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))

    destination = acquire.acquire_semantic_model(repo_root)

    for directory in (
        repo_root / "qualification/.artifacts",
        repo_root / "qualification/.artifacts/models",
        destination.parent,
    ):
        assert stat.S_IMODE(directory.stat().st_mode) == 0o700
        assert directory.stat().st_uid == os.geteuid()
    for leaf in (destination, destination.with_name("model.onnx.lock")):
        metadata = leaf.stat()
        assert stat.S_IMODE(metadata.st_mode) == 0o600
        assert metadata.st_uid == os.geteuid()
        assert metadata.st_nlink == 1


@pytest.mark.parametrize("existing", [False, True], ids=["promoted", "existing"])
def test_final_validation_rejects_combined_hardlink_and_ancestor_swap(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    existing: bool,
) -> None:
    """The returned lexical path must never resolve through a swapped ancestor."""
    body = b"final verified bytes"
    repo_root = _repo(tmp_path / "repo", body=body)
    destination = repo_root / _DESTINATION
    destination.parent.mkdir(parents=True)
    if existing:
        destination.write_bytes(body)
        destination.chmod(0o600)
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _Response(200, body=body))
    outside = tmp_path / f"outside-{existing}"
    outside.mkdir()
    swapped = False

    def hook(stage: str) -> None:
        nonlocal swapped
        if stage != "after-named-destination-check" or swapped:
            return
        os.link(destination, outside / "model.onnx")
        displaced = destination.parent.with_name("all-MiniLM-L6-v2.displaced")
        destination.parent.rename(displaced)
        destination.parent.symlink_to(outside, target_is_directory=True)
        swapped = True

    monkeypatch.setattr(acquire, "_final_validation_hook", hook)

    with pytest.raises(acquire.AcquisitionError):
        acquire.acquire_semantic_model(repo_root)

    assert swapped
    assert destination.resolve(strict=True) == outside / "model.onnx"


@pytest.mark.parametrize("status,body", [(503, b""), (200, b"downloaded bytes")])
def test_response_close_failure_is_redacted_and_cleans_owned_partial(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    status: int,
    body: bytes,
) -> None:
    expected = body or b"expected"
    repo_root = _repo(tmp_path, body=expected)
    response = _Response(
        status,
        body=body,
        close_error=OSError("signed-query-close-secret"),
    )
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: response)
    before_fds = len(os.listdir("/proc/self/fd"))

    with pytest.raises(acquire.AcquisitionError) as caught:
        acquire.acquire_semantic_model(repo_root)

    assert "signed-query-close-secret" not in str(caught.value)
    assert caught.value.__cause__ is None
    assert not (repo_root / _DESTINATION.with_name("model.onnx.part")).exists()
    assert len(os.listdir("/proc/self/fd")) == before_fds


def test_model_directory_context_does_not_reclassify_body_oserror(tmp_path: Path) -> None:
    repo_root = _repo(tmp_path, body=b"model")
    token = acquire._CURRENT_DEADLINE.set(acquire._Deadline.after(5))
    try:
        with pytest.raises(OSError, match="body marker"):
            with acquire._open_model_directory(repo_root):
                raise OSError("body marker")
    finally:
        acquire._CURRENT_DEADLINE.reset(token)


def test_each_response_read_rearms_socket_to_remaining_total_deadline(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"abcd"
    repo_root = _repo(tmp_path, body=body)
    now = 100.0

    class _TimedResponse(_Response):
        def read(self, size: int = -1) -> bytes:
            nonlocal now
            result = super().read(size)
            now += 1.0
            return result

    response = _TimedResponse(200, body=body)
    monkeypatch.setattr(acquire, "_monotonic", lambda: now)
    monkeypatch.setattr(acquire, "_ACQUISITION_TIMEOUT_SECONDS", 10.0)
    monkeypatch.setattr(acquire, "_CHUNK_SIZE", 2)
    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: response)

    acquire.acquire_semantic_model(repo_root)

    assert response.read_timeouts == [10.0, 9.0, 8.0]


def test_response_without_rearmable_socket_fails_closed_and_cleans_partial(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"model"
    repo_root = _repo(tmp_path, body=body)

    class _UnadaptableResponse:
        status = 200
        headers: dict[str, str] = {}

        def read(self, _size: int = -1) -> bytes:
            return body

        def close(self) -> None:
            return None

    monkeypatch.setattr(acquire, "_open_no_redirect", lambda _request: _UnadaptableResponse())

    with pytest.raises(acquire.AcquisitionError, match="timeout"):
        acquire.acquire_semantic_model(repo_root)

    assert not (repo_root / _DESTINATION.with_name("model.onnx.part")).exists()


def test_response_timeout_adapter_supports_urllib_response_socket_shape() -> None:
    observed: list[float] = []

    class _Socket:
        def settimeout(self, timeout: float) -> None:
            observed.append(timeout)

    class _Raw:
        _sock = _Socket()

    class _Buffered:
        raw = _Raw()

    class _UrllibResponse:
        fp = _Buffered()

    acquire._set_response_read_timeout(_UrllibResponse(), 3.25)

    assert observed == [3.25]


def test_response_timeout_adapter_rearms_real_http_response_socket() -> None:
    client, server = acquire.socket.socketpair()
    try:
        server.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        response = http.client.HTTPResponse(client)
        response.begin()

        acquire._set_response_read_timeout(response, 2.5)

        assert client.gettimeout() == 2.5
        response.close()
    finally:
        client.close()
        server.close()


def test_response_timeout_adapter_accepts_closed_real_http_response_eof() -> None:
    client, server = acquire.socket.socketpair()
    try:
        server.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        response = http.client.HTTPResponse(client)
        response.begin()
        assert response.read() == b""
        assert response.isclosed()

        acquire._set_response_read_timeout(response, 2.5)

        assert response.read() == b""
    finally:
        client.close()
        server.close()
