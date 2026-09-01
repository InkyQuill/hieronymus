"""Typed qualification records and immutable risk decision rules."""

from __future__ import annotations

import json
import re
from collections.abc import Iterator, Mapping
from dataclasses import dataclass
from math import isfinite
from pathlib import Path
from typing import Literal, cast

Risk = Literal[
    "mcp-transport",
    "semantic-native",
    "frontend-embedding",
    "legacy-database-import",
]
EvidenceStatus = Literal["pass", "fail", "not-run"]
Decision = Literal["qualified", "blocked", "semantic-enabled", "fts-only"]
ReviewStatus = Literal["pending", "accepted", "rejected"]
JsonScalar = str | int | float | bool | None
MeasurementValue = JsonScalar | tuple[JsonScalar, ...]


class Measurements(Mapping[str, MeasurementValue]):
    """Immutable named scalar measurements with JSON-friendly dataclass copies."""

    __slots__ = ("_items",)

    def __init__(self, values: Mapping[str, MeasurementValue] | None = None) -> None:
        source = {} if values is None else values
        if not isinstance(source, Mapping):
            raise ValueError("evidence measurements must be an object with named measurements")
        items: list[tuple[str, MeasurementValue]] = []
        for key, value in source.items():
            if not isinstance(key, str) or not key.strip():
                raise ValueError("evidence measurement names must be nonblank strings")
            items.append((key, _measurement_value(value, key)))
        object.__setattr__(self, "_items", tuple(sorted(items)))

    def __getitem__(self, key: str) -> MeasurementValue:
        for item_key, value in self._items:
            if item_key == key:
                return value
        raise KeyError(key)

    def __iter__(self) -> Iterator[str]:
        return (key for key, _ in self._items)

    def __len__(self) -> int:
        return len(self._items)

    def __setattr__(self, name: str, value: object) -> None:
        del name, value
        raise AttributeError("Measurements is immutable")

    def __delattr__(self, name: str) -> None:
        del name
        raise AttributeError("Measurements is immutable")

    def __deepcopy__(self, memo: dict[int, object]) -> dict[str, MeasurementValue]:
        """Let dataclasses.asdict produce a plain mapping for strict JSON serialization."""
        del memo
        return dict(self._items)


REQUIRED_CRITERIA: dict[Risk, tuple[str, ...]] = {
    "mcp-transport": (
        "locked-native-build",
        "protocol-2026-07-28",
        "no-handshake-or-session",
        "per-request-required-metadata",
        "unsupported-version-rejected",
        "stdio-newline-jsonrpc",
        "streamable-http-json",
        "streamable-http-sse",
        "http-method-name-headers",
        "http-host-auth-version-cases",
        "official-schema-envelopes",
        "header-mismatch-errors",
        "registry-identity",
        "result-error-identity",
        "result-type-required",
        "required-auth-metadata",
        "private-bridge-absent",
    ),
    "semantic-native": (
        "locked-native-build",
        "binary-size-recorded",
        "install-uninstall",
        "model-checksum-load",
        "ten-thousand-chunk-build",
        "ann-index-created",
        "series-prefilter-before-ann",
        "zero-cross-series-hits",
        "insert-search-delete",
        "generation-isolation",
        "durable-sqlite-job-state",
        "no-sqlite-write-across-native-io",
        "crash-recovery",
        "cancel-recovery",
        "fts-fallback",
        "fifty-query-run",
    ),
    "frontend-embedding": (
        "bun-version-and-frozen-build",
        "missing-bundle-rejected",
        "release-assets-embedded",
        "index-and-spa-fallback",
        "hashed-asset-and-mime",
        "missing-asset-404",
        "runtime-asset-root-inaccessible",
        "no-runtime-bun-node-python",
        "no-source-map-secret",
        "binary-size-recorded",
    ),
    "legacy-database-import": (
        "bundled-sqlite-fts5",
        "fixture-classification",
        "supported-current-read",
        "supported-legacy-read",
        "typed-row-accounting",
        "fts-query-equivalence",
        "ledger-preserved",
        "unsupported-fail-closed",
        "source-byte-identity",
        "no-sensitive-row-output",
    ),
}

