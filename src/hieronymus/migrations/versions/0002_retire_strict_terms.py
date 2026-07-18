"""Prepared strict-term retirement phases.

This module is deliberately not registered with the SQL-only ordered migration runner.
Task 3 will activate the destructive phase after runtime legacy access is removed.
"""

from __future__ import annotations

import sqlite3
from collections.abc import Callable
from pathlib import Path

from hieronymus.legacy_terms import (
    StrictTermRetirementResult,
    prepare_strict_term_retirement,
)
from hieronymus.legacy_terms import (
    drop_legacy_tables as _disabled_drop_legacy_tables,
)
from hieronymus.memory_migration import StrictTermConversionReport, convert_strict_terms

DROP_PHASE_ENABLED = False


def backup_and_convert(
    conn: sqlite3.Connection,
    backup_dir: Path,
    *,
    timestamp: str | None = None,
    failure_hook: Callable[[int], None] | None = None,
) -> StrictTermRetirementResult:
    return prepare_strict_term_retirement(
        conn,
        backup_dir,
        timestamp=timestamp,
        failure_hook=failure_hook,
    )


def convert(conn: sqlite3.Connection) -> StrictTermConversionReport:
    """Run only the conversion phase inside a caller-owned transaction."""
    return convert_strict_terms(conn)


def drop_legacy_tables(conn: sqlite3.Connection) -> None:
    """Remain inert until Task 3 explicitly implements and enables cutover."""
    _disabled_drop_legacy_tables(conn)
