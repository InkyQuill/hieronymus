from __future__ import annotations

import re
from collections.abc import Iterable
from dataclasses import dataclass
from datetime import UTC, datetime

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect

_SLUG_PATTERN = re.compile(r"^[a-z0-9][a-z0-9-]*$")


@dataclass(frozen=True)
class Series:
    slug: str
    title: str
    source_language: str
    target_language: str
    language_tags: tuple[str, ...] = ()
    id: int | None = None


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _validate_slug(slug: str) -> None:
    if not _SLUG_PATTERN.fullmatch(slug):
        raise ValueError(
            "invalid series slug: use lowercase letters, numbers, and hyphens, "
            "starting with a letter or number"
        )


def _normalize_language_tags(tags: Iterable[str]) -> tuple[str, ...]:
    return tuple(sorted({tag.strip().lower() for tag in tags if tag.strip()}))


def _compat_language_tags(source_language: str, target_language: str) -> tuple[str, ...]:
    return _normalize_language_tags(tag for tag in (source_language, target_language) if tag)


class Registry:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def create_series(
        self,
        *,
        slug: str,
        title: str,
        source_language: str = "",
        target_language: str = "",
        language_tags: Iterable[str] | None = None,
    ) -> Series:
        _validate_slug(slug)
        now = _now()
        if language_tags is None:
            normalized_language_tags = _compat_language_tags(source_language, target_language)
        else:
            normalized_language_tags = _normalize_language_tags(language_tags)

        with connect(self.config.database_path) as conn:
            conn.execute(
                """
                insert into series(
                  slug,
                  title,
                  default_source_language,
                  default_target_language,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, ?, ?, ?)
                on conflict(slug) do update set
                  title=excluded.title,
                  default_source_language=excluded.default_source_language,
                  default_target_language=excluded.default_target_language,
                  updated_at=excluded.updated_at
                """,
                (slug, title, source_language, target_language, now, now),
            )
            row = conn.execute("select * from series where slug = ?", (slug,)).fetchone()
            if row is None:
                raise RuntimeError(f"series was not created: {slug}")
            self._replace_series_language_tags(
                conn,
                series_id=int(row["id"]),
                language_tags=normalized_language_tags,
                now=now,
            )
            conn.commit()

        return self.get_series(slug)

    def get_series(self, slug: str) -> Series:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from series where slug = ?", (slug,)).fetchone()
            if row is None:
                raise KeyError(f"unknown series: {slug}")
            language_tags = self._series_language_tags(conn, int(row["id"]))
        return Series(
            slug=row["slug"],
            title=row["title"],
            source_language=row["default_source_language"],
            target_language=row["default_target_language"],
            language_tags=language_tags,
            id=int(row["id"]),
        )

    def list_series(self) -> list[Series]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute("select * from series order by slug").fetchall()
            tags_by_series_id = self._series_language_tags_by_id(
                conn,
                [int(row["id"]) for row in rows],
            )
        return [
            Series(
                slug=row["slug"],
                title=row["title"],
                source_language=row["default_source_language"],
                target_language=row["default_target_language"],
                language_tags=tags_by_series_id.get(int(row["id"]), ()),
                id=int(row["id"]),
            )
            for row in rows
        ]

    def set_series_language_tags(
        self,
        series_id: int,
        language_tags: Iterable[str],
    ) -> None:
        normalized_language_tags = _normalize_language_tags(language_tags)
        now = _now()
        with connect(self.config.database_path) as conn:
            row = conn.execute("select id from series where id = ?", (series_id,)).fetchone()
            if row is None:
                raise KeyError(f"unknown series id: {series_id}")
            self._replace_series_language_tags(
                conn,
                series_id=series_id,
                language_tags=normalized_language_tags,
                now=now,
            )
            conn.commit()

    def _replace_series_language_tags(
        self,
        conn,
        *,
        series_id: int,
        language_tags: Iterable[str],
        now: str,
    ) -> None:
        conn.execute("delete from series_language_tags where series_id = ?", (series_id,))
        conn.executemany(
            """
            insert into series_language_tags(series_id, language_tag, created_at)
            values (?, ?, ?)
            """,
            [(series_id, tag, now) for tag in language_tags],
        )

    def _series_language_tags(self, conn, series_id: int) -> tuple[str, ...]:
        return self._series_language_tags_by_id(conn, [series_id]).get(series_id, ())

    def _series_language_tags_by_id(
        self,
        conn,
        series_ids: Iterable[int],
    ) -> dict[int, tuple[str, ...]]:
        ids = tuple(series_ids)
        if not ids:
            return {}
        placeholders = ", ".join("?" for _ in ids)
        rows = conn.execute(
            f"""
            select series_id, language_tag
            from series_language_tags
            where series_id in ({placeholders})
            order by series_id, language_tag
            """,
            ids,
        ).fetchall()
        tags_by_series_id: dict[int, list[str]] = {series_id: [] for series_id in ids}
        for row in rows:
            tags_by_series_id[int(row["series_id"])].append(row["language_tag"])
        return {
            series_id: tuple(language_tags)
            for series_id, language_tags in tags_by_series_id.items()
        }
