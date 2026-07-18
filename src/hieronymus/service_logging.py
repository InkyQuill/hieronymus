from __future__ import annotations

import logging
from logging.handlers import RotatingFileHandler
from pathlib import Path
from typing import TextIO

from hieronymus.config import HieronymusConfig

LOG_FILENAME = "daemon.log"
LOG_MAX_BYTES = 1_048_576
LOG_BACKUP_COUNT = 3


def daemon_log_path(config: HieronymusConfig) -> Path:
    return config.data_root / LOG_FILENAME


def open_early_daemon_log(config: HieronymusConfig) -> TextIO:
    config.data_root.mkdir(parents=True, exist_ok=True)
    return daemon_log_path(config).open("a", encoding="utf-8")


def configure_daemon_logging(config: HieronymusConfig) -> Path:
    path = daemon_log_path(config)
    config.data_root.mkdir(parents=True, exist_ok=True)
    handler = RotatingFileHandler(
        path,
        maxBytes=LOG_MAX_BYTES,
        backupCount=LOG_BACKUP_COUNT,
        encoding="utf-8",
    )
    handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(name)s: %(message)s"))
    root = logging.getLogger()
    for existing in list(root.handlers):
        root.removeHandler(existing)
        existing.close()
    root.addHandler(handler)
    root.setLevel(logging.INFO)
    return path
