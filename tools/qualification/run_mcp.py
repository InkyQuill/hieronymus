"""Offline MCP transport qualification runner and canonical evidence writer."""

# ruff: noqa: E501 -- replay commands are byte-exact validation allow-list entries.

from __future__ import annotations

import argparse
import http.client
import json
import os
import platform
import re
import selectors
import signal
import stat
import subprocess
import sys
import threading
import time
import tomllib
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import cast

from tools.compatibility.mcp_schema import authority_issues, definition_issues
from tools.qualification.fingerprint import (
    _MCP_TOOL_FIXTURE_LEAVES,
    _MCP_TOOL_NAMES,
    fingerprint_inputs,
    required_fingerprint_inputs,
)
from tools.qualification.model import (
    REQUIRED_CRITERIA,
    CleanupEvidence,
    Environment,
    Evidence,
    LockedDependency,
    Measurements,
    QualificationRecord,
    Review,
    decision_for,
    expected_consequence,
    serialize_record,
    status_for,
)
from tools.qualification.process import (
    ProcessReceipt,
    ToolRoots,
    discover_tool_roots,
    run_owned_process,
    safe_subprocess_env,
)
from tools.qualification.projections import MCP_EXACT_CONTRACT_IDS, projection_issues
from tools.qualification.render import render_record

_RISK = "mcp-transport"
_OWNER = "Pavel Obruchnikov <me@inkyquill.net>"
_TARGET = "x86_64-unknown-linux-gnu"
_MANIFEST = "qualification/harnesses/mcp-transport/Cargo.toml"
_CARGO_TARGET = Path("qualification/.artifacts/cargo-target/mcp-transport")
_LIVE_WORK = Path("qualification/.artifacts/work/mcp-transport")
_RECORD = Path("qualification/records/mcp-transport.json")
_MARKDOWN = Path("docs/qualification/rust/mcp-transport.md")
_COMMANDS = (
    "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_mcp --write",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 metadata --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --format-version 1",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 tree --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked -e features",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --check",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu -- -D warnings",
)
_SPECS = (
    "docs/adr/0015-mcp-protocol-and-transport.md",
    "docs/superpowers/specs/2026-08-31-rust-daemon-mcp-security-design.md",
)
_DEFINITION_BY_ID = {
    1: "ListToolsResultResponse",
    2: "CallToolResultResponse",
}
_HEADER_FAILURE_IDS = {
    "missing-version",
    "protocol-version-header-mismatch",
    "missing-mcp-method",
    "wrong-mcp-method",
    "unexpected-mcp-name-tools-list",
    "missing-mcp-name-tools-call",
    "wrong-mcp-name-tools-call",
}
_ADDRESS = re.compile(r"^127\.0\.0\.1:([1-9][0-9]{0,4})$")
_MAX_CAPTURE = 2 * 1024 * 1024
_PROBE_SCHEMA_VERSION = 1
_PROBE_MARKER = "HIERONYMUS_QUALIFICATION_OWNED_PROBE"


class CandidateMismatch(ValueError):
    """A bounded candidate response that directly disproves the contract."""


class CandidateTimeout(CandidateMismatch):
    """A candidate that did not complete within its fixed observation window."""


class CandidateOutput(CandidateMismatch):
    """A candidate that exceeded the fixed output bound."""


_CRITERION_OBSERVATIONS: Mapping[str, tuple[str, ...]] = {
    "locked-native-build": ("locked-toolchain",),
    "protocol-2026-07-28": ("canonical-stdio", "canonical-http", "metadata-negatives"),
    "no-handshake-or-session": ("stateless-negatives",),
    "per-request-required-metadata": ("metadata-negatives", "reserved-metadata"),
    "unsupported-version-rejected": ("unsupported-version",),
    "stdio-newline-jsonrpc": ("canonical-stdio", "error-stdio"),
    "streamable-http-json": ("canonical-http", "error-http"),
    "streamable-http-sse": ("canonical-http", "error-http"),
    "http-method-name-headers": ("route-header-negatives",),
    "http-host-auth-version-cases": ("route-security-negatives",),
    "official-schema-envelopes": (
        "canonical-stdio",
        "canonical-http",
        "error-stdio",
        "error-http",
    ),
    "header-mismatch-errors": ("route-header-negatives",),
    "registry-identity": ("canonical-stdio", "canonical-http"),
    "result-error-identity": ("error-stdio", "error-http"),
    "result-type-required": (
        "canonical-stdio",
        "canonical-http",
        "error-stdio",
        "error-http",
    ),
    "required-auth-metadata": (
        "metadata-negatives",
        "legacy-metadata-negative",
        "client-info-cases",
        "reserved-metadata",
    ),
    "private-bridge-absent": ("private-bridge-negative",),
}


@dataclass(frozen=True, slots=True)
class _ProbeObservations:
    checks: Mapping[str, tuple[bool, Mapping[str, object]]]
    environment: Mapping[str, str]
    dependencies: tuple[LockedDependency, ...]


def _read_object(path: Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_bytes())
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("MCP qualification input is invalid") from error
    if type(value) is not dict:
        raise ValueError("MCP qualification input must be an object")
    return value


def _mcp_projection(repo_root: Path) -> tuple[str, ...]:
    issues = projection_issues(repo_root)[_RISK]
    if issues:
        raise ValueError("MCP compatibility projection is stale")
    projection = _read_object(repo_root / "qualification/compatibility/mcp-transport.json")
    contracts = projection.get("contracts")
    if type(contracts) is not list or len(contracts) != 42:
        raise ValueError("MCP compatibility projection contract inventory is invalid")
    ids: list[str] = []
    for contract in contracts:
        if type(contract) is not dict or type(contract.get("id")) is not str:
            raise ValueError("MCP compatibility projection contract is invalid")
        contract_id = cast(str, contract["id"])
        ids.append(contract_id)
        if contract_id.startswith("mcp.tool."):
            name = contract_id.removeprefix("mcp.tool.")
            if contract.get("fixture") != (f"compatibility/fixtures/mcp/{name}/success.input.json"):
                raise ValueError("MCP compatibility projection fixture is invalid")
    expected = {
        *MCP_EXACT_CONTRACT_IDS,
        *(f"mcp.tool.{name}" for name in _MCP_TOOL_NAMES),
    }
    if set(ids) != expected or ids != sorted(ids):
        raise ValueError("MCP compatibility projection contract ids are invalid")
    return tuple(ids)


def _validate_policy_inventory(repo_root: Path) -> None:
    root = repo_root / "compatibility/fixtures/mcp"
    expected_names = set(_MCP_TOOL_NAMES)
    direct_directories = {
        item.name for item in root.iterdir() if item.is_dir() and not item.is_symlink()
    }
    if direct_directories != expected_names:
        raise ValueError("MCP fixture directory inventory is invalid")
    expected_leaves = set(_MCP_TOOL_FIXTURE_LEAVES)
    unsupported_output_leaves = {"error.output.json", "success.output.json"}
    for name in _MCP_TOOL_NAMES:
        directory = root / name
        entries = tuple(directory.iterdir())
        policy_leaves = {item.name for item in entries if item.is_file() and not item.is_symlink()}
        if policy_leaves != expected_leaves | unsupported_output_leaves or len(entries) != 6:
            raise ValueError(
                "MCP fixture policy must contain exactly four policy leaves and two unsupported outputs"
            )
        for leaf in expected_leaves:
            _read_object(directory / leaf)


def _validate_envelope(repo_root: Path, envelope: object) -> None:
    if type(envelope) is not dict or type(envelope.get("id")) is not int:
        raise ValueError("MCP response envelope is invalid")
    definition = _DEFINITION_BY_ID.get(cast(int, envelope["id"]))
    if definition is None or definition_issues(repo_root, definition, envelope):  # type: ignore[arg-type]
        raise ValueError("MCP response violates the official schema")


def _flat_leaves(
    value: object, prefix: tuple[object, ...] = ()
) -> dict[tuple[object, ...], object]:
    if type(value) is dict:
        result: dict[tuple[object, ...], object] = {}
        for key, child in cast(dict[str, object], value).items():
            result.update(_flat_leaves(child, (*prefix, key)))
        return result
    if type(value) is list:
        result = {}
        for index, child in enumerate(cast(list[object], value)):
            result.update(_flat_leaves(child, (*prefix, index)))
        return result
    return {prefix: value}


def _leaf_changes(left: object, right: object) -> set[tuple[object, ...]]:
    before = _flat_leaves(left)
    after = _flat_leaves(right)
    return set(before) ^ set(after) | {
        path for path in set(before) & set(after) if before[path] != after[path]
    }


def _validate_tool_wire_oracles(repo_root: Path) -> None:
    for name in _MCP_TOOL_NAMES:
        root = repo_root / "compatibility/fixtures/mcp" / name
        success_input = _read_object(root / "success.input.json")
        error_input = _read_object(root / "error.input.json")
        success_wire = _read_object(root / "wire.success.json")
        error_wire = _read_object(root / "wire.error.json")
        for wire, expected_id, is_error in (
            (success_wire, 1, False),
            (error_wire, 2, True),
        ):
            request = wire.get("request")
            result = wire.get("result")
            official_result = dict(cast(dict[str, object], result)) if type(result) is dict else {}
            official_result["resultType"] = "complete"
            envelope = {"jsonrpc": "2.0", "id": expected_id, "result": official_result}
            if (
                set(wire) != {"request", "result"}
                or type(request) is not dict
                or request.get("jsonrpc") != "2.0"
                or request.get("id") != expected_id
                or request.get("method") != "tools/call"
                or request.get("params", {}).get("name") != name  # type: ignore[union-attr]
                or type(result) is not dict
                or result.get("isError") is not is_error
                or definition_issues(repo_root, "CallToolResultResponse", envelope)
            ):
                raise ValueError("MCP tool wire result violates the official schema")
        success_params = cast(dict[str, object], success_wire["request"])["params"]
        error_params = cast(dict[str, object], error_wire["request"])["params"]
        if (
            cast(dict[str, object], success_params).get("arguments") != success_input
            or error_input.get("params") != error_params
            or error_input.get("setup") not in ({"validation": "invalid"}, {"data_root": "file"})
        ):
            raise ValueError("MCP tool input/wire identity is invalid")


