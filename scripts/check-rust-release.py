#!/usr/bin/env python3
"""Bind Rust release metadata to source, an exact tag, and actual archive bytes."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import tomllib
from pathlib import Path

TARGET = "x86_64-unknown-linux-gnu"


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_source(root: Path, ref: str | None, *, allow_untagged: bool = False) -> str:
    version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if not isinstance(version, str) or not re.fullmatch(r"0\.(0|[1-9]\d*)\.(0|[1-9]\d*)", version):
        raise ValueError("Rust release requires an explicit alpha 0.x.y version (ADR 0006)")
    packages = tomllib.loads((root / "Cargo.lock").read_text())["package"]
    for name in ("hiero", "hieronymus"):
        versions = [item["version"] for item in packages if item["name"] == name]
        if versions != [version]:
            raise ValueError(f"{name} Cargo.lock version mismatch")
        crate = tomllib.loads((root / "crates" / name / "Cargo.toml").read_text())
        if crate["package"]["version"] != {"workspace": True}:
            raise ValueError(f"{name} must inherit the workspace version")
    if ref is None and allow_untagged:
        return version
    if ref != f"refs/tags/v{version}":
        raise ValueError("release ref must be the exact workspace-version tag")
    try:
        head = subprocess.check_output(
            ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
        ).strip()
        tagged = subprocess.check_output(
            ["git", "-C", str(root), "rev-parse", f"{ref}^{{commit}}"], text=True
        ).strip()
    except subprocess.CalledProcessError as error:
        raise ValueError("release tag does not resolve") from error
    if head != tagged:
        raise ValueError("release tag does not point to checked-out HEAD")
    return version


def check_metadata(directory: Path, version: str, channel: str) -> str:
    if channel not in ("stable", "dev"):
        raise ValueError("unsupported release channel")
    metadata = json.loads((directory / "release.json").read_text())
    archive = f"hieronymus-{version}-{TARGET}.tar.gz"
    if (
        any(
            metadata.get(key) != value
            for key, value in {
                "version": version,
                "target": TARGET,
                "channel": channel,
                "archive": archive,
            }.items()
        )
        or "signature" not in metadata
        or metadata["signature"] is not None
    ):
        raise ValueError("release metadata does not match source/target/channel/waiver")
    if metadata.get("sha256") != digest(directory / archive):
        raise ValueError("release archive checksum mismatch")
    return archive


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--ref")
    parser.add_argument(
        "--allow-untagged", action="store_true", help="local candidate only, never publishing"
    )
    parser.add_argument("--release-dir", type=Path)
    parser.add_argument("--channel", default="stable")
    args = parser.parse_args()
    try:
        version = check_source(args.root, args.ref, allow_untagged=args.allow_untagged)
        archive = (
            check_metadata(args.release_dir, version, args.channel) if args.release_dir else None
        )
    except (ValueError, KeyError, OSError) as error:
        parser.exit(1, f"Rust release refused: {error}\n")
    print(
        json.dumps({"version": version, "archive": archive, "tag_verified": args.ref is not None})
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
