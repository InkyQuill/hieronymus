"""Deterministic validation gate for one qualification record."""

# ruff: noqa: E501 -- canonical replay commands are deliberately byte-exact literals.

from __future__ import annotations

import argparse
import shlex
import sys
from collections.abc import Sequence
from pathlib import Path

from tools.qualification.fingerprint import fingerprint_inputs, required_fingerprint_inputs
from tools.qualification.model import (
    REQUIRED_CRITERIA,
    QualificationRecord,
    decision_for,
    expected_consequence,
    load_record,
    serialize_record,
)
from tools.qualification.projections import ProjectionRisk, projection_issues
from tools.qualification.redaction import redaction_issues

_PROJECTION_RISKS: tuple[ProjectionRisk, ...] = (
    "mcp-transport",
    "legacy-database-import",
)


def _planned_replay_commands() -> tuple[str, ...]:
    risks = tuple(REQUIRED_CRITERIA)
    commands = [
        "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_mcp --write",
        "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_semantic --write",
        "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_frontend --write",
        "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_database --write",
        "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run all --write",
        "uv run python -m tools.qualification.projections --check",
        "uv run --no-cache --no-sync python -B -m tools.qualification.projections --check",
        "uv run python -m tools.qualification.check --records-only",
        "uv run python -m tools.qualification.check --require-qualified",
        "uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only",
        "uv run --no-cache --no-sync python -B -m tools.qualification.check --require-qualified",
        "uv run python -m tools.qualification.clean",
        "uv run python -m tools.qualification.clean --apply",
        "uv run python -m tools.qualification.clean --apply --include-model",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 metadata --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --format-version 1",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 tree --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked -e features",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --check",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu -- -D warnings",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/semantic-native/Cargo.toml --check",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --all-targets --features semantic-native --target x86_64-unknown-linux-gnu -- -D warnings",
        "unshare --user --map-root-user --net -- bun run --cwd frontend build -- --outDir ../qualification/.artifacts/frontend-dist/current --emptyOutDir",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 build --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --target x86_64-unknown-linux-gnu",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --check",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --check",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings",
    ]
    for risk in risks:
        commands.extend(
            (
                f"uv run python -m tools.qualification.validate qualification/records/{risk}.json",
                f"uv run python -m tools.qualification.render --check qualification/records/{risk}.json docs/qualification/rust/{risk}.md",
                f"uv run python -m tools.qualification.check --record {risk}",
            )
        )
        for status, objective, normative in (
            ("accepted", "true", "true"),
            ("rejected", "false", "false"),
            ("rejected", "true", "false"),
            ("rejected", "false", "true"),
        ):
            commands.append(
                f"uv run python -m tools.qualification.review {risk} --status {status} "
                '--owner "Pavel Obruchnikov <me@inkyquill.net>" '
                f"--objective-evidence-reviewed {objective} "
                f"--normative-constraints-preserved {normative}"
            )
    return tuple(commands)


PLANNED_REPLAY_COMMANDS = _planned_replay_commands()
_PLANNED_REPLAY_COMMAND_SET = frozenset(PLANNED_REPLAY_COMMANDS)


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
        expected_inputs = required_fingerprint_inputs(risk) if risk in REQUIRED_CRITERIA else None
        if expected_inputs is not None and input_paths != expected_inputs:
            issues.append(f"record input_paths must exactly match the {risk} fingerprint policy")
        try:
            fingerprint = fingerprint_inputs(repo_root, input_paths)
        except ValueError:
            issues.append("record input fingerprint inputs are invalid")
    else:
        issues.append("record input_paths must be an ordered tuple of relative paths")
    if fingerprint is not None and getattr(record, "input_digest", None) != fingerprint:
        issues.append("record input digest is stale")
    if risk in _PROJECTION_RISKS:
        try:
            issues.extend(projection_issues(repo_root)[risk])
        except (OSError, TypeError, ValueError):
            issues.append(f"{risk} compatibility projection could not be checked")

    commands = getattr(record, "commands", None)
    if not replay_commands_are_safe(commands):
        issues.append("record commands must be safe relative replay commands")
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


def replay_commands_are_safe(value: object) -> bool:
    """Return whether commands belong to the closed offline replay grammar."""
    if type(value) is not tuple or not value or any(type(command) is not str for command in value):
        return False
    if len(value) != len(set(value)):
        return False
    return all(_replay_command_is_safe(command) for command in value)


def _replay_command_is_safe(command: object) -> bool:
    if type(command) is str and command in _PLANNED_REPLAY_COMMAND_SET:
        owner_segment = '--owner "Pavel Obruchnikov <me@inkyquill.net>"'
        if owner_segment not in command:
            return True
        try:
            tokens = shlex.split(command, posix=True)
        except ValueError:
            return False
        owner_index = tokens.index("--owner")
        return tokens[owner_index + 1] == "Pavel Obruchnikov <me@inkyquill.net>"
    return False


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Validate a Rust qualification record")
    parser.add_argument("record", type=Path)
    arguments = parser.parse_args(argv)
    try:
        record = load_record(arguments.record)
    except (OSError, TypeError, ValueError):
        print("qualification record could not be loaded", file=sys.stderr)
        return 2

    try:
        issues = validate_record(record, Path.cwd())
    except (OSError, TypeError, ValueError):
        print("qualification record could not be validated", file=sys.stderr)
        return 2
    if issues:
        print("qualification record is invalid", file=sys.stderr)
        for issue in issues:
            print(issue, file=sys.stderr)
        return 1
    print("qualification record is valid")
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
