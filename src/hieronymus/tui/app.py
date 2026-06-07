from __future__ import annotations

from textual.app import App

from hieronymus.config import HieronymusConfig


class HieronymusAdminApp(App[None]):
    TITLE = "Hieronymus Admin"

    def __init__(self, config: HieronymusConfig) -> None:
        super().__init__()
        self.config = config
