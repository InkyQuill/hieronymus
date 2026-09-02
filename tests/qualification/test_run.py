"""Dispatcher tests for the single opt-in live qualification entry point."""

from __future__ import annotations

import json
import sys
from collections.abc import Callable, Mapping
from pathlib import Path
from typing import cast

import pytest

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from factories import make_record, seed_fingerprint_inputs  # noqa: E402

from tools.qualification import run  # noqa: E402
from tools.qualification.model import (  # noqa: E402
    REQUIRED_CRITERIA,
    QualificationRecord,
    Risk,
    load_record,
)


def test_dispatcher_passes_original_environment_to_runner(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    sentinel_env = {"HIERONYMUS_QUALIFICATION_LIVE": "1", "DISPATCH_SENTINEL": "value"}
    sentinel_record = cast(QualificationRecord, object())
    received: dict[str, object] = {}

    def fake_runner(
        repo_root: Path,
        work_root: Path,
        *,
        original_env: Mapping[str, str],
    ) -> QualificationRecord:
        received["repo_root"] = repo_root
        received["work_root"] = work_root
        received["original_env"] = original_env
        return sentinel_record

    monkeypatch.setitem(run.LIVE_RUNNERS, "mcp-transport", fake_runner)
    repo_root = ROOT
    work_root = ROOT / "qualification/.artifacts/work"

    record = run.run_one_live(
        "mcp-transport",
        repo_root,
        work_root,
        original_env=sentinel_env,
    )

    assert record is sentinel_record
    assert received["original_env"] is sentinel_env
    assert received["repo_root"] == repo_root
    assert received["work_root"] == work_root / "mcp-transport"
    assert received["work_root"] != work_root


def test_dispatcher_requires_live_opt_in(monkeypatch: pytest.MonkeyPatch) -> None:
    def forbidden(*args: object, **kwargs: object) -> QualificationRecord:
        del args, kwargs
        raise AssertionError("no runner may be dispatched without the live opt-in")

    monkeypatch.setitem(run.LIVE_RUNNERS, "semantic-native", forbidden)
    with pytest.raises(RuntimeError, match="HIERONYMUS_QUALIFICATION_LIVE=1"):
        run.run_one_live(
            "semantic-native",
            ROOT,
            ROOT / "qualification/.artifacts/work",
            original_env={"HOME": "/sensitive"},
        )


def test_run_all_writes_complete_records_and_blocked_aggregate(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    for risk in REQUIRED_CRITERIA:
        seed_fingerprint_inputs(tmp_path, risk)
    for risk in REQUIRED_CRITERIA:
        run.write_risk_record(tmp_path, make_record(tmp_path, risk, review_status="accepted"))
    calls: list[tuple[Risk, Path, Mapping[str, str]]] = []

    def fake_runner(
        risk: Risk,
        record: QualificationRecord,
    ) -> Callable[[Path, Path, Mapping[str, str]], QualificationRecord]:
        def _runner(
            repo_root: Path,
            work_root: Path,
            *,
            original_env: Mapping[str, str],
        ) -> QualificationRecord:
            del repo_root
            calls.append((risk, work_root, original_env))
            if risk == "frontend-embedding":
                raise RuntimeError("frontend measurement failed")
            return record

        return _runner

    for risk in REQUIRED_CRITERIA:
        pending = make_record(tmp_path, risk, review_status="pending")
        monkeypatch.setitem(run.LIVE_RUNNERS, risk, fake_runner(risk, pending))
    monkeypatch.setenv("HIERONYMUS_QUALIFICATION_LIVE", "1")
    monkeypatch.chdir(tmp_path)

    exit_code = run.main(["all", "--write"])

    assert exit_code == 2
    assert [risk for risk, _work, _env in calls] == list(REQUIRED_CRITERIA)
    assert len({work for _risk, work, _env in calls}) == len(REQUIRED_CRITERIA)
    assert all(
        work == tmp_path / "qualification/.artifacts/work" / risk for risk, work, _env in calls
    )
    assert all(
        env is calls[0][2] and env["HIERONYMUS_QUALIFICATION_LIVE"] == "1"
        for _risk, _work, env in calls
    )
    for risk in REQUIRED_CRITERIA:
        record = load_record(tmp_path / f"qualification/records/{risk}.json")
        expected = "accepted" if risk == "frontend-embedding" else "pending"
        assert record.review.status == expected
    aggregate = json.loads(
        (tmp_path / "qualification/records/aggregate.json").read_bytes().decode("utf-8")
    )
    assert aggregate["status"] == "blocked"
    assert aggregate["release_mode"] is None
    assert set(aggregate["issues"]) == {
        "mcp-transport: review is not accepted",
        "semantic-native: review is not accepted",
        "legacy-database-import: review is not accepted",
    }
    assert (tmp_path / "docs/qualification/rust/gate.md").is_file()
