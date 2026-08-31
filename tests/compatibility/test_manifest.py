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