FAILURE_CONSEQUENCES: dict[Risk, str] = {
    "mcp-transport": (
        "Block rust-workspace-and-contract-harness and rust-daemon-mcp-security; "
        "preserve MCP revision 2026-07-28, stdio, POST /mcp JSON/SSE, and removal "
        "of /api/mcp/{operation}."
    ),
    "semantic-native": (
        "Select FTS5-only for the initial x86_64-unknown-linux-gnu release; do not "
        "block rust-workspace-and-contract-harness or the Linux release."
    ),
    "frontend-embedding": (
        "Block rust-workspace-and-contract-harness, rust-frontend, and "
        "rust-distribution-cutover; preserve embedded Svelte assets and no Bun, "
        "Node, or Python runtime dependency."
    ),
    "legacy-database-import": (
        "Block rust-workspace-and-contract-harness, rust-database-upgrade, and "
        "rust-distribution-cutover; preserve every supported source schema and "
        "never create a fresh sibling database beside legacy data."
    ),
}

_PASSING_DECISIONS: dict[Risk, Decision] = {
    "mcp-transport": "qualified",
    "semantic-native": "semantic-enabled",
    "frontend-embedding": "qualified",
    "legacy-database-import": "qualified",
}
_FAILING_DECISIONS: dict[Risk, Decision] = {
    "mcp-transport": "blocked",
    "semantic-native": "fts-only",
    "frontend-embedding": "blocked",
    "legacy-database-import": "blocked",
}
_OWNER = "Pavel Obruchnikov <me@inkyquill.net>"
_TARGET = "x86_64-unknown-linux-gnu"


@dataclass(frozen=True)
class Evidence:
    criterion: str
    status: EvidenceStatus
    summary: str
    measurements: Measurements
    not_run_reason: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "measurements", Measurements(self.measurements))


@dataclass(frozen=True)
class Environment:
    rustc: str
    cargo: str
    target: str
    os: str
    kernel: str
    architecture: str
    bun: str | None
    native_libraries: tuple[str, ...]


@dataclass(frozen=True)
class LockedDependency:
    name: str
    version: str
    source: str
    checksum: str | None
    features: tuple[str, ...]


@dataclass(frozen=True)
class CleanupEvidence:
    work_dir_removed: bool
    raw_logs_removed: bool
    install_dir_removed: bool
    source_inputs_unchanged: bool
    user_data_opened: bool
    core_dumps_disabled: bool
    owned_process_groups_reaped: bool


@dataclass(frozen=True)
class Review:
    owner: str
    status: ReviewStatus
    objective_evidence_reviewed: bool
    normative_constraints_preserved: bool


@dataclass(frozen=True)
class QualificationRecord:
    schema_version: int
    risk: Risk
    target: str
    status: Literal["pass", "fail"]
    decision: Decision
    acceptance_owner: str
    specs: tuple[str, ...]
    contract_ids: tuple[str, ...]
    input_paths: tuple[str, ...]
    input_digest: str
    commands: tuple[str, ...]
    environment: Environment
    dependencies: tuple[LockedDependency, ...]
    evidence: tuple[Evidence, ...]
    consequence: str
    cleanup: CleanupEvidence
    review: Review


def status_for(evidence: tuple[Evidence, ...]) -> Literal["pass", "fail"]:
    """Derive a status from one complete, ordered criterion set."""
    _validate_evidence(evidence)
    return "pass" if all(item.status == "pass" for item in evidence) else "fail"


def decision_for(risk: Risk, evidence: tuple[Evidence, ...]) -> Decision:
    """Derive the immutable decision for *risk* from complete evidence."""
    _validate_evidence(evidence, risk=risk)
    status = "pass" if all(item.status == "pass" for item in evidence) else "fail"
    return _PASSING_DECISIONS[risk] if status == "pass" else _FAILING_DECISIONS[risk]


