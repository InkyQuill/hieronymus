from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _session(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    *,
    task_type: str,
) -> tuple[HieronymusConfig, int]:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    Registry(config).create_series(
        slug="oso",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    session_id = (
        WorkspaceStore(config)
        .start_session(
            TranslationContext(
                series_slug="oso",
                source_language="ja",
                target_language="en",
                task_type=task_type,
            )
        )
        .id
    )
    return config, session_id


def test_read_and_learn_judgment_tools_are_not_exposed() -> None:
    from hieronymus import mcp_server

    tools = asyncio.run(mcp_server.server.list_tools())
    tool_names = {tool.name for tool in tools}

    assert "hieronymus_read" not in tool_names
    assert "hieronymus_learn" not in tool_names
    assert not hasattr(mcp_server, "hieronymus_read")
    assert not hasattr(mcp_server, "hieronymus_learn")


def test_learn_skill_path_uses_short_term_add_without_crystals(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    config, session_id = _session(monkeypatch, tmp_path, task_type="learning")

    from hieronymus.mcp_server import hieronymus_short_term_add

    result = hieronymus_short_term_add(
        session_id=session_id,
        source_role="mentor",
        kind="learned_block",
        text="Gantz avoids heavy armor.",
        source_ref="notes:gantz",
        metadata={"workflow": "learn"},
        language_tags=["ja", "en"],
        story_scopes=["chapter:1"],
        semantic_tags=["combat"],
        source_credibility="observed_source",
    )

    assert result == {"memory_id": 1}
    memories = WorkspaceStore(config).list_short_term_memories(session_id)
    assert len(memories) == 1
    assert memories[0].kind == "learned_block"
    assert memories[0].text == "Gantz avoids heavy armor."
    assert memories[0].language_tags == ("ja", "en")
    assert memories[0].story_scopes == ("chapter:1",)
    assert memories[0].semantic_tags == ("combat",)
    assert memories[0].source_credibility == "observed_source"
    assert memories[0].metadata == {
        "language_tags": ["ja", "en"],
        "semantic_tags": ["combat"],
        "sentence_count": 1,
        "source_credibility": "observed_source",
        "story_scopes": ["chapter:1"],
        "workflow": "learn",
    }
    with connect(config.database_path) as conn:
        assert conn.execute("select count(*) from crystals").fetchone()[0] == 0


def test_read_skill_path_rejects_huge_extract_with_short_term_validation(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    config, session_id = _session(monkeypatch, tmp_path, task_type="reading")
    text = " ".join(f"Sentence {index}." for index in range(31))

    from hieronymus.mcp_server import hieronymus_short_term_add

    with pytest.raises(ValueError, match="short-term memory is too large"):
        hieronymus_short_term_add(
            session_id=session_id,
            source_role="mundane",
            kind="read_extract",
            text=text,
            source_ref="notes:gantz",
            metadata={"workflow": "read"},
        )

    assert WorkspaceStore(config).list_short_term_memories(session_id) == []


def test_read_skill_path_accepts_warning_sized_extract(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    config, session_id = _session(monkeypatch, tmp_path, task_type="reading")
    text = " ".join(f"sentence {index}." for index in range(7))

    from hieronymus.mcp_server import hieronymus_short_term_add

    result = hieronymus_short_term_add(
        session_id=session_id,
        source_role="mundane",
        kind="read_extract",
        text=text,
        source_ref="notes:gantz",
        metadata={"workflow": "read"},
    )

    assert result == {"memory_id": 1}
    memory = WorkspaceStore(config).list_short_term_memories(session_id)[0]
    assert memory.source_role == "mundane"
    assert memory.kind == "read_extract"
    assert memory.text == text
    assert memory.source_ref == "notes:gantz"
    assert memory.metadata == {
        "sentence_count": 7,
        "validation_warning": "short-term memory is large; prefer 1-6 sentences",
        "workflow": "read",
    }
