"""Single opt-in dispatcher for the four live qualification runners.

The dispatcher snapshots the caller's environment once and passes that one
unsanitized mapping unchanged to every ``run_one_live`` call; each risk runner
performs its own Task 5 tool discovery before sanitizing its child processes.
The snapshot is never placed in a record. Risk decisions are evaluated only by
``tools.qualification.check --require-qualified``, never here, so a valid
blocking record is written rather than discarded.
"""

from __future__ import annotations

import argparse
import os
import sys
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Protocol, cast

from tools.qualification import run_database, run_frontend, run_mcp, run_semantic
from tools.qualification.check import (
    AGGREGATE_PATH,
    GATE_MARKDOWN_PATH,
    MARKDOWN_PATHS,
    RECORD_PATHS,
    GateRecord,
    compute_gate,
    gate_payload,
    gate_schema_issues,
    load_records,
    render_gate,
    serialize_gate,
)
from tools.qualification.model import (
    REQUIRED_CRITERIA,
    QualificationRecord,
    Risk,
    serialize_record,
)
from tools.qualification.render import render_record

_WORK_ROOT = Path("qualification/.artifacts/work")


class LiveRunner(Protocol):
    """One risk runner's guarded live entry point."""

    def __call__(
        self,
        repo_root: Path,
        work_root: Path,
        *,
        original_env: Mapping[str, str],
    ) -> QualificationRecord: ...


LIVE_RUNNERS: dict[Risk, LiveRunner] = {
    "mcp-transport": run_mcp.run_live,
    "semantic-native": run_semantic.run_live,
    "frontend-embedding": run_frontend.run_live,
    "legacy-database-import": run_database.run_live,
}


def run_one_live(
    risk: Risk,
    repo_root: Path,
    work_root: Path,
    *,
    original_env: Mapping[str, str],
) -> QualificationRecord:
    """Dispatch one risk runner with the caller's unsanitized environment."""
    if original_env.get("HIERONYMUS_QUALIFICATION_LIVE") != "1":
        raise RuntimeError("live qualification requires HIERONYMUS_QUALIFICATION_LIVE=1")
    return LIVE_RUNNERS[risk](
        repo_root,
        work_root / risk,
        original_env=original_env,
    )


def write_risk_record(repo_root: Path, record: QualificationRecord) -> None:
    """Write one record's canonical JSON and rendered Markdown."""
    record_path = repo_root / RECORD_PATHS[record.risk]
    markdown_path = repo_root / MARKDOWN_PATHS[record.risk]
    record_path.parent.mkdir(parents=True, exist_ok=True)
    markdown_path.parent.mkdir(parents=True, exist_ok=True)
    record_path.write_text(serialize_record(record) + "\n", encoding="utf-8")
    markdown_path.write_text(render_record(record), encoding="utf-8")


def write_aggregate(
    repo_root: Path,
    records: Mapping[Risk, QualificationRecord],
) -> GateRecord:
    """Recompute the aggregate gate from complete records and write it."""
    gate = compute_gate(records, repo_root)
    if gate_schema_issues(gate_payload(gate)):
        raise ValueError("aggregate gate violates the gate schema")
    aggregate_path = repo_root / AGGREGATE_PATH
    gate_markdown_path = repo_root / GATE_MARKDOWN_PATH
    aggregate_path.parent.mkdir(parents=True, exist_ok=True)
    gate_markdown_path.parent.mkdir(parents=True, exist_ok=True)
    aggregate_path.write_bytes(serialize_gate(gate))
    gate_markdown_path.write_text(render_gate(gate), encoding="utf-8")
    return gate


def _refresh_aggregate(repo_root: Path) -> GateRecord | None:
    """Rewrite the aggregate when all four risk records exist on disk."""
    records = load_records(repo_root)
    if len(records) != len(REQUIRED_CRITERIA):
        missing = sorted(set(REQUIRED_CRITERIA) - set(records))
        print(
            "aggregate not written; missing qualification records: " + ", ".join(missing),
            file=sys.stderr,
        )
        return None
    return write_aggregate(repo_root, records)


def run_all(repo_root: Path, work_root: Path, *, original_env: Mapping[str, str]) -> int:
    """Run every risk once, write complete results, then refresh the aggregate.

    Each risk gets its own work root and runs independently: a failure never
    prevents the remaining risks from being measured and never discards a
    completed result.
    """
    failed = False
    for risk in LIVE_RUNNERS:
        try:
            record = run_one_live(risk, repo_root, work_root, original_env=original_env)
            write_risk_record(repo_root, record)
        except (OSError, TypeError, ValueError, RuntimeError):
            failed = True
            print(f"{risk} qualification did not complete", file=sys.stderr)
            continue
        print(f"{risk} qualification recorded: {record.decision}")
    gate = _refresh_aggregate(repo_root)
    if gate is not None:
        print(f"qualification aggregate recomputed: {gate.status}")
    return 2 if failed else 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Run the offline live Rust qualification measurements"
    )
    parser.add_argument("target", choices=(*LIVE_RUNNERS, "all"))
    parser.add_argument("--write", action="store_true", required=True)
    arguments = parser.parse_args(argv)
    repo_root = Path.cwd()
    # One unsanitized snapshot of the caller's environment feeds every runner;
    # each runner discovers tools from it before sanitizing its own children.
    original_env = dict(os.environ)
    try:
        if arguments.target == "all":
            return run_all(repo_root, repo_root / _WORK_ROOT, original_env=original_env)
        record = run_one_live(
            cast(Risk, arguments.target),
            repo_root,
            repo_root / _WORK_ROOT,
            original_env=original_env,
        )
        write_risk_record(repo_root, record)
    except (OSError, TypeError, ValueError, RuntimeError):
        print(f"{arguments.target} qualification failed", file=sys.stderr)
        return 2
    print(f"{arguments.target} qualification recorded: {record.decision}")
    _refresh_aggregate(repo_root)
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the CLI
    raise SystemExit(main())
