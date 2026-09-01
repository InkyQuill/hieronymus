from __future__ import annotations

import asyncio
import json
from pathlib import Path

from hieronymus import mcp_server
from hieronymus.config import HieronymusConfig
from tools.compatibility.inventory_mcp import _call_real_server, _replace_root, snapshot_mcp
from tools.compatibility.model import load_manifest

ROOT = Path(__file__).resolve().parents[2]


def _successful_result_envelopes(
    target: dict[str, object],
) -> tuple[dict[str, object], ...]:
    stdio = target["stdio"]
    http = target["streamable_http"]
    assert isinstance(stdio, dict)
    assert isinstance(http, dict)
    stdio_exchanges = stdio["exchanges"]
    http_exchanges = http["exchanges"]
    assert isinstance(stdio_exchanges, list)
    assert isinstance(http_exchanges, list)
    return (
        *(json.loads(exchange["response_line"]) for exchange in stdio_exchanges),
        *(exchange["responses"][0]["body"] for exchange in http_exchanges),
        *(exchange["responses"][1]["events"][-1]["data"] for exchange in http_exchanges),
    )


def test_mcp_snapshot_matches_fastmcp_registry() -> None:
    snapshot = snapshot_mcp()
    expected = json.loads((ROOT / "compatibility/snapshots/mcp.json").read_text(encoding="utf-8"))

    assert snapshot == expected
    assert snapshot["protocol_revision"] == "2026-07-28"
    assert snapshot["transports"] == ["stdio", "streamable-http"]
    assert len(snapshot["tools"]) == snapshot["derived_tool_count"]


def test_every_registered_tool_has_manifest_contract_and_complete_fixtures(
    tmp_path: Path,
) -> None:
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
        error_input = fixture_dir / "error.input.json"
        error_output = fixture_dir / "error.output.json"
        wire_success = fixture_dir / "wire.success.json"
        wire_error = fixture_dir / "wire.error.json"
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
        success_arguments = json.loads(success_input.read_text(encoding="utf-8"))
        assert success_arguments is not None
        assert json.loads(success_output.read_text(encoding="utf-8")) is not None
        error_case = json.loads(error_input.read_text(encoding="utf-8"))
        assert error_case["params"]["name"] == tool_name
        assert set(error_case) == {"params", "setup"}
        assert error_case["setup"]
        error = json.loads(error_output.read_text(encoding="utf-8"))
        assert set(error) == {"error"}
        assert set(error["error"]) == {"message", "type"}
        success_envelope = json.loads(wire_success.read_text(encoding="utf-8"))
        error_envelope = json.loads(wire_error.read_text(encoding="utf-8"))
        assert success_envelope["request"]["method"] == "tools/call"
        assert success_envelope["request"]["params"]["name"] == tool_name
        assert success_envelope["result"]["isError"] is False
        assert "content" in success_envelope["result"]
        assert "structuredContent" in success_envelope["result"]
        assert error_envelope["request"]["params"] == error_case["params"]
        assert error_envelope["result"]["isError"] is True
        assert error_envelope["result"]["content"]

        replay_root = tmp_path / tool_name
        config = HieronymusConfig(data_root=replay_root / "data")
        if error_case["setup"] == {"data_root": "file"}:
            config.data_root.parent.mkdir(parents=True, exist_ok=True)
            config.data_root.write_text("not a directory\n", encoding="utf-8")
        replayed_result = _replace_root(
            _call_real_server(config, tool_name, error_case["params"]["arguments"]),
            replay_root,
        )
        assert replayed_result == error_envelope["result"]
        assert error == {
            "error": {
                "message": replayed_result["content"][0]["text"],
                "type": "MCPToolError",
            }
        }


