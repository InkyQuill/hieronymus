from __future__ import annotations

import hashlib
import io
import os
import shutil
import socket
import subprocess
import urllib.request
from pathlib import Path

import pytest

from tools.compatibility import check as compatibility_check
from tools.compatibility.check import (
    GeneratedInventory,
    artifact_diffs,
    emit_report,
    inventory_coverage_failures,
    main,
    manifest_failures,
    render_parity_summary,
)
from tools.compatibility.model import Contract, Manifest
from tools.compatibility.model import TestOwnership as ManifestTestOwnership

ROOT = Path(__file__).resolve().parents[2]
CANONICAL_CHECK = (
    "uv",
    "run",
    "--no-cache",
    "--no-sync",
    "python",
    "-B",
    "-m",
    "tools.compatibility.check",
)


def _manifest(*, fixture: str = "fixture.json") -> Manifest:
    return Manifest(
        manifest_version=1,
        python_reference="0.7.0",
        contracts=(
            Contract(
                id="cli.command.hiero.status",
                surface="cli",
                acceptance_owner="Pavel Obruchnikov <me@inkyquill.net>",
                technical_owner="distribution-cutover",
                python_entry_point="hieronymus.cli:main",
                tests=("tests/test_cli.py",),
                fixture=fixture,
                rust_test_target="crates/hiero-cli/tests/cli_contract.rs::status",
                disposition="preserve",
                last_python_release="0.7.0",
                first_rust_release=None,
                implementation_status="outstanding",
            ),
            Contract(
                id="http.route.get.status",
                surface="http",
                acceptance_owner="Pavel Obruchnikov <me@inkyquill.net>",
                technical_owner="daemon-mcp-security",
                python_entry_point="hieronymus.service_http:status",
                tests=("tests/test_service_http.py",),
                fixture=fixture,
                rust_test_target="crates/hiero-daemon/tests/http_contract.rs::status",
                disposition="intentionally-change",
                last_python_release="0.7.0",
                first_rust_release=None,
                implementation_status="outstanding",
                adr="docs/adr/0012-local-service-authentication.md",
            ),
        ),
        test_ownership=(
            ManifestTestOwnership(
                node_id="tests/test_cli.py::test_status",
                disposition="public_contract",
                contract_ids=("cli.command.hiero.status",),
            ),
            ManifestTestOwnership(
                node_id="tests/test_helpers.py::test_parser",
                disposition="implementation_internal",
                reason="Python-only parser test.",
            ),
            ManifestTestOwnership(
                node_id="tests/test_service_http.py::test_status",
                disposition="public_contract",
                contract_ids=("http.route.get.status",),
            ),
        ),
        frontend_test_ownership=(),
    )


@pytest.mark.parametrize(
    ("relative_path", "expected_diagnostic"),
    [
        ("compatibility/snapshots/cli.json", "snapshot drift"),
        ("compatibility/snapshots/mcp.json", "snapshot drift"),
        ("compatibility/snapshots/http.json", "snapshot drift"),
        ("compatibility/snapshots/state.json", "snapshot drift"),
        ("compatibility/fixtures/database/minimal-python.sqlite", "fixture drift"),
    ],
)
def test_artifact_diffs_report_each_inventory_family_and_binary_state_fixture(
    tmp_path: Path,
    relative_path: str,
    expected_diagnostic: str,
) -> None:
    destination = tmp_path / relative_path
    destination.parent.mkdir(parents=True)
    destination.write_bytes(b"checked-in")

    assert artifact_diffs(tmp_path, {relative_path: b"generated"}) == [
        f"{expected_diagnostic}: {relative_path}"
    ]


def test_artifact_diffs_compare_exact_bytes_and_report_missing_files(tmp_path: Path) -> None:
    path = "compatibility/fixtures/cli/help-status.txt"
    destination = tmp_path / path
    destination.parent.mkdir(parents=True)
    destination.write_bytes(b"same words\r\n")

    assert artifact_diffs(tmp_path, {path: b"same words\n"}) == [f"fixture drift: {path}"]
    destination.unlink()
    assert artifact_diffs(tmp_path, {path: b"same words\n"}) == [
        f"missing generated artifact: {path}"
    ]


@pytest.mark.parametrize(
    "relative_path",
    [
        "compatibility/fixtures/mcp/orphan-tool/stale.json",
        "compatibility/fixtures/cli/stale-fixture.txt",
        "compatibility/snapshots/stale.json",
    ],
)
def test_artifact_diffs_reject_unexpected_generated_artifacts(
    tmp_path: Path, relative_path: str
) -> None:
    unexpected = tmp_path / relative_path
    unexpected.parent.mkdir(parents=True)
    unexpected.write_text("stale\n", encoding="utf-8")

    assert artifact_diffs(tmp_path, {}) == [f"unexpected generated artifact: {relative_path}"]


