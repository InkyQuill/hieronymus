from __future__ import annotations

from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClient, ServiceClientError
from hieronymus.service_state import cleanup_stale_state, read_server_state


def discover_local_service(config: HieronymusConfig) -> dict[str, Any]:
    cleanup_stale_state(config)
    state = read_server_state(config)
    if state is None:
        return {
            "available": False,
            "mode": "direct-local",
            "reason": "no running local service discovered",
        }

    try:
        health = ServiceClient().health(state)
    except (OSError, ServiceClientError) as exc:
        return {
            "available": False,
            "mode": "direct-local",
            "reason": f"local service state exists but health check failed: {exc}",
        }

    return {
        "available": True,
        "mode": "local-http",
        "base_url": state.base_url,
        "pid": state.pid,
        "version": str(health.get("version", state.version)),
        "data_root": state.data_root,
        "database_path": state.database_path,
    }
