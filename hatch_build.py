from pathlib import Path
from subprocess import run

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version: str, build_data: dict[str, object]) -> None:
        frontend = Path(self.root) / "frontend"

        run(
            "mise exec bun@1.3.14 -- bun install --frozen-lockfile".split(),
            check=True,
            cwd=frontend,
        )
        run(
            "mise exec bun@1.3.14 -- bun run build".split(),
            check=True,
            cwd=frontend,
        )
