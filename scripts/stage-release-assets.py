#!/usr/bin/env python3
"""Acquire pinned native release assets and fail closed before exporting build inputs."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

MODEL_REL = Path("qualification/.artifacts/models/paraphrase-multilingual-MiniLM-L12-v2")
RUNTIME_REL = Path("qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0")
MODEL_PINS = {
    "model.onnx": "10f7a088420252b26caf819236ca2c9d2987afd0fc06fec7553b542a5655a05a",
    "tokenizer.json": "2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8",
    "LICENSE": "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
    "README.md": "1e98ea05b0de579fcaad3d625b62ea55647142ed674d5f5ebf1440e4bbbb6f23",
}
RUNTIME_SHA = "1461ef7cc3d9e49982591721683cc3e3a55580aeca9a5254e7aac47b75ee4bab"
CARD_URL = "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/resolve/e8f8c211226b894fcb81acc59f3b34ba3efd5f42/README.md"


def verify(path: Path, expected: str) -> None:
    if not path.is_file():
        raise ValueError(f"missing release asset: {path}")
    with path.open("rb") as stream:
        actual = hashlib.file_digest(stream, "sha256").hexdigest()
    if actual != expected:
        raise ValueError(f"release asset checksum mismatch: {path}")


def acquire(root: Path) -> None:
    # Existing downloader verifies allowlisted redirects, archive extraction and
    # complete runtime provenance. A cache never substitutes for those checks.
    for artifact in ("onnx-runtime", "semantic-model"):
        subprocess.run(
            [sys.executable, "-m", "tools.qualification.acquire", artifact], cwd=root, check=True
        )


def download_card(destination: Path) -> None:
    subprocess.run(
        [
            "curl",
            "--fail",
            "--show-error",
            "--silent",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-redirs",
            "5",
            "--connect-timeout",
            "30",
            "--max-time",
            "120",
            "--max-filesize",
            "65536",
            CARD_URL,
            "--output",
            str(destination),
        ],
        check=True,
    )


def stage(root: Path) -> dict[str, str]:
    acquire(root)
    model = root / MODEL_REL
    runtime = root / RUNTIME_REL
    verify(model / "model.onnx", MODEL_PINS["model.onnx"])
    library = runtime / "lib/libonnxruntime.so"
    verify(library, RUNTIME_SHA)
    for notice in ("LICENSE", "ThirdPartyNotices.txt", "VERSION_NUMBER"):
        path = runtime / notice
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"missing regular runtime notice: {notice}")
    if (runtime / "VERSION_NUMBER").read_text().strip() != "1.28.0":
        raise ValueError("runtime version mismatch")
    fixtures = root / "crates/hieronymus/tests/fixtures"
    for asset, source in {
        "tokenizer.json": "multilingual-minilm-tokenizer.json",
        "LICENSE": "multilingual-minilm-tokenizer.LICENSE",
    }.items():
        verify(fixtures / source, MODEL_PINS[asset])
        destination = model / asset
        if destination.exists():
            verify(destination, MODEL_PINS[asset])
        else:
            part = model / f"{asset}.part"
            try:
                shutil.copyfile(fixtures / source, part)
                verify(part, MODEL_PINS[asset])
                os.replace(part, destination)
            finally:
                part.unlink(missing_ok=True)
    card = model / "README.md"
    if not card.exists():
        part = model / "README.md.part"
        try:
            download_card(part)
            if part.stat().st_size > 65536:
                raise ValueError("model card exceeds transfer bound")
            verify(part, MODEL_PINS["README.md"])
            os.replace(part, card)
        finally:
            part.unlink(missing_ok=True)
    verify(card, MODEL_PINS["README.md"])
    return {
        "HIERO_RELEASE_ONNX_RUNTIME": str(library),
        "HIERO_RELEASE_ONNX_SHA256": RUNTIME_SHA,
        "HIERO_RELEASE_MODEL_DIR": str(model),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--github-env", type=Path)
    args = parser.parse_args()
    try:
        values = stage(args.root.resolve())
        if args.github_env:
            if any("\n" in value or "\r" in value for value in values.values()):
                raise ValueError("invalid environment path")
            with args.github_env.open("a") as stream:
                stream.writelines(f"{key}={value}\n" for key, value in values.items())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Release asset staging refused: {error}\n")
    print(json.dumps(values))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
