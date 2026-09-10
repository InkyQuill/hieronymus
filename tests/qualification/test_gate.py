"""Aggregate gate truth-table and network-free record checker tests."""

from __future__ import annotations

import contextlib
import io
import json
import sys
from dataclasses import replace
from pathlib import Path
from typing import NamedTuple

import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from factories import (  # noqa: E402
    accepted_failure,
    accepted_records,
    pending_review,
    seed_fingerprint_inputs,
)

from tools.qualification import check  # noqa: E402
from tools.qualification.check import (  # noqa: E402
    compute_gate,
    render_gate,
    serialize_gate,
)
from tools.qualification.model import (  # noqa: E402
    REQUIRED_CRITERIA,
    QualificationRecord,
    Risk,
    load_record,
    serialize_record,
)
from tools.qualification.projections import (  # noqa: E402
    DATABASE_STATE_FIELDS,
    build_projection,
    canonical_projection_bytes,
)
from tools.qualification.render import render_record  # noqa: E402


class CheckResult(NamedTuple):
    """One in-process checker invocation's exit code and captured output."""

    exit_code: int
    stdout: str
    stderr: str


def fail_if_called(*args: object, **kwargs: object) -> object:
    del args, kwargs
    raise AssertionError("records-only checking must not invoke live primitives")


def invoke_check(*arguments: str, cwd: Path | None = None) -> CheckResult:
    monkeypatch = pytest.MonkeyPatch()
    with monkeypatch.context() as patch_context:
        if cwd is not None:
            patch_context.chdir(cwd)
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            exit_code = check.main(list(arguments))
    return CheckResult(exit_code, stdout.getvalue(), stderr.getvalue())


def seed_accepted_records_and_projections(
    repo_root: Path,
) -> dict[Risk, QualificationRecord]:
    for risk in ("mcp-transport", "legacy-database-import"):
        projection = build_projection(repo_root, risk)
        projection_path = repo_root / f"qualification/compatibility/{risk}.json"
        projection_path.parent.mkdir(parents=True, exist_ok=True)
        projection_path.write_bytes(canonical_projection_bytes(projection))
    records = accepted_records(repo_root, semantic="semantic-enabled")
    for risk, record in records.items():
        record_path = repo_root / f"qualification/records/{risk}.json"
        markdown_path = repo_root / f"docs/qualification/rust/{risk}.md"
        record_path.parent.mkdir(parents=True, exist_ok=True)
        markdown_path.parent.mkdir(parents=True, exist_ok=True)
        record_path.write_text(serialize_record(record) + "\n", encoding="utf-8")
        markdown_path.write_text(render_record(record), encoding="utf-8")
    gate = compute_gate(records, repo_root)
    (repo_root / "qualification/records/aggregate.json").write_bytes(serialize_gate(gate))
    (repo_root / "docs/qualification/rust/gate.md").write_text(render_gate(gate), encoding="utf-8")
    return records


def _write_canonical_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(canonical_projection_bytes(value))  # type: ignore[arg-type]


def _read_json(path: Path) -> dict[str, object]:
    return json.loads(path.read_bytes().decode("utf-8"))


def append_unrelated_test_ownership(repo_root: Path) -> None:
    manifest = _read_json(repo_root / "compatibility/manifest.json")
    ownership = manifest["test_ownership"]
    assert isinstance(ownership, list)
    ownership.append({"node_id": "tests/qualification/test_later.py::test_unrelated"})
    _write_canonical_json(repo_root / "compatibility/manifest.json", manifest)


def append_unrelated_state_test_node(repo_root: Path) -> None:
    state = _read_json(repo_root / "compatibility/snapshots/state.json")
    tests = state["tests"]
    assert isinstance(tests, dict)
    node_ids = tests["node_ids"]
    assert isinstance(node_ids, list)
    node_ids.append("tests/qualification/test_later.py::test_unrelated")
    _write_canonical_json(repo_root / "compatibility/snapshots/state.json", state)


def load_record_digests(repo_root: Path) -> dict[Risk, str]:
    return {
        risk: load_record(repo_root / f"qualification/records/{risk}.json").input_digest
        for risk in REQUIRED_CRITERIA
    }


@pytest.fixture
def temp_repository(tmp_path: Path) -> Path:
    for risk in REQUIRED_CRITERIA:
        seed_fingerprint_inputs(tmp_path, risk)
    return tmp_path


def test_all_pass_enables_semantic() -> None:
    gate = compute_gate(accepted_records(ROOT, semantic="semantic-enabled"), ROOT)
    assert gate.status == "qualified"
    assert gate.release_mode == "semantic-enabled"
    assert gate.blocked_plans == ()


