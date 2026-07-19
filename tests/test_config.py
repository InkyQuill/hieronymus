from pathlib import Path

from hieronymus.config import HieronymusConfig, load_config
from hieronymus.db import apply_migration, connect, ensure_schema


def test_config_exposes_single_global_database(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path)

    assert config.database_path == tmp_path / "hieronymus.sqlite"
    assert not hasattr(config, "registry_path")
    assert not hasattr(config, "series_dir")


def test_load_config_uses_explicit_data_root(tmp_path: Path) -> None:
    config = load_config(tmp_path)

    assert config.data_root == tmp_path
    assert config.database_path == tmp_path / "hieronymus.sqlite"


def test_load_config_explicit_root_overrides_environment(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "environment"))

    config = load_config(tmp_path / "explicit")

    assert config.data_root == (tmp_path / "explicit").resolve()


def test_load_config_defaults_to_xdg_config_home_when_unset(
    monkeypatch,
) -> None:
    monkeypatch.delenv("HIERONYMUS_DATA_ROOT", raising=False)
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)

    config = load_config()

    assert config.data_root == Path.home() / ".config" / "hieronymus"


def test_load_config_uses_xdg_config_home(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.delenv("HIERONYMUS_DATA_ROOT", raising=False)
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "xdg"))

    config = load_config()

    assert config.data_root == (tmp_path / "xdg" / "hieronymus").resolve()


def test_load_config_ignores_blank_xdg_config_home(monkeypatch) -> None:
    monkeypatch.delenv("HIERONYMUS_DATA_ROOT", raising=False)
    monkeypatch.setenv("XDG_CONFIG_HOME", "   ")

    config = load_config()

    assert config.data_root == (Path.home() / ".config" / "hieronymus").resolve()


def test_load_config_ignores_blank_environment_root(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", "   ")
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "xdg"))

    config = load_config()

    assert config.data_root == (tmp_path / "xdg" / "hieronymus").resolve()


def test_load_config_uses_environment_root(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path))

    config = load_config()

    assert config.data_root == tmp_path


