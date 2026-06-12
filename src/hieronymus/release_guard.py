from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

_MODULE_VERSION_RE = re.compile(r'^__version__ = "([^"]+)"$', re.MULTILINE)
_ALPHA_VERSION_RE = re.compile(r"0\.(0|[1-9]\d*)\.(0|[1-9]\d*)")


class ReleaseGuardError(RuntimeError):
    pass


def _module_version(root: Path) -> str:
    init_path = root / "src" / "hieronymus" / "__init__.py"
    match = _MODULE_VERSION_RE.search(init_path.read_text(encoding="utf-8"))
    if match is None:
        raise ReleaseGuardError("src/hieronymus/__init__.py does not define __version__")
    return match.group(1)


def validate_alpha_version(version_text: str) -> str:
    version = version_text.strip().removeprefix("v")
    if _ALPHA_VERSION_RE.fullmatch(version) is None:
        raise ReleaseGuardError(
            f"alpha releases must stay on 0.x semantic versions until a stable release is "
            f"approved; found {version}"
        )
    return version


def validate_alpha_release_metadata(root: Path) -> str:
    pyproject = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8"))
    project_version = str(pyproject["project"]["version"])
    module_version = _module_version(root)

    if project_version != module_version:
        raise ReleaseGuardError(
            f"version mismatch: pyproject.toml has {project_version}, "
            f"src/hieronymus/__init__.py has {module_version}"
        )
    return validate_alpha_version(project_version)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--version")
    args = parser.parse_args(argv)

    try:
        version = (
            validate_alpha_version(args.version)
            if args.version is not None
            else validate_alpha_release_metadata(args.root)
        )
    except ReleaseGuardError as error:
        print(f"release guard failed: {error}", file=sys.stderr)
        return 1

    print(f"release guard passed: v{version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
