"""Inventory config, database, installer, agent, and Python-test contracts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pty
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import tomllib
from collections import Counter
from collections.abc import Iterable, Iterator, Mapping
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
_DATABASE_VARIANT_FILES = {
    "current-python": "minimal-python.sqlite",
    "supported-legacy-python": "legacy-python.sqlite",
    "empty": "empty.sqlite",
    "partial-python": "partial-python.sqlite",
    "corrupt": "corrupt.sqlite",
    "unknown-schema": "unknown-schema.sqlite",
}
_FIXED_TIMESTAMP = "2000-01-01T00:00:00+00:00"


def collect_test_nodeids(repo_root: Path) -> list[str]:
    """Collect sorted pytest node ids without reading a user data root."""
    resolved_repo_root = repo_root.resolve()
    with tempfile.TemporaryDirectory(prefix="hieronymus-test-collection-") as temp_dir:
        environment = os.environ.copy()
        environment["HIERONYMUS_DATA_ROOT"] = str(Path(temp_dir) / "data-root")
        result = subprocess.run(
            [sys.executable, "-m", "pytest", "--collect-only", "-q"],
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


def collect_frontend_test_nodeids(repo_root: Path) -> list[str]:
    """Collect individual Vitest cases through Vitest's JSON listing boundary."""
    bun = shutil.which("bun")
    if bun is None:
        raise RuntimeError("Bun is required to collect frontend Vitest nodes")
    bun_path = Path(bun)
    if bun_path.parent.name == "shims" and bun_path.parent.parent.name == "mise":
        installed_bun = bun_path.parent.parent / "installs/bun/latest/bin/bun"
        if installed_bun.is_file():
            bun_path = installed_bun

    resolved_repo_root = repo_root.resolve()
    frontend_root = resolved_repo_root / "frontend"
    vitest_entrypoint = frontend_root / "node_modules/vitest/vitest.mjs"
    if not vitest_entrypoint.is_file():
        raise RuntimeError(
            "frontend Vitest dependencies are missing; run bun install --cwd frontend"
        )
    with tempfile.TemporaryDirectory(prefix="hieronymus-vitest-collection-") as temp_dir:
        environment = os.environ.copy()
        environment.update(
            {
                "BUN_INSTALL_CACHE_DIR": str(Path(temp_dir) / "bun-cache"),
                "BUN_RUNTIME_TRANSPILER_CACHE_PATH": "0",
                "HOME": str(Path(temp_dir) / "home"),
            }
        )
        try:
            result = subprocess.run(
                [
                    str(bun_path),
                    str(vitest_entrypoint),
                    "list",
                    "--json",
                    "--no-color",
                ],
                cwd=frontend_root,
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=30,
            )
        except subprocess.TimeoutExpired as error:
            raise RuntimeError("frontend Vitest collection timed out") from error
    if result.returncode != 0:
        diagnostic = result.stderr.strip() or result.stdout.strip() or "unknown Vitest failure"
        raise RuntimeError(f"frontend Vitest collection failed: {diagnostic}")
    try:
        rows = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("frontend Vitest collection did not return JSON") from error
    if not isinstance(rows, list):
        raise RuntimeError("frontend Vitest collection must return an array")

    node_ids: list[str] = []
    for row in rows:
        if (
            not isinstance(row, dict)
            or not isinstance(row.get("file"), str)
            or not isinstance(row.get("name"), str)
        ):
            raise RuntimeError("frontend Vitest collection returned an invalid test record")
        test_file = Path(row["file"]).resolve()
        if not test_file.is_relative_to(frontend_root):
            raise RuntimeError(f"frontend Vitest test outside frontend root: {test_file}")
        relative_file = test_file.relative_to(resolved_repo_root).as_posix()
        node_ids.append(f"{relative_file}::{row['name']}")
    return sorted(node_ids)


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

    indexes = []
    for table_name in table_names:
        index_rows = connection.execute(
            """
            select seq, name, "unique", origin, partial
            from pragma_index_list(?)
            order by name
            """,
            (table_name,),
        ).fetchall()
        for _, name, unique, origin, partial in index_rows:
            index_columns = connection.execute(
                "select seqno, cid, name from pragma_index_info(?) order by seqno",
                (name,),
            ).fetchall()
            sql_row = connection.execute(
                "select sql from sqlite_master where type = 'index' and name = ?",
                (name,),
            ).fetchone()
            indexes.append(
                {
                    "name": str(name),
                    "table": table_name,
                    "unique": bool(unique),
                    "origin": str(origin),
                    "partial": bool(partial),
                    "columns": [
                        {"seq": int(row[0]), "cid": int(row[1]), "name": str(row[2])}
                        for row in index_columns
                    ],
                    "sql": None
                    if sql_row is None or sql_row[0] is None
                    else _normalize_sql(sql_row[0]),
                }
            )
    indexes.sort(key=lambda item: (item["name"], item["table"]))

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

    foreign_keys: dict[str, list[dict[str, object]]] = {}
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
                    "id": int(row[0]),
                    "seq": int(row[1]),
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
        agent_integrations = _agent_integration_inventory(resolved_data_root)

    return {
        "data_root": "<DATA_ROOT>",
        "owned_paths": _owned_path_inventory(resolved_data_root),
        "config": config_inventory,
        "database": database_inventory,
        "distribution": _distribution_inventory(resolved_repo_root, resolved_data_root),
        "agent_integrations": agent_integrations,
        "tests": {
            "node_ids": collect_test_nodeids(resolved_repo_root),
            "frontend_node_ids": collect_frontend_test_nodeids(resolved_repo_root),
        },
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
    previous_home = os.environ.get("HOME")
    synthetic_home = data_root / ".synthetic-home"
    synthetic_home.mkdir(parents=True, exist_ok=True)
    os.environ["HIERONYMUS_DATA_ROOT"] = str(data_root)
    os.environ["HOME"] = str(synthetic_home)
    try:
        yield
    finally:
        if previous_data_root is None:
            os.environ.pop("HIERONYMUS_DATA_ROOT", None)
        else:
            os.environ["HIERONYMUS_DATA_ROOT"] = previous_data_root
        if previous_home is None:
            os.environ.pop("HOME", None)
        else:
            os.environ["HOME"] = previous_home


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

    records = {
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
    _add_config_contract_cases(config, records)
    return records


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
        "fixtures": {"current": fixture},
        "payload": payload,
        "fields": sorted(fields),
        "field_contracts": {field: contract_id for field in sorted(fields)},
    }


def _add_config_contract_cases(config: object, records: dict[str, object]) -> None:
    """Exercise persisted config round trips, rejection paths, and legacy migrations."""
    import tomli_w

    from hieronymus.config import HieronymusConfig
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

    data_root = config.data_root
    case_root = data_root / "config-contract-cases"
    case_root.mkdir()
    loaders = {
        "provider": load_provider_catalog,
        "dream": load_dream_config,
        "ingest": load_ingest_config,
        "release": load_release_config,
    }
    roundtrip_configs = {
        "provider": ProviderCatalog(
            providers={
                "synthetic_openai": ProviderProfile(
                    name="Synthetic OpenAI",
                    type="openai",
                    url="https://provider.invalid/v1",
                    key="synthetic-provider-secret",
                )
            },
            defaults=ProviderDefaults("synthetic_openai", "fixture-model"),
        ),
        "dream": default_dream_config(),
        "ingest": default_ingest_config(),
        "release": default_release_config(),
    }
    savers = {
        "provider": save_provider_catalog,
        "dream": save_dream_config,
        "ingest": save_ingest_config,
        "release": save_release_config,
    }

    for name, record in records.items():
        fixtures = record["fixtures"]
        roundtrip_fixture = f"compatibility/fixtures/config/roundtrip/{name}.json"
        failure_fixture = f"compatibility/fixtures/config/failures/{name}.json"
        fixtures.update({"roundtrip": roundtrip_fixture, "failures": failure_fixture})

        roundtrip_config = HieronymusConfig(data_root=case_root / f"{name}-roundtrip")
        savers[name](roundtrip_config, roundtrip_configs[name])
        loaded = loaders[name](roundtrip_config)
        if name == "provider":
            output = redacted_provider_catalog_payload(loaded)
            output["synthetic_openai"]["key"] = {"configured": True}
        elif name == "dream":
            output = redacted_dream_config_payload(loaded)
        else:
            output = loaded.to_payload()
        record["roundtrip"] = {
            "fixture": roundtrip_fixture,
            "input": record["payload"],
            "output": output,
            "equal": output == record["payload"],
        }

        failure_cases = []
        for index, definition in enumerate(_config_failure_inputs()[name]):
            failure_config = HieronymusConfig(data_root=case_root / f"{name}-failure-{index}")
            failure_config.config_root.mkdir(parents=True)
            for relative_path, content in definition["files"].items():
                destination = failure_config.config_root / relative_path
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(content.encode("utf-8"))
            try:
                loaders[name](failure_config)
            except ValueError as error:
                failure_cases.append(
                    {
                        **definition,
                        "error_module": type(error).__module__,
                        "error_type": type(error).__name__,
                        "loader": (f"{loaders[name].__module__}:{loaders[name].__name__}"),
                        "message": str(error),
                    }
                )
            else:  # pragma: no cover - generation must reject stale cases
                raise AssertionError(f"{definition['id']} invalid config was accepted")
        record["failures"] = {
            "fixture": failure_fixture,
            "cases": failure_cases,
        }

    provider_legacy = HieronymusConfig(data_root=case_root / "provider-legacy")
    provider_legacy.config_root.mkdir(parents=True)
    provider_legacy.dream_config_path.write_text(
        tomli_w.dumps(
            {
                "providers": {
                    "legacy_provider": {
                        "type": "openai",
                        "endpoint": "https://legacy.invalid/v1",
                        "api_key": "synthetic-legacy-secret",
                    }
                }
            }
        ),
        encoding="utf-8",
    )
    migrated_provider = redacted_provider_catalog_payload(load_provider_catalog(provider_legacy))
    migrated_provider["legacy_provider"]["key"] = {"configured": True}
    provider_legacy_fixture = "compatibility/fixtures/config/legacy/provider.json"
    records["provider"]["fixtures"]["legacy"] = provider_legacy_fixture
    records["provider"]["legacy"] = {
        "fixture": provider_legacy_fixture,
        "source": {
            "legacy_provider": {
                "type": "openai",
                "endpoint": "https://legacy.invalid/v1",
                "api_key": {"configured": True},
            }
        },
        "output": migrated_provider,
        "persisted": provider_legacy.provider_config_path.exists(),
    }

    dream_legacy = HieronymusConfig(data_root=case_root / "dream-legacy")
    dream_legacy.config_root.mkdir(parents=True)
    workflow = {
        "provider": "",
        "model": "",
        "enabled": False,
        "max_records_per_pass": 500,
    }
    dream_legacy.dream_config_path.write_text(
        tomli_w.dumps(
            {
                "workflows": {
                    "crystallization": workflow,
                    "relation_discovery": workflow,
                    "reinforcement_compaction": workflow,
                }
            }
        ),
        encoding="utf-8",
    )
    migrated_dream = redacted_dream_config_payload(load_dream_config(dream_legacy))
    dream_legacy_fixture = "compatibility/fixtures/config/legacy/dream.json"
    records["dream"]["fixtures"]["legacy"] = dream_legacy_fixture
    records["dream"]["legacy"] = {
        "fixture": dream_legacy_fixture,
        "source_workflows": [
            "crystallization",
            "relation_discovery",
            "reinforcement_compaction",
        ],
        "output": migrated_dream,
        "persisted_workflows": sorted(migrated_dream["workflows"]),
    }


def _config_failure_inputs() -> dict[str, list[dict[str, object]]]:
    """Return replayable persisted inputs for every public config failure family."""

    def case(case_id: str, target: str, content: str, **other: str) -> dict[str, object]:
        return {"id": case_id, "target": target, "files": {target: content, **other}}

    provider_url = 'url = "https://provider.invalid/v1"\n'
    provider_type = 'type = "openai"\n'
    return {
        "provider": [
            case("provider.invalid-toml", "provider.conf", '[broken\nvalue = "unterminated\n'),
            case(
                "provider.default-provider-missing",
                "provider.conf",
                '[defaults]\nprovider = "missing"\nmodel = "fixture"\n',
            ),
            case(
                "provider.unsupported-type",
                "provider.conf",
                '[bad]\nname = "Bad"\ntype = "made-up"\n' + provider_url,
            ),
            case("provider.missing-type", "provider.conf", "[deepseek]\n" + provider_url),
            case("provider.missing-url", "provider.conf", "[deepseek]\n" + provider_type),
            case(
                "provider.unknown-profile-key",
                "provider.conf",
                "[deepseek]\n" + provider_type + provider_url + 'extra = "nope"\n',
            ),
            case(
                "provider.unknown-default-key",
                "provider.conf",
                '[defaults]\nprovider = ""\nmodel = ""\nextra = "nope"\n',
            ),
            case(
                "provider.invalid-id",
                "provider.conf",
                '["deep.seek"]\n' + provider_type + provider_url,
            ),
            case(
                "provider.name-type",
                "provider.conf",
                "[deepseek]\nname = 1\n" + provider_type + provider_url,
            ),
            case(
                "provider.type-type",
                "provider.conf",
                "[deepseek]\ntype = 1\n" + provider_url,
            ),
            case(
                "provider.url-type",
                "provider.conf",
                "[deepseek]\n" + provider_type + "url = 1\n",
            ),
            case(
                "provider.key-type",
                "provider.conf",
                "[deepseek]\n" + provider_type + provider_url + "key = 1\n",
            ),
            case(
                "provider.default-provider-type",
                "provider.conf",
                "[defaults]\nprovider = 1\n",
            ),
            case(
                "provider.default-model-type",
                "provider.conf",
                "[defaults]\nmodel = 1\n",
            ),
            case(
                "provider.timeout-type",
                "provider.conf",
                "[deepseek]\n" + provider_type + provider_url + 'timeout_seconds = "slow"\n',
            ),
            case(
                "provider.timeout-zero",
                "provider.conf",
                "[deepseek]\n" + provider_type + provider_url + "timeout_seconds = 0\n",
            ),
            case(
                "provider.timeout-negative",
                "provider.conf",
                "[deepseek]\n" + provider_type + provider_url + "timeout_seconds = -1\n",
            ),
            case(
                "provider.timeout-infinite",
                "provider.conf",
                "[deepseek]\n" + provider_type + provider_url + "timeout_seconds = inf\n",
            ),
            case(
                "provider.legacy-collision",
                "dream.conf",
                '[providers.openai]\ntype = "openai"\nendpoint = "https://new.invalid/v1"\n',
                **{
                    "provider.conf": (
                        '[openai]\nname = "Existing"\ntype = "openai"\n'
                        'url = "https://existing.invalid/v1"\n'
                    )
                },
            ),
            case(
                "provider.legacy-missing-type",
                "dream.conf",
                '[providers.openai]\nendpoint = "https://provider.invalid/v1"\n',
            ),
            case(
                "provider.legacy-type-mismatch",
                "dream.conf",
                '[providers.openai]\ntype = 123\nendpoint = "https://provider.invalid/v1"\n',
            ),
            case(
                "provider.legacy-unknown-key",
                "dream.conf",
                '[providers.openai]\ntype = "openai"\n'
                'endpoint = "https://provider.invalid/v1"\nextra = "nope"\n',
            ),
        ],
        "dream": [
            case("dream.invalid-toml", "dream.conf", '[broken\nvalue = "unterminated\n'),
            case(
                "dream.unknown-workflow",
                "dream.conf",
                '[workflows.unknown]\nprovider = ""\nmodel = ""\nenabled = false\n',
            ),
            case(
                "dream.threshold-order",
                "dream.conf",
                "[dreaming]\nmin_pending_short_term_memories = 20\n"
                "max_pending_short_term_memories = 10\n",
            ),
            case(
                "dream.enabled-workflow-empty-model",
                "dream.conf",
                '[workflows.knowledge_crystals]\nprovider = "anthropic"\nmodel = ""\n'
                "enabled = true\n",
            ),
            case("dream.enabled-type", "dream.conf", "[dreaming]\nenabled = 'yes'\n"),
            case(
                "dream.schedule-type",
                "dream.conf",
                "[dreaming]\nschedule_interval_minutes = true\n",
            ),
            case(
                "dream.workflow-provider-type",
                "dream.conf",
                "[workflows.knowledge_crystals]\nprovider = 123\n",
            ),
            case(
                "dream.workflow-enabled-type",
                "dream.conf",
                "[workflows.knowledge_crystals]\nenabled = 'true'\n",
            ),
            case(
                "dream.schedule-minimum",
                "dream.conf",
                "[dreaming]\nschedule_interval_minutes = 0\n",
            ),
            case(
                "dream.pending-minimum",
                "dream.conf",
                "[dreaming]\nmin_pending_short_term_memories = -1\n",
            ),
            case(
                "dream.pending-maximum-minimum",
                "dream.conf",
                "[dreaming]\nmax_pending_short_term_memories = 0\n",
            ),
            case(
                "dream.cycle-maximum-minimum",
                "dream.conf",
                "[dreaming]\nmax_short_term_memories_per_cycle = 0\n",
            ),
            case(
                "dream.not-enough-minimum",
                "dream.conf",
                "[dreaming]\nnot_enough_memories_cycle_threshold = 0\n",
            ),
        ],
        "ingest": [
            case(
                "ingest.sentence-order",
                "ingest.conf",
                "[short_memory]\nwarning_sentence_count = 10\nrejection_sentence_count = 5\n",
            ),
            case(
                "ingest.symbol-order",
                "ingest.conf",
                "[short_memory]\nwarning_symbol_count = 100\nrejection_symbol_count = 50\n",
            ),
            case("ingest.unknown-root", "ingest.conf", "[unknown]\nvalue = 1\n"),
            case(
                "ingest.unknown-short-memory",
                "ingest.conf",
                "[short_memory]\nextra = 1\n",
            ),
            case("ingest.unknown-learn", "ingest.conf", "[learn]\nextra = 1\n"),
            case(
                "ingest.warning-sentence-type",
                "ingest.conf",
                "[short_memory]\nwarning_sentence_count = true\n",
            ),
            case(
                "ingest.rejection-symbol-type",
                "ingest.conf",
                "[short_memory]\nrejection_symbol_count = 1.5\n",
            ),
            case(
                "ingest.max-block-type",
                "ingest.conf",
                "[learn]\nmax_block_chars = '1200'\n",
            ),
            case(
                "ingest.warning-sentence-minimum",
                "ingest.conf",
                "[short_memory]\nwarning_sentence_count = 0\n",
            ),
            case(
                "ingest.rejection-sentence-minimum",
                "ingest.conf",
                "[short_memory]\nrejection_sentence_count = 0\n",
            ),
            case(
                "ingest.warning-symbol-minimum",
                "ingest.conf",
                "[short_memory]\nwarning_symbol_count = -1\n",
            ),
            case(
                "ingest.rejection-symbol-minimum",
                "ingest.conf",
                "[short_memory]\nrejection_symbol_count = -1\n",
            ),
            case(
                "ingest.max-block-minimum",
                "ingest.conf",
                "[learn]\nmax_block_chars = 0\n",
            ),
        ],
        "release": [
            case(
                "release.unknown-channel",
                "release.conf",
                '[updates]\nchannel = "nightly"\n',
            )
        ],
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
    from hieronymus.concepts import ConceptStore
    from hieronymus.config import HieronymusConfig
    from hieronymus.crystals import CrystalStore
    from hieronymus.memory_migration import MemoryGraphMigrator
    from hieronymus.memory_models import TranslationContext
    from hieronymus.rag_store import RagStore
    from hieronymus.recall import RecallService
    from hieronymus.registry import Registry
    from hieronymus.scoring import FeedbackStore
    from hieronymus.termbase import Termbase
    from hieronymus.workspace import WorkspaceStore

    config = HieronymusConfig(data_root=data_root)
    registry = Registry(config)
    series = registry.create_series(
        slug="fixture-series",
        title="Compatibility Fixture",
        source_language="ja",
        target_language="ru",
        language_tags=("ja", "ru", "literary"),
    )
    context = TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
        volume="1",
        chapter="2",
        language_tags=("ja", "ru", "literary"),
        story_scopes=("volume:1", "chapter:2"),
        semantic_tags=("ability", "politics"),
    )

    termbase = Termbase(config, context)
    term_id = termbase.propose(
        category="ability",
        source_text="センス",
        canonical_translation="Чутьё",
        tags=["ability", "ui"],
        notes="Synthetic deterministic terminology rule.",
    )
    termbase.add_alias(
        term_id,
        kind="forbidden_variant",
        text="Ощущение",
        language="ru",
        case_sensitive=True,
    )
    termbase.approve(term_id)
    MemoryGraphMigrator(config).run()

    concepts = ConceptStore(config)
    concept = concepts.create_concept(
        "Council",
        description="Synthetic political institution.",
        status="established",
        confidence=0.9,
        scope_type="series",
        scope_key="series:fixture-series",
        semantic_tags=("politics",),
    )
    concepts.add_facet(
        concept.id,
        "評議会",
        language="ja",
        language_tags=("ja",),
        kind="name",
        confidence=0.9,
        is_canonical=True,
        story_scopes=("volume:1",),
        semantic_tags=("politics",),
    )

    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="correction",
        text="The Council must remain capitalized.",
        source_ref="volume-1/chapter-2",
        metadata={"line": 12, "synthetic": True},
        language_tags=("en",),
        story_scopes=("chapter:2",),
        semantic_tags=("politics",),
        source_credibility="user_correction",
        rule_intent="capitalization",
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="concept_note",
        title="Council capitalization",
        text="The Council is a named political institution.",
        strength=0.8,
        confidence=0.9,
        story_scopes=("volume:1",),
        semantic_tags=("politics",),
        language_tags=("en", "ja"),
        concept_ids=(concept.id,),
        source_memory_ids=[memory_id],
    )
    FeedbackStore(config).record(
        crystal_id,
        "confirmed_by_user",
        "user",
        "Synthetic compatibility confirmation.",
        session.id,
    )

    rag_source = data_root.parent / "compatibility-rag-source.txt"
    rag_source.write_text(
        "The Council appoints the city archivist.\nSense appears in the status screen.\n",
        encoding="utf-8",
    )
    RagStore(config).import_file(
        series.slug,
        rag_source,
        source_ref="fixture-volume-1.txt",
        language_tags=("en",),
        story_scopes=("volume:1",),
        semantic_tags=("politics",),
    )
    RecallService(config).recall(session.id, context, "Council", limit=10)
    workspace.complete_session(session.id)

    with sqlite3.connect(config.database_path) as connection:
        connection.row_factory = sqlite3.Row
        _normalize_database_fixture(connection)
        connection.commit()
        contract = sqlite_contract(connection)
        row_counts = {
            table["name"]: int(
                connection.execute(f'select count(*) from "{table["name"]}"').fetchone()[0]
            )
            for table in contract["tables"]
        }
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
        representative_rows = {
            name: _table_rows(connection, name)
            for name in (
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
            )
        }

    variants = _build_database_variants(data_root, config.database_path)

    migration_root = repo_root / "src/hieronymus/migrations"
    return {
        **contract,
        "fixture": "compatibility/fixtures/database/minimal-python.sqlite",
        "row_counts": row_counts,
        "representative_rows": representative_rows,
        "variants": variants,
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
            "row_counts": "database.schema.current",
            "representative_rows": "database.schema.current",
            "variants": "database.upgrade.preflight",
        },
    }


