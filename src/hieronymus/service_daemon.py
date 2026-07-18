from __future__ import annotations

import argparse
import logging
import os
import signal
import threading
from datetime import UTC, datetime
from typing import Protocol

import uvicorn

from hieronymus.config import HieronymusConfig, load_config
from hieronymus.dream_autostart import DreamAutostart
from hieronymus.presentation import package_version
from hieronymus.service_app import build_app
from hieronymus.service_config import load_service_config
from hieronymus.service_logging import configure_daemon_logging
from hieronymus.service_state import (
    ServerState,
    process_start_identity,
    remove_server_state,
    write_server_state,
)
from hieronymus.session_lifecycle import SessionLifecycle

LOGGER = logging.getLogger(__name__)


class _DreamAutostartFactory(Protocol):
    def __call__(self, config: HieronymusConfig) -> DreamAutostart: ...


class DreamAutostartScheduler:
    def __init__(
        self,
        config: HieronymusConfig,
        *,
        interval_seconds: float = 60.0,
        join_timeout: float = 5.0,
        autostart_cls: _DreamAutostartFactory = DreamAutostart,
    ) -> None:
        self._config = config
        self._interval_seconds = interval_seconds
        self._autostart_cls = autostart_cls
        self._join_timeout = join_timeout
        self._stop = threading.Event()
        self._thread = threading.Thread(
            target=self._run,
            name="hieronymus-dream-autostart",
            daemon=True,
        )

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> bool:
        self._stop.set()
        if self._thread.ident is None or threading.current_thread() is self._thread:
            return True
        self._thread.join(timeout=self._join_timeout)
        if self._thread.is_alive():
            LOGGER.error(
                "Dream autostart scheduler did not stop within %.3f seconds",
                self._join_timeout,
            )
            return False
        return True

    def is_alive(self) -> bool:
        return self._thread.is_alive()

    def _run(self) -> None:
        while not self._stop.is_set():
            try:
                autostart = self._autostart_cls(self._config)
                SessionLifecycle(
                    self._config,
                    threshold_check=getattr(autostart, "run_threshold_now", lambda: None),
                ).run_due()
                autostart.run_due()
            except Exception:
                LOGGER.exception(
                    "Dream autostart run_due failed for %s with config %s",
                    self._autostart_cls,
                    self._config,
                )
            self._stop.wait(self._interval_seconds)


class _ServerProtocol(Protocol):
    should_exit: bool


class _SchedulerProtocol(Protocol):
    def stop(self) -> bool: ...


class ShutdownCoordinator:
    def __init__(
        self,
        config: HieronymusConfig,
        server: _ServerProtocol,
        scheduler: _SchedulerProtocol,
        state: ServerState | None,
    ) -> None:
        self._config = config
        self._server = server
        self._scheduler = scheduler
        self._state = state
        self._lock = threading.Lock()
        self._requested = False
        self._finished = False

    def request(self, source: str) -> None:
        with self._lock:
            if not self._requested:
                LOGGER.info("Daemon shutdown requested by %s", source)
                self._requested = True
            self._server.should_exit = True

    def finish(self, source: str) -> None:
        with self._lock:
            if self._finished:
                return
            self._finished = True
        LOGGER.info("Finishing daemon shutdown after %s", source)
        self._scheduler.stop()
        if self._state is not None:
            remove_server_state(self._config, expected_state=self._state)


class CoordinatedServer(uvicorn.Server):
    shutdown_coordinator: ShutdownCoordinator | None = None

    def handle_exit(self, sig: int, frame: object | None) -> None:
        if self.shutdown_coordinator is None:
            super().handle_exit(sig, frame)
            return
        try:
            source = signal.Signals(sig).name.lower()
        except ValueError:
            source = f"signal-{sig}"
        self.shutdown_coordinator.request(source)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="python -m hieronymus.service_daemon")
    parser.add_argument("--data-root", default=None)
    parser.add_argument("--host", default=None)
    parser.add_argument("--port", type=int, default=None)
    return parser


def main(argv: list[str] | None = None) -> None:
    args = build_parser().parse_args(argv)
    config = load_config(args.data_root)
    config.data_root.mkdir(parents=True, exist_ok=True)
    log_path = configure_daemon_logging(config)
    service_config = load_service_config(config, host=args.host, port=args.port)
    LOGGER.info(
        "Starting Hieronymus daemon pid=%d at %s:%d; log=%s",
        os.getpid(),
        service_config.host,
        service_config.port,
        log_path,
    )
    state = ServerState(
        pid=os.getpid(),
        host=service_config.host,
        port=service_config.port,
        version=package_version(),
        started_at=datetime.now(UTC).isoformat(),
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        process_identity=process_start_identity(os.getpid()),
    )
    app = build_app(config, state)
    server = CoordinatedServer(
        uvicorn.Config(
            app,
            host=state.host,
            port=state.port,
            access_log=False,
            log_config=None,
            timeout_graceful_shutdown=5.0,
        )
    )
    write_server_state(config, state)
    dream_scheduler = DreamAutostartScheduler(config)
    coordinator = ShutdownCoordinator(config, server, dream_scheduler, state)
    server.shutdown_coordinator = coordinator
    app.state.runtime.request_shutdown = lambda: coordinator.request("http")
    dream_scheduler.start()
    try:
        server.run()
    finally:
        coordinator.finish("uvicorn")


if __name__ == "__main__":
    main()
