from __future__ import annotations

from collections.abc import Mapping, Sequence

from rich.text import Text
from textual.widgets import DataTable, Static

from hieronymus.admin_models import AdminDetail, AdminRow


class ViewTabs(Static):
    can_focus = True

    def update_views(self, views: Sequence[str], active: str) -> None:
        text = Text()
        for index, view in enumerate(views, start=1):
            if index > 1:
                text.append("  ")
            style = "bold reverse" if view == active else "dim"
            text.append(f"{index} {view}", style=style)
        self.update(text)


class StatsBar(Static):
    def update_stats(self, stats: Mapping[str, int]) -> None:
        labels = (
            ("series", "series"),
            ("crystals", "crystals"),
            ("lessons", "lessons"),
            ("sessions", "sessions"),
            ("pending_proposals", "proposals"),
            ("audit_events", "audit"),
        )
        self.update("  ".join(f"{label} {stats[key]}" for key, label in labels))


class StatusPane(Static):
    def update_status(
        self,
        short_term_status: Mapping[str, object],
        dream_status: Mapping[str, object],
    ) -> None:
        pending = int(short_term_status.get("pending_count", 0))
        minimum = int(short_term_status.get("min_pending_short_term_memories", 0))
        maximum = int(short_term_status.get("max_pending_short_term_memories", 0))
        urgent = " urgent" if short_term_status.get("urgent") else ""
        short_term = f"Short-term pending {pending} / min {minimum} / max {maximum}{urgent}"

        if short_term_status.get("drain_in_progress"):
            completed = int(short_term_status.get("drain_completed", 0))
            total = int(short_term_status.get("drain_total", 0))
            remaining = int(short_term_status.get("drain_remaining", 0))
            progress = _format_percent(short_term_status.get("drain_progress", 0.0))
            short_term = (
                f"{short_term}  drain {completed}/{total} ({progress}) remaining {remaining}"
            )

        dream_parts = [f"Dream {dream_status.get('state', 'UNKNOWN')}"]
        phase = str(dream_status.get("current_phase") or "")
        if phase:
            dream_parts.append(f"phase {phase}")
        progress = float(dream_status.get("progress") or 0.0)
        if progress > 0.0:
            dream_parts.append(f"progress {_format_percent(progress)}")
        run_id = dream_status.get("run_id")
        if run_id is not None:
            dream_parts.append(f"run {run_id}")
        cycle_id = dream_status.get("cycle_id")
        if cycle_id is not None:
            dream_parts.append(f"cycle {cycle_id}")

        text = Text(short_term)
        text.append("\n")
        text.append("  ".join(dream_parts))
        self.update(text)


def _format_percent(value: object) -> str:
    return f"{float(value) * 100:.0f}%"


class AdminTable(DataTable):
    def load_rows(self, rows: Sequence[AdminRow]) -> None:
        self.clear(columns=True)
        self.cursor_type = "row"
        self.zebra_stripes = True
        self.add_columns("ID", "Kind", "Label", "Status", "Scope", "Quality")
        for row in rows:
            self.add_row(
                str(row.id),
                row.kind,
                row.label,
                row.status,
                row.scope,
                row.quality_label,
                key=str(row.id),
            )


class DetailPane(Static):
    def update_detail(self, detail: AdminDetail) -> None:
        text = Text(detail.title, style="bold")
        if detail.subtitle:
            text.append(f"\n{detail.subtitle}", style="dim")
        if detail.fields:
            text.append("\n\n")
            for index, (label, value) in enumerate(detail.fields):
                if index > 0:
                    text.append("\n")
                text.append(f"{label}: ", style="bold")
                text.append(value or "")
        if detail.body:
            text.append(f"\n\n{detail.body}")
        self.update(text)
