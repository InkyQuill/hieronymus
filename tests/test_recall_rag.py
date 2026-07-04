from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.rag import RagStore
from hieronymus.recall import RecallService
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
        task_type="translation",
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("skill:name",),
    )


def test_recall_returns_memory_and_rag_results(config: HieronymusConfig, tmp_path: Path) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)
    CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Sense Memory",
        text="Render Sense consistently in menu contexts.",
        strength=0.7,
        confidence=0.7,
    )
    source = tmp_path / "glossary.csv"
    source.write_text("source,target\nSense,Сенс\n", encoding="utf-8")
    RagStore(config).import_file(context.series_slug, source, source_ref="glossary.csv")

    results = RecallService(config).recall(session.id, context, "Sense", limit=4)

    assert {"long_term", "rag"} <= {result.source for result in results}
    rag = next(result for result in results if result.source == "rag")
    assert rag.rag_chunk is not None
    assert rag.rag_chunk.source_ref == "glossary.csv"
    assert rag.reason == "rag glossary match"


def test_active_rule_crystal_ranks_above_rag(config: HieronymusConfig, tmp_path: Path) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)
    rule_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="rule",
        title="Sense Rule",
        text="Sense is translated as Сенс.",
        strength=0.95,
        confidence=0.95,
        rule_intent="terminology",
        status="active",
    )
    source = tmp_path / "glossary.csv"
    source.write_text("source,target\nSense,Wrong\n", encoding="utf-8")
    RagStore(config).import_file(context.series_slug, source, source_ref="glossary.csv")

    results = RecallService(config).recall(session.id, context, "Sense", limit=4)

    assert results[0].source == "long_term"
    assert results[0].crystal is not None
    assert results[0].crystal.id == rule_id
    assert any(result.source == "rag" for result in results[1:])


def test_recall_fills_limit_from_memory_when_rag_is_empty(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)
    for index in range(3):
        CrystalStore(config).add_crystal(
            context,
            crystal_type="lesson",
            text=f"Sense memory note {index}.",
            strength=0.8,
            confidence=0.8,
        )

    results = RecallService(config).recall(session.id, context, "Sense", limit=3)

    assert len(results) == 3
    assert {result.source for result in results} == {"long_term"}
