from __future__ import annotations

import json
from pathlib import Path

import pytest

from tools.compatibility.inventory_http import snapshot_http
from tools.compatibility.model import load_manifest

ROOT = Path(__file__).resolve().parents[2]
SENTINEL = "compat-secret-do-not-log"


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
        ("startAdminDreaming", "POST", "/api/admin/actions/{action}"),
    ]
    snapshot_call = next(call for call in calls if call["consumer"] == "loadAdminSnapshot")
    assert snapshot_call["query_fields"] == ["selected_id", "view"]


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


def test_route_cases_cover_every_route_and_do_not_disclose_credentials() -> None:
    snapshot = snapshot_http(ROOT)
    fixture = json.loads(
        (ROOT / "compatibility/fixtures/http/route-cases.json").read_text(encoding="utf-8")
    )
    cases = fixture["routes"]

    assert {case["contract_id"] for case in cases} == {
        route["contract_id"] for route in snapshot["routes"]
    }
    assert all(set(case) == {"contract_id", "failure", "success"} for case in cases)
    for case in cases:
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
