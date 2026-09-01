"""Deterministic validation gate for one qualification record."""

# ruff: noqa: E501 -- canonical replay commands are deliberately byte-exact literals.

from __future__ import annotations

import argparse
import re
import shlex
import sys
from collections.abc import Sequence
from pathlib import Path, PurePosixPath

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

_REPLAY_TOKEN = re.compile(r"^[A-Za-z0-9_./:+,=-]+$")
_RISKS = tuple(REQUIRED_CRITERIA)
_PROJECTION_RISKS: tuple[ProjectionRisk, ...] = (
    "mcp-transport",
    "legacy-database-import",
)
_LIVE_MODULES = {
    "tools.qualification.run",
    "tools.qualification.run_mcp",
    "tools.qualification.run_semantic",
    "tools.qualification.run_frontend",
    "tools.qualification.run_database",
}


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


def _legacy_replay_command_is_safe(command: object) -> bool:
    if type(command) is not str or not command or command.strip() != command:
        return False
    if "  " in command:
        return False
    tokens = command.split(" ")
    if any(not token or _REPLAY_TOKEN.fullmatch(token) is None for token in tokens):
        return False
    if any(re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:(?://|/)", token) for token in tokens):
        return False

    assignments, body = _leading_assignments(tokens)
    if assignments is None or not body:
        return False
    if body[:2] == ["uv", "run"]:
        return _uv_command_is_safe(body, assignments)
    if body[:2] == ["cargo", "+1.96.0"]:
        return _cargo_command_is_safe(body, assignments)
    if body[0] == "unshare":
        return not assignments and body == [
            "unshare",
            "--user",
            "--map-root-user",
            "--net",
            "--",
            "bun",
            "run",
            "--cwd",
            "frontend",
            "build",
            "--",
            "--outDir",
            "qualification/.artifacts/frontend-dist/current",
            "--emptyOutDir",
        ]
    return False


def _leading_assignments(
    tokens: list[str],
) -> tuple[dict[str, str], list[str]] | tuple[None, list[str]]:
    allowed = (
        ("HIERONYMUS_QUALIFICATION_LIVE", "1"),
        ("CARGO_TARGET_DIR", None),
        ("CARGO_NET_OFFLINE", "true"),
    )
    assignments: dict[str, str] = {}
    previous_index = -1
    index = 0
    while index < len(tokens) and "=" in tokens[index] and not tokens[index].startswith("--"):
        name, value = tokens[index].split("=", 1)
        matching = next(
            (
                (allowed_index, expected)
                for allowed_index, (allowed_name, expected) in enumerate(allowed)
                if name == allowed_name
            ),
            None,
        )
        if matching is None or name in assignments:
            return None, []
        allowed_index, expected = matching
        if allowed_index <= previous_index or (expected is not None and value != expected):
            return None, []
        if name == "CARGO_TARGET_DIR" and not _cargo_target_value_is_safe(value):
            return None, []
        assignments[name] = value
        previous_index = allowed_index
        index += 1
    return assignments, tokens[index:]


def _cargo_target_value_is_safe(value: str) -> bool:
    prefix = "qualification/.artifacts/cargo-target/"
    return value.startswith(prefix) and value[len(prefix) :] in _RISKS


def _uv_command_is_safe(body: list[str], assignments: dict[str, str]) -> bool:
    index = 2
    if index < len(body) and body[index] == "--no-cache":
        index += 1
    if index < len(body) and body[index] == "--no-sync":
        index += 1
    if index >= len(body) or body[index] != "python":
        return False
    index += 1
    if index < len(body) and body[index] == "-B":
        index += 1
    if index + 1 >= len(body) or body[index] != "-m":
        return False
    module = body[index + 1]
    arguments = body[index + 2 :]
    if module not in {
        "tools.qualification.validate",
        "tools.qualification.render",
        "tools.qualification.check",
        *_LIVE_MODULES,
    }:
        return False

    is_live = module in _LIVE_MODULES
    if is_live:
        if assignments.get("HIERONYMUS_QUALIFICATION_LIVE") != "1":
            return False
        if assignments.get("CARGO_NET_OFFLINE") != "true":
            return False
    elif assignments:
        return False
    if not _module_arguments_are_safe(module, arguments):
        return False

    target_dir = assignments.get("CARGO_TARGET_DIR")
    if target_dir is None:
        return True
    target_risk = target_dir.rsplit("/", 1)[-1]
    expected_risk = {
        "tools.qualification.run_mcp": "mcp-transport",
        "tools.qualification.run_semantic": "semantic-native",
        "tools.qualification.run_frontend": "frontend-embedding",
        "tools.qualification.run_database": "legacy-database-import",
    }.get(module)
    return expected_risk is None or target_risk == expected_risk


def _module_arguments_are_safe(module: str, arguments: list[str]) -> bool:
    if module == "tools.qualification.validate":
        return len(arguments) == 1 and _record_path_risk(arguments[0]) is not None
    if module == "tools.qualification.render":
        if len(arguments) != 3 or arguments[0] != "--check":
            return False
        risk = _record_path_risk(arguments[1])
        return risk is not None and arguments[2] == f"docs/qualification/rust/{risk}.md"
    if module == "tools.qualification.check":
        return _check_arguments_are_safe(arguments)
    if module == "tools.qualification.run":
        return (
            len(arguments) == 2 and arguments[0] in (*_RISKS, "all") and arguments[1] == "--write"
        )
    if module in {
        "tools.qualification.run_mcp",
        "tools.qualification.run_semantic",
        "tools.qualification.run_frontend",
        "tools.qualification.run_database",
    }:
        return arguments == ["--write"]
    return False


def _record_path_risk(value: str) -> str | None:
    for risk in _RISKS:
        if value == f"qualification/records/{risk}.json":
            return risk
    return None


def _check_arguments_are_safe(arguments: list[str]) -> bool:
    index = 0
    if index < len(arguments) and arguments[index] == "--record":
        if index + 1 >= len(arguments) or arguments[index + 1] not in _RISKS:
            return False
        index += 2
    if index < len(arguments) and arguments[index] == "--records-only":
        index += 1
    if index < len(arguments) and arguments[index] == "--require-qualified":
        index += 1
    return index == len(arguments)


def _cargo_command_is_safe(body: list[str], assignments: dict[str, str]) -> bool:
    if set(assignments) != {"CARGO_TARGET_DIR", "CARGO_NET_OFFLINE"}:
        return False
    if len(body) < 4:
        return False
    subcommand = body[2]
    if subcommand not in {"check", "build", "test", "metadata", "tree", "fmt", "clippy"}:
        return False
    arguments = body[3:]
    manifest: str | None = None
    seen_single: set[str] = set()
    clippy_tail = False
    index = 0
    while index < len(arguments):
        token = arguments[index]
        if token == "--":
            if subcommand != "clippy" or arguments[index + 1 :] != ["-D", "warnings"]:
                return False
            clippy_tail = True
            break
        if token == "--manifest-path":
            if token in seen_single or index + 1 >= len(arguments):
                return False
            seen_single.add(token)
            manifest = arguments[index + 1]
            index += 2
            continue
        if token in {"--target", "--features", "--format-version", "-e", "--test"}:
            if token != "--test" and token in seen_single:
                return False
            if index + 1 >= len(arguments):
                return False
            value = arguments[index + 1]
            expected_values = {
                "--target": {"x86_64-unknown-linux-gnu"},
                "--features": {"semantic-native"},
                "--format-version": {"1"},
                "-e": {"features"},
                "--test": {"transport", "corpus", "index", "recovery", "fts", "assets", "fixtures"},
            }
            if value not in expected_values[token]:
                return False
            seen_single.add(token)
            index += 2
            continue
        if (
            token
            not in {
                "--locked",
                "--release",
                "--no-default-features",
                "--all-targets",
                "--offline",
                "--no-deps",
                "--check",
            }
            or token in seen_single
        ):
            return False
        seen_single.add(token)
        index += 1

    allowed_flags = {
        "check": {
            "--manifest-path",
            "--locked",
            "--target",
            "--release",
            "--no-default-features",
            "--features",
        },
        "build": {"--manifest-path", "--locked", "--target", "--release"},
        "test": {
            "--manifest-path",
            "--locked",
            "--target",
            "--release",
            "--no-default-features",
            "--features",
            "--test",
        },
        "metadata": {
            "--manifest-path",
            "--locked",
            "--offline",
            "--no-deps",
            "--format-version",
        },
        "tree": {"--manifest-path", "--locked", "-e"},
        "fmt": {"--manifest-path", "--check"},
        "clippy": {
            "--manifest-path",
            "--locked",
            "--target",
            "--release",
            "--no-default-features",
            "--features",
            "--all-targets",
        },
    }
    if not seen_single.issubset(allowed_flags[subcommand]):
        return False
    if manifest is None or not _canonical_manifest_path(manifest):
        return False
    risk = manifest.split("/")[2]
    if assignments["CARGO_TARGET_DIR"].rsplit("/", 1)[-1] != risk:
        return False
    if subcommand in {"check", "build", "test", "clippy"} and "--target" not in seen_single:
        return False
    if subcommand != "fmt" and "--locked" not in seen_single:
        return False
    if subcommand == "clippy" and not clippy_tail:
        return False
    return True


def _canonical_manifest_path(value: str) -> bool:
    path = PurePosixPath(value)
    return (
        path.as_posix() == value
        and not path.is_absolute()
        and ".." not in path.parts
        and len(path.parts) == 4
        and path.parts[0] == "qualification"
        and path.parts[1] == "harnesses"
        and path.parts[2] in _RISKS
        and path.parts[3] == "Cargo.toml"
    )


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
