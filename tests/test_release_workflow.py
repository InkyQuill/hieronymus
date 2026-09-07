"""Rust cutover workflow contract; behavioral metadata/asset checks live beside it."""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/release-rust.yml"


def test_only_rust_publisher_remains():
    assert not (ROOT / ".github/workflows/release.yml").exists()
    workflows = list((ROOT / ".github/workflows").glob("*.yml"))
    assert all("semantic-release publish" not in path.read_text() for path in workflows)
    assert 'tags: ["v*"]' in WORKFLOW.read_text()


def test_guards_precede_assets_build_and_publish():
    text = WORKFLOW.read_text()
    guard = text.index("scripts/check-rust-release.py --ref")
    stage = text.index("scripts/stage-release-assets.py")
    build = text.index("./scripts/release-build.sh")
    metadata = text.index("--release-dir target/release-dist")
    publish = text.index("gh release create")
    assert guard < stage < build < metadata < publish
    assert "--allow-untagged" not in text
    assert "RELEASE_REF: ${{ github.ref }}" in text
    assert '--ref "$RELEASE_REF"' in text


def test_assets_and_metadata_are_explicit():
    text = WORKFLOW.read_text()
    assert '--github-env "$GITHUB_ENV"' in text
    assert "HIERONYMUS_RELEASE_CHANNEL: stable" in text
    assert '"$archive" "$checksums" "target/release-dist/release.json"' in text
    assert "ls target/release-dist" not in text
    assert "FTS5-only" not in text
    assert "mandatory" in text


def test_pins_and_declared_release_control_preserved():
    text = WORKFLOW.read_text()
    assert 'bun-version: "1.4.0"' in text
    assert "rustup toolchain install 1.96.0" in text
    assert 'python-version: "3.12"' in text
    assert "environment: release" in text
    assert "contents: write" in text
    assert "persist-credentials: false" in text
    assert "fetch-depth: 0" in text
    for action in re.findall(r"uses: (\S+)", text):
        assert re.fullmatch(r"[^@]+@[0-9a-f]{40}", action)
