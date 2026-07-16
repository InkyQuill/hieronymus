from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class DirectStoreCommand:
    name: str
    consumer: str
    reason: str


DIRECT_STORE_COMMANDS: tuple[DirectStoreCommand, ...] = (
    DirectStoreCommand(
        name="init-series",
        consumer="human-debug",
        reason="bootstrap command that creates registry rows before a service mutation API exists",
    ),
    DirectStoreCommand(
        name="propose-term",
        consumer="human-debug",
        reason=(
            "legacy termbase helper retained for local debugging of deterministic terminology "
            "storage"
        ),
    ),
    DirectStoreCommand(
        name="validate",
        consumer="human-debug",
        reason=(
            "legacy termbase validator that reads files locally and checks deterministic "
            "terminology rules"
        ),
    ),
    DirectStoreCommand(
        name="remember",
        consumer="human-debug",
        reason="legacy long-memory helper retained until old memory primitives are fully retired",
    ),
    DirectStoreCommand(
        name="session-start",
        consumer="agent-automation",
        reason=(
            "agent workflow primitive that starts local workspace sessions through the domain store"
        ),
    ),
    DirectStoreCommand(
        name="session-complete",
        consumer="agent-automation",
        reason=(
            "agent workflow primitive that completes local workspace sessions through the domain "
            "store"
        ),
    ),
    DirectStoreCommand(
        name="remember-short",
        consumer="agent-automation",
        reason=(
            "agent workflow primitive that writes short-term observations through the domain store"
        ),
    ),
    DirectStoreCommand(
        name="recall",
        consumer="agent-automation",
        reason=(
            "agent workflow primitive that combines recall service output without parsing human "
            "CLI text"
        ),
    ),
    DirectStoreCommand(
        name="feedback",
        consumer="agent-automation",
        reason="agent workflow primitive that records correction events through the feedback store",
    ),
    DirectStoreCommand(
        name="dream",
        consumer="maintenance",
        reason=(
            "maintenance command that invokes DreamService directly so local dreaming works "
            "without a daemon"
        ),
    ),
)
