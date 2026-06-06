from __future__ import annotations

import json
from dataclasses import asdict

import click

from hieronymus.config import load_config
from hieronymus.memory import MemoryStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


def _error_message(error: KeyError | ValueError) -> str:
    if isinstance(error, KeyError) and error.args:
        return str(error.args[0])
    return str(error)


def _raise_click_error(error: KeyError | ValueError) -> None:
    raise click.ClickException(_error_message(error)) from error


def _series_context(series, *, task_type: str) -> TranslationContext:
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type=task_type,
    )


@click.group()
@click.option("--data-root", type=click.Path(file_okay=False, dir_okay=True), default=None)
@click.pass_context
def main(ctx: click.Context, data_root: str | None) -> None:
    config = load_config(data_root)
    if config.data_root.exists() and not config.data_root.is_dir():
        raise click.ClickException(f"data root is not a directory: {config.data_root}")
    ctx.obj = {"config": config}


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
