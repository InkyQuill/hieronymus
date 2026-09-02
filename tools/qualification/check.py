"""Network-free qualification record checker and aggregate gate computation.

This module never imports a live runner: ``--records-only`` checking must stay
importable and runnable without Cargo, Bun, model, bundle, subprocess, socket,
or SQLite-library boundaries. SQLite fixture files are only ever read as bytes
through the immutable input fingerprints.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, cast

from jsonschema import Draft202012Validator
from jsonschema.protocols import Validator

from tools.qualification.model import (
    REQUIRED_CRITERIA,
    QualificationRecord,
    Risk,
    _StrictDraft202012Validator,
    load_record,
    serialize_record,
)
from tools.qualification.projections import ProjectionRisk, projection_issues
from tools.qualification.redaction import _literal_markdown
from tools.qualification.render import render_record
from tools.qualification.validate import validate_record

_TARGET = "x86_64-unknown-linux-gnu"
_OWNER = "Pavel Obruchnikov <me@inkyquill.net>"
_ROOT = Path(__file__).resolve().parents[2]
_GATE_SCHEMA_PATH = _ROOT / "qualification/schemas/gate.schema.json"
AGGREGATE_PATH = Path("qualification/records/aggregate.json")
GATE_MARKDOWN_PATH = Path("docs/qualification/rust/gate.md")
RECORD_PATHS: dict[Risk, Path] = {
    risk: Path(f"qualification/records/{risk}.json") for risk in REQUIRED_CRITERIA
}
MARKDOWN_PATHS: dict[Risk, Path] = {
    risk: Path(f"docs/qualification/rust/{risk}.md") for risk in REQUIRED_CRITERIA
}

ReleaseMode = Literal["semantic-enabled", "fts-only"]

BLOCKING_PLANS: dict[Risk, tuple[str, ...]] = {
    "mcp-transport": (
        "rust-workspace-and-contract-harness",
        "rust-daemon-mcp-security",
    ),
    "semantic-native": (),
    "frontend-embedding": (
        "rust-workspace-and-contract-harness",
        "rust-frontend",
        "rust-distribution-cutover",
    ),
    "legacy-database-import": (
        "rust-workspace-and-contract-harness",
        "rust-database-upgrade",
        "rust-distribution-cutover",
    ),
}


@dataclass(frozen=True)
class GateRecord:
    schema_version: int
    target: str
    status: Literal["qualified", "blocked"]
    release_mode: Literal["semantic-enabled", "fts-only"] | None
    blocked_plans: tuple[str, ...]
    issues: tuple[str, ...]
    record_digests: dict[Risk, str]
    acceptance_owner: str


def record_digests(records: Mapping[Risk, QualificationRecord]) -> dict[Risk, str]:
    """Return each record's stored immutable input digest by risk."""
    return {risk: record.input_digest for risk, record in records.items()}


def validation_and_review_issues(
    records: Mapping[Risk, QualificationRecord],
    repo_root: Path,
) -> dict[Risk, tuple[str, ...]]:
    """Report every blocking validation, projection, and review issue by risk.

    ``projection_issues`` is evaluated once; its MCP/database findings attach
    only to their matching risks. Unrelated global manifest/state fields live
    outside the projections and are never staleness here.
    """
    projections = projection_issues(repo_root)
    issues: dict[str, tuple[str, ...]] = {}
    for name in sorted(set(records) - set(REQUIRED_CRITERIA)):
        issues[name] = (f"{name} is not a recognized qualification risk",)
    for risk in REQUIRED_CRITERIA:
        record = records.get(risk)
        if record is None:
            issues[str(risk)] = ("qualification record is missing",)
            continue
        issues[str(risk)] = _record_issues(
            risk,
            record,
            repo_root,
            projections.get(cast(ProjectionRisk, risk), ()),
        )
    return cast(dict[Risk, tuple[str, ...]], issues)


