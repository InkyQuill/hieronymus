from __future__ import annotations

import json
from pathlib import Path

import pytest

from tools.compatibility.inventory_http import runtime_reference_bodies, snapshot_http
from tools.compatibility.model import load_manifest

ROOT = Path(__file__).resolve().parents[2]
SENTINEL = "compat-secret-do-not-log"
RUNTIME_BACKED_ROUTES = {
    "http.route.get.status": "status",
    "http.route.post.api.mcp.operation": "private_mcp_status",
    "http.route.get.api.providers": "provider_list",
    "http.route.post.api.providers": "provider_save",
    "http.route.get.api.providers.id": "provider_detail",
    "http.route.get.api.providers.id.models": "provider_models",
    "http.route.post.api.providers.id.check": "provider_check",
    "http.route.delete.api.providers.id": "provider_delete",
    "http.route.get.api.settings.dream": "dream_get",
    "http.route.post.api.settings.dream": "dream_post",
    "http.route.get.api.settings.ingest": "ingest_get",
    "http.route.post.api.settings.ingest": "ingest_post",
    "http.route.get.api.settings.release": "release_get",
    "http.route.post.api.settings.release": "release_post",
    "http.route.get.api.admin.dashboard": "admin_dashboard",
    "http.route.get.api.admin.snapshot": "admin_snapshot",
    "http.route.post.api.admin.actions.action": "admin_action",
}


def _route_cases_by_id() -> dict[str, dict[str, object]]:
    fixture = json.loads(
        (ROOT / "compatibility/fixtures/http/route-cases.json").read_text(encoding="utf-8")
    )
    return {case["contract_id"]: case for case in fixture["routes"]}


def _routes_by_id() -> dict[str, dict[str, object]]:
    return {route["contract_id"]: route for route in snapshot_http(ROOT)["routes"]}


def test_http_snapshot_matches_router_and_frontend_sources() -> None:
    expected = json.loads((ROOT / "compatibility/snapshots/http.json").read_text(encoding="utf-8"))

    assert snapshot_http(ROOT) == expected


def test_every_frontend_request_has_a_route_contract() -> None:
    snapshot = snapshot_http(ROOT)
    route_keys = {(row["method"], row["path_template"]) for row in snapshot["routes"]}

    for call in snapshot["frontend_calls"]:
        assert (call["method"], call["path_template"]) in route_keys


def test_frontend_consumers_keep_their_actual_methods_and_templates() -> None:
    calls = snapshot_http(ROOT)["frontend_calls"]

    assert [(call["consumer"], call["method"], call["path_template"]) for call in calls] == [
        ("checkProvider", "POST", "/api/providers/{id}/check"),
        ("deleteProvider", "DELETE", "/api/providers/{id}"),
        ("listProviders", "GET", "/api/providers"),
        ("loadAdminDashboard", "GET", "/api/admin/dashboard"),
        ("loadAdminSnapshot", "GET", "/api/admin/snapshot"),
        ("loadDreamSettings", "GET", "/api/settings/dream"),
        ("loadIngestSettings", "GET", "/api/settings/ingest"),
        ("loadReleaseSettings", "GET", "/api/settings/release"),
        ("refreshModels", "GET", "/api/providers/{id}/models"),
        ("runAdminAction", "POST", "/api/admin/actions/{action}"),
        ("saveDreamSettings", "POST", "/api/settings/dream"),
        ("saveIngestSettings", "POST", "/api/settings/ingest"),
        ("saveProvider", "POST", "/api/providers"),
        ("saveReleaseSettings", "POST", "/api/settings/release"),
        (
            "startAdminDreaming",
            "POST",
            "/api/admin/actions/run_manual_dreaming",
        ),
    ]
    snapshot_call = next(call for call in calls if call["consumer"] == "loadAdminSnapshot")
    assert snapshot_call["query_fields"] == ["selected_id", "view"]


def test_manual_dreaming_has_a_distinct_frontend_and_route_shape() -> None:
    routes = _routes_by_id()
    manual = routes["http.route.post.api.admin.actions.run_manual_dreaming"]
    generic = routes["http.route.post.api.admin.actions.action"]

    assert manual["path_template"] == "/api/admin/actions/run_manual_dreaming"
    assert manual["frontend_consumers"] == ["startAdminDreaming"]
    assert manual["request_shape"] == {}
    assert manual["response_shape"] == {"started": "boolean", "status": "string"}
    assert generic["frontend_consumers"] == ["runAdminAction"]
    assert generic["request_shape"] == {
        "id": "string | number",
        "confirmed": "boolean?",
    }
    assert generic["response_shape"] == "AdminActionResult"


