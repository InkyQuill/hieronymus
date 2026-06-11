from __future__ import annotations

import subprocess
from unittest.mock import patch

from hieronymus.doctor import Doctor, report_to_json


def test_doctor_reports_bun_runtime(config, monkeypatch) -> None:
    monkeypatch.setattr("hieronymus.doctor.shutil.which", lambda name: f"/usr/bin/{name}")
    monkeypatch.setattr(
        "hieronymus.doctor.subprocess.run",
        lambda *args, **kwargs: subprocess.CompletedProcess(args[0], 0, stdout="1.3.14"),
    )

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        payload = report_to_json(Doctor(config).run())

    finding_codes = {finding["code"] for findings in payload.values() for finding in findings}

    assert "bun-runtime-available" in finding_codes


def test_doctor_warns_when_bun_runtime_missing(config, monkeypatch) -> None:
    monkeypatch.setattr("hieronymus.doctor.shutil.which", lambda name: None)

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        payload = report_to_json(Doctor(config).run())

    warning_codes = {finding["code"] for finding in payload["warnings"]}
    error_codes = {finding["code"] for finding in payload["errors"]}

    assert "bun-runtime-missing" in warning_codes
    assert "bun-runtime-missing" not in error_codes


def test_doctor_warns_when_bun_runtime_is_too_old(config, monkeypatch) -> None:
    monkeypatch.setattr("hieronymus.doctor.shutil.which", lambda name: f"/usr/bin/{name}")
    monkeypatch.setattr(
        "hieronymus.doctor.subprocess.run",
        lambda *args, **kwargs: subprocess.CompletedProcess(args[0], 0, stdout="1.2.0"),
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

    assert "bun-runtime-too-old" in warning_codes
    assert "bun-runtime-too-old" not in error_codes
    assert "bun-runtime-available" not in info_codes


def test_doctor_warns_when_bun_version_check_times_out(config, monkeypatch) -> None:
    monkeypatch.setattr("hieronymus.doctor.shutil.which", lambda name: f"/usr/bin/{name}")

    def timeout(*args, **kwargs):
        raise subprocess.TimeoutExpired(cmd=args[0], timeout=kwargs["timeout"])

    monkeypatch.setattr("hieronymus.doctor.subprocess.run", timeout)

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        payload = report_to_json(Doctor(config).run())

    warning_codes = {finding["code"] for finding in payload["warnings"]}
    error_codes = {finding["code"] for finding in payload["errors"]}
    info_codes = {finding["code"] for finding in payload["autofixed"]}

    assert "bun-runtime-unusable" in warning_codes
    assert "bun-runtime-unusable" not in error_codes
    assert "bun-runtime-available" not in info_codes
