"""Security and record-gate tests for qualification evidence."""

from __future__ import annotations

import json
import os
import re
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
from tools.qualification.validate import (
    PLANNED_REPLAY_COMMANDS,
    replay_commands_are_safe,
    validate_record,
)

ROOT = Path(__file__).resolve().parents[2]


def _task_section(plan: str, task: int) -> str:
    match = re.search(
        rf"^### Task {task}:.*?(?=^### Task |^## Self-Review Record|\Z)",
        plan,
        re.MULTILINE | re.DOTALL,
    )
    assert match is not None
    return match.group(0)


def _printed_replay_commands(section: str) -> set[str]:
    """Extract replay-like commands independently of the production allowlist."""
    candidates = set(re.findall(r"Run: `([^`]+)`", section))
    for block in re.findall(r"^```bash\n(.*?)^```$", section, re.MULTILINE | re.DOTALL):
        candidates.update(
            line.strip()
            for line in block.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        )

    qualification_modules = (
        "tools.qualification.run",
        "tools.qualification.validate",
        "tools.qualification.render",
        "tools.qualification.check",
        "tools.qualification.clean",
        "tools.qualification.projections",
        "tools.qualification.review",
    )
    cargo_replay_subcommands = {"metadata", "tree", "build", "fmt", "clippy"}
    extracted: set[str] = set()
    for command in candidates:
        if any(module in command for module in qualification_modules):
            if "pytest" not in command and "ruff" not in command:
                extracted.add(command)
            continue
        if " CARGO_NET_OFFLINE=true cargo +1.96.0 " in command:
            tokens = command.split(" ")
            cargo_index = tokens.index("cargo")
            if tokens[cargo_index + 2] in cargo_replay_subcommands:
                extracted.add(command)
            continue
        if command.startswith("unshare --user --map-root-user --net -- bun run "):
            extracted.add(command)
    return extracted


def _expected_replay_commands_from_plan(plan: str) -> set[str]:
    """Expand the canonical Task 3 matrix without consulting production data."""
    task_three = _task_section(plan, 3)
    matrix = task_three.split("The allowlist is finite and derived from this exact matrix", 1)[
        1
    ].split("Implement the matrix as canonical raw command strings", 1)[0]
    templates = {
        value
        for value in re.findall(r"`([^`]+)`", matrix)
        if value.startswith(("HIERONYMUS_", "uv run ", "unshare "))
    }
    risks_and_modules = (
        ("mcp-transport", "mcp"),
        ("semantic-native", "semantic"),
        ("frontend-embedding", "frontend"),
        ("legacy-database-import", "database"),
    )
    statuses = (
        ("accepted", "true", "true"),
        ("rejected", "false", "false"),
        ("rejected", "true", "false"),
        ("rejected", "false", "true"),
    )
    expanded: set[str] = set()
    for template in templates:
        if "<risk-module>" in template:
            expanded.update(
                template.replace("<risk-module>", module).replace("<risk>", risk)
                for risk, module in risks_and_modules
            )
        elif "<status>" in template:
            for risk, _module in risks_and_modules:
                for status, objective, normative in statuses:
                    command = template.replace("<risk>", risk).replace("<status>", status)
                    command = command.replace("<bool>", objective, 1).replace(
                        "<bool>", normative, 1
                    )
                    expanded.add(command)
        elif "<risk>" in template:
            expanded.update(template.replace("<risk>", risk) for risk, _module in risks_and_modules)
        else:
            expanded.add(template)

    printed = set().union(
        *(
            _printed_replay_commands(_task_section(plan, task))
            for task in (8, 12, 15, 18, 19, 20, 21)
        )
    )
    expected = expanded | printed
    assert len(expected) == 54
    return expected


