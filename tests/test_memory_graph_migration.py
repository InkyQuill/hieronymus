from __future__ import annotations

import json
from pathlib import Path
from unittest.mock import patch

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.doctor import Doctor, DoctorFinding
from hieronymus.memory_migration import MemoryGraphMigrator

NOW = "2026-06-10T00:00:00+00:00"


def test_strict_term_migration_creates_rule_graph(config: HieronymusConfig) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        term_id = _insert_strict_term(conn)
        conn.commit()

    MemoryGraphMigrator(config).run()

    with connect(config.database_path) as conn:
        row = conn.execute(
            """
            select
              concept.id as concept_id,
              concept.canonical_name,
              concept.status as concept_status,
              source_facet.value as source_facet,
              source_facet.language as source_language,
              source_facet.is_canonical as source_is_canonical,
              rendering_facet.value as rendering_facet,
              rendering_facet.language as target_language,
              crystal.id as crystal_id,
              crystal.crystal_type,
              crystal.status as crystal_status,
              crystal.text,
              crystal.source_credibility,
              link.link_type
            from concepts concept
            join concept_facets source_facet
              on source_facet.concept_id = concept.id
             and source_facet.facet_type = 'name'
            join concept_facets rendering_facet
              on rendering_facet.concept_id = concept.id
             and rendering_facet.facet_type = 'rendering'
            join crystal_concepts link on link.concept_id = concept.id
            join crystals crystal on crystal.id = link.crystal_id
            where concept.canonical_name = '攻撃力上昇'
            """
        ).fetchone()
        ledger_targets = {
            row["target_table"]
            for row in conn.execute(
                """
                select target_table
                from memory_graph_migration_ledger
                where source_table = 'strict_terms'
                  and source_id in (?, ?, ?)
                """,
                (str(term_id), f"{term_id}:source", f"{term_id}:rendering"),
            )
        }

    assert row is not None
    assert row["concept_status"] == "established"
    assert row["source_facet"] == "攻撃力上昇"
    assert row["source_language"] == "ja"
    assert row["source_is_canonical"] == 1
    assert row["rendering_facet"] == "ATK Up"
    assert row["target_language"] == "en"
    assert row["crystal_type"] == "rule"
    assert row["crystal_status"] == "active"
    assert row["text"] == "攻撃力上昇 is translated as ATK Up."
    assert row["source_credibility"] == "user_rule"
    assert row["link_type"] == "defines"
    assert ledger_targets == {"concepts", "concept_facets", "crystals"}


