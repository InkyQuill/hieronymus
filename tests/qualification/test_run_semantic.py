"""Fake-injected and live-context tests for the semantic-native runner."""

from __future__ import annotations

import json
import sqlite3
import sys
from pathlib import Path, PurePosixPath

import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.qualification.factories import write_fake_executable  # noqa: E402
from tools.qualification import run_semantic  # noqa: E402
from tools.qualification.model import REQUIRED_CRITERIA  # noqa: E402
from tools.qualification.process import ProcessReceipt, ToolRoots  # noqa: E402
from tools.qualification.run_semantic import _live_process_context, run  # noqa: E402

_SEMANTIC_CRITERIA = REQUIRED_CRITERIA["semantic-native"]
_STATE_SCHEMA = """
create table if not exists semantic_generations (
    generation_id text primary key,
    status text not null,
    written_count integer not null default 0,
    active integer not null default 0
);
create table if not exists semantic_jobs (
    job_id text primary key,
    generation_id text not null,
    status text not null,
    next_batch integer not null default 0,
    completed_batches integer not null default 0,
    lease_owner text,
    lease_expires_unix_ms integer
);
"""


def _flags(argv: tuple[str, ...], flag: str) -> str:
    return argv[argv.index(flag) + 1]


def _successful_receipt(exit_code: int = 0) -> ProcessReceipt:
    return ProcessReceipt(
        exit_code=exit_code,
        timed_out=False,
        stdout_sha256="0" * 64,
        stderr_sha256="0" * 64,
        duration_ms=1,
        process_group_reaped=True,
        core_dumps_disabled=True,
    )


def _fixture_state_update(argv: tuple[str, ...]) -> None:
    """Advance the fake durable SQLite state exactly like the real candidate."""
    generation = _flags(argv, "--generation")
    mode = _flags(argv, "--mode")
    stop_after = int(_flags(argv, "--stop-after"))
    state_db = Path(_flags(argv, "--state-db"))
    state_db.parent.mkdir(parents=True, exist_ok=True)
    connection = sqlite3.connect(state_db)
    try:
        connection.executescript(_STATE_SCHEMA)
        status, active = {
            "complete": ("active", 1),
            "crash": ("building", 0),
            "cancel": ("cancelled", 0),
        }[mode]
        if mode == "complete":
            connection.execute("update semantic_generations set status = 'superseded', active = 0")
        connection.execute(
            "insert or replace into semantic_generations"
            " (generation_id, status, written_count, active) values (?, ?, ?, ?)",
            (generation, status, stop_after, active),
        )
        connection.execute(
            "insert or replace into semantic_jobs"
            " (job_id, generation_id, status, next_batch, completed_batches,"
            "  lease_owner, lease_expires_unix_ms)"
            " values (?, ?, ?, ?, ?, ?, ?)",
            (
                f"build:{generation}",
                generation,
                "complete" if mode == "complete" else mode,
                stop_after // 100,
                stop_after // 100,
                None if mode == "complete" else "42:1",
                None if mode == "complete" else 1,
            ),
        )
        connection.commit()
    finally:
        connection.close()


def _receipt_spy(calls: list[tuple[tuple[str, ...], object]], roots: ToolRoots):
    """Return a criterion-fixture-backed ``run_owned_process`` replacement."""

    def spy(argv: tuple[str, ...], **kwargs: object) -> ProcessReceipt:
        environment = kwargs["env"]
        assert isinstance(environment, dict)
        calls.append((argv, environment))
        exit_code = 0
        if argv[0] == str(roots.cargo_invocation):
            assert argv[1] == "+1.96.0"
            if "build" in argv:
                binary = (
                    Path(str(environment["CARGO_TARGET_DIR"]))
                    / "x86_64-unknown-linux-gnu/release/semantic-native"
                )
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_bytes(b"fake-semantic-binary")
        elif "qualification-ldd-capture" in argv[3]:
            Path(argv[4]).write_text(
                "linux-vdso.so.1 (0x0000)\n"
                "libc.so.6 => /usr/lib/libc.so.6 (0x0000)\n"
                "libm.so.6 => /usr/lib/libm.so.6 (0x0000)\n",
                encoding="utf-8",
            )
        elif "qualification-toolchain-capture" in argv[3]:
            Path(argv[4]).write_text(
                json.dumps(
                    {
                        "cargo": "cargo 1.96.0 (measured)",
                        "rustc": "rustc 1.96.0 (measured)",
                        "host": "x86_64-unknown-linux-gnu",
                    }
                ),
                encoding="utf-8",
            )
        elif argv[0].endswith("semantic-native"):
            if "scenario" in argv:
                _fixture_state_update(argv)
                if _flags(argv, "--mode") == "crash":
                    exit_code = -6
        return _successful_receipt(exit_code)

    return spy


def test_any_semantic_failure_selects_fts_only(tmp_path: Path) -> None:
    executable = write_fake_executable(tmp_path, failed_criteria=("series-prefilter-before-ann",))
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "fts-only"
    assert "do not block" in record.consequence


