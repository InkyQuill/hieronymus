"""Adversarial tests for qualification input fingerprints."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

_ROOT = Path(__file__).resolve().parents[2]
if str(_ROOT) not in sys.path:
    sys.path.insert(0, str(_ROOT))

from tools.qualification.fingerprint import fingerprint_inputs  # noqa: E402


def test_input_fingerprint_changes_with_fixture_bytes(tmp_path: Path) -> None:
    fixture = tmp_path / "fixture.txt"
    fixture.write_text("before", encoding="utf-8")
    before = fingerprint_inputs(tmp_path, ("fixture.txt",))

    fixture.write_text("after", encoding="utf-8")

    assert fingerprint_inputs(tmp_path, ("fixture.txt",)) != before


def test_input_fingerprint_is_independent_of_caller_order(tmp_path: Path) -> None:
    (tmp_path / "alpha").write_bytes(b"first\x00bytes")
    (tmp_path / "beta").write_bytes(b"second\xffbytes")

    assert fingerprint_inputs(tmp_path, ("alpha", "beta")) == fingerprint_inputs(
        tmp_path,
        ("beta", "alpha"),
    )


def test_input_fingerprint_frames_names_and_raw_bytes_unambiguously(
    tmp_path: Path,
) -> None:
    # Both naïve path+content concatenations are b"abc". A framed fingerprint must
    # still distinguish the canonical name and raw file bytes.
    (tmp_path / "a").write_bytes(b"bc")
    (tmp_path / "ab").write_bytes(b"c")

    assert fingerprint_inputs(tmp_path, ("a",)) != fingerprint_inputs(tmp_path, ("ab",))


def test_input_fingerprint_hashes_non_utf8_authority_bytes_without_normalizing(
    tmp_path: Path,
) -> None:
    authority = tmp_path / "schema.json"
    authority.write_bytes(b"{\r\n\xff\x00}\n")
    original = fingerprint_inputs(tmp_path, ("schema.json",))

    authority.write_bytes(b"{\n\xff\x00}\n")

    assert fingerprint_inputs(tmp_path, ("schema.json",)) != original


@pytest.mark.parametrize(
    ("relative", "message"),
    [
        ("/tmp/fixture", "relative"),
        ("C:\\Users\\alice\\fixture", "relative"),
        ("../fixture", "canonical"),
        ("nested/../fixture", "canonical"),
        ("./fixture", "canonical"),
        ("", "nonempty"),
    ],
)
def test_input_fingerprint_rejects_noncanonical_or_absolute_paths(
    tmp_path: Path,
    relative: str,
    message: str,
) -> None:
    (tmp_path / "fixture").write_text("fixture", encoding="utf-8")

    with pytest.raises(ValueError, match=message):
        fingerprint_inputs(tmp_path, (relative,))


def test_input_fingerprint_rejects_duplicate_paths(tmp_path: Path) -> None:
    (tmp_path / "fixture").write_text("fixture", encoding="utf-8")

    with pytest.raises(ValueError, match="duplicate"):
        fingerprint_inputs(tmp_path, ("fixture", "fixture"))


@pytest.mark.parametrize("kind", ["missing", "directory", "symlink"])
def test_input_fingerprint_accepts_only_existing_regular_files(
    tmp_path: Path,
    kind: str,
) -> None:
    relative = kind
    if kind == "directory":
        (tmp_path / relative).mkdir()
    elif kind == "symlink":
        target = tmp_path / "target"
        target.write_text("fixture", encoding="utf-8")
        (tmp_path / relative).symlink_to(target)

    with pytest.raises(ValueError, match="regular file"):
        fingerprint_inputs(tmp_path, (relative,))
