from __future__ import annotations

import json
from dataclasses import replace
from datetime import UTC, datetime, timedelta

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect
from hieronymus.dream_autostart import (
    AutostartState,
    DreamAutostart,
    load_autostart_state,
    save_autostart_state,
)
from hieronymus.dream_locks import dream_cycle_lock
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.settings import load_settings, save_settings
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
        task_type="translate",
        volume="1",
        chapter="2",
    )


def _enable_autostart(
    config: HieronymusConfig,
    *,
    min_interval_minutes: int = 30,
    new_short_term_memory_threshold: int = 25,
    max_cycles_per_autostart: int = 1,
) -> None:
    settings = load_settings(config)
    save_settings(
        config,
        settings.with_dreaming(
            replace(
                settings.dreaming,
                autostart_enabled=True,
                min_interval_minutes=min_interval_minutes,
                new_short_term_memory_threshold=new_short_term_memory_threshold,
                max_cycles_per_autostart=max_cycles_per_autostart,
            )
        ),
    )


def _completed_session(
    config: HieronymusConfig,
    context: TranslationContext,
    *,
    memories: int = 1,
) -> int:
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    for index in range(memories):
        workspace.add_short_term_memory(
            session.id,
            "user",
            "note",
            f"Completed short-term memory {index}.",
        )
    workspace.complete_session(session.id)
    return session.id


def test_status_counts_pending_short_term_memories_and_completed_sessions(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    pending_session_id = _completed_session(config, context, memories=2)
    archived_session_id = _completed_session(config, context, memories=1)
    active_session = WorkspaceStore(config).start_session(context)
    WorkspaceStore(config).add_short_term_memory(
        active_session.id,
        "user",
        "note",
        "Active session memory is not pending.",
    )
    with connect(config.database_path) as conn:
        conn.execute(
            "update short_term_memories set archived_at = ? where session_id = ?",
            ("2026-06-07T00:00:00+00:00", archived_session_id),
        )
        conn.commit()

    status = DreamAutostart(config).status()

    assert pending_session_id
    assert status["pending_completed_sessions"] == 1
    assert status["pending_short_term_memories"] == 2


def test_volume_trigger_runs_when_threshold_is_reached(config: HieronymusConfig) -> None:
    _enable_autostart(config, new_short_term_memory_threshold=2)
    _completed_session(config, _context(config), memories=2)
    now = datetime(2026, 6, 7, 12, 0, tzinfo=UTC)

    result = DreamAutostart(config).run_due(now=now)

    assert result == {"ran": True, "reason": "threshold", "cycles": 1}
    assert DreamAutostart(config).status()["pending_short_term_memories"] == 0
    assert load_autostart_state(config).last_started_at == now
    assert load_autostart_state(config).last_error == ""


def test_autostart_skips_and_records_when_cycle_is_active(
    config: HieronymusConfig,
) -> None:
    _enable_autostart(config, new_short_term_memory_threshold=1)
    _completed_session(config, _context(config), memories=1)
    now = datetime(2026, 6, 7, 12, 0, tzinfo=UTC)

    with dream_cycle_lock(config, owner="manual"):
        result = DreamAutostart(config).run_due(now=now)
        status = DreamAutostart(config).status()

    assert result == {"ran": False, "reason": "cycle-active", "cycles": 0}
    assert status["cycle_active"] is True
    assert status["active_cycle"]["owner"] == "manual"
    assert status["last_skip_reason"] == "cycle-active"
    assert status["last_skipped_at"] == now.isoformat()

    with connect(config.database_path) as conn:
        run = conn.execute("select status, error from dream_runs").fetchone()
    assert run["status"] == "skipped"
    assert run["error"] == "dream cycle already running"


def test_interval_trigger_requires_pending_memory_and_elapsed_minutes(
    config: HieronymusConfig,
) -> None:
    _enable_autostart(config, min_interval_minutes=30, new_short_term_memory_threshold=25)
    now = datetime(2026, 6, 7, 12, 0, tzinfo=UTC)

    assert DreamAutostart(config).run_due(now=now) == {
        "ran": False,
        "reason": "no-pending-memory",
        "cycles": 0,
    }

    _completed_session(config, _context(config), memories=1)
    save_autostart_state(
        config,
        AutostartState(last_started_at=now - timedelta(minutes=29)),
    )
    assert DreamAutostart(config).run_due(now=now) == {
        "ran": False,
        "reason": "not-due",
        "cycles": 0,
    }

    save_autostart_state(
        config,
        AutostartState(last_started_at=now - timedelta(minutes=31)),
    )
    assert DreamAutostart(config).run_due(now=now) == {
        "ran": True,
        "reason": "interval",
        "cycles": 1,
    }


def test_autostart_does_not_record_started_at_for_zero_cycles(
    config: HieronymusConfig,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _enable_autostart(config, new_short_term_memory_threshold=1)
    now = datetime(2026, 6, 7, 12, 0, tzinfo=UTC)
    counts = iter([(1, 1), (0, 0)])

    def pending_counts(self):
        return next(counts)

    monkeypatch.setattr(DreamAutostart, "_pending_counts", pending_counts)

    result = DreamAutostart(config).run_due(now=now)

    assert result == {"ran": False, "reason": "threshold", "cycles": 0}
    assert not (config.config_root / "dream-autostart.json").exists()
    assert load_autostart_state(config).last_started_at is None


def test_autostart_state_round_trips(config: HieronymusConfig) -> None:
    last_started_at = datetime(2026, 6, 7, 12, 0, tzinfo=UTC)
    last_skipped_at = datetime(2026, 6, 7, 13, 0, tzinfo=UTC)
    state = AutostartState(
        last_started_at=last_started_at,
        last_error="provider failed",
        last_skipped_at=last_skipped_at,
        last_skip_reason="cycle-active",
    )

    save_autostart_state(config, state)

    assert load_autostart_state(config) == state


def test_autostart_state_loads_legacy_payload(config: HieronymusConfig) -> None:
    last_started_at = datetime(2026, 6, 7, 12, 0, tzinfo=UTC)
    state_path = config.config_root / "dream-autostart.json"
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state_path.write_text(
        json.dumps(
            {
                "last_started_at": last_started_at.isoformat(),
                "last_error": "provider failed",
            },
            ensure_ascii=False,
            sort_keys=True,
        ),
        encoding="utf-8",
    )

    state = load_autostart_state(config)

    assert state.last_started_at == last_started_at
    assert state.last_error == "provider failed"
    assert state.last_skipped_at is None
    assert state.last_skip_reason == ""


def test_run_due_persists_last_error_for_corrupt_autostart_state(
    config: HieronymusConfig,
) -> None:
    _enable_autostart(config, min_interval_minutes=30, new_short_term_memory_threshold=25)
    _completed_session(config, _context(config), memories=1)
    state_path = config.config_root / "dream-autostart.json"
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state_path.write_text("{not valid json", encoding="utf-8")

    with pytest.raises(json.JSONDecodeError):
        DreamAutostart(config).run_due(now=datetime(2026, 6, 7, 12, 0, tzinfo=UTC))

    state = load_autostart_state(config)
    assert state.last_started_at is None
    assert "Expecting property name" in state.last_error
