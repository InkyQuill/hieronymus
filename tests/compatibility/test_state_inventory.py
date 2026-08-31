from __future__ import annotations

import importlib
import json
import sqlite3
from pathlib import Path

import pytest

from tools.compatibility import inventory_state
from tools.compatibility.inventory_state import (
    build_test_ownership,
    collect_test_nodeids,
    generate_state_artifacts,
    snapshot_state,
    sqlite_contract,
)
from tools.compatibility.model import load_manifest

ROOT = Path(__file__).resolve().parents[2]
SELECTED_ROOT_PATHS = {
    "hieronymus.sqlite",
    "provider.conf",
    "dream.conf",
    "ingest.conf",
    "release.conf",
    "llmcache.tmp",
    "backups/",
    "agent-plugins/",
}


def test_state_snapshot_uses_only_explicit_synthetic_root(tmp_path: Path) -> None:
    data_root = tmp_path / "data-root"

    snapshot = snapshot_state(ROOT, data_root)

    assert snapshot["data_root"] == "<DATA_ROOT>"
    assert {item["path"] for item in snapshot["owned_paths"]} == SELECTED_ROOT_PATHS
    assert data_root.exists()


def test_config_inventory_redacts_secret_values_to_configured_flags(tmp_path: Path) -> None:
    snapshot = snapshot_state(ROOT, tmp_path / "data-root")
    provider = snapshot["config"]["provider"]

    assert provider["payload"]["synthetic_openai"]["key"] == {"configured": True}
    assert "synthetic-provider-secret" not in json.dumps(snapshot, sort_keys=True)
    assert provider["fields"] == [
        "defaults.model",
        "defaults.provider",
        "providers.<id>.key.configured",
        "providers.<id>.name",
        "providers.<id>.timeout_seconds",
        "providers.<id>.type",
        "providers.<id>.url",
    ]


def test_database_inventory_includes_schema_objects_and_application_ledgers(
    tmp_path: Path,
) -> None:
    snapshot = snapshot_state(ROOT, tmp_path / "data-root")
    database = snapshot["database"]

    assert database["tables"]
    assert database["columns"]["series"]
    assert len(database["tables"]) == 70
    assert len(database["indexes"]) == 40
    assert len(database["triggers"]) == 9
    assert sum(len(rows) for rows in database["foreign_keys"].values()) == 47
    assert database["triggers"]
    assert database["foreign_keys"]
    assert "memory_graph_migration_ledger" in database["application_migration_ledgers"]
    assert set(database["migration_sources"]) == {"global.sql", "registry.sql", "series.sql"}
    assert {
        "name": "sqlite_autoindex_series_1",
        "table": "series",
        "unique": True,
        "origin": "u",
        "partial": False,
        "columns": [{"seq": 0, "cid": 1, "name": "slug"}],
        "sql": None,
    } in database["indexes"]
    assert database["foreign_keys"]["rag_chunks"][:2] == [
        {
            "id": 0,
            "seq": 0,
            "table": "rag_sources",
            "from": "source_id",
            "to": "id",
            "on_update": "NO ACTION",
            "on_delete": "CASCADE",
            "match": "NONE",
        },
        {
            "id": 0,
            "seq": 1,
            "table": "rag_sources",
            "from": "series_slug",
            "to": "series_slug",
            "on_update": "NO ACTION",
            "on_delete": "CASCADE",
            "match": "NONE",
        },
    ]