def test_semantic_failure_selects_fts_only_and_still_qualifies() -> None:
    gate = compute_gate(accepted_records(ROOT, semantic="fts-only"), ROOT)
    assert gate.status == "qualified"
    assert gate.release_mode == "fts-only"
    assert gate.blocked_plans == ()


@pytest.mark.parametrize(
    ("risk", "blocked_plan"),
    [
        ("mcp-transport", "rust-daemon-mcp-security"),
        ("frontend-embedding", "rust-frontend"),
        ("legacy-database-import", "rust-database-upgrade"),
    ],
)
def test_non_semantic_failure_blocks_without_changing_specs(risk: Risk, blocked_plan: str) -> None:
    records = accepted_records(ROOT, semantic="semantic-enabled")
    records[risk] = accepted_failure(ROOT, risk)
    gate = compute_gate(records, ROOT)
    assert gate.status == "blocked"
    assert "rust-workspace-and-contract-harness" in gate.blocked_plans
    assert blocked_plan in gate.blocked_plans


def test_pending_review_is_blocking() -> None:
    pending = accepted_records(ROOT, semantic="fts-only")
    pending["mcp-transport"] = pending_review(pending["mcp-transport"])
    assert compute_gate(pending, ROOT).status == "blocked"


def test_stale_input_is_blocking() -> None:
    stale = accepted_records(ROOT, semantic="fts-only")
    stale["frontend-embedding"] = replace(stale["frontend-embedding"], input_digest="0" * 64)
    assert compute_gate(stale, ROOT).status == "blocked"


def test_records_only_check_never_invokes_live_runners(
    monkeypatch: pytest.MonkeyPatch,
    temp_repository: Path,
) -> None:
    # Fresh synthetic records test the checker seam, not current product qualification.
    # Real checked-in measurements remain stale after their inputs change.
    seed_accepted_records_and_projections(temp_repository)
    monkeypatch.setattr("socket.create_connection", fail_if_called)
    monkeypatch.setattr("subprocess.run", fail_if_called)
    result = invoke_check("--records-only", cwd=temp_repository)
    assert result.exit_code == 0


def test_unrelated_global_inventory_regeneration_keeps_measured_records_current(
    temp_repository: Path,
) -> None:
    seed_accepted_records_and_projections(temp_repository)
    append_unrelated_test_ownership(temp_repository)
    append_unrelated_state_test_node(temp_repository)
    before = load_record_digests(temp_repository)
    result = invoke_check("--records-only", cwd=temp_repository)
    assert result.exit_code == 0
    assert load_record_digests(temp_repository) == before


@pytest.mark.parametrize(
    ("mutation", "risk"),
    [
        ("mcp-contract", "mcp-transport"),
        ("database-state", "legacy-database-import"),
    ],
)
def test_relevant_projection_mutation_blocks_without_rewriting_anything(
    temp_repository: Path,
    mutation: str,
    risk: Risk,
) -> None:
    seed_accepted_records_and_projections(temp_repository)
    produced = (
        temp_repository / f"qualification/compatibility/{risk}.json",
        temp_repository / f"qualification/records/{risk}.json",
        temp_repository / f"docs/qualification/rust/{risk}.md",
        temp_repository / "qualification/records/aggregate.json",
        temp_repository / "docs/qualification/rust/gate.md",
    )
    before = {path: path.read_bytes() for path in produced}
    if mutation == "mcp-contract":
        manifest = _read_json(temp_repository / "compatibility/manifest.json")
        contracts = manifest["contracts"]
        assert isinstance(contracts, list)
        selected = next(
            item
            for item in contracts
            if isinstance(item, dict) and item.get("id") == "http.route.post.mcp"
        )
        tests = selected["tests"]
        assert isinstance(tests, list)
        tests.append("tests/relevant-change.py")
        _write_canonical_json(temp_repository / "compatibility/manifest.json", manifest)
    else:
        state = _read_json(temp_repository / "compatibility/snapshots/state.json")
        database = state["database"]
        assert isinstance(database, dict)
        database[DATABASE_STATE_FIELDS[0]] = ["changed"]
        _write_canonical_json(temp_repository / "compatibility/snapshots/state.json", state)

    result = invoke_check("--records-only", cwd=temp_repository)

    assert result.exit_code == 1
    assert "projection" in result.stdout + result.stderr
    assert {path: path.read_bytes() for path in produced} == before
