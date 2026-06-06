from __future__ import annotations

from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.mcp_server import hieronymus_learn, hieronymus_read
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def test_mcp_learn_writes_blocks(monkeypatch, tmp_path: Path) -> None:
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
                task_type="learning",
            )
        )
        .id
    )

    result = hieronymus_learn(
        session_id=session_id,
        text="One.\n\nTwo.",
        source_role="user",
        source_ref="user:note",
    )

    assert result == {"session_id": session_id, "block_count": 2, "memory_ids": [1, 2]}


def test_mcp_read_returns_findings_without_memory_write(monkeypatch, tmp_path: Path) -> None:
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
                task_type="reading",
            )
        )
        .id
    )

    result = hieronymus_read(
        session_id=session_id,
        text="Gantz uses speed.",
        source_ref="notes:gantz",
    )

    assert result == {
        "session_id": session_id,
        "candidate_terms": ["Gantz"],
        "findings": ["candidate_term:Gantz"],
        "stored_memory_ids": [],
    }
    assert WorkspaceStore(config).list_short_term_memories(session_id) == []
