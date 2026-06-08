import json
import os
import subprocess
from pathlib import Path

from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.dream_locks import dream_cycle_lock
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.settings import ProviderSettings, load_settings, save_settings
from hieronymus.workspace import WorkspaceStore


def _create_series(data_root: Path) -> TranslationContext:
    config = HieronymusConfig(data_root=data_root)
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
        volume="01",
        chapter="002",
    )


def _start_session(runner: CliRunner, data_root: Path) -> int:
    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "session-start",
            "only-sense-online",
            "--source-language",
            "ja",
            "--target-language",
            "en",
            "--task-type",
            "translation",
            "--volume",
            "01",
            "--chapter",
            "002",
        ],
    )

    assert result.exit_code == 0
    return json.loads(result.output)["session_id"]


def test_init_series_outputs_json_and_creates_database(tmp_path):
    data_root = tmp_path / "hieronymus"
    runner = CliRunner()

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "init-series",
            "only-sense-online",
            "--title",
            "Only Sense Online",
        ],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload == {
        "slug": "only-sense-online",
        "database_path": str(data_root / "hieronymus.sqlite"),
    }
    assert (data_root / "hieronymus.sqlite").exists()


def test_unknown_series_returns_clean_click_error(tmp_path):
    data_root = tmp_path / "hieronymus"
    runner = CliRunner()

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "remember",
            "missing-series",
            "--kind",
            "translation_rationale",
            "--text",
            "Use concise system messages.",
        ],
    )

    assert result.exit_code == 1
    assert "Error: unknown series: missing-series" in result.output
    assert "Traceback" not in result.output


def test_data_root_rejects_existing_file_without_traceback(tmp_path):
    data_root = tmp_path / "data-root-file"
    data_root.write_text("not a directory", encoding="utf-8")

    result = subprocess.run(
        [
            "uv",
            "run",
            "hieronymus",
            "--data-root",
            str(data_root),
            "init-series",
            "only-sense-online",
            "--title",
            "Only Sense Online",
        ],
        check=False,
        cwd=Path.cwd(),
        text=True,
        capture_output=True,
    )

    assert result.returncode != 0
    assert "Invalid value for '--data-root'" in result.stderr
    assert "Traceback" not in result.stderr


def test_env_data_root_rejects_existing_file_without_traceback(tmp_path):
    data_root = tmp_path / "env-data-root-file"
    data_root.write_text("not a directory", encoding="utf-8")

    result = subprocess.run(
        [
            "uv",
            "run",
            "hieronymus",
            "init-series",
            "only-sense-online",
            "--title",
            "Only Sense Online",
        ],
        check=False,
        cwd=Path.cwd(),
        env={**os.environ, "HIERONYMUS_DATA_ROOT": str(data_root)},
        text=True,
        capture_output=True,
    )

    assert result.returncode != 0
    assert "data root is not a directory" in result.stderr
    assert str(data_root) in result.stderr
    assert "Traceback" not in result.stderr


def test_console_entrypoint_init_series_outputs_json(tmp_path):
    data_root = tmp_path / "hieronymus"

    result = subprocess.run(
        [
            "uv",
            "run",
            "hieronymus",
            "--data-root",
            str(data_root),
            "init-series",
            "only-sense-online",
            "--title",
            "Only Sense Online",
        ],
        check=False,
        cwd=Path.cwd(),
        text=True,
        capture_output=True,
    )

    assert result.returncode == 0
    payload = json.loads(result.stdout)
    assert payload == {
        "slug": "only-sense-online",
        "database_path": str(data_root / "hieronymus.sqlite"),
    }


def test_session_start_outputs_session_id_json(tmp_path):
    data_root = tmp_path / "hieronymus"
    _create_series(data_root)
    runner = CliRunner()

    session_id = _start_session(runner, data_root)

    assert session_id > 0


def test_session_start_uses_registry_languages_when_options_are_omitted(tmp_path):
    data_root = tmp_path / "hieronymus"
    _create_series(data_root)
    runner = CliRunner()

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "session-start",
            "only-sense-online",
            "--task-type",
            "translation",
        ],
    )

    assert result.exit_code == 0
    session_id = json.loads(result.output)["session_id"]
    with connect(data_root / "hieronymus.sqlite") as conn:
        row = conn.execute(
            "select source_language, target_language from task_sessions where id = ?",
            (session_id,),
        ).fetchone()
    assert row["source_language"] == "ja"
    assert row["target_language"] == "en"


