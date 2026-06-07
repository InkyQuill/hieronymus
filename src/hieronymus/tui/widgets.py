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
