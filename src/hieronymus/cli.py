from __future__ import annotations

import json
from dataclasses import asdict

import click

from hieronymus.config import load_config
from hieronymus.doctor import Doctor, report_to_json
from hieronymus.dreaming import DeterministicDreamProvider, DreamService
from hieronymus.install import plan_install
from hieronymus.memory import MemoryStore
from hieronymus.memory_models import TranslationContext
from hieronymus.presentation import GUIDE_ICON, render_greeting, render_json
from hieronymus.recall import RecallService
from hieronymus.registry import Registry
from hieronymus.scoring import FeedbackStore
from hieronymus.service_manager import ServiceManager
from hieronymus.termbase import Termbase
from hieronymus.workspace import WorkspaceStore


def _error_message(error: KeyError | ValueError) -> str:
    if isinstance(error, KeyError) and error.args:
        return str(error.args[0])
    return str(error)


def _raise_click_error(error: KeyError | ValueError) -> None:
    raise click.ClickException(_error_message(error)) from error


def _context(
    *,
    series_slug: str,
    source_language: str,
    target_language: str,
    task_type: str,
    volume: str,
    chapter: str,
) -> TranslationContext:
    return TranslationContext(
        series_slug=series_slug,
        source_language=source_language,
        target_language=target_language,
        task_type=task_type,
        volume=volume,
        chapter=chapter,
    )


def _validate_provided_match(
    *,
    field_name: str,
    provided: str | None,
    expected: str,
    owner: str,
) -> str:
    if provided is not None and provided != expected:
        raise ValueError(f"{field_name} does not match {owner}: {expected}")
    return expected


def _validate_context_match(
    *,
    provided: TranslationContext,
    expected: TranslationContext,
    owner: str,
) -> None:
    for field_name in (
        "series_slug",
        "source_language",
        "target_language",
        "task_type",
        "volume",
        "chapter",
    ):
        if getattr(provided, field_name) != getattr(expected, field_name):
            raise ValueError(f"{owner} {field_name} does not match session context")


def _series_context(series, *, task_type: str) -> TranslationContext:
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type=task_type,
    )


def _echo_status_lines(status: dict[str, object]) -> None:
    click.echo("running: yes" if status.get("running") else "running: no")
    if "pid" in status:
        click.echo(f"pid: {status['pid']}")
    if "port" in status:
        click.echo(f"port: {status['port']}")


@click.group(invoke_without_command=True)
@click.option("--data-root", type=click.Path(file_okay=False, dir_okay=True), default=None)
@click.pass_context
def main(ctx: click.Context, data_root: str | None) -> None:
    config = load_config(data_root)
    if config.data_root.exists() and not config.data_root.is_dir():
        raise click.ClickException(f"data root is not a directory: {config.data_root}")
    ctx.obj = {"config": config}
    if ctx.invoked_subcommand is None:
        result = ServiceManager(config).ensure_running()
        status = result["status"]
        click.echo(render_greeting())
        click.echo()
        _echo_status_lines(status)


@main.command("status")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def status(ctx: click.Context, json_output: bool) -> None:
    payload = ServiceManager(ctx.obj["config"]).status()
    if json_output:
        click.echo(render_json(payload))
        return

    click.echo(render_greeting())
    click.echo()
    _echo_status_lines(payload)


@main.command("stop")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def stop(ctx: click.Context, json_output: bool) -> None:
    payload = ServiceManager(ctx.obj["config"]).stop()
    if json_output:
        click.echo(render_json(payload))
        return

    click.echo(render_greeting())
    click.echo()
    _echo_status_lines(payload)


@main.command("restart")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def restart(ctx: click.Context, json_output: bool) -> None:
    payload = ServiceManager(ctx.obj["config"]).restart()
    if json_output:
        click.echo(render_json(payload))
        return

    click.echo(render_greeting())
    click.echo()
    _echo_status_lines(payload["status"])


@main.command("config")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def config_command(ctx: click.Context, json_output: bool) -> None:
    config = ctx.obj["config"]
    payload = {
        "config_root": str(config.config_root),
        "database_path": str(config.database_path),
        "tui": "not-available-in-this-pass",
    }
    if json_output:
        click.echo(render_json(payload))
        return

    click.echo(render_greeting())
    click.echo()
    click.echo(f"config_root: {payload['config_root']}")
    click.echo(f"database_path: {payload['database_path']}")
    click.echo(f"tui: {payload['tui']}")


@main.command("install")
@click.argument("app")
@click.option("--dry-run", is_flag=True)
@click.option("--force", is_flag=True)
@click.option("--json", "as_json", is_flag=True)
@click.pass_context
def install_command(
    ctx: click.Context, app: str, dry_run: bool, force: bool, as_json: bool
) -> None:
    try:
        plan = plan_install(ctx.obj["config"], app)
    except ValueError as error:
        raise click.ClickException(str(error)) from error

    payload = plan.to_json_dict()
    payload["dry_run"] = dry_run
    payload["force"] = force
    if as_json:
        click.echo(render_json(payload))
        return

    click.echo(render_greeting())
    click.echo()
    click.echo(f"Planning {plan.display_name} integration")
    click.echo(plan.protocol_note)
    click.echo("Planned changes:")
    for step in plan.steps:
        click.echo(f"- {step.action}: {step.path}")
        click.echo(f"  {step.description}")
    if plan.result_kind == "stub":
        click.echo("Result: stub; real integration is deferred to the agent workflow spec.")


