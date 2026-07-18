from __future__ import annotations

import hashlib
import json
import os
import sqlite3
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any
from uuid import uuid4

from hieronymus.memory_migration import (
    StrictTermConversionBlocked,
    convert_strict_terms,
)
from hieronymus.rule_crystals import parse_rule_crystal

BACKUP_FORMAT = "hieronymus.strict-terms-backup"
BACKUP_VERSION = 1
ACTIVE_STATUSES = frozenset({"approved", "active"})
LEGACY_TABLES = ("strict_terms", "strict_term_tags", "strict_term_aliases")


class LegacyTermRetirementBlocked(StrictTermConversionBlocked):
    """The retirement cannot proceed without losing legacy information."""


class LegacyTermRetirementDisabled(RuntimeError):
    """The destructive cutover is intentionally unavailable before Task 3."""


@dataclass(frozen=True)
class VerifiedLegacyTermsBackup:
    path: Path
    checksum: str
    term_count: int


@dataclass(frozen=True)
class StrictTermRetirementCoverage:
    total_terms: int
    active_terms: int
    migrated_terms: int
    inactive_audited: int
    blocked_term_ids: tuple[int, ...] = ()

    @property
    def complete(self) -> bool:
        return (
            self.active_terms == self.migrated_terms
            and self.total_terms - self.active_terms == self.inactive_audited
            and not self.blocked_term_ids
        )


@dataclass(frozen=True)
class StrictTermRetirementResult:
    backup: VerifiedLegacyTermsBackup
    coverage: StrictTermRetirementCoverage


def _rows(conn: sqlite3.Connection, table: str) -> list[dict[str, Any]]:
    cursor = conn.execute(f"select * from {table} order by rowid")
    columns = tuple(column[0] for column in cursor.description or ())
    return [dict(zip(columns, row, strict=True)) for row in cursor.fetchall()]


