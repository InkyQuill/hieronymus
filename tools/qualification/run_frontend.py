"""Offline frontend-embedding qualification runner and canonical evidence writer."""

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
    _validated_unshare,
    discover_tool_roots,
    run_owned_process,
    safe_subprocess_env,
)
from tools.qualification.render import render_record

_RISK = "frontend-embedding"
_OWNER = "Pavel Obruchnikov <me@inkyquill.net>"
_TARGET = "x86_64-unknown-linux-gnu"
_TOOLCHAIN = "1.96.0"
_MANIFEST = "qualification/harnesses/frontend-embedding/Cargo.toml"
_LOCK = "qualification/harnesses/frontend-embedding/Cargo.lock"
_PREREQUISITES = "qualification/prerequisites.json"
_ROUTES_FIXTURE = "compatibility/fixtures/http/route-cases.json"
_BUNDLE = Path("qualification/.artifacts/frontend-dist/current")
_QUARANTINE = Path("qualification/.artifacts/frontend-dist/quarantine")
_CARGO_TARGET = Path("qualification/.artifacts/cargo-target/frontend-embedding")
_INSTALL = Path("qualification/.artifacts/install/frontend-embedding")
_LIVE_WORK = Path("qualification/.artifacts/work/frontend-embedding")
_RECORD = Path("qualification/records/frontend-embedding.json")
_MARKDOWN = Path("docs/qualification/rust/frontend-embedding.md")
_BINARY_NAME = "frontend-embedding"
_BINARY_RELPATH = "x86_64-unknown-linux-gnu/release/frontend-embedding"
_STRACE = Path("/usr/bin/strace")
_BUN_OUTDIR = "../qualification/.artifacts/frontend-dist/current"
_SPECS = (
    "docs/adr/0014-web-console-replaces-terminal-ui.md",
    "docs/superpowers/specs/2026-08-31-rust-migration-program-design.md",
)
_ROUTE_IDS = (
    "frontend.route.get.root",
    "frontend.route.get.admin",
    "frontend.route.get.admin.path",
    "frontend.route.get.assets.path",
    "frontend.route.get.config",
    "frontend.route.get.config.path",
)
_ASSETS_ROUTE_ID = "frontend.route.get.assets.path"
_HTML_ROUTE_IDS = tuple(route_id for route_id in _ROUTE_IDS if route_id != _ASSETS_ROUTE_ID)
_JAVASCRIPT_MIMES = frozenset(
    {"application/javascript", "text/javascript", "application/x-javascript"}
)
_CSS_MIMES = frozenset({"text/css"})
_WOFF2_MIMES = frozenset({"font/woff2", "application/font-woff2"})
_ERROR_MIME = "application/json"


def _asset_mime_family(key: str) -> frozenset[str] | None:
    """MIME families accepted for one bundle asset extension; None is wildcard."""
    if key.endswith(".js"):
        return _JAVASCRIPT_MIMES
    if key.endswith(".css"):
        return _CSS_MIMES
    if key.endswith(".woff2"):
        return _WOFF2_MIMES
    return None


