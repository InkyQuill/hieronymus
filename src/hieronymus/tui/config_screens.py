from __future__ import annotations

import os
from dataclasses import replace

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal
from textual.screen import Screen
from textual.widgets import DataTable, Footer, Static

from hieronymus.config import HieronymusConfig
from hieronymus.dream_providers import ProviderRegistry
from hieronymus.settings import (
    ProviderSettings,
    load_settings,
    save_settings,
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
        self.settings = load_settings(config)
        self.registry = ProviderRegistry()

    def compose(self) -> ComposeResult:
        yield Static("Providers", id="config-title")
        with Horizontal(id="workspace"):
            yield DataTable(id="config-table")
            yield Static("", id="config-detail")
        yield Footer()

    def on_mount(self) -> None:
        table = self.query_one("#config-table", DataTable)
        table.add_columns(
            "provider",
            "active",
            "enabled",
            "model",
            "key env",
            "configured",
            "error",
        )
        table.focus()
        self._refresh()

    def action_set_active(self, name: str) -> None:
        provider = self.settings.providers.get(name, ProviderSettings())
        if not provider.enabled:
            self.settings = self.settings.with_provider(
                name,
                replace(provider, enabled=True),
            )
        self.settings = self.settings.with_dreaming(
            replace(self.settings.dreaming, active_provider=name)
        )
        self._refresh(selected_provider=name)

    def action_save(self) -> None:
        save_settings(self.config, self.settings)
        self._refresh()

    def action_reload(self) -> None:
        self.settings = load_settings(self.config)
        self._refresh()

    def action_check_selected(self) -> None:
        selected_provider = self._selected_provider()
        result = self.registry.check(self.config, selected_provider)
        lines = [
            f"Check: {result.name}",
            f"status: {'ok' if result.ok else 'failed'}",
            f"model: {result.model or '-'}",
        ]
        if result.latency_ms is not None:
            lines.append(f"latency: {result.latency_ms}ms")
        if result.error:
            lines.append(f"error: {result.error}")
        self.query_one("#config-detail", Static).update("\n".join(lines))

    def action_quit(self) -> None:
        self.app.exit()

    def on_data_table_row_highlighted(self, message: DataTable.RowHighlighted) -> None:
        if message.data_table is not self.query_one("#config-table", DataTable):
            return
        self._update_detail(message.row_key.value)

    def on_data_table_row_selected(self, message: DataTable.RowSelected) -> None:
        if message.data_table is not self.query_one("#config-table", DataTable):
            return
        self._update_detail(message.row_key.value)

    def _refresh(self, selected_provider: str | None = None) -> None:
        table = self.query_one("#config-table", DataTable)
        table.clear()
        rows = self._provider_rows()
        active_provider = self.settings.dreaming.active_provider
        for row in rows:
            name = str(row["name"])
            table.add_row(
                name,
                "*" if name == active_provider else "",
                _yes_no(bool(row["enabled"])),
                str(row["model"] or "-"),
                str(row["api_key_env"] or "-"),
                _yes_no(bool(row["configured"])),
                str(row["error"] or ""),
                key=name,
            )
        if selected_provider is None:
            selected_provider = self._selected_provider(default=active_provider)
        for index, row in enumerate(table.ordered_rows):
            if row.key.value == selected_provider:
                table.move_cursor(row=index)
                break
        self._update_detail(selected_provider)

    def _provider_rows(self) -> list[dict[str, object]]:
        rows = self.registry.status_payload(self.config)
        by_name = {str(row["name"]): dict(row) for row in rows}
        for metadata in self.registry.list():
            provider = self.settings.providers.get(metadata.name, ProviderSettings())
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

    def _selected_provider(self, default: str | None = None) -> str:
        table = self.query_one("#config-table", DataTable)
        if table.row_count and table.is_valid_row_index(table.cursor_row):
            row_key = table.ordered_rows[table.cursor_row].key.value
            if row_key is not None:
                return str(row_key)
        return default or self.settings.dreaming.active_provider

    def _update_detail(self, selected_provider: str | None) -> None:
        provider_name = selected_provider or self.settings.dreaming.active_provider
        provider = self.settings.providers.get(provider_name, ProviderSettings())
        dreaming = self.settings.dreaming
        configured, error = _configured_status(provider_name, provider)
        detail = [
            f"settings_path: {self.config.settings_path}",
            f"database_path: {self.config.database_path}",
            "",
            "Autostart",
            f"enabled: {_yes_no(dreaming.autostart_enabled)}",
            f"active_provider: {dreaming.active_provider}",
            f"min_interval_minutes: {dreaming.min_interval_minutes}",
            f"new_short_term_memory_threshold: {dreaming.new_short_term_memory_threshold}",
            f"max_cycles_per_autostart: {dreaming.max_cycles_per_autostart}",
            "",
            f"Selected provider: {provider_name}",
            f"enabled: {_yes_no(provider.enabled)}",
            f"configured: {_yes_no(configured)}",
            f"model: {provider.model or '-'}",
            f"key env: {provider.api_key_env or '-'}",
            f"base_url: {provider.base_url or '-'}",
            f"error: {error or '-'}",
            "",
            "Keys: 1-4 set active, s save, r reload, c check selected, q quit.",
        ]
        self.query_one("#config-detail", Static).update("\n".join(detail))


def _configured_status(name: str, provider: ProviderSettings) -> tuple[bool, str]:
    if name == "deterministic":
        return True, ""
    if not provider.model.strip():
        return False, "model is empty"
    if not provider.api_key_env.strip():
        return False, "api_key_env is empty"
    if provider.enabled and not os.environ.get(provider.api_key_env):
        return False, f"missing environment variable: {provider.api_key_env}"
    return True, ""


def _yes_no(value: bool) -> str:
    return "yes" if value else "no"
