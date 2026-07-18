from __future__ import annotations

import asyncio
from collections import deque
from collections.abc import Callable
from datetime import UTC, datetime
from threading import Lock

_DEFAULT_SUBSCRIBER_CAPACITY = 32


class EventSubscriptionClosed(Exception):
    """Raised when an event subscription has been closed."""


class EventSubscriptionOverflow(EventSubscriptionClosed):
    """Raised after a slow subscriber exhausts its bounded backlog."""


class AdminEventSubscription:
    def __init__(
        self,
        *,
        loop: asyncio.AbstractEventLoop,
        capacity: int,
        remove: Callable[[AdminEventSubscription], None],
    ) -> None:
        self._loop = loop
        self._capacity = capacity
        self._remove = remove
        self._lock = Lock()
        self._events: deque[dict[str, object]] = deque()
        self._ready = asyncio.Event()
        self._closed = False
        self._overflowed = False

    def offer(self, event: dict[str, object]) -> None:
        overflowed = False
        with self._lock:
            if self._closed:
                return
            if len(self._events) >= self._capacity:
                self._events.clear()
                self._closed = True
                self._overflowed = True
                overflowed = True
            else:
                self._events.append(event)
        if overflowed:
            self._remove(self)
        self._wake_receiver()

    async def receive(self) -> dict[str, object]:
        while True:
            await self._ready.wait()
            with self._lock:
                if self._overflowed:
                    raise EventSubscriptionOverflow("event subscriber backlog overflowed")
                if self._events:
                    event = self._events.popleft()
                    if not self._events:
                        self._ready.clear()
                    return event
                if self._closed:
                    raise EventSubscriptionClosed("event subscription closed")
                self._ready.clear()

    def close(self) -> None:
        with self._lock:
            if self._closed:
                return
            self._closed = True
            self._events.clear()
        self._remove(self)
        self._wake_receiver()

    def _wake_receiver(self) -> None:
        try:
            self._loop.call_soon_threadsafe(self._ready.set)
        except RuntimeError:
            self._remove(self)


class AdminEventHub:
    def __init__(self, *, default_capacity: int = _DEFAULT_SUBSCRIBER_CAPACITY) -> None:
        if default_capacity < 1:
            raise ValueError("default_capacity must be at least 1")
        self._default_capacity = default_capacity
        self._lock = Lock()
        self._subscribers: set[AdminEventSubscription] = set()

    @property
    def subscriber_count(self) -> int:
        with self._lock:
            return len(self._subscribers)

    def subscribe(self, *, capacity: int | None = None) -> AdminEventSubscription:
        resolved_capacity = self._default_capacity if capacity is None else capacity
        if resolved_capacity < 1:
            raise ValueError("capacity must be at least 1")
        subscription = AdminEventSubscription(
            loop=asyncio.get_running_loop(),
            capacity=resolved_capacity,
            remove=self._remove,
        )
        with self._lock:
            self._subscribers.add(subscription)
        return subscription

    def publish(self, event_type: str, payload: dict[str, object]) -> None:
        event = {
            "type": event_type,
            "timestamp": datetime.now(UTC).isoformat(),
            "payload": payload,
        }
        with self._lock:
            subscribers = tuple(self._subscribers)
        for subscriber in subscribers:
            subscriber.offer(event)

    def _remove(self, subscriber: AdminEventSubscription) -> None:
        with self._lock:
            self._subscribers.discard(subscriber)
