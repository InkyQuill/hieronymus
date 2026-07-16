from __future__ import annotations

import sqlite3

from hieronymus.db import apply_migration, connect


def test_global_migration_adds_all_crystal_columns_used_by_hydration(tmp_path) -> None:
    database_path = tmp_path / "legacy.sqlite"
    with sqlite3.connect(database_path) as legacy:
        legacy.execute(
            """
            create table crystals (
              id integer primary key,
              crystal_type text not null,
              text text not null,
              title text not null default '',
              scope_type text not null,
              scope_key text not null default '',
              series_slug text not null default '',
              source_language text not null default '',
              target_language text not null default '',
              tags_json text not null default '[]',
              strength real not null,
              confidence real not null,
              status text not null,
              created_cycle integer not null default 0,
              last_activated_cycle integer,
              last_reinforced_cycle integer,
              created_at text not null,
              updated_at text not null
            )
            """
        )

    with connect(database_path) as conn:
        apply_migration(conn, "global.sql")
        columns = {row["name"] for row in conn.execute("pragma table_info(crystals)")}

    assert {
        "source_credibility",
        "rule_intent",
        "malformed_penalty",
        "supersedes_crystal_id",
        "soft_origin",
        "is_inferred",
    } <= columns
