from __future__ import annotations

import re
from dataclasses import dataclass
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path

GITHUB_REPO_URL = "https://github.com/InkyQuill/hieronymus.git"
MANAGED_APP_PATH = Path("~/.local/share/hieronymus/app")
_TAG_RE = re.compile(
    r"(?:refs/tags/)?"
    r"(v(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*))$"
)


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


def latest_stable_tag(raw_tags: list[str]) -> str | None:
    parsed = [tag for raw_tag in raw_tags if (tag := parse_release_tag(raw_tag)) is not None]
    if not parsed:
        return None
    return max(parsed, key=lambda tag: tag.version).name
