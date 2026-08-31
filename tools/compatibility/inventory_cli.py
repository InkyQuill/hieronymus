"""Generate a deterministic inventory of installed CLI entry points."""

from __future__ import annotations

import argparse
import copy
import json
import os
import re
import selectors
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import click
from click.testing import CliRunner
from mcp import types as mcp_types

from hieronymus.agent_hooks import main as agent_hook_main
from hieronymus.cli import main as cli_main
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.release import UpdateStatus
from hieronymus.workspace import WorkspaceStore

_CLICK_EXIT_BEHAVIOR = {
    "success": 0,
    "click_exception": 1,
    "usage_error": 2,
}
_ENTRY_POINT_GROUPS: dict[str, click.Group] = {
    "hieronymus.agent_hooks:main": agent_hook_main,
    "hieronymus.cli:main": cli_main,
}
_MCP_ENTRY_POINT = "hieronymus.mcp_server:main"


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
        "behavior_fixture": _behavior_fixture_path(f"{contract_prefix}.{contract_suffix}"),
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
        "behavior_fixture": _behavior_fixture_path(contract_id),
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
                "success_args": [],
                "failure_args": [],
                "success_invocation": _mcp_replay_invocation(),
                "failure_invocation": _mcp_replay_invocation(),
                "success_environment": _mcp_replay_environment(),
                "failure_environment": _mcp_replay_environment(),
                "success_stdin": _mcp_replay_stdin("success"),
                "failure_stdin": _mcp_replay_stdin("failure"),
                "exit_behavior": {"success": 0, "failure": 0},
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


def _behavior_fixture_path(contract_id: str) -> str:
    filename = contract_id.replace(".", "_").replace("-", "_")
    return f"compatibility/fixtures/cli/behavior/{filename}.json"


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


class _FixtureServiceManager:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config

    def status(self) -> dict[str, object]:
        return {"running": False, "data_root": str(self.config.data_root)}

    def stop(self) -> dict[str, object]:
        return self.status()

    def restart(self) -> dict[str, object]:
        return {"status": self.status(), "started": True}

    def ensure_running(self) -> dict[str, object]:
        return {"status": self.status(), "started": False}


@contextmanager
def _bounded_cli_dependencies(root: Path):
    import hieronymus.cli as shipping_cli

    originals = {
        "ServiceManager": shipping_cli.ServiceManager,
        "_launch_web_console": shipping_cli._launch_web_console,
        "agent_install_candidates": shipping_cli.agent_install_candidates,
        "check_update": shipping_cli.check_update,
        "run_update": shipping_cli.run_update,
        "resolve_provider": shipping_cli.resolve_provider,
        "DreamService": shipping_cli.DreamService,
        "Doctor": shipping_cli.Doctor,
    }
    status = UpdateStatus(
        current_version="0.7.0",
        latest_version="0.7.0",
        latest_tag="v0.7.0",
        current_revision=None,
        latest_revision=None,
        update_available=False,
        managed_checkout=root / "managed-checkout",
        managed_install=False,
        target="latest",
    )

    class FixtureDreamService:
        def __init__(self, *_args: object, **_kwargs: object) -> None:
            pass

        def run_all(self, **_kwargs: object) -> SimpleNamespace:
            return SimpleNamespace(
                cycle_id=1,
                status="completed",
                provider="synthetic",
                input_count=0,
                created_crystal_count=0,
                proposal_count=0,
                error=None,
            )

    class FixtureDoctor:
        def __init__(self, _config: HieronymusConfig) -> None:
            pass

        def run(self, *, autofix: bool = False) -> dict[str, list[object]]:
            del autofix
            return {"info": [], "autofixed": [], "warnings": [], "errors": []}

    shipping_cli.ServiceManager = _FixtureServiceManager
    shipping_cli._launch_web_console = lambda route, *, config: click.echo(
        f"Open synthetic web console: {route}"
    )
    shipping_cli.agent_install_candidates = lambda _config: []
    shipping_cli.check_update = lambda **_kwargs: status
    shipping_cli.run_update = lambda **_kwargs: status

    def fixture_provider(_config: HieronymusConfig, provider: str | None) -> object:
        if provider == "unknown-provider":
            raise ValueError("unknown dream provider: unknown-provider")
        return object()

    shipping_cli.resolve_provider = fixture_provider
    shipping_cli.DreamService = FixtureDreamService
    shipping_cli.Doctor = FixtureDoctor
    try:
        yield
    finally:
        for name, value in originals.items():
            setattr(shipping_cli, name, value)