def test_sqlite_contract_is_stable_for_dynamic_object_names() -> None:
    connection = sqlite3.connect(":memory:")
    connection.executescript(
        """
        pragma foreign_keys = on;
        create table parent (
          id integer primary key,
          left_key text not null,
          right_key text not null,
          unique(left_key, right_key)
        );
        create table child (
          id integer primary key,
          parent_id integer references parent(id) on delete cascade,
          left_key text,
          right_key text,
          foreign key(left_key, right_key) references parent(left_key, right_key)
        );
        create index child_parent_idx on child(parent_id);
        create trigger child_delete after delete on child begin
          delete from parent where id = old.parent_id;
        end;
        """
    )

    contract = sqlite_contract(connection)

    assert contract["columns"]["child"][1]["name"] == "parent_id"
    assert {index["name"] for index in contract["indexes"]} == {
        "child_parent_idx",
        "sqlite_autoindex_parent_1",
    }
    parent_index = next(
        index for index in contract["indexes"] if index["name"] == "sqlite_autoindex_parent_1"
    )
    assert parent_index["origin"] == "u"
    assert parent_index["columns"] == [
        {"seq": 0, "cid": 1, "name": "left_key"},
        {"seq": 1, "cid": 2, "name": "right_key"},
    ]
    assert contract["triggers"][0]["name"] == "child_delete"
    assert contract["foreign_keys"]["child"][-1] == {
        "id": 1,
        "seq": 0,
        "from": "parent_id",
        "match": "NONE",
        "on_delete": "CASCADE",
        "on_update": "NO ACTION",
        "table": "parent",
        "to": "id",
    }
    assert contract["foreign_keys"]["child"][:2] == [
        {
            "id": 0,
            "seq": 0,
            "from": "left_key",
            "match": "NONE",
            "on_delete": "NO ACTION",
            "on_update": "NO ACTION",
            "table": "parent",
            "to": "left_key",
        },
        {
            "id": 0,
            "seq": 1,
            "from": "right_key",
            "match": "NONE",
            "on_delete": "NO ACTION",
            "on_update": "NO ACTION",
            "table": "parent",
            "to": "right_key",
        },
    ]


def test_collect_test_nodeids_returns_only_sorted_pytest_node_ids() -> None:
    node_ids = collect_test_nodeids(ROOT)

    assert node_ids == sorted(node_ids)
    assert node_ids
    assert all(node_id.startswith("tests/") for node_id in node_ids)
    assert all("::" in node_id for node_id in node_ids)
    assert (
        "tests/compatibility/test_state_inventory.py::"
        "test_collect_test_nodeids_returns_only_sorted_pytest_node_ids" in node_ids
    )


def test_every_python_test_has_manifest_disposition() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    ownership_by_node = {item.node_id: item for item in manifest.test_ownership}
    collected = set(collect_test_nodeids(ROOT))

    assert set(ownership_by_node) == collected
    assert all(
        item.contract_ids if item.disposition == "public_contract" else item.reason
        for item in ownership_by_node.values()
    )


def test_node_ownership_does_not_inherit_every_contract_from_its_file() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    node_ids = [
        "tests/compatibility/test_state_inventory.py::test_collect_test_nodeids_returns_only_sorted_pytest_node_ids",
        "tests/test_agent_plugin_installers.py::test_writable_plugin_reinstall_is_idempotent[codex]",
        "tests/test_release_scripts.py::test_install_script_dev_channel_checks_out_main_and_writes_release_conf",
        "tests/test_provider_config.py::test_save_and_load_provider_catalog_round_trips_profiles_and_defaults",
        "tests/test_admin_cli.py::test_admin_json_reports_available_tui_and_counts",
        "tests/test_cli_service.py::test_docs_describe_local_web_config_and_llm_providers",
        "tests/test_provider_config.py::test_provider_catalog_validates_default_provider_exists",
        "tests/test_provider_config.py::test_load_provider_catalog_rejects_legacy_dream_provider_collision_on_disk",
        "tests/test_agent_plugin_installers.py::test_json_agent_install_rejects_malformed_section_without_traceback",
    ]

    ownership = {
        item["node_id"]: item for item in build_test_ownership(node_ids, manifest.contracts)
    }

    helper = ownership[node_ids[0]]
    assert helper["disposition"] == "implementation_internal"
    assert "pytest collection parser" in helper["reason"]
    assert ownership[node_ids[1]]["contract_ids"] == ["agent-integration.target.codex"]
    assert ownership[node_ids[2]]["contract_ids"] == ["install-update.install-script"]
    assert ownership[node_ids[3]]["contract_ids"] == ["config.provider.roundtrip"]
    assert ownership[node_ids[4]]["contract_ids"] == ["cli.command.hiero.admin"]
    assert ownership[node_ids[5]]["disposition"] == "implementation_internal"
    assert ownership[node_ids[6]]["contract_ids"] == ["config.provider.failures"]
    assert ownership[node_ids[7]]["contract_ids"] == ["config.provider.failures"]
    assert ownership[node_ids[8]]["contract_ids"] == ["agent-integration.target.gemini.failures"]
    with pytest.raises(ValueError, match="no explicit config ownership rule"):
        build_test_ownership(
            ["tests/test_provider_config.py::test_new_unclassified_behavior"],
            manifest.contracts,
        )


