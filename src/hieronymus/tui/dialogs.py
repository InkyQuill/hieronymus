from __future__ import annotations

from collections.abc import Sequence

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widgets import Button, Input, Label, ListItem, ListView, Static, TextArea


class FilterDialog(ModalScreen[dict[str, str] | None]):
    BINDINGS = [Binding("escape", "cancel", "Cancel", show=False)]

    DEFAULT_CSS = """
    FilterDialog {
        align: center middle;
    }

    FilterDialog > Vertical {
        width: 64;
        height: auto;
        padding: 1 2;
        border: thick $accent;
        background: $surface;
    }
    """

    def __init__(
        self,
        values: dict[str, str] | None = None,
        *,
        show_type_filter: bool = True,
    ) -> None:
        super().__init__(id="filter-dialog")
        self.values = values or {}
        self.show_type_filter = show_type_filter

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Filter")
            yield Input(
                value=self.values.get("series_slug", ""),
                placeholder="series slug",
                id="filter-series-slug",
            )
            yield Input(
                value=self.values.get("status", ""), placeholder="status", id="filter-status"
            )
            if self.show_type_filter:
                yield Input(value=self.values.get("type", ""), placeholder="type", id="filter-type")
            yield Input(value=self.values.get("tags", ""), placeholder="tags", id="filter-tags")
            with Horizontal():
                yield Button("Apply", variant="primary", id="filter-apply")
                yield Button("Cancel", id="filter-cancel")

    def on_mount(self) -> None:
        self.query_one("#filter-series-slug", Input).focus()

    def action_cancel(self) -> None:
        self.dismiss(None)

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "filter-cancel":
            self.dismiss(None)
            return
        if event.button.id != "filter-apply":
            return
        filters = {
            "series_slug": self.query_one("#filter-series-slug", Input).value.strip(),
            "status": self.query_one("#filter-status", Input).value.strip(),
            "tags": self.query_one("#filter-tags", Input).value.strip(),
        }
        if self.show_type_filter:
            filters["type"] = self.query_one("#filter-type", Input).value.strip()
        self.dismiss(filters)


class EditDialog(ModalScreen[dict[str, str] | None]):
    BINDINGS = [Binding("escape", "cancel", "Cancel", show=False)]

    DEFAULT_CSS = """
    EditDialog {
        align: center middle;
    }

    EditDialog > Vertical {
        width: 80;
        height: 24;
        padding: 1 2;
        border: thick $accent;
        background: $surface;
    }

    EditDialog TextArea {
        height: 1fr;
    }
    """

    def __init__(self, *, title: str = "", text: str = "") -> None:
        super().__init__(id="edit-dialog")
        self.initial_title = title
        self.initial_text = text

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Edit")
            yield Input(value=self.initial_title, placeholder="title", id="edit-title")
            yield TextArea(self.initial_text, id="edit-text")
            with Horizontal():
                yield Button("Save", variant="primary", id="edit-save")
                yield Button("Cancel", id="edit-cancel")

    def on_mount(self) -> None:
        self.query_one("#edit-title", Input).focus()

    def action_cancel(self) -> None:
        self.dismiss(None)

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "edit-cancel":
            self.dismiss(None)
            return
        if event.button.id != "edit-save":
            return
        self.dismiss(
            {
                "title": self.query_one("#edit-title", Input).value.strip(),
                "text": self.query_one("#edit-text", TextArea).text.strip(),
            }
        )


class CommandDialog(ModalScreen[str | None]):
    BINDINGS = [Binding("escape", "cancel", "Cancel", show=False)]

    COMMANDS = (
        "edit",
        "delete",
        "approve",
        "reject",
        "deprecate",
        "reinforce",
        "decay",
        "inspect provenance",
    )

    DEFAULT_CSS = """
    CommandDialog {
        align: center middle;
    }

    CommandDialog > Vertical {
        width: 64;
        height: auto;
        max-height: 26;
        padding: 1 2;
        border: thick $accent;
        background: $surface;
    }
    """

    def __init__(self, commands: Sequence[str] | None = None) -> None:
        super().__init__(id="command-dialog")
        self.commands = tuple(self.COMMANDS if commands is None else commands)

    def compose(self) -> ComposeResult:
        with Vertical(id="command-dialog-content"):
            yield Static("Commands")
            yield ListView(
                *(
                    ListItem(Label(command), id=f"command-{index}")
                    for index, command in enumerate(self.commands)
                ),
                id="command-list",
            )
            with Horizontal():
                yield Button("Cancel", id="command-cancel")

    def action_cancel(self) -> None:
        self.dismiss(None)

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "command-cancel":
            self.dismiss(None)

    def on_list_view_selected(self, event: ListView.Selected) -> None:
        index = event.list_view.index
        if index is None:
            self.dismiss(None)
            return
        self.dismiss(self.commands[index])


class ConfirmDialog(ModalScreen[bool]):
    BINDINGS = [Binding("escape", "cancel", "Cancel", show=False)]

    DEFAULT_CSS = """
    ConfirmDialog {
        align: center middle;
    }

    ConfirmDialog > Vertical {
        width: 60;
        height: auto;
        padding: 1 2;
        border: thick $error;
        background: $surface;
    }
    """

    def __init__(self, message: str) -> None:
        super().__init__(id="confirm-dialog")
        self.message = message

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label(self.message, id="confirm-message")
            with Horizontal():
                yield Button("Confirm", variant="error", id="confirm-confirm")
                yield Button("Cancel", id="confirm-cancel")

    def action_cancel(self) -> None:
        self.dismiss(False)

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "confirm-confirm":
            self.dismiss(True)
            return
        if event.button.id == "confirm-cancel":
            self.dismiss(False)
