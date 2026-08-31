"""Generate the reviewed HTTP, WebSocket, and frontend request inventory."""

from __future__ import annotations

import argparse
import ast
import json
import re
from dataclasses import asdict, dataclass
from pathlib import Path

_ADR_0012 = "docs/adr/0012-mcp-transport-authentication-and-discovery.md"
_ADR_0015 = "docs/adr/0015-mcp-protocol-and-transport.md"


@dataclass(frozen=True)
class Route:
    contract_id: str
    method: str
    path_template: str
    surface: str
    handler: str
    current_auth: str
    target_auth: str
    disposition: str
    adr: str | None
    request_shape: object
    response_shape: object
    query_fields: tuple[str, ...] = ()


_BROWSER_READ_AUTH = "daemon-token-or-same-origin-browser-request"
_BROWSER_WRITE_AUTH = "daemon-token-or-same-origin-browser-request"


REVIEWED_ROUTES = (
    Route(
        "frontend.route.get.root",
        "GET",
        "/",
        "frontend",
        "_send_web_app",
        "public-static",
        "public-static",
        "preserve",
        None,
        None,
        "text/html",
    ),
    Route(
        "frontend.route.get.admin",
        "GET",
        "/admin",
        "frontend",
        "_send_web_app",
        "public-static",
        "public-static",
        "preserve",
        None,
        None,
        "text/html",
    ),
    Route(
        "frontend.route.get.admin.path",
        "GET",
        "/admin/{path}",
        "frontend",
        "_send_web_app",
        "public-static",
        "public-static",
        "preserve",
        None,
        None,
        "text/html",
    ),
    Route(
        "frontend.route.get.config",
        "GET",
        "/config",
        "frontend",
        "_send_web_app",
        "public-static",
        "public-static",
        "preserve",
        None,
        None,
        "text/html",
    ),
    Route(
        "frontend.route.get.config.path",
        "GET",
        "/config/{path}",
        "frontend",
        "_send_web_app",
        "public-static",
        "public-static",
        "preserve",
        None,
        None,
        "text/html",
    ),
    Route(
        "frontend.route.get.assets.path",
        "GET",
        "/assets/{path}",
        "frontend",
        "_send_web_asset",
        "public-static",
        "public-static",
        "preserve",
        None,
        None,
        "bytes with detected Content-Type",
    ),
    Route(
        "http.route.get.health",
        "GET",
        "/health",
        "http",
        "do_GET",
        "daemon-token",
        "unauthenticated-minimal-liveness",
        "intentionally-change",
        _ADR_0012,
        None,
        {"ok": "boolean", "service": "string", "version": "string"},
    ),
    Route(
        "http.route.get.status",
        "GET",
        "/status",
        "http",
        "status_payload",
        "daemon-token",
        "bearer-token",
        "intentionally-change",
        _ADR_0012,
        None,
        "StatusPayload",
    ),
    Route(
        "http.route.post.shutdown",
        "POST",
        "/shutdown",
        "http",
        "do_POST",
        "daemon-token",
        "bearer-token",
        "intentionally-change",
        _ADR_0012,
        None,
        {"ok": "boolean", "stopping": "boolean"},
    ),
    Route(
        "http.route.post.mcp",
        "POST",
        "/mcp",
        "http",
        "standard Streamable HTTP endpoint",
        "absent",
        "bearer-token-and-MCP-Protocol-Version",
        "intentionally-change",
        _ADR_0015,
        "MCP JSON-RPC 2.0 request",
        "MCP JSON-RPC 2.0 response or request-scoped SSE stream",
    ),
    Route(
        "http.route.post.api.mcp.operation",
        "POST",
        "/api/mcp/{operation}",
        "http",
        "MCP_OPERATION_HANDLERS",
        "daemon-token",
        "removed",
        "remove",
        _ADR_0015,
        "operation-specific JSON object",
        {"result": "operation-specific JSON value"},
    ),
    Route(
        "http.route.get.api.providers",
        "GET",
        "/api/providers",
        "http",
        "ConfigBridge.provider_list",
        _BROWSER_READ_AUTH,
        "browser-session",
        "intentionally-change",
        _ADR_0012,
        None,
        {"providers": "ProviderProfile[]"},
    ),
    Route(
        "http.route.post.api.providers",
        "POST",
        "/api/providers",
        "http",
        "ConfigBridge.save_provider",
        _BROWSER_WRITE_AUTH,
        "browser-session-and-CSRF-token",
        "intentionally-change",
        _ADR_0012,
        {"provider": "ProviderDraft"},
        {"provider": "ProviderProfile"},
    ),
    Route(
        "http.route.get.api.providers.id",
        "GET",
        "/api/providers/{id}",
        "http",
        "ConfigBridge.provider_detail",
        _BROWSER_READ_AUTH,
        "browser-session",
        "intentionally-change",
        _ADR_0012,
        None,
        {"provider": "ProviderProfile"},
    ),
    Route(
        "http.route.get.api.providers.id.models",
        "GET",
        "/api/providers/{id}/models",
        "http",
        "ConfigBridge.provider_models",
        _BROWSER_READ_AUTH,
        "browser-session",
        "intentionally-change",
        _ADR_0012,
        None,
        {"models": "string[]"},
    ),
    Route(
        "http.route.post.api.providers.id.check",
        "POST",
        "/api/providers/{id}/check",
        "http",
        "ConfigBridge.check_saved_provider",
        _BROWSER_WRITE_AUTH,
        "browser-session-and-CSRF-token",
        "intentionally-change",
        _ADR_0012,
        {},
        {"check": "ProviderCheck"},
    ),
    Route(
        "http.route.delete.api.providers.id",
        "DELETE",
        "/api/providers/{id}",
        "http",
        "ConfigBridge.delete_provider",
        _BROWSER_WRITE_AUTH,
        "browser-session-and-CSRF-token",
        "intentionally-change",
        _ADR_0012,
        None,
        {"deleted": "string", "error": "string"},
    ),
    *(
        Route(
            f"http.route.get.api.settings.{name}",
            "GET",
            f"/api/settings/{name}",
            "http",
            f"ConfigBridge.{handler}",
            _BROWSER_READ_AUTH,
            "browser-session",
            "intentionally-change",
            _ADR_0012,
            None,
            response,
        )
        for name, handler, response in (
            (
                "dream",
                "dream_settings",
                {
                    "dream": "DreamSettings",
                    "model_cache": "ModelCache",
                    "providers": "ProviderProfile[]",
                },
            ),
            ("ingest", "ingest_settings", {"ingest": "IngestSettings"}),
            ("release", "release_settings", {"release": "ReleaseSettings"}),
        )
    ),
    *(
        Route(
            f"http.route.post.api.settings.{name}",
            "POST",
            f"/api/settings/{name}",
            "http",
            f"ConfigBridge.{handler}",
            _BROWSER_WRITE_AUTH,
            "browser-session-and-CSRF-token",
            "intentionally-change",
            _ADR_0012,
            request,
            response,
        )
        for name, handler, request, response in (
            (
                "dream",
                "save_dream_settings",
                {"dream": "DreamSettings"},
                {"dream": "DreamSettings"},
            ),
            (
                "ingest",
                "save_ingest_settings",
                {"ingest": "IngestSettings"},
                {"ingest": "IngestSettings"},
            ),
            (
                "release",
                "save_release_settings",
                {"release": "ReleaseSettings"},
                {"release": "ReleaseSettings"},
            ),
        )
    ),
    Route(
        "http.route.get.api.admin.dashboard",
        "GET",
        "/api/admin/dashboard",
        "http",
        "AdminBridge.dashboard",
        _BROWSER_READ_AUTH,
        "browser-session",
        "intentionally-change",
        _ADR_0012,
        None,
        "AdminDashboard",
    ),
    Route(
        "http.route.get.api.admin.snapshot",
        "GET",
        "/api/admin/snapshot",
        "http",
        "AdminBridge.snapshot",
        _BROWSER_READ_AUTH,
        "browser-session",
        "intentionally-change",
        _ADR_0012,
        None,
        "AdminSnapshot",
        ("selected_id", "view"),
    ),
    Route(
        "http.route.post.api.admin.actions.action",
        "POST",
        "/api/admin/actions/{action}",
        "http",
        "AdminBridge allowlisted action",
        _BROWSER_WRITE_AUTH,
        "browser-session-and-CSRF-token",
        "intentionally-change",
        _ADR_0012,
        {"id": "string | number", "confirmed": "boolean?"},
        "AdminActionResult",
    ),
    Route(
        "websocket.route.get.ws.admin",
        "GET",
        "/ws/admin",
        "websocket",
        "_handle_admin_websocket",
        _BROWSER_READ_AUTH,
        "browser-session-with-credential-rotation-close",
        "intentionally-change",
        _ADR_0012,
        "WebSocket upgrade",
        "admin event frames",
    ),
)


