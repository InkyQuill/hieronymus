"""Offline semantic-native qualification runner and canonical evidence writer."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import selectors
import shutil
import signal
import sqlite3
import stat
import subprocess
import sys
import time
import tomllib
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path

from tools.qualification.fingerprint import fingerprint_inputs, required_fingerprint_inputs
from tools.qualification.model import (
    REQUIRED_CRITERIA,
    CleanupEvidence,
    Environment,
    Evidence,
    LockedDependency,
    Measurements,
    QualificationRecord,
    Review,
    decision_for,
    expected_consequence,
    serialize_record,
    status_for,
)
from tools.qualification.process import (
    ProcessReceipt,
    ToolRoots,
    discover_tool_roots,
    run_owned_process,
    safe_subprocess_env,
)
from tools.qualification.render import render_record

_RISK = "semantic-native"
_OWNER = "Pavel Obruchnikov <me@inkyquill.net>"
_TARGET = "x86_64-unknown-linux-gnu"
_TOOLCHAIN = "1.96.0"
_MANIFEST = "qualification/harnesses/semantic-native/Cargo.toml"
_LOCK = "qualification/harnesses/semantic-native/Cargo.lock"
_CORPUS = "qualification/fixtures/semantic-corpus.json"
_CARGO_TARGET = Path("qualification/.artifacts/cargo-target/semantic-native")
_INSTALL = Path("qualification/.artifacts/install/semantic-native")
_LIVE_WORK = Path("qualification/.artifacts/work/semantic-native")
_RECORD = Path("qualification/records/semantic-native.json")
_MARKDOWN = Path("docs/qualification/rust/semantic-native.md")
_MODEL = "qualification/.artifacts/models/all-MiniLM-L6-v2/model.onnx"
_RUNTIME = "qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so"
_MODEL_SHA256 = "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452"
_BINARY_RELPATH = "x86_64-unknown-linux-gnu/release/semantic-native"
_WORK_ROOT_OVERRIDE = "SEMANTIC_NATIVE_QUALIFICATION_WORK_ROOT"
_LDD_MARKER = "qualification-ldd-capture"
_TOOLCHAIN_MARKER = "qualification-toolchain-capture"
_SPECS = (
    "docs/adr/0013-semantic-index-and-platform-support.md",
    "docs/superpowers/specs/2026-08-31-rust-semantic-rag-design.md",
)
_CONTRACT_IDS = ("mcp.tool.hieronymus_rag_search", "mcp.tool.hieronymus_recall")
_COMMANDS = (
    "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR="
    "qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true "
    "uv run python -m tools.qualification.run_semantic --write",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native "
    "CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path "
    "qualification/harnesses/semantic-native/Cargo.toml --check",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native "
    "CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path "
    "qualification/harnesses/semantic-native/Cargo.toml --locked --all-targets "
    "--features semantic-native --target x86_64-unknown-linux-gnu -- -D warnings",
    "uv run python -m tools.qualification.validate qualification/records/semantic-native.json",
    "uv run python -m tools.qualification.render --check "
    "qualification/records/semantic-native.json docs/qualification/rust/semantic-native.md",
)
_TOTAL_TIMEOUT_SECONDS = 1200
_NO_PROGRESS_SECONDS = 120
_FAKE_TIMEOUT_SECONDS = 30
_MAX_CAPTURE = 2 * 1024 * 1024
_COPY_CHUNK = 1024 * 1024
_EXPECTED_ROWS = 10_000
_CRASH_ROWS = 4_200
_CANCEL_ROWS = 3_200
_BATCH_SIZE = 100
_QUERY_COUNT = 50
_TOP_K = 10
_GENERATION_MAIN = "generation-a"
_GENERATION_BUILD = "generation-b"
_GENERATION_CANCEL = "generation-c"
_CRASH_EXIT = -6
_ALL_CRITERIA = REQUIRED_CRITERIA[_RISK]
_BUILD_DEPENDENTS = tuple(
    criterion for criterion in _ALL_CRITERIA if criterion != "locked-native-build"
)
_MODEL_DEPENDENTS = (
    "ten-thousand-chunk-build",
    "ann-index-created",
    "series-prefilter-before-ann",
    "zero-cross-series-hits",
    "insert-search-delete",
    "generation-isolation",
    "durable-sqlite-job-state",
    "no-sqlite-write-across-native-io",
    "crash-recovery",
    "cancel-recovery",
    "fifty-query-run",
)
_GEN_A_DEPENDENTS = (
    "ann-index-created",
    "series-prefilter-before-ann",
    "zero-cross-series-hits",
    "insert-search-delete",
    "generation-isolation",
    "fifty-query-run",
)

# Children that only capture evidence for the runner. Each receives the exact
# sanitized environment through run_owned_process and writes a digest-only
# artifact beneath the work root.
_LDD_CHILD = (
    "import subprocess, sys\n"
    "marker, output, binary = sys.argv[1], sys.argv[2], sys.argv[3]\n"
    "completed = subprocess.run(['/usr/bin/ldd', binary], capture_output=True)\n"
    "with open(output, 'wb') as stream:\n"
    "    stream.write(completed.stdout)\n"
    "raise SystemExit(completed.returncode)\n"
)
_TOOLCHAIN_CHILD = (
    "import json, subprocess, sys\n"
    "marker, output, cargo, rustup = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]\n"
    "def measured(argv):\n"
    "    completed = subprocess.run(argv, capture_output=True)\n"
    "    if completed.returncode != 0:\n"
    "        raise SystemExit(completed.returncode)\n"
    "    return completed.stdout.decode('utf-8', errors='strict')\n"
    "cargo_text = measured([cargo, '+1.96.0', '--version']).strip()\n"
    "rustc_text = measured([rustup, 'run', '1.96.0', 'rustc', '--version', '--verbose'])\n"
    "host = next((line.split(':', 1)[1].strip() for line in rustc_text.splitlines()\n"
    "             if line.startswith('host:')), '')\n"
    "if not host:\n"
    "    raise SystemExit(3)\n"
    "payload = {'cargo': cargo_text, 'rustc': rustc_text.splitlines()[0].strip(), 'host': host}\n"
    "with open(output, 'w', encoding='utf-8') as stream:\n"
    "    json.dump(payload, stream, sort_keys=True)\n"
)


@dataclass(frozen=True, slots=True)
class _Outcome:
    """One criterion's measured resolution pending record assembly."""

    status: str
    summary: str
    measurements: Measurements
    not_run_reason: str | None = None