def expected_consequence(risk: Risk, status: Literal["pass", "fail"]) -> str:
    """Return the exact consequence dictated by a measured status."""
    if status == "pass":
        return ""
    if status == "fail":
        return FAILURE_CONSEQUENCES[risk]
    raise ValueError(f"unknown qualification status: {status!r}")


def load_record(path: Path) -> QualificationRecord:
    """Load a strictly typed, internally consistent qualification record."""
    data = json.loads(
        path.read_text(encoding="utf-8"),
        parse_float=_finite_json_float,
        parse_constant=lambda value: _invalid_json_constant(value),
    )
    record_data = _object(
        data,
        "record",
        {
            "schema_version",
            "risk",
            "target",
            "status",
            "decision",
            "acceptance_owner",
            "specs",
            "contract_ids",
            "input_paths",
            "input_digest",
            "commands",
            "environment",
            "dependencies",
            "evidence",
            "consequence",
            "cleanup",
            "review",
        },
    )

    schema_version = _integer(record_data["schema_version"], "schema_version")
    if schema_version != 1:
        raise ValueError("schema_version must be 1")
    risk = _risk(record_data["risk"])
    target = _string(record_data["target"], "target")
    if target != _TARGET:
        raise ValueError(f"target must be {_TARGET}")
    evidence = _load_evidence(record_data["evidence"])
    derived_status = status_for(evidence)
    status = _enum(record_data["status"], "status", ("pass", "fail"))
    if status != derived_status:
        raise ValueError(f"status must be derived from criterion evidence: {derived_status}")
    derived_decision = decision_for(risk, evidence)
    decision = _enum(
        record_data["decision"],
        "decision",
        ("qualified", "blocked", "semantic-enabled", "fts-only"),
    )
    if decision != derived_decision:
        raise ValueError(f"decision must be {derived_decision} for {risk} {status} evidence")
    consequence = _string(record_data["consequence"], "consequence")
    required_consequence = expected_consequence(risk, status)
    if consequence != required_consequence:
        raise ValueError(f"consequence must match the immutable {risk} {status} consequence")

    acceptance_owner = _string(record_data["acceptance_owner"], "acceptance_owner")
    if acceptance_owner != _OWNER:
        raise ValueError(f"acceptance_owner must be {_OWNER}")
    review = _load_review(record_data["review"])
    if review.owner != _OWNER:
        raise ValueError(f"review owner must be {_OWNER}")

    return QualificationRecord(
        schema_version=schema_version,
        risk=risk,
        target=target,
        status=cast(Literal["pass", "fail"], status),
        decision=cast(Decision, decision),
        acceptance_owner=acceptance_owner,
        specs=_strings(record_data["specs"], "specs"),
        contract_ids=_strings(record_data["contract_ids"], "contract_ids"),
        input_paths=_strings(record_data["input_paths"], "input_paths"),
        input_digest=_input_digest(record_data["input_digest"]),
        commands=_strings(record_data["commands"], "commands"),
        environment=_load_environment(record_data["environment"]),
        dependencies=_load_dependencies(record_data["dependencies"]),
        evidence=evidence,
        consequence=consequence,
        cleanup=_load_cleanup(record_data["cleanup"]),
        review=review,
    )


def _validate_evidence(evidence: tuple[Evidence, ...], *, risk: Risk | None = None) -> None:
    if not isinstance(evidence, tuple):
        raise ValueError("criterion evidence must be a tuple")
    for item in evidence:
        if not isinstance(item, Evidence):
            raise ValueError("criterion evidence must contain Evidence records")
        _string(item.criterion, "evidence criterion")
        _enum(item.status, "evidence status", ("pass", "fail", "not-run"))
        _string(item.summary, "evidence summary")
        _validate_measurements(item.measurements)
        if item.not_run_reason is not None:
            _string(item.not_run_reason, "not-run reason")
        if item.status == "not-run" and not (item.not_run_reason and item.not_run_reason.strip()):
            raise ValueError("not-run evidence requires a reason")

    criteria = tuple(item.criterion for item in evidence)
    allowed = (REQUIRED_CRITERIA[risk],) if risk is not None else tuple(REQUIRED_CRITERIA.values())
    if criteria not in allowed:
        raise ValueError("criterion evidence must exactly match one ordered required criterion set")


