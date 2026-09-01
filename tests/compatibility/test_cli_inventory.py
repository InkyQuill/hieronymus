from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

from tools.compatibility import inventory_cli
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
        assert case["process_group_drained"] is True
        assert case["stdout_normalization"] == "initialize-result.serverInfo.version-only"
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
    assert success_responses[0]["result"]["serverInfo"] == {
        "name": "hieronymus",
        "version": "<MCP_PACKAGE_VERSION>",
    }
    assert success_responses[1]["id"] == 2
    assert len(success_responses[1]["result"]["tools"]) == 39
    protocol = json.loads(
        (ROOT / "compatibility/fixtures/mcp/protocol.json").read_text(encoding="utf-8")
    )
    assert success_responses[1] == protocol["current"]["tools_list"]["response"]

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


def test_mcp_entrypoint_normalizes_only_volatile_server_package_version() -> None:
    lock_before = (ROOT / "uv.lock").read_bytes()
    protocol = json.loads(
        (ROOT / "compatibility/fixtures/mcp/protocol.json").read_text(encoding="utf-8")
    )
    initialize = {
        "jsonrpc": "2.0",
        "id": 1,
        "result": json.loads(json.dumps(protocol["current"]["initialize"]["result"])),
    }
    initialize["result"]["serverInfo"]["version"] = "999.0.post-uv-sync"
    tools_list = protocol["current"]["tools_list"]["response"]
    raw_stdout = "".join(
        json.dumps(response, separators=(",", ":"), ensure_ascii=False) + "\n"
        for response in (initialize, tools_list)
    )
    normalized = inventory_cli._normalize_mcp_stdout(raw_stdout)
    responses = [json.loads(line) for line in normalized.splitlines()]

    assert responses[0]["result"]["serverInfo"] == {
        "name": "hieronymus",
        "version": "<MCP_PACKAGE_VERSION>",
    }
    assert responses[1] == tools_list
    assert (ROOT / "uv.lock").read_bytes() == lock_before


def test_shipping_mcp_replay_waits_for_complete_responses_before_closing_stdin() -> None:
    for _attempt in range(8):
        replay = replay_mcp_entrypoint_case("success")
        responses = [json.loads(line) for line in replay["stdout"].splitlines()]

        assert [response["id"] for response in responses] == [1, 2]
        assert len(responses[1]["result"]["tools"]) == 39


def test_owned_process_group_kills_stubborn_descendant_after_leader_exit(
    tmp_path: Path,
) -> None:
    command, child_pid_path, term_marker_path = _stubborn_descendant_command(tmp_path, "mismatch")

    with pytest.raises(RuntimeError, match="returned 1 responses; expected 2"):
        inventory_cli._run_owned_stdio_process(
            command,
            "",
            {},
            expected_responses=2,
            response_timeout=0.5,
            terminate_timeout=0.2,
            kill_timeout=1.0,
        )

    child_pid = int(child_pid_path.read_text(encoding="utf-8"))
    assert term_marker_path.read_text(encoding="utf-8") == "TERM"
    assert not Path(f"/proc/{child_pid}").exists()


def test_owned_process_group_drains_after_read_timeout(tmp_path: Path) -> None:
    command, child_pid_path, term_marker_path = _stubborn_descendant_command(tmp_path, "timeout")

    with pytest.raises(subprocess.TimeoutExpired):
        inventory_cli._run_owned_stdio_process(
            command,
            "",
            {},
            expected_responses=1,
            response_timeout=0.1,
            terminate_timeout=0.2,
            kill_timeout=1.0,
        )

    child_pid = int(child_pid_path.read_text(encoding="utf-8"))
    assert term_marker_path.read_text(encoding="utf-8") == "TERM"
    assert not Path(f"/proc/{child_pid}").exists()


def test_owned_process_group_preserves_original_error_when_cleanup_reports_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    command, child_pid_path, _term_marker_path = _stubborn_descendant_command(tmp_path, "mismatch")
    drain = inventory_cli._drain_owned_process_group

    def drain_then_report_failure(*args, **kwargs) -> None:
        drain(*args, **kwargs)
        raise RuntimeError("synthetic cleanup audit failure")

    monkeypatch.setattr(inventory_cli, "_drain_owned_process_group", drain_then_report_failure)

    with pytest.raises(RuntimeError, match="returned 1 responses; expected 2") as caught:
        inventory_cli._run_owned_stdio_process(
            command,
            "",
            {},
            expected_responses=2,
            response_timeout=0.5,
            terminate_timeout=0.2,
            kill_timeout=1.0,
        )

    assert isinstance(caught.value.__cause__, RuntimeError)
    assert str(caught.value.__cause__) == "synthetic cleanup audit failure"
    assert caught.value.__notes__ == [
        "owned process-group cleanup also failed: RuntimeError: synthetic cleanup audit failure"
    ]
    child_pid = int(child_pid_path.read_text(encoding="utf-8"))
    assert not Path(f"/proc/{child_pid}").exists()


def _stubborn_descendant_command(tmp_path: Path, mode: str) -> tuple[list[str], Path, Path]:
    script = tmp_path / "owned_process_group.py"
    child_pid_path = tmp_path / "child.pid"
    term_marker_path = tmp_path / "term.marker"
    script.write_text(
        """
import os
import signal
import sys
import time
from pathlib import Path

mode, child_pid_path, term_marker_path = sys.argv[1:]
ready_read, ready_write = os.pipe()
child_pid = os.fork()
if child_pid == 0:
    os.close(ready_read)

    def record_term(_signum, _frame):
        Path(term_marker_path).write_text("TERM", encoding="utf-8")

    signal.signal(signal.SIGTERM, record_term)
    Path(child_pid_path).write_text(str(os.getpid()), encoding="utf-8")
    os.write(ready_write, b"1")
    os.close(ready_write)
    devnull = os.open(os.devnull, os.O_RDWR)
    os.dup2(devnull, 0)
    os.dup2(devnull, 1)
    os.dup2(devnull, 2)
    while True:
        time.sleep(0.05)

os.close(ready_write)
os.read(ready_read, 1)
os.close(ready_read)
if mode == "mismatch":
    print('{"jsonrpc":"2.0","id":1,"result":{}}', flush=True)
    raise SystemExit(0)
time.sleep(60)
""".lstrip(),
        encoding="utf-8",
    )
    return (
        [sys.executable, str(script), mode, str(child_pid_path), str(term_marker_path)],
        child_pid_path,
        term_marker_path,
    )


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
