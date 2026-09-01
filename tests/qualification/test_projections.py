"""Closed compatibility projection tests for immutable qualification evidence."""

from __future__ import annotations

import json
import os
import socket
import sqlite3
import subprocess
import urllib.request
from pathlib import Path

import pytest
from factories import make_record, seed_fingerprint_inputs

from tools.qualification.fingerprint import fingerprint_inputs
from tools.qualification.projections import (
    DATABASE_CONTRACT_IDS,
    DATABASE_STATE_FIELDS,
    MCP_EXACT_CONTRACT_IDS,
    build_projection,
    canonical_projection_bytes,
    main,
    projection_issues,
)
from tools.qualification.validate import validate_record

_MCP_TOOL_NAMES = (
    "hieronymus_concept_archive",
    "hieronymus_concept_create",
    "hieronymus_concept_facet_add",
    "hieronymus_concept_facet_list",
    "hieronymus_concept_facet_set_canonical",
    "hieronymus_concept_facet_update",
    "hieronymus_concept_get",
    "hieronymus_concept_list",
    "hieronymus_concept_merge",
    "hieronymus_concept_proposals_list",
    "hieronymus_concept_rename",
    "hieronymus_concept_semantic_tags_set",
    "hieronymus_concept_update",
    "hieronymus_crystal_link_concept",
    "hieronymus_crystal_semantic_tags_set",
    "hieronymus_crystal_story_scopes_set",
    "hieronymus_dream",
    "hieronymus_feedback",
    "hieronymus_memory_add",
    "hieronymus_memory_search",
    "hieronymus_rag_import",
    "hieronymus_rag_search",
    "hieronymus_recall",
    "hieronymus_rule_crystal_archive",
    "hieronymus_rule_crystal_validate",
    "hieronymus_rule_crystals_list",
    "hieronymus_series_create",
    "hieronymus_series_init",
    "hieronymus_series_list",
    "hieronymus_series_set_language_tags",
    "hieronymus_session_complete",
    "hieronymus_session_start",
    "hieronymus_short_term_add",
    "hieronymus_short_term_add_batch",
    "hieronymus_status",
    "hieronymus_termbase_approve",
    "hieronymus_termbase_contract",
    "hieronymus_termbase_propose",
    "hieronymus_termbase_validate",
)
_EXPECTED_MCP_IDS = tuple(
    sorted((*MCP_EXACT_CONTRACT_IDS, *(f"mcp.tool.{name}" for name in _MCP_TOOL_NAMES)))
)
_CONTRACT_FIELDS = (
    "acceptance_owner",
    "adr",
    "disposition",
    "first_rust_release",
    "fixture",
    "id",
    "implementation_status",
    "last_python_release",
    "python_entry_point",
    "rust_test_target",
    "surface",
    "technical_owner",
    "tests",
)


def read_json(path: Path) -> dict[str, object]:
    return json.loads(path.read_bytes().decode("utf-8"))


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(canonical_projection_bytes(value))


def _contract(contract_id: str) -> dict[str, object]:
    return {
        "acceptance_owner": "Pavel Obruchnikov <me@inkyquill.net>",
        "adr": "docs/adr/fixture.md",
        "disposition": "preserve",
        "first_rust_release": None,
        "fixture": f"compatibility/fixtures/{contract_id}.json",
        "id": contract_id,
        "implementation_status": "outstanding",
        "last_python_release": "0.7.0",
        "python_entry_point": "fixture.module:entrypoint",
        "rust_test_target": "crates/fixture/tests/contract.rs::contract",
        "surface": "mcp" if contract_id.startswith("mcp.") else "database",
        "technical_owner": "qualification",
        "tests": ["tests/first.py", "tests/second.py"],
    }


def seed_projection_sources_and_checked_in_files(root: Path) -> None:
    contracts = [_contract(contract_id) for contract_id in _EXPECTED_MCP_IDS]
    contracts.extend(_contract(contract_id) for contract_id in DATABASE_CONTRACT_IDS)
    contracts.append(_contract("unrelated.contract"))
    write_json(
        root / "compatibility/manifest.json",
        {
            "contracts": contracts,
            "frontend_test_ownership": [{"node_id": "frontend/unrelated"}],
            "manifest_version": 1,
            "test_ownership": [{"node_id": "tests/unrelated.py::test_one"}],
        },
    )
    database = {field: [field, {"ordered": ["first", "second"]}] for field in DATABASE_STATE_FIELDS}
    database["unlisted"] = "ignored"
    write_json(
        root / "compatibility/snapshots/state.json",
        {
            "config": {"unrelated": True},
            "database": database,
            "tests": {"node_ids": ["tests/unrelated.py::test_one"]},
            "unrelated": {"surface": True},
        },
    )
    for risk in ("mcp-transport", "legacy-database-import"):
        projection = build_projection(root, risk)
        path = root / f"qualification/compatibility/{risk}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(canonical_projection_bytes(projection))