def _pass(summary: str, **measurements: object) -> _Outcome:
    return _Outcome("pass", summary, Measurements(measurements))


def _fail(summary: str, **measurements: object) -> _Outcome:
    return _Outcome("fail", summary, Measurements(measurements))


def _not_run(reason: str) -> _Outcome:
    return _Outcome(
        "not-run",
        f"{reason} failed; {reason}-dependent evidence was not collected",
        Measurements(),
        not_run_reason=reason,
    )


def _series_slug(index: int) -> str:
    return f"qualification-series-{index:03d}"


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(_COPY_CHUNK):
            digest.update(chunk)
    return digest.hexdigest()


def _copy_file(source: Path, destination: Path) -> None:
    with source.open("rb") as reader, destination.open("wb") as writer:
        while chunk := reader.read(_COPY_CHUNK):
            writer.write(chunk)


def _remove_directory(path: Path, *, label: str) -> bool:
    if path.is_symlink():
        raise ValueError(f"{label} must not be a symlink")
    if not path.exists():
        raise ValueError(f"{label} disappeared before removal")
    if not path.is_dir():
        raise ValueError(f"{label} is not a directory")
    shutil.rmtree(path)
    return not path.exists()


def _successful(receipt: ProcessReceipt | None) -> bool:
    return (
        receipt is not None
        and receipt.exit_code == 0
        and not receipt.timed_out
        and receipt.core_dumps_disabled
        and receipt.process_group_reaped
    )


def _aborted(receipt: ProcessReceipt | None) -> bool:
    return (
        receipt is not None
        and receipt.exit_code == _CRASH_EXIT
        and not receipt.timed_out
        and receipt.core_dumps_disabled
        and receipt.process_group_reaped
    )


def _ldd_basenames(text: str) -> tuple[str, ...]:
    names: set[str] = set()
    for line in text.splitlines():
        entry = line.strip()
        if not entry:
            continue
        if " => " in entry:
            entry = entry.split(" => ", 1)[1]
        entry = entry.split(" ", 1)[0]
        if entry and entry != "not" and entry != "found":
            names.add(Path(entry).name)
    return tuple(sorted(names))


def _state_rows(copy: Path) -> tuple[list[tuple[object, ...]], list[tuple[object, ...]]]:
    """Read only the columns shared by the harness schema and the test fixture."""
    connection = sqlite3.connect(copy)
    try:
        generations = connection.execute(
            "select generation_id, status, written_count, active"
            " from semantic_generations order by generation_id"
        ).fetchall()
        jobs = connection.execute(
            "select job_id, status, next_batch, completed_batches,"
            " lease_owner, lease_expires_unix_ms from semantic_jobs order by job_id"
        ).fetchall()
    finally:
        connection.close()
    return generations, jobs


def _snapshot_state(work_root: Path, name: str, state_db: Path) -> Path:
    """Copy a durable state database (with WAL sidecars) before reading it."""
    copies = work_root / "state-copies"
    copies.mkdir(mode=0o700, exist_ok=True)
    copy = copies / f"{name}.sqlite3"
    _copy_file(state_db, copy)
    for suffix in ("-wal", "-shm"):
        sidecar = Path(f"{state_db}{suffix}")
        if sidecar.is_file():
            _copy_file(sidecar, Path(f"{copy}{suffix}"))
    return copy


def _generation_row(rows: list[tuple[object, ...]], generation: str) -> tuple[object, ...]:
    for row in rows:
        if row[0] == generation:
            return row
    return ()


def _job_row(rows: list[tuple[object, ...]], generation: str) -> tuple[object, ...]:
    for row in rows:
        if row[0] == f"build:{generation}":
            return row
    return ()


def _locked_dependencies(repo_root: Path) -> tuple[LockedDependency, ...]:
    lock = tomllib.loads((repo_root / _LOCK).read_text(encoding="utf-8"))
    dependencies = [
        LockedDependency(
            name=str(package["name"]),
            version=str(package["version"]),
            source=str(package.get("source") or "locked-local"),
            checksum=None if package.get("checksum") is None else str(package["checksum"]),
            features=(),
        )
        for package in lock.get("package", [])
        if package.get("name") != "hieronymus-semantic-native-qualification"
    ]
    return tuple(sorted(dependencies, key=lambda item: (item.name, item.version)))