def _record_issues(
    risk: Risk,
    record: QualificationRecord,
    repo_root: Path,
    projection_findings: tuple[str, ...],
) -> tuple[str, ...]:
    found: list[str] = []
    try:
        found.extend(validate_record(record, repo_root))
    except (OSError, TypeError, ValueError):
        found.append("qualification record is invalid")
        return tuple(found)
    try:
        canonical = serialize_record(record)
    except (TypeError, ValueError):
        return tuple(found)

    # The checked-in Markdown is the rendering of the checked-in record JSON.
    # Only a record byte-identical to that checked-in JSON is compared against
    # it, so synthetic in-memory records are not judged by another record's
    # committed rendering.
    try:
        checked_record = (repo_root / RECORD_PATHS[risk]).read_bytes()
        checked_markdown = (repo_root / MARKDOWN_PATHS[risk]).read_bytes()
    except OSError:
        found.append("canonical record or Markdown file is missing")
    else:
        if checked_record == (canonical + "\n").encode("utf-8"):
            try:
                rendered = render_record(record).encode("utf-8")
            except (TypeError, ValueError):
                found.append("qualification Markdown could not be rendered")
            else:
                if checked_markdown != rendered:
                    found.append("qualification Markdown is stale")

    review = record.review
    if review.status != "accepted":
        found.append("review is not accepted")
    else:
        if not review.objective_evidence_reviewed:
            found.append("review must confirm objective evidence was reviewed")
        if not review.normative_constraints_preserved:
            found.append("review must confirm normative constraints were preserved")

    # validate_record already reports the same findings for the two projection
    # risks; attach each finding once, in deterministic projection order.
    for finding in projection_findings:
        if finding not in found:
            found.append(finding)
    return tuple(found)


def compute_gate(
    records: Mapping[Risk, QualificationRecord],
    repo_root: Path,
) -> GateRecord:
    risk_issues = validation_and_review_issues(records, repo_root)
    issues = tuple(
        sorted(f"{risk}: {issue}" for risk, found in risk_issues.items() for issue in found)
    )
    blocked: set[str] = set()
    for risk in REQUIRED_CRITERIA:
        record = records.get(risk)
        if risk_issues[risk]:
            if risk == "semantic-native":
                blocked.add("rust-workspace-and-contract-harness")
            else:
                blocked.update(BLOCKING_PLANS[risk])
        elif risk != "semantic-native" and record is not None and record.decision != "qualified":
            blocked.update(BLOCKING_PLANS[risk])
    qualified = not issues and not blocked
    semantic = records.get("semantic-native")
    if qualified:
        assert semantic is not None
        assert semantic.decision in ("semantic-enabled", "fts-only")
        release_mode = semantic.decision
    else:
        release_mode = None
    return GateRecord(
        schema_version=1,
        target=_TARGET,
        status="qualified" if qualified else "blocked",
        release_mode=cast(ReleaseMode | None, release_mode),
        blocked_plans=tuple(sorted(blocked)),
        issues=issues,
        record_digests=record_digests(records),
        acceptance_owner=_OWNER,
    )


