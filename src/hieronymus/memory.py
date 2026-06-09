from __future__ import annotations

import json

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import search_expression
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.models import MemoryEntry
from hieronymus.recall import RecallService
from hieronymus.registry import Registry, Series
from hieronymus.workspace import WorkspaceStore

_DEFAULT_SOURCE_LANGUAGE = "ja"
_DEFAULT_TARGET_LANGUAGE = "en"
_MAX_SEARCH_LIMIT = 50


class _CompatibleMemoryEntry(MemoryEntry):
    def __getitem__(self, key: str) -> object:
        return getattr(self, key)


class MemoryStore:
    def __init__(
        self,
        config: HieronymusConfig,
        context: TranslationContext | None = None,
    ) -> None:
        self.config = config
        self.context = context
        self._workspace = WorkspaceStore(config)

    def add(
        self,
        *,
        kind: str,
        text: str,
        source_ref: str = "",
        importance: int = 3,
        series_slug: str | None = None,
    ) -> int:
        if not kind.strip():
            raise ValueError("kind must not be empty")
        if not text.strip():
            raise ValueError("text must not be empty")

        context = self._context_for_add(series_slug)
        session = self._ensure_default_session(context)
        short_term_kind = "correction" if kind in {"rule", "correction"} else "note"
        return self._workspace.add_short_term_memory(
            session.id,
            source_role="user",
            kind=short_term_kind,
            text=text,
            source_ref=source_ref,
            metadata={
                "legacy_kind": kind,
                "importance": importance,
                "storage_semantics": "short_term_until_dreamed",
            },
        )

    def search(self, *args: str, limit: int = 5) -> list[MemoryEntry]:
        if limit < 1:
            raise ValueError("limit must be at least 1")

        if len(args) == 1:
            query = args[0]
            if not search_expression(query):
                return []
            context = self._context_for_search(None)
        elif len(args) == 2:
            series_slug, query = args
            if not search_expression(query):
                return []
            context = self._context_for_search(series_slug)
            if context is None:
                return []
        else:
            raise TypeError("search expects query or series_slug and query")

        bounded_limit = min(limit, _MAX_SEARCH_LIMIT)
        session = self._active_default_session(context)
        if session is not None:
            results = RecallService(self.config).recall(
                session["id"],
                context,
                query,
                limit=bounded_limit,
            )
            entries = self._disambiguate_entry_ids(
                [self._entry_from_recall_result(result) for result in results]
            )
            return self._sort_legacy_entries(entries)[:bounded_limit]

        return self._fallback_search(context, query, limit=bounded_limit)

    def _context_for_add(self, series_slug: str | None) -> TranslationContext:
        if series_slug is None:
            if self.context is None:
                raise ValueError("series_slug is required when MemoryStore has no context")
            self._ensure_series_for_context(self.context)
            return self.context

        registry = Registry(self.config)
        try:
            series = registry.get_series(series_slug)
        except KeyError:
            series = registry.create_series(
                slug=series_slug,
                title=series_slug,
                source_language=_DEFAULT_SOURCE_LANGUAGE,
                target_language=_DEFAULT_TARGET_LANGUAGE,
            )
        return self._context_from_series(series)

    def _context_for_search(self, series_slug: str | None) -> TranslationContext | None:
        if series_slug is None:
            if self.context is None:
                raise ValueError("series_slug is required when MemoryStore has no context")
            return self.context

        try:
            series = Registry(self.config).get_series(series_slug)
        except KeyError:
            return None
        return self._context_from_series(series)

    def _context_from_series(self, series: Series) -> TranslationContext:
        return TranslationContext(
            series_slug=series.slug,
            source_language=series.source_language,
            target_language=series.target_language,
            task_type="translation",
        )

    def _ensure_series_for_context(self, context: TranslationContext) -> None:
        registry = Registry(self.config)
        try:
            registry.get_series(context.series_slug)
        except KeyError:
            registry.create_series(
                slug=context.series_slug,
                title=context.series_slug,
                source_language=context.source_language,
                target_language=context.target_language,
            )

    def _ensure_default_session(self, context: TranslationContext):
        row = self._active_default_session(context)
        if row is not None:
            return self._workspace.get_session(int(row["id"]))
        return self._workspace.start_session(context)

    def _active_default_session(self, context: TranslationContext):
        with connect(self.config.database_path) as conn:
            return conn.execute(
                """
                select *
                from task_sessions
                where series_slug = ?
                  and source_language = ?
                  and target_language = ?
                  and task_type = ?
                  and volume = ?
                  and chapter = ?
                  and status = 'active'
                order by id desc
                limit 1
                """,
                (
                    context.series_slug,
                    context.source_language,
                    context.target_language,
                    context.task_type,
                    context.volume,
                    context.chapter,
                ),
            ).fetchone()

    def _entry_from_recall_result(self, result) -> tuple[MemoryEntry, str]:
        if result.source == "short_term":
            memory = result.short_term_memory
            metadata = memory.metadata
            return (
                _CompatibleMemoryEntry(
                    id=memory.id,
                    kind=str(metadata.get("legacy_kind") or memory.kind),
                    text=memory.text,
                    importance=_importance_from_metadata(metadata),
                    source_ref=memory.source_ref,
                ),
                "short_term",
            )

        crystal = result.crystal
        return (
            _CompatibleMemoryEntry(
                id=crystal.id,
                kind=crystal.title or crystal.crystal_type,
                text=crystal.text,
                importance=round(crystal.strength * 5),
                source_ref="",
            ),
            "long_term",
        )

    def _fallback_search(
        self,
        context: TranslationContext,
        query: str,
        *,
        limit: int,
    ) -> list[MemoryEntry]:
        expression = search_expression(query)
        with connect(self.config.database_path) as conn:
            short_rows = conn.execute(
                """
                select short_term_memories.*
                from short_term_memories_fts
                join short_term_memories
                  on short_term_memories.id = short_term_memories_fts.rowid
                join task_sessions
                  on task_sessions.id = short_term_memories.session_id
                where short_term_memories_fts match ?
                  and short_term_memories.archived_at is null
                  and task_sessions.series_slug = ?
                  and task_sessions.source_language = ?
                  and task_sessions.target_language = ?
                  and task_sessions.task_type = ?
                  and task_sessions.volume = ?
                  and task_sessions.chapter = ?
                order by bm25(short_term_memories_fts), short_term_memories.id
                limit ?
                """,
                (
                    expression,
                    context.series_slug,
                    context.source_language,
                    context.target_language,
                    context.task_type,
                    context.volume,
                    context.chapter,
                    limit,
                ),
            ).fetchall()
            long_rows = conn.execute(
                """
                select crystals.*
                from crystals_fts
                join crystals on crystals.id = crystals_fts.rowid
                where crystals_fts match ?
                  and crystals.status = 'active'
                  and (
                    (
                      crystals.scope_type = 'series'
                      and crystals.scope_key = ?
                    )
                    or crystals.scope_type = 'global'
                  )
                  and (
                    crystals.source_language = ?
                    or crystals.source_language = ''
                  )
                  and (
                    crystals.target_language = ?
                    or crystals.target_language = ''
                  )
                order by bm25(crystals_fts), crystals.id
                limit ?
                """,
                (
                    expression,
                    context.scope_key,
                    context.source_language,
                    context.target_language,
                    limit,
                ),
            ).fetchall()

        entries = self._disambiguate_entry_ids(
            [
                (
                    _CompatibleMemoryEntry(
                        id=row["id"],
                        kind=str(
                            _json_object(row["metadata_json"]).get("legacy_kind") or row["kind"]
                        ),
                        text=row["text"],
                        importance=_importance_from_metadata(_json_object(row["metadata_json"])),
                        source_ref=row["source_ref"],
                    ),
                    "short_term",
                )
                for row in short_rows
            ]
            + [
                (
                    _CompatibleMemoryEntry(
                        id=row["id"],
                        kind=row["title"] or row["crystal_type"],
                        text=row["text"],
                        importance=round(float(row["strength"]) * 5),
                        source_ref="",
                    ),
                    "long_term",
                )
                for row in long_rows
            ]
        )
        return self._sort_legacy_entries(entries)[:limit]

    def _disambiguate_entry_ids(
        self,
        entries: list[tuple[MemoryEntry, str]],
    ) -> list[MemoryEntry]:
        id_counts: dict[int, int] = {}
        for entry, _source in entries:
            id_counts[entry.id] = id_counts.get(entry.id, 0) + 1

        disambiguated: list[MemoryEntry] = []
        for entry, source in entries:
            if source == "long_term" and id_counts[entry.id] > 1:
                disambiguated.append(
                    _CompatibleMemoryEntry(
                        id=_legacy_long_term_id(entry.id),
                        kind=entry.kind,
                        text=entry.text,
                        importance=entry.importance,
                        source_ref=entry.source_ref,
                    )
                )
                continue
            disambiguated.append(entry)
        return disambiguated

    def _sort_legacy_entries(self, entries: list[MemoryEntry]) -> list[MemoryEntry]:
        return sorted(entries, key=lambda entry: (-entry.importance, abs(entry.id), entry.id < 0))


def _json_object(raw: str) -> dict[str, object]:
    try:
        value = json.loads(raw)
    except json.JSONDecodeError:
        return {}
    return value if isinstance(value, dict) else {}


def _importance_from_metadata(metadata: dict[str, object]) -> int:
    value = metadata.get("importance", 3)
    if isinstance(value, bool):
        return 3
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return round(value)
    return 3


def _legacy_long_term_id(crystal_id: int) -> int:
    return -abs(crystal_id)