def test_protocol_fixture_is_stateless_2026_07_28() -> None:
    snapshot = snapshot_mcp()
    protocol = json.loads(
        (ROOT / "compatibility/fixtures/mcp/protocol.json").read_text(encoding="utf-8")
    )
    manifest = load_manifest(ROOT / "compatibility/manifest.json")

    target = protocol["target"]
    compact = json.dumps(protocol, sort_keys=True, separators=(",", ":"))
    assert '"initialize":' not in compact
    assert "notifications/initialized" not in compact
    assert "Mcp-Session-Id" not in compact
    assert {"initialize", "initialized", "session"}.isdisjoint(target)
    for request in target["requests"]:
        assert {"protocolVersion", "clientCapabilities", "clientInfo"}.isdisjoint(request["params"])
        meta = request["params"]["_meta"]
        assert meta["io.modelcontextprotocol/protocolVersion"] == "2026-07-28"
        assert meta["io.modelcontextprotocol/clientCapabilities"] == {}
        assert meta["io.modelcontextprotocol/clientInfo"] == {
            "name": "compatibility-replay",
            "version": "1.0.0",
        }
    assert target["metadata_rules"] == {
        "required": [
            "io.modelcontextprotocol/protocolVersion",
            "io.modelcontextprotocol/clientCapabilities",
        ],
        "should": ["io.modelcontextprotocol/clientInfo"],
    }
    stdio_exchanges = target["stdio"]["exchanges"]
    assert len(stdio_exchanges) == 2
    for exchange in stdio_exchanges:
        assert exchange["request_line"].endswith("\n")
        assert exchange["response_line"].endswith("\n")
        assert "\n" not in exchange["request_line"][:-1]
        assert "\n" not in exchange["response_line"][:-1]
    assert target["stdio"]["diagnostics_stream"] == "stderr"
    http_exchanges = {
        exchange["request"]["body"]["method"]: exchange
        for exchange in target["streamable_http"]["exchanges"]
    }
    assert set(http_exchanges) == {"tools/list", "tools/call"}
    list_headers = http_exchanges["tools/list"]["request"]["headers"]
    assert list_headers["Mcp-Method"] == "tools/list"
    assert "Mcp-Name" not in list_headers
    call_headers = http_exchanges["tools/call"]["request"]["headers"]
    assert call_headers["Mcp-Method"] == "tools/call"
    assert call_headers["Mcp-Name"] == "hieronymus_status"
    assert all(
        "Mcp-Session-Id" not in exchange["request"]["headers"]
        for exchange in http_exchanges.values()
    )
    assert target["streamable_http"]["header_rules"] == {
        "required": ["MCP-Protocol-Version", "Mcp-Method"],
        "mcp_name_required_for": ["tools/call", "resources/read", "prompts/get"],
    }
    successes = _successful_result_envelopes(target)
    assert len(successes) == 6
    assert all(envelope["result"]["resultType"] == "complete" for envelope in successes)

    from tools.compatibility.check import _request_metadata_issues

    without_client_info = json.loads(json.dumps(target["requests"][0]))
    del without_client_info["params"]["_meta"]["io.modelcontextprotocol/clientInfo"]
    assert _request_metadata_issues(without_client_info) == ()
    missing_capabilities = json.loads(json.dumps(without_client_info))
    del missing_capabilities["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]
    assert _request_metadata_issues(missing_capabilities)
    direct_legacy = json.loads(json.dumps(without_client_info))
    direct_legacy["params"]["protocolVersion"] = "2026-07-28"
    assert _request_metadata_issues(direct_legacy)

    assert (
        protocol["registry_identity"]["stdio"] == protocol["registry_identity"]["streamable_http"]
    )
    assert protocol["registry_identity"]["stdio"] == [tool["name"] for tool in snapshot["tools"]]
    assert protocol["current"]["basis"] == "current-python-server"
    current_tools = asyncio.run(mcp_server.server.list_tools())
    exact_current_envelope = {
        "jsonrpc": "2.0",
        "id": 2,
        "result": {
            "tools": [
                tool.model_dump(mode="json", by_alias=True, exclude_none=True)
                for tool in current_tools
            ]
        },
    }
    assert protocol["current"]["tools_list"] == {
        "request": {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        "response": exact_current_envelope,
    }
    snapshot_registry = {tool["name"]: tool["input_schema"] for tool in snapshot["tools"]}
    current_registry = {
        tool["name"]: tool["inputSchema"]
        for tool in protocol["current"]["tools_list"]["response"]["result"]["tools"]
    }
    target_registry = {
        tool["name"]: tool["input_schema"]
        for tool in target["tools_list"]["response"]["result"]["tools"]
    }
    assert current_registry == target_registry == snapshot_registry
    assert protocol["private_python_bridge"] == {
        "path": "/api/mcp/{operation}",
        "classification": "private_python_bridge",
        "disposition": "remove",
        "adr": "docs/adr/0015-mcp-protocol-and-transport.md",
    }
    assert snapshot["private_python_bridge"] == protocol["private_python_bridge"]
    assert all(
        contract.python_entry_point != "/api/mcp/{operation}" for contract in manifest.contracts
    )