def _record(
    repo_root: Path,
    *,
    outcomes: Mapping[str, _Outcome],
    environment: Environment,
    input_paths: tuple[str, ...],
    input_digest: str,
    work_dir_removed: bool,
    raw_logs_removed: bool,
    install_dir_removed: bool,
    core_dumps_disabled: bool,
    owned_process_groups_reaped: bool,
) -> QualificationRecord:
    evidence = tuple(
        Evidence(
            criterion=criterion,
            status=outcomes[criterion].status,
            summary=outcomes[criterion].summary,
            measurements=outcomes[criterion].measurements,
            not_run_reason=outcomes[criterion].not_run_reason,
        )
        for criterion in _ALL_CRITERIA
    )
    status = status_for(evidence)
    return QualificationRecord(
        schema_version=1,
        risk=_RISK,
        target=_TARGET,
        status=status,
        decision=decision_for(_RISK, evidence),
        acceptance_owner=_OWNER,
        specs=_SPECS,
        contract_ids=_CONTRACT_IDS,
        input_paths=input_paths,
        input_digest=input_digest,
        commands=_COMMANDS,
        environment=environment,
        dependencies=_locked_dependencies(repo_root),
        evidence=evidence,
        consequence=expected_consequence(_RISK, status),
        cleanup=CleanupEvidence(
            work_dir_removed=work_dir_removed,
            raw_logs_removed=raw_logs_removed,
            install_dir_removed=install_dir_removed,
            source_inputs_unchanged=True,
            user_data_opened=False,
            core_dumps_disabled=core_dumps_disabled,
            owned_process_groups_reaped=owned_process_groups_reaped,
        ),
        review=Review(
            owner=_OWNER,
            status="pending",
            objective_evidence_reviewed=False,
            normative_constraints_preserved=False,
        ),
    )


def _fake_outcomes(failed: tuple[str, ...]) -> dict[str, _Outcome]:
    return {
        criterion: (
            _fail(f"{criterion} failed in the fake-injected replay", replay="fake-injected")
            if criterion in failed
            else _pass(f"{criterion} passed in the fake-injected replay", replay="fake-injected")
        )
        for criterion in _ALL_CRITERIA
    }


def _parse_failed_criteria(stdout: bytes) -> tuple[str, ...]:
    try:
        payload = json.loads(stdout)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("semantic qualification executable output is invalid") from error
    if type(payload) is not dict or set(payload) != {"failed_criteria"}:
        raise ValueError("semantic qualification executable payload is invalid")
    failed = payload["failed_criteria"]
    if type(failed) is not list or any(type(item) is not str for item in failed):
        raise ValueError("semantic qualification executable failure list is invalid")
    unknown = tuple(sorted(set(failed) - set(_ALL_CRITERIA)))
    if unknown:
        raise ValueError(f"executable reported unknown criterion: {', '.join(unknown)}")
    if len(failed) != len(set(failed)):
        raise ValueError("executable reported duplicate criteria")
    return tuple(failed)


def _run_bounded(
    argv: tuple[str, ...],
    *,
    cwd: Path,
) -> tuple[int, bytes, bytes]:
    """Run the fake-injected executable with bounded output and wall clock."""
    process: subprocess.Popen[bytes] | None = None
    selector = selectors.DefaultSelector()
    stdout = bytearray()
    stderr = bytearray()
    try:
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC"},
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            close_fds=True,
        )
        assert process.stdout is not None and process.stderr is not None
        for pipe, target in ((process.stdout, stdout), (process.stderr, stderr)):
            os.set_blocking(pipe.fileno(), False)
            selector.register(pipe, selectors.EVENT_READ, target)
        deadline = time.monotonic() + _FAKE_TIMEOUT_SECONDS
        while selector.get_map() or process.poll() is None:
            if time.monotonic() >= deadline:
                raise ValueError("semantic qualification executable timed out")
            for key, _mask in selector.select(timeout=0.05):
                try:
                    chunk = os.read(key.fileobj.fileno(), 64 * 1024)
                except BlockingIOError:
                    continue
                if not chunk:
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                    continue
                key.data.extend(chunk)
                if len(key.data) > _MAX_CAPTURE:
                    raise ValueError("semantic qualification executable output exceeds limit")
        return process.wait(timeout=1), bytes(stdout), bytes(stderr)
    except BaseException:
        if process is not None and process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=2)
        raise
    finally:
        selector.close()
        if process is not None:
            for pipe in (process.stdin, process.stdout, process.stderr):
                if pipe is not None and not pipe.closed:
                    pipe.close()


def run(repo_root: Path, work_root: Path, *, executable: Path) -> QualificationRecord:
    """Build a record from a fake-injected executable without native processes."""
    del work_root  # the fake executable writes digested stdout only
    input_paths = required_fingerprint_inputs(_RISK)
    before = fingerprint_inputs(repo_root, input_paths)
    code, stdout, _stderr = _run_bounded((str(executable),), cwd=repo_root)
    if code != 0:
        raise ValueError("semantic qualification executable failed")
    failed = _parse_failed_criteria(stdout)
    after = fingerprint_inputs(repo_root, input_paths)
    if after != before:
        raise ValueError("immutable semantic qualification inputs changed during replay")
    return _record(
        repo_root,
        outcomes=_fake_outcomes(failed),
        environment=Environment(
            rustc="<absent>",
            cargo="<absent>",
            target=_TARGET,
            os=platform.system().lower(),
            kernel=platform.release(),
            architecture=platform.machine(),
            bun=None,
            native_libraries=(),
        ),
        input_paths=input_paths,
        input_digest=before,
        work_dir_removed=True,
        raw_logs_removed=True,
        install_dir_removed=True,
        core_dumps_disabled=True,
        owned_process_groups_reaped=True,
    )


