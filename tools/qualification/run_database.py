"""Offline legacy-database-import qualification runner and canonical evidence writer."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import selectors
import shutil
import signal
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
from tools.qualification.projections import DATABASE_CONTRACT_IDS, projection_issues
from tools.qualification.render import render_record

_RISK = "legacy-database-import"
_OWNER = "Pavel Obruchnikov <me@inkyquill.net>"
_TARGET = "x86_64-unknown-linux-gnu"
_TOOLCHAIN = "1.96.0"
_MANIFEST = "qualification/harnesses/legacy-database-import/Cargo.toml"
_LOCK = "qualification/harnesses/legacy-database-import/Cargo.lock"
_PROJECTION = "qualification/compatibility/legacy-database-import.json"
_FIXTURE_ROOT = "compatibility/fixtures/database"
_CARGO_TARGET = Path("qualification/.artifacts/cargo-target/legacy-database-import")
_LIVE_WORK = Path("qualification/.artifacts/work/legacy-database-import")
_RECORD = Path("qualification/records/legacy-database-import.json")
_MARKDOWN = Path("docs/qualification/rust/legacy-database-import.md")
_BINARY_RELPATH = "x86_64-unknown-linux-gnu/release/legacy-database-import"
_SPECS = (
    "docs/adr/0010-data-locations-schema-ownership-and-upgrade.md",
    "docs/superpowers/specs/2026-08-31-rust-database-upgrade-design.md",
)
# ruff: noqa: E501 - canonical replay commands are byte-exact closed-grammar literals.
_COMMANDS = (
    "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR="
    "qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true "
    "uv run python -m tools.qualification.run_database --write",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import "
    "CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path "
    "qualification/harnesses/legacy-database-import/Cargo.toml --check",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import "
    "CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path "
    "qualification/harnesses/legacy-database-import/Cargo.toml --locked "
    "--all-targets --target x86_64-unknown-linux-gnu -- -D warnings",
    "uv run python -m tools.qualification.validate qualification/records/legacy-database-import.json",
    "uv run python -m tools.qualification.render --check "
    "qualification/records/legacy-database-import.json docs/qualification/rust/legacy-database-import.md",
)
_FIXTURES = (
    "minimal-python.sqlite",
    "legacy-python.sqlite",
    "empty.sqlite",
    "partial-python.sqlite",
    "corrupt.sqlite",
    "unknown-schema.sqlite",
)
_SUPPORTED_PROBES = ("minimal-python.sqlite", "legacy-python.sqlite")
_UNSUPPORTED_PROBES = (
    "empty.sqlite",
    "partial-python.sqlite",
    "corrupt.sqlite",
    "unknown-schema.sqlite",
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
_TOOLCHAIN_MARKER = "qualification-toolchain-capture"
_LDD_MARKER = "qualification-ldd-capture"
_HARNESS_MARKER = "qualification-database-capture"
_TOTAL_TIMEOUT_SECONDS = 1200
_NO_PROGRESS_SECONDS = 120
_FAKE_TIMEOUT_SECONDS = 30
_MAX_CAPTURE = 2 * 1024 * 1024
_COPY_CHUNK = 1024 * 1024
_FORBIDDEN_MARKERS = ("/home/", "Yandex.Disk")
_ALL_CRITERIA = REQUIRED_CRITERIA[_RISK]
_BUILD_CRITERION = "bundled-sqlite-fts5"
_BUILD_DEPENDENTS = tuple(criterion for criterion in _ALL_CRITERIA if criterion != _BUILD_CRITERION)
_CLASSIFICATION_DEPENDENTS = (
    "supported-current-read",
    "supported-legacy-read",
    "typed-row-accounting",
    "fts-query-equivalence",
    "ledger-preserved",
    "unsupported-fail-closed",
)

# Children that only capture evidence for the runner. Each receives the exact
# sanitized environment through run_owned_process and writes a digest-only
# artifact beneath the work root.
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
_LDD_CHILD = (
    "import subprocess, sys\n"
    "marker, output, binary = sys.argv[1], sys.argv[2], sys.argv[3]\n"
    "completed = subprocess.run(['/usr/bin/ldd', binary], capture_output=True)\n"
    "with open(output, 'wb') as stream:\n"
    "    stream.write(completed.stdout)\n"
    "raise SystemExit(completed.returncode)\n"
)
_HARNESS_CHILD = (
    "import subprocess, sys\n"
    "marker, output, binary = sys.argv[1], sys.argv[2], sys.argv[3]\n"
    "completed = subprocess.run([binary, *sys.argv[4:]], capture_output=True)\n"
    "with open(output, 'wb') as stream:\n"
    "    stream.write(completed.stdout)\n"
    "raise SystemExit(completed.returncode)\n"
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


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(_COPY_CHUNK):
            digest.update(chunk)
    return digest.hexdigest()


def _is_digest(value: object) -> bool:
    return (
        type(value) is str
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def _walk_regular_files(directory: Path, prefix: str) -> tuple[str, ...]:
    relatives: list[str] = []
    with os.scandir(directory) as entries:
        for entry in sorted(entries, key=lambda item: item.name):
            if entry.is_symlink():
                raise ValueError("the frozen fixture root must not contain symlinks")
            if entry.is_dir(follow_symlinks=False):
                relatives.extend(_walk_regular_files(Path(entry.path), f"{prefix}{entry.name}/"))
            elif entry.is_file(follow_symlinks=False):
                relatives.append(f"{prefix}{entry.name}")
            else:
                raise ValueError("the frozen fixture root must contain only directories and files")
    return tuple(relatives)


def _fixture_tree_digest(root: Path) -> str:
    """Deterministic digest over every regular file below the fixture root."""
    lines: list[str] = []
    for relative in _walk_regular_files(root, ""):
        lines.append(f"{relative}\x00{_sha256_file(root / relative)}")
    return hashlib.sha256("\n".join(lines).encode()).hexdigest()


def _successful(receipt: ProcessReceipt | None) -> bool:
    return (
        receipt is not None
        and receipt.exit_code == 0
        and not receipt.timed_out
        and receipt.core_dumps_disabled
        and receipt.process_group_reaped
    )


def _refused(receipt: ProcessReceipt | None) -> bool:
    """Whether one bounded child failed cleanly with the exact refusal signal."""
    return (
        receipt is not None
        and receipt.exit_code == 1
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


def _projection_document(repo_root: Path) -> dict[str, object]:
    """Require the current database projection and return its parsed document."""
    issues = projection_issues(repo_root)
    if issues["legacy-database-import"]:
        raise ValueError("legacy-database-import compatibility projection is not current")
    try:
        document = json.loads((repo_root / _PROJECTION).read_bytes())
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("database compatibility projection is invalid") from error
    if type(document) is not dict:
        raise ValueError("database compatibility projection must be an object")
    if document.get("projection_version") != 1 or document.get("risk") != _RISK:
        raise ValueError("database compatibility projection identity is invalid")
    contracts = document.get("contracts")
    if type(contracts) is not list:
        raise ValueError("database compatibility projection contracts are invalid")
    contract_ids = tuple(
        sorted(
            contract_id
            for contract in contracts
            if type(contract) is dict and type(contract_id := contract.get("id")) is str
        )
    )
    if contract_ids != tuple(sorted(DATABASE_CONTRACT_IDS)):
        raise ValueError("database compatibility projection contract ids are invalid")
    return document


def _expected_variants(document: Mapping[str, object]) -> dict[str, tuple[str, bool, str]]:
    """Parse the projected per-fixture expectations keyed by fixture basename."""
    database = document.get("database")
    variants = database.get("variants") if type(database) is dict else None
    if type(variants) is not list:
        raise ValueError("database compatibility projection variants are invalid")
    expected: dict[str, tuple[str, bool, str]] = {}
    for entry in variants:
        if type(entry) is not dict:
            raise ValueError("database compatibility projection variants are invalid")
        fixture = entry.get("fixture")
        variant_id = entry.get("id")
        expectation = entry.get("expected")
        if (
            type(fixture) is not str
            or type(variant_id) is not str
            or type(expectation) is not dict
            or type(expectation.get("classification")) is not str
            or type(expectation.get("safe_to_convert")) is not bool
            or type(expectation.get("integrity")) is not str
        ):
            raise ValueError("database compatibility projection variants are invalid")
        expected[fixture.rsplit("/", 1)[-1]] = (
            str(expectation["classification"]),
            bool(expectation["safe_to_convert"]),
            str(expectation["integrity"]),
        )
    if set(expected) != set(_FIXTURES):
        raise ValueError("database compatibility projection must expect every frozen fixture")
    return expected


def _classification_of(variants: Mapping[str, tuple[str, bool, str]], name: str) -> str:
    return variants[name][0]


def _parse_classification(payload: bytes) -> dict[str, object] | None:
    """Parse one strict classification receipt; anything else is invalid."""
    try:
        value = json.loads(payload)
    except (UnicodeError, json.JSONDecodeError):
        return None
    if type(value) is not dict or set(value) != {
        "foreign_key_violations",
        "integrity",
        "name",
        "safe_to_convert",
        "schema_digest",
        "source_name",
        "source_sha256",
        "variant_id",
    }:
        return None
    if (
        type(value["name"]) is not str
        or type(value["source_name"]) is not str
        or type(value["integrity"]) is not str
        or type(value["variant_id"]) is not str
        or type(value["safe_to_convert"]) is not bool
        or type(value["foreign_key_violations"]) is not int
        or not _is_digest(value["schema_digest"])
        or not _is_digest(value["source_sha256"])
    ):
        return None
    return value


def _parse_probe_receipt(payload: bytes) -> dict[str, object] | None:
    """Parse one strict neutral probe receipt; anything else is invalid."""
    try:
        value = json.loads(payload)
    except (UnicodeError, json.JSONDecodeError):
        return None
    if type(value) is not dict or set(value) != {
        "classification",
        "error_code",
        "fixture_directory_identical",
        "fts",
        "ledger",
        "migration_sources_digest",
        "ok",
        "probe_row_count",
        "representative_row_digests",
        "row_counts",
        "rows_per_table",
        "safe_to_convert",
        "schema_digest",
        "source_bytes_identical",
        "source_name",
        "source_sha256",
        "target_sha256",
    }:
        return None
    ledger = value["ledger"]
    if (
        type(value["classification"]) is not str
        or type(value["source_name"]) is not str
        or type(value["ok"]) is not bool
        or type(value["safe_to_convert"]) is not bool
        or type(value["source_bytes_identical"]) is not bool
        or type(value["fixture_directory_identical"]) is not bool
        or type(value["probe_row_count"]) is not int
        or value["probe_row_count"] < 0
        or not (value["error_code"] is None or type(value["error_code"]) is str)
        or not _is_digest(value["source_sha256"])
        or not _is_digest(value["schema_digest"])
        or not _is_digest(value["migration_sources_digest"])
        or not _is_digest(value["target_sha256"])
        or type(ledger) is not dict
        or set(ledger) != {"read", "skipped", "blocking"}
        or any(
            type(ledger[name]) is not int or ledger[name] < 0
            for name in ("read", "skipped", "blocking")
        )
    ):
        return None
    for mapping, value_check in (
        (value["rows_per_table"], lambda item: type(item) is int and item >= 0),
        (value["row_counts"], lambda item: type(item) is int),
        (value["representative_row_digests"], _is_digest),
    ):
        if type(mapping) is not dict or any(
            type(key) is not str or not value_check(item) for key, item in mapping.items()
        ):
            return None
    fts = value["fts"]
    if type(fts) is not dict:
        return None
    for table, probe in fts.items():
        if (
            type(table) is not str
            or type(probe) is not dict
            or set(probe) != {"ids", "digest"}
            or type(probe["ids"]) is not list
            or any(type(item) is not int for item in probe["ids"])
            or not _is_digest(probe["digest"])
        ):
            return None
    return value


def _parse_refusal(payload: bytes) -> dict[str, object] | None:
    """Parse one strict refusal report; anything else is invalid."""
    try:
        value = json.loads(payload)
    except (UnicodeError, json.JSONDecodeError):
        return None
    if type(value) is not dict or set(value) != {
        "classification",
        "error_code",
        "ok",
        "safe_to_convert",
        "source_name",
    }:
        return None
    if (
        type(value["classification"]) is not str
        or type(value["error_code"]) is not str
        or type(value["source_name"]) is not str
        or type(value["ok"]) is not bool
        or type(value["safe_to_convert"]) is not bool
    ):
        return None
    return value


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
        if package.get("name") != "hieronymus-legacy-database-import-qualification"
    ]
    return tuple(sorted(dependencies, key=lambda item: (item.name, item.version)))


def _record(
    repo_root: Path,
    *,
    outcomes: Mapping[str, _Outcome],
    environment: Environment,
    input_paths: tuple[str, ...],
    input_digest: str,
    contract_ids: tuple[str, ...],
    work_dir_removed: bool,
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
        contract_ids=contract_ids,
        input_paths=input_paths,
        input_digest=input_digest,
        commands=_COMMANDS,
        environment=environment,
        dependencies=_locked_dependencies(repo_root),
        evidence=evidence,
        consequence=expected_consequence(_RISK, status),
        cleanup=CleanupEvidence(
            work_dir_removed=work_dir_removed,
            raw_logs_removed=work_dir_removed,
            install_dir_removed=True,
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
        raise ValueError("database qualification executable output is invalid") from error
    if type(payload) is not dict or set(payload) != {"failed_criteria"}:
        raise ValueError("database qualification executable payload is invalid")
    failed = payload["failed_criteria"]
    if type(failed) is not list or any(type(item) is not str for item in failed):
        raise ValueError("database qualification executable failure list is invalid")
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
                raise ValueError("database qualification executable timed out")
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
                    raise ValueError("database qualification executable output exceeds limit")
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
    document = _projection_document(repo_root)
    contract_ids = tuple(
        sorted(
            contract["id"]
            for contract in document["contracts"]  # type: ignore[union-attr]
            if type(contract) is dict and type(contract.get("id")) is str
        )
    )
    input_paths = required_fingerprint_inputs(_RISK)
    before = fingerprint_inputs(repo_root, input_paths)
    code, stdout, _stderr = _run_bounded((str(executable),), cwd=repo_root)
    if code != 0:
        raise ValueError("database qualification executable failed")
    failed = _parse_failed_criteria(stdout)
    after = fingerprint_inputs(repo_root, input_paths)
    if after != before:
        raise ValueError("immutable database qualification inputs changed during replay")
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
        contract_ids=contract_ids,
        work_dir_removed=True,
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
        raise ValueError("database work root must not be a symlink")
    if work_root.exists():
        if not work_root.is_dir():
            raise ValueError("database work root is not a directory")
        shutil.rmtree(work_root)
    work_root.mkdir(mode=0o700, parents=True)


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


def _remove_directory_if_present(path: Path, *, label: str) -> bool:
    if path.is_symlink():
        raise ValueError(f"{label} must not be a symlink")
    if not path.exists():
        return False
    if not path.is_dir():
        raise ValueError(f"{label} is not a directory")
    shutil.rmtree(path)
    return not path.exists()


def _harness_argv(
    binary: Path,
    output: Path,
    verb: str,
    arguments: tuple[str, ...],
) -> tuple[str, ...]:
    return (
        sys.executable,
        "-c",
        _HARNESS_CHILD,
        _HARNESS_MARKER,
        str(output),
        str(binary),
        verb,
        *arguments,
    )


def _accounting_ok(parsed: Mapping[str, object]) -> bool:
    ledger = parsed["ledger"]
    rows_per_table = parsed["rows_per_table"]
    assert type(ledger) is dict and type(rows_per_table) is dict
    # Domain tables may legitimately hold zero rows; accounting still requires
    # the row multiset to be conserved exactly once across rows and ledger.
    return (
        parsed["probe_row_count"] == sum(rows_per_table.values())  # type: ignore[union-attr]
        and ledger["read"] + ledger["skipped"] + ledger["blocking"]  # type: ignore[union-attr]
        == parsed["probe_row_count"]
    )


def _ledger_preserved(parsed: Mapping[str, object]) -> bool:
    ledger = parsed["ledger"]
    assert type(ledger) is dict
    return (
        ledger["read"] == parsed["probe_row_count"]  # type: ignore[union-attr]
        and ledger["skipped"] == 0
        and ledger["blocking"] == 0
    )


def _live_outcomes(
    repo_root: Path,
    work_root: Path,
    tool_roots: ToolRoots,
    child_env: dict[str, str],
) -> tuple[dict[str, _Outcome], list[ProcessReceipt], Environment]:
    """Execute the measured database sequence and resolve every criterion."""
    receipts: list[ProcessReceipt] = []
    outcomes: dict[str, _Outcome] = {}
    captures = work_root / "captures"
    captures.mkdir(mode=0o700, exist_ok=True)
    binary = Path(child_env["CARGO_TARGET_DIR"]) / _BINARY_RELPATH
    cargo = str(tool_roots.cargo_invocation)
    fixture_root = repo_root / _FIXTURE_ROOT
    projection_path = repo_root / _PROJECTION
    variants = _expected_variants(_projection_document(repo_root))
    digests_before = {name: _sha256_file(fixture_root / name) for name in _FIXTURES}
    tree_before = _fixture_tree_digest(fixture_root)
    capture_paths: list[Path] = []
    captures_valid = 0
    platform_environment = {
        "os": platform.system().lower(),
        "kernel": platform.release(),
        "architecture": platform.machine(),
    }
    environment = Environment(
        rustc="<absent>",
        cargo="<absent>",
        target=_TARGET,
        bun=None,
        native_libraries=(),
        **platform_environment,
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

    def capture(output: Path, parser, receipt: ProcessReceipt | None) -> object:
        """Read one bounded child's stdout artifact through its strict parser.

        Refusal probes exit 1 with their JSON report on stdout, so the receipt
        status is intentionally not gated here; every caller checks it.
        """
        nonlocal captures_valid
        capture_paths.append(output)
        if receipt is None or not output.is_file():
            return None
        try:
            parsed = parser(output.read_bytes())
        except OSError:
            return None
        if parsed is not None:
            captures_valid += 1
        return parsed

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
        raise ValueError("database toolchain measurement failed")
    try:
        measured = json.loads(toolchain_output.read_bytes())
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("database toolchain capture is invalid") from error
    if (
        type(measured) is not dict
        or set(measured) != {"cargo", "rustc", "host"}
        or type(measured["host"]) is not str
        or measured["host"] != _TARGET
    ):
        raise ValueError("measured Rust target differs from the database qualification target")
    environment = Environment(
        rustc=str(measured["rustc"]),
        cargo=str(measured["cargo"]),
        target=_TARGET,
        bun=None,
        native_libraries=(),
        **platform_environment,
    )

    # Build stage: locked release build over the warm sanitized cargo target,
    # then inspect the linked libraries for the bundled SQLite proof.
    build_receipt = launch(
        (
            cargo,
            f"+{_TOOLCHAIN}",
            "build",
            "--manifest-path",
            _MANIFEST,
            "--release",
            "--locked",
            "--target",
            _TARGET,
        )
    )
    build_ok = _successful(build_receipt) and binary.is_file()
    basenames: tuple[str, ...] = ()
    if build_ok:
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
        if ldd_receipt is not None and _successful(ldd_receipt) and ldd_output.is_file():
            try:
                basenames = _ldd_basenames(ldd_output.read_text(encoding="utf-8", errors="strict"))
            except OSError:
                basenames = ()
    external_sqlite = sum(1 for name in basenames if "sqlite" in name.lower())
    environment = Environment(
        rustc=environment.rustc,
        cargo=environment.cargo,
        target=_TARGET,
        bun=None,
        native_libraries=basenames,
        **platform_environment,
    )
    outcomes[_BUILD_CRITERION] = (
        _pass(
            "the locked release build over the bundled-SQLite rusqlite candidate"
            " succeeded and links no external sqlite library",
            locked_commands=1,
            toolchain=_TOOLCHAIN,
            output_sha256=build_receipt.stdout_sha256 if build_receipt else "",
            binary_bytes=binary.stat().st_size if build_ok else 0,
            native_library_basenames=basenames,
            external_sqlite_libraries=external_sqlite,
        )
        if build_ok and basenames and external_sqlite == 0
        else _fail(
            "the locked release build failed or the binary links an external sqlite library",
            locked_commands=1,
            native_library_basenames=basenames,
            external_sqlite_libraries=external_sqlite,
        )
    )
    if not outcomes[_BUILD_CRITERION].status == "pass":
        for criterion in _BUILD_DEPENDENTS:
            outcomes[criterion] = _not_run(_BUILD_CRITERION)
        return outcomes, receipts, environment

    # Classification stage: every frozen fixture is classified read-only and
    # must match the verified projection's variant expectation exactly.
    classification_ok = True
    classification_labels: list[str] = []
    classification_matches = 0
    for name in _FIXTURES:
        output = captures / f"classify-{name}.json"
        receipt = launch(
            _harness_argv(
                binary,
                output,
                "classify",
                (
                    "--fixture-root",
                    str(fixture_root),
                    "--source-name",
                    name,
                    "--contract",
                    str(projection_path),
                ),
            )
        )
        parsed = capture(output, _parse_classification, receipt)
        matched = (
            _successful(receipt)
            and type(parsed) is dict
            and parsed["source_name"] == name
            and parsed["name"] == _classification_of(variants, name)
            and parsed["safe_to_convert"] is variants[name][1]
        )
        classification_ok = classification_ok and matched
        if matched:
            classification_matches += 1
        if type(parsed) is dict:
            classification_labels.append(f"{name}={parsed['name']}")
    outcomes["fixture-classification"] = (
        _pass(
            "all six frozen fixtures classified read-only exactly as the verified"
            " projection's variants expect",
            fixtures=len(_FIXTURES),
            matched=len(classification_labels),
            classifications=tuple(classification_labels),
        )
        if classification_ok and len(classification_labels) == len(_FIXTURES)
        else _fail(
            "a frozen fixture classification contradicted the verified projection",
            fixtures=len(_FIXTURES),
            matched=classification_matches,
        )
    )
    if not classification_ok:
        for criterion in _CLASSIFICATION_DEPENDENTS:
            outcomes[criterion] = _not_run("fixture-classification")

    supported: dict[str, dict[str, object] | None] = {}
    if classification_ok:
        # Supported probes: the current-schema probe is deep (projection
        # representative rows, ledgers, FTS), the legacy probe is shallow.
        for name in _SUPPORTED_PROBES:
            stem = name.removesuffix(".sqlite")
            output = captures / f"probe-{name}.json"
            probe_root = work_root / f"probe-{stem}"
            probe_root.mkdir(mode=0o700, exist_ok=True)
            receipt = launch(
                _harness_argv(
                    binary,
                    output,
                    "probe-import",
                    (
                        "--fixture-root",
                        str(fixture_root),
                        "--source-name",
                        name,
                        "--contract",
                        str(projection_path),
                        "--work-root",
                        str(probe_root),
                        "--target-name",
                        f"{stem}-probe-target.sqlite",
                    ),
                )
            )
            parsed = capture(output, _parse_probe_receipt, receipt)
            supported[name] = (
                parsed
                if _successful(receipt)
                and type(parsed) is dict
                and parsed["ok"] is True
                and parsed["classification"] == _classification_of(variants, name)
                and parsed["safe_to_convert"] is True
                and parsed["source_bytes_identical"] is True
                and parsed["fixture_directory_identical"] is True
                and parsed["error_code"] is None
                else None
            )

    def supported_ok(name: str) -> bool:
        return supported.get(name) is not None

    if classification_ok:
        outcomes["supported-current-read"] = (
            _pass(
                "the full current-schema fixture imported into a disposable neutral"
                " target with byte-identical frozen sources",
                source_name="minimal-python.sqlite",
                probe_row_count=supported["minimal-python.sqlite"]["probe_row_count"]
                if supported_ok("minimal-python.sqlite")
                else 0,
                source_bytes_identical=1 if supported_ok("minimal-python.sqlite") else 0,
            )
            if supported_ok("minimal-python.sqlite")
            else _fail("the current-schema probe import did not produce a clean neutral receipt")
        )
        outcomes["supported-legacy-read"] = (
            _pass(
                "the legacy-schema fixture imported into a disposable neutral target"
                " with byte-identical frozen sources",
                source_name="legacy-python.sqlite",
                probe_row_count=supported["legacy-python.sqlite"]["probe_row_count"]
                if supported_ok("legacy-python.sqlite")
                else 0,
                source_bytes_identical=1 if supported_ok("legacy-python.sqlite") else 0,
            )
            if supported_ok("legacy-python.sqlite")
            else _fail("the legacy-schema probe import did not produce a clean neutral receipt")
        )

        accounting_ok = all(
            supported.get(name) is not None and _accounting_ok(supported[name])
            for name in _SUPPORTED_PROBES
        )
        outcomes["typed-row-accounting"] = (
            _pass(
                "every source row of both supported fixtures produced exactly one"
                " neutral probe row and one ledger outcome",
                probes=len(_SUPPORTED_PROBES),
                probe_rows=sum(
                    (supported[name]["probe_row_count"] for name in _SUPPORTED_PROBES),  # type: ignore[index]
                    start=0,
                )
                if accounting_ok
                else 0,
            )
            if accounting_ok
            else _fail("typed row accounting diverged between probe rows and the ledger")
        )

        current_parsed = supported.get("minimal-python.sqlite")
        fts_ok = (
            type(current_parsed) is dict
            and set(current_parsed["fts"]) == set(_FTS_TABLES)  # type: ignore[arg-type]
            and all(
                type(probe["ids"]) is list and bool(probe["ids"])  # type: ignore[index]
                for probe in current_parsed["fts"].values()  # type: ignore[union-attr]
            )
        )
        outcomes["fts-query-equivalence"] = (
            _pass(
                "all five projected FTS probes matched their representative-row terms"
                " with stable matched-id digests",
                probes=len(_FTS_TABLES),
                matched_probes=len(current_parsed["fts"]) if fts_ok else 0,  # type: ignore[union-attr]
            )
            if fts_ok
            else _fail("the FTS probe set did not match the projected searchable domains")
        )

        ledger_ok = all(
            supported.get(name) is not None and _ledger_preserved(supported[name])
            for name in _SUPPORTED_PROBES
        )
        outcomes["ledger-preserved"] = (
            _pass(
                "the neutral probe ledger recorded a clean read outcome for every row"
                " with nothing skipped or blocking",
                ledgers=len(_SUPPORTED_PROBES),
                ledger_read=sum(
                    (supported[name]["probe_row_count"] for name in _SUPPORTED_PROBES),  # type: ignore[index]
                    start=0,
                )
                if ledger_ok
                else 0,
                skipped=0,
                blocking=0,
            )
            if ledger_ok
            else _fail("the neutral probe ledger did not preserve every row as cleanly read")
        )

        refusals_ok = True
        refusal_codes: list[str] = []
        for name in _UNSUPPORTED_PROBES:
            stem = name.removesuffix(".sqlite")
            output = captures / f"probe-{name}.json"
            probe_root = work_root / f"probe-{stem}"
            probe_root.mkdir(mode=0o700, exist_ok=True)
            receipt = launch(
                _harness_argv(
                    binary,
                    output,
                    "probe-import",
                    (
                        "--fixture-root",
                        str(fixture_root),
                        "--source-name",
                        name,
                        "--contract",
                        str(projection_path),
                        "--work-root",
                        str(probe_root),
                        "--target-name",
                        f"{stem}-probe-target.sqlite",
                    ),
                )
            )
            parsed = capture(output, _parse_refusal, receipt)
            expected_code = _REFUSAL_CODES.get(_classification_of(variants, name))
            refused = (
                _refused(receipt)
                and type(parsed) is dict
                and parsed["ok"] is False
                and parsed["safe_to_convert"] is False
                and parsed["source_name"] == name
                and parsed["classification"] == _classification_of(variants, name)
                and parsed["error_code"] == expected_code
            )
            refusals_ok = refusals_ok and refused
            refusal_codes.append(f"{name}={expected_code}")
        outcomes["unsupported-fail-closed"] = (
            _pass(
                "every non-convertible fixture was refused before any target was"
                " created, each with its exact error code",
                refusals=len(_UNSUPPORTED_PROBES),
                error_codes=tuple(refusal_codes),
            )
            if refusals_ok
            else _fail("a non-convertible fixture was not refused with its exact error code")
        )

    # The database-specific core proof: every frozen fixture byte is identical
    # before and after every child, and both supported probes self-report
    # byte-identical sources and fixture directories.
    tree_after = _fixture_tree_digest(fixture_root)
    digests_after = {name: _sha256_file(fixture_root / name) for name in _FIXTURES}
    probe_flags_ok = all(
        supported.get(name) is None
        or (
            supported[name]["source_bytes_identical"] is True  # type: ignore[index]
            and supported[name]["fixture_directory_identical"] is True  # type: ignore[index]
        )
        for name in _SUPPORTED_PROBES
    )
    identity_ok = tree_after == tree_before and digests_after == digests_before and probe_flags_ok
    outcomes["source-byte-identity"] = (
        _pass(
            "all six frozen fixture digests and the fixture tree digest are"
            " byte-identical before and after every child",
            fixtures=len(_FIXTURES),
            unchanged=sum(1 for name in _FIXTURES if digests_after[name] == digests_before[name]),
            probe_reports=sum(1 for name in _SUPPORTED_PROBES if supported.get(name) is not None),
        )
        if identity_ok
        else _fail(
            "a frozen fixture changed bytes during the qualification sequence",
            fixtures=len(_FIXTURES),
            unchanged=sum(1 for name in _FIXTURES if digests_after[name] == digests_before[name]),
        )
    )

    # No capture may carry a forbidden marker, and every capture must have
    # parsed through its strict digest-only receipt schema. When an earlier
    # stage cascades, only the captures that were actually produced are judged.
    marker_hits = 0
    scan_ok = bool(capture_paths) and captures_valid == len(capture_paths)
    for output in capture_paths:
        try:
            payload = output.read_bytes()
        except OSError:
            scan_ok = False
            break
        for marker in _FORBIDDEN_MARKERS:
            if marker.encode("utf-8") in payload:
                marker_hits += 1
    outcomes["no-sensitive-row-output"] = (
        _pass(
            "every captured harness output matched its exact digest-only schema"
            " and none carries a forbidden marker",
            captures=len(capture_paths),
            schema_valid=captures_valid,
            forbidden_markers=0,
        )
        if scan_ok and marker_hits == 0
        else _fail(
            "a captured output failed its receipt schema or carried a forbidden marker",
            captures=len(capture_paths),
            schema_valid=captures_valid,
            forbidden_markers=marker_hits,
        )
    )
    return outcomes, receipts, environment


def run_live(
    repo_root: Path,
    work_root: Path,
    *,
    original_env: Mapping[str, str],
) -> QualificationRecord:
    """Run the database candidate under the offline, owned-process boundary."""
    caller_env = dict(original_env)
    if caller_env.get("HIERONYMUS_QUALIFICATION_LIVE") != "1":
        raise ValueError("HIERONYMUS_QUALIFICATION_LIVE=1 is required")
    document = _projection_document(repo_root)
    contract_ids = tuple(
        sorted(
            contract["id"]
            for contract in document["contracts"]  # type: ignore[union-attr]
            if type(contract) is dict and type(contract.get("id")) is str
        )
    )
    input_paths = required_fingerprint_inputs(_RISK)
    before = fingerprint_inputs(repo_root, input_paths)
    _prepare_work_root(work_root)
    _reconcile_cargo_target(repo_root)
    receipts: list[ProcessReceipt] = []
    outcomes: dict[str, _Outcome] | None = None
    environment: Environment | None = None
    try:
        tool_roots, _cargo_target_dir, child_env = _live_process_context(
            repo_root, work_root, caller_env
        )
        outcomes, receipts, environment = _live_outcomes(
            repo_root, work_root, tool_roots, child_env
        )
    finally:
        work_removed = _remove_directory_if_present(work_root, label="database work root")
    after = fingerprint_inputs(repo_root, input_paths)
    if after != before:
        raise ValueError("immutable database qualification inputs changed during replay")
    if outcomes is None or environment is None:
        raise ValueError("database live outcomes were not produced")
    if not work_removed:
        raise ValueError("database work root could not be removed after the run")
    if not (bool(receipts) and all(item.core_dumps_disabled for item in receipts)):
        raise ValueError("database children did not all run with core dumps disabled")
    if not (bool(receipts) and all(item.process_group_reaped for item in receipts)):
        raise ValueError("database children were not all reaped from owned process groups")
    return _record(
        repo_root,
        outcomes=outcomes,
        environment=environment,
        input_paths=input_paths,
        input_digest=before,
        contract_ids=contract_ids,
        work_dir_removed=work_removed,
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
    parser = argparse.ArgumentParser(
        description="Run the offline legacy-database-import qualification"
    )
    parser.add_argument("--write", action="store_true", required=True)
    parser.parse_args(argv)
    repo_root = Path.cwd()
    try:
        record = run_live(
            repo_root,
            repo_root / _LIVE_WORK,
            original_env=dict(os.environ),
        )
        _write_record(repo_root, record)
    except (OSError, TypeError, ValueError, RuntimeError):
        print("legacy-database-import qualification failed", file=sys.stderr)
        return 2
    print(f"legacy-database-import qualification recorded: {record.decision}")
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
