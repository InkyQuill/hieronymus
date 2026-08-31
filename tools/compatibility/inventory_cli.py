"""Generate a deterministic inventory of installed CLI entry points."""

from __future__ import annotations

import argparse
import copy
import json
import os
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path
from typing import Any

import click
from click.testing import CliRunner

from hieronymus.agent_hooks import main as agent_hook_main
from hieronymus.cli import main as cli_main

_CLICK_EXIT_BEHAVIOR = {
    "success": 0,
    "click_exception": 1,
    "usage_error": 2,
}
_ENTRY_POINT_GROUPS: dict[str, click.Group] = {
    "hieronymus.agent_hooks:main": agent_hook_main,
    "hieronymus.cli:main": cli_main,
}
_MCP_REPLAY_OPTION = "--replay-mcp-entrypoint"
_MCP_REPLAY_OUTCOME_ENV = "HIERONYMUS_MCP_FIXTURE_OUTCOME"


def snapshot_cli(repo_root: Path) -> dict[str, object]:
    """Return installed scripts and Click metadata without running callbacks."""
    scripts = _installed_scripts(repo_root)
    commands = {
        name: walk(
            _ENTRY_POINT_GROUPS[entry_point],
            invocation_prefix=(name,),
            canonical_invocation_prefix=tuple(_canonical_invocation(name)),
            fixture_prefix=_fixture_prefix(name),
            contract_prefix=f"cli.command.{name}",
        )
        for name, entry_point in scripts.items()
        if entry_point in _ENTRY_POINT_GROUPS
    }

    return {
        "scripts": scripts,
        "script_contracts": {
            name: _script_contract(name, entry_point) for name, entry_point in scripts.items()
        },
        "command_paths": {
            name: [row["path"] for row in commands.get(name, [])] for name in scripts
        },
        "commands": commands,
    }


def walk(
    group: click.Group,
    prefix: tuple[str, ...] = (),
    *,
    invocation_prefix: tuple[str, ...] = ("hiero",),
    canonical_invocation_prefix: tuple[str, ...] = ("hiero",),
    fixture_prefix: str = "",
    contract_prefix: str = "cli.command",
) -> list[dict[str, object]]:
    """Recursively inventory *group* in stable command-name order."""
    rows: list[dict[str, object]] = []
    for name, command in sorted(group.commands.items()):
        path = (*prefix, name)
        rows.append(
            command_record(
                path,
                command,
                invocation_prefix=invocation_prefix,
                canonical_invocation_prefix=canonical_invocation_prefix,
                fixture_prefix=fixture_prefix,
                contract_prefix=contract_prefix,
            )
        )
        if isinstance(command, click.Group):
            rows.extend(
                walk(
                    command,
                    path,
                    invocation_prefix=invocation_prefix,
                    canonical_invocation_prefix=canonical_invocation_prefix,
                    fixture_prefix=fixture_prefix,
                    contract_prefix=contract_prefix,
                )
            )
    return rows


def command_record(
    path: tuple[str, ...],
    command: click.Command,
    *,
    invocation_prefix: tuple[str, ...],
    canonical_invocation_prefix: tuple[str, ...],
    fixture_prefix: str,
    contract_prefix: str,
) -> dict[str, object]:
    """Serialize the public metadata of one Click command."""
    slug = "-".join(path)
    contract_suffix = ".".join(path)
    failure_args = _failure_args(command)
    return {
        "contract_id": f"{contract_prefix}.{contract_suffix}",
        "path": " ".join(path),
        "kind": "group" if isinstance(command, click.Group) else "command",
        "help": command.help or "",
        "parameters": [_parameter_record(parameter) for parameter in command.params],
        "exit_behavior": dict(_CLICK_EXIT_BEHAVIOR),
        "success_fixture": f"compatibility/fixtures/cli/{fixture_prefix}help-{slug}.txt",
        "failure_fixture": f"compatibility/fixtures/cli/{fixture_prefix}failure-{slug}.txt",
        "success_args": ["--help"],
        "failure_args": failure_args,
        "fixture_contract": _contract_fixture_path(f"{contract_prefix}.{contract_suffix}"),
        "invocation": [*invocation_prefix, *path],
        "canonical_rust_invocation": [*canonical_invocation_prefix, *path],
    }