def snapshot_http(repo_root: Path) -> dict[str, object]:
    """Return the reviewed route table and calls extracted from the Svelte client."""
    service_source = (repo_root / "src/hieronymus/service_http.py").read_text(encoding="utf-8")
    python_routes = _python_routes(service_source)
    _assert_python_routes_reviewed(python_routes)
    frontend_source = (repo_root / "frontend/src/web/lib/api.ts").read_text(encoding="utf-8")
    frontend_calls = _frontend_calls(frontend_source)
    consumers: dict[tuple[str, str], list[str]] = {}
    for call in frontend_calls:
        key = (str(call["method"]), str(call["path_template"]))
        consumers.setdefault(key, []).append(str(call["consumer"]))

    routes = []
    for route in REVIEWED_ROUTES:
        record = asdict(route)
        record["query_fields"] = list(route.query_fields)
        record["frontend_consumers"] = sorted(
            consumers.get((route.method, route.path_template), [])
        )
        routes.append(record)
    return {
        "routes": sorted(routes, key=lambda row: (row["path_template"], row["method"])),
        "python_discovered_routes": python_routes,
        "frontend_calls": frontend_calls,
    }


def _python_routes(source: str) -> list[dict[str, str]]:
    tree = ast.parse(source)
    string_maps: dict[str, tuple[str, ...]] = {}
    for node in tree.body:
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            continue
        target = node.targets[0]
        if not isinstance(target, ast.Name) or not isinstance(node.value, ast.Dict):
            continue
        keys = tuple(key.value for key in node.value.keys if isinstance(key, ast.Constant))
        if keys and all(isinstance(key, str) for key in keys):
            string_maps[target.id] = keys

    discovered: set[tuple[str, str, str]] = set()
    for node in ast.walk(tree):
        if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) or node.name not in {
            "do_GET",
            "do_POST",
            "do_DELETE",
        }:
            continue
        method = node.name.removeprefix("do_")
        for candidate in ast.walk(node):
            if isinstance(candidate, ast.Compare):
                _record_path_comparison(discovered, method, candidate, string_maps)
            elif isinstance(candidate, ast.Call):
                _record_path_prefix(discovered, method, candidate)
    return [
        {"method": method, "match": match, "value": value}
        for method, match, value in sorted(discovered)
    ]


