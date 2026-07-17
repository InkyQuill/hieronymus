from __future__ import annotations

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect
from hieronymus.dream_workflows import DREAM_PASS_NAMES
from hieronymus.dreaming import DreamService
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


class EvidenceProvider:
    name = "evidence-test"

    def __init__(self, *, omit_coverage: bool = False) -> None:
        self.calls: list[tuple[str, tuple[int, ...]]] = []
        self.omit_coverage = omit_coverage

    def run_pass(self, pass_name: str, context, memories):
        ids = tuple(memory.id for memory in memories)
        self.calls.append((pass_name, ids))
        if pass_name == "coverage_audit":
            return {"covered_memory_ids": list(ids[1:] if self.omit_coverage else ids)}
        if pass_name == "knowledge_crystals":
            return {
                "crystals": [
                    {
                        "crystal_type": "observation",
                        "title": "Evidence",
                        "text": "The selected memory is important.",
                        "strength": 0.6,
                        "confidence": 0.8,
                        "source_memory_ids": list(ids),
                    }
                ]
            }
        return {}


def _completed_session(config: HieronymusConfig, count: int = 2) -> int:
    Registry(config).create_series(
        slug="book",
        title="Book",
        source_language="en",
        target_language="ru",
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(TranslationContext("book", "en", "ru", task_type="reading"))
    for index in range(count):
        workspace.add_short_term_memory(session.id, "user", "reading", f"Conclusion {index}.")
    workspace.complete_session(session.id)
    return session.id


def test_evidence_dream_runs_all_passes_over_the_same_selection(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    _completed_session(config)
    workspace = WorkspaceStore(config)
    second_session = workspace.start_session(
        TranslationContext("book", "en", "ru", task_type="reading")
    )
    workspace.add_short_term_memory(
        second_session.id,
        "user",
        "reading",
        "A conclusion from the next completed session.",
    )
    workspace.complete_session(second_session.id)
    provider = EvidenceProvider()

    run = DreamService(config, provider).run_cycle()

    assert run.status == "completed"
    assert [name for name, _ids in provider.calls] == list(DREAM_PASS_NAMES)
    assert len({ids for _name, ids in provider.calls}) == 1
    assert len(provider.calls[0][1]) == 3
    with connect(config.database_path) as conn:
        phases = conn.execute("select phase from dream_phase_runs order by id").fetchall()
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]
    assert [row["phase"] for row in phases] == list(DREAM_PASS_NAMES)
    assert crystal_count == 1


def test_incomplete_coverage_rolls_back_all_dream_mutations(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    session_id = _completed_session(config)

    with pytest.raises(ValueError, match="coverage_incomplete"):
        DreamService(config, EvidenceProvider(omit_coverage=True)).run_cycle()

    with connect(config.database_path) as conn:
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]
    assert crystal_count == 0
    assert len(WorkspaceStore(config).list_short_term_memories(session_id)) == 2


def test_book_scale_batch_is_covered_by_every_dream_pass(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "memory")
    Registry(config).create_series(
        slug="book",
        title="Book",
        source_language="en",
        target_language="ru",
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(TranslationContext("book", "en", "ru", task_type="reading"))
    memory_ids = workspace.add_short_term_memories_batch(
        session.id,
        [
            {
                "source_role": "mundane",
                "kind": "reading",
                "text": f"Distinct reading conclusion {index}.",
            }
            for index in range(500)
        ],
    )
    workspace.complete_session(session.id)
    provider = EvidenceProvider()

    run = DreamService(config, provider).run_cycle()

    assert run.status == "completed"
    assert all(ids == tuple(memory_ids) for _pass_name, ids in provider.calls)
    assert workspace.list_short_term_memories(session.id) == []
