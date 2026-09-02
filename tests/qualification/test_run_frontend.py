"""Fake-injected and live-context tests for the frontend-embedding runner."""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.qualification.factories import write_fake_executable  # noqa: E402
from tools.qualification import run_frontend  # noqa: E402
from tools.qualification.model import REQUIRED_CRITERIA  # noqa: E402
from tools.qualification.process import ProcessReceipt, ToolRoots  # noqa: E402
from tools.qualification.run_frontend import run  # noqa: E402

_FRONTEND_CRITERIA = REQUIRED_CRITERIA["frontend-embedding"]
_ROUTE_CONTRACT_IDS = {
    "frontend.route.get.root",
    "frontend.route.get.admin",
    "frontend.route.get.admin.path",
    "frontend.route.get.assets.path",
    "frontend.route.get.config",
    "frontend.route.get.config.path",
}
_UNSHARE_PREFIX = ("/usr/bin/unshare", "--user", "--map-root-user", "--net", "--")
_BUN_BUILD_TAIL = (
    "run",
    "--cwd",
    "frontend",
    "build",
    "--",
    "--outDir",
    "../qualification/.artifacts/frontend-dist/current",
    "--emptyOutDir",
)


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


def _receipt_with_stdout(exit_code: int, stdout: bytes) -> ProcessReceipt:
    return ProcessReceipt(
        exit_code=exit_code,
        timed_out=False,
        stdout_sha256=hashlib.sha256(stdout).hexdigest(),
        stderr_sha256="0" * 64,
        duration_ms=1,
        process_group_reaped=True,
        core_dumps_disabled=True,
    )


# ---------------------------------------------------------------------------
# Fake-only runner tests (brief Step 1 snippets verbatim)
# ---------------------------------------------------------------------------


def test_frontend_runner_owns_embedded_path_contracts_only(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=write_fake_executable(tmp_path))
    assert set(record.contract_ids) == _ROUTE_CONTRACT_IDS
    assert "compatibility/fixtures/http/route-cases.json" in record.input_paths
    assert not {"http-host-validation", "browser-auth", "csrf"} & {
        item.criterion for item in record.evidence
    }


def test_frontend_failure_does_not_select_serve_dir(tmp_path: Path) -> None:
    executable = write_fake_executable(
        tmp_path, failed_criteria=("runtime-asset-root-inaccessible",)
    )
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "blocked"
    assert "embedded Svelte assets" in record.consequence


def test_frontend_fake_runner_requires_exact_criterion_set(tmp_path: Path) -> None:
    executable = write_fake_executable(tmp_path, failed_criteria=("not-a-criterion",))
    with pytest.raises(ValueError, match="criterion"):
        run(ROOT, tmp_path, executable=executable)


# ---------------------------------------------------------------------------
# Live context ordering
# ---------------------------------------------------------------------------