def _record_path_comparison(
    discovered: set[tuple[str, str, str]],
    method: str,
    node: ast.Compare,
    string_maps: dict[str, tuple[str, ...]],
) -> None:
    if len(node.ops) != 1 or len(node.comparators) != 1:
        return
    if not isinstance(node.left, ast.Name) or node.left.id != "path":
        return
    comparator = node.comparators[0]
    if isinstance(node.ops[0], ast.Eq) and isinstance(comparator, ast.Constant):
        if isinstance(comparator.value, str) and comparator.value.startswith("/"):
            discovered.add((method, "exact", comparator.value))
    elif isinstance(node.ops[0], ast.In) and isinstance(comparator, ast.Name):
        for path in string_maps.get(comparator.id, ()):
            discovered.add((method, "exact", path))


def _record_path_prefix(discovered: set[tuple[str, str, str]], method: str, node: ast.Call) -> None:
    if not isinstance(node.func, ast.Attribute) or node.func.attr != "startswith":
        return
    if not isinstance(node.func.value, ast.Name) or node.func.value.id != "path" or not node.args:
        return
    argument = node.args[0]
    values = argument.elts if isinstance(argument, ast.Tuple) else (argument,)
    for value in values:
        if isinstance(value, ast.Constant) and isinstance(value.value, str):
            if value.value.startswith("/"):
                discovered.add((method, "prefix", value.value))


