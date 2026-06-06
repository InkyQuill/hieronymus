from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ProjectAgentContext:
    series_slug: str
    source_language: str = "ja"
    target_language: str = "en"
    task_type: str = "translation"
    volume: str = ""
    chapter: str = ""


def discover_project_context(cwd: Path) -> ProjectAgentContext | None:
    current = cwd.resolve()
    for path in (current, *current.parents):
        candidate = path / ".hieronymus.json"
        if not candidate.exists():
            continue

        payload = json.loads(candidate.read_text(encoding="utf-8"))
        return ProjectAgentContext(
            series_slug=str(payload["series_slug"]),
            source_language=str(payload.get("source_language", "ja")),
            target_language=str(payload.get("target_language", "en")),
            task_type=str(payload.get("task_type", "translation")),
            volume=str(payload.get("volume", "")),
            chapter=str(payload.get("chapter", "")),
        )
    return None