def _prepare_cli_case(path: str, root: Path) -> dict[str, str]:
    config = HieronymusConfig(data_root=root / "data")
    source = root / "source.txt"
    translated = root / "translated.txt"
    source.write_text("ユン walked home.\n", encoding="utf-8")
    translated.write_text("Yun walked home.\n", encoding="utf-8")
    substitutions = {"SOURCE": str(source), "TRANSLATED": str(translated)}
    database_commands = {
        "propose-term",
        "validate",
        "remember",
        "rag",
        "rag import",
        "rag search",
        "session-start",
        "session-complete",
        "remember-short",
        "recall",
        "feedback",
    }
    if path in database_commands:
        Registry(config).create_series(
            slug="synthetic-series",
            title="Synthetic Series",
            source_language="ja",
            target_language="en",
        )
    if path in {"session-complete", "remember-short", "recall"}:
        context = TranslationContext(
            series_slug="synthetic-series",
            source_language="ja",
            target_language="en",
            task_type="translation",
        )
        WorkspaceStore(config).start_session(context)
    if path == "feedback":
        context = TranslationContext(
            series_slug="synthetic-series",
            source_language="ja",
            target_language="en",
            task_type="translation",
        )
        CrystalStore(config).add_crystal(
            context,
            crystal_type="rule",
            text="Use Sense, not Feeling.",
        )
    return substitutions


def _success_args(path: str, substitutions: dict[str, str]) -> list[str]:
    cases = {
        "admin": [],
        "config": [],
        "doctor": [],
        "dream": [],
        "feedback": ["1", "--event", "confirmed_by_user", "--role", "user"],
        "help": [],
        "init-series": ["synthetic-series", "--title", "Synthetic Series"],
        "install": ["list"],
        "propose-term": [
            "synthetic-series",
            "--category",
            "character",
            "--source",
            "ユン",
            "--translation",
            "Yun",
        ],
        "rag": ["search", "synthetic-series", "Sense"],
        "rag import": ["synthetic-series", substitutions["SOURCE"]],
        "rag search": ["synthetic-series", "Sense"],
        "recall": [
            "1",
            "--series",
            "synthetic-series",
            "--query",
            "Sense",
            "--source-language",
            "ja",
            "--target-language",
            "en",
            "--task-type",
            "translation",
        ],
        "remember": [
            "synthetic-series",
            "--kind",
            "correction",
            "--text",
            "Use Sense.",
        ],
        "remember-short": [
            "1",
            "--role",
            "agent",
            "--kind",
            "correction",
            "--text",
            "Use Sense.",
        ],
        "restart": [],
        "session-complete": ["1"],
        "session-start": ["synthetic-series", "--task-type", "translation"],
        "skills": ["install", "--target", "agents", "--dry-run"],
        "skills install": ["--target", "agents", "--dry-run"],
        "skills uninstall": ["--target", "agents", "--dry-run"],
        "status": [],
        "stop": [],
        "update": ["--check"],
        "validate": [
            "synthetic-series",
            "--raw-file",
            substitutions["SOURCE"],
            "--translated-file",
            substitutions["TRANSLATED"],
        ],
    }
    return list(cases[path])


def _semantic_failure_args(path: str, substitutions: dict[str, str]) -> list[str]:
    cases = {
        "help": ["unexpected-argument"],
        "install": ["unknown-agent"],
        "dream": ["--provider", "unknown-provider"],
        "init-series": ["invalid/slug", "--title", "Invalid"],
        "propose-term": [
            "unknown-series",
            "--category",
            "character",
            "--source",
            "ユン",
            "--translation",
            "Yun",
        ],
        "rag": ["search", "unknown-series", "Sense"],
        "rag import": ["unknown-series", substitutions["SOURCE"]],
        "rag search": ["unknown-series", "Sense"],
        "recall": [
            "999",
            "--series",
            "synthetic-series",
            "--query",
            "Sense",
            "--source-language",
            "ja",
            "--target-language",
            "en",
            "--task-type",
            "translation",
        ],
        "remember": ["unknown-series", "--kind", "correction", "--text", "Sense"],
        "remember-short": [
            "999",
            "--role",
            "agent",
            "--kind",
            "correction",
            "--text",
            "Sense",
        ],
        "session-complete": ["999"],
        "session-start": ["unknown-series", "--task-type", "translation"],
        "skills": ["install"],
        "skills install": [],
        "skills uninstall": [],
        "validate": [
            "unknown-series",
            "--raw-file",
            substitutions["SOURCE"],
            "--translated-file",
            substitutions["TRANSLATED"],
        ],
        "feedback": ["999", "--event", "confirmed_by_user", "--role", "user"],
    }
    return list(cases.get(path, []))


