"""Central sensitive-data checks for qualification JSON and Markdown."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from html import unescape as unescape_html
from urllib.parse import unquote


@dataclass(frozen=True)
class _Rule:
    issue: str
    patterns: tuple[re.Pattern[str], ...]


_SAFE_TEXT_SENTINEL = r"(?:<absent>|<redacted>|&lt;absent&gt;|&lt;redacted&gt;|none)"


def _structured_patterns(*names: str) -> tuple[re.Pattern[str], ...]:
    alternatives = "|".join(re.escape(name) for name in names)
    return (
        re.compile(
            rf'"(?:{alternatives})"\s*:\s*"(?!\s*{_SAFE_TEXT_SENTINEL}\s*")[^"\r\n]+"',
            re.IGNORECASE,
        ),
        re.compile(
            rf"^\s*\|\s*(?:{alternatives})\s*\|\s*"
            rf"(?!\s*(?:{_SAFE_TEXT_SENTINEL}|\(none\))\s*\|)[^|\r\n]+\|",
            re.IGNORECASE | re.MULTILINE,
        ),
    )


def _assignment_pattern(*names: str) -> re.Pattern[str]:
    alternatives = "|".join(re.escape(name) for name in names)
    return re.compile(
        rf"(?<![A-Za-z0-9_])(?:{alternatives})\s*=\s*"
        rf"(?!\s*(?:<absent>|none|null)(?:\s|$))[^\s\"'|,;}}]+",
        re.IGNORECASE,
    )


_WINDOWS_SEPARATOR = r"(?:\\\\|[\\/])"
_SAFE_SENTINELS = frozenset({"none", "<absent>", "<redacted>"})
_STRUCTURED_ISSUES = {
    "authorization": frozenset(
        {"auth", "authheader", "authorization", "authorizationheader", "proxyauthorization"}
    ),
    "cookie": frozenset({"cookie", "cookieheader", "setcookie"}),
    "provider": frozenset(
        {
            "providerkey",
            "openaiapikey",
            "anthropicapikey",
            "geminiapikey",
        }
    ),
    "secret": frozenset(
        {
            "password",
            "privatekey",
            "accesstoken",
            "refreshtoken",
            "clientsecret",
            "apikey",
            "secret",
            "token",
            "launchgrant",
        }
    ),
    "source": frozenset(
        {
            "memorytext",
            "sourcetext",
            "sourcerow",
            "rowtext",
            "chunktext",
            "translationtext",
            "notetext",
        }
    ),
    "host": frozenset({"hostname", "machinename", "host"}),
    "user": frozenset({"username", "loginuser"}),
    "raw": frozenset({"rawlog", "rawlogs", "stdout", "stderr"}),
}
_STRUCTURED_CLASS_ISSUES = {
    "authorization": "record contains authorization material",
    "cookie": "record contains cookie material",
    "provider": "record contains a provider key",
    "secret": "record contains a token or secret value",
    "source": "record contains source-row text",
    "host": "record contains a hostname",
    "user": "record contains a username",
    "raw": "record contains raw process output",
}
_RULES = (
    _Rule(
        "record contains forbidden literal: compat-secret-do-not-log",
        (re.compile(re.escape("compat-secret-do-not-log"), re.IGNORECASE),),
    ),
    _Rule(
        "record contains forbidden literal: Yandex.Disk",
        (re.compile(re.escape("Yandex.Disk"), re.IGNORECASE),),
    ),
    _Rule(
        "record contains authorization material",
        (
            *_structured_patterns(
                "auth",
                "auth_header",
                "authHeader",
                "authorization",
                "authorization_header",
                "authorizationHeader",
                "proxy_authorization",
                "proxyAuthorization",
            ),
            re.compile(
                r"\b(?:proxy-)?authorization\s*:\s*"
                rf"(?!\s*(?:{_SAFE_TEXT_SENTINEL}|\(none\))\s*[.,;:!?)]?\s*(?:$|\|))"
                r"[A-Za-z][A-Za-z0-9._~+/=-]*(?:\s+[^\s|,;]+)?",
                re.IGNORECASE,
            ),
            re.compile(r"\bbearer\s+[A-Za-z0-9._~+/=-]{8,}", re.IGNORECASE),
        ),
    ),
    _Rule(
        "record contains cookie material",
        (
            *_structured_patterns(
                "cookie",
                "cookie_header",
                "cookieHeader",
                "set_cookie",
                "setCookie",
            ),
            re.compile(
                r"\b(?:set-cookie|cookie)\s*:\s*"
                r"[^\s|,;=]+=[^\s|,;]+(?:\s*;\s*[^\s|,;=]+=[^\s|,;]+)*",
                re.IGNORECASE,
            ),
        ),
    ),
    _Rule(
        "record contains a provider key",
        _structured_patterns(
            "provider_key",
            "openai_api_key",
            "openaiApiKey",
            "anthropic_api_key",
            "anthropicApiKey",
            "gemini_api_key",
            "geminiApiKey",
        )
        + (
            _assignment_pattern(
                "provider_key",
                "openai_api_key",
                "openaiApiKey",
                "anthropic_api_key",
                "anthropicApiKey",
                "gemini_api_key",
                "geminiApiKey",
            ),
            re.compile(r"(?<![A-Za-z0-9_-])sk-proj-[A-Za-z0-9_-]{16,}", re.IGNORECASE),
            re.compile(r"(?<![A-Z0-9])AKIA[A-Z0-9]{16}(?![A-Z0-9])"),
        ),
    ),
    _Rule(
        "record contains a token or secret value",
        (
            *_structured_patterns(
                "access_token",
                "refresh_token",
                "client_secret",
                "api_key",
                "apiKey",
                "secret",
                "token",
                "launch_grant",
                "launchGrant",
                "password",
                "pass_word",
                "passWord",
                "private_key",
                "privateKey",
            ),
            re.compile(r"\b(?:ghp|gho|github_pat)_[A-Za-z0-9_]{16,}\b"),
            re.compile(r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b"),
            _assignment_pattern(
                "access_token",
                "refresh_token",
                "client_secret",
                "api_key",
                "apiKey",
                "secret",
                "token",
                "launch_grant",
                "launchGrant",
                "password",
                "pass_word",
                "passWord",
                "private_key",
                "privateKey",
            ),
        ),
    ),
    _Rule(
        "record contains source-row text",
        _structured_patterns(
            "source_text",
            "sourceText",
            "source_row",
            "row_text",
            "memory_text",
            "chunk_text",
            "translation_text",
            "note_text",
        ),
    ),
    _Rule(
        "record contains a hostname",
        _structured_patterns("hostname", "host_name", "machine_name", "host")
        + (_assignment_pattern("hostname", "host_name", "machine_name", "host"),),
    ),
    _Rule(
        "record contains a username",
        _structured_patterns("username", "user_name", "login_user")
        + (
            _assignment_pattern(
                "username",
                "user_name",
                "login_user",
                "user",
                "logname",
            ),
        ),
    ),
    _Rule(
        "record contains raw process output",
        (
            *_structured_patterns("stdout", "stderr", "raw_log", "raw_logs"),
            re.compile(
                r"\b(?:stdout|stderr|raw[-_ ]logs?)\s*:\s*"
                rf"(?!\s*(?:{_SAFE_TEXT_SENTINEL}|\(none\))\s*[.,;:!?)]?\s*(?:$|\|))\S+",
                re.IGNORECASE,
            ),
        ),
    ),
    _Rule(
        "record contains an absolute home path",
        (
            re.compile(r"/(?:home|Users)/[^/\s\"'|<>]+(?=/|$|[\s\"'|<>])"),
            re.compile(r"/root(?=/|$|[\s\"'|<>])"),
            re.compile(
                rf"\b[A-Za-z]:{_WINDOWS_SEPARATOR}Users{_WINDOWS_SEPARATOR}"
                rf"[^\\/\s\"'|<>]+"
                rf"(?={_WINDOWS_SEPARATOR}|$|[\s\"'|<>])",
                re.IGNORECASE,
            ),
            re.compile(
                rf"\b[A-Za-z]:{_WINDOWS_SEPARATOR}Documents and Settings"
                rf"{_WINDOWS_SEPARATOR}[^\\/\s\"'|<>]+"
                rf"(?={_WINDOWS_SEPARATOR}|$|[\s\"'|<>])",
                re.IGNORECASE,
            ),
        ),
    ),
    _Rule(
        "record contains private-key material",
        (),
    ),
)

_PRIVATE_KEY_BEGIN = "-----BEGIN "
_PRIVATE_KEY_SUFFIX = " PRIVATE KEY"
_PRIVATE_KEY_TERMINATOR = "-----"
_PRIVATE_KEY_BOUNDARIES = frozenset(" \t\r\n\\\"',.;:!?)]}|")

_CRITERIA_HEADING = "## Required Criteria"
_CRITERIA_HEADER = "| Criterion | Status | Summary | Measurements | Not-run reason |"
_CRITERIA_SEPARATOR = "| --- | --- | --- | --- | --- |"
_NEXT_CRITERIA_HEADING = "## Consumed Compatibility Contracts"


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
        if ord(character) < 32 or 127 <= ord(character) <= 159
        else replacements.get(character, character)
        for character in rendered
    )


@dataclass(frozen=True, slots=True)
class RequiredCriteriaRow:
    """Semantic source for one exact renderer-owned Required Criteria row."""

    criterion: str
    status: str
    summary: str
    canonical_measurements: str
    not_run_reason: str

    @property
    def rendered_row(self) -> str:
        return self._render(self.canonical_measurements)

    @property
    def masked_row(self) -> str:
        return self._render("{}")

    def _render(self, measurements: str) -> str:
        return (
            "| "
            + " | ".join(
                _literal_markdown(value)
                for value in (
                    self.criterion,
                    self.status,
                    self.summary,
                    measurements,
                    self.not_run_reason,
                )
            )
            + " |"
        )


def _contains_private_key_marker(value: str) -> bool:
    """Recognize one exact printable PEM private-key BEGIN marker."""
    search_from = 0
    while True:
        marker_start = value.find(_PRIVATE_KEY_BEGIN, search_from)
        if marker_start < 0:
            return False
        search_from = marker_start + 1
        if marker_start > 0 and value[marker_start - 1] == "-":
            continue

        label_start = marker_start + len(_PRIVATE_KEY_BEGIN)
        terminator_start = label_start
        while True:
            terminator_start = value.find(_PRIVATE_KEY_TERMINATOR, terminator_start)
            if terminator_start < 0:
                break
            after_terminator = terminator_start + len(_PRIVATE_KEY_TERMINATOR)
            if after_terminator < len(value):
                boundary = value[after_terminator]
                if boundary == "-" or boundary not in _PRIVATE_KEY_BOUNDARIES:
                    terminator_start += 1
                    continue
            label = value[label_start:terminator_start]
            if label != "PRIVATE KEY" and not label.endswith(_PRIVATE_KEY_SUFFIX):
                terminator_start += 1
                continue
            if all("\x20" <= character <= "\x7e" for character in label):
                return True
            terminator_start += 1


def redaction_issues(serialized_record: str) -> list[str]:
    """Return each sensitive-data class found, once, in fixed rule order."""
    if type(serialized_record) is not str:
        raise TypeError("serialized record must be text")
    try:
        parsed: object | None = json.loads(serialized_record)
        is_json = True
    except (json.JSONDecodeError, UnicodeDecodeError):
        parsed = None
        is_json = False
    structured = _structured_redaction_issues(parsed) if is_json else set()
    pem_values = _string_nodes(parsed) if is_json else (unescape_html(serialized_record),)
    raw = _raw_redaction_issue_set(serialized_record, pem_values=pem_values)
    return _ordered_issues(structured | raw)


def markdown_redaction_issues(
    rendered_markdown: str,
    *,
    required_criteria: tuple[RequiredCriteriaRow, ...] = (),
) -> list[str]:
    """Scan completed Markdown while preserving canonical-JSON cell semantics.

    The renderer owns the fourth column of each Required Criteria row.  Those
    cells are parsed from their exact canonical JSON source instead of guessing
    whether a printable backslash escape in Markdown represents a control.
    Every other Markdown field is scanned after exactly one HTML-entity decode.
    """
    if type(rendered_markdown) is not str:
        raise TypeError("rendered Markdown must be text")
    if type(required_criteria) is not tuple or any(
        type(row) is not RequiredCriteriaRow for row in required_criteria
    ):
        raise TypeError("required criteria must be a tuple of typed rows")

    semantic_issues: set[str] = set()
    for row in required_criteria:
        cell = row.canonical_measurements
        parsed = _parse_canonical_json_cell(cell)
        semantic_issues.update(_structured_redaction_issues(parsed))
        semantic_issues.update(_raw_redaction_issue_set(cell, pem_values=_string_nodes(parsed)))

    masked = _mask_required_criteria_table(rendered_markdown, required_criteria)
    decoded_markdown = unescape_html(masked)
    markdown_issues = _raw_redaction_issue_set(
        decoded_markdown,
        pem_values=(decoded_markdown,),
    )
    return _ordered_issues(semantic_issues | markdown_issues)


def _raw_redaction_issue_set(
    value: str,
    *,
    pem_values: tuple[str, ...],
) -> set[str]:
    decoded_view = unquote(value)
    issues = {
        rule.issue for rule in _RULES if any(pattern.search(value) for pattern in rule.patterns)
    }
    home_rule = next(
        rule for rule in _RULES if rule.issue == "record contains an absolute home path"
    )
    if any(pattern.search(decoded_view) for pattern in home_rule.patterns):
        issues.add(home_rule.issue)
    if any(_contains_private_key_marker(value) for value in pem_values):
        issues.add("record contains private-key material")
    return issues


def _ordered_issues(issues: set[str]) -> list[str]:
    return [rule.issue for rule in _RULES if rule.issue in issues]


def _parse_canonical_json_cell(serialized: str) -> object:
    try:
        parsed = json.loads(serialized)
        canonical = json.dumps(
            parsed,
            allow_nan=False,
            sort_keys=True,
            separators=(",", ":"),
        )
    except (json.JSONDecodeError, TypeError, ValueError) as error:
        raise ValueError("renderer-owned JSON cell is not canonical") from error
    if type(parsed) is not dict or canonical != serialized:
        raise ValueError("renderer-owned JSON cell is not canonical")
    return parsed


def _mask_required_criteria_table(
    rendered_markdown: str,
    required_criteria: tuple[RequiredCriteriaRow, ...],
) -> str:
    lines = rendered_markdown.split("\n")
    heading_positions = [index for index, line in enumerate(lines) if line == _CRITERIA_HEADING]
    header_positions = [index for index, line in enumerate(lines) if line == _CRITERIA_HEADER]
    if not required_criteria:
        if heading_positions or header_positions:
            raise ValueError("Required Criteria table is malformed")
        return rendered_markdown
    if len(heading_positions) != 1 or len(header_positions) != 1:
        raise ValueError("Required Criteria table is malformed")

    start = heading_positions[0]
    rows_start = start + 4
    after_rows = rows_start + len(required_criteria)
    if (
        start == 0
        or lines[start - 1] != ""
        or lines[start : start + 4]
        != [
            _CRITERIA_HEADING,
            "",
            _CRITERIA_HEADER,
            _CRITERIA_SEPARATOR,
        ]
        or after_rows + 1 >= len(lines)
        or lines[after_rows] != ""
        or lines[after_rows + 1] != _NEXT_CRITERIA_HEADING
    ):
        raise ValueError("Required Criteria table is malformed")

    actual_rows = lines[rows_start:after_rows]
    expected_rows = [row.rendered_row for row in required_criteria]
    if actual_rows != expected_rows:
        raise ValueError("Required Criteria table is malformed")

    masked_lines = list(lines)
    masked_lines[rows_start:after_rows] = [row.masked_row for row in required_criteria]
    return "\n".join(masked_lines)


def _structured_redaction_issues(parsed: object) -> set[str]:
    issues: set[str] = set()

    def visit(node: object) -> None:
        if isinstance(node, dict):
            for key, child in node.items():
                normalized = _normalize_key(key)
                for issue_class, names in _STRUCTURED_ISSUES.items():
                    if normalized in names and not _safe_structured_value(child):
                        issues.add(_STRUCTURED_CLASS_ISSUES[issue_class])
                visit(child)
        elif isinstance(node, list):
            for child in node:
                visit(child)

    visit(parsed)
    return issues


def _string_nodes(value: object) -> tuple[str, ...]:
    strings: list[str] = []

    def visit(node: object) -> None:
        if type(node) is str:
            strings.append(node)
        elif isinstance(node, dict):
            for key, child in node.items():
                if type(key) is str:
                    strings.append(key)
                visit(child)
        elif isinstance(node, list):
            for child in node:
                visit(child)

    visit(value)
    return tuple(strings)


def _normalize_key(value: object) -> str:
    if type(value) is not str:
        return ""
    return "".join(
        character.lower() if "A" <= character <= "Z" else character
        for character in value
        if character != "_"
    )


def _safe_structured_value(value: object) -> bool:
    if type(value) is str:
        return value.strip().lower() in _SAFE_SENTINELS
    if type(value) is list and value:
        return all(type(item) is str and item.strip().lower() in _SAFE_SENTINELS for item in value)
    return False
