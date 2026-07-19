from __future__ import annotations

import hashlib
import json
import logging
import sqlite3
import threading
import uuid
from collections.abc import Callable, Sequence
from dataclasses import dataclass

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect, ensure_schema
from hieronymus.embeddings import EmbeddingIdentity, EmbeddingProvider
from hieronymus.provider_config import load_provider_catalog
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.semantic_index import (
    GenerationConflictError,
    IndexRow,
    SemanticIndex,
)
from hieronymus.values import utc_now

LOGGER = logging.getLogger(__name__)
DEFAULT_POLL_INTERVAL = 30.0


class AuthoritativeChunkChanged(RuntimeError):
    """Raised when SQLite changed after a batch was selected for embedding."""


@dataclass(frozen=True)
class SemanticJob:
    id: int
    generation: str
    identity: EmbeddingIdentity
    previous_generation: str | None
    manifest_checksum: str
    status: str
    expected_count: int
    processed_count: int
    error: str
    retryable: bool
    cancel_requested: bool
    index_started: bool
    created_at: str
    updated_at: str


@dataclass(frozen=True)
class _JobItem:
    chunk_id: int
    operation: str
    checksum: str | None


@dataclass(frozen=True)
class _AuthoritativeChunk:
    chunk_id: int
    text: str
    series_slug: str
    source_language: str
    target_language: str
    story_scope: str
    semantic_scope: str
    checksum: str

    def index_row(
        self,
        generation: str,
        identity: EmbeddingIdentity,
        vector: Sequence[float],
    ) -> IndexRow:
        return IndexRow(
            chunk_id=str(self.chunk_id),
            generation=generation,
            series_slug=self.series_slug,
            source_language=self.source_language,
            target_language=self.target_language,
            story_scope=self.story_scope,
            semantic_scope=self.semantic_scope,
            checksum=self.checksum,
            identity=identity,
            vector=tuple(vector),
        )


class SQLiteGenerationPointer:
    """SQLite-backed compare-and-set pointer shared by jobs and LanceDB."""

    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(config.database_path) as conn:
            ensure_schema(conn)
            _ensure_state_row(conn)
            conn.commit()

    def read(self) -> str | None:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select active_generation from semantic_index_state where singleton = 1"
            ).fetchone()
        return None if row is None else row["active_generation"]

    def compare_and_set(self, expected: str | None, generation: str) -> bool:
        if not generation.strip():
            raise ValueError("generation must not be blank")
        with connect(self.config.database_path) as conn:
            conn.execute("begin immediate")
            _ensure_state_row(conn)
            current = conn.execute(
                "select active_generation from semantic_index_state where singleton = 1"
            ).fetchone()["active_generation"]
            if current != expected:
                conn.rollback()
                return False
            job = conn.execute(
                """
                select status, cancel_requested, expected_count, processed_count
                from semantic_index_jobs where generation = ?
                """,
                (generation,),
            ).fetchone()
            if job is not None and (
                job["status"] != "running"
                or bool(job["cancel_requested"])
                or int(job["processed_count"]) != int(job["expected_count"])
            ):
                conn.rollback()
                return False
            conn.execute(
                """
                update semantic_index_state
                set active_generation = ?, updated_at = ?
                where singleton = 1
                """,
                (generation, utc_now()),
            )
            conn.commit()
            return True


class SemanticJobStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(config.database_path) as conn:
            ensure_schema(conn)
            _ensure_state_row(conn)
            conn.commit()

    def enqueue_change(
        self,
        conn: sqlite3.Connection,
        *,
        identity: EmbeddingIdentity,
        obsolete_chunk_ids: Sequence[int] = (),
    ) -> SemanticJob:
        now = utc_now()
        conn.execute(
            """
            update semantic_index_jobs
            set cancel_requested = 1,
                status = case when status = 'pending' then 'cancelled' else status end,
                updated_at = ?
            where status in ('pending', 'running')
            """,
            (now,),
        )
        _ensure_state_row(conn)
        conn.execute(
            """
            update semantic_index_state
            set desired_provider = ?, desired_model = ?, desired_revision = ?,
                desired_dimensions = ?, updated_at = ?
            where singleton = 1
            """,
            (identity.provider, identity.model, identity.revision, identity.dimensions, now),
        )
        chunks = _authoritative_chunks(conn)
        generation = f"generation-{uuid.uuid4().hex}"
        manifest_checksum = _manifest_checksum(chunks, generation, identity)
        previous = conn.execute(
            "select active_generation from semantic_index_state where singleton = 1"
        ).fetchone()["active_generation"]
        expected_count = len(chunks) + len(set(obsolete_chunk_ids))
        cursor = conn.execute(
            """
            insert into semantic_index_jobs(
              generation, provider, model, revision, dimensions, previous_generation,
              manifest_checksum, status, expected_count, processed_count, error,
              retryable, cancel_requested, index_started, created_at, updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, 'pending', ?, 0, '', 0, 0, 0, ?, ?)
            """,
            (
                generation,
                identity.provider,
                identity.model,
                identity.revision,
                identity.dimensions,
                previous,
                manifest_checksum,
                expected_count,
                now,
                now,
            ),
        )
        job_id = int(cursor.lastrowid)
        conn.executemany(
            """
            insert into semantic_index_job_items(job_id, chunk_id, operation, checksum)
            values (?, ?, 'upsert', ?)
            """,
            [(job_id, chunk.chunk_id, chunk.checksum) for chunk in chunks],
        )
        conn.executemany(
            """
            insert or ignore into semantic_index_job_items(job_id, chunk_id, operation, checksum)
            values (?, ?, 'delete', null)
            """,
            [(job_id, chunk_id) for chunk_id in sorted(set(obsolete_chunk_ids))],
        )
        return self.job(job_id, conn=conn)

    def latest_job(self) -> SemanticJob | None:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select * from semantic_index_jobs order by id desc limit 1"
            ).fetchone()
        return None if row is None else _job_from_row(row)

    def job(self, job_id: int, *, conn: sqlite3.Connection | None = None) -> SemanticJob:
        if conn is not None:
            row = conn.execute(
                "select * from semantic_index_jobs where id = ?", (job_id,)
            ).fetchone()
        else:
            with connect(self.config.database_path) as owned:
                row = owned.execute(
                    "select * from semantic_index_jobs where id = ?", (job_id,)
                ).fetchone()
        if row is None:
            raise KeyError(job_id)
        return _job_from_row(row)

    def claim_next(self) -> SemanticJob | None:
        with connect(self.config.database_path) as conn:
            conn.execute("begin immediate")
            row = conn.execute(
                """
                select * from semantic_index_jobs
                where status = 'pending'
                order by id limit 1
                """
            ).fetchone()
            if row is None:
                conn.rollback()
                return None
            now = utc_now()
            conn.execute(
                "update semantic_index_jobs set status = 'running', updated_at = ? where id = ?",
                (now, int(row["id"])),
            )
            conn.commit()
        return self.job(int(row["id"]))

    def recover_interrupted(self) -> None:
        with connect(self.config.database_path) as conn:
            conn.execute(
                """
                update semantic_index_jobs
                set status = 'pending', retryable = 1,
                    error = 'worker interrupted; resuming', updated_at = ?
                where status = 'running'
                """,
                (utc_now(),),
            )
            conn.commit()

    def pending_items(self, job_id: int, *, limit: int) -> list[_JobItem]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select chunk_id, operation, checksum
                from semantic_index_job_items
                where job_id = ? and processed = 0
                order by operation, chunk_id
                limit ?
                """,
                (job_id, limit),
            ).fetchall()
        return [_JobItem(int(row["chunk_id"]), row["operation"], row["checksum"]) for row in rows]

    def manifest_items_after(
        self,
        job_id: int,
        *,
        after_chunk_id: int,
        limit: int,
    ) -> list[_JobItem]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select chunk_id, operation, checksum
                from semantic_index_job_items
                where job_id = ? and operation = 'upsert' and chunk_id > ?
                order by chunk_id
                limit ?
                """,
                (job_id, after_chunk_id, limit),
            ).fetchall()
        return [_JobItem(int(row["chunk_id"]), row["operation"], row["checksum"]) for row in rows]

    def mark_index_started(self, job_id: int) -> None:
        self._update(job_id, "index_started = 1")

    def mark_processed(
        self,
        job: SemanticJob,
        items: Sequence[_JobItem],
        indexed: Sequence[_AuthoritativeChunk] = (),
    ) -> None:
        if not items:
            return
        now = utc_now()
        with connect(self.config.database_path) as conn:
            conn.execute("begin immediate")
            conn.executemany(
                """
                update semantic_index_job_items set processed = 1
                where job_id = ? and operation = ? and chunk_id = ?
                """,
                [(job.id, item.operation, item.chunk_id) for item in items],
            )
            conn.executemany(
                """
                insert into semantic_indexed_chunks(generation, chunk_id, checksum)
                values (?, ?, ?)
                on conflict(generation, chunk_id) do update set checksum = excluded.checksum
                """,
                [(job.generation, chunk.chunk_id, chunk.checksum) for chunk in indexed],
            )
            conn.execute(
                """
                update semantic_index_jobs
                set processed_count = (
                  select count(*) from semantic_index_job_items
                  where job_id = ? and processed = 1
                ), updated_at = ?
                where id = ?
                """,
                (job.id, now, job.id),
            )
            conn.commit()

    def finish(self, job_id: int) -> None:
        self._update(job_id, "status = 'completed', error = '', retryable = 0")

    def fail(self, job_id: int, error: str) -> None:
        self._update(job_id, "status = 'failed', error = ?, retryable = 1", error)

    def cancel(self, job_id: int) -> None:
        self._update(
            job_id,
            "status = 'cancelled', cancel_requested = 1, error = '', retryable = 0",
        )

    def request_cancel(self, job_id: int) -> None:
        with connect(self.config.database_path) as conn:
            conn.execute(
                """
                update semantic_index_jobs
                set cancel_requested = 1,
                    status = case when status = 'pending' then 'cancelled' else status end,
                    updated_at = ?
                where id = ? and status in ('pending', 'running', 'failed')
                """,
                (utc_now(), job_id),
            )
            conn.commit()

    def retry(self, job_id: int) -> None:
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                update semantic_index_jobs
                set status = 'pending', error = '', retryable = 0,
                    cancel_requested = 0, updated_at = ?
                where id = ? and status = 'failed'
                """,
                (utc_now(), job_id),
            )
            conn.commit()
        if cursor.rowcount != 1:
            raise ValueError("only failed semantic jobs can be retried")

    def authoritative_chunks(
        self, chunk_ids: Sequence[int] | None = None
    ) -> list[_AuthoritativeChunk]:
        with connect(self.config.database_path) as conn:
            return _authoritative_chunks(conn, chunk_ids)

    def authoritative_checksums(self) -> dict[int, str]:
        return {chunk.chunk_id: chunk.checksum for chunk in self.authoritative_chunks()}

    def indexed_checksums(self, generation: str) -> dict[str, str]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select chunk_id, checksum from semantic_indexed_chunks
                where generation = ? order by chunk_id
                """,
                (generation,),
            ).fetchall()
        return {str(row["chunk_id"]): row["checksum"] for row in rows}

    def job_operations(self, job_id: int) -> list[tuple[str, str]]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select chunk_id, operation from semantic_index_job_items
                where job_id = ? order by operation, chunk_id
                """,
                (job_id,),
            ).fetchall()
        return [(str(row["chunk_id"]), row["operation"]) for row in rows]

    def _update(self, job_id: int, assignment: str, *params: object) -> None:
        with connect(self.config.database_path) as conn:
            conn.execute(
                f"update semantic_index_jobs set {assignment}, updated_at = ? where id = ?",
                (*params, utc_now(), job_id),
            )
            conn.commit()


class SemanticIndexWorker:
    def __init__(
        self,
        config: HieronymusConfig,
        *,
        provider: EmbeddingProvider | None = None,
        index: SemanticIndex | None = None,
        batch_size: int = 32,
        poll_interval: float = DEFAULT_POLL_INTERVAL,
        provider_factory: Callable[[SemanticJob], EmbeddingProvider | None] | None = None,
        index_factory: Callable[[SemanticJob], SemanticIndex] | None = None,
    ) -> None:
        if batch_size < 1:
            raise ValueError("batch_size must be positive")
        self.config = config
        self.store = SemanticJobStore(config)
        self.provider = provider
        self.index = index
        self.batch_size = batch_size
        self.poll_interval = poll_interval
        self._provider_factory = provider_factory
        self._index_factory = index_factory
        self._stop = threading.Event()
        self._wake = threading.Event()
        self._thread = threading.Thread(
            target=self._run,
            name="hieronymus-semantic-index",
            daemon=True,
        )
        self._recovered = False

    def start(self) -> None:
        if self._thread.ident is not None:
            return
        _register_worker(self.config, self._wake)
        self._thread.start()
        self.wake()

    def wake(self) -> None:
        self._wake.set()

    def stop(self, *, timeout: float | None = None) -> bool:
        self._stop.set()
        self._wake.set()
        if self._thread.ident is None or threading.current_thread() is self._thread:
            if self.index is not None:
                self.index.close()
            return True
        self._thread.join(timeout=None if timeout is None else max(0.0, timeout))
        stopped = not self._thread.is_alive()
        if stopped:
            _unregister_worker(self.config, self._wake)
            if self.index is not None:
                self.index.close()
        else:
            LOGGER.error("Semantic index worker did not stop within %.3f seconds", timeout or 0.0)
        return stopped

    def is_alive(self) -> bool:
        return self._thread.is_alive()

    def run_once(self) -> bool:
        if not self._recovered:
            self.store.recover_interrupted()
            self._recovered = True
        job = self.store.claim_next()
        if job is None:
            return False
        try:
            provider = self._provider(job)
            index = self._index(job)
            self._process(job, provider, index)
        except Exception as error:
            redacted = self._redacted_error(error)
            LOGGER.error("Semantic index job %d failed: %s", job.id, redacted)
            self.store.fail(job.id, redacted)
        return True

    def _run(self) -> None:
        while not self._stop.is_set():
            self._wake.wait(self.poll_interval)
            self._wake.clear()
            while not self._stop.is_set() and self.run_once():
                pass

    def _provider(self, job: SemanticJob) -> EmbeddingProvider:
        provider = self.provider
        if provider is None or provider.identity != job.identity:
            provider = self._provider_factory(job) if self._provider_factory is not None else None
            self.provider = provider
        if provider is None:
            raise RuntimeError("semantic embedding provider is unavailable")
        if provider.identity != job.identity:
            raise RuntimeError("semantic embedding identity changed before job execution")
        return provider

    def _index(self, job: SemanticJob) -> SemanticIndex:
        if self.index is None:
            if self._index_factory is None:
                raise RuntimeError("semantic index is unavailable")
            self.index = self._index_factory(job)
        return self.index

    def _process(self, job: SemanticJob, provider: EmbeddingProvider, index: SemanticIndex) -> None:
        if self._cancel_if_requested(job, index):
            return
        if not job.index_started:
            expected_count = self._verify_manifest(job)
            try:
                index.begin_rebuild(
                    job.generation,
                    identity=job.identity,
                    expected_count=expected_count,
                    manifest_checksum=job.manifest_checksum,
                )
            except GenerationConflictError:
                if job.processed_count != 0 or index.active_generation == job.generation:
                    raise
                index.cancel_generation(job.generation)
                index.begin_rebuild(
                    job.generation,
                    identity=job.identity,
                    expected_count=expected_count,
                    manifest_checksum=job.manifest_checksum,
                )
            self.store.mark_index_started(job.id)

        while True:
            current = self.store.job(job.id)
            if self._stop.is_set() or self._cancel_if_requested(current, index):
                return
            items = self.store.pending_items(
                job.id, limit=min(self.batch_size, provider.max_batch_size)
            )
            if not items:
                break
            operation = items[0].operation
            batch = [item for item in items if item.operation == operation]
            if operation == "delete":
                index.delete_chunk_ids(job.generation, [str(item.chunk_id) for item in batch])
                self.store.mark_processed(job, batch)
                continue
            selected = self._selected_authority(batch)
            vectors = provider.embed_documents([chunk.text for chunk in selected])
            if self.store.job(job.id).cancel_requested:
                self._cancel_generation(job, index)
                return
            current_chunks = self._selected_authority(batch)
            if [chunk.checksum for chunk in current_chunks] != [
                chunk.checksum for chunk in selected
            ]:
                raise AuthoritativeChunkChanged("authoritative chunk changed after embedding")
            rows = [
                chunk.index_row(job.generation, job.identity, vector)
                for chunk, vector in zip(current_chunks, vectors, strict=True)
            ]
            index.upsert(job.generation, rows)
            self.store.mark_processed(job, batch, current_chunks)

        latest = self.store.job(job.id)
        if latest.processed_count != latest.expected_count:
            raise RuntimeError("semantic job item count changed during processing")
        if index.active_generation != job.generation:
            try:
                index.activate_generation(
                    job.generation,
                    expected_active_generation=job.previous_generation,
                )
            except GenerationConflictError:
                current = self.store.job(job.id)
                if current.cancel_requested:
                    self._cancel_generation(current, index)
                    return
                raise
        self.store.finish(job.id)

    def _verify_manifest(self, job: SemanticJob) -> int:
        count = 0
        after_chunk_id = 0
        while True:
            items = self.store.manifest_items_after(
                job.id,
                after_chunk_id=after_chunk_id,
                limit=self.batch_size,
            )
            if not items:
                return count
            self._selected_authority(items)
            count += len(items)
            after_chunk_id = items[-1].chunk_id

    def _selected_authority(self, items: Sequence[_JobItem]) -> list[_AuthoritativeChunk]:
        chunks = self.store.authoritative_chunks([item.chunk_id for item in items])
        by_id = {chunk.chunk_id: chunk for chunk in chunks}
        selected: list[_AuthoritativeChunk] = []
        for item in items:
            chunk = by_id.get(item.chunk_id)
            if chunk is None or chunk.checksum != item.checksum:
                raise AuthoritativeChunkChanged(
                    f"authoritative chunk changed before index write: {item.chunk_id}"
                )
            selected.append(chunk)
        return selected

    def _cancel_if_requested(self, job: SemanticJob, index: SemanticIndex) -> bool:
        if not job.cancel_requested:
            return False
        self._cancel_generation(job, index)
        return True

    def _cancel_generation(self, job: SemanticJob, index: SemanticIndex) -> None:
        if job.index_started or self.store.job(job.id).index_started:
            try:
                index.cancel_generation(job.generation)
            except Exception as error:
                LOGGER.warning("Could not discard cancelled semantic generation: %s", error)
        self.store.cancel(job.id)

    def _redacted_error(self, error: Exception) -> str:
        message = str(error) or type(error).__name__
        try:
            message = redact_configured_secret_values(message, load_provider_catalog(self.config))
        except Exception:
            message = type(error).__name__
        return message[:2_000]


_WORKERS_LOCK = threading.Lock()
_WORKER_EVENTS: dict[str, set[threading.Event]] = {}


def wake_semantic_workers(config: HieronymusConfig) -> None:
    with _WORKERS_LOCK:
        events = tuple(_WORKER_EVENTS.get(str(config.data_root), ()))
    for event in events:
        event.set()


def _register_worker(config: HieronymusConfig, event: threading.Event) -> None:
    with _WORKERS_LOCK:
        _WORKER_EVENTS.setdefault(str(config.data_root), set()).add(event)


def _unregister_worker(config: HieronymusConfig, event: threading.Event) -> None:
    with _WORKERS_LOCK:
        events = _WORKER_EVENTS.get(str(config.data_root))
        if events is None:
            return
        events.discard(event)
        if not events:
            _WORKER_EVENTS.pop(str(config.data_root), None)


def _ensure_state_row(conn: sqlite3.Connection) -> None:
    conn.execute(
        """
        insert or ignore into semantic_index_state(singleton, updated_at)
        values (1, ?)
        """,
        (utc_now(),),
    )


def _authoritative_chunks(
    conn: sqlite3.Connection,
    chunk_ids: Sequence[int] | None = None,
) -> list[_AuthoritativeChunk]:
    where = ""
    params: tuple[object, ...] = ()
    if chunk_ids is not None:
        if not chunk_ids:
            return []
        placeholders = ", ".join("?" for _ in chunk_ids)
        where = f"where chunks.id in ({placeholders})"
        params = tuple(chunk_ids)
    rows = conn.execute(
        f"""
        select chunks.id, chunks.text, chunks.display_text, chunks.location,
               chunks.metadata_json, chunks.series_slug,
               series.default_source_language as source_language,
               series.default_target_language as target_language,
               coalesce((select min(story_scope) from rag_chunk_story_scopes
                         where chunk_id = chunks.id), '*') as story_scope,
               coalesce((select min(semantic_tag) from rag_chunk_semantic_tags
                         where chunk_id = chunks.id), '*') as semantic_scope
        from rag_chunks as chunks
        join series on series.slug = chunks.series_slug
        {where}
        order by chunks.id
        """,
        params,
    ).fetchall()
    result: list[_AuthoritativeChunk] = []
    for row in rows:
        payload = {
            "chunk_id": int(row["id"]),
            "text": row["text"],
            "display_text": row["display_text"],
            "location": row["location"],
            "metadata_json": row["metadata_json"],
            "series_slug": row["series_slug"],
            "source_language": row["source_language"] or "*",
            "target_language": row["target_language"] or "*",
            "story_scope": row["story_scope"],
            "semantic_scope": row["semantic_scope"],
        }
        checksum = hashlib.sha256(
            json.dumps(payload, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest()
        result.append(
            _AuthoritativeChunk(
                chunk_id=int(row["id"]),
                text=row["text"],
                series_slug=row["series_slug"],
                source_language=row["source_language"] or "*",
                target_language=row["target_language"] or "*",
                story_scope=row["story_scope"],
                semantic_scope=row["semantic_scope"],
                checksum=checksum,
            )
        )
    return result


def _job_from_row(row: sqlite3.Row) -> SemanticJob:
    return SemanticJob(
        id=int(row["id"]),
        generation=row["generation"],
        identity=EmbeddingIdentity(
            provider=row["provider"],
            model=row["model"],
            revision=row["revision"],
            dimensions=int(row["dimensions"]),
        ),
        previous_generation=row["previous_generation"],
        manifest_checksum=row["manifest_checksum"],
        status=row["status"],
        expected_count=int(row["expected_count"]),
        processed_count=int(row["processed_count"]),
        error=row["error"],
        retryable=bool(row["retryable"]),
        cancel_requested=bool(row["cancel_requested"]),
        index_started=bool(row["index_started"]),
        created_at=row["created_at"],
        updated_at=row["updated_at"],
    )


def _manifest_checksum(
    chunks: Sequence[_AuthoritativeChunk],
    generation: str,
    identity: EmbeddingIdentity,
) -> str:
    digest = hashlib.sha256()
    digest.update(b"[")
    for index, chunk in enumerate(sorted(chunks, key=lambda item: str(item.chunk_id))):
        if index:
            digest.update(b",")
        record = {
            "checksum": chunk.checksum,
            "chunk_id": str(chunk.chunk_id),
            "generation": generation,
            "model": identity.model,
            "provider": identity.provider,
            "revision": identity.revision,
            "dimensions": identity.dimensions,
            "semantic_scope": chunk.semantic_scope,
            "series_slug": chunk.series_slug,
            "source_language": chunk.source_language,
            "story_scope": chunk.story_scope,
            "target_language": chunk.target_language,
        }
        digest.update(
            json.dumps(
                record,
                ensure_ascii=False,
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8")
        )
    digest.update(b"]")
    return digest.hexdigest()