def _adjacent_replay_mutation(command: str) -> tuple[str, frozenset[str]]:
    """Keep the command family while breaking one named safety coupling."""
    risks = (
        "mcp-transport",
        "semantic-native",
        "frontend-embedding",
        "legacy-database-import",
    )
    live_risk_commands = tuple(
        item
        for item in PLANNED_REPLAY_COMMANDS
        if item.startswith("HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=")
    )
    if command in live_risk_commands:
        row = live_risk_commands.index(command)
        if row == 0:
            return (
                command.replace("run_mcp --write", "run_semantic --write"),
                frozenset({"risk-runner-mismatch"}),
            )
        if row == 1:
            return (
                command.replace(
                    "HIERONYMUS_QUALIFICATION_LIVE=1 "
                    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native "
                    "CARGO_NET_OFFLINE=true",
                    "CARGO_NET_OFFLINE=true "
                    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native "
                    "HIERONYMUS_QUALIFICATION_LIVE=1",
                ),
                frozenset({"environment-order"}),
            )
        runners = ("run_mcp", "run_semantic", "run_frontend", "run_database")
        runner = next(item for item in runners if f".{item} --write" in command)
        replacement = runners[(runners.index(runner) + 1) % len(runners)]
        return (
            command.replace(f".{runner} --write", f".{replacement} --write"),
            frozenset({"risk-runner-mismatch"}),
        )
    if command.startswith("HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_NET_OFFLINE=true"):
        return (
            command.replace(
                "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_NET_OFFLINE=true",
                "CARGO_NET_OFFLINE=true HIERONYMUS_QUALIFICATION_LIVE=1",
            ),
            frozenset({"environment-order"}),
        )
    if "tools.qualification.validate qualification/records/" in command:
        return (
            command.replace("qualification/records/", "qualification/records/../records/"),
            frozenset({"record-input-path"}),
        )
    if "tools.qualification.render --check" in command:
        risk = next(item for item in risks if f"records/{item}.json" in command)
        replacement = risks[(risks.index(risk) + 1) % len(risks)]
        return (
            command.replace(f"rust/{risk}.md", f"rust/{replacement}.md"),
            frozenset({"record-output-pair"}),
        )
    if "tools.qualification.review" in command:
        risk = next(item for item in risks if f"review {item} " in command)
        risk_row = risks.index(risk)
        if "--status accepted" in command:
            status_row = 0
        elif command.endswith("false --normative-constraints-preserved false"):
            status_row = 1
        elif command.endswith("true --normative-constraints-preserved false"):
            status_row = 2
        else:
            status_row = 3
        mutation_kind = (risk_row + status_row) % 5
        if mutation_kind == 0:
            return (
                command.replace(
                    "Pavel Obruchnikov <me@inkyquill.net>",
                    "Another Owner <owner@example.invalid>",
                ),
                frozenset({"review-owner-changed"}),
            )
        if mutation_kind == 1:
            return (
                command.replace(
                    '--owner "Pavel Obruchnikov <me@inkyquill.net>"',
                    "--owner Pavel Obruchnikov <me@inkyquill.net>",
                ),
                frozenset({"review-owner-unquoted"}),
            )
        if mutation_kind == 2:
            changed = (
                command.replace("--status accepted", "--status rejected")
                if "--status accepted" in command
                else command.replace("--status rejected", "--status accepted")
            )
            return (
                changed,
                frozenset({"review-status-mismatch"}),
            )
        if mutation_kind == 3:
            if "--status accepted" in command:
                changed = command.replace(
                    "--objective-evidence-reviewed true",
                    "--objective-evidence-reviewed false",
                )
            else:
                changed = re.sub(
                    r"--objective-evidence-reviewed (?:true|false) "
                    r"--normative-constraints-preserved (?:true|false)$",
                    "--objective-evidence-reviewed TRUE --normative-constraints-preserved TRUE",
                    command,
                )
            return (
                changed,
                frozenset({"review-assertion-mismatch"}),
            )
        return (
            re.sub(
                r"(--objective-evidence-reviewed (?:true|false)) "
                r"(--normative-constraints-preserved (?:true|false))$",
                r"\2 \1",
                command,
            ),
            frozenset({"review-flag-order"}),
        )
    if "tools.qualification.check --record" in command:
        return command + "-stale", frozenset({"checker-record-risk"})
    if command.endswith("tools.qualification.projections --check"):
        return (
            command.removesuffix("--check") + "--write",
            frozenset({"projection-mode"}),
        )
    if command.endswith("tools.qualification.check --records-only"):
        return command + " --require-qualified", frozenset({"checker-flags"})
    if command.endswith("tools.qualification.check --require-qualified"):
        return command + " --records-only", frozenset({"checker-flags"})
    if command == "uv run python -m tools.qualification.clean":
        return command + " --include-model", frozenset({"cleanup-mode"})
    if command.endswith("tools.qualification.clean --apply"):
        return command + " --stale", frozenset({"cleanup-mode"})
    if command.endswith("tools.qualification.clean --apply --include-model"):
        return (
            command.replace("--apply --include-model", "--include-model --apply"),
            frozenset({"cleanup-flag-order"}),
        )
    if command.startswith("unshare --user --map-root-user --net -- bun run"):
        return (
            command.replace("--outDir ../qualification/", "--outDir qualification/"),
            frozenset({"frontend-parent-output"}),
        )
    if " cargo +1.96.0 " in command:
        risk = next(item for item in risks if f"cargo-target/{item}" in command)
        cargo_rows = tuple(item for item in PLANNED_REPLAY_COMMANDS if " cargo +1.96.0 " in item)
        row = cargo_rows.index(command)
        family = row % 4
        if family == 0:
            replacement = risks[(risks.index(risk) + 1) % len(risks)]
            return (
                command.replace(
                    f"qualification/harnesses/{risk}/Cargo.toml",
                    f"qualification/harnesses/{replacement}/Cargo.toml",
                ),
                frozenset({"cargo-manifest"}),
            )
        if family == 1:
            if "--target x86_64-unknown-linux-gnu" in command:
                changed = command.replace(
                    "--target x86_64-unknown-linux-gnu",
                    "--target aarch64-unknown-linux-gnu",
                )
            else:
                changed = command + " --target aarch64-unknown-linux-gnu"
            return changed, frozenset({"cargo-target"})
        if family == 2:
            return command + " --stale-extra", frozenset({"cargo-extra-flag"})
        if "--locked --all-targets" in command:
            changed = command.replace("--locked --all-targets", "--all-targets --locked")
        elif "--locked --target" in command:
            changed = command.replace("--locked --target", "--target").replace(
                " -- -D warnings", " --locked -- -D warnings"
            )
        else:
            changed = command.replace("--manifest-path", "--check --manifest-path", 1)
        return changed, frozenset({"cargo-flag-order"})
    raise AssertionError(f"unhandled canonical replay command family: {command!r}")


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
    ("name", "issue"),
    [
        *(
            (name, "record contains authorization material")
            for name in (
                "auth",
                "auth_header",
                "authHeader",
                "authorization",
                "authorization_header",
                "authorizationHeader",
                "proxy_authorization",
                "proxyAuthorization",
            )
        ),
        *(
            (name, "record contains cookie material")
            for name in (
                "cookie",
                "cookie_header",
                "cookieHeader",
                "set_cookie",
                "setCookie",
            )
        ),
        *(
            (name, "record contains a provider key")
            for name in (
                "provider_key",
                "providerKey",
                "openai_api_key",
                "openaiApiKey",
                "anthropic_api_key",
                "anthropicApiKey",
                "gemini_api_key",
                "geminiApiKey",
            )
        ),
        *(
            (name, "record contains a token or secret value")
            for name in (
                "password",
                "pass_word",
                "passWord",
                "private_key",
                "privateKey",
                "access_token",
                "accessToken",
                "refresh_token",
                "refreshToken",
                "client_secret",
                "clientSecret",
                "api_key",
                "apiKey",
                "secret",
                "token",
                "launch_grant",
                "launchGrant",
            )
        ),
        *(
            (name, "record contains source-row text")
            for name in (
                "memory_text",
                "memoryText",
                "source_text",
                "sourceText",
                "source_row",
                "sourceRow",
                "row_text",
                "rowText",
                "chunk_text",
                "chunkText",
                "translation_text",
                "translationText",
                "note_text",
                "noteText",
            )
        ),
        *(
            (name, "record contains raw process output")
            for name in (
                "raw_log",
                "rawLog",
                "raw_logs",
                "rawLogs",
                "stdout",
                "stderr",
            )
        ),
        *(
            (name, "record contains a hostname")
            for name in (
                "hostname",
                "host_name",
                "hostName",
                "machine_name",
                "machineName",
                "host",
            )
        ),
        *(
            (name, "record contains a username")
            for name in (
                "username",
                "user_name",
                "userName",
                "login_user",
                "loginUser",
            )
        ),
    ],
)
@pytest.mark.parametrize("value", [8675309, True, None, ["private", 7, False, None]])
@pytest.mark.parametrize("nested", [False, True])
def test_redaction_recursively_rejects_structured_values_of_every_json_type(
    name: str,
    issue: str,
    value: object,
    nested: bool,
) -> None:
    payload: object = {name: value}
    if nested:
        payload = {"outer": [{"inner": payload}]}

    issues = redaction_issues(json.dumps(payload, separators=(",", ":")))

    assert issue in issues
    assert all(str(value) not in diagnostic for diagnostic in issues)


