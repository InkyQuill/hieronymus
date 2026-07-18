import re
from pathlib import Path
from shutil import which
from subprocess import CalledProcessError, run

from hatchling.builders.hooks.plugin.interface import BuildHookInterface

MINIMUM_BUN_VERSION = (1, 3, 0)
BUN_VERSION_PATTERN = re.compile(r"(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?")


def validate_bun_version() -> None:
    try:
        result = run(
            ["bun", "--version"],
            capture_output=True,
            check=True,
            text=True,
        )
    except (CalledProcessError, OSError) as error:
        raise RuntimeError(
            "Bun >=1.3 is required to build the Hieronymus web console; "
            "could not validate the installed Bun version"
        ) from error

    version = result.stdout.strip()
    match = BUN_VERSION_PATTERN.fullmatch(version)
    if match is None:
        raise RuntimeError(
            "Bun >=1.3 is required to build the Hieronymus web console; "
            "could not validate the installed Bun version"
        )

    parsed_version = tuple(int(part) for part in match.groups())
    if parsed_version < MINIMUM_BUN_VERSION:
        raise RuntimeError(
            "Bun >=1.3 is required to build the Hieronymus web console; "
            f"installed Bun version {version} is unsupported"
        )


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version: str, build_data: dict[str, object]) -> None:
        if which("bun") is None:
            raise RuntimeError(
                "Bun >=1.3 is required to build the Hieronymus web console; "
                "install it and ensure `bun` is on PATH"
            )

        validate_bun_version()

        frontend = Path(self.root) / "frontend"

        run(
            "bun install --frozen-lockfile".split(),
            check=True,
            cwd=frontend,
        )
        run(
            "bun run build".split(),
            check=True,
            cwd=frontend,
        )
