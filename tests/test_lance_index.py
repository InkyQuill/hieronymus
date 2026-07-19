from __future__ import annotations

import json
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest
from test_semantic_index_contract import (
    IDENTITY,
    row,
    run_semantic_index_contract,
    run_unusable_active_generation_contract,
)

from hieronymus.lance_index import LanceSemanticIndex
from hieronymus.semantic_index import (
    GenerationConflictError,
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
    run_unusable_active_generation_contract(
        lambda: make_index(root),
        lambda _index: shutil.rmtree(root / "generations" / "generation-1"),
    )


def test_corrupt_generation_manifest_is_unhealthy(tmp_path: Path) -> None:
    root = tmp_path / "semantic-index"
    run_unusable_active_generation_contract(
        lambda: make_index(root),
        lambda _index: (root / "generations" / "generation-1" / "manifest.json").write_text(
            "not-json", encoding="utf-8"
        ),
    )


def test_generation_names_cannot_escape_index_root(tmp_path: Path) -> None:
    index = make_index(tmp_path / "semantic-index")
    with pytest.raises(ValueError, match="generation"):
        index.begin_rebuild(
            "../escape",
            identity=IDENTITY,
            expected_count=0,
            manifest_checksum=compute_manifest_checksum([]),
        )


def test_generation_symlink_cannot_escape_index_root(tmp_path: Path) -> None:
    root = tmp_path / "semantic-index"
    outside = tmp_path / "outside"
    outside.mkdir()
    (root / "generations").mkdir(parents=True)
    (root / "generations" / "generation-1").symlink_to(outside, target_is_directory=True)
    index = make_index(root)

    with pytest.raises(ValueError, match="outside semantic index root"):
        index.cancel_generation("generation-1")

    assert outside.is_dir()


def test_health_detects_active_table_row_tampering(tmp_path: Path) -> None:
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
    table = index._open_table("generation-1")
    table.delete("chunk_id = 'alpha'")

    assert not index.health().healthy


def test_health_detects_manifest_dimension_schema_mismatch(tmp_path: Path) -> None:
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
    manifest_path = root / "generations" / "generation-1" / "manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["identity"]["dimensions"] = 3
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    assert not index.health().healthy


def test_independent_adapters_serialize_activation_and_mutation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = tmp_path / "semantic-index"
    pointer = InMemoryGenerationPointer()
    activating = LanceSemanticIndex(root, pointer=pointer, max_batch_size=2)
    mutating = LanceSemanticIndex(root, pointer=pointer, max_batch_size=2)
    item = row("alpha", (1.0, 0.0))
    activating.begin_rebuild(
        "generation-1",
        identity=IDENTITY,
        expected_count=1,
        manifest_checksum=compute_manifest_checksum([item]),
    )
    activating.upsert("generation-1", [item])
    verified = threading.Event()
    release = threading.Event()
    original = activating._verify_generation

    def blocking_verify(generation: str, manifest):
        result = original(generation, manifest)
        verified.set()
        assert release.wait(2)
        return result

    monkeypatch.setattr(activating, "_verify_generation", blocking_verify)
    activation = threading.Thread(
        target=activating.activate_generation,
        args=("generation-1",),
        kwargs={"expected_active_generation": None},
    )
    mutation_errors: list[BaseException] = []

    def mutate() -> None:
        try:
            mutating.upsert("generation-1", [item])
        except BaseException as error:
            mutation_errors.append(error)

    activation.start()
    assert verified.wait(2)
    mutation = threading.Thread(target=mutate)
    mutation.start()
    time.sleep(0.05)
    assert mutation.is_alive()
    release.set()
    activation.join(2)
    mutation.join(2)

    assert mutation_errors and isinstance(mutation_errors[0], GenerationConflictError)


def test_search_lease_blocks_generation_switch_across_adapters(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = tmp_path / "semantic-index"
    pointer = InMemoryGenerationPointer()
    searching = LanceSemanticIndex(root, pointer=pointer, max_batch_size=2)
    switching = LanceSemanticIndex(root, pointer=pointer, max_batch_size=2)
    first = row("alpha", (1.0, 0.0))
    second = row("beta", (0.0, 1.0), generation="generation-2")
    searching.begin_rebuild(
        "generation-1",
        identity=IDENTITY,
        expected_count=1,
        manifest_checksum=compute_manifest_checksum([first]),
    )
    searching.upsert("generation-1", [first])
    searching.activate_generation("generation-1", expected_active_generation=None)
    switching.begin_rebuild(
        "generation-2",
        identity=IDENTITY,
        expected_count=1,
        manifest_checksum=compute_manifest_checksum([second]),
    )
    switching.upsert("generation-2", [second])
    started = threading.Event()
    release = threading.Event()
    original = searching._open_table

    def blocking_open(generation: str):
        if generation == "generation-1" and threading.current_thread().name == "search":
            started.set()
            assert release.wait(2)
        return original(generation)

    monkeypatch.setattr(searching, "_open_table", blocking_open)
    search = threading.Thread(
        name="search",
        target=searching.search,
        args=("generation-1", (0.0, 0.0)),
        kwargs={"limit": 1},
    )
    switch = threading.Thread(
        target=switching.activate_generation,
        args=("generation-2",),
        kwargs={"expected_active_generation": "generation-1"},
    )
    search.start()
    assert started.wait(2)
    switch.start()
    time.sleep(0.05)
    assert switch.is_alive()
    release.set()
    search.join(2)
    switch.join(2)

    assert pointer.read() == "generation-2"
    switching.cancel_generation("generation-1")


def test_coordination_lease_blocks_independent_process(tmp_path: Path) -> None:
    root = tmp_path / "semantic-index"
    index = make_index(root)
    script = """
from pathlib import Path
from hieronymus.lance_index import LanceSemanticIndex
from hieronymus.semantic_index import InMemoryGenerationPointer, compute_manifest_checksum
from hieronymus.embeddings import EmbeddingIdentity
index = LanceSemanticIndex(
    Path(__import__('sys').argv[1]),
    pointer=InMemoryGenerationPointer(),
    max_batch_size=2,
)
(Path(__import__('sys').argv[1]) / 'child-ready').write_text('ready')
index.begin_rebuild(
    'child',
    identity=EmbeddingIdentity('local', 'model', None, 2),
    expected_count=0,
    manifest_checksum=compute_manifest_checksum([]),
)
"""
    with index._lease(exclusive=False):
        child = subprocess.Popen([sys.executable, "-c", script, str(root)])
        deadline = time.monotonic() + 5
        while not (root / "child-ready").exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        assert (root / "child-ready").exists()
        assert child.poll() is None
    assert child.wait(timeout=5) == 0
