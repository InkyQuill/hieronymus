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
_MIN_BUN_VERSION = (1, 3)


@dataclass(frozen=True, order=True)
class ReleaseTag:
    version: tuple[int, int, int]
    name: str


@dataclass(frozen=True)
class UpdateStatus:
    current_version: str
    latest_version: str | None
    latest_tag: str | None
    current_revision: str | None
    latest_revision: str | None
    update_available: bool
    managed_checkout: Path
    managed_install: bool
    target: str

    def as_dict(self) -> dict[str, object]:
        return {
            "current_version": self.current_version,
            "latest_version": self.latest_version,
            "latest_tag": self.latest_tag,
            "current_revision": self.current_revision,
            "latest_revision": self.latest_revision,
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


def fetch_remote_head(branch: str = "main", repo_url: str = GITHUB_REPO_URL) -> str | None:
    result = subprocess.run(
        ["git", "ls-remote", repo_url, branch],
        check=True,
        capture_output=True,
        text=True,
    )
    first_line = result.stdout.splitlines()[0] if result.stdout.splitlines() else ""
    if not first_line:
        return None
    return _short_revision(first_line.split(maxsplit=1)[0])


def _short_revision(revision: str | None) -> str | None:
    if revision is None:
        return None
    return revision[:7]


def _validate_target(target: str) -> None:
    if target not in _VALID_TARGETS:
        valid_targets = ", ".join(sorted(_VALID_TARGETS))
        raise ValueError(f"Unsupported update target {target!r}; expected one of: {valid_targets}.")


def _validate_dev_target(target: str, *, allow_dev: bool) -> None:
    if target == "main" and not allow_dev:
        raise ValueError("Updating from main is a development action; pass --dev to allow it.")


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


def _checkout_revision(checkout: Path) -> str | None:
    try:
        return _output(["git", "rev-parse", "--short", "HEAD"], cwd=checkout)
    except subprocess.CalledProcessError:
        return None


def _bun_version_tuple(version_text: str) -> tuple[int, int]:
    major_text, minor_text, *_ = version_text.strip().split("-", maxsplit=1)[0].split(".")
    return int(major_text), int(minor_text)


def ensure_bun_available_or_raise(
    min_version: tuple[int, int] = _MIN_BUN_VERSION,
) -> None:
    required = ".".join(str(part) for part in min_version)
    remediation = f"Install or update Bun to >= {required} from https://bun.sh before updating."
    try:
        raw_version = _output(["bun", "--version"])
    except FileNotFoundError as error:
        raise RuntimeError(
            f"Bun >= {required} is required to build the Hieronymus TUI. {remediation}"
        ) from error
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f"Unable to check Bun version. {remediation}") from error

    try:
        version_tuple = _bun_version_tuple(raw_version)
    except (ValueError, IndexError) as error:
        raise RuntimeError(f"Unable to parse Bun version {raw_version!r}. {remediation}") from error

    if version_tuple < min_version:
        raise RuntimeError(
            f"Bun >= {required} is required to build the Hieronymus TUI; found {raw_version}. "
            f"{remediation}"
        )


def check_update(*, target: str = "latest", allow_dev: bool = False) -> UpdateStatus:
    _validate_target(target)
    _validate_dev_target(target, allow_dev=allow_dev)
    current_version = package_version()
    checkout = managed_app_path()
    managed_install = is_managed_install(checkout)
    current_revision = (
        _checkout_revision(checkout) if target == "main" and managed_install else None
    )

    if target == "latest":
        latest_tag = latest_stable_tag(fetch_remote_tags())
        latest_revision = None
    else:
        latest_tag = target
        latest_revision = _short_revision(fetch_remote_head(target))

    latest_version = _tag_version(latest_tag)
    if target == "main":
        if managed_install and current_revision is None:
            raise RuntimeError("Cannot determine current managed checkout revision.")
        if latest_revision is None:
            raise RuntimeError("Cannot determine latest main revision.")

    update_available = False
    if latest_version is not None:
        update_available = _version_tuple(latest_version) > _version_tuple(current_version)
    elif latest_tag == "main":
        update_available = latest_revision != current_revision

    return UpdateStatus(
        current_version=current_version,
        latest_version=latest_version,
        latest_tag=latest_tag,
        current_revision=current_revision,
        latest_revision=latest_revision,
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


def _build_frontend(checkout: Path) -> None:
    _run(["bun", "install", "--cwd", "frontend", "--frozen-lockfile"], cwd=checkout)
    _run(["bun", "run", "--cwd", "frontend", "build"], cwd=checkout)


def run_update(*, target: str = "latest", allow_dev: bool = False) -> UpdateStatus:
    _validate_target(target)
    _validate_dev_target(target, allow_dev=allow_dev)
    checkout = managed_app_path()
    if not is_managed_install(checkout):
        raise RuntimeError("Updates require installation through the managed installer.")

    status = check_update(target=target, allow_dev=allow_dev)
    if status.latest_tag is None:
        return status
    if target == "latest" and not status.update_available:
        return status

    ensure_bun_available_or_raise()
    _checkout_update_target(target, status.latest_tag, checkout)
    _build_frontend(checkout)
    _run(["uv", "tool", "install", "--force", str(checkout)])
    return check_update(target=target, allow_dev=allow_dev)