def _installed_scripts(repo_root: Path) -> dict[str, str]:
    data = tomllib.loads((repo_root / "pyproject.toml").read_text(encoding="utf-8"))
    scripts = data["project"]["scripts"]
    if not isinstance(scripts, dict) or not all(
        isinstance(name, str) and isinstance(target, str) for name, target in scripts.items()
    ):
        raise ValueError("project.scripts must map script names to entry points")
    return dict(sorted(scripts.items()))


def _script_contract(name: str, entry_point: str) -> dict[str, object]:
    fixture_names = {
        "hiero": ("help-root.txt", "failure-root.txt"),
        "hieronymus": ("help-hieronymus-root.txt", "failure-hieronymus-root.txt"),
        "hieronymus-agent-hook": ("agent-hook-help.txt", "agent-hook-failure.txt"),
        "hieronymus-mcp": ("mcp-entrypoint-success.json", "mcp-entrypoint-failure.json"),
    }
    success_name, failure_name = fixture_names[name]
    contract_id = f"cli.script.{name}"
    record: dict[str, object] = {
        "contract_id": contract_id,
        "entry_point": entry_point,
        "canonical_rust_invocation": _canonical_invocation(name),
        "success_fixture": f"compatibility/fixtures/cli/{success_name}",
        "failure_fixture": f"compatibility/fixtures/cli/{failure_name}",
        "success_args": ["--help"] if entry_point in _ENTRY_POINT_GROUPS else [],
        "failure_args": (["--compat-invalid-option"] if entry_point in _ENTRY_POINT_GROUPS else []),
        "fixture_contract": _contract_fixture_path(contract_id),
    }
    command = _ENTRY_POINT_GROUPS.get(entry_point)
    if command is not None:
        record.update(
            {
                "kind": "group",
                "help": command.help or "",
                "parameters": [_parameter_record(parameter) for parameter in command.params],
                "exit_behavior": dict(_CLICK_EXIT_BEHAVIOR),
            }
        )
    else:
        record.update(
            {
                "kind": "stdio",
                "success_args": [_MCP_REPLAY_OPTION, "success"],
                "failure_args": [_MCP_REPLAY_OPTION, "failure"],
                "success_invocation": _mcp_replay_invocation("success"),
                "failure_invocation": _mcp_replay_invocation("failure"),
                "success_environment": _mcp_replay_environment("success"),
                "failure_environment": _mcp_replay_environment("failure"),
                "exit_behavior": {"success": 0, "failure": 1},
            }
        )
    return record


def _canonical_invocation(script_name: str) -> list[str]:
    return {
        "hiero": ["hiero"],
        "hieronymus": ["hiero"],
        "hieronymus-agent-hook": ["hiero", "agent-hook"],
        "hieronymus-mcp": ["hiero", "mcp"],
    }[script_name]


def _fixture_prefix(script_name: str) -> str:
    return {
        "hiero": "",
        "hieronymus": "hieronymus-",
        "hieronymus-agent-hook": "agent-hook-",
    }[script_name]


def _parameter_record(parameter: click.Parameter) -> dict[str, object]:
    if isinstance(parameter, click.Option):
        declarations = [*parameter.opts, *parameter.secondary_opts]
        kind = "option"
        multiple = parameter.multiple
        is_flag = parameter.is_flag
    else:
        declarations = [parameter.human_readable_name]
        kind = "argument"
        multiple = parameter.nargs == -1
        is_flag = False

    return {
        "name": parameter.name,
        "declarations": declarations,
        "kind": kind,
        "type": _type_record(parameter.type),
        "default": _normalize_default(parameter.default, parameter.type),
        "required": parameter.required,
        "multiple": multiple,
        "nargs": parameter.nargs,
        "is_flag": is_flag,
        "help": parameter.help if isinstance(parameter, click.Option) else None,
    }


def _type_record(parameter_type: click.ParamType) -> dict[str, object]:
    record: dict[str, object] = {"name": parameter_type.name}
    if isinstance(parameter_type, click.Choice):
        record.update(
            {
                "choices": list(parameter_type.choices),
                "case_sensitive": parameter_type.case_sensitive,
            }
        )
    elif isinstance(parameter_type, click.Path):
        record.update(
            {
                "exists": parameter_type.exists,
                "file_okay": parameter_type.file_okay,
                "dir_okay": parameter_type.dir_okay,
                "writable": parameter_type.writable,
                "readable": parameter_type.readable,
                "resolve_path": parameter_type.resolve_path,
                "allow_dash": parameter_type.allow_dash,
            }
        )
    return record


