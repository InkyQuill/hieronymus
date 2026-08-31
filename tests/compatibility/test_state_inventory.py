from __future__ import annotations

import json
import sqlite3
from pathlib import Path

from tools.compatibility.inventory_state import (
    collect_test_nodeids,
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
    assert database["indexes"]
    assert database["triggers"]
    assert database["foreign_keys"]
    assert "memory_graph_migration_ledger" in database["application_migration_ledgers"]
    assert set(database["migration_sources"]) == {"global.sql", "registry.sql", "series.sql"}


def test_sqlite_contract_is_stable_for_dynamic_object_names() -> None:
    connection = sqlite3.connect(":memory:")
    connection.executescript(
        """
        pragma foreign_keys = on;
        create table parent (id integer primary key);
        create table child (
          id integer primary key,
          parent_id integer references parent(id) on delete cascade
        );
        create index child_parent_idx on child(parent_id);
        create trigger child_delete after delete on child begin
          delete from parent where id = old.parent_id;
        end;
        """
    )

    contract = sqlite_contract(connection)

    assert contract["columns"]["child"][1]["name"] == "parent_id"
    assert contract["indexes"][0]["name"] == "child_parent_idx"
    assert contract["triggers"][0]["name"] == "child_delete"
    assert contract["foreign_keys"]["child"] == [
        {
            "from": "parent_id",
            "match": "NONE",
            "on_delete": "CASCADE",
            "on_update": "NO ACTION",
            "table": "parent",
            "to": "id",
        }
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


def test_checked_in_state_snapshot_matches_fresh_synthetic_inventory(tmp_path: Path) -> None:
    expected = json.loads((ROOT / "compatibility/snapshots/state.json").read_text(encoding="utf-8"))

    assert snapshot_state(ROOT, tmp_path / "data-root") == expected