def _normalize_database_fixture(connection: sqlite3.Connection) -> None:
    """Normalize only volatile fields after data was created through public stores."""
    table_names = [
        str(row[0])
        for row in connection.execute(
            "select name from sqlite_master where type = 'table' and name not like 'sqlite_%'"
        )
    ]
    timestamp_names = {
        "created_at",
        "updated_at",
        "completed_at",
        "last_activity_at",
        "migrated_at",
        "archived_at",
        "superseded_at",
    }
    for table_name in table_names:
        columns = {
            str(row[0])
            for row in connection.execute("select name from pragma_table_info(?)", (table_name,))
        }
        for column_name in sorted(timestamp_names & columns):
            quoted_table = '"' + table_name.replace('"', '""') + '"'
            quoted_column = '"' + column_name.replace('"', '""') + '"'
            connection.execute(
                f"update {quoted_table} set {quoted_column} = ? "
                f"where {quoted_column} is not null and {quoted_column} != ''",
                (_FIXED_TIMESTAMP,),
            )
    if "rag_sources" in table_names:
        rows = connection.execute("select id, metadata_json from rag_sources").fetchall()
        for row in rows:
            metadata = json.loads(row["metadata_json"])
            for key in ("original_path", "normalized_path"):
                if key in metadata:
                    metadata[key] = f"<SYNTHETIC_SOURCE>/{Path(str(metadata[key])).name}"
            connection.execute(
                "update rag_sources set metadata_json = ? where id = ?",
                (json.dumps(metadata, ensure_ascii=False, sort_keys=True), int(row["id"])),
            )


