"""Security and record-gate tests for qualification evidence."""

from __future__ import annotations

import os
from dataclasses import fields, replace
from pathlib import Path

import pytest
from factories import make_record

from tools.qualification.fingerprint import COMMON_FINGERPRINT_INPUTS, fingerprint_inputs
from tools.qualification.model import QualificationRecord, serialize_record
from tools.qualification.redaction import redaction_issues
from tools.qualification.validate import validate_record


def _seed_common_inputs(root: Path) -> None:
    for relative in COMMON_FINGERPRINT_INPUTS:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"qualification fixture: {relative}\n", encoding="utf-8")


def _unchecked_record(
    record: QualificationRecord,
    **updates: object,
) -> QualificationRecord:
    copied = object.__new__(QualificationRecord)
    for field in fields(record):
        object.__setattr__(
            copied,
            field.name,
            updates.get(field.name, getattr(record, field.name)),
        )
    return copied


def test_record_rejects_secret_and_home_path(tmp_path: Path) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(tmp_path, "frontend-embedding")
    leaked = replace(
        record,
        commands=(*record.commands, "compat-secret-do-not-log /home/alice/private"),
    )
    serialized = serialize_record(leaked)

    assert redaction_issues(serialized) == [
        "record contains forbidden literal: compat-secret-do-not-log",
        "record contains an absolute home path",
    ]
    assert validate_record(leaked, tmp_path) == redaction_issues(serialized)


@pytest.mark.parametrize(
    ("json_value", "markdown_value", "issue"),
    [
        (
            '"authorization":"Bearer abc.def.ghi"',
            "| Authorization | Bearer abc.def.ghi |",
            "record contains authorization material",
        ),
        (
            '"cookie":"session=private-value"',
            "| Cookie | session=private-value |",
            "record contains cookie material",
        ),
        (
            '"access_token":"ghp_1234567890abcdefghijklmnop"',
            "| access_token | ghp_1234567890abcdefghijklmnop |",
            "record contains a token or secret value",
        ),
        (
            '"provider_key":"sk-provider-private-value"',
            "| provider_key | sk-provider-private-value |",
            "record contains a provider key",
        ),
        (
            '"command":"OPENAI_API_KEY=sk-provider-private-value"',
            "OPENAI_API_KEY=sk-provider-private-value",
            "record contains a provider key",
        ),
        (
            '"command":"CLIENT_SECRET=private-value"',
            "CLIENT_SECRET=private-value",
            "record contains a token or secret value",
        ),
        (
            '"source_text":"A private source row"',
            "| source_text | A private source row |",
            "record contains source-row text",
        ),
        (
            '"hostname":"translator-workstation"',
            "| hostname | translator-workstation |",
            "record contains a hostname",
        ),
        (
            '"command":"HOSTNAME=translator-workstation"',
            "HOSTNAME=translator-workstation",
            "record contains a hostname",
        ),
        (
            '"username":"alice"',
            "| username | alice |",
            "record contains a username",
        ),
        (
            '"command":"USER=alice"',
            "USER=alice",
            "record contains a username",
        ),
        (
            '"path":"C:\\\\Users\\\\Alice\\\\private\\\\record.json"',
            "C:\\Users\\Alice\\private\\record.json",
            "record contains an absolute home path",
        ),
        (
            '"path":"/Users/alice/private/record.json"',
            "/Users/alice/private/record.json",
            "record contains an absolute home path",
        ),
        (
            '"path":"/home/alice"',
            "/home/alice",
            "record contains an absolute home path",
        ),
        (
            '"path":"/root"',
            "/root",
            "record contains an absolute home path",
        ),
        (
            '"path":"C:\\\\Users\\\\Alice"',
            "C:\\Users\\Alice",
            "record contains an absolute home path",
        ),
        (
            '"summary":"stderr: raw child output"',
            "stderr: raw child output",
            "record contains raw process output",
        ),
    ],
)
def test_json_and_markdown_use_the_same_sensitive_data_rules(
    json_value: str,
    markdown_value: str,
    issue: str,
) -> None:
    assert redaction_issues("{" + json_value + "}") == [issue]
    assert redaction_issues(markdown_value) == [issue]


