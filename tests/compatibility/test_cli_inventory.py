from __future__ import annotations

import json
from pathlib import Path

from tools.compatibility.inventory_cli import (
    replay_mcp_entrypoint_case,
    snapshot_cli,
    write_snapshot,
)
from tools.compatibility.model import load_manifest

ROOT = Path(__file__).resolve().parents[2]


def test_cli_snapshot_matches_shipping_commands() -> None:
    actual = snapshot_cli(ROOT)
    expected = json.loads((ROOT / "compatibility/snapshots/cli.json").read_text(encoding="utf-8"))

    assert actual == expected
    assert set(actual["scripts"]) == {
        "hiero",
        "hieronymus",
        "hieronymus-agent-hook",
        "hieronymus-mcp",
    }
    assert "session-start" in actual["command_paths"]["hieronymus-agent-hook"]
    assert "session-end" in actual["command_paths"]["hieronymus-agent-hook"]
    assert str(ROOT) not in json.dumps(actual)


def test_every_public_cli_contract_has_success_failure_fixtures_and_manifest_entry() -> None:
    snapshot = snapshot_cli(ROOT)
    records = _contract_records(snapshot)
    manifest = load_manifest(ROOT / "compatibility/manifest.json")
    cli_contracts = {
        contract.id: contract for contract in manifest.contracts if contract.surface == "cli"
    }

    assert set(cli_contracts) == {str(record["contract_id"]) for record in records}
    fixture_pairs: set[tuple[str, str]] = set()
    for record in records:
        success_fixture = str(record["success_fixture"])
        failure_fixture = str(record["failure_fixture"])
        fixture_pair = (success_fixture, failure_fixture)
        fixture_pairs.add(fixture_pair)

        assert success_fixture != failure_fixture
        assert (ROOT / success_fixture).is_file()
        assert (ROOT / failure_fixture).is_file()
        assert cli_contracts[str(record["contract_id"])].fixture == record["fixture_contract"]
        contract_fixture = json.loads(
            (ROOT / str(record["fixture_contract"])).read_text(encoding="utf-8")
        )
        assert contract_fixture["success"]["fixture"] == success_fixture
        assert contract_fixture["failure"]["fixture"] == failure_fixture

    assert len(fixture_pairs) == len(records)


def test_each_installed_click_command_path_has_its_own_contract() -> None:
    snapshot = snapshot_cli(ROOT)
    commands = snapshot["commands"]

    assert set(commands) == {"hiero", "hieronymus", "hieronymus-agent-hook"}
    assert sum(len(rows) for rows in commands.values()) == sum(
        len(paths) for paths in snapshot["command_paths"].values()
    )


def test_snapshot_generation_never_executes_click_callbacks(tmp_path: Path, monkeypatch) -> None:
    (tmp_path / "pyproject.toml").write_text(
        (ROOT / "pyproject.toml").read_text(encoding="utf-8"),
        encoding="utf-8",
    )

    def forbidden_callback(*_args: object, **_kwargs: object) -> None:
        raise AssertionError("shipping callback executed")

    monkeypatch.setattr("tools.compatibility.inventory_cli.cli_main.callback", forbidden_callback)
    monkeypatch.setattr(
        "tools.compatibility.inventory_cli.agent_hook_main.callback", forbidden_callback
    )

    generated = write_snapshot(tmp_path)

    assert generated == snapshot_cli(tmp_path)
    assert not list(tmp_path.rglob("hieronymus.sqlite"))
    for record in _contract_records(generated):
        for field in ("success_fixture", "failure_fixture", "fixture_contract"):
            reference = Path(str(record[field]))
            assert (tmp_path / reference).read_bytes() == (ROOT / reference).read_bytes()


def test_filesystem_defaults_are_normalized() -> None:
    snapshot = snapshot_cli(ROOT)
    hook_records = snapshot["commands"]["hieronymus-agent-hook"]
    session_start = next(row for row in hook_records if row["path"] == "session-start")
    cwd = next(parameter for parameter in session_start["parameters"] if parameter["name"] == "cwd")

    assert cwd["default"] == "<PATH>"


def test_legacy_entrypoints_record_canonical_rust_routes() -> None:
    snapshot = snapshot_cli(ROOT)

    assert snapshot["script_contracts"]["hieronymus"]["canonical_rust_invocation"] == ["hiero"]
    assert snapshot["script_contracts"]["hieronymus-mcp"]["canonical_rust_invocation"] == [
        "hiero",
        "mcp",
    ]
    hook_records = snapshot["commands"]["hieronymus-agent-hook"]
    assert {tuple(record["canonical_rust_invocation"]) for record in hook_records} == {
        ("hiero", "agent-hook", "session-end"),
        ("hiero", "agent-hook", "session-start"),
    }


def test_mcp_entrypoint_fixtures_replay_distinct_success_and_failure() -> None:
    snapshot = snapshot_cli(ROOT)
    record = snapshot["script_contracts"]["hieronymus-mcp"]
    expected_success = {
        "args": ["--replay-mcp-entrypoint", "success"],
        "invocation": [
            "<PYTHON>",
            "-m",
            "tools.compatibility.inventory_cli",
            "--replay-mcp-entrypoint",
            "success",
        ],
        "environment": {
            "HIERONYMUS_DATA_ROOT": "<PATH>",
            "HIERONYMUS_MCP_FIXTURE_OUTCOME": "success",
        },
        "exit_code": 0,
        "stdout": "",
        "stderr": "",
    }
    expected_failure = {
        "args": ["--replay-mcp-entrypoint", "failure"],
        "invocation": [
            "<PYTHON>",
            "-m",
            "tools.compatibility.inventory_cli",
            "--replay-mcp-entrypoint",
            "failure",
        ],
        "environment": {
            "HIERONYMUS_DATA_ROOT": "<PATH>",
            "HIERONYMUS_MCP_FIXTURE_OUTCOME": "failure",
        },
        "exit_code": 1,
        "stdout": "",
        "stderr": "synthetic MCP startup failure\n",
    }

    assert record["success_args"] != record["failure_args"]
    assert json.loads((ROOT / record["success_fixture"]).read_text(encoding="utf-8")) == (
        expected_success
    )
    assert json.loads((ROOT / record["failure_fixture"]).read_text(encoding="utf-8")) == (
        expected_failure
    )
    assert replay_mcp_entrypoint_case("success") == expected_success
    assert replay_mcp_entrypoint_case("failure") == expected_failure


def _contract_records(snapshot: dict[str, object]) -> list[dict[str, object]]:
    script_contracts = snapshot["script_contracts"]
    commands = snapshot["commands"]
    assert isinstance(script_contracts, dict)
    assert isinstance(commands, dict)
    records = [record for record in script_contracts.values() if isinstance(record, dict)]
    for rows in commands.values():
        assert isinstance(rows, list)
        records.extend(row for row in rows if isinstance(row, dict))
    return records
