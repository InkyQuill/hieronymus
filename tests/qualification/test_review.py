"""Review-only transition and atomic artifact regeneration tests."""

# ruff: noqa: E501 -- canonical review commands are deliberately byte-exact literals.

from __future__ import annotations

import contextlib
import io
import json
import os
import shlex
import sys
from collections.abc import Callable, Sequence
from dataclasses import asdict
from pathlib import Path
from typing import Literal, NamedTuple, cast

import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from factories import make_record, seed_fingerprint_inputs  # noqa: E402

from tools.qualification import check, review  # noqa: E402
from tools.qualification.check import (  # noqa: E402
    AGGREGATE_PATH,
    GATE_MARKDOWN_PATH,
    MARKDOWN_PATHS,
    RECORD_PATHS,
    compute_gate,
    render_gate,
    serialize_gate,
)
from tools.qualification.model import (  # noqa: E402
    REQUIRED_CRITERIA,
    QualificationRecord,
    Review,
    Risk,
    load_record,
    serialize_record,
)
from tools.qualification.projections import (  # noqa: E402
    DATABASE_STATE_FIELDS,
    canonical_projection_bytes,
)
from tools.qualification.render import render_record  # noqa: E402
from tools.qualification.validate import replay_commands_are_safe  # noqa: E402

_OWNER = "Pavel Obruchnikov <me@inkyquill.net>"

# The eight exact Step 3 commands: four rejected then four accepted.
REJECTED_REVIEW_COMMANDS = (
    'uv run python -m tools.qualification.review mcp-transport --status rejected --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed false --normative-constraints-preserved false',
    'uv run python -m tools.qualification.review semantic-native --status rejected --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed false --normative-constraints-preserved false',
    'uv run python -m tools.qualification.review frontend-embedding --status rejected --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed false --normative-constraints-preserved false',
    'uv run python -m tools.qualification.review legacy-database-import --status rejected --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed false --normative-constraints-preserved false',
)
ACCEPTED_REVIEW_COMMANDS = (
    'uv run python -m tools.qualification.review mcp-transport --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true',
    'uv run python -m tools.qualification.review semantic-native --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true',
    'uv run python -m tools.qualification.review frontend-embedding --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true',
    'uv run python -m tools.qualification.review legacy-database-import --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true',
)
EIGHT_EXACT_REVIEW_COMMANDS = REJECTED_REVIEW_COMMANDS + ACCEPTED_REVIEW_COMMANDS

UNSAFE_REVIEW_COMMANDS = (
    # unquoted owner
    "uv run python -m tools.qualification.review mcp-transport --status accepted --owner Pavel Obruchnikov <me@inkyquill.net> --objective-evidence-reviewed true --normative-constraints-preserved true",
    # changed owner
    'uv run python -m tools.qualification.review mcp-transport --status accepted --owner "Someone Else <someone@example.net>" --objective-evidence-reviewed true --normative-constraints-preserved true',
    # reordered flags
    'uv run python -m tools.qualification.review --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" mcp-transport --objective-evidence-reviewed true --normative-constraints-preserved true',
    # accepted with a false assertion
    'uv run python -m tools.qualification.review mcp-transport --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved false',
    # rejected while claiming both assertions true
    'uv run python -m tools.qualification.review mcp-transport --status rejected --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true',
    # pending is outside the review contract
    'uv run python -m tools.qualification.review mcp-transport --status pending --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed false --normative-constraints-preserved false',
)


class ReviewResult(NamedTuple):
    """One in-process review CLI invocation's exit code and captured output."""

    exit_code: int
    stdout: str
    stderr: str


class CheckResult(NamedTuple):
    """One in-process checker invocation's exit code and captured output."""

    exit_code: int
    stdout: str
    stderr: str


def fail_if_called(*args: object, **kwargs: object) -> object:
    del args, kwargs
    raise AssertionError("review regeneration must not invoke live primitives")


