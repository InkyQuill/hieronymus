"""Fake-injected and live-context tests for the legacy-database-import runner."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.qualification.factories import write_fake_executable  # noqa: E402
from tools.qualification import run_database  # noqa: E402
from tools.qualification.model import REQUIRED_CRITERIA  # noqa: E402
from tools.qualification.process import ProcessReceipt, ToolRoots  # noqa: E402
from tools.qualification.projections import projection_issues  # noqa: E402
from tools.qualification.run_database import run  # noqa: E402

_DATABASE_CRITERIA = REQUIRED_CRITERIA["legacy-database-import"]

# The frozen fixture matrix: (basename, classification, safe_to_convert).
_FIXTURE_MATRIX = (
    ("minimal-python.sqlite", "supported-python", True),
    ("legacy-python.sqlite", "supported-legacy-python", True),
    ("empty.sqlite", "empty", False),
    ("partial-python.sqlite", "partial-python", False),
    ("corrupt.sqlite", "corrupt", False),
    ("unknown-schema.sqlite", "unknown-schema", False),
)
_REFUSAL_CODES = {
    "corrupt": "source-unreadable",
    "empty": "empty-database",
    "partial-python": "partial-schema",
    "unknown-schema": "unknown-schema",
}
_FTS_TABLES = (
    "concepts_fts",
    "crystals_fts",
    "rag_chunks_fts",
    "short_term_memories_fts",
    "strict_terms_fts",
)


def _classification_of(name: str) -> str:
    return next(entry[1] for entry in _FIXTURE_MATRIX if entry[0] == name)


def _classification_payload(name: str) -> bytes:
    payload = {
        "foreign_key_violations": 0,
        "integrity": "unreadable" if _classification_of(name) == "corrupt" else "ok",
        "name": _classification_of(name),
        "safe_to_convert": _classification_of(name).startswith("supported-"),
        "schema_digest": "b" * 64,
        "source_name": name,
        "source_sha256": "a" * 64,
        "variant_id": f"variant-{_classification_of(name)}",
    }
    return json.dumps(payload, sort_keys=True).encode()


def _probe_receipt_payload(name: str) -> bytes:
    deep = _classification_of(name) == "supported-python"
    # Zero-row domain tables are legitimate; accounting conserves the row
    # multiset rather than requiring every table to be populated.
    rows = (
        {"series": 3, "strict_terms": 5}
        if deep
        else {
            "crystals": 1,
            "dream_runs": 0,
            "series": 2,
            "strict_terms": 3,
        }
    )
    total = sum(rows.values())
    payload = {
        "classification": _classification_of(name),
        "error_code": None,
        "fixture_directory_identical": True,
        "fts": {table: {"digest": "d" * 64, "ids": [1, 2]} for table in _FTS_TABLES}
        if deep
        else {},
        "ledger": {"blocking": 0, "read": total, "skipped": 0},
        "migration_sources_digest": "c" * 64,
        "ok": True,
        "probe_row_count": total,
        "representative_row_digests": {"series": "e" * 64} if deep else {},
        "row_counts": {**rows, "concepts_fts": 5},
        "rows_per_table": rows,
        "safe_to_convert": True,
        "schema_digest": "b" * 64,
        "source_bytes_identical": True,
        "source_name": name,
        "source_sha256": "a" * 64,
        "target_sha256": "f" * 64,
    }
    return json.dumps(payload, sort_keys=True).encode()


def _refusal_payload(name: str) -> bytes:
    classification = _classification_of(name)
    payload = {
        "classification": classification,
        "error_code": _REFUSAL_CODES[classification],
        "ok": False,
        "safe_to_convert": False,
        "source_name": name,
    }
    return json.dumps(payload, sort_keys=True).encode()


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


def _roots(tmp_path: Path) -> ToolRoots:
    return ToolRoots(
        cargo_home=tmp_path / "cargo-home",
        rustup_home=tmp_path / "rustup-home",
        cargo_invocation=tmp_path / "cargo",
        cargo_resolved_target=tmp_path / "resolved-cargo",
        rustup_invocation=tmp_path / "rustup",
        rustup_resolved_target=tmp_path / "resolved-rustup",
    )


def _patch_live_boundaries(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> ToolRoots:
    roots = _roots(tmp_path)
    monkeypatch.setattr(run_database, "_CARGO_TARGET", tmp_path / "cargo-target")
    monkeypatch.setattr(run_database, "discover_tool_roots", lambda _env: roots)
    return roots


# ---------------------------------------------------------------------------
# Fake-only runner tests (brief Step 1 snippets verbatim)
# ---------------------------------------------------------------------------


def test_database_runner_uses_only_frozen_fixture_root(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=write_fake_executable(tmp_path))
    assert "qualification/compatibility/legacy-database-import.json" in record.input_paths
    assert "tools/qualification/projections.py" in record.input_paths
    assert "compatibility/manifest.json" not in record.input_paths
    assert "compatibility/snapshots/state.json" not in record.input_paths
    assert all(
        not path.endswith(".sqlite") or path.startswith("compatibility/fixtures/database/")
        for path in record.input_paths
    )
    assert projection_issues(ROOT)["legacy-database-import"] == ()


def test_database_failure_preserves_data_disposition(tmp_path: Path) -> None:
    executable = write_fake_executable(tmp_path, failed_criteria=("source-byte-identity",))
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "blocked"
    assert "fresh sibling database" in record.consequence


def test_database_fake_runner_requires_exact_criterion_set(tmp_path: Path) -> None:
    executable = write_fake_executable(tmp_path, failed_criteria=("not-a-criterion",))
    with pytest.raises(ValueError, match="criterion"):
        run(ROOT, tmp_path, executable=executable)


# ---------------------------------------------------------------------------
# Live context ordering
# ---------------------------------------------------------------------------


def test_database_live_context_discovers_before_sanitizing(
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
    target = ROOT / "qualification/.artifacts/cargo-target/legacy-database-import"
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

    monkeypatch.setattr(run_database, "discover_tool_roots", discover)
    monkeypatch.setattr(run_database, "safe_subprocess_env", sanitize)

    actual = run_database._live_process_context(ROOT, tmp_path, original_env)

    assert actual == (roots, target, safe_env)
    assert calls == [
        ("discover", original_env),
        ("sanitize", tmp_path, True, roots, target),
    ]


def test_database_live_rejects_missing_opt_in_before_discovery(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        run_database,
        "discover_tool_roots",
        lambda _env: pytest.fail("tool discovery must not run without explicit opt-in"),
    )
    with pytest.raises(ValueError, match="HIERONYMUS_QUALIFICATION_LIVE=1"):
        run_database.run_live(ROOT, tmp_path, original_env={"HOME": "/sensitive"})


# ---------------------------------------------------------------------------
# Live spy fixtures
# ---------------------------------------------------------------------------


def _happy_spy(calls: list[tuple[tuple[str, ...], object]], roots: ToolRoots):
    """Serve every database live child from deterministic fixtures."""

    def spy(argv: tuple[str, ...], **kwargs: object) -> ProcessReceipt:
        environment = kwargs["env"]
        assert isinstance(environment, dict)
        calls.append((argv, environment))
        if run_database._TOOLCHAIN_MARKER in argv:
            Path(argv[argv.index(run_database._TOOLCHAIN_MARKER) + 1]).write_text(
                json.dumps(
                    {
                        "cargo": "cargo 1.96.0 (measured)",
                        "rustc": "rustc 1.96.0 (measured)",
                        "host": "x86_64-unknown-linux-gnu",
                    }
                ),
                encoding="utf-8",
            )
            return _successful_receipt(0)
        if argv[0] == str(roots.cargo_invocation):
            assert argv[1] == "+1.96.0"
            assert "build" in argv
            binary = Path(str(environment["CARGO_TARGET_DIR"])) / (
                "x86_64-unknown-linux-gnu/release/legacy-database-import"
            )
            binary.parent.mkdir(parents=True, exist_ok=True)
            binary.write_bytes(b"fake-database-binary")
            return _successful_receipt(0)
        if run_database._LDD_MARKER in argv:
            Path(argv[argv.index(run_database._LDD_MARKER) + 1]).write_text(
                "linux-vdso.so.1 (0x0000)\n"
                "libc.so.6 => /usr/lib/libc.so.6 (0x0000)\n"
                "libgcc_s.so.1 => /usr/lib/libgcc_s.so.1 (0x0000)\n",
                encoding="utf-8",
            )
            return _successful_receipt(0)
        if run_database._HARNESS_MARKER in argv:
            output = Path(argv[argv.index(run_database._HARNESS_MARKER) + 1])
            verb = argv[6]
            arguments = argv[7:]
            if verb == "classify":
                name = arguments[arguments.index("--source-name") + 1]
                output.write_bytes(_classification_payload(name))
                return _successful_receipt(0)
            if verb == "probe-import":
                name = arguments[arguments.index("--source-name") + 1]
                if _classification_of(name).startswith("supported-"):
                    output.write_bytes(_probe_receipt_payload(name))
                    return _successful_receipt(0)
                output.write_bytes(_refusal_payload(name))
                return _successful_receipt(1)
            pytest.fail(f"unexpected harness verb: {verb}")
        pytest.fail(f"unexpected live child: {argv}")

    return spy


# ---------------------------------------------------------------------------
# Live children discipline
# ---------------------------------------------------------------------------


def test_database_live_children_reuse_safe_environment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    roots = _patch_live_boundaries(monkeypatch, tmp_path)
    live_work = tmp_path / "live"
    safe_env = {
        "HOME": str(tmp_path / "safe-home"),
        "CARGO_TARGET_DIR": str(tmp_path / "cargo-target"),
        "CARGO_NET_OFFLINE": "true",
    }
    monkeypatch.setattr(run_database, "safe_subprocess_env", lambda *_a, **_k: safe_env)
    original_env = {
        "HIERONYMUS_QUALIFICATION_LIVE": "1",
        "HOME": "/sensitive",
        "TOKEN": "do-not-copy",
    }
    calls: list[tuple[tuple[str, ...], object]] = []
    monkeypatch.setattr(run_database, "run_owned_process", _happy_spy(calls, roots))

    record = run_database.run_live(ROOT, live_work, original_env=original_env)

    assert record.decision == "qualified"
    assert {item.criterion for item in record.evidence} == set(_DATABASE_CRITERIA)
    assert all(item.status == "pass" for item in record.evidence)
    assert calls
    assert all(environment is safe_env for _, environment in calls)
    assert all(environment is not original_env for _, environment in calls)

    cargo_calls = [argv for argv, _ in calls if argv[0] == str(roots.cargo_invocation)]
    assert len(cargo_calls) == 1
    assert all(argv[1] == "+1.96.0" for argv in cargo_calls)
    assert all("build" in argv and "--locked" in argv for argv in cargo_calls)

    harness_calls = [argv for argv, _ in calls if run_database._HARNESS_MARKER in argv]
    # six classifications, two supported probes, four fail-closed refusals.
    assert len(harness_calls) == 12
    assert all(argv[5].endswith("release/legacy-database-import") for argv in harness_calls)
    for argv in harness_calls:
        arguments = argv[7:]
        assert arguments[arguments.index("--fixture-root") + 1] == str(
            ROOT / "compatibility/fixtures/database"
        )
        assert arguments[arguments.index("--contract") + 1] == str(
            ROOT / "qualification/compatibility/legacy-database-import.json"
        )
    probe_calls = [argv for argv in harness_calls if argv[6] == "probe-import"]
    assert len(probe_calls) == 6
    for argv in probe_calls:
        arguments = argv[7:]
        work_root = Path(arguments[arguments.index("--work-root") + 1])
        assert work_root.is_relative_to(live_work)
        assert work_root.is_dir() or not work_root.exists()

    ldd_calls = [argv for argv, _ in calls if run_database._LDD_MARKER in argv]
    assert len(ldd_calls) == 1
    versions_calls = [argv for argv, _ in calls if run_database._TOOLCHAIN_MARKER in argv]
    assert len(versions_calls) == 1

    serialized = json.dumps(json.loads(run_database.serialize_record(record)), sort_keys=True)
    for forbidden in (
        str(roots.cargo_invocation),
        str(roots.cargo_resolved_target),
        str(roots.cargo_home),
        str(tmp_path / "cargo-target"),
        str(live_work),
        "/tmp/",
        "/home/",
    ):
        assert forbidden not in serialized


def test_database_live_build_failure_writes_complete_blocked_record(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    roots = _patch_live_boundaries(monkeypatch, tmp_path)
    safe_env = {
        "HOME": str(tmp_path / "safe-home"),
        "CARGO_TARGET_DIR": str(tmp_path / "cargo-target"),
    }
    monkeypatch.setattr(run_database, "safe_subprocess_env", lambda *_a, **_k: safe_env)
    original_env = {"HIERONYMUS_QUALIFICATION_LIVE": "1"}
    calls: list[tuple[tuple[str, ...], object]] = []

    def build_failure_spy(argv: tuple[str, ...], **kwargs: object) -> ProcessReceipt:
        environment = kwargs["env"]
        assert isinstance(environment, dict)
        calls.append((argv, environment))
        if run_database._TOOLCHAIN_MARKER in argv:
            Path(argv[argv.index(run_database._TOOLCHAIN_MARKER) + 1]).write_text(
                json.dumps(
                    {
                        "cargo": "cargo 1.96.0 (measured)",
                        "rustc": "rustc 1.96.0 (measured)",
                        "host": "x86_64-unknown-linux-gnu",
                    }
                ),
                encoding="utf-8",
            )
            return _successful_receipt(0)
        if argv[0] == str(roots.cargo_invocation):
            return _successful_receipt(1)
        pytest.fail("no qualification child may launch after the frozen build failed")

    monkeypatch.setattr(run_database, "run_owned_process", build_failure_spy)

    record = run_database.run_live(ROOT, tmp_path / "live", original_env=original_env)

    assert record.decision == "blocked"
    assert "fresh sibling database" in record.consequence
    evidence = {item.criterion: item for item in record.evidence}
    assert set(evidence) == set(_DATABASE_CRITERIA)
    assert evidence["bundled-sqlite-fts5"].status == "fail"
    assert all(
        item.status == "not-run" and item.not_run_reason == "bundled-sqlite-fts5"
        for name, item in evidence.items()
        if name != "bundled-sqlite-fts5"
    )
    assert record.cleanup.work_dir_removed is True
    assert record.cleanup.install_dir_removed is True
    assert record.cleanup.source_inputs_unchanged is True
    assert all(environment is safe_env for _, environment in calls)
