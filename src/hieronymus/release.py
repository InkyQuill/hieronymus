from __future__ import annotations

import re
import subprocess
from dataclasses import dataclass
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path

GITHUB_REPO_URL = "https://github.com/InkyQuill/hieronymus.git"
MANAGED_APP_PATH = Path("~/.local/share/hieronymus/app")
_TAG_RE = re.compile(
    r"(?:refs/tags/)?"
    r"(v(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*))$"
)
_VALID_TARGETS = frozenset({"latest", "main"})


@dataclass(frozen=True, order=True)
class ReleaseTag:
    version: tuple[int, int, int]
    name: str


@dataclass(frozen=True)
class UpdateStatus:
    current_version: str
    latest_version: str | None
    latest_tag: str | None
    update_available: bool
    managed_checkout: Path
    managed_install: bool
    target: str

    def as_dict(self) -> dict[str, object]:
        return {
            "current_version": self.current_version,
            "latest_version": self.latest_version,
            "latest_tag": self.latest_tag,
            "update_available": self.update_available,
            "managed_checkout": str(self.managed_checkout),
            "managed_install": self.managed_install,
            "target": self.target,
        }


def managed_app_path() -> Path:
    return MANAGED_APP_PATH.expanduser()


def package_version() -> str:
    try:
        return version("hieronymus")
    except PackageNotFoundError:
        from hieronymus import __version__

        return __version__


def _version_tuple(version_text: str) -> tuple[int, int, int]:
    major, minor, patch = version_text.removeprefix("v").split(".", maxsplit=2)
    return int(major), int(minor), int(patch)


def parse_release_tag(raw_tag: str) -> ReleaseTag | None:
    tag = raw_tag.strip().removesuffix("^{}")
    match = _TAG_RE.fullmatch(tag)
    if match is None:
        return None
    version_tuple = (
        int(match.group("major")),
        int(match.group("minor")),
        int(match.group("patch")),
    )
    return ReleaseTag(version=version_tuple, name=match.group(1))


def _tag_version(tag: str | None) -> str | None:
    if tag is None:
        return None
    release_tag = parse_release_tag(tag)
    if release_tag is None:
        return None
    return ".".join(str(part) for part in release_tag.version)


def latest_stable_tag(raw_tags: list[str]) -> str | None:
    parsed = [tag for raw_tag in raw_tags if (tag := parse_release_tag(raw_tag)) is not None]
    if not parsed:
        return None
    return max(parsed, key=lambda tag: tag.version).name


def _run(command: list[str], *, cwd: Path | None = None) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def _output(command: list[str], *, cwd: Path | None = None) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def fetch_remote_tags(repo_url: str = GITHUB_REPO_URL) -> list[str]:
    result = subprocess.run(
        ["git", "ls-remote", "--tags", repo_url],
        check=True,
        capture_output=True,
        text=True,
    )
    return [
        line.split(maxsplit=1)[1]
        for line in result.stdout.splitlines()
        if len(line.split(maxsplit=1)) == 2
    ]


def _validate_target(target: str) -> None:
    if target not in _VALID_TARGETS:
        valid_targets = ", ".join(sorted(_VALID_TARGETS))
        raise ValueError(f"Unsupported update target {target!r}; expected one of: {valid_targets}.")


def is_managed_install(checkout: Path | None = None) -> bool:
    app_checkout = checkout if checkout is not None else managed_app_path()
    if app_checkout != managed_app_path() or not (app_checkout / ".git").exists():
        return False
    try:
        return _checkout_origin_url(app_checkout) == GITHUB_REPO_URL
    except subprocess.CalledProcessError:
        return False


def _checkout_origin_url(checkout: Path) -> str:
    return _output(["git", "remote", "get-url", "origin"], cwd=checkout)


def check_update(*, target: str = "latest") -> UpdateStatus:
    _validate_target(target)
    current_version = package_version()
    checkout = managed_app_path()
    managed_install = is_managed_install(checkout)

    if target == "latest":
        latest_tag = latest_stable_tag(fetch_remote_tags())
    else:
        latest_tag = target

    latest_version = _tag_version(latest_tag)
    update_available = False
    if latest_version is not None:
        update_available = _version_tuple(latest_version) > _version_tuple(current_version)
    elif latest_tag == "main":
        update_available = True

    return UpdateStatus(
        current_version=current_version,
        latest_version=latest_version,
        latest_tag=latest_tag,
        update_available=update_available,
        managed_checkout=checkout,
        managed_install=managed_install,
        target=target,
    )


def _checkout_update_target(target: str, latest_tag: str, checkout: Path) -> None:
    if target == "main":
        _run(["git", "fetch", GITHUB_REPO_URL, "main"], cwd=checkout)
        _run(["git", "checkout", "--detach", "FETCH_HEAD"], cwd=checkout)
        return

    _run(["git", "fetch", "--force", GITHUB_REPO_URL, f"refs/tags/{latest_tag}"], cwd=checkout)
    _run(["git", "checkout", "--detach", "FETCH_HEAD"], cwd=checkout)


def run_update(*, target: str = "latest") -> UpdateStatus:
    _validate_target(target)
    checkout = managed_app_path()
    if not is_managed_install(checkout):
        raise RuntimeError("Updates require installation through the managed installer.")

    status = check_update(target=target)
    if status.latest_tag is None:
        return status
    if target == "latest" and not status.update_available:
        return status

    _checkout_update_target(target, status.latest_tag, checkout)
    _run(["uv", "tool", "install", "--force", str(checkout)])
    return check_update(target=target)