def _build_database_variants(
    data_root: Path,
    current_database_path: Path,
) -> list[dict[str, object]]:
    variant_root = data_root / ".compatibility-database-variants"
    variant_root.mkdir(parents=True, exist_ok=True)
    current_path = variant_root / _DATABASE_VARIANT_FILES["current-python"]
    with (
        sqlite3.connect(current_database_path) as source,
        sqlite3.connect(current_path) as destination,
    ):
        source.backup(destination)

    legacy_path = variant_root / _DATABASE_VARIANT_FILES["supported-legacy-python"]
    with sqlite3.connect(legacy_path) as connection:
        connection.executescript(
            """
            create table series (
              id integer primary key,
              slug text not null unique,
              title text not null,
              default_source_language text not null default '',
              default_target_language text not null default '',
              created_at text not null,
              updated_at text not null
            );
            create table strict_terms (
              id integer primary key,
              series_slug text not null,
              source_language text not null,
              target_language text not null,
              category text not null,
              source_text text not null,
              canonical_translation text not null,
              status text not null,
              notes text not null default '',
              created_at text not null,
              updated_at text not null
            );
            create table crystals (
              id integer primary key,
              crystal_type text not null,
              text text not null,
              title text not null default '',
              scope_type text not null,
              scope_key text not null default '',
              series_slug text not null default '',
              source_language text not null default '',
              target_language text not null default '',
              tags_json text not null default '[]',
              strength real not null,
              confidence real not null,
              status text not null,
              created_at text not null,
              updated_at text not null
            );
            insert into series values (
              1, 'fixture-series', 'Legacy Fixture', 'ja', 'ru',
              '2000-01-01T00:00:00+00:00', '2000-01-01T00:00:00+00:00'
            );
            """
        )

    empty_path = variant_root / _DATABASE_VARIANT_FILES["empty"]
    empty_path.touch()
    partial_path = variant_root / _DATABASE_VARIANT_FILES["partial-python"]
    with sqlite3.connect(partial_path) as connection:
        connection.execute("create table series (id integer primary key, slug text not null)")
    corrupt_path = variant_root / _DATABASE_VARIANT_FILES["corrupt"]
    corrupt_path.write_bytes(b"not a sqlite database\n")
    unknown_path = variant_root / _DATABASE_VARIANT_FILES["unknown-schema"]
    with sqlite3.connect(unknown_path) as connection:
        connection.execute("create table unrelated_application (id integer primary key)")

    records = []
    for variant_id, filename in _DATABASE_VARIANT_FILES.items():
        path = variant_root / filename
        records.append(
            {
                "id": variant_id,
                "fixture": f"compatibility/fixtures/database/{filename}",
                "expected": preflight_database(path),
            }
        )
    return records


def preflight_database(path: Path) -> dict[str, object]:
    """Classify one SQLite fixture read-only using the accepted upgrade boundary."""
    uri = f"file:{path.resolve().as_posix()}?mode=ro"
    try:
        with sqlite3.connect(uri, uri=True) as connection:
            integrity_row = connection.execute("pragma integrity_check").fetchone()
            integrity = str(integrity_row[0]) if integrity_row else "unknown"
            foreign_key_violations = len(connection.execute("pragma foreign_key_check").fetchall())
            tables = {
                str(row[0])
                for row in connection.execute(
                    "select name from sqlite_master "
                    "where type = 'table' and name not like 'sqlite_%'"
                )
            }
            if not tables:
                classification = "empty"
                safe_to_convert = False
            elif {"series", "strict_terms", "crystals", "task_sessions", "rag_sources"} <= tables:
                crystal_columns = {
                    str(row[1]) for row in connection.execute("pragma table_info(crystals)")
                }
                if {"source_credibility", "rule_intent", "soft_origin"} <= crystal_columns:
                    classification = "supported-python"
                else:
                    classification = "supported-legacy-python"
                safe_to_convert = integrity == "ok" and foreign_key_violations == 0
            elif {"series", "strict_terms", "crystals"} <= tables:
                classification = "supported-legacy-python"
                safe_to_convert = integrity == "ok" and foreign_key_violations == 0
            elif "series" in tables or tables.intersection({"strict_terms", "crystals"}):
                classification = "partial-python"
                safe_to_convert = False
            else:
                classification = "unknown-schema"
                safe_to_convert = False
    except sqlite3.DatabaseError:
        classification = "corrupt"
        integrity = "unreadable"
        foreign_key_violations = 0
        safe_to_convert = False
    return {
        "classification": classification,
        "foreign_key_violations": foreign_key_violations,
        "integrity": integrity,
        "safe_to_convert": safe_to_convert,
    }


def _canonical_database_bytes(database_path: Path) -> bytes:
    """Return deterministic SQLite bytes after checkpointing and canonical vacuuming."""
    with tempfile.TemporaryDirectory(prefix="hieronymus-sqlite-canonical-") as temp_dir:
        canonical_path = Path(temp_dir) / "canonical.sqlite"
        with sqlite3.connect(database_path) as connection:
            connection.execute("pragma wal_checkpoint(truncate)")
            connection.execute("vacuum into ?", (str(canonical_path),))
        return canonical_path.read_bytes()


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


def _distribution_inventory(repo_root: Path, data_root: Path) -> dict[str, object]:
    project = tomllib.loads((repo_root / "pyproject.toml").read_text(encoding="utf-8"))
    scripts = project["project"]["scripts"]
    source_hashes = {name: _sha256(repo_root / name) for name in ("install.sh", "uninstall.sh")}
    cases = _run_distribution_cases(repo_root, data_root / ".synthetic-distribution-cases")
    for case in cases:
        case["source_sha256"] = source_hashes[case["source"]]
    return {
        "entry_points": dict(sorted(scripts.items())),
        "fixture": "compatibility/fixtures/install-update/cases.json",
        "cases": cases,
        "branches": _distribution_branch_inventory(cases),
        "source_sha256": source_hashes,
    }


