from __future__ import annotations

import json
import stat
import sys
import time
from pathlib import Path, PurePosixPath

import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.qualification.factories import write_fake_executable  # noqa: E402
from tools.qualification import run_mcp  # noqa: E402
from tools.qualification.model import FAILURE_CONSEQUENCES, REQUIRED_CRITERIA  # noqa: E402
from tools.qualification.process import ProcessReceipt, ToolRoots  # noqa: E402
from tools.qualification.projections import projection_issues  # noqa: E402
from tools.qualification.run_mcp import _live_process_context, run  # noqa: E402


def _typed_probe_payload(*, failed: tuple[str, ...] = ()) -> dict[str, object]:
    return {
        "schemaVersion": 1,
        "checks": {
            criterion: {
                "passed": criterion not in failed,
                "measurements": {"observed": 1},
            }
            for criterion in REQUIRED_CRITERIA["mcp-transport"]
        },
        "environment": {
            "rustc": "rustc 1.96.0 (measured)",
            "cargo": "cargo 1.96.0",
            "target": "x86_64-unknown-linux-gnu",
            "os": "linux",
            "kernel": "measured-kernel",
            "architecture": "x86_64",
        },
        "dependencies": [
            {
                "name": "rmcp",
                "version": "3.1.4",
                "source": "registry+https://github.com/rust-lang/crates.io-index",
                "checksum": "0" * 64,
                "features": ["server", "transport-io", "transport-streamable-http-server"],
            }
        ],
    }


def _write_typed_fake(tmp_path: Path, *, failed: tuple[str, ...] = ()) -> Path:
    executable = tmp_path / "typed-probe"
    payload = json.dumps(_typed_probe_payload(failed=failed), sort_keys=True)
    executable.write_text(
        f"#!{sys.executable}\n"
        "import pathlib, sys\n"
        f"pathlib.Path(sys.argv[2]).write_text({payload!r}, encoding='utf-8')\n",
        encoding="utf-8",
    )
    executable.chmod(0o700)
    return executable


def test_mcp_runner_consumes_corrected_oracle(tmp_path: Path) -> None:
    executable = _write_typed_fake(tmp_path)
    record = run(ROOT, tmp_path, executable=executable)
    assert "compatibility/authorities/mcp/2026-07-28/schema.json" in record.input_paths
    assert "compatibility/authorities/mcp/2026-07-28/schema.source.json" in record.input_paths
    assert "qualification/compatibility/mcp-transport.json" in record.input_paths
    assert "tools/qualification/projections.py" in record.input_paths
    assert "compatibility/manifest.json" not in record.input_paths
    assert "compatibility/snapshots/state.json" not in record.input_paths
    assert "compatibility/snapshots/mcp.json" in record.input_paths
    assert "compatibility/fixtures/mcp/protocol.json" in record.input_paths
    assert "compatibility/fixtures/http/route-cases.json" in record.input_paths
    assert "compatibility/fixtures/mcp/hieronymus_status/success.input.json" in record.input_paths
    assert len(record.input_paths) == 180
    assert (
        sum(
            path.startswith("compatibility/fixtures/mcp/")
            and path.endswith(
                (
                    "error.input.json",
                    "success.input.json",
                    "wire.error.json",
                    "wire.success.json",
                )
            )
            for path in record.input_paths
        )
        == 156
    )
    assert all(
        PurePosixPath(path).parts[3] != "tools"
        for path in record.input_paths
        if path.startswith("compatibility/fixtures/mcp/hieronymus_")
    )
    assert {item.criterion for item in record.evidence} == set(REQUIRED_CRITERIA["mcp-transport"])
    assert len(record.evidence) == 17
    assert projection_issues(ROOT)["mcp-transport"] == ()


def test_mcp_failure_preserves_adr_0015(tmp_path: Path) -> None:
    executable = _write_typed_fake(tmp_path, failed=("official-schema-envelopes",))
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "blocked"
    assert record.consequence == FAILURE_CONSEQUENCES["mcp-transport"]


