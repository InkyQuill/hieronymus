from __future__ import annotations

import json
from pathlib import Path

import pytest

from tools.compatibility.model import load_manifest, validate_manifest

ROOT = Path(__file__).resolve().parents[2]


def complete_contract(*, contract_id: str = "cli.hiero", **overrides: object) -> dict[str, object]:
    contract: dict[str, object] = {
        "id": contract_id,
        "surface": "cli",
        "acceptance_owner": "Pavel Obruchnikov <me@inkyquill.net>",
        "technical_owner": "distribution-cutover",
        "python_entry_point": "hieronymus.cli:main",
        "tests": ["tests/test_cli.py"],
        "fixture": "README.md",
        "rust_test_target": "crates/hiero/tests/cli.rs",
        "disposition": "preserve",
        "last_python_release": "0.7.0",
        "first_rust_release": None,
        "implementation_status": "outstanding",
    }
    contract.update(overrides)
    return contract


def write_manifest(path: Path, contracts: list[dict[str, object]]) -> None:
    path.write_text(
        json.dumps(
            {
                "manifest_version": 1,
                "python_reference": "0.7.0",
                "contracts": contracts,
                "test_ownership": [],
                "frontend_test_ownership": [],
            }
        ),
        encoding="utf-8",
    )


def test_checked_in_manifest_is_valid() -> None:
    manifest = load_manifest(ROOT / "compatibility/manifest.json")

    assert validate_manifest(manifest, ROOT) == []


def test_duplicate_complete_contract_id_is_rejected(tmp_path: Path) -> None:
    path = tmp_path / "manifest.json"
    write_manifest(path, [complete_contract(), complete_contract()])

    errors = validate_manifest(load_manifest(path), ROOT)

    assert errors == ["duplicate contract id: cli.hiero"]


def test_missing_fixture_and_test_paths_are_rejected(tmp_path: Path) -> None:
    path = tmp_path / "manifest.json"
    write_manifest(
        path,
        [
            complete_contract(
                fixture="compatibility/fixtures/missing.json",
                tests=["tests/missing_test.py"],
            )
        ],
    )

    assert validate_manifest(load_manifest(path), ROOT) == [
        "missing fixture path: compatibility/fixtures/missing.json",
        "missing test path: tests/missing_test.py",
    ]


@pytest.mark.parametrize("disposition", ["intentionally-change", "remove"])
def test_blank_acceptance_owner_and_missing_adr_for_changed_contract_are_rejected(
    tmp_path: Path, disposition: str
) -> None:
    path = tmp_path / "manifest.json"
    write_manifest(
        path,
        [
            complete_contract(
                acceptance_owner=" ",
                disposition=disposition,
            )
        ],
    )

    assert validate_manifest(load_manifest(path), ROOT) == [
        "blank acceptance owner: cli.hiero",
        f"missing adr for {disposition} contract: cli.hiero",
    ]


def test_loader_rejects_missing_required_contract_field(tmp_path: Path) -> None:
    path = tmp_path / "manifest.json"
    contract = complete_contract()
    del contract["fixture"]
    write_manifest(path, [contract])

    with pytest.raises(ValueError, match="fixture"):
        load_manifest(path)


def test_loader_rejects_blank_technical_owner(tmp_path: Path) -> None:
    path = tmp_path / "manifest.json"
    write_manifest(path, [complete_contract(technical_owner=" ")])

    with pytest.raises(ValueError, match="technical_owner"):
        load_manifest(path)


def test_schema_requires_nonblank_acceptance_owner_and_adr_for_changes() -> None:
    schema = json.loads((ROOT / "compatibility/manifest.schema.json").read_text(encoding="utf-8"))
    contract_schema = schema["properties"]["contracts"]["items"]

    assert contract_schema["properties"]["acceptance_owner"]["pattern"] == r"\S"
    change_control = contract_schema["allOf"][0]
    assert change_control["if"]["properties"]["disposition"]["enum"] == [
        "intentionally-change",
        "remove",
    ]
    assert change_control["then"]["required"] == ["adr"]
    assert change_control["then"]["properties"]["adr"]["pattern"] == r"\S"


def test_absolute_fixture_and_parent_test_paths_outside_repository_are_rejected(
    tmp_path: Path,
) -> None:
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    outside_fixture = tmp_path / "outside-fixture.json"
    outside_fixture.write_text("{}", encoding="utf-8")
    outside_test = tmp_path / "outside_test.py"
    outside_test.write_text("", encoding="utf-8")
    manifest_path = tmp_path / "manifest.json"
    write_manifest(
        manifest_path,
        [
            complete_contract(
                fixture=str(outside_fixture),
                tests=["../outside_test.py"],
            )
        ],
    )

    assert validate_manifest(load_manifest(manifest_path), repo_root) == [
        f"fixture path outside repository root: {outside_fixture}",
        "test path outside repository root: ../outside_test.py",
    ]


