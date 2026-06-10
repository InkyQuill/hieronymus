from __future__ import annotations

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal
from textual.screen import Screen
from textual.widgets import DataTable

from hieronymus.admin import ADMIN_VIEWS, AdminStore
from hieronymus.admin_models import AdminDetail, AdminRow, AdminSnapshot
from hieronymus.tui.dialogs import (
    CommandDialog,
    ConfirmDialog,
    EditDialog,
    FilterDialog,
    FormDialog,
)
from hieronymus.tui.widgets import AdminTable, DetailPane, StatsBar, StatusPane, ViewTabs

CRYSTAL_COMMANDS = (
    "add",
    "edit",
    "delete",
    "merge",
    "split",
    "deprecate",
    "supersede",
    "reinforce",
    "decay",
    "inspect provenance",
    "inspect recall reason",
)
LESSON_COMMANDS = CRYSTAL_COMMANDS + ("promote local lesson", "activate global lesson")
PROPOSAL_COMMANDS = ("approve", "reject")
DREAM_COMMANDS = ("run manual dreaming", "review dream outputs")


class ManagementScreen(Screen[None]):
    BINDINGS = [
        Binding(str(index), f"switch_view({index - 1})", view, show=False)
        for index, view in enumerate(ADMIN_VIEWS, start=1)
    ] + [
        Binding("r", "refresh", "Refresh"),
        Binding("f", "open_filter", "Filter"),
        Binding("/", "open_filter", "Filter", show=False),
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
        yield StatusPane(id="status-pane")
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
        if not self._filter_enabled():
            return
        self.app.push_screen(
            FilterDialog(
                self._active_filters(),
                show_type_filter=self.active_view not in {"Lessons", "Proposals"},
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

    def action_add_selected(self) -> None:
        if self.active_view not in {"Crystals", "Lessons"}:
            return
        crystal_type = "lesson" if self.active_view == "Lessons" else "concept"
        self.app.push_screen(
            FormDialog(
                "Add crystal",
                (
                    ("series_slug", "series slug", self.filters.get("series_slug", "")),
                    ("source_language", "source language", "ja"),
                    ("target_language", "target language", "ru"),
                    ("type", "type", crystal_type),
                    ("title", "title", ""),
                    ("text", "text", ""),
                    ("tags", "tags", self.filters.get("tags", "")),
                ),
                multiline=frozenset({"text"}),
            ),
            self._add_crystal,
        )

    def action_merge_selected(self) -> None:
        if self.active_view not in {"Crystals", "Lessons"}:
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        self.app.push_screen(
            FormDialog(
                "Merge crystals",
                (
                    ("ids", "comma-separated crystal ids", str(selected_id)),
                    ("title", "title", ""),
                    ("text", "text", ""),
                ),
                multiline=frozenset({"text"}),
            ),
            self._merge_crystals,
        )

    def action_split_selected(self) -> None:
        if self.active_view not in {"Crystals", "Lessons"}:
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        self.app.push_screen(
            FormDialog(
                "Split crystal",
                (
                    ("part_one_title", "first title", ""),
                    ("part_one_text", "first text", ""),
                    ("part_two_title", "second title", ""),
                    ("part_two_text", "second text", ""),
                ),
                multiline=frozenset({"part_one_text", "part_two_text"}),
            ),
            lambda result: self._split_crystal(selected_id, result),
        )

    def action_supersede_selected(self) -> None:
        if self.active_view not in {"Crystals", "Lessons"}:
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        self.app.push_screen(
            FormDialog(
                "Supersede crystal",
                (("replacement_id", "replacement crystal id", ""),),
            ),
            lambda result: self._supersede_crystal(selected_id, result),
        )

    def action_promote_selected(self) -> None:
        if self.active_view != "Lessons":
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        promoted_id = self.store.promote_local_lesson(
            int(selected_id),
            evidence="Promoted from admin TUI",
        )
        self.refresh_view(promoted_id)

    def action_activate_selected(self) -> None:
        if self.active_view != "Lessons":
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        self.store.activate_global_lesson(int(selected_id), evidence="Activated from admin TUI")
        self.refresh_view(selected_id)

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

    def action_inspect_recall_reason(self) -> None:
        if self.active_view not in {"Crystals", "Lessons"}:
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        reasons = self.store.recall_reasons_for_crystal(int(selected_id))
        fields = tuple(
            (
                f"Session {reason['session_id']}",
                f"rank {reason['rank']} / score {reason['score']}",
            )
            for reason in reasons
        )
        body = (
            "\n\n".join(f"{reason['query']}\n{reason['reason']}" for reason in reasons)
            or "No recall activations recorded."
        )
        self.query_one("#detail", DetailPane).update_detail(
            AdminDetail(
                title=f"Recall reasons: {selected_id}",
                subtitle=f"{len(reasons)} activation(s)",
                fields=fields,
                body=body,
            )
        )

    def action_run_manual_dreaming(self) -> None:
        run = self.store.run_manual_dreaming()
        self.active_view = "Dream Runs"
        self.app.active_view = self.active_view
        self.refresh_view(run.id)

    def action_review_dream_outputs(self) -> None:
        if self.active_view != "Dream Runs":
            return
        selected_id = self._selected_row_id()
        if selected_id is None:
            return
        review = self.store.dream_review(int(selected_id))
        fields = (
            ("Source sessions", ", ".join(str(value) for value in review.source_sessions)),
            ("Consumed memories", str(len(review.consumed_memories))),
            ("Created crystals", str(len(review.created_crystals))),
            ("Updated crystals", str(len(review.updated_crystals))),
            ("Decayed crystals", str(len(review.decayed_crystals))),
            ("Strict proposals", str(len(review.strict_proposals))),
            ("Failed outputs", str(len(review.failed_outputs))),
            ("Validation errors", str(len(review.validation_errors))),
        )
        sections = (
            ("Consumed", review.consumed_memories),
            ("Created", review.created_crystals),
            ("Updated", review.updated_crystals),
            ("Decayed", review.decayed_crystals),
            ("Proposals", review.strict_proposals),
            ("Failures", review.failed_outputs),
            ("Validation", review.validation_errors),
        )
        body = "\n\n".join(f"{title}\n" + "\n".join(values) for title, values in sections if values)
        self.query_one("#detail", DetailPane).update_detail(
            AdminDetail(
                title=f"Dream review: {review.run_id}",
                subtitle="Cycle output review",
                fields=fields,
                body=body or "No dream outputs recorded.",
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
        status_payload = self.store.dashboard_status_payload()
        self.query_one("#view-tabs", ViewTabs).update_views(ADMIN_VIEWS, self.active_view)
        self.query_one("#stats", StatsBar).update_stats(stats)
        self.query_one("#status-pane", StatusPane).update_status(
            status_payload["short_term_status"],
            status_payload["dream_status"],
        )
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
        if command == "add":
            self.action_add_selected()
        elif command == "edit":
            self.action_edit_selected()
        elif command == "delete":
            self.action_delete_selected()
        elif command == "merge":
            self.action_merge_selected()
        elif command == "split":
            self.action_split_selected()
        elif command == "approve":
            self.action_approve_selected()
        elif command == "reject":
            self.action_reject_selected()
        elif command == "deprecate":
            self.action_deprecate_selected()
        elif command == "supersede":
            self.action_supersede_selected()
        elif command == "reinforce":
            self.action_reinforce_selected()
        elif command == "decay":
            self.action_decay_selected()
        elif command == "promote local lesson":
            self.action_promote_selected()
        elif command == "activate global lesson":
            self.action_activate_selected()
        elif command == "inspect provenance":
            self.action_inspect_provenance()
        elif command == "inspect recall reason":
            self.action_inspect_recall_reason()
        elif command == "run manual dreaming":
            self.action_run_manual_dreaming()
        elif command == "review dream outputs":
            self.action_review_dream_outputs()

    def _commands_for_active_view(self) -> tuple[str, ...]:
        if self.active_view == "Crystals":
            return CRYSTAL_COMMANDS
        if self.active_view == "Lessons":
            return LESSON_COMMANDS
        if self.active_view == "Proposals":
            return PROPOSAL_COMMANDS
        if self.active_view == "Dream Runs":
            return DREAM_COMMANDS
        return ()

    def _add_crystal(self, result: dict[str, str] | None) -> None:
        if result is None:
            return
        crystal_id = self.store.add_crystal(
            series_slug=result["series_slug"],
            source_language=result["source_language"],
            target_language=result["target_language"],
            crystal_type=result["type"],
            title=result["title"],
            text=result["text"],
            tags=tuple(tag.strip() for tag in result.get("tags", "").split(",") if tag.strip()),
        )
        self.refresh_view(crystal_id)

    def _save_edit(self, selected_id: int | str, result: dict[str, str] | None) -> None:
        if result is None:
            return
        self.store.edit_crystal(int(selected_id), title=result["title"], text=result["text"])
        self.refresh_view(selected_id)

    def _merge_crystals(self, result: dict[str, str] | None) -> None:
        if result is None:
            return
        crystal_ids = [int(value.strip()) for value in result["ids"].split(",") if value.strip()]
        merged_id = self.store.merge_crystals(
            crystal_ids,
            title=result["title"],
            text=result["text"],
        )
        self.refresh_view(merged_id)

    def _split_crystal(self, selected_id: int | str, result: dict[str, str] | None) -> None:
        if result is None:
            return
        new_ids = self.store.split_crystal(
            int(selected_id),
            parts=[
                {"title": result["part_one_title"], "text": result["part_one_text"]},
                {"title": result["part_two_title"], "text": result["part_two_text"]},
            ],
        )
        self.refresh_view(new_ids[0] if new_ids else None)

    def _supersede_crystal(self, selected_id: int | str, result: dict[str, str] | None) -> None:
        if result is None:
            return
        self.store.supersede_crystal(
            int(selected_id),
            replacement_id=int(result["replacement_id"]),
            evidence="Superseded from admin TUI",
        )
        self.refresh_view()

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

        if self.active_view in {"Crystals", "Lessons"}:
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
        else:
            rows = self.store.snapshot(self.active_view).rows
        rows = self._filter_rows(rows, filter_values)
        return self._filtered_snapshot(rows, filter_values, selected_id)

    def _active_filters(self) -> dict[str, str]:
        if not self._filter_enabled():
            return {}
        if self.active_view == "Lessons":
            return {key: value for key, value in self.filters.items() if key != "type"}
        return dict(self.filters)

    def _filter_enabled(self) -> bool:
        return self.active_view in {
            "Concepts",
            "Renderings",
            "Crystals",
            "Lessons",
            "Short-Term Sessions",
            "Dream Runs",
            "Proposals",
            "Audit Log",
        }

    def _filter_rows(self, rows: list[AdminRow], filter_values: dict[str, str]) -> list[AdminRow]:
        series_slug = filter_values.get("series_slug")
        status = filter_values.get("status")
        row_type = filter_values.get("type")
        language_pair = filter_values.get("language_pair")
        cycle = filter_values.get("cycle")
        confidence = _optional_percent(filter_values.get("confidence"))
        strength = _optional_percent(filter_values.get("strength"))
        tags = tuple(tag.strip() for tag in filter_values.get("tags", "").split(",") if tag.strip())
        filtered = []
        for row in rows:
            if series_slug and row.scope != series_slug:
                continue
            if status and row.status != status:
                continue
            if row_type and row.kind != row_type:
                continue
            if language_pair and row.language_pair != language_pair:
                continue
            if cycle and cycle not in row.label and cycle not in row.quality_label:
                continue
            if confidence is not None and _quality_percent(row.quality_label, "conf") < confidence:
                continue
            if strength is not None and _quality_percent(row.quality_label, "str") < strength:
                continue
            if tags and not set(tags).issubset(row.tags):
                continue
            filtered.append(row)
        return filtered

    def _filtered_snapshot(
        self,
        rows: list[AdminRow],
        filter_values: dict[str, str],
        selected_id: int | str | None,
    ) -> AdminSnapshot:
        selected = self._select_row(rows, selected_id)
        detail = (
            self.store.snapshot(self.active_view, selected_id=selected.id).detail
            if selected is not None
            else AdminDetail("No rows", "", "No rows match the current filters.")
        )
        labels = [f"{key}: {value}" for key, value in sorted(filter_values.items())]
        return AdminSnapshot(self.active_view, rows, selected, detail, filters=labels)

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


def _optional_percent(value: str | None) -> float | None:
    if not value:
        return None
    try:
        parsed = float(value.strip().removesuffix("%"))
    except ValueError:
        return None
    return parsed / 100 if parsed > 1 else parsed


def _quality_percent(label: str, marker: str) -> float:
    for part in label.split("/"):
        cleaned = part.strip()
        if marker not in cleaned:
            continue
        value = cleaned.split(maxsplit=1)[0].removesuffix("%")
        try:
            return float(value) / 100
        except ValueError:
            return 0.0
    return 0.0