def test_http_and_agent_behavior_nodes_have_explicit_public_contract_ownership() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    ownership = {item.node_id: item for item in manifest.test_ownership}

    assert ownership[
        "tests/test_service_http.py::test_health_endpoint_returns_daemon_identity"
    ].contract_ids == ("http.route.get.health",)
    assert ownership[
        "tests/test_service_http.py::test_provider_api_creates_and_lists_custom_profiles"
    ].contract_ids == (
        "http.route.get.api.providers",
        "http.route.post.api.providers",
    )
    assert ownership[
        "tests/test_agent_hooks.py::test_hook_session_end_outputs_json"
    ].contract_ids == ("cli.command.hieronymus-agent-hook.session-end",)
    assert not [
        item
        for item in manifest.test_ownership
        if item.disposition == "implementation_internal"
        and item.node_id.startswith(("tests/test_service_http.py::", "tests/test_agent_hooks.py::"))
    ]


def test_http_and_agent_ownership_rules_do_not_grant_false_file_level_coverage() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    synthetic = build_test_ownership(
        [
            "tests/test_service_http.py::test_python_only_request_helper",
            "tests/test_agent_hooks.py::test_python_only_discovery_helper",
        ],
        manifest.contracts,
    )

    assert [item["disposition"] for item in synthetic] == [
        "implementation_internal",
        "implementation_internal",
    ]


def test_agent_plugin_registry_resolver_and_availability_nodes_are_public_contracts() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    ownership = {item.node_id: item for item in manifest.test_ownership}
    prefix = "tests/test_agent_plugins.py::"
    all_targets = {
        f"agent-integration.target.{target}"
        for target in ("claude", "codex", "openclaw", "opencode", "gemini", "mimo", "pi", "hermes")
    }

    assert (
        set(
            ownership[
                prefix + "test_available_plugins_lists_canonical_targets_in_order"
            ].contract_ids
        )
        == all_targets
    )
    assert ownership[prefix + "test_resolve_plugin_returns_provider"].contract_ids == (
        "agent-integration.target.codex",
    )
    assert ownership[prefix + "test_resolve_plugin_supports_aliases"].contract_ids == (
        "agent-integration.target.mimo",
    )
    assert ownership[
        prefix + "test_codex_availability_detects_assets_and_managed_marker"
    ].contract_ids == ("agent-integration.target.codex",)
    assert ownership[prefix + "test_mimo_availability_detects_mimocode_home"].contract_ids == (
        "agent-integration.target.mimo",
    )
    assert ownership[prefix + "test_claude_availability_checks_all_detect_paths"].contract_ids == (
        "agent-integration.target.claude",
    )
    internal = {
        item.node_id
        for item in manifest.test_ownership
        if item.disposition == "implementation_internal" and item.node_id.startswith(prefix)
    }
    assert internal == {
        prefix + "test_availability_json_paths_are_fresh_lists",
        prefix + "test_invalid_plugin_reports_empty_detect_paths",
        prefix + "test_invalid_plugin_reports_empty_config_paths",
    }


def test_agent_plugin_ownership_does_not_silently_internalize_new_public_behavior() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")

    with pytest.raises(ValueError, match="no explicit agent-plugin ownership rule"):
        build_test_ownership(
            ["tests/test_agent_plugins.py::test_new_public_resolver_behavior"],
            manifest.contracts,
        )