def test_prefilter_and_recovery_evidence_are_required(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=write_fake_executable(tmp_path))
    criteria = {item.criterion for item in record.evidence}
    assert "series-prefilter-before-ann" in criteria
    assert "no-sqlite-write-across-native-io" in criteria
    assert (
        "compatibility/fixtures/mcp/hieronymus_rag_search/success.input.json" in record.input_paths
    )
    assert "compatibility/fixtures/mcp/hieronymus_recall/success.input.json" in record.input_paths
    assert len(record.input_paths) == 28
    assert all(
        PurePosixPath(path).parts[3] != "tools"
        for path in record.input_paths
        if path.startswith("compatibility/fixtures/mcp/hieronymus_")
    )


def test_semantic_fake_runner_requires_exact_criterion_set(tmp_path: Path) -> None:
    executable = write_fake_executable(tmp_path, failed_criteria=("not-a-criterion",))
    with pytest.raises(ValueError, match="criterion"):
        run(ROOT, tmp_path, executable=executable)


def test_semantic_live_context_discovers_before_sanitizing(
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
    target = ROOT / "qualification/.artifacts/cargo-target/semantic-native"
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
        calls.append(("sanitize", work_root, cargo_offline, tool_roots, cargo_target_dir))
        return safe_env

    monkeypatch.setattr(run_semantic, "discover_tool_roots", discover)
    monkeypatch.setattr(run_semantic, "safe_subprocess_env", sanitize)

    actual = _live_process_context(ROOT, tmp_path, original_env)

    assert actual == (roots, target, safe_env)
    assert calls == [
        ("discover", original_env),
        ("sanitize", tmp_path, True, roots, target),
    ]


def test_semantic_live_rejects_missing_opt_in_before_discovery(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        run_semantic,
        "discover_tool_roots",
        lambda _env: pytest.fail("tool discovery must not run without explicit opt-in"),
    )
    with pytest.raises(ValueError, match="HIERONYMUS_QUALIFICATION_LIVE=1"):
        run_semantic.run_live(ROOT, tmp_path, original_env={"HOME": "/sensitive"})


def test_semantic_live_rejects_work_root_override_in_safe_environment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    roots = ToolRoots(*(tmp_path / name for name in ("ch", "rh", "c", "cr", "r", "rr")))
    override = run_semantic._WORK_ROOT_OVERRIDE
    unsafe_env = {"SAFE": "yes", override: str(tmp_path / "relocated")}
    monkeypatch.setattr(run_semantic, "discover_tool_roots", lambda _env: roots)
    monkeypatch.setattr(run_semantic, "safe_subprocess_env", lambda *_args, **_kwargs: unsafe_env)
    monkeypatch.setattr(run_semantic, "_CARGO_TARGET", tmp_path / "cargo-target")
    monkeypatch.setattr(
        run_semantic,
        "run_owned_process",
        lambda *_args, **_kwargs: pytest.fail("children must not launch after override leak"),
    )

    with pytest.raises(ValueError, match="work root override"):
        run_semantic.run_live(
            ROOT,
            tmp_path / "work",
            original_env={"HIERONYMUS_QUALIFICATION_LIVE": "1"},
        )


def test_semantic_live_children_reuse_safe_environment(
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
    safe_env = {
        "HOME": str(tmp_path / "safe-home"),
        "CARGO_TARGET_DIR": str(tmp_path / "cargo-target"),
        "CARGO_NET_OFFLINE": "true",
    }
    original_env = {
        "HIERONYMUS_QUALIFICATION_LIVE": "1",
        "HOME": "/sensitive",
        "TOKEN": "do-not-copy",
    }
    calls: list[tuple[tuple[str, ...], object]] = []

    monkeypatch.setattr(run_semantic, "_CARGO_TARGET", tmp_path / "cargo-target")
    monkeypatch.setattr(run_semantic, "_INSTALL", tmp_path / "install")
    monkeypatch.setattr(run_semantic, "discover_tool_roots", lambda _env: roots)
    monkeypatch.setattr(run_semantic, "safe_subprocess_env", lambda *_a, **_k: safe_env)
    monkeypatch.setattr(run_semantic, "run_owned_process", _receipt_spy(calls, roots))

    record = run_semantic.run_live(
        ROOT,
        tmp_path / "live",
        original_env=original_env,
    )

    assert record.decision == "semantic-enabled"
    assert {item.criterion for item in record.evidence} == set(_SEMANTIC_CRITERIA)
    assert calls
    assert all(environment is safe_env for _, environment in calls)
    assert all(environment is not original_env for _, environment in calls)
    cargo_calls = [argv for argv, _ in calls if argv[0] == str(cargo)]
    assert len(cargo_calls) == 2
    assert all(argv[1] == "+1.96.0" for argv in cargo_calls)
    assert all("build" in argv for argv in cargo_calls)
    assert all(str(roots.cargo_resolved_target) not in argv for argv, _ in calls)
    assert all(str(roots.rustup_resolved_target) not in argv for argv, _ in calls)
    binary_calls = [argv for argv, _ in calls if Path(argv[0]).name == "semantic-native"]
    # one installed probe, generation-a, 50 queries, isolation crash, two
    # isolation queries, crash at 4,200, resume, cancel, 50 FTS queries.
    assert len(binary_calls) == 109
    ldd_calls = [argv for argv, _ in calls if "qualification-ldd-capture" in argv[3]]
    versions_calls = [argv for argv, _ in calls if "qualification-toolchain-capture" in argv[3]]
    assert len(ldd_calls) == 1
    assert len(versions_calls) == 1
    cargo_markers = sum("cargo" in argv[5] for argv in versions_calls)
    assert cargo_markers == 1
