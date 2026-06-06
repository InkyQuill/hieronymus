from __future__ import annotations

from pathlib import Path

from hieronymus.agent_ingestion import IngestionService, split_learning_blocks
from hieronymus.config import HieronymusConfig
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _session(config: HieronymusConfig, *, task_type: str = "learning") -> int:
    Registry(config).create_series(
        slug="oso",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    context = TranslationContext(
        series_slug="oso",
        source_language="ja",
        target_language="en",
        task_type=task_type,
        volume="1",
        chapter="1",
    )
    return WorkspaceStore(config).start_session(context).id


def test_split_learning_blocks_keeps_source_order() -> None:
    blocks = split_learning_blocks(
        "First paragraph.\n\nSecond paragraph has a term: Gantz.\n\nThird.",
        max_chars=80,
    )

    assert [block.index for block in blocks] == [1, 2, 3]
    assert blocks[1].text == "Second paragraph has a term: Gantz."


def test_split_learning_blocks_splits_long_paragraph_on_sentence_boundaries() -> None:
    blocks = split_learning_blocks(
        "First sentence is short. Second sentence is also short. Third sentence ends it.",
        max_chars=60,
    )

    assert [block.index for block in blocks] == [1, 2]
    assert blocks[0].text == "First sentence is short. Second sentence is also short."
    assert blocks[1].text == "Third sentence ends it."


def test_learn_writes_short_term_memories(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    session_id = _session(config)

    result = IngestionService(config).learn(
        session_id=session_id,
        text="Gantz avoids heavy armor.\n\nGantz relies on speed.",
        source_role="mentor",
        source_ref="audit:chapter-1",
    )

    assert result.session_id == session_id
    assert result.block_count == 2
    assert result.memory_ids == [1, 2]
    memories = WorkspaceStore(config).list_short_term_memories(session_id)
    assert [memory.source_role for memory in memories] == ["mentor", "mentor"]
    assert [memory.text for memory in memories] == [
        "Gantz avoids heavy armor.",
        "Gantz relies on speed.",
    ]
    assert memories[0].kind == "learned_block"
    assert memories[0].source_ref == "audit:chapter-1"
    assert memories[0].metadata == {
        "ingestion_mode": "learn",
        "block_index": 1,
        "block_count": 2,
    }
    assert memories[1].metadata["block_index"] == 2


def test_read_extracts_candidates_without_writing_by_default(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    session_id = _session(config, task_type="reading")

    result = IngestionService(config).read(
        session_id=session_id,
        text="Gantz: a martial arts user. Soumen may need a footnote.",
        source_ref="notes:gantz",
    )

    assert result.session_id == session_id
    assert result.stored_memory_ids == []
    assert result.candidate_terms == ["Gantz", "Soumen"]
    assert result.findings == ["candidate_term:Gantz", "candidate_term:Soumen"]
    assert WorkspaceStore(config).list_short_term_memories(session_id) == []


def test_read_can_store_deliberate_observation_when_requested(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    session_id = _session(config, task_type="reading")

    result = IngestionService(config).read(
        session_id=session_id,
        text="Gantz: a martial arts user.",
        source_ref="notes:gantz",
        store_observation=True,
    )

    assert result.stored_memory_ids == [1]
    memory = WorkspaceStore(config).list_short_term_memories(session_id)[0]
    assert memory.source_role == "mundane"
    assert memory.kind == "read_observation"
    assert memory.text == "candidate_term:Gantz"
    assert memory.source_ref == "notes:gantz"
    assert memory.metadata == {
        "ingestion_mode": "read",
        "candidate_terms": ["Gantz"],
    }
