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
def test_blank_owners_and_missing_adr_for_changed_contract_are_rejected(
    tmp_path: Path, disposition: str
) -> None:
    path = tmp_path / "manifest.json"
    write_manifest(
        path,
        [
            complete_contract(
                acceptance_owner=" ",
                technical_owner=" ",
                disposition=disposition,
            )
        ],
    )

    assert validate_manifest(load_manifest(path), ROOT) == [
        "blank acceptance owner: cli.hiero",
        "blank technical owner: cli.hiero",
        f"missing adr for {disposition} contract: cli.hiero",
    ]


def test_loader_rejects_missing_required_contract_field(tmp_path: Path) -> None:
    path = tmp_path / "manifest.json"
    contract = complete_contract()
    del contract["fixture"]
    write_manifest(path, [contract])

    with pytest.raises(ValueError, match="fixture"):
        load_manifest(path)


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
