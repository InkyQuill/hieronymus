from __future__ import annotations

from textual.app import App

from hieronymus.admin import AdminStore
from hieronymus.config import HieronymusConfig
from hieronymus.tui.screens import ManagementScreen


class HieronymusAdminApp(App[None]):
    TITLE = "Hieronymus Admin"
    CSS_PATH = "styles.tcss"

    def __init__(self, config: HieronymusConfig) -> None:
        super().__init__()
        self.config = config
        self.store = AdminStore(config)
        self.active_view = "Crystals"

    def on_mount(self) -> None:
        self.push_screen(ManagementScreen(self.store))