def invoke_review(
    arguments: Sequence[str],
    cwd: Path,
    *,
    replace_policy: Callable[[Path, Path], None] | None = None,
) -> tuple[ReviewResult, list[tuple[Path, Path]]]:
    """Run the review CLI in-process, tracking every os.replace call."""
    calls: list[tuple[Path, Path]] = []
    monkeypatch = pytest.MonkeyPatch()
    with monkeypatch.context() as patch_context:
        patch_context.chdir(cwd)
        patch_context.setattr("socket.create_connection", fail_if_called)
        patch_context.setattr("subprocess.run", fail_if_called)
        real_replace = os.replace

        def tracked_replace(
            source: object,
            destination: object,
            /,
            *args: object,
            **kwargs: object,
        ) -> object:
            src, dst = Path(str(source)), Path(str(destination))
            if replace_policy is not None:
                replace_policy(src, dst)
            calls.append((src, dst))
            return real_replace(source, destination)

        patch_context.setattr(os, "replace", tracked_replace)
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            exit_code = review.main(list(arguments))
    return ReviewResult(exit_code, stdout.getvalue(), stderr.getvalue()), calls


def invoke_check(*arguments: str, cwd: Path | None = None) -> CheckResult:
    monkeypatch = pytest.MonkeyPatch()
    with monkeypatch.context() as patch_context:
        if cwd is not None:
            patch_context.chdir(cwd)
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            exit_code = check.main(list(arguments))
    return CheckResult(exit_code, stdout.getvalue(), stderr.getvalue())


def review_destinations(repo_root: Path, risk: Risk) -> tuple[Path, Path, Path, Path]:
    """Return the four files one review CLI run may replace."""
    return (
        repo_root / RECORD_PATHS[risk],
        repo_root / MARKDOWN_PATHS[risk],
        repo_root / AGGREGATE_PATH,
        repo_root / GATE_MARKDOWN_PATH,
    )


def snapshot_bytes(repo_root: Path) -> dict[Path, bytes]:
    return {path: path.read_bytes() for path in sorted(repo_root.rglob("*")) if path.is_file()}


def seed_pending_records(repo_root: Path) -> dict[Risk, QualificationRecord]:
    """Seed four pending measured records, projections, and a blocked aggregate."""
    records = {
        risk: make_record(repo_root, risk, review_status="pending") for risk in REQUIRED_CRITERIA
    }
    for risk, record in records.items():
        record_path = repo_root / RECORD_PATHS[risk]
        markdown_path = repo_root / MARKDOWN_PATHS[risk]
        record_path.parent.mkdir(parents=True, exist_ok=True)
        markdown_path.parent.mkdir(parents=True, exist_ok=True)
        record_path.write_text(serialize_record(record) + "\n", encoding="utf-8")
        markdown_path.write_text(render_record(record), encoding="utf-8")
    gate = compute_gate(records, repo_root)
    (repo_root / AGGREGATE_PATH).parent.mkdir(parents=True, exist_ok=True)
    (repo_root / GATE_MARKDOWN_PATH).parent.mkdir(parents=True, exist_ok=True)
    (repo_root / AGGREGATE_PATH).write_bytes(serialize_gate(gate))
    (repo_root / GATE_MARKDOWN_PATH).write_text(render_gate(gate), encoding="utf-8")
    return records


def write_canonical_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(canonical_projection_bytes(value))


def read_json(path: Path) -> dict[str, object]:
    return json.loads(path.read_bytes().decode("utf-8"))


def append_unrelated_test_ownership(repo_root: Path) -> None:
    manifest = read_json(repo_root / "compatibility/manifest.json")
    ownership = manifest["test_ownership"]
    assert isinstance(ownership, list)
    ownership.append({"node_id": "tests/qualification/test_later.py::test_unrelated"})
    write_canonical_json(repo_root / "compatibility/manifest.json", manifest)


def append_unrelated_state_test_node(repo_root: Path) -> None:
    state = read_json(repo_root / "compatibility/snapshots/state.json")
    tests = state["tests"]
    assert isinstance(tests, dict)
    node_ids = tests["node_ids"]
    assert isinstance(node_ids, list)
    node_ids.append("tests/qualification/test_later.py::test_unrelated")
    write_canonical_json(repo_root / "compatibility/snapshots/state.json", state)


@pytest.fixture
def temp_repository(tmp_path: Path) -> Path:
    for risk in REQUIRED_CRITERIA:
        seed_fingerprint_inputs(tmp_path, risk)
    seed_pending_records(tmp_path)
    return tmp_path


