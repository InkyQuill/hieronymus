from __future__ import annotations

import errno
import hashlib
import json
import os
import re
import sqlite3
import stat
from collections import Counter
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any
from uuid import uuid4

from hieronymus.memory_migration import (
    MemoryGraphMigrator,
    _columns,
    _generated_graph_schema_is_complete,
    _generated_rule_artifacts_exist,
    _has_columns,
    _has_table,
    _has_unique_columns,
    _now,
    _row_exists,
    _rule_text,
    _scalar_count,
    _validate_rule_shape,
)
from hieronymus.rule_crystals import parse_rule_crystal

BACKUP_FORMAT = "hieronymus.strict-terms-backup"
BACKUP_VERSION = 1
ACTIVE_STATUSES = frozenset({"approved", "active"})
LEGACY_TABLES = ("strict_terms", "strict_term_tags", "strict_term_aliases")
_TIMESTAMP_RE = re.compile(r"^[0-9]{8}T[0-9]{6}Z$")
_BACKUP_FILENAME_RE = re.compile(
    rf"^strict-terms-v{BACKUP_VERSION}-(?P<timestamp>[0-9]{{8}}T[0-9]{{6}}Z)-"
    r"(?P<backup_id>[0-9a-f]{32})\.json$"
)
_UNSUPPORTED_DIRECTORY_FSYNC_ERRNOS = {
    errno.EINVAL,
    errno.ENOTSUP,
    getattr(errno, "EOPNOTSUPP", errno.ENOTSUP),
}


class StrictTermConversionBlocked(ValueError):
    """Raised when a legacy strict term cannot be represented without loss."""


