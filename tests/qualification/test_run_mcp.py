from __future__ import annotations

import sys
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


def test_mcp_runner_consumes_corrected_oracle(tmp_path: Path) -> None:
    executable = write_fake_executable(tmp_path)
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
    executable = write_fake_executable(tmp_path, failed_criteria=("official-schema-envelopes",))
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "blocked"
    assert record.consequence == FAILURE_CONSEQUENCES["mcp-transport"]


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
    record = run_mcp.run_live(ROOT, tmp_path, original_env=original_env)

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
