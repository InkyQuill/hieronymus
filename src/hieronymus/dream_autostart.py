from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.dream_locks import read_dream_cycle_state
from hieronymus.dream_providers import resolve_provider
from hieronymus.dreaming import DreamService
from hieronymus.settings import load_settings


@dataclass(frozen=True)
class AutostartState:
    last_started_at: datetime | None = None
    last_error: str = ""
    last_skipped_at: datetime | None = None
    last_skip_reason: str = ""

    def to_json_dict(self) -> dict[str, object]:
        return {
            "last_started_at": self.last_started_at.isoformat()
            if self.last_started_at is not None
            else None,
            "last_error": self.last_error,
            "last_skipped_at": self.last_skipped_at.isoformat()
            if self.last_skipped_at is not None
            else None,
            "last_skip_reason": self.last_skip_reason,
        }


def _state_path(config: HieronymusConfig):
    return config.config_root / "dream-autostart.json"


def load_autostart_state(config: HieronymusConfig) -> AutostartState:
    path = _state_path(config)
    if not path.exists():
        return AutostartState()
    payload = json.loads(path.read_text(encoding="utf-8"))
    return AutostartState(
        last_started_at=_parse_datetime(payload.get("last_started_at"), "last_started_at"),
        last_error=str(payload.get("last_error", "")),
        last_skipped_at=_parse_datetime(payload.get("last_skipped_at"), "last_skipped_at"),
        last_skip_reason=str(payload.get("last_skip_reason", "")),
    )


def save_autostart_state(config: HieronymusConfig, state: AutostartState) -> None:
    atomic_write_text(
        _state_path(config),
        json.dumps(state.to_json_dict(), ensure_ascii=False, sort_keys=True),
    )


class DreamAutostart:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config

    def status(self) -> dict[str, object]:
        settings = load_settings(self.config)
        state = load_autostart_state(self.config)
        pending_completed_sessions, pending_short_term_memories = self._pending_counts()
        active_cycle = read_dream_cycle_state(self.config)
        active_cycle_payload = (
            {
                "owner": active_cycle.owner,
                "pid": active_cycle.pid,
                "started_at": active_cycle.started_at,
            }
            if active_cycle is not None
            else None
        )
        state_payload = state.to_json_dict()
        return {
            "enabled": settings.dreaming.autostart_enabled,
            "active_provider": settings.dreaming.active_provider,
            "min_interval_minutes": settings.dreaming.min_interval_minutes,
            "new_short_term_memory_threshold": settings.dreaming.new_short_term_memory_threshold,
            "max_cycles_per_autostart": settings.dreaming.max_cycles_per_autostart,
            "pending_completed_sessions": pending_completed_sessions,
            "pending_short_term_memories": pending_short_term_memories,
            "last_started_at": state_payload["last_started_at"],
            "last_error": state.last_error,
            "last_skipped_at": state_payload["last_skipped_at"],
            "last_skip_reason": state.last_skip_reason,
            "cycle_active": active_cycle is not None,
            "active_cycle": active_cycle_payload,
        }

    def run_due(self, now: datetime | None = None) -> dict[str, object]:
        now = now or datetime.now(UTC)
        state_for_error = AutostartState()
        attempted_run = False
        try:
            settings = load_settings(self.config)
            if not settings.dreaming.autostart_enabled:
                return {"ran": False, "reason": "disabled", "cycles": 0}

            _pending_completed_sessions, pending_short_term_memories = self._pending_counts()
            if pending_short_term_memories == 0:
                return {"ran": False, "reason": "no-pending-memory", "cycles": 0}

            state = load_autostart_state(self.config)
            state_for_error = state
            if pending_short_term_memories >= settings.dreaming.new_short_term_memory_threshold:
                reason = "threshold"
            elif self._interval_elapsed(
                now,
                state.last_started_at,
                settings.dreaming.min_interval_minutes,
            ):
                reason = "interval"
            else:
                return {"ran": False, "reason": "not-due", "cycles": 0}

            cycles = 0
            attempted_run = True
            service = DreamService(self.config, resolve_provider(self.config))
            for _ in range(settings.dreaming.max_cycles_per_autostart):
                _pending_completed_sessions, pending_short_term_memories = self._pending_counts()
                if pending_short_term_memories == 0:
                    break
                run = service.run_cycle(owner="autostart", skip_when_locked=True)
                if run.status == "skipped":
                    save_autostart_state(
                        self.config,
                        AutostartState(
                            last_started_at=state.last_started_at,
                            last_skipped_at=now,
                            last_skip_reason="cycle-active",
                        ),
                    )
                    return {"ran": False, "reason": "cycle-active", "cycles": cycles}
                cycles += 1
            save_autostart_state(self.config, AutostartState(last_started_at=now))
            return {"ran": True, "reason": reason, "cycles": cycles}
        except Exception as exc:
            save_autostart_state(
                self.config,
                AutostartState(
                    last_started_at=now if attempted_run else state_for_error.last_started_at,
                    last_error=str(exc),
                    last_skipped_at=state_for_error.last_skipped_at,
                    last_skip_reason=state_for_error.last_skip_reason,
                ),
            )
            raise

    def _pending_counts(self) -> tuple[int, int]:
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")
            row = conn.execute(
                """
                select
                  count(distinct task_sessions.id) as completed_sessions,
                  count(short_term_memories.id) as memories
                from task_sessions
                join short_term_memories
                  on short_term_memories.session_id = task_sessions.id
                where task_sessions.status = 'completed'
                  and task_sessions.cycle_id is null
                  and short_term_memories.archived_at is null
                """
            ).fetchone()
        return int(row["completed_sessions"]), int(row["memories"])

    def _interval_elapsed(
        self,
        now: datetime,
        last_started_at: datetime | None,
        min_interval_minutes: int,
    ) -> bool:
        if last_started_at is None:
            return True
        return (now - last_started_at).total_seconds() / 60 >= min_interval_minutes


def _parse_datetime(value: Any, field_name: str) -> datetime | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{field_name} must be a string or null")
    return datetime.fromisoformat(value)
