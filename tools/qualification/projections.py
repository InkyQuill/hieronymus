"""Closed compatibility projections for immutable qualification evidence."""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from collections.abc import Mapping, Sequence
from copy import deepcopy
from pathlib import Path
from typing import Literal, cast

from tools.qualification.fingerprint import _MCP_TOOL_NAMES

ProjectionRisk = Literal["mcp-transport", "legacy-database-import"]

MCP_EXACT_CONTRACT_IDS = (
    "cli.script.hieronymus-mcp",
    "http.route.post.mcp",
    "http.route.post.api.mcp.operation",
)
DATABASE_CONTRACT_IDS = (
    "database.schema.current",
    "database.migrations.current",
    "database.upgrade.preflight",
)
DATABASE_STATE_FIELDS = (
    "application_migration_ledgers",
    "columns",
    "fixture",
    "foreign_keys",
    "indexes",
    "migration_sources",
    "object_contracts",
    "representative_rows",
    "row_counts",
    "tables",
    "triggers",
    "variants",
)

_RISKS: tuple[ProjectionRisk, ...] = (
    "mcp-transport",
    "legacy-database-import",
)
_PROJECTION_PATHS: Mapping[ProjectionRisk, str] = {
    "mcp-transport": "qualification/compatibility/mcp-transport.json",
    "legacy-database-import": "qualification/compatibility/legacy-database-import.json",
}
_EXPECTED_MCP_TOOL_IDS = frozenset(f"mcp.tool.{name}" for name in _MCP_TOOL_NAMES)


def build_projection(repo_root: Path, risk: ProjectionRisk) -> dict[str, object]:
    """Build one lossless, closed compatibility projection from current sources."""
    if risk not in _RISKS:
        raise ValueError("unknown qualification projection risk")
    manifest = _read_json_object(
        repo_root / "compatibility/manifest.json",
        "compatibility projection manifest is invalid",
    )
    if risk == "mcp-transport":
        contracts = _full_source_contracts(
            manifest,
            exact_ids=frozenset(MCP_EXACT_CONTRACT_IDS),
            id_prefix="mcp.tool.",
        )
        selected_tool_ids = frozenset(
            cast(str, contract["id"])
            for contract in contracts
            if cast(str, contract["id"]).startswith("mcp.tool.")
        )
        if len(contracts) != 42 or selected_tool_ids != _EXPECTED_MCP_TOOL_IDS:
            raise ValueError("mcp compatibility projection contract inventory is invalid")
        return {
            "projection_version": 1,
            "risk": risk,
            "contracts": contracts,
        }

    contracts = _full_source_contracts(
        manifest,
        exact_ids=frozenset(DATABASE_CONTRACT_IDS),
    )
    if len(contracts) != 3:
        raise ValueError("database compatibility projection contract inventory is invalid")
    state = _read_json_object(
        repo_root / "compatibility/snapshots/state.json",
        "compatibility projection state is invalid",
    )
    database = state.get("database")
    if type(database) is not dict:
        raise ValueError("database compatibility projection state is invalid")
    if any(field not in database for field in DATABASE_STATE_FIELDS):
        raise ValueError("database compatibility projection state is invalid")
    projected_database = {field: deepcopy(database[field]) for field in DATABASE_STATE_FIELDS}
    return {
        "projection_version": 1,
        "risk": risk,
        "contracts": contracts,
        "database": projected_database,
    }


def canonical_projection_bytes(projection: Mapping[str, object]) -> bytes:
    """Serialize one projection as canonical UTF-8 JSON with a trailing newline."""
    if not isinstance(projection, Mapping):
        raise TypeError("compatibility projection must be a mapping")
    try:
        serialized = json.dumps(
            dict(projection),
            ensure_ascii=False,
            allow_nan=False,
            sort_keys=True,
            separators=(",", ":"),
        )
    except (TypeError, ValueError) as error:
        raise ValueError("compatibility projection cannot be serialized") from error
    return (serialized + "\n").encode("utf-8")


