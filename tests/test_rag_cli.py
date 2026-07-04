import json
from pathlib import Path

from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus.config import HieronymusConfig
from hieronymus.registry import Registry


def _series(data_root: Path) -> None:
    Registry(HieronymusConfig(data_root=data_root)).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )


def test_rag_import_and_search_json(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    _series(data_root)
    source = tmp_path / "glossary.csv"
    source.write_text("source,target\nSense,Сенс\n", encoding="utf-8")
    runner = CliRunner()

    import_result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "rag",
            "import",
            "only-sense-online",
            str(source),
            "--source-ref",
            "glossary.csv",
            "--json",
        ],
    )
    search_result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "rag",
            "search",
            "only-sense-online",
            "Sense",
            "--json",
        ],
    )

    assert import_result.exit_code == 0
    assert json.loads(import_result.output)["chunk_count"] == 1
    assert search_result.exit_code == 0
    payload = json.loads(search_result.output)
    assert payload[0]["source_ref"] == "glossary.csv"
    assert payload[0]["rank_reason"] == "rag glossary match"
    assert payload[0]["metadata"]["target"] == "Сенс"


def test_rag_import_rejects_unknown_series(tmp_path: Path) -> None:
    source = tmp_path / "chapter.txt"
    source.write_text("Sense note.", encoding="utf-8")

    result = CliRunner().invoke(
        main,
        [
            "--data-root",
            str(tmp_path / "hieronymus"),
            "rag",
            "import",
            "missing",
            str(source),
        ],
    )

    assert result.exit_code == 1
    assert "unknown series: missing" in result.output
