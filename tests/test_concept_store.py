from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect


def test_memory_design_tables_exist(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        table_names = {
            row["name"]
            for row in conn.execute(
                "select name from sqlite_master where type in ('table', 'view')"
            )
        }

    assert {
        "concepts",
        "concept_facets",
        "concept_semantic_tags",
        "concept_renames",
        "crystal_concepts",
        "crystal_story_scopes",
        "crystal_semantic_tags",
        "dream_audit_entries",
        "dream_phase_runs",
    }.issubset(table_names)