def _assert_python_routes_reviewed(discovered: list[dict[str, str]]) -> None:
    reviewed = {(route.method, route.path_template) for route in REVIEWED_ROUTES}
    for route in discovered:
        method = route["method"]
        value = route["value"]
        match = route["match"]
        covered = (method, value) in reviewed
        if match == "prefix":
            covered = any(
                reviewed_method == method and reviewed_path.startswith(value)
                for reviewed_method, reviewed_path in reviewed
            )
        if not covered:
            raise ValueError(f"unreviewed Python HTTP route: {method} {match} {value}")


def _frontend_calls(source: str) -> list[dict[str, object]]:
    functions = [
        (match.start(), match.group(1))
        for match in re.finditer(r"export\s+async\s+function\s+(\w+)", source)
    ]
    query_fields = sorted(set(re.findall(r'query\.set\("([^"]+)"', source)) | {"view"})
    calls: list[dict[str, object]] = []
    for start, arguments in _request_arguments(source):
        _, consumer = max(
            ((position, name) for position, name in functions if position < start),
            default=(-1, "request"),
        )
        parts = _split_top_level(arguments)
        path_template = _normalize_frontend_path(parts[0])
        method_match = re.search(r'method\s*:\s*"([A-Z]+)"', arguments)
        record: dict[str, object] = {
            "consumer": consumer,
            "method": method_match.group(1) if method_match else "GET",
            "path_template": path_template,
        }
        if "${query}" in parts[0]:
            record["query_fields"] = query_fields
        calls.append(record)
    return sorted(calls, key=lambda row: (row["consumer"], row["method"]))


def _request_arguments(source: str) -> list[tuple[int, str]]:
    calls: list[tuple[int, str]] = []
    for match in re.finditer(r"\brequest\b", source):
        if re.search(r"function\s+$", source[max(0, match.start() - 20) : match.start()]):
            continue
        cursor = match.end()
        while cursor < len(source) and source[cursor].isspace():
            cursor += 1
        if cursor < len(source) and source[cursor] == "<":
            cursor = _balanced_end(source, cursor, "<", ">")
            while cursor < len(source) and source[cursor].isspace():
                cursor += 1
        if cursor >= len(source) or source[cursor] != "(":
            continue
        end = _balanced_end(source, cursor, "(", ")")
        calls.append((match.start(), source[cursor + 1 : end - 1]))
    return calls


def _balanced_end(source: str, start: int, opening: str, closing: str) -> int:
    depth = 0
    quote = ""
    escaped = False
    for cursor in range(start, len(source)):
        character = source[cursor]
        if quote:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == quote:
                quote = ""
            continue
        if character in {'"', "'", "`"}:
            quote = character
        elif character == opening:
            depth += 1
        elif character == closing:
            depth -= 1
            if depth == 0:
                return cursor + 1
    raise ValueError(f"unbalanced {opening}{closing} expression")


def _split_top_level(arguments: str) -> list[str]:
    parts: list[str] = []
    start = 0
    depths = {"(": 0, "[": 0, "{": 0}
    closings = {")": "(", "]": "[", "}": "{"}
    quote = ""
    escaped = False
    for cursor, character in enumerate(arguments):
        if quote:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == quote:
                quote = ""
            continue
        if character in {'"', "'", "`"}:
            quote = character
        elif character in depths:
            depths[character] += 1
        elif character in closings:
            depths[closings[character]] -= 1
        elif character == "," and not any(depths.values()):
            parts.append(arguments[start:cursor].strip())
            start = cursor + 1
    parts.append(arguments[start:].strip())
    return parts