def _checksum_payload(payload: Mapping[str, object]) -> str:
    content = {key: value for key, value in payload.items() if key != "checksum"}
    encoded = json.dumps(
        content,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def write_legacy_terms_backup(
    conn: sqlite3.Connection,
    backup_dir: Path,
    *,
    timestamp: str | None = None,
) -> VerifiedLegacyTermsBackup:
    """Write and verify a complete snapshot without changing the database."""
    stamp = timestamp or datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    payload: dict[str, object] = {
        "format": BACKUP_FORMAT,
        "version": BACKUP_VERSION,
        "created_at": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
        "strict_terms": _rows(conn, "strict_terms"),
        "strict_term_tags": _rows(conn, "strict_term_tags"),
        "strict_term_aliases": _rows(conn, "strict_term_aliases"),
    }
    payload["checksum"] = _checksum_payload(payload)
    backup_dir.mkdir(parents=True, exist_ok=True)
    destination = backup_dir / f"strict-terms-v{BACKUP_VERSION}-{stamp}.json"
    temporary = backup_dir / f".{destination.name}.{uuid4().hex}.tmp"
    try:
        with temporary.open("x", encoding="utf-8") as stream:
            json.dump(payload, stream, ensure_ascii=False, sort_keys=True, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, destination)
        _fsync_directory(backup_dir)
    finally:
        temporary.unlink(missing_ok=True)

    verified = verify_legacy_terms_backup(destination)
    return VerifiedLegacyTermsBackup(
        path=destination,
        checksum=str(verified["checksum"]),
        term_count=len(verified["strict_terms"]),
    )


def _fsync_directory(directory: Path) -> None:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
    try:
        descriptor = os.open(directory, flags)
    except OSError:
        return
    try:
        os.fsync(descriptor)
    except OSError:
        pass
    finally:
        os.close(descriptor)


def verify_legacy_terms_backup(path: Path) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise LegacyTermRetirementBlocked(f"invalid legacy terms backup: {path}") from exc
    if not isinstance(payload, dict):
        raise LegacyTermRetirementBlocked("legacy terms backup root must be an object")
    if payload.get("format") != BACKUP_FORMAT or payload.get("version") != BACKUP_VERSION:
        raise LegacyTermRetirementBlocked("unsupported legacy terms backup format or version")
    for table in LEGACY_TABLES:
        if not isinstance(payload.get(table), list):
            raise LegacyTermRetirementBlocked(f"legacy terms backup is missing {table}")
    expected = _checksum_payload(payload)
    if payload.get("checksum") != expected:
        raise LegacyTermRetirementBlocked("legacy terms backup checksum mismatch")
    return payload


def prepare_strict_term_retirement(
    conn: sqlite3.Connection,
    backup_dir: Path,
    *,
    timestamp: str | None = None,
    failure_hook: Callable[[int], None] | None = None,
) -> StrictTermRetirementResult:
    """Back up, convert, audit, and verify legacy terms in one owned transaction."""
    if conn.in_transaction:
        raise sqlite3.ProgrammingError(
            "strict term retirement requires explicit transaction ownership; "
            "the supplied connection already has an active transaction"
        )

    conn.execute("begin immediate")
    try:
        backup = write_legacy_terms_backup(conn, backup_dir, timestamp=timestamp)
        payload = verify_legacy_terms_backup(backup.path)
        try:
            conversion = convert_strict_terms(conn, failure_hook=failure_hook)
        except StrictTermConversionBlocked as exc:
            raise LegacyTermRetirementBlocked(str(exc)) from exc
        inactive_audited = _audit_inactive_terms(conn, payload)
        coverage = check_strict_term_retirement_parity(
            conn,
            payload,
            migrated_terms=conversion.migrated_terms,
            inactive_audited=inactive_audited,
        )
        if not coverage.complete:
            raise LegacyTermRetirementBlocked(
                f"strict term retirement parity failed for terms {coverage.blocked_term_ids}"
            )
        conn.commit()
    except BaseException:
        conn.rollback()
        raise
    return StrictTermRetirementResult(backup=backup, coverage=coverage)


def _audit_inactive_terms(conn: sqlite3.Connection, payload: Mapping[str, object]) -> int:
    inactive = [
        row
        for row in payload["strict_terms"]
        if isinstance(row, dict) and row.get("status") not in ACTIVE_STATUSES
    ]
    tag_rows = [row for row in payload["strict_term_tags"] if isinstance(row, dict)]
    alias_rows = [row for row in payload["strict_term_aliases"] if isinstance(row, dict)]
    for row in inactive:
        entity_id = str(row["id"])
        exists = conn.execute(
            """
            select 1 from audit_log
            where action = 'strict_term_retirement'
              and entity_type = 'strict_term'
              and entity_id = ?
            """,
            (entity_id,),
        ).fetchone()
        if exists is not None:
            continue
        audit_snapshot = {
            "term": row,
            "tags": [tag for tag in tag_rows if str(tag.get("term_id")) == entity_id],
            "aliases": [alias for alias in alias_rows if str(alias.get("term_id")) == entity_id],
        }
        conn.execute(
            """
            insert into audit_log(
              actor, action, entity_type, entity_id, note, before_json, after_json, created_at
            ) values ('migration', 'strict_term_retirement', 'strict_term', ?,
                      'Preserved inactive legacy term before retirement.', ?, '{}', datetime('now'))
            """,
            (entity_id, json.dumps(audit_snapshot, ensure_ascii=False, sort_keys=True)),
        )
    return sum(
        conn.execute(
            """
            select exists(
              select 1 from audit_log
              where action = 'strict_term_retirement'
                and entity_type = 'strict_term'
                and entity_id = ?
            )
            """,
            (str(row["id"]),),
        ).fetchone()[0]
        for row in inactive
    )


def check_strict_term_retirement_parity(
    conn: sqlite3.Connection,
    payload: Mapping[str, object],
    *,
    migrated_terms: int,
    inactive_audited: int,
) -> StrictTermRetirementCoverage:
    terms = [row for row in payload["strict_terms"] if isinstance(row, dict)]
    active = [row for row in terms if row.get("status") in ACTIVE_STATUSES]
    aliases_by_term: dict[int, list[dict[str, Any]]] = {}
    for alias in payload["strict_term_aliases"]:
        if isinstance(alias, dict):
            aliases_by_term.setdefault(int(alias["term_id"]), []).append(alias)
    tags_by_term: dict[int, set[str]] = {}
    for tag in payload["strict_term_tags"]:
        if isinstance(tag, dict):
            tags_by_term.setdefault(int(tag["term_id"]), set()).add(str(tag["tag"]))

    blocked = tuple(
        int(term["id"])
        for term in active
        if not _active_term_has_parity(
            conn,
            term,
            aliases_by_term.get(int(term["id"]), []),
            tags_by_term.get(int(term["id"]), set()),
        )
    )
    return StrictTermRetirementCoverage(
        total_terms=len(terms),
        active_terms=len(active),
        migrated_terms=migrated_terms,
        inactive_audited=inactive_audited,
        blocked_term_ids=blocked,
    )


def _active_term_has_parity(
    conn: sqlite3.Connection,
    term: Mapping[str, Any],
    aliases: list[dict[str, Any]],
    tags: set[str],
) -> bool:
    term_id = str(term["id"])
    targets = {
        (row["source_id"], row["target_table"]): int(row["target_id"])
        for row in conn.execute(
            """
            select source_id, target_table, target_id
            from memory_graph_migration_ledger
            where source_table = 'strict_terms'
              and (source_id = ? or source_id like ?)
            """,
            (term_id, f"{term_id}:%"),
        )
    }
    concept_id = targets.get((term_id, "concepts"))
    crystal_id = targets.get((term_id, "crystals"))
    source_id = targets.get((f"{term_id}:source", "concept_facets"))
    rendering_id = targets.get((f"{term_id}:rendering", "concept_facets"))
    if None in {concept_id, crystal_id, source_id, rendering_id}:
        return False
    crystal = conn.execute(
        "select text, status from crystals where id = ?", (crystal_id,)
    ).fetchone()
    if (
        crystal is None
        or crystal["status"] != "active"
        or parse_rule_crystal(crystal["text"]) is None
    ):
        return False
    facets = {
        int(row["id"]): (row["facet_type"], row["value"], row["language"])
        for row in conn.execute(
            "select id, facet_type, value, language from concept_facets where concept_id = ?",
            (concept_id,),
        )
    }
    source_facet = facets.get(source_id)
    rendering_facet = facets.get(rendering_id)
    if source_facet is None or source_facet[:2] != ("name", term["source_text"]):
        return False
    if rendering_facet is None or rendering_facet[:2] != (
        "rendering",
        term["canonical_translation"],
    ):
        return False
    for alias in aliases:
        alias_source_id = f"{term_id}:alias:{alias['id']}"
        if alias["kind"] in {"source_variant", "search_alias", "approved_variant"}:
            alias_target = targets.get((alias_source_id, "concept_facets"))
            expected_type = "rendering" if alias["kind"] == "approved_variant" else "alias"
            if alias_target is None or facets.get(alias_target) != (
                expected_type,
                alias["text"].strip(),
                alias["language"],
            ):
                return False
        if (
            alias["kind"] == "forbidden_variant"
            and targets.get((alias_source_id, "crystals")) != crystal_id
        ):
            return False
    parsed = parse_rule_crystal(crystal["text"])
    forbidden = tuple(
        alias["text"].strip()
        for alias in aliases
        if alias["kind"] == "forbidden_variant" and alias["text"].strip()
    )
    if parsed is None or tuple(parsed.forbidden_variants) != forbidden:
        return False
    concept_tags = {
        row["tag"]
        for row in conn.execute(
            "select tag from concept_semantic_tags where concept_id = ?", (concept_id,)
        )
    }
    crystal_tags = {
        row["tag"]
        for row in conn.execute(
            "select tag from crystal_semantic_tags where crystal_id = ?", (crystal_id,)
        )
    }
    linked = conn.execute(
        """
        select 1 from crystal_concepts
        where crystal_id = ? and concept_id = ? and link_type = 'defines'
        """,
        (crystal_id, concept_id),
    ).fetchone()
    return tags <= concept_tags and tags <= crystal_tags and linked is not None


def drop_legacy_tables(conn: sqlite3.Connection) -> None:
    raise LegacyTermRetirementDisabled(
        "strict term table dropping remains disabled until Task 3 removes runtime access"
    )