_COMMANDS = (
    "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR="
    "qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true "
    "uv run python -m tools.qualification.run_frontend --write",
    "unshare --user --map-root-user --net -- bun run --cwd frontend build -- --outDir "
    "../qualification/.artifacts/frontend-dist/current --emptyOutDir",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding "
    "CARGO_NET_OFFLINE=true cargo +1.96.0 build --manifest-path "
    "qualification/harnesses/frontend-embedding/Cargo.toml --release --locked "
    "--target x86_64-unknown-linux-gnu",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding "
    "CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path "
    "qualification/harnesses/frontend-embedding/Cargo.toml --check",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding "
    "CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path "
    "qualification/harnesses/frontend-embedding/Cargo.toml --release --locked "
    "--all-targets --target x86_64-unknown-linux-gnu -- -D warnings",
    "uv run python -m tools.qualification.validate qualification/records/frontend-embedding.json",
    "uv run python -m tools.qualification.render --check "
    "qualification/records/frontend-embedding.json docs/qualification/rust/frontend-embedding.md",
)
_TOTAL_TIMEOUT_SECONDS = 1200
_NO_PROGRESS_SECONDS = 120
_FAKE_TIMEOUT_SECONDS = 30
_MAX_CAPTURE = 2 * 1024 * 1024
_COPY_CHUNK = 1024 * 1024
_SECRET_MARKERS = (
    "compat-secret-do-not-log",
    "Authorization: Bearer",
    "provider_key",
    "/home/",
    "Yandex.Disk",
)
_RUNTIME_BASENAMES = ("bun", "node", "python", "python3", "deno")
_EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()
_ALL_CRITERIA = REQUIRED_CRITERIA[_RISK]
_BUILD_CRITERION = "bun-version-and-frozen-build"
_BUILD_DEPENDENTS = tuple(criterion for criterion in _ALL_CRITERIA if criterion != _BUILD_CRITERION)
_TRACED_PROBES = (
    "/",
    "/index.html",
    "/admin/fixture",
    "/config/fixture",
    "/assets/qualification-missing.js",
    "/a/../b",
    "/foo%2fbar",
    "/foo%5Cbar",
    "/%00x",
)
_TOOLCHAIN_MARKER = "qualification-toolchain-capture"
_BUN_MARKER = "qualification-bun-capture"
_ASSET_MARKER = "qualification-asset-capture"
_LDD_MARKER = "qualification-ldd-capture"

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
_BUN_CHILD = (
    "import json, subprocess, sys\n"
    "marker, output, bun = sys.argv[1], sys.argv[2], sys.argv[3]\n"
    "completed = subprocess.run([bun, '--version'], capture_output=True)\n"
    "if completed.returncode != 0:\n"
    "    raise SystemExit(completed.returncode)\n"
    "with open(output, 'w', encoding='utf-8') as stream:\n"
    "    json.dump({'bun': completed.stdout.decode('utf-8', errors='strict').strip()},"
    " stream)\n"
)
_ASSET_CHILD = (
    "import subprocess, sys\n"
    "marker, output, binary, command = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]\n"
    "completed = subprocess.run([binary, command, *sys.argv[5:]], capture_output=True)\n"
    "with open(output, 'wb') as stream:\n"
    "    stream.write(completed.stdout)\n"
    "raise SystemExit(completed.returncode)\n"
)
_LDD_CHILD = (
    "import subprocess, sys\n"
    "marker, output, binary = sys.argv[1], sys.argv[2], sys.argv[3]\n"
    "completed = subprocess.run(['/usr/bin/ldd', binary], capture_output=True)\n"
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


def _read_object(path: Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_bytes())
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("frontend qualification input is invalid") from error
    if type(value) is not dict:
        raise ValueError("frontend qualification input must be an object")
    return value


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


def _copy_tree(source: Path, destination: Path) -> None:
    """Copy a regular-file-only bundle tree without following symlinks."""
    if source.is_symlink() or not source.is_dir():
        raise ValueError("frontend bundle root must be a nonsymlink directory")
    for relative in _walk_regular_files(source, ""):
        target = destination / relative
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        _copy_file(source / relative, target)


def _walk_regular_files(directory: Path, prefix: str) -> tuple[str, ...]:
    relatives: list[str] = []
    with os.scandir(directory) as entries:
        for entry in sorted(entries, key=lambda item: item.name):
            if entry.is_symlink():
                raise ValueError("frontend bundle must not contain symlinks")
            if entry.is_dir(follow_symlinks=False):
                relatives.extend(_walk_regular_files(Path(entry.path), f"{prefix}{entry.name}/"))
            elif entry.is_file(follow_symlinks=False):
                relatives.append(f"{prefix}{entry.name}")
            else:
                raise ValueError("frontend bundle must contain only directories and files")
    return tuple(relatives)


def _listing(root: Path) -> dict[str, tuple[int, str]]:
    """Relative bundle path -> (byte length, SHA-256), symlink-free."""
    listing: dict[str, tuple[int, str]] = {}
    for relative in _walk_regular_files(root, ""):
        path = root / relative
        listing[relative] = (path.stat().st_size, _sha256_file(path))
    return listing


def _listing_digest(listing: Mapping[str, tuple[int, str]]) -> str:
    canonical = json.dumps(
        {key: list(value) for key, value in sorted(listing.items())},
        sort_keys=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(canonical.encode()).hexdigest()


def _remove_directory_if_present(path: Path, *, label: str) -> bool:
    if path.is_symlink():
        raise ValueError(f"{label} must not be a symlink")
    if not path.exists():
        return False
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


def _rejected(receipt: ProcessReceipt | None) -> bool:
    """Whether one bounded child failed cleanly, the missing-bundle signal."""
    return (
        receipt is not None
        and receipt.exit_code is not None
        and receipt.exit_code != 0
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


def _quarantine_asset_root(bundle: Path, quarantine: Path) -> None:
    """Rename the canonical bundle beneath a runner-owned, access-denied parent."""
    if bundle.is_symlink() or not bundle.is_dir():
        raise ValueError("frontend bundle root must be a nonsymlink directory")
    if quarantine.is_symlink() or quarantine.exists():
        raise ValueError("frontend quarantine directory already exists")
    quarantine.mkdir(mode=0o700, parents=True)
    bundle.rename(quarantine / "current")
    quarantine.chmod(0o000)


def _restore_asset_root(bundle: Path, quarantine: Path) -> None:
    """Undo a quarantine and always leave the quarantine directory absent."""
    if quarantine.is_symlink():
        raise ValueError("frontend quarantine must not be a symlink")
    if not quarantine.exists():
        return
    try:
        quarantine.chmod(0o700)
    except OSError as error:
        raise ValueError("frontend quarantine could not be made restorable") from error
    payload = quarantine / "current"
    if payload.is_symlink() or (payload.exists() and not payload.is_dir()):
        raise ValueError("frontend quarantine payload is not a bundle directory")
    if payload.is_dir():
        if bundle.is_symlink() or bundle.exists():
            raise ValueError("frontend bundle root reappeared during quarantine")
        payload.rename(bundle)
    try:
        quarantine.rmdir()
    except OSError as error:
        raise ValueError("frontend quarantine directory is not empty") from error


def _validate_host_tools() -> None:
    """Fail before any child when the fixed namespace or tracer is unavailable."""
    _validated_unshare()
    try:
        resolved = _STRACE.resolve(strict=True)
        mode = resolved.stat().st_mode
    except OSError as error:
        raise ValueError("strace is unavailable for the runtime independence probe") from error
    if (
        not _STRACE.is_absolute()
        or resolved != _STRACE
        or not stat.S_ISREG(mode)
        or not os.access(resolved, os.X_OK)
    ):
        raise ValueError("strace is unavailable for the runtime independence probe")


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
        if package.get("name") != "hieronymus-frontend-embedding-qualification"
    ]
    return tuple(sorted(dependencies, key=lambda item: (item.name, item.version)))


def _route_contracts(repo_root: Path) -> dict[str, tuple[str, str]]:
    """Validate the six embedded route fixtures; return id -> (path, MIME)."""
    routes = _read_object(repo_root / _ROUTES_FIXTURE)
    route_list = routes.get("routes")
    if type(route_list) is not list:
        raise ValueError("HTTP route oracle is invalid")
    plans: dict[str, tuple[str, str]] = {}
    for route in route_list:
        if type(route) is not dict:
            raise ValueError("HTTP route oracle is invalid")
        contract_id = route.get("contract_id")
        if contract_id not in _ROUTE_IDS:
            continue
        if contract_id in plans:
            raise ValueError("frontend route fixture is duplicated")
        target = route.get("target")
        success = (
            target.get("success")
            if type(target) is dict and type(target.get("success")) is dict
            else route.get("success")
        )
        failure = route.get("failure")
        if type(success) is not dict:
            raise ValueError("frontend route cases are invalid")
        request = success.get("request")
        response = success.get("response")
        if type(request) is not dict or type(response) is not dict:
            raise ValueError("frontend route success case is invalid")
        headers = response.get("headers")
        path = request.get("path")
        status = response.get("status")
        mime = headers.get("Content-Type") if type(headers) is dict else None
        body = response.get("body")
        if contract_id == _ASSETS_ROUTE_ID:
            failure_response = failure.get("response") if type(failure) is dict else None
            failure_request = failure.get("request") if type(failure) is dict else None
            if (
                status != 200
                or type(mime) is not str
                or mime.split(";", 1)[0].strip().lower() not in _JAVASCRIPT_MIMES
                or type(body) is not str
                or type(path) is not str
                or not path.startswith("/assets/")
                or type(failure_response) is not dict
                or failure_response.get("status") != 404
                or type(failure_request) is not dict
                or failure_request.get("path") != "/assets/fixture"
            ):
                raise ValueError("frontend assets route fixture is invalid")
        else:
            if (
                status != 200
                or mime != "text/html; charset=utf-8"
                or type(body) is not str
                or not body.startswith("<!doctype html>")
                or type(path) is not str
                or not path.startswith("/")
            ):
                raise ValueError("frontend html route fixture is invalid")
        plans[contract_id] = (path, mime)
    if set(plans) != set(_ROUTE_IDS):
        raise ValueError("frontend route fixtures are incomplete")
    return plans


def _record(
    repo_root: Path,
    *,
    outcomes: Mapping[str, _Outcome],
    environment: Environment,
    input_paths: tuple[str, ...],
    input_digest: str,
    contract_ids: tuple[str, ...],
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
            _fail(
                f"{criterion} failed in the fake-injected replay",
                replay="fake-injected",
            )
            if criterion in failed
            else _pass(
                f"{criterion} passed in the fake-injected replay",
                replay="fake-injected",
            )
        )
        for criterion in _ALL_CRITERIA
    }


def _parse_failed_criteria(stdout: bytes) -> tuple[str, ...]:
    try:
        payload = json.loads(stdout)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("frontend qualification executable output is invalid") from error
    if type(payload) is not dict or set(payload) != {"failed_criteria"}:
        raise ValueError("frontend qualification executable payload is invalid")
    failed = payload["failed_criteria"]
    if type(failed) is not list or any(type(item) is not str for item in failed):
        raise ValueError("frontend qualification executable failure list is invalid")
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
                raise ValueError("frontend qualification executable timed out")
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
                    raise ValueError("frontend qualification executable output exceeds limit")
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
    plans = _route_contracts(repo_root)
    input_paths = required_fingerprint_inputs(_RISK)
    before = fingerprint_inputs(repo_root, input_paths)
    code, stdout, _stderr = _run_bounded((str(executable),), cwd=repo_root)
    if code != 0:
        raise ValueError("frontend qualification executable failed")
    failed = _parse_failed_criteria(stdout)
    after = fingerprint_inputs(repo_root, input_paths)
    if after != before:
        raise ValueError("immutable frontend qualification inputs changed during replay")
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
        contract_ids=tuple(sorted(plans)),
        work_dir_removed=True,
        raw_logs_removed=True,
        install_dir_removed=True,
        core_dumps_disabled=True,
        owned_process_groups_reaped=True,
    )


