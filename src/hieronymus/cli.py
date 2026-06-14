from __future__ import annotations

import os
import subprocess
import sys
from dataclasses import asdict
from pathlib import Path

import click

from hieronymus.admin import AdminStore
from hieronymus.agent_plugins import resolve_plugin
from hieronymus.config import load_config
from hieronymus.doctor import Doctor, report_to_json
from hieronymus.dream_autostart import DreamAutostart
from hieronymus.dream_config import (
    DreamConfigError,
    load_dream_config,
    redacted_dream_config_payload,
)
from hieronymus.dream_locks import DreamCycleAlreadyRunning
from hieronymus.dream_providers import ProviderRegistry, resolve_provider
from hieronymus.dreaming import DreamService
from hieronymus.ingest_config import IngestConfigError, load_ingest_config
from hieronymus.install import agent_install_candidates
from hieronymus.memory import MemoryStore
from hieronymus.memory_models import CrystalRecord, ShortTermMemoryRecord, TranslationContext
from hieronymus.presentation import GUIDE_ICON, display_version, render_greeting, render_json
from hieronymus.recall import RecallService
from hieronymus.registry import Registry
from hieronymus.release import check_update, run_update
from hieronymus.release_config import ReleaseConfigError, load_release_config
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


def _crystal_payload(crystal: CrystalRecord | None) -> dict[str, object] | None:
    if crystal is None:
        return None
    return {
        "id": crystal.id,
        "crystal_type": crystal.crystal_type,
        "text": crystal.text,
        "title": crystal.title,
        "confidence": crystal.confidence,
        "strength": crystal.strength,
        "status": crystal.status,
        "source_credibility": crystal.source_credibility,
        "rule_intent": crystal.rule_intent,
        "story_scopes": list(crystal.story_scopes),
        "semantic_tags": list(crystal.semantic_tags),
        "concept_ids": list(crystal.concept_ids),
    }


def _short_term_memory_payload(memory: ShortTermMemoryRecord | None) -> dict[str, object] | None:
    if memory is None:
        return None
    return {
        "id": memory.id,
        "source_role": memory.source_role,
        "kind": memory.kind,
        "text": memory.text,
        "metadata": memory.metadata,
    }


def _echo_json_or_line(payload: object, *, json_output: bool, line: str) -> None:
    if json_output:
        click.echo(render_json(payload))
        return
    click.echo(line)


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


def _subprocess_error_message(error: subprocess.CalledProcessError) -> str:
    command = " ".join(str(part) for part in error.cmd)
    return f"Update command failed: {command} exited with code {error.returncode}"


def _update_status_current_display(status) -> str:
    if status.target == "main" and status.current_revision:
        return str(status.current_revision)
    return display_version(str(status.current_version))


def _update_status_target_display(status) -> str:
    if status.latest_version is not None:
        return display_version(str(status.latest_version))
    if status.target == "main" and status.latest_revision:
        return str(status.latest_revision)
    return str(status.latest_tag or "unknown")


def _frontend_entrypoint() -> str:
    candidate = Path(__file__).resolve().parent / "frontend" / "dist" / "main.js"
    searched = [candidate]
    if candidate.exists():
        return str(candidate)
    for ancestor in Path(__file__).resolve().parents[:5]:
        repo_candidate = ancestor / "frontend" / "dist" / "main.js"
        searched.append(repo_candidate)
        if repo_candidate.exists():
            return str(repo_candidate)
    searched_paths = ", ".join(str(path) for path in searched)
    raise FileNotFoundError(f"OpenTUI frontend bundle not found; looked for: {searched_paths}")


def _launch_opentui(mode: str, *, data_root: Path) -> None:
    command = [
        "bun",
        _frontend_entrypoint(),
        mode,
        "--bridge-command",
        sys.executable,
        "--bridge-arg",
        "-m",
        "--bridge-arg",
        "hieronymus",
    ]
    env = {**os.environ, "HIERONYMUS_DATA_ROOT": str(data_root)}
    try:
        subprocess.run(command, check=True, env=env)
    except FileNotFoundError as error:
        raise click.ClickException("OpenTUI launch failed: bun executable not found") from error
    except subprocess.CalledProcessError as error:
        raise click.ClickException(f"OpenTUI exited with code {error.returncode}") from error


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


