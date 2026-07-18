from __future__ import annotations

import errno
import importlib
import json
import re
import sqlite3
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from unittest.mock import patch

import pytest

from hieronymus.db import apply_migration, connect, discover_schema_migrations
from hieronymus.legacy_terms import (
    LegacyTermRetirementBlocked,
    LegacyTermRetirementDisabled,
    _fsync_directory,
    prepare_strict_term_retirement,
    verify_legacy_terms_backup,
    write_legacy_terms_backup,
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
    assert re.fullmatch(
        r"strict-terms-v1-20260719T120000Z-[0-9a-f]{32}\.json",
        report.backup.path.name,
    )
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


def test_rerun_reconciles_changed_legacy_projection_exactly(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    term_id = _term(conn, source="旧名", rendering="Old Rendering")
    conn.execute("insert into strict_term_tags(term_id, tag) values (?, 'old-tag')", (term_id,))
    _alias(conn, term_id, text="旧別名", kind="source_variant", language="ja")
    removed_alias_id = conn.execute(
        """
        insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
        values (?, 'ja', 'obsolete search', 'search_alias', 1)
        """,
        (term_id,),
    ).lastrowid
    forbidden_id = conn.execute(
        """
        insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
        values (?, 'en', 'Old Forbidden', 'forbidden_variant', 1)
        """,
        (term_id,),
    ).lastrowid
    conn.commit()
    prepare_strict_term_retirement(conn, backup_dir)

    targets = {
        (row["source_id"], row["target_table"]): int(row["target_id"])
        for row in conn.execute(
            """
            select source_id, target_table, target_id
            from memory_graph_migration_ledger where source_table = 'strict_terms'
            """
        )
    }
    concept_id = targets[(str(term_id), "concepts")]
    source_facet_id = targets[(f"{term_id}:source", "concept_facets")]
    rendering_facet_id = targets[(f"{term_id}:rendering", "concept_facets")]
    crystal_id = targets[(str(term_id), "crystals")]
    conn.execute(
        """
        update strict_terms set source_text = '新名', canonical_translation = 'New Rendering',
          notes = 'Changed note' where id = ?
        """,
        (term_id,),
    )
    conn.execute("delete from strict_term_tags where term_id = ?", (term_id,))
    conn.execute("insert into strict_term_tags(term_id, tag) values (?, 'new-tag')", (term_id,))
    conn.execute(
        """
        update strict_term_aliases set text = '新別名', language = 'zh'
        where term_id = ? and kind = 'source_variant'
        """,
        (term_id,),
    )
    conn.execute("delete from strict_term_aliases where id = ?", (removed_alias_id,))
    conn.execute(
        "update strict_term_aliases set text = 'New Forbidden' where id = ?", (forbidden_id,)
    )
    conn.execute(
        "update concepts set status = 'archived', confidence = 0.99 where id = ?", (concept_id,)
    )
    conn.execute(
        """
        update concept_facets set value = 'stale source', facet_type = 'former_label',
          language = 'xx', is_canonical = 0, confidence = 0.99 where id = ?
        """,
        (source_facet_id,),
    )
    conn.execute(
        """
        update concept_facets set value = 'stale rendering', facet_type = 'alias',
          language = 'xx', is_canonical = 1 where id = ?
        """,
        (rendering_facet_id,),
    )
    conn.execute(
        "update crystals set status = 'archived', strength = 0.99 where id = ?", (crystal_id,)
    )
    conn.commit()

    result = prepare_strict_term_retirement(conn, backup_dir)

    assert result.coverage.complete
    concept = conn.execute(
        "select canonical_name, description, status, confidence from concepts where id = ?",
        (concept_id,),
    ).fetchone()
    assert tuple(concept) == ("新名", "Changed note", "established", 0.99)
    source_facet = conn.execute(
        "select value, facet_type, language, is_canonical, confidence "
        "from concept_facets where id = ?",
        (source_facet_id,),
    ).fetchone()
    assert tuple(source_facet) == ("新名", "name", "ja", 1, 0.99)
    rendering_facet = conn.execute(
        "select value, facet_type, language, is_canonical from concept_facets where id = ?",
        (rendering_facet_id,),
    ).fetchone()
    assert tuple(rendering_facet) == ("New Rendering", "rendering", "en", 0)
    assert {row["tag"] for row in conn.execute("select tag from concept_semantic_tags")} == {
        "new-tag"
    }
    assert {row["tag"] for row in conn.execute("select tag from crystal_semantic_tags")} == {
        "new-tag"
    }
    crystal = conn.execute(
        "select text, status, strength from crystals where id = ?", (crystal_id,)
    ).fetchone()
    assert tuple(crystal) == (
        "新名 is translated as New Rendering, not New Forbidden.",
        "active",
        0.99,
    )
    assert (
        conn.execute(
            "select 1 from memory_graph_migration_ledger where source_id = ?",
            (f"{term_id}:alias:{removed_alias_id}",),
        ).fetchone()
        is None
    )
    assert (
        conn.execute(
            "select count(*) from concept_facets where value = 'obsolete search'"
        ).fetchone()[0]
        == 0
    )


def test_same_timestamp_backups_are_unique_under_concurrency(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    _term(conn, source="用語", rendering="Term")
    conn.commit()
    database_path = Path(conn.execute("pragma database_list").fetchone()[2])

    def create() -> Path:
        with connect(database_path) as worker_conn:
            return write_legacy_terms_backup(
                worker_conn, backup_dir, timestamp="20260719T120000Z"
            ).path

    with ThreadPoolExecutor(max_workers=4) as executor:
        paths = list(executor.map(lambda _: create(), range(4)))

    assert len(set(paths)) == 4
    assert all(path.parent == backup_dir.resolve() for path in paths)
    assert all(verify_legacy_terms_backup(path)["strict_terms"] for path in paths)


@pytest.mark.parametrize("timestamp", ["../escape", "20260719/120000Z", "bad timestamp"])
def test_backup_rejects_unsafe_timestamp_text(
    legacy_database: tuple[sqlite3.Connection, Path], timestamp: str
) -> None:
    conn, backup_dir = legacy_database

    with pytest.raises(ValueError, match="timestamp"):
        write_legacy_terms_backup(conn, backup_dir, timestamp=timestamp)

    assert not backup_dir.exists()


def test_backup_rejects_symlink_directory_escape(
    legacy_database: tuple[sqlite3.Connection, Path], tmp_path: Path
) -> None:
    conn, backup_dir = legacy_database
    outside = tmp_path / "outside"
    outside.mkdir()
    backup_dir.symlink_to(outside, target_is_directory=True)

    with pytest.raises(LegacyTermRetirementBlocked, match="symlink"):
        write_legacy_terms_backup(conn, backup_dir)

    assert not list(outside.iterdir())


def test_directory_fsync_suppresses_only_unsupported_platform_errors(tmp_path: Path) -> None:
    with patch("hieronymus.legacy_terms.os.open", side_effect=OSError(errno.ENOTSUP, "no fsync")):
        _fsync_directory(tmp_path)

    with patch("hieronymus.legacy_terms.os.open", side_effect=OSError(errno.EIO, "disk error")):
        with pytest.raises(OSError, match="disk error"):
            _fsync_directory(tmp_path)


def test_directory_fsync_io_error_prevents_database_mutation(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    _term(conn, source="用語", rendering="Term")
    conn.commit()

    with (
        patch(
            "hieronymus.legacy_terms._fsync_directory",
            side_effect=OSError(errno.EIO, "durability failure"),
        ),
        pytest.raises(OSError, match="durability failure"),
    ):
        prepare_strict_term_retirement(conn, backup_dir)

    assert conn.execute("select count(*) from concepts").fetchone()[0] == 0
    assert conn.execute("select count(*) from crystals").fetchone()[0] == 0


@pytest.mark.parametrize("stale_json", ["not-json", '{"term":{"status":"stale"}}'])
def test_rerun_reconciles_malformed_or_stale_inactive_audit_snapshot(
    legacy_database: tuple[sqlite3.Connection, Path], stale_json: str
) -> None:
    conn, backup_dir = legacy_database
    term_id = _term(conn, source="廃語", rendering="Retired", status="rejected")
    conn.commit()
    prepare_strict_term_retirement(conn, backup_dir)
    conn.execute(
        "update audit_log set before_json = ? where action = 'strict_term_retirement'",
        (stale_json,),
    )
    conn.execute("update strict_terms set notes = 'Changed inactive note' where id = ?", (term_id,))
    conn.execute("insert into strict_term_tags(term_id, tag) values (?, 'history')", (term_id,))
    _alias(conn, term_id, text="Former", kind="forbidden_variant", language="en")
    conn.commit()

    result = prepare_strict_term_retirement(conn, backup_dir)

    assert result.coverage.complete
    rows = conn.execute(
        "select before_json from audit_log where action = 'strict_term_retirement'"
    ).fetchall()
    assert len(rows) == 1
    snapshot = json.loads(rows[0]["before_json"])
    assert snapshot["term"]["notes"] == "Changed inactive note"
    assert snapshot["tags"] == [{"tag": "history", "term_id": term_id}]
    assert snapshot["aliases"][0]["text"] == "Former"


@pytest.mark.parametrize("broken_table", ["concept_semantic_tags", "memory_graph_migration_ledger"])
def test_incomplete_generated_shape_blocks_with_every_active_term_id(
    legacy_database: tuple[sqlite3.Connection, Path], broken_table: str
) -> None:
    conn, backup_dir = legacy_database
    first_id = _term(conn, source="一", rendering="One")
    second_id = _term(conn, source="二", rendering="Two")
    conn.execute(f"drop table {broken_table}")
    if broken_table == "memory_graph_migration_ledger":
        conn.execute("create table memory_graph_migration_ledger(source_table text)")
    conn.commit()

    with pytest.raises(
        LegacyTermRetirementBlocked,
        match=rf"terms {first_id}, {second_id}.*incomplete",
    ):
        prepare_strict_term_retirement(conn, backup_dir)

    assert conn.execute("select count(*) from concepts").fetchone()[0] == 0


def test_rerun_replaces_alias_relationship_when_kind_changes(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    term_id = _term(conn, source="用語", rendering="Term")
    _alias(conn, term_id, text="別名", kind="source_variant", language="ja")
    conn.commit()
    prepare_strict_term_retirement(conn, backup_dir)
    alias_id = conn.execute(
        "select id from strict_term_aliases where term_id = ?", (term_id,)
    ).fetchone()[0]

    conn.execute(
        """
        update strict_term_aliases
        set kind = 'forbidden_variant', text = 'Wrong', language = 'en'
        where id = ?
        """,
        (alias_id,),
    )
    conn.commit()

    result = prepare_strict_term_retirement(conn, backup_dir)

    assert result.coverage.complete
    relationships = conn.execute(
        """
        select target_table from memory_graph_migration_ledger
        where source_table = 'strict_terms' and source_id = ?
        """,
        (f"{term_id}:alias:{alias_id}",),
    ).fetchall()
    assert [row["target_table"] for row in relationships] == ["crystals"]
    assert (
        conn.execute("select count(*) from concept_facets where value = '別名'").fetchone()[0] == 0
    )


@pytest.mark.parametrize(
    ("text", "language", "detail"),
    [("", "ja", "text"), ("alias", "", "language")],
)
def test_incomplete_alias_blocks_with_term_id(
    legacy_database: tuple[sqlite3.Connection, Path],
    text: str,
    language: str,
    detail: str,
) -> None:
    conn, backup_dir = legacy_database
    term_id = _term(conn, source="用語", rendering="Term")
    _alias(conn, term_id, text=text, kind="source_variant", language=language)
    conn.commit()

    with pytest.raises(LegacyTermRetirementBlocked, match=rf"term {term_id}.*{detail}.*empty"):
        prepare_strict_term_retirement(conn, backup_dir)

    assert conn.execute("select count(*) from concepts").fetchone()[0] == 0


def test_incomplete_audit_shape_blocks_with_inactive_term_id_before_active_mutation(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    active_id = _term(conn, source="用語", rendering="Term")
    inactive_id = _term(conn, source="廃語", rendering="Retired", status="rejected")
    conn.execute("drop table audit_log")
    conn.execute("create table audit_log(id integer primary key, action text)")
    conn.commit()

    with pytest.raises(
        LegacyTermRetirementBlocked,
        match=rf"term {inactive_id}.*audit.*incomplete",
    ):
        prepare_strict_term_retirement(conn, backup_dir)

    assert active_id != inactive_id
    assert conn.execute("select count(*) from concepts").fetchone()[0] == 0