def test_symlinked_fixture_and_test_paths_outside_repository_are_rejected(tmp_path: Path) -> None:
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    outside_fixture = tmp_path / "outside-fixture.json"
    outside_fixture.write_text("{}", encoding="utf-8")
    outside_test = tmp_path / "outside_test.py"
    outside_test.write_text("", encoding="utf-8")
    fixture_link = repo_root / "fixture-link.json"
    fixture_link.symlink_to(outside_fixture)
    tests_dir = repo_root / "tests"
    tests_dir.mkdir()
    test_link = tests_dir / "test-link.py"
    test_link.symlink_to(outside_test)
    manifest_path = tmp_path / "manifest.json"
    write_manifest(
        manifest_path,
        [complete_contract(fixture="fixture-link.json", tests=["tests/test-link.py"])],
    )

    assert validate_manifest(load_manifest(manifest_path), repo_root) == [
        "fixture path outside repository root: fixture-link.json",
        "test path outside repository root: tests/test-link.py",
    ]


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("surface", "private-api"),
        ("technical_owner", "frontend-team"),
        ("disposition", "defer"),
    ],
)
def test_loader_rejects_unknown_enum_value(tmp_path: Path, field: str, value: str) -> None:
    path = tmp_path / "manifest.json"
    write_manifest(path, [complete_contract(**{field: value})])

    with pytest.raises(ValueError, match=field):
        load_manifest(path)


def test_loader_parses_typed_public_and_internal_test_ownership(tmp_path: Path) -> None:
    path = tmp_path / "manifest.json"
    payload = {
        "manifest_version": 1,
        "python_reference": "0.7.0",
        "contracts": [complete_contract()],
        "test_ownership": [
            {
                "node_id": "tests/test_cli.py::test_public_contract",
                "disposition": "public_contract",
                "contract_ids": ["cli.hiero"],
            },
            {
                "node_id": "tests/test_helpers.py::test_private_helper",
                "disposition": "implementation_internal",
                "reason": "Exercises a Python-only helper implementation.",
            },
        ],
        "frontend_test_ownership": [],
    }
    path.write_text(json.dumps(payload), encoding="utf-8")

    manifest = load_manifest(path)

    assert manifest.test_ownership[0].contract_ids == ("cli.hiero",)
    assert manifest.test_ownership[0].reason is None
    assert manifest.test_ownership[1].contract_ids == ()
    assert manifest.test_ownership[1].reason == "Exercises a Python-only helper implementation."


@pytest.mark.parametrize(
    ("ownership", "error"),
    [
        (
            {
                "node_id": "tests/test_cli.py::test_public_contract",
                "disposition": "public_contract",
            },
            "contract_ids",
        ),
        (
            {
                "node_id": "tests/test_helpers.py::test_private_helper",
                "disposition": "implementation_internal",
            },
            "reason",
        ),
        (
            {
                "node_id": "tests/test_cli.py::test_public_contract",
                "disposition": "public_contract",
                "contract_ids": ["cli.hiero"],
                "reason": "not allowed",
            },
            "unexpected fields",
        ),
        (
            {
                "node_id": "tests/test_helpers.py::test_private_helper",
                "disposition": "implementation_internal",
                "reason": "Python-only helper.",
                "contract_ids": ["cli.hiero"],
            },
            "unexpected fields",
        ),
    ],
)
def test_loader_enforces_conditional_test_ownership_fields(
    tmp_path: Path,
    ownership: dict[str, object],
    error: str,
) -> None:
    path = tmp_path / "manifest.json"
    payload = {
        "manifest_version": 1,
        "python_reference": "0.7.0",
        "contracts": [complete_contract()],
        "test_ownership": [ownership],
        "frontend_test_ownership": [],
    }
    path.write_text(json.dumps(payload), encoding="utf-8")

    with pytest.raises(ValueError, match=error):
        load_manifest(path)


def test_loader_rejects_empty_test_path_segment_for_internal_ownership(tmp_path: Path) -> None:
    path = tmp_path / "manifest.json"
    payload = {
        "manifest_version": 1,
        "python_reference": "0.7.0",
        "contracts": [complete_contract()],
        "test_ownership": [
            {
                "node_id": "tests/::test_private_helper",
                "disposition": "implementation_internal",
                "reason": "Exercises a Python-only helper implementation.",
            }
        ],
        "frontend_test_ownership": [],
    }
    path.write_text(json.dumps(payload), encoding="utf-8")

    with pytest.raises(ValueError, match="pytest node id"):
        load_manifest(path)


