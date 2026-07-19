from __future__ import annotations

import time
from collections.abc import Callable, Sequence
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect, ensure_schema
from hieronymus.embeddings import EmbeddingIdentity
from hieronymus.rag import RagStore
from hieronymus.registry import Registry
from hieronymus.semantic_index import (
    GenerationConflictError,
    IndexHealth,
    IndexRow,
    SearchFilters,
    SearchResult,
    compute_manifest_checksum,
)
from hieronymus.semantic_jobs import (
    SemanticIndexWorker,
    SemanticJobStore,
    SQLiteGenerationPointer,
)

IDENTITY = EmbeddingIdentity("local", "test-model", None, 2)


def _series(config: HieronymusConfig) -> str:
    return (
        Registry(config)
        .create_series(
            slug="oso",
            title="Only Sense Online",
            source_language="ja",
            target_language="ru",
        )
        .slug
    )


def _import_chunks(
    config: HieronymusConfig,
    tmp_path: Path,
    count: int,
    *,
    source_ref: str = "chapter.txt",
) -> None:
    config.data_root.mkdir(parents=True, exist_ok=True)
    config.semantic_config_path.write_text(
        '[semantic]\nmodel = "test-model"\ndimensions = 2\nbatch_size = 2\n',
        encoding="utf-8",
    )
    path = tmp_path / source_ref
    path.write_text("\n\n".join(f"chunk {index}" for index in range(count)), encoding="utf-8")
    RagStore(config).import_file(_series(config), path, source_ref=source_ref)


class FakeProvider:
    def __init__(
        self,
        *,
        identity: EmbeddingIdentity = IDENTITY,
        fail: Exception | None = None,
        after_batch: Callable[[int], None] | None = None,
    ) -> None:
        self.identity = identity
        self.max_batch_size = 256
        self.fail = fail
        self.after_batch = after_batch
        self.batches: list[tuple[str, ...]] = []

    def embed_documents(self, documents: Sequence[str], *, progress=None) -> list[list[float]]:
        del progress
        batch = tuple(documents)
        self.batches.append(batch)
        if self.fail is not None:
            raise self.fail
        if self.after_batch is not None:
            self.after_batch(len(self.batches))
        return [[float(index), 1.0] for index, _ in enumerate(batch)]

    def embed_query(self, query: str, *, progress=None) -> list[float]:
        del query, progress
        return [0.0, 1.0]


class FakeIndex:
    def __init__(self, pointer: SQLiteGenerationPointer) -> None:
        self.pointer = pointer
        self.manifests: dict[str, tuple[int, str]] = {}
        self.rows: dict[str, dict[str, IndexRow]] = {}
        self.upsert_batches: list[tuple[str, ...]] = []
        self.delete_batches: list[tuple[str, ...]] = []
        self.cancelled: list[str] = []

    @property
    def active_generation(self) -> str | None:
        return self.pointer.read()

    def health(self) -> IndexHealth:
        return IndexHealth(True, "ok")

    def begin_rebuild(
        self,
        generation: str,
        *,
        identity: EmbeddingIdentity,
        expected_count: int,
        manifest_checksum: str,
    ) -> None:
        del identity
        if generation in self.manifests:
            raise GenerationConflictError("generation already exists")
        self.manifests[generation] = (expected_count, manifest_checksum)
        self.rows[generation] = {}

    def upsert(self, generation: str, rows: Sequence[IndexRow]) -> None:
        self.upsert_batches.append(tuple(row.chunk_id for row in rows))
        self.rows[generation].update({row.chunk_id: row for row in rows})

    def delete_chunk_ids(self, generation: str, chunk_ids: Sequence[str]) -> None:
        self.delete_batches.append(tuple(chunk_ids))
        for chunk_id in chunk_ids:
            self.rows[generation].pop(chunk_id, None)

    def search(
        self,
        generation: str,
        vector: Sequence[float],
        *,
        limit: int,
        filters: SearchFilters | None = None,
    ) -> list[SearchResult]:
        del generation, vector, limit, filters
        return []

    def activate_generation(
        self, generation: str, *, expected_active_generation: str | None
    ) -> None:
        expected_count, checksum = self.manifests[generation]
        rows = list(self.rows[generation].values())
        assert len(rows) == expected_count
        assert compute_manifest_checksum(rows) == checksum
        if not self.pointer.compare_and_set(expected_active_generation, generation):
            raise GenerationConflictError("active generation changed")

    def cancel_generation(self, generation: str) -> None:
        self.cancelled.append(generation)
        self.manifests.pop(generation, None)
        self.rows.pop(generation, None)

    def close(self) -> None:
        return