@main.command("tui-bridge", hidden=True)
@click.pass_context
def tui_bridge_command(ctx: click.Context) -> None:
    from hieronymus.tui_bridge.server import run_stdio

    run_stdio(ctx.obj["config"])


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


@main.command("config", help="Open the configuration TUI.")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def config_command(ctx: click.Context, json_output: bool) -> None:
    config = ctx.obj["config"]
    if not json_output:
        _launch_opentui("config", data_root=config.data_root)
        return

    try:
        dream_config = load_dream_config(config)
        release_config = load_release_config(config)
        ingest_config = load_ingest_config(config)
        payload = {
            "config_root": str(config.config_root),
            "database_path": str(config.database_path),
            "dream_config_path": str(config.dream_config_path),
            "ingest_config_path": str(config.ingest_config_path),
            "release_config_path": str(config.release_config_path),
            "tui": "available",
            "dream": redacted_dream_config_payload(dream_config),
            "ingest": ingest_config.to_payload(),
            "release": {
                "update_channel": release_config.update_channel,
                "update_target": release_config.update_target,
            },
            "providers": ProviderRegistry().status_payload(config),
            "dreaming": DreamAutostart(config).status(),
        }
    except (DreamConfigError, IngestConfigError, ReleaseConfigError) as error:
        raise click.ClickException(str(error)) from error

    click.echo(render_json(payload))


@main.command("install")
@click.argument("app", required=False)
@click.option("--dry-run", is_flag=True)
@click.option("--force", is_flag=True)
@click.option("--json", "as_json", is_flag=True)
@click.pass_context
def install_command(
    ctx: click.Context, app: str | None, dry_run: bool, force: bool, as_json: bool
) -> None:
    config = ctx.obj["config"]
    if app is None or app == "list":
        payload = {"candidates": agent_install_candidates(config)}
        if as_json:
            click.echo(render_json(payload))
            return

        click.echo(render_greeting())
        click.echo()
        click.echo("Agent integration candidates:")
        for item in payload["candidates"]:
            available = "available" if item["available"] else "not found"
            installed = "installed" if item["installed"] else "not installed"
            click.echo(f"- {item['display_name']}: {available}, {installed}")
        return

    try:
        plugin = resolve_plugin(app)
        plan = plugin.plan(config) if dry_run else plugin.install(config, force=force)
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
    installed = plan.result_kind == "installed"
    action_label = "Installed" if installed else "Planning"
    changes_label = "Applied changes" if installed else "Planned changes"
    click.echo(f"{action_label} {plan.display_name} integration")
    click.echo(plan.protocol_note)
    click.echo(f"{changes_label}:")
    for step in plan.steps:
        click.echo(f"- {step.action}: {step.path}")
        click.echo(f"  {step.description}")
    if plan.result_kind == "stub":
        click.echo("Result: stub; real integration is deferred to the agent workflow spec.")
    else:
        click.echo(f"Result: {plan.result_kind}.")


@main.command("update")
@click.option("--check", "check_only", is_flag=True)
@click.option("--dev", "allow_dev", is_flag=True, help="Allow updating to the latest main commit.")
@click.option("--json", "as_json", is_flag=True)
@click.option(
    "--target",
    type=click.Choice(["latest", "main"]),
    default=None,
    help="Override the configured update channel for this run.",
)
@click.pass_context
def update_command(
    ctx: click.Context,
    check_only: bool,
    allow_dev: bool,
    as_json: bool,
    target: str | None,
) -> None:
    before_status = None
    try:
        release_config = load_release_config(ctx.obj["config"])
        update_target = target or release_config.update_target
        update_allows_dev = allow_dev or (target is None and release_config.allows_dev_updates)
        if check_only:
            status = check_update(target=update_target, allow_dev=update_allows_dev)
        else:
            before_status = check_update(target=update_target, allow_dev=update_allows_dev)
            status = (
                run_update(target=update_target, allow_dev=update_allows_dev)
                if before_status.update_available
                else before_status
            )
    except (RuntimeError, ValueError, ReleaseConfigError) as error:
        raise click.ClickException(str(error)) from error
    except subprocess.CalledProcessError as error:
        raise click.ClickException(_subprocess_error_message(error)) from error

    if as_json:
        click.echo(render_json(status.as_dict()))
        return

    click.echo(render_greeting())
    click.echo()
    current_display = _update_status_current_display(status)
    if before_status is not None and before_status.update_available and not status.update_available:
        click.echo(
            "Updated Hieronymus: "
            f"{_update_status_current_display(before_status)} -> "
            f"{current_display}"
        )
    elif status.update_available:
        click.echo(
            f"Update available: {current_display} -> {_update_status_target_display(status)}"
        )
    elif check_only:
        click.echo(f"No update available: {current_display}")
    else:
        click.echo(f"Hieronymus is up to date: {current_display}")
    click.echo(f"managed checkout: {status.managed_checkout}")


