from __future__ import annotations

import argparse
import os
import secrets
from datetime import UTC, datetime

from hieronymus.config import load_config
from hieronymus.presentation import package_version
from hieronymus.service_http import build_server
from hieronymus.service_state import (
    ServerState,
    remove_server_state,
    write_server_state,
)


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
    try:
        from hieronymus.dream_autostart import DreamAutostart

        DreamAutostart(config).run_due()
    except Exception:
        pass
    try:
        server.serve_forever()
    finally:
        server.server_close()
        remove_server_state(config, expected_state=state)


if __name__ == "__main__":
    main()
