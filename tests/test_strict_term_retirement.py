from __future__ import annotations

import importlib
import json
import sqlite3
from pathlib import Path
from unittest.mock import patch

import pytest

from hieronymus.db import apply_migration, connect, discover_schema_migrations
from hieronymus.legacy_terms import (
    LegacyTermRetirementBlocked,
    LegacyTermRetirementDisabled,
    prepare_strict_term_retirement,
    verify_legacy_terms_backup,
)

NOW = "2026-07-19T12:00:00+00:00"


@pytest.fixture
def legacy_database(tmp_path: Path) -> tuple[sqlite3.Connection, Path]:
    database_path = tmp_path / "hieronymus.sqlite"
    conn = connect(database_path)
    apply_migration(conn, "global.sql")
    conn.execute(
        """
        insert into series(
          slug, title, default_source_language, default_target_language, created_at, updated_at
        ) values ('book', 'Book', 'ja', 'en', ?, ?)
        """,
        (NOW, NOW),
    )
    conn.commit()
    try:
        yield conn, tmp_path / "backups"
    finally:
        conn.close()


def _term(
    conn: sqlite3.Connection,
    *,
    source: str,
    rendering: str,
    status: str = "approved",
) -> int:
    term_id = conn.execute(
        """
        insert into strict_terms(
          series_slug, source_language, target_language, category, source_text,
          canonical_translation, status, notes, created_at, updated_at
        ) values ('book', 'ja', 'en', 'skill', ?, ?, ?, 'Legacy note', ?, ?)
        """,
        (source, rendering, status, NOW, NOW),
    ).lastrowid
    return int(term_id)


def _alias(
    conn: sqlite3.Connection,
    term_id: int,
    *,
    text: str,
    kind: str,
    language: str,
    case_sensitive: bool = True,
) -> None:
    conn.execute(
        """
        insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
        values (?, ?, ?, ?, ?)
        """,
        (term_id, language, text, kind, int(case_sensitive)),
    )