def gate_schema_validator() -> Validator:
    """Return the single strict Draft 2020-12 validator for aggregate gates."""
    schema = json.loads(_GATE_SCHEMA_PATH.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    return _StrictDraft202012Validator(schema)


def gate_payload(gate: GateRecord) -> dict[str, object]:
    """Return the gate as the exact JSON object the gate schema describes."""
    return {
        "schema_version": gate.schema_version,
        "target": gate.target,
        "status": gate.status,
        "release_mode": gate.release_mode,
        "blocked_plans": list(gate.blocked_plans),
        "issues": list(gate.issues),
        "record_digests": dict(sorted(gate.record_digests.items())),
        "acceptance_owner": gate.acceptance_owner,
    }


def gate_schema_issues(payload: Mapping[str, object]) -> list[str]:
    """Return every gate-schema violation of one aggregate payload, sorted."""
    errors = sorted(
        gate_schema_validator().iter_errors(dict(payload)),
        key=lambda error: error.message,
    )
    return [error.message for error in errors]


def serialize_gate(gate: GateRecord) -> bytes:
    """Serialize one gate as canonical UTF-8 JSON with a trailing newline."""
    return (
        json.dumps(
            gate_payload(gate),
            ensure_ascii=False,
            allow_nan=False,
            sort_keys=True,
            separators=(",", ":"),
        )
        + "\n"
    ).encode("utf-8")


def render_gate(gate: GateRecord) -> str:
    """Render one gate as deterministic, inert Markdown."""
    release_mode = gate.release_mode if gate.release_mode is not None else "(none)"
    lines = [
        "# Rust Qualification Gate",
        "",
        "## Decision",
        "",
        "| Field | Value |",
        "| --- | --- |",
        f"| Target | {_literal_markdown(gate.target)} |",
        f"| Status | {_literal_markdown(gate.status)} |",
        f"| Release mode | {_literal_markdown(release_mode)} |",
        f"| Acceptance owner | {_literal_markdown(gate.acceptance_owner)} |",
        "",
        "## Blocked Plans",
        "",
    ]
    lines.extend(f"- {_literal_markdown(plan)}" for plan in gate.blocked_plans)
    if not gate.blocked_plans:
        lines.append("- (none)")
    lines.extend(["", "## Gate Issues", ""])
    lines.extend(f"- {_literal_markdown(issue)}" for issue in gate.issues)
    if not gate.issues:
        lines.append("- (none)")
    lines.extend(
        [
            "",
            "## Record Digests",
            "",
            "| Risk | Input digest |",
            "| --- | --- |",
        ]
    )
    lines.extend(
        f"| {_literal_markdown(risk)} | `{gate.record_digests[risk]}` |"
        for risk in sorted(gate.record_digests)
    )
    lines.append("")
    return "\n".join(lines)


def load_records(repo_root: Path) -> dict[Risk, QualificationRecord]:
    """Load every checked-in risk record that exists, in risk order."""
    records: dict[Risk, QualificationRecord] = {}
    for risk, relative in RECORD_PATHS.items():
        path = repo_root / relative
        if path.is_file():
            records[risk] = load_record(path)
    return records


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Check Rust qualification records and the aggregate gate"
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--record", choices=tuple(REQUIRED_CRITERIA), metavar="RISK")
    mode.add_argument("--records-only", action="store_true")
    mode.add_argument("--require-qualified", action="store_true")
    arguments = parser.parse_args(argv)
    repo_root = Path.cwd()

    if arguments.record is not None:
        return _check_record(cast(Risk, arguments.record), repo_root)
    try:
        records = load_records(repo_root)
    except (OSError, TypeError, ValueError):
        print("qualification records could not be loaded", file=sys.stderr)
        return 2
    gate = compute_gate(records, repo_root)
    if arguments.require_qualified:
        return _require_qualified(gate)
    return _records_only(repo_root, gate)


def _check_record(risk: Risk, repo_root: Path) -> int:
    try:
        record = load_record(repo_root / RECORD_PATHS[risk])
        projections = projection_issues(repo_root)
    except (OSError, TypeError, ValueError):
        print(f"{risk} qualification record could not be loaded", file=sys.stderr)
        return 2
    issues = _record_issues(
        risk,
        record,
        repo_root,
        projections.get(cast(ProjectionRisk, risk), ()),
    )
    if issues:
        print(f"{risk} qualification record is blocked", file=sys.stderr)
        for issue in issues:
            print(f"{risk}: {issue}", file=sys.stderr)
        return 1
    print(f"{risk} qualification record and Markdown are current")
    return 0


def _records_only(repo_root: Path, gate: GateRecord) -> int:
    drift = gate_schema_issues(gate_payload(gate))
    try:
        aggregate = (repo_root / AGGREGATE_PATH).read_bytes()
        gate_markdown = (repo_root / GATE_MARKDOWN_PATH).read_bytes()
    except OSError:
        drift.append("qualification aggregate record or Markdown file is missing")
    else:
        if aggregate != serialize_gate(gate):
            drift.append("qualification aggregate JSON is stale")
        if gate_markdown != render_gate(gate).encode("utf-8"):
            drift.append("qualification gate Markdown is stale")
    if drift:
        print("qualification records or aggregate gate are stale", file=sys.stderr)
        for line in drift:
            print(line, file=sys.stderr)
        for issue in gate.issues:
            print(issue, file=sys.stderr)
        return 1
    print(f"Rust qualification gate: {gate.status} (records internally consistent)")
    return 0


def _require_qualified(gate: GateRecord) -> int:
    if gate.status == "qualified":
        assert gate.release_mode is not None
        print(f"Rust qualification gate: qualified ({gate.release_mode})")
        return 0
    print("Rust qualification gate: blocked")
    for issue in gate.issues:
        print(issue)
    for plan in gate.blocked_plans:
        print(plan)
    return 1


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
