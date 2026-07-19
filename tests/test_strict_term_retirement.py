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

from hieronymus.db import apply_migration, connect, discover_schema_migrations, ensure_schema
from hieronymus.legacy_terms import (
    LegacyTermRetirementBlocked,
    _fsync_directory,
    check_strict_term_retirement_parity,
    prepare_strict_term_retirement,
    verify_legacy_terms_backup,
    write_legacy_terms_backup,
)
from hieronymus.memory_migration import (
    _canonical_alias_source,
    _is_canonical_facet_source,
    convert_strict_terms,
)

NOW = "2026-07-19T12:00:00+00:00"


@pytest.fixture
def legacy_database(tmp_path: Path) -> tuple[sqlite3.Connection, Path]:
    database_path = tmp_path / "hieronymus.sqlite"
    conn = connect(database_path)
    apply_migration(conn, "global.sql")
    conn.executescript(
        """
        create table strict_terms (
          id integer primary key, series_slug text not null references series(slug),
          source_language text not null, target_language text not null, category text not null,
          source_text text not null, canonical_translation text not null, status text not null,
          notes text not null default '', created_at text not null, updated_at text not null
        );
        create table strict_term_tags (
          term_id integer not null references strict_terms(id) on delete cascade,
          tag text not null, primary key(term_id, tag)
        );
        create table strict_term_aliases (
          id integer primary key,
          term_id integer not null references strict_terms(id) on delete cascade,
          language text not null, text text not null, kind text not null,
          case_sensitive integer not null default 1
        );
        create virtual table strict_terms_fts using fts5(
          source_text, canonical_translation, notes,
          content='strict_terms', content_rowid='id'
        );
        """
    )
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