def test_backup_is_versioned_complete_verified_and_atomically_published(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    approved_id = _term(conn, source="攻撃力上昇", rendering="ATK Up")
    rejected_id = _term(conn, source="旧称", rendering="Old Name", status="rejected")
    conn.execute("insert into strict_term_tags(term_id, tag) values (?, 'skill')", (approved_id,))
    _alias(
        conn,
        approved_id,
        text="Attack Up",
        kind="forbidden_variant",
        language="en",
    )
    conn.commit()

    with (
        patch("hieronymus.legacy_terms.os.fsync", wraps=__import__("os").fsync) as fsync,
        patch("hieronymus.legacy_terms.os.replace", wraps=__import__("os").replace) as replace,
    ):
        report = prepare_strict_term_retirement(
            conn,
            backup_dir,
            timestamp="20260719T120000Z",
        )

    payload = verify_legacy_terms_backup(report.backup.path)
    assert report.backup.path.name == "strict-terms-v1-20260719T120000Z.json"
    assert payload["format"] == "hieronymus.strict-terms-backup"
    assert payload["version"] == 1
    assert {row["id"] for row in payload["strict_terms"]} == {approved_id, rejected_id}
    assert payload["strict_term_tags"] == [{"tag": "skill", "term_id": approved_id}]
    assert payload["strict_term_aliases"][0]["text"] == "Attack Up"
    assert payload["checksum"] == report.backup.checksum
    assert fsync.call_count >= 1
    replace.assert_called_once()
    assert not list(backup_dir.glob("*.tmp"))


def test_preparation_maps_active_graph_aliases_tags_provenance_and_audits_inactive(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    active_id = _term(conn, source="攻撃力上昇", rendering="ATK Up", status="active")
    inactive_id = _term(conn, source="廃語", rendering="Retired", status="inactive")
    conn.execute("insert into strict_term_tags(term_id, tag) values (?, 'skill')", (active_id,))
    _alias(conn, active_id, text="攻撃バフ", kind="source_variant", language="ja")
    _alias(conn, active_id, text="ATK Up", kind="approved_variant", language="en")
    _alias(conn, active_id, text="Attack Up", kind="forbidden_variant", language="en")
    conn.commit()

    result = prepare_strict_term_retirement(conn, backup_dir)

    assert result.coverage.total_terms == 2
    assert result.coverage.active_terms == 1
    assert result.coverage.migrated_terms == 1
    assert result.coverage.inactive_audited == 1
    assert result.coverage.blocked_term_ids == ()
    assert result.coverage.complete
    facets = {
        (row["facet_type"], row["value"])
        for row in conn.execute("select facet_type, value from concept_facets")
    }
    assert facets == {
        ("name", "攻撃力上昇"),
        ("alias", "攻撃バフ"),
        ("rendering", "ATK Up"),
    }
    assert conn.execute("select tag from concept_semantic_tags").fetchone()[0] == "skill"
    assert conn.execute("select tag from crystal_semantic_tags").fetchone()[0] == "skill"
    crystal = conn.execute("select text, status from crystals").fetchone()
    assert tuple(crystal) == ("攻撃力上昇 is translated as ATK Up, not Attack Up.", "active")
    ledger_ids = {
        row["source_id"]
        for row in conn.execute(
            "select source_id from memory_graph_migration_ledger "
            "where source_table = 'strict_terms'"
        )
    }
    assert {str(active_id), f"{active_id}:source", f"{active_id}:rendering"} <= ledger_ids
    assert sum(source_id.startswith(f"{active_id}:alias:") for source_id in ledger_ids) == 3
    audit = conn.execute(
        "select entity_id, before_json from audit_log where action = 'strict_term_retirement'"
    ).fetchone()
    assert audit["entity_id"] == str(inactive_id)
    assert json.loads(audit["before_json"])["term"]["status"] == "inactive"
    assert conn.execute("select count(*) from strict_terms").fetchone()[0] == 2


def test_unsupported_alias_blocks_with_term_id_before_any_lasting_mutation(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    term_id = _term(conn, source="攻撃力上昇", rendering="ATK Up")
    _alias(
        conn,
        term_id,
        text="atk up",
        kind="forbidden_variant",
        language="en",
        case_sensitive=False,
    )
    conn.commit()

    with pytest.raises(LegacyTermRetirementBlocked, match=rf"term {term_id}.*case-sensitive"):
        prepare_strict_term_retirement(conn, backup_dir)

    backups = list(backup_dir.glob("*.json"))
    assert len(backups) == 1
    assert verify_legacy_terms_backup(backups[0])["strict_terms"][0]["id"] == term_id
    assert conn.execute("select count(*) from concepts").fetchone()[0] == 0
    assert conn.execute("select count(*) from crystals").fetchone()[0] == 0
    assert conn.execute("select count(*) from audit_log").fetchone()[0] == 0


def test_rerun_reconciles_provenance_without_duplicate_graph_or_audit(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    _term(conn, source="攻撃力上昇", rendering="ATK Up")
    _term(conn, source="廃語", rendering="Retired", status="rejected")
    conn.commit()

    first = prepare_strict_term_retirement(conn, backup_dir, timestamp="20260719T120000Z")
    counts_before = {
        table: conn.execute(f"select count(*) from {table}").fetchone()[0]
        for table in ("concepts", "concept_facets", "crystals", "audit_log")
    }
    second = prepare_strict_term_retirement(conn, backup_dir, timestamp="20260719T120100Z")

    assert second.coverage == first.coverage
    assert {
        table: conn.execute(f"select count(*) from {table}").fetchone()[0]
        for table in counts_before
    } == counts_before


def test_injected_mid_conversion_failure_rolls_back_database_but_keeps_backup(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    first_id = _term(conn, source="一", rendering="One")
    _term(conn, source="二", rendering="Two")
    conn.commit()

    def fail_after_first(term_id: int) -> None:
        if term_id == first_id:
            raise RuntimeError("injected failure")

    with pytest.raises(RuntimeError, match="injected failure"):
        prepare_strict_term_retirement(conn, backup_dir, failure_hook=fail_after_first)

    backup = next(backup_dir.glob("*.json"))
    assert len(verify_legacy_terms_backup(backup)["strict_terms"]) == 2
    assert conn.execute("select count(*) from concepts").fetchone()[0] == 0
    assert conn.execute("select count(*) from concept_facets").fetchone()[0] == 0
    assert conn.execute("select count(*) from crystals").fetchone()[0] == 0
    assert conn.execute("select count(*) from memory_graph_migration_ledger").fetchone()[0] == 0


def test_orchestrator_rejects_ambiguous_existing_transaction_ownership(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    _term(conn, source="攻撃力上昇", rendering="ATK Up")

    with pytest.raises(sqlite3.ProgrammingError, match="transaction ownership"):
        prepare_strict_term_retirement(conn, backup_dir)

    conn.rollback()
    assert not backup_dir.exists()


def test_drop_phase_is_importable_but_disabled_and_not_sql_discovered(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, _ = legacy_database
    module = importlib.import_module("hieronymus.migrations.versions.0002_retire_strict_terms")

    assert module.DROP_PHASE_ENABLED is False
    with pytest.raises(LegacyTermRetirementDisabled, match="Task 3"):
        module.drop_legacy_tables(conn)
    assert [migration.version for migration in discover_schema_migrations()] == ["0001"]
    assert conn.execute(
        "select 1 from sqlite_master where type = 'table' and name = 'strict_terms'"
    ).fetchone()