@dataclass(frozen=True)
class StrictTermConversionReport:
    active_terms: int
    migrated_terms: int
    created: Mapping[str, int] = field(default_factory=dict)
    blocked_term_ids: tuple[int, ...] = ()

    @property
    def complete(self) -> bool:
        return self.migrated_terms == self.active_terms and not self.blocked_term_ids


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
    if _TIMESTAMP_RE.fullmatch(stamp) is None:
        raise ValueError("timestamp must use the safe UTC form YYYYMMDDTHHMMSSZ")
    requested_dir = backup_dir.absolute()
    resolved_dir = backup_dir.resolve(strict=False)
    if requested_dir != resolved_dir:
        raise LegacyTermRetirementBlocked("backup directory must not contain symlinks")
    resolved_dir.mkdir(parents=True, exist_ok=True)
    if resolved_dir.is_symlink() or resolved_dir.resolve(strict=True) != resolved_dir:
        raise LegacyTermRetirementBlocked("backup directory changed to an unsafe symlink")
    unique_id = uuid4().hex
    payload: dict[str, object] = {
        "format": BACKUP_FORMAT,
        "version": BACKUP_VERSION,
        "timestamp": stamp,
        "backup_id": unique_id,
        "created_at": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
        "strict_terms": _rows(conn, "strict_terms"),
        "strict_term_tags": _rows(conn, "strict_term_tags"),
        "strict_term_aliases": _rows(conn, "strict_term_aliases"),
    }
    payload["checksum"] = _checksum_payload(payload)
    destination = resolved_dir / f"strict-terms-v{BACKUP_VERSION}-{stamp}-{unique_id}.json"
    temporary_name = f".{destination.name}.{uuid4().hex}.tmp"
    if destination.parent != resolved_dir or Path(temporary_name).name != temporary_name:
        raise LegacyTermRetirementBlocked("backup path escaped the resolved backup directory")
    directory_flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0)
    directory_fd = os.open(resolved_dir, directory_flags)
    temporary_created = False
    try:
        pinned = os.fstat(directory_fd)
        _assert_directory_still_pinned(resolved_dir, pinned)
        file_flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
        temporary_fd = os.open(temporary_name, file_flags, 0o600, dir_fd=directory_fd)
        temporary_created = True
        with os.fdopen(temporary_fd, "w", encoding="utf-8") as stream:
            json.dump(payload, stream, ensure_ascii=False, sort_keys=True, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(
            temporary_name,
            destination.name,
            src_dir_fd=directory_fd,
            dst_dir_fd=directory_fd,
        )
        temporary_created = False
        _fsync_directory_descriptor(directory_fd)
        _assert_directory_still_pinned(resolved_dir, pinned)
        verified = _read_and_verify_backup_at(directory_fd, destination.name)
    finally:
        if temporary_created:
            try:
                os.unlink(temporary_name, dir_fd=directory_fd)
            except FileNotFoundError:
                pass
        os.close(directory_fd)

    return VerifiedLegacyTermsBackup(
        path=destination,
        checksum=str(verified["checksum"]),
        term_count=len(verified["strict_terms"]),
    )


def _fsync_directory(directory: Path) -> None:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
    try:
        descriptor = os.open(directory, flags)
    except OSError as exc:
        if exc.errno in _UNSUPPORTED_DIRECTORY_FSYNC_ERRNOS:
            return
        raise
    try:
        os.fsync(descriptor)
    except OSError as exc:
        if exc.errno not in _UNSUPPORTED_DIRECTORY_FSYNC_ERRNOS:
            raise
    finally:
        os.close(descriptor)


def _fsync_directory_descriptor(descriptor: int) -> None:
    try:
        os.fsync(descriptor)
    except OSError as exc:
        if exc.errno not in _UNSUPPORTED_DIRECTORY_FSYNC_ERRNOS:
            raise


def _assert_directory_still_pinned(directory: Path, pinned: os.stat_result) -> None:
    try:
        current = os.stat(directory, follow_symlinks=False)
    except OSError as exc:
        raise LegacyTermRetirementBlocked("backup directory changed during creation") from exc
    if (
        not stat.S_ISDIR(current.st_mode)
        or current.st_dev != pinned.st_dev
        or current.st_ino != pinned.st_ino
    ):
        raise LegacyTermRetirementBlocked("backup directory changed or became a symlink")


def _read_and_verify_backup_at(directory_fd: int, filename: str) -> dict[str, Any]:
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(filename, flags, dir_fd=directory_fd)
        with os.fdopen(descriptor, encoding="utf-8") as stream:
            payload = json.load(stream)
    except (OSError, json.JSONDecodeError) as exc:
        raise LegacyTermRetirementBlocked(f"invalid legacy terms backup: {filename}") from exc
    return _verify_backup_payload(payload, filename)


def verify_legacy_terms_backup(path: Path) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise LegacyTermRetirementBlocked(f"invalid legacy terms backup: {path}") from exc
    return _verify_backup_payload(payload, path.name)


def _verify_backup_payload(payload: object, filename: str) -> dict[str, Any]:
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
    match = _BACKUP_FILENAME_RE.fullmatch(filename)
    if (
        match is None
        or payload.get("timestamp") != match.group("timestamp")
        or payload.get("backup_id") != match.group("backup_id")
    ):
        raise LegacyTermRetirementBlocked("legacy terms backup filename identity mismatch")
    return payload


def prepare_strict_term_retirement(
    conn: sqlite3.Connection,
    backup_dir: Path,
    *,
    timestamp: str | None = None,
    failure_hook: Callable[[int], None] | None = None,
    transaction_owned_by_caller: bool = False,
    finalizer: Callable[[sqlite3.Connection], None] | None = None,
) -> StrictTermRetirementResult:
    """Back up, convert, audit, and verify legacy terms in one owned transaction."""
    if conn.in_transaction and not transaction_owned_by_caller:
        raise sqlite3.ProgrammingError(
            "strict term retirement requires explicit transaction ownership; "
            "the supplied connection already has an active transaction"
        )

    if transaction_owned_by_caller:
        if not conn.in_transaction:
            raise sqlite3.ProgrammingError("caller-owned retirement requires an active transaction")
    else:
        conn.execute("begin immediate")
    try:
        backup = write_legacy_terms_backup(conn, backup_dir, timestamp=timestamp)
        payload = verify_legacy_terms_backup(backup.path)
        _validate_inactive_audit_shape(conn, payload)
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
            blocked = ", ".join(str(term_id) for term_id in coverage.blocked_term_ids)
            raise LegacyTermRetirementBlocked(
                f"strict term retirement parity failed for terms {blocked}"
            )
        if finalizer is not None:
            finalizer(conn)
        if not transaction_owned_by_caller:
            conn.commit()
    except BaseException:
        if not transaction_owned_by_caller:
            conn.rollback()
        raise
    return StrictTermRetirementResult(backup=backup, coverage=coverage)


def _validate_inactive_audit_shape(conn: sqlite3.Connection, payload: Mapping[str, object]) -> None:
    inactive_ids = [
        int(row["id"])
        for row in payload["strict_terms"]
        if isinstance(row, dict) and row.get("status") not in ACTIVE_STATUSES
    ]
    if not inactive_ids:
        return
    columns = {row["name"] for row in conn.execute("pragma table_info(audit_log)")}
    required = {
        "id",
        "actor",
        "action",
        "entity_type",
        "entity_id",
        "note",
        "before_json",
        "after_json",
        "created_at",
    }
    if not required <= columns:
        affected = ", ".join(str(term_id) for term_id in inactive_ids)
        noun = "term" if len(inactive_ids) == 1 else "terms"
        raise LegacyTermRetirementBlocked(f"{noun} {affected}: inactive audit schema is incomplete")


def _audit_inactive_terms(conn: sqlite3.Connection, payload: Mapping[str, object]) -> int:
    expected = _expected_inactive_audit_json(payload)
    for entity_id, before_json in expected.items():
        existing = conn.execute(
            """
            select id, before_json from audit_log
            where action = 'strict_term_retirement'
              and entity_type = 'strict_term'
              and entity_id = ?
            order by id
            """,
            (entity_id,),
        ).fetchall()
        if any(row["before_json"] == before_json for row in existing):
            continue
        conn.execute(
            """
            insert into audit_log(
              actor, action, entity_type, entity_id, note, before_json, after_json, created_at
            ) values ('migration', 'strict_term_retirement', 'strict_term', ?,
                      'Preserved inactive legacy term before retirement.', ?, '{}', datetime('now'))
            """,
            (entity_id, before_json),
        )
    return _count_matching_inactive_audits(conn, expected)


def _expected_inactive_audit_json(payload: Mapping[str, object]) -> dict[str, str]:
    inactive = [
        row
        for row in payload["strict_terms"]
        if isinstance(row, dict) and row.get("status") not in ACTIVE_STATUSES
    ]
    tag_rows = [row for row in payload["strict_term_tags"] if isinstance(row, dict)]
    alias_rows = [row for row in payload["strict_term_aliases"] if isinstance(row, dict)]
    return {
        str(row["id"]): json.dumps(
            {
                "term": row,
                "tags": [tag for tag in tag_rows if tag.get("term_id") == row["id"]],
                "aliases": [alias for alias in alias_rows if alias.get("term_id") == row["id"]],
            },
            ensure_ascii=False,
            sort_keys=True,
        )
        for row in inactive
    }


def _count_matching_inactive_audits(conn: sqlite3.Connection, expected: Mapping[str, str]) -> int:
    return sum(
        conn.execute(
            """
            select count(*) from audit_log
            where action = 'strict_term_retirement'
              and entity_type = 'strict_term'
              and entity_id = ?
              and before_json = ?
            """,
            (entity_id, before_json),
        ).fetchone()[0]
        >= 1
        for entity_id, before_json in expected.items()
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
    inactive_audited = _count_matching_inactive_audits(conn, _expected_inactive_audit_json(payload))

    active_ids = tuple(int(term["id"]) for term in active)
    try:
        _validate_retirement_ownership_integrity(
            conn,
            active_ids,
            ", ".join(str(term_id) for term_id in active_ids) or "<none>",
        )
    except (StrictTermConversionBlocked, sqlite3.DatabaseError):
        return StrictTermRetirementCoverage(
            total_terms=len(terms),
            active_terms=len(active),
            migrated_terms=migrated_terms,
            inactive_audited=inactive_audited,
            blocked_term_ids=active_ids,
        )

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
    for source_key, object_type, object_id in (
        (term_id, "concepts", concept_id),
        (term_id, "crystals", crystal_id),
        (f"{term_id}:source", "concept_facets", source_id),
        (f"{term_id}:rendering", "concept_facets", rendering_id),
    ):
        if not _is_owned_target(conn, source_key, object_type, int(object_id)):
            return False
    concept = conn.execute(
        """
        select canonical_name, description, scope_type, scope_key, status, confidence
        from concepts where id = ?
        """,
        (concept_id,),
    ).fetchone()
    if (
        concept is None
        or concept["canonical_name"] != term["source_text"]
        or concept["description"] != term["notes"]
        or concept["scope_type"] != "series"
        or concept["scope_key"] != f"series:{term['series_slug']}"
        or concept["status"] != "established"
        or float(concept["confidence"]) < 0.95
    ):
        return False
    crystal = conn.execute(
        """
        select text, status, strength, confidence, source_credibility,
               source_language, target_language, crystal_type, title,
               scope_type, scope_key, series_slug
        from crystals where id = ?
        """,
        (crystal_id,),
    ).fetchone()
    parsed = None if crystal is None else parse_rule_crystal(crystal["text"])
    if (
        crystal is None
        or crystal["status"] != "active"
        or parsed is None
        or parsed.source_text != term["source_text"]
        or parsed.canonical_translation != term["canonical_translation"]
        or float(crystal["strength"]) < 0.8
        or float(crystal["confidence"]) < 0.95
        or crystal["source_credibility"] != "user_rule"
        or crystal["source_language"] != term["source_language"]
        or crystal["target_language"] != term["target_language"]
        or crystal["crystal_type"] != "rule"
        or crystal["title"] != ""
        or crystal["scope_type"] != "series"
        or crystal["scope_key"] != f"series:{term['series_slug']}"
        or crystal["series_slug"] != term["series_slug"]
    ):
        return False
    facets = {
        int(row["id"]): (
            row["facet_type"],
            row["value"],
            row["language"],
            bool(row["is_canonical"]),
            float(row["confidence"]),
        )
        for row in conn.execute(
            """
            select id, facet_type, value, language, is_canonical, confidence
            from concept_facets where concept_id = ?
            """,
            (concept_id,),
        )
    }
    source_facet = facets.get(source_id)
    rendering_facet = facets.get(rendering_id)
    if source_facet is None or source_facet[:4] != (
        "name",
        term["source_text"],
        term["source_language"],
        True,
    ):
        return False
    if source_facet[4] < 0.95:
        return False
    if rendering_facet is None or rendering_facet[:4] != (
        "rendering",
        term["canonical_translation"],
        term["target_language"],
        False,
    ):
        return False
    if rendering_facet[4] < 0.95:
        return False
    for facet_id, expected_language in (
        (source_id, term["source_language"]),
        (rendering_id, term["target_language"]),
    ):
        language_tags = {
            row["language_tag"]
            for row in conn.execute(
                "select language_tag from concept_facet_language_tags where facet_id = ?",
                (facet_id,),
            )
        }
        if str(expected_language).casefold() not in language_tags:
            return False
    expected_alias_targets = {
        (
            f"{term_id}:alias:{alias['id']}",
            "crystals" if alias["kind"] == "forbidden_variant" else "concept_facets",
        )
        for alias in aliases
    }
    actual_alias_targets = {
        (source_id, target_table)
        for source_id, target_table in targets
        if source_id.startswith(f"{term_id}:alias:")
    }
    if actual_alias_targets != expected_alias_targets:
        return False
    for alias in aliases:
        alias_source_id = f"{term_id}:alias:{alias['id']}"
        if alias["kind"] in {"source_variant", "search_alias", "approved_variant"}:
            alias_target = targets.get((alias_source_id, "concept_facets"))
            expected_type = "rendering" if alias["kind"] == "approved_variant" else "alias"
            expected_canonical = False
            alias_facet = facets.get(alias_target) if alias_target is not None else None
            if alias_facet is None or alias_facet[:4] != (
                expected_type,
                alias["text"].strip(),
                alias["language"],
                expected_canonical,
            ):
                return False
            if alias_facet[4] < 0.95:
                return False
            if not _is_owned_target(conn, alias_source_id, "concept_facets", int(alias_target)):
                return False
            alias_language_tags = {
                row["language_tag"]
                for row in conn.execute(
                    "select language_tag from concept_facet_language_tags where facet_id = ?",
                    (alias_target,),
                )
            }
            if str(alias["language"]).casefold() not in alias_language_tags:
                return False
        if (
            alias["kind"] == "forbidden_variant"
            and targets.get((alias_source_id, "crystals")) != crystal_id
        ):
            return False
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
        select confidence from crystal_concepts
        where crystal_id = ? and concept_id = ? and link_type = 'defines'
        """,
        (crystal_id, concept_id),
    ).fetchone()
    expected_languages = {
        str(term["source_language"]).casefold(),
        str(term["target_language"]).casefold(),
    }
    crystal_languages = {
        row["language_tag"]
        for row in conn.execute(
            "select language_tag from crystal_language_tags where crystal_id = ?",
            (crystal_id,),
        )
    }
    return (
        tags <= concept_tags
        and tags <= crystal_tags
        and expected_languages <= crystal_languages
        and linked is not None
        and float(linked["confidence"]) >= 0.95
    )


def _is_owned_target(
    conn: sqlite3.Connection, source_id: str, object_type: str, object_id: int
) -> bool:
    if not conn.execute(
        "select 1 from sqlite_master where type = 'table' "
        "and name = 'strict_term_retirement_ownership'"
    ).fetchone():
        return False
    return (
        conn.execute(
            """
            select 1 from strict_term_retirement_ownership
            where source_table = 'strict_terms' and source_id = ?
              and object_type = ? and object_id = ?
            """,
            (source_id, object_type, object_id),
        ).fetchone()
        is not None
    )


def drop_legacy_tables(conn: sqlite3.Connection) -> None:
    """Drop verified legacy storage in foreign-key-safe child/FTS/parent order."""
    conn.execute("drop table if exists strict_terms_fts")
    conn.execute("drop table strict_term_aliases")
    conn.execute("drop table strict_term_tags")
    conn.execute("drop table strict_terms")


class _LegacyTermProjection(MemoryGraphMigrator):
    def _migrate_strict_terms(
        self,
        conn: sqlite3.Connection,
        created: Counter[str],
        skipped: Counter[str],
    ) -> None:
        if not (
            _has_table(conn, "strict_terms")
            and _has_columns(
                conn,
                "strict_terms",
                {
                    "id",
                    "status",
                    "source_text",
                    "canonical_translation",
                    "notes",
                    "series_slug",
                    "source_language",
                    "target_language",
                },
            )
        ):
            return
        if not _generated_graph_schema_is_complete(conn):
            if _scalar_count(
                conn,
                "select count(*) from strict_terms where status in ('approved', 'active')",
            ):
                skipped["generated_graph.incomplete_schema"] += 1
            return
        try:
            report = convert_strict_terms(conn, blocking=False, require_retirement_schema=False)
        except StrictTermConversionBlocked:
            return
        created.update(report.created)
        if report.blocked_term_ids:
            skipped["strict_terms.unsupported_alias"] += len(report.blocked_term_ids)

    def _convert_strict_terms_unchecked(
        self,
        conn: sqlite3.Connection,
        created: Counter[str],
        skipped: Counter[str],
        *,
        failure_hook: Callable[[int], None] | None = None,
    ) -> None:
        if not (
            _has_table(conn, "strict_terms")
            and _has_columns(
                conn,
                "strict_terms",
                {
                    "id",
                    "status",
                    "source_text",
                    "canonical_translation",
                    "notes",
                    "series_slug",
                    "source_language",
                    "target_language",
                },
            )
        ):
            return
        if not _generated_graph_schema_is_complete(conn):
            if _scalar_count(
                conn,
                "select count(*) from strict_terms where status in ('approved', 'active')",
            ):
                skipped["generated_graph.incomplete_schema"] += 1
            return
        rows = conn.execute(
            """
            select *
            from strict_terms
            where status in ('approved', 'active')
            order by id
            """
        ).fetchall()
        for term in rows:
            tags = self._strict_term_tags(conn, int(term["id"]))
            aliases = self._strict_term_rule_aliases(conn, int(term["id"]))
            if aliases is None:
                skipped["strict_terms.unsupported_alias"] += 1
                continue
            approved, forbidden = aliases
            if not _validate_rule_shape(
                source_text=term["source_text"],
                canonical_translation=term["canonical_translation"],
                approved_variants=approved,
                forbidden_variants=forbidden,
            ):
                skipped["strict_terms.unsupported_rule_shape"] += 1
                continue

            concept_id = self._ensure_concept(
                conn,
                source_table="strict_terms",
                source_id=str(term["id"]),
                canonical_name=term["source_text"],
                description=term["notes"],
                scope_key=f"series:{term['series_slug']}",
                status="established",
                confidence=0.95,
                semantic_tags=tags,
                created=created,
                exact_projection=True,
            )
            self._ensure_facet(
                conn,
                source_table="strict_terms",
                source_id=f"{term['id']}:source",
                concept_id=concept_id,
                value=term["source_text"],
                facet_type="name",
                language=term["source_language"],
                confidence=0.95,
                is_canonical=True,
                created=created,
                exact_projection=True,
            )
            for alias_id, language, text, kind in self._strict_term_alias_rows(
                conn, int(term["id"])
            ):
                if kind not in {"source_variant", "search_alias", "approved_variant"}:
                    continue
                alias_facet_id = self._ensure_facet(
                    conn,
                    source_table="strict_terms",
                    source_id=f"{term['id']}:alias:{alias_id}",
                    concept_id=concept_id,
                    value=text,
                    facet_type="rendering" if kind == "approved_variant" else "alias",
                    language=language,
                    confidence=0.95,
                    is_canonical=False,
                    created=created,
                    exact_projection=True,
                )
                _record_alias_provenance(
                    conn,
                    term_id=int(term["id"]),
                    alias_id=alias_id,
                    target_table="concept_facets",
                    target_id=alias_facet_id,
                    alias_kind=kind,
                )
            self._ensure_facet(
                conn,
                source_table="strict_terms",
                source_id=f"{term['id']}:rendering",
                concept_id=concept_id,
                value=term["canonical_translation"],
                facet_type="rendering",
                language=term["target_language"],
                confidence=0.95,
                is_canonical=False,
                created=created,
                exact_projection=True,
            )
            crystal_id = self._ensure_rule_crystal(
                conn,
                source_table="strict_terms",
                source_id=str(term["id"]),
                title="",
                text=_rule_text(term["source_text"], term["canonical_translation"], forbidden),
                series_slug=term["series_slug"],
                source_language=term["source_language"],
                target_language=term["target_language"],
                status="active",
                strength=0.8,
                confidence=0.95,
                semantic_tags=tags,
                created=created,
                exact_projection=True,
            )
            self._ensure_owned_crystal_concept_link(
                conn,
                source_table="strict_terms",
                source_id=str(term["id"]),
                crystal_id=crystal_id,
                concept_id=concept_id,
                confidence=0.95,
            )
            for alias_id, _, _, kind in self._strict_term_alias_rows(conn, int(term["id"])):
                if kind == "forbidden_variant":
                    alias_source_id = f"{term['id']}:alias:{alias_id}"
                    self._record_ledger(
                        conn,
                        "strict_terms",
                        alias_source_id,
                        "crystals",
                        crystal_id,
                    )
                    _record_alias_provenance(
                        conn,
                        term_id=int(term["id"]),
                        alias_id=alias_id,
                        target_table="crystals",
                        target_id=crystal_id,
                        alias_kind=kind,
                    )
            self._remove_stale_strict_term_alias_artifacts(
                conn,
                int(term["id"]),
                {
                    alias_id: ("crystals" if kind == "forbidden_variant" else "concept_facets")
                    for alias_id, _, _, kind in self._strict_term_alias_rows(conn, int(term["id"]))
                },
            )
            if failure_hook is not None:
                failure_hook(int(term["id"]))

    def _strict_term_tags(self, conn: sqlite3.Connection, term_id: int) -> tuple[str, ...]:
        if not _has_table(conn, "strict_term_tags"):
            return ()
        if not _has_columns(conn, "strict_term_tags", {"term_id", "tag"}):
            return ()
        return tuple(
            row["tag"]
            for row in conn.execute(
                """
                select tag
                from strict_term_tags
                where term_id = ?
                order by tag
                """,
                (term_id,),
            )
        )

    def _strict_term_rule_aliases(
        self,
        conn: sqlite3.Connection,
        term_id: int,
    ) -> tuple[tuple[str, ...], tuple[str, ...]] | None:
        if not _has_table(conn, "strict_term_aliases"):
            return ((), ())
        columns = _columns(conn, "strict_term_aliases")
        if not {"term_id", "text", "kind"} <= columns:
            return None
        case_sensitive_sql = "case_sensitive" if "case_sensitive" in columns else "1"
        order_column = "id" if "id" in columns else "rowid"
        rows = conn.execute(
            f"""
            select text, kind, {case_sensitive_sql} as case_sensitive
            from strict_term_aliases
            where term_id = ?
            order by {order_column}
            """,
            (term_id,),
        ).fetchall()
        for row in rows:
            if row["kind"] not in {
                "source_variant",
                "search_alias",
                "approved_variant",
                "forbidden_variant",
            }:
                return None
            if not bool(row["case_sensitive"]):
                return None
        approved = tuple(
            row["text"].strip()
            for row in rows
            if row["kind"] == "approved_variant" and row["text"].strip()
        )
        forbidden = tuple(
            row["text"].strip()
            for row in rows
            if row["kind"] == "forbidden_variant" and row["text"].strip()
        )
        return (approved, forbidden)

    def _strict_term_alias_rows(
        self,
        conn: sqlite3.Connection,
        term_id: int,
    ) -> tuple[tuple[int, str, str, str], ...]:
        if not _has_table(conn, "strict_term_aliases"):
            return ()
        columns = _columns(conn, "strict_term_aliases")
        if not {"term_id", "language", "text", "kind"} <= columns:
            return ()
        id_column = "id" if "id" in columns else "rowid"
        return tuple(
            (
                int(row["alias_id"]),
                str(row["language"]),
                str(row["text"]).strip(),
                str(row["kind"]),
            )
            for row in conn.execute(
                f"""
                select {id_column} as alias_id, language, text, kind
                from strict_term_aliases
                where term_id = ?
                  and trim(text) != ''
                order by {id_column}
                """,
                (term_id,),
            )
        )

    def _remove_stale_strict_term_alias_artifacts(
        self,
        conn: sqlite3.Connection,
        term_id: int,
        current_alias_targets: Mapping[int, str],
    ) -> None:
        prefix = f"{term_id}:alias:"
        rows = conn.execute(
            """
            select source_id, target_table, target_id
            from memory_graph_migration_ledger
            where source_table = 'strict_terms' and source_id like ?
            order by source_id, target_table
            """,
            (f"{prefix}%",),
        ).fetchall()
        for row in rows:
            suffix = str(row["source_id"])[len(prefix) :]
            if suffix.isdigit() and current_alias_targets.get(int(suffix)) == row["target_table"]:
                continue
            conn.execute(
                """
                delete from memory_graph_migration_ledger
                where source_table = 'strict_terms' and source_id = ? and target_table = ?
                """,
                (row["source_id"], row["target_table"]),
            )
            conn.execute(
                """
                delete from strict_term_retirement_alias_provenance
                where source_table = 'strict_terms' and source_id = ?
                  and target_table = ? and target_id = ?
                """,
                (row["source_id"], row["target_table"], row["target_id"]),
            )
            if row["target_table"] != "concept_facets":
                continue
            owned_id = _owned_object_id(
                conn, "strict_terms", str(row["source_id"]), "concept_facets"
            )
            if owned_id == int(row["target_id"]):
                stale_source_id = str(row["source_id"])
                if _facet_is_exclusively_owned(conn, stale_source_id, owned_id):
                    _delete_owned_object(
                        conn, "strict_terms", stale_source_id, "concept_facets", owned_id
                    )
                else:
                    _release_owned_object(
                        conn, "strict_terms", stale_source_id, "concept_facets", owned_id
                    )

    def _count_migratable_strict_terms(self, conn: sqlite3.Connection) -> int:
        rows = conn.execute(
            """
            select *
            from strict_terms
            where status in ('approved', 'active')
            order by id
            """
        ).fetchall()

        count = 0
        for term in rows:
            if _generated_rule_artifacts_exist(conn, "strict_terms", str(term["id"])):
                continue
            aliases = self._strict_term_rule_aliases(conn, int(term["id"]))
            if aliases is None:
                continue
            approved, forbidden = aliases
            if _validate_rule_shape(
                source_text=term["source_text"],
                canonical_translation=term["canonical_translation"],
                approved_variants=approved,
                forbidden_variants=forbidden,
            ):
                count += 1
        return count


_RETIREMENT_ALIAS_PROVENANCE_COLUMNS = {
    "source_table",
    "source_id",
    "term_id",
    "alias_id",
    "target_table",
    "target_id",
    "alias_kind",
    "created_at",
}


_RETIREMENT_RELATIONSHIP_OWNERSHIP_COLUMNS = {
    "source_table",
    "source_id",
    "relationship_type",
    "owner_type",
    "owner_id",
    "related_type",
    "related_id",
    "value",
    "created_at",
}


_RETIREMENT_OWNERSHIP_COLUMNS = {
    "source_table",
    "source_id",
    "object_type",
    "object_id",
    "created_at",
}


def convert_strict_terms(
    conn: sqlite3.Connection,
    *,
    blocking: bool = True,
    require_retirement_schema: bool = True,
    failure_hook: Callable[[int], None] | None = None,
) -> StrictTermConversionReport:
    """Convert approved legacy terms using the caller's connection and transaction.

    The function never commits or rolls back. Callers own transaction boundaries; the
    retirement orchestrator uses this to keep backup verification outside DB mutation.
    """
    if not _has_table(conn, "strict_terms"):
        return StrictTermConversionReport(active_terms=0, migrated_terms=0)

    active_ids = _active_strict_term_ids(conn)
    affected = ", ".join(str(term_id) for term_id in active_ids) or "<none>"

    required_term_columns = {
        "id",
        "status",
        "source_text",
        "canonical_translation",
        "notes",
        "series_slug",
        "source_language",
        "target_language",
    }
    if not _has_columns(conn, "strict_terms", required_term_columns):
        raise StrictTermConversionBlocked(
            f"terms {affected}: strict_terms has an incomplete legacy shape"
        )
    if not _generated_graph_schema_is_complete(conn):
        raise StrictTermConversionBlocked(
            f"terms {affected}: generated rule graph or ledger shape is incomplete"
        )
    if require_retirement_schema:
        _validate_strict_relationship_schema(conn, active_ids, affected)
        _validate_ledger_uniqueness(conn, affected)
    _validate_retirement_ownership_schema(conn, affected)
    if require_retirement_schema:
        _discard_repairable_stale_retirement_ownership(conn)
        _validate_retirement_ownership_integrity(conn, active_ids, affected)

    active_rows = conn.execute(
        "select * from strict_terms where status in ('approved', 'active') order by id"
    ).fetchall()
    blocked: list[tuple[int, str]] = []
    writer = object.__new__(_LegacyTermProjection)
    for term in active_rows:
        term_id = int(term["id"])
        for column in (
            "source_text",
            "canonical_translation",
            "series_slug",
            "source_language",
            "target_language",
        ):
            if not str(term[column]).strip():
                blocked.append((term_id, f"{column} is empty"))
                break
        else:
            incomplete_alias = _strict_term_incomplete_alias(conn, term_id)
            if incomplete_alias is not None:
                blocked.append((term_id, incomplete_alias))
                continue
            aliases = writer._strict_term_rule_aliases(conn, term_id)
            if aliases is None:
                blocked.append((term_id, "aliases must use known kinds and be case-sensitive"))
                continue
            approved, forbidden = aliases
            if not _validate_rule_shape(
                source_text=term["source_text"],
                canonical_translation=term["canonical_translation"],
                approved_variants=approved,
                forbidden_variants=forbidden,
            ):
                blocked.append((term_id, "alias or rule shape cannot be represented losslessly"))

    if blocked and blocking:
        details = "; ".join(f"term {term_id}: {reason}" for term_id, reason in blocked)
        raise StrictTermConversionBlocked(details)

    _ensure_retirement_ownership_schema(conn)
    created: Counter[str] = Counter()
    skipped: Counter[str] = Counter()
    writer._convert_strict_terms_unchecked(
        conn,
        created,
        skipped,
        failure_hook=failure_hook,
    )
    migrated = sum(
        _generated_rule_artifacts_exist(conn, "strict_terms", str(int(term["id"])))
        for term in active_rows
    )
    blocked_ids = tuple(term_id for term_id, _ in blocked)
    return StrictTermConversionReport(
        active_terms=len(active_rows),
        migrated_terms=migrated,
        created=dict(created),
        blocked_term_ids=blocked_ids,
    )


def _strict_term_incomplete_alias(conn: sqlite3.Connection, term_id: int) -> str | None:
    if not _has_table(conn, "strict_term_aliases"):
        return None
    required = {"term_id", "text", "language", "kind"}
    if not _has_columns(conn, "strict_term_aliases", required):
        return "alias relationship shape is incomplete"
    for row in conn.execute(
        "select text, language from strict_term_aliases where term_id = ? order by rowid",
        (term_id,),
    ):
        if not str(row["text"]).strip():
            return "alias text is empty"
        if not str(row["language"]).strip():
            return "alias language is empty"
    return None


def _active_strict_term_ids(conn: sqlite3.Connection) -> tuple[int, ...]:
    columns = _columns(conn, "strict_terms")
    if "id" not in columns:
        return ()
    where = "where status in ('approved', 'active')" if "status" in columns else ""
    return tuple(
        int(row["id"]) for row in conn.execute(f"select id from strict_terms {where} order by id")
    )


def _validate_strict_relationship_schema(
    conn: sqlite3.Connection, active_ids: tuple[int, ...], affected: str
) -> None:
    del active_ids
    required = {
        "strict_term_tags": {"term_id", "tag"},
        "strict_term_aliases": {"id", "term_id", "language", "text", "kind"},
    }
    for table, columns in required.items():
        if not _has_table(conn, table) or not _has_columns(conn, table, columns):
            raise StrictTermConversionBlocked(
                f"terms {affected}: {table} relationship shape is incomplete"
            )
    if not _has_unique_columns(conn, "strict_term_aliases", ("id",)):
        raise StrictTermConversionBlocked(
            f"terms {affected}: strict_term_aliases requires stable unique alias IDs"
        )
    alias_id = next(
        row for row in conn.execute("pragma table_info(strict_term_aliases)") if row["name"] == "id"
    )
    if not int(alias_id["pk"]) and not int(alias_id["notnull"]):
        raise StrictTermConversionBlocked(
            f"terms {affected}: strict_term_aliases requires non-null stable alias IDs"
        )


def _validate_ledger_uniqueness(conn: sqlite3.Connection, affected: str) -> None:
    if not _has_unique_columns(
        conn,
        "memory_graph_migration_ledger",
        ("source_table", "source_id", "target_table"),
    ):
        raise StrictTermConversionBlocked(
            f"terms {affected}: memory graph ledger requires a unique source relationship"
        )


def _validate_retirement_ownership_schema(conn: sqlite3.Connection, affected: str) -> None:
    expected = {
        "strict_term_retirement_ownership": _RETIREMENT_OWNERSHIP_COLUMNS,
        "strict_term_retirement_relationship_ownership": (
            _RETIREMENT_RELATIONSHIP_OWNERSHIP_COLUMNS
        ),
        "strict_term_retirement_alias_provenance": _RETIREMENT_ALIAS_PROVENANCE_COLUMNS,
    }
    for table, columns in expected.items():
        if _has_table(conn, table) and not _has_columns(conn, table, set(columns)):
            raise StrictTermConversionBlocked(
                f"terms {affected}: {table} provenance shape is incomplete"
            )
    if _has_table(conn, "strict_term_retirement_ownership") and (
        not _has_unique_columns(
            conn,
            "strict_term_retirement_ownership",
            ("source_table", "source_id", "object_type"),
        )
        or not _has_unique_columns(
            conn,
            "strict_term_retirement_ownership",
            ("object_type", "object_id"),
        )
    ):
        raise StrictTermConversionBlocked(
            f"terms {affected}: strict term retirement object ownership must be unique"
        )
    if _has_table(conn, "strict_term_retirement_relationship_ownership") and (
        not _has_unique_columns(
            conn,
            "strict_term_retirement_relationship_ownership",
            (
                "source_table",
                "source_id",
                "relationship_type",
                "owner_type",
                "owner_id",
                "related_type",
                "related_id",
                "value",
            ),
        )
        or not _has_unique_columns(
            conn,
            "strict_term_retirement_relationship_ownership",
            (
                "relationship_type",
                "owner_type",
                "owner_id",
                "related_type",
                "related_id",
                "value",
            ),
        )
    ):
        raise StrictTermConversionBlocked(
            f"terms {affected}: strict term retirement relationship ownership must be unique"
        )
    if _has_table(conn, "strict_term_retirement_alias_provenance") and (
        not _has_unique_columns(
            conn,
            "strict_term_retirement_alias_provenance",
            ("source_table", "source_id"),
        )
        or not _has_unique_columns(
            conn,
            "strict_term_retirement_alias_provenance",
            ("term_id", "alias_id"),
        )
    ):
        raise StrictTermConversionBlocked(
            f"terms {affected}: strict term retirement alias ownership must be unique"
        )


def _validate_retirement_ownership_integrity(
    conn: sqlite3.Connection, active_ids: tuple[int, ...], affected: str
) -> None:
    _validate_alias_ownership_integrity(conn, active_ids, affected)
    has_objects = _has_table(conn, "strict_term_retirement_ownership")
    has_relationships = _has_table(conn, "strict_term_retirement_relationship_ownership")
    if not has_objects and not has_relationships:
        return
    if not has_objects or not has_relationships:
        raise StrictTermConversionBlocked(
            f"terms {affected}: strict term retirement ownership integrity failed"
        )
    term_ids = {int(row["id"]) for row in conn.execute("select id from strict_terms")}
    objects: dict[tuple[str, str], int] = {}
    valid = True
    for row in conn.execute(
        """
        select source_table, source_id, object_type, object_id
        from strict_term_retirement_ownership
        order by source_table, source_id, object_type
        """
    ):
        source_id = str(row["source_id"])
        object_type = str(row["object_type"])
        object_id = _strict_positive_int(row["object_id"])
        term_id = _canonical_term_id_from_source(source_id)
        source_shape_valid = (
            source_id == str(term_id)
            if object_type in {"concepts", "crystals"}
            else _is_canonical_facet_source(source_id, term_id)
        )
        ledger = conn.execute(
            """
            select 1 from memory_graph_migration_ledger
            where source_table = ? and source_id = ? and target_table = ? and target_id = ?
            """,
            (row["source_table"], source_id, object_type, object_id),
        ).fetchone()
        row_valid = (
            row["source_table"] == "strict_terms"
            and term_id in term_ids
            and source_shape_valid
            and object_type in {"concepts", "concept_facets", "crystals"}
            and object_id is not None
            and _row_exists(conn, object_type, object_id)
            and ledger is not None
        )
        valid = valid and row_valid
        if row_valid:
            objects[(source_id, object_type)] = object_id

    for (source_id, object_type), object_id in objects.items():
        if object_type != "concept_facets":
            continue
        term_id = _canonical_term_id_from_source(source_id)
        concept_id = objects.get((str(term_id), "concepts"))
        facet = conn.execute(
            "select concept_id from concept_facets where id = ?", (object_id,)
        ).fetchone()
        valid = valid and facet is not None and int(facet["concept_id"]) == concept_id

    for row in conn.execute(
        """
        select source_table, source_id, relationship_type, owner_type, owner_id,
               related_type, related_id, value
        from strict_term_retirement_relationship_ownership
        order by source_table, source_id, relationship_type, owner_id, related_id, value
        """
    ):
        valid = valid and _relationship_ownership_row_is_valid(conn, row, objects, term_ids)

    if not valid:
        listed = ", ".join(str(term_id) for term_id in active_ids) or affected
        raise StrictTermConversionBlocked(
            f"terms {listed}: strict term retirement ownership integrity failed"
        )


def _discard_repairable_stale_retirement_ownership(conn: sqlite3.Connection) -> None:
    """Forget stale projection metadata without mutating any surviving graph row."""
    if not _has_table(conn, "strict_term_retirement_ownership"):
        return
    term_ids = {int(row["id"]) for row in conn.execute("select id from strict_terms")}
    rows = conn.execute(
        """
        select source_table, source_id, object_type, object_id
        from strict_term_retirement_ownership
        order by source_table, source_id, object_type
        """
    ).fetchall()
    objects = {
        (str(row["source_id"]), str(row["object_type"])): int(row["object_id"])
        for row in rows
        if _strict_positive_int(row["object_id"]) is not None
    }
    stale: list[tuple[str, str, str, int]] = []
    for row in rows:
        source_table = str(row["source_table"])
        source_id = str(row["source_id"])
        object_type = str(row["object_type"])
        object_id = _strict_positive_int(row["object_id"])
        term_id = _canonical_term_id_from_source(source_id)
        source_valid = (
            source_id == str(term_id)
            if object_type in {"concepts", "crystals"}
            else _is_canonical_facet_source(source_id, term_id)
        )
        ledger_exists = (
            object_id is not None
            and conn.execute(
                """
            select 1 from memory_graph_migration_ledger
            where source_table = ? and source_id = ? and target_table = ? and target_id = ?
            """,
                (source_table, source_id, object_type, object_id),
            ).fetchone()
        )
        if not (
            source_table == "strict_terms"
            and term_id in term_ids
            and source_valid
            and object_type in {"concepts", "concept_facets", "crystals"}
            and object_id is not None
            and ledger_exists is not None
        ):
            continue
        row_missing = not _row_exists(conn, object_type, object_id)
        wrong_concept = False
        if object_type == "concept_facets" and not row_missing:
            expected_concept = objects.get((str(term_id), "concepts"))
            facet = conn.execute(
                "select concept_id from concept_facets where id = ?", (object_id,)
            ).fetchone()
            wrong_concept = (
                expected_concept is not None
                and facet is not None
                and int(facet["concept_id"]) != expected_concept
            )
        if row_missing or wrong_concept:
            stale.append((source_table, source_id, object_type, object_id))

    for source_table, source_id, object_type, object_id in stale:
        conn.execute(
            """
            delete from strict_term_retirement_relationship_ownership
            where (owner_type = ? and owner_id = ?)
               or (related_type = ? and related_id = ?)
            """,
            (object_type, object_id, object_type, object_id),
        )
        conn.execute(
            """
            delete from memory_graph_migration_ledger
            where source_table = ? and source_id = ? and target_table = ? and target_id = ?
            """,
            (source_table, source_id, object_type, object_id),
        )
        conn.execute(
            """
            delete from strict_term_retirement_ownership
            where source_table = ? and source_id = ? and object_type = ? and object_id = ?
            """,
            (source_table, source_id, object_type, object_id),
        )


def _validate_alias_ownership_integrity(
    conn: sqlite3.Connection, active_ids: tuple[int, ...], affected: str
) -> None:
    ledger_rows = conn.execute(
        """
        select source_id, target_table, target_id
        from memory_graph_migration_ledger
        where source_table = 'strict_terms'
        order by source_id, target_table
        """
    ).fetchall()
    alias_ledgers: set[tuple[str, str, int]] = set()
    ledger_targets_valid = True
    for row in ledger_rows:
        target_id = _strict_positive_int(row["target_id"])
        ledger_targets_valid = ledger_targets_valid and target_id is not None
        if ":alias:" in str(row["source_id"]) and target_id is not None:
            alias_ledgers.add((str(row["source_id"]), str(row["target_table"]), target_id))
    has_provenance = _has_table(conn, "strict_term_retirement_alias_provenance")
    if alias_ledgers and not has_provenance:
        _raise_alias_ownership_blocked(active_ids, affected)
    provenance: set[tuple[str, str, int]] = set()
    valid = ledger_targets_valid
    if has_provenance:
        for row in conn.execute(
            """
            select source_table, source_id, term_id, alias_id, target_table,
                   target_id, alias_kind
            from strict_term_retirement_alias_provenance
            order by source_id
            """
        ):
            source_id = str(row["source_id"])
            parsed = _canonical_alias_source(source_id)
            term_id = _strict_positive_int(row["term_id"])
            alias_id = _strict_positive_int(row["alias_id"])
            target_table = str(row["target_table"])
            target_id = _strict_positive_int(row["target_id"])
            alias_kind = str(row["alias_kind"])
            expected_table = {
                "source_variant": "concept_facets",
                "search_alias": "concept_facets",
                "approved_variant": "concept_facets",
                "forbidden_variant": "crystals",
            }.get(alias_kind)
            row_valid = (
                row["source_table"] == "strict_terms"
                and parsed == (term_id, alias_id)
                and target_table == expected_table
                and target_id is not None
                and (source_id, target_table, target_id) in alias_ledgers
                and _alias_target_matches_term(conn, source_id, term_id, target_table, target_id)
                and _existing_alias_belongs_to_term(conn, alias_id, term_id)
            )
            valid = valid and row_valid
            if row_valid:
                provenance.add((source_id, target_table, target_id))
    valid = valid and provenance == alias_ledgers
    for row in ledger_rows:
        valid = valid and _strict_term_ledger_source_is_canonical(conn, row)
    if not valid:
        _raise_alias_ownership_blocked(active_ids, affected)


def _raise_alias_ownership_blocked(active_ids: tuple[int, ...], affected: str) -> None:
    listed = ", ".join(str(term_id) for term_id in active_ids) or affected
    raise StrictTermConversionBlocked(
        f"terms {listed}: strict term alias ownership integrity failed"
    )


def _canonical_alias_source(source_id: str) -> tuple[int, int] | None:
    parts = source_id.split(":")
    if len(parts) != 3 or parts[1] != "alias":
        return None
    term_id = _canonical_positive_decimal(parts[0])
    alias_id = _canonical_positive_decimal(parts[2])
    if term_id is None or alias_id is None:
        return None
    return term_id, alias_id


def _canonical_positive_decimal(value: str) -> int | None:
    if not value.isdigit():
        return None
    parsed = int(value)
    return parsed if parsed > 0 and str(parsed) == value else None


def _existing_alias_belongs_to_term(
    conn: sqlite3.Connection, alias_id: int | None, term_id: int | None
) -> bool:
    if alias_id is None or term_id is None:
        return False
    row = conn.execute(
        "select term_id from strict_term_aliases where id = ?", (alias_id,)
    ).fetchone()
    return row is None or int(row["term_id"]) == term_id


def _alias_target_matches_term(
    conn: sqlite3.Connection,
    source_id: str,
    term_id: int | None,
    target_table: str,
    target_id: int,
) -> bool:
    if term_id is None:
        return False
    if target_table == "concept_facets":
        return _object_is_owned_by_source(
            conn, "strict_terms", source_id, "concept_facets", target_id
        )
    if target_table == "crystals":
        return _object_is_owned_by_source(conn, "strict_terms", str(term_id), "crystals", target_id)
    return False


def _strict_term_ledger_source_is_canonical(conn: sqlite3.Connection, row: sqlite3.Row) -> bool:
    source_id = str(row["source_id"])
    target_table = str(row["target_table"])
    term_id = _canonical_term_id_from_source(source_id)
    if conn.execute("select 1 from strict_terms where id = ?", (term_id,)).fetchone() is None:
        return False
    if source_id == str(term_id):
        return target_table in {"concepts", "crystals"}
    if source_id in {f"{term_id}:source", f"{term_id}:rendering"}:
        return target_table == "concept_facets"
    return _canonical_alias_source(source_id) is not None and target_table in {
        "concept_facets",
        "crystals",
    }


def _canonical_term_id_from_source(source_id: str) -> int:
    head = source_id.partition(":")[0]
    if not head.isdigit() or str(int(head)) != head or int(head) <= 0:
        return -1
    return int(head)


def _strict_positive_int(value: object) -> int | None:
    return value if isinstance(value, int) and not isinstance(value, bool) and value > 0 else None


def _is_canonical_facet_source(source_id: str, term_id: int) -> bool:
    if term_id <= 0:
        return False
    if source_id in {f"{term_id}:source", f"{term_id}:rendering"}:
        return True
    prefix = f"{term_id}:alias:"
    suffix = source_id.removeprefix(prefix)
    return source_id.startswith(prefix) and suffix.isdigit() and str(int(suffix)) == suffix


def _relationship_ownership_row_is_valid(
    conn: sqlite3.Connection,
    row: sqlite3.Row,
    objects: Mapping[tuple[str, str], int],
    term_ids: set[int],
) -> bool:
    source_id = str(row["source_id"])
    relationship_type = str(row["relationship_type"])
    owner_type = str(row["owner_type"])
    owner_id = _strict_positive_int(row["owner_id"])
    related_type = str(row["related_type"])
    raw_related_id = row["related_id"]
    related_id = (
        raw_related_id
        if isinstance(raw_related_id, int) and not isinstance(raw_related_id, bool)
        else None
    )
    value = str(row["value"])
    term_id = _canonical_term_id_from_source(source_id)
    if (
        row["source_table"] != "strict_terms"
        or term_id not in term_ids
        or owner_id is None
        or related_id is None
        or not value
    ):
        return False
    expected = {
        "concept_semantic_tags": ("concepts", "concept_semantic_tags", "concept_id", "tag"),
        "concept_facet_language_tags": (
            "concept_facets",
            "concept_facet_language_tags",
            "facet_id",
            "language_tag",
        ),
        "crystal_language_tags": (
            "crystals",
            "crystal_language_tags",
            "crystal_id",
            "language_tag",
        ),
        "crystal_semantic_tags": (
            "crystals",
            "crystal_semantic_tags",
            "crystal_id",
            "tag",
        ),
    }
    if relationship_type == "crystal_concepts":
        return (
            source_id == str(term_id)
            and owner_type == "crystals"
            and objects.get((str(term_id), "crystals")) == owner_id
            and related_type == "concepts"
            and objects.get((str(term_id), "concepts")) == related_id
            and value == "defines"
            and conn.execute(
                """
                select 1 from crystal_concepts
                where crystal_id = ? and concept_id = ? and link_type = ?
                """,
                (owner_id, related_id, value),
            ).fetchone()
            is not None
        )
    definition = expected.get(relationship_type)
    if definition is None:
        return False
    expected_owner, table, owner_column, value_column = definition
    expected_source = source_id if expected_owner == "concept_facets" else str(term_id)
    canonical_source = (
        _is_canonical_facet_source(source_id, term_id)
        if expected_owner == "concept_facets"
        else source_id == str(term_id)
    )
    return (
        canonical_source
        and owner_type == expected_owner
        and objects.get((expected_source, expected_owner)) == owner_id
        and related_type == ""
        and related_id == 0
        and conn.execute(
            f"select 1 from {table} where {owner_column} = ? and {value_column} = ?",
            (owner_id, value),
        ).fetchone()
        is not None
    )


def _ensure_retirement_ownership_schema(conn: sqlite3.Connection) -> None:
    conn.execute(
        """
        create table if not exists strict_term_retirement_ownership(
          source_table text not null,
          source_id text not null,
          object_type text not null
            check(object_type in ('concepts', 'concept_facets', 'crystals')),
          object_id integer not null check(object_id > 0),
          created_at text not null,
          primary key(source_table, source_id, object_type),
          unique(object_type, object_id)
        )
        """
    )
    conn.execute(
        """
        create table if not exists strict_term_retirement_relationship_ownership(
          source_table text not null,
          source_id text not null,
          relationship_type text not null check(relationship_type in (
            'concept_semantic_tags', 'concept_facet_language_tags',
            'crystal_language_tags', 'crystal_semantic_tags', 'crystal_concepts'
          )),
          owner_type text not null check(owner_type in ('concepts', 'concept_facets', 'crystals')),
          owner_id integer not null check(owner_id > 0),
          related_type text not null default '' check(related_type in ('', 'concepts')),
          related_id integer not null default 0 check(related_id >= 0),
          value text not null check(value != ''),
          created_at text not null,
          primary key(
            source_table, source_id, relationship_type, owner_type, owner_id,
            related_type, related_id, value
          ),
          unique(relationship_type, owner_type, owner_id, related_type, related_id, value)
        )
        """
    )
    conn.execute(
        """
        create table if not exists strict_term_retirement_alias_provenance(
          source_table text not null check(source_table = 'strict_terms'),
          source_id text not null,
          term_id integer not null check(term_id > 0),
          alias_id integer not null check(alias_id > 0),
          target_table text not null check(target_table in ('concept_facets', 'crystals')),
          target_id integer not null check(target_id > 0),
          alias_kind text not null check(alias_kind in (
            'source_variant', 'search_alias', 'approved_variant', 'forbidden_variant'
          )),
          created_at text not null,
          primary key(source_table, source_id),
          unique(term_id, alias_id)
        )
        """
    )


def _owned_object_id(
    conn: sqlite3.Connection, source_table: str, source_id: str, object_type: str
) -> int | None:
    if not _has_table(conn, "strict_term_retirement_ownership"):
        return None
    row = conn.execute(
        """
        select object_id from strict_term_retirement_ownership
        where source_table = ? and source_id = ? and object_type = ?
        """,
        (source_table, source_id, object_type),
    ).fetchone()
    if row is None:
        return None
    try:
        object_id = int(row["object_id"])
    except (TypeError, ValueError):
        return None
    if not _row_exists(conn, object_type, object_id):
        conn.execute(
            """
            delete from strict_term_retirement_ownership
            where source_table = ? and source_id = ? and object_type = ?
            """,
            (source_table, source_id, object_type),
        )
        return None
    return object_id


def _record_owned_object(
    conn: sqlite3.Connection,
    source_table: str,
    source_id: str,
    object_type: str,
    object_id: int,
) -> None:
    conn.execute(
        """
        insert into strict_term_retirement_ownership(
          source_table, source_id, object_type, object_id, created_at
        ) values (?, ?, ?, ?, ?)
        on conflict(source_table, source_id, object_type) do update set
          object_id = excluded.object_id
        """,
        (source_table, source_id, object_type, str(object_id), _now(conn)),
    )


def _record_alias_provenance(
    conn: sqlite3.Connection,
    *,
    term_id: int,
    alias_id: int,
    target_table: str,
    target_id: int,
    alias_kind: str,
) -> None:
    source_id = f"{term_id}:alias:{alias_id}"
    conn.execute(
        """
        insert into strict_term_retirement_alias_provenance(
          source_table, source_id, term_id, alias_id, target_table,
          target_id, alias_kind, created_at
        ) values ('strict_terms', ?, ?, ?, ?, ?, ?, ?)
        on conflict(source_table, source_id) do update set
          term_id = excluded.term_id,
          alias_id = excluded.alias_id,
          target_table = excluded.target_table,
          target_id = excluded.target_id,
          alias_kind = excluded.alias_kind
        """,
        (source_id, term_id, alias_id, target_table, target_id, alias_kind, _now(conn)),
    )


def _sync_owned_values(
    conn: sqlite3.Connection,
    *,
    source_table: str,
    source_id: str,
    relationship_type: str,
    table: str,
    owner_column: str,
    owner_id: int,
    value_column: str,
    wanted: Iterable[str],
    confidence_column: str | None = None,
    confidence: float = 0.0,
) -> None:
    wanted_values = tuple(dict.fromkeys(str(value) for value in wanted))
    owner_type = {
        "concept_id": "concepts",
        "facet_id": "concept_facets",
        "crystal_id": "crystals",
    }[owner_column]
    owned_rows = conn.execute(
        """
        select owner_type, owner_id, related_type, related_id, value
        from strict_term_retirement_relationship_ownership
        where source_table = ? and source_id = ? and relationship_type = ?
        """,
        (source_table, source_id, relationship_type),
    ).fetchall()
    owned_values: set[str] = set()
    for row in owned_rows:
        row_owner_id = _strict_positive_int(row["owner_id"])
        row_related_id = row["related_id"]
        if (
            row["owner_type"] != owner_type
            or row_owner_id != owner_id
            or row["related_type"] != ""
            or row_related_id != 0
            or not _relationship_value_edge_is_safe(
                conn,
                source_table=source_table,
                source_id=source_id,
                owner_type=owner_type,
                owner_id=owner_id,
            )
        ):
            _delete_relationship_ownership_row(
                conn, source_table, source_id, relationship_type, row
            )
            continue
        value = str(row["value"])
        owned_values.add(value)
        if value in wanted_values:
            continue
        actual = conn.execute(
            f"select 1 from {table} where {owner_column} = ? and {value_column} = ?",
            (owner_id, value),
        ).fetchone()
        if actual is None:
            _delete_relationship_ownership_row(
                conn, source_table, source_id, relationship_type, row
            )
            continue
        conn.execute(
            f"delete from {table} where {owner_column} = ? and {value_column} = ?",
            (owner_id, value),
        )
        conn.execute(
            """
            delete from strict_term_retirement_relationship_ownership
            where source_table = ? and source_id = ? and relationship_type = ?
              and owner_type = ? and owner_id = ? and related_type = ''
              and related_id = 0 and value = ?
            """,
            (source_table, source_id, relationship_type, owner_type, owner_id, value),
        )
    for value in wanted_values:
        present = conn.execute(
            f"select 1 from {table} where {owner_column} = ? and {value_column} = ?",
            (owner_id, value),
        ).fetchone()
        is_owned = value in owned_values
        if present is None:
            if confidence_column is None:
                conn.execute(
                    f"insert into {table}({owner_column}, {value_column}) values (?, ?)",
                    (owner_id, value),
                )
            else:
                conn.execute(
                    f"insert into {table}({owner_column}, {value_column}, "
                    f"{confidence_column}, created_at) values (?, ?, ?, ?)",
                    (owner_id, value, confidence, _now(conn)),
                )
            is_owned = True
        elif is_owned and confidence_column is not None:
            conn.execute(
                f"update {table} set {confidence_column} = max({confidence_column}, ?) "
                f"where {owner_column} = ? and {value_column} = ?",
                (confidence, owner_id, value),
            )
        if is_owned:
            conn.execute(
                """
                insert or ignore into strict_term_retirement_relationship_ownership(
                  source_table, source_id, relationship_type, owner_type, owner_id,
                  related_type, related_id, value, created_at
                ) values (?, ?, ?, ?, ?, '', 0, ?, ?)
                """,
                (
                    source_table,
                    source_id,
                    relationship_type,
                    owner_type,
                    owner_id,
                    value,
                    _now(conn),
                ),
            )


def _sync_owned_crystal_concept_link(
    conn: sqlite3.Connection,
    *,
    source_table: str,
    source_id: str,
    crystal_id: int,
    concept_id: int,
    confidence: float,
) -> None:
    relationship_type = "crystal_concepts"
    rows = conn.execute(
        """
        select owner_type, owner_id, related_type, related_id, value
        from strict_term_retirement_relationship_ownership
        where source_table = ? and source_id = ? and relationship_type = ?
        """,
        (source_table, source_id, relationship_type),
    ).fetchall()
    is_owned = False
    for row in rows:
        item = (
            str(row["owner_type"]),
            _strict_positive_int(row["owner_id"]),
            str(row["related_type"]),
            _strict_positive_int(row["related_id"]),
            str(row["value"]),
        )
        if item == ("crystals", crystal_id, "concepts", concept_id, "defines"):
            is_owned = True
        else:
            _delete_relationship_ownership_row(
                conn, source_table, source_id, relationship_type, row
            )
    present = conn.execute(
        """
        select 1 from crystal_concepts
        where crystal_id = ? and concept_id = ? and link_type = 'defines'
        """,
        (crystal_id, concept_id),
    ).fetchone()
    if present is None:
        conn.execute(
            """
            insert into crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at)
            values (?, ?, 'defines', ?, ?)
            """,
            (crystal_id, concept_id, confidence, _now(conn)),
        )
        is_owned = True
    elif is_owned:
        conn.execute(
            """
            update crystal_concepts set confidence = max(confidence, ?)
            where crystal_id = ? and concept_id = ? and link_type = 'defines'
            """,
            (confidence, crystal_id, concept_id),
        )
    if is_owned:
        conn.execute(
            """
            insert or ignore into strict_term_retirement_relationship_ownership(
              source_table, source_id, relationship_type, owner_type, owner_id,
              related_type, related_id, value, created_at
            ) values (?, ?, ?, 'crystals', ?, 'concepts', ?, 'defines', ?)
            """,
            (source_table, source_id, relationship_type, crystal_id, concept_id, _now(conn)),
        )


def _object_is_owned_by_source(
    conn: sqlite3.Connection,
    source_table: str,
    source_id: str,
    object_type: str,
    object_id: int,
) -> bool:
    return (
        conn.execute(
            """
            select 1 from strict_term_retirement_ownership
            where source_table = ? and source_id = ? and object_type = ? and object_id = ?
            """,
            (source_table, source_id, object_type, object_id),
        ).fetchone()
        is not None
    )


def _relationship_value_edge_is_safe(
    conn: sqlite3.Connection,
    *,
    source_table: str,
    source_id: str,
    owner_type: str,
    owner_id: int,
) -> bool:
    term_id = _canonical_term_id_from_source(source_id)
    canonical_source = (
        _is_canonical_facet_source(source_id, term_id)
        if owner_type == "concept_facets"
        else source_id == str(term_id)
    )
    return (
        source_table == "strict_terms"
        and canonical_source
        and conn.execute(
            "select 1 from strict_terms where id = ? and status in ('approved', 'active')",
            (term_id,),
        ).fetchone()
        is not None
        and _object_is_owned_by_source(conn, source_table, source_id, owner_type, owner_id)
    )


def _delete_relationship_ownership_row(
    conn: sqlite3.Connection,
    source_table: str,
    source_id: str,
    relationship_type: str,
    row: sqlite3.Row,
) -> None:
    conn.execute(
        """
        delete from strict_term_retirement_relationship_ownership
        where source_table = ? and source_id = ? and relationship_type = ?
          and owner_type = ? and owner_id = ? and related_type = ?
          and related_id = ? and value = ?
        """,
        (
            source_table,
            source_id,
            relationship_type,
            row["owner_type"],
            row["owner_id"],
            row["related_type"],
            row["related_id"],
            row["value"],
        ),
    )


def _delete_owned_object(
    conn: sqlite3.Connection,
    source_table: str,
    source_id: str,
    object_type: str,
    object_id: int,
) -> None:
    if object_type != "concept_facets" or not _facet_is_exclusively_owned(
        conn, source_id, object_id
    ):
        raise StrictTermConversionBlocked(
            f"term {source_id.split(':', 1)[0]}: strict term retirement ownership is not exclusive"
        )
    conn.execute(
        """
        delete from strict_term_retirement_ownership
        where source_table = ? and source_id = ? and object_type = ? and object_id = ?
        """,
        (source_table, source_id, object_type, object_id),
    )
    conn.execute(f"delete from {object_type} where id = ?", (object_id,))
    conn.execute(
        """
        delete from strict_term_retirement_relationship_ownership
        where source_table = ? and source_id = ?
        """,
        (source_table, source_id),
    )


def _release_owned_object(
    conn: sqlite3.Connection,
    source_table: str,
    source_id: str,
    object_type: str,
    object_id: int,
) -> None:
    conn.execute(
        """
        delete from strict_term_retirement_ownership
        where source_table = ? and source_id = ? and object_type = ? and object_id = ?
        """,
        (source_table, source_id, object_type, object_id),
    )
    conn.execute(
        """
        delete from strict_term_retirement_relationship_ownership
        where source_table = ? and source_id = ?
        """,
        (source_table, source_id),
    )


def _facet_is_exclusively_owned(conn: sqlite3.Connection, source_id: str, facet_id: int) -> bool:
    term_id = _canonical_term_id_from_source(source_id)
    if (
        not _is_canonical_facet_source(source_id, term_id)
        or not _object_is_owned_by_source(
            conn, "strict_terms", source_id, "concept_facets", facet_id
        )
        or conn.execute(
            "select 1 from strict_terms where id = ? and status in ('approved', 'active')",
            (term_id,),
        ).fetchone()
        is None
    ):
        return False
    facet = conn.execute(
        "select source_crystal_id from concept_facets where id = ?", (facet_id,)
    ).fetchone()
    if facet is None or facet["source_crystal_id"] is not None:
        return False
    if conn.execute(
        """
        select 1 from memory_graph_migration_ledger
        where target_table = 'concept_facets' and target_id = ? limit 1
        """,
        (facet_id,),
    ).fetchone():
        return False
    if conn.execute(
        """
        select 1 from strict_term_retirement_ownership
        where object_type = 'concept_facets' and object_id = ?
          and not (source_table = 'strict_terms' and source_id = ?)
        limit 1
        """,
        (facet_id, source_id),
    ).fetchone():
        return False
    if conn.execute(
        """
        select 1 from strict_term_retirement_relationship_ownership
        where owner_type = 'concept_facets' and owner_id = ?
          and not (source_table = 'strict_terms' and source_id = ?)
        limit 1
        """,
        (facet_id, source_id),
    ).fetchone():
        return False
    owned_languages = {
        str(row["value"])
        for row in conn.execute(
            """
            select value from strict_term_retirement_relationship_ownership
            where source_table = 'strict_terms' and source_id = ?
              and relationship_type = 'concept_facet_language_tags'
              and owner_type = 'concept_facets' and owner_id = ?
            """,
            (source_id, facet_id),
        )
    }
    actual_languages = {
        str(row["language_tag"])
        for row in conn.execute(
            "select language_tag from concept_facet_language_tags where facet_id = ?",
            (facet_id,),
        )
    }
    if actual_languages - owned_languages:
        return False
    for table in ("concept_facet_story_scopes", "concept_facet_semantic_tags"):
        if conn.execute(
            f"select 1 from {table} where facet_id = ? limit 1", (facet_id,)
        ).fetchone():
            return False
    return True