_ROUTE_FAILURES: Mapping[str, tuple[int, object]] = {
    "invalid-host": (400, {"error": "invalid_host"}),
    "missing-bearer": (401, {"error": "unauthorized"}),
    "invalid-bearer": (401, {"error": "unauthorized"}),
    "missing-version": (
        400,
        {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32020,
                "message": "Header mismatch: required MCP-Protocol-Version header is missing",
            },
        },
    ),
    "protocol-version-header-mismatch": (
        400,
        {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32020,
                "message": "Header mismatch: MCP-Protocol-Version header value '2025-06-18' does not match body value '2026-07-28'",
            },
        },
    ),
    "unsupported-version": (
        400,
        {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32022,
                "message": "Unsupported protocol version: 2025-06-18",
                "data": {"requested": "2025-06-18", "supported": ["2026-07-28"]},
            },
        },
    ),
    "missing-mcp-method": (
        400,
        {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32020,
                "message": "Header mismatch: required Mcp-Method header is missing",
            },
        },
    ),
    "wrong-mcp-method": (
        400,
        {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32020,
                "message": "Header mismatch: Mcp-Method header value 'tools/call' does not match body value 'tools/list'",
            },
        },
    ),
    "unexpected-mcp-name-tools-list": (
        400,
        {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32020,
                "message": "Header mismatch: Mcp-Name header must be omitted for tools/list",
            },
        },
    ),
    "missing-mcp-name-tools-call": (
        400,
        {
            "jsonrpc": "2.0",
            "id": 2,
            "error": {
                "code": -32020,
                "message": "Header mismatch: required Mcp-Name header is missing for tools/call",
            },
        },
    ),
    "wrong-mcp-name-tools-call": (
        400,
        {
            "jsonrpc": "2.0",
            "id": 2,
            "error": {
                "code": -32020,
                "message": "Header mismatch: Mcp-Name header value 'hieronymus_recall' does not match body value 'hieronymus_status'",
            },
        },
    ),
}


def _expected_failure_request(failure_id: str, baseline: Mapping[str, object]) -> object:
    expected = json.loads(json.dumps(baseline))
    headers = expected["headers"]
    metadata = expected["body"]["params"]["_meta"]
    if failure_id == "invalid-host":
        headers["Host"] = "attacker.invalid"
    elif failure_id == "missing-bearer":
        del headers["Authorization"]
    elif failure_id == "invalid-bearer":
        headers["Authorization"] = "Bearer <INVALID_BEARER_TOKEN>"
    elif failure_id == "missing-version":
        del headers["MCP-Protocol-Version"]
    elif failure_id == "protocol-version-header-mismatch":
        headers["MCP-Protocol-Version"] = "2025-06-18"
    elif failure_id == "unsupported-version":
        headers["MCP-Protocol-Version"] = "2025-06-18"
        metadata["io.modelcontextprotocol/protocolVersion"] = "2025-06-18"
    elif failure_id == "missing-mcp-method":
        del headers["Mcp-Method"]
    elif failure_id == "wrong-mcp-method":
        headers["Mcp-Method"] = "tools/call"
    elif failure_id == "unexpected-mcp-name-tools-list":
        headers["Mcp-Name"] = "hieronymus_status"
    elif failure_id == "missing-mcp-name-tools-call":
        del headers["Mcp-Name"]
    elif failure_id == "wrong-mcp-name-tools-call":
        headers["Mcp-Name"] = "hieronymus_recall"
    else:
        raise ValueError("unknown MCP route failure")
    return expected