def _normalize_default(value: Any, parameter_type: click.ParamType) -> object:
    if isinstance(parameter_type, click.Path) and value is not None:
        return "<PATH>"
    if isinstance(value, Path):
        return "<PATH>"
    if isinstance(value, tuple):
        return [_normalize_default(item, parameter_type) for item in value]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    return f"<{type(value).__name__.upper()}>"


def _contract_fixture_path(contract_id: str) -> str:
    filename = contract_id.replace(".", "_").replace("-", "_")
    return f"compatibility/fixtures/cli/contracts/{filename}.json"


def _failure_args(command: click.Command) -> list[str]:
    if any(parameter.required for parameter in command.params):
        return []
    return ["--compat-invalid-option"]


def _render_case(command: click.Command, args: list[str], prog_name: str) -> tuple[int, str]:
    isolated_command = copy.copy(command)

    def callback_guard(*_args: object, **_kwargs: object) -> None:
        raise AssertionError("CLI inventory must not execute command callbacks")

    isolated_command.callback = callback_guard
    result = CliRunner().invoke(isolated_command, args, prog_name=prog_name)
    output = result.output.replace("\r\n", "\n")
    return result.exit_code, output


def _write_text_fixture(repo_root: Path, reference: str, content: str) -> None:
    destination = repo_root / reference
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(content, encoding="utf-8")


def _write_click_fixtures(repo_root: Path, snapshot: dict[str, object]) -> None:
    scripts = snapshot["script_contracts"]
    assert isinstance(scripts, dict)
    roots = {
        "hiero": cli_main,
        "hieronymus": cli_main,
        "hieronymus-agent-hook": agent_hook_main,
    }
    for script_name, command in roots.items():
        contract = scripts[script_name]
        assert isinstance(contract, dict)
        success_args = [str(arg) for arg in contract["success_args"]]
        failure_args = [str(arg) for arg in contract["failure_args"]]
        success_code, success_output = _render_case(command, success_args, script_name)
        failure_code, failure_output = _render_case(command, failure_args, script_name)
        if (success_code, failure_code) != (0, 2):
            raise RuntimeError(f"unexpected Click exit behavior for {script_name}")
        _write_text_fixture(repo_root, str(contract["success_fixture"]), success_output)
        _write_text_fixture(repo_root, str(contract["failure_fixture"]), failure_output)

    command_groups = snapshot["commands"]
    assert isinstance(command_groups, dict)
    for script_name, rows in command_groups.items():
        entry_point = snapshot["scripts"][script_name]
        group = _ENTRY_POINT_GROUPS[entry_point]
        assert isinstance(rows, list)
        for row in rows:
            assert isinstance(row, dict)
            path = str(row["path"]).split()
            command = _resolve_command(group, path)
            prog_name = " ".join(str(part) for part in row["invocation"])
            success_args = [str(arg) for arg in row["success_args"]]
            failure_args = [str(arg) for arg in row["failure_args"]]
            success_code, success_output = _render_case(command, success_args, prog_name)
            failure_code, failure_output = _render_case(command, failure_args, prog_name)
            if (success_code, failure_code) != (0, 2):
                raise RuntimeError(f"unexpected Click exit behavior for {prog_name}")
            _write_text_fixture(repo_root, str(row["success_fixture"]), success_output)
            _write_text_fixture(repo_root, str(row["failure_fixture"]), failure_output)


def _resolve_command(group: click.Group, path: list[str]) -> click.Command:
    command: click.Command = group
    for name in path:
        if not isinstance(command, click.Group):
            raise ValueError(f"non-group command before end of path: {' '.join(path)}")
        command = command.commands[name]
    return command


def _write_non_click_entrypoint_fixtures(repo_root: Path, snapshot: dict[str, object]) -> None:
    scripts = snapshot["script_contracts"]
    assert isinstance(scripts, dict)
    contract = scripts["hieronymus-mcp"]
    assert isinstance(contract, dict)
    success = replay_mcp_entrypoint_case("success")
    failure = replay_mcp_entrypoint_case("failure")
    if (success["exit_code"], failure["exit_code"]) != (0, 1):
        raise RuntimeError("unexpected replayed MCP entrypoint exit behavior")
    _write_json_fixture(repo_root, str(contract["success_fixture"]), success)
    _write_json_fixture(repo_root, str(contract["failure_fixture"]), failure)


