from __future__ import annotations

import inspect
import json
from pathlib import Path
from unittest.mock import patch

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.doctor import Doctor, DoctorFinding
from hieronymus.memory_migration import MemoryGraphMigrator

NOW = "2026-06-10T00:00:00+00:00"


def test_memory_graph_migrator_constructor_signature_matches_public_api() -> None:
    parameter = inspect.signature(MemoryGraphMigrator).parameters["db"]

    assert parameter.annotation == "Database"


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


def test_migration_reconciles_existing_ledger_concept(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        term_id = _insert_strict_term(conn)
        conn.commit()
    MemoryGraphMigrator(config).run()
    with connect(config.database_path) as conn:
        conn.execute(
            """
            update strict_terms
            set notes = 'A corrected skill rendering.'
            where id = ?
            """,
            (term_id,),
        )
        conn.execute(
            """
            update concepts
            set description = 'stale',
                status = 'established',
                confidence = 0.1
            where id = (
              select target_id
              from memory_graph_migration_ledger
              where source_table = 'strict_terms'
                and source_id = ?
                and target_table = 'concepts'
            )
            """,
            (str(term_id),),
        )
        conn.commit()

    report = MemoryGraphMigrator(config).run()

    with connect(config.database_path) as conn:
        row = conn.execute(
            """
            select description, status, confidence
            from concepts
            where id = (
              select target_id
              from memory_graph_migration_ledger
              where source_table = 'strict_terms'
                and source_id = ?
                and target_table = 'concepts'
            )
            """,
            (str(term_id),),
        ).fetchone()
    assert report.created == {}
    assert row["description"] == "A corrected skill rendering."
    assert row["status"] == "established"
    assert row["confidence"] == 0.95


def test_migration_does_not_downgrade_stronger_matching_concept(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        concept_id = conn.execute(
            """
            insert into concepts(
              canonical_name,
              description,
              scope_type,
              scope_key,
              status,
              confidence,
              created_at,
              updated_at
            )
            values ('Sense', 'Established identity.', 'series', 'series:book',
                    'established', 0.95, ?, ?)
            """,
            (NOW, NOW),
        ).lastrowid
        source_facet_id = conn.execute(
            """
            insert into concept_facets(
              concept_id,
              language,
              facet_type,
              value,
              confidence,
              is_canonical,
              created_at,
              updated_at
            )
            values (?, 'ja', 'name', 'センス', 0.1, 0, ?, ?)
            """,
            (concept_id, NOW, NOW),
        ).lastrowid
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
            values ('book', 'ja', 'en', 'Sense', 'センス', 'Sense', '["Sense"]',
                    '[]', 'Weaker pending proposal.', 'pending', ?, ?)
            """,
            (NOW, NOW),
        )
        conn.commit()

    MemoryGraphMigrator(config).run()

    with connect(config.database_path) as conn:
        concept = conn.execute(
            "select description, status, confidence from concepts where id = ?",
            (concept_id,),
        ).fetchone()
        source_facet = conn.execute(
            "select confidence, is_canonical from concept_facets where id = ?",
            (source_facet_id,),
        ).fetchone()
    assert concept["description"] == "Established identity."
    assert concept["status"] == "established"
    assert concept["confidence"] == 0.95
    assert source_facet["confidence"] == 0.45
    assert source_facet["is_canonical"] == 1


def test_migration_reconciles_existing_ledger_rule_crystal(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        term_id = _insert_strict_term(conn)
        conn.commit()
    MemoryGraphMigrator(config).run()
    with connect(config.database_path) as conn:
        conn.execute(
            """
            update strict_terms
            set canonical_translation = 'Attack Boost',
                notes = 'A renamed skill rendering.'
            where id = ?
            """,
            (term_id,),
        )
        conn.execute(
            """
            update crystals
            set text = 'stale',
                title = 'stale',
                status = 'active',
                confidence = 0.1
            where id = (
              select target_id
              from memory_graph_migration_ledger
              where source_table = 'strict_terms'
                and source_id = ?
                and target_table = 'crystals'
            )
            """,
            (str(term_id),),
        )
        conn.commit()

    report = MemoryGraphMigrator(config).run()

    with connect(config.database_path) as conn:
        row = conn.execute(
            """
            select id, text, title, status, confidence
            from crystals
            where id = (
              select target_id
              from memory_graph_migration_ledger
              where source_table = 'strict_terms'
                and source_id = ?
                and target_table = 'crystals'
            )
            """,
            (str(term_id),),
        ).fetchone()
        fts_text = conn.execute(
            """
            select text
            from crystals_fts
            where rowid = ?
            """,
            (row["id"],),
        ).fetchone()["text"]
    assert report.created == {}
    assert row["text"] == "攻撃力上昇 is translated as Attack Boost."
    assert row["title"] == ""
    assert row["status"] == "active"
    assert row["confidence"] == 0.95
    assert fts_text == "攻撃力上昇 is translated as Attack Boost."


def test_migration_reconciles_existing_ledger_facet(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        term_id = _insert_strict_term(conn)
        conn.commit()
    MemoryGraphMigrator(config).run()
    with connect(config.database_path) as conn:
        source_facet_id = conn.execute(
            """
            select target_id
            from memory_graph_migration_ledger
            where source_table = 'strict_terms'
              and source_id = ?
              and target_table = 'concept_facets'
            """,
            (f"{term_id}:source",),
        ).fetchone()["target_id"]
        conn.execute(
            """
            update concept_facets
            set confidence = 0.1,
                is_canonical = 0
            where id = ?
            """,
            (source_facet_id,),
        )
        conn.commit()

    MemoryGraphMigrator(config).run()

    with connect(config.database_path) as conn:
        row = conn.execute(
            "select confidence, is_canonical from concept_facets where id = ?",
            (source_facet_id,),
        ).fetchone()
    assert row["confidence"] == 0.95
    assert row["is_canonical"] == 1


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


def test_strict_term_with_source_alias_in_partial_alias_schema_is_skipped(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        _create_partial_alias_schema(conn)
        term_id = _insert_strict_term(conn)
        conn.execute(
            """
            insert into strict_term_aliases(term_id, language, text, kind)
            values (?, 'ja', '攻撃バフ', 'source_variant')
            """,
            (term_id,),
        )
        conn.commit()

    report = MemoryGraphMigrator(config).run()

    assert report.skipped == {"strict_terms.unsupported_alias": 1}
    with connect(config.database_path) as conn:
        assert conn.execute("select count(*) from concepts").fetchone()[0] == 0
        assert conn.execute("select count(*) from crystals").fetchone()[0] == 0


def test_strict_term_with_search_alias_in_partial_alias_schema_is_skipped(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        _create_partial_alias_schema(conn)
        term_id = _insert_strict_term(conn)
        conn.execute(
            """
            insert into strict_term_aliases(term_id, language, text, kind)
            values (?, 'ja', 'atk buff', 'search_alias')
            """,
            (term_id,),
        )
        conn.commit()

    report = MemoryGraphMigrator(config).run()

    assert report.skipped == {"strict_terms.unsupported_alias": 1}
    with connect(config.database_path) as conn:
        assert conn.execute("select count(*) from concepts").fetchone()[0] == 0
        assert conn.execute("select count(*) from crystals").fetchone()[0] == 0


def test_skipped_strict_term_does_not_remain_pending_in_dry_report(
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

    MemoryGraphMigrator(config).run()
    report = MemoryGraphMigrator.inspect(config)

    assert report.pending.get("strict_terms", 0) == 0


def test_doctor_does_not_warn_for_skipped_strict_term(
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

    MemoryGraphMigrator(config).run()
    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=False)

    assert all(finding.code != "memory-graph-migration-pending" for finding in report["warnings"])


def test_dry_report_counts_missing_crystal_semantic_tag_pairs(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        crystal_id = conn.execute(
            """
            insert into crystals(
              crystal_type,
              text,
              scope_type,
              tags_json,
              strength,
              confidence,
              status,
              created_at,
              updated_at
            )
            values ('lesson', 'Legacy lesson.', 'series', '["ui", "term"]', 0.5, 0.5,
                    'active', ?, ?)
            """,
            (NOW, NOW),
        ).lastrowid
        conn.execute(
            """
            insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
            values (?, 'ui', 0.5, ?)
            """,
            (crystal_id, NOW),
        )
        conn.commit()

    report = MemoryGraphMigrator.inspect(config)

    assert report.pending["crystal_semantic_tags"] == 1


def test_dry_report_counts_missing_crystal_semantic_tag_pairs_from_csv(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        crystal_id = conn.execute(
            """
            insert into crystals(
              crystal_type,
              text,
              scope_type,
              tags_json,
              strength,
              confidence,
              status,
              created_at,
              updated_at
            )
            values ('lesson', 'Legacy lesson.', 'series', 'ui, term', 0.5, 0.5,
                    'active', ?, ?)
            """,
            (NOW, NOW),
        ).lastrowid
        conn.execute(
            """
            insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
            values (?, 'ui', 0.5, ?)
            """,
            (crystal_id, NOW),
        )
        conn.commit()

    report = MemoryGraphMigrator.inspect(config)

    assert report.pending["crystal_semantic_tags"] == 1


def test_dry_report_counts_missing_task_session_semantic_tag_pairs(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table task_sessions (
              id integer primary key,
              tags_json text not null default '[]'
            );
            create table task_session_semantic_tags (
              session_id integer not null,
              semantic_tag text not null,
              primary key (session_id, semantic_tag)
            );
            insert into task_sessions(id, tags_json) values (1, '["ui", "term"]');
            insert into task_session_semantic_tags(session_id, semantic_tag)
            values (1, 'ui');
            """
        )
        conn.commit()

    report = MemoryGraphMigrator.inspect(config)

    assert report.pending["task_session_semantic_tags"] == 1


def test_dry_report_counts_missing_task_session_semantic_tag_pairs_from_csv(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table task_sessions (
              id integer primary key,
              tags text not null default ''
            );
            create table task_session_semantic_tags (
              session_id integer not null,
              semantic_tag text not null,
              primary key (session_id, semantic_tag)
            );
            insert into task_sessions(id, tags) values (1, 'ui, term');
            insert into task_session_semantic_tags(session_id, semantic_tag)
            values (1, 'ui');
            """
        )
        conn.commit()

    report = MemoryGraphMigrator.inspect(config)

    assert report.pending["task_session_semantic_tags"] == 1


def test_dry_report_counts_strict_term_when_ledger_rule_target_is_dangling(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        _insert_strict_term(conn)
        conn.commit()

    MemoryGraphMigrator(config).run()
    with connect(config.database_path) as conn:
        crystal_id = conn.execute(
            """
            select target_id
            from memory_graph_migration_ledger
            where source_table = 'strict_terms'
              and target_table = 'crystals'
            """
        ).fetchone()["target_id"]
        conn.execute("delete from crystal_concepts where crystal_id = ?", (crystal_id,))
        conn.execute("delete from crystals where id = ?", (crystal_id,))
        conn.commit()

    report = MemoryGraphMigrator.inspect(config)

    assert report.pending["strict_terms"] == 1


def test_dry_report_counts_proposal_when_ledger_facet_target_is_dangling(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        proposal_id = conn.execute(
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
              '[]',
              'Translator needs review.',
              'pending',
              ?,
              ?
            )
            """,
            (NOW, NOW),
        ).lastrowid
        conn.commit()

    MemoryGraphMigrator(config).run()
    with connect(config.database_path) as conn:
        facet_id = conn.execute(
            """
            select target_id
            from memory_graph_migration_ledger
            where source_table = 'strict_concept_proposals'
              and source_id = ?
              and target_table = 'concept_facets'
            """,
            (f"{proposal_id}:rendering",),
        ).fetchone()["target_id"]
        conn.execute("delete from concept_facet_language_tags where facet_id = ?", (facet_id,))
        conn.execute("delete from concept_facets where id = ?", (facet_id,))
        conn.commit()

    report = MemoryGraphMigrator.inspect(config)

    assert report.pending["strict_concept_proposals"] == 1


def test_run_repairs_strict_term_ledger_facet_with_wrong_concept(
    config: HieronymusConfig,
) -> None:
    _seed_base(config)
    with connect(config.database_path) as conn:
        term_id = _insert_strict_term(conn)
        conn.commit()

    MemoryGraphMigrator(config).run()
    with connect(config.database_path) as conn:
        original_concept_id = conn.execute(
            """
            select target_id
            from memory_graph_migration_ledger
            where source_table = 'strict_terms'
              and source_id = ?
              and target_table = 'concepts'
            """,
            (str(term_id),),
        ).fetchone()["target_id"]
        source_facet_id = conn.execute(
            """
            select target_id
            from memory_graph_migration_ledger
            where source_table = 'strict_terms'
              and source_id = ?
              and target_table = 'concept_facets'
            """,
            (f"{term_id}:source",),
        ).fetchone()["target_id"]
        other_concept_id = conn.execute(
            """
            insert into concepts(
              canonical_name,
              description,
              scope_type,
              scope_key,
              status,
              confidence,
              created_at,
              updated_at
            )
            values ('Other', '', 'series', 'series:book', 'established', 0.5, ?, ?)
            """,
            (NOW, NOW),
        ).lastrowid
        conn.execute(
            "update concept_facets set concept_id = ? where id = ?",
            (other_concept_id, source_facet_id),
        )
        conn.commit()

    pending_report = MemoryGraphMigrator.inspect(config)
    repair_report = MemoryGraphMigrator(config).run()
    final_report = MemoryGraphMigrator.inspect(config)

    assert pending_report.pending["strict_terms"] == 1
    assert repair_report.created["concept_facets"] == 1
    assert final_report.pending.get("strict_terms", 0) == 0
    with connect(config.database_path) as conn:
        repaired_facet_id = conn.execute(
            """
            select target_id
            from memory_graph_migration_ledger
            where source_table = 'strict_terms'
              and source_id = ?
              and target_table = 'concept_facets'
            """,
            (f"{term_id}:source",),
        ).fetchone()["target_id"]
        repaired_concept_id = conn.execute(
            "select concept_id from concept_facets where id = ?",
            (repaired_facet_id,),
        ).fetchone()["concept_id"]

    assert repaired_facet_id != source_facet_id
    assert repaired_concept_id == original_concept_id


def test_strict_term_generation_skips_when_destination_graph_schema_is_partial(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        _create_partial_alias_schema(conn)
        _insert_strict_term(conn)
        conn.execute(
            """
            create table crystals (
              id integer primary key,
              crystal_type text not null,
              text text not null
            )
            """
        )
        conn.commit()

    report = MemoryGraphMigrator(config).run()

    assert report.skipped == {"generated_graph.incomplete_schema": 1}
    with connect(config.database_path) as conn:
        assert conn.execute("select count(*) from crystals").fetchone()[0] == 0


def test_unsupported_proposal_shape_does_not_remain_pending_in_dry_report(
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
              '["Sense", "Senses"]',
              '[]',
              'Unsupported approved variant.',
              'pending',
              ?,
              ?
            )
            """,
            (NOW, NOW),
        )
        conn.commit()

    run_report = MemoryGraphMigrator(config).run()
    dry_report = MemoryGraphMigrator.inspect(config)

    assert run_report.skipped == {"strict_concept_proposals.unsupported_rule_shape": 1}
    assert dry_report.pending.get("strict_concept_proposals", 0) == 0


def test_strict_term_with_partial_tags_schema_does_not_crash(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        _create_partial_alias_schema(conn)
        conn.execute("drop table strict_term_tags")
        conn.execute("create table strict_term_tags(term_id integer not null)")
        _insert_strict_term(conn, insert_tag=False)
        conn.execute("insert into strict_term_tags(term_id) values (1)")
        conn.commit()

    report = MemoryGraphMigrator(config).run()

    assert report.skipped == {}
    with connect(config.database_path) as conn:
        assert (
            conn.execute("select count(*) from crystals where crystal_type = 'rule'").fetchone()[0]
            == 1
        )


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


def test_migrator_tolerates_partial_concepts_without_status(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table concepts (
              id integer primary key,
              canonical_name text not null
            );
            insert into concepts(id, canonical_name) values (1, 'Legacy');
            """
        )
        conn.commit()

    dry_report = MemoryGraphMigrator.inspect(config)
    run_report = MemoryGraphMigrator(config).run()

    assert dry_report.pending.get("concepts.status", 0) == 0
    assert "concepts.status" not in run_report.updated


def test_migrator_tolerates_partial_series_language_tags_destination(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table series (
              id integer primary key,
              slug text not null unique,
              title text not null,
              default_source_language text not null,
              default_target_language text not null,
              created_at text not null,
              updated_at text not null
            );
            create table series_language_tags (
              series_id integer not null
            );
            """
        )
        conn.execute(
            """
            insert into series(
              id,
              slug,
              title,
              default_source_language,
              default_target_language,
              created_at,
              updated_at
            )
            values (1, 'book', 'Book', 'ja', 'en', ?, ?);
            """,
            (NOW, NOW),
        )
        conn.commit()

    dry_report = MemoryGraphMigrator.inspect(config)
    run_report = MemoryGraphMigrator(config).run()

    assert dry_report.pending.get("series_language_tags", 0) == 0
    assert "series_language_tags" not in run_report.updated


def test_migrator_tolerates_partial_task_session_side_table_destinations(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table task_sessions (
              id integer primary key,
              source_language text not null,
              target_language text not null,
              volume text not null,
              chapter text not null,
              tags_json text not null
            );
            create table task_session_language_tags (
              session_id integer not null
            );
            create table task_session_story_scopes (
              session_id integer not null
            );
            create table task_session_semantic_tags (
              session_id integer not null
            );
            insert into task_sessions(
              id,
              source_language,
              target_language,
              volume,
              chapter,
              tags_json
            )
            values (1, 'ja', 'en', '1', '2', '["ui"]');
            """
        )
        conn.commit()

    dry_report = MemoryGraphMigrator.inspect(config)
    run_report = MemoryGraphMigrator(config).run()

    assert dry_report.pending.get("task_session_language_tags", 0) == 0
    assert dry_report.pending.get("task_session_story_scopes", 0) == 0
    assert dry_report.pending.get("task_session_semantic_tags", 0) == 0
    assert "task_session_language_tags" not in run_report.updated
    assert "task_session_story_scopes" not in run_report.updated
    assert "task_session_semantic_tags" not in run_report.updated


def test_migrator_tolerates_partial_crystal_side_table_destinations(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table crystals (
              id integer primary key,
              source_language text not null,
              target_language text not null,
              tags_json text not null
            );
            create table crystal_language_tags (
              crystal_id integer not null
            );
            create table crystal_semantic_tags (
              crystal_id integer not null
            );
            insert into crystals(id, source_language, target_language, tags_json)
            values (1, 'ja', 'en', '["ui"]');
            """
        )
        conn.commit()

    dry_report = MemoryGraphMigrator.inspect(config)
    run_report = MemoryGraphMigrator(config).run()

    assert dry_report.pending.get("crystal_language_tags", 0) == 0
    assert dry_report.pending.get("crystal_semantic_tags", 0) == 0
    assert "crystal_language_tags" not in run_report.updated
    assert "crystal_semantic_tags" not in run_report.updated


def test_dry_report_skips_partial_crystal_semantic_tags_missing_metadata(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        conn.executescript(
            """
            create table crystals (
              id integer primary key,
              tags_json text not null
            );
            create table crystal_semantic_tags (
              crystal_id integer not null,
              tag text not null
            );
            insert into crystals(id, tags_json) values (1, '["ui"]');
            """
        )
        conn.commit()

    dry_report = MemoryGraphMigrator.inspect(config)
    run_report = MemoryGraphMigrator(config).run()

    assert dry_report.pending.get("crystal_semantic_tags", 0) == 0
    assert "crystal_semantic_tags" not in run_report.updated


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


def _create_partial_alias_schema(conn) -> None:
    conn.executescript(
        """
        create table series (
          id integer primary key,
          slug text not null unique,
          title text not null,
          default_source_language text not null,
          default_target_language text not null,
          created_at text not null,
          updated_at text not null
        );
        create table strict_terms (
          id integer primary key,
          series_slug text not null references series(slug),
          source_language text not null,
          target_language text not null,
          category text not null,
          source_text text not null,
          canonical_translation text not null,
          status text not null,
          notes text not null default '',
          created_at text not null,
          updated_at text not null
        );
        create table strict_term_tags (
          term_id integer not null references strict_terms(id) on delete cascade,
          tag text not null,
          primary key(term_id, tag)
        );
        create table strict_term_aliases (
          id integer primary key,
          term_id integer not null references strict_terms(id) on delete cascade,
          language text not null,
          text text not null,
          kind text not null
        );
        insert into series(
          slug,
          title,
          default_source_language,
          default_target_language,
          created_at,
          updated_at
        )
        values ('book', 'Book', 'ja', 'en', '2026-06-10T00:00:00+00:00',
                '2026-06-10T00:00:00+00:00');
        """
    )
    conn.commit()


def _insert_strict_term(conn, *, insert_tag: bool = True) -> int:
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
    if insert_tag:
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
