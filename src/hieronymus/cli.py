from __future__ import annotations

import json

import click

from hieronymus.config import load_config
from hieronymus.memory import MemoryStore
from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


@click.group()
@click.option("--data-root", type=click.Path(), default=None)
@click.pass_context
def main(ctx: click.Context, data_root: str | None) -> None:
    ctx.obj = {"config": load_config(data_root)}


@main.command("init-series")
@click.argument("slug")
@click.option("--title", required=True)
@click.option("--source-language", default="ja")
@click.option("--target-language", default="en")
@click.pass_context
def init_series(
    ctx: click.Context, slug: str, title: str, source_language: str, target_language: str
) -> None:
    series = Registry(ctx.obj["config"]).create_series(
        slug=slug,
        title=title,
        source_language=source_language,
        target_language=target_language,
    )
    click.echo(
        json.dumps(
            {"slug": series.slug, "database_path": str(series.database_path)}, ensure_ascii=False
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
    series = Registry(ctx.obj["config"]).get_series(series_slug)
    term_id = Termbase(series.database_path).propose(
        category=category,
        source_text=source_text,
        canonical_translation=translation,
        tags=list(tags),
    )
    click.echo(json.dumps({"term_id": term_id}, ensure_ascii=False))


@main.command("validate")
@click.argument("series_slug")
@click.option("--raw-file", type=click.Path(exists=True), required=True)
@click.option("--translated-file", type=click.Path(exists=True), required=True)
@click.pass_context
def validate(ctx: click.Context, series_slug: str, raw_file: str, translated_file: str) -> None:
    series = Registry(ctx.obj["config"]).get_series(series_slug)
    with (
        open(raw_file, encoding="utf-8") as raw,
        open(translated_file, encoding="utf-8") as translated,
    ):
        findings = Termbase(series.database_path).validate(
            raw_text=raw.read(),
            translated_text=translated.read(),
        )
    click.echo(json.dumps([finding.__dict__ for finding in findings], ensure_ascii=False, indent=2))


@main.command("remember")
@click.argument("series_slug")
@click.option("--kind", required=True)
@click.option("--text", required=True)
@click.option("--source-ref", default="")
@click.pass_context
def remember(ctx: click.Context, series_slug: str, kind: str, text: str, source_ref: str) -> None:
    series = Registry(ctx.obj["config"]).get_series(series_slug)
    memory_id = MemoryStore(series.database_path).add(kind=kind, text=text, source_ref=source_ref)
    click.echo(json.dumps({"memory_id": memory_id}, ensure_ascii=False))
