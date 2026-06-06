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
    allocate_loopback_port,
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
    port = args.port if args.port > 0 else allocate_loopback_port()
    state = ServerState(
        pid=os.getpid(),
        host="127.0.0.1",
        port=port,
        version=package_version(),
        started_at=datetime.now(UTC).isoformat(),
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token=secrets.token_hex(16),
    )
    write_server_state(config, state)
    server = build_server(config, state)
    try:
        server.serve_forever()
    finally:
        server.server_close()
        remove_server_state(config)


if __name__ == "__main__":
    main()