@pytest.mark.parametrize(
    "marker",
    [
        "-----BEGIN PRIVATE KEY-----",
        "-----BEGIN RSA PRIVATE KEY-----",
        "-----BEGIN EC PRIVATE KEY-----",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        "-----BEGIN FOO-BAR PRIVATE KEY-----",
        "-----BEGIN ACME/V2+HSM PRIVATE KEY-----",
        "-----BEGIN X.509_TEST PRIVATE KEY-----",
        "-----BEGIN FOO  BAR PRIVATE KEY-----",
        "-----BEGIN FOO-----BAR PRIVATE KEY-----",
        "-----BEGIN !\"#$%&'()*+,./:;<=>?@[\\]^_`{|}~ PRIVATE KEY-----",
    ],
)
def test_redaction_rejects_private_key_pem_without_echo(marker: str) -> None:
    payload = "private payload must not be echoed"
    issues = redaction_issues(marker + "\n" + payload)

    assert issues == ["record contains private-key material"]
    assert all(marker not in issue and payload not in issue for issue in issues)


@pytest.mark.parametrize("codepoint", range(0x20, 0x7F))
def test_redaction_pem_accepts_each_ascii_printable_label_character(codepoint: int) -> None:
    character = chr(codepoint)
    marker = f"-----BEGIN LEFT{character}RIGHT PRIVATE KEY-----"

    assert redaction_issues(marker) == ["record contains private-key material"]


