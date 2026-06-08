from __future__ import annotations

from dataclasses import replace

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import Screen
from textual.widgets import DataTable, Footer, Input, Static

from hieronymus.config import HieronymusConfig
from hieronymus.dream_providers import ProviderRegistry
from hieronymus.secrets import env_value_exists
from hieronymus.settings import (
    ProviderSettings,
    SettingsError,
    load_settings,
    save_settings,
)
from hieronymus.tui.config_state import (
    ConfigDraft,
    apply_dreaming_form,
    apply_provider_form,
    field_value,
    validate_draft,
    yes_no,
)


class ConfigScreen(Screen[None]):
    BINDINGS = [
        Binding("1", "set_active('deterministic')", "Deterministic"),
        Binding("2", "set_active('openai')", "OpenAI"),
        Binding("3", "set_active('gemini')", "Gemini"),
        Binding("4", "set_active('anthropic')", "Anthropic"),
        Binding("s", "save", "Save"),
        Binding("r", "reload", "Reload"),
        Binding("c", "check_selected", "Check"),
        Binding("q", "quit", "Quit"),
    ]

    def __init__(self, config: HieronymusConfig) -> None:
        super().__init__()
        self.config = config
        settings = load_settings(config)
        self.draft = ConfigDraft(saved=settings, edited=settings)
        self._syncing_form = False
        self.registry = ProviderRegistry()

    def compose(self) -> ComposeResult:
        yield Static("Providers", id="config-title")
        with Horizontal(id="workspace"):
            yield DataTable(id="config-table")
            with Vertical(id="config-form"):
                yield Static("", id="config-detail")
                yield Input(id="provider-enabled")
                yield Input(id="provider-model")
                yield Input(id="provider-api-key-env")
                yield Input(id="provider-base-url")
                yield Input(id="provider-timeout-seconds")
                yield Input(id="dreaming-active-provider")
                yield Input(id="dreaming-autostart-enabled")
                yield Input(id="dreaming-min-interval-minutes")
                yield Input(id="dreaming-new-short-term-memory-threshold")
                yield Input(id="dreaming-max-cycles-per-autostart")
        yield Footer()

    def on_mount(self) -> None:
        table = self.query_one("#config-table", DataTable)
        table.add_columns(
            ("provider", "provider"),
            ("active", "active"),
            ("enabled", "enabled"),
            ("model", "model"),
            ("key env", "key_env"),
            ("configured", "configured"),
            ("error", "error"),
        )
        table.focus()
        self._refresh()

    def action_set_active(self, name: str) -> None:
        if not self._apply_form_to_draft():
            return
        provider = self.draft.edited.providers.get(name, ProviderSettings())
        if not provider.enabled:
            self.draft = self.draft.with_edited(
                self.draft.edited.with_provider(name, replace(provider, enabled=True))
            )
        self.draft = self.draft.with_edited(
            self.draft.edited.with_dreaming(
                replace(self.draft.edited.dreaming, active_provider=name)
            )
        )
        self._refresh(selected_provider=name)

    def action_save(self) -> None:
        if not self._apply_form_to_draft():
            return
        errors = validate_draft(self.draft.edited)
        if errors:
            self.draft = self.draft.with_errors(errors)
            self._update_detail(self._selected_provider())
            return
        save_settings(self.config, self.draft.edited)
        saved = load_settings(self.config)
        self.draft = ConfigDraft(saved=saved, edited=saved)
        self._refresh()

    def action_reload(self) -> None:
        selected_provider = self._selected_provider()
        settings = load_settings(self.config)
        self.draft = ConfigDraft(saved=settings, edited=settings)
        self._refresh(selected_provider=selected_provider)

    def action_check_selected(self) -> None:
        if not self._apply_form_to_draft():
            return
        selected_provider = self._selected_provider()
        result = self.registry.check(
            self.config,
            selected_provider,
            settings=self.draft.edited,
        )
        lines = [
            f"Check: {result.name}",
            f"status: {'ok' if result.ok else 'failed'}",
            f"model: {result.model or '-'}",
        ]
        if result.latency_ms is not None:
            lines.append(f"latency: {result.latency_ms}ms")
        if result.error:
            lines.append(f"error: {result.error}")
        self.draft = self.draft.with_check_result("\n".join(lines))
        self._update_detail(selected_provider)

    def action_quit(self) -> None:
        self.app.exit()

    def on_data_table_row_highlighted(self, message: DataTable.RowHighlighted) -> None:
        if message.data_table is not self.query_one("#config-table", DataTable):
            return
        self._sync_form_from_draft(str(message.row_key.value))
        self._update_detail(message.row_key.value)

    def on_data_table_row_selected(self, message: DataTable.RowSelected) -> None:
        if message.data_table is not self.query_one("#config-table", DataTable):
            return
        self._sync_form_from_draft(str(message.row_key.value))
        self._update_detail(message.row_key.value)

    def on_data_table_cell_highlighted(self, message: DataTable.CellHighlighted) -> None:
        if message.data_table is not self.query_one("#config-table", DataTable):
            return
        provider_name = self._provider_for_row(message.coordinate.row)
        if provider_name is None:
            return
        self._sync_form_from_draft(provider_name)
        self._update_detail(provider_name)

    def on_data_table_cell_selected(self, message: DataTable.CellSelected) -> None:
        if message.data_table is not self.query_one("#config-table", DataTable):
            return
        provider_name = self._provider_for_row(message.coordinate.row)
        if provider_name is None:
            return
        self._sync_form_from_draft(provider_name)
        self._update_detail(provider_name)

    def on_input_changed(self, message: Input.Changed) -> None:
        self._handle_draft_input_changed(message.input)

    def _handle_draft_input_changed(self, input_widget: Input) -> None:
        if self._syncing_form:
            return
        if input_widget.id is None or not input_widget.id.startswith(("provider-", "dreaming-")):
            return
        selected_provider = self._selected_provider()
        if self._apply_form_to_draft():
            self._update_provider_row(selected_provider)
            self._update_active_provider_markers()
            self._update_detail(selected_provider)

    def _refresh(self, selected_provider: str | None = None) -> None:
        table = self.query_one("#config-table", DataTable)
        table.clear()
        rows = self._provider_rows()
        active_provider = self.draft.edited.dreaming.active_provider
        for row in rows:
            name = str(row["name"])
            table.add_row(
                name,
                "*" if name == active_provider else "",
                yes_no(bool(row["enabled"])),
                str(row["model"] or "-"),
                str(row["api_key_env"] or "-"),
                yes_no(bool(row["configured"])),
                str(row["error"] or ""),
                key=name,
            )
        if selected_provider is None:
            selected_provider = self._selected_provider(default=active_provider)
        for index, row in enumerate(table.ordered_rows):
            if row.key.value == selected_provider:
                table.move_cursor(row=index)
                break
        self._sync_form_from_draft(selected_provider)
        self._update_detail(selected_provider)

    def _provider_rows(self) -> list[dict[str, object]]:
        rows = self.registry.status_payload(self.config, settings=self.draft.edited)
        by_name = {str(row["name"]): dict(row) for row in rows}
        for metadata in self.registry.list():
            provider = self.draft.edited.providers.get(
                metadata.name,
                ProviderSettings(),
            )
            configured, error = _configured_status(metadata.name, provider)
            by_name[metadata.name] = {
                **by_name.get(
                    metadata.name,
                    {
                        "name": metadata.name,
                        "display_name": metadata.display_name,
                    },
                ),
                "enabled": provider.enabled,
                "configured": configured,
                "model": provider.model,
                "api_key_env": provider.api_key_env,
                "base_url": provider.base_url,
                "error": error,
            }
        return [by_name[metadata.name] for metadata in self.registry.list()]

    def _update_provider_row(self, provider_name: str) -> None:
        table = self.query_one("#config-table", DataTable)
        provider = self.draft.edited.providers.get(provider_name, ProviderSettings())
        configured, error = _configured_status(provider_name, provider)
        active_provider = self.draft.edited.dreaming.active_provider
        table.update_cell(provider_name, "active", "*" if provider_name == active_provider else "")
        table.update_cell(provider_name, "enabled", yes_no(provider.enabled))
        table.update_cell(provider_name, "model", str(provider.model or "-"))
        table.update_cell(provider_name, "key_env", str(provider.api_key_env or "-"))
        table.update_cell(provider_name, "configured", yes_no(configured))
        table.update_cell(provider_name, "error", str(error or ""))

    def _update_active_provider_markers(self) -> None:
        table = self.query_one("#config-table", DataTable)
        active_provider = self.draft.edited.dreaming.active_provider
        for metadata in self.registry.list():
            table.update_cell(
                metadata.name,
                "active",
                "*" if metadata.name == active_provider else "",
            )

    def _selected_provider(self, default: str | None = None) -> str:
        table = self.query_one("#config-table", DataTable)
        if table.row_count and table.is_valid_row_index(table.cursor_row):
            row_key = table.ordered_rows[table.cursor_row].key.value
            if row_key is not None:
                return str(row_key)
        return default or self.draft.edited.dreaming.active_provider

    def _provider_for_row(self, row_index: int) -> str | None:
        table = self.query_one("#config-table", DataTable)
        if not table.is_valid_row_index(row_index):
            return None
        row_key = table.ordered_rows[row_index].key.value
        if row_key is None:
            return None
        return str(row_key)

    def _sync_form_from_draft(self, selected_provider: str) -> None:
        provider = self.draft.edited.providers.get(selected_provider, ProviderSettings())
        dreaming = self.draft.edited.dreaming
        values = {
            "#provider-enabled": field_value(provider.enabled),
            "#provider-model": field_value(provider.model),
            "#provider-api-key-env": field_value(provider.api_key_env),
            "#provider-base-url": field_value(provider.base_url),
            "#provider-timeout-seconds": field_value(provider.timeout_seconds),
            "#dreaming-active-provider": field_value(dreaming.active_provider),
            "#dreaming-autostart-enabled": field_value(dreaming.autostart_enabled),
            "#dreaming-min-interval-minutes": field_value(dreaming.min_interval_minutes),
            "#dreaming-new-short-term-memory-threshold": field_value(
                dreaming.new_short_term_memory_threshold
            ),
            "#dreaming-max-cycles-per-autostart": field_value(dreaming.max_cycles_per_autostart),
        }
        self._syncing_form = True
        try:
            for selector, value in values.items():
                self.query_one(selector, Input).value = value
        finally:
            self._syncing_form = False

    def _provider_form_values(self) -> dict[str, str]:
        return {
            "enabled": self.query_one("#provider-enabled", Input).value,
            "model": self.query_one("#provider-model", Input).value,
            "api_key_env": self.query_one("#provider-api-key-env", Input).value,
            "base_url": self.query_one("#provider-base-url", Input).value,
            "timeout_seconds": self.query_one(
                "#provider-timeout-seconds",
                Input,
            ).value,
        }

    def _dreaming_form_values(self) -> dict[str, str]:
        return {
            "active_provider": self.query_one("#dreaming-active-provider", Input).value,
            "autostart_enabled": self.query_one(
                "#dreaming-autostart-enabled",
                Input,
            ).value,
            "min_interval_minutes": self.query_one(
                "#dreaming-min-interval-minutes",
                Input,
            ).value,
            "new_short_term_memory_threshold": self.query_one(
                "#dreaming-new-short-term-memory-threshold",
                Input,
            ).value,
            "max_cycles_per_autostart": self.query_one(
                "#dreaming-max-cycles-per-autostart",
                Input,
            ).value,
        }

    def _apply_form_to_draft(self) -> bool:
        selected_provider = self._selected_provider()
        try:
            edited = apply_provider_form(
                self.draft.edited,
                selected_provider,
                self._provider_form_values(),
            )
            edited = apply_dreaming_form(edited, self._dreaming_form_values())
        except SettingsError as error:
            self.draft = self.draft.with_errors([str(error)])
            self._update_detail(selected_provider)
            return False
        self.draft = self.draft.with_edited(edited)
        return True

    def _update_detail(self, selected_provider: str | None) -> None:
        provider_name = selected_provider or self.draft.edited.dreaming.active_provider
        provider = self.draft.edited.providers.get(provider_name, ProviderSettings())
        dreaming = self.draft.edited.dreaming
        configured, error = _configured_status(provider_name, provider)
        detail = [
            f"settings_path: {self.config.settings_path}",
            f"database_path: {self.config.database_path}",
            f"state: {'unsaved' if self.draft.has_unsaved_changes else 'saved'}",
            "",
            "Autostart",
            f"enabled: {yes_no(dreaming.autostart_enabled)}",
            f"active_provider: {dreaming.active_provider}",
            f"min_interval_minutes: {dreaming.min_interval_minutes}",
            (f"new_short_term_memory_threshold: {dreaming.new_short_term_memory_threshold}"),
            f"max_cycles_per_autostart: {dreaming.max_cycles_per_autostart}",
            "",
            f"Selected provider: {provider_name}",
            f"enabled: {yes_no(provider.enabled)}",
            f"configured: {yes_no(configured)}",
            f"model: {provider.model or '-'}",
            f"key env: {provider.api_key_env or '-'}",
            f"api_key_present: {yes_no(env_value_exists(provider.api_key_env))}",
            f"base_url: {provider.base_url or '-'}",
            f"timeout_seconds: {provider.timeout_seconds}",
            f"error: {error or '-'}",
        ]
        if self._check_result_matches_provider(provider_name):
            detail.extend(["", self.draft.check_result])
        if self.draft.errors:
            detail.extend(["", "Validation errors", *self.draft.errors])
        detail.extend(["", "Keys: 1-4 set active, s save, r reload, c check selected, q quit."])
        self.query_one("#config-detail", Static).update("\n".join(detail))

    def _check_result_matches_provider(self, provider_name: str) -> bool:
        return self.draft.check_result.startswith(f"Check: {provider_name}\n")


def _configured_status(name: str, provider: ProviderSettings) -> tuple[bool, str]:
    if name == "deterministic":
        return True, ""
    if not provider.model.strip():
        return False, "model is empty"
    if not provider.api_key_env.strip():
        return False, "api_key_env is empty"
    if provider.enabled and not env_value_exists(provider.api_key_env):
        return False, f"missing environment variable: {provider.api_key_env}"
    return True, ""
