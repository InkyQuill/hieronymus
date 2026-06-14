from __future__ import annotations

from dataclasses import asdict
from pathlib import Path

import click

from hieronymus.agent_context import discover_project_context
from hieronymus.config import load_config
from hieronymus.presentation import render_json
from hieronymus.service_discovery import discover_local_service


@click.group()
def main() -> None:
    pass


@main.command("session-start")
@click.option("--cwd", type=click.Path(file_okay=False, dir_okay=True), default=".")
@click.option("--json", "as_json", is_flag=True)
def session_start(cwd: str, as_json: bool) -> None:
    context = discover_project_context(Path(cwd))
    if context is None:
        payload = {
            "event": "session-start",
            "handled": False,
            "reason": "no .hieronymus.json context found",
        }
    else:
        payload = {
            "event": "session-start",
            "handled": True,
            **asdict(context),
        }

    if as_json:
        payload["service"] = discover_local_service(load_config())
        click.echo(render_json(payload))
        return

    click.echo("Hieronymus context loaded" if payload["handled"] else payload["reason"])


@main.command("session-end")
@click.option("--json", "as_json", is_flag=True)
def session_end(as_json: bool) -> None:
    payload = {"event": "session-end", "handled": True}
    click.echo(render_json(payload) if as_json else "Hieronymus session hook complete")


if __name__ == "__main__":
    main()
