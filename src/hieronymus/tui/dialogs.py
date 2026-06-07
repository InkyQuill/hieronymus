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
                value=self.values.get("language_pair", ""),
                placeholder="language pair",
                id="filter-language-pair",
            )
            yield Input(
                value=self.values.get("status", ""), placeholder="status", id="filter-status"
            )
            if self.show_type_filter:
                yield Input(value=self.values.get("type", ""), placeholder="type", id="filter-type")
            yield Input(
                value=self.values.get("confidence", ""),
                placeholder="minimum confidence percent",
                id="filter-confidence",
            )
            yield Input(
                value=self.values.get("strength", ""),
                placeholder="minimum strength percent",
                id="filter-strength",
            )
            yield Input(value=self.values.get("cycle", ""), placeholder="cycle", id="filter-cycle")
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
            "language_pair": self.query_one("#filter-language-pair", Input).value.strip(),
            "status": self.query_one("#filter-status", Input).value.strip(),
            "confidence": self.query_one("#filter-confidence", Input).value.strip(),
            "strength": self.query_one("#filter-strength", Input).value.strip(),
            "cycle": self.query_one("#filter-cycle", Input).value.strip(),
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


class FormDialog(ModalScreen[dict[str, str] | None]):
    BINDINGS = [Binding("escape", "cancel", "Cancel", show=False)]

    DEFAULT_CSS = """
    FormDialog {
        align: center middle;
    }

    FormDialog > Vertical {
        width: 76;
        height: auto;
        max-height: 28;
        padding: 1 2;
        border: thick $accent;
        background: $surface;
    }

    FormDialog TextArea {
        height: 8;
    }
    """

    def __init__(
        self,
        title: str,
        fields: Sequence[tuple[str, str, str]],
        *,
        multiline: frozenset[str] = frozenset(),
    ) -> None:
        super().__init__(id="form-dialog")
        self.form_title = title
        self.fields = tuple(fields)
        self.multiline = multiline

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label(self.form_title)
            for field_id, placeholder, value in self.fields:
                if field_id in self.multiline:
                    yield TextArea(value, id=f"form-{field_id}")
                else:
                    yield Input(value=value, placeholder=placeholder, id=f"form-{field_id}")
            with Horizontal():
                yield Button("Apply", variant="primary", id="form-apply")
                yield Button("Cancel", id="form-cancel")

    def on_mount(self) -> None:
        first_field = self.fields[0][0] if self.fields else ""
        if first_field:
            self.query_one(f"#form-{first_field}").focus()

    def action_cancel(self) -> None:
        self.dismiss(None)

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "form-cancel":
            self.dismiss(None)
            return
        if event.button.id != "form-apply":
            return
        result = {}
        for field_id, _, _ in self.fields:
            widget = self.query_one(f"#form-{field_id}")
            if isinstance(widget, TextArea):
                result[field_id] = widget.text.strip()
            elif isinstance(widget, Input):
                result[field_id] = widget.value.strip()
        self.dismiss(result)


class CommandDialog(ModalScreen[str | None]):
    BINDINGS = [Binding("escape", "cancel", "Cancel", show=False)]

    COMMANDS = (
        "add",
        "edit",
        "delete",
        "merge",
        "split",
        "approve",
        "reject",
        "deprecate",
        "supersede",
        "reinforce",
        "decay",
        "promote local lesson",
        "activate global lesson",
        "inspect provenance",
        "inspect recall reason",
        "run manual dreaming",
        "review dream outputs",
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