def test_accepted_review_requires_exact_owner_and_both_assertions() -> None:
    record = make_record(ROOT, "mcp-transport")
    with pytest.raises(ValueError, match="review owner"):
        review.review_record(
            record,
            owner="Someone Else <someone@example.net>",
            status="accepted",
            objective_evidence_reviewed=True,
            normative_constraints_preserved=True,
        )
    with pytest.raises(ValueError, match="both review assertions"):
        review.review_record(
            record,
            owner=_OWNER,
            status="accepted",
            objective_evidence_reviewed=False,
            normative_constraints_preserved=True,
        )
    with pytest.raises(ValueError, match="both review assertions"):
        review.review_record(
            record,
            owner=_OWNER,
            status="accepted",
            objective_evidence_reviewed=True,
            normative_constraints_preserved=False,
        )
    reviewed = review.review_record(
        record,
        owner=_OWNER,
        status="accepted",
        objective_evidence_reviewed=True,
        normative_constraints_preserved=True,
    )
    assert reviewed.review == Review(
        owner=_OWNER,
        status="accepted",
        objective_evidence_reviewed=True,
        normative_constraints_preserved=True,
    )


@pytest.mark.parametrize(
    ("objective", "normative"),
    [(False, False), (True, False), (False, True)],
)
def test_rejected_review_preserves_false_assertions(
    objective: bool,
    normative: bool,
) -> None:
    record = make_record(ROOT, "legacy-database-import")
    reviewed = review.review_record(
        record,
        owner=_OWNER,
        status="rejected",
        objective_evidence_reviewed=objective,
        normative_constraints_preserved=normative,
    )
    assert reviewed.review == Review(
        owner=_OWNER,
        status="rejected",
        objective_evidence_reviewed=objective,
        normative_constraints_preserved=normative,
    )


def test_rejected_review_with_both_assertions_true_is_refused() -> None:
    record = make_record(ROOT, "frontend-embedding")
    with pytest.raises(ValueError, match="at least one review assertion"):
        review.review_record(
            record,
            owner=_OWNER,
            status="rejected",
            objective_evidence_reviewed=True,
            normative_constraints_preserved=True,
        )


def test_review_record_refuses_pending_and_divergent_decisions() -> None:
    record = make_record(ROOT, "semantic-native")
    with pytest.raises(ValueError, match="accepted or rejected"):
        review.review_record(
            record,
            owner=_OWNER,
            status="pending",
            objective_evidence_reviewed=False,
            normative_constraints_preserved=False,
        )
    accepted = review.review_record(
        record,
        owner=_OWNER,
        status="accepted",
        objective_evidence_reviewed=True,
        normative_constraints_preserved=True,
    )
    with pytest.raises(ValueError, match="already recorded"):
        review.review_record(
            accepted,
            owner=_OWNER,
            status="rejected",
            objective_evidence_reviewed=False,
            normative_constraints_preserved=False,
        )
    converged = review.review_record(
        accepted,
        owner=_OWNER,
        status="accepted",
        objective_evidence_reviewed=True,
        normative_constraints_preserved=True,
    )
    assert converged == accepted


def test_review_record_refuses_malformed_records() -> None:
    for malformed in (None, object(), "mcp-transport"):
        with pytest.raises(ValueError, match="qualification record is invalid"):
            review.review_record(
                cast(QualificationRecord, malformed),
                owner=_OWNER,
                status="accepted",
                objective_evidence_reviewed=True,
                normative_constraints_preserved=True,
            )


@pytest.mark.parametrize("risk", tuple(REQUIRED_CRITERIA))
@pytest.mark.parametrize("status", ("accepted", "rejected"))
def test_review_record_changes_only_the_review_block(risk: Risk, status: str) -> None:
    assertions = status == "accepted"
    record = make_record(ROOT, risk)
    before = asdict(record)
    del before["review"]

    reviewed = review.review_record(
        record,
        owner=_OWNER,
        status=cast(Literal["accepted", "rejected"], status),
        objective_evidence_reviewed=assertions,
        normative_constraints_preserved=assertions,
    )

    after = asdict(reviewed)
    del after["review"]
    assert after == before
    assert reviewed.review.status == status
    assert reviewed.review.objective_evidence_reviewed is assertions
    assert reviewed.review.normative_constraints_preserved is assertions