def test_agent_contract_entry_points_are_importable_symbols() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")

    for contract in manifest.contracts:
        if contract.surface != "agent-integration":
            continue
        module_name, symbol_name = contract.python_entry_point.split(":", 1)
        module = importlib.import_module(module_name)
        assert getattr(module, symbol_name)


def test_state_snapshot_covers_config_failures_migrations_and_replayable_integrations(
    tmp_path: Path,
) -> None:
    snapshot = snapshot_state(ROOT, tmp_path / "data-root")

    assert set(snapshot["config"]["provider"]["fixtures"]) == {
        "current",
        "roundtrip",
        "failures",
        "legacy",
    }
    assert set(snapshot["config"]["dream"]["fixtures"]) == {
        "current",
        "roundtrip",
        "failures",
        "legacy",
    }
    assert set(snapshot["config"]["ingest"]["fixtures"]) == {
        "current",
        "roundtrip",
        "failures",
    }
    assert set(snapshot["config"]["release"]["fixtures"]) == {
        "current",
        "roundtrip",
        "failures",
    }
    installer_cases = snapshot["distribution"]["cases"]
    assert {
        "install.stable-success",
        "install.dev-success",
        "install.uv-bootstrap",
        "install.bun-bootstrap",
        "install.bun-upgrade",
        "install.missing-git",
        "install.missing-curl",
        "install.uv-declined",
        "install.bun-declined",
        "install.no-release-tags",
        "install.existing-non-checkout",
        "install.invalid-channel",
        "install.wrong-origin",
        "uninstall.keep-data-idempotent",
        "uninstall.purge-data",
        "uninstall.interactive-remove",
        "uninstall.interactive-keep",
        "uninstall.noninteractive-keep",
        "uninstall.invalid-option",
        "uninstall.unsafe-path",
    } <= {case["id"] for case in installer_cases}
    assert all(case["source_sha256"] for case in installer_cases)
    writable = [
        target
        for target in snapshot["agent_integrations"]["targets"]
        if target["installs_managed_config"]
    ]
    assert all(target["reinstall_idempotent"] for target in writable)
    assert snapshot["agent_integrations"]["project_skills"]["uninstall_idempotent"] is True


def test_config_contract_cases_capture_round_trips_failures_and_migrations(
    tmp_path: Path,
) -> None:
    config = snapshot_state(ROOT, tmp_path / "data-root")["config"]

    assert all(record["roundtrip"]["equal"] is True for record in config.values())
    assert all(record["failures"]["cases"][0]["error_type"] for record in config.values())
    assert config["provider"]["legacy"]["output"]["legacy_provider"]["key"] == {"configured": True}
    assert config["provider"]["legacy"]["persisted"] is True
    assert set(config["dream"]["legacy"]["persisted_workflows"]) == {
        "concepts",
        "coverage_audit",
        "knowledge_crystals",
        "reinforcement",
        "relations",
        "rule_crystals",
        "terminology_candidates",
    }
    assert "synthetic-provider-secret" not in json.dumps(config, sort_keys=True)
    assert "synthetic-legacy-secret" not in json.dumps(config, sort_keys=True)