def test_projection_builder_selects_exact_closed_lossless_subtrees(tmp_path: Path) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)

    mcp = build_projection(tmp_path, "mcp-transport")
    database = build_projection(tmp_path, "legacy-database-import")

    assert tuple(mcp) == ("projection_version", "risk", "contracts")
    assert tuple(item["id"] for item in mcp["contracts"]) == _EXPECTED_MCP_IDS
    assert len(mcp["contracts"]) == 42
    assert tuple(database) == ("projection_version", "risk", "contracts", "database")
    assert tuple(item["id"] for item in database["contracts"]) == tuple(
        sorted(DATABASE_CONTRACT_IDS)
    )
    assert tuple(database["database"]) == DATABASE_STATE_FIELDS
    assert database["database"][DATABASE_STATE_FIELDS[0]][1]["ordered"] == [
        "first",
        "second",
    ]
    assert "unlisted" not in database["database"]


def test_projection_canonical_bytes_are_exact_and_reject_nonfinite_values() -> None:
    projection = {"z": ["second", "first"], "a": {"unicode": "Ж"}}

    assert canonical_projection_bytes(projection) == (
        '{"a":{"unicode":"Ж"},"z":["second","first"]}\n'.encode()
    )
    with pytest.raises(ValueError):
        canonical_projection_bytes({"value": float("nan")})


def test_unrelated_inventory_changes_do_not_stale_projections(tmp_path: Path) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)
    manifest = read_json(tmp_path / "compatibility/manifest.json")
    manifest["test_ownership"].append({"node_id": "tests/later.py::test_unrelated"})
    manifest["frontend_test_ownership"].append({"node_id": "frontend/later"})
    manifest["contracts"][-1]["disposition"] = "remove"
    write_json(tmp_path / "compatibility/manifest.json", manifest)
    state = read_json(tmp_path / "compatibility/snapshots/state.json")
    state["tests"]["node_ids"].append("tests/later.py::test_unrelated")
    state["config"]["later"] = True
    state["database"]["unlisted"] = "changed but irrelevant"
    state["unrelated"]["surface"] = False
    write_json(tmp_path / "compatibility/snapshots/state.json", state)

    assert projection_issues(tmp_path) == {
        "mcp-transport": (),
        "legacy-database-import": (),
    }


@pytest.mark.parametrize("risk", ["mcp-transport", "legacy-database-import"])
@pytest.mark.parametrize("field", _CONTRACT_FIELDS)
def test_every_selected_contract_field_change_is_projection_drift(
    tmp_path: Path,
    risk: str,
    field: str,
) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)
    manifest = read_json(tmp_path / "compatibility/manifest.json")
    selected_id = "http.route.post.mcp" if risk == "mcp-transport" else "database.schema.current"
    selected = next(item for item in manifest["contracts"] if item["id"] == selected_id)
    if field == "id":
        selected[field] = f"changed.{selected_id}"
    elif field == "tests":
        selected[field].reverse()
    else:
        selected[field] = "changed"
    write_json(tmp_path / "compatibility/manifest.json", manifest)

    issues = projection_issues(tmp_path)

    assert issues[risk]
    other = "legacy-database-import" if risk == "mcp-transport" else "mcp-transport"
    assert issues[other] == ()


@pytest.mark.parametrize("field", DATABASE_STATE_FIELDS)
def test_every_projected_database_state_field_change_is_drift(
    tmp_path: Path,
    field: str,
) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)
    state = read_json(tmp_path / "compatibility/snapshots/state.json")
    state["database"][field] = ["changed"]
    write_json(tmp_path / "compatibility/snapshots/state.json", state)

    assert projection_issues(tmp_path)["mcp-transport"] == ()
    assert projection_issues(tmp_path)["legacy-database-import"]


@pytest.mark.parametrize("mutation", ["add", "remove"])
def test_mcp_projection_rejects_tool_inventory_addition_or_removal(
    tmp_path: Path,
    mutation: str,
) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)
    manifest = read_json(tmp_path / "compatibility/manifest.json")
    if mutation == "add":
        manifest["contracts"].append(_contract("mcp.tool.unexpected"))
    else:
        manifest["contracts"] = [
            item for item in manifest["contracts"] if item["id"] != "mcp.tool.hieronymus_status"
        ]
    write_json(tmp_path / "compatibility/manifest.json", manifest)

    assert projection_issues(tmp_path)["mcp-transport"]
    assert projection_issues(tmp_path)["legacy-database-import"] == ()


@pytest.mark.parametrize("risk", ["mcp-transport", "legacy-database-import"])
@pytest.mark.parametrize("mutation", ["invalid-json", "extra-field", "array-reorder"])
def test_checked_projection_drift_is_blocking_and_deterministic(
    tmp_path: Path,
    risk: str,
    mutation: str,
) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)
    projection_path = tmp_path / f"qualification/compatibility/{risk}.json"
    if mutation == "invalid-json":
        projection_path.write_bytes(b"{invalid\n")
    else:
        projection = read_json(projection_path)
        if mutation == "extra-field":
            projection["extra"] = True
        else:
            projection["contracts"][0]["tests"].reverse()
        write_json(projection_path, projection)

    first = projection_issues(tmp_path)

    assert first == projection_issues(tmp_path)
    assert first[risk]
    assert all(str(tmp_path) not in issue for issue in first[risk])


