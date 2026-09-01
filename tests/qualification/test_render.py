"""Deterministic qualification rendering and CLI contract tests."""

from __future__ import annotations

import os
import subprocess
import sys
from dataclasses import replace
from pathlib import Path

import pytest
from factories import make_record

from tools.qualification.fingerprint import COMMON_FINGERPRINT_INPUTS
from tools.qualification.model import LockedDependency, Risk, serialize_record
from tools.qualification.render import render_record

ROOT = Path(__file__).resolve().parents[2]


def _seed_common_inputs(root: Path) -> None:
    for relative in COMMON_FINGERPRINT_INPUTS:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"qualification fixture: {relative}\n", encoding="utf-8")


def _cli_env() -> dict[str, str]:
    env = dict(os.environ)
    env["PYTHONPATH"] = str(ROOT)
    return env


def _write_record(root: Path, risk: Risk = "mcp-transport") -> tuple[Path, object]:
    _seed_common_inputs(root)
    record = make_record(root, risk)
    path = root / "record.json"
    path.write_text(serialize_record(record) + "\n", encoding="utf-8")
    return path, record


@pytest.mark.parametrize(
    ("risk", "title", "review_status"),
    [
        ("mcp-transport", "MCP Transport", "pending"),
        ("semantic-native", "Semantic Native", "accepted"),
        ("frontend-embedding", "Frontend Embedding", "rejected"),
        ("legacy-database-import", "Legacy Database Import", "pending"),
    ],
)
def test_render_is_deterministic_for_every_risk_and_review_state(
    tmp_path: Path,
    risk: Risk,
    title: str,
    review_status: str,
) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(tmp_path, risk, review_status=review_status)  # type: ignore[arg-type]

    first = render_record(record)

    assert first == render_record(record)
    assert first.startswith(f"# {title} Qualification Record\n")
    assert f"| Status | {review_status} |" in first
    assert first.endswith("\n")


def test_render_contains_every_required_section_and_record_value(tmp_path: Path) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(
        tmp_path,
        "legacy-database-import",
        failed=("source-byte-identity",),
    )
    rendered = render_record(record)

    headings = (
        "## Decision",
        "## Replay Commands",
        "## Environment",
        "## Locked Dependencies",
        "## Required Criteria",
        "## Consumed Compatibility Contracts",
        "## Input Fingerprint",
        "## Cleanup Assertions",
        "## Review",
        "## Immutable Consequence",
    )
    assert tuple(rendered.index(heading) for heading in headings) == tuple(
        sorted(rendered.index(heading) for heading in headings)
    )
    assert record.commands[0] in rendered
    assert record.contract_ids[0] in rendered
    assert record.input_digest in rendered
    assert record.consequence in rendered
    assert all(item.criterion in rendered for item in record.evidence)
    assert all(field in rendered for field in record.cleanup.__dataclass_fields__)


def test_render_sorts_dependency_rows_without_mutating_record_order(tmp_path: Path) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(tmp_path, "semantic-native")
    zeta = LockedDependency(
        name="zeta",
        version="2.0.0",
        source="registry+https://example.invalid/index",
        checksum=None,
        features=("feature-z", "feature-a"),
    )
    alpha = LockedDependency(
        name="alpha",
        version="1.0.0",
        source="registry+https://example.invalid/index",
        checksum="b" * 64,
        features=(),
    )
    record = replace(record, dependencies=(zeta, alpha))

    rendered = render_record(record)

    assert rendered.index("| alpha |") < rendered.index("| zeta |")
    assert record.dependencies == (zeta, alpha)
    assert "feature-a, feature-z" in rendered


def test_render_rejects_raw_logs_and_secrets_before_producing_markdown(
    tmp_path: Path,
) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(tmp_path, "frontend-embedding")
    leaked = replace(
        record,
        evidence=(
            replace(
                record.evidence[0],
                summary="stdout: compat-secret-do-not-log",
            ),
            *record.evidence[1:],
        ),
    )

    with pytest.raises(ValueError, match="record contains forbidden literal"):
        render_record(leaked)


def test_validate_cli_has_stable_success_validation_and_load_exit_codes(
    tmp_path: Path,
) -> None:
    record_path, record = _write_record(tmp_path)
    command = (sys.executable, "-m", "tools.qualification.validate", str(record_path))

    valid = subprocess.run(
        command,
        cwd=tmp_path,
        env=_cli_env(),
        check=False,
        capture_output=True,
        text=True,
    )
    assert (valid.returncode, valid.stdout, valid.stderr) == (
        0,
        "qualification record is valid\n",
        "",
    )

    leaked = replace(
        record,
        commands=(*record.commands, "compat-secret-do-not-log"),
    )
    record_path.write_text(serialize_record(leaked) + "\n", encoding="utf-8")
    invalid = subprocess.run(
        command,
        cwd=tmp_path,
        env=_cli_env(),
        check=False,
        capture_output=True,
        text=True,
    )
    assert (invalid.returncode, invalid.stdout, invalid.stderr) == (
        1,
        "",
        "qualification record is invalid\n"
        "record contains forbidden literal: compat-secret-do-not-log\n",
    )

    record_path.write_text('{"schema_version":1,"secret":"do-not-echo"}\n', encoding="utf-8")
    malformed = subprocess.run(
        command,
        cwd=tmp_path,
        env=_cli_env(),
        check=False,
        capture_output=True,
        text=True,
    )
    assert (malformed.returncode, malformed.stdout, malformed.stderr) == (
        2,
        "",
        "qualification record could not be loaded\n",
    )


def test_render_check_cli_is_read_only_and_rejects_invalid_records_before_output(
    tmp_path: Path,
) -> None:
    record_path, record = _write_record(tmp_path, "frontend-embedding")
    markdown_path = tmp_path / "record.md"
    markdown_path.write_text(render_record(record), encoding="utf-8")
    command = (
        sys.executable,
        "-m",
        "tools.qualification.render",
        "--check",
        str(record_path),
        str(markdown_path),
    )

    current = subprocess.run(
        command,
        cwd=tmp_path,
        env=_cli_env(),
        check=False,
        capture_output=True,
        text=True,
    )
    assert (current.returncode, current.stdout, current.stderr) == (
        0,
        "qualification Markdown is current\n",
        "",
    )

    markdown_path.write_text("stale\n", encoding="utf-8")
    stale = subprocess.run(
        command,
        cwd=tmp_path,
        env=_cli_env(),
        check=False,
        capture_output=True,
        text=True,
    )
    assert (stale.returncode, stale.stdout, stale.stderr) == (
        1,
        "",
        "qualification Markdown is stale\n",
    )
    assert markdown_path.read_text(encoding="utf-8") == "stale\n"

    leaked = replace(
        record,
        commands=(*record.commands, "Authorization: Bearer private-token"),
    )
    record_path.write_text(serialize_record(leaked) + "\n", encoding="utf-8")
    rejected = subprocess.run(
        command,
        cwd=tmp_path,
        env=_cli_env(),
        check=False,
        capture_output=True,
        text=True,
    )
    assert (rejected.returncode, rejected.stdout, rejected.stderr) == (
        2,
        "",
        "qualification record is invalid\nrecord contains authorization material\n",
    )
    assert markdown_path.read_text(encoding="utf-8") == "stale\n"