def _normalize_frontend_path(expression: str) -> str:
    value = expression.strip().strip("\"'`")

    def placeholder(match: re.Match[str]) -> str:
        interpolation = match.group(1)
        if "action" in interpolation:
            return "{action}"
        if interpolation.strip() == "query":
            return ""
        return "{id}"

    value = re.sub(r"\$\{([^}]+)\}", placeholder, value)
    if value.startswith("/api/admin/actions/"):
        value = "/api/admin/actions/{action}"
    return value.removesuffix("?")


def _write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def _route_cases(snapshot: dict[str, object]) -> dict[str, object]:
    routes = snapshot["routes"]
    assert isinstance(routes, list)
    return {
        "routes": [
            {
                "contract_id": route["contract_id"],
                "success": _fixture_outcome(route, success=True),
                "failure": _fixture_outcome(route, success=False),
            }
            for route in routes
        ]
    }


def _fixture_outcome(route: dict[str, object], *, success: bool) -> dict[str, object]:
    method = str(route["method"])
    path = _concrete_path(str(route["path_template"]))
    request_headers = _fixture_request_headers(route) if success else {}
    response_status = _success_status(route) if success else _failure_status(route)
    return {
        "request": {
            "method": method,
            "path": path,
            "query": _fixture_query(route) if success else {},
            "headers": request_headers,
            "body": _fixture_request_body(route) if success else _failure_request_body(route),
        },
        "response": {
            "status": response_status,
            "headers": _fixture_response_headers(route, success=success),
            "body": _fixture_success_body(route) if success else _fixture_failure_body(route),
        },
        "normalized_log_fields": {
            "method": method,
            "path_template": route["path_template"],
            "status": response_status,
            "credential": "<redacted>" if request_headers else "<absent>",
        },
    }


def _concrete_path(path_template: str) -> str:
    return (
        path_template.replace("{id}", "synthetic-provider")
        .replace("{action}", "reinforce_crystal")
        .replace("{operation}", "status")
        .replace("{path}", "fixture")
    )


def _fixture_request_headers(route: dict[str, object]) -> dict[str, str]:
    contract_id = str(route["contract_id"])
    if str(route["current_auth"]) == "public-static":
        return {}
    if contract_id == "websocket.route.get.ws.admin":
        return {
            "Cookie": "hieronymus_token=compat-secret-do-not-log",
            "Origin": "http://127.0.0.1:<PORT>",
            "Sec-WebSocket-Key": "Zml4dHVyZS13ZWJzb2NrZXQta2V5",
            "Upgrade": "websocket",
        }
    if contract_id == "http.route.post.mcp":
        return {"Authorization": "Bearer compat-secret-do-not-log"}
    return {"X-Hieronymus-Token": "compat-secret-do-not-log"}


def _fixture_query(route: dict[str, object]) -> dict[str, str]:
    return {
        field: "Crystals" if field == "view" else "1" for field in route.get("query_fields", [])
    }


def _fixture_request_body(route: dict[str, object]) -> object:
    contract_id = str(route["contract_id"])
    if contract_id == "http.route.post.mcp":
        return {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        }
    if contract_id == "http.route.post.api.providers":
        return {
            "provider": {
                "id": "synthetic-provider",
                "name": "Synthetic Provider",
                "type": "openai",
                "url": "https://provider.invalid/v1",
                "key": "compat-secret-do-not-log",
                "timeout_seconds": "30",
            }
        }
    if contract_id == "http.route.post.api.settings.dream":
        return {"dream": {"dreaming": {"enabled": False}, "workflows": {}}}
    if contract_id == "http.route.post.api.settings.ingest":
        return {"ingest": _ingest_fixture()}
    if contract_id == "http.route.post.api.settings.release":
        return {"release": {"update_channel": "stable"}}
    if contract_id == "http.route.post.api.admin.actions.action":
        return {"id": 1, "confirmed": True}
    request_shape = route["request_shape"]
    return {} if request_shape == {} else None


def _failure_request_body(route: dict[str, object]) -> object:
    if str(route["contract_id"]) == "http.route.post.mcp":
        return {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}
    return _fixture_request_body(route)


