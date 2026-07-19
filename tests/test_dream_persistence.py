import json

from hieronymus.db import connect, ensure_schema
from hieronymus.dream_persistence import append_audit_entry, insert_crystal
from hieronymus.dream_validation import NormalizedDreamCrystal
from hieronymus.memory_models import TranslationContext


def _context() -> TranslationContext:
    return TranslationContext(
        series_slug="story",
        source_language="en",
        target_language="ru",
        task_type="translate",
    )


def _candidate(*, concept_id: int) -> NormalizedDreamCrystal:
    return NormalizedDreamCrystal(
        crystal_type="rule",
        title="Rule",
        text="Use the approved form.",
        strength=0.8,
        confidence=0.9,
        source_memory_ids=[],
        source_credibility="user_rule",
        rule_intent="strict terminology",
        story_scopes=("volume-1",),
        semantic_tags=("name",),
        concept_ids=(concept_id,),
    )


def test_insert_crystal_writes_graph_rows_without_committing(config) -> None:
    with connect(config.database_path) as conn:
        ensure_schema(conn)
        concept_id = int(
            conn.execute(
                """
                insert into concepts(
                  canonical_name, scope_type, scope_key, status, confidence, created_at, updated_at
                ) values ('Name', 'global', '', 'active', 0.8, 'now', 'now')
                """
            ).lastrowid
        )
        conn.commit()

        conn.execute("begin immediate")
        crystal_id = insert_crystal(conn, _context(), _candidate(concept_id=concept_id))
        assert (
            conn.execute(
                "select scope from crystal_story_scopes where crystal_id = ?", (crystal_id,)
            ).fetchone()[0]
            == "volume-1"
        )
        assert (
            conn.execute(
                "select tag from crystal_semantic_tags where crystal_id = ?", (crystal_id,)
            ).fetchone()[0]
            == "name"
        )
        assert (
            conn.execute(
                "select concept_id from crystal_concepts where crystal_id = ?", (crystal_id,)
            ).fetchone()[0]
            == concept_id
        )
        conn.rollback()

        assert conn.execute("select 1 from crystals where id = ?", (crystal_id,)).fetchone() is None


def test_append_audit_entry_uses_callers_transaction_and_redacts(config) -> None:
    with connect(config.database_path) as conn:
        ensure_schema(conn)
        run_id = int(
            conn.execute(
                """
                insert into dream_runs(cycle_id, status, provider, created_at)
                values (1, 'running', 'test', 'now')
                """
            ).lastrowid
        )
        conn.commit()

        conn.execute("begin immediate")
        audit_id = append_audit_entry(
            conn,
            dream_run_id=run_id,
            phase_run_id=None,
            event_type="provider_response",
            severity="info",
            summary="received response",
            payload={"api_token": "secret", "count": 1},
        )
        payload = json.loads(
            conn.execute(
                "select payload_json from dream_audit_entries where id = ?", (audit_id,)
            ).fetchone()[0]
        )
        assert payload == {"api_token": "[REDACTED]", "count": 1}
        conn.rollback()

        assert (
            conn.execute("select 1 from dream_audit_entries where id = ?", (audit_id,)).fetchone()
            is None
        )