def test_manifest_validation_rejects_duplicate_nodes_unknown_contracts_and_file_mismatches(
    tmp_path: Path,
) -> None:
    path = tmp_path / "manifest.json"
    payload = {
        "manifest_version": 1,
        "python_reference": "0.7.0",
        "contracts": [complete_contract()],
        "test_ownership": [
            {
                "node_id": "tests/test_cli.py::test_one",
                "disposition": "public_contract",
                "contract_ids": ["missing.contract"],
            },
            {
                "node_id": "tests/test_cli.py::test_one",
                "disposition": "public_contract",
                "contract_ids": ["cli.hiero"],
            },
            {
                "node_id": "tests/test_workspace.py::test_wrong_file",
                "disposition": "public_contract",
                "contract_ids": ["cli.hiero"],
            },
        ],
        "frontend_test_ownership": [],
    }
    path.write_text(json.dumps(payload), encoding="utf-8")

    assert validate_manifest(load_manifest(path), ROOT) == [
        "unknown contract id for test ownership: tests/test_cli.py::test_one: missing.contract",
        "duplicate test ownership node id: tests/test_cli.py::test_one",
        "test ownership contract does not own node file: "
        "tests/test_workspace.py::test_wrong_file: cli.hiero",
    ]


def test_schema_defines_strict_conditional_test_ownership_records() -> None:
    schema = json.loads((ROOT / "compatibility/manifest.schema.json").read_text(encoding="utf-8"))
    ownership = schema["properties"]["test_ownership"]["items"]

    assert "test_ownership" in schema["required"]
    assert ownership["additionalProperties"] is False
    assert ownership["properties"]["disposition"]["enum"] == [
        "public_contract",
        "implementation_internal",
    ]
    assert ownership["allOf"][0]["then"]["required"] == ["contract_ids"]
    assert ownership["allOf"][1]["then"]["required"] == ["reason"]


def test_contract_release_state_is_typed_and_outstanding_requires_no_rust_release(
    tmp_path: Path,
) -> None:
    path = tmp_path / "manifest.json"
    contract = complete_contract(
        last_python_release="0.7.0",
        first_rust_release=None,
        implementation_status="outstanding",
    )
    payload = {
        "manifest_version": 1,
        "python_reference": "0.7.0",
        "contracts": [contract],
        "test_ownership": [],
        "frontend_test_ownership": [],
    }
    path.write_text(json.dumps(payload), encoding="utf-8")

    manifest = load_manifest(path)

    assert manifest.contracts[0].last_python_release == "0.7.0"
    assert manifest.contracts[0].first_rust_release is None
    assert manifest.contracts[0].implementation_status == "outstanding"
    assert validate_manifest(manifest, ROOT) == []


@pytest.mark.parametrize(
    ("implementation_status", "first_rust_release", "expected_error"),
    [
        ("outstanding", "1.0.0", "outstanding contract has first Rust release"),
        ("implemented", None, "implemented contract missing first Rust release"),
    ],
)
def test_release_state_consistency_is_validated(
    tmp_path: Path,
    implementation_status: str,
    first_rust_release: str | None,
    expected_error: str,
) -> None:
    path = tmp_path / "manifest.json"
    contract = complete_contract(
        last_python_release="0.7.0",
        first_rust_release=first_rust_release,
        implementation_status=implementation_status,
    )
    payload = {
        "manifest_version": 1,
        "python_reference": "0.7.0",
        "contracts": [contract],
        "test_ownership": [],
        "frontend_test_ownership": [],
    }
    path.write_text(json.dumps(payload), encoding="utf-8")

    assert validate_manifest(load_manifest(path), ROOT) == [f"{expected_error}: cli.hiero"]


def test_frontend_test_ownership_has_a_strict_separate_node_namespace(tmp_path: Path) -> None:
    path = tmp_path / "manifest.json"
    contract = complete_contract(
        surface="frontend",
        tests=["frontend/src/web/app.test.ts"],
        last_python_release="0.7.0",
        first_rust_release=None,
        implementation_status="outstanding",
    )
    payload = {
        "manifest_version": 1,
        "python_reference": "0.7.0",
        "contracts": [contract],
        "test_ownership": [],
        "frontend_test_ownership": [
            {
                "node_id": "frontend/src/web/app.test.ts::renders the console shell",
                "disposition": "public_contract",
                "contract_ids": ["cli.hiero"],
            }
        ],
    }
    path.write_text(json.dumps(payload), encoding="utf-8")

    manifest = load_manifest(path)

    assert manifest.frontend_test_ownership[0].node_id.endswith("::renders the console shell")
    assert validate_manifest(manifest, ROOT) == []

    payload["frontend_test_ownership"][0]["node_id"] = "tests/test_cli.py::wrong namespace"
    path.write_text(json.dumps(payload), encoding="utf-8")
    with pytest.raises(ValueError, match="frontend Vitest node id"):
        load_manifest(path)


def test_schema_requires_release_state_and_frontend_ownership() -> None:
    schema = json.loads((ROOT / "compatibility/manifest.schema.json").read_text(encoding="utf-8"))
    contract = schema["properties"]["contracts"]["items"]

    assert {
        "last_python_release",
        "first_rust_release",
        "implementation_status",
    } <= set(contract["required"])
    assert contract["properties"]["implementation_status"]["enum"] == [
        "outstanding",
        "implemented",
    ]
    assert "frontend_test_ownership" in schema["required"]
    assert schema["properties"]["frontend_test_ownership"]["items"]["properties"]["node_id"][
        "pattern"
    ].startswith("^frontend/")
