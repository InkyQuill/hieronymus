"""Contract tests for qualification record loading and risk decisions."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from dataclasses import FrozenInstanceError, asdict, replace
from pathlib import Path

import pytest
from factories import make_record, pending_review
from jsonschema import Draft202012Validator

from tools.qualification.model import (
    FAILURE_CONSEQUENCES,
    REQUIRED_CRITERIA,
    Risk,
    decision_for,
    expected_consequence,
    load_record,
    status_for,
)

ROOT = Path(__file__).resolve().parents[2]
SCHEMA_PATH = ROOT / "qualification/schemas/record.schema.json"


def _write_payload(tmp_path: Path, payload: dict[str, object]) -> Path:
    path = tmp_path / "record.json"
    path.write_text(json.dumps(payload), encoding="utf-8")
    return path


def _payload_for(risk: Risk = "mcp-transport") -> dict[str, object]:
    return json.loads(json.dumps(asdict(make_record(ROOT, risk))))


def _schema_errors(payload: object) -> list[object]:
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    return list(Draft202012Validator(schema).iter_errors(payload))


def test_semantic_failure_is_complete_and_selects_fts_only() -> None:
    record = make_record(ROOT, "semantic-native", failed=("locked-native-build",))
    assert record.decision == "fts-only"
    assert decision_for(record.risk, record.evidence) == "fts-only"


def test_blocking_failure_cannot_change_the_contract() -> None:
    record = make_record(ROOT, "mcp-transport", failed=("locked-native-build",))
    assert status_for(record.evidence) == "fail"
    assert record.decision == "blocked"


def test_every_risk_has_exactly_one_ordered_criterion_set() -> None:
    assert tuple(REQUIRED_CRITERIA) == (
        "mcp-transport",
        "semantic-native",
        "frontend-embedding",
        "legacy-database-import",
    )
    assert tuple(map(len, REQUIRED_CRITERIA.values())) == (17, 16, 10, 10)
    assert all(len(criteria) == len(set(criteria)) for criteria in REQUIRED_CRITERIA.values())


@pytest.mark.parametrize(
    ("risk", "passing", "failing"),
    [
        ("mcp-transport", "qualified", "blocked"),
        ("semantic-native", "semantic-enabled", "fts-only"),
        ("frontend-embedding", "qualified", "blocked"),
        ("legacy-database-import", "qualified", "blocked"),
    ],
)
def test_decision_and_consequence_are_fixed_by_risk_and_complete_evidence(
    risk: Risk,
    passing: str,
    failing: str,
) -> None:
    passed = make_record(ROOT, risk)
    failed = make_record(ROOT, risk, failed=(REQUIRED_CRITERIA[risk][-1],))
    assert (passed.status, passed.decision, passed.consequence) == ("pass", passing, "")
    assert (failed.status, failed.decision, failed.consequence) == (
        "fail",
        failing,
        FAILURE_CONSEQUENCES[risk],
    )
    assert expected_consequence(risk, "pass") == ""
    assert expected_consequence(risk, "fail") == FAILURE_CONSEQUENCES[risk]


@pytest.mark.parametrize("mutation", ["missing", "duplicate", "unknown", "out-of-order"])
def test_incomplete_or_ambiguous_criterion_evidence_is_rejected(mutation: str) -> None:
    evidence = list(make_record(ROOT, "frontend-embedding").evidence)
    if mutation == "missing":
        evidence.pop()
    elif mutation == "duplicate":
        evidence[-1] = evidence[0]
    elif mutation == "unknown":
        evidence[-1] = replace(evidence[-1], criterion="not-a-criterion")
    else:
        evidence[0], evidence[1] = evidence[1], evidence[0]

    with pytest.raises(ValueError, match="criterion evidence"):
        status_for(tuple(evidence))
    with pytest.raises(ValueError, match="criterion evidence"):
        decision_for("frontend-embedding", tuple(evidence))


def test_not_run_is_a_failure_and_requires_a_nonblank_reason() -> None:
    record = make_record(ROOT, "semantic-native")
    not_run = replace(
        record.evidence[0],
        status="not-run",
        not_run_reason="native dependency was unavailable",
    )
    evidence = (not_run, *record.evidence[1:])
    assert status_for(evidence) == "fail"
    assert decision_for(record.risk, evidence) == "fts-only"

    for reason in (None, "", "   "):
        invalid = (replace(not_run, not_run_reason=reason), *record.evidence[1:])
        with pytest.raises(ValueError, match="not-run evidence requires a reason"):
            status_for(invalid)


def test_load_record_round_trips_to_frozen_typed_records(tmp_path: Path) -> None:
    original = replace(
        make_record(ROOT, "semantic-native"),
        evidence=(
            replace(
                make_record(ROOT, "semantic-native").evidence[0],
                measurements={"duration_ms": 12, "samples": (1, 2.5, None, True)},
            ),
            *make_record(ROOT, "semantic-native").evidence[1:],
        ),
    )
    loaded = load_record(_write_payload(tmp_path, json.loads(json.dumps(asdict(original)))))
    assert loaded == original
    assert isinstance(loaded.specs, tuple)
    assert isinstance(loaded.evidence, tuple)
    assert loaded.evidence[0].measurements["samples"] == (1, 2.5, None, True)
    with pytest.raises(FrozenInstanceError):
        loaded.status = "fail"  # type: ignore[misc]


def test_factory_loaded_and_review_copy_measurements_are_deeply_immutable(
    tmp_path: Path,
) -> None:
    base = make_record(ROOT, "semantic-native")
    factory_record = replace(
        base,
        evidence=(
            replace(
                base.evidence[0],
                measurements={"duration_ms": 12, "samples": (1, 2.5)},
            ),
            *base.evidence[1:],
        ),
    )
    loaded_record = load_record(
        _write_payload(tmp_path, json.loads(json.dumps(asdict(factory_record))))
    )
    review_copy = pending_review(factory_record)

    for record in (factory_record, loaded_record, review_copy):
        with pytest.raises(TypeError):
            record.evidence[0].measurements["duration_ms"] = 13  # type: ignore[index]
        samples = record.evidence[0].measurements["samples"]
        assert isinstance(samples, tuple)
        with pytest.raises(TypeError):
            samples[0] = 99  # type: ignore[index]

    serialized = json.dumps(asdict(loaded_record), allow_nan=False, sort_keys=True)
    assert json.loads(serialized)["evidence"][0]["measurements"] == {
        "duration_ms": 12,
        "samples": [1, 2.5],
    }


def test_evidence_construction_detaches_from_mutable_measurement_input() -> None:
    base = make_record(ROOT, "mcp-transport")
    source = {"samples": (1, 2)}
    evidence = replace(base.evidence[0], measurements=source)

    source["samples"] = (3, 4)

    assert evidence.measurements["samples"] == (1, 2)


@pytest.mark.parametrize("mutation", ["missing", "duplicate", "unknown", "out-of-order"])
def test_loader_and_schema_reject_the_same_invalid_criterion_sets(
    tmp_path: Path,
    mutation: str,
) -> None:
    payload = _payload_for("legacy-database-import")
    evidence = payload["evidence"]
    assert isinstance(evidence, list)
    if mutation == "missing":
        evidence.pop()
    elif mutation == "duplicate":
        evidence[-1] = evidence[0]
    elif mutation == "unknown":
        assert isinstance(evidence[-1], dict)
        evidence[-1]["criterion"] = "not-a-criterion"
    else:
        evidence[0], evidence[1] = evidence[1], evidence[0]

    with pytest.raises(ValueError, match="criterion evidence"):
        load_record(_write_payload(tmp_path, payload))
    assert _schema_errors(payload)


def test_loader_and_schema_reject_record_rule_mismatches(tmp_path: Path) -> None:
    mutations = (
        ("status", "fail"),
        ("decision", "blocked"),
        ("consequence", FAILURE_CONSEQUENCES["mcp-transport"]),
        ("risk", "unknown-risk"),
    )
    for field, value in mutations:
        payload = _payload_for()
        payload[field] = value
        with pytest.raises(ValueError):
            load_record(_write_payload(tmp_path, payload))
        assert _schema_errors(payload)


def test_loader_and_schema_derive_status_from_evidence_even_if_rules_change_together(
    tmp_path: Path,
) -> None:
    payload = _payload_for("mcp-transport")
    payload["status"] = "fail"
    payload["decision"] = "blocked"
    payload["consequence"] = FAILURE_CONSEQUENCES["mcp-transport"]
    with pytest.raises(ValueError, match="status must be derived"):
        load_record(_write_payload(tmp_path, payload))
    assert _schema_errors(payload)


def test_loader_and_schema_require_a_canonical_sha256_input_digest(tmp_path: Path) -> None:
    payload = _payload_for()
    payload["input_digest"] = "not-a-sha256"
    with pytest.raises(ValueError, match="input_digest"):
        load_record(_write_payload(tmp_path, payload))
    assert _schema_errors(payload)


def test_loader_and_schema_reject_unknown_or_missing_fixed_fields(tmp_path: Path) -> None:
    payload = _payload_for()
    environment = payload["environment"]
    assert isinstance(environment, dict)
    environment["unexpected"] = True
    with pytest.raises(ValueError, match="unexpected fields"):
        load_record(_write_payload(tmp_path, payload))
    assert _schema_errors(payload)

    payload = _payload_for()
    review = payload["review"]
    assert isinstance(review, dict)
    del review["owner"]
    with pytest.raises(ValueError, match="missing required fields"):
        load_record(_write_payload(tmp_path, payload))
    assert _schema_errors(payload)


def test_loader_and_schema_couple_environment_to_the_fixed_target(tmp_path: Path) -> None:
    payload = _payload_for()
    environment = payload["environment"]
    assert isinstance(environment, dict)
    environment["target"] = "aarch64-unknown-linux-gnu"

    with pytest.raises(ValueError, match="environment target"):
        load_record(_write_payload(tmp_path, payload))
    assert _schema_errors(payload)


def test_measurements_allow_only_named_scalars_or_scalar_arrays(tmp_path: Path) -> None:
    valid = _payload_for()
    evidence = valid["evidence"]
    assert isinstance(evidence, list) and isinstance(evidence[0], dict)
    evidence[0]["measurements"] = {
        "text": "ok",
        "integer": 1,
        "number": 1.5,
        "flag": False,
        "empty": None,
        "samples": ["a", 2, 3.5, True, None],
    }
    assert load_record(_write_payload(tmp_path, valid)).evidence[0].measurements["samples"] == (
        "a",
        2,
        3.5,
        True,
        None,
    )
    assert _schema_errors(valid) == []

    for invalid_measurement in ({"raw": {"stdout": "payload"}}, {"nested": [[1]]}):
        invalid = _payload_for()
        invalid_evidence = invalid["evidence"]
        assert isinstance(invalid_evidence, list) and isinstance(invalid_evidence[0], dict)
        invalid_evidence[0]["measurements"] = invalid_measurement
        with pytest.raises(ValueError, match="measurement"):
            load_record(_write_payload(tmp_path, invalid))
        assert _schema_errors(invalid)


@pytest.mark.parametrize(
    "measurement",
    [
        pytest.param(float("inf"), id="positive-infinity"),
        pytest.param(float("-inf"), id="negative-infinity"),
        pytest.param(float("nan"), id="nan"),
        pytest.param((1, float("inf")), id="nested-positive-infinity"),
        pytest.param((float("nan"), 1), id="nested-nan"),
    ],
)
def test_evidence_construction_rejects_non_finite_measurements(
    measurement: object,
) -> None:
    evidence = make_record(ROOT, "mcp-transport").evidence[0]

    with pytest.raises(ValueError, match="finite"):
        replace(evidence, measurements={"probe": measurement})  # type: ignore[dict-item]


def test_loader_and_schema_reject_overflowed_json_number_token(tmp_path: Path) -> None:
    payload = _payload_for()
    serialized = json.dumps(payload).replace(
        '"measurements": {}',
        '"measurements": {"overflow": 1e999}',
        1,
    )
    path = tmp_path / "overflow.json"
    path.write_text(serialized, encoding="utf-8")

    with pytest.raises(ValueError, match="finite"):
        load_record(path)
    assert _schema_errors(json.loads(serialized))


def test_loader_and_schema_enforce_finite_binary64_measurement_range(tmp_path: Path) -> None:
    valid = _payload_for()
    valid_evidence = valid["evidence"]
    assert isinstance(valid_evidence, list) and isinstance(valid_evidence[0], dict)
    valid_evidence[0]["measurements"] = {
        "minimum": -sys.float_info.max,
        "ordinary": 12.5,
        "maximum": sys.float_info.max,
    }
    loaded = load_record(_write_payload(tmp_path, valid))
    assert _schema_errors(valid) == []
    json.dumps(asdict(loaded), allow_nan=False)

    for outside_range in (10**309, -(10**309)):
        invalid = _payload_for()
        invalid_evidence = invalid["evidence"]
        assert isinstance(invalid_evidence, list) and isinstance(invalid_evidence[0], dict)
        invalid_evidence[0]["measurements"] = {"outside_binary64": outside_range}
        with pytest.raises(ValueError, match="finite IEEE-754 range"):
            load_record(_write_payload(tmp_path, invalid))
        assert _schema_errors(invalid)


def test_schema_is_draft_2020_12_and_accepts_every_factory_record() -> None:
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    for risk in REQUIRED_CRITERIA:
        assert _schema_errors(_payload_for(risk)) == []


def test_factory_rejects_unknown_failed_criterion() -> None:
    with pytest.raises(ValueError, match="unknown failed criteria"):
        make_record(ROOT, "mcp-transport", failed=("not-a-criterion",))  # type: ignore[arg-type]


def test_status_rejects_invalid_evidence_value_types() -> None:
    record = make_record(ROOT, "mcp-transport")
    with pytest.raises(ValueError, match="measurement"):
        replace(
            record.evidence[0],
            measurements={"payload": ({"not": "a scalar"},)},  # type: ignore[dict-item]
        )

    invalid_status = replace(record.evidence[0], status="unknown")  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="evidence status"):
        status_for((invalid_status, *record.evidence[1:]))


def test_record_set_factories_express_acceptance_and_semantic_fallback() -> None:
    from factories import accepted_failure, accepted_records, pending_review

    enabled = accepted_records(ROOT, semantic="semantic-enabled")
    assert tuple(enabled) == tuple(REQUIRED_CRITERIA)
    assert all(record.review.status == "accepted" for record in enabled.values())
    assert all(record.status == "pass" for record in enabled.values())

    fallback = accepted_records(ROOT, semantic="fts-only")
    assert fallback["semantic-native"].decision == "fts-only"
    assert fallback["semantic-native"].review.status == "accepted"

    blocking = accepted_failure(ROOT, "frontend-embedding")
    assert (blocking.status, blocking.decision, blocking.review.status) == (
        "fail",
        "blocked",
        "accepted",
    )
    pending = pending_review(blocking)
    assert pending.review.status == "pending"
    assert not pending.review.objective_evidence_reviewed
    assert not pending.review.normative_constraints_preserved
    assert replace(pending, review=blocking.review) == blocking


def test_fake_executable_reports_only_the_requested_failed_criteria(tmp_path: Path) -> None:
    from factories import write_fake_executable

    executable = write_fake_executable(
        tmp_path,
        failed_criteria=("locked-native-build", "crash-recovery"),
    )
    result = subprocess.run((str(executable),), check=True, capture_output=True, text=True)
    assert os.access(executable, os.X_OK)
    assert json.loads(result.stdout) == {
        "failed_criteria": ["locked-native-build", "crash-recovery"]
    }
    assert result.stderr == ""


def test_child_and_grandchild_factory_returns_a_real_python_command(tmp_path: Path) -> None:
    from factories import fake_child_and_grandchild

    command = fake_child_and_grandchild(tmp_path)
    assert command[0] == sys.executable
    assert len(command) == 2
    assert Path(command[1]).is_file()
