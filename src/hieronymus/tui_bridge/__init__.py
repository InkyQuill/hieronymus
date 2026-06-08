from __future__ import annotations

from typing import Any


def run_stdio(*args: Any, **kwargs: Any) -> Any:
    from hieronymus.tui_bridge.server import run_stdio as _run_stdio

    return _run_stdio(*args, **kwargs)


__all__ = ["run_stdio"]