def replay_mcp_entrypoint_case(outcome: str) -> dict[str, object]:
    """Replay one isolated MCP startup outcome and capture process behavior."""
    if outcome not in {"success", "failure"}:
        raise ValueError(f"unknown MCP replay outcome: {outcome}")

    args = [_MCP_REPLAY_OPTION, outcome]
    with tempfile.TemporaryDirectory(prefix="hieronymus-mcp-compat-") as data_root:
        environment = os.environ.copy()
        environment.update(
            {
                "HIERONYMUS_DATA_ROOT": data_root,
                _MCP_REPLAY_OUTCOME_ENV: outcome,
            }
        )
        process = subprocess.run(
            [sys.executable, "-m", "tools.compatibility.inventory_cli", *args],
            cwd=Path(__file__).resolve().parents[2],
            env=environment,
            check=False,
            capture_output=True,
            text=True,
        )

    return {
        "args": args,
        "invocation": _mcp_replay_invocation(outcome),
        "environment": _mcp_replay_environment(outcome),
        "exit_code": process.returncode,
        "stdout": process.stdout.replace("\r\n", "\n"),
        "stderr": process.stderr.replace("\r\n", "\n"),
    }


def _mcp_replay_environment(outcome: str) -> dict[str, str]:
    return {
        "HIERONYMUS_DATA_ROOT": "<PATH>",
        _MCP_REPLAY_OUTCOME_ENV: outcome,
    }


def _mcp_replay_invocation(outcome: str) -> list[str]:
    return [
        "<PYTHON>",
        "-m",
        "tools.compatibility.inventory_cli",
        _MCP_REPLAY_OPTION,
        outcome,
    ]


def _write_json_fixture(repo_root: Path, reference: str, payload: dict[str, object]) -> None:
    destination = repo_root / reference
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


class _McpReplayServer:
    def __init__(self, outcome: str) -> None:
        self.outcome = outcome

    def run(self, *, transport: str) -> None:
        if transport != "stdio":
            raise AssertionError(f"unexpected MCP transport: {transport}")
        if self.outcome == "failure":
            sys.stderr.write("synthetic MCP startup failure\n")
            raise SystemExit(1)


def _replay_mcp_entrypoint(outcome: str) -> int:
    configured_outcome = os.environ.get(_MCP_REPLAY_OUTCOME_ENV)
    if configured_outcome != outcome:
        raise RuntimeError(
            f"MCP replay environment mismatch: expected {outcome}, got {configured_outcome}"
        )

    from hieronymus import mcp_server

    shipping_server = mcp_server.server
    mcp_server.server = _McpReplayServer(outcome)
    try:
        mcp_server.main()
    finally:
        mcp_server.server = shipping_server
    return 0


def _write_contract_fixtures(repo_root: Path, snapshot: dict[str, object]) -> None:
    records: list[dict[str, object]] = []
    script_records = snapshot["script_contracts"]
    command_groups = snapshot["commands"]
    assert isinstance(script_records, dict)
    assert isinstance(command_groups, dict)
    records.extend(record for record in script_records.values() if isinstance(record, dict))
    for rows in command_groups.values():
        assert isinstance(rows, list)
        records.extend(row for row in rows if isinstance(row, dict))

    for record in records:
        fixture_contract = str(record["fixture_contract"])
        payload = {
            "contract_id": record["contract_id"],
            "success": {
                "args": record["success_args"],
                "fixture": record["success_fixture"],
                "exit_code": 0,
            },
            "failure": {
                "args": record["failure_args"],
                "fixture": record["failure_fixture"],
                "exit_code": record["exit_behavior"].get(
                    "usage_error", record["exit_behavior"].get("failure")
                ),
            },
        }
        if "success_environment" in record:
            payload["success"]["invocation"] = record["success_invocation"]
            payload["failure"]["invocation"] = record["failure_invocation"]
            payload["success"]["environment"] = record["success_environment"]
            payload["failure"]["environment"] = record["failure_environment"]
        destination = repo_root / fixture_contract
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(
            json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )


