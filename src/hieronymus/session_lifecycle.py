from __future__ import annotations

from collections.abc import Callable
from datetime import UTC, datetime, timedelta

from hieronymus.config import HieronymusConfig
from hieronymus.workspace import WorkspaceStore


class SessionLifecycle:
    def __init__(
        self,
        config: HieronymusConfig,
        *,
        threshold_check: Callable[[], object],
    ) -> None:
        self._config = config
        self._threshold_check = threshold_check

    def run_due(self, now: datetime | None = None) -> tuple[int, ...]:
        cutoff = (now or datetime.now(UTC)) - timedelta(minutes=30)
        session_ids = WorkspaceStore(self._config).complete_inactive_sessions(cutoff)
        if session_ids:
            self._threshold_check()
        return session_ids