def test_redaction_avoids_hash_and_ordinary_prose_false_positives() -> None:
    sha256 = "a3" * 32
    ordinary = (
        f"Digest {sha256}. Token budgets, cookie recipes, host applications, "
        "and user-facing prose are ordinary documentation."
    )

    assert redaction_issues(ordinary) == []


def test_redaction_reports_all_matches_once_in_rule_order() -> None:
    serialized = (
        '"username":"alice" Cookie: private '
        "compat-secret-do-not-log Authorization: Bearer abc.def.ghi "
        '"username":"alice" /home/alice/private'
    )

    assert redaction_issues(serialized) == [
        "record contains forbidden literal: compat-secret-do-not-log",
        "record contains authorization material",
        "record contains cookie material",
        "record contains a username",
        "record contains an absolute home path",
    ]


def test_validation_accepts_exact_common_prefix_and_ordered_risk_suffix(
    tmp_path: Path,
) -> None:
    _seed_common_inputs(tmp_path)
    suffix = (
        "qualification/fixtures/frontend-manifest.json",
        "tools/qualification/run_frontend.py",
    )
    for relative in suffix:
        path = tmp_path / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(relative, encoding="utf-8")
    input_paths = (*COMMON_FINGERPRINT_INPUTS, *suffix)
    record = replace(
        make_record(tmp_path, "frontend-embedding"),
        input_paths=input_paths,
        input_digest=fingerprint_inputs(tmp_path, input_paths),
    )

    assert validate_record(record, tmp_path) == []

    reordered = (COMMON_FINGERPRINT_INPUTS[1], COMMON_FINGERPRINT_INPUTS[0], *input_paths[2:])
    assert validate_record(
        replace(record, input_paths=reordered),
        tmp_path,
    ) == ["record input_paths must start with the exact common fingerprint inputs"]


def test_validation_rejects_stale_digest_after_input_change(tmp_path: Path) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(tmp_path, "semantic-native")
    (tmp_path / COMMON_FINGERPRINT_INPUTS[0]).write_text("changed", encoding="utf-8")

    assert validate_record(record, tmp_path) == ["record input digest is stale"]


def test_validation_reports_every_cleanup_and_command_issue_deterministically(
    tmp_path: Path,
) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(tmp_path, "legacy-database-import")
    invalid = replace(
        record,
        commands=("", "qualification replay", "qualification replay", "/opt/probe"),
        cleanup=replace(
            record.cleanup,
            work_dir_removed=False,
            raw_logs_removed=False,
            source_inputs_unchanged=False,
            user_data_opened=True,
        ),
    )

    issues = validate_record(invalid, tmp_path)
    assert issues == validate_record(invalid, tmp_path)
    assert issues == [
        "record commands must be nonempty single-line relative commands",
        "record commands must not contain duplicates",
        "record cleanup work_dir_removed must be true",
        "record cleanup raw_logs_removed must be true",
        "record cleanup source_inputs_unchanged must be true",
        "record cleanup user_data_opened must be false",
    ]


def test_validation_reports_cross_field_issues_without_trusting_forged_records(
    tmp_path: Path,
) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(tmp_path, "mcp-transport")
    evidence = (record.evidence[1], record.evidence[0], *record.evidence[2:])
    forged = _unchecked_record(
        record,
        evidence=evidence,
        decision="blocked",
        consequence="mutable consequence",
    )

    issues = validate_record(forged, tmp_path)
    assert issues == validate_record(forged, tmp_path)
    assert "record violates the strict typed/schema boundary" in issues
    assert (
        "record criterion evidence must exactly match the ordered required criteria "
        "for mcp-transport"
    ) in issues
    assert "record decision must be qualified for mcp-transport pass evidence" in issues
    assert ("record consequence must match the immutable mcp-transport pass consequence") in issues


def test_validation_never_probes_home_or_environment(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(tmp_path, "mcp-transport")
    monkeypatch.setenv("HOME", "/home/compat-secret-do-not-log")
    monkeypatch.setenv("USER", "private-user")
    monkeypatch.setattr(
        Path,
        "home",
        classmethod(lambda cls: (_ for _ in ()).throw(AssertionError())),
    )

    assert validate_record(record, tmp_path) == []
    assert os.environ["HOME"] == "/home/compat-secret-do-not-log"