def _recreate_relationship_ownership_schema(
    conn: sqlite3.Connection,
    *,
    source_key: tuple[str, ...] | None,
    edge_key: tuple[str, ...] | None,
) -> None:
    columns = (
        "source_table",
        "source_id",
        "relationship_type",
        "owner_type",
        "owner_id",
        "related_type",
        "related_id",
        "value",
    )
    definitions = [
        "source_table text not null",
        "source_id text not null",
        "relationship_type text not null",
        "owner_type text not null",
        "owner_id integer not null",
        "related_type text not null",
        "related_id integer not null",
        "value text not null",
        "created_at text not null",
    ]
    for key in (source_key, edge_key):
        if key is not None:
            assert set(key) <= set(columns)
            definitions.append(f"unique({', '.join(key)})")
    conn.execute("drop table strict_term_retirement_relationship_ownership")
    conn.execute(
        "create table strict_term_retirement_relationship_ownership(" + ", ".join(definitions) + ")"
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


def test_python_retirement_migration_is_discovered_and_drops_legacy_once(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    _term(conn, source="攻撃力上昇", rendering="ATK Up")
    conn.commit()
    module = importlib.import_module("hieronymus.migrations.versions.0002_retire_strict_terms")

    assert module.DROP_PHASE_ENABLED is True
    assert [migration.version for migration in discover_schema_migrations()] == ["0001", "0002"]
    ensure_schema(conn)
    migration_backup_dir = backup_dir / "strict-terms"
    first_backups = list(migration_backup_dir.glob("*.json"))
    ensure_schema(conn)

    names = {row["name"] for row in conn.execute("select name from sqlite_master")}
    legacy_tables = {"strict_terms", "strict_term_tags", "strict_term_aliases", "strict_terms_fts"}
    assert not legacy_tables & names
    assert len(first_backups) == 1
    assert list(migration_backup_dir.glob("*.json")) == first_backups


def test_python_retirement_drop_failure_rolls_back_schema_and_data(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    term_id = _term(conn, source="攻撃力上昇", rendering="ATK Up")
    conn.commit()
    module = importlib.import_module("hieronymus.migrations.versions.0002_retire_strict_terms")

    def fail_during_drop(connection: sqlite3.Connection) -> None:
        connection.execute("drop table strict_terms_fts")
        raise RuntimeError("injected drop failure")

    with (
        patch.object(module, "drop_legacy_tables", side_effect=fail_during_drop),
        pytest.raises(RuntimeError, match="injected drop failure"),
    ):
        ensure_schema(conn)

    names = {row["name"] for row in conn.execute("select name from sqlite_master")}
    assert {"strict_terms", "strict_term_tags", "strict_term_aliases", "strict_terms_fts"} <= names
    assert conn.execute("select id from strict_terms").fetchone()[0] == term_id
    assert conn.execute("select 1 from schema_migrations where version = '0002'").fetchone() is None
    assert len(list((backup_dir / "strict-terms").glob("*.json"))) == 1


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
            "hieronymus.legacy_terms._fsync_directory_descriptor",
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
    historical = conn.execute(
        """
        select id, before_json, created_at from audit_log
        where action = 'strict_term_retirement'
        """
    ).fetchone()
    conn.execute("update strict_terms set notes = 'Changed inactive note' where id = ?", (term_id,))
    conn.execute("insert into strict_term_tags(term_id, tag) values (?, 'history')", (term_id,))
    _alias(conn, term_id, text="Former", kind="forbidden_variant", language="en")
    conn.commit()

    result = prepare_strict_term_retirement(conn, backup_dir)

    assert result.coverage.complete
    rows = conn.execute(
        "select id, before_json, created_at from audit_log "
        "where action = 'strict_term_retirement' order by id"
    ).fetchall()
    assert len(rows) == 2
    assert tuple(rows[0]) == tuple(historical)
    snapshot = json.loads(rows[1]["before_json"])
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


def test_unowned_ledger_targets_are_preserved_and_repointed_to_owned_replacements(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    term_id = _term(conn, source="旧名", rendering="Old")
    conn.execute("insert into strict_term_tags(term_id, tag) values (?, 'legacy')", (term_id,))
    conn.commit()
    convert_strict_terms(conn)
    conn.commit()
    conn.execute("delete from strict_term_retirement_relationship_ownership")
    conn.execute("delete from strict_term_retirement_ownership")
    conn.commit()
    old_targets = {
        (row["source_id"], row["target_table"]): int(row["target_id"])
        for row in conn.execute(
            """
            select source_id, target_table, target_id from memory_graph_migration_ledger
            where source_table = 'strict_terms'
            """
        )
    }
    old_concept = old_targets[(str(term_id), "concepts")]
    old_crystal = old_targets[(str(term_id), "crystals")]
    old_source = old_targets[(f"{term_id}:source", "concept_facets")]
    conn.execute(
        "update concepts set description = 'user-owned', status = 'archived', "
        "confidence = 0.99 where id = ?",
        (old_concept,),
    )
    conn.execute(
        "update concept_facets set value = 'user facet', language = 'zz' where id = ?",
        (old_source,),
    )
    user_concept = conn.execute(
        """
        insert into concepts(canonical_name, description, scope_type, scope_key, status,
          confidence, created_at, updated_at)
        values ('User', 'User link target', 'series', 'series:book', 'established', 1, ?, ?)
        """,
        (NOW, NOW),
    ).lastrowid
    conn.execute(
        """
        insert into crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at)
        values (?, ?, 'mentions', 1, ?)
        """,
        (old_crystal, user_concept, NOW),
    )
    conn.execute(
        "update strict_terms set source_text = '新名', canonical_translation = 'New' where id = ?",
        (term_id,),
    )
    conn.commit()

    result = prepare_strict_term_retirement(conn, backup_dir)

    assert result.coverage.complete
    new_targets = {
        (row["source_id"], row["target_table"]): int(row["target_id"])
        for row in conn.execute(
            """
            select source_id, target_table, target_id from memory_graph_migration_ledger
            where source_table = 'strict_terms'
            """
        )
    }
    assert new_targets[(str(term_id), "concepts")] != old_concept
    assert new_targets[(str(term_id), "crystals")] != old_crystal
    assert tuple(
        conn.execute(
            "select description, status, confidence from concepts where id = ?", (old_concept,)
        ).fetchone()
    ) == ("user-owned", "archived", 0.99)
    assert tuple(
        conn.execute(
            "select value, language from concept_facets where id = ?", (old_source,)
        ).fetchone()
    ) == ("user facet", "zz")
    assert conn.execute(
        """
        select 1 from crystal_concepts
        where crystal_id = ? and concept_id = ? and link_type = 'mentions'
        """,
        (old_crystal, user_concept),
    ).fetchone()
    owned = {
        (row["object_type"], int(row["object_id"]))
        for row in conn.execute(
            "select object_type, object_id from strict_term_retirement_ownership"
        )
    }
    assert ("concepts", new_targets[(str(term_id), "concepts")]) in owned
    assert ("crystals", new_targets[(str(term_id), "crystals")]) in owned


def test_rerun_preserves_user_relationships_on_owned_projection(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    term_id = _term(conn, source="用語", rendering="Term")
    conn.execute("insert into strict_term_tags(term_id, tag) values (?, 'old')", (term_id,))
    conn.commit()
    prepare_strict_term_retirement(conn, backup_dir)
    targets = {
        (row["source_id"], row["target_table"]): int(row["target_id"])
        for row in conn.execute(
            "select source_id, target_table, target_id from memory_graph_migration_ledger"
        )
    }
    concept_id = targets[(str(term_id), "concepts")]
    crystal_id = targets[(str(term_id), "crystals")]
    source_facet_id = targets[(f"{term_id}:source", "concept_facets")]
    user_concept = conn.execute(
        """
        insert into concepts(canonical_name, description, scope_type, scope_key, status,
          confidence, created_at, updated_at)
        values ('User', '', 'series', 'series:book', 'established', 1, ?, ?)
        """,
        (NOW, NOW),
    ).lastrowid
    conn.execute(
        "insert into concept_semantic_tags values (?, 'user-tag', 1, ?)", (concept_id, NOW)
    )
    conn.execute(
        "insert into crystal_semantic_tags values (?, 'user-crystal-tag', 1, ?)",
        (crystal_id, NOW),
    )
    conn.execute(
        "insert into concept_facet_language_tags values (?, 'user-lang')", (source_facet_id,)
    )
    conn.execute("insert into crystal_language_tags values (?, 'user-crystal-lang')", (crystal_id,))
    conn.execute(
        "insert into crystal_concepts values (?, ?, 'mentions', 1, ?)",
        (crystal_id, user_concept, NOW),
    )
    conn.execute("delete from strict_term_tags where term_id = ?", (term_id,))
    conn.execute("insert into strict_term_tags values (?, 'new')", (term_id,))
    conn.commit()

    result = prepare_strict_term_retirement(conn, backup_dir)

    assert result.coverage.complete
    assert {
        row["tag"]
        for row in conn.execute(
            "select tag from concept_semantic_tags where concept_id = ?", (concept_id,)
        )
    } == {"new", "user-tag"}
    assert {
        row["tag"]
        for row in conn.execute(
            "select tag from crystal_semantic_tags where crystal_id = ?", (crystal_id,)
        )
    } == {"new", "user-crystal-tag"}
    assert {
        row["language_tag"]
        for row in conn.execute(
            "select language_tag from concept_facet_language_tags where facet_id = ?",
            (source_facet_id,),
        )
    } == {"ja", "user-lang"}
    assert {
        row["language_tag"]
        for row in conn.execute(
            "select language_tag from crystal_language_tags where crystal_id = ?", (crystal_id,)
        )
    } == {"ja", "en", "user-crystal-lang"}
    assert conn.execute(
        "select 1 from crystal_concepts where crystal_id = ? and concept_id = ? "
        "and link_type = 'mentions'",
        (crystal_id, user_concept),
    ).fetchone()
    owned_links = {
        (
            row["owner_type"],
            int(row["owner_id"]),
            row["related_type"],
            int(row["related_id"]),
            row["value"],
        )
        for row in conn.execute(
            """
            select owner_type, owner_id, related_type, related_id, value
            from strict_term_retirement_relationship_ownership
            where source_table = 'strict_terms' and source_id = ?
              and relationship_type = 'crystal_concepts'
            """,
            (str(term_id),),
        )
    }
    assert owned_links == {("crystals", crystal_id, "concepts", concept_id, "defines")}


def test_backup_filename_identity_is_authenticated_and_rename_tamper_blocks(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    backup = write_legacy_terms_backup(conn, backup_dir, timestamp="20260719T120000Z")
    payload = verify_legacy_terms_backup(backup.path)

    assert payload["timestamp"] == "20260719T120000Z"
    assert re.fullmatch(r"[0-9a-f]{32}", payload["backup_id"])
    assert payload["backup_id"] in backup.path.name
    tampered = backup.path.with_name(backup.path.name.replace(payload["backup_id"], "0" * 32))
    backup.path.rename(tampered)
    with pytest.raises(LegacyTermRetirementBlocked, match="filename identity"):
        verify_legacy_terms_backup(tampered)


def test_descriptor_pinned_backup_fails_safely_on_directory_symlink_swap(
    legacy_database: tuple[sqlite3.Connection, Path], tmp_path: Path
) -> None:
    conn, backup_dir = legacy_database
    backup_dir.mkdir()
    moved = tmp_path / "moved-original"
    outside = tmp_path / "outside"
    outside.mkdir()
    real_open = __import__("os").open
    swapped = False

    def swapping_open(path, flags, *args, **kwargs):
        nonlocal swapped
        descriptor = real_open(path, flags, *args, **kwargs)
        if not swapped and Path(path) == backup_dir.resolve(strict=False):
            swapped = True
            backup_dir.rename(moved)
            backup_dir.symlink_to(outside, target_is_directory=True)
        return descriptor

    with (
        patch("hieronymus.legacy_terms.os.open", side_effect=swapping_open),
        pytest.raises(LegacyTermRetirementBlocked, match="changed|symlink"),
    ):
        write_legacy_terms_backup(conn, backup_dir)

    assert not list(outside.iterdir())


@pytest.mark.parametrize("table", ["strict_term_tags", "strict_term_aliases"])
def test_malformed_relationship_schema_blocks_with_all_affected_term_ids(
    legacy_database: tuple[sqlite3.Connection, Path], table: str
) -> None:
    conn, backup_dir = legacy_database
    first_id = _term(conn, source="一", rendering="One")
    second_id = _term(conn, source="二", rendering="Two")
    conn.execute(f"drop table {table}")
    if table == "strict_term_tags":
        conn.execute("create table strict_term_tags(term_id integer)")
    else:
        conn.execute(
            "create table strict_term_aliases(term_id integer, language text, text text, kind text)"
        )
    conn.commit()

    with pytest.raises(
        LegacyTermRetirementBlocked,
        match=rf"terms {first_id}, {second_id}.*{table}.*incomplete",
    ):
        prepare_strict_term_retirement(conn, backup_dir)


def test_ledger_without_required_uniqueness_blocks_with_all_active_term_ids(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    first_id = _term(conn, source="一", rendering="One")
    second_id = _term(conn, source="二", rendering="Two")
    conn.execute("drop table memory_graph_migration_ledger")
    conn.execute(
        """
        create table memory_graph_migration_ledger(
          source_table text, source_id text, target_table text, target_id integer
        )
        """
    )
    conn.commit()

    with pytest.raises(
        LegacyTermRetirementBlocked,
        match=rf"terms {first_id}, {second_id}.*ledger.*unique",
    ):
        prepare_strict_term_retirement(conn, backup_dir)


def test_alias_ids_must_be_stable_and_unique_for_every_active_term(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    first_id = _term(conn, source="一", rendering="One")
    second_id = _term(conn, source="二", rendering="Two")
    conn.execute("drop table strict_term_aliases")
    conn.execute(
        """
        create table strict_term_aliases(
          id integer, term_id integer, language text, text text, kind text,
          case_sensitive integer
        )
        """
    )
    conn.commit()

    with pytest.raises(
        LegacyTermRetirementBlocked,
        match=rf"terms {first_id}, {second_id}.*stable unique alias IDs",
    ):
        prepare_strict_term_retirement(conn, backup_dir)


def test_malformed_ownership_schema_blocks_before_graph_mutation(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    first_id = _term(conn, source="一", rendering="One")
    second_id = _term(conn, source="二", rendering="Two")
    conn.execute("create table strict_term_retirement_ownership(source_id text)")
    conn.commit()

    with pytest.raises(
        LegacyTermRetirementBlocked,
        match=rf"terms {first_id}, {second_id}.*ownership.*incomplete",
    ):
        prepare_strict_term_retirement(conn, backup_dir)

    assert conn.execute("select count(*) from concepts").fetchone()[0] == 0


def test_cross_term_relationship_ownership_spoof_blocks_before_any_deletion(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    first_id = _term(conn, source="一", rendering="One")
    second_id = _term(conn, source="二", rendering="Two")
    conn.execute("insert into strict_term_tags values (?, 'first')", (first_id,))
    conn.execute("insert into strict_term_tags values (?, 'second')", (second_id,))
    conn.commit()
    prepare_strict_term_retirement(conn, backup_dir)
    first_concept = conn.execute(
        """
        select target_id from memory_graph_migration_ledger
        where source_table = 'strict_terms' and source_id = ? and target_table = 'concepts'
        """,
        (str(first_id),),
    ).fetchone()[0]
    conn.execute(
        """
        update strict_term_retirement_relationship_ownership
        set source_id = ?
        where source_table = 'strict_terms' and source_id = ?
          and relationship_type = 'concept_semantic_tags'
        """,
        (str(second_id), str(first_id)),
    )
    conn.execute("delete from strict_term_tags where term_id = ?", (second_id,))
    conn.commit()
    before = tuple(
        conn.execute(
            "select concept_id, tag from concept_semantic_tags order by concept_id, tag"
        ).fetchall()
    )

    with pytest.raises(
        LegacyTermRetirementBlocked,
        match=rf"terms {first_id}, {second_id}.*ownership integrity",
    ):
        prepare_strict_term_retirement(conn, backup_dir)

    assert (
        tuple(
            conn.execute(
                "select concept_id, tag from concept_semantic_tags order by concept_id, tag"
            ).fetchall()
        )
        == before
    )
    assert conn.execute(
        "select 1 from concept_semantic_tags where concept_id = ? and tag = 'first'",
        (first_concept,),
    ).fetchone()


def test_stale_alias_cleanup_preserves_facet_referenced_by_another_ledger(
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
    source_id = f"{term_id}:alias:{alias_id}"
    facet_id = conn.execute(
        """
        select target_id from memory_graph_migration_ledger
        where source_table = 'strict_terms' and source_id = ?
          and target_table = 'concept_facets'
        """,
        (source_id,),
    ).fetchone()[0]
    conn.execute(
        """
        insert into memory_graph_migration_ledger(
          source_table, source_id, target_table, target_id, created_at
        ) values ('strict_concept_proposals', 'shared-alias', 'concept_facets', ?, ?)
        """,
        (facet_id, NOW),
    )
    conn.execute("delete from strict_term_aliases where id = ?", (alias_id,))
    conn.commit()

    result = prepare_strict_term_retirement(conn, backup_dir)

    assert result.coverage.complete
    assert conn.execute("select 1 from concept_facets where id = ?", (facet_id,)).fetchone()
    assert conn.execute(
        """
        select 1 from memory_graph_migration_ledger
        where source_table = 'strict_concept_proposals' and source_id = 'shared-alias'
          and target_table = 'concept_facets' and target_id = ?
        """,
        (facet_id,),
    ).fetchone()
    assert not conn.execute(
        """
        select 1 from memory_graph_migration_ledger
        where source_table = 'strict_terms' and source_id = ?
        """,
        (source_id,),
    ).fetchone()


def test_missing_owned_relationship_blocks_and_parity_cannot_report_complete(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    first_id = _term(conn, source="一", rendering="One")
    second_id = _term(conn, source="二", rendering="Two")
    conn.execute("insert into strict_term_tags values (?, 'owned')", (first_id,))
    conn.commit()
    first = prepare_strict_term_retirement(conn, backup_dir)
    payload = verify_legacy_terms_backup(first.backup.path)
    concept_id = conn.execute(
        """
        select target_id from memory_graph_migration_ledger
        where source_table = 'strict_terms' and source_id = ? and target_table = 'concepts'
        """,
        (str(first_id),),
    ).fetchone()[0]
    conn.execute(
        "delete from concept_semantic_tags where concept_id = ? and tag = 'owned'",
        (concept_id,),
    )
    conn.commit()

    coverage = check_strict_term_retirement_parity(
        conn,
        payload,
        migrated_terms=2,
        inactive_audited=0,
    )

    assert not coverage.complete
    assert coverage.blocked_term_ids == (first_id, second_id)
    with pytest.raises(
        LegacyTermRetirementBlocked,
        match=rf"terms {first_id}, {second_id}.*ownership integrity",
    ):
        prepare_strict_term_retirement(conn, backup_dir)


_RELATIONSHIP_SOURCE_KEY = (
    "source_table",
    "source_id",
    "relationship_type",
    "owner_type",
    "owner_id",
    "related_type",
    "related_id",
    "value",
)
_RELATIONSHIP_EDGE_KEY = (
    "relationship_type",
    "owner_type",
    "owner_id",
    "related_type",
    "related_id",
    "value",
)


@pytest.mark.parametrize(
    ("case", "source_key", "edge_key", "blocks"),
    [
        ("neither", None, None, True),
        ("source-only", _RELATIONSHIP_SOURCE_KEY, None, True),
        ("edge-only", None, _RELATIONSHIP_EDGE_KEY, True),
        (
            "wrong-order",
            ("source_id", "source_table", *_RELATIONSHIP_SOURCE_KEY[2:]),
            _RELATIONSHIP_EDGE_KEY,
            True,
        ),
        ("exact", _RELATIONSHIP_SOURCE_KEY, _RELATIONSHIP_EDGE_KEY, False),
    ],
)
def test_relationship_ownership_requires_both_exact_ordered_unique_keys(
    legacy_database: tuple[sqlite3.Connection, Path],
    case: str,
    source_key: tuple[str, ...] | None,
    edge_key: tuple[str, ...] | None,
    blocks: bool,
) -> None:
    del case
    conn, backup_dir = legacy_database
    first_id = _term(conn, source="一", rendering="One")
    second_id = _term(conn, source="二", rendering="Two")
    conn.commit()
    prepare_strict_term_retirement(conn, backup_dir)
    _recreate_relationship_ownership_schema(conn, source_key=source_key, edge_key=edge_key)
    conn.commit()
    before = conn.execute("select count(*) from concepts").fetchone()[0]

    if blocks:
        with pytest.raises(
            LegacyTermRetirementBlocked,
            match=rf"terms {first_id}, {second_id}.*relationship ownership.*unique",
        ):
            prepare_strict_term_retirement(conn, backup_dir)
        assert conn.execute("select count(*) from concepts").fetchone()[0] == before
    else:
        assert prepare_strict_term_retirement(conn, backup_dir).coverage.complete


@pytest.mark.parametrize(
    ("kind", "target_table"),
    [("source_variant", "concept_facets"), ("forbidden_variant", "crystals")],
)
def test_cross_term_existing_alias_id_blocks_even_with_consistent_provenance(
    legacy_database: tuple[sqlite3.Connection, Path], kind: str, target_table: str
) -> None:
    conn, backup_dir = legacy_database
    first_id = _term(conn, source="一", rendering="One")
    second_id = _term(conn, source="二", rendering="Two")
    _alias(conn, first_id, text="first alias", kind=kind, language="en")
    _alias(conn, second_id, text="second alias", kind=kind, language="en")
    conn.commit()
    prepare_strict_term_retirement(conn, backup_dir)
    aliases = {
        int(row["term_id"]): int(row["id"])
        for row in conn.execute("select id, term_id from strict_term_aliases")
    }
    old_source = f"{first_id}:alias:{aliases[first_id]}"
    forged_source = f"{first_id}:alias:{aliases[second_id]}"
    conn.execute(
        """
        update memory_graph_migration_ledger set source_id = ?
        where source_table = 'strict_terms' and source_id = ? and target_table = ?
        """,
        (forged_source, old_source, target_table),
    )
    if target_table == "concept_facets":
        conn.execute(
            """
            update strict_term_retirement_ownership set source_id = ?
            where source_table = 'strict_terms' and source_id = ?
              and object_type = 'concept_facets'
            """,
            (forged_source, old_source),
        )
        conn.execute(
            """
            update strict_term_retirement_relationship_ownership set source_id = ?
            where source_table = 'strict_terms' and source_id = ?
            """,
            (forged_source, old_source),
        )
    if conn.execute(
        """
        select 1 from sqlite_master
        where type = 'table' and name = 'strict_term_retirement_alias_provenance'
        """
    ).fetchone():
        conn.execute(
            """
            update strict_term_retirement_alias_provenance
            set source_id = ?, alias_id = ?
            where source_table = 'strict_terms' and source_id = ?
            """,
            (forged_source, aliases[second_id], old_source),
        )
    conn.commit()

    with pytest.raises(
        LegacyTermRetirementBlocked,
        match=rf"terms {first_id}, {second_id}.*alias ownership",
    ):
        prepare_strict_term_retirement(conn, backup_dir)


def test_genuinely_stale_alias_cleanup_uses_durable_same_source_provenance(
    legacy_database: tuple[sqlite3.Connection, Path],
) -> None:
    conn, backup_dir = legacy_database
    term_id = _term(conn, source="用語", rendering="Term")
    _alias(conn, term_id, text="stale", kind="source_variant", language="en")
    conn.commit()
    prepare_strict_term_retirement(conn, backup_dir)
    alias_id = conn.execute(
        "select id from strict_term_aliases where term_id = ?", (term_id,)
    ).fetchone()[0]
    source_id = f"{term_id}:alias:{alias_id}"
    facet_id = conn.execute(
        """
        select target_id from memory_graph_migration_ledger
        where source_table = 'strict_terms' and source_id = ?
          and target_table = 'concept_facets'
        """,
        (source_id,),
    ).fetchone()[0]
    conn.execute("delete from strict_term_aliases where id = ?", (alias_id,))
    conn.commit()

    result = prepare_strict_term_retirement(conn, backup_dir)

    assert result.coverage.complete
    assert not conn.execute("select 1 from concept_facets where id = ?", (facet_id,)).fetchone()
    assert not conn.execute(
        """
        select 1 from memory_graph_migration_ledger
        where source_table = 'strict_terms' and source_id = ?
        """,
        (source_id,),
    ).fetchone()


@pytest.mark.parametrize(
    ("source_id", "expected"),
    [
        ("1:alias:2", (1, 2)),
        ("01:alias:2", None),
        ("1:Alias:2", None),
        ("1:alias:02", None),
        ("1:aliases:2", None),
        ("1:alias:2:extra", None),
        ("1:alias:+2", None),
    ],
)
def test_alias_source_parser_rejects_case_duplicate_and_numeric_variants(
    source_id: str, expected: tuple[int, int] | None
) -> None:
    assert _canonical_alias_source(source_id) == expected


@pytest.mark.parametrize(
    ("source_id", "expected"),
    [
        ("1:source", True),
        ("1:rendering", True),
        ("1:alias:2", True),
        ("1:Source", False),
        ("1:Rendering", False),
        ("1:Alias:2", False),
        ("1:alias:02", False),
        ("1:alias:2:duplicate", False),
    ],
)
def test_facet_source_parser_requires_exact_primary_and_alias_roles(
    source_id: str, expected: bool
) -> None:
    assert _is_canonical_facet_source(source_id, 1) is expected
