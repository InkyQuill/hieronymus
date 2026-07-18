from pathlib import Path
from shutil import which
from subprocess import run

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version: str, build_data: dict[str, object]) -> None:
        if which("bun") is None:
            raise RuntimeError(
                "Bun >=1.3 is required to build the Hieronymus web console; "
                "install it and ensure `bun` is on PATH"
            )

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
