from __future__ import annotations

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect, ensure_schema
from hieronymus.dream_maintenance import (
    DREAM_MAINTENANCE_INDEX,
    DreamMaintenance,
)


def _insert_crystals(conn, count: int, *, cycle_id: int = 0) -> list[int]:
    rows = [
        (
            "series",
            "lesson",
            f"Candidate {index}",
            "series",
            0.5,
            0.5,
            "active",
            cycle_id,
            "now",
            "now",
        )
        for index in range(count)
    ]
    conn.executemany(
        """
        insert into crystals(
          series_slug, crystal_type, text, scope_type, strength, confidence, status,
          created_cycle, created_at, updated_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        rows,
    )
    return [
        int(row["id"]) for row in conn.execute("select id from crystals order by id").fetchall()
    ]


def test_decay_selection_is_cap_plus_one_and_excludes_current_cycle_rows(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        ensure_schema(conn)
        ids = _insert_crystals(conn, 2_005)
        conn.execute(
            "update crystals set last_activated_cycle = 9 where id = ?",
            (ids[0],),
        )
        conn.execute(
            "update crystals set last_reinforced_cycle = 9 where id = ?",
            (ids[1],),
        )
        conn.execute(
            "update crystals set crystal_type = 'rule' where id = ?",
            (ids[2],),
        )
        conn.execute(
            "update crystals set created_cycle = 9 where id = ?",
            (ids[3],),
        )

        statements: list[str] = []
        conn.set_trace_callback(statements.append)
        selection = DreamMaintenance(conn).select_decay_candidates(cycle_id=9, limit=3)
        conn.set_trace_callback(None)

    assert tuple(row.id for row in selection.candidates) == tuple(ids[4:7])
    assert selection.has_more is True
    candidate_selects = [sql.lower() for sql in statements if "from crystals" in sql.lower()]
    assert len(candidate_selects) == 1
    assert "limit 4" in candidate_selects[0]
    assert "count(" not in candidate_selects[0]
    assert "offset" not in candidate_selects[0]


def test_decay_selection_uses_named_composite_index(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        ensure_schema(conn)
        _insert_crystals(conn, 3)

        plan = DreamMaintenance(conn).explain_decay_candidate_query(cycle_id=2, limit=3)

    assert any(DREAM_MAINTENANCE_INDEX in detail for detail in plan)


def test_cycle_decay_reports_magnitude_safe_overflow_and_uses_caller_transaction(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        ensure_schema(conn)
        ids = _insert_crystals(conn, 2_000)
        conn.commit()

        summary = DreamMaintenance(conn).apply_cycle_decay(
            cycle_id=4,
            max_changed_crystals=2,
        )
        changed = conn.execute(
            "select id, strength from crystals where strength != 0.5 order by id"
        ).fetchall()

        assert [int(row["id"]) for row in changed] == ids[:2]
        assert summary.decayed_crystal_ids == tuple(ids[:2])
        assert summary.skipped_candidates == (
            {
                "entry_path": f"cycle_decay.crystals[{ids[2]}]",
                "reason": "changed_crystal_cap",
                "crystal_id": ids[2],
            },
            {
                "entry_path": "cycle_decay.crystals",
                "reason": "changed_crystal_cap",
                "skipped_count_at_least": 1,
            },
        )

        conn.rollback()

    with connect(config.database_path) as conn:
        changed_count = conn.execute(
            "select count(*) from crystals where strength != 0.5"
        ).fetchone()[0]
        assert changed_count == 0
        assert conn.execute("select count(*) from memory_events").fetchone()[0] == 0


def test_activation_marking_uses_caller_transaction(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        ensure_schema(conn)
        crystal_id = _insert_crystals(conn, 1)[0]
        conn.execute(
            """
            insert into series(
              slug, title, default_source_language, default_target_language,
              created_at, updated_at
            ) values ('series', 'Series', 'ja', 'ru', 'now', 'now')
            """
        )
        session_id = int(
            conn.execute(
                """
                insert into task_sessions(
                  series_slug, source_language, target_language, task_type,
                  status, created_at, last_activity_at
                ) values ('series', 'ja', 'ru', 'translate', 'complete', 'now', 'now')
                """
            ).lastrowid
        )
        conn.execute(
            """
            insert into crystal_activations(
              crystal_id, session_id, recall_query, rank, score, created_at
            ) values (?, ?, 'query', 1, 1.0, 'now')
            """,
            (crystal_id, session_id),
        )
        conn.commit()

        DreamMaintenance(conn).mark_activations_for_cycle(7, [session_id])
        conn.rollback()

    with connect(config.database_path) as conn:
        activation = conn.execute("select cycle_id from crystal_activations").fetchone()
        crystal = conn.execute(
            "select last_activated_cycle from crystals where id = ?", (crystal_id,)
        ).fetchone()
    assert activation["cycle_id"] is None
    assert crystal["last_activated_cycle"] is None