def _validate_oracles(repo_root: Path) -> tuple[str, ...]:
    if authority_issues(repo_root):
        raise ValueError("official MCP schema authority is invalid")
    contract_ids = _mcp_projection(repo_root)
    _validate_policy_inventory(repo_root)
    _validate_tool_wire_oracles(repo_root)

    snapshot = _read_object(repo_root / "compatibility/snapshots/mcp.json")
    tools = snapshot.get("tools")
    if type(tools) is not list or len(tools) != 39:
        raise ValueError("MCP snapshot registry is invalid")
    mapped_tools = []
    for tool in tools:
        if type(tool) is not dict or set(tool) != {"name", "description", "input_schema"}:
            raise ValueError("MCP snapshot tool is invalid")
        mapped_tools.append(
            {
                "name": tool["name"],
                "description": tool["description"],
                "inputSchema": tool["input_schema"],
            }
        )

    protocol = _read_object(repo_root / "compatibility/fixtures/mcp/protocol.json")
    target = protocol.get("target")
    if type(target) is not dict or set(target) != {
        "adr",
        "basis",
        "metadata_rules",
        "requests",
        "response_metadata_rules",
        "stdio",
        "streamable_http",
        "tools_call",
        "tools_list",
    }:
        raise ValueError("MCP protocol target is invalid")
    if (
        target.get("adr") != "docs/adr/0015-mcp-protocol-and-transport.md"
        or target.get("basis") != "adr-backed-target"
        or target.get("metadata_rules")
        != {
            "required": [
                "io.modelcontextprotocol/protocolVersion",
                "io.modelcontextprotocol/clientCapabilities",
            ],
            "should": ["io.modelcontextprotocol/clientInfo"],
        }
        or target.get("response_metadata_rules")
        != {
            "io.modelcontextprotocol/serverInfo": {
                "configured": "omit",
                "rationale": (
                    "The compatibility oracle is implementation-neutral; freezing "
                    "self-reported package identity would create a volatile version "
                    "contract unrelated to protocol behavior."
                ),
            }
        }
    ):
        raise ValueError("MCP protocol metadata rules are invalid")
    compact = json.dumps(target, sort_keys=True, separators=(",", ":"))
    if any(
        token in compact
        for token in ('"initialize"', "notifications/initialized", "Mcp-Session-Id")
    ):
        raise ValueError("MCP protocol target is not stateless")
    list_response = target.get("tools_list", {}).get("response")  # type: ignore[union-attr]
    call_response = target.get("tools_call", {}).get("response")  # type: ignore[union-attr]
    for envelope in (list_response, call_response):
        _validate_envelope(repo_root, envelope)
    if type(list_response) is not dict or list_response.get("result") != {
        "cacheScope": "private",
        "resultType": "complete",
        "tools": mapped_tools,
        "ttlMs": 0,
    }:
        raise ValueError("MCP list result does not match the frozen registry")
    requests = target.get("requests")
    if requests != [
        target.get("tools_list", {}).get("request"),
        target.get("tools_call", {}).get("request"),
    ]:  # type: ignore[union-attr]
        raise ValueError("MCP protocol request inventory is invalid")
    for request in cast(list[object], requests):
        if type(request) is not dict:
            raise ValueError("MCP protocol request is invalid")
        meta = request.get("params", {}).get("_meta")  # type: ignore[union-attr]
        if (
            type(meta) is not dict
            or meta.get("io.modelcontextprotocol/protocolVersion") != "2026-07-28"
            or type(meta.get("io.modelcontextprotocol/clientCapabilities")) is not dict
        ):
            raise ValueError("MCP request metadata is invalid")
    stdio = target.get("stdio")
    if type(stdio) is not dict or stdio.get("framing") != "newline-delimited-json-rpc":
        raise ValueError("MCP stdio framing is invalid")
    stdio_exchanges = stdio.get("exchanges")
    if type(stdio_exchanges) is not list or len(stdio_exchanges) != 2:
        raise ValueError("MCP stdio exchanges are invalid")
    for index, exchange in enumerate(stdio_exchanges):
        if type(exchange) is not dict or type(exchange.get("response_line")) is not str:
            raise ValueError("MCP stdio exchange is invalid")
        response_line = cast(str, exchange["response_line"])
        if not response_line.endswith("\n") or response_line.count("\n") != 1:
            raise ValueError("MCP stdio exchange framing is invalid")
        expected_request = cast(list[object], requests)[index]
        expected_response = (list_response, call_response)[index]
        if (
            exchange.get("request_line")
            != json.dumps(expected_request, sort_keys=True, separators=(",", ":")) + "\n"
        ):
            raise ValueError("MCP stdio request line differs from target")
        parsed_response = json.loads(response_line)
        _validate_envelope(repo_root, parsed_response)
        if parsed_response != expected_response:
            raise ValueError("MCP stdio response line differs from target")
    streamable = target.get("streamable_http")
    if type(streamable) is not dict or len(streamable.get("exchanges", [])) != 2:
        raise ValueError("MCP Streamable HTTP oracle is invalid")
    for index, exchange in enumerate(streamable["exchanges"]):
        if type(exchange) is not dict or type(exchange.get("responses")) is not list:
            raise ValueError("MCP Streamable HTTP exchange is invalid")
        stream_request = exchange.get("request")
        if (
            type(stream_request) is not dict
            or stream_request.get("body") != cast(list[object], requests)[index]
            or stream_request.get("method") != "POST"
            or stream_request.get("path") != "/mcp"
            or set(stream_request) != {"body", "headers", "method", "path"}
        ):
            raise ValueError("MCP Streamable HTTP request differs from target")
        content_types = set()
        for response in exchange["responses"]:
            if type(response) is not dict:
                raise ValueError("MCP Streamable HTTP response is invalid")
            content_types.add(response.get("content_type"))
            body = response.get("body")
            expected_response = (list_response, call_response)[index]
            if type(body) is dict:
                _validate_envelope(repo_root, body)
                if body != expected_response:
                    raise ValueError("MCP JSON response differs from target")
            elif type(response.get("events")) is list:
                events = cast(list[object], response["events"])
                if len(events) != 1 or type(events[0]) is not dict:
                    raise ValueError("MCP SSE event is invalid")
                _validate_envelope(repo_root, cast(dict[str, object], events[0]).get("data"))
                if events != [{"event": "message", "data": expected_response}]:
                    raise ValueError("MCP SSE response differs from target")
        if content_types != {"application/json", "text/event-stream"}:
            raise ValueError("MCP Streamable HTTP variants are incomplete")

    routes = _read_object(repo_root / "compatibility/fixtures/http/route-cases.json")
    route_list = routes.get("routes")
    if type(route_list) is not list:
        raise ValueError("HTTP route oracle is invalid")
    route = next(
        (
            item
            for item in route_list
            if type(item) is dict and item.get("contract_id") == "http.route.post.mcp"
        ),
        None,
    )
    if type(route) is not dict or type(route.get("target")) is not dict:
        raise ValueError("MCP HTTP route target is missing")
    route_target = cast(dict[str, object], route["target"])
    successes = route_target.get("successes")
    failures = route_target.get("failures")
    if type(successes) is not list or type(failures) is not list:
        raise ValueError("MCP HTTP route cases are invalid")
    if [item.get("id") for item in successes if type(item) is dict] != ["tools-list", "tools-call"]:
        raise ValueError("MCP HTTP success ids are invalid")
    failure_ids = [item.get("id") for item in failures if type(item) is dict]
    if len(failures) != 11 or set(failure_ids) != {
        "invalid-host",
        "missing-bearer",
        "invalid-bearer",
        "missing-version",
        "protocol-version-header-mismatch",
        "unsupported-version",
        "missing-mcp-method",
        "wrong-mcp-method",
        "unexpected-mcp-name-tools-list",
        "missing-mcp-name-tools-call",
        "wrong-mcp-name-tools-call",
    }:
        raise ValueError("MCP HTTP failure ids are invalid")
    for success in successes:
        if type(success) is not dict:
            raise ValueError("MCP HTTP success is invalid")
        response = success.get("response")
        request = success.get("request")
        success_id = success.get("id")
        expected_envelope = list_response if success_id == "tools-list" else call_response
        request_key = "tools_list" if success_id == "tools-list" else "tools_call"
        expected_request = cast(dict[str, object], target[request_key])["request"]
        stream_request = json.loads(
            json.dumps(
                cast(list[dict[str, object]], streamable["exchanges"])[
                    0 if success_id == "tools-list" else 1
                ]["request"]
            )
        )
        stream_request["query"] = {}
        expected_route_result = (
            {"cacheScope": "private", "resultType": "complete", "tools": [], "ttlMs": 0}
            if success_id == "tools-list"
            else {"content": [], "isError": False, "resultType": "complete"}
        )
        if (
            type(response) is not dict
            or type(request) is not dict
            or response.get("status") != 200
            or response.get("headers") != {"Content-Type": "application/json; charset=utf-8"}
            or request.get("method") != "POST"
            or request.get("path") != "/mcp"
            or request.get("query") != {}
            or request.get("body") != expected_request
            or request != stream_request
        ):
            raise ValueError("MCP HTTP success route is invalid")
        _validate_envelope(repo_root, response.get("body"))
        route_body = cast(dict[str, object], response["body"])
        if (
            route_body.get("id") != cast(dict[str, object], expected_envelope).get("id")
            or route_body.get("jsonrpc") != "2.0"
            or type(route_body.get("result")) is not dict
            or route_body.get("result") != expected_route_result
            or "io.modelcontextprotocol/serverInfo" in cast(dict[str, object], route_body["result"])
        ):
            raise ValueError("MCP HTTP success response is invalid")
    for failure in failures:
        if type(failure) is not dict or type(failure.get("response")) is not dict:
            raise ValueError("MCP HTTP failure is invalid")
        response = cast(dict[str, object], failure["response"])
        body = response.get("body")
        failure_id = failure.get("id")
        expected_failure = _ROUTE_FAILURES.get(cast(str, failure_id))
        if (
            expected_failure is None
            or response.get("status") != expected_failure[0]
            or response.get("headers") != {"Content-Type": "application/json; charset=utf-8"}
            or body != expected_failure[1]
        ):
            raise ValueError("MCP HTTP failure response differs from the frozen contract")
        request = failure.get("request")
        if (
            type(request) is not dict
            or request.get("method") != "POST"
            or request.get("path") != "/mcp"
            or request.get("query") != {}
        ):
            raise ValueError("MCP HTTP failure route is invalid")
        if failure_id in _HEADER_FAILURE_IDS:
            if response.get("status") != 400 or definition_issues(
                repo_root, "HeaderMismatchError", body
            ):
                raise ValueError("MCP header mismatch error is invalid")
        elif failure_id == "unsupported-version":
            if response.get("status") != 400 or definition_issues(
                repo_root, "UnsupportedProtocolVersionError", body
            ):
                raise ValueError("MCP unsupported-version error is invalid")
    successes_by_method = {
        cast(dict[str, object], item["request"])["body"]["method"]: item  # type: ignore[index]
        for item in cast(list[dict[str, object]], successes)
    }
    for failure in cast(list[dict[str, object]], failures):
        request = cast(dict[str, object], failure["request"])
        method = cast(dict[str, object], request["body"])["method"]
        baseline = cast(dict[str, object], successes_by_method[method]["request"])
        if request != _expected_failure_request(cast(str, failure["id"]), baseline):
            raise ValueError("MCP HTTP failure request differs from the frozen contract")
        changes = _leaf_changes(baseline, request)
        expected_count = 2 if failure["id"] == "unsupported-version" else 1
        if len(changes) != expected_count:
            raise ValueError("MCP HTTP raw request mutation invariant is invalid")
        if failure["id"] == "unsupported-version" and changes != {
            ("headers", "MCP-Protocol-Version"),
            ("body", "params", "_meta", "io.modelcontextprotocol/protocolVersion"),
        }:
            raise ValueError("MCP HTTP unsupported-version mutation is invalid")
    if any(
        type(item) is dict
        and type(item.get("request")) is dict
        and cast(dict[str, object], item["request"]).get("path") == "/api/mcp/fixture"
        for item in (*successes, *failures)
    ):
        raise ValueError("private MCP bridge route remains present")
    return contract_ids


def _safe_observed_string(value: object, *, label: str) -> str:
    if (
        type(value) is not str
        or not value
        or len(value) > 512
        or "\x00" in value
        or "/home/" in value
        or "/Users/" in value
        or "\\" in value
    ):
        raise ValueError(f"MCP probe artifact {label} is invalid")
    return value


def _probe_observations(payload: object) -> _ProbeObservations:
    if (
        type(payload) is not dict
        or set(payload)
        != {
            "schemaVersion",
            "checks",
            "environment",
            "dependencies",
        }
        or payload.get("schemaVersion") != _PROBE_SCHEMA_VERSION
    ):
        raise ValueError("MCP typed probe artifact is invalid")
    raw_checks = payload.get("checks")
    if type(raw_checks) is not dict or set(raw_checks) != set(REQUIRED_CRITERIA[_RISK]):
        raise ValueError("MCP probe artifact checks are invalid")
    checks: dict[str, tuple[bool, Mapping[str, object]]] = {}
    for criterion in REQUIRED_CRITERIA[_RISK]:
        check = raw_checks[criterion]
        if type(check) is not dict or set(check) != {"passed", "measurements"}:
            raise ValueError("MCP probe artifact check is invalid")
        passed = check.get("passed")
        measurements = check.get("measurements")
        if type(passed) is not bool or type(measurements) is not dict or not measurements:
            raise ValueError("MCP probe artifact check is invalid")
        # Measurements validates the exact scalar-only JSON shape and finiteness.
        checked_measurements = Measurements(cast(dict[str, object], measurements))
        checks[criterion] = (passed, dict(checked_measurements))
    raw_environment = payload.get("environment")
    environment_keys = {"rustc", "cargo", "target", "os", "kernel", "architecture"}
    if type(raw_environment) is not dict or set(raw_environment) != environment_keys:
        raise ValueError("MCP probe artifact environment is invalid")
    environment = {
        key: _safe_observed_string(raw_environment[key], label=key) for key in environment_keys
    }
    raw_dependencies = payload.get("dependencies")
    if type(raw_dependencies) is not list or not raw_dependencies:
        raise ValueError("MCP probe artifact dependencies are invalid")
    dependencies: list[LockedDependency] = []
    for item in raw_dependencies:
        if type(item) is not dict or set(item) != {
            "name",
            "version",
            "source",
            "checksum",
            "features",
        }:
            raise ValueError("MCP probe artifact dependency is invalid")
        features = item.get("features")
        if type(features) is not list or any(type(feature) is not str for feature in features):
            raise ValueError("MCP probe artifact dependency features are invalid")
        dependencies.append(
            LockedDependency(
                name=_safe_observed_string(item.get("name"), label="dependency name"),
                version=_safe_observed_string(item.get("version"), label="dependency version"),
                source=_safe_observed_string(item.get("source"), label="dependency source"),
                checksum=(
                    None
                    if item.get("checksum") is None
                    else _safe_observed_string(item.get("checksum"), label="dependency checksum")
                ),
                features=tuple(sorted(cast(list[str], features))),
            )
        )
    return _ProbeObservations(
        checks=checks,
        environment=environment,
        dependencies=tuple(sorted(dependencies, key=lambda item: (item.name, item.version))),
    )


