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


def test_snapshot_generation_replays_callbacks_only_below_synthetic_root(tmp_path: Path) -> None:
    (tmp_path / "pyproject.toml").write_text(
        (ROOT / "pyproject.toml").read_text(encoding="utf-8"),
        encoding="utf-8",
    )

    generated = write_snapshot(tmp_path)

    assert generated == snapshot_cli(tmp_path)
    for record in _contract_records(generated):
        for field in (
            "success_fixture",
            "failure_fixture",
            "behavior_fixture",
            "fixture_contract",
        ):
            reference = Path(str(record[field]))
            assert (tmp_path / reference).read_bytes() == (ROOT / reference).read_bytes()


def test_every_click_command_has_real_behavior_successes_and_semantic_failure() -> None:
    snapshot = snapshot_cli(ROOT)
    click_records = [
        record
        for record in _contract_records(snapshot)
        if record.get("kind") in {"command", "group"}
    ]

    for record in click_records:
        behavior = json.loads((ROOT / str(record["behavior_fixture"])).read_text(encoding="utf-8"))
        cases = behavior["cases"]
        successes = [case for case in cases if case["expected"] == "success"]
        failures = [case for case in cases if case["expected"] == "semantic-failure"]

        assert successes, record["contract_id"]
        assert failures, record["contract_id"]
        assert all(case["basis"] == "shipping-callback" for case in cases)
        assert all(case["exit_code"] == 0 for case in successes)
        assert all("--help" not in case["args"] for case in successes)
        assert all("--compat-invalid-option" not in case["args"] for case in failures)
        assert all(case["exit_code"] != 0 for case in failures)
        if any(
            parameter["name"] in {"as_json", "json_output"} for parameter in record["parameters"]
        ):
            assert {case["format"] for case in successes} == {"human", "json"}


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


def test_mcp_entrypoint_fixtures_replay_shipping_stdio_success_and_protocol_failure() -> None:
    snapshot = snapshot_cli(ROOT)
    record = snapshot["script_contracts"]["hieronymus-mcp"]
    success = json.loads((ROOT / record["success_fixture"]).read_text(encoding="utf-8"))
    failure = json.loads((ROOT / record["failure_fixture"]).read_text(encoding="utf-8"))

    assert record["entry_point"] == "hieronymus.mcp_server:main"
    assert record["success_args"] == record["failure_args"] == []
    for case in (success, failure):
        assert case["args"] == []
        assert case["invocation"] == ["hieronymus-mcp"]
        assert case["entry_point"] == "hieronymus.mcp_server:main"
        assert case["boundary"] == "shipping-stdio-entrypoint"
        assert "tools.compatibility" not in json.dumps(case)
        assert "--replay-mcp-entrypoint" not in json.dumps(case)
        assert case["exit_code"] == 0

    success_requests = [json.loads(line) for line in success["stdin"].splitlines()]
    success_responses = [json.loads(line) for line in success["stdout"].splitlines()]
    assert [request["method"] for request in success_requests] == [
        "initialize",
        "notifications/initialized",
        "tools/list",
    ]
    assert success_responses[0]["id"] == 1
    assert success_responses[0]["result"]["protocolVersion"] == "2025-11-25"
    assert success_responses[1]["id"] == 2
    assert len(success_responses[1]["result"]["tools"]) == 39

    assert [json.loads(line) for line in failure["stdin"].splitlines()] == [
        {"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}
    ]
    assert [json.loads(line) for line in failure["stdout"].splitlines()] == [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32602,
                "message": "Invalid request parameters",
                "data": "",
            },
        }
    ]
    assert replay_mcp_entrypoint_case("success") == success
    assert replay_mcp_entrypoint_case("failure") == failure


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
