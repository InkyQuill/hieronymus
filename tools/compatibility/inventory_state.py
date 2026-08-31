"""Inventory config, database, installer, agent, and Python-test contracts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import subprocess
import tempfile
import tomllib
from collections.abc import Iterator, Mapping
from contextlib import contextmanager
from pathlib import Path

_ACCEPTANCE_OWNER = "Pavel Obruchnikov <me@inkyquill.net>"
_SELECTED_ROOT_PATHS = (
    ("hieronymus.sqlite", "file"),
    ("provider.conf", "file"),
    ("dream.conf", "file"),
    ("ingest.conf", "file"),
    ("release.conf", "file"),
    ("llmcache.tmp", "file"),
    ("backups/", "directory"),
    ("agent-plugins/", "directory"),
)


def collect_test_nodeids(repo_root: Path) -> list[str]:
    """Collect sorted pytest node ids without reading a user data root."""
    resolved_repo_root = repo_root.resolve()
    with tempfile.TemporaryDirectory(prefix="hieronymus-test-collection-") as temp_dir:
        environment = os.environ.copy()
        environment["HIERONYMUS_DATA_ROOT"] = str(Path(temp_dir) / "data-root")
        environment["UV_NO_SYNC"] = "1"
        result = subprocess.run(
            ["uv", "run", "pytest", "--collect-only", "-q"],
            cwd=resolved_repo_root,
            env=environment,
            check=True,
            capture_output=True,
            text=True,
        )
    return sorted(
        {
            line.strip()
            for line in result.stdout.splitlines()
            if line.startswith("tests/") and "::" in line
        }
    )


def sqlite_contract(connection: sqlite3.Connection) -> dict[str, object]:
    """Return a deterministic inventory of SQLite schema objects."""
    table_rows = connection.execute(
        """
        select name, sql
        from sqlite_master
        where type = 'table' and name not like 'sqlite_%'
        order by name
        """
    ).fetchall()
    table_names = [str(row[0]) for row in table_rows]
    tables = [{"name": str(row[0]), "sql": _normalize_sql(row[1])} for row in table_rows]

    columns: dict[str, list[dict[str, object]]] = {}
    for table_name in table_names:
        rows = connection.execute(
            """
            select cid, name, type, "notnull", dflt_value, pk, hidden
            from pragma_table_xinfo(?)
            order by cid
            """,
            (table_name,),
        ).fetchall()
        columns[table_name] = [
            {
                "cid": int(row[0]),
                "name": str(row[1]),
                "type": str(row[2]),
                "not_null": bool(row[3]),
                "default": row[4],
                "primary_key": int(row[5]),
                "hidden": int(row[6]),
            }
            for row in rows
        ]

    index_rows = connection.execute(
        """
        select name, tbl_name, sql
        from sqlite_master
        where type = 'index' and sql is not null
        order by name
        """
    ).fetchall()
    indexes = []
    for name, table_name, sql in index_rows:
        index_columns = connection.execute(
            "select seqno, cid, name from pragma_index_info(?) order by seqno",
            (name,),
        ).fetchall()
        indexes.append(
            {
                "name": str(name),
                "table": str(table_name),
                "unique": _normalize_sql(sql).lower().startswith("create unique index"),
                "columns": [str(row[2]) for row in index_columns],
                "sql": _normalize_sql(sql),
            }
        )

    trigger_rows = connection.execute(
        """
        select name, tbl_name, sql
        from sqlite_master
        where type = 'trigger'
        order by name
        """
    ).fetchall()
    triggers = [
        {
            "name": str(row[0]),
            "table": str(row[1]),
            "sql": _normalize_sql(row[2]),
        }
        for row in trigger_rows
    ]

    foreign_keys: dict[str, list[dict[str, str]]] = {}
    for table_name in table_names:
        rows = connection.execute(
            """
            select id, seq, \"table\", \"from\", \"to\", on_update, on_delete, match
            from pragma_foreign_key_list(?)
            order by id, seq
            """,
            (table_name,),
        ).fetchall()
        if rows:
            foreign_keys[table_name] = [
                {
                    "table": str(row[2]),
                    "from": str(row[3]),
                    "to": str(row[4]),
                    "on_update": str(row[5]),
                    "on_delete": str(row[6]),
                    "match": str(row[7]),
                }
                for row in rows
            ]

    return {
        "tables": tables,
        "columns": columns,
        "indexes": indexes,
        "triggers": triggers,
        "foreign_keys": foreign_keys,
    }


def snapshot_state(repo_root: Path, data_root: Path) -> dict[str, object]:
    """Build the state snapshot in a caller-provided empty synthetic root."""
    resolved_repo_root = repo_root.resolve()
    resolved_data_root = _prepare_synthetic_root(data_root)
    with _isolated_environment(resolved_data_root):
        config_inventory = _build_config_inventory(resolved_data_root)
        database_inventory = _build_database_inventory(resolved_repo_root, resolved_data_root)
        agent_integrations = _agent_integration_inventory()

    return {
        "data_root": "<DATA_ROOT>",
        "owned_paths": _owned_path_inventory(resolved_data_root),
        "config": config_inventory,
        "database": database_inventory,
        "distribution": _distribution_inventory(resolved_repo_root),
        "agent_integrations": agent_integrations,
        "tests": {"node_ids": collect_test_nodeids(resolved_repo_root)},
    }


def _prepare_synthetic_root(data_root: Path) -> Path:
    expanded = data_root.expanduser()
    if expanded.is_symlink():
        raise ValueError("synthetic data root must not be a symlink")
    resolved = expanded.resolve()
    default_root = (Path.home() / ".config" / "hieronymus").resolve()
    if resolved in {Path.home().resolve(), default_root}:
        raise ValueError("refusing to inventory a user data root")
    if resolved.exists() and any(resolved.iterdir()):
        raise ValueError("synthetic data root must be empty")
    resolved.mkdir(parents=True, exist_ok=True)
    return resolved


@contextmanager
def _isolated_environment(data_root: Path) -> Iterator[None]:
    previous_data_root = os.environ.get("HIERONYMUS_DATA_ROOT")
    os.environ["HIERONYMUS_DATA_ROOT"] = str(data_root)
    try:
        yield
    finally:
        if previous_data_root is None:
            os.environ.pop("HIERONYMUS_DATA_ROOT", None)
        else:
            os.environ["HIERONYMUS_DATA_ROOT"] = previous_data_root


def _build_config_inventory(data_root: Path) -> dict[str, object]:
    from hieronymus.config import load_config
    from hieronymus.dream_config import (
        default_dream_config,
        load_dream_config,
        redacted_dream_config_payload,
        save_dream_config,
    )
    from hieronymus.ingest_config import (
        default_ingest_config,
        load_ingest_config,
        save_ingest_config,
    )
    from hieronymus.provider_config import (
        ProviderCatalog,
        ProviderDefaults,
        ProviderProfile,
        load_provider_catalog,
        redacted_provider_catalog_payload,
        save_provider_catalog,
    )
    from hieronymus.release_config import (
        default_release_config,
        load_release_config,
        save_release_config,
    )

    config = load_config()
    if config.data_root.resolve() != data_root:
        raise RuntimeError("config loader did not honor the synthetic data root")

    save_provider_catalog(
        config,
        ProviderCatalog(
            providers={
                "synthetic_openai": ProviderProfile(
                    name="Synthetic OpenAI",
                    type="openai",
                    url="https://provider.invalid/v1",
                    key="synthetic-provider-secret",
                    timeout_seconds=30.0,
                )
            },
            defaults=ProviderDefaults(provider="synthetic_openai", model="fixture-model"),
        ),
    )
    save_dream_config(config, default_dream_config())
    save_ingest_config(config, default_ingest_config())
    save_release_config(config, default_release_config())
    config.llm_cache_path.write_text("synthetic cache placeholder\n", encoding="utf-8")
    config.backups_root.mkdir()
    config.agent_plugins_root.mkdir()

    provider_payload = redacted_provider_catalog_payload(load_provider_catalog(config))
    provider_payload["synthetic_openai"]["key"] = {"configured": True}
    dream_payload = redacted_dream_config_payload(load_dream_config(config))
    ingest_payload = load_ingest_config(config).to_payload()
    release_payload = load_release_config(config).to_payload()

    return {
        "provider": _config_record(
            "compatibility/fixtures/config/current/provider.json",
            provider_payload,
            "config.provider.current",
            provider_prefix=True,
        ),
        "dream": _config_record(
            "compatibility/fixtures/config/current/dream.json",
            dream_payload,
            "config.dream.current",
        ),
        "ingest": _config_record(
            "compatibility/fixtures/config/current/ingest.json",
            ingest_payload,
            "config.ingest.current",
        ),
        "release": _config_record(
            "compatibility/fixtures/config/current/release.json",
            release_payload,
            "config.release.current",
        ),
    }


def _config_record(
    fixture: str,
    payload: dict[str, object],
    contract_id: str,
    *,
    provider_prefix: bool = False,
) -> dict[str, object]:
    fields = _leaf_paths(payload)
    if provider_prefix:
        fields = [
            field.replace("synthetic_openai", "providers.<id>", 1)
            if field.startswith("synthetic_openai.")
            else field
            for field in fields
        ]
    return {
        "fixture": fixture,
        "payload": payload,
        "fields": sorted(fields),
        "field_contracts": {field: contract_id for field in sorted(fields)},
    }


def _leaf_paths(value: object, prefix: str = "") -> list[str]:
    if isinstance(value, Mapping):
        paths: list[str] = []
        for key in sorted(value):
            child_prefix = f"{prefix}.{key}" if prefix else str(key)
            paths.extend(_leaf_paths(value[key], child_prefix))
        return paths
    return [prefix]


def _build_database_inventory(repo_root: Path, data_root: Path) -> dict[str, object]:
    from hieronymus.config import HieronymusConfig
    from hieronymus.registry import Registry

    config = HieronymusConfig(data_root=data_root)
    registry = Registry(config)
    registry.create_series(
        slug="fixture-series",
        title="Compatibility Fixture",
        source_language="ja",
        target_language="ru",
    )

    with sqlite3.connect(config.database_path) as connection:
        connection.execute(
            "update series set created_at = ?, updated_at = ?",
            ("2000-01-01T00:00:00+00:00", "2000-01-01T00:00:00+00:00"),
        )
        connection.commit()
        contract = sqlite_contract(connection)
        ledger_names = [
            table["name"]
            for table in contract["tables"]
            if "migration" in table["name"] or "ledger" in table["name"]
        ]
        ledgers = {
            name: {
                "columns": [column["name"] for column in contract["columns"][name]],
                "rows": _table_rows(connection, name),
            }
            for name in ledger_names
        }

    migration_root = repo_root / "src/hieronymus/migrations"
    return {
        **contract,
        "fixture": "compatibility/fixtures/database/minimal-python.sqlite",
        "application_migration_ledgers": ledgers,
        "migration_sources": sorted(path.name for path in migration_root.glob("*.sql")),
        "object_contracts": {
            "tables": "database.schema.current",
            "columns": "database.schema.current",
            "indexes": "database.schema.current",
            "triggers": "database.schema.current",
            "foreign_keys": "database.schema.current",
            "application_migration_ledgers": "database.migrations.current",
            "migration_sources": "database.migrations.current",
        },
    }


def _table_rows(connection: sqlite3.Connection, table_name: str) -> list[dict[str, object]]:
    columns = [
        str(row[1])
        for row in connection.execute(
            "select cid, name from pragma_table_info(?) order by cid",
            (table_name,),
        ).fetchall()
    ]
    quoted_name = '"' + table_name.replace('"', '""') + '"'
    rows = connection.execute(f"select * from {quoted_name}").fetchall()
    return [dict(zip(columns, row, strict=True)) for row in rows]


def _distribution_inventory(repo_root: Path) -> dict[str, object]:
    project = tomllib.loads((repo_root / "pyproject.toml").read_text(encoding="utf-8"))
    scripts = project["project"]["scripts"]
    behaviors = [
        {
            "id": "install.managed-checkout",
            "source": "install.sh",
            "contract_id": "install-update.install-script",
        },
        {
            "id": "install.origin-guard",
            "source": "install.sh",
            "contract_id": "install-update.install-script",
        },
        {
            "id": "install.stable-latest-semver-tag",
            "source": "install.sh",
            "contract_id": "install-update.install-script",
        },
        {
            "id": "install.dev-main",
            "source": "install.sh",
            "contract_id": "install-update.install-script",
        },
        {
            "id": "install.release-channel-config",
            "source": "install.sh",
            "contract_id": "install-update.install-script",
        },
        {
            "id": "install.uv-tool-reinstall",
            "source": "install.sh",
            "contract_id": "install-update.install-script",
        },
        {
            "id": "uninstall.safe-path-guards",
            "source": "uninstall.sh",
            "contract_id": "install-update.uninstall-script",
        },
        {
            "id": "uninstall.keep-data",
            "source": "uninstall.sh",
            "contract_id": "install-update.uninstall-script",
        },
        {
            "id": "uninstall.purge-data",
            "source": "uninstall.sh",
            "contract_id": "install-update.uninstall-script",
        },
        {
            "id": "uninstall.noninteractive-keeps-data",
            "source": "uninstall.sh",
            "contract_id": "install-update.uninstall-script",
        },
    ]
    return {
        "entry_points": dict(sorted(scripts.items())),
        "installer_behaviors": behaviors,
        "source_sha256": {
            name: _sha256(repo_root / name) for name in ("install.sh", "uninstall.sh")
        },
    }


def _agent_integration_inventory() -> list[dict[str, object]]:
    from hieronymus.agent_plugins import available_plugins

    return [
        {
            "target": plugin.name,
            "aliases": list(plugin.aliases),
            "detect_paths": list(plugin.detect_paths),
            "config_paths": list(plugin.config_paths),
            "installs_managed_config": plugin.installs_managed_config,
            "required_asset_paths": list(plugin.required_asset_paths),
            "contract_id": f"agent-integration.target.{plugin.name}",
        }
        for plugin in available_plugins()
    ]


def _owned_path_inventory(data_root: Path) -> list[dict[str, object]]:
    return [
        {
            "path": path,
            "kind": kind,
            "exists": (data_root / path.rstrip("/")).exists(),
        }
        for path, kind in _SELECTED_ROOT_PATHS
    ]


def _normalize_sql(value: object) -> str:
    return " ".join(str(value or "").split())


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _state_contracts() -> list[dict[str, object]]:
    contracts = [
        _contract(
            "config.provider.current",
            "config",
            "data-config",
            "hieronymus.provider_config",
            ["tests/compatibility/test_state_inventory.py", "tests/test_provider_config.py"],
            "compatibility/fixtures/config/current/provider.json",
            "crates/hiero-config/tests/config_contract.rs::provider_current",
        ),
        _contract(
            "config.dream.current",
            "config",
            "dreaming",
            "hieronymus.dream_config",
            ["tests/compatibility/test_state_inventory.py", "tests/test_dream_config.py"],
            "compatibility/fixtures/config/current/dream.json",
            "crates/hiero-config/tests/config_contract.rs::dream_current",
        ),
        _contract(
            "config.ingest.current",
            "config",
            "data-config",
            "hieronymus.ingest_config",
            ["tests/compatibility/test_state_inventory.py", "tests/test_ingest_config.py"],
            "compatibility/fixtures/config/current/ingest.json",
            "crates/hiero-config/tests/config_contract.rs::ingest_current",
        ),
        _contract(
            "config.release.current",
            "config",
            "distribution-cutover",
            "hieronymus.release_config",
            ["tests/compatibility/test_state_inventory.py", "tests/test_release_config.py"],
            "compatibility/fixtures/config/current/release.json",
            "crates/hiero-config/tests/config_contract.rs::release_current",
        ),
        _contract(
            "database.schema.current",
            "database",
            "database-upgrade",
            "hieronymus.registry:Registry",
            ["tests/compatibility/test_state_inventory.py", "tests/test_db_compatibility.py"],
            "compatibility/fixtures/database/minimal-python.sqlite",
            "crates/hiero-db/tests/schema_contract.rs::current_schema",
        ),
        _contract(
            "database.migrations.current",
            "database",
            "database-upgrade",
            "hieronymus.db:apply_migration",
            [
                "tests/compatibility/test_state_inventory.py",
                "tests/test_memory_graph_migration.py",
            ],
            "compatibility/fixtures/database/minimal-python.sqlite",
            "crates/hiero-db/tests/schema_contract.rs::migration_ledgers",
        ),
        _contract(
            "install-update.install-script",
            "install-update",
            "distribution-cutover",
            "install.sh",
            ["tests/compatibility/test_state_inventory.py", "tests/test_release_scripts.py"],
            "compatibility/snapshots/state.json",
            "crates/hiero-cli/tests/install_contract.rs::install_script",
        ),
        _contract(
            "install-update.uninstall-script",
            "install-update",
            "distribution-cutover",
            "uninstall.sh",
            ["tests/compatibility/test_state_inventory.py", "tests/test_release_scripts.py"],
            "compatibility/snapshots/state.json",
            "crates/hiero-cli/tests/install_contract.rs::uninstall_script",
        ),
    ]
    for target in ("claude", "codex", "openclaw", "opencode", "gemini", "mimo", "pi", "hermes"):
        contracts.append(
            _contract(
                f"agent-integration.target.{target}",
                "agent-integration",
                "daemon-mcp-security",
                f"hieronymus.agent_plugins:{target}",
                [
                    "tests/compatibility/test_state_inventory.py",
                    "tests/test_agent_plugins.py",
                    "tests/test_agent_plugin_installers.py",
                ],
                "compatibility/snapshots/state.json",
                f"crates/hiero-cli/tests/agent_contract.rs::{target}",
            )
        )
    return contracts


def _contract(
    contract_id: str,
    surface: str,
    technical_owner: str,
    python_entry_point: str,
    tests: list[str],
    fixture: str,
    rust_test_target: str,
) -> dict[str, object]:
    return {
        "acceptance_owner": _ACCEPTANCE_OWNER,
        "disposition": "preserve",
        "fixture": fixture,
        "id": contract_id,
        "python_entry_point": python_entry_point,
        "rust_test_target": rust_test_target,
        "surface": surface,
        "technical_owner": technical_owner,
        "tests": tests,
    }


def _test_ownership(
    node_ids: list[str],
    contracts: list[dict[str, object]],
) -> list[dict[str, object]]:
    contracts_by_test: dict[str, list[str]] = {}
    for contract in contracts:
        for test_path in contract["tests"]:
            contracts_by_test.setdefault(test_path, []).append(contract["id"])

    ownership = []
    for node_id in node_ids:
        node_file = node_id.split("::", 1)[0]
        contract_ids = sorted(contracts_by_test.get(node_file, []))
        if contract_ids:
            ownership.append(
                {
                    "contract_ids": contract_ids,
                    "disposition": "public_contract",
                    "node_id": node_id,
                }
            )
        else:
            ownership.append(
                {
                    "disposition": "implementation_internal",
                    "node_id": node_id,
                    "reason": _internal_test_reason(node_file),
                }
            )
    return ownership


def _internal_test_reason(node_file: str) -> str:
    name = Path(node_file).stem
    categories = (
        (("config", "provider"), "Python configuration parsing and validation internals"),
        (("db", "database", "migration"), "Python SQLite store and migration internals"),
        (("rag", "semantic"), "Python semantic-RAG implementation details"),
        (("dream",), "Python Dream pipeline implementation details"),
        (("term", "concept", "crystal", "memory", "recall"), "Python memory-model internals"),
        (("mcp", "http", "server", "tui"), "Python daemon and transport internals"),
        (("agent", "install", "release", "skill"), "Python distribution integration internals"),
        (("cli",), "Python Click orchestration internals"),
        (("compatibility", "inventory", "manifest"), "Python compatibility-harness internals"),
    )
    for markers, description in categories:
        if any(marker in name for marker in markers):
            return f"{description} exercised by {node_file}."
    return f"Python application implementation details exercised by {node_file}."


def _write_outputs(repo_root: Path, snapshot: dict[str, object], data_root: Path) -> None:
    fixture_root = repo_root / "compatibility/fixtures/config/current"
    fixture_root.mkdir(parents=True, exist_ok=True)
    for name, record in snapshot["config"].items():
        (fixture_root / f"{name}.json").write_text(
            json.dumps(record["payload"], indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    database_fixture = repo_root / "compatibility/fixtures/database/minimal-python.sqlite"
    database_fixture.parent.mkdir(parents=True, exist_ok=True)
    with (
        sqlite3.connect(data_root / "hieronymus.sqlite") as source,
        sqlite3.connect(database_fixture) as destination,
    ):
        source.backup(destination)

    snapshot_path = repo_root / "compatibility/snapshots/state.json"
    snapshot_path.write_text(
        json.dumps(snapshot, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    manifest_path = repo_root / "compatibility/manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    contracts = manifest["contracts"]
    existing_ids = {contract["id"] for contract in contracts}
    for contract in _state_contracts():
        if contract["id"] not in existing_ids:
            contracts.append(contract)
            existing_ids.add(contract["id"])
    manifest["test_ownership"] = _test_ownership(snapshot["tests"]["node_ids"], contracts)
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="write checked-in fixtures")
    args = parser.parse_args()
    repo_root = Path(__file__).resolve().parents[2]
    with tempfile.TemporaryDirectory(prefix="hieronymus-state-inventory-") as temp_dir:
        data_root = Path(temp_dir) / "data-root"
        snapshot = snapshot_state(repo_root, data_root)
        if args.write:
            _write_outputs(repo_root, snapshot, data_root)
        else:
            print(json.dumps(snapshot, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