def _read_probe_artifact(path: Path) -> _ProbeObservations:
    try:
        info = path.lstat()
        if not stat.S_ISREG(info.st_mode) or info.st_size > _MAX_CAPTURE:
            raise ValueError
        payload = json.loads(path.read_bytes())
    except (OSError, ValueError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("MCP typed probe artifact is invalid") from error
    return _probe_observations(payload)


def _run_bounded(
    argv: tuple[str, ...],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
    stdin: bytes | None = None,
    timeout_seconds: int = 20,
) -> tuple[int, bytes, bytes]:
    process: subprocess.Popen[bytes] | None = None
    selector = selectors.DefaultSelector()
    stdout = bytearray()
    stderr = bytearray()
    try:
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            env=None if env is None else dict(env),
            stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            close_fds=True,
        )
        if stdin is not None:
            assert process.stdin is not None
            process.stdin.write(stdin)
            process.stdin.close()
        assert process.stdout is not None and process.stderr is not None
        for pipe, target in ((process.stdout, stdout), (process.stderr, stderr)):
            os.set_blocking(pipe.fileno(), False)
            selector.register(pipe, selectors.EVENT_READ, target)
        deadline = time.monotonic() + timeout_seconds
        while selector.get_map() or process.poll() is None:
            if time.monotonic() >= deadline:
                raise CandidateTimeout("bounded MCP child timed out")
            for key, _mask in selector.select(timeout=0.05):
                try:
                    chunk = os.read(key.fileobj.fileno(), 64 * 1024)
                except BlockingIOError:
                    continue
                if not chunk:
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                    continue
                key.data.extend(chunk)
                if len(key.data) > _MAX_CAPTURE:
                    raise CandidateOutput("bounded MCP child output exceeds limit")
        return process.wait(timeout=1), bytes(stdout), bytes(stderr)
    except CandidateMismatch:
        if process is not None and process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=2)
        raise
    except subprocess.TimeoutExpired as error:
        if process is not None and process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=2)
        raise CandidateTimeout("bounded MCP child timed out") from error
    finally:
        selector.close()
        if process is not None:
            for pipe in (process.stdin, process.stdout, process.stderr):
                if pipe is not None and not pipe.closed:
                    pipe.close()


def _record(
    repo_root: Path,
    *,
    contract_ids: tuple[str, ...],
    observations: _ProbeObservations,
    work_dir_removed: bool,
    raw_logs_removed: bool,
    install_dir_removed: bool,
    core_dumps_disabled: bool = True,
    process_groups_reaped: bool = True,
) -> QualificationRecord:
    evidence = tuple(
        Evidence(
            criterion=criterion,
            status="pass" if observations.checks[criterion][0] else "fail",
            summary=(
                f"{criterion} passed the bounded offline replay"
                if observations.checks[criterion][0]
                else f"{criterion} failed in the bounded offline replay"
            ),
            measurements=Measurements(observations.checks[criterion][1]),
        )
        for criterion in REQUIRED_CRITERIA[_RISK]
    )
    status = status_for(evidence)
    input_paths = required_fingerprint_inputs(_RISK)
    return QualificationRecord(
        schema_version=1,
        risk=_RISK,
        target=_TARGET,
        status=status,
        decision=decision_for(_RISK, evidence),
        acceptance_owner=_OWNER,
        specs=_SPECS,
        contract_ids=contract_ids,
        input_paths=input_paths,
        input_digest=fingerprint_inputs(repo_root, input_paths),
        commands=_COMMANDS,
        environment=Environment(
            rustc=observations.environment["rustc"],
            cargo=observations.environment["cargo"],
            target=observations.environment["target"],
            os=observations.environment["os"],
            kernel=observations.environment["kernel"],
            architecture=observations.environment["architecture"],
            bun=None,
            native_libraries=(),
        ),
        dependencies=observations.dependencies,
        evidence=evidence,
        consequence=expected_consequence(_RISK, status),
        cleanup=CleanupEvidence(
            work_dir_removed=work_dir_removed,
            raw_logs_removed=raw_logs_removed,
            install_dir_removed=install_dir_removed,
            source_inputs_unchanged=True,
            user_data_opened=False,
            core_dumps_disabled=core_dumps_disabled,
            owned_process_groups_reaped=process_groups_reaped,
        ),
        review=Review(
            owner=_OWNER,
            status="pending",
            objective_evidence_reviewed=False,
            normative_constraints_preserved=False,
        ),
    )


def run(repo_root: Path, work_root: Path, *, executable: Path) -> QualificationRecord:
    """Run a fake-injected MCP probe after validating every immutable oracle."""
    contract_ids = _validate_oracles(repo_root)
    before = fingerprint_inputs(repo_root, required_fingerprint_inputs(_RISK))
    bounded_work = work_root / "mcp-transport-run"
    bounded_work.mkdir(mode=0o700, parents=True, exist_ok=False)
    artifact = bounded_work / "probe-result.json"
    work_removed = False
    try:
        code, _stdout, _stderr = _run_bounded(
            (str(executable), "--probe-artifact", str(artifact)),
            cwd=repo_root,
            env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC"},
        )
        if code != 0:
            raise ValueError("MCP qualification executable failed")
        observations = _read_probe_artifact(artifact)
    finally:
        work_removed = _remove_exact_directory(bounded_work)
    after = fingerprint_inputs(repo_root, required_fingerprint_inputs(_RISK))
    if after != before:
        raise ValueError("immutable MCP qualification inputs changed during replay")
    return _record(
        repo_root,
        contract_ids=contract_ids,
        observations=observations,
        work_dir_removed=work_removed,
        raw_logs_removed=work_removed,
        install_dir_removed=work_removed,
    )


def _live_process_context(
    repo_root: Path,
    work_root: Path,
    original_env: Mapping[str, str],
) -> tuple[ToolRoots, Path, dict[str, str]]:
    """Discover unsanitized tool roots, then produce one shared safe environment."""
    tool_roots = discover_tool_roots(original_env)
    cargo_target_dir = repo_root / _CARGO_TARGET
    _reconcile_live_cargo_target(cargo_target_dir)
    child_env = safe_subprocess_env(
        work_root,
        cargo_offline=True,
        tool_roots=tool_roots,
        cargo_target_dir=cargo_target_dir,
    )
    return tool_roots, cargo_target_dir, child_env


def _directory_open_flags() -> int:
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    return flags


