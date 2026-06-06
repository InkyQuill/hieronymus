from __future__ import annotations

import asyncio
import tomllib

import pytest

from hieronymus.concepts import ConceptProposalStore
from hieronymus.config import load_config
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
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

    approved = mcp_server.hieronymus_termbase_approve(series.slug, proposed["term_id"])
    assert approved == {"term_id": 1, "approved": True}

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

    contract = mcp_server.hieronymus_termbase_contract(series.slug, "ユン walked home.")
    assert contract == [
        {
            "id": 1,
            "category": "character",
            "source_text": "ユン",
            "canonical_translation": "Yun",
            "forbidden_variants": ["Yuun"],
            "tags": ["name"],
            "notes": "Main character.",
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
    assert added == {"memory_id": 1}

    memories = mcp_server.hieronymus_memory_search(series.slug, "Yun", limit=5)
    assert memories == [
        {
            "id": 1,
            "kind": "translation_rationale",
            "text": "Use Yun for ユン.",
            "importance": 4,
            "source_ref": "",
        }
    ]


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
            "category": "character",
            "source_text": "ユン",
            "canonical_translation": "Yun",
            "forbidden_variants": [],
            "tags": [],
            "notes": "",
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
    assert dreamed == {"cycle_id": 1, "status": "completed"}


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

    assert recalled == [
        {
            "crystal_id": crystal_id,
            "text": "Render Sense as Sense in UI references.",
            "rank": 1,
            "score": pytest.approx(recalled[0]["score"]),
            "reason": "weighted search match",
        }
    ]


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

    assert recalled[0]["crystal_id"] == crystal_id
    assert recalled[0]["text"] == "Render Holo as Horo in this translation."


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

    assert recalled[0]["crystal_id"] == crystal_id
    with connect(config.database_path) as conn:
        activation_count = conn.execute("select count(*) from crystal_activations").fetchone()[0]
    assert activation_count == 1


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


def test_mcp_feedback_returns_event_id(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    context = TranslationContext(
        series_slug=series.slug,
        source_language="ja",
        target_language="en",
        task_type="translation",
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Keep UI labels concise.",
    )

    from hieronymus import mcp_server

    event = mcp_server.hieronymus_feedback(
        crystal_id,
        event_type="confirmed_by_user",
        source_role="user",
        evidence="Applied in chapter 1.",
    )

    assert event == {"event_id": 1}


def test_mcp_feedback_rejects_mismatched_session_without_event(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    alpha_series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    Registry(config).create_series(
        slug="beta-series",
        title="Beta Series",
        source_language="ja",
        target_language="en",
    )
    beta_session = WorkspaceStore(config).start_session(
        TranslationContext(
            series_slug="beta-series",
            source_language="ja",
            target_language="en",
            task_type="translation",
        )
    )
    crystal_id = CrystalStore(config).add_crystal(
        TranslationContext(
            series_slug=alpha_series.slug,
            source_language="ja",
            target_language="en",
            task_type="translation",
        ),
        crystal_type="lesson",
        text="Keep UI labels concise.",
    )

    from hieronymus import mcp_server

    with pytest.raises(ValueError, match="series_slug"):
        mcp_server.hieronymus_feedback(
            crystal_id,
            event_type="confirmed_by_user",
            source_role="user",
            session_id=beta_session.id,
        )

    with connect(config.database_path) as conn:
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]
    assert event_count == 0


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


def test_mcp_dream_rejects_unsupported_provider(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    with pytest.raises(ValueError, match="unsupported dream provider"):
        mcp_server.hieronymus_dream(provider="external")