def test_distribution_cases_freeze_bootstrap_errors_uninstall_and_idempotence(
    tmp_path: Path,
) -> None:
    distribution = snapshot_state(ROOT, tmp_path / "data-root")["distribution"]
    cases = {case["id"]: case for case in distribution["cases"]}

    assert cases["install.stable-success"]["release_config"] == ('[updates]\nchannel = "stable"\n')
    assert cases["install.dev-success"]["release_config"] == ('[updates]\nchannel = "dev"\n')
    assert cases["install.uv-bootstrap"]["uv_bootstrapped"] is True
    assert cases["install.bun-bootstrap"]["bun_bootstrapped"] is True
    assert cases["install.bun-upgrade"]["bun_upgraded"] is True
    assert cases["install.missing-python-warning"]["exit_class"] == "success"
    assert "python3 is not installed" in cases["install.missing-python-warning"]["stderr"]
    assert cases["install.old-python-warning"]["exit_class"] == "success"
    assert "Python version is 3.11.9" in cases["install.old-python-warning"]["stderr"]
    assert cases["install.bun-upgrade-declined"]["exit_class"] == "error"
    assert cases["install.bun-upgrade-still-old"]["exit_class"] == "error"
    assert cases["install.uv-bootstrap-not-on-path"]["exit_class"] == "error"
    assert cases["install.bun-bootstrap-not-on-path"]["exit_class"] == "error"
    assert cases["install.fresh-clone"]["exit_class"] == "success"
    assert any(
        command.startswith("git:clone ") for command in cases["install.fresh-clone"]["commands"]
    )
    for case_id in (
        "install.missing-git",
        "install.invalid-channel",
        "install.wrong-origin",
        "uninstall.invalid-option",
        "uninstall.unsafe-path",
    ):
        assert cases[case_id]["exit_class"] == "error"
    assert cases["uninstall.keep-data-idempotent"]["repeat_idempotent"] is True
    assert cases["uninstall.keep-data-idempotent"]["tool_removal_attempted"] is True
    assert cases["uninstall.purge-data"]["data_exists"] is False
    assert cases["uninstall.interactive-remove"]["data_exists"] is False
    assert cases["uninstall.interactive-keep"]["data_exists"] is True
    assert cases["uninstall.noninteractive-keep"]["data_exists"] is True
    assert all(
        branch.get("case_ids") or (branch.get("disposition") and branch.get("reason"))
        for branch in distribution["branches"]
    )
    referenced_cases = {
        case_id for branch in distribution["branches"] for case_id in branch.get("case_ids", [])
    }
    assert set(cases) <= referenced_cases


def test_checked_config_failure_fixtures_replay_exact_loader_errors(tmp_path: Path) -> None:
    fixture_root = ROOT / "compatibility/fixtures/config/failures"
    required_case_ids = {
        "provider": {
            "provider.invalid-toml",
            "provider.default-provider-missing",
            "provider.unsupported-type",
            "provider.missing-type",
            "provider.missing-url",
            "provider.unknown-profile-key",
            "provider.unknown-default-key",
            "provider.invalid-id",
            "provider.name-type",
            "provider.type-type",
            "provider.url-type",
            "provider.key-type",
            "provider.default-provider-type",
            "provider.default-model-type",
            "provider.timeout-type",
            "provider.timeout-zero",
            "provider.timeout-negative",
            "provider.timeout-infinite",
            "provider.legacy-collision",
            "provider.legacy-missing-type",
            "provider.legacy-type-mismatch",
            "provider.legacy-unknown-key",
        },
        "dream": {
            "dream.invalid-toml",
            "dream.unknown-workflow",
            "dream.threshold-order",
            "dream.enabled-workflow-empty-model",
            "dream.enabled-type",
            "dream.schedule-type",
            "dream.workflow-provider-type",
            "dream.workflow-enabled-type",
            "dream.schedule-minimum",
            "dream.pending-minimum",
            "dream.pending-maximum-minimum",
            "dream.cycle-maximum-minimum",
            "dream.not-enough-minimum",
        },
        "ingest": {
            "ingest.sentence-order",
            "ingest.symbol-order",
            "ingest.unknown-root",
            "ingest.unknown-short-memory",
            "ingest.unknown-learn",
            "ingest.warning-sentence-type",
            "ingest.rejection-symbol-type",
            "ingest.max-block-type",
            "ingest.warning-sentence-minimum",
            "ingest.rejection-sentence-minimum",
            "ingest.warning-symbol-minimum",
            "ingest.rejection-symbol-minimum",
            "ingest.max-block-minimum",
        },
        "release": {"release.unknown-channel"},
    }
    loaders = {
        "provider": "hieronymus.provider_config:load_provider_catalog",
        "dream": "hieronymus.dream_config:load_dream_config",
        "ingest": "hieronymus.ingest_config:load_ingest_config",
        "release": "hieronymus.release_config:load_release_config",
    }

    from hieronymus.config import HieronymusConfig

    for name, required_ids in required_case_ids.items():
        fixture = json.loads((fixture_root / f"{name}.json").read_text(encoding="utf-8"))
        assert required_ids <= {case["id"] for case in fixture["cases"]}
        for index, case in enumerate(fixture["cases"]):
            assert case["loader"] == loaders[name]
            assert case["target"] in case["files"]
            assert all(isinstance(content, str) for content in case["files"].values())
            case_config = HieronymusConfig(data_root=tmp_path / name / str(index) / "hieronymus")
            case_config.config_root.mkdir(parents=True)
            for relative_path, content in case["files"].items():
                destination = case_config.config_root / relative_path
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(content.encode("utf-8"))
            module_name, symbol_name = case["loader"].split(":", 1)
            loader = getattr(importlib.import_module(module_name), symbol_name)
            expected_type = getattr(
                importlib.import_module(case["error_module"]), case["error_type"]
            )
            with pytest.raises(expected_type) as raised:
                loader(case_config)
            assert str(raised.value) == case["message"]


