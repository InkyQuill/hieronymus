"""Offline MCP transport qualification runner and canonical evidence writer."""

# ruff: noqa: E501 -- replay commands are byte-exact validation allow-list entries.

from __future__ import annotations

import argparse
import http.client
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tomllib
from collections.abc import Mapping, Sequence
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
        ids.append(cast(str, contract["id"]))
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
    for name in _MCP_TOOL_NAMES:
        directory = root / name
        policy_leaves = {
            item.name
            for item in directory.iterdir()
            if item.is_file() and item.name in expected_leaves and not item.is_symlink()
        }
        if policy_leaves != expected_leaves:
            raise ValueError("MCP fixture policy leaf inventory is invalid")
        if any(item.is_dir() for item in directory.iterdir()):
            raise ValueError("MCP fixture policy has an intermediate grouping directory")
        for leaf in expected_leaves:
            _read_object(directory / leaf)


def _validate_envelope(repo_root: Path, envelope: object) -> None:
    if type(envelope) is not dict or type(envelope.get("id")) is not int:
        raise ValueError("MCP response envelope is invalid")
    definition = _DEFINITION_BY_ID.get(cast(int, envelope["id"]))
    if definition is None or definition_issues(repo_root, definition, envelope):  # type: ignore[arg-type]
        raise ValueError("MCP response violates the official schema")


def _validate_oracles(repo_root: Path) -> tuple[str, ...]:
    if authority_issues(repo_root):
        raise ValueError("official MCP schema authority is invalid")
    contract_ids = _mcp_projection(repo_root)
    _validate_policy_inventory(repo_root)

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
    if type(target) is not dict:
        raise ValueError("MCP protocol target is invalid")
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
    for request in target.get("requests", []):
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
    for exchange in stdio.get("exchanges", []):
        if type(exchange) is not dict or type(exchange.get("response_line")) is not str:
            raise ValueError("MCP stdio exchange is invalid")
        response_line = cast(str, exchange["response_line"])
        if not response_line.endswith("\n") or response_line.count("\n") != 1:
            raise ValueError("MCP stdio exchange framing is invalid")
        _validate_envelope(repo_root, json.loads(response_line))
    streamable = target.get("streamable_http")
    if type(streamable) is not dict or len(streamable.get("exchanges", [])) != 2:
        raise ValueError("MCP Streamable HTTP oracle is invalid")
    for exchange in streamable["exchanges"]:
        if type(exchange) is not dict or type(exchange.get("responses")) is not list:
            raise ValueError("MCP Streamable HTTP exchange is invalid")
        content_types = set()
        for response in exchange["responses"]:
            if type(response) is not dict:
                raise ValueError("MCP Streamable HTTP response is invalid")
            content_types.add(response.get("content_type"))
            body = response.get("body")
            if type(body) is dict:
                _validate_envelope(repo_root, body)
            elif type(response.get("events")) is list:
                events = cast(list[object], response["events"])
                if len(events) != 1 or type(events[0]) is not dict:
                    raise ValueError("MCP SSE event is invalid")
                _validate_envelope(repo_root, cast(dict[str, object], events[0]).get("data"))
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
        _validate_envelope(repo_root, success.get("response", {}).get("body"))  # type: ignore[union-attr]
    for failure in failures:
        if type(failure) is not dict or type(failure.get("response")) is not dict:
            raise ValueError("MCP HTTP failure is invalid")
        response = cast(dict[str, object], failure["response"])
        body = response.get("body")
        failure_id = failure.get("id")
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
    if any(
        type(item) is dict
        and type(item.get("request")) is dict
        and cast(dict[str, object], item["request"]).get("path") == "/api/mcp/fixture"
        for item in (*successes, *failures)
    ):
        raise ValueError("private MCP bridge route remains present")
    return contract_ids


