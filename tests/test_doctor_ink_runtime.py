from __future__ import annotations

from unittest.mock import patch

from hieronymus.doctor import Doctor, report_to_json


def test_doctor_reports_node_and_pnpm_runtime(config, monkeypatch) -> None:
    monkeypatch.setattr("hieronymus.doctor.shutil.which", lambda name: f"/usr/bin/{name}")

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        payload = report_to_json(Doctor(config).run())

    finding_codes = {finding["code"] for findings in payload.values() for finding in findings}

    assert "node-runtime-available" in finding_codes
    assert "pnpm-available" in finding_codes