def test_load_config_expands_and_absolutizes_relative_environment_root(
    monkeypatch, tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", "relative/root")

    config = load_config()

    assert config.data_root == (tmp_path / "relative" / "root").resolve()


def test_config_has_no_data_root_alias(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path)

    assert not hasattr(config, "config" + "_root")


def test_global_migration_creates_memory_dreaming_schema(tmp_path: Path) -> None:
    with connect(tmp_path / "hieronymus.sqlite") as conn:
        ensure_schema(conn)
        tables = {
            row["name"]
            for row in conn.execute(
                "select name from sqlite_master where type in ('table', 'view')"
            )
        }

    assert {
        "series",
        "task_sessions",
        "short_term_memories",
        "short_term_memories_fts",
        "crystals",
        "crystals_fts",
        "crystal_sources",
        "crystal_links",
        "crystal_activations",
        "memory_events",
        "dream_runs",
        "strict_concept_proposals",
    } <= tables
    legacy_tables = {"strict_terms", "strict_term_tags", "strict_term_aliases", "strict_terms_fts"}
    assert not legacy_tables & tables
    assert (
        not {
            "terms",
            "term_tags",
            "term_aliases",
            "term_evidence",
            "memories",
            "terms_fts",
            "memories_fts",
        }
        & tables
    )


def test_global_schema_and_ordered_upgrade_create_dream_maintenance_index(tmp_path: Path) -> None:
    fresh_path = tmp_path / "fresh.sqlite"
    with connect(fresh_path) as conn:
        ensure_schema(conn)
        fresh_indexes = {
            row["name"]
            for row in conn.execute("select name from sqlite_master where type = 'index'")
        }
        fresh_versions = [
            row["version"]
            for row in conn.execute("select version from schema_migrations order by version")
        ]

    assert "idx_crystals_dream_maintenance" in fresh_indexes
    assert fresh_versions[-1] == "0003"

    with connect(fresh_path) as conn:
        conn.execute("drop index idx_crystals_dream_maintenance")
        conn.execute("delete from schema_migrations where version = '0003'")
        conn.commit()
        ensure_schema(conn)
        upgraded_index = conn.execute(
            """
            select name from sqlite_master
            where type = 'index' and name = 'idx_crystals_dream_maintenance'
            """
        ).fetchone()
        upgraded_version = conn.execute(
            "select name from schema_migrations where version = '0003'"
        ).fetchone()

    assert upgraded_index["name"] == "idx_crystals_dream_maintenance"
    assert upgraded_version["name"] == "index_dream_maintenance"


def test_global_migration_allows_cycle_less_records(tmp_path: Path) -> None:
    with connect(tmp_path / "hieronymus.sqlite") as conn:
        ensure_schema(conn)
        nullable = {
            table: {
                row["name"]: not row["notnull"]
                for row in conn.execute(f"pragma table_info({table})")
            }
            for table in ("task_sessions", "crystal_activations", "memory_events")
        }

    assert nullable["task_sessions"]["cycle_id"]
    assert nullable["crystal_activations"]["cycle_id"]
    assert nullable["memory_events"]["cycle_id"]


def test_global_memory_fts_triggers_track_raw_mutations_and_session_cascade(
    tmp_path: Path,
) -> None:
    with connect(tmp_path / "hieronymus.sqlite") as conn:
        ensure_schema(conn)
        conn.execute(
            """
            insert into series(
              slug, title, default_source_language, default_target_language,
              created_at, updated_at
            ) values ('series', 'Series', 'ja', 'ru', 'now', 'now')
            """
        )
        session_id = conn.execute(
            """
            insert into task_sessions(
              series_slug, source_language, target_language, task_type, status,
              created_at, last_activity_at
            ) values ('series', 'ja', 'ru', 'translate', 'active', 'now', 'now')
            """
        ).lastrowid
        memory_id = conn.execute(
            """
            insert into short_term_memories(
              session_id, source_role, kind, text, created_at
            ) values (?, 'user', 'note', 'raw insertion token', 'now')
            """,
            (session_id,),
        ).lastrowid

        assert (
            conn.execute(
                """
            select count(*) from short_term_memories_fts
            where short_term_memories_fts match 'insertion'
            """
            ).fetchone()[0]
            == 1
        )
        conn.execute(
            "update short_term_memories set text = 'raw update token' where id = ?",
            (memory_id,),
        )
        assert (
            conn.execute(
                """
            select count(*) from short_term_memories_fts
            where short_term_memories_fts match 'insertion'
            """
            ).fetchone()[0]
            == 0
        )
        assert (
            conn.execute(
                """
            select count(*) from short_term_memories_fts
            where short_term_memories_fts match 'update'
            """
            ).fetchone()[0]
            == 1
        )

        conn.execute("delete from task_sessions where id = ?", (session_id,))
        assert (
            conn.execute(
                """
            select count(*) from short_term_memories_fts
            where short_term_memories_fts match 'update'
            """
            ).fetchone()[0]
            == 0
        )


def test_global_migration_repairs_legacy_memory_fts_drift(tmp_path: Path) -> None:
    with connect(tmp_path / "hieronymus.sqlite") as conn:
        apply_migration(conn, "global.sql")
        conn.execute("drop table schema_migrations")
        conn.execute(
            """
            insert into series(
              slug, title, default_source_language, default_target_language,
              created_at, updated_at
            ) values ('series', 'Series', 'ja', 'ru', 'now', 'now')
            """
        )
        session_id = conn.execute(
            """
            insert into task_sessions(
              series_slug, source_language, target_language, task_type, status,
              created_at, last_activity_at
            ) values ('series', 'ja', 'ru', 'translate', 'active', 'now', 'now')
            """
        ).lastrowid
        conn.execute("drop trigger short_term_memories_ai")
        memory_id = conn.execute(
            """
            insert into short_term_memories(
              session_id, source_role, kind, text, created_at
            ) values (?, 'user', 'note', 'legacy missing token', 'now')
            """,
            (session_id,),
        ).lastrowid
        conn.execute("drop trigger crystals_ai")
        crystal_id = conn.execute(
            """
            insert into crystals(
              crystal_type, text, title, scope_type, strength, confidence,
              status, created_at, updated_at
            ) values (
              'lesson', 'legacy stale token', 'Legacy', 'global', 0.5, 0.5,
              'active', 'now', 'now'
            )
            """
        ).lastrowid
        conn.execute(
            "insert into crystals_fts(rowid, title, text) values (?, 'Wrong', 'drifted token')",
            (crystal_id,),
        )
        conn.commit()

        ensure_schema(conn)

        assert (
            conn.execute(
                """
            select rowid from short_term_memories_fts
            where short_term_memories_fts match 'missing'
            """
            ).fetchone()[0]
            == memory_id
        )
        assert (
            conn.execute(
                "select rowid from crystals_fts where crystals_fts match 'stale'"
            ).fetchone()[0]
            == crystal_id
        )
        assert (
            conn.execute(
                "select count(*) from crystals_fts where crystals_fts match 'drifted'"
            ).fetchone()[0]
            == 0
        )
