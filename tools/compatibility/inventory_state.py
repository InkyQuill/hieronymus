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
import tempfile
import tomllib
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
    invalid_toml = {
        "provider": '[broken\nvalue = "unterminated\n',
        "dream": '[broken\nvalue = "unterminated\n',
        "ingest": "[short_memory]\nwarning_sentence_count = 0\n",
        "release": '[updates]\nchannel = "nightly"\n',
    }
    config_paths = {
        "provider": "provider.conf",
        "dream": "dream.conf",
        "ingest": "ingest.conf",
        "release": "release.conf",
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

        failure_config = HieronymusConfig(data_root=case_root / f"{name}-failure")
        failure_config.config_root.mkdir(parents=True)
        failure_path = failure_config.config_root / config_paths[name]
        failure_path.write_text(invalid_toml[name], encoding="utf-8")
        try:
            loaders[name](failure_config)
        except ValueError as error:
            failure = {"error_type": type(error).__name__, "message": str(error)}
        else:  # pragma: no cover - a regression is reported as generation failure
            raise AssertionError(f"{name} invalid config was accepted")
        record["failures"] = {
            "fixture": failure_fixture,
            "cases": [{"id": f"{name}.invalid", **failure}],
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
        connection.execute(
            "update series_language_tags set created_at = ?",
            ("2000-01-01T00:00:00+00:00",),
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
        "source_sha256": source_hashes,
    }


def _run_distribution_cases(repo_root: Path, root: Path) -> list[dict[str, object]]:
    install_specs = (
        ("install.stable-success", "stable", "normal", 0),
        ("install.dev-success", "dev", "normal", 0),
        ("install.uv-bootstrap", "stable", "uv-bootstrap", 0),
        ("install.bun-bootstrap", "stable", "bun-bootstrap", 0),
        ("install.bun-upgrade", "stable", "bun-upgrade", 0),
        ("install.missing-git", "stable", "missing-git", 1),
        ("install.missing-curl", "stable", "missing-curl", 1),
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
        ("uninstall.keep-data-idempotent", "--keep-data", None, True, 0),
        ("uninstall.purge-data", "--purge-data", None, False, 0),
        ("uninstall.interactive-remove", None, "yes\n", False, 0),
        ("uninstall.interactive-keep", None, "no\n", True, 0),
        ("uninstall.noninteractive-keep", None, None, True, 0),
        ("uninstall.invalid-option", "--invalid", None, True, 1),
        ("uninstall.unsafe-path", "--keep-data", None, True, 1),
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
        )
        for case_id, option, interactive_input, expects_data, expected in uninstall_specs
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
    app.mkdir(parents=True)
    (app / ".git").mkdir()
    if scenario == "existing-non-checkout":
        (app / ".git").rmdir()
    data = root / "config" / "hieronymus"
    log = root / "commands.log"
    temp_root = root / "tmp"
    temp_root.mkdir()
    fake_bin = _base_fake_bin(root)
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
if [ "$1" = "-C" ] && [ "$3" = "remote" ]; then echo "{git_origin}"; exit 0; fi
if [ "$1" = "ls-remote" ] && [ "{scenario}" != "no-release-tags" ]; then
  echo "0 refs/tags/v1.2.3"
  echo "0 refs/tags/v1.10.0"
fi
exit 0
''',
        )
    _write_executable(fake_bin / "python3", "#!/bin/sh\nexit 0\n")
    uv_template = root / "uv-template"
    _write_executable(
        uv_template,
        f'#!/bin/sh\necho "uv:$@" >> "{log}"\nexit 0\n',
    )
    if scenario not in {"uv-bootstrap", "uv-declined"}:
        shutil.copy2(uv_template, fake_bin / "uv")
    bun_template = root / "bun-template"
    _write_executable(
        bun_template,
        f'''#!/bin/sh
echo "bun:$@" >> "{log}"
if [ "$1" = "upgrade" ]; then echo "1.3.14" > "{root / "bun-version"}"; exit 0; fi
if [ "$1" = "--version" ]; then
  if [ -f "{root / "bun-version"}" ]; then cat "{root / "bun-version"}"; else echo "1.3.14"; fi
fi
exit 0
''',
    )
    if scenario == "bun-upgrade":
        (root / "bun-version").write_text("1.2.0\n", encoding="utf-8")
        shutil.copy2(bun_template, fake_bin / "bun")
    elif scenario not in {"bun-bootstrap", "bun-declined", "missing-curl"}:
        shutil.copy2(bun_template, fake_bin / "bun")
    if scenario != "missing-curl":
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
mkdir -p "$HOME/.local/bin"
cp "$HIERONYMUS_FIXTURE_UV" "$HOME/.local/bin/uv"
chmod +x "$HOME/.local/bin/uv"
EOF
else
  cat <<'EOF'
mkdir -p "$HOME/.bun/bin"
cp "$HIERONYMUS_FIXTURE_BUN" "$HOME/.bun/bin/bun"
chmod +x "$HOME/.bun/bin/bun"
EOF
fi
''',
        )
    bash = shutil.which("bash")
    if bash is None:
        raise RuntimeError("bash is required for installer fixture generation")
    (fake_bin / "bash").symlink_to(bash)
    environment = {
        "HOME": str(home),
        "PATH": str(fake_bin),
        "TMPDIR": str(temp_root),
        "HIERONYMUS_APP_DIR": str(app),
        "HIERONYMUS_DATA_ROOT": str(data),
        "HIERONYMUS_INSTALL_CHANNEL": channel,
        "HIERONYMUS_INSTALL_YES": ("0" if scenario in {"uv-declined", "bun-declined"} else "1"),
        "HIERONYMUS_FIXTURE_UV": str(uv_template),
        "HIERONYMUS_FIXTURE_BUN": str(bun_template),
    }
    result = subprocess.run(
        ["/bin/sh", str(repo_root / "install.sh")],
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
) -> dict[str, object]:
    root.mkdir(parents=True)
    home = root / "home"
    home.mkdir()
    app = home if case_id == "uninstall.unsafe-path" else root / "managed/hieronymus/app"
    data = root / "config/hieronymus"
    if app != home:
        app.mkdir(parents=True)
        (app / "sentinel").write_text("app\n", encoding="utf-8")
    data.mkdir(parents=True)
    (data / "sentinel").write_text("data\n", encoding="utf-8")
    fake_bin = _base_fake_bin(root)
    log = root / "commands.log"
    _write_executable(fake_bin / "uv", f'#!/bin/sh\necho "uv:$@" >> "{log}"\nexit 0\n')
    environment = {
        "HOME": str(home),
        "PATH": str(fake_bin),
        "HIERONYMUS_APP_DIR": str(app),
        "HIERONYMUS_DATA_ROOT": str(data),
    }
    argv = ["/bin/sh", str(repo_root / "uninstall.sh")]
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
        "app_exists": app.exists(),
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
    from hieronymus.agent_plugins import available_plugins
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
    return {
        "fixture": "compatibility/fixtures/agent-integration/current.json",
        "targets": targets,
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


def _contract_value(contract: object, field: str) -> object:
    if isinstance(contract, Mapping):
        return contract[field]
    return getattr(contract, field)


def _public_contract_ids(node_id: str, contracts: list[object]) -> set[str]:
    node_file, test_case = node_id.split("::", 1)
    normalized = test_case.lower().replace("-", "_")

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
        if any(marker in normalized for marker in ("migrat", "legacy", "canonicalizes")):
            candidate = f"config.{config_kind}.legacy"
        elif any(marker in normalized for marker in ("reject", "unreadable", "invalid", "error")):
            candidate = f"config.{config_kind}.failures"
        elif any(marker in normalized for marker in ("round_trip", "save_and_load", "persists")):
            candidate = f"config.{config_kind}.roundtrip"
        else:
            candidate = f"config.{config_kind}.current"
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
    if node_id.endswith("test_collect_test_nodeids_returns_only_sorted_pytest_node_ids"):
        return "Validates the Python pytest collection parser; inventory implementation only."
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
    for _name, record in snapshot["config"].items():
        artifacts[record["fixtures"]["current"]] = _json_bytes(record["payload"])
        for behavior in ("roundtrip", "failures", "legacy"):
            if behavior in record["fixtures"]:
                artifacts[record["fixtures"][behavior]] = _json_bytes(record[behavior])

    manifest_path = repo_root / "compatibility/manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    original_contracts = manifest["contracts"][:123]
    if len(original_contracts) != 123:
        raise ValueError("expected the accepted 123 transport contracts")
    contracts = [*original_contracts, *_state_contracts()]
    manifest["contracts"] = contracts
    manifest["test_ownership"] = build_test_ownership(snapshot["tests"]["node_ids"], contracts)
    artifacts["compatibility/manifest.json"] = _json_bytes(manifest)
    return dict(sorted(artifacts.items()))


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
