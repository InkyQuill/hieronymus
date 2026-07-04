from pathlib import Path

from hieronymus import mcp_server
from hieronymus.config import HieronymusConfig
from hieronymus.registry import Registry


def test_mcp_rag_import_and_search(monkeypatch, tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    config = HieronymusConfig(data_root=data_root)
    Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    source = tmp_path / "chapter.txt"
    source.write_text("Cooking Talent appears here.", encoding="utf-8")
    monkeypatch.setattr(mcp_server, "load_config", lambda: config)

    imported = mcp_server.hieronymus_rag_import(
        "only-sense-online",
        str(source),
        source_ref="chapter.txt",
    )
    hits = mcp_server.hieronymus_rag_search("only-sense-online", "Cooking Talent")

    assert imported["chunk_count"] == 1
    assert imported["skipped"] is False
    assert hits[0]["source_ref"] == "chapter.txt"
    assert hits[0]["rank_reason"] == "rag project text match"