def test_projection_checker_is_read_only_and_has_no_active_boundary_calls(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)
    tracked = tuple(
        tmp_path / relative
        for relative in (
            "compatibility/manifest.json",
            "compatibility/snapshots/state.json",
            "qualification/compatibility/mcp-transport.json",
            "qualification/compatibility/legacy-database-import.json",
        )
    )
    before = {path: path.read_bytes() for path in tracked}

    def forbidden(*args: object, **kwargs: object) -> object:
        del args, kwargs
        raise AssertionError("projection checking crossed an active boundary")

    monkeypatch.setattr(os, "replace", forbidden)
    monkeypatch.setattr(subprocess, "run", forbidden)
    monkeypatch.setattr(socket, "create_connection", forbidden)
    monkeypatch.setattr(urllib.request, "urlopen", forbidden)
    monkeypatch.setattr(sqlite3, "connect", forbidden)

    assert projection_issues(tmp_path) == {
        "mcp-transport": (),
        "legacy-database-import": (),
    }
    assert {path: path.read_bytes() for path in tracked} == before


def test_projection_cli_check_is_read_only_and_write_is_idempotent(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)
    projection_path = tmp_path / "qualification/compatibility/mcp-transport.json"
    projection_path.write_bytes(b"stale\n")
    monkeypatch.chdir(tmp_path)

    assert main(["--check"]) == 1
    assert projection_path.read_bytes() == b"stale\n"
    assert "compatibility projections are stale" in capsys.readouterr().err

    assert main(["--write"]) == 0
    first = {
        risk: (tmp_path / f"qualification/compatibility/{risk}.json").read_bytes()
        for risk in ("mcp-transport", "legacy-database-import")
    }
    assert main(["--write"]) == 0
    assert {
        risk: (tmp_path / f"qualification/compatibility/{risk}.json").read_bytes()
        for risk in ("mcp-transport", "legacy-database-import")
    } == first


def test_projection_write_validates_both_documents_before_replacement(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)
    paths = tuple(
        tmp_path / f"qualification/compatibility/{risk}.json"
        for risk in ("mcp-transport", "legacy-database-import")
    )
    before = {path: path.read_bytes() for path in paths}
    state = read_json(tmp_path / "compatibility/snapshots/state.json")
    del state["database"][DATABASE_STATE_FIELDS[0]]
    write_json(tmp_path / "compatibility/snapshots/state.json", state)
    monkeypatch.chdir(tmp_path)

    assert main(["--write"]) == 2
    assert {path: path.read_bytes() for path in paths} == before


@pytest.mark.parametrize("risk", ["mcp-transport", "legacy-database-import"])
def test_unrelated_later_inventory_keeps_measured_record_digest_current(
    tmp_path: Path,
    risk: str,
) -> None:
    seed_fingerprint_inputs(tmp_path, risk)
    record = make_record(tmp_path, risk)
    digest = record.input_digest
    manifest = read_json(tmp_path / "compatibility/manifest.json")
    manifest["test_ownership"].append({"node_id": "tests/task_19.py::test_later"})
    write_json(tmp_path / "compatibility/manifest.json", manifest)
    state = read_json(tmp_path / "compatibility/snapshots/state.json")
    state["tests"]["node_ids"].append("tests/task_19.py::test_later")
    write_json(tmp_path / "compatibility/snapshots/state.json", state)

    assert fingerprint_inputs(tmp_path, record.input_paths) == digest
    assert validate_record(record, tmp_path) == []


@pytest.mark.parametrize("risk", ["mcp-transport", "legacy-database-import"])
def test_relevant_projection_source_change_blocks_unchanged_record(
    tmp_path: Path,
    risk: str,
) -> None:
    seed_fingerprint_inputs(tmp_path, risk)
    record = make_record(tmp_path, risk)
    if risk == "mcp-transport":
        manifest = read_json(tmp_path / "compatibility/manifest.json")
        selected = next(
            item for item in manifest["contracts"] if item["id"] == "http.route.post.mcp"
        )
        selected["tests"].append("tests/relevant-change.py")
        write_json(tmp_path / "compatibility/manifest.json", manifest)
    else:
        state = read_json(tmp_path / "compatibility/snapshots/state.json")
        state["database"][DATABASE_STATE_FIELDS[0]] = ["changed"]
        write_json(tmp_path / "compatibility/snapshots/state.json", state)

    assert fingerprint_inputs(tmp_path, record.input_paths) == record.input_digest
    issues = validate_record(record, tmp_path)
    assert any("compatibility projection" in issue for issue in issues)
    assert all(str(tmp_path) not in issue for issue in issues)
