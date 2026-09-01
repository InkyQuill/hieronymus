"""Contract tests for qualification record loading and risk decisions."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from dataclasses import FrozenInstanceError, asdict, dataclass, fields, replace
from decimal import Decimal
from math import copysign
from pathlib import Path

import pytest
from factories import make_record, pending_review
from jsonschema import Draft202012Validator

from tools.qualification.model import (
    FAILURE_CONSEQUENCES,
    REQUIRED_CRITERIA,
    CleanupEvidence,
    Environment,
    Evidence,
    LockedDependency,
    Measurements,
    QualificationRecord,
    Review,
    Risk,
    decision_for,
    expected_consequence,
    load_record,
    record_schema_validator,
    serialize_record,
    status_for,
)

ROOT = Path(__file__).resolve().parents[2]
SCHEMA_PATH = ROOT / "qualification/schemas/record.schema.json"


@dataclass(frozen=True)
class _ExtendedEvidence(Evidence):
    extra: str = "unexpected"


@dataclass(frozen=True)
class _ExtendedEnvironment(Environment):
    extra: str = "unexpected"


@dataclass(frozen=True)
class _ExtendedLockedDependency(LockedDependency):
    extra: str = "unexpected"


@dataclass(frozen=True)
class _ExtendedCleanupEvidence(CleanupEvidence):
    extra: str = "unexpected"


@dataclass(frozen=True)
class _ExtendedReview(Review):
    extra: str = "unexpected"


@dataclass(frozen=True)
class _ExtendedQualificationRecord(QualificationRecord):
    extra: str = "unexpected"


def _write_payload(tmp_path: Path, payload: dict[str, object]) -> Path:
    path = tmp_path / "record.json"
    path.write_text(json.dumps(payload), encoding="utf-8")
    return path


def _payload_for(risk: Risk = "mcp-transport") -> dict[str, object]:
    return json.loads(json.dumps(asdict(make_record(ROOT, risk))))


def _schema_errors(payload: object) -> list[object]:
    return list(record_schema_validator().iter_errors(payload))


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
        with pytest.raises(ValueError, match="not-run evidence requires a reason"):
            replace(not_run, not_run_reason=reason)


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


def _construct_record(
    record: QualificationRecord,
    **updates: object,
) -> QualificationRecord:
    values = {field.name: getattr(record, field.name) for field in fields(record)}
    values.update(updates)
    return QualificationRecord(**values)  # type: ignore[arg-type]


def _construct_dataclass(value: object, **updates: object) -> object:
    values = {field.name: getattr(value, field.name) for field in fields(value)}
    values.update(updates)
    return type(value)(**values)


def _unchecked_subclass_copy(value: object, subclass: type[object]) -> object:
    copied = object.__new__(subclass)
    for field in fields(value):
        object.__setattr__(copied, field.name, getattr(value, field.name))
    object.__setattr__(copied, "extra", "unexpected")
    return copied


def _locked_dependency() -> LockedDependency:
    return LockedDependency(
        name="example",
        version="1.0.0",
        source="registry+https://example.invalid/index",
        checksum=None,
        features=("default",),
    )


@pytest.mark.parametrize("construction", [_construct_record, replace])
def test_record_sequence_construction_detaches_and_normalizes_lists(
    construction: object,
) -> None:
    record = make_record(ROOT, "mcp-transport")
    scalar_sequences = {
        "specs": ["docs/qualification/example.md"],
        "contract_ids": ["qualification.example"],
        "input_paths": ["tools/qualification/model.py"],
        "commands": ["qualification example"],
    }
    for field_name, source in scalar_sequences.items():
        constructed = construction(record, **{field_name: source})  # type: ignore[operator]
        source.append("caller mutation")
        assert getattr(constructed, field_name) == tuple(source[:-1])
        assert type(getattr(constructed, field_name)) is tuple

    dependency_source = [_locked_dependency()]
    dependency_record = construction(  # type: ignore[operator]
        record,
        dependencies=dependency_source,
    )
    dependency_source.clear()
    assert dependency_record.dependencies == (_locked_dependency(),)
    assert type(dependency_record.dependencies) is tuple

    evidence_source = list(record.evidence)
    evidence_record = construction(record, evidence=evidence_source)  # type: ignore[operator]
    evidence_source.clear()
    assert evidence_record.evidence == record.evidence
    assert type(evidence_record.evidence) is tuple


@pytest.mark.parametrize("construction", [_construct_dataclass, replace])
def test_nested_sequence_construction_detaches_and_normalizes_lists(
    construction: object,
) -> None:
    record = make_record(ROOT, "mcp-transport")

    libraries = ["libsqlite3.so"]
    environment = construction(  # type: ignore[operator]
        record.environment,
        native_libraries=libraries,
    )
    libraries.append("caller mutation")
    assert environment.native_libraries == ("libsqlite3.so",)
    assert type(environment.native_libraries) is tuple

    features = ["default"]
    dependency = construction(_locked_dependency(), features=features)  # type: ignore[operator]
    features.append("caller mutation")
    assert dependency.features == ("default",)
    assert type(dependency.features) is tuple

    samples = [1, 2.5, None, True]
    measurement_source = {"samples": samples}
    evidence = construction(  # type: ignore[operator]
        record.evidence[0],
        measurements=measurement_source,
    )
    samples.append("caller mutation")
    measurement_source["new"] = 1
    assert evidence.measurements == {"samples": (1, 2.5, None, True)}
    assert type(evidence.measurements) is Measurements
    assert type(evidence.measurements["samples"]) is tuple


@pytest.mark.parametrize("construction", [_construct_dataclass, replace])
def test_nested_record_construction_rejects_wrong_scalar_shapes(
    construction: object,
) -> None:
    record = make_record(ROOT, "mcp-transport")
    invalid_mutations = (
        (record.evidence[0], {"criterion": 1}),
        (record.evidence[0], {"status": "unknown"}),
        (record.evidence[0], {"summary": 1}),
        (record.evidence[0], {"measurements": {"nested": [[1]]}}),
        (record.evidence[0], {"not_run_reason": 1}),
        (record.environment, {"rustc": 1}),
        (record.environment, {"cargo": 1}),
        (record.environment, {"target": "aarch64-unknown-linux-gnu"}),
        (record.environment, {"os": 1}),
        (record.environment, {"kernel": 1}),
        (record.environment, {"architecture": 1}),
        (record.environment, {"bun": 1}),
        (record.environment, {"native_libraries": ["valid", 1]}),
        (_locked_dependency(), {"name": 1}),
        (_locked_dependency(), {"version": 1}),
        (_locked_dependency(), {"source": 1}),
        (_locked_dependency(), {"checksum": 1}),
        (_locked_dependency(), {"features": ["valid", 1]}),
        (record.cleanup, {"work_dir_removed": 1}),
        (record.cleanup, {"raw_logs_removed": 1}),
        (record.cleanup, {"install_dir_removed": 1}),
        (record.cleanup, {"source_inputs_unchanged": 1}),
        (record.cleanup, {"user_data_opened": 0}),
        (record.cleanup, {"core_dumps_disabled": 1}),
        (record.cleanup, {"owned_process_groups_reaped": 1}),
        (record.review, {"owner": "Another Owner <owner@example.com>"}),
        (record.review, {"status": "unknown"}),
        (record.review, {"objective_evidence_reviewed": 1}),
        (record.review, {"normative_constraints_preserved": 1}),
    )

    for nested_record, mutation in invalid_mutations:
        with pytest.raises(ValueError):
            construction(nested_record, **mutation)  # type: ignore[operator]


@pytest.mark.parametrize("construction", [_construct_record, replace])
def test_record_construction_rejects_wrong_sequence_and_nested_record_shapes(
    construction: object,
) -> None:
    record = make_record(ROOT, "mcp-transport")
    dependency = _locked_dependency()
    invalid_mutations = (
        {"specs": ["valid", 1]},
        {"contract_ids": ["valid", 1]},
        {"input_paths": ["valid", 1]},
        {"commands": ["valid", 1]},
        {"environment": asdict(record.environment)},
        {"dependencies": [asdict(dependency)]},
        {"evidence": [asdict(item) for item in record.evidence]},
        {"cleanup": asdict(record.cleanup)},
        {"review": asdict(record.review)},
    )

    for mutation in invalid_mutations:
        with pytest.raises(ValueError):
            construction(record, **mutation)  # type: ignore[operator]


def test_closed_dataclass_shapes_reject_subclasses_directly_and_when_nested() -> None:
    record = make_record(ROOT, "mcp-transport")
    dependency = _locked_dependency()
    subclass_pairs = (
        (record.evidence[0], _ExtendedEvidence),
        (record.environment, _ExtendedEnvironment),
        (dependency, _ExtendedLockedDependency),
        (record.cleanup, _ExtendedCleanupEvidence),
        (record.review, _ExtendedReview),
        (record, _ExtendedQualificationRecord),
    )

    for value, subclass in subclass_pairs:
        values = {field.name: getattr(value, field.name) for field in fields(value)}
        with pytest.raises(ValueError, match="exact"):
            subclass(**values)

    invalid_evidence = _unchecked_subclass_copy(record.evidence[0], _ExtendedEvidence)
    invalid_environment = _unchecked_subclass_copy(record.environment, _ExtendedEnvironment)
    invalid_dependency = _unchecked_subclass_copy(dependency, _ExtendedLockedDependency)
    invalid_cleanup = _unchecked_subclass_copy(record.cleanup, _ExtendedCleanupEvidence)
    invalid_review = _unchecked_subclass_copy(record.review, _ExtendedReview)
    mutations = (
        {"evidence": (invalid_evidence, *record.evidence[1:])},
        {"environment": invalid_environment},
        {"dependencies": (invalid_dependency,)},
        {"cleanup": invalid_cleanup},
        {"review": invalid_review},
    )
    for mutation in mutations:
        with pytest.raises(ValueError, match="exact"):
            replace(record, **mutation)


@pytest.mark.parametrize("construction", [_construct_record, replace])
def test_record_construction_rejects_fixed_and_cross_field_mutations(
    construction: object,
) -> None:
    record = make_record(ROOT, "mcp-transport")
    alternate_target = "aarch64-unknown-linux-gnu"
    alternate_environment = object.__new__(Environment)
    for field in fields(record.environment):
        object.__setattr__(
            alternate_environment,
            field.name,
            alternate_target if field.name == "target" else getattr(record.environment, field.name),
        )
    alternate_review = object.__new__(Review)
    for field in fields(record.review):
        object.__setattr__(
            alternate_review,
            field.name,
            "Another Owner <owner@example.com>"
            if field.name == "owner"
            else getattr(record.review, field.name),
        )
    mismatched_evidence = make_record(ROOT, "frontend-embedding").evidence
    mutations = (
        {"target": alternate_target},
        {"environment": alternate_environment},
        {"target": alternate_target, "environment": alternate_environment},
        {"schema_version": 2},
        {"acceptance_owner": "Another Owner <owner@example.com>"},
        {"review": alternate_review},
        {"input_digest": "not-a-sha256"},
        {"evidence": mismatched_evidence},
        {"status": "fail"},
        {"decision": "blocked"},
        {"consequence": FAILURE_CONSEQUENCES["mcp-transport"]},
    )

    for mutation in mutations:
        with pytest.raises(ValueError):
            construction(record, **mutation)  # type: ignore[operator]


@pytest.mark.parametrize(
    ("status", "objective_reviewed", "constraints_preserved"),
    [
        ("pending", False, False),
        ("accepted", True, True),
        ("rejected", False, False),
    ],
)
def test_record_construction_preserves_task_20_review_states(
    status: str,
    objective_reviewed: bool,
    constraints_preserved: bool,
) -> None:
    record = make_record(ROOT, "mcp-transport")
    review = replace(
        record.review,
        status=status,  # type: ignore[arg-type]
        objective_evidence_reviewed=objective_reviewed,
        normative_constraints_preserved=constraints_preserved,
    )

    assert replace(record, review=review).review == review
    assert _construct_record(record, review=review).review == review


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


@pytest.mark.parametrize(
    "measurements",
    [
        pytest.param({"probe": float("nan")}, id="scalar-nan"),
        pytest.param({"probe": float("inf")}, id="scalar-positive-infinity"),
        pytest.param({"probe": float("-inf")}, id="scalar-negative-infinity"),
        pytest.param({"probe": [1, float("nan")]}, id="nested-nan"),
        pytest.param({"probe": [float("inf"), 1]}, id="nested-infinity"),
    ],
)
def test_strict_record_schema_validator_rejects_python_non_finite_numbers(
    measurements: dict[str, object],
) -> None:
    payload = _payload_for()
    evidence = payload["evidence"]
    assert isinstance(evidence, list) and isinstance(evidence[0], dict)
    evidence[0]["measurements"] = measurements

    assert _schema_errors(payload)


@pytest.mark.parametrize("token", ["NaN", "Infinity", "-Infinity"])
def test_raw_json_loader_rejects_non_finite_constants(tmp_path: Path, token: str) -> None:
    serialized = json.dumps(_payload_for()).replace(
        '"measurements": {}',
        f'"measurements": {{"non_finite": {token}}}',
        1,
    )
    path = tmp_path / "non-finite.json"
    path.write_text(serialized, encoding="utf-8")

    with pytest.raises(ValueError, match="invalid JSON constant"):
        load_record(path)


@pytest.mark.parametrize(
    "measurement",
    [
        pytest.param(float("nan"), id="nan"),
        pytest.param(float("inf"), id="positive-infinity"),
        pytest.param((1, float("-inf")), id="nested-negative-infinity"),
    ],
)
def test_record_serializer_revalidates_non_finite_measurements(measurement: object) -> None:
    record = make_record(ROOT, "mcp-transport")
    object.__setattr__(
        record.evidence[0].measurements,
        "_items",
        (("probe", measurement),),
    )

    with pytest.raises(ValueError, match="finite"):
        serialize_record(record)


def test_loader_rejects_exact_out_of_range_tokens_at_any_measurement_depth(
    tmp_path: Path,
) -> None:
    measurement_objects = (
        '{"outside": 1.7976931348623158e308}',
        '{"outside": -1.7976931348623158e308}',
        '{"outside": [1, 1.7976931348623158e308]}',
        '{"outside": [-1.7976931348623158e308, 1]}',
    )
    for index, measurements in enumerate(measurement_objects):
        serialized = json.dumps(_payload_for()).replace(
            '"measurements": {}',
            f'"measurements": {measurements}',
            1,
        )
        path = tmp_path / f"outside-{index}.json"
        path.write_text(serialized, encoding="utf-8")

        with pytest.raises(ValueError, match="finite IEEE-754 range"):
            load_record(path)


def test_loader_and_serializer_round_trip_exact_numeric_endpoints(
    tmp_path: Path,
) -> None:
    serialized = json.dumps(_payload_for()).replace(
        '"measurements": {}',
        (
            '"measurements": {'
            '"maximum": 1.7976931348623157e308,'
            '"minimum": -1.7976931348623157e308,'
            '"negative_zero": -0.0,'
            '"ordinary": [1.25e-3, -2e2]'
            "}"
        ),
        1,
    )
    path = tmp_path / "numeric-endpoints.json"
    path.write_text(serialized, encoding="utf-8")

    loaded = load_record(path)
    measurements = loaded.evidence[0].measurements
    assert measurements["maximum"] == sys.float_info.max
    assert measurements["minimum"] == -sys.float_info.max
    assert copysign(1.0, measurements["negative_zero"]) == -1.0  # type: ignore[arg-type]
    assert measurements["ordinary"] == (0.00125, -200.0)

    strict_json = serialize_record(loaded)
    assert '"negative_zero":-0.0' in strict_json
    strict_payload = json.loads(strict_json)
    serialized_measurements = strict_payload["evidence"][0]["measurements"]
    assert type(serialized_measurements["maximum"]) is float
    assert serialized_measurements["maximum"] == sys.float_info.max
    assert _schema_errors(strict_payload) == []
    round_trip = tmp_path / "numeric-round-trip.json"
    round_trip.write_text(strict_json, encoding="utf-8")
    assert load_record(round_trip) == loaded


def test_loader_and_schema_enforce_finite_binary64_measurement_range(tmp_path: Path) -> None:
    maximum_integer = int(sys.float_info.max)
    base = make_record(ROOT, "mcp-transport")
    bounded_evidence = replace(
        base.evidence[0],
        measurements={"integer_bounds": [-maximum_integer, maximum_integer]},
    )
    constructed = replace(base, evidence=(bounded_evidence, *base.evidence[1:]))
    assert constructed.evidence[0].measurements["integer_bounds"] == (
        -maximum_integer,
        maximum_integer,
    )
    assert json.loads(serialize_record(constructed))["evidence"][0]["measurements"] == {
        "integer_bounds": [-maximum_integer, maximum_integer]
    }

    valid = _payload_for()
    valid_evidence = valid["evidence"]
    assert isinstance(valid_evidence, list) and isinstance(valid_evidence[0], dict)
    valid_evidence[0]["measurements"] = {
        "integer_bounds": [-maximum_integer, maximum_integer],
    }
    loaded = load_record(_write_payload(tmp_path, valid))
    assert loaded.evidence[0].measurements["integer_bounds"] == (
        -maximum_integer,
        maximum_integer,
    )
    assert _schema_errors(valid) == []
    assert json.loads(serialize_record(loaded))["evidence"][0]["measurements"] == {
        "integer_bounds": [-maximum_integer, maximum_integer]
    }

    for outside_range in (maximum_integer + 1, -maximum_integer - 1, 10**999, -(10**999)):
        with pytest.raises(ValueError, match="finite IEEE-754 range"):
            replace(
                base.evidence[0],
                measurements={"outside_binary64": outside_range},
            )
        invalid = _payload_for()
        invalid_evidence = invalid["evidence"]
        assert isinstance(invalid_evidence, list) and isinstance(invalid_evidence[0], dict)
        invalid_evidence[0]["measurements"] = {"outside_binary64": outside_range}
        with pytest.raises(ValueError, match="finite IEEE-754 range"):
            load_record(_write_payload(tmp_path, invalid))
        assert _schema_errors(invalid)

    object.__setattr__(
        base.evidence[0].measurements,
        "_items",
        (("outside_binary64", maximum_integer + 1),),
    )
    with pytest.raises(ValueError, match="finite IEEE-754 range"):
        serialize_record(base)


def test_loader_and_schema_reject_overflowed_json_number_token(tmp_path: Path) -> None:
    measurement_objects = (
        '{"overflow": 1e999}',
        '{"overflow": -1e999}',
        '{"overflow": [1, 1e999]}',
        '{"overflow": [-1e999, 1]}',
    )
    for index, measurements in enumerate(measurement_objects):
        serialized = json.dumps(_payload_for()).replace(
            '"measurements": {}',
            f'"measurements": {measurements}',
            1,
        )
        path = tmp_path / f"overflow-{index}.json"
        path.write_text(serialized, encoding="utf-8")

        with pytest.raises(ValueError, match="finite IEEE-754 range"):
            load_record(path)
        assert _schema_errors(json.loads(serialized))


def test_schema_is_draft_2020_12_and_accepts_every_factory_record() -> None:
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    for risk in REQUIRED_CRITERIA:
        assert _schema_errors(_payload_for(risk)) == []


def test_schema_encodes_the_exact_binary64_integer_bound_portably() -> None:
    maximum = int(sys.float_info.max)
    schema_text = SCHEMA_PATH.read_text(encoding="utf-8")
    ordinary_schema = json.loads(schema_text)
    decimal_preserving_schema = json.loads(schema_text, parse_float=Decimal)
    ordinary_bounds = ordinary_schema["$defs"]["jsonScalar"]
    precise_bounds = decimal_preserving_schema["$defs"]["jsonScalar"]

    assert ordinary_bounds["maximum"] == maximum
    assert ordinary_bounds["minimum"] == -maximum
    assert type(ordinary_bounds["maximum"]) is int
    assert type(ordinary_bounds["minimum"]) is int
    assert Decimal(precise_bounds["maximum"]) == Decimal(maximum)
    assert Decimal(precise_bounds["minimum"]) == Decimal(-maximum)
    Draft202012Validator.check_schema(decimal_preserving_schema)

    endpoints = _payload_for()
    endpoint_evidence = endpoints["evidence"]
    assert isinstance(endpoint_evidence, list) and isinstance(endpoint_evidence[0], dict)
    endpoint_evidence[0]["measurements"] = {"bounds": [-maximum, maximum]}
    just_outside = (
        {"too_large": maximum + 1},
        {"too_small": -maximum - 1},
    )

    for schema in (ordinary_schema, decimal_preserving_schema):
        validator = Draft202012Validator(schema)
        assert list(validator.iter_errors(endpoints)) == []
        for measurements in just_outside:
            payload = _payload_for()
            evidence = payload["evidence"]
            assert isinstance(evidence, list) and isinstance(evidence[0], dict)
            evidence[0]["measurements"] = measurements
            assert list(validator.iter_errors(payload))


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

    with pytest.raises(ValueError, match="evidence status"):
        replace(record.evidence[0], status="unknown")  # type: ignore[arg-type]


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
