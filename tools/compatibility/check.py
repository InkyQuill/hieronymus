"""Check every frozen Python compatibility artifact without updating it."""

from __future__ import annotations

import json
import os
import sys
import tempfile
from collections import Counter
from collections.abc import Iterator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import TextIO

from tools.compatibility import inventory_cli, inventory_http, inventory_mcp, inventory_state
from tools.compatibility.model import Manifest, load_manifest, validate_manifest

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path("compatibility/manifest.json")

_FAMILY_SURFACES = {
    "cli": frozenset({"cli"}),
    "mcp": frozenset({"mcp"}),
    "http/frontend": frozenset({"http", "websocket", "frontend"}),
    "state": frozenset(
        {"config", "database", "agent-integration", "install-update", "diagnostics"}
    ),
}


@dataclass(frozen=True)
class GeneratedInventory:
    """In-memory bytes and reviewed item ids regenerated from shipping Python."""

    artifacts: Mapping[str, bytes]
    inventory_ids: Mapping[str, set[str]]


def _json_bytes(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def _state_json_bytes(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def _add_artifact(artifacts: dict[str, bytes], relative_path: str, content: bytes) -> None:
    previous = artifacts.setdefault(relative_path, content)
    if previous != content:
        raise ValueError(f"generators disagree about artifact: {relative_path}")


def _cli_inventory(
    repo_root: Path,
) -> tuple[dict[str, bytes], set[str], list[dict[str, object]]]:
    snapshot = inventory_cli.snapshot_cli(repo_root)
    artifacts = {"compatibility/snapshots/cli.json": _json_bytes(snapshot)}
    records: list[dict[str, object]] = []

    script_contracts = snapshot["script_contracts"]
    commands = snapshot["commands"]
    scripts = snapshot["scripts"]
    assert isinstance(script_contracts, dict)
    assert isinstance(commands, dict)
    assert isinstance(scripts, dict)

    roots = {
        "hiero": inventory_cli.cli_main,
        "hieronymus": inventory_cli.cli_main,
        "hieronymus-agent-hook": inventory_cli.agent_hook_main,
    }
    for script_name, command in roots.items():
        record = script_contracts[script_name]
        assert isinstance(record, dict)
        records.append(record)
        success_code, success_output = inventory_cli._render_case(
            command,
            [str(argument) for argument in record["success_args"]],
            script_name,
        )
        failure_code, failure_output = inventory_cli._render_case(
            command,
            [str(argument) for argument in record["failure_args"]],
            script_name,
        )
        if (success_code, failure_code) != (0, 2):
            raise RuntimeError(f"unexpected Click exit behavior for {script_name}")
        artifacts[str(record["success_fixture"])] = success_output.encode("utf-8")
        artifacts[str(record["failure_fixture"])] = failure_output.encode("utf-8")

    mcp_entrypoint = script_contracts["hieronymus-mcp"]
    assert isinstance(mcp_entrypoint, dict)
    records.append(mcp_entrypoint)
    success = inventory_cli.replay_mcp_entrypoint_case("success")
    failure = inventory_cli.replay_mcp_entrypoint_case("failure")
    if (success["exit_code"], failure["exit_code"]) != (0, 1):
        raise RuntimeError("unexpected replayed MCP entrypoint exit behavior")
    artifacts[str(mcp_entrypoint["success_fixture"])] = _json_bytes(success)
    artifacts[str(mcp_entrypoint["failure_fixture"])] = _json_bytes(failure)

    for script_name, rows in commands.items():
        entry_point = scripts[script_name]
        group = inventory_cli._ENTRY_POINT_GROUPS[entry_point]
        assert isinstance(rows, list)
        for row in rows:
            assert isinstance(row, dict)
            records.append(row)
            command = inventory_cli._resolve_command(group, str(row["path"]).split())
            program_name = " ".join(str(part) for part in row["invocation"])
            success_code, success_output = inventory_cli._render_case(
                command,
                [str(argument) for argument in row["success_args"]],
                program_name,
            )
            failure_code, failure_output = inventory_cli._render_case(
                command,
                [str(argument) for argument in row["failure_args"]],
                program_name,
            )
            if (success_code, failure_code) != (0, 2):
                raise RuntimeError(f"unexpected Click exit behavior for {program_name}")
            artifacts[str(row["success_fixture"])] = success_output.encode("utf-8")
            artifacts[str(row["failure_fixture"])] = failure_output.encode("utf-8")

    for record in records:
        contract = {
            "contract_id": record["contract_id"],
            "success": {
                "args": record["success_args"],
                "fixture": record["success_fixture"],
                "exit_code": 0,
            },
            "failure": {
                "args": record["failure_args"],
                "fixture": record["failure_fixture"],
                "exit_code": record["exit_behavior"].get(
                    "usage_error", record["exit_behavior"].get("failure")
                ),
            },
        }
        if "success_environment" in record:
            contract["success"]["invocation"] = record["success_invocation"]
            contract["failure"]["invocation"] = record["failure_invocation"]
            contract["success"]["environment"] = record["success_environment"]
            contract["failure"]["environment"] = record["failure_environment"]
        artifacts[str(record["fixture_contract"])] = _json_bytes(contract)

    manifest_contracts = sorted(
        inventory_cli._manifest_contracts(snapshot), key=lambda contract: str(contract["id"])
    )
    return (
        artifacts,
        {str(record["contract_id"]) for record in records},
        manifest_contracts,
    )


def _mcp_inventory() -> tuple[dict[str, bytes], set[str], list[dict[str, object]]]:
    snapshot = inventory_mcp.snapshot_mcp()
    artifacts = {"compatibility/snapshots/mcp.json": _json_bytes(snapshot)}
    protocol = {
        "protocol_revision": snapshot["protocol_revision"],
        "transports": snapshot["transports"],
        "private_python_bridge": snapshot["private_python_bridge"],
    }
    artifacts["compatibility/fixtures/mcp/protocol.json"] = _json_bytes(protocol)

    tools = snapshot["tools"]
    assert isinstance(tools, list)
    inventory_ids: set[str] = set()
    manifest_contracts: list[dict[str, object]] = []
    for tool in tools:
        tool_name = str(tool["name"])
        inventory_ids.add(f"mcp.tool.{tool_name}")
        manifest_contracts.append(
            {
                "id": f"mcp.tool.{tool_name}",
                "surface": "mcp",
                "acceptance_owner": "Pavel Obruchnikov <me@inkyquill.net>",
                "technical_owner": "daemon-mcp-security",
                "python_entry_point": f"hieronymus.mcp_server:{tool_name}",
                "tests": inventory_mcp._behavior_tests(tool_name),
                "fixture": f"compatibility/fixtures/mcp/{tool_name}/success.input.json",
                "rust_test_target": (f"crates/hiero-mcp/tests/registry_contract.rs::{tool_name}"),
                "disposition": "preserve",
            }
        )
        with tempfile.TemporaryDirectory(prefix="hieronymus-mcp-check-") as directory:
            success_input, success_output = inventory_mcp._fixture_case(tool_name, Path(directory))
        fixture_root = f"compatibility/fixtures/mcp/{tool_name}"
        artifacts[f"{fixture_root}/success.input.json"] = _json_bytes(success_input)
        artifacts[f"{fixture_root}/success.output.json"] = _json_bytes(success_output)
        artifacts[f"{fixture_root}/error.output.json"] = _json_bytes(inventory_mcp._DATA_ROOT_ERROR)
    return artifacts, inventory_ids, manifest_contracts


def _http_inventory(
    repo_root: Path,
) -> tuple[dict[str, bytes], set[str], list[dict[str, object]]]:
    snapshot = inventory_http.snapshot_http(repo_root)
    artifacts = {
        "compatibility/snapshots/http.json": _json_bytes(snapshot),
        "compatibility/fixtures/http/route-cases.json": _json_bytes(
            inventory_http._route_cases(snapshot)
        ),
    }
    routes = snapshot["routes"]
    assert isinstance(routes, list)
    return (
        artifacts,
        {str(route["contract_id"]) for route in routes},
        [inventory_http._manifest_contract(route) for route in routes],
    )


@contextmanager
def _collection_without_repository_cache() -> Iterator[None]:
    previous_addopts = os.environ.get("PYTEST_ADDOPTS")
    previous_bytecode = os.environ.get("PYTHONDONTWRITEBYTECODE")
    addopts = previous_addopts.split() if previous_addopts else []
    if "no:cacheprovider" not in addopts:
        addopts.extend(["-p", "no:cacheprovider"])
    os.environ["PYTEST_ADDOPTS"] = " ".join(addopts)
    os.environ["PYTHONDONTWRITEBYTECODE"] = "1"
    try:
        yield
    finally:
        if previous_addopts is None:
            os.environ.pop("PYTEST_ADDOPTS", None)
        else:
            os.environ["PYTEST_ADDOPTS"] = previous_addopts
        if previous_bytecode is None:
            os.environ.pop("PYTHONDONTWRITEBYTECODE", None)
        else:
            os.environ["PYTHONDONTWRITEBYTECODE"] = previous_bytecode


def _state_inventory(
    repo_root: Path,
) -> tuple[dict[str, bytes], set[str], list[dict[str, object]]]:
    with tempfile.TemporaryDirectory(prefix="hieronymus-state-check-") as directory:
        with _collection_without_repository_cache():
            artifacts = inventory_state.generate_state_artifacts(
                repo_root, Path(directory) / "data-root"
            )
    artifacts.pop(str(MANIFEST))
    contracts = inventory_state._state_contracts()
    return artifacts, {str(contract["id"]) for contract in contracts}, contracts


def generate_inventory(repo_root: Path) -> GeneratedInventory:
    """Regenerate every checked artifact in memory or in isolated state roots."""
    artifacts: dict[str, bytes] = {}
    inventory_ids: dict[str, set[str]] = {}
    transport_contracts: list[dict[str, object]] = []
    state_contracts: list[dict[str, object]] = []
    generators = (
        ("cli", lambda: _cli_inventory(repo_root)),
        ("mcp", _mcp_inventory),
        ("http/frontend", lambda: _http_inventory(repo_root)),
        ("state", lambda: _state_inventory(repo_root)),
    )
    for family, generate in generators:
        family_artifacts, family_ids, family_contracts = generate()
        for relative_path, content in family_artifacts.items():
            _add_artifact(artifacts, relative_path, content)
        inventory_ids[family] = family_ids
        if family == "state":
            state_contracts.extend(family_contracts)
        else:
            transport_contracts.extend(family_contracts)

    contracts = [
        *sorted(transport_contracts, key=lambda contract: str(contract["id"])),
        *state_contracts,
    ]

    manifest_source = json.loads((repo_root / MANIFEST).read_text(encoding="utf-8"))
    state_snapshot = json.loads(artifacts["compatibility/snapshots/state.json"])
    expected_manifest = {
        "manifest_version": manifest_source["manifest_version"],
        "python_reference": manifest_source["python_reference"],
        "contracts": contracts,
        "test_ownership": inventory_state.build_test_ownership(
            state_snapshot["tests"]["node_ids"], contracts
        ),
    }
    artifacts[str(MANIFEST)] = _state_json_bytes(expected_manifest)
    artifacts["compatibility/fixtures/diagnostics/check-success.txt"] = (
        inventory_state._render_compatibility_success(expected_manifest).encode()
    )
    return GeneratedInventory(dict(sorted(artifacts.items())), inventory_ids)


def artifact_diffs(repo_root: Path, expected_artifacts: Mapping[str, bytes]) -> list[str]:
    """Return byte-exact drift diagnostics for generated artifacts."""
    failures: list[str] = []
    for relative_path in sorted(expected_artifacts):
        checked_path = repo_root / relative_path
        if not checked_path.is_file():
            failures.append(f"missing generated artifact: {relative_path}")
            continue
        if checked_path.read_bytes() == expected_artifacts[relative_path]:
            continue
        if relative_path.startswith("compatibility/snapshots/"):
            kind = "snapshot"
        elif relative_path == str(MANIFEST):
            kind = "manifest"
        else:
            kind = "fixture"
        failures.append(f"{kind} drift: {relative_path}")
    return failures


def manifest_failures(repo_root: Path) -> tuple[Manifest | None, list[str]]:
    """Load the checked manifest and return stable structural diagnostics."""
    path = repo_root / MANIFEST
    try:
        manifest = load_manifest(path)
    except FileNotFoundError:
        return None, [f"missing manifest: {MANIFEST}"]
    except (json.JSONDecodeError, ValueError) as error:
        return None, [f"invalid manifest: {error}"]
    return manifest, sorted(validate_manifest(manifest, repo_root))


def inventory_coverage_failures(
    manifest: Manifest,
    inventory_ids: Mapping[str, set[str]],
) -> list[str]:
    """Reject inventory items that lack a reviewed manifest disposition."""
    failures: list[str] = []
    for family, surfaces in _FAMILY_SURFACES.items():
        reviewed = {contract.id for contract in manifest.contracts if contract.surface in surfaces}
        current = inventory_ids.get(family, set())
        for contract_id in current - reviewed:
            failures.append(f"unreviewed inventory item: {family}: {contract_id}")
        for contract_id in reviewed - current:
            failures.append(f"manifest item absent from inventory: {family}: {contract_id}")
    return sorted(failures)


def _count_lines(values: list[str]) -> list[str]:
    return [f"  {name}: {count}" for name, count in sorted(Counter(values).items())]


def render_parity_summary(manifest: Manifest) -> str:
    """Render deterministic counts derived only from the current manifest."""
    lines = [
        "Parity summary",
        f"Contracts: {len(manifest.contracts)}",
        "By surface:",
        *_count_lines([contract.surface for contract in manifest.contracts]),
        "By disposition:",
        *_count_lines([contract.disposition for contract in manifest.contracts]),
        "By technical owner:",
        *_count_lines([contract.technical_owner for contract in manifest.contracts]),
        f"Test ownership: {len(manifest.test_ownership)}",
        "By test-ownership disposition:",
        *_count_lines([ownership.disposition for ownership in manifest.test_ownership]),
    ]
    return "\n".join(lines)


def emit_report(failures: list[str], summary: str | None, output: TextIO) -> None:
    """Print failures in stable order followed by the available parity summary."""
    if failures:
        print("Compatibility check failed", file=output)
        for failure in sorted(set(failures)):
            print(failure, file=output)
    else:
        print("Compatibility check passed", file=output)
    if summary is not None:
        print(summary, file=output)


def main(*, repo_root: Path = ROOT, output: TextIO = sys.stdout) -> int:
    """Run the complete read-only compatibility check."""
    resolved_root = repo_root.resolve()
    manifest, failures = manifest_failures(resolved_root)
    try:
        generated = generate_inventory(resolved_root)
    except Exception as error:  # noqa: BLE001 - command must turn generator failures into a report
        failures.append(f"inventory generation failed: {type(error).__name__}: {error}")
    else:
        failures.extend(artifact_diffs(resolved_root, generated.artifacts))
        if manifest is not None:
            failures.extend(inventory_coverage_failures(manifest, generated.inventory_ids))

    summary = render_parity_summary(manifest) if manifest is not None else None
    emit_report(failures, summary, output)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