def _load_evidence(value: object) -> tuple[Evidence, ...]:
    evidence = []
    for index, item in enumerate(_array(value, "evidence")):
        data = _object(
            item,
            f"evidence[{index}]",
            {"criterion", "status", "summary", "measurements", "not_run_reason"},
        )
        reason = data["not_run_reason"]
        if reason is not None:
            reason = _string(reason, f"evidence[{index}] not_run_reason")
        evidence.append(
            Evidence(
                criterion=_string(data["criterion"], f"evidence[{index}] criterion"),
                status=cast(
                    EvidenceStatus,
                    _enum(
                        data["status"],
                        f"evidence[{index}] status",
                        ("pass", "fail", "not-run"),
                    ),
                ),
                summary=_string(data["summary"], f"evidence[{index}] summary"),
                measurements=_measurements(data["measurements"], f"evidence[{index}] measurements"),
                not_run_reason=reason,
            )
        )
    return tuple(evidence)


def _load_environment(value: object) -> Environment:
    data = _object(
        value,
        "environment",
        {
            "rustc",
            "cargo",
            "target",
            "os",
            "kernel",
            "architecture",
            "bun",
            "native_libraries",
        },
    )
    bun = data["bun"]
    if bun is not None:
        bun = _string(bun, "environment bun")
    target = _string(data["target"], "environment target")
    if target != _TARGET:
        raise ValueError(f"environment target must be {_TARGET}")
    return Environment(
        rustc=_string(data["rustc"], "environment rustc"),
        cargo=_string(data["cargo"], "environment cargo"),
        target=target,
        os=_string(data["os"], "environment os"),
        kernel=_string(data["kernel"], "environment kernel"),
        architecture=_string(data["architecture"], "environment architecture"),
        bun=bun,
        native_libraries=_strings(data["native_libraries"], "environment native_libraries"),
    )


def _load_dependencies(value: object) -> tuple[LockedDependency, ...]:
    dependencies = []
    for index, item in enumerate(_array(value, "dependencies")):
        data = _object(
            item,
            f"dependencies[{index}]",
            {"name", "version", "source", "checksum", "features"},
        )
        checksum = data["checksum"]
        if checksum is not None:
            checksum = _string(checksum, f"dependencies[{index}] checksum")
        dependencies.append(
            LockedDependency(
                name=_string(data["name"], f"dependencies[{index}] name"),
                version=_string(data["version"], f"dependencies[{index}] version"),
                source=_string(data["source"], f"dependencies[{index}] source"),
                checksum=checksum,
                features=_strings(data["features"], f"dependencies[{index}] features"),
            )
        )
    return tuple(dependencies)


def _load_cleanup(value: object) -> CleanupEvidence:
    fields = {
        "work_dir_removed",
        "raw_logs_removed",
        "install_dir_removed",
        "source_inputs_unchanged",
        "user_data_opened",
        "core_dumps_disabled",
        "owned_process_groups_reaped",
    }
    data = _object(value, "cleanup", fields)
    return CleanupEvidence(**{field: _boolean(data[field], f"cleanup {field}") for field in fields})


def _load_review(value: object) -> Review:
    data = _object(
        value,
        "review",
        {
            "owner",
            "status",
            "objective_evidence_reviewed",
            "normative_constraints_preserved",
        },
    )
    return Review(
        owner=_string(data["owner"], "review owner"),
        status=cast(
            ReviewStatus,
            _enum(data["status"], "review status", ("pending", "accepted", "rejected")),
        ),
        objective_evidence_reviewed=_boolean(
            data["objective_evidence_reviewed"], "review objective_evidence_reviewed"
        ),
        normative_constraints_preserved=_boolean(
            data["normative_constraints_preserved"], "review normative_constraints_preserved"
        ),
    )