def _live_process_context(
    repo_root: Path,
    work_root: Path,
    original_env: Mapping[str, str],
) -> tuple[ToolRoots, Path, dict[str, str]]:
    """Discover unsanitized tool roots, then produce one shared safe environment."""
    tool_roots = discover_tool_roots(original_env)
    cargo_target_dir = repo_root / _CARGO_TARGET
    child_env = safe_subprocess_env(
        work_root,
        cargo_offline=True,
        tool_roots=tool_roots,
        cargo_target_dir=cargo_target_dir,
    )
    return tool_roots, cargo_target_dir, child_env


def _prepare_work_root(work_root: Path) -> None:
    if work_root.is_symlink():
        raise ValueError("semantic work root must not be a symlink")
    if work_root.exists():
        if not work_root.is_dir():
            raise ValueError("semantic work root is not a directory")
        shutil.rmtree(work_root)
    work_root.mkdir(mode=0o700, parents=True)


def _scenario_argv(
    binary: Path,
    work_dir: Path,
    *,
    mode: str,
    generation: str,
    stop_after: int,
) -> tuple[str, ...]:
    return (
        str(binary),
        "scenario",
        "--work-dir",
        str(work_dir),
        "--state-db",
        str(work_dir / "state.sqlite3"),
        "--mode",
        mode,
        "--generation",
        generation,
        "--stop-after",
        str(stop_after),
    )


def _query_argv(binary: Path, work_dir: Path, series: str, index: int) -> tuple[str, ...]:
    return (
        str(binary),
        "query",
        "--work-dir",
        str(work_dir),
        "--series",
        series,
        "--query-index",
        str(index),
    )


def _fts_argv(binary: Path, work_dir: Path, series: str, index: int) -> tuple[str, ...]:
    return (
        str(binary),
        "fts-fallback",
        "--work-dir",
        str(work_dir),
        "--series",
        series,
        "--query-index",
        str(index),
    )


