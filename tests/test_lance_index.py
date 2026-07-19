from __future__ import annotations

import shutil
from pathlib import Path

import pytest
from test_semantic_index_contract import (
    IDENTITY,
    row,
    run_semantic_index_contract,
)

from hieronymus.lance_index import LanceSemanticIndex
from hieronymus.semantic_index import (
    GenerationNotFoundError,
    IndexManifestError,
    InMemoryGenerationPointer,
    compute_manifest_checksum,
)


def make_index(root: Path) -> LanceSemanticIndex:
    return LanceSemanticIndex(
        root,
        pointer=InMemoryGenerationPointer(),
        max_batch_size=2,
    )


def test_lance_index_satisfies_contract(tmp_path: Path) -> None:
    run_semantic_index_contract(lambda: make_index(tmp_path / "semantic-index"))


def test_activation_rejects_manifest_checksum_mismatch(tmp_path: Path) -> None:
    index = make_index(tmp_path / "semantic-index")
    item = row("alpha", (1.0, 0.0))
    index.begin_rebuild(
        "generation-1",
        identity=IDENTITY,
        expected_count=1,
        manifest_checksum="not-the-real-checksum",
    )
    index.upsert("generation-1", [item])

    with pytest.raises(IndexManifestError, match="checksum"):
        index.activate_generation("generation-1", expected_active_generation=None)


def test_missing_active_generation_directory_is_unhealthy(tmp_path: Path) -> None:
    root = tmp_path / "semantic-index"
    pointer = InMemoryGenerationPointer()
    index = LanceSemanticIndex(root, pointer=pointer, max_batch_size=2)
    item = row("alpha", (1.0, 0.0))
    index.begin_rebuild(
        "generation-1",
        identity=IDENTITY,
        expected_count=1,
        manifest_checksum=compute_manifest_checksum([item]),
    )
    index.upsert("generation-1", [item])
    index.activate_generation("generation-1", expected_active_generation=None)
    shutil.rmtree(next((root / "generations").iterdir()))

    assert not index.health().healthy
    with pytest.raises(GenerationNotFoundError):
        index.search("generation-1", (0.0, 0.0), limit=1)


def test_corrupt_generation_manifest_is_unhealthy(tmp_path: Path) -> None:
    root = tmp_path / "semantic-index"
    index = make_index(root)
    item = row("alpha", (1.0, 0.0))
    index.begin_rebuild(
        "generation-1",
        identity=IDENTITY,
        expected_count=1,
        manifest_checksum=compute_manifest_checksum([item]),
    )
    index.upsert("generation-1", [item])
    index.activate_generation("generation-1", expected_active_generation=None)
    manifest = next((root / "generations").iterdir()) / "manifest.json"
    manifest.write_text("not-json", encoding="utf-8")

    assert not index.health().healthy
    with pytest.raises(IndexManifestError):
        index.search("generation-1", (0.0, 0.0), limit=1)


def test_generation_names_cannot_escape_index_root(tmp_path: Path) -> None:
    index = make_index(tmp_path / "semantic-index")
    with pytest.raises(ValueError, match="generation"):
        index.begin_rebuild(
            "../escape",
            identity=IDENTITY,
            expected_count=0,
            manifest_checksum=compute_manifest_checksum([]),
        )
