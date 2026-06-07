from __future__ import annotations

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal
from textual.screen import Screen
from textual.widgets import DataTable

from hieronymus.admin import ADMIN_VIEWS, AdminStore
from hieronymus.admin_models import AdminDetail, AdminRow, AdminSnapshot
from hieronymus.tui.dialogs import CommandDialog, ConfirmDialog, EditDialog, FilterDialog
from hieronymus.tui.widgets import AdminTable, DetailPane, StatsBar, ViewTabs

CRYSTAL_COMMANDS = (
    "edit",
    "delete",
    "deprecate",
    "reinforce",
    "decay",
    "inspect provenance",
)
PROPOSAL_COMMANDS = ("approve", "reject")


class ManagementScreen(Screen[None]):
    BINDINGS = [
        Binding(str(index), f"switch_view({index - 1})", view, show=False)
        for index, view in enumerate(ADMIN_VIEWS, start=1)
    ] + [
        Binding("r", "refresh", "Refresh"),
        Binding("f", "open_filter", "Filter"),
        Binding("e", "edit_selected", "Edit"),
        Binding("ctrl+p", "command_palette", "Commands", priority=True),
        Binding("a", "approve_selected", "Approve"),
        Binding("x", "reject_selected", "Reject"),
        Binding("+", "reinforce_selected", "Reinforce"),
        Binding("-", "decay_selected", "Decay"),
        Binding("d", "deprecate_selected", "Deprecate"),
        Binding("delete", "delete_selected", "Delete"),
        Binding("p", "inspect_provenance", "Provenance"),
        Binding("q", "quit", "Quit"),
    ]

    def __init__(self, store: AdminStore) -> None:
        super().__init__()
        self.store = store
        self.active_view = "Crystals"
        self.filters: dict[str, str] = {}

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
        if self.active_view == "Lessons":
            self.filters.pop("type", None)
        self.app.active_view = self.active_view
        self.refresh_view()

    def action_refresh(self) -> None:
        self.refresh_view()

    def action_open_filter(self) -> None:
        self.app.push_screen(
            FilterDialog(
                self._active_filters(),
                show_type_filter=self.active_view != "Lessons",
            ),
            self._apply_filters,
        )

    def action_command_palette(self) -> None:
        self.app.push_screen(
            CommandDialog(self._commands_for_active_view()), self._dispatch_command
        )

    def action_edit_selected(self) -> None:
        if self.active_view not in {"Crystals", "Lessons"}:
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        snapshot = self.store.snapshot(self.active_view, selected_id=selected_id)
        if snapshot.selected is None:
            return
        payload = self.store.crystal_edit_payload(int(selected_id))
        self.app.push_screen(
            EditDialog(title=payload.title, text=payload.text),
            lambda result: self._save_edit(selected_id, result),
        )

    def action_reinforce_selected(self) -> None:
        self._act_on_selected_crystal("reinforce")

    def action_decay_selected(self) -> None:
        self._act_on_selected_crystal("decay")

    def action_deprecate_selected(self) -> None:
        self._act_on_selected_crystal("deprecate")

    def action_delete_selected(self) -> None:
        if self.active_view not in {"Crystals", "Lessons"}:
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        self.app.push_screen(
            ConfirmDialog("Delete the selected crystal?"),
            lambda confirmed: self._delete_selected(selected_id, confirmed),
        )

    def action_approve_selected(self) -> None:
        if self.active_view != "Proposals":
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        self.store.approve_proposal(int(selected_id))
        self.refresh_view()

    def action_reject_selected(self) -> None:
        if self.active_view != "Proposals":
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        self.store.reject_proposal(int(selected_id), evidence="Rejected from admin TUI")
        self.refresh_view()

    def action_inspect_provenance(self) -> None:
        if self.active_view not in {"Crystals", "Lessons"}:
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        provenance = self.store.provenance_for_crystal(int(selected_id))
        body = "\n\n".join(source["text"] for source in provenance.sources)
        fields = tuple(
            (
                f"Source {source['id']}",
                "  ".join(
                    (
                        f"session {source['session_id']}",
                        source["source_role"],
                        source["kind"],
                        source["source_ref"],
                    )
                ),
            )
            for source in provenance.sources
        )
        self.query_one("#detail", DetailPane).update_detail(
            AdminDetail(
                title=f"Provenance: {provenance.title}",
                subtitle=f"{len(provenance.sources)} source(s)",
                fields=fields,
                body=body or "No provenance sources.",
            )
        )

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

    def refresh_view(self, selected_id: int | str | None = None) -> None:
        snapshot = self._snapshot(selected_id)
        stats = self.store.stats().as_dict()
        self.query_one("#view-tabs", ViewTabs).update_views(ADMIN_VIEWS, self.active_view)
        self.query_one("#stats", StatsBar).update_stats(stats)
        table = self.query_one("#entries", AdminTable)
        table.load_rows(snapshot.rows)
        if snapshot.selected is not None:
            selected_key = str(snapshot.selected.id)
            for index, row in enumerate(snapshot.rows):
                if str(row.id) == selected_key:
                    table.move_cursor(row=index)
                    break
        self.query_one("#detail", DetailPane).update_detail(snapshot.detail)

    def _update_detail_for_row_key(self, row_id: str | None) -> None:
        if row_id is None:
            return
        detail = self._snapshot(row_id).detail
        self.query_one("#detail", DetailPane).update_detail(detail)

    def _apply_filters(self, result: dict[str, str] | None) -> None:
        if result is not None:
            self.filters = {key: value for key, value in result.items() if value}
            if self.active_view == "Lessons":
                self.filters.pop("type", None)
        self.refresh_view()

    def _dispatch_command(self, command: str | None) -> None:
        if command is None:
            return
        if command == "edit":
            self.action_edit_selected()
        elif command == "delete":
            self.action_delete_selected()
        elif command == "approve":
            self.action_approve_selected()
        elif command == "reject":
            self.action_reject_selected()
        elif command == "deprecate":
            self.action_deprecate_selected()
        elif command == "reinforce":
            self.action_reinforce_selected()
        elif command == "decay":
            self.action_decay_selected()
        elif command == "inspect provenance":
            self.action_inspect_provenance()

    def _commands_for_active_view(self) -> tuple[str, ...]:
        if self.active_view in {"Crystals", "Lessons"}:
            return CRYSTAL_COMMANDS
        if self.active_view == "Proposals":
            return PROPOSAL_COMMANDS
        return ()

    def _save_edit(self, selected_id: int | str, result: dict[str, str] | None) -> None:
        if result is None:
            return
        self.store.edit_crystal(int(selected_id), title=result["title"], text=result["text"])
        self.refresh_view(selected_id)

    def _delete_selected(self, selected_id: int | str, confirmed: bool) -> None:
        if not confirmed:
            return
        self.store.delete_crystal(int(selected_id), evidence="Deleted from admin TUI")
        self.refresh_view()

    def _act_on_selected_crystal(self, action: str) -> None:
        if self.active_view not in {"Crystals", "Lessons"}:
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        crystal_id = int(selected_id)
        evidence = f"{action.capitalize()} from admin TUI"
        if action == "reinforce":
            self.store.reinforce_crystal(crystal_id, evidence=evidence)
        elif action == "decay":
            self.store.decay_crystal(crystal_id, evidence=evidence)
        elif action == "deprecate":
            self.store.deprecate_crystal(crystal_id, evidence=evidence)
        self.refresh_view(selected_id)

    def _selected_row_id(self) -> int | str | None:
        table = self.query_one("#entries", AdminTable)
        if table.row_count:
            cursor_row = table.cursor_row
            if table.is_valid_row_index(cursor_row):
                return table.ordered_rows[cursor_row].key.value
        snapshot = self._snapshot()
        return snapshot.selected.id if snapshot.selected is not None else None

    def _snapshot(self, selected_id: int | str | None = None) -> AdminSnapshot:
        filter_values = self._active_filters()
        if not filter_values:
            return self.store.snapshot(self.active_view, selected_id=selected_id)

        crystal_type = filter_values.get("type") or None
        if self.active_view == "Lessons":
            crystal_type = "lesson"
        rows = self.store.list_crystals(
            series_slug=filter_values.get("series_slug") or None,
            crystal_type=crystal_type,
            status=filter_values.get("status") or None,
            tags=tuple(
                tag.strip() for tag in filter_values.get("tags", "").split(",") if tag.strip()
            ),
        )
        selected = self._select_row(rows, selected_id)
        detail = (
            self.store.snapshot(self.active_view, selected_id=selected.id).detail
            if selected is not None
            else AdminDetail("No rows", "", "No rows match the current filters.")
        )
        labels = [f"{key}: {value}" for key, value in sorted(filter_values.items())]
        return AdminSnapshot(self.active_view, rows, selected, detail, filters=labels)

    def _active_filters(self) -> dict[str, str]:
        if self.active_view not in {"Crystals", "Lessons"}:
            return {}
        if self.active_view == "Lessons":
            return {key: value for key, value in self.filters.items() if key != "type"}
        return dict(self.filters)

    def _select_row(
        self,
        rows: list[AdminRow],
        selected_id: int | str | None,
    ) -> AdminRow | None:
        if selected_id is not None:
            selected_key = str(selected_id)
            for row in rows:
                if str(row.id) == selected_key:
                    return row
        return rows[0] if rows else None
