from __future__ import annotations

import asyncio
import json
import threading
import time
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any

from starlette.applications import Starlette
from starlette.concurrency import run_in_threadpool
from starlette.exceptions import HTTPException
from starlette.requests import Request
from starlette.responses import FileResponse, JSONResponse, Response
from starlette.routing import Mount, Route, WebSocketRoute
from starlette.staticfiles import StaticFiles
from starlette.types import ASGIApp, Receive, Scope, Send
from starlette.websockets import WebSocket, WebSocketDisconnect

from hieronymus.config import HieronymusConfig
from hieronymus.daemon_events import AdminEventHub
from hieronymus.db import connect
from hieronymus.dream_autostart import DreamAutostart
from hieronymus.dream_providers import ProviderRegistry
from hieronymus.mcp_operations import MCP_OPERATION_HANDLERS, mcp_transport_diagnostics
from hieronymus.mcp_server import build_http_mcp_server
from hieronymus.provider_config import load_provider_catalog
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.service_state import EXPECTED_LAUNCH_ID_HEADER, ServerState
from hieronymus.tui_bridge.admin_api import AdminBridge
from hieronymus.tui_bridge.config_api import ConfigBridge

_MAX_JSON_BODY = 1_000_000


class _HostValidationMiddleware:
    def __init__(self, app: ASGIApp, *, expected_host: str, expected_origin: str) -> None:
        self._app = app
        self._expected_host = expected_host
        self._expected_origin = expected_origin

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] not in {"http", "websocket"}:
            await self._app(scope, receive, send)
            return
        headers = {
            key.decode("latin-1").lower(): value.decode("latin-1")
            for key, value in scope["headers"]
        }
        valid_host = headers.get("host", "") == self._expected_host
        valid_origin = headers.get("origin") in {None, self._expected_origin}
        if valid_host and valid_origin:
            await self._app(scope, receive, send)
            return
        if scope["type"] == "websocket":
            websocket = WebSocket(scope, receive=receive, send=send)
            await websocket.send_denial_response(_json({"error": "forbidden"}, 403))
            return
        await _json({"error": "forbidden"}, 403)(scope, receive, send)


class _ExactMcpMountPathMiddleware:
    def __init__(self, app: ASGIApp) -> None:
        self._app = app

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] == "http" and scope["path"] == "/mcp":
            scope = {
                **scope,
                "path": "/mcp/",
                "raw_path": b"/mcp/",
            }
        await self._app(scope, receive, send)


class _AssetFiles(StaticFiles):
    async def check_config(self) -> None:
        return

    async def get_response(self, path: str, scope: Scope) -> Response:
        try:
            response = await super().get_response(path, scope)
        except HTTPException as error:
            if error.status_code == 404:
                return _json({"error": "not_found"}, 404)
            raise
        if response.status_code == 404:
            return _json({"error": "not_found"}, 404)
        return response