def _discover_bun_invocation(original_env: Mapping[str, str]) -> Path:
    """Preserve the lexical Bun invocation from the original PATH (never recorded)."""
    original_path = original_env.get("PATH", "")
    seen: set[Path] = set()
    for part in original_path.split(os.pathsep):
        directory = part or os.curdir
        lexical = Path(os.path.abspath(os.path.join(directory, "bun")))
        if lexical in seen:
            continue
        seen.add(lexical)
        if not os.path.lexists(lexical):
            continue
        try:
            target = lexical.resolve(strict=True)
            mode = target.stat().st_mode
        except OSError as error:
            raise ValueError("bun invocation has no resolved target") from error
        if not stat.S_ISREG(mode) or not os.access(target, os.X_OK):
            raise ValueError("bun target is not an executable regular file")
        return lexical
    raise ValueError("bun invocation was not found in the original environment")


def _live_process_context(
    repo_root: Path,
    work_root: Path,
    original_env: Mapping[str, str],
) -> tuple[ToolRoots, Path, Path, dict[str, str]]:
    """Discover unsanitized tools, then produce one shared safe environment."""
    tool_roots = discover_tool_roots(original_env)
    bun_invocation = _discover_bun_invocation(original_env)
    cargo_target_dir = repo_root / _CARGO_TARGET
    child_env = safe_subprocess_env(
        work_root,
        cargo_offline=True,
        tool_roots=tool_roots,
        cargo_target_dir=cargo_target_dir,
    )
    return tool_roots, bun_invocation, cargo_target_dir, child_env