@main.command("admin")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def admin(ctx: click.Context, json_output: bool) -> None:
    config = ctx.obj["config"]
    if json_output:
        payload = AdminStore(config).status_payload()
        click.echo(render_json(payload))
        return

    _launch_opentui("admin", data_root=config.data_root)


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
    click.echo("Alpha software: local-first, usable at your own risk.")
    click.echo()
    click.echo(f"{GUIDE_ICON} Service")
    click.echo("  hiero                  Start or connect to the local service")
    click.echo("  hiero status           Show daemon and provider status")
    click.echo("  hiero status --json    Emit daemon and provider status for scripts")
    click.echo("  hiero stop             Request graceful daemon shutdown")
    click.echo("  hiero restart          Restart the local daemon")
    click.echo()
    click.echo(f"{GUIDE_ICON} Management")
    click.echo("  hiero config           Open the configuration TUI")
    click.echo(
        "  hiero config --json    Emit config, provider, dreaming, ingest, and release state"
    )
    click.echo("  hiero admin            Open the local management TUI")
    click.echo("  hiero admin --json     Emit management counts and available views")
    click.echo("  hiero doctor           Check configuration and service health")
    click.echo("  hiero doctor --json    Emit configuration and service diagnostics")
    click.echo()
    click.echo(f"{GUIDE_ICON} Agent and automation")
    click.echo("  hiero session-start <series> --task-type <type> --json")
    click.echo("  hiero remember-short <session-id> --role user --kind correction")
    click.echo("      --text <text> --json")
    click.echo("  hiero recall <session-id> --series <series> --query <query>")
    click.echo("      --source-language <src> --target-language <dst>")
    click.echo("      --task-type <type> --json")
    click.echo("  hiero feedback <crystal-id> --event helpful --role user --json")
    click.echo()
    click.echo(f"{GUIDE_ICON} Maintenance")
    click.echo("  hiero install codex --dry-run")
    click.echo("  hiero update           Update managed installs in place")
    click.echo("  hiero dream --json     Run local dreaming and emit machine-readable status")
    click.echo()
    click.echo(f"{GUIDE_ICON} Examples")
    click.echo("  hiero status --json")
    click.echo("  hiero session-start oso --task-type translation --json")
    click.echo('  hiero recall 1 --series oso --query "style"')
    click.echo("      --source-language ja --target-language en")
    click.echo("      --task-type translation --json")


