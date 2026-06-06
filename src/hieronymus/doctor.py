from __future__ import annotations

import sqlite3
from dataclasses import asdict, dataclass

from hieronymus.config import HieronymusConfig
from hieronymus.service_manager import ServiceManager


@dataclass(frozen=True)
class DoctorFinding:
    level: str
    code: str
    message: str
    autofixed: bool = False

    def to_json_dict(self) -> dict[str, object]:
        return asdict(self)


DoctorReport = dict[str, list[DoctorFinding]]


class Doctor:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config

    def run(self, autofix: bool = False) -> DoctorReport:
        report: DoctorReport = {"autofixed": [], "warnings": [], "errors": []}

        self._check_config_root(report, autofix=autofix)
        self._check_database(report)
        self._check_daemon(report)

        return report

    def _check_config_root(self, report: DoctorReport, *, autofix: bool) -> None:
        config_root = self.config.config_root
        if not config_root.exists():
            if autofix:
                config_root.mkdir(parents=True, exist_ok=True)
                report["autofixed"].append(
                    DoctorFinding(
                        level="info",
                        code="config-root-created",
                        message=f"Config root created: {config_root}",
                        autofixed=True,
                    )
                )
                return
            report["warnings"].append(
                DoctorFinding(
                    level="warning",
                    code="config-root-missing",
                    message=f"Config root does not exist: {config_root}",
                )
            )
            return

        if not config_root.is_dir():
            report["errors"].append(
                DoctorFinding(
                    level="error",
                    code="config-root-not-directory",
                    message=f"Config root is not a directory: {config_root}",
                )
            )

    def _check_database(self, report: DoctorReport) -> None:
        database_path = self.config.database_path
        if not database_path.exists():
            return

        try:
            with sqlite3.connect(database_path) as connection:
                connection.execute("select 1")
        except sqlite3.DatabaseError:
            report["errors"].append(
                DoctorFinding(
                    level="error",
                    code="database-unreadable",
                    message=f"Database file is unreadable: {database_path}",
                )
            )

    def _check_daemon(self, report: DoctorReport) -> None:
        status = ServiceManager(self.config).status()
        if status.get("running") is True:
            report["autofixed"].append(
                DoctorFinding(
                    level="info",
                    code="daemon-running",
                    message="Hieronymus daemon is reachable.",
                )
            )
            return

        report["warnings"].append(
            DoctorFinding(
                level="warning",
                code="daemon-not-running",
                message="Hieronymus daemon is not running.",
            )
        )


def report_to_json(report: DoctorReport) -> dict[str, list[dict[str, object]]]:
    return {
        section: [finding.to_json_dict() for finding in findings]
        for section, findings in report.items()
    }