def test_manifest_failures_report_invalid_manifest_and_missing_references(tmp_path: Path) -> None:
    manifest_path = tmp_path / "compatibility/manifest.json"
    manifest_path.parent.mkdir(parents=True)
    manifest_path.write_text("{}", encoding="utf-8")

    manifest, failures = manifest_failures(tmp_path)

    assert manifest is None
    assert failures == [
        "invalid manifest: manifest missing required fields: contracts, "
        "frontend_test_ownership, manifest_version, python_reference, test_ownership"
    ]

    manifest_path.write_text(
        """{
  "manifest_version": 1,
  "python_reference": "0.7.0",
  "contracts": [{
    "id": "cli.command.hiero.status",
    "surface": "cli",
    "acceptance_owner": "Pavel Obruchnikov <me@inkyquill.net>",
    "technical_owner": "distribution-cutover",
    "python_entry_point": "hieronymus.cli:main",
    "tests": ["tests/missing.py"],
    "fixture": "compatibility/fixtures/missing.json",
    "rust_test_target": "crates/hiero-cli/tests/cli_contract.rs::status",
    "disposition": "preserve",
    "last_python_release": "0.7.0",
    "first_rust_release": null,
    "implementation_status": "outstanding"
  }],
  "test_ownership": [],
  "frontend_test_ownership": []
}\n""",
        encoding="utf-8",
    )

    manifest, failures = manifest_failures(tmp_path)

    assert manifest is not None
    assert failures == [
        "missing fixture path: compatibility/fixtures/missing.json",
        "missing test path: tests/missing.py",
    ]


def test_inventory_coverage_rejects_unreviewed_items() -> None:
    failures = inventory_coverage_failures(
        _manifest(),
        {
            "cli": {"cli.command.hiero.status", "cli.command.hiero.unreviewed"},
            "mcp": {"mcp.tool.hieronymus_status"},
            "http/frontend": {"http.route.get.status"},
            "state": set(),
        },
    )

    assert failures == [
        "unreviewed inventory item: cli: cli.command.hiero.unreviewed",
        "unreviewed inventory item: mcp: mcp.tool.hieronymus_status",
    ]