def projection_issues(repo_root: Path) -> dict[ProjectionRisk, tuple[str, ...]]:
    """Report projection source or checked-in drift without changing any file."""
    issues: dict[ProjectionRisk, tuple[str, ...]] = {}
    for risk in _RISKS:
        try:
            expected = build_projection(repo_root, risk)
            expected_bytes = canonical_projection_bytes(expected)
        except (OSError, TypeError, ValueError):
            issues[risk] = (f"{risk} compatibility projection source is invalid",)
            continue
        try:
            actual_bytes = (repo_root / _PROJECTION_PATHS[risk]).read_bytes()
            actual = _decode_json_object(
                actual_bytes,
                "checked compatibility projection is invalid",
            )
            actual_canonical = canonical_projection_bytes(actual)
        except (OSError, TypeError, ValueError):
            issues[risk] = (f"{risk} compatibility projection is stale",)
            continue
        if actual != expected or actual_canonical != actual_bytes or actual_bytes != expected_bytes:
            issues[risk] = (f"{risk} compatibility projection is stale",)
        else:
            issues[risk] = ()
    return issues


def _full_source_contracts(
    manifest: Mapping[str, object],
    *,
    exact_ids: frozenset[str],
    id_prefix: str | None = None,
) -> list[dict[str, object]]:
    contracts = manifest.get("contracts")
    if type(contracts) is not list:
        raise ValueError("compatibility projection contracts are invalid")
    indexed: dict[str, dict[str, object]] = {}
    for contract in contracts:
        if type(contract) is not dict:
            raise ValueError("compatibility projection contracts are invalid")
        contract_id = contract.get("id")
        if type(contract_id) is not str or not contract_id:
            raise ValueError("compatibility projection contract ids are invalid")
        if contract_id in indexed:
            raise ValueError("compatibility projection contract ids are duplicated")
        indexed[contract_id] = contract

    selected_ids = set(exact_ids)
    if id_prefix is not None:
        selected_ids.update(
            contract_id for contract_id in indexed if contract_id.startswith(id_prefix)
        )
    if not exact_ids.issubset(indexed) or any(
        contract_id not in indexed for contract_id in selected_ids
    ):
        raise ValueError("compatibility projection contracts are missing")
    return [deepcopy(indexed[contract_id]) for contract_id in sorted(selected_ids)]


def _read_json_object(path: Path, message: str) -> dict[str, object]:
    try:
        return _decode_json_object(path.read_bytes(), message)
    except (OSError, TypeError, ValueError) as error:
        raise ValueError(message) from error


def _decode_json_object(content: bytes, message: str) -> dict[str, object]:
    def closed_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(message)
            result[key] = value
        return result

    try:
        decoded = content.decode("utf-8", errors="strict")
        value = json.loads(decoded, object_pairs_hook=closed_object)
    except (UnicodeDecodeError, json.JSONDecodeError, TypeError, ValueError) as error:
        raise ValueError(message) from error
    if type(value) is not dict:
        raise ValueError(message)
    return value


def _write_projections(repo_root: Path) -> None:
    staged: dict[ProjectionRisk, bytes] = {}
    for risk in _RISKS:
        projection = build_projection(repo_root, risk)
        content = canonical_projection_bytes(projection)
        if (
            _decode_json_object(content, "generated compatibility projection is invalid")
            != projection
        ):
            raise ValueError("generated compatibility projection is invalid")
        staged[risk] = content

    temporary_paths: list[Path] = []
    try:
        for risk in _RISKS:
            destination = repo_root / _PROJECTION_PATHS[risk]
            destination.parent.mkdir(parents=True, exist_ok=True)
            descriptor, temporary_name = tempfile.mkstemp(
                prefix=f".{destination.name}.",
                dir=destination.parent,
            )
            temporary = Path(temporary_name)
            temporary_paths.append(temporary)
            try:
                with os.fdopen(descriptor, "wb") as stream:
                    stream.write(staged[risk])
                    stream.flush()
                    os.fsync(stream.fileno())
            except BaseException:
                try:
                    os.close(descriptor)
                except OSError:
                    pass
                raise
            os.replace(temporary, destination)
            temporary_paths.remove(temporary)
    finally:
        for temporary in temporary_paths:
            try:
                temporary.unlink()
            except OSError:
                pass


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Check qualification compatibility projections")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--write", action="store_true")
    arguments = parser.parse_args(argv)
    repo_root = Path.cwd()

    if arguments.check:
        try:
            issues = projection_issues(repo_root)
        except (OSError, TypeError, ValueError):
            print("compatibility projections could not be checked", file=sys.stderr)
            return 2
        if any(issues.values()):
            print("compatibility projections are stale", file=sys.stderr)
            for risk in _RISKS:
                for issue in issues[risk]:
                    print(issue, file=sys.stderr)
            return 1
        print("compatibility projections are current")
        return 0

    try:
        _write_projections(repo_root)
    except (OSError, TypeError, ValueError):
        print("compatibility projections could not be written", file=sys.stderr)
        return 2
    print("compatibility projections were written")
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
