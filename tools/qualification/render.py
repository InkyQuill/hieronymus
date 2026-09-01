"""Byte-deterministic Markdown rendering for qualification records."""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from pathlib import Path

from tools.qualification.model import QualificationRecord, load_record, serialize_record
from tools.qualification.redaction import redaction_issues
from tools.qualification.validate import replay_commands_are_safe, validate_record

_TITLES = {
    "mcp-transport": "MCP Transport",
    "semantic-native": "Semantic Native",
    "frontend-embedding": "Frontend Embedding",
    "legacy-database-import": "Legacy Database Import",
}


def render_record(record: QualificationRecord) -> str:
    """Render one strictly serialized, redaction-clean record as stable Markdown."""
    serialized = serialize_record(record)
    issues = redaction_issues(serialized)
    if issues:
        raise ValueError("; ".join(issues))
    if not replay_commands_are_safe(record.commands):
        raise ValueError("record commands must be safe relative replay commands")

    lines = [f"# {_TITLES[record.risk]} Qualification Record", ""]
    lines.extend(
        _key_value_section(
            "Decision",
            (
                ("Risk", record.risk),
                ("Status", record.status),
                ("Decision", record.decision),
                ("Target", record.target),
                ("Acceptance owner", record.acceptance_owner),
            ),
        )
    )
    lines.extend(["## Normative Specifications", ""])
    lines.extend(_bullet_values(sorted(record.specs)))
    lines.extend(
        [
            "## Replay Commands",
            "",
            "| Order | Command |",
            "| --- | --- |",
        ]
    )
    lines.extend(
        f"| {index} | {_cell(command)} |" for index, command in enumerate(record.commands, start=1)
    )
    lines.append("")
    lines.extend(
        _key_value_section(
            "Environment",
            (
                ("Architecture", record.environment.architecture),
                ("Bun", record.environment.bun or "(none)"),
                ("Cargo", record.environment.cargo),
                ("Kernel", record.environment.kernel),
                (
                    "Native libraries",
                    ", ".join(sorted(record.environment.native_libraries)) or "(none)",
                ),
                ("Operating system", record.environment.os),
                ("Rust compiler", record.environment.rustc),
                ("Target", record.environment.target),
            ),
        )
    )
    lines.extend(
        [
            "## Locked Dependencies",
            "",
            "| Name | Version | Source | Checksum | Features |",
            "| --- | --- | --- | --- | --- |",
        ]
    )
    if record.dependencies:
        for dependency in sorted(
            record.dependencies,
            key=lambda item: (
                item.name,
                item.version,
                item.source,
                item.checksum is not None,
                item.checksum or "",
                tuple(sorted(item.features)),
            ),
        ):
            lines.append(
                "| "
                + " | ".join(
                    _cell(value)
                    for value in (
                        dependency.name,
                        dependency.version,
                        dependency.source,
                        dependency.checksum or "(none)",
                        ", ".join(sorted(dependency.features)) or "(none)",
                    )
                )
                + " |"
            )
    else:
        lines.append("| (none) | (none) | (none) | (none) | (none) |")
    lines.append("")

    lines.extend(
        [
            "## Required Criteria",
            "",
            "| Criterion | Status | Summary | Measurements | Not-run reason |",
            "| --- | --- | --- | --- | --- |",
        ]
    )
    for evidence in record.evidence:
        measurements = json.dumps(
            dict(evidence.measurements),
            allow_nan=False,
            sort_keys=True,
            separators=(",", ":"),
        )
        lines.append(
            "| "
            + " | ".join(
                _cell(value)
                for value in (
                    evidence.criterion,
                    evidence.status,
                    evidence.summary,
                    measurements,
                    evidence.not_run_reason or "(none)",
                )
            )
            + " |"
        )
    lines.append("")

    lines.extend(["## Consumed Compatibility Contracts", ""])
    lines.extend(_bullet_values(sorted(record.contract_ids)))
    lines.extend(["## Input Fingerprint", "", f"SHA-256: `{record.input_digest}`", ""])
    lines.extend(["Input paths:", ""])
    lines.extend(_bullet_values(record.input_paths))
    lines.extend(
        _key_value_section(
            "Cleanup Assertions",
            (
                ("work_dir_removed", record.cleanup.work_dir_removed),
                ("raw_logs_removed", record.cleanup.raw_logs_removed),
                ("install_dir_removed", record.cleanup.install_dir_removed),
                ("source_inputs_unchanged", record.cleanup.source_inputs_unchanged),
                ("user_data_opened", record.cleanup.user_data_opened),
                ("core_dumps_disabled", record.cleanup.core_dumps_disabled),
                (
                    "owned_process_groups_reaped",
                    record.cleanup.owned_process_groups_reaped,
                ),
            ),
        )
    )
    lines.extend(
        _key_value_section(
            "Review",
            (
                ("Owner", record.review.owner),
                ("Status", record.review.status),
                (
                    "Objective evidence reviewed",
                    record.review.objective_evidence_reviewed,
                ),
                (
                    "Normative constraints preserved",
                    record.review.normative_constraints_preserved,
                ),
            ),
        )
    )
    lines.extend(
        [
            "## Immutable Consequence",
            "",
            _cell(record.consequence or "(none)"),
            "",
        ]
    )
    rendered = "\n".join(lines)
    markdown_issues = redaction_issues(rendered)
    if markdown_issues:
        raise ValueError("; ".join(markdown_issues))
    return rendered


def _key_value_section(
    title: str,
    values: tuple[tuple[str, object], ...],
) -> list[str]:
    lines = [f"## {title}", "", "| Field | Value |", "| --- | --- |"]
    lines.extend(f"| {_cell(key)} | {_cell(value)} |" for key, value in values)
    lines.append("")
    return lines


def _bullet_values(values: Sequence[str]) -> list[str]:
    lines = [f"- {_cell(value)}" for value in values]
    lines.append("")
    return lines


def _literal_markdown(value: object) -> str:
    """Encode one arbitrary record value as inert Markdown literal text."""
    if type(value) is bool:
        rendered = str(value).lower()
    else:
        rendered = str(value)
    replacements = {
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        "|": "&#124;",
        "`": "&#96;",
        "\\": "&#92;",
        "!": "&#33;",
        "[": "&#91;",
        "]": "&#93;",
        "(": "&#40;",
        ")": "&#41;",
        "\r": "&#13;",
        "\n": "&#10;",
    }
    return "".join(
        replacements.get(character, f"&#{ord(character)};")
        if ord(character) < 32 or ord(character) == 127
        else replacements.get(character, character)
        for character in rendered
    )


def _cell(value: object) -> str:
    """Backward-compatible private alias for the one literal encoder."""
    return _literal_markdown(value)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Check rendered qualification Markdown")
    parser.add_argument("--check", action="store_true", required=True)
    parser.add_argument("record", type=Path)
    parser.add_argument("markdown", type=Path)
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
        return 2
    try:
        expected = render_record(record)
        actual = arguments.markdown.read_bytes()
    except (OSError, TypeError, ValueError):
        print("qualification Markdown could not be checked", file=sys.stderr)
        return 2
    if actual != expected.encode("utf-8"):
        print("qualification Markdown is stale", file=sys.stderr)
        return 1
    print("qualification Markdown is current")
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
