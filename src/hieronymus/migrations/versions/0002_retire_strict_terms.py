"""One-time verified retirement of legacy strict-term storage."""

from __future__ import annotations

import sqlite3
from collections.abc import Callable
from pathlib import Path

from hieronymus.db import MigrationContext
from hieronymus.legacy_terms import (
    StrictTermConversionReport,
    StrictTermRetirementResult,
    convert_strict_terms,
    prepare_strict_term_retirement,
)
from hieronymus.legacy_terms import drop_legacy_tables as _drop_legacy_tables

DROP_PHASE_ENABLED = True


def verify_fresh(conn: sqlite3.Connection) -> None:
    legacy = conn.execute(
        """
        select name from sqlite_master
        where name in (
          'strict_terms', 'strict_term_tags', 'strict_term_aliases', 'strict_terms_fts'
        )
        """
    ).fetchall()
    if legacy:
        raise RuntimeError("fresh schema still declares legacy strict-term storage")


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
    _drop_legacy_tables(conn)


def migrate(conn: sqlite3.Connection, context: MigrationContext) -> None:
    """Back up, prove parity/ownership, and drop in the runner-owned transaction."""
    legacy_table = conn.execute(
        "select 1 from sqlite_master where type = 'table' and name = 'strict_terms'"
    ).fetchone()
    if legacy_table is None:
        return
    prepare_strict_term_retirement(
        conn,
        context.backup_root,
        transaction_owned_by_caller=True,
        finalizer=drop_legacy_tables,
    )