@pytest.mark.parametrize("command", EIGHT_EXACT_REVIEW_COMMANDS)
def test_exact_review_commands_are_replay_safe(command: str) -> None:
    assert replay_commands_are_safe((command,))


@pytest.mark.parametrize("command", EIGHT_EXACT_REVIEW_COMMANDS)
def test_exact_review_commands_parse_owner_as_single_exact_token(command: str) -> None:
    tokens = shlex.split(command, posix=True)
    assert tokens[tokens.index("--owner") + 1] == _OWNER


@pytest.mark.parametrize("command", UNSAFE_REVIEW_COMMANDS)
def test_unsafe_review_command_variants_are_rejected(command: str) -> None:
    assert not replay_commands_are_safe((command,))


def test_empty_and_duplicate_command_tuples_are_unsafe() -> None:
    command = ACCEPTED_REVIEW_COMMANDS[0]
    assert not replay_commands_are_safe(())
    assert not replay_commands_are_safe((command, command))


def test_cli_replaces_exactly_four_files_atomically(temp_repository: Path) -> None:
    risk: Risk = "mcp-transport"
    destinations = review_destinations(temp_repository, risk)
    before = snapshot_bytes(temp_repository)
    record = load_record(temp_repository / RECORD_PATHS[risk])
    expected_reviewed = review.review_record(
        record,
        owner=_OWNER,
        status="accepted",
        objective_evidence_reviewed=True,
        normative_constraints_preserved=True,
    )
    updated: dict[Risk, QualificationRecord] = {}
    for other in REQUIRED_CRITERIA:
        updated[other] = (
            expected_reviewed
            if other == risk
            else load_record(temp_repository / RECORD_PATHS[other])
        )
    gate = compute_gate(updated, temp_repository)

    result, calls = invoke_review(
        (
            "mcp-transport",
            "--status",
            "accepted",
            "--owner",
            _OWNER,
            "--objective-evidence-reviewed",
            "true",
            "--normative-constraints-preserved",
            "true",
        ),
        temp_repository,
    )

    assert result.exit_code == 0
    after = snapshot_bytes(temp_repository)
    changed = {path for path in after if before[path] != after[path]}
    assert changed == set(destinations)
    assert {destination for _source, destination in calls} == set(destinations)
    assert all(
        source.parent == destination.parent and source.name != destination.name
        for source, destination in calls
    )
    for destination in destinations:
        assert not list(destination.parent.glob(f".{destination.name}.*"))
    assert after[destinations[0]] == (serialize_record(expected_reviewed) + "\n").encode("utf-8")
    assert after[destinations[1]] == render_record(expected_reviewed).encode("utf-8")
    assert after[destinations[2]] == serialize_gate(gate)
    assert after[destinations[3]] == render_gate(gate).encode("utf-8")
    for other in REQUIRED_CRITERIA:
        if other != risk:
            path = temp_repository / RECORD_PATHS[other]
            assert after[path] == before[path]
    consistency = invoke_check("--records-only", cwd=temp_repository)
    assert consistency.exit_code == 0


def test_cli_rerun_converges_to_identical_bytes(temp_repository: Path) -> None:
    arguments = (
        "mcp-transport",
        "--status",
        "accepted",
        "--owner",
        _OWNER,
        "--objective-evidence-reviewed",
        "true",
        "--normative-constraints-preserved",
        "true",
    )
    first, _calls = invoke_review(arguments, temp_repository)
    assert first.exit_code == 0
    after_first = snapshot_bytes(temp_repository)
    second, _calls = invoke_review(arguments, temp_repository)
    assert second.exit_code == 0
    assert snapshot_bytes(temp_repository) == after_first


