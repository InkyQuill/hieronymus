from __future__ import annotations

import hashlib
import io
import subprocess
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


def test_manifest_failures_report_invalid_manifest_and_missing_references(tmp_path: Path) -> None:
    manifest_path = tmp_path / "compatibility/manifest.json"
    manifest_path.parent.mkdir(parents=True)
    manifest_path.write_text("{}", encoding="utf-8")

    manifest, failures = manifest_failures(tmp_path)

    assert manifest is None
    assert failures == [
        "invalid manifest: manifest missing required fields: contracts, "
        "manifest_version, python_reference, test_ownership"
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
    "disposition": "preserve"
  }],
  "test_ownership": []
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
Test ownership: 3
By test-ownership disposition:
  implementation_internal: 1
  public_contract: 2"""
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


def test_checked_in_contracts_pass_without_modifying_repository() -> None:
    before_status = subprocess.run(
        ["git", "status", "--short", "--untracked-files=all"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    before_hashes = _compatibility_hashes()
    output = io.StringIO()

    exit_code = main(repo_root=ROOT, output=output)

    after_status = subprocess.run(
        ["git", "status", "--short", "--untracked-files=all"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    assert exit_code == 0, output.getvalue()
    assert output.getvalue() == (
        ROOT / "compatibility/fixtures/diagnostics/check-success.txt"
    ).read_text(encoding="utf-8")
    assert _compatibility_hashes() == before_hashes
    assert after_status == before_status


def _compatibility_hashes() -> dict[str, str]:
    return {
        str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted((ROOT / "compatibility").rglob("*"))
        if path.is_file()
    }