def _success_status(route: dict[str, object]) -> int:
    return 101 if str(route["surface"]) == "websocket" else 200


def _failure_status(route: dict[str, object]) -> int:
    contract_id = str(route["contract_id"])
    if str(route["current_auth"]) == "public-static":
        return 404
    if contract_id == "http.route.post.mcp":
        return 400
    if str(route["surface"]) == "websocket" or "browser" in str(route["current_auth"]):
        return 403
    return 401


def _fixture_response_headers(route: dict[str, object], *, success: bool) -> dict[str, str]:
    if success and str(route["surface"]) == "websocket":
        return {"Connection": "Upgrade", "Upgrade": "websocket"}
    response_shape = str(route["response_shape"])
    if response_shape == "text/html":
        return {"Content-Type": "text/html; charset=utf-8"}
    if response_shape == "bytes with detected Content-Type":
        return {"Content-Type": "application/javascript"}
    return {"Content-Type": "application/json; charset=utf-8"}


def _fixture_success_body(route: dict[str, object]) -> object:
    contract_id = str(route["contract_id"])
    if contract_id.startswith("frontend.route.get."):
        return (
            "console.log('Hieronymus fixture');"
            if contract_id == "frontend.route.get.assets.path"
            else "<!doctype html><title>Hieronymus Web Console</title>"
        )
    if contract_id == "http.route.get.health":
        return {"ok": True, "service": "hieronymus", "version": "<VERSION>"}
    if contract_id == "http.route.get.status":
        return {
            "running": True,
            "pid": "<PID>",
            "host": "127.0.0.1",
            "port": "<PORT>",
            "version": "<VERSION>",
            "data_root": "<DATA_ROOT>",
        }
    if contract_id == "http.route.post.shutdown":
        return {"ok": True, "stopping": True}
    if contract_id == "http.route.post.mcp":
        return {"jsonrpc": "2.0", "id": 1, "result": {"tools": []}}
    if contract_id == "http.route.post.api.mcp.operation":
        return {"result": {"service": {"available": True, "mode": "local-http"}}}
    if contract_id == "http.route.get.api.providers":
        return {"providers": [], "error": ""}
    if contract_id in {
        "http.route.post.api.providers",
        "http.route.get.api.providers.id",
    }:
        return {"provider": _provider_fixture(), "error": ""}
    if contract_id == "http.route.get.api.providers.id.models":
        return {"models": ["synthetic-model"], "source": "fixture", "error": ""}
    if contract_id == "http.route.post.api.providers.id.check":
        return {
            "check": {
                "ok": True,
                "models": ["synthetic-model"],
                "source": "fixture",
                "error": "",
            },
            "error": "",
        }
    if contract_id == "http.route.delete.api.providers.id":
        return {"deleted": "synthetic-provider", "error": ""}
    if contract_id == "http.route.get.api.settings.dream":
        return {
            "dream": {"dreaming": {}, "workflows": {}},
            "providers": [],
            "model_cache": {"providers": {}},
            "error": "",
        }
    if contract_id == "http.route.post.api.settings.dream":
        return {"dream": {"dreaming": {"enabled": False}, "workflows": {}}, "error": ""}
    if contract_id in {
        "http.route.get.api.settings.ingest",
        "http.route.post.api.settings.ingest",
    }:
        return {"ingest": _ingest_fixture(), "error": ""}
    if contract_id in {
        "http.route.get.api.settings.release",
        "http.route.post.api.settings.release",
    }:
        return {"release": {"update_channel": "stable"}, "error": ""}
    if contract_id == "http.route.get.api.admin.dashboard":
        return {"default_view": "Crystals", "views": ["Crystals"], "stats": {}}
    if contract_id == "http.route.get.api.admin.snapshot":
        return {"snapshot": _admin_snapshot_fixture()}
    if contract_id == "http.route.post.api.admin.actions.action":
        return {
            "result": {"message": "Synthetic action completed"},
            "snapshot": _admin_snapshot_fixture(),
        }
    if contract_id == "websocket.route.get.ws.admin":
        return None
    raise ValueError(f"missing success fixture body: {contract_id}")