def _normalize_cli_value(value: object, root: Path) -> object:
    if isinstance(value, str):
        return value.replace(str(root), "<SYNTHETIC_ROOT>")
    if isinstance(value, list):
        return [_normalize_cli_value(item, root) for item in value]
    if isinstance(value, dict):
        return {key: _normalize_cli_value(item, root) for key, item in value.items()}
    return value


def _invoke_click_behavior(
    command: click.Group,
    *,
    script_name: str,
    path: str,
    args: list[str],
    root: Path,
    json_format: bool,
    force_invalid_root: bool = False,
) -> dict[str, object]:
    data_root = root / "data"
    if force_invalid_root:
        data_root.write_text("not a directory\n", encoding="utf-8")
    invocation_args = list(args)
    if command is cli_main:
        invocation_args = ["--data-root", str(data_root), *path.split(), *invocation_args]
    else:
        invocation_args = [*path.split(), *invocation_args]
    if json_format:
        invocation_args.append("--json")
    environment = {"HIERONYMUS_DATA_ROOT": str(data_root)}
    workspace = root / "workspace"
    workspace.mkdir(exist_ok=True)
    previous_cwd = Path.cwd()
    try:
        os.chdir(workspace)
        with _bounded_cli_dependencies(root):
            result = CliRunner(mix_stderr=False).invoke(
                command,
                invocation_args,
                prog_name=script_name,
                env=environment,
            )
    finally:
        os.chdir(previous_cwd)
    payload = {
        "args": args + (["--json"] if json_format else []),
        "invocation": [*_canonical_invocation(script_name), *path.split()],
        "format": "json" if json_format else "human",
        "basis": "shipping-callback",
        "exit_code": result.exit_code,
        "stdout": result.stdout.replace("\r\n", "\n"),
        "stderr": result.stderr.replace("\r\n", "\n"),
    }
    normalized = _normalize_cli_value(payload, root)
    assert isinstance(normalized, dict)
    return normalized


def _click_behavior_fixture(script_name: str, record: dict[str, object]) -> dict[str, object]:
    entry_point = str(record.get("entry_point", ""))
    command = _ENTRY_POINT_GROUPS.get(entry_point)
    if command is None:
        command = _ENTRY_POINT_GROUPS[
            str(_installed_scripts(Path(__file__).parents[2])[script_name])
        ]
    path = str(record.get("path", ""))
    cases: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory(prefix="hieronymus-cli-behavior-") as directory:
        root = Path(directory)
        substitutions = _prepare_cli_case(path, root)
        replay_path = path
        if command is agent_hook_main and not replay_path:
            replay_path = "session-end"
        if command is agent_hook_main:
            workspace = root / "workspace"
            workspace.mkdir()
            success_args = ["--cwd", str(workspace)] if replay_path == "session-start" else []
        else:
            success_args = _success_args(path, substitutions) if path else []
        human = _invoke_click_behavior(
            command,
            script_name=script_name,
            path=replay_path,
            args=success_args,
            root=root,
            json_format=False,
        )
        human.update({"id": "human-success", "expected": "success"})
        cases.append(human)
    parameters = record.get("parameters", [])
    assert isinstance(parameters, list)
    supports_json = any(
        isinstance(parameter, dict) and parameter.get("name") in {"as_json", "json_output"}
        for parameter in parameters
    )
    if not path and command is cli_main:
        supports_json = True
    if command is agent_hook_main:
        supports_json = True
    if supports_json:
        with tempfile.TemporaryDirectory(prefix="hieronymus-cli-behavior-") as directory:
            root = Path(directory)
            substitutions = _prepare_cli_case(path or "status", root)
            json_path = path or ("session-end" if command is agent_hook_main else "status")
            if command is agent_hook_main:
                workspace = root / "workspace"
                workspace.mkdir()
                json_args = ["--cwd", str(workspace)] if json_path == "session-start" else []
            else:
                json_args = _success_args(json_path, substitutions)
            machine = _invoke_click_behavior(
                command,
                script_name=script_name,
                path=json_path,
                args=json_args,
                root=root,
                json_format=True,
            )
            machine.update({"id": "json-success", "expected": "success"})
            cases.append(machine)
    with tempfile.TemporaryDirectory(prefix="hieronymus-cli-behavior-") as directory:
        root = Path(directory)
        substitutions = _prepare_cli_case(path, root)
        failure_path = path
        if command is agent_hook_main:
            failure_path = path or "session-end"
            if failure_path == "session-start":
                invalid_cwd = root / "not-a-directory"
                invalid_cwd.write_text("file\n", encoding="utf-8")
                failure_args = ["--cwd", str(invalid_cwd)]
            else:
                failure_args = ["unexpected-argument"]
        else:
            failure_args = _semantic_failure_args(path, substitutions)
        force_invalid_root = not failure_args and command is cli_main
        failure = _invoke_click_behavior(
            command,
            script_name=script_name,
            path=failure_path,
            args=failure_args,
            root=root,
            json_format=False,
            force_invalid_root=force_invalid_root,
        )
        failure.update({"id": "semantic-failure", "expected": "semantic-failure"})
        cases.append(failure)
    return {"contract_id": record["contract_id"], "cases": cases}


