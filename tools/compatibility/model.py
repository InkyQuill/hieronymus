"""Typed compatibility manifest loading and invariant validation."""

from __future__ import annotations

import json
import re
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
_TEST_DISPOSITIONS = frozenset({"public_contract", "implementation_internal"})


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
class TestOwnership:
    node_id: str
    disposition: str
    contract_ids: tuple[str, ...] = ()
    reason: str | None = None


@dataclass(frozen=True)
class Manifest:
    manifest_version: int
    python_reference: str
    contracts: tuple[Contract, ...]
    test_ownership: tuple[TestOwnership, ...]


def load_manifest(path: Path) -> Manifest:
    """Load a structurally valid compatibility manifest from *path*."""
    data = json.loads(path.read_text(encoding="utf-8"))
    manifest_data = _mapping(data, "manifest")
    _require_fields(
        manifest_data,
        {"manifest_version", "python_reference", "contracts", "test_ownership"},
        "manifest",
    )

    contracts_data = manifest_data["contracts"]
    if not isinstance(contracts_data, list):
        raise ValueError("manifest contracts must be an array")
    test_ownership_data = manifest_data["test_ownership"]
    if not isinstance(test_ownership_data, list):
        raise ValueError("manifest test_ownership must be an array")

    return Manifest(
        manifest_version=_integer(manifest_data["manifest_version"], "manifest_version"),
        python_reference=_string(manifest_data["python_reference"], "python_reference"),
        contracts=tuple(_load_contract(contract_data) for contract_data in contracts_data),
        test_ownership=tuple(
            _load_test_ownership(ownership_data) for ownership_data in test_ownership_data
        ),
    )


def validate_manifest(manifest: Manifest, repo_root: Path) -> list[str]:
    """Return invariant violations for *manifest* relative to *repo_root*."""
    errors: list[str] = []
    seen_ids: set[str] = set()
    contracts_by_id = {contract.id: contract for contract in manifest.contracts}
    resolved_repo_root = repo_root.resolve()

    for contract in manifest.contracts:
        if contract.id in seen_ids:
            errors.append(f"duplicate contract id: {contract.id}")
        seen_ids.add(contract.id)

        if not contract.acceptance_owner.strip():
            errors.append(f"blank acceptance owner: {contract.id}")
        if not contract.technical_owner.strip():
            errors.append(f"blank technical owner: {contract.id}")

        fixture_error = _referenced_path_error(resolved_repo_root, contract.fixture, "fixture")
        if fixture_error is not None:
            errors.append(fixture_error)
        for test_path in contract.tests:
            test_error = _referenced_path_error(resolved_repo_root, test_path, "test")
            if test_error is not None:
                errors.append(test_error)

        if contract.disposition in {"intentionally-change", "remove"} and not (
            contract.adr and contract.adr.strip()
        ):
            errors.append(f"missing adr for {contract.disposition} contract: {contract.id}")

    seen_node_ids: set[str] = set()
    for ownership in manifest.test_ownership:
        if ownership.node_id in seen_node_ids:
            errors.append(f"duplicate test ownership node id: {ownership.node_id}")
        seen_node_ids.add(ownership.node_id)

        node_file = ownership.node_id.split("::", 1)[0]
        for contract_id in ownership.contract_ids:
            contract = contracts_by_id.get(contract_id)
            if contract is None:
                errors.append(
                    f"unknown contract id for test ownership: {ownership.node_id}: {contract_id}"
                )
            elif node_file not in contract.tests:
                errors.append(
                    "test ownership contract does not own node file: "
                    f"{ownership.node_id}: {contract_id}"
                )

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
    if technical_owner not in _TECHNICAL_OWNERS:
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


def _load_test_ownership(data: object) -> TestOwnership:
    ownership_data = _mapping(data, "test ownership")
    disposition = _enum(
        ownership_data.get("disposition"),
        "test ownership disposition",
        _TEST_DISPOSITIONS,
    )
    required_fields = {"node_id", "disposition"}
    if disposition == "public_contract":
        required_fields.add("contract_ids")
    else:
        required_fields.add("reason")
    _require_fields(ownership_data, required_fields, "test ownership")

    node_id = _string(ownership_data["node_id"], "test ownership node_id")
    if re.fullmatch(r"tests/.+::.+", node_id) is None:
        raise ValueError("test ownership node_id must be a pytest node id under tests/")

    if disposition == "public_contract":
        contract_ids = _strings(ownership_data["contract_ids"], "test ownership contract_ids")
        if not contract_ids:
            raise ValueError("test ownership contract_ids must not be empty")
        if len(set(contract_ids)) != len(contract_ids):
            raise ValueError("test ownership contract_ids must be unique")
        return TestOwnership(
            node_id=node_id,
            disposition=disposition,
            contract_ids=contract_ids,
        )

    reason = _string(ownership_data["reason"], "test ownership reason")
    if not reason.strip():
        raise ValueError("test ownership reason must not be blank")
    return TestOwnership(
        node_id=node_id,
        disposition=disposition,
        reason=reason,
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


def _referenced_path_error(repo_root: Path, reference: str, kind: str) -> str | None:
    resolved_path = (repo_root / reference).resolve()
    if not resolved_path.is_relative_to(repo_root):
        return f"{kind} path outside repository root: {reference}"
    if not resolved_path.is_file():
        return f"missing {kind} path: {reference}"
    return None


def _enum(value: object, name: str, values: frozenset[str]) -> str:
    parsed_value = _string(value, name)
    if parsed_value not in values:
        raise ValueError(f"{name} must be one of: {', '.join(sorted(values))}")
    return parsed_value
