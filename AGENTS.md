# AGENTS.md

## Project

Hieronymus is a local-first translation memory MCP for literary translation workflows. It is separate from the translation workspace; source code lives here, while book projects and runtime databases live elsewhere.

## Development Defaults

- Use `Pavel Obruchnikov <me@inkyquill.net>` for formal author metadata unless local git config overrides it.
- Prefer small, testable Python modules with explicit boundaries.
- Keep strict terminology logic deterministic. Fuzzy memory and semantic recall must never silently override approved termbase entries.
- Do not write tool source code into `/home/inky/Yandex.Disk/Translation`.

## Planned Stack

- Python 3.12+
- `uv` for package management
- SQLite with FTS5
- `pytest`
- MCP server over stdio for agent integration
- CLI for local debugging, imports, exports, and validation

## Verification

Run these before claiming implementation work is complete:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```
