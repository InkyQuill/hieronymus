from __future__ import annotations

import base64
import hashlib
import json
import secrets
import struct
import threading
import time
from dataclasses import replace
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from mimetypes import guess_type
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

from hieronymus.config import HieronymusConfig
from hieronymus.daemon_events import AdminEventHub
from hieronymus.db import connect
from hieronymus.dream_autostart import DreamAutostart
from hieronymus.dream_providers import ProviderRegistry
from hieronymus.mcp_operations import MCP_OPERATION_HANDLERS
from hieronymus.provider_config import load_provider_catalog
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.service_state import ServerState
from hieronymus.tui_bridge.admin_api import AdminBridge
from hieronymus.tui_bridge.config_api import ConfigBridge


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
        self.events = AdminEventHub()
        self.dream_lock = threading.Lock()
        self.dream_running = False


class HieronymusRequestHandler(BaseHTTPRequestHandler):
    server: HieronymusHTTPServer

    def log_message(self, format: str, *args: object) -> None:
        return

    def do_GET(self) -> None:
        request_url = urlparse(self.path)
        path = request_url.path
        if path == "/ws/admin":
            self._handle_admin_websocket()
            return
        if _is_web_route(path):
            self._send_web_app()
            return
        if path.startswith("/assets/"):
            self._send_web_asset(path)
            return
        if path == "/api/providers":
            if not self._is_browser_authorized():
                self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
                return
            self._send_json(ConfigBridge(self.server.config).provider_list({}))
            return
        if path.startswith("/api/providers/"):
            if not self._is_browser_authorized():
                self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
                return
            self._handle_provider_get(path)
            return
        if path in _SETTINGS_GET_METHODS:
            if not self._is_browser_authorized():
                self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
                return
            self._send_config_result(_SETTINGS_GET_METHODS[path], {})
            return
        if path == "/api/admin/dashboard":
            if not self._is_browser_authorized():
                self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
                return
            self._send_admin_result("dashboard", {})
            return
        if path == "/api/admin/snapshot":
            if not self._is_browser_authorized():
                self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
                return
            view = parse_qs(request_url.query).get("view", [""])[0]
            selected_id = parse_qs(request_url.query).get("selected_id", [""])[0]
            self._send_admin_result(
                "snapshot",
                {"view": view, "selected_id": selected_id},
            )
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
        path = urlparse(self.path).path
        if path == "/shutdown":
            if not self._is_authorized():
                self._send_json({"error": "unauthorized"}, status=HTTPStatus.UNAUTHORIZED)
                return
            self._send_json({"ok": True, "stopping": True})
            self.server.shutdown()
            return
        if path == "/api/providers":
            if not self._is_browser_authorized():
                self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
                return
            self._send_config_result("save_provider", self._request_json())
            return
        if path.startswith("/api/providers/") and path.endswith("/check"):
            if not self._is_browser_authorized():
                self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
                return
            provider_id = path.removeprefix("/api/providers/").removesuffix("/check").rstrip("/")
            self._send_config_result("check_saved_provider", {"provider_id": provider_id})
            return
        if path in _SETTINGS_SAVE_METHODS:
            if not self._is_browser_authorized():
                self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
                return
            self._send_config_result(_SETTINGS_SAVE_METHODS[path], self._request_json())
            return
        if path.startswith("/api/admin/actions/"):
            if not self._is_browser_authorized():
                self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
                return
            action = path.removeprefix("/api/admin/actions/").strip("/")
            method = _ADMIN_ACTION_METHODS.get(action)
            if method is None:
                self._send_json({"error": "unknown_admin_action"}, status=HTTPStatus.NOT_FOUND)
                return
            if method == "run_manual_dreaming":
                self._start_manual_dreaming()
                return
            self._send_admin_result(method, self._request_json())
            return
        if path.startswith("/api/mcp/"):
            if not self._is_authorized():
                self._send_json({"error": "unauthorized"}, status=HTTPStatus.UNAUTHORIZED)
                return
            operation = path.removeprefix("/api/mcp/").strip("/")
            handler = MCP_OPERATION_HANDLERS.get(operation)
            if handler is None:
                self._send_json({"error": "unknown_mcp_operation"}, status=HTTPStatus.NOT_FOUND)
                return
            try:
                payload = handler(self.server.config, self._request_json())
            except ValueError as error:
                self._send_json({"error": str(error)}, status=HTTPStatus.BAD_REQUEST)
                return
            except KeyError as error:
                self._send_json(
                    {"error": str(error), "error_type": "KeyError"},
                    status=HTTPStatus.BAD_REQUEST,
                )
                return
            self._send_json({"result": payload})
            return
        self._send_json({"error": "not_found", "path": path}, status=HTTPStatus.NOT_FOUND)

    def do_DELETE(self) -> None:
        path = urlparse(self.path).path
        if not path.startswith("/api/providers/"):
            self._send_json({"error": "not_found", "path": path}, status=HTTPStatus.NOT_FOUND)
            return
        if not self._is_browser_authorized():
            self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
            return
        provider_id = path.removeprefix("/api/providers/")
        self._send_config_result("delete_provider", {"provider_id": provider_id})

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

    def _handle_provider_get(self, path: str) -> None:
        suffix = path.removeprefix("/api/providers/")
        if suffix.endswith("/models"):
            provider_id = suffix.removesuffix("/models").rstrip("/")
            self._send_config_result("provider_models", {"provider_id": provider_id})
            return
        self._send_config_result("provider_detail", {"provider_id": suffix})

    def _send_config_result(self, method: str, params: dict[str, object]) -> None:
        bridge = ConfigBridge(self.server.config)
        handler = getattr(bridge, method)
        try:
            payload = handler(params)
        except ValueError as error:
            self._send_json({"error": str(error)}, status=HTTPStatus.BAD_REQUEST)
            return
        status = HTTPStatus.BAD_REQUEST if payload.get("error") else HTTPStatus.OK
        self._send_json(payload, status=status)

    def _send_admin_result(self, method: str, params: dict[str, object]) -> None:
        bridge = AdminBridge(self.server.config)
        handler = getattr(bridge, method)
        try:
            payload = handler(params)
        except ValueError as error:
            self._send_json({"error": str(error)}, status=HTTPStatus.BAD_REQUEST)
            return
        self._send_json(payload)

    def _request_json(self) -> dict[str, object]:
        try:
            content_length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            return {}
        if content_length <= 0 or content_length > 1_000_000:
            return {}
        try:
            payload = json.loads(self.rfile.read(content_length).decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            return {}
        return payload if type(payload) is dict else {}

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

    def _is_browser_authorized(self) -> bool:
        if self._is_authorized():
            return True
        origin = self.headers.get("Origin", "")
        expected = f"http://{self.server.state.host}:{self.server.state.port}"
        if origin:
            return secrets.compare_digest(origin, expected)
        if self.headers.get("Sec-Fetch-Site", "") != "same-origin":
            return False
        referer = urlparse(self.headers.get("Referer", ""))
        return secrets.compare_digest(
            f"{referer.scheme}://{referer.netloc}",
            expected,
        )

    def _handle_admin_websocket(self) -> None:
        if not self._is_browser_authorized():
            self._send_json({"error": "forbidden"}, status=HTTPStatus.FORBIDDEN)
            return
        key = self.headers.get("Sec-WebSocket-Key", "")
        if self.headers.get("Upgrade", "").lower() != "websocket" or not key:
            self._send_json({"error": "websocket_upgrade_required"}, status=HTTPStatus.BAD_REQUEST)
            return
        accept = base64.b64encode(
            hashlib.sha1(f"{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11".encode()).digest()
        ).decode()
        self.send_response(HTTPStatus.SWITCHING_PROTOCOLS.value)
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.end_headers()

        send_lock = threading.Lock()

        def send_event(event: dict[str, object]) -> None:
            with send_lock:
                body = json.dumps(event, ensure_ascii=False).encode("utf-8")
                header = bytes([0x81])
                if len(body) < 126:
                    header += bytes([len(body)])
                elif len(body) <= 0xFFFF:
                    header += bytes([126]) + struct.pack("!H", len(body))
                else:
                    header += bytes([127]) + struct.pack("!Q", len(body))
                self.connection.sendall(header + body)

        unsubscribe = self.server.events.subscribe(send_event)
        try:
            self.connection.settimeout(1)
            while self.connection.recv(2):
                pass
        except TimeoutError:
            while True:
                try:
                    if not self.connection.recv(2):
                        break
                except TimeoutError:
                    continue
                except OSError:
                    break
        except OSError:
            pass
        finally:
            unsubscribe()

    def _start_manual_dreaming(self) -> None:
        with self.server.dream_lock:
            if self.server.dream_running:
                self._send_json({"started": False, "status": "running"})
                return
            self.server.dream_running = True
        self.server.events.publish("dream_started", {"trigger": "manual"})

        def monitor() -> None:
            seen_phase_id = 0
            while self.server.dream_running:
                try:
                    with connect(self.server.config.database_path) as conn:
                        row = conn.execute(
                            """
                            select p.id, p.phase, p.dream_run_id, r.cycle_id
                            from dream_phase_runs as p
                            join dream_runs as r on r.id = p.dream_run_id
                            where p.status = 'running'
                            order by p.id desc limit 1
                            """
                        ).fetchone()
                except Exception:
                    row = None
                if row is not None and int(row["id"]) != seen_phase_id:
                    seen_phase_id = int(row["id"])
                    self.server.events.publish(
                        "dream_phase_progress",
                        {
                            "run_id": int(row["dream_run_id"]),
                            "cycle_id": int(row["cycle_id"]),
                            "phase": row["phase"],
                        },
                    )
                time.sleep(0.2)

        def run() -> None:
            try:
                payload = AdminBridge(self.server.config).run_manual_dreaming({})
                result = payload["result"]
                self.server.events.publish(
                    "dream_completed", {"trigger": "manual", "result": result}
                )
            except Exception as error:
                message = redact_configured_secret_values(
                    str(error), load_provider_catalog(self.server.config)
                )
                self.server.events.publish("dream_failed", {"trigger": "manual", "error": message})
            finally:
                with self.server.dream_lock:
                    self.server.dream_running = False

        threading.Thread(target=run, name="hieronymus-manual-dream", daemon=True).start()
        threading.Thread(target=monitor, name="hieronymus-dream-progress", daemon=True).start()
        self._send_json({"started": True, "status": "running"})


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


def _is_web_route(path: str) -> bool:
    return path in {"/config", "/admin"} or path.startswith(("/config/", "/admin/"))


_SETTINGS_GET_METHODS = {
    "/api/settings/dream": "dream_settings",
    "/api/settings/ingest": "ingest_settings",
    "/api/settings/release": "release_settings",
}

_SETTINGS_SAVE_METHODS = {
    "/api/settings/dream": "save_dream_settings",
    "/api/settings/ingest": "save_ingest_settings",
    "/api/settings/release": "save_release_settings",
}

_ADMIN_ACTION_METHODS = {
    "reinforce_crystal": "reinforce_crystal",
    "decay_crystal": "decay_crystal",
    "deprecate_crystal": "deprecate_crystal",
    "delete_crystal": "delete_crystal",
    "approve_proposal": "approve_proposal",
    "reject_proposal": "reject_proposal",
    "reinforce_concept": "reinforce_concept",
    "decay_concept": "decay_concept",
    "archive_concept": "archive_concept",
    "remove_short_term_memory": "remove_short_term_memory",
    "close_session": "close_session",
    "run_manual_dreaming": "run_manual_dreaming",
}


def status_payload(config: HieronymusConfig, state: ServerState) -> dict[str, Any]:
    try:
        dreaming_status = DreamAutostart(config).status()
    except Exception as error:
        dreaming_status = {
            "available": False,
            "pending_short_term_memories": 0,
            "error": str(error),
        }
    try:
        provider_statuses = ProviderRegistry().status_payload(config)
        provider_status_error = ""
    except Exception as error:
        provider_statuses = []
        provider_status_error = str(error)
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
        "providers": provider_statuses,
        "providers_error": provider_status_error,
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
