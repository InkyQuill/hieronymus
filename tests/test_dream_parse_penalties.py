import json

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect
from hieronymus.dreaming import DeterministicDreamProvider, DreamService
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(config: HieronymusConfig) -> TranslationContext:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translate",
        volume="1",
        chapter="2",
    )


def _completed_session(config: HieronymusConfig, context: TranslationContext) -> int:
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Dream parse input.")
    workspace.complete_session(session.id)
    return session.id


def test_malformed_optional_facet_metadata_is_accepted_with_lower_confidence(
    config: HieronymusConfig,
) -> None:
    class MalformedFacetProvider:
        name = "malformed-facet"

        def crystallize(self, context, memories):
            return {
                "concepts": [{"canonical_name": "Sense"}],
                "facets": [
                    {
                        "concept_name": "Sense",
                        "value": "Сенс",
                        "kind": "rendering",
                        "language_tags": ["ru"],
                        "confidence": 0.8,
                    },
                    {
                        "concept_name": "Sense",
                        "value": "Sense",
                        "kind": "alias",
                        "language_tags": ["en", 123],
                        "story_scopes": [False],
                        "is_canonical": "yes",
                        "confidence": 0.8,
                    },
                ],
            }

    context = _context(config)
    _completed_session(config, context)

    run = DreamService(config, MalformedFacetProvider()).run_all()

    with connect(config.database_path) as conn:
        facets = conn.execute(
            """
            select value, confidence, is_canonical
            from concept_facets
            order by id
            """
        ).fetchall()
        audit = conn.execute(
            "select event_type, severity, payload_json from dream_audit_entries"
        ).fetchone()

    assert run.status == "completed"
    assert [(row["value"], row["confidence"], row["is_canonical"]) for row in facets] == [
        ("Сенс", 0.8, 0),
        ("Sense", 0.05, 1),
    ]
    assert audit["event_type"] == "parse_warnings"
    assert audit["severity"] == "warning"
    warning_codes = {warning["code"] for warning in json.loads(audit["payload_json"])["warnings"]}
    assert {
        "malformed_facet_kind",
        "malformed_facet_language_tags",
        "malformed_facet_story_scopes",
        "malformed_facet_canonical",
    } <= warning_codes


def test_missing_crystal_content_is_rejected(config: HieronymusConfig) -> None:
    class MissingContentProvider:
        name = "missing-content"

        def crystallize(self, context, memories):
            return {"crystals": [{"type": "lesson"}]}

    context = _context(config)
    _completed_session(config, context)

    with pytest.raises(ValueError, match="dream candidate content is required"):
        DreamService(config, MissingContentProvider()).run_all()

    with connect(config.database_path) as conn:
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]
        run = conn.execute("select status from dream_runs").fetchone()

    assert crystal_count == 0
    assert run["status"] == "failed"


def test_provider_suggested_thoughts_are_low_confidence_inferred_crystals(
    config: HieronymusConfig,
) -> None:
    class ThoughtProvider:
        name = "thoughts"

        def crystallize(self, context, memories):
            return {
                "thoughts": [
                    {
                        "content": "The UI label may imply a crafting affordance.",
                        "type": "rule",
                        "source_credibility": "expert",
                        "confidence": 0.99,
                    }
                ]
            }

    context = _context(config)
    _completed_session(config, context)

    DreamService(config, ThoughtProvider()).run_all()

    with connect(config.database_path) as conn:
        crystal = conn.execute(
            "select crystal_type, confidence, source_credibility, is_inferred from crystals"
        ).fetchone()

    assert dict(crystal) == {
        "crystal_type": "thought",
        "confidence": 0.2,
        "source_credibility": "thought",
        "is_inferred": 1,
    }


def test_user_rule_short_term_memory_creates_stronger_rule_than_thought(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        "user",
        "correction",
        "Use Sense for センス in UI terminology.",
        source_credibility="user_rule",
        rule_intent="terminology",
    )
    workspace.complete_session(session.id)

    DreamService(config, DeterministicDreamProvider()).run_all()

    class ThoughtProvider:
        name = "thought-after-rule"

        def crystallize(self, context, memories):
            return {"thoughts": ["The UI term may have menu-specific nuance."]}

    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Speculative follow-up.")
    workspace.complete_session(session.id)

    DreamService(config, ThoughtProvider()).run_all()

    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select crystal_type, confidence, source_credibility, is_inferred
            from crystals
            order by id
            """
        ).fetchall()

    assert [
        (row["crystal_type"], row["confidence"], row["source_credibility"], row["is_inferred"])
        for row in rows
    ] == [
        ("rule", 0.95, "user_rule", 0),
        ("thought", 0.2, "thought", 1),
    ]
