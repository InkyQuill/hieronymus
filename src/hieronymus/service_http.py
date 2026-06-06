from __future__ import annotations

import json
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.service_state import ServerState


class HieronymusHTTPServer(ThreadingHTTPServer):
    def __init__(
        self,
        server_address: tuple[str, int],
        config: HieronymusConfig,
        state: ServerState,
    ) -> None:
        super().__init__(server_address, HieronymusRequestHandler)
        self.config = config
        self.state = state


class HieronymusRequestHandler(BaseHTTPRequestHandler):
    server: HieronymusHTTPServer

    def log_message(self, format: str, *args: object) -> None:
        return

    def do_GET(self) -> None:
        if self.path == "/health":
            self._send_json(
                {
                    "ok": True,
                    "service": "hieronymus",
                    "version": self.server.state.version,
                }
            )
            return
        if self.path == "/status":
            self._send_json(status_payload(self.server.config, self.server.state))
            return
        self._send_json({"error": "not_found", "path": self.path}, status=HTTPStatus.NOT_FOUND)

    def do_POST(self) -> None:
        if self.path == "/shutdown":
            self._send_json({"ok": True, "stopping": True})
            self.server.shutdown()
            return
        self._send_json({"error": "not_found", "path": self.path}, status=HTTPStatus.NOT_FOUND)

    def _send_json(self, payload: dict[str, Any], status: HTTPStatus = HTTPStatus.OK) -> None:
        body = json.dumps(payload, ensure_ascii=False, sort_keys=True).encode("utf-8")
        self.send_response(status.value)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def status_payload(config: HieronymusConfig, state: ServerState) -> dict[str, Any]:
    return {
        "running": True,
        "pid": state.pid,
        "host": state.host,
        "port": state.port,
        "version": state.version,
        "started_at": state.started_at,
        "data_root": str(config.data_root),
        "database_path": str(config.database_path),
        "config_path": str(config.config_root),
        "providers": [],
        "mcp_adapter": {"available": True, "mode": "local-http"},
        "housekeeping": {"last_cycle": None, "pending": False},
    }


def build_server(config: HieronymusConfig, state: ServerState) -> HieronymusHTTPServer:
    return HieronymusHTTPServer((state.host, state.port), config, state)
