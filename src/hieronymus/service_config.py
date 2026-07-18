from __future__ import annotations

import os
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any

from hieronymus.config import HieronymusConfig

DEFAULT_SERVICE_HOST = "127.0.0.1"
DEFAULT_SERVICE_PORT = 9768


class ServiceConfigError(ValueError):
    """Raised when the daemon address configuration is invalid."""


@dataclass(frozen=True)
class ServiceConfig:
    host: str = DEFAULT_SERVICE_HOST
    port: int = DEFAULT_SERVICE_PORT


def load_service_config(
    config: HieronymusConfig,
    *,
    environ: Mapping[str, str] | None = None,
    host: str | None = None,
    port: int | None = None,
) -> ServiceConfig:
    service = _load_service_config_file(config)
    environment = os.environ if environ is None else environ
    if "HIERONYMUS_HOST" in environment:
        service = ServiceConfig(host=environment["HIERONYMUS_HOST"], port=service.port)
    if "HIERONYMUS_PORT" in environment:
        service = ServiceConfig(
            host=service.host,
            port=_parse_environment_port(environment["HIERONYMUS_PORT"]),
        )
    if host is not None:
        service = ServiceConfig(host=host, port=service.port)
    if port is not None:
        service = ServiceConfig(host=service.host, port=port)
    return _validate_service_config(service)


def _load_service_config_file(config: HieronymusConfig) -> ServiceConfig:
    path = config.config_root / "service.conf"
    if not path.exists():
        return ServiceConfig()
    try:
        payload = tomllib.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise ServiceConfigError(f"service.conf could not be read: {error}") from error
    except tomllib.TOMLDecodeError as error:
        raise ServiceConfigError(f"service.conf is not valid TOML: {error}") from error
    service_payload = payload.get("service", {})
    if type(service_payload) is not dict:
        raise ServiceConfigError("service must be a table")
    return _service_config_from_payload(service_payload)


def _service_config_from_payload(payload: dict[str, Any]) -> ServiceConfig:
    for key in payload:
        if key not in {"host", "port"}:
            raise ServiceConfigError(f"unknown service config setting: service.{key}")
    return _validate_service_config(
        ServiceConfig(
            host=payload.get("host", DEFAULT_SERVICE_HOST),
            port=payload.get("port", DEFAULT_SERVICE_PORT),
        )
    )


def _parse_environment_port(value: str) -> int:
    try:
        port = int(value)
    except ValueError as error:
        raise ServiceConfigError("HIERONYMUS_PORT must be an integer from 1 to 65535") from error
    if not 1 <= port <= 65535:
        raise ServiceConfigError("HIERONYMUS_PORT must be an integer from 1 to 65535")
    return port


def _validate_service_config(config: ServiceConfig) -> ServiceConfig:
    if type(config.host) is not str or not config.host.strip():
        raise ServiceConfigError("service host must be a non-blank string")
    if ":" in config.host:
        raise ServiceConfigError("IPv6 hosts are not supported; use an IPv4 address or hostname")
    if type(config.port) is not int or not 1 <= config.port <= 65535:
        raise ServiceConfigError("service port must be an integer from 1 to 65535")
    return config
