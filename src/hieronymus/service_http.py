from __future__ import annotations

import json
import secrets
from dataclasses import replace
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from mimetypes import guess_type
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

from hieronymus.config import HieronymusConfig
from hieronymus.dream_autostart import DreamAutostart
from hieronymus.dream_providers import ProviderRegistry
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
        request_url = urlparse(self.path)
        path = request_url.path
        if path in {"/config", "/admin"}:
            token = parse_qs(request_url.query).get("token", [""])[0]
            if token and secrets.compare_digest(token, self.server.state.token):
                self.send_response(HTTPStatus.FOUND.value)
                self.send_header(
                    "Set-Cookie",
                    f"hieronymus_token={token}; HttpOnly; SameSite=Strict; Path=/",
                )
                self.send_header("Location", path)
                self.end_headers()
                return
            if not self._is_authorized():
                self._send_json({"error": "unauthorized"}, status=HTTPStatus.UNAUTHORIZED)
                return
            self._send_web_app()
            return
        if path.startswith("/assets/"):
            if not self._is_authorized():
                self._send_json({"error": "unauthorized"}, status=HTTPStatus.UNAUTHORIZED)
                return
            self._send_web_asset(path)
            return
        if path == "/health":
            if not self._is_authorized():
                self._send_json({"error": "unauthorized"}, status=HTTPStatus.UNAUTHORIZED)
                return
            self._send_json(
                {
                    "ok": True,
                    "service": "hieronymus",
                    "version": self.server.state.version,
                }
            )
            return
        if path == "/status":
            if not self._is_authorized():
                self._send_json({"error": "unauthorized"}, status=HTTPStatus.UNAUTHORIZED)
                return
            self._send_json(status_payload(self.server.config, self.server.state))
            return
        self._send_json({"error": "not_found", "path": path}, status=HTTPStatus.NOT_FOUND)

    def do_POST(self) -> None:
        if self.path == "/shutdown":
            if not self._is_authorized():
                self._send_json({"error": "unauthorized"}, status=HTTPStatus.UNAUTHORIZED)
                return
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

    def _send_web_app(self) -> None:
        for root in _web_asset_roots():
            index = root / "index.html"
            if index.exists():
                body = index.read_bytes()
                self.send_response(HTTPStatus.OK.value)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
        self._send_json({"error": "web_console_not_built"}, status=HTTPStatus.NOT_FOUND)

    def _send_web_asset(self, request_path: str) -> None:
        relative_path = Path(request_path.lstrip("/"))
        if relative_path.is_absolute() or ".." in relative_path.parts:
            self._send_json({"error": "not_found"}, status=HTTPStatus.NOT_FOUND)
            return
        for root in _web_asset_roots():
            candidate = root / relative_path
            if candidate.is_file():
                body = candidate.read_bytes()
                content_type = guess_type(candidate.name)[0] or "application/octet-stream"
                self.send_response(HTTPStatus.OK.value)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
        self._send_json({"error": "not_found"}, status=HTTPStatus.NOT_FOUND)

    def _is_authorized(self) -> bool:
        token = self.headers.get("X-Hieronymus-Token", "") or _session_token(
            self.headers.get("Cookie", "")
        )
        return secrets.compare_digest(token, self.server.state.token)


def _web_asset_roots() -> list[Path]:
    package_root = Path(__file__).resolve().parent / "frontend" / "dist"
    roots = [package_root]
    for ancestor in Path(__file__).resolve().parents[:5]:
        roots.append(ancestor / "frontend" / "dist")
        roots.append(ancestor / "frontend")
    return roots


def _session_token(cookie_header: str) -> str:
    for item in cookie_header.split(";"):
        name, separator, value = item.strip().partition("=")
        if separator and name == "hieronymus_token":
            return value
    return ""


def status_payload(config: HieronymusConfig, state: ServerState) -> dict[str, Any]:
    try:
        dreaming_status = DreamAutostart(config).status()
    except Exception as error:
        dreaming_status = {
            "available": False,
            "pending_short_term_memories": 0,
            "error": str(error),
        }
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
        "providers": ProviderRegistry().status_payload(config),
        "dreaming": dreaming_status,
        "mcp_adapter": {"available": True, "mode": "local-http"},
        "housekeeping": {
            "last_cycle": None,
            "pending": int(dreaming_status.get("pending_short_term_memories", 0)) > 0,
        },
    }


def build_server(config: HieronymusConfig, state: ServerState) -> HieronymusHTTPServer:
    server = HieronymusHTTPServer((state.host, state.port), config, state)
    actual_port = int(server.server_address[1])
    if actual_port != state.port:
        server.state = replace(state, port=actual_port)
    return server