class FailingIndex(FakeIndex):
    def upsert(self, generation: str, rows: Sequence[IndexRow]) -> None:
        del generation, rows
        raise RuntimeError("index write failed")


class CancelOnActivateIndex(FakeIndex):
    def __init__(
        self, pointer: SQLiteGenerationPointer, store: SemanticJobStore, job_id: int
    ) -> None:
        super().__init__(pointer)
        self.store = store
        self.job_id = job_id

    def activate_generation(
        self, generation: str, *, expected_active_generation: str | None
    ) -> None:
        self.store.request_cancel(self.job_id)
        super().activate_generation(
            generation,
            expected_active_generation=expected_active_generation,
        )


def test_schema_tracks_pointer_jobs_counts_errors_cancellation_and_checksums(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        ensure_schema(conn)
        state_columns = {
            row["name"] for row in conn.execute("pragma table_info(semantic_index_state)")
        }
        job_columns = {
            row["name"] for row in conn.execute("pragma table_info(semantic_index_jobs)")
        }
        checksum_columns = {
            row["name"] for row in conn.execute("pragma table_info(semantic_indexed_chunks)")
        }

    assert {
        "active_generation",
        "desired_provider",
        "desired_model",
        "desired_revision",
        "desired_dimensions",
        "updated_at",
    } <= state_columns
    assert {
        "status",
        "expected_count",
        "processed_count",
        "error",
        "retryable",
        "cancel_requested",
        "created_at",
        "updated_at",
    } <= job_columns
    assert {"generation", "chunk_id", "checksum"} <= checksum_columns


def test_sqlite_generation_pointer_is_durable_compare_and_set(config: HieronymusConfig) -> None:
    first = SQLiteGenerationPointer(config)
    assert first.read() is None
    assert first.compare_and_set(None, "generation-1") is True
    assert first.compare_and_set(None, "generation-2") is False
    assert SQLiteGenerationPointer(config).read() == "generation-1"


def test_generation_pointer_rejects_cancelled_job_activation(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 1)
    store = SemanticJobStore(config)
    job = store.latest_job()
    assert job is not None
    store.request_cancel(job.id)

    assert SQLiteGenerationPointer(config).compare_and_set(None, job.generation) is False


def test_activation_race_with_new_authority_cancels_ready_generation(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 1)
    store = SemanticJobStore(config)
    job = store.latest_job()
    assert job is not None
    pointer = SQLiteGenerationPointer(config)
    index = CancelOnActivateIndex(pointer, store, job.id)

    SemanticIndexWorker(
        config,
        provider=FakeProvider(),
        index=index,
        batch_size=1,
    ).run_once()

    assert store.job(job.id).status == "cancelled"
    assert pointer.read() is None
    assert index.cancelled == [job.generation]


def test_pending_job_survives_store_restart(config: HieronymusConfig, tmp_path: Path) -> None:
    _import_chunks(config, tmp_path, 1)
    first = SemanticJobStore(config).latest_job()

    assert first is not None
    assert SemanticJobStore(config).job(first.id) == first


def test_worker_uses_bounded_deterministic_id_order_and_activates_verified_generation(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 5)
    pointer = SQLiteGenerationPointer(config)
    index = FakeIndex(pointer)
    provider = FakeProvider()

    assert SemanticIndexWorker(config, provider=provider, index=index, batch_size=2).run_once()

    job = SemanticJobStore(config).latest_job()
    assert job is not None and job.status == "completed"
    assert job.processed_count == job.expected_count == 5
    assert [len(batch) for batch in provider.batches] == [2, 2, 1]
    assert index.upsert_batches == [("1", "2"), ("3", "4"), ("5",)]
    assert pointer.read() == job.generation
    assert SemanticJobStore(config).indexed_checksums(job.generation) == {
        str(index): checksum
        for index, checksum in SemanticJobStore(config).authoritative_checksums().items()
    }


def test_worker_resumes_retryable_job_after_restart(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 3)
    pointer = SQLiteGenerationPointer(config)
    index = FakeIndex(pointer)
    failing = FakeProvider(
        after_batch=lambda count: (
            (_ for _ in ()).throw(RuntimeError("offline")) if count == 2 else None
        )
    )
    first_worker = SemanticIndexWorker(config, provider=failing, index=index, batch_size=1)

    assert first_worker.run_once()
    failed = SemanticJobStore(config).latest_job()
    assert failed is not None and failed.status == "failed" and failed.retryable
    SemanticJobStore(config).retry(failed.id)

    assert SemanticIndexWorker(
        config, provider=FakeProvider(), index=index, batch_size=1
    ).run_once()
    resumed = SemanticJobStore(config).job(failed.id)
    assert resumed.status == "completed"
    assert resumed.processed_count == resumed.expected_count == 3


def test_worker_reclaims_running_job_after_process_restart(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 2)
    store = SemanticJobStore(config)
    interrupted = store.claim_next()
    assert interrupted is not None and interrupted.status == "running"
    pointer = SQLiteGenerationPointer(config)

    restarted = SemanticIndexWorker(
        config,
        provider=FakeProvider(),
        index=FakeIndex(pointer),
        batch_size=1,
    )

    assert restarted.run_once()
    assert store.job(interrupted.id).status == "completed"


def test_worker_recovers_crash_between_index_creation_and_state_checkpoint(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 1)
    store = SemanticJobStore(config)
    job = store.latest_job()
    assert job is not None and job.index_started is False
    pointer = SQLiteGenerationPointer(config)
    index = FakeIndex(pointer)
    index.begin_rebuild(
        job.generation,
        identity=job.identity,
        expected_count=1,
        manifest_checksum=job.manifest_checksum,
    )

    worker = SemanticIndexWorker(
        config,
        provider=FakeProvider(),
        index=index,
        batch_size=1,
    )

    assert worker.run_once()
    assert store.job(job.id).status == "completed"
    assert index.cancelled == [job.generation]


def test_worker_finishes_pending_cancellation_after_restart(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 1)
    store = SemanticJobStore(config)
    job = store.claim_next()
    assert job is not None
    pointer = SQLiteGenerationPointer(config)
    index = FakeIndex(pointer)
    index.begin_rebuild(
        job.generation,
        identity=job.identity,
        expected_count=1,
        manifest_checksum=job.manifest_checksum,
    )
    store.mark_index_started(job.id)
    store.request_cancel(job.id)

    restarted = SemanticIndexWorker(
        config,
        provider=FakeProvider(),
        index=index,
        batch_size=1,
    )

    assert restarted.run_once()
    assert store.job(job.id).status == "cancelled"
    assert index.cancelled == [job.generation]


def test_worker_honors_cancellation_between_batches(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 3)
    store = SemanticJobStore(config)
    job = store.latest_job()
    assert job is not None
    pointer = SQLiteGenerationPointer(config)
    index = FakeIndex(pointer)
    provider = FakeProvider(
        after_batch=lambda count: store.request_cancel(job.id) if count == 1 else None
    )

    SemanticIndexWorker(config, provider=provider, index=index, batch_size=1).run_once()

    cancelled = store.job(job.id)
    assert cancelled.status == "cancelled"
    assert len(provider.batches) == 1
    assert index.cancelled == [job.generation]


def test_model_identity_change_supersedes_unfinished_generation(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 1)
    old = SemanticJobStore(config).latest_job()
    assert old is not None
    config.semantic_config_path.write_text(
        '[semantic]\nmodel = "replacement-model"\ndimensions = 2\nbatch_size = 2\n',
        encoding="utf-8",
    )
    second_path = tmp_path / "second.txt"
    second_path.write_text("second chunk", encoding="utf-8")
    RagStore(config).import_file("oso", second_path, source_ref="second.txt")

    store = SemanticJobStore(config)
    new = store.latest_job()
    assert new is not None and new.generation != old.generation
    assert new.identity.model == "replacement-model"
    assert store.job(old.id).cancel_requested is True

    pointer = SQLiteGenerationPointer(config)
    index = FakeIndex(pointer)
    worker = SemanticIndexWorker(
        config,
        provider=FakeProvider(),
        index=index,
        batch_size=2,
        provider_factory=lambda job: FakeProvider(identity=job.identity),
    )
    assert worker.run_once()
    assert store.job(new.id).status == "completed"


def test_stale_checksum_is_excluded_before_index_write(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 1)
    pointer = SQLiteGenerationPointer(config)
    index = FakeIndex(pointer)

    def change_authority(_: int) -> None:
        with connect(config.database_path) as conn:
            conn.execute("update rag_chunks set text = 'changed after embedding' where id = 1")
            conn.commit()

    SemanticIndexWorker(
        config,
        provider=FakeProvider(after_batch=change_authority),
        index=index,
        batch_size=1,
    ).run_once()

    job = SemanticJobStore(config).latest_job()
    assert job is not None and job.status == "failed" and job.retryable
    assert index.upsert_batches == []
    assert "authoritative chunk changed" in job.error


def test_failure_is_redacted_and_does_not_break_lexical_rag(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    secret = "sk-private-value"
    config.provider_config_path.parent.mkdir(parents=True, exist_ok=True)
    config.provider_config_path.write_text(
        f'[openai]\nname = "OpenAI"\ntype = "openai"\n'
        f'url = "https://example.test"\nkey = "{secret}"\n',
        encoding="utf-8",
    )
    _import_chunks(config, tmp_path, 1)
    pointer = SQLiteGenerationPointer(config)
    worker = SemanticIndexWorker(
        config,
        provider=FakeProvider(fail=RuntimeError(f"provider rejected {secret}")),
        index=FakeIndex(pointer),
        batch_size=1,
    )

    assert worker.run_once()

    job = SemanticJobStore(config).latest_job()
    assert job is not None and job.status == "failed" and job.retryable
    assert secret not in job.error
    assert "[redacted]" in job.error
    assert [hit.chunk.text for hit in RagStore(config).search("oso", "chunk", limit=1)] == [
        "chunk 0"
    ]


def test_index_failure_is_retryable_after_authoritative_import(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    _import_chunks(config, tmp_path, 1)
    pointer = SQLiteGenerationPointer(config)

    assert SemanticIndexWorker(
        config,
        provider=FakeProvider(),
        index=FailingIndex(pointer),
        batch_size=1,
    ).run_once()

    job = SemanticJobStore(config).latest_job()
    assert job is not None and job.status == "failed" and job.retryable
    assert [hit.chunk.text for hit in RagStore(config).search("oso", "chunk", limit=1)] == [
        "chunk 0"
    ]


def test_worker_wakes_for_import_without_waiting_for_fallback_poll(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    pointer = SQLiteGenerationPointer(config)
    worker = SemanticIndexWorker(
        config,
        provider=FakeProvider(),
        index=FakeIndex(pointer),
        batch_size=1,
        poll_interval=30.0,
    )
    worker.start()
    try:
        _import_chunks(config, tmp_path, 1)
        deadline = time.monotonic() + 2
        job = None
        while time.monotonic() < deadline:
            job = SemanticJobStore(config).latest_job()
            if job is not None and job.status == "completed":
                break
            time.sleep(0.01)
        assert job is not None and job.status == "completed"
    finally:
        assert worker.stop(timeout=1)