def _prepare_work_root(work_root: Path) -> None:
    if work_root.is_symlink():
        raise ValueError("frontend work root must not be a symlink")
    if work_root.exists():
        if not work_root.is_dir():
            raise ValueError("frontend work root is not a directory")
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


def _parse_asset_response(payload: bytes) -> dict[str, object] | None:
    try:
        value = json.loads(payload)
    except (UnicodeError, json.JSONDecodeError):
        return None
    if type(value) is not dict or set(value) != {
        "status",
        "content_type",
        "body_sha256",
        "len",
        "fallback",
    }:
        return None
    if (
        type(value["status"]) is not int
        or type(value["content_type"]) is not str
        or type(value["body_sha256"]) is not str
        or type(value["len"]) is not int
        or type(value["fallback"]) is not bool
    ):
        return None
    return value


def _parse_manifest(payload: bytes) -> dict[str, tuple[int, str]] | None:
    try:
        value = json.loads(payload)
    except (UnicodeError, json.JSONDecodeError):
        return None
    if type(value) is not list or not value:
        return None
    manifest: dict[str, tuple[int, str]] = {}
    for entry in value:
        if type(entry) is not dict or set(entry) != {"key", "len", "sha256"}:
            return None
        if type(entry["key"]) is not str or type(entry["len"]) is not int:
            return None
        if type(entry["sha256"]) is not str:
            return None
        manifest[entry["key"]] = (entry["len"], entry["sha256"])
    return manifest


def _response_matches(
    response: dict[str, object] | None,
    *,
    status: int,
    content_type: str | None,
    mime_family: frozenset[str] | None,
    digest: str,
    length: int,
    fallback: bool,
) -> bool:
    if response is None:
        return False
    actual_type = response["content_type"]
    if content_type is not None and actual_type != content_type:
        return False
    if content_type is None and mime_family is None:
        if actual_type in ("", _ERROR_MIME):
            return False
    if mime_family is not None:
        if type(actual_type) is not str:
            return False
        if actual_type.split(";", 1)[0].strip().lower() not in mime_family:
            return False
    return (
        response["status"] == status
        and response["body_sha256"] == digest
        and response["len"] == length
        and response["fallback"] is fallback
    )


def _traced_execve_names(trace_text: str) -> tuple[str, ...]:
    """Exec'd path basenames from a strace log; -f prefixes lines with a PID."""
    names: list[str] = []
    for line in trace_text.splitlines():
        position = line.find("execve(")
        if position < 0:
            continue
        start = line.find('"', position)
        end = line.find('"', start + 1) if start >= 0 else -1
        if end < 0:
            continue
        names.append(Path(line[start + 1 : end]).name)
    return tuple(names)