def test_cli_crash_between_replacements_leaves_drift_then_converges(
    temp_repository: Path,
    tmp_path_factory: pytest.TempPathFactory,
) -> None:
    arguments = (
        "mcp-transport",
        "--status",
        "accepted",
        "--owner",
        _OWNER,
        "--objective-evidence-reviewed",
        "true",
        "--normative-constraints-preserved",
        "true",
    )
    state = {"replacements": 0}

    def crash_after_first(source: Path, destination: Path) -> None:
        del source, destination
        state["replacements"] += 1
        if state["replacements"] > 1:
            raise OSError("simulated crash between replacements")

    crashed, calls = invoke_review(
        arguments,
        temp_repository,
        replace_policy=crash_after_first,
    )
    assert crashed.exit_code == 2
    assert [destination for _source, destination in calls] == [
        temp_repository / RECORD_PATHS["mcp-transport"]
    ]
    drifted = invoke_check("--records-only", cwd=temp_repository)
    assert drifted.exit_code == 1

    reference = tmp_path_factory.mktemp("review-reference")
    for risk in REQUIRED_CRITERIA:
        seed_fingerprint_inputs(reference, risk)
    seed_pending_records(reference)
    clean, _calls = invoke_review(arguments, reference)
    assert clean.exit_code == 0

    recovery, _calls = invoke_review(arguments, temp_repository)
    assert recovery.exit_code == 0
    for relative in (
        RECORD_PATHS["mcp-transport"],
        MARKDOWN_PATHS["mcp-transport"],
        AGGREGATE_PATH,
        GATE_MARKDOWN_PATH,
    ):
        assert (temp_repository / relative).read_bytes() == (reference / relative).read_bytes()
    consistency = invoke_check("--records-only", cwd=temp_repository)
    assert consistency.exit_code == 0


def test_rejected_cli_review_keeps_aggregate_blocked(temp_repository: Path) -> None:
    result, _calls = invoke_review(
        (
            "mcp-transport",
            "--status",
            "rejected",
            "--owner",
            _OWNER,
            "--objective-evidence-reviewed",
            "false",
            "--normative-constraints-preserved",
            "false",
        ),
        temp_repository,
    )
    assert result.exit_code == 0
    record = load_record(temp_repository / RECORD_PATHS["mcp-transport"])
    assert record.review.status == "rejected"
    assert record.review.objective_evidence_reviewed is False
    assert record.review.normative_constraints_preserved is False
    gate = read_json(temp_repository / AGGREGATE_PATH)
    assert gate["status"] == "blocked"
    assert gate["release_mode"] is None
    assert set(cast(list[str], gate["issues"])) == {
        "mcp-transport: review is not accepted",
        "semantic-native: review is not accepted",
        "frontend-embedding: review is not accepted",
        "legacy-database-import: review is not accepted",
    }


def test_partially_reviewed_rejection_keeps_aggregate_blocked(
    temp_repository: Path,
) -> None:
    result, _calls = invoke_review(
        (
            "semantic-native",
            "--status",
            "rejected",
            "--owner",
            _OWNER,
            "--objective-evidence-reviewed",
            "true",
            "--normative-constraints-preserved",
            "false",
        ),
        temp_repository,
    )
    assert result.exit_code == 0
    record = load_record(temp_repository / RECORD_PATHS["semantic-native"])
    assert record.review.status == "rejected"
    assert record.review.objective_evidence_reviewed is True
    assert record.review.normative_constraints_preserved is False
    gate = read_json(temp_repository / AGGREGATE_PATH)
    assert gate["status"] == "blocked"
    assert "semantic-native: review is not accepted" in cast(list[str], gate["issues"])


def test_unrelated_inventory_regeneration_tolerated(temp_repository: Path) -> None:
    append_unrelated_test_ownership(temp_repository)
    append_unrelated_state_test_node(temp_repository)
    projection_risks = ("mcp-transport", "legacy-database-import")
    digests_before = {
        risk: load_record(temp_repository / RECORD_PATHS[risk]).input_digest
        for risk in projection_risks
    }
    for risk in projection_risks:
        result, _calls = invoke_review(
            (
                risk,
                "--status",
                "accepted",
                "--owner",
                _OWNER,
                "--objective-evidence-reviewed",
                "true",
                "--normative-constraints-preserved",
                "true",
            ),
            temp_repository,
        )
        assert result.exit_code == 0, result.stderr
    digests_after = {
        risk: load_record(temp_repository / RECORD_PATHS[risk]).input_digest
        for risk in projection_risks
    }
    assert digests_after == digests_before
    gate = read_json(temp_repository / AGGREGATE_PATH)
    record_digests = cast(dict[str, str], gate["record_digests"])
    assert {risk: record_digests[risk] for risk in projection_risks} == digests_before