def _open_canonical_directory(path: Path, *, label: str) -> int:
    """Open every absolute component without symlink traversal."""
    absolute = path.absolute()
    if path != absolute:
        raise ValueError(f"{label} path must be absolute")
    descriptor = os.open(absolute.anchor, _directory_open_flags())
    try:
        for component in absolute.parts[1:]:
            child = os.open(component, _directory_open_flags(), dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        return descriptor
    except OSError as error:
        os.close(descriptor)
        raise ValueError(f"{label} path is unsafe") from error


def _mount_id(fd: int) -> int:
    """Read the Linux mount identity for an already-opened directory."""
    try:
        lines = Path(f"/proc/self/fdinfo/{fd}").read_text(encoding="ascii").splitlines()
    except OSError as error:  # pragma: no cover - Linux procfs is a qualification prerequisite
        raise ValueError("MCP cleanup requires Linux mount identity data") from error
    for line in lines:
        if line.startswith("mnt_id:"):
            try:
                return int(line.split(":", 1)[1].strip())
            except ValueError as error:  # pragma: no cover - kernel-owned format
                raise ValueError("MCP cleanup found malformed mount identity data") from error
    raise ValueError("MCP cleanup requires a mount identity for every directory")


def _identity(info: os.stat_result) -> tuple[int, int, int, int]:
    return info.st_mode, info.st_dev, info.st_ino, info.st_nlink


def _entry_identity_at(parent_fd: int, name: str) -> tuple[int, int, int, int]:
    return _identity(os.stat(name, dir_fd=parent_fd, follow_symlinks=False))


def _same_identity(
    current: tuple[int, int, int, int],
    expected: tuple[int, int, int, int],
    *,
    children_removed: bool = False,
) -> bool:
    if children_removed and stat.S_ISDIR(expected[0]):
        return current[:3] == expected[:3]
    return current == expected


@dataclass(frozen=True, slots=True)
class _CleanupNode:
    name: str
    identity: tuple[int, int, int, int]
    children: tuple[_CleanupNode, ...]


def _validate_cleanup_tree(
    directory_fd: int,
    *,
    mount_id: int,
    hardlinks: dict[tuple[int, int], tuple[int, int]],
) -> tuple[_CleanupNode, ...]:
    nodes: list[_CleanupNode] = []
    for entry in sorted(os.scandir(directory_fd), key=lambda item: item.name):
        identity = _entry_identity_at(directory_fd, entry.name)
        mode, _, _, links = identity
        if stat.S_ISLNK(mode):
            raise ValueError(f"MCP cleanup refuses symlink entry {entry.name}")
        if stat.S_ISDIR(mode):
            child_fd = os.open(entry.name, _directory_open_flags(), dir_fd=directory_fd)
            try:
                if _identity(os.fstat(child_fd)) != identity:
                    raise ValueError(f"MCP cleanup entry {entry.name} changed during validation")
                if _mount_id(child_fd) != mount_id:
                    raise ValueError(f"MCP cleanup refuses mounted directory {entry.name}")
                children = _validate_cleanup_tree(
                    child_fd,
                    mount_id=mount_id,
                    hardlinks=hardlinks,
                )
            finally:
                os.close(child_fd)
            nodes.append(_CleanupNode(entry.name, identity, children))
        elif stat.S_ISREG(mode):
            if links > 1:
                key = identity[1], identity[2]
                expected_links, observed_links = hardlinks.get(key, (links, 0))
                if expected_links != links:
                    raise ValueError(f"MCP cleanup hard-linked file {entry.name} changed")
                hardlinks[key] = expected_links, observed_links + 1
            nodes.append(_CleanupNode(entry.name, identity, ()))
        else:
            raise ValueError(f"MCP cleanup refuses special entry {entry.name}")
    return tuple(nodes)


def _remove_validated_cleanup_tree(
    directory_fd: int,
    nodes: tuple[_CleanupNode, ...],
    *,
    mount_id: int,
    remaining_links: dict[tuple[int, int], int],
) -> None:
    for node in nodes:
        current = _entry_identity_at(directory_fd, node.name)
        key = node.identity[1], node.identity[2]
        if key in remaining_links and stat.S_ISREG(node.identity[0]):
            if current[:3] != node.identity[:3] or current[3] != remaining_links[key]:
                raise ValueError(f"MCP cleanup entry {node.name} changed before deletion")
        elif not _same_identity(current, node.identity):
            raise ValueError(f"MCP cleanup entry {node.name} changed before deletion")
        if stat.S_ISDIR(node.identity[0]):
            child_fd = os.open(node.name, _directory_open_flags(), dir_fd=directory_fd)
            try:
                if _identity(os.fstat(child_fd)) != node.identity:
                    raise ValueError(f"MCP cleanup entry {node.name} changed before deletion")
                if _mount_id(child_fd) != mount_id:
                    raise ValueError(f"MCP cleanup refuses mounted directory {node.name}")
                _remove_validated_cleanup_tree(
                    child_fd,
                    node.children,
                    mount_id=mount_id,
                    remaining_links=remaining_links,
                )
            finally:
                os.close(child_fd)
            if not _same_identity(
                _entry_identity_at(directory_fd, node.name),
                node.identity,
                children_removed=True,
            ):
                raise ValueError(f"MCP cleanup entry {node.name} changed before deletion")
            os.rmdir(node.name, dir_fd=directory_fd)
        else:
            os.unlink(node.name, dir_fd=directory_fd)
            if key in remaining_links:
                remaining_links[key] -= 1


def _remove_exact_directory(path: Path) -> bool:
    """Remove one exact nonsymlink directory after descriptor-safe full-tree validation."""
    absolute = path.absolute()
    if absolute == Path(absolute.anchor):
        raise ValueError("MCP cleanup target is an unsafe root")
    if not os.path.lexists(absolute):
        return True
    lexical = absolute.lstat()
    if not stat.S_ISDIR(lexical.st_mode) or stat.S_ISLNK(lexical.st_mode):
        raise ValueError("MCP cleanup target must be a nonsymlink directory")
    parent_fd = _open_canonical_directory(absolute.parent, label="MCP cleanup parent")
    target_fd: int | None = None
    try:
        parent_identity = _identity(os.fstat(parent_fd))
        parent_mount = _mount_id(parent_fd)
        target_identity = _entry_identity_at(parent_fd, absolute.name)
        target_fd = os.open(absolute.name, _directory_open_flags(), dir_fd=parent_fd)
        if (
            target_identity != _identity(lexical)
            or _identity(os.fstat(target_fd)) != target_identity
            or target_identity[1] != parent_identity[1]
            or _mount_id(target_fd) != parent_mount
            or os.fstat(target_fd).st_uid != os.getuid()
        ):
            raise ValueError("MCP cleanup target identity or mount is unsafe")
        hardlinks: dict[tuple[int, int], tuple[int, int]] = {}
        children = _validate_cleanup_tree(
            target_fd,
            mount_id=parent_mount,
            hardlinks=hardlinks,
        )
        if any(expected != observed for expected, observed in hardlinks.values()):
            raise ValueError("MCP cleanup refuses external hard-linked files")
        if (
            _entry_identity_at(parent_fd, absolute.name) != target_identity
            or _identity(os.fstat(target_fd)) != target_identity
            or _mount_id(target_fd) != parent_mount
        ):
            raise ValueError("MCP cleanup target changed before deletion")
        _remove_validated_cleanup_tree(
            target_fd,
            children,
            mount_id=parent_mount,
            remaining_links={key: value[0] for key, value in hardlinks.items()},
        )
        os.close(target_fd)
        target_fd = None
        if not _same_identity(
            _entry_identity_at(parent_fd, absolute.name),
            target_identity,
            children_removed=True,
        ):
            raise ValueError("MCP cleanup target changed before deletion")
        os.rmdir(absolute.name, dir_fd=parent_fd)
        try:
            _entry_identity_at(parent_fd, absolute.name)
        except FileNotFoundError:
            return True
        raise ValueError("MCP cleanup target still exists after deletion")
    except FileNotFoundError as error:
        raise ValueError("MCP cleanup target changed before deletion") from error
    finally:
        if target_fd is not None:
            os.close(target_fd)
        os.close(parent_fd)


def _reconcile_live_cargo_target(target: Path) -> None:
    """Privatize only an exact, owned, nonsymlink Cargo target left by Cargo."""
    absolute = target.absolute()
    if not os.path.lexists(absolute):
        return
    try:
        lexical = absolute.lstat()
    except OSError as error:
        raise ValueError("cargo target cannot be inspected") from error
    if not stat.S_ISDIR(lexical.st_mode) or stat.S_ISLNK(lexical.st_mode):
        raise ValueError("cargo target must be a nonsymlink directory")
    parent_fd = _open_canonical_directory(absolute.parent, label="cargo target parent")
    target_fd: int | None = None
    try:
        target_fd = os.open(absolute.name, _directory_open_flags(), dir_fd=parent_fd)
        parent = os.fstat(parent_fd)
        opened = os.fstat(target_fd)
        parent_mount = _mount_id(parent_fd)
        target_mount = _mount_id(target_fd)
        mode = stat.S_IMODE(opened.st_mode)
        if target_mount != parent_mount:
            raise ValueError("mounted cargo target is unsafe")
        if (
            not stat.S_ISDIR(opened.st_mode)
            or opened.st_uid != os.getuid()
            or opened.st_dev != parent.st_dev
            or _identity(opened) != _identity(lexical)
            or mode & 0o022
        ):
            raise ValueError("cargo target ownership or mode is unsafe")
        if mode != 0o700:
            if (
                _entry_identity_at(parent_fd, absolute.name) != _identity(opened)
                or _identity(os.fstat(target_fd)) != _identity(opened)
                or _mount_id(target_fd) != parent_mount
            ):
                raise ValueError("cargo target changed before chmod")
            os.fchmod(target_fd, 0o700)
            if stat.S_IMODE(os.fstat(target_fd).st_mode) != 0o700:
                raise ValueError("cargo target could not be made private")
    except OSError as error:
        raise ValueError("cargo target path is unsafe") from error
    finally:
        if target_fd is not None:
            os.close(target_fd)
        os.close(parent_fd)


def _prepare_private_work(work_root: Path) -> None:
    lexical = work_root.absolute()
    if os.path.lexists(lexical):
        raise ValueError("MCP private work root already exists or is a symlink")
    try:
        parent = lexical.parent.resolve(strict=True)
    except OSError as error:
        raise ValueError("MCP private work parent is invalid") from error
    if parent != lexical.parent or not parent.is_dir():
        raise ValueError("MCP private work parent is a symlink")
    lexical.mkdir(mode=0o700)
    info = lexical.lstat()
    if not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError("MCP private work root is invalid")


def _successful(receipt: ProcessReceipt) -> bool:
    return (
        receipt.exit_code == 0
        and not receipt.timed_out
        and receipt.core_dumps_disabled
        and receipt.process_group_reaped
    )


def _stdio_probe(
    executable: Path,
    repo_root: Path,
    target: dict[str, object],
    protocol_path: Path,
) -> None:
    for key in ("tools_list", "tools_call"):
        exchange = target.get(key)
        if type(exchange) is not dict:
            raise ValueError("MCP stdio target exchange is invalid")
        request = exchange.get("request")
        expected = exchange.get("response")
        payload = json.dumps(request, sort_keys=True, separators=(",", ":")).encode() + b"\n"
        code, stdout, _stderr = _run_bounded(
            (
                str(executable),
                "stdio",
                "--registry",
                str(repo_root / "compatibility/snapshots/mcp.json"),
                "--protocol",
                str(protocol_path),
            ),
            cwd=repo_root,
            stdin=payload,
        )
        if code != 0 or stdout.count(b"\n") != 1:
            raise CandidateMismatch("bounded MCP stdio replay failed")
        try:
            actual = json.loads(stdout)
        except (UnicodeError, json.JSONDecodeError) as error:
            raise CandidateMismatch("bounded MCP stdio response is invalid") from error
        if actual != expected:
            raise CandidateMismatch("bounded MCP stdio response differs from the oracle")


def _http_exchange(
    port: int,
    request: Mapping[str, object],
    *,
    accept: str | None = None,
    timeout_seconds: float = 20,
) -> tuple[int, str, object]:
    headers_value = request.get("headers")
    if type(headers_value) is not dict:
        raise ValueError("MCP HTTP request headers are invalid")
    headers = {str(name): str(value) for name, value in headers_value.items()}
    if headers.get("Host") == "127.0.0.1:<PORT>":
        headers["Host"] = f"127.0.0.1:{port}"
    if accept is not None:
        headers["Accept"] = accept
    body = json.dumps(request.get("body"), sort_keys=True, separators=(",", ":")).encode()
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout_seconds)
    try:
        try:
            connection.request(
                cast(str, request.get("method")),
                cast(str, request.get("path")),
                body=body,
                headers=headers,
            )
            response = connection.getresponse()
            content_type = response.getheader("Content-Type", "")
            raw = response.read(2 * 1024 * 1024 + 1)
        except TimeoutError as error:
            raise CandidateTimeout("MCP HTTP candidate timed out") from error
        except http.client.HTTPException as error:
            raise CandidateMismatch("MCP HTTP candidate response is invalid") from error
    finally:
        connection.close()
    if len(raw) > 2 * 1024 * 1024:
        raise CandidateOutput("MCP HTTP response exceeds the bound")
    try:
        if content_type == "text/event-stream":
            text = raw.decode("utf-8", errors="strict")
            data_lines = [line[6:] for line in text.splitlines() if line.startswith("data: ")]
            if len(data_lines) != 1:
                raise CandidateMismatch("MCP SSE response framing is invalid")
            payload = json.loads(data_lines[0])
        else:
            payload = json.loads(raw)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise CandidateMismatch("MCP HTTP response is invalid") from error
    return response.status, content_type, payload