def test_required_route_families_are_explicit() -> None:
    paths = {row["path_template"] for row in snapshot_http(ROOT)["routes"]}

    assert {"/health", "/status", "/shutdown", "/mcp", "/ws/admin"} <= paths
    assert "/api/mcp/{operation}" in paths


def test_unreviewed_python_router_branch_is_rejected(tmp_path: Path) -> None:
    service_source = (ROOT / "src/hieronymus/service_http.py").read_text(encoding="utf-8")
    service_source = service_source.replace(
        '        if path == "/status":\n',
        '        if path == "/compat-unreviewed":\n'
        '            self._send_json({"ok": True})\n'
        "            return\n"
        '        if path == "/status":\n',
        1,
    )
    service_path = tmp_path / "src/hieronymus/service_http.py"
    service_path.parent.mkdir(parents=True)
    service_path.write_text(service_source, encoding="utf-8")
    frontend_path = tmp_path / "frontend/src/web/lib/api.ts"
    frontend_path.parent.mkdir(parents=True)
    frontend_path.write_text(
        (ROOT / "frontend/src/web/lib/api.ts").read_text(encoding="utf-8"),
        encoding="utf-8",
    )

    with pytest.raises(
        ValueError,
        match=r"unreviewed Python HTTP route: GET exact /compat-unreviewed",
    ):
        snapshot_http(tmp_path)


def test_preserved_route_missing_from_current_router_is_rejected(tmp_path: Path) -> None:
    service_source = (ROOT / "src/hieronymus/service_http.py").read_text(encoding="utf-8")
    service_source = service_source.replace(
        'return path in {"/config", "/admin"}',
        'return path in {"/config"}',
        1,
    ).replace(
        'path.startswith(("/config/", "/admin/"))',
        'path.startswith(("/config/",))',
        1,
    )
    service_path = tmp_path / "src/hieronymus/service_http.py"
    service_path.parent.mkdir(parents=True)
    service_path.write_text(service_source, encoding="utf-8")
    frontend_path = tmp_path / "frontend/src/web/lib/api.ts"
    frontend_path.parent.mkdir(parents=True)
    frontend_path.write_text(
        (ROOT / "frontend/src/web/lib/api.ts").read_text(encoding="utf-8"),
        encoding="utf-8",
    )

    with pytest.raises(
        ValueError,
        match=r"preserved reviewed route absent from current Python router: GET /admin",
    ):
        snapshot_http(tmp_path)


def test_route_cases_cover_every_route_and_do_not_disclose_credentials() -> None:
    snapshot = snapshot_http(ROOT)
    fixture = json.loads(
        (ROOT / "compatibility/fixtures/http/route-cases.json").read_text(encoding="utf-8")
    )
    cases = fixture["routes"]

    assert {case["contract_id"] for case in cases} == {
        route["contract_id"] for route in snapshot["routes"]
    }
    for case in cases:
        expected_case_fields = {"contract_id", "failure", "success"}
        route = next(
            route for route in snapshot["routes"] if route["contract_id"] == case["contract_id"]
        )
        if route["disposition"] in {"intentionally-change", "remove"}:
            expected_case_fields |= {"current", "target"}
        if case["contract_id"] == "websocket.route.get.ws.admin":
            expected_case_fields.add("websocket_contract")
        assert set(case) == expected_case_fields
        for outcome_name in ("success", "failure"):
            outcome = case[outcome_name]
            assert set(outcome) == {"normalized_log_fields", "request", "response"}
            assert {"method", "path"} <= set(outcome["request"])
            assert {"body", "headers", "status"} <= set(outcome["response"])
            non_request_fields = json.dumps(
                {
                    "response": outcome["response"],
                    "normalized_log_fields": outcome["normalized_log_fields"],
                },
                sort_keys=True,
            )
            assert SENTINEL not in non_request_fields
        if route["disposition"] == "intentionally-change":
            target_successes = (
                case["target"]["successes"]
                if case["contract_id"] == "http.route.post.mcp"
                else [case["target"]["success"]]
            )
            for outcome in [
                case["current"]["success"],
                *case["current"]["failures"],
                *target_successes,
                *case["target"]["failures"],
            ]:
                non_request_fields = json.dumps(
                    {
                        "response": outcome["response"],
                        "normalized_log_fields": outcome["normalized_log_fields"],
                    },
                    sort_keys=True,
                )
                assert SENTINEL not in non_request_fields

    credential_values = {
        value.removeprefix("Bearer ")
        for case in cases
        for outcome_name in ("success", "failure")
        for name, value in case[outcome_name]["request"].get("headers", {}).items()
        if name in {"Authorization", "X-Hieronymus-Token"}
    }
    assert credential_values == {SENTINEL}