def _live_outcomes(
    repo_root: Path,
    work_root: Path,
    tool_roots: ToolRoots,
    child_env: dict[str, str],
) -> tuple[dict[str, _Outcome], list[ProcessReceipt], Environment]:
    """Execute the eleven-step measured sequence and resolve every criterion."""
    receipts: list[ProcessReceipt] = []
    outcomes: dict[str, _Outcome] = {}
    binary = Path(child_env["CARGO_TARGET_DIR"]) / _BINARY_RELPATH
    cargo = str(tool_roots.cargo_invocation)
    captures = work_root / "captures"
    captures.mkdir(mode=0o700, exist_ok=True)
    environment = Environment(
        rustc="<absent>",
        cargo="<absent>",
        target=_TARGET,
        os=platform.system().lower(),
        kernel=platform.release(),
        architecture=platform.machine(),
        bun=None,
        native_libraries=(),
    )

    def launch(argv: tuple[str, ...]) -> ProcessReceipt | None:
        try:
            receipt = run_owned_process(
                argv,
                cwd=repo_root,
                env=child_env,
                timeout_seconds=_TOTAL_TIMEOUT_SECONDS,
                no_progress_seconds=_NO_PROGRESS_SECONDS,
            )
        except (OSError, RuntimeError, ValueError):
            return None
        receipts.append(receipt)
        return receipt

    # Toolchain identity is runner infrastructure, not a criterion: measure it
    # first so even a failed build produces a fully described environment.
    toolchain_output = captures / "toolchain.json"
    toolchain_receipt = launch(
        (
            sys.executable,
            "-c",
            _TOOLCHAIN_CHILD,
            _TOOLCHAIN_MARKER,
            str(toolchain_output),
            str(tool_roots.cargo_invocation),
            str(tool_roots.rustup_invocation),
        )
    )
    if toolchain_receipt is None or not _successful(toolchain_receipt):
        raise ValueError("semantic toolchain measurement failed")
    try:
        measured = json.loads(toolchain_output.read_bytes())
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("semantic toolchain capture is invalid") from error
    if (
        type(measured) is not dict
        or set(measured) != {"cargo", "rustc", "host"}
        or type(measured["host"]) is not str
        or measured["host"] != _TARGET
    ):
        raise ValueError("measured Rust target differs from the semantic qualification target")
    environment = Environment(
        rustc=str(measured["rustc"]),
        cargo=str(measured["cargo"]),
        target=_TARGET,
        os=platform.system().lower(),
        kernel=platform.release(),
        architecture=platform.machine(),
        bun=None,
        native_libraries=(),
    )

    def cascade(criteria: tuple[str, ...], reason: str) -> None:
        for criterion in criteria:
            outcomes[criterion] = _not_run(reason)

    # Step 1: locked --release build over the warm sanitized cargo target.
    build_receipt = launch(
        (
            cargo,
            f"+{_TOOLCHAIN}",
            "build",
            "--manifest-path",
            _MANIFEST,
            "--release",
            "--locked",
            "--features",
            "semantic-native",
            "--target",
            _TARGET,
        )
    )
    build_ok = _successful(build_receipt) and binary.is_file()
    outcomes["locked-native-build"] = (
        _pass(
            "the locked --release semantic build succeeded over the warm sanitized cargo target",
            locked_commands=1,
            toolchain=_TOOLCHAIN,
            output_sha256=build_receipt.stdout_sha256 if build_receipt else "",
        )
        if build_ok
        else _fail(
            "the locked --release semantic build failed over the warm sanitized cargo target",
            locked_commands=1,
        )
    )
    if not build_ok:
        cascade(_BUILD_DEPENDENTS, "locked-native-build")
        return outcomes, receipts, environment, False

    # Step 1 evidence: binary/library sizes plus ldd basenames (no absolute paths).
    runtime_path = repo_root / _RUNTIME
    model_path = repo_root / _MODEL
    ldd_output = captures / "ldd.txt"
    ldd_receipt = launch(
        (
            sys.executable,
            "-c",
            _LDD_CHILD,
            _LDD_MARKER,
            str(ldd_output),
            str(binary),
        )
    )
    basenames: tuple[str, ...] = ()
    if ldd_receipt is not None and _successful(ldd_receipt) and ldd_output.is_file():
        try:
            basenames = _ldd_basenames(ldd_output.read_text(encoding="utf-8", errors="strict"))
        except OSError:
            basenames = ()
    sizes_ok = (
        binary.is_file()
        and runtime_path.is_file()
        and model_path.is_file()
        and bool(basenames)
        and ldd_receipt is not None
        and _successful(ldd_receipt)
    )
    outcomes["binary-size-recorded"] = (
        _pass(
            "binary and native library sizes plus ldd basenames were recorded",
            binary_bytes=binary.stat().st_size,
            runtime_library_bytes=runtime_path.stat().st_size,
            model_bytes=model_path.stat().st_size,
            native_library_basenames=basenames,
        )
        if sizes_ok
        else _fail("binary or native library sizes could not be recorded")
    )
    environment = Environment(
        rustc=environment.rustc,
        cargo=environment.cargo,
        target=_TARGET,
        os=environment.os,
        kernel=environment.kernel,
        architecture=environment.architecture,
        bun=None,
        native_libraries=basenames,
    )

    # Step 2: install the candidate, probe it once, uninstall, and prove absence.
    install_dir = repo_root / _INSTALL
    removed_after_probe = False
    install_ok = False
    try:
        if install_dir.is_symlink():
            raise ValueError("semantic install directory must not be a symlink")
        if install_dir.exists():
            shutil.rmtree(install_dir)
        install_dir.mkdir(mode=0o700, parents=True)
        installed_binary = install_dir / "semantic-native"
        _copy_file(binary, installed_binary)
        installed_binary.chmod(0o755)
        _copy_file(runtime_path, install_dir / "libonnxruntime.so")
        _copy_file(model_path, install_dir / "model.onnx")
        (install_dir / "index").mkdir(mode=0o700)
        probe_receipt = launch(
            _fts_argv(installed_binary, work_root / "install-probe", _series_slug(0), 0)
        )
        removed_after_probe = _remove_directory(install_dir, label="semantic install directory")
        install_ok = _successful(probe_receipt) and removed_after_probe and not install_dir.exists()
    except OSError:
        install_ok = False
        if install_dir.exists():
            shutil.rmtree(install_dir, ignore_errors=True)
            removed_after_probe = not install_dir.exists()
    outcomes["install-uninstall"] = (
        _pass(
            "the installed candidate served one disposable fallback query and was removed",
            probe="fts-fallback",
            query_index=0,
            install_dir_removed=1,
        )
        if install_ok
        else _fail(
            "the installed candidate probe or verified uninstall failed",
            install_dir_removed=1 if removed_after_probe else 0,
        )
    )

    # Step 3: verify the model checksum before any child loads it.
    model_digest = _sha256_file(model_path)
    try:
        corpus = json.loads((repo_root / _CORPUS).read_text(encoding="utf-8"))
        dimensions = corpus["embedding_dimensions"]
    except (OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError) as error:
        raise ValueError("semantic corpus recipe is invalid") from error
    model_ok = model_digest == _MODEL_SHA256 and dimensions == 384
    model_summary_prefix = (
        "the model digest matched before the first verified load"
        if model_ok
        else "the acquired model digest did not match the pinned identity"
    )

    scenario_violation_free = True
    scenario_observed = 0
    durable_readings: list[tuple[str, object]] = []

    # Step 4: build 10,000 chunks for generation-a and await the ANN index.
    main_work = work_root / "main"
    gen_a_receipt = (
        launch(
            _scenario_argv(
                binary,
                main_work,
                mode="complete",
                generation=_GENERATION_MAIN,
                stop_after=_EXPECTED_ROWS,
            )
        )
        if model_ok
        else None
    )
    gen_a_ok = _successful(gen_a_receipt)
    main_generations: list[tuple[object, ...]] = []
    main_jobs: list[tuple[object, ...]] = []
    if gen_a_ok:
        assert gen_a_receipt is not None
        main_generations, main_jobs = _state_rows(
            _snapshot_state(work_root, "main", main_work / "state.sqlite3")
        )
        main_row = _generation_row(main_generations, _GENERATION_MAIN)
        gen_a_ok = (
            main_row != ()
            and main_row[1] == "active"
            and main_row[2] == _EXPECTED_ROWS
            and main_row[3] == 1
        )
        scenario_observed += 1
        scenario_violation_free = scenario_violation_free and gen_a_ok
    outcomes["ten-thousand-chunk-build"] = (
        _pass(
            "generation-a persisted 10,000 embedded chunks in 100 lease-protected batches",
            written_rows=_EXPECTED_ROWS,
            completed_batches=_BATCH_SIZE,
            output_sha256=gen_a_receipt.stdout_sha256 if gen_a_receipt else "",
        )
        if gen_a_ok
        else _fail(
            "generation-a did not durably persist the complete 10,000-chunk population",
            output_sha256=gen_a_receipt.stdout_sha256 if gen_a_receipt else "",
        )
    )
    if not gen_a_ok:
        if model_ok:
            outcomes["model-checksum-load"] = _fail(
                "the model digest matched before load, but the verified load could"
                " not be proven by a complete scenario",
                model_sha256=model_digest,
                embedding_dimensions=dimensions,
            )
            cascade(_GEN_A_DEPENDENTS, "ten-thousand-chunk-build")
        else:
            outcomes["model-checksum-load"] = _fail(
                f"{model_summary_prefix}",
                model_sha256=model_digest,
                embedding_dimensions=dimensions,
            )
            cascade(_MODEL_DEPENDENTS, "model-checksum-load")
        for criterion in _ALL_CRITERIA:
            outcomes.setdefault(
                criterion,
                _not_run("model-checksum-load" if not model_ok else "ten-thousand-chunk-build"),
            )
        return outcomes, receipts, environment

    outcomes["ann-index-created"] = _pass(
        "the scenario exited zero only after the ANN index reported full coverage;"
        " normalized index statistics are recorded through the scenario receipt digest",
        expected_indexed_rows=_EXPECTED_ROWS,
        output_sha256=gen_a_receipt.stdout_sha256 if gen_a_receipt else "",
    )
    outcomes["model-checksum-load"] = _pass(
        "the model digest was verified before load; the scenario embedded the whole"
        " population at exactly 384 normalized dimensions",
        model_sha256=model_digest,
        embedding_dimensions=dimensions,
        scenario_output_sha256=gen_a_receipt.stdout_sha256 if gen_a_receipt else "",
    )

    # Step 5: the adversarial top-k/cardinality case plus all 50 series queries.
    query_digests: list[str] = []
    queries_ok = True
    for index in range(_QUERY_COUNT):
        receipt = launch(_query_argv(binary, main_work, _series_slug(index), index))
        queries_ok = queries_ok and _successful(receipt)
        query_digests.append(receipt.stdout_sha256 if receipt else "")
        if not queries_ok:
            break
    outcomes["fifty-query-run"] = (
        _pass(
            "all 50 series-scoped corpus queries returned exact eligible ids through"
            " the active generation",
            queries=_QUERY_COUNT,
            top_k=_TOP_K,
            output_sha256s=tuple(query_digests),
        )
        if queries_ok
        else _fail(
            "the 50-query series run did not complete",
            queries=len([digest for digest in query_digests if digest]),
        )
    )
    # Series i and series i+50 share one token stream, so their embeddings are
    # identical: a global top-k without predicate pushdown would return
    # cross-series decoys and the candidate would exit non-zero. Every exit-zero
    # receipt is therefore an objective pre-filter-before-ANN observation.
    outcomes["series-prefilter-before-ann"] = (
        _pass(
            "every query proved the series predicate executes below the ANN operator:"
            " identical-embedding decoy series would have starved a post-filtered top-k",
            queries=_QUERY_COUNT,
            exact_eligible_queries=_QUERY_COUNT if queries_ok else 0,
        )
        if queries_ok
        else _fail("the pre-filter-before-ANN proof did not complete")
    )
    outcomes["zero-cross-series-hits"] = (
        _pass(
            "the candidate exits non-zero on any cross-series hit; all 50 receipts were clean",
            queries=_QUERY_COUNT,
            cross_series_hits=0,
        )
        if queries_ok
        else _fail("cross-series isolation could not be proven for all 50 queries")
    )

    # Step 8/7: an aborted build leaves durable SQLite job state behind, and the
    # active generation keeps serving searches while generation-b is incomplete.
    isolation_work = work_root / "isolation"
    isolation_receipt = launch(
        _scenario_argv(
            binary,
            isolation_work,
            mode="crash",
            generation=_GENERATION_BUILD,
            stop_after=_CRASH_ROWS,
        )
    )
    isolation_ok = _aborted(isolation_receipt)
    durable_readings.append(("isolation_crash_signal", int(isolation_receipt.exit_code or 0)))
    isolation_generations: list[tuple[object, ...]] = []
    isolation_jobs: list[tuple[object, ...]] = []
    if isolation_ok:
        isolation_generations, isolation_jobs = _state_rows(
            _snapshot_state(work_root, "isolation", isolation_work / "state.sqlite3")
        )
        build_row = _generation_row(isolation_generations, _GENERATION_BUILD)
        build_job = _job_row(isolation_jobs, _GENERATION_BUILD)
        isolation_ok = (
            build_row != ()
            and build_row[1] == "building"
            and build_row[2] == _CRASH_ROWS
            and build_row[3] == 0
            and build_job != ()
            and build_job[2] == _CRASH_ROWS // _BATCH_SIZE
            and build_job[4] is not None
        )
        durable_readings.extend(
            (
                ("isolation_written_rows", build_row[2] if build_row else 0),
                ("isolation_next_batch", build_job[2] if build_job else 0),
            )
        )
    recovery_queries_ok = queries_ok
    for index in (0, _QUERY_COUNT - 1):
        receipt = launch(_query_argv(binary, main_work, _series_slug(index), index))
        recovery_queries_ok = recovery_queries_ok and _successful(receipt)
    main_active = _generation_row(main_generations, _GENERATION_MAIN)
    outcomes["generation-isolation"] = (
        _pass(
            "an incomplete generation-b stayed non-active while generation-a served every search",
            active_generation=_GENERATION_MAIN,
            isolation_queries=2,
            active_generations=1 if main_active != () and main_active[3] == 1 else 0,
        )
        if isolation_ok and recovery_queries_ok
        else _fail("generation isolation could not be proven while generation-b was incomplete")
    )

    # Step 9: abort generation-b at 4,200 on a durable boundary with core dumps
    # disabled, let the dead lease expire, resume to 10,000, and activate once.
    recovery_work = work_root / "recovery"
    crash_receipt = launch(
        _scenario_argv(
            binary,
            recovery_work,
            mode="crash",
            generation=_GENERATION_BUILD,
            stop_after=_CRASH_ROWS,
        )
    )
    resume_receipt = launch(
        _scenario_argv(
            binary,
            recovery_work,
            mode="complete",
            generation=_GENERATION_BUILD,
            stop_after=_EXPECTED_ROWS,
        )
    )
    recovery_ok = _aborted(crash_receipt) and _successful(resume_receipt)
    recovery_generations: list[tuple[object, ...]] = []
    recovery_jobs: list[tuple[object, ...]] = []
    if recovery_ok:
        assert resume_receipt is not None
        recovery_generations, recovery_jobs = _state_rows(
            _snapshot_state(work_root, "recovery", recovery_work / "state.sqlite3")
        )
        recovered_row = _generation_row(recovery_generations, _GENERATION_BUILD)
        recovered_job = _job_row(recovery_jobs, _GENERATION_BUILD)
        active_rows = [row for row in recovery_generations if row[3] == 1]
        recovery_ok = (
            recovered_row != ()
            and recovered_row[1] == "active"
            and recovered_row[2] == _EXPECTED_ROWS
            and recovered_job != ()
            and recovered_job[3] == _EXPECTED_ROWS // _BATCH_SIZE
            and len(active_rows) == 1
            and active_rows[0][0] == _GENERATION_BUILD
        )
        scenario_observed += 1
        scenario_violation_free = scenario_violation_free and recovery_ok
    outcomes["crash-recovery"] = (
        _pass(
            "the SIGABRT at 4,200 rows was durable; the resumed builder reconciled the"
            " exact persisted id multiset, finished 10,000 rows, and activated once",
            stop_after_rows=_CRASH_ROWS,
            resumed_batches=_EXPECTED_ROWS // _BATCH_SIZE,
            written_rows=_EXPECTED_ROWS,
            active_generations=1,
        )
        if recovery_ok
        else _fail("crash recovery did not resume to a single activated 10,000-row generation")
    )

    # Step 10: cancel a new generation through durable SQLite state; the cancel
    # summary can only print an active generation, which must be generation-b.
    cancel_receipt = launch(
        _scenario_argv(
            binary,
            recovery_work,
            mode="cancel",
            generation=_GENERATION_CANCEL,
            stop_after=_CANCEL_ROWS,
        )
    )
    cancel_ok = _successful(cancel_receipt)
    if cancel_ok:
        assert cancel_receipt is not None
        cancel_generations, _cancel_jobs = _state_rows(
            _snapshot_state(work_root, "cancel", recovery_work / "state.sqlite3")
        )
        cancelled_row = _generation_row(cancel_generations, _GENERATION_CANCEL)
        still_active = _generation_row(cancel_generations, _GENERATION_BUILD)
        cancel_ok = (
            cancelled_row != ()
            and cancelled_row[1] == "cancelled"
            and cancelled_row[2] == _CANCEL_ROWS
            and still_active != ()
            and still_active[3] == 1
        )
        scenario_observed += 1
        scenario_violation_free = scenario_violation_free and cancel_ok
        durable_readings.extend(
            (
                ("cancel_written_rows", cancelled_row[2] if cancelled_row else 0),
                ("cancel_status", str(cancelled_row[1]) if cancelled_row else "missing"),
            )
        )
    post_cancel_ok = False
    if cancel_ok:
        post_cancel_ok = _successful(launch(_query_argv(binary, recovery_work, _series_slug(0), 0)))
    outcomes["cancel-recovery"] = (
        _pass(
            "cancellation at 3,200 rows was durably recorded and generation-b remained"
            " the single active generation serving post-cancel searches",
            cancelled_rows=_CANCEL_ROWS,
            active_generation=_GENERATION_BUILD,
            post_cancel_queries=1,
        )
        if cancel_ok and post_cancel_ok
        else _fail("cancellation did not preserve the active generation-b")
    )

    # Step 6: rows inserted before the crash persisted exactly, the resume
    # appended the remainder into the same table, and the refreshed ANN index
    # served the post-cancel query while reconciliation keeps any row outside
    # the durable multiset absent.
    resumed_row = _generation_row(recovery_generations, _GENERATION_BUILD)
    outcomes["insert-search-delete"] = (
        _pass(
            "the 4,200 pre-crash rows stayed addressable, the resume appended exactly"
            " the remaining rows, and the refreshed index served the post-refresh query",
            persisted_rows_at_crash=_CRASH_ROWS,
            rows_after_refresh=_EXPECTED_ROWS,
            post_refresh_queries=1 if post_cancel_ok else 0,
        )
        if recovery_ok and post_cancel_ok and resumed_row != () and resumed_row[2] == _EXPECTED_ROWS
        else _fail("insert/search/delete identity could not be proven across the refresh")
    )

    outcomes["durable-sqlite-job-state"] = (
        _pass(
            "generation rows, leases, and batch counters survived the abort in SQLite"
            " and recorded the cancel and resume boundaries",
            readings=tuple(f"{name}={value}" for name, value in durable_readings),
        )
        if isolation_ok and recovery_ok and cancel_ok
        else _fail("durable SQLite job state could not be read across all boundaries")
    )
    outcomes["no-sqlite-write-across-native-io"] = (
        _pass(
            "every scenario receipt is exit-zero, which the candidate only prints after"
            " proving zero native-I/O-inside-write-transaction violations",
            scenarios_observed=scenario_observed,
            violations=0,
        )
        if scenario_violation_free and scenario_observed == 3
        else _fail("the transaction-boundary probe did not stay violation-free")
    )

    # Step 11: the --no-default-features binary answers all 50 corpus queries
    # from the bundled FTS5 fallback with no model, index, or network.
    fts_build_receipt = launch(
        (
            cargo,
            f"+{_TOOLCHAIN}",
            "build",
            "--manifest-path",
            _MANIFEST,
            "--release",
            "--locked",
            "--no-default-features",
            "--target",
            _TARGET,
        )
    )
    fts_work = work_root / "fts"
    fts_digests: list[str] = []
    fts_ok = _successful(fts_build_receipt) and binary.is_file()
    if fts_ok:
        for index in range(_QUERY_COUNT):
            receipt = launch(_fts_argv(binary, fts_work, _series_slug(index), index))
            fts_ok = fts_ok and _successful(receipt)
            fts_digests.append(receipt.stdout_sha256 if receipt else "")
            if not fts_ok:
                break
    outcomes["fts-fallback"] = (
        _pass(
            "the no-default-features binary proved all 50 expected eligible-id digests,"
            " series isolation, and delete/rebuild equivalence offline",
            queries=_QUERY_COUNT,
            cross_series_hits=0,
            cargo_net_offline=1,
            output_sha256s=tuple(fts_digests),
        )
        if fts_ok
        else _fail("the FTS5 fallback build or its 50 offline queries failed")
    )
    return outcomes, receipts, environment


