"""Central sensitive-data checks for qualification JSON and Markdown."""

from __future__ import annotations

import re
from dataclasses import dataclass


@dataclass(frozen=True)
class _Rule:
    issue: str
    patterns: tuple[re.Pattern[str], ...]


def _structured_patterns(*names: str) -> tuple[re.Pattern[str], ...]:
    alternatives = "|".join(re.escape(name) for name in names)
    return (
        re.compile(
            rf'"(?:{alternatives})"\s*:\s*"(?!\s*(?:<absent>|none|null)\s*")[^"\r\n]+"',
            re.IGNORECASE,
        ),
        re.compile(
            rf"^\s*\|\s*(?:{alternatives})\s*\|\s*"
            rf"(?!\s*(?:<absent>|\(none\)|none|null)\s*\|)[^|\r\n]+\|",
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
                "authorization",
                "authorization_header",
                "proxy_authorization",
            ),
            re.compile(
                r"\b(?:proxy-)?authorization\s*:\s*"
                r"(?!\s*(?:<absent>|\(none\)|none|null)\s*(?:$|\|))"
                r"[A-Za-z][A-Za-z0-9._~+/=-]*(?:\s+[^\s|,;]+)?",
                re.IGNORECASE,
            ),
            re.compile(r"\bbearer\s+[A-Za-z0-9._~+/=-]{8,}", re.IGNORECASE),
        ),
    ),
    _Rule(
        "record contains cookie material",
        (
            *_structured_patterns("cookie", "set_cookie", "cookie_header"),
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
                "private_key",
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
                r"(?!\s*(?:<absent>|\(none\)|none|null)\s*(?:$|\|))\S+",
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
)


def redaction_issues(serialized_record: str) -> list[str]:
    """Return each sensitive-data class found, once, in fixed rule order."""
    if type(serialized_record) is not str:
        raise TypeError("serialized record must be text")
    return [
        rule.issue
        for rule in _RULES
        if any(pattern.search(serialized_record) for pattern in rule.patterns)
    ]
