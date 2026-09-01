"""Deterministic factories shared by Rust qualification tests."""

from __future__ import annotations

import json
import sys
from dataclasses import replace
from pathlib import Path
from typing import Literal

_ROOT = Path(__file__).resolve().parents[2]
if str(_ROOT) not in sys.path:
    sys.path.insert(0, str(_ROOT))

from tools.qualification.model import (  # noqa: E402
    REQUIRED_CRITERIA,
    CleanupEvidence,
    Environment,
    Evidence,
    Measurements,
    QualificationRecord,
    Review,
    ReviewStatus,
    Risk,
    decision_for,
    expected_consequence,
    status_for,
)

_OWNER = "Pavel Obruchnikov <me@inkyquill.net>"
_TARGET = "x86_64-unknown-linux-gnu"


def make_record(
    repo_root: Path,
    risk: Risk,
    *,
    failed: tuple[str, ...] = (),
    review_status: ReviewStatus = "pending",
) -> QualificationRecord:
    """Build one deterministic, internally consistent qualification record."""
    del repo_root  # Task 3 replaces the placeholder digest with an input fingerprint.
    unknown = set(failed) - set(REQUIRED_CRITERIA[risk])
    if unknown:
        raise ValueError(f"unknown failed criteria: {', '.join(sorted(unknown))}")

    evidence = tuple(
        Evidence(
            criterion=criterion,
            status="fail" if criterion in failed else "pass",
            summary=f"{criterion} {'failed' if criterion in failed else 'passed'}",
            measurements=Measurements(),
        )
        for criterion in REQUIRED_CRITERIA[risk]
    )
    status = status_for(evidence)
    reviewed = review_status == "accepted"
    return QualificationRecord(
        schema_version=1,
        risk=risk,
        target=_TARGET,
        status=status,
        decision=decision_for(risk, evidence),
        acceptance_owner=_OWNER,
        specs=(f"docs/qualification/{risk}.md",),
        contract_ids=(f"qualification.{risk}",),
        input_paths=("tools/qualification/model.py",),
        input_digest="0" * 64,
        commands=(f"qualification {risk}",),
        environment=Environment(
            rustc="rustc 1.96.0",
            cargo="cargo 1.96.0",
            target=_TARGET,
            os="linux",
            kernel="test-kernel",
            architecture="x86_64",
            bun=None,
            native_libraries=(),
        ),
        dependencies=(),
        evidence=evidence,
        consequence=expected_consequence(risk, status),
        cleanup=CleanupEvidence(
            work_dir_removed=True,
            raw_logs_removed=True,
            install_dir_removed=True,
            source_inputs_unchanged=True,
            user_data_opened=False,
            core_dumps_disabled=True,
            owned_process_groups_reaped=True,
        ),
        review=Review(
            owner=_OWNER,
            status=review_status,
            objective_evidence_reviewed=reviewed,
            normative_constraints_preserved=reviewed,
        ),
    )


def accepted_records(
    repo_root: Path,
    *,
    semantic: Literal["semantic-enabled", "fts-only"],
) -> dict[Risk, QualificationRecord]:
    """Return a complete accepted-record set for aggregate-gate tests."""
    records = {
        risk: make_record(repo_root, risk, review_status="accepted") for risk in REQUIRED_CRITERIA
    }
    if semantic == "fts-only":
        records["semantic-native"] = accepted_failure(repo_root, "semantic-native")
    return records


def accepted_failure(repo_root: Path, risk: Risk) -> QualificationRecord:
    """Return an accepted record with the risk's first criterion failed."""
    return make_record(
        repo_root,
        risk,
        failed=(REQUIRED_CRITERIA[risk][0],),
        review_status="accepted",
    )


def pending_review(record: QualificationRecord) -> QualificationRecord:
    """Copy a measured record back to the initial pending-review state."""
    return replace(
        record,
        review=Review(
            owner=record.review.owner,
            status="pending",
            objective_evidence_reviewed=False,
            normative_constraints_preserved=False,
        ),
    )


def write_fake_executable(
    tmp_path: Path,
    *,
    failed_criteria: tuple[str, ...] = (),
) -> Path:
    """Write an executable that reports a deterministic criterion failure list."""
    executable = tmp_path / "fake-qualification-executable"
    payload = json.dumps({"failed_criteria": list(failed_criteria)}, sort_keys=True)
    executable.write_text(
        f"#!{sys.executable}\nprint({payload!r})\n",
        encoding="utf-8",
    )
    executable.chmod(executable.stat().st_mode | 0o111)
    return executable


def fake_child_and_grandchild(tmp_path: Path) -> tuple[str, ...]:
    """Return a command whose child process owns a sleeping grandchild."""
    script = tmp_path / "child-and-grandchild.py"
    grandchild = "import time; time.sleep(60)"
    script.write_text(
        "import subprocess\n"
        "import sys\n"
        "import time\n"
        f"subprocess.Popen([sys.executable, '-c', {grandchild!r}])\n"
        "time.sleep(60)\n",
        encoding="utf-8",
    )
    return (sys.executable, str(script))
