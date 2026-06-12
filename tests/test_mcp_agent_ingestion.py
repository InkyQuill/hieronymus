from __future__ import annotations

from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.mcp_server import hieronymus_short_term_add
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _session(tmp_path: Path, *, task_type: str) -> tuple[HieronymusConfig, int]:
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


def test_mcp_short_term_add_stores_agent_learn_block(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config, session_id = _session(tmp_path, task_type="learning")

    result = hieronymus_short_term_add(
        session_id=session_id,
        source_role="user",
        kind="learned_block",
        text="One.",
        source_ref="user:note",
        metadata={"workflow": "learn", "block_index": 1},
    )

    assert result == {"memory_id": 1}
    memory = WorkspaceStore(config).list_short_term_memories(session_id)[0]
    assert memory.text == "One."
    assert memory.kind == "learned_block"
    assert memory.metadata == {
        "block_index": 1,
        "sentence_count": 1,
        "symbol_count": 4,
        "workflow": "learn",
    }


def test_mcp_short_term_add_stores_agent_read_extract(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config, session_id = _session(tmp_path, task_type="reading")

    result = hieronymus_short_term_add(
        session_id=session_id,
        source_role="mundane",
        kind="read_extract",
        text="Gantz uses speed.",
        source_ref="notes:gantz",
        metadata={"workflow": "read"},
    )

    assert result == {"memory_id": 1}
    memory = WorkspaceStore(config).list_short_term_memories(session_id)[0]
    assert memory.text == "Gantz uses speed."
    assert memory.kind == "read_extract"
    assert memory.metadata == {
        "sentence_count": 1,
        "symbol_count": 17,
        "workflow": "read",
    }