@pytest.mark.parametrize(
    ("mutation", "risk"),
    [
        ("mcp-contract", "mcp-transport"),
        ("database-state", "legacy-database-import"),
    ],
)
def test_projection_drift_refuses_before_any_replacement(
    temp_repository: Path,
    mutation: str,
    risk: Risk,
) -> None:
    destinations = review_destinations(temp_repository, risk)
    before = {path: path.read_bytes() for path in destinations}
    if mutation == "mcp-contract":
        manifest = read_json(temp_repository / "compatibility/manifest.json")
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
        write_canonical_json(temp_repository / "compatibility/manifest.json", manifest)
    else:
        state = read_json(temp_repository / "compatibility/snapshots/state.json")
        database = state["database"]
        assert isinstance(database, dict)
        database[DATABASE_STATE_FIELDS[0]] = ["changed"]
        write_canonical_json(temp_repository / "compatibility/snapshots/state.json", state)

    def forbid(source: Path, destination: Path) -> None:
        del source, destination
        raise AssertionError("review must refuse before any replacement")

    result, calls = invoke_review(
        (
            risk,
            "--status",
            "accepted",
            "--owner",
            _OWNER,
            "--objective-evidence-reviewed",
            "true",
            "--normative-constraints-preserved",
            "true",
        ),
        temp_repository,
        replace_policy=forbid,
    )

    assert result.exit_code == 1
    assert "projection" in result.stdout + result.stderr
    assert calls == []
    assert {path: path.read_bytes() for path in destinations} == before


def test_cli_refuses_stale_input_digest_without_replacement(
    temp_repository: Path,
) -> None:
    risk: Risk = "semantic-native"
    destinations = review_destinations(temp_repository, risk)
    before = {path: path.read_bytes() for path in destinations}
    (temp_repository / "qualification/rust-toolchain.toml").write_text(
        "changed\n", encoding="utf-8"
    )
    result, calls = invoke_review(
        (
            risk,
            "--status",
            "accepted",
            "--owner",
            _OWNER,
            "--objective-evidence-reviewed",
            "true",
            "--normative-constraints-preserved",
            "true",
        ),
        temp_repository,
    )
    assert result.exit_code == 1
    assert "stale" in result.stdout + result.stderr
    assert calls == []
    assert {path: path.read_bytes() for path in destinations} == before


def test_cli_refuses_malformed_record_without_replacement(temp_repository: Path) -> None:
    risk: Risk = "frontend-embedding"
    destinations = review_destinations(temp_repository, risk)
    (temp_repository / RECORD_PATHS[risk]).write_bytes(b'{"broken": true}\n')
    # Snapshot after the deliberate corruption so the comparison below
    # measures exactly what the CLI did, not the seeded corruption.
    before = {path: path.read_bytes() for path in destinations}
    result, calls = invoke_review(
        (
            risk,
            "--status",
            "accepted",
            "--owner",
            _OWNER,
            "--objective-evidence-reviewed",
            "true",
            "--normative-constraints-preserved",
            "true",
        ),
        temp_repository,
    )
    assert result.exit_code == 2
    assert calls == []
    assert {path: path.read_bytes() for path in destinations} == before


def test_cli_refuses_wrong_owner_without_replacement(temp_repository: Path) -> None:
    risk: Risk = "mcp-transport"
    destinations = review_destinations(temp_repository, risk)
    before = {path: path.read_bytes() for path in destinations}
    result, calls = invoke_review(
        (
            risk,
            "--status",
            "accepted",
            "--owner",
            "Someone Else <someone@example.net>",
            "--objective-evidence-reviewed",
            "true",
            "--normative-constraints-preserved",
            "true",
        ),
        temp_repository,
    )
    assert result.exit_code == 2
    assert calls == []
    assert {path: path.read_bytes() for path in destinations} == before


def test_cli_refuses_pending_status(temp_repository: Path) -> None:
    with pytest.raises(SystemExit) as excinfo:
        invoke_review(
            (
                "mcp-transport",
                "--status",
                "pending",
                "--owner",
                _OWNER,
                "--objective-evidence-reviewed",
                "false",
                "--normative-constraints-preserved",
                "false",
            ),
            temp_repository,
        )
    assert excinfo.value.code == 2
