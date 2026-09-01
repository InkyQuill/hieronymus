"""Deterministic tests for checksum-verified qualification acquisitions."""

from __future__ import annotations

import hashlib
import json
import os
import sys
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


class _Response:
    def __init__(
        self,
        status: int,
        *,
        headers: Mapping[str, str] | None = None,
        body: bytes = b"",
        read_error: OSError | None = None,
        before_read: Callable[[], None] | None = None,
    ) -> None:
        self.status = status
        self.headers = dict(headers or {})
        self._body = body
        self._offset = 0
        self._read_error = read_error
        self._before_read = before_read

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
    replace_calls: list[tuple[Path, Path]] = []
    real_replace = os.replace

    def atomic_replace(source: Path, destination: Path) -> None:
        replace_calls.append((source, destination))
        real_replace(source, destination)

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
    assert replace_calls == [(destination.with_name("model.onnx.part"), destination)]


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
    part.write_bytes(b"stale partial")
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


def test_existing_verified_destination_skips_network_and_stale_partial_is_removed(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    body = b"already verified"
    repo_root = _repo(tmp_path, body=body)
    destination = repo_root / _DESTINATION
    destination.parent.mkdir(parents=True)
    destination.write_bytes(body)
    part = destination.with_name("model.onnx.part")
    part.write_bytes(b"stale")
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
    destination.write_bytes(b"old invalid model")
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


def test_cli_accepts_only_semantic_model(monkeypatch: pytest.MonkeyPatch) -> None:
    destination = _ROOT / _DESTINATION
    monkeypatch.setattr(acquire, "acquire_semantic_model", lambda _repo_root: destination)

    assert acquire.main(["semantic-model"], repo_root=_ROOT) == 0
    with pytest.raises(SystemExit):
        acquire.main(["onnx-runtime"], repo_root=_ROOT)