@main.command("admin")
@click.option("--json", "json_output", is_flag=True)
def admin(json_output: bool) -> None:
    payload = {"tui": "not-available-in-this-pass"}
    if json_output:
        click.echo(render_json(payload))
        return

    click.echo(render_greeting())
    click.echo()
    click.echo(f"tui: {payload['tui']}")


@main.command("doctor")
@click.option("--fix", "autofix", is_flag=True)
@click.option("--json", "as_json", is_flag=True)
@click.pass_context
def doctor_command(ctx: click.Context, autofix: bool, as_json: bool) -> None:
    report = Doctor(ctx.obj["config"]).run(autofix=autofix)
    payload = report_to_json(report)
    if as_json:
        click.echo(render_json(payload))
        return

    click.echo(render_greeting())
    click.echo()
    for title, key in (
        ("Autofixed", "autofixed"),
        ("Doctor warnings", "warnings"),
        ("Doctor errors", "errors"),
    ):
        click.echo(f"{title}:")
        findings = payload[key]
        if not findings:
            click.echo("  none")
        for finding in findings:
            click.echo(f"  - {finding['message']}")


@main.command("help")
def help_command() -> None:
    click.echo(render_greeting())
    click.echo()
    click.echo(f"{GUIDE_ICON} Common commands")
    click.echo("  hiero                  Start or connect to the local service")
    click.echo("  hiero status           Show daemon and provider status")
    click.echo("  hiero doctor           Check configuration and service health")
    click.echo("  hiero restart          Restart the local daemon")
    click.echo("  hiero admin            Show admin TUI placeholder")
    click.echo("  hiero config           Show config paths and TUI placeholder")
    click.echo("  hiero install codex --dry-run")


@main.command("init-series")
@click.argument("slug")
@click.option("--title", required=True)
@click.option("--source-language", default="ja")
@click.option("--target-language", default="en")
@click.pass_context
def init_series(
    ctx: click.Context, slug: str, title: str, source_language: str, target_language: str
) -> None:
    try:
        series = Registry(ctx.obj["config"]).create_series(
            slug=slug,
            title=title,
            source_language=source_language,
            target_language=target_language,
        )
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    click.echo(
        json.dumps(
            {"slug": series.slug, "database_path": str(ctx.obj["config"].database_path)},
            ensure_ascii=False,
        )
    )


@main.command("propose-term")
@click.argument("series_slug")
@click.option("--category", required=True)
@click.option("--source", "source_text", required=True)
@click.option("--translation", required=True)
@click.option("--tag", "tags", multiple=True)
@click.pass_context
def propose_term(
    ctx: click.Context,
    series_slug: str,
    category: str,
    source_text: str,
    translation: str,
    tags: tuple[str, ...],
) -> None:
    try:
        series = Registry(ctx.obj["config"]).get_series(series_slug)
        termbase = Termbase(
            ctx.obj["config"],
            _series_context(series, task_type="translation"),
        )
        term_id = termbase.propose(
            category=category,
            source_text=source_text,
            canonical_translation=translation,
            tags=list(tags),
        )
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    click.echo(json.dumps({"term_id": term_id}, ensure_ascii=False))


@main.command("validate")
@click.argument("series_slug")
@click.option(
    "--raw-file", type=click.Path(exists=True, dir_okay=False, readable=True), required=True
)
@click.option(
    "--translated-file",
    type=click.Path(exists=True, dir_okay=False, readable=True),
    required=True,
)
@click.pass_context
def validate(ctx: click.Context, series_slug: str, raw_file: str, translated_file: str) -> None:
    try:
        series = Registry(ctx.obj["config"]).get_series(series_slug)
        with (
            open(raw_file, encoding="utf-8") as raw,
            open(translated_file, encoding="utf-8") as translated,
        ):
            findings = Termbase(
                ctx.obj["config"],
                _series_context(series, task_type="translation"),
            ).validate(
                raw_text=raw.read(),
                translated_text=translated.read(),
            )
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    click.echo(json.dumps([asdict(finding) for finding in findings], ensure_ascii=False, indent=2))


@main.command("remember")
@click.argument("series_slug")
@click.option("--kind", required=True)
@click.option("--text", required=True)
@click.option("--source-ref", default="")
@click.pass_context
def remember(ctx: click.Context, series_slug: str, kind: str, text: str, source_ref: str) -> None:
    try:
        series = Registry(ctx.obj["config"]).get_series(series_slug)
        memory_id = MemoryStore(
            ctx.obj["config"],
            _series_context(series, task_type="translation"),
        ).add(kind=kind, text=text, source_ref=source_ref)
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    click.echo(json.dumps({"memory_id": memory_id}, ensure_ascii=False))


