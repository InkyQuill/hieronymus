"""Behavioral release guard checks; these do not publish or qualify a host."""

import hashlib
import importlib.util
import json
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


@pytest.fixture
def source(tmp_path):
    (tmp_path / "Cargo.toml").write_text('[workspace.package]\nversion="0.8.0"\n')
    (tmp_path / "Cargo.lock").write_text(
        '[[package]]\nname="hiero"\nversion="0.8.0"\n[[package]]\nname="hieronymus"\nversion="0.8.0"\n'
    )
    for name in ("hiero", "hieronymus"):
        crate = tmp_path / "crates" / name
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_text("[package]\nversion.workspace=true\n")
    subprocess.run(["git", "init", "-q", str(tmp_path)], check=True)
    subprocess.run(["git", "-C", str(tmp_path), "add", "."], check=True)
    subprocess.run(
        [
            "git",
            "-C",
            str(tmp_path),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ],
        check=True,
    )
    subprocess.run(["git", "-C", str(tmp_path), "tag", "v0.8.0"], check=True)
    return tmp_path


def test_exact_tag_and_explicit_untagged_candidate(source):
    guard = module("check-rust-release")
    assert guard.check_source(source, "refs/tags/v0.8.0") == "0.8.0"
    assert guard.check_source(source, None, allow_untagged=True) == "0.8.0"
    with pytest.raises(ValueError):
        guard.check_source(source, None)


@pytest.mark.parametrize(
    "ref", ["refs/heads/main", "refs/tags/v0.9.0", "refs/tags/v0.8.0;echo bad"]
)
def test_wrong_ref_rejected_before_release(source, ref):
    with pytest.raises(ValueError):
        module("check-rust-release").check_source(source, ref)


def test_tag_must_point_to_checked_out_commit(source):
    (source / "new").write_text("new")
    subprocess.run(["git", "-C", str(source), "add", "."], check=True)
    subprocess.run(
        [
            "git",
            "-C",
            str(source),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "new",
        ],
        check=True,
    )
    with pytest.raises(ValueError):
        module("check-rust-release").check_source(source, "refs/tags/v0.8.0")


@pytest.mark.parametrize("broken", ["version", "lock", "inheritance"])
def test_version_sources_must_agree(source, broken):
    path = {
        "version": source / "Cargo.toml",
        "lock": source / "Cargo.lock",
        "inheritance": source / "crates/hiero/Cargo.toml",
    }[broken]
    path.write_text(
        path.read_text().replace("0.8.0", "1.0.0")
        if broken != "inheritance"
        else '[package]\nversion="0.8.0"\n'
    )
    with pytest.raises(ValueError):
        module("check-rust-release").check_source(source, None, allow_untagged=True)


@pytest.fixture
def release(tmp_path):
    archive = "hieronymus-0.8.0-x86_64-unknown-linux-gnu.tar.gz"
    data = b"archive fixture"
    (tmp_path / archive).write_bytes(data)
    metadata = dict(
        version="0.8.0",
        target="x86_64-unknown-linux-gnu",
        channel="stable",
        archive=archive,
        sha256=hashlib.sha256(data).hexdigest(),
        signature=None,
    )
    (tmp_path / "release.json").write_text(json.dumps(metadata))
    return tmp_path, metadata


def test_metadata_binds_exact_archive(release):
    root, metadata = release
    assert (
        module("check-rust-release").check_metadata(root, "0.8.0", "stable") == metadata["archive"]
    )


@pytest.mark.parametrize(
    "key,value",
    [
        ("version", "0.7.0"),
        ("target", "other"),
        ("channel", "dev"),
        ("archive", "../escape.tar.gz"),
        ("sha256", "0" * 64),
        ("signature", "unexpected"),
    ],
)
def test_corrupt_metadata_cannot_publish(release, key, value):
    root, metadata = release
    metadata[key] = value
    (root / "release.json").write_text(json.dumps(metadata))
    with pytest.raises(ValueError):
        module("check-rust-release").check_metadata(root, "0.8.0", "stable")
