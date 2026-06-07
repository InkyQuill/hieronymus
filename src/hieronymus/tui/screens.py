from __future__ import annotations

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal
from textual.screen import Screen
from textual.widgets import DataTable

from hieronymus.admin import ADMIN_VIEWS, AdminStore
from hieronymus.tui.widgets import AdminTable, DetailPane, StatsBar, ViewTabs


class ManagementScreen(Screen[None]):
    BINDINGS = [
        Binding(str(index), f"switch_view({index - 1})", view, show=False)
        for index, view in enumerate(ADMIN_VIEWS, start=1)
    ] + [
        Binding("r", "refresh", "Refresh"),
        Binding("q", "quit", "Quit"),
    ]

    def __init__(self, store: AdminStore) -> None:
        super().__init__()
        self.store = store
        self.active_view = "Crystals"

    def compose(self) -> ComposeResult:
        yield ViewTabs(id="view-tabs")
        yield StatsBar(id="stats")
        with Horizontal(id="workspace"):
            yield AdminTable(id="entries")
            yield DetailPane(id="detail")

    def on_mount(self) -> None:
        self.query_one("#view-tabs", ViewTabs).focus()
        self.refresh_view()

    def action_switch_view(self, index: int) -> None:
        views = list(ADMIN_VIEWS)
        if index < 0 or index >= len(views):
            return
        self.active_view = views[index]
        self.app.active_view = self.active_view
        self.refresh_view()

    def action_refresh(self) -> None:
        self.refresh_view()

    def action_quit(self) -> None:
        self.app.exit()

    def on_data_table_row_highlighted(self, message: DataTable.RowHighlighted) -> None:
        if message.data_table is not self.query_one("#entries", AdminTable):
            return
        self._update_detail_for_row_key(message.row_key.value)

    def on_data_table_row_selected(self, message: DataTable.RowSelected) -> None:
        if message.data_table is not self.query_one("#entries", AdminTable):
            return
        self._update_detail_for_row_key(message.row_key.value)

    def refresh_view(self) -> None:
        snapshot = self.store.snapshot(self.active_view)
        stats = self.store.stats().as_dict()
        self.query_one("#view-tabs", ViewTabs).update_views(ADMIN_VIEWS, self.active_view)
        self.query_one("#stats", StatsBar).update_stats(stats)
        self.query_one("#entries", AdminTable).load_rows(snapshot.rows)
        self.query_one("#detail", DetailPane).update_detail(snapshot.detail)

    def _update_detail_for_row_key(self, row_id: str | None) -> None:
        if row_id is None:
            return
        detail = self.store.snapshot(self.active_view, selected_id=row_id).detail
        self.query_one("#detail", DetailPane).update_detail(detail)
