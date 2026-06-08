from __future__ import annotations

import subprocess
from unittest.mock import patch

from hieronymus.doctor import Doctor, report_to_json


def test_doctor_reports_node_and_pnpm_runtime(config, monkeypatch) -> None:
    monkeypatch.setattr("hieronymus.doctor.shutil.which", lambda name: f"/usr/bin/{name}")
    monkeypatch.setattr(
        "hieronymus.doctor.subprocess.run",
        lambda *args, **kwargs: subprocess.CompletedProcess(args[0], 0, stdout="v22.17.0"),
    )

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        payload = report_to_json(Doctor(config).run())

    finding_codes = {finding["code"] for findings in payload.values() for finding in findings}

    assert "node-runtime-available" in finding_codes
    assert "pnpm-available" in finding_codes


def test_doctor_warns_when_node_and_pnpm_runtime_missing(config, monkeypatch) -> None:
    monkeypatch.setattr("hieronymus.doctor.shutil.which", lambda name: None)

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        payload = report_to_json(Doctor(config).run())

    warning_codes = {finding["code"] for finding in payload["warnings"]}
    error_codes = {finding["code"] for finding in payload["errors"]}

    assert "node-runtime-missing" in warning_codes
    assert "pnpm-missing" in warning_codes
    assert "node-runtime-missing" not in error_codes
    assert "pnpm-missing" not in error_codes


def test_doctor_warns_when_node_runtime_is_too_old(config, monkeypatch) -> None:
    monkeypatch.setattr("hieronymus.doctor.shutil.which", lambda name: f"/usr/bin/{name}")
    monkeypatch.setattr(
        "hieronymus.doctor.subprocess.run",
        lambda *args, **kwargs: subprocess.CompletedProcess(args[0], 0, stdout="v21.7.3"),
    )

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        payload = report_to_json(Doctor(config).run())

    warning_codes = {finding["code"] for finding in payload["warnings"]}
    error_codes = {finding["code"] for finding in payload["errors"]}
    info_codes = {finding["code"] for finding in payload["autofixed"]}

    assert "node-runtime-too-old" in warning_codes
    assert "node-runtime-too-old" not in error_codes
    assert "node-runtime-available" not in info_codes
    assert "pnpm-available" in info_codes