@main.command("init-series")
@click.argument("slug")
@click.option("--title", required=True)
@click.option("--source-language", default="ja")
@click.option("--target-language", default="en")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def init_series(
    ctx: click.Context,
    slug: str,
    title: str,
    source_language: str,
    target_language: str,
    json_output: bool,
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
    _echo_json_or_line(
        {"slug": series.slug, "database_path": str(ctx.obj["config"].database_path)},
        json_output=json_output,
        line=f"Series {series.slug} initialized at {ctx.obj['config'].database_path}",
    )


@main.command("propose-term")
@click.argument("series_slug")
@click.option("--category", required=True)
@click.option("--source", "source_text", required=True)
@click.option("--translation", required=True)
@click.option("--tag", "tags", multiple=True)
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def propose_term(
    ctx: click.Context,
    series_slug: str,
    category: str,
    source_text: str,
    translation: str,
    tags: tuple[str, ...],
    json_output: bool,
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
    _echo_json_or_line(
        {"term_id": term_id},
        json_output=json_output,
        line=f"Term proposal {term_id} created",
    )


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
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def validate(
    ctx: click.Context,
    series_slug: str,
    raw_file: str,
    translated_file: str,
    json_output: bool,
) -> None:
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
    payload = [asdict(finding) for finding in findings]
    _echo_json_or_line(
        payload,
        json_output=json_output,
        line=(
            "No terminology findings." if not payload else f"{len(payload)} terminology finding(s)."
        ),
    )


@main.command("remember")
@click.argument("series_slug")
@click.option("--kind", required=True)
@click.option("--text", required=True)
@click.option("--source-ref", default="")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def remember(
    ctx: click.Context,
    series_slug: str,
    kind: str,
    text: str,
    source_ref: str,
    json_output: bool,
) -> None:
    try:
        series = Registry(ctx.obj["config"]).get_series(series_slug)
        memory_id = MemoryStore(
            ctx.obj["config"],
            _series_context(series, task_type="translation"),
        ).add(kind=kind, text=text, source_ref=source_ref)
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    _echo_json_or_line(
        {"memory_id": memory_id},
        json_output=json_output,
        line=f"Memory {memory_id} stored",
    )


@main.command("session-start")
@click.argument("series_slug")
@click.option("--source-language", default=None)
@click.option("--target-language", default=None)
@click.option("--task-type", required=True)
@click.option("--volume", default="")
@click.option("--chapter", default="")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def session_start(
    ctx: click.Context,
    series_slug: str,
    source_language: str,
    target_language: str,
    task_type: str,
    volume: str,
    chapter: str,
    json_output: bool,
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
    _echo_json_or_line(
        {"session_id": session.id},
        json_output=json_output,
        line=f"Session {session.id} started",
    )


@main.command("session-complete")
@click.argument("session_id", type=int)
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def session_complete(ctx: click.Context, session_id: int, json_output: bool) -> None:
    try:
        WorkspaceStore(ctx.obj["config"]).complete_session(session_id)
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    _echo_json_or_line(
        {"session_id": session_id, "completed": True},
        json_output=json_output,
        line=f"Session {session_id} completed",
    )


@main.command("remember-short")
@click.argument("session_id", type=int)
@click.option("--role", required=True)
@click.option("--kind", required=True)
@click.option("--text", required=True)
@click.option("--source-ref", default="")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def remember_short(
    ctx: click.Context,
    session_id: int,
    role: str,
    kind: str,
    text: str,
    source_ref: str,
    json_output: bool,
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
    _echo_json_or_line(
        {"memory_id": memory_id},
        json_output=json_output,
        line=f"Short-term memory {memory_id} stored",
    )


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
@click.option("--json", "json_output", is_flag=True)
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
    json_output: bool,
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
    payload = [
        {
            "source": result.source,
            "rank": result.rank,
            "score": result.score,
            "reason": result.reason,
            "crystal": _crystal_payload(result.crystal),
            "short_term_memory": _short_term_memory_payload(result.short_term_memory),
        }
        for result in results
    ]
    _echo_json_or_line(
        payload,
        json_output=json_output,
        line="No recall results." if not payload else f"{len(payload)} recall result(s).",
    )


@main.command("feedback")
@click.argument("crystal_id", type=int)
@click.option("--event", "event_type", required=True)
@click.option("--role", "source_role", required=True)
@click.option("--evidence", default="")
@click.option("--session-id", type=int, default=None)
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def feedback(
    ctx: click.Context,
    crystal_id: int,
    event_type: str,
    source_role: str,
    evidence: str,
    session_id: int | None,
    json_output: bool,
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
    _echo_json_or_line(
        {"event_id": event_id},
        json_output=json_output,
        line=f"Feedback event {event_id} recorded",
    )


@main.command("dream")
@click.option("--provider", default=None)
@click.option("--json", "json_output", is_flag=True)
@click.option("--wait", is_flag=True, help="Wait for an active dream cycle to finish.")
@click.pass_context
def dream(
    ctx: click.Context,
    provider: str | None,
    json_output: bool,
    wait: bool,
) -> None:
    try:
        dream_provider = resolve_provider(ctx.obj["config"], provider)
        run = DreamService(
            ctx.obj["config"],
            dream_provider,
        ).run_all(wait=wait, owner="cli", ignore_minimum=True)
    except (KeyError, ValueError, DreamCycleAlreadyRunning) as error:
        _raise_click_error(error)
    payload = {
        "cycle_id": run.cycle_id,
        "status": run.status,
        "provider": run.provider,
        "input_count": run.input_count,
        "created_crystal_count": run.created_crystal_count,
        "proposal_count": run.proposal_count,
        "error": run.error,
    }
    if json_output:
        click.echo(render_json(payload))
        return
    click.echo(f"Dream run {run.cycle_id}: {run.status}")
