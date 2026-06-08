from __future__ import annotations

import argparse
import logging
import os
import secrets
import threading
from datetime import UTC, datetime
from typing import Protocol

from hieronymus.config import HieronymusConfig, load_config
from hieronymus.dream_autostart import DreamAutostart
from hieronymus.presentation import package_version
from hieronymus.service_http import build_server
from hieronymus.service_state import (
    ServerState,
    remove_server_state,
    write_server_state,
)

LOGGER = logging.getLogger(__name__)


class _DreamAutostartFactory(Protocol):
    def __call__(self, config: HieronymusConfig) -> DreamAutostart: ...


class DreamAutostartScheduler:
    def __init__(
        self,
        config: HieronymusConfig,
        *,
        interval_seconds: float = 60.0,
        autostart_cls: _DreamAutostartFactory = DreamAutostart,
    ) -> None:
        self._config = config
        self._interval_seconds = interval_seconds
        self._autostart_cls = autostart_cls
        self._stop = threading.Event()
        self._thread = threading.Thread(
            target=self._run,
            name="hieronymus-dream-autostart",
        )

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        if self._thread.ident is None or threading.current_thread() is self._thread:
            return
        self._thread.join()

    def is_alive(self) -> bool:
        return self._thread.is_alive()

    def _run(self) -> None:
        while not self._stop.is_set():
            try:
                self._autostart_cls(self._config).run_due()
            except Exception:
                LOGGER.exception(
                    "Dream autostart run_due failed for %s with config %s",
                    self._autostart_cls,
                    self._config,
                )
            self._stop.wait(self._interval_seconds)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="python -m hieronymus.service_daemon")
    parser.add_argument("--data-root", default=None)
    parser.add_argument("--port", type=int, default=0)
    return parser


def main(argv: list[str] | None = None) -> None:
    args = build_parser().parse_args(argv)
    config = load_config(args.data_root)
    config.data_root.mkdir(parents=True, exist_ok=True)
    state = ServerState(
        pid=os.getpid(),
        host="127.0.0.1",
        port=args.port if args.port > 0 else 0,
        version=package_version(),
        started_at=datetime.now(UTC).isoformat(),
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token=secrets.token_hex(16),
    )
    server = build_server(config, state)
    state = server.state
    write_server_state(config, state)
    dream_scheduler = DreamAutostartScheduler(config)
    dream_scheduler.start()
    try:
        server.serve_forever()
    finally:
        dream_scheduler.stop()
        server.server_close()
        remove_server_state(config, expected_state=state)


if __name__ == "__main__":
    main()