@pytest.mark.parametrize("codepoint", (*range(0x20), 0x7F, *range(0x80, 0xA0)))
def test_redaction_pem_rejects_each_ascii_control_label_character(codepoint: int) -> None:
    marker = f"-----BEGIN LEFT{chr(codepoint)}RIGHT PRIVATE KEY-----"

    assert redaction_issues(marker) == []


def test_redaction_pem_accepts_exact_marker_inside_canonical_json() -> None:
    serialized = json.dumps(
        {"artifact": "-----BEGIN FOO-----BAR PRIVATE KEY-----"},
        sort_keys=True,
        separators=(",", ":"),
    )

    assert redaction_issues(serialized) == ["record contains private-key material"]


def test_redaction_pem_accepts_marker_before_json_escaped_payload() -> None:
    serialized = json.dumps(
        {"artifact": "-----BEGIN FOO PRIVATE KEY-----\nprivate payload"},
        sort_keys=True,
        separators=(",", ":"),
    )

    assert redaction_issues(serialized) == ["record contains private-key material"]


@pytest.mark.parametrize("codepoint", (*range(0x20), 0x7F, *range(0x80, 0xA0)))
def test_json_pem_scans_decoded_string_leaves_not_escape_spelling(codepoint: int) -> None:
    serialized = json.dumps(
        {"artifact": f"-----BEGIN LEFT{chr(codepoint)}RIGHT PRIVATE KEY-----"},
        sort_keys=True,
        separators=(",", ":"),
    )

    assert redaction_issues(serialized) == []


def test_json_pem_detects_marker_created_by_printable_unicode_escape() -> None:
    serialized = r'{"artifact":"-----\u0042EGIN PRIVATE KEY-----"}'

    assert redaction_issues(serialized) == ["record contains private-key material"]


def test_percent_decoding_is_not_used_for_pem_detection() -> None:
    encoded = "%2D%2D%2D%2D%2DBEGIN%20PRIVATE%20KEY%2D%2D%2D%2D%2D"

    assert redaction_issues(encoded) == []


@pytest.mark.parametrize(
    "near_miss",
    [
        "------BEGIN PRIVATE KEY-----",
        "-----BEGIN PUBLIC KEY-----",
        "-----BEGIN PRIVATE KEYS-----",
        "-----BEGIN PRIVATE KEYCHAIN-----",
        "-----BEGINPRIVATE KEY-----",
        "-----BEGIN FOO PRIVATE KEY ----",
        "-----BEGIN FOO PRIVATE KEY------",
        "-----BEGIN FOO PRIVATE KEY-----trailing",
        "-----BEGIN FOO\nPRIVATE KEY-----",
        "-----BEGIN FOO\tPRIVATE KEY-----",
        "-----BEGIN FOO\x7f PRIVATE KEY-----",
        "-----BEGIN FOO\x80 PRIVATE KEY-----",
    ],
)
def test_redaction_private_key_pem_rule_has_exact_begin_label_boundaries(
    near_miss: str,
) -> None:
    assert redaction_issues(near_miss) == []


@pytest.mark.parametrize(
    "safe",
    [
        "stderr: none;",
        "raw log: <absent>.",
        "| authorizationHeader | <redacted> |",
    ],
)
def test_redaction_allows_punctuated_safe_sentinels(safe: str) -> None:
    assert redaction_issues(safe) == []