def _assert_candidate_error(status: int, body: object, *, code: int) -> None:
    if (
        status != 400
        or type(body) is not dict
        or "result" in body
        or type(body.get("error")) is not dict
        or body["error"].get("code") != code
    ):
        raise CandidateMismatch(f"MCP negative probe did not return error {code}")


def _request_copy(value: Mapping[str, object]) -> dict[str, object]:
    return cast(dict[str, object], json.loads(json.dumps(value)))


def _probe_metadata_cases(
    port: int,
    target: Mapping[str, object],
    route_target: Mapping[str, object],
) -> None:
    successes = cast(list[dict[str, object]], route_target["successes"])
    baseline = cast(dict[str, object], successes[0]["request"])
    expected = cast(dict[str, object], target["tools_list"])["response"]

    for key, replacement, expected_code in (
        ("io.modelcontextprotocol/protocolVersion", None, -32020),
        ("io.modelcontextprotocol/protocolVersion", 20260728, -32020),
        ("io.modelcontextprotocol/clientCapabilities", None, -32602),
        ("io.modelcontextprotocol/clientCapabilities", [], -32602),
    ):
        request = _request_copy(baseline)
        metadata = request["body"]["params"]["_meta"]  # type: ignore[index]
        if replacement is None:
            del metadata[key]
        else:
            metadata[key] = replacement
        status, _, body = _http_exchange(port, request)
        _assert_candidate_error(status, body, code=expected_code)

    for legacy in ("protocolVersion", "clientCapabilities", "clientInfo"):
        request = _request_copy(baseline)
        request["body"]["params"][legacy] = {}  # type: ignore[index]
        status, _, body = _http_exchange(port, request)
        _assert_candidate_error(status, body, code=-32602)

    request = _request_copy(baseline)
    request["body"]["params"]["_meta"][  # type: ignore[index]
        "io.modelcontextprotocol/clientInfo"
    ] = {"name": "qualification-client", "version": "1"}
    status, _, body = _http_exchange(port, request)
    if status != 200 or body != expected:
        raise CandidateMismatch("valid optional MCP clientInfo was rejected")

    request = _request_copy(baseline)
    request["body"]["params"]["_meta"][  # type: ignore[index]
        "io.modelcontextprotocol/clientInfo"
    ] = {"name": "qualification-client"}
    status, _, body = _http_exchange(port, request)
    _assert_candidate_error(status, body, code=-32602)


def _probe_stateless_cases(port: int, route_target: Mapping[str, object]) -> None:
    successes = cast(list[dict[str, object]], route_target["successes"])
    baseline = cast(dict[str, object], successes[0]["request"])
    initialize = _request_copy(baseline)
    initialize["headers"]["Mcp-Method"] = "initialize"  # type: ignore[index]
    initialize["body"]["method"] = "initialize"  # type: ignore[index]
    status, _, body = _http_exchange(port, initialize)
    _assert_candidate_error(status, body, code=-32602)

    for header_name in ("Mcp-Session-Id", "Last-Event-ID"):
        request = _request_copy(baseline)
        request["headers"][header_name] = "forbidden-session"  # type: ignore[index]
        status, _, body = _http_exchange(port, request)
        _assert_candidate_error(status, body, code=-32602)


def _probe_reserved_metadata_cases(
    port: int,
    target: Mapping[str, object],
    route_target: Mapping[str, object],
) -> None:
    successes = cast(list[dict[str, object]], route_target["successes"])
    for index, key in enumerate(("tools_list", "tools_call")):
        request = _request_copy(cast(dict[str, object], successes[index]["request"]))
        request["body"]["params"]["_meta"][  # type: ignore[index]
            "io.modelcontextprotocol/qualificationProbe"
        ] = {"accepted": True}
        status, _, body = _http_exchange(port, request)
        expected = cast(dict[str, object], target[key])["response"]
        if status != 200 or body != expected:
            raise CandidateMismatch(f"reserved metadata was rejected for {key}")


def _http_negotiations(
    target: Mapping[str, object], exchange_index: int
) -> tuple[tuple[str, str], ...]:
    try:
        exchange = cast(
            dict[str, object],
            cast(dict[str, object], target["streamable_http"])["exchanges"][exchange_index],
        )  # type: ignore[index]
        responses = cast(list[dict[str, object]], exchange["responses"])
        content_types = tuple(cast(str, item["content_type"]) for item in responses)
    except (KeyError, IndexError, TypeError) as error:
        raise ValueError("MCP HTTP negotiation oracle is invalid") from error
    if content_types != ("application/json", "text/event-stream"):
        raise ValueError("MCP HTTP negotiation oracle is invalid")
    return (
        (content_types[0], "application/json; charset=utf-8"),
        (content_types[1], "text/event-stream"),
    )


