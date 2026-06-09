from __future__ import annotations

import asyncio
import json
import tomllib

import pytest

from hieronymus.concepts import ConceptProposalStore, ConceptStore
from hieronymus.config import load_config
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.settings import DreamingSettings, load_settings, save_settings
from hieronymus.termbase import Termbase
from hieronymus.workspace import WorkspaceStore


def test_mcp_tools_wrap_core_services(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    series = Registry(load_config()).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    proposed = mcp_server.hieronymus_termbase_propose(
        series.slug,
        "character",
        "ユン",
        "Yun",
        tags=["name"],
        notes="Main character.",
    )
    assert proposed == {"term_id": 1}

    termbase = Termbase(
        load_config(),
        TranslationContext(
            series_slug=series.slug,
            source_language=series.source_language,
            target_language=series.target_language,
            task_type="translation",
        ),
    )
    termbase.add_alias(
        proposed["term_id"],
        kind="forbidden_variant",
        text="Yuun",
        language="en",
    )

    approved = mcp_server.hieronymus_termbase_approve(series.slug, proposed["term_id"])
    assert approved == {"term_id": 1, "approved": True}

    contract = mcp_server.hieronymus_termbase_contract(series.slug, "ユン walked home.")
    assert contract == [
        {
            "id": 1,
            "category": "rule",
            "source_text": "ユン",
            "canonical_translation": "Yun",
            "forbidden_variants": ["Yuun"],
            "tags": ["name"],
            "notes": "ユン is translated as Yun, not Yuun.",
        }
    ]

    findings = mcp_server.hieronymus_termbase_validate(
        series.slug,
        "ユン walked home.",
        "Yuun walked home.",
    )
    assert findings == [
        {
            "term_id": 1,
            "kind": "forbidden_variant",
            "severity": "high",
            "expected": "Yun",
            "observed": "Yuun",
            "message": "Use 'Yun' for 'ユン'; 'Yuun' is forbidden.",
        },
        {
            "term_id": 1,
            "kind": "missing_canonical",
            "severity": "medium",
            "expected": "Yun",
            "observed": "",
            "message": (
                "Raw text contains 'ユン', but translation does not contain approved form 'Yun'."
            ),
        },
    ]

    added = mcp_server.hieronymus_memory_add(
        series.slug,
        "translation_rationale",
        "Use Yun for ユン.",
        source_ref="chapter-1",
        importance=4,
    )
    assert added == {"memory_id": 1, "storage": "short_term"}

    memories = mcp_server.hieronymus_memory_search(series.slug, "translation_rationale", limit=5)
    assert memories == []


def test_mcp_memory_add_routes_legacy_calls_to_short_term(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    correction = mcp_server.hieronymus_memory_add(
        series.slug,
        "rule",
        "Use Sense, not Feeling, for センス.",
        source_ref="chapter-2",
        importance=5,
    )
    note = mcp_server.hieronymus_memory_add(
        series.slug,
        "translation_rationale",
        "Keep UI terminology concise.",
        source_ref="chapter-2",
        importance=2,
    )

    assert correction == {"memory_id": 1, "storage": "short_term"}
    assert note == {"memory_id": 2, "storage": "short_term"}
    with connect(config.database_path) as conn:
        sessions = conn.execute("select * from task_sessions order by id").fetchall()
        memories = conn.execute("select * from short_term_memories order by id").fetchall()
        dream_count = conn.execute("select count(*) from dream_runs").fetchone()[0]

    assert len(sessions) == 1
    assert sessions[0]["status"] == "active"
    assert sessions[0]["series_slug"] == series.slug
    assert memories[0]["session_id"] == sessions[0]["id"]
    assert memories[0]["source_role"] == "user"
    assert memories[0]["kind"] == "correction"
    assert memories[0]["source_ref"] == "chapter-2"
    assert json.loads(memories[0]["metadata_json"]) == {
        "importance": 5,
        "legacy_kind": "rule",
        "sentence_count": 1,
    }
    assert memories[1]["kind"] == "note"
    assert json.loads(memories[1]["metadata_json"]) == {
        "importance": 2,
        "legacy_kind": "translation_rationale",
        "sentence_count": 1,
    }
    assert dream_count == 0


def test_mcp_memory_add_rejects_empty_kind_without_creating_rows(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    with pytest.raises(ValueError, match="kind must not be empty"):
        mcp_server.hieronymus_memory_add(
            series.slug,
            "   ",
            "Use Sense, not Feeling, for センス.",
        )

    with connect(config.database_path) as conn:
        session_count = conn.execute("select count(*) from task_sessions").fetchone()[0]
        memory_count = conn.execute("select count(*) from short_term_memories").fetchone()[0]

    assert session_count == 0
    assert memory_count == 0


def test_mcp_termbase_contract_accepts_volume_and_chapter(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    series = Registry(load_config()).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    proposed = mcp_server.hieronymus_termbase_propose(
        series.slug,
        "character",
        "ユン",
        "Yun",
        volume="1",
        chapter="2",
    )
    mcp_server.hieronymus_termbase_approve(
        series.slug,
        proposed["term_id"],
        volume="1",
        chapter="2",
    )

    contract = mcp_server.hieronymus_termbase_contract(
        series.slug,
        "ユン walked home.",
        volume="1",
        chapter="2",
    )

    assert contract == [
        {
            "id": 1,
            "category": "rule",
            "source_text": "ユン",
            "canonical_translation": "Yun",
            "forbidden_variants": [],
            "tags": [],
            "notes": "ユン is translated as Yun.",
        }
    ]


def test_mcp_script_entrypoint_is_declared():
    with open("pyproject.toml", "rb") as pyproject:
        data = tomllib.load(pyproject)

    assert data["project"]["scripts"]["hieronymus-mcp"] == "hieronymus.mcp_server:main"


def test_mcp_main_is_callable():
    from hieronymus.mcp_server import main

    assert callable(main)


def test_mcp_tools_raise_clear_error_when_data_root_is_file(monkeypatch, tmp_path):
    data_root = tmp_path / "data-root-file"
    data_root.write_text("not a directory", encoding="utf-8")
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(data_root))

    from hieronymus import mcp_server

    with pytest.raises(ValueError, match=f"data root is not a directory: {data_root}"):
        mcp_server.hieronymus_memory_search("only-sense-online", "Yun")


def test_mcp_server_registers_expected_tool_names():
    from hieronymus.mcp_server import server

    tools = asyncio.run(server.list_tools())

    assert {tool.name for tool in tools} == {
        "hieronymus_termbase_contract",
        "hieronymus_termbase_validate",
        "hieronymus_termbase_propose",
        "hieronymus_termbase_approve",
        "hieronymus_memory_search",
        "hieronymus_memory_add",
        "hieronymus_session_start",
        "hieronymus_session_complete",
        "hieronymus_short_term_add",
        "hieronymus_learn",
        "hieronymus_read",
        "hieronymus_recall",
        "hieronymus_feedback",
        "hieronymus_dream",
        "hieronymus_concept_proposals_list",
    }


def test_mcp_session_memory_complete_and_dream_happy_path(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    series = Registry(load_config()).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    started = mcp_server.hieronymus_session_start(
        series.slug,
        volume="1",
        chapter="2",
    )
    assert started == {"session_id": 1}

    added = mcp_server.hieronymus_short_term_add(
        started["session_id"],
        source_role="user",
        kind="correction",
        text="Use Sense as a game-system term.",
        source_ref="chapter-2",
        metadata={"line": 12},
    )
    assert added == {"memory_id": 1}

    completed = mcp_server.hieronymus_session_complete(started["session_id"])
    assert completed == {"session_id": 1, "completed": True}

    dreamed = mcp_server.hieronymus_dream()
    assert dreamed == {
        "cycle_id": 1,
        "status": "completed",
        "provider": "deterministic",
        "input_count": 1,
        "created_crystal_count": 1,
        "proposal_count": 0,
    }


def test_mcp_session_start_without_languages_uses_registry_pair(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="spice-and-wolf",
        title="Spice and Wolf",
        source_language="de",
        target_language="ru",
    )

    from hieronymus import mcp_server

    started = mcp_server.hieronymus_session_start(series.slug)

    with connect(config.database_path) as conn:
        session = conn.execute("select * from task_sessions where id = ?", (1,)).fetchone()
    assert started == {"session_id": 1}
    assert session["source_language"] == "de"
    assert session["target_language"] == "ru"


def test_mcp_recall_returns_expected_dict(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    session_id = mcp_server.hieronymus_session_start(series.slug)["session_id"]
    context = TranslationContext(
        series_slug=series.slug,
        source_language="ja",
        target_language="en",
        task_type="translation",
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Render Sense as Sense in UI references.",
        title="Sense Rendering",
        strength=0.8,
        confidence=0.9,
    )

    recalled = mcp_server.hieronymus_recall(
        session_id,
        series.slug,
        "Sense UI",
        limit=5,
    )

    assert len(recalled) == 1
    assert recalled[0] == {
        "source": "long_term",
        "rank": 1,
        "score": pytest.approx(recalled[0]["score"]),
        "reason": "weighted search match",
        "crystal": {
            "id": crystal_id,
            "crystal_type": "lesson",
            "text": "Render Sense as Sense in UI references.",
            "title": "Sense Rendering",
            "confidence": 0.9,
            "strength": 0.8,
            "status": "active",
            "source_credibility": "observation",
            "rule_intent": "",
            "story_scopes": [],
            "semantic_tags": [],
            "concept_ids": [],
        },
        "short_term_memory": None,
    }


def test_mcp_recall_without_languages_uses_stored_non_default_session_context(
    monkeypatch,
    tmp_path,
):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="spice-and-wolf",
        title="Spice and Wolf",
        source_language="de",
        target_language="ru",
    )

    from hieronymus import mcp_server

    session_id = mcp_server.hieronymus_session_start(series.slug)["session_id"]
    context = TranslationContext(
        series_slug=series.slug,
        source_language="de",
        target_language="ru",
        task_type="translation",
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Render Holo as Horo in this translation.",
        title="Horo Name",
    )

    recalled = mcp_server.hieronymus_recall(session_id, series.slug, "Holo")

    assert recalled[0]["source"] == "long_term"
    assert recalled[0]["crystal"]["id"] == crystal_id
    assert recalled[0]["crystal"]["text"] == "Render Holo as Horo in this translation."
    assert recalled[0]["short_term_memory"] is None


def test_mcp_recall_without_context_args_uses_stored_session_context(
    monkeypatch,
    tmp_path,
):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    session_id = mcp_server.hieronymus_session_start(
        series.slug,
        task_type="revision",
        volume="1",
        chapter="2",
    )["session_id"]
    context = TranslationContext(
        series_slug=series.slug,
        source_language="ja",
        target_language="en",
        task_type="revision",
        volume="1",
        chapter="2",
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Keep revised UI labels concise.",
        title="Revision Labels",
    )

    recalled = mcp_server.hieronymus_recall(session_id, series.slug, "revised labels")

    assert recalled[0]["crystal"]["id"] == crystal_id
    with connect(config.database_path) as conn:
        activation_count = conn.execute("select count(*) from crystal_activations").fetchone()[0]
    assert activation_count == 1


def test_mcp_recall_returns_short_term_payload(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    session_id = mcp_server.hieronymus_session_start(series.slug)["session_id"]
    memory_id = WorkspaceStore(config).add_short_term_memory(
        session_id,
        source_role="mentor",
        kind="note",
        text="Keep Sense as Sense in menu labels.",
        metadata={"source": "review"},
    )

    recalled = mcp_server.hieronymus_recall(session_id, series.slug, "Sense labels")

    assert recalled == [
        {
            "source": "short_term",
            "rank": 1,
            "score": pytest.approx(recalled[0]["score"]),
            "reason": "active session short-term memory match",
            "crystal": None,
            "short_term_memory": {
                "id": memory_id,
                "source_role": "mentor",
                "kind": "note",
                "text": "Keep Sense as Sense in menu labels.",
                "metadata": {
                    "sentence_count": 1,
                    "source": "review",
                },
            },
        }
    ]


def test_mcp_recall_rejects_mismatched_session_context_without_activation(
    monkeypatch,
    tmp_path,
):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    session_id = mcp_server.hieronymus_session_start(series.slug, volume="1")["session_id"]

    with pytest.raises(ValueError, match="session context mismatch"):
        mcp_server.hieronymus_recall(
            session_id,
            series.slug,
            "Sense",
            volume="2",
        )

    with connect(config.database_path) as conn:
        activation_count = conn.execute("select count(*) from crystal_activations").fetchone()[0]
    assert activation_count == 0


def test_mcp_recall_rejects_explicit_language_mismatch_without_activation(
    monkeypatch,
    tmp_path,
):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="spice-and-wolf",
        title="Spice and Wolf",
        source_language="de",
        target_language="ru",
    )

    from hieronymus import mcp_server

    session_id = mcp_server.hieronymus_session_start(series.slug)["session_id"]

    with pytest.raises(ValueError, match="source_language"):
        mcp_server.hieronymus_recall(
            session_id,
            series.slug,
            "Holo",
            source_language="ja",
        )

    with connect(config.database_path) as conn:
        activation_count = conn.execute("select count(*) from crystal_activations").fetchone()[0]
    assert activation_count == 0


def test_mcp_feedback_records_named_correction_short_term_memory(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )

    from hieronymus import mcp_server

    started = mcp_server.hieronymus_session_start(series.slug)
    result = mcp_server.hieronymus_feedback(
        session_id=started["session_id"],
        correction_text=(
            "User told me to remember that Cooking Talent is translated as Кулинария."
        ),
    )
    memories = WorkspaceStore(load_config()).list_short_term_memories(started["session_id"])

    assert result == {"memory_id": 1}
    assert memories[0].kind == "correction"
    assert memories[0].source_role == "user"
    assert memories[0].text.startswith("User told me to remember")
    assert memories[0].metadata == {"sentence_count": 1}


def test_mcp_feedback_records_correction_short_term_memory(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )

    from hieronymus import mcp_server

    started = mcp_server.hieronymus_session_start(series.slug)
    result = mcp_server.hieronymus_feedback(
        session_id=started["session_id"],
        correction_text="User told me to remember that Cooking Talent is translated as Кулинария.",
    )
    memories = WorkspaceStore(load_config()).list_short_term_memories(started["session_id"])

    assert result == {"memory_id": 1}
    assert memories[0].kind == "correction"
    assert memories[0].source_role == "user"
    assert memories[0].text.startswith("User told me to remember")
    assert memories[0].metadata == {"sentence_count": 1}
    with connect(config.database_path) as conn:
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]
        dream_run_count = conn.execute("select count(*) from dream_runs").fetchone()[0]
    assert event_count == 0
    assert dream_run_count == 0


def test_mcp_feedback_rejects_unknown_session_without_memory(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    with pytest.raises(KeyError, match="unknown session"):
        mcp_server.hieronymus_feedback(
            session_id=999,
            correction_text="User told me to remember that Sense is Сенс.",
        )

    with connect(config.database_path) as conn:
        memory_count = conn.execute("select count(*) from short_term_memories").fetchone()[0]
    assert memory_count == 0


def test_mcp_concept_proposals_list_returns_pending(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    store = ConceptProposalStore(load_config())
    store.create(
        dream_run_id=None,
        series_slug="only-sense-online",
        source_language="ja",
        target_language="en",
        concept_text="Sense",
        source_form="センス",
        canonical_rendering="Sense",
        approved_variants=["Sense"],
        forbidden_variants=["Senses"],
        rationale="User correction.",
    )

    from hieronymus import mcp_server

    assert mcp_server.hieronymus_concept_proposals_list() == [
        {
            "id": 1,
            "series_slug": "only-sense-online",
            "source_language": "ja",
            "target_language": "en",
            "concept_text": "Sense",
            "source_form": "センス",
            "canonical_rendering": "Sense",
            "approved_variants": ["Sense"],
            "forbidden_variants": ["Senses"],
            "rationale": "User correction.",
            "status": "pending",
        }
    ]


def test_mcp_concept_proposals_list_includes_vague_concepts(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    ConceptStore(load_config()).create_or_reinforce(
        "Sense",
        description="A game-like aptitude category.",
        confidence_delta=0.2,
        scope_type="project",
        scope_key="only-sense-online",
    )

    from hieronymus import mcp_server

    assert mcp_server.hieronymus_concept_proposals_list() == [
        {
            "id": -1,
            "series_slug": "only-sense-online",
            "source_language": "",
            "target_language": "",
            "concept_text": "Sense",
            "source_form": "Sense",
            "canonical_rendering": "Sense",
            "approved_variants": [],
            "forbidden_variants": [],
            "rationale": "A game-like aptitude category.",
            "status": "vague",
        }
    ]


def test_mcp_concept_proposals_list_cleans_malformed_audit_payloads(
    monkeypatch,
    tmp_path,
):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    ConceptProposalStore(config)
    with connect(config.database_path) as conn:
        old_run_id = conn.execute(
            """
            insert into dream_runs(cycle_id, status, provider, created_at)
            values (1, 'completed', 'deterministic', '2026-06-09T00:00:00+00:00')
            """
        ).lastrowid
        conn.execute(
            """
            insert into dream_audit_entries(
              dream_run_id,
              phase_run_id,
              event_type,
              severity,
              summary,
              payload_json,
              created_at
            )
            values (?, null, 'provider_output', 'info', 'old proposal', ?, ?)
            """,
            (
                old_run_id,
                json.dumps(
                    {
                        "concept_proposals": [
                            {
                                "series_slug": "old-series",
                                "source_language": "ja",
                                "target_language": "en",
                                "concept_text": "Old",
                                "source_form": "古い",
                                "canonical_rendering": "Old",
                                "approved_variants": ["Old"],
                                "forbidden_variants": [],
                                "rationale": "outside recent window",
                            }
                        ]
                    }
                ),
                "2026-06-09T00:00:00+00:00",
            ),
        )
        for cycle_id in range(2, 51):
            run_id = conn.execute(
                """
                insert into dream_runs(cycle_id, status, provider, created_at)
                values (?, 'completed', 'deterministic', '2026-06-09T00:00:00+00:00')
                """,
                (cycle_id,),
            ).lastrowid
            conn.execute(
                """
                insert into dream_audit_entries(
                  dream_run_id,
                  phase_run_id,
                  event_type,
                  severity,
                  summary,
                  payload_json,
                  created_at
                )
                values (?, null, 'noise', 'info', 'noise', '{"noise": true}', ?)
                """,
                (run_id, "2026-06-09T00:00:00+00:00"),
            )
        recent_run_id = conn.execute(
            """
            insert into dream_runs(cycle_id, status, provider, created_at)
            values (51, 'completed', 'deterministic', '2026-06-09T00:00:00+00:00')
            """
        ).lastrowid
        conn.execute(
            """
            insert into dream_audit_entries(
              dream_run_id,
              phase_run_id,
              event_type,
              severity,
              summary,
              payload_json,
              created_at
            )
            values (?, null, 'provider_output', 'info', 'recent proposals', ?, ?)
            """,
            (
                recent_run_id,
                json.dumps(
                    {
                        "concept_proposals": [
                            "not a proposal",
                            {
                                "concept_text": "",
                                "source_form": "空",
                                "canonical_rendering": "Empty",
                            },
                            {
                                "concept_text": "Bad Source",
                                "source_form": 42,
                                "canonical_rendering": "Bad Source",
                            },
                            {
                                "series_slug": 7,
                                "source_language": None,
                                "target_language": ["en"],
                                "concept_text": "Sense",
                                "source_form": "センス",
                                "canonical_rendering": 17,
                                "approved_variants": "Sense",
                                "forbidden_variants": ["Feeling", 3],
                                "rationale": {"why": "malformed"},
                            },
                        ]
                    }
                ),
                "2026-06-09T00:00:00+00:00",
            ),
        )
        conn.commit()

    from hieronymus import mcp_server

    assert mcp_server.hieronymus_concept_proposals_list() == [
        {
            "id": 51,
            "series_slug": "",
            "source_language": "",
            "target_language": "",
            "concept_text": "Sense",
            "source_form": "センス",
            "canonical_rendering": "Sense",
            "approved_variants": [],
            "forbidden_variants": ["Feeling"],
            "rationale": "",
            "status": "audit",
        }
    ]


def test_mcp_dream_rejects_unsupported_provider(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    with pytest.raises(ValueError, match="unsupported dream provider: external"):
        mcp_server.hieronymus_dream(provider="external")


def test_mcp_dream_rejects_active_cycle(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()

    from hieronymus import mcp_server
    from hieronymus.dream_locks import dream_cycle_lock

    with dream_cycle_lock(config, owner="manual"):
        with pytest.raises(ValueError, match="dream cycle already running"):
            mcp_server.hieronymus_dream()


def test_mcp_dream_uses_configured_provider(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    save_settings(
        config,
        load_settings(config).with_dreaming(DreamingSettings(active_provider="deterministic")),
    )

    from hieronymus import mcp_server

    dreamed = mcp_server.hieronymus_dream()

    assert dreamed["provider"] == "deterministic"
    assert dreamed["status"] == "completed"