def _write_behavior_fixtures(repo_root: Path, snapshot: dict[str, object]) -> None:
    scripts = snapshot["script_contracts"]
    commands = snapshot["commands"]
    assert isinstance(scripts, dict)
    assert isinstance(commands, dict)
    for script_name, record in scripts.items():
        assert isinstance(record, dict)
        if record.get("kind") in {"command", "group"}:
            _write_json_fixture(
                repo_root,
                str(record["behavior_fixture"]),
                _click_behavior_fixture(str(script_name), record),
            )
    for script_name, rows in commands.items():
        assert isinstance(rows, list)
        for record in rows:
            assert isinstance(record, dict)
            _write_json_fixture(
                repo_root,
                str(record["behavior_fixture"]),
                _click_behavior_fixture(str(script_name), record),
            )


def _write_non_click_entrypoint_fixtures(repo_root: Path, snapshot: dict[str, object]) -> None:
    scripts = snapshot["script_contracts"]
    assert isinstance(scripts, dict)
    contract = scripts["hieronymus-mcp"]
    assert isinstance(contract, dict)
    success = replay_mcp_entrypoint_case("success")
    failure = replay_mcp_entrypoint_case("failure")
    if (success["exit_code"], failure["exit_code"]) != (0, 0):
        raise RuntimeError("unexpected replayed MCP entrypoint exit behavior")
    _write_json_fixture(repo_root, str(contract["success_fixture"]), success)
    _write_json_fixture(repo_root, str(contract["failure_fixture"]), failure)
    _write_json_fixture(
        repo_root,
        str(contract["behavior_fixture"]),
        {
            "contract_id": contract["contract_id"],
            "cases": [
                {**success, "id": "stdio-success", "expected": "success"},
                {**failure, "id": "stdio-failure", "expected": "semantic-failure"},
            ],
        },
    )


def replay_mcp_entrypoint_case(outcome: str) -> dict[str, object]:
    """Replay the shipping stdio entrypoint with bounded MCP NDJSON input."""
    if outcome not in {"success", "failure"}:
        raise ValueError(f"unknown MCP replay outcome: {outcome}")

    stdin = _mcp_replay_stdin(outcome)
    with tempfile.TemporaryDirectory(prefix="hieronymus-mcp-compat-") as data_root:
        environment = os.environ.copy()
        environment.update(_mcp_process_environment(data_root))
        return_code, stdout, stderr = _run_shipping_mcp_process(
            stdin,
            environment,
            expected_responses=2 if outcome == "success" else 1,
        )

    return {
        "boundary": "shipping-stdio-entrypoint",
        "entry_point": _MCP_ENTRY_POINT,
        "args": [],
        "invocation": _mcp_replay_invocation(),
        "environment": _mcp_replay_environment(),
        "stdin": stdin,
        "exit_code": return_code,
        "stdout": _normalize_mcp_stdout(stdout),
        "stdout_normalization": "initialize-result.serverInfo.version-only",
        "stderr": _normalize_mcp_stderr(stderr),
        "stderr_normalization": "rich-timestamp-and-source-location-redacted",
        "process_group_drained": True,
    }