def test_compatibility_gate_validates_official_mcp_schema_offline(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    node_id = (
        "tests/compatibility/test_check.py::"
        "test_compatibility_gate_validates_official_mcp_schema_offline"
    )
    assert compatibility_check.inventory_state._internal_test_reason(node_id) == (
        "Validates the offline MCP schema authority and compatibility-gate integration; "
        "implementation-internal self-test, not a public contract."
    )

    def reject_network(*_args: object, **_kwargs: object) -> None:
        raise AssertionError("compatibility validation attempted network access")

    monkeypatch.setattr(socket, "create_connection", reject_network)
    monkeypatch.setattr(urllib.request, "urlopen", reject_network)
    protocol_fixture = compatibility_check.inventory_mcp._protocol_fixture

    def invalid_protocol_fixture(snapshot: dict[str, object]) -> dict[str, object]:
        fixture = protocol_fixture(snapshot)
        del fixture["target"]["tools_list"]["response"]["result"]["ttlMs"]
        return fixture

    monkeypatch.setattr(
        compatibility_check.inventory_mcp,
        "_protocol_fixture",
        invalid_protocol_fixture,
    )

    with pytest.raises(
        ValueError,
        match=(
            r"invalid official MCP ListToolsResultResponse: "
            r"result: 'ttlMs' is a required property"
        ),
    ):
        compatibility_check._mcp_inventory()


def test_parity_summary_counts_are_derived_from_manifest() -> None:
    assert (
        render_parity_summary(_manifest())
        == """Parity summary
Contracts: 2
By surface:
  cli: 1
  http: 1
By disposition:
  intentionally-change: 1
  preserve: 1
By technical owner:
  daemon-mcp-security: 1
  distribution-cutover: 1
Implementation state:
  implemented (0):
    (none)
  changed (1):
    http.route.get.status
  removed (0):
    (none)
  outstanding (2):
    cli.command.hiero.status
    http.route.get.status
Test ownership: 3
By test-ownership disposition:
  implementation_internal: 1
  public_contract: 2
Frontend test ownership: 0
By frontend test-ownership disposition:"""
    )


def test_report_sorts_failures_before_summary() -> None:
    output = io.StringIO()

    emit_report(["z failure", "a failure"], "Parity summary\nContracts: 2", output)

    assert (
        output.getvalue()
        == """Compatibility check failed
a failure
z failure
Parity summary
Contracts: 2
"""
    )


def test_main_exits_one_with_sorted_drift_and_parity_summary(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = "compatibility/snapshots/cli.json"
    destination = tmp_path / path
    destination.parent.mkdir(parents=True)
    destination.write_bytes(b"drifted")
    manifest = _manifest()
    generated = GeneratedInventory(
        artifacts={path: b"expected"},
        inventory_ids={
            "cli": {"cli.command.hiero.status"},
            "mcp": set(),
            "http/frontend": {"http.route.get.status"},
            "state": set(),
        },
    )
    monkeypatch.setattr(
        compatibility_check,
        "manifest_failures",
        lambda _repo_root: (manifest, ["z manifest failure", "a manifest failure"]),
    )
    monkeypatch.setattr(
        compatibility_check,
        "generate_inventory",
        lambda _repo_root: generated,
    )
    output = io.StringIO()

    exit_code = main(repo_root=tmp_path, output=output)

    assert exit_code == 1
    assert output.getvalue().splitlines()[:4] == [
        "Compatibility check failed",
        "a manifest failure",
        "snapshot drift: compatibility/snapshots/cli.json",
        "z manifest failure",
    ]
    assert "Parity summary\nContracts: 2" in output.getvalue()


def test_canonical_check_passes_without_writing_repo_or_caller_state(tmp_path: Path) -> None:
    repo_copy = _copy_tracked_repository(tmp_path / "repo")

    result = _invoke_canonical_check(repo_copy, tmp_path / "caller")

    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout == (
        repo_copy / "compatibility/fixtures/diagnostics/check-success.txt"
    ).read_text(encoding="utf-8")


def test_canonical_check_reports_one_sorted_composite_failure_without_writing(
    tmp_path: Path,
) -> None:
    repo_copy = _copy_tracked_repository(tmp_path / "repo")
    expected_failures = []
    for relative_path, diagnostic in (
        ("compatibility/snapshots/cli.json", "snapshot drift"),
        ("compatibility/snapshots/mcp.json", "snapshot drift"),
        ("compatibility/snapshots/http.json", "snapshot drift"),
        ("compatibility/snapshots/state.json", "snapshot drift"),
        ("compatibility/fixtures/database/minimal-python.sqlite", "fixture drift"),
    ):
        target = repo_copy / relative_path
        target.write_bytes(target.read_bytes() + b"\nDRIFT")
        expected_failures.append(f"{diagnostic}: {relative_path}")
    for relative_path in (
        "compatibility/snapshots/z-stale.json",
        "compatibility/fixtures/cli/a-stale.txt",
    ):
        orphan = repo_copy / relative_path
        orphan.parent.mkdir(parents=True, exist_ok=True)
        orphan.write_text("stale\n", encoding="utf-8")
        expected_failures.append(f"unexpected generated artifact: {relative_path}")

    result = _invoke_canonical_check(repo_copy, tmp_path / "caller")

    assert result.returncode == 1
    failure_lines = result.stdout.split("Parity summary\n", 1)[0].splitlines()[1:]
    assert failure_lines == sorted(failure_lines)
    assert failure_lines == sorted(expected_failures)


def _copy_tracked_repository(destination: Path) -> Path:
    destination.mkdir(parents=True)
    tracked = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    for raw_path in tracked:
        if not raw_path:
            continue
        relative_path = Path(os.fsdecode(raw_path))
        source = ROOT / relative_path
        target = destination / relative_path
        target.parent.mkdir(parents=True, exist_ok=True)
        if source.is_symlink():
            target.symlink_to(os.readlink(source))
        else:
            shutil.copy2(source, target)
    # CI performs ``uv sync --dev`` before the canonical no-sync gate.  Reuse
    # that already-synced environment and the Bun-installed frontend dependencies
    # without copying or mutating either dependency tree.
    (destination / ".venv").symlink_to(ROOT / ".venv", target_is_directory=True)
    (destination / "frontend/node_modules").symlink_to(
        ROOT / "frontend/node_modules", target_is_directory=True
    )
    return destination


def _tree_hashes(root: Path) -> dict[str, str]:
    if not root.exists():
        return {".": "missing"}
    hashes: dict[str, str] = {".": f"directory:{root.stat().st_mode:o}"}
    for path in sorted(root.rglob("*")):
        relative_path = path.relative_to(root).as_posix()
        if path.is_symlink():
            hashes[relative_path] = f"symlink:{path.lstat().st_mode:o}:{os.readlink(path)}"
        elif path.is_dir():
            hashes[f"{relative_path}/"] = f"directory:{path.stat().st_mode:o}"
        else:
            hashes[relative_path] = (
                f"file:{path.stat().st_mode:o}:{hashlib.sha256(path.read_bytes()).hexdigest()}"
            )
    return hashes


def _invoke_canonical_check(repo_copy: Path, caller_root: Path) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    # Exercise the repository's synced environment as CI does, independent of
    # the outer pytest process's active virtualenv.
    environment.pop("VIRTUAL_ENV", None)
    path_entries = [
        entry
        for entry in environment["PATH"].split(os.pathsep)
        if not entry.endswith("/.local/share/mise/shims")
    ]
    environment.update(
        {
            "HOME": str(caller_root / "home"),
            "XDG_CACHE_HOME": str(caller_root / "xdg-cache"),
            "XDG_CONFIG_HOME": str(caller_root / "xdg-config"),
            "XDG_DATA_HOME": str(caller_root / "xdg-data"),
            "UV_CACHE_DIR": str(caller_root / "uv-cache"),
            "PYTHONPATH": os.pathsep.join((str(repo_copy), str(repo_copy / "src"))),
            "PATH": os.pathsep.join((str(repo_copy / ".venv/bin"), *path_entries)),
        }
    )
    before_repo = _tree_hashes(repo_copy)
    before_caller = _tree_hashes(caller_root)

    result = subprocess.run(
        CANONICAL_CHECK,
        cwd=repo_copy,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )

    assert _tree_hashes(repo_copy) == before_repo
    assert _tree_hashes(caller_root) == before_caller
    return result