def test_http_mcp_cases_cover_official_metadata_and_local_security() -> None:
    case = _route_cases_by_id()["http.route.post.mcp"]["target"]
    successes = {item["id"]: item["request"] for item in case["successes"]}
    tools_list = successes["tools-list"]
    assert tools_list["headers"]["Mcp-Method"] == "tools/list"
    assert "Mcp-Name" not in tools_list["headers"]
    assert tools_list["body"]["method"] == "tools/list"
    tools_call = successes["tools-call"]
    assert tools_call["headers"]["Mcp-Method"] == "tools/call"
    assert tools_call["headers"]["Mcp-Name"] == "hieronymus_status"
    assert tools_call["body"]["method"] == "tools/call"
    assert tools_call["body"]["params"]["name"] == "hieronymus_status"
    meta = tools_call["body"]["params"]["_meta"]
    assert meta["io.modelcontextprotocol/protocolVersion"] == "2026-07-28"
    assert meta["io.modelcontextprotocol/clientCapabilities"] == {}
    assert meta["io.modelcontextprotocol/clientInfo"] == {
        "name": "compatibility-replay",
        "version": "1.0.0",
    }
    assert {failure["id"] for failure in case["failures"]} == {
        "invalid-host",
        "missing-bearer",
        "invalid-bearer",
        "missing-version",
        "unsupported-version",
        "missing-mcp-method",
        "wrong-mcp-method",
        "unexpected-mcp-name-tools-list",
        "missing-mcp-name-tools-call",
        "wrong-mcp-name-tools-call",
    }


def test_runtime_response_fixtures_keep_complete_status_and_dashboard_shapes() -> None:
    cases = _route_cases_by_id()
    status = cases["http.route.get.status"]["success"]["response"]["body"]
    dashboard = cases["http.route.get.api.admin.dashboard"]["success"]["response"]["body"]
    runtime_bodies = runtime_reference_bodies()

    assert status == runtime_bodies["status"]
    assert dashboard == runtime_bodies["admin_dashboard"]

    assert set(status) == {
        "config_path",
        "data_root",
        "database_path",
        "dreaming",
        "host",
        "housekeeping",
        "mcp_adapter",
        "pid",
        "port",
        "providers",
        "providers_error",
        "running",
        "started_at",
        "version",
    }
    assert set(status["dreaming"]) == {
        "active_cycle",
        "active_provider",
        "cycle_active",
        "enabled",
        "last_error",
        "last_skip_reason",
        "last_skipped_at",
        "last_started_at",
        "max_pending_short_term_memories",
        "max_short_term_memories_per_cycle",
        "min_pending_short_term_memories",
        "not_enough_memories_cycle_threshold",
        "not_enough_memories_skipped_count",
        "pending_completed_sessions",
        "pending_short_term_memories",
        "schedule_interval_minutes",
        "skipped_count",
    }
    assert {
        "header",
        "stats",
        "views",
        "short_term_status",
        "dream_status",
    } <= set(dashboard)
    assert set(dashboard["header"]) == {"logo", "product", "tagline", "version"}
    assert set(dashboard["stats"]) == {
        "audit_events",
        "crystals",
        "dream_runs",
        "lessons",
        "pending_proposals",
        "series",
        "sessions",
        "short_term_memories",
    }


def test_private_mcp_status_and_admin_action_keep_complete_runtime_shapes() -> None:
    cases = _route_cases_by_id()
    private_status = cases["http.route.post.api.mcp.operation"]["success"]["response"]["body"][
        "result"
    ]
    admin_action = cases["http.route.post.api.admin.actions.action"]["success"]["response"]["body"]

    assert set(private_status) == {"service", "data_root", "database_path"}
    assert private_status["service"] == {
        "available": False,
        "mode": "direct-local",
        "reason": "no running local service discovered",
    }
    assert set(admin_action) == {
        "result",
        "stats",
        "snapshot",
        "dream_status",
        "dream_config_error",
        "short_term_status",
    }
    assert admin_action["result"] == {
        "action": "reinforce",
        "entity_id": 1,
        "entity_type": "crystal",
        "message": "Crystal reinforced",
    }
    assert admin_action["stats"]["audit_events"] == 1
    assert admin_action["snapshot"]["selected"]["label"] == "Synthetic Rule"


