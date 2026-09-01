"""Validate frozen MCP envelopes against the byte-pinned official schema."""

from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
from typing import Literal

from jsonschema import Draft202012Validator
from jsonschema.exceptions import SchemaError

SchemaDefinition = Literal[
    "ListToolsResultResponse",
    "CallToolResultResponse",
    "HeaderMismatchError",
    "UnsupportedProtocolVersionError",
]

OFFICIAL_SCHEMA_SHA256 = "ef70b61f99b6d2e5e3b46863822eab08dff6a45bedc7a08914e0e5b133f40203"
OFFICIAL_SCHEMA_SOURCE: dict[str, object] = {
    "schema_version": 1,
    "protocol_revision": "2026-07-28",
    "draft": "https://json-schema.org/draft/2020-12/schema",
    "upstream_commit": "271ecc9accafdd9b83a3c869fa67c22953b2af80",
    "upstream_url": (
        "https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/"
        "271ecc9accafdd9b83a3c869fa67c22953b2af80/schema/2026-07-28/schema.json"
    ),
    "sha256": OFFICIAL_SCHEMA_SHA256,
}

_AUTHORITY_ROOT = Path("compatibility/authorities/mcp/2026-07-28")
_DRAFT_2020_12 = "https://json-schema.org/draft/2020-12/schema"
_SUPPORTED_DEFINITIONS = frozenset(
    {
        "ListToolsResultResponse",
        "CallToolResultResponse",
        "HeaderMismatchError",
        "UnsupportedProtocolVersionError",
    }
)


def authority_issues(repo_root: Path) -> tuple[str, ...]:
    """Return deterministic problems with the checked-in official schema pin."""
    authority_root = repo_root / _AUTHORITY_ROOT
    source_path = authority_root / "schema.source.json"
    schema_path = authority_root / "schema.json"
    issues: list[str] = []

    try:
        source = json.loads(source_path.read_bytes())
    except FileNotFoundError:
        issues.append(f"missing official MCP schema source metadata: {source_path}")
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        issues.append(f"invalid official MCP schema source metadata: {error}")
    else:
        if source != OFFICIAL_SCHEMA_SOURCE:
            issues.append("official MCP schema source metadata does not match the immutable pin")

    try:
        schema_bytes = schema_path.read_bytes()
    except FileNotFoundError:
        issues.append(f"missing official MCP schema: {schema_path}")
        return tuple(issues)

    digest = hashlib.sha256(schema_bytes).hexdigest()
    if digest != OFFICIAL_SCHEMA_SHA256:
        issues.append(
            f"official MCP schema SHA-256 mismatch: expected {OFFICIAL_SCHEMA_SHA256}, got {digest}"
        )
        return tuple(issues)

    try:
        schema = json.loads(schema_bytes)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        issues.append(f"invalid official MCP schema JSON: {error}")
        return tuple(issues)

    if not isinstance(schema, dict):
        issues.append("official MCP schema must be an object")
        return tuple(issues)
    if schema.get("$schema") != _DRAFT_2020_12:
        issues.append(
            "official MCP schema dialect mismatch: "
            f"expected {_DRAFT_2020_12}, got {schema.get('$schema')!r}"
        )
        return tuple(issues)
    try:
        Draft202012Validator.check_schema(schema)
    except SchemaError as error:
        issues.append(f"invalid official MCP Draft 2020-12 schema: {error.message}")
    return tuple(issues)


def definition_issues(
    repo_root: Path,
    definition: SchemaDefinition,
    instance: object,
) -> tuple[str, ...]:
    """Validate an instance against one named official MCP schema definition."""
    if definition not in _SUPPORTED_DEFINITIONS:
        return (f"unknown official MCP schema definition: {definition}",)

    issues = authority_issues(repo_root)
    if issues:
        return issues

    schema = json.loads((repo_root / _AUTHORITY_ROOT / "schema.json").read_bytes())
    validator_schema = copy.deepcopy(schema)
    validator_schema["$ref"] = f"#/$defs/{definition}"
    validator = Draft202012Validator(validator_schema)
    errors = sorted(
        validator.iter_errors(instance),
        key=lambda error: (
            tuple(str(part) for part in error.absolute_path),
            tuple(str(part) for part in error.absolute_schema_path),
            error.message,
        ),
    )
    return tuple(f"{_instance_path(error.absolute_path)}: {error.message}" for error in errors)


def _instance_path(path: object) -> str:
    parts = tuple(str(part) for part in path)
    return ".".join(parts) if parts else "$"
