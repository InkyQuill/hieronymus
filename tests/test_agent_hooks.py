from __future__ import annotations

import json
from pathlib import Path

from click.testing import CliRunner

from hieronymus.agent_hooks import main


def test_hook_session_start_outputs_missing_context_when_no_project_file(tmp_path: Path) -> None:
    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path), "--json"])

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "event": "session-start",
        "handled": False,
        "reason": "no .hieronymus.json context found",
    }


def test_hook_session_start_outputs_context_when_project_file_exists(tmp_path: Path) -> None:
    (tmp_path / ".hieronymus.json").write_text(
        json.dumps(
            {
                "series_slug": "oso",
                "source_language": "ja",
                "target_language": "en",
                "task_type": "translation",
                "volume": "1",
                "chapter": "2",
            }
        ),
        encoding="utf-8",
    )

    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path), "--json"])

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "event": "session-start",
        "handled": True,
        "series_slug": "oso",
        "source_language": "ja",
        "target_language": "en",
        "task_type": "translation",
        "volume": "1",
        "chapter": "2",
    }


def test_hook_session_start_human_output_is_concise(tmp_path: Path) -> None:
    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path)])

    assert result.exit_code == 0
    assert result.output == "no .hieronymus.json context found\n"


def test_hook_session_end_outputs_json() -> None:
    result = CliRunner().invoke(main, ["session-end", "--json"])

    assert result.exit_code == 0
    assert json.loads(result.output) == {"event": "session-end", "handled": True}
