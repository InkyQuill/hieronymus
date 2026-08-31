from __future__ import annotations

import json
from pathlib import Path

from tools.compatibility.inventory_mcp import snapshot_mcp
from tools.compatibility.model import load_manifest

ROOT = Path(__file__).resolve().parents[2]


def test_mcp_snapshot_matches_fastmcp_registry() -> None:
    snapshot = snapshot_mcp()
    expected = json.loads((ROOT / "compatibility/snapshots/mcp.json").read_text(encoding="utf-8"))

    assert snapshot == expected
    assert snapshot["protocol_revision"] == "2026-07-28"
    assert snapshot["transports"] == ["stdio", "streamable-http"]
    assert len(snapshot["tools"]) == snapshot["derived_tool_count"]


def test_every_registered_tool_has_manifest_contract_and_complete_fixtures() -> None:
    snapshot = snapshot_mcp()
    tool_names = {str(tool["name"]) for tool in snapshot["tools"]}
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    mcp_contracts = {
        contract.id.removeprefix("mcp.tool."): contract
        for contract in manifest.contracts
        if contract.id.startswith("mcp.tool.")
    }

    assert set(mcp_contracts) == tool_names
    for tool_name in tool_names:
        fixture_dir = ROOT / "compatibility/fixtures/mcp" / tool_name
        success_input = fixture_dir / "success.input.json"
        success_output = fixture_dir / "success.output.json"
        error_output = fixture_dir / "error.output.json"
        contract = mcp_contracts[tool_name]

        assert contract.surface == "mcp"
        assert contract.acceptance_owner == "Pavel Obruchnikov <me@inkyquill.net>"
        assert contract.technical_owner == "daemon-mcp-security"
        assert contract.python_entry_point == f"hieronymus.mcp_server:{tool_name}"
        assert contract.fixture == str(success_input.relative_to(ROOT))
        assert contract.rust_test_target == (
            f"crates/hiero-mcp/tests/registry_contract.rs::{tool_name}"
        )
        assert contract.disposition == "preserve"
        assert "tests/compatibility/test_mcp_inventory.py" in contract.tests
        assert len(contract.tests) > 1
        assert json.loads(success_input.read_text(encoding="utf-8")) is not None
        assert json.loads(success_output.read_text(encoding="utf-8")) is not None
        error = json.loads(error_output.read_text(encoding="utf-8"))
        assert set(error) == {"error"}
        assert set(error["error"]) == {"message", "type"}


def test_protocol_fixture_records_adr_pinned_boundary_without_manifest_duplication() -> None:
    snapshot = snapshot_mcp()
    protocol = json.loads(
        (ROOT / "compatibility/fixtures/mcp/protocol.json").read_text(encoding="utf-8")
    )
    manifest = load_manifest(ROOT / "compatibility/manifest.json")

    assert protocol == {
        "protocol_revision": "2026-07-28",
        "transports": ["stdio", "streamable-http"],
        "private_python_bridge": {
            "path": "/api/mcp/{operation}",
            "classification": "private_python_bridge",
            "disposition": "remove",
            "adr": "docs/adr/0015-mcp-protocol-and-transport.md",
        },
    }
    assert snapshot["private_python_bridge"] == protocol["private_python_bridge"]
    assert all(
        contract.python_entry_point != "/api/mcp/{operation}" for contract in manifest.contracts
    )
