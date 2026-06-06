from pathlib import Path

from hieronymus.config import HieronymusConfig, load_config
from hieronymus.db import apply_migration, connect


def test_config_exposes_single_global_database(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path)

    assert config.database_path == tmp_path / "hieronymus.sqlite"
    assert not hasattr(config, "registry_path")
    assert not hasattr(config, "series_dir")


def test_load_config_uses_explicit_data_root(tmp_path: Path) -> None:
    config = load_config(tmp_path)

    assert config.data_root == tmp_path
    assert config.database_path == tmp_path / "hieronymus.sqlite"


def test_load_config_defaults_to_xdg_config_home_when_unset(
    monkeypatch,
) -> None:
    monkeypatch.delenv("HIERONYMUS_DATA_ROOT", raising=False)

    config = load_config()

    assert config.data_root == Path.home() / ".config" / "hieronymus"


def test_load_config_uses_environment_root(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path))

    config = load_config()

    assert config.data_root == tmp_path


def test_global_migration_creates_memory_dreaming_schema(tmp_path: Path) -> None:
    with connect(tmp_path / "hieronymus.sqlite") as conn:
        apply_migration(conn, "global.sql")
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
        "strict_terms",
        "strict_term_tags",
        "strict_term_aliases",
        "strict_terms_fts",
    } <= tables
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


def test_global_migration_allows_cycle_less_records(tmp_path: Path) -> None:
    with connect(tmp_path / "hieronymus.sqlite") as conn:
        apply_migration(conn, "global.sql")
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