def test_crystal_legacy_languages_and_tags_migrate_to_side_tables(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        crystal_id = conn.execute(
            """
            insert into crystals(
              crystal_type,
              text,
              title,
              scope_type,
              scope_key,
              series_slug,
              source_language,
              target_language,
              tags_json,
              strength,
              confidence,
              status,
              created_at,
              updated_at
            )
            values (
              'lesson',
              'Render menu labels tersely.',
              'Menu labels',
              'series',
              'series:book',
              'book',
              'JA',
              'EN',
              ?,
              0.5,
              0.5,
              'active',
              ?,
              ?
            )
            """,
            (json.dumps(["ui", " term ", "", "ui"]), NOW, NOW),
        ).lastrowid
        conn.commit()

    MemoryGraphMigrator(config).run()

    with connect(config.database_path) as conn:
        language_tags = conn.execute(
            """
            select language_tag
            from crystal_language_tags
            where crystal_id = ?
            order by language_tag
            """,
            (crystal_id,),
        ).fetchall()
        semantic_tags = conn.execute(
            """
            select tag
            from crystal_semantic_tags
            where crystal_id = ?
            order by tag
            """,
            (crystal_id,),
        ).fetchall()

    assert [row["language_tag"] for row in language_tags] == ["en", "ja"]
    assert [row["tag"] for row in semantic_tags] == ["term", "ui"]


def test_pending_strict_concept_proposal_migrates_to_candidate_concept_and_facets(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        conn.execute(
            """
            insert into strict_concept_proposals(
              series_slug,
              source_language,
              target_language,
              concept_text,
              source_form,
              canonical_rendering,
              approved_variants_json,
              forbidden_variants_json,
              rationale,
              status,
              created_at,
              updated_at
            )
            values (
              'book',
              'ja',
              'en',
              'Sense',
              'センス',
              'Sense',
              '["Sense"]',
              '["Senses"]',
              'Translator needs review.',
              'pending',
              ?,
              ?
            )
            """,
            (NOW, NOW),
        )
        conn.commit()

    MemoryGraphMigrator(config).run()

    with connect(config.database_path) as conn:
        concept = conn.execute(
            """
            select *
            from concepts
            where canonical_name = 'Sense'
            """
        ).fetchone()
        facets = conn.execute(
            """
            select facet_type, value, language, is_canonical
            from concept_facets
            where concept_id = ?
            order by facet_type, value
            """,
            (concept["id"],),
        ).fetchall()
        rule = conn.execute(
            """
            select crystal.status, crystal.text, link.link_type
            from crystals crystal
            join crystal_concepts link on link.crystal_id = crystal.id
            where link.concept_id = ?
            """,
            (concept["id"],),
        ).fetchone()

    assert concept["status"] == "candidate"
    assert concept["description"] == "Translator needs review."
    assert [tuple(row) for row in facets] == [
        ("name", "センス", "ja", 1),
        ("rendering", "Sense", "en", 0),
    ]
    assert rule["status"] == "candidate"
    assert rule["text"] == "センス is translated as Sense, not Senses."
    assert rule["link_type"] == "defines"


def test_migration_is_idempotent_across_second_run(config: HieronymusConfig) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        _insert_strict_term(conn)
        conn.execute(
            """
            insert into crystals(
              crystal_type,
              text,
              scope_type,
              source_language,
              target_language,
              tags_json,
              strength,
              confidence,
              status,
              created_at,
              updated_at
            )
            values ('lesson', 'Legacy lesson.', 'series', 'ja', 'en', '["ui"]', 0.5, 0.5,
                    'active', ?, ?)
            """,
            (NOW, NOW),
        )
        conn.commit()

    migrator = MemoryGraphMigrator(config)
    migrator.run()
    first_counts = _graph_counts(config)
    migrator.run()
    second_counts = _graph_counts(config)

    assert second_counts == first_counts


def test_migration_rolls_back_partial_backfill_on_failure(config: HieronymusConfig) -> None:
    _seed_base(config)
    migrator = MemoryGraphMigrator(config)

    with pytest.raises(RuntimeError, match="forced migration failure"):
        with patch.object(
            migrator,
            "_migrate_strict_terms",
            side_effect=RuntimeError("forced migration failure"),
        ):
            migrator.run()

    with connect(config.database_path) as conn:
        assert conn.execute("select count(*) from series_language_tags").fetchone()[0] == 0


def test_strict_term_with_unsupported_source_alias_is_skipped(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        term_id = _insert_strict_term(conn)
        conn.execute(
            """
            insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
            values (?, 'ja', '攻撃バフ', 'source_variant', 1)
            """,
            (term_id,),
        )
        conn.commit()

    report = MemoryGraphMigrator(config).run()

    assert report.skipped == {"strict_terms.unsupported_alias": 1}
    with connect(config.database_path) as conn:
        assert conn.execute("select count(*) from concepts").fetchone()[0] == 0
        assert conn.execute("select count(*) from crystals").fetchone()[0] == 0


def test_strict_term_with_unsupported_search_alias_is_skipped(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        term_id = _insert_strict_term(conn)
        conn.execute(
            """
            insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
            values (?, 'ja', 'atk buff', 'search_alias', 1)
            """,
            (term_id,),
        )
        conn.commit()

    report = MemoryGraphMigrator(config).run()

    assert report.skipped == {"strict_terms.unsupported_alias": 1}
    with connect(config.database_path) as conn:
        assert conn.execute("select count(*) from concepts").fetchone()[0] == 0
        assert conn.execute("select count(*) from crystals").fetchone()[0] == 0


def test_strict_term_with_case_insensitive_forbidden_alias_is_skipped(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        term_id = _insert_strict_term(conn)
        conn.execute(
            """
            insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
            values (?, 'en', 'Attack Up', 'forbidden_variant', 0)
            """,
            (term_id,),
        )
        conn.commit()

    report = MemoryGraphMigrator(config).run()

    assert report.skipped == {"strict_terms.unsupported_alias": 1}
    with connect(config.database_path) as conn:
        assert conn.execute("select count(*) from concepts").fetchone()[0] == 0
        assert conn.execute("select count(*) from crystals").fetchone()[0] == 0


def test_migrator_tolerates_partial_older_task_and_crystal_tables(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table task_sessions (
              id integer primary key,
              status text not null
            );
            create table crystals (
              id integer primary key,
              crystal_type text not null,
              text text not null,
              confidence real not null,
              created_at text not null
            );
            insert into task_sessions(id, status) values (1, 'active');
            insert into crystals(id, crystal_type, text, confidence, created_at)
            values (1, 'lesson', 'Legacy.', 0.4, '2026-06-10T00:00:00+00:00');
            """
        )
        conn.commit()

    report = MemoryGraphMigrator(config).run()

    assert report.skipped == {}


def test_doctor_tolerates_partial_older_task_and_crystal_tables(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table task_sessions (
              id integer primary key,
              status text not null
            );
            create table crystals (
              id integer primary key,
              crystal_type text not null
            );
            insert into task_sessions(id, status) values (1, 'active');
            insert into crystals(id, crystal_type) values (1, 'lesson');
            """
        )
        conn.commit()

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=False)

    assert all(finding.code != "database-unreadable" for finding in report["errors"])


def test_dry_report_counts_crystal_soft_origin_backfill(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table crystals (
              id integer primary key,
              source_ref text not null default '',
              soft_origin text
            );
            insert into crystals(id, source_ref, soft_origin)
            values (1, 'legacy-note', '');
            """
        )
        conn.commit()

    report = MemoryGraphMigrator.inspect(config)

    assert report.pending["crystals.soft_origin"] == 1


def test_doctor_reports_crystal_soft_origin_dry_run_work(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table crystals (
              id integer primary key,
              source_ref text not null default '',
              soft_origin text
            );
            insert into crystals(id, source_ref, soft_origin)
            values (1, 'legacy-note', '');
            """
        )
        conn.commit()

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=False)

    assert (
        DoctorFinding(
            level="warning",
            code="memory-graph-migration-pending",
            message=(
                "Legacy memory graph migration has pending dry-run work: crystals.soft_origin: 1"
            ),
        )
        in report["warnings"]
    )


def test_doctor_migration_inspect_does_not_use_per_row_missing_value_scan(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    _seed_base(config)
    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        with patch(
            "hieronymus.memory_migration._missing_values",
            create=True,
            side_effect=AssertionError("per-row scan should not run during doctor"),
        ):
            Doctor(config).run(autofix=False)


def _seed_base(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
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
            values ('book', 'Book', 'ja', 'en', ?, ?)
            """,
            (NOW, NOW),
        )
        conn.commit()


def _insert_strict_term(conn) -> int:
    term_id = conn.execute(
        """
        insert into strict_terms(
          series_slug,
          source_language,
          target_language,
          category,
          source_text,
          canonical_translation,
          status,
          notes,
          created_at,
          updated_at
        )
        values ('book', 'ja', 'en', 'skill', '攻撃力上昇', 'ATK Up', 'approved',
                'A stable skill rendering.', ?, ?)
        """,
        (NOW, NOW),
    ).lastrowid
    conn.execute(
        "insert into strict_term_tags(term_id, tag) values (?, 'skill')",
        (term_id,),
    )
    return int(term_id)


def _graph_counts(config: HieronymusConfig) -> dict[str, int]:
    tables = (
        "series_language_tags",
        "crystal_language_tags",
        "crystal_semantic_tags",
        "concepts",
        "concept_facets",
        "concept_facet_language_tags",
        "concept_semantic_tags",
        "crystals",
        "crystal_concepts",
        "memory_graph_migration_ledger",
    )
    with connect(config.database_path) as conn:
        return {
            table: int(conn.execute(f"select count(*) from {table}").fetchone()[0])
            for table in tables
        }
