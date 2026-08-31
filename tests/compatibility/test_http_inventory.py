from __future__ import annotations

import json
from pathlib import Path

import pytest

from tools.compatibility.inventory_http import runtime_reference_bodies, snapshot_http
from tools.compatibility.model import load_manifest

ROOT = Path(__file__).resolve().parents[2]
SENTINEL = "compat-secret-do-not-log"
MCP_REVISION = "2026-07-28"


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
        if case["contract_id"] == "frontend.route.get.root":
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

    credential_values = {
        value.removeprefix("Bearer ")
        for case in cases
        for outcome_name in ("success", "failure")
        for name, value in case[outcome_name]["request"].get("headers", {}).items()
        if name in {"Authorization", "X-Hieronymus-Token"}
    }
    assert credential_values == {SENTINEL}


def test_streamable_http_fixture_authenticates_version_negotiation() -> None:
    case = _route_cases_by_id()["http.route.post.mcp"]
    success_headers = case["success"]["request"]["headers"]
    failure_headers = case["failure"]["request"]["headers"]

    assert success_headers == {
        "Authorization": f"Bearer {SENTINEL}",
        "MCP-Protocol-Version": MCP_REVISION,
    }
    assert failure_headers["Authorization"] == f"Bearer {SENTINEL}"
    assert failure_headers["MCP-Protocol-Version"] != MCP_REVISION
    assert case["failure"]["response"] == {
        "status": 400,
        "headers": {"Content-Type": "application/json; charset=utf-8"},
        "body": {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32600,
                "message": "unsupported MCP protocol version",
            },
        },
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


def test_root_route_records_current_404_and_adr_0014_target_shell() -> None:
    route = _routes_by_id()["frontend.route.get.root"]
    case = _route_cases_by_id()["frontend.route.get.root"]

    assert route["disposition"] == "intentionally-change"
    assert route["adr"] == "docs/adr/0014-web-console-replaces-terminal-ui.md"
    assert case["current"]["response"] == {
        "status": 404,
        "headers": {"Content-Type": "application/json; charset=utf-8"},
        "body": {"error": "not_found", "path": "/"},
    }
    assert case["target"]["response"] == {
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
