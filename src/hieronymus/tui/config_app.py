from __future__ import annotations

from textual.app import App

from hieronymus.config import HieronymusConfig
from hieronymus.tui.config_screens import ConfigScreen


class HieronymusConfigApp(App[None]):
    TITLE = "Hieronymus Config"
    CSS_PATH = "styles.tcss"
    COMMAND_PALETTE_BINDING = "ctrl+shift+p"

    def __init__(self, config: HieronymusConfig) -> None:
        super().__init__()
        self.config = config

    def on_mount(self) -> None:
        self.push_screen(ConfigScreen(self.config))