def test_agent_cases_freeze_host_config_and_owned_skill_lifecycle(tmp_path: Path) -> None:
    integrations = snapshot_state(ROOT, tmp_path / "data-root")["agent_integrations"]
    targets = {target["target"]: target for target in integrations["targets"]}

    for name in ("claude", "codex", "openclaw", "opencode", "gemini"):
        target = targets[name]
        assert target["result_kind"] == "installed"
        assert target["availability_installed"] is True
        assert target["reinstall_idempotent"] is True
        assert any(path.startswith("<HOME>") for path in target["generated_files"])
        assert any(path.startswith("<DATA_ROOT>") for path in target["generated_files"])
    for name in ("mimo", "pi", "hermes"):
        assert targets[name]["result_kind"] == "reserved"
        assert targets[name]["generated_files"] == {}

    project_skills = integrations["project_skills"]
    assert project_skills["reinstall_idempotent"] is True
    assert project_skills["custom_file_preserved"] is True
    assert project_skills["owned_files_removed"] is True
    assert project_skills["uninstall_idempotent"] is True
    assert integrations["failure_cases"] == [
        {
            "id": "gemini.malformed-mcp-servers",
            "target": "gemini",
            "config_path": "<HOME>/.gemini/settings.json",
            "input_bytes": '{"mcpServers": []}\n',
            "error_type": "ValueError",
            "message": "expected object at mcpServers in <HOME>/.gemini/settings.json",
            "host_config_unchanged": True,
            "managed_assets_absent": True,
        }
    ]


def test_every_generated_artifact_matches_two_fresh_builds(tmp_path: Path) -> None:
    first = generate_state_artifacts(ROOT, tmp_path / "first-data-root")
    second = generate_state_artifacts(ROOT, tmp_path / "second-data-root")

    assert first == second
    assert {
        "compatibility/snapshots/state.json",
        "compatibility/fixtures/database/minimal-python.sqlite",
        "compatibility/fixtures/agent-integration/current.json",
        "compatibility/fixtures/install-update/cases.json",
        "compatibility/fixtures/config/current/provider.json",
        "compatibility/fixtures/config/roundtrip/provider.json",
        "compatibility/fixtures/config/failures/provider.json",
        "compatibility/fixtures/config/legacy/provider.json",
    } <= set(first)
    for relative_path, generated in first.items():
        assert (ROOT / relative_path).read_bytes() == generated, relative_path


def test_checked_in_state_snapshot_matches_fresh_synthetic_inventory(tmp_path: Path) -> None:
    expected = json.loads((ROOT / "compatibility/snapshots/state.json").read_text(encoding="utf-8"))

    assert snapshot_state(ROOT, tmp_path / "data-root") == expected


