from __future__ import annotations

from importlib.metadata import PackageNotFoundError
from pathlib import Path

import pytest

from hieronymus.release import (
    MANAGED_APP_PATH,
    ReleaseTag,
    UpdateStatus,
    latest_stable_tag,
    managed_app_path,
    package_version,
    parse_release_tag,
)


def test_parse_release_tag_accepts_stable_semver() -> None:
    assert parse_release_tag("v1.2.3") == ReleaseTag(version=(1, 2, 3), name="v1.2.3")
    assert parse_release_tag("refs/tags/v10.20.30") == ReleaseTag(
        version=(10, 20, 30),
        name="v10.20.30",
    )
    assert parse_release_tag("refs/tags/v2.3.4^{}") == ReleaseTag(
        version=(2, 3, 4),
        name="v2.3.4",
    )


def test_release_tag_orders_by_semantic_version() -> None:
    older = ReleaseTag(version=(9, 0, 0), name="v9.0.0")
    newer = ReleaseTag(version=(10, 0, 0), name="v10.0.0")

    assert older < newer


@pytest.mark.parametrize("tag", ["1.2.3", "v1.2", "v1.2.3-rc.1", "not-a-tag"])
def test_parse_release_tag_rejects_non_stable_tags(tag: str) -> None:
    assert parse_release_tag(tag) is None


def test_latest_stable_tag_selects_highest_version() -> None:
    assert (
        latest_stable_tag(["refs/tags/v0.2.0", "refs/tags/v0.10.0", "refs/tags/v0.9.9"])
        == "v0.10.0"
    )


def test_latest_stable_tag_returns_none_without_stable_tags() -> None:
    assert latest_stable_tag(["refs/tags/v0.2.0-rc.1", "refs/heads/main"]) is None


def test_managed_app_path_uses_home_by_default(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))

    assert managed_app_path() == tmp_path / ".local" / "share" / "hieronymus" / "app"
    assert MANAGED_APP_PATH == Path("~/.local/share/hieronymus/app")


def test_package_version_falls_back_to_module_version(monkeypatch: pytest.MonkeyPatch) -> None:
    def missing_version(_: str) -> str:
        raise PackageNotFoundError

    monkeypatch.setattr("hieronymus.release.version", missing_version)

    assert package_version() == "0.1.0"


def test_update_status_as_dict_serializes_cli_payload(tmp_path: Path) -> None:
    status = UpdateStatus(
        current_version="0.1.0",
        latest_version="0.2.0",
        latest_tag="v0.2.0",
        update_available=True,
        managed_checkout=tmp_path / "app",
        managed_install=True,
        target="latest",
    )

    assert status.as_dict() == {
        "current_version": "0.1.0",
        "latest_version": "0.2.0",
        "latest_tag": "v0.2.0",
        "update_available": True,
        "managed_checkout": str(tmp_path / "app"),
        "managed_install": True,
        "target": "latest",
    }