@pytest.mark.parametrize(
    "safe",
    [
        "stderr: &lt;redacted&gt;;",
        "stderr: &lt;redacted&gt;:",
        "stderr: &lt;redacted&gt;!",
        "stderr: &lt;redacted&gt;?",
        "stderr: &lt;redacted&gt;)",
        "raw log: &lt;absent&gt;.",
        "raw log: &lt;absent&gt;,",
        "| authorizationHeader | &lt;redacted&gt; |",
    ],
)
def test_redaction_allows_markdown_encoded_safe_sentinels(safe: str) -> None:
    assert redaction_issues(safe) == []


@pytest.mark.parametrize(
    "leak",
    [
        "stderr: &lt;redacted&gt; followed-by-output",
        "raw log: &lt;script&gt;.",
        "| authorizationHeader | &lt;credential&gt; |",
    ],
)
def test_encoded_safe_sentinel_support_does_not_mask_html_or_following_secrets(
    leak: str,
) -> None:
    assert redaction_issues(leak)


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
        "compatibility/fixtures/mcp/hieronymus_rag_search/success.input.json",
        "compatibility/fixtures/mcp/hieronymus_recall/success.input.json",
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
    commands = PLANNED_REPLAY_COMMANDS
    record = replace(make_record(tmp_path, "mcp-transport"), commands=commands)

    assert validate_record(record, tmp_path) == []


def test_replay_safety_owns_nonempty_uniqueness_and_future_command_matrix() -> None:
    assert PLANNED_REPLAY_COMMANDS
    assert len(PLANNED_REPLAY_COMMANDS) == len(set(PLANNED_REPLAY_COMMANDS))
    assert replay_commands_are_safe(PLANNED_REPLAY_COMMANDS)
    assert replay_commands_are_safe(()) is False
    assert replay_commands_are_safe((PLANNED_REPLAY_COMMANDS[0],) * 2) is False
    assert "uv run python -m tools.qualification.projections --check" in PLANNED_REPLAY_COMMANDS
    assert (
        "uv run python -m tools.qualification.clean --apply --include-model"
        in PLANNED_REPLAY_COMMANDS
    )
    assert any(
        '--owner "Pavel Obruchnikov <me@inkyquill.net>"' in command
        for command in PLANNED_REPLAY_COMMANDS
    )
    assert replay_commands_are_safe((["not hashable"],)) is False


@pytest.mark.parametrize("command", PLANNED_REPLAY_COMMANDS)
def test_every_finite_replay_matrix_entry_is_individually_accepted(command: str) -> None:
    assert replay_commands_are_safe((command,))


def test_replay_matrix_mechanically_covers_commands_printed_by_owning_tasks() -> None:
    plan = (ROOT / "docs/superpowers/plans/2026-09-01-rust-qualification.md").read_text(
        encoding="utf-8"
    )
    expected = _expected_replay_commands_from_plan(plan)
    actual = set(PLANNED_REPLAY_COMMANDS)

    assert expected == actual
    assert expected != actual - {PLANNED_REPLAY_COMMANDS[0]}
    assert expected != actual | {PLANNED_REPLAY_COMMANDS[0] + " --stale"}


def test_every_replay_row_has_a_unique_rejected_adjacent_mutation() -> None:
    paired = tuple(_adjacent_replay_mutation(command) for command in PLANNED_REPLAY_COMMANDS)
    mutations = tuple(mutation for mutation, _tags in paired)
    covered_families = set().union(*(tags for _mutation, tags in paired))
    required_families = {
        "risk-runner-mismatch",
        "environment-order",
        "record-input-path",
        "record-output-pair",
        "review-owner-changed",
        "review-owner-unquoted",
        "review-status-mismatch",
        "review-assertion-mismatch",
        "review-flag-order",
        "checker-record-risk",
        "projection-mode",
        "checker-flags",
        "cleanup-mode",
        "cleanup-flag-order",
        "cargo-manifest",
        "cargo-target",
        "cargo-extra-flag",
        "cargo-flag-order",
        "frontend-parent-output",
    }

    assert len(PLANNED_REPLAY_COMMANDS) == 54
    assert len(mutations) == len(set(mutations)) == 54
    assert covered_families == required_families
    assert set(mutations).isdisjoint(PLANNED_REPLAY_COMMANDS)
    assert all(not replay_commands_are_safe((mutation,)) for mutation in mutations)
