from __future__ import annotations

from collections.abc import Callable
from typing import TypeAlias

from hieronymus.config import HieronymusConfig

McpPayload: TypeAlias = dict[str, object] | list[dict[str, object]]
McpOperation: TypeAlias = Callable[[HieronymusConfig, dict[str, object]], McpPayload]

MCP_OPERATION_HANDLERS: dict[str, McpOperation] = {}