@main.command("session-start")
@click.argument("series_slug")
@click.option("--source-language", default=None)
@click.option("--target-language", default=None)
@click.option("--task-type", required=True)
@click.option("--volume", default="")
@click.option("--chapter", default="")
@click.pass_context
def session_start(
    ctx: click.Context,
    series_slug: str,
    source_language: str,
    target_language: str,
    task_type: str,
    volume: str,
    chapter: str,
) -> None:
    try:
        series = Registry(ctx.obj["config"]).get_series(series_slug)
        resolved_source_language = _validate_provided_match(
            field_name="source_language",
            provided=source_language,
            expected=series.source_language,
            owner="series source_language",
        )
        resolved_target_language = _validate_provided_match(
            field_name="target_language",
            provided=target_language,
            expected=series.target_language,
            owner="series target_language",
        )
        session = WorkspaceStore(ctx.obj["config"]).start_session(
            _context(
                series_slug=series_slug,
                source_language=resolved_source_language,
                target_language=resolved_target_language,
                task_type=task_type,
                volume=volume,
                chapter=chapter,
            )
        )
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    click.echo(json.dumps({"session_id": session.id}, ensure_ascii=False))


@main.command("session-complete")
@click.argument("session_id", type=int)
@click.pass_context
def session_complete(ctx: click.Context, session_id: int) -> None:
    try:
        WorkspaceStore(ctx.obj["config"]).complete_session(session_id)
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    click.echo(json.dumps({"session_id": session_id, "completed": True}, ensure_ascii=False))


@main.command("remember-short")
@click.argument("session_id", type=int)
@click.option("--role", required=True)
@click.option("--kind", required=True)
@click.option("--text", required=True)
@click.option("--source-ref", default="")
@click.pass_context
def remember_short(
    ctx: click.Context,
    session_id: int,
    role: str,
    kind: str,
    text: str,
    source_ref: str,
) -> None:
    try:
        memory_id = WorkspaceStore(ctx.obj["config"]).add_short_term_memory(
            session_id=session_id,
            source_role=role,
            kind=kind,
            text=text,
            source_ref=source_ref,
        )
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    click.echo(json.dumps({"memory_id": memory_id}, ensure_ascii=False))


@main.command("recall")
@click.argument("session_id", type=int)
@click.option("--series", "series_slug", required=True)
@click.option("--query", required=True)
@click.option("--source-language", required=True)
@click.option("--target-language", required=True)
@click.option("--task-type", required=True)
@click.option("--volume", default="")
@click.option("--chapter", default="")
@click.option("--limit", default=10, type=int)
@click.pass_context
def recall(
    ctx: click.Context,
    session_id: int,
    series_slug: str,
    query: str,
    source_language: str,
    target_language: str,
    task_type: str,
    volume: str,
    chapter: str,
    limit: int,
) -> None:
    try:
        Registry(ctx.obj["config"]).get_series(series_slug)
        workspace = WorkspaceStore(ctx.obj["config"])
        session = workspace.get_session(session_id)
        provided_context = _context(
            series_slug=series_slug,
            source_language=source_language,
            target_language=target_language,
            task_type=task_type,
            volume=volume,
            chapter=chapter,
        )
        _validate_context_match(
            provided=provided_context,
            expected=session.context,
            owner="recall",
        )
        results = RecallService(ctx.obj["config"]).recall(
            session_id,
            session.context,
            query,
            limit=limit,
        )
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    click.echo(
        json.dumps(
            [
                {
                    "crystal_id": result.crystal.id,
                    "text": result.crystal.text,
                    "rank": result.rank,
                    "score": result.score,
                    "reason": result.reason,
                }
                for result in results
            ],
            ensure_ascii=False,
        )
    )


@main.command("feedback")
@click.argument("crystal_id", type=int)
@click.option("--event", "event_type", required=True)
@click.option("--role", "source_role", required=True)
@click.option("--evidence", default="")
@click.option("--session-id", type=int, default=None)
@click.pass_context
def feedback(
    ctx: click.Context,
    crystal_id: int,
    event_type: str,
    source_role: str,
    evidence: str,
    session_id: int | None,
) -> None:
    try:
        event_id = FeedbackStore(ctx.obj["config"]).record(
            crystal_id=crystal_id,
            event_type=event_type,
            source_role=source_role,
            evidence=evidence,
            session_id=session_id,
        )
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    click.echo(json.dumps({"event_id": event_id}, ensure_ascii=False))


@main.command("dream")
@click.option("--provider", default="deterministic")
@click.pass_context
def dream(ctx: click.Context, provider: str) -> None:
    if provider != "deterministic":
        raise click.ClickException(f"unsupported dream provider: {provider}")

    try:
        run = DreamService(
            ctx.obj["config"],
            DeterministicDreamProvider(),
        ).run_cycle()
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    click.echo(
        json.dumps(
            {"cycle_id": run.cycle_id, "status": run.status},
            ensure_ascii=False,
        )
    )
