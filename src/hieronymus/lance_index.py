from __future__ import annotations

import json
import math
import re
import shutil
import threading
import uuid
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import lancedb
import pyarrow as pa

from hieronymus.embeddings import EmbeddingIdentity
from hieronymus.semantic_index import (
    GenerationConflictError,
    GenerationNotActiveError,
    GenerationNotFoundError,
    GenerationPointer,
    IndexBatchSizeError,
    IndexClosedError,
    IndexHealth,
    IndexManifestError,
    IndexRow,
    SearchFilters,
    SearchResult,
    compute_manifest_checksum,
)

_GENERATION_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}")
_TABLE_NAME = "chunks"


class LanceSemanticIndex:
    """Generation-scoped local LanceDB index with externally owned activation."""

    def __init__(
        self,
        root: Path,
        *,
        pointer: GenerationPointer,
        max_batch_size: int,
    ) -> None:
        if max_batch_size < 1:
            raise ValueError("max_batch_size must be positive")
        self._root = root.expanduser().resolve()
        self._pointer = pointer
        self._max_batch_size = max_batch_size
        self._lock = threading.RLock()
        self._closed = False

    @property
    def active_generation(self) -> str | None:
        with self._lock:
            self._ensure_open()
            return self._pointer.read()

    def health(self) -> IndexHealth:
        with self._lock:
            if self._closed:
                return IndexHealth(healthy=False, detail="closed")
            active = self._pointer.read()
            if active is None:
                return IndexHealth(healthy=True, detail="ok")
            try:
                manifest = self._load_manifest(active)
                self._require_state(manifest, "ready")
                self._open_table(active)
            except (
                GenerationConflictError,
                GenerationNotFoundError,
                IndexManifestError,
                OSError,
                ValueError,
            ) as error:
                return IndexHealth(healthy=False, detail=str(error))
            return IndexHealth(healthy=True, detail="ok")

    def begin_rebuild(
        self,
        generation: str,
        *,
        identity: EmbeddingIdentity,
        expected_count: int,
        manifest_checksum: str,
    ) -> None:
        with self._lock:
            self._ensure_open()
            self._validate_generation(generation)
            if expected_count < 0:
                raise ValueError("expected_count must not be negative")
            if not manifest_checksum.strip():
                raise ValueError("manifest_checksum must not be blank")
            destination = self._generation_path(generation)
            if destination.exists():
                raise GenerationConflictError(f"generation already exists: {generation}")
            self._generations_root.mkdir(parents=True, exist_ok=True)
            staging = self._generations_root / f".building-{uuid.uuid4().hex}"
            try:
                staging.mkdir()
                connection = lancedb.connect(staging / "lance")
                connection.create_table(_TABLE_NAME, schema=self._schema(identity))
                self._write_json_atomic(
                    staging / "manifest.json",
                    {
                        "generation": generation,
                        "state": "building",
                        "expected_count": expected_count,
                        "manifest_checksum": manifest_checksum,
                        "identity": {
                            "provider": identity.provider,
                            "model": identity.model,
                            "revision": identity.revision,
                            "dimensions": identity.dimensions,
                        },
                    },
                )
                staging.rename(destination)
            except Exception:
                shutil.rmtree(staging, ignore_errors=True)
                raise

    def upsert(self, generation: str, rows: Sequence[IndexRow]) -> None:
        with self._lock:
            self._ensure_open()
            if self._pointer.read() == generation:
                raise GenerationConflictError("cannot mutate the active generation")
            if len(rows) > self._max_batch_size:
                raise IndexBatchSizeError(
                    f"index mutation accepts at most {self._max_batch_size} rows per batch"
                )
            manifest = self._load_manifest(generation)
            self._require_state(manifest, "building")
            identity = self._identity(manifest)
            if not rows:
                return
            for row in rows:
                if row.generation != generation:
                    raise ValueError("row generation does not match target generation")
                if row.identity != identity:
                    raise ValueError("row embedding identity does not match generation identity")
            table = self._open_table(generation)
            chunk_ids = [row.chunk_id for row in rows]
            table.delete(self._in_filter("chunk_id", chunk_ids))
            table.add([self._row_to_record(row) for row in rows])

    def delete_chunk_ids(self, generation: str, chunk_ids: Sequence[str]) -> None:
        with self._lock:
            self._ensure_open()
            if self._pointer.read() == generation:
                raise GenerationConflictError("cannot mutate the active generation")
            manifest = self._load_manifest(generation)
            self._require_state(manifest, "building")
            if not chunk_ids:
                return
            if len(chunk_ids) > self._max_batch_size:
                raise IndexBatchSizeError(
                    f"index mutation accepts at most {self._max_batch_size} chunk IDs per batch"
                )
            self._open_table(generation).delete(self._in_filter("chunk_id", chunk_ids))

    def search(
        self,
        generation: str,
        vector: Sequence[float],
        *,
        limit: int,
        filters: SearchFilters | None = None,
    ) -> list[SearchResult]:
        with self._lock:
            self._ensure_open()
            if limit < 1:
                raise ValueError("limit must be positive")
            if self._pointer.read() != generation:
                raise GenerationNotActiveError(f"generation is not active: {generation}")
            manifest = self._load_manifest(generation)
            self._require_state(manifest, "ready")
            identity = self._identity(manifest)
            query_vector = tuple(float(value) for value in vector)
            if len(query_vector) != identity.dimensions:
                raise ValueError(
                    f"expected {identity.dimensions} query dimensions, received {len(query_vector)}"
                )
            if any(not math.isfinite(value) for value in query_vector):
                raise ValueError("query vector components must be finite")
            table = self._open_table(generation)
            query = table.search(list(query_vector)).metric("l2")
            where = self._where(filters)
            if where:
                query = query.where(where, prefilter=True)
            candidate_count = table.count_rows(where)
            records = query.limit(candidate_count).to_list() if candidate_count else []
            results = [
                SearchResult(
                    chunk_id=str(record["chunk_id"]),
                    distance=float(record["_distance"]),
                    generation=generation,
                    identity=identity,
                )
                for record in records
            ]
            return sorted(results, key=lambda item: (item.distance, item.chunk_id))[:limit]

    def activate_generation(
        self, generation: str, *, expected_active_generation: str | None
    ) -> None:
        with self._lock:
            self._ensure_open()
            manifest = self._load_manifest(generation)
            rows = [
                self._record_to_row(item)
                for item in self._open_table(generation).to_arrow().to_pylist()
            ]
            expected_count = int(manifest["expected_count"])
            if len(rows) != expected_count:
                raise IndexManifestError(
                    f"generation {generation} has {len(rows)} rows; expected {expected_count}"
                )
            actual_checksum = compute_manifest_checksum(rows)
            expected_checksum = str(manifest["manifest_checksum"])
            if actual_checksum != expected_checksum:
                raise IndexManifestError(
                    f"generation {generation} checksum does not match its manifest"
                )
            manifest["state"] = "ready"
            self._write_json_atomic(self._generation_path(generation) / "manifest.json", manifest)
            if not self._pointer.compare_and_set(expected_active_generation, generation):
                raise GenerationConflictError("active generation changed during activation")

    def cancel_generation(self, generation: str) -> None:
        with self._lock:
            self._ensure_open()
            if self._pointer.read() == generation:
                raise GenerationConflictError("cannot cancel the active generation")
            path = self._generation_path(generation)
            if not path.is_dir():
                raise GenerationNotFoundError(f"generation does not exist: {generation}")
            shutil.rmtree(path)

    def close(self) -> None:
        with self._lock:
            self._closed = True

    @property
    def _generations_root(self) -> Path:
        return self._root / "generations"

    def _generation_path(self, generation: str) -> Path:
        self._validate_generation(generation)
        return self._generations_root / generation

    @staticmethod
    def _validate_generation(generation: str) -> None:
        if _GENERATION_PATTERN.fullmatch(generation) is None:
            raise ValueError("generation must be a safe non-empty identifier")

    def _load_manifest(self, generation: str) -> dict[str, Any]:
        path = self._generation_path(generation)
        if not path.is_dir():
            raise GenerationNotFoundError(f"generation does not exist: {generation}")
        try:
            manifest = json.loads((path / "manifest.json").read_text(encoding="utf-8"))
            if manifest["generation"] != generation:
                raise ValueError("generation identity mismatch")
            self._identity(manifest)
            int(manifest["expected_count"])
            str(manifest["manifest_checksum"])
            if manifest["state"] not in {"building", "ready"}:
                raise ValueError("invalid generation state")
        except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
            raise IndexManifestError(f"generation manifest is invalid: {generation}") from error
        return manifest

    @staticmethod
    def _require_state(manifest: dict[str, Any], expected: str) -> None:
        if manifest["state"] != expected:
            raise GenerationConflictError(f"generation is {manifest['state']}; expected {expected}")

    def _open_table(self, generation: str):
        path = self._generation_path(generation)
        if not path.is_dir():
            raise GenerationNotFoundError(f"generation does not exist: {generation}")
        try:
            return lancedb.connect(path / "lance").open_table(_TABLE_NAME)
        except Exception as error:
            raise IndexManifestError(f"generation table is unavailable: {generation}") from error

    @staticmethod
    def _schema(identity: EmbeddingIdentity) -> pa.Schema:
        return pa.schema(
            [
                pa.field("chunk_id", pa.string(), nullable=False),
                pa.field("generation", pa.string(), nullable=False),
                pa.field("series_slug", pa.string(), nullable=False),
                pa.field("source_language", pa.string(), nullable=False),
                pa.field("target_language", pa.string(), nullable=False),
                pa.field("story_scope", pa.string(), nullable=False),
                pa.field("semantic_scope", pa.string(), nullable=False),
                pa.field("checksum", pa.string(), nullable=False),
                pa.field("provider", pa.string(), nullable=False),
                pa.field("model", pa.string(), nullable=False),
                pa.field("revision", pa.string()),
                pa.field("dimensions", pa.int32(), nullable=False),
                pa.field("vector", pa.list_(pa.float32(), identity.dimensions), nullable=False),
            ]
        )

    @staticmethod
    def _identity(manifest: dict[str, Any]) -> EmbeddingIdentity:
        raw = manifest["identity"]
        return EmbeddingIdentity(
            provider=str(raw["provider"]),
            model=str(raw["model"]),
            revision=None if raw["revision"] is None else str(raw["revision"]),
            dimensions=int(raw["dimensions"]),
        )

    @staticmethod
    def _row_to_record(row: IndexRow) -> dict[str, object]:
        return {
            "chunk_id": row.chunk_id,
            "generation": row.generation,
            "series_slug": row.series_slug,
            "source_language": row.source_language,
            "target_language": row.target_language,
            "story_scope": row.story_scope,
            "semantic_scope": row.semantic_scope,
            "checksum": row.checksum,
            "provider": row.identity.provider,
            "model": row.identity.model,
            "revision": row.identity.revision,
            "dimensions": row.identity.dimensions,
            "vector": list(row.vector),
        }

    @staticmethod
    def _record_to_row(record: dict[str, object]) -> IndexRow:
        return IndexRow(
            chunk_id=str(record["chunk_id"]),
            generation=str(record["generation"]),
            series_slug=str(record["series_slug"]),
            source_language=str(record["source_language"]),
            target_language=str(record["target_language"]),
            story_scope=str(record["story_scope"]),
            semantic_scope=str(record["semantic_scope"]),
            checksum=str(record["checksum"]),
            identity=EmbeddingIdentity(
                provider=str(record["provider"]),
                model=str(record["model"]),
                revision=None if record["revision"] is None else str(record["revision"]),
                dimensions=int(record["dimensions"]),
            ),
            vector=tuple(record["vector"]),  # type: ignore[arg-type]
        )

    @staticmethod
    def _quote(value: str) -> str:
        return "'" + value.replace("'", "''") + "'"

    @classmethod
    def _in_filter(cls, field: str, values: Sequence[str]) -> str:
        return f"{field} IN ({', '.join(cls._quote(value) for value in values)})"

    @classmethod
    def _where(cls, filters: SearchFilters | None) -> str | None:
        if filters is None:
            return None
        clauses = [
            f"{name} = {cls._quote(value)}"
            for name, value in (
                ("series_slug", filters.series_slug),
                ("source_language", filters.source_language),
                ("target_language", filters.target_language),
                ("story_scope", filters.story_scope),
                ("semantic_scope", filters.semantic_scope),
            )
            if value is not None
        ]
        return " AND ".join(clauses) or None

    @staticmethod
    def _write_json_atomic(path: Path, payload: dict[str, object]) -> None:
        temporary = path.with_suffix(".tmp")
        temporary.write_text(
            json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        temporary.replace(path)

    def _ensure_open(self) -> None:
        if self._closed:
            raise IndexClosedError("index is closed")
