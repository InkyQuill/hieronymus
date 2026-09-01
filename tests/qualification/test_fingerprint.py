"""Adversarial tests for qualification input fingerprints."""

from __future__ import annotations

import os
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


def test_input_fingerprint_rejects_repository_root_reached_through_parent_symlink(
    tmp_path: Path,
) -> None:
    real_parent = tmp_path / "real-parent"
    repo = real_parent / "repo"
    repo.mkdir(parents=True)
    (repo / "fixture").write_text("fixture", encoding="utf-8")
    alias = tmp_path / "parent-alias"
    alias.symlink_to(real_parent, target_is_directory=True)

    with pytest.raises(ValueError, match="repository root"):
        fingerprint_inputs(alias / "repo", ("fixture",))


def test_input_fingerprint_rejects_repository_root_symlink(tmp_path: Path) -> None:
    repository = tmp_path / "repository"
    repository.mkdir()
    (repository / "fixture").write_text("fixture", encoding="utf-8")
    alias = tmp_path / "repository-alias"
    alias.symlink_to(repository, target_is_directory=True)

    with pytest.raises(ValueError, match="repository root"):
        fingerprint_inputs(alias, ("fixture",))


def test_input_fingerprint_rejects_noncanonical_repository_root_spelling(
    tmp_path: Path,
) -> None:
    chosen = tmp_path / "chosen"
    chosen.mkdir()
    (chosen / "fixture").write_text("chosen", encoding="utf-8")
    other = tmp_path / "other"
    other.mkdir()
    alias = tmp_path / "alias"
    alias.symlink_to(other, target_is_directory=True)
    noncanonical = Path(f"{alias}/../chosen")

    with pytest.raises(ValueError, match="repository root") as caught:
        fingerprint_inputs(noncanonical, ("fixture",))

    assert str(tmp_path) not in str(caught.value)


def test_input_fingerprint_rejects_symlinked_input_ancestor(tmp_path: Path) -> None:
    target = tmp_path / "target"
    target.mkdir()
    (target / "fixture").write_text("fixture", encoding="utf-8")
    (tmp_path / "alias").symlink_to(target, target_is_directory=True)

    with pytest.raises(ValueError, match="alias/fixture"):
        fingerprint_inputs(tmp_path, ("alias/fixture",))


def test_input_fingerprint_rejects_hard_link_aliases(tmp_path: Path) -> None:
    first = tmp_path / "first"
    first.write_text("fixture", encoding="utf-8")
    os.link(first, tmp_path / "second")

    with pytest.raises(ValueError, match="hard-link alias.*second"):
        fingerprint_inputs(tmp_path, ("first", "second"))


@pytest.mark.parametrize("kind", ["missing", "non-directory", "special"])
def test_input_fingerprint_reports_safe_deterministic_filesystem_errors(
    tmp_path: Path,
    kind: str,
) -> None:
    relative = f"{kind}/fixture" if kind != "special" else "special"
    if kind == "non-directory":
        (tmp_path / kind).write_text("not a directory", encoding="utf-8")
    elif kind == "special":
        os.mkfifo(tmp_path / relative)

    with pytest.raises(ValueError) as caught:
        fingerprint_inputs(tmp_path, (relative,))

    assert relative in str(caught.value)
    assert str(tmp_path) not in str(caught.value)


def test_input_fingerprint_reports_unreadable_input_without_host_path(tmp_path: Path) -> None:
    protected = tmp_path / "protected"
    protected.mkdir()
    fixture = protected / "fixture"
    fixture.write_text("fixture", encoding="utf-8")
    protected.chmod(0)
    try:
        if os.access(fixture, os.R_OK):
            pytest.skip("test account can bypass directory permissions")
        with pytest.raises(ValueError) as caught:
            fingerprint_inputs(tmp_path, ("protected/fixture",))
    finally:
        protected.chmod(0o700)

    assert "protected/fixture" in str(caught.value)
    assert str(tmp_path) not in str(caught.value)