@pytest.mark.parametrize(
    ("contract_id", "runtime_body_key"),
    sorted(RUNTIME_BACKED_ROUTES.items()),
)
def test_runtime_backed_route_fixture_matches_fresh_current_output(
    contract_id: str,
    runtime_body_key: str,
) -> None:
    route = _routes_by_id()[contract_id]
    fixture_body = _route_cases_by_id()[contract_id]["success"]["response"]["body"]
    runtime_bodies = runtime_reference_bodies()

    assert route["runtime_body_key"] == runtime_body_key
    assert fixture_body == runtime_bodies[runtime_body_key]


def test_root_route_records_current_404_and_adr_0014_target_shell() -> None:
    route = _routes_by_id()["frontend.route.get.root"]
    case = _route_cases_by_id()["frontend.route.get.root"]

    assert route["disposition"] == "intentionally-change"
    assert route["adr"] == "docs/adr/0014-web-console-replaces-terminal-ui.md"
    assert case["current"]["success"]["response"] == {
        "status": 404,
        "headers": {"Content-Type": "application/json; charset=utf-8"},
        "body": {"error": "not_found", "path": "/"},
    }
    assert case["target"]["success"]["response"] == {
        "status": 200,
        "headers": {"Content-Type": "text/html; charset=utf-8"},
        "body": "<!doctype html><title>Hieronymus Web Console</title>",
    }


def test_static_runtime_failures_are_json_not_success_content_types() -> None:
    cases = _route_cases_by_id()

    for contract_id in (
        "frontend.route.get.admin",
        "frontend.route.get.admin.path",
        "frontend.route.get.assets.path",
        "frontend.route.get.config",
        "frontend.route.get.config.path",
    ):
        assert cases[contract_id]["failure"]["response"]["headers"] == {
            "Content-Type": "application/json; charset=utf-8"
        }


def test_websocket_fixture_freezes_events_resume_fallback_and_rotation() -> None:
    route = _routes_by_id()["websocket.route.get.ws.admin"]
    websocket = _route_cases_by_id()["websocket.route.get.ws.admin"]["websocket_contract"]

    assert route["disposition"] == "intentionally-change"
    assert route["adr"] == "docs/adr/0012-mcp-transport-authentication-and-discovery.md"
    assert websocket["current"] == {
        "event": {
            "type": "dream_phase_progress",
            "timestamp": "<TIMESTAMP>",
            "payload": {"cycle_id": 7, "phase": "crystallization", "run_id": 11},
        },
        "resume": {"supported": False},
        "snapshot_refresh_fallback": {"supported": False},
    }
    assert websocket["target"]["event"] == {
        "version": 1,
        "event_id": 42,
        "event_type": "dream_phase_progress",
        "payload": {"cycle_id": 7, "phase": "crystallization", "run_id": 11},
    }
    assert websocket["target"]["resume"] == {
        "last_event_id": 41,
        "replayed_event_ids": [42],
    }
    assert websocket["target"]["snapshot_refresh_fallback"]["request"] == {
        "method": "GET",
        "path": "/api/admin/snapshot?view=Crystals&selected_id=1",
    }
    assert websocket["target"]["credentials_rotated"] == {
        "error": {
            "code": "credentials_rotated",
            "message": "local service credentials rotated",
        },
        "close": {"code": 4001, "reason": "credentials_rotated"},
    }


def test_route_manifest_contracts_have_uniform_ownership_and_adr_rulings() -> None:
    snapshot = snapshot_http(ROOT)
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    route_ids = {str(route["contract_id"]) for route in snapshot["routes"]}
    route_contracts = {
        contract.id: contract for contract in manifest.contracts if contract.id in route_ids
    }

    assert set(route_contracts) == route_ids
    for route in snapshot["routes"]:
        contract = route_contracts[str(route["contract_id"])]
        assert contract.surface == route["surface"]
        assert contract.acceptance_owner == "Pavel Obruchnikov <me@inkyquill.net>"
        assert contract.technical_owner == "daemon-mcp-security"
        assert contract.disposition == route["disposition"]
        assert contract.adr == route["adr"]
        assert contract.fixture == "compatibility/fixtures/http/route-cases.json"
        assert "tests/compatibility/test_http_inventory.py" in contract.tests

    private_bridge = route_contracts["http.route.post.api.mcp.operation"]
    assert private_bridge.disposition == "remove"
    assert private_bridge.adr == "docs/adr/0015-mcp-protocol-and-transport.md"
    assert sum(contract.id == private_bridge.id for contract in manifest.contracts) == 1
    assert route_contracts["http.route.post.mcp"].adr == (
        "docs/adr/0015-mcp-protocol-and-transport.md"
    )


