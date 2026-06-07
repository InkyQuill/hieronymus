from __future__ import annotations

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.service_manager import ServiceManager

ADMIN_VIEWS = [
    "Concepts",
    "Renderings",
    "Crystals",
    "Lessons",
    "Short-Term Sessions",
    "Dream Runs",
    "Proposals",
    "Audit Log",
]


class AdminStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def status_payload(self) -> dict[str, object]:
        with connect(self.config.database_path) as conn:
            counts = {
                "series": int(conn.execute("select count(*) from series").fetchone()[0]),
                "crystals": int(conn.execute("select count(*) from crystals").fetchone()[0]),
            }
        return {
            "tui": "available",
            "views": ADMIN_VIEWS,
            "counts": counts,
            "service": ServiceManager(self.config).status(),
        }
