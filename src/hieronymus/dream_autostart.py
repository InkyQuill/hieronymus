from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.dream_config import DreamConfig, load_dream_config
from hieronymus.dream_locks import read_dream_cycle_state
from hieronymus.dream_providers import resolve_provider
from hieronymus.dream_workflows import resolve_effective_workflow
from hieronymus.dreaming import DreamService
from hieronymus.provider_config import ProviderCatalogError, load_provider_catalog


@dataclass(frozen=True)
class AutostartState:
    last_started_at: datetime | None = None
    last_error: str = ""
    last_skipped_at: datetime | None = None
    last_skip_reason: str = ""
    not_enough_memories_skipped_count: int = 0

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
            "not_enough_memories_skipped_count": self.not_enough_memories_skipped_count,
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
        not_enough_memories_skipped_count=_parse_non_negative_int(
            payload.get("not_enough_memories_skipped_count", 0),
            "not_enough_memories_skipped_count",
        ),
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
        dream_config = load_dream_config(self.config)
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
            "enabled": dream_config.enabled,
            "active_provider": _active_provider(dream_config, load_provider_catalog(self.config)),
            "schedule_interval_minutes": dream_config.schedule_interval_minutes,
            "min_pending_short_term_memories": dream_config.min_pending_short_term_memories,
            "max_pending_short_term_memories": dream_config.max_pending_short_term_memories,
            "max_short_term_memories_per_cycle": dream_config.max_short_term_memories_per_cycle,
            "not_enough_memories_cycle_threshold": (
                dream_config.not_enough_memories_cycle_threshold
            ),
            "pending_completed_sessions": pending_completed_sessions,
            "pending_short_term_memories": pending_short_term_memories,
            "last_started_at": state_payload["last_started_at"],
            "last_error": state.last_error,
            "last_skipped_at": state_payload["last_skipped_at"],
            "last_skip_reason": state.last_skip_reason,
            "not_enough_memories_skipped_count": state.not_enough_memories_skipped_count,
            "skipped_count": state.not_enough_memories_skipped_count,
            "cycle_active": active_cycle is not None,
            "active_cycle": active_cycle_payload,
        }

    def run_due(self, now: datetime | None = None) -> dict[str, object]:
        now = now or datetime.now(UTC)
        state_for_error = AutostartState()
        attempted_run = False
        try:
            dream_config = load_dream_config(self.config)
            if not dream_config.enabled:
                return {"ran": False, "reason": "disabled", "cycles": 0}

            state = load_autostart_state(self.config)
            state_for_error = state
            _pending_completed_sessions, pending_short_term_memories = self._pending_counts()
            if pending_short_term_memories == 0:
                if (
                    state.not_enough_memories_skipped_count != 0
                    or state.last_skip_reason == "not_enough_memories"
                ):
                    save_autostart_state(
                        self.config,
                        AutostartState(
                            last_started_at=state.last_started_at,
                            last_error=state.last_error,
                            last_skipped_at=now,
                            last_skip_reason="no-pending-memory",
                        ),
                    )
                return {"ran": False, "reason": "no-pending-memory", "cycles": 0}

            ignore_minimum = False
            if pending_short_term_memories >= dream_config.max_pending_short_term_memories:
                reason = "urgent"
                ignore_minimum = True
            elif self._interval_elapsed(
                now,
                self._interval_anchor(state),
                dream_config.schedule_interval_minutes,
            ):
                if pending_short_term_memories >= dream_config.min_pending_short_term_memories:
                    reason = "scheduled"
                else:
                    skipped_count = state.not_enough_memories_skipped_count + 1
                    if skipped_count <= dream_config.not_enough_memories_cycle_threshold:
                        save_autostart_state(
                            self.config,
                            AutostartState(
                                last_started_at=state.last_started_at,
                                last_error=state.last_error,
                                last_skipped_at=now,
                                last_skip_reason="not_enough_memories",
                                not_enough_memories_skipped_count=skipped_count,
                            ),
                        )
                        return {
                            "ran": False,
                            "reason": "not_enough_memories",
                            "cycles": 0,
                        }
                    reason = "backlog_escape"
                    ignore_minimum = True
            else:
                return {"ran": False, "reason": "not-due", "cycles": 0}

            service = DreamService(self.config, resolve_provider(self.config))
            run = service.run_all(
                owner="autostart",
                skip_when_locked=True,
                ignore_minimum=ignore_minimum,
                trigger_type=reason,
            )
            if run.status == "skipped":
                save_autostart_state(
                    self.config,
                    AutostartState(
                        last_started_at=state.last_started_at,
                        last_error=state.last_error,
                        last_skipped_at=now,
                        last_skip_reason="cycle-active",
                        not_enough_memories_skipped_count=(state.not_enough_memories_skipped_count),
                    ),
                )
                return {"ran": False, "reason": "cycle-active", "cycles": 0}
            attempted_run = True
            save_autostart_state(self.config, AutostartState(last_started_at=now))
            return {"ran": True, "reason": reason, "cycles": 1}
        except Exception as exc:
            save_autostart_state(
                self.config,
                AutostartState(
                    last_started_at=now if attempted_run else state_for_error.last_started_at,
                    last_error=str(exc),
                    last_skipped_at=state_for_error.last_skipped_at,
                    last_skip_reason=state_for_error.last_skip_reason,
                    not_enough_memories_skipped_count=(
                        state_for_error.not_enough_memories_skipped_count
                    ),
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
        last_decision_at: datetime | None,
        min_interval_minutes: int,
    ) -> bool:
        if last_decision_at is None:
            return True
        return (now - last_decision_at).total_seconds() / 60 >= min_interval_minutes

    def _interval_anchor(self, state: AutostartState) -> datetime | None:
        if state.last_skip_reason == "not_enough_memories" and state.last_skipped_at is not None:
            return state.last_skipped_at
        if state.last_started_at is None:
            return state.last_skipped_at
        return state.last_started_at


def _active_provider(dream_config: DreamConfig, provider_catalog) -> str:
    if not dream_config.enabled:
        return "deterministic"
    workflow = dream_config.workflows.get("crystallization")
    if workflow is not None and workflow.enabled:
        return resolve_effective_workflow(
            dream_config,
            provider_catalog,
            "crystallization",
        ).provider
    for name, workflow in dream_config.workflows.items():
        if workflow.enabled:
            try:
                return resolve_effective_workflow(dream_config, provider_catalog, name).provider
            except ProviderCatalogError:
                return workflow.provider
    return "deterministic"


def _parse_datetime(value: Any, field_name: str) -> datetime | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{field_name} must be a string or null")
    return datetime.fromisoformat(value)


def _parse_non_negative_int(value: Any, field_name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{field_name} must be an integer")
    if value < 0:
        raise ValueError(f"{field_name} must be non-negative")
    return value