def test_collect_frontend_test_nodeids_uses_vitest_case_listing() -> None:
    node_ids = inventory_state.collect_frontend_test_nodeids(ROOT)

    assert node_ids == sorted(node_ids)
    assert len(node_ids) == 16
    assert (
        "frontend/src/web/components/editors.test.ts::"
        "provider editor opens, submits edited fields, and closes"
    ) in node_ids
    assert all(node_id.startswith("frontend/src/") and "::" in node_id for node_id in node_ids)


def test_collect_frontend_test_nodeids_fails_explicitly_without_bun(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(inventory_state.shutil, "which", lambda _name: None)

    with pytest.raises(RuntimeError, match="Bun is required to collect frontend Vitest nodes"):
        inventory_state.collect_frontend_test_nodeids(ROOT)


def test_every_frontend_vitest_case_has_typed_manifest_ownership() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    collected = set(inventory_state.collect_frontend_test_nodeids(ROOT))
    ownership = {item.node_id: item for item in manifest.frontend_test_ownership}

    assert set(ownership) == collected
    assert all(
        item.contract_ids if item.disposition == "public_contract" else item.reason
        for item in ownership.values()
    )


def test_frontend_ownership_does_not_inherit_route_contracts_from_a_test_file() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    nodes = [
        "frontend/src/web/components/editors.test.ts::"
        "provider editor opens, submits edited fields, and closes",
        "frontend/src/web/components/editors.test.ts::"
        "dreaming editor submits the toggled schedule state",
        "frontend/src/web/components/editors.test.ts::unrelated visual helper",
    ]

    ownership = {
        item["node_id"]: item
        for item in inventory_state.build_frontend_test_ownership(nodes, manifest.contracts)
    }

    assert ownership[nodes[0]]["contract_ids"] == ["http.route.post.api.providers"]
    assert ownership[nodes[1]]["contract_ids"] == ["http.route.post.api.settings.dream"]
    assert ownership[nodes[2]]["disposition"] == "implementation_internal"
    assert "specific public request" in ownership[nodes[2]]["reason"]


def test_owned_data_root_paths_link_to_one_layout_contract(tmp_path: Path) -> None:
    snapshot = snapshot_state(ROOT, tmp_path / "data-root")
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    contract = next(item for item in manifest.contracts if item.id == "data-root.layout")

    assert contract.acceptance_owner == "Pavel Obruchnikov <me@inkyquill.net>"
    assert contract.technical_owner == "data-config"
    assert contract.python_entry_point == "hieronymus.config:HieronymusConfig"
    assert contract.rust_test_target == "crates/hiero-config/tests/data_root_contract.rs::layout"
    assert all(item["contract_id"] == "data-root.layout" for item in snapshot["owned_paths"])


def test_database_fixture_freezes_representative_rows_and_preflight_variants(
    tmp_path: Path,
) -> None:
    database = snapshot_state(ROOT, tmp_path / "data-root")["database"]

    row_counts = database["row_counts"]
    for table in (
        "strict_terms",
        "strict_term_aliases",
        "concepts",
        "concept_facets",
        "crystals",
        "task_sessions",
        "short_term_memories",
        "memory_events",
        "crystal_activations",
        "rag_sources",
        "rag_chunks",
        "memory_graph_migration_ledger",
    ):
        assert row_counts[table] >= 1, table

    variants = {item["id"]: item for item in database["variants"]}
    assert {
        "current-python",
        "supported-legacy-python",
        "empty",
        "partial-python",
        "corrupt",
        "unknown-schema",
    } <= set(variants)
    assert variants["current-python"]["expected"] == {
        "classification": "supported-python",
        "foreign_key_violations": 0,
        "integrity": "ok",
        "safe_to_convert": True,
    }
    assert variants["supported-legacy-python"]["expected"]["safe_to_convert"] is True
    assert variants["partial-python"]["expected"]["safe_to_convert"] is False
    assert variants["corrupt"]["expected"]["classification"] == "corrupt"

    for variant in variants.values():
        fixture = ROOT / variant["fixture"]
        assert fixture.is_file()
        assert inventory_state.preflight_database(fixture) == variant["expected"]