def test_every_changed_route_has_separate_replayable_current_and_target_outcomes() -> None:
    routes = _routes_by_id()
    cases = _route_cases_by_id()

    for contract_id, route in routes.items():
        if route["disposition"] != "intentionally-change":
            continue
        case = cases[contract_id]
        assert case["current"]["basis"] == "current-python-runtime"
        assert case["target"]["basis"] == "adr-backed-target"
        assert case["target"]["adr"] == route["adr"]
        success_field = "successes" if contract_id == "http.route.post.mcp" else "success"
        assert {success_field, "failures"} <= set(case["target"])
        assert case["current"] is not case["target"]


def test_target_health_is_unauthenticated_minimal_liveness() -> None:
    health = _route_cases_by_id()["http.route.get.health"]

    assert health["current"]["success"]["request"]["headers"] == {"X-Hieronymus-Token": SENTINEL}
    assert health["current"]["success"]["response"]["body"] == {
        "ok": True,
        "service": "hieronymus",
        "version": "<VERSION>",
    }
    assert health["target"]["success"]["request"]["headers"] == {"Host": "127.0.0.1:<PORT>"}
    assert health["target"]["success"]["response"]["body"] == {"ok": True}
    assert "service" not in health["target"]["success"]["response"]["body"]
    assert "version" not in health["target"]["success"]["response"]["body"]


def test_launch_grant_exchange_sets_strict_http_only_session_and_csrf() -> None:
    exchange = _route_cases_by_id()["http.route.post.auth.launch-grant.exchange"]
    target = exchange["target"]

    assert target["success"]["request"] == {
        "method": "POST",
        "path": "/auth/launch-grant/exchange",
        "query": {},
        "headers": {
            "Content-Type": "application/json",
            "Host": "127.0.0.1:<PORT>",
            "Origin": "http://127.0.0.1:<PORT>",
        },
        "body": {"launch_grant": "<SINGLE_USE_LAUNCH_GRANT>"},
    }
    assert target["success"]["response"] == {
        "status": 200,
        "headers": {
            "Content-Type": "application/json; charset=utf-8",
            "Set-Cookie": ("hieronymus_session=<SESSION>; Path=/; HttpOnly; SameSite=Strict"),
        },
        "body": {"csrf_token": "<CSRF_TOKEN>"},
    }
    failure_ids = {failure["id"] for failure in target["failures"]}
    assert {"grant-reuse", "invalid-host", "cross-origin"} <= failure_ids


def test_target_browser_auth_covers_host_origin_csrf_success_and_failures() -> None:
    cases = _route_cases_by_id()
    read = cases["http.route.get.api.providers"]["target"]
    write = cases["http.route.post.api.providers"]["target"]

    assert read["success"]["request"]["headers"] == {
        "Cookie": "hieronymus_session=<SESSION>",
        "Host": "127.0.0.1:<PORT>",
        "Origin": "http://127.0.0.1:<PORT>",
    }
    assert write["success"]["request"]["headers"] == {
        "Cookie": "hieronymus_session=<SESSION>",
        "Host": "127.0.0.1:<PORT>",
        "Origin": "http://127.0.0.1:<PORT>",
        "X-CSRF-Token": "<CSRF_TOKEN>",
    }
    assert {failure["id"] for failure in read["failures"]} == {
        "cross-origin",
        "invalid-host",
        "missing-session",
    }
    assert {failure["id"] for failure in write["failures"]} == {
        "cross-origin",
        "invalid-csrf",
        "invalid-host",
        "missing-csrf",
        "missing-session",
    }


def test_target_websocket_uses_cookie_origin_resume_and_rotation_cases() -> None:
    case = _route_cases_by_id()["websocket.route.get.ws.admin"]
    target = case["target"]

    assert target["success"]["request"]["headers"] == {
        "Cookie": "hieronymus_session=<SESSION>",
        "Host": "127.0.0.1:<PORT>",
        "Origin": "http://127.0.0.1:<PORT>",
        "Sec-WebSocket-Key": "Zml4dHVyZS13ZWJzb2NrZXQta2V5",
        "Upgrade": "websocket",
    }
    assert target["success"]["request"]["body"] == {"resume_from_event_id": 41}
    assert target["websocket"]["resume"] == {
        "last_event_id": 41,
        "replayed_event_ids": [42],
    }
    assert target["websocket"]["credentials_rotated"]["close"]["reason"] == ("credentials_rotated")
    assert {failure["id"] for failure in target["failures"]} == {
        "cross-origin",
        "expired-session",
        "invalid-host",
        "missing-session",
    }
