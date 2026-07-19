from __future__ import annotations

from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.service_config import ServiceConfig, ServiceConfigError, load_service_config


def test_service_config_uses_stable_loopback_defaults(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    service = load_service_config(config, environ={})

    assert service == ServiceConfig(host="127.0.0.1", port=9768)


def test_service_config_file_overrides_defaults(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    (config.data_root / "service.conf").write_text(
        '[service]\nhost = "192.0.2.10"\nport = 19768\n',
        encoding="utf-8",
    )

    service = load_service_config(config, environ={})

    assert service == ServiceConfig(host="192.0.2.10", port=19768)


def test_service_environment_overrides_service_config(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    (config.data_root / "service.conf").write_text(
        '[service]\nhost = "192.0.2.10"\nport = 19768\n',
        encoding="utf-8",
    )

    service = load_service_config(
        config,
        environ={"HIERONYMUS_HOST": "198.51.100.20", "HIERONYMUS_PORT": "29768"},
    )

    assert service == ServiceConfig(host="198.51.100.20", port=29768)


def test_explicit_daemon_arguments_override_environment(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    service = load_service_config(
        config,
        environ={"HIERONYMUS_HOST": "198.51.100.20", "HIERONYMUS_PORT": "29768"},
        host="203.0.113.30",
        port=39768,
    )

    assert service == ServiceConfig(host="203.0.113.30", port=39768)


@pytest.mark.parametrize("port", ["", "not-a-port", "0", "-1", "65536"])
def test_service_config_rejects_invalid_environment_ports(tmp_path: Path, port: str) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(ServiceConfigError, match="HIERONYMUS_PORT"):
        load_service_config(config, environ={"HIERONYMUS_PORT": port})


@pytest.mark.parametrize("host", ["", "   "])
def test_service_config_rejects_blank_hosts(tmp_path: Path, host: str) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(ServiceConfigError, match="host"):
        load_service_config(config, environ={}, host=host)


def test_service_config_rejects_ipv6_host_from_config_file(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    (config.data_root / "service.conf").write_text(
        '[service]\nhost = "::1"\n',
        encoding="utf-8",
    )

    with pytest.raises(ServiceConfigError, match="IPv6 hosts are not supported"):
        load_service_config(config, environ={})


def test_service_config_rejects_ipv6_host_from_environment(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(ServiceConfigError, match="IPv6 hosts are not supported"):
        load_service_config(config, environ={"HIERONYMUS_HOST": "::1"})


def test_service_config_rejects_ipv6_host_from_explicit_argument(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(ServiceConfigError, match="IPv6 hosts are not supported"):
        load_service_config(config, environ={}, host="::1")


def test_service_config_rejects_explicit_port_zero(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with pytest.raises(ServiceConfigError, match="port"):
        load_service_config(config, environ={}, port=0)
