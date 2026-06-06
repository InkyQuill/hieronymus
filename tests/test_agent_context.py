from __future__ import annotations

import json
from pathlib import Path

from hieronymus.agent_context import ProjectAgentContext, discover_project_context


def test_discover_project_context_reads_hieronymus_json(tmp_path: Path) -> None:
    (tmp_path / ".hieronymus.json").write_text(
        json.dumps(
            {
                "series_slug": "oso",
                "source_language": "ja",
                "target_language": "en",
                "volume": "1",
                "chapter": "2",
            }
        ),
        encoding="utf-8",
    )

    context = discover_project_context(tmp_path)

    assert context == ProjectAgentContext(
        series_slug="oso",
        source_language="ja",
        target_language="en",
        task_type="translation",
        volume="1",
        chapter="2",
    )


def test_discover_project_context_walks_upward_and_applies_defaults(tmp_path: Path) -> None:
    project = tmp_path / "translation" / "volume-1"
    chapter = project / "chapter-2"
    chapter.mkdir(parents=True)
    (project / ".hieronymus.json").write_text(
        json.dumps({"series_slug": "oso"}),
        encoding="utf-8",
    )

    context = discover_project_context(chapter)

    assert context == ProjectAgentContext(series_slug="oso")


def test_discover_project_context_returns_none_without_project_file(tmp_path: Path) -> None:
    assert discover_project_context(tmp_path) is None