def write_snapshot(repo_root: Path) -> dict[str, object]:
    """Write the canonical snapshot and fixture set below *repo_root*."""
    snapshot = snapshot_cli(repo_root)
    _write_click_fixtures(repo_root, snapshot)
    _write_non_click_entrypoint_fixtures(repo_root, snapshot)
    _write_contract_fixtures(repo_root, snapshot)
    destination = repo_root / "compatibility/snapshots/cli.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(
        json.dumps(snapshot, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return snapshot


def _write_manifest_entries(repo_root: Path, snapshot: dict[str, object]) -> None:
    manifest_path = repo_root / "compatibility/manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    retained_contracts = [
        contract for contract in manifest["contracts"] if contract.get("surface") != "cli"
    ]
    retained_contracts.extend(_manifest_contracts(snapshot))
    manifest["contracts"] = sorted(retained_contracts, key=lambda contract: contract["id"])
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=False) + "\n",
        encoding="utf-8",
    )


def _manifest_contracts(snapshot: dict[str, object]) -> list[dict[str, object]]:
    contracts: list[dict[str, object]] = []
    script_records = snapshot["script_contracts"]
    command_groups = snapshot["commands"]
    assert isinstance(script_records, dict)
    assert isinstance(command_groups, dict)

    for record in script_records.values():
        assert isinstance(record, dict)
        contract_id = str(record["contract_id"])
        entry_point = str(record["entry_point"])
        contracts.append(
            _manifest_contract(
                contract_id=contract_id,
                python_entry_point=entry_point,
                fixture=str(record["fixture_contract"]),
                tests=_owning_tests(contract_id),
            )
        )

    for script_name, rows in command_groups.items():
        entry_point = snapshot["scripts"][script_name]
        assert isinstance(rows, list)
        for record in rows:
            assert isinstance(record, dict)
            contract_id = str(record["contract_id"])
            contracts.append(
                _manifest_contract(
                    contract_id=contract_id,
                    python_entry_point=entry_point,
                    fixture=str(record["fixture_contract"]),
                    tests=_owning_tests(contract_id),
                )
            )
    return contracts


def _manifest_contract(
    *,
    contract_id: str,
    python_entry_point: str,
    fixture: str,
    tests: list[str],
) -> dict[str, object]:
    rust_test_name = contract_id.replace(".", "_").replace("-", "_")
    return {
        "id": contract_id,
        "surface": "cli",
        "acceptance_owner": "Pavel Obruchnikov <me@inkyquill.net>",
        "technical_owner": "distribution-cutover",
        "python_entry_point": python_entry_point,
        "tests": ["tests/compatibility/test_cli_inventory.py", *tests],
        "fixture": fixture,
        "rust_test_target": f"crates/hiero-cli/tests/cli_contract.rs::{rust_test_name}",
        "disposition": "preserve",
    }


def _owning_tests(contract_id: str) -> list[str]:
    if "hieronymus-agent-hook" in contract_id:
        return ["tests/test_agent_hooks.py"]
    if contract_id == "cli.script.hieronymus-mcp":
        return ["tests/test_mcp_server.py"]
    if ".rag" in contract_id:
        return ["tests/test_rag_cli.py"]
    if ".skills" in contract_id:
        return ["tests/test_cli_project_skills.py"]
    if contract_id.endswith(".admin"):
        return ["tests/test_admin_cli.py"]
    command_name = contract_id.rsplit(".", maxsplit=1)[-1]
    if (
        command_name
        in {
            "config",
            "doctor",
            "help",
            "install",
            "restart",
            "status",
            "stop",
            "update",
        }
        or contract_id == "cli.script.hiero"
    ):
        return ["tests/test_cli_service.py"]
    return ["tests/test_cli.py"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="write the checked-in snapshot")
    parser.add_argument(
        _MCP_REPLAY_OPTION,
        choices=("success", "failure"),
        help=argparse.SUPPRESS,
    )
    args = parser.parse_args()
    if args.replay_mcp_entrypoint is not None:
        return _replay_mcp_entrypoint(args.replay_mcp_entrypoint)
    repo_root = Path(__file__).resolve().parents[2]
    snapshot = write_snapshot(repo_root) if args.write else snapshot_cli(repo_root)
    if args.write:
        _write_manifest_entries(repo_root, snapshot)
    if not args.write:
        print(json.dumps(snapshot, ensure_ascii=False, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