def test_mcp_fake_runner_requires_closed_typed_check_artifact(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    contract_ids = tuple(
        sorted(
            {
                *run_mcp.MCP_EXACT_CONTRACT_IDS,
                *(f"mcp.tool.{name}" for name in run_mcp._MCP_TOOL_NAMES),
            }
        )
    )
    monkeypatch.setattr(run_mcp, "_validate_oracles", lambda _root: contract_ids)
    aggregate = write_fake_executable(tmp_path)
    with pytest.raises(ValueError, match="typed probe artifact"):
        run(ROOT, tmp_path / "aggregate", executable=aggregate)

    typed = _write_typed_fake(tmp_path)
    record = run(ROOT, tmp_path / "typed", executable=typed)
    assert record.decision == "qualified"
    assert {item.criterion for item in record.evidence} == set(REQUIRED_CRITERIA["mcp-transport"])


@pytest.mark.parametrize("criterion", REQUIRED_CRITERIA["mcp-transport"])
def test_mcp_each_observed_check_can_independently_block(
    tmp_path: Path, criterion: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    contract_ids = tuple(
        sorted(
            {
                *run_mcp.MCP_EXACT_CONTRACT_IDS,
                *(f"mcp.tool.{name}" for name in run_mcp._MCP_TOOL_NAMES),
            }
        )
    )
    monkeypatch.setattr(run_mcp, "_validate_oracles", lambda _root: contract_ids)
    executable = _write_typed_fake(tmp_path, failed=(criterion,))
    record = run(ROOT, tmp_path / criterion, executable=executable)
    evidence = {item.criterion: item.status for item in record.evidence}
    assert record.decision == "blocked"
    assert evidence[criterion] == "fail"
    assert sum(status == "fail" for status in evidence.values()) == 1


def test_mcp_typed_artifact_rejects_missing_extra_and_path_values(tmp_path: Path) -> None:
    for _name, mutate in (
        ("missing", lambda value: value["checks"].pop("registry-identity")),
        (
            "extra",
            lambda value: value["checks"].update(
                {"invented": {"passed": True, "measurements": {}}}
            ),
        ),
        (
            "path",
            lambda value: value["environment"].update(
                {"cargo": "/home/private/.cargo/bin/cargo 1.96.0"}
            ),
        ),
    ):
        payload = _typed_probe_payload()
        mutate(payload)
        with pytest.raises(ValueError, match="probe artifact"):
            run_mcp._probe_observations(payload)


def test_mcp_policy_directory_rejects_every_extra_entry(tmp_path: Path) -> None:
    root = tmp_path / "compatibility/fixtures/mcp"
    for tool in run_mcp._MCP_TOOL_NAMES:
        directory = root / tool
        directory.mkdir(parents=True)
        for leaf in run_mcp._MCP_TOOL_FIXTURE_LEAVES:
            (directory / leaf).write_text("{}", encoding="utf-8")
    (root / run_mcp._MCP_TOOL_NAMES[0] / "ignored.txt").write_text("extra", encoding="utf-8")
    with pytest.raises(ValueError, match="exactly four"):
        run_mcp._validate_policy_inventory(tmp_path)


def test_mcp_http_negotiations_are_derived_from_frozen_response_variants() -> None:
    target = json.loads(
        (ROOT / "compatibility/fixtures/mcp/protocol.json").read_text(encoding="utf-8")
    )["target"]
    assert run_mcp._http_negotiations(target, 0) == (
        ("application/json", "application/json; charset=utf-8"),
        ("text/event-stream", "text/event-stream"),
    )
    target["streamable_http"]["exchanges"][0]["responses"].reverse()
    with pytest.raises(ValueError, match="negotiation oracle"):
        run_mcp._http_negotiations(target, 0)


def test_mcp_projection_requires_exact_direct_child_success_fixture(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    projection = json.loads(
        (ROOT / "qualification/compatibility/mcp-transport.json").read_text(encoding="utf-8")
    )
    tool = next(item for item in projection["contracts"] if item["id"].startswith("mcp.tool."))
    tool["fixture"] = "compatibility/fixtures/mcp/tools/fabricated/success.input.json"
    monkeypatch.setattr(run_mcp, "projection_issues", lambda _root: {"mcp-transport": ()})
    monkeypatch.setattr(run_mcp, "_read_object", lambda _path: projection)
    with pytest.raises(ValueError, match="fixture"):
        run_mcp._mcp_projection(ROOT)


def test_mcp_live_context_discovers_before_sanitizing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    calls: list[object] = []
    roots = ToolRoots(
        *(
            tmp_path / name
            for name in (
                "cargo-home",
                "rustup-home",
                "cargo",
                "cargo-bin",
                "rustup",
                "rustup-bin",
            )
        )
    )
    safe_env = {"SAFE": "yes"}
    original_env = {"HOME": "/sensitive", "TOKEN": "do-not-copy"}

    def discover(candidate: object) -> ToolRoots:
        calls.append(("discover", candidate))
        return roots

    def sanitize(
        work_root: Path,
        *,
        cargo_offline: bool,
        tool_roots: ToolRoots,
        cargo_target_dir: Path,
    ) -> dict[str, str]:
        calls.append(
            (
                "sanitize",
                work_root,
                cargo_offline,
                tool_roots,
                cargo_target_dir,
            )
        )
        return safe_env

    monkeypatch.setattr(run_mcp, "discover_tool_roots", discover)
    monkeypatch.setattr(run_mcp, "safe_subprocess_env", sanitize)

    actual = _live_process_context(ROOT, tmp_path, original_env)

    target = ROOT / "qualification/.artifacts/cargo-target/mcp-transport"
    assert actual == (roots, target, safe_env)
    assert calls == [
        ("discover", original_env),
        ("sanitize", tmp_path, True, roots, target),
    ]


def test_mcp_live_reconciles_exact_owned_cargo_target_before_safe_environment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target = tmp_path / "cargo-target"
    target.mkdir(mode=0o755)
    monkeypatch.setattr(run_mcp, "_CARGO_TARGET", target)
    calls: list[int] = []

    def sanitize(*_args: object, **_kwargs: object) -> dict[str, str]:
        calls.append(stat.S_IMODE(target.stat().st_mode))
        return {"SAFE": "yes"}

    roots = ToolRoots(*(tmp_path / name for name in ("ch", "rh", "c", "cr", "r", "rr")))
    monkeypatch.setattr(run_mcp, "discover_tool_roots", lambda _env: roots)
    monkeypatch.setattr(run_mcp, "safe_subprocess_env", sanitize)

    run_mcp._live_process_context(ROOT, tmp_path / "first", {})
    run_mcp._live_process_context(ROOT, tmp_path / "second", {})

    assert calls == [0o700, 0o700]
    assert target.is_dir()


@pytest.mark.parametrize("unsafe", ["symlink", "world-writable"])
def test_mcp_live_never_mutates_unsafe_cargo_target(tmp_path: Path, unsafe: str) -> None:
    target = tmp_path / "cargo-target"
    if unsafe == "symlink":
        real = tmp_path / "real"
        real.mkdir(mode=0o755)
        target.symlink_to(real, target_is_directory=True)
        before = stat.S_IMODE(real.stat().st_mode)
    else:
        target.mkdir(mode=0o777)
        target.chmod(0o777)
        real = target
        before = stat.S_IMODE(target.stat().st_mode)

    with pytest.raises(ValueError, match="cargo target"):
        run_mcp._reconcile_live_cargo_target(target)

    assert stat.S_IMODE(real.stat().st_mode) == before


def test_mcp_live_rejects_missing_opt_in_before_discovery(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        run_mcp,
        "discover_tool_roots",
        lambda _env: pytest.fail("tool discovery must not run without explicit opt-in"),
    )
    with pytest.raises(ValueError, match="HIERONYMUS_QUALIFICATION_LIVE=1"):
        run_mcp.run_live(ROOT, tmp_path, original_env={"HOME": "/sensitive"})


def test_mcp_live_children_reuse_safe_environment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    cargo = tmp_path / "cargo"
    roots = ToolRoots(
        cargo_home=tmp_path / "cargo-home",
        rustup_home=tmp_path / "rustup-home",
        cargo_invocation=cargo,
        cargo_resolved_target=tmp_path / "resolved-cargo",
        rustup_invocation=tmp_path / "rustup",
        rustup_resolved_target=tmp_path / "resolved-rustup",
    )
    safe_env = {"HOME": str(tmp_path / "safe-home"), "CARGO_NET_OFFLINE": "true"}
    original_env = {
        "HIERONYMUS_QUALIFICATION_LIVE": "1",
        "HOME": "/sensitive",
        "TOKEN": "do-not-copy",
    }
    calls: list[tuple[tuple[str, ...], object]] = []

    monkeypatch.setattr(run_mcp, "discover_tool_roots", lambda _env: roots)
    monkeypatch.setattr(run_mcp, "safe_subprocess_env", lambda *args, **kwargs: safe_env)

    def process_spy(argv: tuple[str, ...], **kwargs: object) -> ProcessReceipt:
        calls.append((argv, kwargs["env"]))
        if "--transport-probe" in argv:
            marker = argv.index("--transport-probe")
            Path(argv[marker + 3]).write_text(json.dumps(_typed_probe_payload()), encoding="utf-8")
        return ProcessReceipt(
            exit_code=0,
            timed_out=False,
            stdout_sha256="0" * 64,
            stderr_sha256="0" * 64,
            duration_ms=1,
            process_group_reaped=True,
            core_dumps_disabled=True,
        )

    monkeypatch.setattr(run_mcp, "run_owned_process", process_spy)
    record = run_mcp.run_live(ROOT, tmp_path / "live", original_env=original_env)

    assert record.decision == "qualified"
    assert calls
    assert all(environment is safe_env for _, environment in calls)
    assert all(environment is not original_env for _, environment in calls)
    cargo_calls = [
        argv for argv, _ in calls if "metadata" in argv or "tree" in argv or "test" in argv
    ]
    assert len(cargo_calls) == 3
    assert all(argv[:2] == (str(cargo), "+1.96.0") for argv in cargo_calls)
    transport_calls = [argv for argv, _ in calls if "--transport-probe" in argv]
    assert len(transport_calls) == 1
    assert transport_calls[0][:4] == (
        sys.executable,
        "-B",
        "-m",
        "tools.qualification.run_mcp",
    )
    assert all(str(roots.cargo_resolved_target) not in argv for argv, _ in calls)


def _run_live_through_real_probe(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    *,
    candidate_source: str | None,
    cargo_test_exit_code: int = 0,
    toolchain_graph_matches: bool = True,
) -> tuple[object, Path, dict[str, object]]:
    cargo_target = tmp_path / "cargo-target"
    executable = cargo_target / run_mcp._TARGET / "debug/mcp-transport"
    roots = ToolRoots(
        cargo_home=tmp_path / "cargo-home",
        rustup_home=tmp_path / "rustup-home",
        cargo_invocation=tmp_path / "cargo",
        cargo_resolved_target=tmp_path / "resolved-cargo",
        rustup_invocation=tmp_path / "rustup",
        rustup_resolved_target=tmp_path / "resolved-rustup",
    )
    environment = {
        "rustc": "rustc 1.96.0 (measured)",
        "cargo": "cargo 1.96.0",
        "target": run_mcp._TARGET,
        "os": "linux",
        "kernel": "measured-kernel",
        "architecture": "x86_64",
    }
    dependency = run_mcp.LockedDependency(
        name="rmcp",
        version="3.1.4",
        source="registry+https://github.com/rust-lang/crates.io-index",
        checksum="0" * 64,
        features=("server", "transport-io", "transport-streamable-http-server"),
    )
    work_root = tmp_path / "live"
    captured_artifacts: list[dict[str, object]] = []
    monkeypatch.setattr(run_mcp, "_CARGO_TARGET", cargo_target)
    monkeypatch.setattr(run_mcp, "_LIVE_WORK", work_root)
    monkeypatch.setattr(run_mcp, "_validate_oracles", lambda _root: ())
    monkeypatch.setattr(run_mcp, "fingerprint_inputs", lambda *_args: "0" * 64)
    monkeypatch.setattr(
        run_mcp,
        "_live_process_context",
        lambda *_args: (roots, cargo_target, {"SAFE": "yes"}),
    )
    monkeypatch.setattr(run_mcp, "_require_owned_probe_environment", lambda *_args: None)
    monkeypatch.setattr(
        run_mcp,
        "_toolchain_observations",
        lambda *_args: (environment, (dependency,), 123, toolchain_graph_matches),
    )
    monkeypatch.setattr(run_mcp, "_http_probe", lambda *_args, **_kwargs: None)

    def process_spy(argv: tuple[str, ...], **_kwargs: object) -> ProcessReceipt:
        exit_code = 0
        if "test" in argv:
            exit_code = cargo_test_exit_code
            if candidate_source is not None:
                executable.parent.mkdir(parents=True, exist_ok=True)
                executable.write_text(candidate_source, encoding="utf-8")
                executable.chmod(0o700)
        if "--transport-probe" in argv:
            marker = argv.index("--transport-probe")
            values = tuple(Path(value) for value in argv[marker + 1 : marker + 6])
            run_mcp._transport_probe(values[0], ROOT, *values[1:])
            captured_artifacts.append(json.loads(values[2].read_text(encoding="utf-8")))
        return ProcessReceipt(
            exit_code=exit_code,
            timed_out=False,
            stdout_sha256="0" * 64,
            stderr_sha256="0" * 64,
            duration_ms=1,
            process_group_reaped=True,
            core_dumps_disabled=True,
        )

    monkeypatch.setattr(run_mcp, "run_owned_process", process_spy)
    record = run_mcp.run_live(
        ROOT,
        work_root,
        original_env={"HIERONYMUS_QUALIFICATION_LIVE": "1"},
    )
    assert len(captured_artifacts) == 1
    return record, work_root, captured_artifacts[0]


def test_mcp_live_wrong_candidate_response_is_retained_as_blocked_record(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    candidate = (
        f"#!{sys.executable}\n"
        "import json, sys\n"
        "sys.stdin.readline()\n"
        "print(json.dumps({'jsonrpc': '2.0', 'id': 1, 'result': {'wrong': True}}))\n"
    )
    record, work_root, artifact = _run_live_through_real_probe(
        tmp_path,
        monkeypatch,
        candidate_source=candidate,
    )

    evidence = {item.criterion: item.status for item in record.evidence}
    failed = {criterion for criterion, status in evidence.items() if status == "fail"}
    assert record.status == "fail"
    assert record.decision == "blocked"
    assert record.consequence == FAILURE_CONSEQUENCES["mcp-transport"]
    assert record.review.status == "pending"
    assert "stdio-newline-jsonrpc" in failed
    assert "streamable-http-json" not in failed
    assert 0 < len(failed) < len(REQUIRED_CRITERIA["mcp-transport"])
    checks = artifact["checks"]
    assert isinstance(checks, dict)
    assert set(checks) == set(REQUIRED_CRITERIA["mcp-transport"])
    assert checks["stdio-newline-jsonrpc"]["passed"] is False
    assert checks["stdio-newline-jsonrpc"]["measurements"] == {"failed_observations": 2}
    assert not work_root.exists()


def test_mcp_live_missing_candidate_after_failed_build_retains_blocked_record(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    record, _work_root, artifact = _run_live_through_real_probe(
        tmp_path,
        monkeypatch,
        candidate_source=None,
        cargo_test_exit_code=1,
    )

    evidence = {item.criterion: item.status for item in record.evidence}
    assert record.status == "fail"
    assert record.decision == "blocked"
    assert record.consequence == FAILURE_CONSEQUENCES["mcp-transport"]
    assert evidence["locked-native-build"] == "fail"
    assert evidence["stdio-newline-jsonrpc"] == "fail"
    assert evidence["streamable-http-json"] == "fail"
    assert record.environment.rustc == "rustc 1.96.0 (measured)"
    assert tuple(item.name for item in record.dependencies) == ("rmcp",)
    checks = artifact["checks"]
    assert isinstance(checks, dict)
    assert len(checks) == 17


def test_mcp_live_toolchain_graph_mismatch_blocks_only_locked_build(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    candidate = (
        f"#!{sys.executable}\n"
        "import json, pathlib, sys\n"
        "request = json.loads(sys.stdin.readline())\n"
        "protocol = pathlib.Path(sys.argv[sys.argv.index('--protocol') + 1])\n"
        "target = json.loads(protocol.read_text(encoding='utf-8'))['target']\n"
        "key = 'tools_list' if request['method'] == 'tools/list' else 'tools_call'\n"
        "print(json.dumps(target[key]['response']))\n"
    )
    record, _work_root, artifact = _run_live_through_real_probe(
        tmp_path,
        monkeypatch,
        candidate_source=candidate,
        toolchain_graph_matches=False,
    )

    failed = {item.criterion for item in record.evidence if item.status == "fail"}
    assert failed == {"locked-native-build"}
    checks = artifact["checks"]
    assert isinstance(checks, dict)
    assert checks["locked-native-build"] == {
        "passed": False,
        "measurements": {
            "failed_observations": 1,
            "feature_tree_bytes": 123,
            "locked_commands": 3,
        },
    }


def test_mcp_live_noop_success_receipts_without_artifact_block(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    roots = ToolRoots(*(tmp_path / name for name in ("ch", "rh", "c", "cr", "r", "rr")))
    safe_env = {"HOME": str(tmp_path / "safe")}
    monkeypatch.setattr(
        run_mcp,
        "_live_process_context",
        lambda *_args: (roots, ROOT / run_mcp._CARGO_TARGET, safe_env),
    )
    monkeypatch.setattr(
        run_mcp,
        "run_owned_process",
        lambda *_args, **_kwargs: ProcessReceipt(
            exit_code=0,
            timed_out=False,
            stdout_sha256="0" * 64,
            stderr_sha256="0" * 64,
            duration_ms=1,
            process_group_reaped=True,
            core_dumps_disabled=True,
        ),
    )
    with pytest.raises(ValueError, match="probe artifact"):
        run_mcp.run_live(
            ROOT,
            ROOT / run_mcp._LIVE_WORK,
            original_env={"HIERONYMUS_QUALIFICATION_LIVE": "1"},
        )


def test_mcp_live_rejects_symlink_work_root_without_chmod(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target = tmp_path / "target"
    target.mkdir(mode=0o755)
    work = tmp_path / "work"
    work.symlink_to(target, target_is_directory=True)
    monkeypatch.setattr(run_mcp, "_validate_oracles", lambda _root: ())
    monkeypatch.setattr(run_mcp, "fingerprint_inputs", lambda *_args: "0" * 64)
    with pytest.raises(ValueError, match="symlink|private work"):
        run_mcp.run_live(
            ROOT,
            work,
            original_env={"HIERONYMUS_QUALIFICATION_LIVE": "1"},
        )
    assert stat.S_IMODE(target.stat().st_mode) == 0o755


def test_mcp_hidden_probe_rejects_direct_bypass(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executable = tmp_path / "mcp-transport"
    executable.write_text("fake", encoding="utf-8")
    executable.chmod(0o700)
    work = tmp_path / "work"
    work.mkdir(mode=0o700)
    monkeypatch.setattr(run_mcp, "_CARGO_TARGET", Path("."))
    monkeypatch.setattr(run_mcp, "_LIVE_WORK", Path("work"))
    monkeypatch.setenv(run_mcp._PROBE_MARKER, "1")
    with pytest.raises(ValueError, match="owned probe"):
        run_mcp._transport_probe(
            executable,
            tmp_path,
            work,
            work / "result.json",
            tmp_path / "cargo",
            tmp_path / "rustup",
        )


def test_mcp_inner_capture_rejects_output_flood(tmp_path: Path) -> None:
    script = tmp_path / "flood.py"
    script.write_text(
        "import sys\nsys.stdout.buffer.write(b'x' * (2 * 1024 * 1024 + 1))\n",
        encoding="utf-8",
    )
    with pytest.raises(run_mcp.CandidateOutput, match="output exceeds"):
        run_mcp._run_bounded((sys.executable, str(script)), cwd=tmp_path)


def test_mcp_inner_timeout_reaps_descendant_process(tmp_path: Path) -> None:
    marker = tmp_path / "child.pid"
    script = tmp_path / "timeout.py"
    script.write_text(
        "import pathlib, subprocess, sys, time\n"
        "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
        "pathlib.Path(sys.argv[1]).write_text(str(child.pid), encoding='ascii')\n"
        "time.sleep(60)\n",
        encoding="utf-8",
    )
    with pytest.raises(run_mcp.CandidateTimeout, match="timed out"):
        run_mcp._run_bounded(
            (sys.executable, str(script), str(marker)),
            cwd=tmp_path,
            timeout_seconds=1,
        )
    child_pid = int(marker.read_text(encoding="ascii"))
    deadline = time.monotonic() + 2
    while Path(f"/proc/{child_pid}").exists() and time.monotonic() < deadline:
        time.sleep(0.01)
    assert not Path(f"/proc/{child_pid}").exists()


@pytest.mark.parametrize("error", [OSError("spawn failed"), ValueError("harness bug")])
def test_mcp_candidate_group_does_not_convert_infrastructure_errors_to_evidence(
    error: Exception,
) -> None:
    checks = run_mcp._checks_from_observations(set(), tree_bytes=1)

    def fail() -> None:
        raise error

    with pytest.raises(type(error), match=str(error)):
        run_mcp._collect_candidate_group(set(), "canonical-stdio", fail)

    assert all(not passed for passed, _ in checks.values())


def test_mcp_candidate_group_retains_only_typed_candidate_mismatch() -> None:
    observed: set[str] = set()

    run_mcp._collect_candidate_group(
        observed,
        "canonical-stdio",
        lambda: (_ for _ in ()).throw(run_mcp.CandidateMismatch("wrong response")),
    )

    assert observed == set()


@pytest.mark.parametrize(
    ("source", "error"),
    [
        ("stdio", OSError("candidate spawn failed")),
        ("stdio", ValueError("runner invariant failed")),
        ("http", RuntimeError("report reader failed")),
    ],
)
def test_mcp_probe_infrastructure_failure_never_writes_blocked_artifact(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    source: str,
    error: Exception,
) -> None:
    target = tmp_path / "cargo-target"
    executable = target / run_mcp._TARGET / "debug/mcp-transport"
    executable.parent.mkdir(parents=True)
    executable.write_text("candidate", encoding="utf-8")
    work = tmp_path / "work"
    work.mkdir(mode=0o700)
    artifact = work / "probe-result.json"
    environment = {
        "rustc": "rustc 1.96.0",
        "cargo": "cargo 1.96.0",
        "target": run_mcp._TARGET,
        "os": "linux",
        "kernel": "measured",
        "architecture": "x86_64",
    }
    dependency = run_mcp.LockedDependency(
        name="rmcp",
        version="3.1.4",
        source="registry+https://github.com/rust-lang/crates.io-index",
        checksum="0" * 64,
        features=("server", "transport-io", "transport-streamable-http-server"),
    )
    monkeypatch.setattr(run_mcp, "_CARGO_TARGET", target)
    monkeypatch.setattr(run_mcp, "_LIVE_WORK", work)
    monkeypatch.setattr(run_mcp, "_require_owned_probe_environment", lambda *_args: None)
    monkeypatch.setattr(
        run_mcp,
        "_toolchain_observations",
        lambda *_args: (environment, (dependency,), 1, True),
    )

    def fail(*_args: object, **_kwargs: object) -> None:
        raise error

    monkeypatch.setattr(run_mcp, "_stdio_probe", fail if source == "stdio" else lambda *_a: None)
    monkeypatch.setattr(
        run_mcp,
        "_http_probe",
        fail if source == "http" else lambda *_args, **_kwargs: None,
    )
    monkeypatch.setattr(
        run_mcp,
        "_write_private_json",
        lambda *_args: pytest.fail("infrastructure failure must not write evidence"),
    )

    with pytest.raises(type(error), match=str(error)):
        run_mcp._transport_probe(
            executable,
            ROOT,
            work,
            artifact,
            tmp_path / "cargo",
            tmp_path / "rustup",
        )

    assert not artifact.exists()


@pytest.mark.parametrize("criterion", REQUIRED_CRITERIA["mcp-transport"])
def test_mcp_each_criterion_requires_every_named_live_observation(criterion: str) -> None:
    all_observations = {
        observation
        for dependencies in run_mcp._CRITERION_OBSERVATIONS.values()
        for observation in dependencies
    }
    passed = run_mcp._checks_from_observations(all_observations, tree_bytes=123)
    assert passed[criterion][0] is True

    for dependency in run_mcp._CRITERION_OBSERVATIONS[criterion]:
        missing = run_mcp._checks_from_observations(all_observations - {dependency}, tree_bytes=123)
        assert missing[criterion][0] is False


def test_mcp_runner_rejects_projection_drift_before_executing_candidate(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        run_mcp,
        "projection_issues",
        lambda _root: {
            "mcp-transport": ("mcp compatibility projection is stale",),
            "legacy-database-import": (),
        },
    )
    missing = tmp_path / "must-not-run"
    with pytest.raises(ValueError, match="projection"):
        run(ROOT, tmp_path, executable=missing)
