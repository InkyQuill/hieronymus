"""Fail-closed release asset assembly; fake bytes here earn no native qualification."""

import hashlib
import importlib.util
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture
def staging(tmp_path, monkeypatch):
    spec = importlib.util.spec_from_file_location("stage", ROOT / "scripts/stage-release-assets.py")
    stage = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(stage)
    model = tmp_path / stage.MODEL_REL
    runtime = tmp_path / stage.RUNTIME_REL
    model.mkdir(parents=True)
    runtime.mkdir(parents=True)
    fixture = tmp_path / "crates/hieronymus/tests/fixtures"
    fixture.mkdir(parents=True)
    for name, data in {
        "model.onnx": b"model",
        "tokenizer.json": b"tokenizer",
        "LICENSE": b"license",
        "README.md": b"card",
    }.items():
        monkeypatch.setitem(stage.MODEL_PINS, name, hashlib.sha256(data).hexdigest())
        if name == "model.onnx":
            (model / name).write_bytes(data)
        elif name != "README.md":
            destination = (
                "multilingual-minilm-tokenizer.json"
                if name == "tokenizer.json"
                else "multilingual-minilm-tokenizer.LICENSE"
            )
            (fixture / destination).write_bytes(data)
    library = runtime / "lib/libonnxruntime.so"
    library.parent.mkdir()
    library.write_bytes(b"runtime")
    monkeypatch.setattr(stage, "RUNTIME_SHA", hashlib.sha256(b"runtime").hexdigest())
    for name in ("LICENSE", "ThirdPartyNotices.txt", "VERSION_NUMBER"):
        (runtime / name).write_text("1.28.0" if name == "VERSION_NUMBER" else "notice")
    monkeypatch.setattr(stage, "acquire", lambda root: None)
    monkeypatch.setattr(stage, "download_card", lambda path: path.write_bytes(b"card"))
    return stage, tmp_path, model, runtime


def test_fresh_staging_assembles_verified_companions(staging):
    stage, root, model, runtime = staging
    result = stage.stage(root)
    assert result["HIERO_RELEASE_MODEL_DIR"] == str(model)
    assert result["HIERO_RELEASE_ONNX_RUNTIME"] == str(runtime / "lib/libonnxruntime.so")
    assert (model / "README.md").read_bytes() == b"card"
    assert not (model / "README.md.part").exists()
    assert stage.stage(root) == result


@pytest.mark.parametrize(
    "asset", ["model.onnx", "tokenizer.json", "LICENSE", "README.md", "runtime"]
)
def test_mutated_cache_refused(staging, asset):
    stage, root, model, runtime = staging
    stage.stage(root)
    path = runtime / "lib/libonnxruntime.so" if asset == "runtime" else model / asset
    path.write_bytes(b"corrupt")
    with pytest.raises(ValueError):
        stage.stage(root)


@pytest.mark.parametrize("notice", ["LICENSE", "ThirdPartyNotices.txt", "VERSION_NUMBER"])
def test_runtime_notices_are_required(staging, notice):
    stage, root, model, runtime = staging
    (runtime / notice).unlink()
    with pytest.raises((ValueError, OSError)):
        stage.stage(root)


@pytest.mark.parametrize("failure", ["transfer", "oversized", "wrong-hash"])
def test_failed_card_never_promoted(staging, monkeypatch, failure):
    stage, root, model, runtime = staging

    def download(path):
        path.write_bytes(b"x" * 65537 if failure == "oversized" else b"wrong")
        if failure == "transfer":
            raise OSError("transfer failed")

    monkeypatch.setattr(stage, "download_card", download)
    with pytest.raises((ValueError, OSError)):
        stage.stage(root)
    assert not (model / "README.md").exists()
    assert not (model / "README.md.part").exists()