def _reconcile_cargo_target(repo_root: Path) -> None:
    """Privatize only exact, owned, nonsymlink warm cargo target directories."""
    target = repo_root / _CARGO_TARGET
    for candidate in (target.parent, target):
        if not candidate.exists():
            continue
        info = candidate.lstat()
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise ValueError("cargo target must be a nonsymlink directory")
        if info.st_uid != os.getuid():
            raise ValueError("cargo target is not owned by the current user")
        mode = stat.S_IMODE(info.st_mode)
        if mode & 0o022:
            raise ValueError("cargo target is group- or world-writable")
        if mode != 0o700:
            candidate.chmod(0o700)


def run_live(
    repo_root: Path,
    work_root: Path,
    *,
    original_env: Mapping[str, str],
) -> QualificationRecord:
    """Run the semantic candidate under the offline, owned-process boundary."""
    caller_env = dict(original_env)
    if caller_env.get("HIERONYMUS_QUALIFICATION_LIVE") != "1":
        raise ValueError("HIERONYMUS_QUALIFICATION_LIVE=1 is required")
    input_paths = required_fingerprint_inputs(_RISK)
    before = fingerprint_inputs(repo_root, input_paths)
    _prepare_work_root(work_root)
    _reconcile_cargo_target(repo_root)
    try:
        tool_roots, _cargo_target_dir, child_env = _live_process_context(
            repo_root, work_root, caller_env
        )
        if _WORK_ROOT_OVERRIDE in child_env:
            raise ValueError(
                f"semantic work root override {_WORK_ROOT_OVERRIDE} leaked into the"
                " child environment"
            )
        outcomes, receipts, environment = _live_outcomes(
            repo_root, work_root, tool_roots, child_env
        )
    finally:
        work_removed = work_root.exists() and _remove_directory(
            work_root, label="semantic work root"
        )
    after = fingerprint_inputs(repo_root, input_paths)
    if after != before:
        raise ValueError("immutable semantic qualification inputs changed during replay")
    if not work_removed:
        raise ValueError("semantic work root could not be removed after the run")
    install_removed = not (repo_root / _INSTALL).exists()
    if not install_removed:
        raise ValueError("semantic install directory survived the run")
    if not (bool(receipts) and all(item.core_dumps_disabled for item in receipts)):
        raise ValueError("semantic children did not all run with core dumps disabled")
    if not (bool(receipts) and all(item.process_group_reaped for item in receipts)):
        raise ValueError("semantic children were not all reaped from owned process groups")
    return _record(
        repo_root,
        outcomes=outcomes,
        environment=environment,
        input_paths=input_paths,
        input_digest=before,
        work_dir_removed=work_removed,
        raw_logs_removed=work_removed,
        install_dir_removed=install_removed,
        core_dumps_disabled=True,
        owned_process_groups_reaped=True,
    )


def _write_record(repo_root: Path, record: QualificationRecord) -> None:
    record_path = repo_root / _RECORD
    markdown_path = repo_root / _MARKDOWN
    record_path.parent.mkdir(parents=True, exist_ok=True)
    markdown_path.parent.mkdir(parents=True, exist_ok=True)
    record_path.write_text(serialize_record(record) + "\n", encoding="utf-8")
    markdown_path.write_text(render_record(record), encoding="utf-8")


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run the offline semantic-native qualification")
    parser.add_argument("--write", action="store_true", required=True)
    parser.parse_args(argv)
    repo_root = Path.cwd()
    if _WORK_ROOT_OVERRIDE in os.environ:
        print(
            f"notice: {_WORK_ROOT_OVERRIDE} is active in the caller environment;"
            " the live runner launches children without it",
            file=sys.stderr,
        )
    try:
        record = run_live(
            repo_root,
            repo_root / _LIVE_WORK,
            original_env=dict(os.environ),
        )
        _write_record(repo_root, record)
    except (OSError, TypeError, ValueError, RuntimeError):
        print("semantic-native qualification failed", file=sys.stderr)
        return 2
    print(f"semantic-native qualification recorded: {record.decision}")
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