def _distribution_branch_inventory(
    cases: list[dict[str, object]],
) -> list[dict[str, object]]:
    case_ids = {str(case["id"]) for case in cases}
    branch_cases = {
        "install.git.present": ["install.stable-success"],
        "install.git.missing": ["install.missing-git"],
        "install.python.supported": ["install.stable-success"],
        "install.python.missing-warning": ["install.missing-python-warning"],
        "install.python.old-warning": ["install.old-python-warning"],
        "install.bun.present-supported": ["install.stable-success"],
        "install.bun.missing-bootstrap": ["install.bun-bootstrap"],
        "install.bun.bootstrap-completed-without-binary": ["install.bun-bootstrap-not-on-path"],
        "install.bun.missing-declined": ["install.bun-declined"],
        "install.bun.old-upgraded": ["install.bun-upgrade"],
        "install.bun.old-upgrade-declined": ["install.bun-upgrade-declined"],
        "install.bun.old-after-upgrade": ["install.bun-upgrade-still-old"],
        "install.uv.present": ["install.stable-success"],
        "install.uv.missing-bootstrap": ["install.uv-bootstrap"],
        "install.uv.bootstrap-completed-without-binary": ["install.uv-bootstrap-not-on-path"],
        "install.uv.missing-declined": ["install.uv-declined"],
        "install.channel.explicit-stable": ["install.stable-success"],
        "install.channel.explicit-dev": ["install.dev-success"],
        "install.channel.invalid": ["install.invalid-channel"],
        "install.channel.default-noninteractive": ["install.default-noninteractive-stable"],
        "install.checkout.existing-valid": ["install.stable-success"],
        "install.checkout.existing-wrong-origin": ["install.wrong-origin"],
        "install.checkout.existing-non-git": ["install.existing-non-checkout"],
        "install.checkout.fresh-clone": ["install.fresh-clone"],
        "install.stable.tags-found": ["install.stable-success"],
        "install.stable.no-tags": ["install.no-release-tags"],
        "install.dev.main": ["install.dev-success"],
        "install.release-config.stable": ["install.stable-success"],
        "install.release-config.dev": ["install.dev-success"],
        "install.uv-tool-reinstall": ["install.stable-success"],
        "install.hiero.present": ["install.hiero-on-path"],
        "install.hiero.missing-warning": ["install.stable-success"],
        "install.prerequisite.curl-missing": ["install.missing-curl"],
        "install.prerequisite.mktemp-missing": ["install.missing-mktemp"],
        "install.prerequisite.bash-missing": ["install.missing-bash"],
        "uninstall.option.keep": ["uninstall.keep-data-idempotent"],
        "uninstall.option.purge": ["uninstall.purge-data"],
        "uninstall.option.invalid": ["uninstall.invalid-option"],
        "uninstall.path.empty-root-dot-home": ["uninstall.unsafe-path"],
        "uninstall.path.parent-segment": ["uninstall.unsafe-parent-path"],
        "uninstall.path.relative": ["uninstall.unsafe-relative-path"],
        "uninstall.path.absolute-outside-owned-shape": ["uninstall.unsafe-outside-path"],
        "uninstall.path.accepted": ["uninstall.keep-data-idempotent"],
        "uninstall.uv.present-success": ["uninstall.keep-data-idempotent"],
        "uninstall.uv.present-failure-ignored": ["uninstall.tool-removal-failure-ignored"],
        "uninstall.uv.missing": ["uninstall.uv-missing"],
        "uninstall.data.explicit-keep": ["uninstall.keep-data-idempotent"],
        "uninstall.data.explicit-purge": ["uninstall.purge-data"],
        "uninstall.data.tty-remove": ["uninstall.interactive-remove"],
        "uninstall.data.tty-keep": ["uninstall.interactive-keep"],
        "uninstall.data.noninteractive-keep": ["uninstall.noninteractive-keep"],
        "uninstall.repeat-safe": ["uninstall.keep-data-idempotent"],
    }
    unknown = {case_id for ids in branch_cases.values() for case_id in ids} - case_ids
    if unknown:
        raise AssertionError(f"distribution branches reference missing cases: {sorted(unknown)}")
    branches = [
        {"id": branch_id, "case_ids": ids} for branch_id, ids in sorted(branch_cases.items())
    ]
    branches.extend(
        [
            {
                "id": "install.channel.interactive-selection",
                "disposition": "implementation_internal",
                "reason": (
                    "The /dev/tty prompt adapter is shell-platform plumbing; explicit stable, "
                    "dev, invalid, and noninteractive-default outcomes are frozen instead."
                ),
            },
            {
                "id": "install.confirm.interactive-prompt",
                "disposition": "implementation_internal",
                "reason": (
                    "The /dev/tty confirmation adapter is shell-platform plumbing; accepted "
                    "and declined dependency outcomes are frozen through environment overrides."
                ),
            },
            {
                "id": "install.data-root.tilde-expansion",
                "disposition": "implementation_internal",
                "reason": (
                    "Tilde string expansion is shell-path plumbing; all persisted release-config "
                    "effects are frozen with absolute synthetic data roots."
                ),
            },
        ]
    )
    return branches


def _run_distribution_cases(repo_root: Path, root: Path) -> list[dict[str, object]]:
    install_specs = (
        ("install.stable-success", "stable", "normal", 0),
        ("install.dev-success", "dev", "normal", 0),
        ("install.uv-bootstrap", "stable", "uv-bootstrap", 0),
        (
            "install.uv-bootstrap-not-on-path",
            "stable",
            "uv-bootstrap-missing-binary",
            1,
        ),
        ("install.bun-bootstrap", "stable", "bun-bootstrap", 0),
        (
            "install.bun-bootstrap-not-on-path",
            "stable",
            "bun-bootstrap-missing-binary",
            1,
        ),
        ("install.bun-upgrade", "stable", "bun-upgrade", 0),
        ("install.bun-upgrade-declined", "stable", "bun-upgrade-declined", 1),
        ("install.bun-upgrade-still-old", "stable", "bun-upgrade-still-old", 1),
        ("install.missing-python-warning", "stable", "missing-python", 0),
        ("install.old-python-warning", "stable", "old-python", 0),
        ("install.fresh-clone", "stable", "fresh-clone", 0),
        ("install.default-noninteractive-stable", "", "default-channel", 0),
        ("install.hiero-on-path", "stable", "hiero-present", 0),
        ("install.missing-git", "stable", "missing-git", 1),
        ("install.missing-curl", "stable", "missing-curl", 1),
        ("install.missing-mktemp", "stable", "missing-mktemp", 1),
        ("install.missing-bash", "stable", "missing-bash", 1),
        ("install.uv-declined", "stable", "uv-declined", 1),
        ("install.bun-declined", "stable", "bun-declined", 1),
        ("install.no-release-tags", "stable", "no-release-tags", 1),
        ("install.existing-non-checkout", "stable", "existing-non-checkout", 1),
        ("install.invalid-channel", "nightly", "normal", 1),
        ("install.wrong-origin", "stable", "wrong-origin", 1),
    )
    cases = [
        _run_install_case(repo_root, root / case_id, case_id, channel, scenario, expected)
        for case_id, channel, scenario, expected in install_specs
    ]
    uninstall_specs = (
        ("uninstall.keep-data-idempotent", "--keep-data", None, True, 0, "normal"),
        ("uninstall.purge-data", "--purge-data", None, False, 0, "normal"),
        ("uninstall.interactive-remove", None, "yes\n", False, 0, "normal"),
        ("uninstall.interactive-keep", None, "no\n", True, 0, "normal"),
        ("uninstall.noninteractive-keep", None, None, True, 0, "normal"),
        ("uninstall.invalid-option", "--invalid", None, True, 1, "normal"),
        ("uninstall.unsafe-path", "--keep-data", None, True, 1, "home-path"),
        ("uninstall.unsafe-parent-path", "--keep-data", None, True, 1, "parent-path"),
        ("uninstall.unsafe-relative-path", "--keep-data", None, True, 1, "relative-path"),
        ("uninstall.unsafe-outside-path", "--keep-data", None, True, 1, "outside-path"),
        ("uninstall.uv-missing", "--keep-data", None, True, 0, "uv-missing"),
        (
            "uninstall.tool-removal-failure-ignored",
            "--keep-data",
            None,
            True,
            0,
            "uv-fails",
        ),
    )
    cases.extend(
        _run_uninstall_case(
            repo_root,
            root / case_id,
            case_id,
            option,
            interactive_input,
            expects_data,
            expected,
            scenario,
        )
        for case_id, option, interactive_input, expects_data, expected, scenario in uninstall_specs
    )
    return cases


def _write_executable(path: Path, text: str) -> None:
    path.write_text(text.lstrip(), encoding="utf-8")
    path.chmod(0o755)


def _base_fake_bin(root: Path) -> Path:
    fake_bin = root / "bin"
    fake_bin.mkdir(parents=True)
    for command in (
        "awk",
        "cat",
        "chmod",
        "cp",
        "dirname",
        "mkdir",
        "mktemp",
        "rm",
        "sh",
        "sort",
        "tail",
    ):
        source = shutil.which(command)
        if source is None:
            raise RuntimeError(f"required fixture command unavailable: {command}")
        (fake_bin / command).symlink_to(source)
    return fake_bin