def test_frontend_live_context_discovers_before_sanitizing(
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
    bun = tmp_path / "bun"
    safe_env = {"SAFE": "yes"}
    target = ROOT / "qualification/.artifacts/cargo-target/frontend-embedding"
    original_env = {"HOME": "/sensitive", "TOKEN": "do-not-copy"}

    def discover(candidate: object) -> ToolRoots:
        calls.append(("discover-tools", candidate))
        return roots

    def discover_bun(candidate: object) -> Path:
        calls.append(("discover-bun", candidate))
        return bun

    def sanitize(
        work_root: Path,
        *,
        cargo_offline: bool,
        tool_roots: ToolRoots,
        cargo_target_dir: Path,
    ) -> dict[str, str]:
        calls.append(("sanitize", work_root, cargo_offline, tool_roots, cargo_target_dir))
        return safe_env

    monkeypatch.setattr(run_frontend, "discover_tool_roots", discover)
    monkeypatch.setattr(run_frontend, "_discover_bun_invocation", discover_bun)
    monkeypatch.setattr(run_frontend, "safe_subprocess_env", sanitize)

    actual = run_frontend._live_process_context(ROOT, tmp_path, original_env)

    assert actual == (roots, bun, target, safe_env)
    assert calls == [
        ("discover-tools", original_env),
        ("discover-bun", original_env),
        ("sanitize", tmp_path, True, roots, target),
    ]


def test_frontend_live_rejects_missing_opt_in_before_discovery(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        run_frontend,
        "discover_tool_roots",
        lambda _env: pytest.fail("tool discovery must not run without explicit opt-in"),
    )
    with pytest.raises(ValueError, match="HIERONYMUS_QUALIFICATION_LIVE=1"):
        run_frontend.run_live(ROOT, tmp_path, original_env={"HOME": "/sensitive"})


# ---------------------------------------------------------------------------
# Live spy fixtures
# ---------------------------------------------------------------------------


def _fake_bundle(root: Path) -> dict[str, bytes]:
    files = {
        "index.html": b"<!doctype html><title>Hieronymus Web Console</title>\n",
        "assets/index-AbCdEf12.js": b"console.log('fake bundle');\n",
        "assets/index-XyZw9876.css": b"body { color: rebeccapurple; }\n",
        "assets/font-Aa1Bb2Cc3.woff2": b"fake-woff2-bytes\n",
    }
    for relative, payload in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
    return files


def _fake_mime(key: str) -> str:
    if key.endswith(".html"):
        return "text/html; charset=utf-8"
    if key.endswith(".js"):
        return "text/javascript"
    if key.endswith(".css"):
        return "text/css"
    if key.endswith(".woff2"):
        return "font/woff2"
    return "application/octet-stream"


def _fake_resolve(files: dict[str, bytes], request_path: str) -> dict[str, object]:
    """Resolve like the embedded binary: from bytes, never from the filesystem."""
    lowered = request_path.lower()
    if "%2f" in lowered or "%5c" in lowered or "%00" in lowered:
        return _empty_response(400)
    if "\\" in request_path or "\0" in request_path or ".." in request_path.split("/"):
        return _empty_response(400)
    key = request_path.lstrip("/") or "index.html"
    payload = files.get(key)
    if payload is not None:
        return {
            "status": 200,
            "content_type": _fake_mime(key),
            "body_sha256": hashlib.sha256(payload).hexdigest(),
            "len": len(payload),
            "fallback": False,
        }
    if key.startswith("assets/"):
        return _empty_response(404)
    payload = files["index.html"]
    return {
        "status": 200,
        "content_type": "text/html; charset=utf-8",
        "body_sha256": hashlib.sha256(payload).hexdigest(),
        "len": len(payload),
        "fallback": True,
    }


def _empty_response(status: int) -> dict[str, object]:
    return {
        "status": status,
        "content_type": "application/json",
        "body_sha256": hashlib.sha256(b"").hexdigest(),
        "len": 0,
        "fallback": False,
    }


def _canonical_response(response: dict[str, object]) -> bytes:
    return json.dumps(response, sort_keys=True, separators=(",", ":")).encode()


def _probe_of(argv: tuple[str, ...]) -> str:
    return argv[argv.index("--path") + 1]


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
    monkeypatch.setattr(run_frontend, "_CARGO_TARGET", tmp_path / "cargo-target")
    monkeypatch.setattr(run_frontend, "_BUNDLE", tmp_path / "frontend-dist/current")
    monkeypatch.setattr(run_frontend, "_QUARANTINE", tmp_path / "frontend-dist/quarantine")
    monkeypatch.setattr(run_frontend, "_INSTALL", tmp_path / "install")
    monkeypatch.setattr(run_frontend, "discover_tool_roots", lambda _env: roots)
    monkeypatch.setattr(run_frontend, "_discover_bun_invocation", lambda _env: tmp_path / "bun")
    return roots


def _happy_spy(
    calls: list[tuple[tuple[str, ...], object]],
    roots: ToolRoots,
    bun: Path,
    safe_env: dict[str, str],
):
    """Serve every frontend live child from deterministic fixtures."""
    embedded: dict[str, bytes] = {}

    def spy(argv: tuple[str, ...], **kwargs: object) -> ProcessReceipt:
        environment = kwargs["env"]
        assert isinstance(environment, dict)
        calls.append((argv, environment))
        if run_frontend._TOOLCHAIN_MARKER in argv:
            Path(argv[argv.index(run_frontend._TOOLCHAIN_MARKER) + 1]).write_text(
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
        if run_frontend._BUN_MARKER in argv:
            Path(argv[argv.index(run_frontend._BUN_MARKER) + 1]).write_text(
                json.dumps({"bun": "1.4.0"}), encoding="utf-8"
            )
            return _successful_receipt(0)
        if tuple(argv[: len(_UNSHARE_PREFIX)]) == _UNSHARE_PREFIX:
            assert argv[len(_UNSHARE_PREFIX) :] == (str(bun), *_BUN_BUILD_TAIL)
            embedded.update(_fake_bundle(Path(str(run_frontend._BUNDLE))))
            return _successful_receipt(0)
        if argv[0] == str(roots.cargo_invocation):
            assert argv[1] == "+1.96.0"
            if "--target-dir" in argv:
                return _successful_receipt(1)
            if "clippy" in argv:
                return _successful_receipt(0)
            if "build" in argv:
                binary = Path(str(environment["CARGO_TARGET_DIR"])) / (
                    "x86_64-unknown-linux-gnu/release/frontend-embedding"
                )
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_bytes(b"fake-frontend-binary")
                return _successful_receipt(0)
            pytest.fail(f"unexpected cargo call: {argv}")
        if run_frontend._ASSET_MARKER in argv:
            output = Path(argv[argv.index(run_frontend._ASSET_MARKER) + 1])
            command = argv[argv.index(run_frontend._ASSET_MARKER) + 3]
            if command == "manifest":
                entries = []
                for relative in sorted(embedded):
                    payload = embedded[relative]
                    entries.append(
                        {
                            "key": relative,
                            "len": len(payload),
                            "sha256": hashlib.sha256(payload).hexdigest(),
                        }
                    )
                output.write_bytes(json.dumps(entries).encode())
            elif command == "get":
                response = _fake_resolve(embedded, _probe_of(argv))
                output.write_bytes(json.dumps(response).encode())
            else:  # pragma: no cover - fixture guard
                pytest.fail(f"unexpected asset command: {command}")
            return _successful_receipt(0)
        if Path(argv[0]).name == "frontend-embedding":
            response = _fake_resolve(embedded, _probe_of(argv))
            return _receipt_with_stdout(0, _canonical_response(response))
        if run_frontend._LDD_MARKER in argv:
            Path(argv[argv.index(run_frontend._LDD_MARKER) + 1]).write_text(
                "linux-vdso.so.1 (0x0000)\n"
                "libc.so.6 => /usr/lib/libc.so.6 (0x0000)\n"
                "libgcc_s.so.1 => /usr/lib/libgcc_s.so.1 (0x0000)\n",
                encoding="utf-8",
            )
            return _successful_receipt(0)
        if argv[0] == str(run_frontend._STRACE):
            probe = _probe_of(argv)
            trace = Path(argv[argv.index("-o") + 1])
            response = _fake_resolve(embedded, probe)
            trace.write_text(
                f'    4     execve("{run_frontend._INSTALL}/bin/frontend-embedding", '
                '["frontend-embedding", "get", "--path", "<probe>"], 0x...) = 0\n'
                '    4     openat(AT_FDCWD, "/etc/ld.so.cache",'
                " O_RDONLY|O_CLOEXEC) = 3\n"
                '    4     openat(AT_FDCWD, "/usr/lib/libc.so.6",'
                " O_RDONLY|O_CLOEXEC) = 3\n"
                f"# canonical response: {_canonical_response(response).decode()}\n",
                encoding="utf-8",
            )
            return _receipt_with_stdout(0, _canonical_response(response))
        pytest.fail(f"unexpected live child: {argv}")

    return spy


# ---------------------------------------------------------------------------
# Live children discipline
# ---------------------------------------------------------------------------


def test_frontend_live_children_reuse_safe_environment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    roots = _patch_live_boundaries(monkeypatch, tmp_path)
    bun = tmp_path / "bun"
    safe_env = {
        "HOME": str(tmp_path / "safe-home"),
        "CARGO_TARGET_DIR": str(tmp_path / "cargo-target"),
        "CARGO_NET_OFFLINE": "true",
    }
    monkeypatch.setattr(run_frontend, "safe_subprocess_env", lambda *_a, **_k: safe_env)
    original_env = {
        "HIERONYMUS_QUALIFICATION_LIVE": "1",
        "HOME": "/sensitive",
        "TOKEN": "do-not-copy",
    }
    calls: list[tuple[tuple[str, ...], object]] = []
    monkeypatch.setattr(run_frontend, "run_owned_process", _happy_spy(calls, roots, bun, safe_env))

    record = run_frontend.run_live(ROOT, tmp_path / "live", original_env=original_env)

    assert record.decision == "qualified"
    assert {item.criterion for item in record.evidence} == set(_FRONTEND_CRITERIA)
    assert all(item.status == "pass" for item in record.evidence)
    assert calls
    assert all(environment is safe_env for _, environment in calls)
    assert all(environment is not original_env for _, environment in calls)

    build_calls = [
        argv for argv, _ in calls if tuple(argv[: len(_UNSHARE_PREFIX)]) == _UNSHARE_PREFIX
    ]
    assert len(build_calls) == 1
    bun_argv = build_calls[0][len(_UNSHARE_PREFIX) :]
    assert bun_argv[0] == str(bun)
    assert bun_argv[1:] == _BUN_BUILD_TAIL

    cargo_calls = [argv for argv, _ in calls if argv[0] == str(roots.cargo_invocation)]
    assert len(cargo_calls) == 3
    assert all(argv[1] == "+1.96.0" for argv in cargo_calls)
    build_releases = [
        argv for argv in cargo_calls if "build" in argv and "--target-dir" not in argv
    ]
    missing_builds = [argv for argv in cargo_calls if "--target-dir" in argv]
    clippy_calls = [argv for argv in cargo_calls if "clippy" in argv]
    assert len(build_releases) == 1 and len(missing_builds) == 1 and len(clippy_calls) == 1
    assert "-D" in clippy_calls[0] and "warnings" in clippy_calls[0]

    strace_calls = [argv for argv, _ in calls if argv[0] == str(run_frontend._STRACE)]
    assert strace_calls
    assert all("-e" in argv and "trace=%file" in argv for argv in strace_calls)
    binary_calls = [argv for argv, _ in calls if Path(argv[0]).name == "frontend-embedding"]
    assert binary_calls
    ldd_calls = [argv for argv, _ in calls if run_frontend._LDD_MARKER in argv]
    assert len(ldd_calls) == 1

    serialized = json.dumps(json.loads(run_frontend.serialize_record(record)), sort_keys=True)
    for forbidden in (
        str(bun),
        str(roots.cargo_invocation),
        str(roots.cargo_resolved_target),
        str(roots.cargo_home),
        str(tmp_path / "cargo-target"),
        str(tmp_path / "install"),
        "/tmp/",
    ):
        assert forbidden not in serialized


def test_frontend_live_build_failure_writes_complete_blocked_record(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    roots = _patch_live_boundaries(monkeypatch, tmp_path)
    safe_env = {
        "HOME": str(tmp_path / "safe-home"),
        "CARGO_TARGET_DIR": str(tmp_path / "cargo-target"),
    }
    monkeypatch.setattr(run_frontend, "safe_subprocess_env", lambda *_a, **_k: safe_env)
    original_env = {"HIERONYMUS_QUALIFICATION_LIVE": "1"}
    calls: list[tuple[tuple[str, ...], object]] = []

    def build_failure_spy(argv: tuple[str, ...], **kwargs: object) -> ProcessReceipt:
        environment = kwargs["env"]
        assert isinstance(environment, dict)
        calls.append((argv, environment))
        if run_frontend._TOOLCHAIN_MARKER in argv:
            Path(argv[argv.index(run_frontend._TOOLCHAIN_MARKER) + 1]).write_text(
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
        if run_frontend._BUN_MARKER in argv:
            Path(argv[argv.index(run_frontend._BUN_MARKER) + 1]).write_text(
                json.dumps({"bun": "1.4.0"}), encoding="utf-8"
            )
            return _successful_receipt(0)
        if tuple(argv[: len(_UNSHARE_PREFIX)]) == _UNSHARE_PREFIX:
            return _successful_receipt(1)
        pytest.fail("no qualification child may launch after the frozen build failed")

    monkeypatch.setattr(run_frontend, "run_owned_process", build_failure_spy)

    record = run_frontend.run_live(ROOT, tmp_path / "live", original_env=original_env)

    assert record.decision == "blocked"
    assert "embedded Svelte assets" in record.consequence
    evidence = {item.criterion: item for item in record.evidence}
    assert set(evidence) == set(_FRONTEND_CRITERIA)
    assert evidence["bun-version-and-frozen-build"].status == "fail"
    assert all(
        item.status == "not-run" and item.not_run_reason == "bun-version-and-frozen-build"
        for name, item in evidence.items()
        if name != "bun-version-and-frozen-build"
    )
    assert record.cleanup.work_dir_removed is True
    assert record.cleanup.install_dir_removed is True
    assert record.cleanup.source_inputs_unchanged is True
    assert all(environment is safe_env for _, environment in calls)
    assert not any(argv[0] == str(roots.cargo_invocation) for argv, _ in calls)
