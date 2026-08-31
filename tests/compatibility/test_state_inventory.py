from __future__ import annotations

import importlib
import json
import sqlite3
from pathlib import Path

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