def test_input_fingerprint_reports_read_error_without_host_path(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    (tmp_path / "fixture").write_text("fixture", encoding="utf-8")

    def failed_read(descriptor: int, size: int) -> bytes:
        del descriptor, size
        raise OSError(5, str(tmp_path / "private-read-error"))

    monkeypatch.setattr(os, "read", failed_read)

    with pytest.raises(ValueError) as caught:
        fingerprint_inputs(tmp_path, ("fixture",))

    assert str(caught.value) == "fingerprint input 'fixture' must be an existing regular file"
    assert str(tmp_path) not in str(caught.value)


def test_input_fingerprint_reports_dup_error_without_host_path(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    (tmp_path / "fixture").write_text("fixture", encoding="utf-8")

    def failed_dup(descriptor: int) -> int:
        del descriptor
        raise OSError(24, str(tmp_path / "private-dup-error"))

    monkeypatch.setattr(os, "dup", failed_dup)

    with pytest.raises(ValueError) as caught:
        fingerprint_inputs(tmp_path, ("fixture",))

    assert str(caught.value) == "fingerprint input 'fixture' must be an existing regular file"
    assert str(tmp_path) not in str(caught.value)


def test_input_fingerprint_rejects_same_size_in_place_change_during_read(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    fixture = tmp_path / "fixture"
    fixture.write_bytes(b"a" * (2 * 1024 * 1024))
    original_read = os.read
    changed = False

    def changing_read(descriptor: int, size: int) -> bytes:
        nonlocal changed
        chunk = original_read(descriptor, size)
        if chunk and not changed:
            changed = True
            fixture.write_bytes(b"b" * (2 * 1024 * 1024))
        return chunk

    monkeypatch.setattr(os, "read", changing_read)

    with pytest.raises(ValueError, match="changed while being read") as caught:
        fingerprint_inputs(tmp_path, ("fixture",))

    assert changed
    assert str(tmp_path) not in str(caught.value)


def test_input_fingerprint_uses_open_directory_after_ancestor_path_swap(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    original_parent = tmp_path / "safe"
    original_parent.mkdir()
    (original_parent / "fixture.txt").write_text("trusted", encoding="utf-8")
    comparison_root = tmp_path / "comparison"
    (comparison_root / "safe").mkdir(parents=True)
    (comparison_root / "safe/fixture.txt").write_text("trusted", encoding="utf-8")
    attacker = tmp_path / "attacker"
    attacker.mkdir()
    (attacker / "fixture.txt").write_text("attacker", encoding="utf-8")
    detached = tmp_path / "detached"
    expected = fingerprint_inputs(comparison_root, ("safe/fixture.txt",))
    original_open = os.open
    swapped = False

    def swapping_open(
        path: str,
        flags: int,
        mode: int = 0o777,
        *,
        dir_fd: int | None = None,
    ) -> int:
        nonlocal swapped
        if path == "fixture.txt" and dir_fd is not None and not swapped:
            swapped = True
            original_parent.rename(detached)
            original_parent.symlink_to(attacker, target_is_directory=True)
        return original_open(path, flags, mode, dir_fd=dir_fd)

    monkeypatch.setattr(os, "open", swapping_open)

    assert fingerprint_inputs(tmp_path, ("safe/fixture.txt",)) == expected
    assert swapped


def test_input_fingerprint_closes_descriptors_on_repeated_failures(tmp_path: Path) -> None:
    descriptor_root = Path("/proc/self/fd")
    if not descriptor_root.is_dir():
        pytest.skip("Linux descriptor accounting is unavailable")
    before = len(tuple(descriptor_root.iterdir()))

    for _ in range(50):
        with pytest.raises(ValueError):
            fingerprint_inputs(tmp_path, ("missing/fixture",))

    assert len(tuple(descriptor_root.iterdir())) == before