def test_session_start_rejects_language_mismatch(tmp_path):
    data_root = tmp_path / "hieronymus"
    _create_series(data_root)
    runner = CliRunner()

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "session-start",
            "only-sense-online",
            "--source-language",
            "ko",
            "--target-language",
            "en",
            "--task-type",
            "translation",
        ],
    )

    assert result.exit_code == 1
    assert "Error: source_language does not match series source_language: ja" in result.output
    assert "Traceback" not in result.output


def test_session_complete_outputs_completed_json(tmp_path):
    data_root = tmp_path / "hieronymus"
    _create_series(data_root)
    runner = CliRunner()
    session_id = _start_session(runner, data_root)

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "session-complete",
            str(session_id),
        ],
    )

    assert result.exit_code == 0
    assert json.loads(result.output) == {"session_id": session_id, "completed": True}


def test_remember_short_outputs_memory_id_json(tmp_path):
    data_root = tmp_path / "hieronymus"
    _create_series(data_root)
    runner = CliRunner()
    session_id = _start_session(runner, data_root)

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "remember-short",
            str(session_id),
            "--role",
            "user",
            "--kind",
            "correction",
            "--text",
            "Use Sense, not Feeling, in UI references.",
            "--source-ref",
            "v01c002",
        ],
    )

    assert result.exit_code == 0
    assert json.loads(result.output)["memory_id"] > 0


def test_dream_outputs_completed_cycle_after_completed_session_with_memory(tmp_path):
    data_root = tmp_path / "hieronymus"
    _create_series(data_root)
    runner = CliRunner()
    session_id = _start_session(runner, data_root)
    runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "remember-short",
            str(session_id),
            "--role",
            "user",
            "--kind",
            "correction",
            "--text",
            "Use Sense, not Feeling, in UI references.",
        ],
    )
    runner.invoke(
        main,
        ["--data-root", str(data_root), "session-complete", str(session_id)],
    )

    result = runner.invoke(
        main,
        ["--data-root", str(data_root), "dream", "--provider", "deterministic"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload == {
        "cycle_id": 1,
        "status": "completed",
        "provider": "deterministic",
        "input_count": 1,
        "created_crystal_count": 1,
        "proposal_count": 0,
        "error": "",
    }


def test_config_json_does_not_include_raw_api_key_value(tmp_path, monkeypatch):
    data_root = tmp_path / "hieronymus"
    config = HieronymusConfig(data_root=data_root)
    monkeypatch.setenv("HIERONYMUS_OPENAI_KEY", "raw-secret-value")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="HIERONYMUS_OPENAI_KEY",
            base_url="https://api.example.test/v1",
        ),
    )
    save_settings(config, settings)

    result = CliRunner().invoke(main, ["--data-root", str(data_root), "config", "--json"])

    assert result.exit_code == 0
    assert "HIERONYMUS_OPENAI_KEY" in result.output
    assert "raw-secret-value" not in result.output


def test_recall_outputs_ranked_crystal_results(tmp_path):
    data_root = tmp_path / "hieronymus"
    context = _create_series(data_root)
    crystal_id = CrystalStore(HieronymusConfig(data_root=data_root)).add_crystal(
        context,
        crystal_type="lesson",
        text="Use Sense as a technical UI term.",
        strength=0.7,
        confidence=0.8,
    )
    runner = CliRunner()
    session_id = _start_session(runner, data_root)

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "recall",
            str(session_id),
            "--series",
            "only-sense-online",
            "--query",
            "Sense UI",
            "--source-language",
            "ja",
            "--target-language",
            "en",
            "--task-type",
            "translation",
            "--volume",
            "01",
            "--chapter",
            "002",
        ],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload[0]["crystal_id"] == crystal_id
    assert payload[0]["rank"] == 1
    assert payload[0]["score"] > 0
    assert payload[0]["reason"] == "weighted search match"


