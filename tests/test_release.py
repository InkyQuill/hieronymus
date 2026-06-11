from __future__ import annotations

from importlib.metadata import PackageNotFoundError
from pathlib import Path

import pytest

import hieronymus.release as release
from hieronymus.release import (
    MANAGED_APP_PATH,
    ReleaseTag,
    UpdateStatus,
    check_update,
    latest_stable_tag,
    managed_app_path,
    package_version,
    parse_release_tag,
    run_update,
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

    assert package_version() == "1.0.0"


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


def test_check_update_reports_newer_latest_tag(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(release, "package_version", lambda: "0.1.0")
    monkeypatch.setattr(
        release,
        "fetch_remote_tags",
        lambda: ["refs/tags/v0.1.0", "refs/tags/v0.2.0"],
    )

    status = check_update()

    assert status.current_version == "0.1.0"
    assert status.latest_version == "0.2.0"
    assert status.latest_tag == "v0.2.0"
    assert status.update_available is True
    assert status.target == "latest"


def test_check_update_handles_missing_remote_tags_cleanly(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(release, "package_version", lambda: "0.1.0")
    monkeypatch.setattr(release, "fetch_remote_tags", lambda: [])

    status = check_update()

    assert status.latest_version is None
    assert status.latest_tag is None
    assert status.update_available is False


def test_check_update_rejects_unknown_targets(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(release, "fetch_remote_tags", pytest.fail)

    with pytest.raises(ValueError, match="target"):
        check_update(target="v0.2.0")


def test_run_update_rejects_unmanaged_installs(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(release, "is_managed_install", lambda checkout=None: False)

    with pytest.raises(RuntimeError, match="managed installer"):
        run_update()


def test_run_update_rejects_unknown_targets_before_git(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(release, "is_managed_install", lambda checkout=None: True)
    monkeypatch.setattr(release, "_run", pytest.fail)

    with pytest.raises(ValueError, match="target"):
        run_update(target="v0.2.0")


def test_run_update_skips_current_latest_tag_install(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[tuple[list[str], Path | None]] = []

    def record_run(command: list[str], *, cwd: Path | None = None) -> None:
        commands.append((command, cwd))

    monkeypatch.setattr(release, "is_managed_install", lambda checkout=None: True)
    monkeypatch.setattr(release, "package_version", lambda: "0.2.0")
    monkeypatch.setattr(release, "fetch_remote_tags", lambda: ["refs/tags/v0.2.0"])
    monkeypatch.setattr(release, "_run", record_run)

    status = run_update()

    assert status.update_available is False
    assert commands == []


def test_run_update_fetches_checks_out_and_reinstalls_managed_install(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    checkout = tmp_path / "app"
    commands: list[tuple[list[str], Path | None]] = []

    def record_run(command: list[str], *, cwd: Path | None = None) -> None:
        commands.append((command, cwd))

    monkeypatch.setattr(release, "managed_app_path", lambda: checkout)
    monkeypatch.setattr(release, "is_managed_install", lambda checkout=None: True)
    monkeypatch.setattr(release, "package_version", lambda: "0.1.0")
    monkeypatch.setattr(release, "fetch_remote_tags", lambda: ["refs/tags/v0.2.0"])
    monkeypatch.setattr(release, "_run", record_run)

    status = run_update()

    assert status.latest_tag == "v0.2.0"
    assert commands == [
        (
            [
                "git",
                "fetch",
                "--force",
                release.GITHUB_REPO_URL,
                "refs/tags/v0.2.0",
            ],
            checkout,
        ),
        (["git", "checkout", "--detach", "FETCH_HEAD"], checkout),
        (["uv", "tool", "install", "--force", str(checkout)], None),
    ]


def test_run_update_fetches_origin_main_before_checkout(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    checkout = tmp_path / "app"
    commands: list[tuple[list[str], Path | None]] = []

    def record_run(command: list[str], *, cwd: Path | None = None) -> None:
        commands.append((command, cwd))

    monkeypatch.setattr(release, "managed_app_path", lambda: checkout)
    monkeypatch.setattr(release, "is_managed_install", lambda checkout=None: True)
    monkeypatch.setattr(release, "package_version", lambda: "0.1.0")
    monkeypatch.setattr(release, "_run", record_run)

    status = run_update(target="main")

    assert status.target == "main"
    assert commands == [
        (["git", "fetch", release.GITHUB_REPO_URL, "main"], checkout),
        (["git", "checkout", "--detach", "FETCH_HEAD"], checkout),
        (["uv", "tool", "install", "--force", str(checkout)], None),
    ]


def test_run_update_returns_refreshed_status_after_reinstall(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    checkout = tmp_path / "app"
    versions = iter(["0.1.0", "0.2.0"])

    monkeypatch.setattr(release, "managed_app_path", lambda: checkout)
    monkeypatch.setattr(release, "is_managed_install", lambda checkout=None: True)
    monkeypatch.setattr(release, "package_version", lambda: next(versions))
    monkeypatch.setattr(release, "fetch_remote_tags", lambda: ["refs/tags/v0.2.0"])
    monkeypatch.setattr(release, "_run", lambda command, *, cwd=None: None)

    status = run_update()

    assert status.current_version == "0.2.0"
    assert status.latest_version == "0.2.0"
    assert status.update_available is False


def test_is_managed_install_rejects_wrong_origin(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    checkout = tmp_path / "app"
    (checkout / ".git").mkdir(parents=True)

    monkeypatch.setattr(release, "managed_app_path", lambda: checkout)
    monkeypatch.setattr(
        release, "_checkout_origin_url", lambda _: "https://example.invalid/repo.git"
    )

    assert release.is_managed_install(checkout) is False


def test_is_managed_install_accepts_expected_origin(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    checkout = tmp_path / "app"
    (checkout / ".git").mkdir(parents=True)

    monkeypatch.setattr(release, "managed_app_path", lambda: checkout)
    monkeypatch.setattr(release, "_checkout_origin_url", lambda _: release.GITHUB_REPO_URL)

    assert release.is_managed_install(checkout) is True
