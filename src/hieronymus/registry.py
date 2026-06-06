from __future__ import annotations

import re
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect

_SLUG_PATTERN = re.compile(r"^[a-z0-9][a-z0-9-]*$")


@dataclass(frozen=True)
class Series:
    slug: str
    title: str
    source_language: str
    target_language: str
    database_path: Path


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _validate_slug(slug: str) -> None:
    if not _SLUG_PATTERN.fullmatch(slug):
        raise ValueError(
            "invalid series slug: use lowercase letters, numbers, and hyphens, "
            "starting with a letter or number"
        )


class Registry:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        self.config.series_dir.mkdir(parents=True, exist_ok=True)
        with connect(self.config.registry_path) as conn:
            apply_migration(conn, "registry.sql")

    def create_series(
        self,
        *,
        slug: str,
        title: str,
        source_language: str,
        target_language: str,
    ) -> Series:
        _validate_slug(slug)
        database_path = self.config.series_dir / f"{slug}.sqlite"
        now = _now()

        with connect(database_path) as conn:
            apply_migration(conn, "series.sql")

        with connect(self.config.registry_path) as conn:
            conn.execute(
                """
                insert into series(
                  slug,
                  title,
                  source_language,
                  target_language,
                  database_path,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, ?, ?, ?, ?)
                on conflict(slug) do update set
                  title=excluded.title,
                  source_language=excluded.source_language,
                  target_language=excluded.target_language,
                  database_path=excluded.database_path,
                  updated_at=excluded.updated_at
                """,
                (slug, title, source_language, target_language, str(database_path), now, now),
            )
            conn.commit()

        return Series(slug, title, source_language, target_language, database_path)

    def get_series(self, slug: str) -> Series:
        with connect(self.config.registry_path) as conn:
            row = conn.execute("select * from series where slug = ?", (slug,)).fetchone()
        if row is None:
            raise KeyError(f"unknown series: {slug}")
        return Series(
            slug=row["slug"],
            title=row["title"],
            source_language=row["source_language"],
            target_language=row["target_language"],
            database_path=Path(row["database_path"]),
        )