def _run_install_case(
    repo_root: Path,
    root: Path,
    case_id: str,
    channel: str,
    scenario: str,
    expected_exit: int,
) -> dict[str, object]:
    root.mkdir(parents=True)
    home = root / "home"
    app = root / "managed" / "hieronymus" / "app"
    if scenario != "fresh-clone":
        app.mkdir(parents=True)
        (app / ".git").mkdir()
    if scenario == "existing-non-checkout":
        (app / ".git").rmdir()
    data = root / "config" / "hieronymus"
    log = root / "commands.log"
    temp_root = root / "tmp"
    temp_root.mkdir()
    fake_bin = _base_fake_bin(root)
    if scenario == "missing-mktemp":
        (fake_bin / "mktemp").unlink()
    git_origin = (
        "https://example.invalid/wrong.git"
        if scenario == "wrong-origin"
        else ("https://github.com/InkyQuill/hieronymus.git")
    )
    if scenario != "missing-git":
        _write_executable(
            fake_bin / "git",
            f'''#!/bin/sh
echo "git:$@" >> "{log}"
if [ "$1" = "clone" ]; then mkdir -p "$3/.git"; exit 0; fi
if [ "$1" = "-C" ] && [ "$3" = "remote" ]; then echo "{git_origin}"; exit 0; fi
if [ "$1" = "ls-remote" ] && [ "{scenario}" != "no-release-tags" ]; then
  echo "0 refs/tags/v1.2.3"
  echo "0 refs/tags/v1.10.0"
fi
exit 0
''',
        )
    if scenario == "old-python":
        _write_executable(
            fake_bin / "python3",
            """#!/bin/sh
if printf '%s' "$2" | grep -q print; then echo "3.11.9"; exit 0; fi
exit 1
""",
        )
        grep = shutil.which("grep")
        if grep is None:
            raise RuntimeError("grep is required for installer fixture generation")
        (fake_bin / "grep").symlink_to(grep)
    elif scenario != "missing-python":
        _write_executable(fake_bin / "python3", "#!/bin/sh\nexit 0\n")
    uv_template = root / "uv-template"
    _write_executable(
        uv_template,
        f'#!/bin/sh\necho "uv:$@" >> "{log}"\nexit 0\n',
    )
    if scenario not in {
        "uv-bootstrap",
        "uv-bootstrap-missing-binary",
        "uv-declined",
        "missing-mktemp",
    }:
        shutil.copy2(uv_template, fake_bin / "uv")
    bun_template = root / "bun-template"
    _write_executable(
        bun_template,
        f'''#!/bin/sh
echo "bun:$@" >> "{log}"
if [ "$1" = "upgrade" ]; then
  if [ "{scenario}" != "bun-upgrade-still-old" ]; then echo "1.3.14" > "{root / "bun-version"}"; fi
  exit 0
fi
if [ "$1" = "--version" ]; then
  if [ -f "{root / "bun-version"}" ]; then cat "{root / "bun-version"}"; else echo "1.3.14"; fi
fi
exit 0
''',
    )
    if scenario in {"bun-upgrade", "bun-upgrade-declined", "bun-upgrade-still-old"}:
        (root / "bun-version").write_text("1.2.0\n", encoding="utf-8")
        shutil.copy2(bun_template, fake_bin / "bun")
    elif scenario not in {
        "bun-bootstrap",
        "bun-bootstrap-missing-binary",
        "bun-declined",
        "missing-curl",
        "missing-bash",
    }:
        shutil.copy2(bun_template, fake_bin / "bun")
    if scenario != "missing-curl":
        uv_install_body = (
            "exit 0"
            if scenario == "uv-bootstrap-missing-binary"
            else '''mkdir -p "$HOME/.local/bin"
cp "$HIERONYMUS_FIXTURE_UV" "$HOME/.local/bin/uv"
chmod +x "$HOME/.local/bin/uv"'''
        )
        bun_install_body = (
            "exit 0"
            if scenario == "bun-bootstrap-missing-binary"
            else '''mkdir -p "$HOME/.bun/bin"
cp "$HIERONYMUS_FIXTURE_BUN" "$HOME/.bun/bin/bun"
chmod +x "$HOME/.bun/bin/bun"'''
        )
        _write_executable(
            fake_bin / "curl",
            f'''#!/bin/sh
echo "curl:$@" >> "{log}"
output=
previous=
for argument in "$@"; do
  if [ "$previous" = "-o" ]; then output="$argument"; fi
  previous="$argument"
done
if [ -n "$output" ]; then
  cat > "$output" <<'EOF'
#!/bin/sh
{uv_install_body}
EOF
else
  cat <<'EOF'
{bun_install_body}
EOF
fi
''',
        )
    bash = shutil.which("bash")
    if bash is None:
        raise RuntimeError("bash is required for installer fixture generation")
    if scenario != "missing-bash":
        (fake_bin / "bash").symlink_to(bash)
    environment = {
        "HOME": str(home),
        "PATH": str(fake_bin),
        "TMPDIR": str(temp_root),
        "HIERONYMUS_APP_DIR": str(app),
        "HIERONYMUS_DATA_ROOT": str(data),
        "HIERONYMUS_INSTALL_CHANNEL": channel,
        "HIERONYMUS_INSTALL_YES": (
            "0" if scenario in {"uv-declined", "bun-declined", "bun-upgrade-declined"} else "1"
        ),
        "HIERONYMUS_FIXTURE_UV": str(uv_template),
        "HIERONYMUS_FIXTURE_BUN": str(bun_template),
    }
    if scenario == "default-channel":
        environment.pop("HIERONYMUS_INSTALL_CHANNEL")
    if scenario == "hiero-present":
        _write_executable(fake_bin / "hiero", "#!/bin/sh\nexit 0\n")
    result = subprocess.run(
        ["/bin/sh", "./install.sh"],
        cwd=repo_root,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    if (result.returncode == 0) != (expected_exit == 0):
        raise AssertionError(f"{case_id} returned {result.returncode}: {result.stderr}")
    release_conf = data / "release.conf"
    return {
        "id": case_id,
        "source": "install.sh",
        "exit_class": "success" if result.returncode == 0 else "error",
        "stdout": _normalize_case_text(result.stdout, root),
        "stderr": _normalize_case_text(result.stderr, root),
        "commands": _normalized_command_log(log, root),
        "release_config": release_conf.read_text(encoding="utf-8")
        if release_conf.exists()
        else None,
        "uv_bootstrapped": (home / ".local/bin/uv").exists(),
        "bun_bootstrapped": (home / ".bun/bin/bun").exists(),
        "bun_upgraded": "bun:upgrade" in (log.read_text(encoding="utf-8") if log.exists() else ""),
    }


def _run_uninstall_case(
    repo_root: Path,
    root: Path,
    case_id: str,
    option: str | None,
    interactive_input: str | None,
    expects_data: bool,
    expected_exit: int,
    scenario: str,
) -> dict[str, object]:
    root.mkdir(parents=True)
    home = root / "home"
    home.mkdir()
    app = {
        "home-path": home,
        "parent-path": Path("../hieronymus/app"),
        "relative-path": Path("relative/hieronymus/app"),
        "outside-path": root / "outside/app",
    }.get(scenario, root / "managed/hieronymus/app")
    data = root / "config/hieronymus"
    if app.is_absolute() and app != home:
        app.mkdir(parents=True)
        (app / "sentinel").write_text("app\n", encoding="utf-8")
    data.mkdir(parents=True)
    (data / "sentinel").write_text("data\n", encoding="utf-8")
    fake_bin = _base_fake_bin(root)
    log = root / "commands.log"
    if scenario != "uv-missing":
        uv_exit = 1 if scenario == "uv-fails" else 0
        _write_executable(
            fake_bin / "uv",
            f'#!/bin/sh\necho "uv:$@" >> "{log}"\nexit {uv_exit}\n',
        )
    environment = {
        "HOME": str(home),
        "PATH": str(fake_bin),
        "HIERONYMUS_APP_DIR": str(app),
        "HIERONYMUS_DATA_ROOT": str(data),
    }
    argv = ["/bin/sh", "./uninstall.sh"]
    if option is not None:
        argv.append(option)
    if interactive_input is None:
        result = subprocess.run(
            argv,
            cwd=repo_root,
            env=environment,
            input="",
            capture_output=True,
            text=True,
            check=False,
        )
    else:
        master, slave = pty.openpty()
        process = subprocess.Popen(
            argv,
            cwd=repo_root,
            env=environment,
            stdin=slave,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        os.close(slave)
        os.write(master, interactive_input.encode())
        stdout, stderr = process.communicate(timeout=10)
        os.close(master)
        result = subprocess.CompletedProcess(argv, process.returncode, stdout, stderr)
    if (result.returncode == 0) != (expected_exit == 0):
        raise AssertionError(f"{case_id} returned {result.returncode}: {result.stderr}")
    first_data_exists = data.exists()
    idempotent = None
    if case_id == "uninstall.keep-data-idempotent":
        second = subprocess.run(
            argv,
            cwd=repo_root,
            env=environment,
            input="",
            capture_output=True,
            text=True,
            check=False,
        )
        idempotent = second.returncode == 0 and data.exists()
    if result.returncode == 0 and first_data_exists != expects_data:
        raise AssertionError(f"{case_id} data retention mismatch")
    return {
        "id": case_id,
        "source": "uninstall.sh",
        "exit_class": "success" if result.returncode == 0 else "error",
        "stdout": _normalize_case_text(result.stdout, root),
        "stderr": _normalize_case_text(result.stderr, root),
        "commands": _normalized_command_log(log, root),
        "tool_removal_attempted": any(
            command == "uv:tool uninstall hieronymus"
            for command in _normalized_command_log(log, root)
        ),
        "app_exists": app.exists() if app.is_absolute() else False,
        "data_exists": first_data_exists,
        "repeat_idempotent": idempotent,
    }


def _normalize_case_text(value: str, root: Path) -> str:
    normalized = value.replace(str(root), "<CASE_ROOT>")
    return re.sub(
        r"hieronymus-uv-install\.[A-Za-z0-9]+",
        "hieronymus-uv-install.<RANDOM>",
        normalized,
    )


def _normalized_command_log(path: Path, root: Path) -> list[str]:
    if not path.exists():
        return []
    return [
        _normalize_case_text(line, root) for line in path.read_text(encoding="utf-8").splitlines()
    ]


def _agent_integration_inventory(data_root: Path) -> dict[str, object]:
    from hieronymus.agent_plugins import available_plugins, resolve_plugin
    from hieronymus.config import HieronymusConfig
    from hieronymus.project_skills import install_project_skills, uninstall_project_skills

    entry_points = {
        "claude": "hieronymus.agent_plugins.claude:ClaudePlugin",
        "codex": "hieronymus.agent_plugins.codex:CodexPlugin",
        "openclaw": "hieronymus.agent_plugins.openclaw:OpenClawPlugin",
        "opencode": "hieronymus.agent_plugins.opencode:OpenCodePlugin",
        "gemini": "hieronymus.agent_plugins.gemini:GeminiPlugin",
        "mimo": "hieronymus.agent_plugins.reserved:MimoPlugin",
        "pi": "hieronymus.agent_plugins.reserved:PiPlugin",
        "hermes": "hieronymus.agent_plugins.reserved:HermesPlugin",
    }
    config = HieronymusConfig(data_root=data_root)
    targets = []
    for plugin in available_plugins():
        first_plan = plugin.install(config)
        first_files = _integration_files(data_root, plugin.name)
        second_plan = plugin.install(config)
        second_files = _integration_files(data_root, plugin.name)
        targets.append(
            {
                "target": plugin.name,
                "python_entry_point": entry_points[plugin.name],
                "aliases": list(plugin.aliases),
                "detect_paths": list(plugin.detect_paths),
                "config_paths": list(plugin.config_paths),
                "installs_managed_config": plugin.installs_managed_config,
                "required_asset_paths": list(plugin.required_asset_paths),
                "result_kind": first_plan.result_kind,
                "availability_installed": plugin.availability(config).installed,
                "generated_files": first_files,
                "reinstall_result_kind": second_plan.result_kind,
                "reinstall_idempotent": first_files == second_files,
                "contract_id": f"agent-integration.target.{plugin.name}",
            }
        )

    workspace = data_root / ".synthetic-project-workspace"
    workspace.mkdir()
    install_project_skills(workspace, ("agents", "claude"))
    first_skills = _relative_file_hashes(workspace, workspace)
    install_project_skills(workspace, ("agents", "claude"))
    second_skills = _relative_file_hashes(workspace, workspace)
    custom = workspace / ".agents/skills/custom/SKILL.md"
    custom.parent.mkdir(parents=True)
    custom.write_text("custom\n", encoding="utf-8")
    uninstall_project_skills(workspace, ("agents", "claude"))
    after_first_uninstall = _relative_file_hashes(workspace, workspace)
    uninstall_project_skills(workspace, ("agents", "claude"))
    after_second_uninstall = _relative_file_hashes(workspace, workspace)

    previous_home = os.environ["HOME"]
    failure_home = data_root / ".synthetic-agent-failure-home"
    failure_settings = failure_home / ".gemini/settings.json"
    failure_settings.parent.mkdir(parents=True)
    malformed_input = '{"mcpServers": []}\n'
    failure_settings.write_text(malformed_input, encoding="utf-8")
    failure_config = HieronymusConfig(data_root=data_root / ".synthetic-agent-failure-data")
    os.environ["HOME"] = str(failure_home)
    try:
        try:
            resolve_plugin("gemini").install(failure_config)
        except ValueError as error:
            failure_case = {
                "id": "gemini.malformed-mcp-servers",
                "target": "gemini",
                "config_path": "<HOME>/.gemini/settings.json",
                "input_bytes": malformed_input,
                "error_type": type(error).__name__,
                "message": str(error).replace(str(failure_home), "<HOME>"),
                "host_config_unchanged": failure_settings.read_text(encoding="utf-8")
                == malformed_input,
                "managed_assets_absent": not (
                    failure_config.agent_plugins_root / "gemini"
                ).exists(),
            }
        else:  # pragma: no cover - fixture generation rejects stale failure cases
            raise AssertionError("Gemini accepted a malformed mcpServers section")
    finally:
        os.environ["HOME"] = previous_home
    return {
        "fixture": "compatibility/fixtures/agent-integration/current.json",
        "targets": targets,
        "failure_cases": [failure_case],
        "project_skills": {
            "python_entry_point": "hieronymus.project_skills:install_project_skills",
            "targets": ["agents", "claude"],
            "installed_files": first_skills,
            "reinstall_idempotent": first_skills == second_skills,
            "custom_file_preserved": ".agents/skills/custom/SKILL.md" in after_first_uninstall,
            "owned_files_removed": after_first_uninstall
            == {".agents/skills/custom/SKILL.md": _sha256(custom)},
            "uninstall_idempotent": after_first_uninstall == after_second_uninstall,
        },
    }


def _integration_files(data_root: Path, target: str) -> dict[str, str]:
    home = Path.home()
    paths = [data_root / "agent-plugins" / target]
    config_candidates = {
        "claude": [home / ".claude.json"],
        "codex": [home / ".codex/config.toml"],
        "openclaw": [home / ".openclaw/openclaw.json"],
        "opencode": [home / ".config/opencode/plugin.json"],
        "gemini": [home / ".gemini/settings.json"],
    }
    paths.extend(config_candidates.get(target, []))
    files: dict[str, str] = {}
    for path in paths:
        if path.is_file():
            files[_portable_path(path, home, data_root)] = _normalized_file_sha256(
                path, home, data_root
            )
        elif path.is_dir():
            for child in sorted(item for item in path.rglob("*") if item.is_file()):
                files[_portable_path(child, home, data_root)] = _normalized_file_sha256(
                    child, home, data_root
                )
    return files


def _portable_path(path: Path, home: Path, data_root: Path) -> str:
    text = str(path)
    text = text.replace(str(home), "<HOME>")
    text = text.replace(str(data_root), "<DATA_ROOT>")
    return text


def _normalized_file_sha256(path: Path, home: Path, data_root: Path) -> str:
    content = path.read_text(encoding="utf-8")
    content = content.replace(str(data_root), "<DATA_ROOT>")
    content = content.replace(str(home), "<HOME>")
    return hashlib.sha256(content.encode()).hexdigest()


def _relative_file_hashes(root: Path, data_root: Path) -> dict[str, str]:
    return {
        path.relative_to(root).as_posix(): _normalized_file_sha256(path, root.parent, data_root)
        for path in sorted(item for item in root.rglob("*") if item.is_file())
    }


def _owned_path_inventory(data_root: Path) -> list[dict[str, object]]:
    return [
        {
            "path": path,
            "kind": kind,
            "exists": (data_root / path.rstrip("/")).exists(),
            "contract_id": "data-root.layout",
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
            "data-root.layout",
            "config",
            "data-config",
            "hieronymus.config:HieronymusConfig",
            [
                "tests/compatibility/test_state_inventory.py",
                "tests/test_agent_plugins.py",
                "tests/test_config.py",
            ],
            "compatibility/snapshots/state.json",
            "crates/hiero-config/tests/data_root_contract.rs::layout",
        ),
        _contract(
            "diagnostics.compatibility.check",
            "diagnostics",
            "distribution-cutover",
            "tools.compatibility.check:main",
            ["tests/compatibility/test_check.py"],
            "compatibility/fixtures/diagnostics/check-success.txt",
            "crates/hiero-compatibility/tests/freeze.rs::compatibility_check",
        ),
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
            "database.upgrade.preflight",
            "database",
            "database-upgrade",
            "tools.compatibility.inventory_state:preflight_database",
            ["tests/compatibility/test_state_inventory.py", "tests/test_db_compatibility.py"],
            "compatibility/snapshots/state.json",
            "crates/hiero-db/tests/upgrade_contract.rs::preflight_variants",
            disposition="intentionally-change",
            adr="docs/adr/0010-data-locations-schema-ownership-and-upgrade.md",
        ),
        _contract(
            "install-update.install-script",
            "install-update",
            "distribution-cutover",
            "install.sh",
            ["tests/compatibility/test_state_inventory.py", "tests/test_release_scripts.py"],
            "compatibility/fixtures/install-update/cases.json",
            "crates/hiero-cli/tests/install_contract.rs::install_script",
        ),
        _contract(
            "install-update.uninstall-script",
            "install-update",
            "distribution-cutover",
            "uninstall.sh",
            ["tests/compatibility/test_state_inventory.py", "tests/test_release_scripts.py"],
            "compatibility/fixtures/install-update/cases.json",
            "crates/hiero-cli/tests/install_contract.rs::uninstall_script",
        ),
    ]
    for name, owner, entry_point, test_path, rust_name in (
        (
            "provider",
            "data-config",
            "hieronymus.provider_config:load_provider_catalog",
            "tests/test_provider_config.py",
            "provider",
        ),
        (
            "dream",
            "dreaming",
            "hieronymus.dream_config:load_dream_config",
            "tests/test_dream_config.py",
            "dream",
        ),
        (
            "ingest",
            "data-config",
            "hieronymus.ingest_config:load_ingest_config",
            "tests/test_ingest_config.py",
            "ingest",
        ),
        (
            "release",
            "distribution-cutover",
            "hieronymus.release_config:load_release_config",
            "tests/test_release_config.py",
            "release",
        ),
    ):
        for behavior in ("roundtrip", "failures"):
            contracts.append(
                _contract(
                    f"config.{name}.{behavior}",
                    "config",
                    owner,
                    entry_point,
                    ["tests/compatibility/test_state_inventory.py", test_path],
                    f"compatibility/fixtures/config/{behavior}/{name}.json",
                    f"crates/hiero-config/tests/config_contract.rs::{rust_name}_{behavior}",
                )
            )
        if name in {"provider", "dream"}:
            contracts.append(
                _contract(
                    f"config.{name}.legacy",
                    "config",
                    owner,
                    entry_point,
                    ["tests/compatibility/test_state_inventory.py", test_path],
                    f"compatibility/fixtures/config/legacy/{name}.json",
                    f"crates/hiero-config/tests/config_contract.rs::{rust_name}_legacy",
                )
            )

    entry_points = {
        "claude": "hieronymus.agent_plugins.claude:ClaudePlugin",
        "codex": "hieronymus.agent_plugins.codex:CodexPlugin",
        "openclaw": "hieronymus.agent_plugins.openclaw:OpenClawPlugin",
        "opencode": "hieronymus.agent_plugins.opencode:OpenCodePlugin",
        "gemini": "hieronymus.agent_plugins.gemini:GeminiPlugin",
        "mimo": "hieronymus.agent_plugins.reserved:MimoPlugin",
        "pi": "hieronymus.agent_plugins.reserved:PiPlugin",
        "hermes": "hieronymus.agent_plugins.reserved:HermesPlugin",
    }
    for target in ("claude", "codex", "openclaw", "opencode", "gemini", "mimo", "pi", "hermes"):
        contracts.append(
            _contract(
                f"agent-integration.target.{target}",
                "agent-integration",
                "daemon-mcp-security",
                entry_points[target],
                [
                    "tests/compatibility/test_state_inventory.py",
                    "tests/test_agent_plugins.py",
                    "tests/test_agent_plugin_installers.py",
                ],
                "compatibility/fixtures/agent-integration/current.json",
                f"crates/hiero-cli/tests/agent_contract.rs::{target}",
            )
        )
    contracts.append(
        _contract(
            "agent-integration.target.gemini.failures",
            "agent-integration",
            "daemon-mcp-security",
            "hieronymus.agent_plugins.gemini:GeminiPlugin",
            [
                "tests/compatibility/test_state_inventory.py",
                "tests/test_agent_plugin_installers.py",
            ],
            "compatibility/fixtures/agent-integration/current.json",
            "crates/hiero-cli/tests/agent_contract.rs::gemini_failures",
        )
    )
    contracts.append(
        _contract(
            "agent-integration.project-skills",
            "agent-integration",
            "daemon-mcp-security",
            "hieronymus.project_skills:install_project_skills",
            [
                "tests/compatibility/test_state_inventory.py",
                "tests/test_project_skills.py",
            ],
            "compatibility/fixtures/agent-integration/current.json",
            "crates/hiero-cli/tests/agent_contract.rs::project_skills",
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
    *,
    disposition: str = "preserve",
    adr: str | None = None,
) -> dict[str, object]:
    contract: dict[str, object] = {
        "acceptance_owner": _ACCEPTANCE_OWNER,
        "disposition": disposition,
        "fixture": fixture,
        "id": contract_id,
        "python_entry_point": python_entry_point,
        "rust_test_target": rust_test_target,
        "surface": surface,
        "technical_owner": technical_owner,
        "tests": tests,
        "last_python_release": "0.7.0",
        "first_rust_release": None,
        "implementation_status": "outstanding",
    }
    if adr is not None:
        contract["adr"] = adr
    return contract


def build_test_ownership(
    node_ids: Iterable[str],
    contracts: Iterable[object],
) -> list[dict[str, object]]:
    """Classify individual pytest cases using explicit, behavior-level rules."""
    contract_list = list(contracts)
    contract_ids = {_contract_value(contract, "id") for contract in contract_list}
    ownership = []
    for node_id in sorted(node_ids):
        matched = _public_contract_ids(node_id, contract_list)
        missing = matched - contract_ids
        if missing:
            raise ValueError(f"ownership rule references missing contracts: {sorted(missing)}")
        if matched:
            ownership.append(
                {
                    "contract_ids": sorted(matched),
                    "disposition": "public_contract",
                    "node_id": node_id,
                }
            )
        else:
            ownership.append(
                {
                    "disposition": "implementation_internal",
                    "node_id": node_id,
                    "reason": _internal_test_reason(node_id),
                }
            )
    return ownership


def build_frontend_test_ownership(
    node_ids: Iterable[str],
    contracts: Iterable[object],
) -> list[dict[str, object]]:
    """Classify each Vitest case without granting file-level route coverage."""
    contract_list = list(contracts)
    contract_ids = {_contract_value(contract, "id") for contract in contract_list}
    public_nodes = {
        (
            "frontend/src/web/components/editors.test.ts::"
            "provider editor opens, submits edited fields, and closes"
        ): {"http.route.post.api.providers"},
        (
            "frontend/src/web/components/editors.test.ts::"
            "dreaming editor submits the toggled schedule state"
        ): {"http.route.post.api.settings.dream"},
        (
            "frontend/src/web/components/MemoryViews.test.ts::"
            "destructive memory actions require confirmation and send the exact payload"
        ): {"http.route.post.api.admin.actions.action"},
    }
    ownership: list[dict[str, object]] = []
    for node_id in sorted(node_ids):
        matched = public_nodes.get(node_id, set())
        missing = matched - contract_ids
        if missing:
            raise ValueError(f"frontend ownership references missing contracts: {sorted(missing)}")
        if matched:
            ownership.append(
                {
                    "contract_ids": sorted(matched),
                    "disposition": "public_contract",
                    "node_id": node_id,
                }
            )
            continue
        _node_file, test_name = node_id.split("::", 1)
        ownership.append(
            {
                "disposition": "implementation_internal",
                "node_id": node_id,
                "reason": (
                    "Frontend implementation behavior with no specific public request contract: "
                    f"{test_name}."
                ),
            }
        )
    return ownership


def _contract_value(contract: object, field: str) -> object:
    if isinstance(contract, Mapping):
        return contract[field]
    return getattr(contract, field)


def _public_contract_ids(node_id: str, contracts: list[object]) -> set[str]:
    node_file, test_case = node_id.split("::", 1)
    normalized = test_case.lower().replace("-", "_")

    if node_file == "tests/test_agent_hooks.py":
        if normalized.startswith("test_hook_session_start_"):
            return {"cli.command.hieronymus-agent-hook.session-start"}
        if normalized.startswith("test_hook_session_end_"):
            return {"cli.command.hieronymus-agent-hook.session-end"}
        return set()

    if node_file == "tests/test_service_http.py":
        service_http_contracts = {
            "test_health_endpoint_returns_daemon_identity": {"http.route.get.health"},
            "test_config_page_is_available_without_a_browser_token": {"frontend.route.get.config"},
            "test_config_and_admin_memory_routes_serve_the_web_application_after_session_setup": {
                "frontend.route.get.admin",
                "frontend.route.get.config",
            },
            "test_web_assets_require_the_same_local_session": {"frontend.route.get.assets.path"},
            "test_provider_api_creates_and_lists_custom_profiles": {
                "http.route.get.api.providers",
                "http.route.post.api.providers",
            },
            "test_provider_check_api_returns_a_structured_failure": {
                "http.route.post.api.providers.id.check"
            },
            "test_settings_apis_are_scoped_to_their_configuration_files": {
                "http.route.get.api.settings.dream",
                "http.route.get.api.settings.ingest",
                "http.route.get.api.settings.release",
                "http.route.post.api.settings.dream",
                "http.route.post.api.settings.ingest",
                "http.route.post.api.settings.release",
            },
            "test_admin_dashboard_api_returns_local_admin_snapshot": {
                "http.route.get.api.admin.dashboard"
            },
            "test_local_origin_can_use_admin_api_without_token": {
                "http.route.get.api.admin.dashboard"
            },
            "test_same_origin_browser_get_without_origin_is_accepted": {
                "http.route.get.api.admin.dashboard"
            },
            "test_admin_websocket_rejects_foreign_origin": {"websocket.route.get.ws.admin"},
            "test_admin_snapshot_api_accepts_a_view_parameter": {
                "http.route.get.api.admin.snapshot"
            },
            "test_admin_memory_actions_are_explicitly_allowlisted": {
                "http.route.post.api.admin.actions.action"
            },
            "test_mcp_route_rejects_unknown_operation": {"http.route.post.api.mcp.operation"},
            "test_mcp_route_executes_series_operation_in_daemon": {
                "http.route.post.api.mcp.operation"
            },
            "test_status_endpoint_returns_paths_and_pid": {"http.route.get.status"},
            "test_status_endpoint_reports_active_dream_cycle": {"http.route.get.status"},
            "test_status_payload_degrades_when_dreaming_status_fails": {"http.route.get.status"},
            "test_status_endpoint_survives_an_obsolete_dream_workflow_config": {
                "http.route.get.status"
            },
            "test_shutdown_endpoint_stops_server": {"http.route.post.shutdown"},
            "test_service_endpoints_reject_missing_or_wrong_token": {
                "http.route.get.health",
                "http.route.get.status",
                "http.route.post.shutdown",
            },
        }
        return service_http_contracts.get(normalized, set())

    if node_file == "tests/test_agent_plugins.py":
        all_targets = {
            f"agent-integration.target.{target}"
            for target in (
                "claude",
                "codex",
                "openclaw",
                "opencode",
                "gemini",
                "mimo",
                "pi",
                "hermes",
            )
        }
        agent_plugin_contracts = {
            "test_available_plugins_lists_canonical_targets_in_order": all_targets,
            "test_resolve_plugin_returns_provider": {"agent-integration.target.codex"},
            "test_resolve_plugin_normalizes_lower_case_name": {"agent-integration.target.codex"},
            "test_resolve_plugin_supports_aliases": {"agent-integration.target.mimo"},
            "test_resolve_plugin_reports_supported_targets": all_targets,
            "test_config_has_agent_plugins_root": {"data-root.layout"},
            "test_codex_availability_requires_host_marker_for_install": {
                "agent-integration.target.codex"
            },
            "test_codex_availability_detects_assets_and_managed_marker": {
                "agent-integration.target.codex"
            },
            "test_codex_availability_rejects_stale_marker_without_entries": {
                "agent-integration.target.codex"
            },
            "test_codex_availability_rejects_incomplete_asset_directory": {
                "agent-integration.target.codex"
            },
            "test_codex_availability_rejects_symlink_required_asset": {
                "agent-integration.target.codex"
            },
            "test_codex_availability_rejects_symlink_asset_directory": {
                "agent-integration.target.codex"
            },
            "test_reserved_provider_ignores_stale_managed_marker": {"agent-integration.target.pi"},
            "test_mimo_availability_detects_mimocode_home": {"agent-integration.target.mimo"},
            "test_reserved_plugins_report_reserved_install_plan": {"agent-integration.target.pi"},
            "test_claude_availability_checks_all_detect_paths": {"agent-integration.target.claude"},
            "test_plugin_plan_includes_availability": {"agent-integration.target.codex"},
        }
        return agent_plugin_contracts.get(normalized, set())

    if node_file == "tests/compatibility/test_check.py":
        test_name = normalized.split("[", 1)[0]
        public_tests = {
            "test_manifest_failures_report_invalid_manifest_and_missing_references",
            "test_inventory_coverage_rejects_unreviewed_items",
            "test_parity_summary_counts_are_derived_from_manifest",
            "test_report_sorts_failures_before_summary",
            "test_main_exits_one_with_sorted_drift_and_parity_summary",
            "test_canonical_check_passes_without_writing_repo_or_caller_state",
            "test_canonical_check_reports_invalid_manifest_without_writing",
            "test_canonical_check_sorts_real_failures_before_summary",
        }
        if test_name in public_tests or test_name.startswith("test_canonical_check_"):
            return {"diagnostics.compatibility.check"}
        return set()

    if node_file.startswith("tests/compatibility/"):
        return set()

    config_kind = next(
        (
            kind
            for kind in ("provider", "dream", "ingest", "release")
            if node_file == f"tests/test_{kind}_config.py"
        ),
        None,
    )
    if config_kind is not None:
        if any(
            marker in normalized
            for marker in (
                "reject",
                "unreadable",
                "invalid",
                "error",
                "reports_",
                "validates_default_provider_exists",
            )
        ):
            candidate = f"config.{config_kind}.failures"
        elif any(marker in normalized for marker in ("migrat", "legacy", "canonicalizes")):
            candidate = f"config.{config_kind}.legacy"
        elif any(marker in normalized for marker in ("round_trip", "save_and_load", "persists")):
            candidate = f"config.{config_kind}.roundtrip"
        elif any(
            marker in normalized
            for marker in (
                "_path_live",
                "_paths_live",
                "test_default_",
                "_defaults_when_missing",
                "redacted_",
                "_accepts_supported_",
                "_defaults_provider_name_",
                "_allows_default_",
                "_does_not_write_provider_profiles_or_secrets",
                "_deprecated_",
                "_existence_is_validated_outside_",
            )
        ):
            candidate = f"config.{config_kind}.current"
        else:
            raise ValueError(f"no explicit config ownership rule for {node_id}")
        return {candidate}

    if node_file == "tests/test_db_compatibility.py":
        return {"database.schema.current"}
    if node_file == "tests/test_memory_graph_migration.py":
        return {"database.migrations.current"}
    if node_file == "tests/test_release_scripts.py":
        if "uninstall" in normalized:
            return {"install-update.uninstall-script"}
        if "install" in normalized and "valid_shell" not in normalized:
            return {"install-update.install-script"}
        return set()
    if node_file == "tests/test_project_skills.py":
        return {"agent-integration.project-skills"}
    if node_file == "tests/test_agent_plugin_installers.py":
        targets = {"claude", "codex", "openclaw", "opencode", "gemini", "mimo", "pi", "hermes"}
        explicit = {
            target
            for target in targets
            if re.search(rf"(?:^|[\[_-]){target}(?:$|[\]_-])", normalized)
        }
        if "writable_plugin_reinstall_is_idempotent" in normalized:
            explicit = {target for target in targets if f"[{target}]" in normalized}
        if "reserved_targets" in normalized:
            explicit = {"mimo", "pi", "hermes"}
        if normalized == "test_json_agent_install_rejects_malformed_section_without_traceback":
            return {"agent-integration.target.gemini.failures"}
        return {f"agent-integration.target.{target}" for target in explicit}

    owning_contracts = [
        contract for contract in contracts if node_file in _contract_value(contract, "tests")
    ]
    matched: set[str] = set()
    cli_matches: list[tuple[str, str]] = []
    for contract in owning_contracts:
        contract_id = str(_contract_value(contract, "id"))
        if contract_id.startswith("mcp.tool."):
            operation = contract_id.removeprefix("mcp.tool.")
            if operation in normalized or operation.removeprefix("hieronymus_") in normalized:
                matched.add(contract_id)
        elif contract_id.startswith("cli.command.hiero."):
            command = (
                contract_id.removeprefix("cli.command.hiero.").replace("-", "_").replace(".", "_")
            )
            if normalized.startswith(
                (
                    f"test_{command}",
                    f"test_console_entrypoint_{command}",
                    f"test_{command}_console_entrypoint",
                )
            ):
                cli_matches.append((command, contract_id))
        elif contract_id.startswith("cli.script."):
            script = contract_id.removeprefix("cli.script.").replace("-", "_")
            if normalized.startswith((f"test_{script}_", f"test_{script}_console_")):
                matched.add(contract_id)
    if cli_matches:
        longest = max(len(command) for command, _ in cli_matches)
        matched.update(
            contract_id for command, contract_id in cli_matches if len(command) == longest
        )
    return matched


def _internal_test_reason(node_id: str) -> str:
    node_file, test_case = node_id.split("::", 1)
    if node_file == "tests/compatibility/test_check.py":
        if test_case.startswith("test_artifact_diffs_"):
            return (
                "Validates byte comparison and closed-world scan helpers; the canonical "
                "command behavior is owned separately."
            )
        if test_case == "test_compatibility_gate_validates_official_mcp_schema_offline":
            return (
                "Validates the offline MCP schema authority and compatibility-gate integration; "
                "implementation-internal self-test, not a public contract."
            )
        raise ValueError(f"no explicit aggregate-check ownership rule for {node_id}")
    if node_id.endswith("test_collect_test_nodeids_returns_only_sorted_pytest_node_ids"):
        return "Validates the Python pytest collection parser; inventory implementation only."
    if node_file == "tests/test_agent_plugins.py":
        reasons = {
            "test_availability_json_paths_are_fresh_lists": (
                "Checks defensive list-copy freshness in the Python JSON serializer; no public "
                "agent-target behavior is added."
            ),
            "test_invalid_plugin_reports_empty_detect_paths": (
                "Checks the abstract Python plugin-base invariant for test-only invalid "
                "subclasses; no supported agent target has this shape."
            ),
            "test_invalid_plugin_reports_empty_config_paths": (
                "Checks the abstract Python plugin-base invariant for test-only invalid "
                "subclasses; no supported agent target has this shape."
            ),
        }
        normalized = test_case.lower().replace("-", "_").split("[", 1)[0]
        if normalized not in reasons:
            raise ValueError(f"no explicit agent-plugin ownership rule for {node_id}")
        return reasons[normalized]
    name = Path(node_file).stem
    categories = (
        (
            ("compatibility", "inventory", "manifest"),
            "Compatibility schema and artifact generator validation",
        ),
        (
            ("config", "provider", "secret", "cache"),
            "Configuration parsing, redaction, and local secret handling",
        ),
        (
            ("db", "database", "migration", "registry", "schema"),
            "SQLite store and migration implementation",
        ),
        (("rag", "semantic"), "Semantic-RAG parsing, conversion, and storage implementation"),
        (("dream",), "Dream scheduler and bounded-pass implementation"),
        (
            ("term", "concept", "crystal", "memory", "recall", "scoring", "short"),
            "Memory-model, scoring, and recall implementation",
        ),
        (
            ("mcp", "http", "server", "tui", "daemon", "service"),
            "Daemon, transport, and UI bridge implementation",
        ),
        (
            ("agent", "install", "release", "skill", "hatch", "workflow"),
            "Distribution and agent integration implementation",
        ),
        (("cli", "doctor", "admin"), "Python command orchestration and diagnostics"),
        (
            ("session", "workspace", "series"),
            "Session, workspace, and series lifecycle implementation",
        ),
    )
    for markers, description in categories:
        if any(marker in name for marker in markers):
            return f"{description}: {test_case.split('[', 1)[0]}."
    raise ValueError(f"no explicit ownership classification rule for {node_id}")


def generate_state_artifacts(repo_root: Path, data_root: Path) -> dict[str, bytes]:
    """Generate every Task 5 artifact from one fresh synthetic root."""
    snapshot = snapshot_state(repo_root, data_root)
    artifacts = {
        "compatibility/snapshots/state.json": _json_bytes(snapshot),
        "compatibility/fixtures/database/minimal-python.sqlite": (
            _canonical_database_bytes(data_root / "hieronymus.sqlite")
        ),
        "compatibility/fixtures/agent-integration/current.json": _json_bytes(
            snapshot["agent_integrations"]
        ),
        "compatibility/fixtures/install-update/cases.json": _json_bytes(snapshot["distribution"]),
    }
    for variant in snapshot["database"]["variants"]:
        if variant["id"] == "current-python":
            continue
        filename = Path(str(variant["fixture"])).name
        source = data_root / ".compatibility-database-variants" / filename
        artifacts[str(variant["fixture"])] = source.read_bytes()
    for _name, record in snapshot["config"].items():
        artifacts[record["fixtures"]["current"]] = _json_bytes(record["payload"])
        for behavior in ("roundtrip", "failures", "legacy"):
            if behavior in record["fixtures"]:
                artifacts[record["fixtures"][behavior]] = _json_bytes(record[behavior])

    manifest_path = repo_root / "compatibility/manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    state_contract_ids = {str(contract["id"]) for contract in _state_contracts()}
    original_contracts = sorted(
        (
            contract
            for contract in manifest["contracts"]
            if str(contract.get("id")) not in state_contract_ids
        ),
        key=lambda contract: str(contract["id"]),
    )
    contracts = [*original_contracts, *_state_contracts()]
    for contract in contracts:
        contract.setdefault("last_python_release", str(manifest["python_reference"]))
        contract.setdefault("first_rust_release", None)
        contract.setdefault("implementation_status", "outstanding")
    manifest["contracts"] = contracts
    manifest["test_ownership"] = build_test_ownership(snapshot["tests"]["node_ids"], contracts)
    manifest["frontend_test_ownership"] = build_frontend_test_ownership(
        snapshot["tests"]["frontend_node_ids"], contracts
    )
    artifacts["compatibility/manifest.json"] = _json_bytes(manifest)
    artifacts["compatibility/fixtures/diagnostics/check-success.txt"] = (
        _render_compatibility_success(manifest).encode()
    )
    return dict(sorted(artifacts.items()))


def _render_compatibility_success(manifest: dict[str, object]) -> str:
    contracts = manifest["contracts"]
    test_ownership = manifest["test_ownership"]
    assert isinstance(contracts, list)
    assert isinstance(test_ownership, list)

    def count_lines(values: list[str]) -> list[str]:
        return [f"  {name}: {count}" for name, count in sorted(Counter(values).items())]

    categories = {
        "implemented": sorted(
            str(contract["id"])
            for contract in contracts
            if contract["implementation_status"] == "implemented"
        ),
        "changed": sorted(
            str(contract["id"])
            for contract in contracts
            if contract["disposition"] == "intentionally-change"
        ),
        "removed": sorted(
            str(contract["id"]) for contract in contracts if contract["disposition"] == "remove"
        ),
        "outstanding": sorted(
            str(contract["id"])
            for contract in contracts
            if contract["implementation_status"] == "outstanding"
        ),
    }
    implementation_lines = ["Implementation state:"]
    for category, contract_ids in categories.items():
        implementation_lines.append(f"  {category} ({len(contract_ids)}):")
        implementation_lines.extend(f"    {contract_id}" for contract_id in contract_ids)
        if not contract_ids:
            implementation_lines.append("    (none)")

    frontend_test_ownership = manifest["frontend_test_ownership"]
    assert isinstance(frontend_test_ownership, list)

    lines = [
        "Compatibility check passed",
        "Parity summary",
        f"Contracts: {len(contracts)}",
        "By surface:",
        *count_lines([str(contract["surface"]) for contract in contracts]),
        "By disposition:",
        *count_lines([str(contract["disposition"]) for contract in contracts]),
        "By technical owner:",
        *count_lines([str(contract["technical_owner"]) for contract in contracts]),
        *implementation_lines,
        f"Test ownership: {len(test_ownership)}",
        "By test-ownership disposition:",
        *count_lines([str(ownership["disposition"]) for ownership in test_ownership]),
        f"Frontend test ownership: {len(frontend_test_ownership)}",
        "By frontend test-ownership disposition:",
        *count_lines([str(ownership["disposition"]) for ownership in frontend_test_ownership]),
    ]
    return "\n".join(lines) + "\n"


def _json_bytes(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def _write_outputs(repo_root: Path, artifacts: Mapping[str, bytes]) -> None:
    for relative_path, content in artifacts.items():
        destination = repo_root / relative_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(content)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="write checked-in fixtures")
    args = parser.parse_args()
    repo_root = Path(__file__).resolve().parents[2]
    with tempfile.TemporaryDirectory(prefix="hieronymus-state-inventory-") as temp_dir:
        data_root = Path(temp_dir) / "data-root"
        artifacts = generate_state_artifacts(repo_root, data_root)
        if args.write:
            _write_outputs(repo_root, artifacts)
        else:
            print(artifacts["compatibility/snapshots/state.json"].decode(), end="")


if __name__ == "__main__":
    main()
