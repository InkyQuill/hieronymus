from __future__ import annotations

from collections.abc import Callable
from datetime import UTC, datetime
from threading import Lock


class AdminEventHub:
    def __init__(self) -> None:
        self._lock = Lock()
        self._subscribers: list[Callable[[dict[str, object]], None]] = []

    def subscribe(self, subscriber: Callable[[dict[str, object]], None]) -> Callable[[], None]:
        with self._lock:
            self._subscribers.append(subscriber)

        def unsubscribe() -> None:
            with self._lock:
                if subscriber in self._subscribers:
                    self._subscribers.remove(subscriber)

        return unsubscribe

    def publish(self, event_type: str, payload: dict[str, object]) -> None:
        event = {
            "type": event_type,
            "timestamp": datetime.now(UTC).isoformat(),
            "payload": payload,
        }
        with self._lock:
            subscribers = tuple(self._subscribers)
        for subscriber in subscribers:
            subscriber(event)