def _provider_fixture() -> dict[str, object]:
    return {
        "id": "synthetic-provider",
        "name": "Synthetic Provider",
        "type": "openai",
        "url": "https://provider.invalid/v1",
        "key_configured": True,
        "model": "",
        "timeout_seconds": 30.0,
    }


def _ingest_fixture() -> dict[str, object]:
    return {
        "short_memory": {
            "warning_sentence_count": 6,
            "rejection_sentence_count": 30,
            "warning_symbol_count": 1200,
            "rejection_symbol_count": 5000,
        },
        "learn": {"max_block_chars": 1200},
    }


def _admin_snapshot_fixture() -> dict[str, object]:
    return {
        "view": "Crystals",
        "rows": [],
        "selected": None,
        "detail": {"title": "", "subtitle": "", "body": "", "fields": []},
    }


def _fixture_failure_body(route: dict[str, object]) -> object:
    contract_id = str(route["contract_id"])
    if str(route["current_auth"]) == "public-static":
        return {"error": "not_found"}
    if contract_id == "http.route.post.mcp":
        return {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -32600, "message": "unsupported MCP protocol version"},
        }
    if str(route["surface"]) == "websocket" or "browser" in str(route["current_auth"]):
        return {"error": "forbidden"}
    return {"error": "unauthorized"}


def _merge_manifest(repo_root: Path, snapshot: dict[str, object]) -> None:
    manifest_path = repo_root / "compatibility/manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    contracts = manifest["contracts"]
    if not isinstance(contracts, list):
        raise ValueError("manifest contracts must be an array")
    task_four_prefixes = ("http.route.", "websocket.route.", "frontend.route.")
    preserved = [
        contract
        for contract in contracts
        if not (
            isinstance(contract, dict)
            and str(contract.get("id", "")).startswith(task_four_prefixes)
        )
    ]
    routes = snapshot["routes"]
    assert isinstance(routes, list)
    generated = [_manifest_contract(route) for route in routes]
    manifest["contracts"] = sorted(
        [*preserved, *generated], key=lambda contract: str(contract["id"])
    )
    _write_json(manifest_path, manifest)


def _manifest_contract(route: dict[str, object]) -> dict[str, object]:
    contract_id = str(route["contract_id"])
    rust_name = contract_id.replace(".", "_").replace("-", "_")
    surface = str(route["surface"])
    target_file = "websocket_contract.rs" if surface == "websocket" else "http_contract.rs"
    tests = ["tests/compatibility/test_http_inventory.py", "tests/test_service_http.py"]
    if surface == "frontend":
        tests.append("frontend/src/web/app.test.ts")
    contract: dict[str, object] = {
        "id": contract_id,
        "surface": surface,
        "acceptance_owner": "Pavel Obruchnikov <me@inkyquill.net>",
        "technical_owner": "daemon-mcp-security",
        "python_entry_point": f"hieronymus.service_http:{route['handler']}",
        "tests": tests,
        "fixture": "compatibility/fixtures/http/route-cases.json",
        "rust_test_target": f"crates/hiero-daemon/tests/{target_file}::{rust_name}",
        "disposition": route["disposition"],
    }
    if route["adr"] is not None:
        contract["adr"] = route["adr"]
    return contract


def write_snapshot(repo_root: Path) -> dict[str, object]:
    """Write the HTTP snapshot, route cases, and Task 4 manifest records."""
    snapshot = snapshot_http(repo_root)
    _write_json(repo_root / "compatibility/snapshots/http.json", snapshot)
    _write_json(
        repo_root / "compatibility/fixtures/http/route-cases.json",
        _route_cases(snapshot),
    )
    _merge_manifest(repo_root, snapshot)
    return snapshot


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="write compatibility files")
    args = parser.parse_args()
    repo_root = Path(__file__).resolve().parents[2]
    snapshot = write_snapshot(repo_root) if args.write else snapshot_http(repo_root)
    if not args.write:
        print(json.dumps(snapshot, ensure_ascii=False, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
