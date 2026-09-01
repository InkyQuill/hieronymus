"""Deterministic validation gate for one qualification record."""

from __future__ import annotations

import argparse
import sys
from collections.abc import Sequence
from pathlib import Path, PureWindowsPath

from tools.qualification.fingerprint import COMMON_FINGERPRINT_INPUTS, fingerprint_inputs
from tools.qualification.model import (
    REQUIRED_CRITERIA,
    QualificationRecord,
    decision_for,
    expected_consequence,
    load_record,
    serialize_record,
)
from tools.qualification.redaction import redaction_issues


def validate_record(record: QualificationRecord, repo_root: Path) -> list[str]:
    """Report every deterministic record, content, cleanup, and freshness issue."""
    issues: list[str] = []
    try:
        serialized = serialize_record(record)
    except (TypeError, ValueError):
        serialized = None
        issues.append("record violates the strict typed/schema boundary")
    if serialized is not None:
        issues.extend(redaction_issues(serialized))

    risk = getattr(record, "risk", None)
    evidence = getattr(record, "evidence", None)
    ordered_evidence = None
    expected_status = None
    if risk in REQUIRED_CRITERIA and type(evidence) is tuple:
        expected_criteria = REQUIRED_CRITERIA[risk]
        criteria = tuple(getattr(item, "criterion", None) for item in evidence)
        if criteria != expected_criteria:
            issues.append(
                "record criterion evidence must exactly match the ordered required "
                f"criteria for {risk}"
            )
        if len(criteria) == len(set(criteria)) and set(criteria) == set(expected_criteria):
            evidence_by_criterion = {getattr(item, "criterion", None): item for item in evidence}
            ordered_evidence = tuple(
                evidence_by_criterion[criterion] for criterion in expected_criteria
            )
        if all(getattr(item, "status", None) == "pass" for item in evidence):
            expected_status = "pass"
        elif all(getattr(item, "status", None) in ("pass", "fail", "not-run") for item in evidence):
            expected_status = "fail"

    if expected_status is not None:
        if getattr(record, "status", None) != expected_status:
            issues.append(f"record status must be {expected_status} for its criterion evidence")
        if ordered_evidence is not None:
            expected_decision = decision_for(risk, ordered_evidence)
            if getattr(record, "decision", None) != expected_decision:
                issues.append(
                    f"record decision must be {expected_decision} for "
                    f"{risk} {expected_status} evidence"
                )
        expected = expected_consequence(risk, expected_status)
        if getattr(record, "consequence", None) != expected:
            issues.append(
                f"record consequence must match the immutable {risk} {expected_status} consequence"
            )

    input_paths = getattr(record, "input_paths", None)
    fingerprint = None
    if type(input_paths) is tuple and all(type(path) is str for path in input_paths):
        if input_paths[: len(COMMON_FINGERPRINT_INPUTS)] != COMMON_FINGERPRINT_INPUTS:
            issues.append("record input_paths must start with the exact common fingerprint inputs")
        try:
            fingerprint = fingerprint_inputs(repo_root, input_paths)
        except ValueError:
            issues.append("record input fingerprint inputs are invalid")
    else:
        issues.append("record input_paths must be an ordered tuple of relative paths")
    if fingerprint is not None and getattr(record, "input_digest", None) != fingerprint:
        issues.append("record input digest is stale")

    commands = getattr(record, "commands", None)
    if not _valid_commands(commands):
        issues.append("record commands must be nonempty single-line relative commands")
    if type(commands) is tuple and len(commands) != len(set(commands)):
        issues.append("record commands must not contain duplicates")

    cleanup = getattr(record, "cleanup", None)
    cleanup_expectations = (
        ("work_dir_removed", True),
        ("raw_logs_removed", True),
        ("install_dir_removed", True),
        ("source_inputs_unchanged", True),
        ("user_data_opened", False),
        ("core_dumps_disabled", True),
        ("owned_process_groups_reaped", True),
    )
    for name, expected in cleanup_expectations:
        if getattr(cleanup, name, None) is not expected:
            issues.append(f"record cleanup {name} must be {str(expected).lower()}")
    return issues


def _valid_commands(value: object) -> bool:
    if type(value) is not tuple or not value:
        return False
    for command in value:
        if type(command) is not str or not command.strip() or "\n" in command or "\r" in command:
            return False
        first = command.lstrip().split(maxsplit=1)[0]
        if first.startswith(("/", "~")):
            return False
        windows = PureWindowsPath(first)
        if windows.is_absolute() or windows.drive or windows.root:
            return False
    return True


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Validate a Rust qualification record")
    parser.add_argument("record", type=Path)
    arguments = parser.parse_args(argv)
    try:
        record = load_record(arguments.record)
    except (OSError, TypeError, ValueError):
        print("qualification record could not be loaded", file=sys.stderr)
        return 2

    issues = validate_record(record, Path.cwd())
    if issues:
        print("qualification record is invalid", file=sys.stderr)
        for issue in issues:
            print(issue, file=sys.stderr)
        return 1
    print("qualification record is valid")
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
