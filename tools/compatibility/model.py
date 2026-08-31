"""Typed compatibility manifest loading and invariant validation."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

_SURFACES = frozenset(
    {
        "cli",
        "mcp",
        "http",
        "websocket",
        "config",
        "database",
        "agent-integration",
        "frontend",
        "install-update",
        "diagnostics",
    }
)
_TECHNICAL_OWNERS = frozenset(
    {
        "data-config",
        "database-upgrade",
        "terminology-memory",
        "dreaming",
        "semantic-rag",
        "daemon-mcp-security",
        "distribution-cutover",
    }
)
_DISPOSITIONS = frozenset({"preserve", "intentionally-change", "remove"})


@dataclass(frozen=True)
class Contract:
    id: str
    surface: str
    acceptance_owner: str
    technical_owner: str
    python_entry_point: str
    tests: tuple[str, ...]
    fixture: str
    rust_test_target: str
    disposition: str
    adr: str | None = None


@dataclass(frozen=True)
class Manifest:
    manifest_version: int
    python_reference: str
    contracts: tuple[Contract, ...]


def load_manifest(path: Path) -> Manifest:
    """Load a structurally valid compatibility manifest from *path*."""
    data = json.loads(path.read_text(encoding="utf-8"))
    manifest_data = _mapping(data, "manifest")
    _require_fields(
        manifest_data,
        {"manifest_version", "python_reference", "contracts"},
        "manifest",
    )

    contracts_data = manifest_data["contracts"]
    if not isinstance(contracts_data, list):
        raise ValueError("manifest contracts must be an array")

    return Manifest(
        manifest_version=_integer(manifest_data["manifest_version"], "manifest_version"),
        python_reference=_string(manifest_data["python_reference"], "python_reference"),
        contracts=tuple(_load_contract(contract_data) for contract_data in contracts_data),
    )


def validate_manifest(manifest: Manifest, repo_root: Path) -> list[str]:
    """Return invariant violations for *manifest* relative to *repo_root*."""
    errors: list[str] = []
    seen_ids: set[str] = set()

    for contract in manifest.contracts:
        if contract.id in seen_ids:
            errors.append(f"duplicate contract id: {contract.id}")
        seen_ids.add(contract.id)

        if not contract.acceptance_owner.strip():
            errors.append(f"blank acceptance owner: {contract.id}")
        if not contract.technical_owner.strip():
            errors.append(f"blank technical owner: {contract.id}")

        fixture_path = repo_root / contract.fixture
        if not fixture_path.is_file():
            errors.append(f"missing fixture path: {contract.fixture}")
        for test_path in contract.tests:
            if not (repo_root / test_path).is_file():
                errors.append(f"missing test path: {test_path}")

        if contract.disposition in {"intentionally-change", "remove"} and not (
            contract.adr and contract.adr.strip()
        ):
            errors.append(f"missing adr for {contract.disposition} contract: {contract.id}")

    return errors


def _load_contract(data: object) -> Contract:
    contract_data = _mapping(data, "contract")
    required_fields = {
        "id",
        "surface",
        "acceptance_owner",
        "technical_owner",
        "python_entry_point",
        "tests",
        "fixture",
        "rust_test_target",
        "disposition",
    }
    _require_fields(contract_data, required_fields, "contract", optional_fields={"adr"})

    surface = _enum(contract_data["surface"], "surface", _SURFACES)
    technical_owner = _string(contract_data["technical_owner"], "technical_owner")
    if technical_owner.strip() and technical_owner not in _TECHNICAL_OWNERS:
        raise ValueError(f"technical_owner must be one of: {', '.join(sorted(_TECHNICAL_OWNERS))}")

    adr = contract_data.get("adr")
    if adr is not None:
        adr = _string(adr, "adr")

    return Contract(
        id=_string(contract_data["id"], "id"),
        surface=surface,
        acceptance_owner=_string(contract_data["acceptance_owner"], "acceptance_owner"),
        technical_owner=technical_owner,
        python_entry_point=_string(contract_data["python_entry_point"], "python_entry_point"),
        tests=_strings(contract_data["tests"], "tests"),
        fixture=_string(contract_data["fixture"], "fixture"),
        rust_test_target=_string(contract_data["rust_test_target"], "rust_test_target"),
        disposition=_enum(contract_data["disposition"], "disposition", _DISPOSITIONS),
        adr=adr,
    )


def _mapping(value: object, name: str) -> dict[str, object]:
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value):
        raise ValueError(f"{name} must be an object")
    return value


def _require_fields(
    data: dict[str, object],
    required_fields: set[str],
    name: str,
    *,
    optional_fields: set[str] | None = None,
) -> None:
    missing = required_fields - data.keys()
    if missing:
        raise ValueError(f"{name} missing required fields: {', '.join(sorted(missing))}")

    unexpected = data.keys() - required_fields - (optional_fields or set())
    if unexpected:
        raise ValueError(f"{name} has unexpected fields: {', '.join(sorted(unexpected))}")


def _integer(value: object, name: str) -> int:
    if type(value) is not int:
        raise ValueError(f"{name} must be an integer")
    return value


def _string(value: object, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _strings(value: object, name: str) -> tuple[str, ...]:
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise ValueError(f"{name} must be an array of strings")
    return tuple(value)


def _enum(value: object, name: str, values: frozenset[str]) -> str:
    parsed_value = _string(value, name)
    if parsed_value not in values:
        raise ValueError(f"{name} must be one of: {', '.join(sorted(values))}")
    return parsed_value