def _http_probe(
    executable: Path,
    repo_root: Path,
    work_root: Path,
    target: dict[str, object],
    protocol_path: Path,
    *,
    replay_routes: bool = True,
    extra_call: tuple[Mapping[str, object], object] | None = None,
    probe_profile: str = "canonical",
) -> None:
    routes = _read_object(repo_root / "compatibility/fixtures/http/route-cases.json")
    route = next(
        item
        for item in cast(list[object], routes["routes"])
        if type(item) is dict and item.get("contract_id") == "http.route.post.mcp"
    )
    route_target = cast(dict[str, object], cast(dict[str, object], route)["target"])
    ready = work_root / "http-ready.json"
    for path in (ready,):
        try:
            path.unlink()
        except FileNotFoundError:
            pass
    server = subprocess.Popen(
        (
            str(executable),
            "http",
            "--registry",
            str(repo_root / "compatibility/snapshots/mcp.json"),
            "--protocol",
            str(protocol_path),
            "--route-cases",
            str(repo_root / "compatibility/fixtures/http/route-cases.json"),
            "--bind",
            "127.0.0.1:0",
            "--ready-file",
            str(ready),
        ),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        start_new_session=True,
        close_fds=True,
    )
    reports = bytearray()
    overflow = threading.Event()
    report_errors: list[BaseException] = []

    def drain_reports() -> None:
        try:
            if server.stderr is None:
                raise RuntimeError("MCP HTTP report pipe is unavailable")
            while True:
                chunk = server.stderr.read(64 * 1024)
                if not chunk:
                    return
                reports.extend(chunk)
                if len(reports) > _MAX_CAPTURE:
                    overflow.set()
                    server.stderr.close()
                    return
        except BaseException as error:  # propagated on the controlling thread
            report_errors.append(error)

    report_thread = threading.Thread(target=drain_reports, daemon=True)
    report_thread.start()
    try:
        import time

        deadline = time.monotonic() + 20
        while not ready.is_file():
            if server.poll() is not None:
                raise CandidateMismatch("bounded MCP HTTP server exited before ready")
            if time.monotonic() >= deadline:
                raise CandidateTimeout("bounded MCP HTTP server did not become ready")
            time.sleep(0.01)
        try:
            ready_value = json.loads(ready.read_bytes())
        except (UnicodeError, json.JSONDecodeError) as error:
            raise CandidateMismatch("MCP HTTP ready document is invalid") from error
        if type(ready_value) is not dict:
            raise CandidateMismatch("MCP HTTP ready document is invalid")
        address = ready_value.get("address")
        match = _ADDRESS.fullmatch(address) if type(address) is str else None
        if match is None or int(match.group(1)) > 65535:
            raise CandidateMismatch("MCP HTTP ready address is invalid")
        port = int(match.group(1))
        successes = cast(list[object], route_target["successes"])
        expected_by_id = {
            "tools-list": cast(dict[str, object], target["tools_list"])["response"],
            "tools-call": cast(dict[str, object], target["tools_call"])["response"],
        }
        for success_index, success in enumerate(
            successes if replay_routes and probe_profile == "canonical" else ()
        ):
            case = cast(dict[str, object], success)
            request = cast(dict[str, object], case["request"])
            for accept, expected_content_type in _http_negotiations(target, success_index):
                status, content_type, body = _http_exchange(port, request, accept=accept)
                if (
                    status != 200
                    or content_type != expected_content_type
                    or body != expected_by_id[cast(str, case["id"])]
                ):
                    raise CandidateMismatch("MCP HTTP success differs from the oracle")
        for failure in (
            cast(list[object], route_target["failures"])
            if replay_routes and probe_profile == "canonical"
            else ()
        ):
            case = cast(dict[str, object], failure)
            expected = cast(dict[str, object], case["response"])
            status, content_type, body = _http_exchange(
                port, cast(dict[str, object], case["request"])
            )
            expected_headers = cast(dict[str, object], expected["headers"])
            if (
                status != expected["status"]
                or content_type != expected_headers["Content-Type"]
                or body != expected["body"]
            ):
                raise CandidateMismatch("MCP HTTP failure differs from the oracle")
        if replay_routes and probe_profile == "canonical":
            bridge_request = cast(
                dict[str, object], cast(dict[str, object], successes[0])["request"]
            )
            bridge_request = dict(bridge_request)
            bridge_request["path"] = "/api/mcp/fixture"
            status, _, body = _http_exchange(port, bridge_request)
            if status != 404 or body != {"error": "not_found"}:
                raise CandidateMismatch("private MCP bridge remains present")
        if extra_call is not None:
            for accept, expected_content_type in _http_negotiations(target, 1):
                status, content_type, body = _http_exchange(port, extra_call[0], accept=accept)
                if status != 200 or content_type != expected_content_type or body != extra_call[1]:
                    raise CandidateMismatch("MCP HTTP error result differs from the frozen oracle")
        if probe_profile == "metadata":
            _probe_metadata_cases(port, target, route_target)
        elif probe_profile == "stateless":
            _probe_stateless_cases(port, route_target)
        elif probe_profile == "reserved":
            _probe_reserved_metadata_cases(port, target, route_target)
        elif probe_profile != "canonical":
            raise ValueError("unknown MCP HTTP probe profile")
        if overflow.is_set():
            raise CandidateOutput("MCP HTTP report output exceeds the bound")
    finally:
        if server.poll() is None:
            try:
                os.killpg(server.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        try:
            server.wait(timeout=2)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(server.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            server.wait(timeout=2)
        report_thread.join(timeout=2)
        if report_thread.is_alive():
            raise RuntimeError("MCP HTTP report reader did not terminate")
        if report_errors:
            raise RuntimeError("MCP HTTP report reader failed") from report_errors[0]


def _write_private_json(path: Path, value: object) -> None:
    payload = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    if len(payload) > _MAX_CAPTURE:
        raise ValueError("MCP probe artifact exceeds the bound")
    temporary = path.with_suffix(path.suffix + ".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        os.close(descriptor)
    os.replace(temporary, path)


def _derived_error_oracle(
    repo_root: Path, work_root: Path
) -> tuple[Path, dict[str, object], object]:
    protocol = _read_object(repo_root / "compatibility/fixtures/mcp/protocol.json")
    derived = json.loads(json.dumps(protocol))
    target = cast(dict[str, object], derived["target"])
    wire = _read_object(
        repo_root / "compatibility/fixtures/mcp" / _MCP_TOOL_NAMES[0] / "wire.error.json"
    )
    request = cast(dict[str, object], wire["request"])
    request = json.loads(json.dumps(request))
    canonical_call = cast(dict[str, object], target["tools_call"])
    metadata = cast(
        dict[str, object], cast(dict[str, object], canonical_call["request"])["params"]
    )["_meta"]
    cast(dict[str, object], request["params"])["_meta"] = metadata
    result = dict(cast(dict[str, object], wire["result"]))
    result["resultType"] = "complete"
    response = {"jsonrpc": "2.0", "id": request["id"], "result": result}
    target["tools_call"] = {"request": request, "response": response}
    target["requests"] = [cast(dict[str, object], target["tools_list"])["request"], request]
    stdio = cast(dict[str, object], target["stdio"])
    exchanges = cast(list[dict[str, object]], stdio["exchanges"])
    exchanges[1] = {
        "request_line": json.dumps(request, sort_keys=True, separators=(",", ":")) + "\n",
        "response_line": json.dumps(response, sort_keys=True, separators=(",", ":")) + "\n",
    }
    streamable = cast(dict[str, object], target["streamable_http"])
    stream_exchange = cast(list[dict[str, object]], streamable["exchanges"])[1]
    cast(dict[str, object], stream_exchange["request"])["body"] = request
    stream_exchange["responses"] = [
        {"content_type": "application/json", "body": response},
        {"content_type": "text/event-stream", "events": [{"event": "message", "data": response}]},
    ]
    path = work_root / "derived-error-protocol.json"
    _write_private_json(path, derived)

    routes = _read_object(repo_root / "compatibility/fixtures/http/route-cases.json")
    route = next(
        item
        for item in cast(list[object], routes["routes"])
        if type(item) is dict and item.get("contract_id") == "http.route.post.mcp"
    )
    success = cast(dict[str, object], cast(dict[str, object], route)["target"])["successes"]  # type: ignore[index]
    call_route = json.loads(json.dumps(cast(list[object], success)[1]["request"]))  # type: ignore[index]
    call_route["body"] = request
    call_route["headers"]["Mcp-Name"] = cast(dict[str, object], request["params"])["name"]
    return path, call_route, response


def _toolchain_observations(
    repo_root: Path, cargo: Path, rustup: Path
) -> tuple[dict[str, str], tuple[LockedDependency, ...], int, bool]:
    cargo_argv = (str(cargo), "+1.96.0")
    code, cargo_version_raw, _ = _run_bounded((*cargo_argv, "--version"), cwd=repo_root)
    if code != 0:
        raise ValueError("Cargo version measurement failed")
    code, rustc_raw, _ = _run_bounded(
        (str(rustup), "run", "1.96.0", "rustc", "--version", "--verbose"), cwd=repo_root
    )
    if code != 0:
        raise ValueError("Rust version measurement failed")
    metadata_argv = (
        *cargo_argv,
        "metadata",
        "--manifest-path",
        _MANIFEST,
        "--locked",
        "--format-version",
        "1",
    )
    tree_argv = (*cargo_argv, "tree", "--manifest-path", _MANIFEST, "--locked", "-e", "features")
    code, metadata_raw, _ = _run_bounded(metadata_argv, cwd=repo_root)
    if code != 0:
        raise ValueError("Cargo metadata measurement failed")
    tree_code, tree_raw, _ = _run_bounded(tree_argv, cwd=repo_root)
    try:
        metadata = json.loads(metadata_raw)
        packages = metadata["packages"]
        nodes = metadata["resolve"]["nodes"]
    except (UnicodeError, json.JSONDecodeError, KeyError, TypeError) as error:
        raise ValueError("Cargo metadata output is invalid") from error
    features_by_id = {node["id"]: tuple(sorted(node.get("features", []))) for node in nodes}
    lock = tomllib.loads(
        (repo_root / "qualification/harnesses/mcp-transport/Cargo.lock").read_text(encoding="utf-8")
    )
    checksums = {
        (item.get("name"), item.get("version"), item.get("source")): item.get("checksum")
        for item in lock.get("package", [])
        if type(item) is dict
    }
    dependencies: list[LockedDependency] = []
    for package in packages:
        if package["name"] == "hieronymus-mcp-transport-qualification":
            continue
        source = package.get("source") or "locked-local"
        dependencies.append(
            LockedDependency(
                name=package["name"],
                version=package["version"],
                source=source,
                checksum=package.get("checksum")
                or checksums.get((package["name"], package["version"], package.get("source"))),
                features=features_by_id.get(package["id"], ()),
            )
        )
    rmcp = next((item for item in dependencies if item.name == "rmcp"), None)
    required_features = {"server", "transport-io", "transport-streamable-http-server"}
    graph_matches = (
        tree_code == 0
        and bool(tree_raw.strip())
        and rmcp is not None
        and required_features.issubset(rmcp.features)
    )
    rustc_text = rustc_raw.decode("utf-8", errors="strict").strip()
    cargo_text = cargo_version_raw.decode("utf-8", errors="strict").strip()
    host = next(
        (
            line.split(":", 1)[1].strip()
            for line in rustc_text.splitlines()
            if line.startswith("host:")
        ),
        None,
    )
    if host != _TARGET:
        raise ValueError("measured Rust target differs from qualification target")
    environment = {
        "rustc": rustc_text.splitlines()[0],
        "cargo": cargo_text,
        "target": host,
        "os": platform.system().lower(),
        "kernel": platform.release(),
        "architecture": platform.machine(),
    }
    return (
        environment,
        tuple(sorted(dependencies, key=lambda item: (item.name, item.version))),
        len(tree_raw),
        graph_matches,
    )


def _require_owned_probe_environment(repo_root: Path, work_root: Path) -> None:
    expected_keys = {
        _PROBE_MARKER,
        "HOME",
        "TMPDIR",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "CARGO_TARGET_DIR",
        "CARGO_NET_OFFLINE",
        "RUSTUP_AUTO_INSTALL",
        "PATH",
        "LANG",
        "LC_ALL",
        "TZ",
    }
    expected_paths = {
        "HOME": work_root / "home",
        "TMPDIR": work_root / "tmp",
        "XDG_CACHE_HOME": work_root / "xdg/cache",
        "XDG_CONFIG_HOME": work_root / "xdg/config",
        "XDG_DATA_HOME": work_root / "xdg/data",
        "CARGO_TARGET_DIR": repo_root / _CARGO_TARGET,
    }
    if (
        os.getpid() != 2
        or os.getppid() != 1
        or set(os.environ) != expected_keys
        or os.environ.get(_PROBE_MARKER) != "1"
        or os.environ.get("CARGO_NET_OFFLINE") != "true"
        or os.environ.get("RUSTUP_AUTO_INSTALL") != "0"
        or os.environ.get("LANG") != "C.UTF-8"
        or os.environ.get("LC_ALL") != "C.UTF-8"
        or os.environ.get("TZ") != "UTC"
        or any(os.environ.get(name) != str(path) for name, path in expected_paths.items())
    ):
        raise ValueError("MCP hidden mode requires the owned probe environment")


def _probe_measurements(tree_bytes: int) -> dict[str, Mapping[str, object]]:
    return {
        "locked-native-build": {"locked_commands": 3, "feature_tree_bytes": tree_bytes},
        "protocol-2026-07-28": {"protocol_revision": "2026-07-28"},
        "no-handshake-or-session": {"stateful_fields": 0},
        "per-request-required-metadata": {"requests": 2},
        "unsupported-version-rejected": {"cases": 1},
        "stdio-newline-jsonrpc": {"exchanges": 4},
        "streamable-http-json": {"exchanges": 3},
        "streamable-http-sse": {"exchanges": 3},
        "http-method-name-headers": {"failure_cases": 5},
        "http-host-auth-version-cases": {"route_cases": 11},
        "official-schema-envelopes": {"tool_wire_results": 78},
        "header-mismatch-errors": {"cases": 7},
        "registry-identity": {"tools": 39},
        "result-error-identity": {"call_outcomes_per_transport": 2},
        "result-type-required": {"transport_variants": 6},
        "required-auth-metadata": {"reserved_fields": 3},
        "private-bridge-absent": {"expected_status": 404},
    }


def _checks_from_observations(
    observed: set[str], *, tree_bytes: int
) -> dict[str, tuple[bool, Mapping[str, object]]]:
    """Resolve criteria only from their complete, named live observations."""
    measurements = _probe_measurements(tree_bytes)
    checks: dict[str, tuple[bool, Mapping[str, object]]] = {}
    for criterion in REQUIRED_CRITERIA[_RISK]:
        required = _CRITERION_OBSERVATIONS[criterion]
        missing = tuple(name for name in required if name not in observed)
        if not missing:
            checks[criterion] = (True, measurements[criterion])
            continue
        failed_measurements = (
            dict(measurements[criterion]) if criterion == "locked-native-build" else {}
        )
        failed_measurements["failed_observations"] = len(missing)
        checks[criterion] = (False, failed_measurements)
    return checks


def _collect_candidate_group(
    observed: set[str],
    observations: str | Sequence[str],
    operation: Callable[[], None],
) -> None:
    try:
        operation()
    except CandidateMismatch:
        return
    if isinstance(observations, str):
        observed.add(observations)
    else:
        observed.update(observations)


def _transport_probe(
    executable: Path,
    repo_root: Path,
    work_root: Path,
    artifact: Path,
    cargo: Path,
    rustup: Path,
) -> None:
    """Replay both transports as descendants of one owned PID namespace."""
    expected_executable = repo_root / _CARGO_TARGET / _TARGET / "debug/mcp-transport"
    expected_work = repo_root / _LIVE_WORK
    _require_owned_probe_environment(repo_root, work_root)
    if executable != expected_executable or executable.is_symlink():
        raise ValueError("MCP transport executable is outside the qualification target")
    if executable.exists() and not executable.is_file():
        raise ValueError("MCP transport executable is outside the qualification target")
    if work_root != expected_work or work_root.is_symlink() or not work_root.is_dir():
        raise ValueError("MCP transport work root is outside the qualification boundary")
    if artifact != work_root / "probe-result.json" or artifact.exists() or artifact.is_symlink():
        raise ValueError("MCP probe artifact is outside the qualification boundary")
    environment, dependencies, tree_bytes, graph_matches = _toolchain_observations(
        repo_root, cargo, rustup
    )
    observed: set[str] = set()
    if graph_matches and executable.is_file():
        observed.add("locked-toolchain")
    if executable.is_file():
        target = cast(
            dict[str, object],
            _read_object(repo_root / "compatibility/fixtures/mcp/protocol.json")["target"],
        )
        protocol_path = repo_root / "compatibility/fixtures/mcp/protocol.json"
        _collect_candidate_group(
            observed,
            "canonical-stdio",
            lambda: _stdio_probe(executable, repo_root, target, protocol_path),
        )
        _collect_candidate_group(
            observed,
            (
                "canonical-http",
                "unsupported-version",
                "route-header-negatives",
                "route-security-negatives",
                "private-bridge-negative",
            ),
            lambda: _http_probe(executable, repo_root, work_root, target, protocol_path),
        )
        _collect_candidate_group(
            observed,
            ("metadata-negatives", "legacy-metadata-negative", "client-info-cases"),
            lambda: _http_probe(
                executable,
                repo_root,
                work_root,
                target,
                protocol_path,
                replay_routes=False,
                probe_profile="metadata",
            ),
        )
        _collect_candidate_group(
            observed,
            "stateless-negatives",
            lambda: _http_probe(
                executable,
                repo_root,
                work_root,
                target,
                protocol_path,
                replay_routes=False,
                probe_profile="stateless",
            ),
        )
        _collect_candidate_group(
            observed,
            "reserved-metadata",
            lambda: _http_probe(
                executable,
                repo_root,
                work_root,
                target,
                protocol_path,
                replay_routes=False,
                probe_profile="reserved",
            ),
        )
        derived_path, error_request, error_response = _derived_error_oracle(repo_root, work_root)
        derived_target = cast(dict[str, object], _read_object(derived_path)["target"])
        _collect_candidate_group(
            observed,
            "error-stdio",
            lambda: _stdio_probe(executable, repo_root, derived_target, derived_path),
        )
        _collect_candidate_group(
            observed,
            "error-http",
            lambda: _http_probe(
                executable,
                repo_root,
                work_root,
                derived_target,
                derived_path,
                replay_routes=False,
                extra_call=(error_request, error_response),
            ),
        )
    checks = _checks_from_observations(observed, tree_bytes=tree_bytes)
    payload = {
        "schemaVersion": _PROBE_SCHEMA_VERSION,
        "checks": {
            criterion: {
                "passed": checks[criterion][0],
                "measurements": checks[criterion][1],
            }
            for criterion in REQUIRED_CRITERIA[_RISK]
        },
        "environment": environment,
        "dependencies": [
            {
                "name": item.name,
                "version": item.version,
                "source": item.source,
                "checksum": item.checksum,
                "features": list(item.features),
            }
            for item in dependencies
        ],
    }
    _probe_observations(payload)
    _write_private_json(artifact, payload)


def run_live(
    repo_root: Path,
    work_root: Path,
    *,
    original_env: Mapping[str, str],
) -> QualificationRecord:
    """Run the accepted candidate under the offline, owned-process boundary."""
    caller_env = dict(original_env)
    if caller_env.get("HIERONYMUS_QUALIFICATION_LIVE") != "1":
        raise ValueError("HIERONYMUS_QUALIFICATION_LIVE=1 is required")
    contract_ids = _validate_oracles(repo_root)
    before = fingerprint_inputs(repo_root, required_fingerprint_inputs(_RISK))
    _prepare_private_work(work_root)
    receipts: list[ProcessReceipt] = []
    observations: _ProbeObservations | None = None
    work_removed = False
    install_removed = False
    try:
        tool_roots, cargo_target_dir, child_env = _live_process_context(
            repo_root, work_root, caller_env
        )
        cargo = str(tool_roots.cargo_invocation)
        child_env[_PROBE_MARKER] = "1"
        shared = {
            "cwd": repo_root,
            "env": child_env,
            "timeout_seconds": 120,
            "no_progress_seconds": 60,
        }
        for argv in (
            (
                cargo,
                "+1.96.0",
                "metadata",
                "--manifest-path",
                _MANIFEST,
                "--locked",
                "--format-version",
                "1",
            ),
            (
                cargo,
                "+1.96.0",
                "tree",
                "--manifest-path",
                _MANIFEST,
                "--locked",
                "-e",
                "features",
            ),
            (
                cargo,
                "+1.96.0",
                "test",
                "--manifest-path",
                _MANIFEST,
                "--locked",
                "--target",
                _TARGET,
            ),
        ):
            receipts.append(run_owned_process(argv, **shared))
        executable = cargo_target_dir / _TARGET / "debug/mcp-transport"
        artifact = work_root / "probe-result.json"
        receipts.append(
            run_owned_process(
                (
                    sys.executable,
                    "-B",
                    "-m",
                    "tools.qualification.run_mcp",
                    "--transport-probe",
                    str(executable),
                    str(work_root),
                    str(artifact),
                    str(tool_roots.cargo_invocation),
                    str(tool_roots.rustup_invocation),
                ),
                cwd=repo_root,
                env=child_env,
                timeout_seconds=120,
                no_progress_seconds=60,
            )
        )
        if _successful(receipts[-1]):
            observations = _read_probe_artifact(artifact)
    finally:
        work_removed = _remove_exact_directory(work_root)
        install_removed = _remove_exact_directory(repo_root / _CARGO_TARGET)
    after = fingerprint_inputs(repo_root, required_fingerprint_inputs(_RISK))
    if after != before:
        raise ValueError("immutable MCP qualification inputs changed during replay")
    if observations is None:
        raise ValueError("MCP typed probe artifact was not produced by the owned probe")
    if len(receipts) != 4 or not all(_successful(receipt) for receipt in receipts[:3]):
        checks = dict(observations.checks)
        _, prior = checks["locked-native-build"]
        failed = dict(prior)
        failed["failed_observations"] = int(failed.get("failed_observations", 0)) + 1
        checks["locked-native-build"] = (False, failed)
        observations = _ProbeObservations(
            checks=checks,
            environment=observations.environment,
            dependencies=observations.dependencies,
        )
    return _record(
        repo_root,
        contract_ids=contract_ids,
        observations=observations,
        work_dir_removed=work_removed,
        raw_logs_removed=work_removed,
        install_dir_removed=install_removed,
        core_dumps_disabled=bool(receipts) and all(item.core_dumps_disabled for item in receipts),
        process_groups_reaped=bool(receipts)
        and all(item.process_group_reaped for item in receipts),
    )


def _write_record(repo_root: Path, record: QualificationRecord) -> None:
    record_path = repo_root / _RECORD
    markdown_path = repo_root / _MARKDOWN
    record_path.parent.mkdir(parents=True, exist_ok=True)
    markdown_path.parent.mkdir(parents=True, exist_ok=True)
    record_path.write_text(serialize_record(record) + "\n", encoding="utf-8")
    markdown_path.write_text(render_record(record), encoding="utf-8")


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run the offline MCP transport qualification")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument(
        "--transport-probe",
        nargs=5,
        metavar=("EXECUTABLE", "WORK_ROOT", "ARTIFACT", "CARGO", "RUSTUP"),
    )
    arguments = parser.parse_args(argv)
    repo_root = Path.cwd()
    if arguments.transport_probe is not None:
        try:
            executable, work_root, artifact, cargo, rustup = (
                Path(item) for item in arguments.transport_probe
            )
            _transport_probe(executable, repo_root, work_root, artifact, cargo, rustup)
        except (OSError, TypeError, ValueError, RuntimeError):
            print("MCP transport probe failed", file=sys.stderr)
            return 2
        return 0
    try:
        record = run_live(
            repo_root,
            repo_root / _LIVE_WORK,
            original_env=dict(os.environ),
        )
        _write_record(repo_root, record)
    except (OSError, TypeError, ValueError, RuntimeError):
        print("MCP transport qualification failed", file=sys.stderr)
        return 2
    print(f"MCP transport qualification recorded: {record.decision}")
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