def _read_fake_failures(executable: Path, repo_root: Path) -> tuple[str, ...]:
    try:
        completed = subprocess.run(
            (str(executable),),
            cwd=repo_root,
            env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC"},
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            timeout=20,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError("MCP qualification executable failed") from error
    if completed.returncode != 0 or len(completed.stdout) > 1024 * 1024:
        raise ValueError("MCP qualification executable failed")
    try:
        payload = json.loads(completed.stdout)
        failures = payload["failed_criteria"]
    except (UnicodeError, json.JSONDecodeError, KeyError, TypeError) as error:
        raise ValueError("MCP qualification executable report is invalid") from error
    if type(failures) is not list or any(type(item) is not str for item in failures):
        raise ValueError("MCP qualification executable report is invalid")
    result = tuple(cast(list[str], failures))
    if len(result) != len(set(result)) or set(result) - set(REQUIRED_CRITERIA[_RISK]):
        raise ValueError("MCP qualification executable criteria are invalid")
    return result


def _dependencies(repo_root: Path) -> tuple[LockedDependency, ...]:
    lock = tomllib.loads(
        (repo_root / "qualification/harnesses/mcp-transport/Cargo.lock").read_text(encoding="utf-8")
    )
    result = []
    for package in lock.get("package", []):
        if (
            type(package) is not dict
            or package.get("name") == "hieronymus-mcp-transport-qualification"
        ):
            continue
        source = package.get("source")
        result.append(
            LockedDependency(
                name=cast(str, package["name"]),
                version=cast(str, package["version"]),
                source=source if type(source) is str else "locked-local",
                checksum=package.get("checksum") if type(package.get("checksum")) is str else None,
                features=(),
            )
        )
    return tuple(sorted(result, key=lambda item: (item.name, item.version, item.source)))


def _record(
    repo_root: Path,
    *,
    contract_ids: tuple[str, ...],
    failed: tuple[str, ...],
    core_dumps_disabled: bool = True,
    process_groups_reaped: bool = True,
) -> QualificationRecord:
    measurements: dict[str, Mapping[str, object]] = {
        "locked-native-build": {"locked_commands": 3},
        "protocol-2026-07-28": {"protocol_revision": "2026-07-28"},
        "no-handshake-or-session": {"stateful_fields": 0},
        "per-request-required-metadata": {"requests": 2},
        "unsupported-version-rejected": {"cases": 1},
        "stdio-newline-jsonrpc": {"exchanges": 2},
        "streamable-http-json": {"exchanges": 2},
        "streamable-http-sse": {"exchanges": 2},
        "http-method-name-headers": {"failure_cases": 5},
        "http-host-auth-version-cases": {"route_cases": 11},
        "official-schema-envelopes": {"definitions": 4},
        "header-mismatch-errors": {"cases": 7},
        "registry-identity": {"tools": 39},
        "result-error-identity": {"call_outcomes": 2},
        "result-type-required": {"transport_variants": 6},
        "required-auth-metadata": {"reserved_fields": 3},
        "private-bridge-absent": {"expected_status": 404},
    }
    evidence = tuple(
        Evidence(
            criterion=criterion,
            status="fail" if criterion in failed else "pass",
            summary=(
                f"{criterion} failed in the bounded offline replay"
                if criterion in failed
                else f"{criterion} passed the bounded offline replay"
            ),
            measurements=Measurements(measurements[criterion]),
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
            rustc="rustc 1.96.0 (ac68faa20 2026-05-25)",
            cargo="cargo 1.96.0",
            target=_TARGET,
            os="linux",
            kernel=platform.release(),
            architecture="x86_64",
            bun=None,
            native_libraries=(),
        ),
        dependencies=_dependencies(repo_root),
        evidence=evidence,
        consequence=expected_consequence(_RISK, status),
        cleanup=CleanupEvidence(
            work_dir_removed=True,
            raw_logs_removed=True,
            install_dir_removed=True,
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
    try:
        failed = _read_fake_failures(executable, repo_root)
    finally:
        shutil.rmtree(bounded_work)
    after = fingerprint_inputs(repo_root, required_fingerprint_inputs(_RISK))
    if after != before:
        raise ValueError("immutable MCP qualification inputs changed during replay")
    return _record(repo_root, contract_ids=contract_ids, failed=failed)


def _live_process_context(
    repo_root: Path,
    work_root: Path,
    original_env: Mapping[str, str],
) -> tuple[ToolRoots, Path, dict[str, str]]:
    """Discover unsanitized tool roots, then produce one shared safe environment."""
    tool_roots = discover_tool_roots(original_env)
    cargo_target_dir = repo_root / _CARGO_TARGET
    child_env = safe_subprocess_env(
        work_root,
        cargo_offline=True,
        tool_roots=tool_roots,
        cargo_target_dir=cargo_target_dir,
    )
    return tool_roots, cargo_target_dir, child_env


def _successful(receipt: ProcessReceipt) -> bool:
    return (
        receipt.exit_code == 0
        and not receipt.timed_out
        and receipt.core_dumps_disabled
        and receipt.process_group_reaped
    )


def _stdio_probe(executable: Path, repo_root: Path, target: dict[str, object]) -> None:
    for key in ("tools_list", "tools_call"):
        exchange = target.get(key)
        if type(exchange) is not dict:
            raise ValueError("MCP stdio target exchange is invalid")
        request = exchange.get("request")
        expected = exchange.get("response")
        payload = json.dumps(request, sort_keys=True, separators=(",", ":")).encode() + b"\n"
        try:
            completed = subprocess.run(
                (
                    str(executable),
                    "stdio",
                    "--registry",
                    str(repo_root / "compatibility/snapshots/mcp.json"),
                    "--protocol",
                    str(repo_root / "compatibility/fixtures/mcp/protocol.json"),
                ),
                input=payload,
                capture_output=True,
                timeout=20,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ValueError("bounded MCP stdio replay failed") from error
        if completed.returncode != 0 or completed.stdout.count(b"\n") != 1:
            raise ValueError("bounded MCP stdio replay failed")
        try:
            actual = json.loads(completed.stdout)
        except (UnicodeError, json.JSONDecodeError) as error:
            raise ValueError("bounded MCP stdio response is invalid") from error
        if actual != expected:
            raise ValueError("bounded MCP stdio response differs from the oracle")


def _http_exchange(
    port: int,
    request: Mapping[str, object],
    *,
    accept: str | None = None,
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
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=20)
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
    finally:
        connection.close()
    if len(raw) > 2 * 1024 * 1024:
        raise ValueError("MCP HTTP response exceeds the bound")
    try:
        if content_type == "text/event-stream":
            text = raw.decode("utf-8", errors="strict")
            data_lines = [line[6:] for line in text.splitlines() if line.startswith("data: ")]
            if len(data_lines) != 1:
                raise ValueError("MCP SSE response framing is invalid")
            payload = json.loads(data_lines[0])
        else:
            payload = json.loads(raw)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("MCP HTTP response is invalid") from error
    return response.status, content_type, payload


def _http_probe(
    executable: Path,
    repo_root: Path,
    work_root: Path,
    target: dict[str, object],
) -> None:
    routes = _read_object(repo_root / "compatibility/fixtures/http/route-cases.json")
    route = next(
        item
        for item in cast(list[object], routes["routes"])
        if type(item) is dict and item.get("contract_id") == "http.route.post.mcp"
    )
    route_target = cast(dict[str, object], cast(dict[str, object], route)["target"])
    ready = work_root / "http-ready.json"
    raw_log = work_root / "http-report.log"
    for path in (ready, raw_log):
        try:
            path.unlink()
        except FileNotFoundError:
            pass
    with raw_log.open("wb") as log:
        server = subprocess.Popen(
            (
                str(executable),
                "http",
                "--registry",
                str(repo_root / "compatibility/snapshots/mcp.json"),
                "--protocol",
                str(repo_root / "compatibility/fixtures/mcp/protocol.json"),
                "--route-cases",
                str(repo_root / "compatibility/fixtures/http/route-cases.json"),
                "--bind",
                "127.0.0.1:0",
                "--ready-file",
                str(ready),
            ),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=log,
        )
    try:
        import time

        deadline = time.monotonic() + 20
        while not ready.is_file():
            if server.poll() is not None or time.monotonic() >= deadline:
                raise ValueError("bounded MCP HTTP server did not become ready")
            time.sleep(0.01)
        ready_value = _read_object(ready)
        address = ready_value.get("address")
        match = _ADDRESS.fullmatch(address) if type(address) is str else None
        if match is None or int(match.group(1)) > 65535:
            raise ValueError("MCP HTTP ready address is invalid")
        port = int(match.group(1))
        successes = cast(list[object], route_target["successes"])
        expected_by_id = {
            "tools-list": cast(dict[str, object], target["tools_list"])["response"],
            "tools-call": cast(dict[str, object], target["tools_call"])["response"],
        }
        for success in successes:
            case = cast(dict[str, object], success)
            request = cast(dict[str, object], case["request"])
            for accept, expected_content_type in (
                ("application/json", "application/json; charset=utf-8"),
                ("text/event-stream", "text/event-stream"),
            ):
                status, content_type, body = _http_exchange(port, request, accept=accept)
                if (
                    status != 200
                    or content_type != expected_content_type
                    or body != expected_by_id[cast(str, case["id"])]
                ):
                    raise ValueError("MCP HTTP success differs from the oracle")
        for failure in cast(list[object], route_target["failures"]):
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
                raise ValueError("MCP HTTP failure differs from the oracle")
        bridge_request = cast(dict[str, object], cast(dict[str, object], successes[0])["request"])
        bridge_request = dict(bridge_request)
        bridge_request["path"] = "/api/mcp/fixture"
        status, _, body = _http_exchange(port, bridge_request)
        if status != 404 or body != {"error": "not_found"}:
            raise ValueError("private MCP bridge remains present")
    finally:
        if server.poll() is None:
            server.terminate()
        try:
            server.wait(timeout=20)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait(timeout=20)


def _transport_probe(executable: Path, repo_root: Path, work_root: Path) -> None:
    """Replay both transports as descendants of one owned PID namespace."""
    expected_executable = repo_root / _CARGO_TARGET / _TARGET / "debug/mcp-transport"
    expected_work = repo_root / _LIVE_WORK
    if executable != expected_executable or executable.is_symlink() or not executable.is_file():
        raise ValueError("MCP transport executable is outside the qualification target")
    if work_root != expected_work or work_root.is_symlink() or not work_root.is_dir():
        raise ValueError("MCP transport work root is outside the qualification boundary")
    target = cast(
        dict[str, object],
        _read_object(repo_root / "compatibility/fixtures/mcp/protocol.json")["target"],
    )
    _stdio_probe(executable, repo_root, target)
    _http_probe(executable, repo_root, work_root, target)


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
    work_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    work_root.chmod(0o700)
    receipts: list[ProcessReceipt] = []
    try:
        tool_roots, cargo_target_dir, child_env = _live_process_context(
            repo_root, work_root, caller_env
        )
        cargo = str(tool_roots.cargo_invocation)
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
                ),
                cwd=repo_root,
                env=child_env,
                timeout_seconds=120,
                no_progress_seconds=60,
            )
        )
    finally:
        shutil.rmtree(work_root, ignore_errors=True)
        shutil.rmtree(repo_root / _CARGO_TARGET, ignore_errors=True)
    after = fingerprint_inputs(repo_root, required_fingerprint_inputs(_RISK))
    if after != before:
        raise ValueError("immutable MCP qualification inputs changed during replay")
    failed: tuple[str, ...] = ()
    if len(receipts) != 4 or not all(_successful(receipt) for receipt in receipts):
        failed = REQUIRED_CRITERIA[_RISK]
    return _record(
        repo_root,
        contract_ids=contract_ids,
        failed=failed,
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
    mode.add_argument("--transport-probe", nargs=2, metavar=("EXECUTABLE", "WORK_ROOT"))
    arguments = parser.parse_args(argv)
    repo_root = Path.cwd()
    if arguments.transport_probe is not None:
        try:
            executable, work_root = (Path(item) for item in arguments.transport_probe)
            _transport_probe(executable, repo_root, work_root)
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