def test_recall_rejects_series_mismatch_without_writing_trace(tmp_path):
    data_root = tmp_path / "hieronymus"
    _create_series(data_root)
    config = HieronymusConfig(data_root=data_root)
    other_series = Registry(config).create_series(
        slug="another-series",
        title="Another Series",
        source_language="ja",
        target_language="en",
    )
    other_context = TranslationContext(
        series_slug=other_series.slug,
        source_language=other_series.source_language,
        target_language=other_series.target_language,
        task_type="translation",
        volume="01",
        chapter="002",
    )
    CrystalStore(config).add_crystal(
        other_context,
        crystal_type="lesson",
        text="Use Sense as a different-series term.",
    )
    runner = CliRunner()
    session_id = _start_session(runner, data_root)

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "recall",
            str(session_id),
            "--series",
            "another-series",
            "--query",
            "Sense",
            "--source-language",
            "ja",
            "--target-language",
            "en",
            "--task-type",
            "translation",
            "--volume",
            "01",
            "--chapter",
            "002",
        ],
    )

    assert result.exit_code == 1
    assert "Error: recall series_slug does not match session context" in result.output
    assert "Traceback" not in result.output
    with connect(data_root / "hieronymus.sqlite") as conn:
        activation_count = conn.execute("select count(*) from crystal_activations").fetchone()[0]
        memory_count = conn.execute("select count(*) from short_term_memories").fetchone()[0]
    assert activation_count == 0
    assert memory_count == 0


def test_recall_rejects_language_mismatch_without_writing_trace(tmp_path):
    data_root = tmp_path / "hieronymus"
    context = _create_series(data_root)
    CrystalStore(HieronymusConfig(data_root=data_root)).add_crystal(
        context,
        crystal_type="lesson",
        text="Use Sense as a technical UI term.",
    )
    runner = CliRunner()
    session_id = _start_session(runner, data_root)

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "recall",
            str(session_id),
            "--series",
            "only-sense-online",
            "--query",
            "Sense",
            "--source-language",
            "ko",
            "--target-language",
            "en",
            "--task-type",
            "translation",
            "--volume",
            "01",
            "--chapter",
            "002",
        ],
    )

    assert result.exit_code == 1
    assert "Error: recall source_language does not match session context" in result.output
    assert "Traceback" not in result.output
    with connect(data_root / "hieronymus.sqlite") as conn:
        activation_count = conn.execute("select count(*) from crystal_activations").fetchone()[0]
        memory_count = conn.execute("select count(*) from short_term_memories").fetchone()[0]
    assert activation_count == 0
    assert memory_count == 0


def test_feedback_outputs_event_id_json(tmp_path):
    data_root = tmp_path / "hieronymus"
    context = _create_series(data_root)
    crystal_id = CrystalStore(HieronymusConfig(data_root=data_root)).add_crystal(
        context,
        crystal_type="lesson",
        text="Use Sense as a technical UI term.",
    )
    runner = CliRunner()

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "feedback",
            str(crystal_id),
            "--event",
            "confirmed_by_user",
            "--role",
            "user",
            "--evidence",
            "Accepted during review.",
        ],
    )

    assert result.exit_code == 0
    assert json.loads(result.output)["event_id"] > 0


def test_feedback_rejects_mismatched_session_without_event(tmp_path):
    data_root = tmp_path / "hieronymus"
    alpha_context = _create_series(data_root)
    config = HieronymusConfig(data_root=data_root)
    Registry(config).create_series(
        slug="beta-series",
        title="Beta Series",
        source_language="ja",
        target_language="en",
    )
    beta_session = WorkspaceStore(config).start_session(
        TranslationContext(
            series_slug="beta-series",
            source_language="ja",
            target_language="en",
            task_type="translation",
        )
    )
    crystal_id = CrystalStore(config).add_crystal(
        alpha_context,
        crystal_type="lesson",
        text="Use Sense as a technical UI term.",
    )
    runner = CliRunner()

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "feedback",
            str(crystal_id),
            "--event",
            "confirmed_by_user",
            "--role",
            "user",
            "--session-id",
            str(beta_session.id),
        ],
    )

    assert result.exit_code == 1
    assert "Error: feedback session series_slug does not match crystal context" in result.output
    assert "Traceback" not in result.output
    with connect(data_root / "hieronymus.sqlite") as conn:
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]
    assert event_count == 0


def test_dream_unsupported_provider_returns_clean_click_error(tmp_path):
    data_root = tmp_path / "hieronymus"
    runner = CliRunner()

    result = runner.invoke(
        main,
        ["--data-root", str(data_root), "dream", "--provider", "llm"],
    )

    assert result.exit_code == 1
    assert "Error: unsupported dream provider: llm" in result.output
    assert "Traceback" not in result.output


def test_dream_returns_clean_error_when_cycle_is_active(tmp_path):
    data_root = tmp_path / "hieronymus"
    config = HieronymusConfig(data_root=data_root)
    runner = CliRunner()

    with dream_cycle_lock(config, owner="manual"):
        result = runner.invoke(main, ["--data-root", str(data_root), "dream"])

    assert result.exit_code == 1
    assert "Error: dream cycle already running" in result.output
    assert "Traceback" not in result.output