class _ServiceRuntime:
    def __init__(self, config: HieronymusConfig, state: ServerState) -> None:
        self.config = config
        self.state = state
        self.events = AdminEventHub()
        self.dream_lock = threading.Lock()
        self.dream_running = False
        self.shutdown_requested = threading.Event()
        self.request_shutdown: Any = None

    def start_manual_dreaming(self) -> dict[str, object]:
        with self.dream_lock:
            if self.dream_running:
                return {"started": False, "status": "running"}
            self.dream_running = True
        self.events.publish("dream_started", {"trigger": "manual"})

        def monitor() -> None:
            seen_phase_id = 0
            while self.dream_running:
                try:
                    with connect(self.config.database_path) as conn:
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
                    self.events.publish(
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
                payload = AdminBridge(self.config).run_manual_dreaming({})
                self.events.publish(
                    "dream_completed", {"trigger": "manual", "result": payload["result"]}
                )
            except Exception as error:
                message = _safe_dream_failure_message(self.config, error)
                self.events.publish("dream_failed", {"trigger": "manual", "error": message})
            finally:
                with self.dream_lock:
                    self.dream_running = False

        threading.Thread(target=run, name="hieronymus-manual-dream", daemon=True).start()
        threading.Thread(target=monitor, name="hieronymus-dream-progress", daemon=True).start()
        return {"started": True, "status": "running"}


def _json(payload: dict[str, Any], status_code: int = 200) -> JSONResponse:
    return JSONResponse(payload, status_code=status_code)


async def _cancel_tasks(tasks: set[asyncio.Task[Any]]) -> None:
    for task in tasks:
        task.cancel()
    if tasks:
        await asyncio.gather(*tasks, return_exceptions=True)


def _runtime(request: Request) -> _ServiceRuntime:
    return request.app.state.runtime


def _expected_origin(runtime: _ServiceRuntime) -> str:
    return f"http://{runtime.state.host}:{runtime.state.port}"


def _browser_authorized(request: Request) -> bool:
    origin = request.headers.get("origin")
    return origin is None or origin == _expected_origin(_runtime(request))


def _forbidden_unless_browser_authorized(request: Request) -> JSONResponse | None:
    if _browser_authorized(request):
        return None
    return _json({"error": "forbidden"}, 403)


async def _request_json(request: Request) -> dict[str, object]:
    content_length = request.headers.get("content-length")
    if content_length:
        try:
            if int(content_length) > _MAX_JSON_BODY:
                return {}
        except ValueError:
            return {}
    body = bytearray()
    async for chunk in request.stream():
        body.extend(chunk)
        if len(body) > _MAX_JSON_BODY:
            return {}
    if not body:
        return {}
    try:
        payload = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return {}
    return payload if type(payload) is dict else {}


async def _web_app(request: Request) -> Response:
    index = request.app.state.asset_root / "index.html"
    if not index.is_file():
        return _json({"error": "web_console_not_built"}, 404)
    return FileResponse(index, media_type="text/html")


async def _health(request: Request) -> JSONResponse:
    state = _runtime(request).state
    return _json(
        {
            "ok": True,
            "service": "hieronymus",
            "version": state.version,
            "pid": state.pid,
            "data_root": state.data_root,
            "database_path": state.database_path,
            "launch_id": state.launch_id,
        }
    )


async def _status(request: Request) -> JSONResponse:
    runtime = _runtime(request)
    payload = await run_in_threadpool(status_payload, runtime.config, runtime.state)
    return _json(payload)


async def _shutdown(request: Request) -> JSONResponse:
    runtime = _runtime(request)
    expected_launch_id = request.headers.get(EXPECTED_LAUNCH_ID_HEADER)
    if runtime.state.launch_id is None or expected_launch_id != runtime.state.launch_id:
        return _json(
            {
                "error": "launch_identity_mismatch",
                "error_type": "launch_identity_mismatch",
            },
            412,
        )
    runtime.shutdown_requested.set()
    if runtime.request_shutdown is not None:
        runtime.request_shutdown()
    return _json({"ok": True, "stopping": True})


async def _providers(request: Request) -> JSONResponse:
    if forbidden := _forbidden_unless_browser_authorized(request):
        return forbidden
    runtime = _runtime(request)
    if request.method == "GET":
        return await _config_result(runtime.config, "provider_list", {})
    return await _config_result(runtime.config, "save_provider", await _request_json(request))


async def _provider(request: Request) -> JSONResponse:
    if forbidden := _forbidden_unless_browser_authorized(request):
        return forbidden
    runtime = _runtime(request)
    provider_id = request.path_params["provider_id"]
    if request.method == "DELETE":
        return await _config_result(runtime.config, "delete_provider", {"provider_id": provider_id})
    return await _config_result(runtime.config, "provider_detail", {"provider_id": provider_id})


async def _provider_models(request: Request) -> JSONResponse:
    if forbidden := _forbidden_unless_browser_authorized(request):
        return forbidden
    runtime = _runtime(request)
    return await _config_result(
        runtime.config, "provider_models", {"provider_id": request.path_params["provider_id"]}
    )


async def _provider_check(request: Request) -> JSONResponse:
    if forbidden := _forbidden_unless_browser_authorized(request):
        return forbidden
    runtime = _runtime(request)
    return await _config_result(
        runtime.config,
        "check_saved_provider",
        {"provider_id": request.path_params["provider_id"]},
    )


async def _settings(request: Request) -> JSONResponse:
    if forbidden := _forbidden_unless_browser_authorized(request):
        return forbidden
    runtime = _runtime(request)
    name = request.path_params["name"]
    methods = _SETTINGS_GET_METHODS if request.method == "GET" else _SETTINGS_SAVE_METHODS
    method = methods.get(name)
    if method is None:
        return _json({"error": "not_found", "path": request.url.path}, 404)
    params = {} if request.method == "GET" else await _request_json(request)
    return await _config_result(runtime.config, method, params)


async def _admin_dashboard(request: Request) -> JSONResponse:
    if forbidden := _forbidden_unless_browser_authorized(request):
        return forbidden
    return await _admin_result(_runtime(request).config, "dashboard", {})


async def _admin_snapshot(request: Request) -> JSONResponse:
    if forbidden := _forbidden_unless_browser_authorized(request):
        return forbidden
    return await _admin_result(
        _runtime(request).config,
        "snapshot",
        {
            "view": request.query_params.get("view", ""),
            "selected_id": request.query_params.get("selected_id", ""),
        },
    )


async def _admin_action(request: Request) -> JSONResponse:
    if forbidden := _forbidden_unless_browser_authorized(request):
        return forbidden
    action = request.path_params["action"]
    method = _ADMIN_ACTION_METHODS.get(action)
    if method is None:
        return _json({"error": "unknown_admin_action"}, 404)
    runtime = _runtime(request)
    if method == "run_manual_dreaming":
        return _json(runtime.start_manual_dreaming())
    return await _admin_result(runtime.config, method, await _request_json(request))


async def _mcp_operation(request: Request) -> JSONResponse:
    handler = MCP_OPERATION_HANDLERS.get(request.path_params["operation"])
    if handler is None:
        return _json({"error": "unknown_mcp_operation"}, 404)
    config = _runtime(request).config
    params = await _request_json(request)
    return await run_in_threadpool(_mcp_result, handler, config, params)


def _mcp_result(handler: Any, config: HieronymusConfig, params: dict[str, object]) -> JSONResponse:
    try:
        payload = handler(config, params)
    except ValueError as error:
        return _json({"error": str(error)}, 400)
    except KeyError as error:
        return _json({"error": str(error), "error_type": "KeyError"}, 400)
    return _json({"result": payload})


async def _admin_websocket(websocket: WebSocket) -> None:
    runtime: _ServiceRuntime = websocket.app.state.runtime
    origin = websocket.headers.get("origin")
    if origin is not None and origin != _expected_origin(runtime):
        await websocket.send_denial_response(_json({"error": "forbidden"}, 403))
        return
    loop = asyncio.get_running_loop()
    events: asyncio.Queue[dict[str, object]] = asyncio.Queue()

    def receive_event(event: dict[str, object]) -> None:
        loop.call_soon_threadsafe(events.put_nowait, event)

    unsubscribe = runtime.events.subscribe(receive_event)
    await websocket.accept()
    try:
        while True:
            event_task = asyncio.create_task(events.get())
            receive_task = asyncio.create_task(websocket.receive())
            race_tasks = {event_task, receive_task}
            done: set[asyncio.Task[Any]] = set()
            try:
                done, _ = await asyncio.wait(race_tasks, return_when=asyncio.FIRST_COMPLETED)
            finally:
                await _cancel_tasks(race_tasks - done)
            if receive_task in done:
                message = receive_task.result()
                if message["type"] == "websocket.disconnect":
                    break
            if event_task in done:
                await websocket.send_json(event_task.result())
    except WebSocketDisconnect:
        pass
    finally:
        unsubscribe()


async def _config_result(
    config: HieronymusConfig, method: str, params: dict[str, object]
) -> JSONResponse:
    return await run_in_threadpool(_call_config, config, method, params)


def _call_config(config: HieronymusConfig, method: str, params: dict[str, object]) -> JSONResponse:
    try:
        payload = getattr(ConfigBridge(config), method)(params)
    except ValueError as error:
        return _json({"error": str(error)}, 400)
    return _json(payload, 400 if payload.get("error") else 200)


async def _admin_result(
    config: HieronymusConfig, method: str, params: dict[str, object]
) -> JSONResponse:
    return await run_in_threadpool(_call_admin, config, method, params)


def _call_admin(config: HieronymusConfig, method: str, params: dict[str, object]) -> JSONResponse:
    try:
        payload = getattr(AdminBridge(config), method)(params)
    except ValueError as error:
        return _json({"error": str(error)}, 400)
    return _json(payload)


def _safe_dream_failure_message(config: HieronymusConfig, error: Exception) -> str:
    try:
        return redact_configured_secret_values(str(error), load_provider_catalog(config))
    except Exception:
        return "manual dreaming failed"


async def _not_found(request: Request, _: Exception) -> JSONResponse:
    return _json({"error": "not_found", "path": request.url.path}, 404)


def build_app(
    config: HieronymusConfig,
    state: ServerState,
    *,
    asset_root: Path | None = None,
) -> Starlette:
    resolved_asset_root = asset_root or Path(__file__).resolve().parent / "frontend" / "dist"
    runtime = _ServiceRuntime(config, state)
    mcp_server = build_http_mcp_server(config)
    mcp_app = mcp_server.streamable_http_app()

    @asynccontextmanager
    async def lifespan(_: Starlette):
        async with mcp_app.router.lifespan_context(mcp_app):
            yield

    routes = [
        Mount("/mcp", app=mcp_app, name="mcp"),
        Route("/config", _web_app),
        Route("/config/{path:path}", _web_app),
        Route("/admin", _web_app),
        Route("/admin/{path:path}", _web_app),
        Mount(
            "/assets",
            app=_AssetFiles(directory=resolved_asset_root / "assets", check_dir=False),
            name="assets",
        ),
        Route("/api/providers", _providers, methods=["GET", "POST"]),
        Route("/api/providers/{provider_id}/models", _provider_models, methods=["GET"]),
        Route("/api/providers/{provider_id}/check", _provider_check, methods=["POST"]),
        Route("/api/providers/{provider_id}", _provider, methods=["GET", "DELETE"]),
        Route("/api/settings/{name}", _settings, methods=["GET", "POST"]),
        Route("/api/admin/dashboard", _admin_dashboard, methods=["GET"]),
        Route("/api/admin/snapshot", _admin_snapshot, methods=["GET"]),
        Route("/api/admin/actions/{action}", _admin_action, methods=["POST"]),
        Route("/api/mcp/{operation}", _mcp_operation, methods=["POST"]),
        WebSocketRoute("/ws/admin", _admin_websocket),
        Route("/health", _health, methods=["GET"]),
        Route("/status", _status, methods=["GET"]),
        Route("/shutdown", _shutdown, methods=["POST"]),
    ]
    app = Starlette(
        routes=routes,
        exception_handlers={404: _not_found, 405: _not_found},
        lifespan=lifespan,
    )
    app.state.runtime = runtime
    app.state.asset_root = resolved_asset_root
    app.state.shutdown_requested = runtime.shutdown_requested
    app.add_middleware(
        _HostValidationMiddleware,
        expected_host=f"{state.host}:{state.port}",
        expected_origin=f"http://{state.host}:{state.port}",
    )
    app.add_middleware(_ExactMcpMountPathMiddleware)
    return app


_SETTINGS_GET_METHODS = {
    "dream": "dream_settings",
    "ingest": "ingest_settings",
    "release": "release_settings",
}

_SETTINGS_SAVE_METHODS = {
    "dream": "save_dream_settings",
    "ingest": "save_ingest_settings",
    "release": "save_release_settings",
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
        "launch_id": state.launch_id,
        "started_at": state.started_at,
        "data_root": str(config.data_root),
        "database_path": str(config.database_path),
        "config_path": str(config.config_root),
        "providers": provider_statuses,
        "providers_error": provider_status_error,
        "dreaming": dreaming_status,
        "mcp_adapter": {"available": True, "mode": "local-http"},
        "mcp_transports": mcp_transport_diagnostics(),
        "housekeeping": {
            "last_cycle": None,
            "pending": int(dreaming_status.get("pending_short_term_memories", 0)) > 0,
        },
    }