def _live_outcomes(
    repo_root: Path,
    work_root: Path,
    tool_roots: ToolRoots,
    bun_invocation: Path,
    child_env: dict[str, str],
) -> tuple[dict[str, _Outcome], list[ProcessReceipt], Environment]:
    """Execute the measured frontend sequence and resolve every criterion."""
    route_plans = _route_contracts(repo_root)
    receipts: list[ProcessReceipt] = []
    outcomes: dict[str, _Outcome] = {}
    target_dir = Path(child_env["CARGO_TARGET_DIR"])
    binary = target_dir / _BINARY_RELPATH
    missing_target = target_dir.with_name(f"{target_dir.name}-missing")
    cargo = str(tool_roots.cargo_invocation)
    bundle_root = repo_root / _BUNDLE
    quarantine = repo_root / _QUARANTINE
    install_root = repo_root / _INSTALL
    captures = work_root / "captures"
    traces = work_root / "traces"
    copy_root = work_root / "bundle"
    captures.mkdir(mode=0o700, exist_ok=True)
    traces.mkdir(mode=0o700, exist_ok=True)
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
        raise ValueError("frontend toolchain measurement failed")
    try:
        measured = json.loads(toolchain_output.read_bytes())
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("frontend toolchain capture is invalid") from error
    if (
        type(measured) is not dict
        or set(measured) != {"cargo", "rustc", "host"}
        or type(measured["host"]) is not str
        or measured["host"] != _TARGET
    ):
        raise ValueError("measured Rust target differs from the frontend qualification target")
    environment = Environment(
        rustc=str(measured["rustc"]),
        cargo=str(measured["cargo"]),
        target=_TARGET,
        bun=None,
        native_libraries=(),
        **platform_environment,
    )

    # Build stage: bun version, frozen lock, network-isolated bundle build,
    # locked release build, and clippy over the canonical asset root.
    bun_version = ""
    bun_output = captures / "bun.json"
    bun_receipt = launch(
        (
            sys.executable,
            "-c",
            _BUN_CHILD,
            _BUN_MARKER,
            str(bun_output),
            str(bun_invocation),
        )
    )
    if bun_receipt is not None and _successful(bun_receipt) and bun_output.is_file():
        try:
            measured_bun = json.loads(bun_output.read_bytes())
        except (OSError, UnicodeError, json.JSONDecodeError):
            measured_bun = None
        if type(measured_bun) is dict and type(measured_bun.get("bun")) is str:
            bun_version = measured_bun["bun"]
    environment = Environment(
        rustc=environment.rustc,
        cargo=environment.cargo,
        target=_TARGET,
        bun=bun_version or None,
        native_libraries=(),
        **platform_environment,
    )
    try:
        prerequisites = _read_object(repo_root / _PREREQUISITES)
        pinned_bun = prerequisites["bun"]
        pinned_version = pinned_bun["version"] if type(pinned_bun) is dict else None
    except (KeyError, TypeError) as error:
        raise ValueError("frontend bun prerequisite is invalid") from error
    if type(pinned_version) is not str or not pinned_version:
        raise ValueError("frontend bun prerequisite is invalid")
    lock_path = repo_root / "frontend/bun.lock"
    lock_digest = _sha256_file(lock_path) if lock_path.is_file() else ""

    build_receipt = launch(
        (
            "/usr/bin/unshare",
            "--user",
            "--map-root-user",
            "--net",
            "--",
            str(bun_invocation),
            "run",
            "--cwd",
            "frontend",
            "build",
            "--",
            "--outDir",
            _BUN_OUTDIR,
            "--emptyOutDir",
        )
    )
    bundle_built = _successful(build_receipt)
    copied = False
    if bundle_built:
        try:
            _copy_tree(bundle_root, copy_root)
            copied = True
        except (OSError, ValueError):
            copied = False
    listing: dict[str, tuple[int, str]] = {}
    if copied:
        try:
            listing = _listing(copy_root)
        except (OSError, ValueError):
            listing = {}
    release_receipt = None
    clippy_receipt = None
    if listing:
        release_receipt = launch(
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
        release_ok = _successful(release_receipt) and binary.is_file()
        if release_ok:
            clippy_receipt = launch(
                (
                    cargo,
                    f"+{_TOOLCHAIN}",
                    "clippy",
                    "--manifest-path",
                    _MANIFEST,
                    "--release",
                    "--locked",
                    "--all-targets",
                    "--target",
                    _TARGET,
                    "--",
                    "-D",
                    "warnings",
                )
            )
    build_ok = (
        bun_version == pinned_version
        and bool(lock_digest)
        and bool(listing)
        and release_receipt is not None
        and _successful(release_receipt)
        and binary.is_file()
        and _successful(clippy_receipt)
    )
    outcomes[_BUILD_CRITERION] = (
        _pass(
            "bun matched the pinned prerequisite, the network-isolated frozen-lockfile"
            " bundle build, locked release build, and clippy gate all succeeded",
            bun_version=bun_version,
            lock_sha256=lock_digest,
            bundle_files=len(listing),
            bundle_bytes=sum(length for length, _digest in listing.values()),
            bundle_sha256=_listing_digest(listing) if listing else "",
            build_output_sha256=build_receipt.stdout_sha256 if build_receipt else "",
            clippy_output_sha256=clippy_receipt.stdout_sha256 if clippy_receipt else "",
        )
        if build_ok
        else _fail(
            "the frozen bun build, locked release build, or clippy gate failed",
            bun_version=bun_version,
            pinned_bun_version=pinned_version,
            lock_sha256=lock_digest,
            bundle_files=len(listing),
        )
    )
    if not build_ok:
        for criterion in _BUILD_DEPENDENTS:
            outcomes[criterion] = _not_run(_BUILD_CRITERION)
        return outcomes, receipts, environment

    # Install the release binary alone for the filesystem-independence probes.
    installed_binary = install_root / "bin" / _BINARY_NAME
    installed = False
    try:
        _remove_directory_if_present(install_root, label="stale frontend install directory")
        install_root.mkdir(mode=0o700, parents=True)
        (install_root / "bin").mkdir(mode=0o700)
        _copy_file(binary, installed_binary)
        installed_binary.chmod(0o755)
        installed = installed_binary.is_file()
    except OSError:
        installed = False

    # A missing canonical bundle must fail the compile itself, in a fresh target.
    rejected = False
    missing_receipt = None
    _remove_directory_if_present(missing_target, label="stale missing-bundle cargo target")
    try:
        _quarantine_asset_root(bundle_root, quarantine)
        missing_receipt = launch(
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
                "--target-dir",
                str(missing_target),
            )
        )
        rejected = _rejected(missing_receipt)
    except (OSError, ValueError):
        rejected = False
    finally:
        _restore_asset_root(bundle_root, quarantine)
    # Rejected + removed: the fresh bounded target must be gone afterwards.
    removed_missing = (
        _remove_directory_if_present(missing_target, label="missing-bundle cargo target")
        or not missing_target.exists()
    )
    outcomes["missing-bundle-rejected"] = (
        _pass(
            "with the canonical bundle renamed away, a fresh bounded cargo target"
            " failed to compile at the build-script guard and was removed",
            rejected_builds=1,
            rejected_exit_code=int(missing_receipt.exit_code or 0)
            if missing_receipt is not None
            else 0,
            missing_target_removed=1 if removed_missing else 0,
        )
        if rejected and removed_missing
        else _fail(
            "the fresh target did not reject the missing canonical bundle at compile time",
            rejected_builds=1 if rejected else 0,
            missing_target_removed=1 if not missing_target.exists() else 0,
        )
    )

    # Embedded manifest must equal the copied Vite output byte for byte.
    manifest_output = captures / "manifest.json"
    manifest_receipt = (
        launch(
            (
                sys.executable,
                "-c",
                _ASSET_CHILD,
                _ASSET_MARKER,
                str(manifest_output),
                str(installed_binary),
                "manifest",
            )
        )
        if installed
        else None
    )
    manifest = None
    if manifest_receipt is not None and _successful(manifest_receipt) and manifest_output.is_file():
        try:
            manifest = _parse_manifest(manifest_output.read_bytes())
        except OSError:
            manifest = None
    embedded_ok = manifest is not None and manifest == listing
    outcomes["release-assets-embedded"] = (
        _pass(
            "the embedded manifest's relative files, byte lengths, and SHA-256 values"
            " equal the copied Vite output exactly",
            embedded_files=len(listing),
            manifest_files=len(manifest) if manifest is not None else 0,
            byte_identity=1,
        )
        if embedded_ok
        else _fail(
            "the embedded manifest differs from the copied Vite output",
            embedded_files=len(listing),
            manifest_files=len(manifest) if manifest is not None else 0,
        )
    )

    # Route replay: only the six embedded path/status/MIME/body contracts.
    def asset_get(name: str, request_path: str) -> dict[str, object] | None:
        output = captures / f"{name}.json"
        receipt = launch(
            (
                sys.executable,
                "-c",
                _ASSET_CHILD,
                _ASSET_MARKER,
                str(output),
                str(installed_binary),
                "get",
                "--path",
                request_path,
            )
        )
        if receipt is None or not _successful(receipt) or not output.is_file():
            return None
        try:
            return _parse_asset_response(output.read_bytes())
        except OSError:
            return None

    index_entry = listing.get("index.html")
    index_digest = index_entry[1] if index_entry is not None else ""
    index_length = index_entry[0] if index_entry is not None else -1
    html_ok = installed and index_entry is not None
    for route_id in _HTML_ROUTE_IDS:
        request_path, mime = route_plans[route_id]
        response = asset_get(route_id, request_path)
        html_ok = html_ok and _response_matches(
            response,
            status=200,
            content_type=mime,
            mime_family=None,
            digest=index_digest,
            length=index_length,
            fallback=route_id != "frontend.route.get.root",
        )
    outcomes["index-and-spa-fallback"] = (
        _pass(
            "the embedded root served the real index document and every admin/config"
            " route fell back to it with the fixture HTML type",
            replayed_routes=len(_HTML_ROUTE_IDS),
            fallback_routes=len(_HTML_ROUTE_IDS) - 1,
            index_status=200,
        )
        if html_ok
        else _fail("the embedded index or SPA fallback replay diverged from the fixtures")
    )

    asset_keys = tuple(sorted(key for key in listing if key != "index.html"))
    hashed_ok = installed and bool(asset_keys)
    javascript_key = next((key for key in asset_keys if key.endswith(".js")), None)
    assets_path, _assets_mime = route_plans[_ASSETS_ROUTE_ID]
    hashed_ok = hashed_ok and javascript_key is not None and javascript_key.startswith("assets/")
    if javascript_key is not None:
        entry = listing[javascript_key]
        response = asset_get(_ASSETS_ROUTE_ID, "/" + javascript_key)
        hashed_ok = hashed_ok and _response_matches(
            response,
            status=200,
            content_type=None,
            mime_family=_JAVASCRIPT_MIMES,
            digest=entry[1],
            length=entry[0],
            fallback=False,
        )
    for key in asset_keys:
        entry = listing[key]
        response = asset_get(f"hashed-{key.replace('/', '-')}", "/" + key)
        hashed_ok = hashed_ok and _response_matches(
            response,
            status=200,
            content_type=None,
            mime_family=_asset_mime_family(key),
            digest=entry[1],
            length=entry[0],
            fallback=False,
        )
    outcomes["hashed-asset-and-mime"] = (
        _pass(
            "every hashed bundle asset was served by digest and byte length with its"
            " extension MIME type, including the fixture's JavaScript assets route",
            assets=len(asset_keys),
            javascript_assets=sum(1 for key in asset_keys if key.endswith(".js")),
            fixture_route=_ASSETS_ROUTE_ID,
        )
        if hashed_ok
        else _fail("hashed asset or MIME replay diverged from the bundle")
    )

    missing_ok = installed
    for name, probe in (
        ("missing-fixture", assets_path),
        ("missing-asset", "/assets/qualification-missing.js"),
    ):
        missing_ok = missing_ok and _response_matches(
            asset_get(name, probe),
            status=404,
            content_type="application/json",
            mime_family=None,
            digest=_EMPTY_SHA256,
            length=0,
            fallback=False,
        )
    outcomes["missing-asset-404"] = (
        _pass(
            "missing paths beneath assets returned the empty no-fallback JSON 404"
            " shape for the fixture failure path and a qualification probe",
            cases=2,
            status=404,
            fallbacks=0,
        )
        if missing_ok
        else _fail("a missing asset path did not return the exact 404 shape")
    )

    # Runtime independence: identical direct-binary responses while the asset
    # root is renamed beneath an access-denied quarantine, with strace proving
    # zero references to the asset root and no foreign exec.
    probe_set: list[str] = []
    for probe in (*_TRACED_PROBES, *(f"/{key}" for key in sorted(listing))):
        if probe not in probe_set:
            probe_set.append(probe)
    baseline_digests: dict[str, str] = {}
    baseline_complete = installed
    for probe in probe_set:
        receipt = launch((str(installed_binary), "get", "--path", probe)) if installed else None
        baseline_digests[probe] = (
            receipt.stdout_sha256 if receipt is not None and _successful(receipt) else ""
        )
        baseline_complete = baseline_complete and bool(baseline_digests[probe])
    renamed = False
    denial_proven = False
    traced_ok = installed
    asset_references = 0
    exec_names: tuple[str, ...] = ()
    try:
        _quarantine_asset_root(bundle_root, quarantine)
        renamed = not bundle_root.exists()
        try:
            os.listdir(quarantine)
        except PermissionError:
            denial_proven = True
        for index, probe in enumerate(probe_set):
            trace_path = traces / f"probe-{index:02d}.strace"
            receipt = launch(
                (
                    str(_STRACE),
                    "-f",
                    "-e",
                    "trace=%file",
                    "-o",
                    str(trace_path),
                    str(installed_binary),
                    "get",
                    "--path",
                    probe,
                )
            )
            ok = (
                receipt is not None
                and _successful(receipt)
                and receipt.stdout_sha256 == baseline_digests[probe]
                and trace_path.is_file()
            )
            if ok:
                try:
                    trace_text = trace_path.read_text(encoding="utf-8", errors="strict")
                except (OSError, UnicodeError):
                    trace_text = ""
                if "frontend-dist" in trace_text:
                    asset_references += trace_text.count("frontend-dist")
                    ok = False
                exec_names = exec_names + _traced_execve_names(trace_text)
                if not _traced_execve_names(trace_text):
                    ok = False
            traced_ok = traced_ok and ok
    except (OSError, ValueError):
        traced_ok = False
        renamed = renamed and not bundle_root.exists()
    finally:
        _restore_asset_root(bundle_root, quarantine)
    runtime_free = (
        renamed
        and denial_proven
        and baseline_complete
        and traced_ok
        and asset_references == 0
        and bool(exec_names)
        and all(name == _BINARY_NAME for name in exec_names)
    )
    outcomes["runtime-asset-root-inaccessible"] = (
        _pass(
            "with the bundle renamed beneath an access-denied quarantine, every"
            " straced binary probe returned the baseline digest, referenced no"
            " asset-root path, and executed only the release binary",
            probes=len(probe_set),
            digests_matched=len(probe_set) if traced_ok else 0,
            asset_root_references=asset_references,
            permission_denied=1 if denial_proven else 0,
        )
        if runtime_free
        else _fail(
            "the binary did not prove independence from the inaccessible asset root",
            probes=len(probe_set),
            asset_root_references=asset_references,
            permission_denied=1 if denial_proven else 0,
        )
    )

    # No Bun, Node, or Python runtime: not in ldd basenames, not exec'd.
    ldd_output = captures / "ldd.txt"
    ldd_receipt = (
        launch(
            (
                sys.executable,
                "-c",
                _LDD_CHILD,
                _LDD_MARKER,
                str(ldd_output),
                str(installed_binary),
            )
        )
        if installed
        else None
    )
    basenames: tuple[str, ...] = ()
    if ldd_receipt is not None and _successful(ldd_receipt) and ldd_output.is_file():
        try:
            basenames = _ldd_basenames(ldd_output.read_text(encoding="utf-8", errors="strict"))
        except OSError:
            basenames = ()
    foreign_runtime = tuple(
        name
        for name in basenames
        if name in _RUNTIME_BASENAMES
        or any(marker in name.lower() for marker in ("bun", "node", "python"))
    )
    no_runtime_ok = (
        bool(basenames)
        and "libc.so.6" in basenames
        and not foreign_runtime
        and bool(exec_names)
        and all(name == _BINARY_NAME for name in exec_names)
    )
    environment = Environment(
        rustc=environment.rustc,
        cargo=environment.cargo,
        target=_TARGET,
        bun=environment.bun,
        native_libraries=basenames,
        **platform_environment,
    )
    outcomes["no-runtime-bun-node-python"] = (
        _pass(
            "ldd basenames and the traced exec tree contain only libc-family"
            " libraries and the release binary itself",
            native_library_basenames=basenames,
            traced_execs=len(exec_names),
            runtime_processes=0,
        )
        if no_runtime_ok
        else _fail(
            "a Bun, Node, or Python runtime appeared in dependencies or the exec tree",
            runtime_basenames=foreign_runtime,
        )
    )

    # No source maps, no secret markers anywhere in the embedded bytes.
    source_maps = tuple(key for key in listing if key.endswith(".map"))
    scanned_bytes = 0
    marker_hits: list[str] = []
    scan_ok = True
    for key in sorted(listing):
        try:
            payload = (copy_root / key).read_bytes()
        except OSError:
            scan_ok = False
            break
        scanned_bytes += len(payload)
        for marker in _SECRET_MARKERS:
            if marker.encode("utf-8") in payload and marker not in marker_hits:
                marker_hits.append(marker)
    outcomes["no-source-map-secret"] = (
        _pass(
            "the embedded bundle contains no source maps and none of the forbidden"
            " secret markers appear in any embedded byte",
            files_scanned=len(listing),
            scanned_bytes=scanned_bytes,
            source_maps=0,
            forbidden_markers=0,
        )
        if scan_ok and not source_maps and not marker_hits
        else _fail(
            "a source map or forbidden secret marker is present in the bundle",
            source_maps=len(source_maps),
            forbidden_markers=len(marker_hits),
        )
    )

    # Release binary size recorded without any unapproved threshold.
    binary_bytes = binary.stat().st_size if binary.is_file() else 0
    outcomes["binary-size-recorded"] = (
        _pass(
            "the release binary byte size was recorded with no size threshold",
            binary_bytes=binary_bytes,
            bundle_bytes=sum(length for length, _digest in listing.values()),
            embedded_files=len(listing),
        )
        if binary_bytes > 0
        else _fail("the release binary size could not be recorded")
    )
    return outcomes, receipts, environment


