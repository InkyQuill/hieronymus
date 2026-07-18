from pathlib import Path
from subprocess import run

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version: str, build_data: dict[str, object]) -> None:
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