def _measurements(value: object, name: str) -> Measurements:
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value):
        raise ValueError(f"{name} must be an object with named measurements")
    converted: dict[str, MeasurementValue] = {}
    for key, item in value.items():
        if not key.strip():
            raise ValueError(f"{name} measurement names must not be blank")
        if _is_json_scalar(item):
            converted[key] = _json_scalar(item, f"{name} measurement {key!r}")
        elif isinstance(item, list) and all(_is_json_scalar(element) for element in item):
            converted[key] = tuple(
                _json_scalar(element, f"{name} measurement {key!r}") for element in item
            )
        else:
            raise ValueError(f"{name} measurement {key!r} must be a scalar or scalar array")
    return Measurements(converted)


def _validate_measurements(measurements: Measurements) -> None:
    if not isinstance(measurements, Measurements):
        raise ValueError("evidence measurements must be an object with named measurements")
    for key, value in measurements.items():
        _measurement_value(value, key)


def _is_json_scalar(value: object) -> bool:
    return value is None or type(value) in (str, int, float, bool)


_MAX_FINITE_MEASUREMENT = float.fromhex("0x1.fffffffffffffp+1023")


def _measurement_value(value: object, name: str) -> MeasurementValue:
    if _is_json_scalar(value):
        return _json_scalar(value, f"evidence measurement {name!r}")
    if isinstance(value, tuple):
        if not all(_is_json_scalar(item) for item in value):
            raise ValueError(f"evidence measurement {name!r} must be a scalar or scalar tuple")
        return tuple(_json_scalar(item, f"evidence measurement {name!r}") for item in value)
    raise ValueError(f"evidence measurement {name!r} must be a scalar or scalar tuple")


def _json_scalar(value: object, name: str) -> JsonScalar:
    if type(value) is float:
        if not isfinite(value):
            raise ValueError(f"{name} must be finite")
    elif type(value) is int and not (-_MAX_FINITE_MEASUREMENT <= value <= _MAX_FINITE_MEASUREMENT):
        raise ValueError(f"{name} must be within the finite IEEE-754 range")
    return cast(JsonScalar, value)


def _finite_json_float(value: str) -> float:
    number = float(value)
    if not isfinite(number):
        raise ValueError(f"JSON number must be finite: {value}")
    return number


def _object(value: object, name: str, fields: set[str]) -> dict[str, object]:
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value):
        raise ValueError(f"{name} must be an object")
    missing = fields - value.keys()
    if missing:
        raise ValueError(f"{name} missing required fields: {', '.join(sorted(missing))}")
    unexpected = value.keys() - fields
    if unexpected:
        raise ValueError(f"{name} has unexpected fields: {', '.join(sorted(unexpected))}")
    return value


def _array(value: object, name: str) -> list[object]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be an array")
    return value


def _strings(value: object, name: str) -> tuple[str, ...]:
    return tuple(_string(item, f"{name} item") for item in _array(value, name))


def _string(value: object, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _integer(value: object, name: str) -> int:
    if type(value) is not int:
        raise ValueError(f"{name} must be an integer")
    return value


def _boolean(value: object, name: str) -> bool:
    if type(value) is not bool:
        raise ValueError(f"{name} must be a boolean")
    return value


def _enum(value: object, name: str, allowed: tuple[str, ...]) -> str:
    text = _string(value, name)
    if text not in allowed:
        raise ValueError(f"{name} must be one of: {', '.join(allowed)}")
    return text


def _risk(value: object) -> Risk:
    return cast(Risk, _enum(value, "risk", tuple(REQUIRED_CRITERIA)))


def _input_digest(value: object) -> str:
    digest = _string(value, "input_digest")
    if re.fullmatch(r"[0-9a-f]{64}", digest) is None:
        raise ValueError("input_digest must be a lowercase SHA-256 digest")
    return digest


def _invalid_json_constant(value: str) -> None:
    raise ValueError(f"invalid JSON constant: {value}")
