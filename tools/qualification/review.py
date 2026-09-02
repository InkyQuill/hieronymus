"""Review-only record transitions and atomic artifact regeneration.

This module never imports a live runner and never changes measured evidence:
``review_record`` copies one already-validated record with only its ``Review``
block set to the owner's decision, and the CLI rewrites one review block plus
the aggregate JSON/Markdown it implies. Every replacement file is staged in
its own destination directory, validated in memory, and installed with
``os.replace`` only after its bytes are valid. The reviewed record JSON and
Markdown land first so the regenerated aggregate always describes the record
set this command leaves behind: a crash between replacements leaves
detectable drift and rerunning the same command converges to the same bytes.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from collections.abc import Sequence
from dataclasses import replace
from pathlib import Path
from typing import Literal, cast

from tools.qualification.check import (
    AGGREGATE_PATH,
    GATE_MARKDOWN_PATH,
    MARKDOWN_PATHS,
    RECORD_PATHS,
    GateRecord,
    compute_gate,
    gate_payload,
    gate_schema_issues,
    load_records,
    render_gate,
    serialize_gate,
)
from tools.qualification.model import (
    REQUIRED_CRITERIA,
    QualificationRecord,
    Review,
    Risk,
    load_record,
    serialize_record,
)
from tools.qualification.render import render_record
from tools.qualification.validate import validate_record

_OWNER = "Pavel Obruchnikov <me@inkyquill.net>"
_REVIEW_OUTCOMES = ("accepted", "rejected")


def review_record(
    record: QualificationRecord,
    *,
    owner: str,
    status: Literal["accepted", "rejected"],
    objective_evidence_reviewed: bool,
    normative_constraints_preserved: bool,
) -> QualificationRecord:
    """Copy one record with only its ``Review`` block set to the decision.

    The immutable measured record is validated through the strict typed
    boundary first, then the exact owner and the closed CLI contract
    (accepted only with both assertions true; rejected only while at least
    one assertion remains conservatively false) are enforced. An identical
    re-review converges; any divergent transition after a recorded decision
    is refused.
    """
    try:
        serialize_record(record)
    except (TypeError, ValueError) as error:
        raise ValueError("qualification record is invalid") from error
    if owner != _OWNER:
        raise ValueError(f"review owner must be {_OWNER}")
    if status not in _REVIEW_OUTCOMES:
        raise ValueError("review status must be accepted or rejected")
    if status == "accepted":
        if not objective_evidence_reviewed or not normative_constraints_preserved:
            raise ValueError("acceptance requires both review assertions to be true")
    elif objective_evidence_reviewed and normative_constraints_preserved:
        raise ValueError("rejection requires at least one review assertion to remain false")
    requested = Review(
        owner=owner,
        status=cast(Literal["accepted", "rejected"], status),
        objective_evidence_reviewed=objective_evidence_reviewed,
        normative_constraints_preserved=normative_constraints_preserved,
    )
    if record.review == requested:
        return record
    if record.review.status != "pending":
        raise ValueError(
            "review decision is already recorded; rerun the owning harness to "
            "create a new measured record before reviewing it again"
        )
    return replace(record, review=requested)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Record one qualification review decision and regenerate artifacts"
    )
    parser.add_argument("risk", choices=tuple(REQUIRED_CRITERIA), metavar="RISK")
    parser.add_argument("--status", choices=_REVIEW_OUTCOMES, required=True)
    parser.add_argument("--owner", required=True)
    parser.add_argument(
        "--objective-evidence-reviewed",
        choices=("true", "false"),
        required=True,
    )
    parser.add_argument(
        "--normative-constraints-preserved",
        choices=("true", "false"),
        required=True,
    )
    arguments = parser.parse_args(argv)
    repo_root = Path.cwd()
    risk = cast(Risk, arguments.risk)

    if arguments.owner != _OWNER:
        print(f"review owner must be {_OWNER}", file=sys.stderr)
        return 2
    try:
        record = load_record(repo_root / RECORD_PATHS[risk])
    except (OSError, TypeError, ValueError):
        print(f"{risk} qualification record could not be loaded", file=sys.stderr)
        return 2

    # Task 19's single validation path: the strict record boundary, immutable
    # input-digest freshness, replay-command safety, cleanup assertions, and
    # the appropriate projection currency for this risk.
    try:
        issues = validate_record(record, repo_root)
    except (OSError, TypeError, ValueError):
        issues = [f"{risk} qualification record could not be validated"]
    if issues:
        print(f"{risk} qualification record is blocked", file=sys.stderr)
        for issue in issues:
            print(f"{risk}: {issue}", file=sys.stderr)
        return 1

    try:
        reviewed = review_record(
            record,
            owner=arguments.owner,
            status=cast(Literal["accepted", "rejected"], arguments.status),
            objective_evidence_reviewed=arguments.objective_evidence_reviewed == "true",
            normative_constraints_preserved=(arguments.normative_constraints_preserved == "true"),
        )
    except (TypeError, ValueError) as error:
        print(f"{risk} review transition is invalid: {error}", file=sys.stderr)
        return 2

    try:
        records = load_records(repo_root)
    except (OSError, TypeError, ValueError):
        print("qualification records could not be loaded", file=sys.stderr)
        return 2
    records[risk] = reviewed
    if len(records) != len(REQUIRED_CRITERIA):
        missing = sorted(set(REQUIRED_CRITERIA) - set(records))
        print(
            "aggregate not written; missing qualification records: " + ", ".join(missing),
            file=sys.stderr,
        )
        return 2

    try:
        # The reviewed record JSON and Markdown are validated in memory and
        # replaced first: the aggregate gate must describe the record set as
        # this command leaves it, so a crash between replacements leaves
        # detectable drift and rerunning the same command converges to the
        # same bytes instead of baking that drift into a regenerated gate.
        record_bytes = (serialize_record(reviewed) + "\n").encode("utf-8")
        markdown_bytes = render_record(reviewed).encode("utf-8")
        _validate_record_payloads(record_bytes, markdown_bytes, reviewed)
        _stage_and_replace(
            (
                (repo_root / RECORD_PATHS[risk], record_bytes),
                (repo_root / MARKDOWN_PATHS[risk], markdown_bytes),
            )
        )
        gate = compute_gate(load_records(repo_root), repo_root)
        if gate_schema_issues(gate_payload(gate)):
            raise ValueError("aggregate gate violates the gate schema")
        aggregate_bytes = serialize_gate(gate)
        gate_markdown_bytes = render_gate(gate).encode("utf-8")
        _validate_gate_payloads(aggregate_bytes, gate_markdown_bytes, gate)
        _stage_and_replace(
            (
                (repo_root / AGGREGATE_PATH, aggregate_bytes),
                (repo_root / GATE_MARKDOWN_PATH, gate_markdown_bytes),
            )
        )
    except (OSError, TypeError, ValueError):
        print(f"{risk} review artifacts could not be replaced", file=sys.stderr)
        return 2

    print(f"{risk} review recorded: {reviewed.review.status}")
    print(f"qualification aggregate recomputed: {gate.status}")
    return 0


def _stage_and_replace(replacements: Sequence[tuple[Path, bytes]]) -> None:
    """Stage every payload beside its destination, then os.replace each."""
    temporary_paths: list[Path] = []
    try:
        staged: list[tuple[Path, Path]] = []
        for destination, content in replacements:
            destination.parent.mkdir(parents=True, exist_ok=True)
            descriptor, name = tempfile.mkstemp(
                prefix=f".{destination.name}.",
                dir=destination.parent,
            )
            temporary = Path(name)
            temporary_paths.append(temporary)
            try:
                with os.fdopen(descriptor, "wb") as stream:
                    stream.write(content)
                    stream.flush()
                    os.fsync(stream.fileno())
            except BaseException:
                try:
                    os.close(descriptor)
                except OSError:
                    pass
                raise
            staged.append((temporary, destination))
        for temporary, destination in staged:
            os.replace(temporary, destination)
            temporary_paths.remove(temporary)
    finally:
        for temporary in temporary_paths:
            try:
                temporary.unlink()
            except OSError:
                pass


def _validate_record_payloads(
    record_bytes: bytes,
    record_markdown: bytes,
    reviewed: QualificationRecord,
) -> None:
    """Fail before staging unless both record replacement bytes are valid."""
    if json.loads(record_bytes.decode("utf-8")) != json.loads(serialize_record(reviewed)):
        raise ValueError("reviewed qualification record payload is invalid")
    if record_markdown.decode("utf-8") != render_record(reviewed):
        raise ValueError("reviewed qualification Markdown payload is invalid")


def _validate_gate_payloads(
    aggregate_bytes: bytes,
    gate_markdown: bytes,
    gate: GateRecord,
) -> None:
    """Fail before staging unless both aggregate replacement bytes are valid."""
    if json.loads(aggregate_bytes.decode("utf-8")) != gate_payload(gate):
        raise ValueError("aggregate gate payload is invalid")
    if gate_markdown.decode("utf-8") != render_gate(gate):
        raise ValueError("aggregate gate Markdown payload is invalid")


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