def _run_shipping_mcp_process(
    stdin: str,
    environment: dict[str, str],
    *,
    expected_responses: int,
) -> tuple[int, str, str]:
    """Read the bounded response set before closing the shipping server's stdin."""
    process = subprocess.Popen(
        [str(_shipping_mcp_entrypoint())],
        cwd=Path(__file__).resolve().parents[2],
        env=environment,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    assert process.stdin is not None
    assert process.stdout is not None
    deadline = time.monotonic() + 30
    output = bytearray()
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    try:
        process.stdin.write(stdin.encode())
        process.stdin.flush()
        while output.count(b"\n") < expected_responses:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise subprocess.TimeoutExpired("hieronymus-mcp", 30)
            if not selector.select(remaining):
                raise subprocess.TimeoutExpired("hieronymus-mcp", 30)
            chunk = os.read(process.stdout.fileno(), 65536)
            if not chunk:
                break
            output.extend(chunk)
        process.stdin.close()
        process.stdin = None
        remaining = max(deadline - time.monotonic(), 0.1)
        trailing_stdout, stderr = process.communicate(timeout=remaining)
        output.extend(trailing_stdout)
    except BaseException:
        _terminate_process_group(process)
        raise
    finally:
        selector.close()

    if output.count(b"\n") != expected_responses:
        raise RuntimeError(
            "shipping hieronymus-mcp returned "
            f"{output.count(b'\n')} responses; expected {expected_responses}"
        )
    _assert_process_group_drained(process.pid)
    return process.returncode, output.decode(), stderr.decode()


def _terminate_process_group(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
    try:
        process.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate(timeout=5)


def _assert_process_group_drained(process_group: int) -> None:
    try:
        os.killpg(process_group, 0)
    except ProcessLookupError:
        return
    os.killpg(process_group, signal.SIGTERM)
    raise RuntimeError("shipping hieronymus-mcp left a descendant process running")


def _shipping_mcp_entrypoint() -> Path:
    entrypoint = Path(sys.executable).with_name("hieronymus-mcp")
    if not entrypoint.is_file():
        raise RuntimeError(f"installed hieronymus-mcp entrypoint is missing: {entrypoint}")
    return entrypoint


def _mcp_process_environment(data_root: str) -> dict[str, str]:
    return {
        "HIERONYMUS_DATA_ROOT": data_root,
        "COLUMNS": "500",
        "LINES": "100",
        "TERM": "dumb",
        "NO_COLOR": "1",
        "FORCE_COLOR": "0",
    }


def _mcp_replay_environment() -> dict[str, str]:
    return _mcp_process_environment("<PATH>")


def _mcp_replay_invocation() -> list[str]:
    return ["hieronymus-mcp"]


def _mcp_replay_stdin(outcome: str) -> str:
    if outcome == "failure":
        requests = [{"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}]
    elif outcome == "success":
        requests = [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": mcp_types.LATEST_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "compatibility-replay", "version": "1.0.0"},
                },
            },
            {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        ]
    else:
        raise ValueError(f"unknown MCP replay outcome: {outcome}")
    return "".join(
        json.dumps(request, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n"
        for request in requests
    )


def _normalize_mcp_stderr(stderr: str) -> str:
    normalized = []
    for line in stderr.replace("\r\n", "\n").splitlines():
        line = re.sub(r"^\[[^]]+\]\s+", "", line)
        line = re.sub(r"\s+[A-Za-z_][A-Za-z0-9_]*\.py:\d+\s*$", "", line)
        normalized.append(line.rstrip())
    return "\n".join(normalized) + ("\n" if normalized else "")


def _normalize_mcp_stdout(stdout: str) -> str:
    """Normalize only FastMCP's environment-derived initialize package version."""
    normalized = []
    for line in stdout.replace("\r\n", "\n").splitlines():
        message = json.loads(line)
        result = message.get("result") if isinstance(message, dict) else None
        server_info = result.get("serverInfo") if isinstance(result, dict) else None
        if isinstance(server_info, dict) and "protocolVersion" in result:
            version = server_info.get("version")
            if not isinstance(version, str) or not version:
                raise ValueError("MCP initialize response has no server package version")
            server_info["version"] = "<MCP_PACKAGE_VERSION>"
        normalized.append(json.dumps(message, separators=(",", ":"), ensure_ascii=False))
    return "\n".join(normalized) + ("\n" if normalized else "")


def _write_json_fixture(repo_root: Path, reference: str, payload: dict[str, object]) -> None:
    destination = repo_root / reference
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


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
            "behavior_fixture": record["behavior_fixture"],
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
    _write_behavior_fixtures(repo_root, snapshot)
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
        "last_python_release": "0.7.0",
        "first_rust_release": None,
        "implementation_status": "outstanding",
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
    args = parser.parse_args()
    repo_root = Path(__file__).resolve().parents[2]
    snapshot = write_snapshot(repo_root) if args.write else snapshot_cli(repo_root)
    if args.write:
        _write_manifest_entries(repo_root, snapshot)
    if not args.write:
        print(json.dumps(snapshot, ensure_ascii=False, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
