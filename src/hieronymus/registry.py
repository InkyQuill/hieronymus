from __future__ import annotations

import re
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
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def create_series(
        self,
        *,
        slug: str,
        title: str,
        source_language: str,
        target_language: str,
    ) -> Series:
        _validate_slug(slug)
        now = _now()

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
            conn.commit()

        return Series(slug, title, source_language, target_language)

    def get_series(self, slug: str) -> Series:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from series where slug = ?", (slug,)).fetchone()
        if row is None:
            raise KeyError(f"unknown series: {slug}")
        return Series(
            slug=row["slug"],
            title=row["title"],
            source_language=row["default_source_language"],
            target_language=row["default_target_language"],
        )

    def list_series(self) -> list[Series]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute("select * from series order by slug").fetchall()
        return [
            Series(
                slug=row["slug"],
                title=row["title"],
                source_language=row["default_source_language"],
                target_language=row["default_target_language"],
            )
            for row in rows
        ]