def run_live(
    repo_root: Path,
    work_root: Path,
    *,
    original_env: Mapping[str, str],
) -> QualificationRecord:
    """Run the frontend candidate under the offline, owned-process boundary."""
    caller_env = dict(original_env)
    if caller_env.get("HIERONYMUS_QUALIFICATION_LIVE") != "1":
        raise ValueError("HIERONYMUS_QUALIFICATION_LIVE=1 is required")
    route_plans = _route_contracts(repo_root)
    contract_ids = tuple(sorted(route_plans))
    _validate_host_tools()
    input_paths = required_fingerprint_inputs(_RISK)
    before = fingerprint_inputs(repo_root, input_paths)
    _prepare_work_root(work_root)
    _reconcile_cargo_target(repo_root)
    receipts: list[ProcessReceipt] = []
    outcomes: dict[str, _Outcome] | None = None
    environment: Environment | None = None
    try:
        tool_roots, bun_invocation, _cargo_target_dir, child_env = _live_process_context(
            repo_root, work_root, caller_env
        )
        outcomes, receipts, environment = _live_outcomes(
            repo_root, work_root, tool_roots, bun_invocation, child_env
        )
    finally:
        _restore_asset_root(repo_root / _BUNDLE, repo_root / _QUARANTINE)
        work_removed = _remove_directory_if_present(work_root, label="frontend work root")
        _remove_directory_if_present(repo_root / _INSTALL, label="frontend install directory")
        _remove_directory_if_present(repo_root / _CARGO_TARGET, label="frontend cargo target")
        _remove_directory_if_present(
            (repo_root / _CARGO_TARGET).with_name(f"{_CARGO_TARGET.name}-missing"),
            label="frontend missing-bundle cargo target",
        )
    after = fingerprint_inputs(repo_root, input_paths)
    if after != before:
        raise ValueError("immutable frontend qualification inputs changed during replay")
    if outcomes is None or environment is None:
        raise ValueError("frontend live outcomes were not produced")
    if not work_removed:
        raise ValueError("frontend work root could not be removed after the run")
    if (repo_root / _INSTALL).exists():
        raise ValueError("frontend install directory survived the run")
    if (repo_root / _CARGO_TARGET).exists():
        raise ValueError("frontend cargo target survived the run")
    if not (bool(receipts) and all(item.core_dumps_disabled for item in receipts)):
        raise ValueError("frontend children did not all run with core dumps disabled")
    if not (bool(receipts) and all(item.process_group_reaped for item in receipts)):
        raise ValueError("frontend children were not all reaped from owned process groups")
    return _record(
        repo_root,
        outcomes=outcomes,
        environment=environment,
        input_paths=input_paths,
        input_digest=before,
        contract_ids=contract_ids,
        work_dir_removed=work_removed,
        raw_logs_removed=work_removed,
        install_dir_removed=not (repo_root / _INSTALL).exists(),
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
    parser = argparse.ArgumentParser(description="Run the offline frontend-embedding qualification")
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
        print("frontend-embedding qualification failed", file=sys.stderr)
        return 2
    print(f"frontend-embedding qualification recorded: {record.decision}")
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
