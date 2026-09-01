"""Security and record-gate tests for qualification evidence."""

from __future__ import annotations

import os
from dataclasses import fields, replace
from pathlib import Path

import pytest
from factories import make_record, seed_fingerprint_inputs

from tools.qualification.fingerprint import (
    COMMON_FINGERPRINT_INPUTS,
    MCP_TOOL_INPUT_WIRE_INPUTS,
    RISK_FINGERPRINT_SUFFIXES,
    fingerprint_inputs,
    required_fingerprint_inputs,
)
from tools.qualification.model import (
    REQUIRED_CRITERIA,
    Measurements,
    QualificationRecord,
    Risk,
    serialize_record,
)
from tools.qualification.redaction import redaction_issues
from tools.qualification.validate import validate_record


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
    seed_fingerprint_inputs(tmp_path, "frontend-embedding")
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
    assert validate_record(leaked, tmp_path) == [
        *redaction_issues(serialized),
        "record commands must be safe relative replay commands",
    ]


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
            '"path":"C:\\\\Documents and Settings\\\\Alice\\\\private"',
            "C:\\Documents and Settings\\Alice\\private",
            "record contains an absolute home path",
        ),
        (
            '"summary":"stderr: raw child output"',
            "stderr: raw child output",
            "record contains raw process output",
        ),
        (
            '"summary":"sk-proj-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG"',
            "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG",
            "record contains a provider key",
        ),
        (
            '"summary":"AKIAIOSFODNN7EXAMPLE"',
            "AKIAIOSFODNN7EXAMPLE",
            "record contains a provider key",
        ),
        (
            '"summary":"Authorization: Basic dXNlcjpwYXNz"',
            "Authorization: Basic dXNlcjpwYXNz",
            "record contains authorization material",
        ),
        (
            '"summary":"Authorization: Digest opaque-credential"',
            "Authorization: Digest opaque-credential",
            "record contains authorization material",
        ),
        (
            '"apiKey":"private-api-value"',
            "| apiKey | private-api-value |",
            "record contains a token or secret value",
        ),
        (
            '"sourceText":"A private source row"',
            "| sourceText | A private source row |",
            "record contains source-row text",
        ),
        (
            '"launch_grant":"private-grant"',
            "| launch_grant | private-grant |",
            "record contains a token or secret value",
        ),
        (
            '"command":"HOST=translator-workstation"',
            "HOST=translator-workstation",
            "record contains a hostname",
        ),
        (
            '"command":"LOGNAME=alice"',
            "LOGNAME=alice",
            "record contains a username",
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


@pytest.mark.parametrize(
    "safe",
    [
        "Cookie: enabled for the synthetic route fixture",
        "stderr: none",
        "stdout: <absent>",
        "Authorization: none",
        "HOST support is checked without recording its value",
        "The apiKey field is documented but never serialized",
    ],
)
def test_redaction_allows_sanitized_status_prose(safe: str) -> None:
    assert redaction_issues(safe) == []


@pytest.mark.parametrize(
    "leak",
    [
        "Authorization: none private-credential",
        "stderr: none raw child output",
    ],
)
def test_redaction_sentinels_do_not_mask_following_sensitive_values(leak: str) -> None:
    assert redaction_issues(leak)


def test_redaction_diagnostics_never_echo_matched_values() -> None:
    secrets = (
        "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG",
        "AKIAIOSFODNN7EXAMPLE",
        "dXNlcjpwYXNz",
    )
    issues = redaction_issues(f"{secrets[0]} {secrets[1]} Authorization: Basic {secrets[2]}")

    assert issues
    assert all(secret not in issue for issue in issues for secret in secrets)


def test_redaction_reports_all_matches_once_in_rule_order() -> None:
    serialized = (
        '"username":"alice" Cookie: session=private '
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


@pytest.mark.parametrize(
    ("name", "issue"),
    [
        ("password", "record contains a token or secret value"),
        ("pass_word", "record contains a token or secret value"),
        ("passWord", "record contains a token or secret value"),
        ("private_key", "record contains a token or secret value"),
        ("privateKey", "record contains a token or secret value"),
        ("auth", "record contains authorization material"),
        ("auth_header", "record contains authorization material"),
        ("authHeader", "record contains authorization material"),
        ("authorization", "record contains authorization material"),
        ("authorization_header", "record contains authorization material"),
        ("authorizationHeader", "record contains authorization material"),
        ("proxy_authorization", "record contains authorization material"),
        ("proxyAuthorization", "record contains authorization material"),
        ("cookie", "record contains cookie material"),
        ("cookie_header", "record contains cookie material"),
        ("cookieHeader", "record contains cookie material"),
        ("set_cookie", "record contains cookie material"),
        ("setCookie", "record contains cookie material"),
    ],
)
def test_redaction_rejects_every_structured_secret_name_variant(
    tmp_path: Path,
    name: str,
    issue: str,
) -> None:
    seed_fingerprint_inputs(tmp_path, "frontend-embedding")
    record = make_record(tmp_path, "frontend-embedding")
    evidence = replace(
        record.evidence[0],
        measurements=Measurements({name: "private-value"}),
    )
    leaked = replace(record, evidence=(evidence, *record.evidence[1:]))

    issues = validate_record(leaked, tmp_path)

    assert issue in issues
    assert all("private-value" not in diagnostic for diagnostic in issues)


@pytest.mark.parametrize(
    "encoded_home",
    [
        "%2Fhome%2Falice%2Fprivate",
        "%2FUsers%2Falice%2Fprivate",
        "%2Froot%2Fprivate",
        "C%3A%5CUsers%5CAlice%5Cprivate",
        "c%3a%5cdocuments%20and%20settings%5calice%5cprivate",
    ],
)
def test_redaction_rejects_one_case_insensitive_percent_decoded_home_view(
    encoded_home: str,
) -> None:
    issues = redaction_issues(f'{{"artifact":"{encoded_home}"}}')

    assert issues == ["record contains an absolute home path"]
    assert encoded_home not in issues[0]


@pytest.mark.parametrize("risk", tuple(REQUIRED_CRITERIA))
def test_validation_accepts_only_exact_ordered_risk_input_policy(
    tmp_path: Path,
    risk: Risk,
) -> None:
    input_paths = seed_fingerprint_inputs(tmp_path, risk)
    record = make_record(tmp_path, risk)
    assert record.input_paths == input_paths
    assert validate_record(record, tmp_path) == []

    variants = (
        input_paths[:-1],
        (*input_paths, "unrelated.txt"),
        (*input_paths, input_paths[-1]),
        (*input_paths[:-2], input_paths[-1], input_paths[-2]),
    )
    (tmp_path / "unrelated.txt").write_text("unrelated", encoding="utf-8")
    for variant in variants:
        issues = validate_record(
            replace(
                record,
                input_paths=variant,
                input_digest=(
                    fingerprint_inputs(tmp_path, variant)
                    if len(variant) == len(set(variant))
                    else record.input_digest
                ),
            ),
            tmp_path,
        )
        assert issues[0] == f"record input_paths must exactly match the {risk} fingerprint policy"


def test_validation_accepts_exact_common_prefix_and_ordered_risk_suffix(
    tmp_path: Path,
) -> None:
    """Preserve the original Task 3 acceptance node against the stricter policy."""
    seed_fingerprint_inputs(tmp_path, "frontend-embedding")

    assert validate_record(make_record(tmp_path, "frontend-embedding"), tmp_path) == []


def test_risk_input_policy_covers_every_planned_runner_harness_and_fixture() -> None:
    assert len(MCP_TOOL_INPUT_WIRE_INPUTS) == 156
    assert len(set(MCP_TOOL_INPUT_WIRE_INPUTS)) == 156
    assert RISK_FINGERPRINT_SUFFIXES["mcp-transport"][:7] == (
        "tools/qualification/run_mcp.py",
        "qualification/harnesses/mcp-transport/Cargo.toml",
        "qualification/harnesses/mcp-transport/Cargo.lock",
        "qualification/harnesses/mcp-transport/src/main.rs",
        "qualification/harnesses/mcp-transport/src/registry.rs",
        "qualification/harnesses/mcp-transport/src/report.rs",
        "qualification/harnesses/mcp-transport/tests/transport.rs",
    )
    assert RISK_FINGERPRINT_SUFFIXES["semantic-native"][-3:] == (
        "qualification/fixtures/semantic-corpus.json",
        "compatibility/fixtures/mcp/tools/hieronymus_rag_search/success.input.json",
        "compatibility/fixtures/mcp/tools/hieronymus_recall/success.input.json",
    )
    assert "frontend/index.html" in RISK_FINGERPRINT_SUFFIXES["frontend-embedding"]
    assert "frontend/src/web/main.ts" in RISK_FINGERPRINT_SUFFIXES["frontend-embedding"]
    assert RISK_FINGERPRINT_SUFFIXES["frontend-embedding"][-1] == (
        "compatibility/fixtures/http/route-cases.json"
    )
    assert RISK_FINGERPRINT_SUFFIXES["legacy-database-import"][-6:] == (
        "compatibility/fixtures/database/corrupt.sqlite",
        "compatibility/fixtures/database/empty.sqlite",
        "compatibility/fixtures/database/legacy-python.sqlite",
        "compatibility/fixtures/database/minimal-python.sqlite",
        "compatibility/fixtures/database/partial-python.sqlite",
        "compatibility/fixtures/database/unknown-schema.sqlite",
    )
    assert tuple(len(required_fingerprint_inputs(risk)) for risk in REQUIRED_CRITERIA) == (
        180,
        28,
        47,
        26,
    )
    assert "tools/qualification/projections.py" in COMMON_FINGERPRINT_INPUTS
    assert (
        "qualification/compatibility/mcp-transport.json"
        in RISK_FINGERPRINT_SUFFIXES["mcp-transport"]
    )
    assert (
        "qualification/compatibility/legacy-database-import.json"
        in RISK_FINGERPRINT_SUFFIXES["legacy-database-import"]
    )
    for excluded in (
        "compatibility/manifest.json",
        "compatibility/snapshots/state.json",
    ):
        assert all(excluded not in suffix for suffix in RISK_FINGERPRINT_SUFFIXES.values())
    for risk in REQUIRED_CRITERIA:
        assert required_fingerprint_inputs(risk) == (
            *COMMON_FINGERPRINT_INPUTS,
            *RISK_FINGERPRINT_SUFFIXES[risk],
        )


def test_validation_rejects_stale_digest_after_input_change(tmp_path: Path) -> None:
    seed_fingerprint_inputs(tmp_path, "semantic-native")
    record = make_record(tmp_path, "semantic-native")
    (tmp_path / COMMON_FINGERPRINT_INPUTS[0]).write_text("changed", encoding="utf-8")

    assert validate_record(record, tmp_path) == ["record input digest is stale"]


def test_validation_reports_every_cleanup_and_command_issue_deterministically(
    tmp_path: Path,
) -> None:
    seed_fingerprint_inputs(tmp_path, "legacy-database-import")
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
        "record commands must be safe relative replay commands",
        "record commands must not contain duplicates",
        "record cleanup work_dir_removed must be true",
        "record cleanup raw_logs_removed must be true",
        "record cleanup source_inputs_unchanged must be true",
        "record cleanup user_data_opened must be false",
    ]


def test_validation_reports_cross_field_issues_without_trusting_forged_records(
    tmp_path: Path,
) -> None:
    seed_fingerprint_inputs(tmp_path, "mcp-transport")
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
    seed_fingerprint_inputs(tmp_path, "mcp-transport")
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


@pytest.mark.parametrize(
    "command",
    [
        "../probe",
        "uv run probe nested/../fixture",
        r"uv run probe nested\..\fixture",
        "uv run probe /etc/passwd",
        "uv run probe --input=/etc/passwd",
        r"uv run probe C:\Users\Alice\private",
        r"uv run probe \Users\Alice\private",
        r"uv run probe \\server\share\private",
        "uv run probe ~/private",
        "uv run probe $HOME/private",
        r"uv run probe %USERPROFILE%\private",
        "```",
        "uv run probe | # injected",
        "uv run probe <script>alert(1)</script>",
        "uv run probe\n# injected",
        "uv run probe\r| forged | row |",
        "uv run probe\x00hidden",
        "uv run probe $OLDPWD/script",
        "uv run probe ${PWD}/script",
        "uv run probe ${INPUT:-/etc/passwd}",
        "uv run probe *.json",
        "uv run probe fixture?.json",
        "uv run probe fixture[0].json",
        "uv run probe {first,second}",
        "uv run probe $(id)",
        "uv run probe >artifact",
        "uv run probe 2>artifact",
        "uv run probe (nested)",
        "uv run probe 'quoted'",
        'uv run probe "quoted"',
        "uv run probe !history",
        "uv run probe #comment",
        "curl https://example.invalid",
        "wget example.invalid/file",
        "cargo +1.96.0 fetch --manifest-path qualification/harnesses/mcp-transport/Cargo.toml",
        "bun install --cwd frontend",
        "uv sync",
        "uv run python -m tools.qualification.validate https://example.invalid/record.json",
        "qualification mcp-transport",
    ],
)
def test_validation_rejects_unsafe_replay_command_tokens(
    tmp_path: Path,
    command: str,
) -> None:
    seed_fingerprint_inputs(tmp_path, "mcp-transport")
    record = replace(make_record(tmp_path, "mcp-transport"), commands=(command,))

    assert "record commands must be safe relative replay commands" in validate_record(
        record,
        tmp_path,
    )


def test_validation_allows_exact_planned_relative_replay_commands(tmp_path: Path) -> None:
    seed_fingerprint_inputs(tmp_path, "mcp-transport")
    commands = (
        "uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only",
        "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport "
        "CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path "
        "qualification/harnesses/mcp-transport/Cargo.toml --locked "
        "--target x86_64-unknown-linux-gnu",
        "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_NET_OFFLINE=true "
        "uv run python -m tools.qualification.run all --write",
        "uv run python -m tools.qualification.validate qualification/records/mcp-transport.json",
        "uv run python -m tools.qualification.render --check "
        "qualification/records/mcp-transport.json "
        "docs/qualification/rust/mcp-transport.md",
        "unshare --user --map-root-user --net -- bun run --cwd frontend build -- "
        "--outDir qualification/.artifacts/frontend-dist/current --emptyOutDir",
    )
    record = replace(make_record(tmp_path, "mcp-transport"), commands=commands)

    assert validate_record(record, tmp_path) == []
