# Hieronymus MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first working Hieronymus release: a local-first per-series translation memory with strict termbase validation, fuzzy memory search, CLI commands, and MCP tools.

**Architecture:** Implement a Python package with a small core domain layer, SQLite repositories, CLI commands, and an MCP stdio server. Runtime databases live outside the source repo; the tool source lives in `~/Development/hieronymus`.

**Tech Stack:** Python 3.12+, uv, SQLite/FTS5, pytest, ruff, MCP Python SDK or a minimal stdio MCP adapter if the SDK is not suitable.

---

## File Structure

Create this structure:

```text
hieronymus/
├── AGENTS.md
├── README.md
├── pyproject.toml
├── src/hieronymus/
│   ├── __init__.py
│   ├── cli.py
│   ├── config.py
│   ├── db.py
│   ├── mcp_server.py
│   ├── models.py
│   ├── registry.py
│   ├── termbase.py
│   ├── memory.py
│   └── migrations/
│       ├── registry.sql
│       └── series.sql
├── tests/
│   ├── conftest.py
│   ├── test_registry.py
│   ├── test_termbase_contract.py
│   ├── test_termbase_validate.py
│   └── test_memory_search.py
└── docs/
    └── superpowers/
        ├── specs/2026-06-06-hieronymus-design.md
        └── plans/2026-06-06-hieronymus-mvp.md
```

Responsibilities:

- `config.py`: resolve data root and runtime paths.
- `db.py`: SQLite connection, migrations, FTS setup.
- `registry.py`: create/list series and locate per-series databases.
- `models.py`: dataclasses and validation enums.
- `termbase.py`: strict term lookup, contract generation, validation, proposals, approval.
- `memory.py`: fuzzy memory add/search using FTS5.
- `cli.py`: user-facing CLI for debugging and imports.
- `mcp_server.py`: MCP tool wrappers around the core services.

## Task 1: Project Skeleton

**Files:**
- Create: `pyproject.toml`
- Create: `src/hieronymus/__init__.py`
- Create: `src/hieronymus/config.py`
- Create: `tests/conftest.py`

- [ ] **Step 1: Write `pyproject.toml`**

```toml
[project]
name = "hieronymus"
version = "0.1.0"
description = "Local-first translation memory MCP for long-form book translation"
readme = "README.md"
requires-python = ">=3.12"
authors = [{ name = "Pavel Obruchnikov", email = "me@inkyquill.net" }]
dependencies = [
  "click>=8.1.8",
  "mcp>=1.2.0",
]

[project.scripts]
hieronymus = "hieronymus.cli:main"

[dependency-groups]
dev = [
  "pytest>=8.3.0",
  "ruff>=0.8.0",
]

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[tool.ruff]
line-length = 100
target-version = "py312"

[tool.ruff.lint]
select = ["E", "F", "I", "UP", "B"]
```

- [ ] **Step 2: Create package marker**

```python
"""Hieronymus translation memory."""

__all__ = ["__version__"]

__version__ = "0.1.0"
```

- [ ] **Step 3: Add config path resolution**

```python
from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class HieronymusConfig:
    data_root: Path

    @property
    def registry_path(self) -> Path:
        return self.data_root / "registry.sqlite"

    @property
    def series_dir(self) -> Path:
        return self.data_root / "series"


def load_config(data_root: str | None = None) -> HieronymusConfig:
    raw_root = data_root or os.environ.get("HIERONYMUS_DATA_ROOT")
    root = Path(raw_root).expanduser() if raw_root else Path.home() / ".hieronymus"
    return HieronymusConfig(data_root=root)
```

- [ ] **Step 4: Add pytest fixture**

```python
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig


@pytest.fixture
def config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "memory")
```

- [ ] **Step 5: Run skeleton checks**

Run: `uv sync`

Expected: dependencies install and `uv.lock` is created.

Run: `uv run pytest`

Expected: `no tests ran` or all existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add pyproject.toml uv.lock src/hieronymus/__init__.py src/hieronymus/config.py tests/conftest.py
git commit -m "chore: scaffold Hieronymus Python project"
```

## Task 2: SQLite Registry and Series Databases

**Files:**
- Create: `src/hieronymus/db.py`
- Create: `src/hieronymus/registry.py`
- Create: `src/hieronymus/migrations/registry.sql`
- Create: `src/hieronymus/migrations/series.sql`
- Test: `tests/test_registry.py`

- [ ] **Step 1: Write failing registry test**

```python
from hieronymus.registry import Registry


def test_create_series_initializes_registry_and_series_database(config):
    registry = Registry(config)

    series = registry.create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    assert series.slug == "only-sense-online"
    assert config.registry_path.exists()
    assert (config.series_dir / "only-sense-online.sqlite").exists()
    assert registry.get_series("only-sense-online").title == "Only Sense Online"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_registry.py -v`

Expected: FAIL because `hieronymus.registry` does not exist.

- [ ] **Step 3: Add migrations**

`src/hieronymus/migrations/registry.sql`:

```sql
create table if not exists series (
  id integer primary key,
  slug text unique not null,
  title text not null,
  source_language text not null,
  target_language text not null,
  database_path text not null,
  created_at text not null,
  updated_at text not null
);
```

`src/hieronymus/migrations/series.sql`:

```sql
create table if not exists terms (
  id integer primary key,
  override_of_term_id integer references terms(id),
  category text not null,
  source_text text not null,
  canonical_translation text not null,
  status text not null,
  scope text not null,
  volume text,
  confidence real not null default 1.0,
  notes text not null default '',
  created_at text not null,
  updated_at text not null
);

create table if not exists term_tags (
  term_id integer not null references terms(id),
  tag text not null,
  primary key(term_id, tag)
);

create table if not exists term_aliases (
  id integer primary key,
  term_id integer not null references terms(id),
  language text not null,
  text text not null,
  kind text not null,
  case_sensitive integer not null default 1
);

create table if not exists term_evidence (
  id integer primary key,
  term_id integer not null references terms(id),
  source_type text not null,
  source_ref text not null,
  quote text not null default '',
  url text not null default '',
  notes text not null default '',
  created_at text not null
);

create table if not exists memories (
  id integer primary key,
  kind text not null,
  text text not null,
  importance integer not null default 3,
  status text not null default 'active',
  source_ref text not null default '',
  created_at text not null,
  updated_at text not null
);

create virtual table if not exists terms_fts using fts5(
  source_text,
  canonical_translation,
  notes,
  content='terms',
  content_rowid='id'
);

create virtual table if not exists memories_fts using fts5(
  text,
  content='memories',
  content_rowid='id'
);
```

- [ ] **Step 4: Implement DB helpers**

```python
from __future__ import annotations

import sqlite3
from importlib.resources import files
from pathlib import Path


def connect(path: Path) -> sqlite3.Connection:
    path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(path)
    conn.row_factory = sqlite3.Row
    conn.execute("pragma foreign_keys = on")
    conn.execute("pragma journal_mode = wal")
    return conn


def apply_migration(conn: sqlite3.Connection, name: str) -> None:
    sql = files("hieronymus.migrations").joinpath(name).read_text(encoding="utf-8")
    conn.executescript(sql)
    conn.commit()
```

- [ ] **Step 5: Implement registry**

```python
from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect


@dataclass(frozen=True)
class Series:
    slug: str
    title: str
    source_language: str
    target_language: str
    database_path: Path


def _now() -> str:
    return datetime.now(UTC).isoformat()


class Registry:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        self.config.series_dir.mkdir(parents=True, exist_ok=True)
        with connect(self.config.registry_path) as conn:
            apply_migration(conn, "registry.sql")

    def create_series(
        self,
        *,
        slug: str,
        title: str,
        source_language: str,
        target_language: str,
    ) -> Series:
        database_path = self.config.series_dir / f"{slug}.sqlite"
        now = _now()
        with connect(self.config.registry_path) as conn:
            conn.execute(
                """
                insert into series(slug, title, source_language, target_language, database_path, created_at, updated_at)
                values (?, ?, ?, ?, ?, ?, ?)
                on conflict(slug) do update set
                  title=excluded.title,
                  source_language=excluded.source_language,
                  target_language=excluded.target_language,
                  database_path=excluded.database_path,
                  updated_at=excluded.updated_at
                """,
                (slug, title, source_language, target_language, str(database_path), now, now),
            )
            conn.commit()

        with connect(database_path) as conn:
            apply_migration(conn, "series.sql")

        return Series(slug, title, source_language, target_language, database_path)

    def get_series(self, slug: str) -> Series:
        with connect(self.config.registry_path) as conn:
            row = conn.execute("select * from series where slug = ?", (slug,)).fetchone()
        if row is None:
            raise KeyError(f"unknown series: {slug}")
        return Series(
            slug=row["slug"],
            title=row["title"],
            source_language=row["source_language"],
            target_language=row["target_language"],
            database_path=Path(row["database_path"]),
        )
```

- [ ] **Step 6: Run registry tests**

Run: `uv run pytest tests/test_registry.py -v`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/db.py src/hieronymus/registry.py src/hieronymus/migrations tests/test_registry.py
git commit -m "feat: add series registry and database migrations"
```

## Task 3: Strict Termbase Contract

**Files:**
- Create: `src/hieronymus/models.py`
- Create: `src/hieronymus/termbase.py`
- Test: `tests/test_termbase_contract.py`

- [ ] **Step 1: Write failing contract test**

```python
from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


def test_contract_returns_terms_found_in_raw_text(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = Termbase(series.database_path)
    term_id = termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
        tags=["sense"],
        notes="OSO Sense name.",
    )
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Increase", language="en")
    termbase.approve(term_id)

    contract = termbase.contract("ユンは攻撃力上昇を取るべきだと言われた。")

    assert contract[0].source_text == "攻撃力上昇"
    assert contract[0].canonical_translation == "ATK Up"
    assert "Attack Increase" in contract[0].forbidden_variants
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_termbase_contract.py -v`

Expected: FAIL because `Termbase` does not exist.

- [ ] **Step 3: Add models**

```python
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class ContractTerm:
    id: int
    category: str
    source_text: str
    canonical_translation: str
    forbidden_variants: list[str]
    tags: list[str]
    notes: str
```

- [ ] **Step 4: Implement termbase propose/approve/contract**

```python
from __future__ import annotations

from datetime import UTC, datetime
from pathlib import Path

from hieronymus.db import connect
from hieronymus.models import ContractTerm


def _now() -> str:
    return datetime.now(UTC).isoformat()


class Termbase:
    def __init__(self, database_path: Path) -> None:
        self.database_path = database_path

    def propose(
        self,
        *,
        category: str,
        source_text: str,
        canonical_translation: str,
        tags: list[str] | None = None,
        notes: str = "",
    ) -> int:
        now = _now()
        with connect(self.database_path) as conn:
            cursor = conn.execute(
                """
                insert into terms(category, source_text, canonical_translation, status, scope, notes, created_at, updated_at)
                values (?, ?, ?, 'pending', 'series', ?, ?, ?)
                """,
                (category, source_text, canonical_translation, notes, now, now),
            )
            term_id = int(cursor.lastrowid)
            for tag in tags or []:
                conn.execute("insert into term_tags(term_id, tag) values (?, ?)", (term_id, tag))
            conn.execute(
                "insert into terms_fts(rowid, source_text, canonical_translation, notes) values (?, ?, ?, ?)",
                (term_id, source_text, canonical_translation, notes),
            )
            conn.commit()
        return term_id

    def approve(self, term_id: int) -> None:
        with connect(self.database_path) as conn:
            conn.execute(
                "update terms set status = 'approved', updated_at = ? where id = ?",
                (_now(), term_id),
            )
            conn.commit()

    def add_alias(self, term_id: int, *, kind: str, text: str, language: str, case_sensitive: bool = True) -> None:
        with connect(self.database_path) as conn:
            conn.execute(
                """
                insert into term_aliases(term_id, language, text, kind, case_sensitive)
                values (?, ?, ?, ?, ?)
                """,
                (term_id, language, text, kind, int(case_sensitive)),
            )
            conn.commit()

    def contract(self, raw_text: str) -> list[ContractTerm]:
        with connect(self.database_path) as conn:
            rows = conn.execute(
                "select * from terms where status = 'approved' order by id"
            ).fetchall()
            result: list[ContractTerm] = []
            for row in rows:
                source_forms = [row["source_text"]]
                alias_rows = conn.execute(
                    "select * from term_aliases where term_id = ?",
                    (row["id"],),
                ).fetchall()
                source_forms.extend(
                    alias["text"] for alias in alias_rows if alias["kind"] == "source_variant"
                )
                if not any(form in raw_text for form in source_forms):
                    continue
                tags = [
                    tag_row["tag"]
                    for tag_row in conn.execute(
                        "select tag from term_tags where term_id = ? order by tag",
                        (row["id"],),
                    )
                ]
                forbidden = [
                    alias["text"] for alias in alias_rows if alias["kind"] == "forbidden_variant"
                ]
                result.append(
                    ContractTerm(
                        id=row["id"],
                        category=row["category"],
                        source_text=row["source_text"],
                        canonical_translation=row["canonical_translation"],
                        forbidden_variants=forbidden,
                        tags=tags,
                        notes=row["notes"],
                    )
                )
        return result
```

- [ ] **Step 5: Run contract tests**

Run: `uv run pytest tests/test_termbase_contract.py -v`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/models.py src/hieronymus/termbase.py tests/test_termbase_contract.py
git commit -m "feat: generate strict glossary contracts"
```

## Task 4: Termbase Validation

**Files:**
- Modify: `src/hieronymus/models.py`
- Modify: `src/hieronymus/termbase.py`
- Test: `tests/test_termbase_validate.py`

- [ ] **Step 1: Write failing validation tests**

```python
from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


def test_validate_flags_forbidden_variant(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = Termbase(series.database_path)
    term_id = termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
        tags=["sense"],
    )
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Increase", language="en")
    termbase.approve(term_id)

    findings = termbase.validate(
        raw_text="攻撃力上昇を取るべきだ。",
        translated_text="You should pick up Attack Increase.",
    )

    assert findings[0].severity == "high"
    assert findings[0].observed == "Attack Increase"
    assert findings[0].expected == "ATK Up"


def test_validate_flags_missing_canonical_when_source_present(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = Termbase(series.database_path)
    term_id = termbase.propose(
        category="person_name",
        source_text="ガンツ",
        canonical_translation="Ganz",
    )
    termbase.approve(term_id)

    findings = termbase.validate(raw_text="ガンツが笑った。", translated_text="Gantz laughed.")

    assert findings[0].kind == "missing_canonical"
    assert findings[0].expected == "Ganz"
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_termbase_validate.py -v`

Expected: FAIL because `validate` and `ValidationFinding` do not exist.

- [ ] **Step 3: Add validation model**

```python
@dataclass(frozen=True)
class ValidationFinding:
    term_id: int
    kind: str
    severity: str
    expected: str
    observed: str
    message: str
```

- [ ] **Step 4: Implement validation**

Add to `Termbase`:

```python
    def validate(self, *, raw_text: str, translated_text: str) -> list[ValidationFinding]:
        findings: list[ValidationFinding] = []
        for term in self.contract(raw_text):
            for forbidden in term.forbidden_variants:
                if forbidden in translated_text:
                    findings.append(
                        ValidationFinding(
                            term_id=term.id,
                            kind="forbidden_variant",
                            severity="high",
                            expected=term.canonical_translation,
                            observed=forbidden,
                            message=(
                                f"Use {term.canonical_translation!r} for {term.source_text!r}; "
                                f"{forbidden!r} is forbidden."
                            ),
                        )
                    )
            if term.canonical_translation not in translated_text:
                findings.append(
                    ValidationFinding(
                        term_id=term.id,
                        kind="missing_canonical",
                        severity="medium",
                        expected=term.canonical_translation,
                        observed="",
                        message=(
                            f"Raw text contains {term.source_text!r}, but translation does not "
                            f"contain approved form {term.canonical_translation!r}."
                        ),
                    )
                )
        return findings
```

Also import `ValidationFinding` in `termbase.py`.

- [ ] **Step 5: Run validation tests**

Run: `uv run pytest tests/test_termbase_validate.py -v`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/models.py src/hieronymus/termbase.py tests/test_termbase_validate.py
git commit -m "feat: validate translations against approved terms"
```

## Task 5: Fuzzy Memory Storage and Search

**Files:**
- Create: `src/hieronymus/memory.py`
- Test: `tests/test_memory_search.py`

- [ ] **Step 1: Write failing memory tests**

```python
from hieronymus.memory import MemoryStore
from hieronymus.registry import Registry


def test_memory_search_returns_relevant_entries(config):
    series = Registry(config).create_series(
        slug="death-march",
        title="Death March to the Parallel World Rhapsody",
        source_language="ja",
        target_language="en",
    )
    store = MemoryStore(series.database_path)
    store.add(
        kind="translation_rationale",
        text="Satou's system messages should stay concise and game-like.",
        source_ref="user:2026-06-06",
        importance=4,
    )

    results = store.search("system messages")

    assert results[0].text == "Satou's system messages should stay concise and game-like."
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_memory_search.py -v`

Expected: FAIL because `hieronymus.memory` does not exist.

- [ ] **Step 3: Add memory model**

Add to `models.py`:

```python
@dataclass(frozen=True)
class MemoryEntry:
    id: int
    kind: str
    text: str
    importance: int
    source_ref: str
```

- [ ] **Step 4: Implement memory store**

```python
from __future__ import annotations

from datetime import UTC, datetime
from pathlib import Path

from hieronymus.db import connect
from hieronymus.models import MemoryEntry


def _now() -> str:
    return datetime.now(UTC).isoformat()


class MemoryStore:
    def __init__(self, database_path: Path) -> None:
        self.database_path = database_path

    def add(self, *, kind: str, text: str, source_ref: str = "", importance: int = 3) -> int:
        now = _now()
        with connect(self.database_path) as conn:
            cursor = conn.execute(
                """
                insert into memories(kind, text, importance, source_ref, created_at, updated_at)
                values (?, ?, ?, ?, ?, ?)
                """,
                (kind, text, importance, source_ref, now, now),
            )
            memory_id = int(cursor.lastrowid)
            conn.execute(
                "insert into memories_fts(rowid, text) values (?, ?)",
                (memory_id, text),
            )
            conn.commit()
        return memory_id

    def search(self, query: str, *, limit: int = 5) -> list[MemoryEntry]:
        with connect(self.database_path) as conn:
            rows = conn.execute(
                """
                select memories.*
                from memories_fts
                join memories on memories.id = memories_fts.rowid
                where memories_fts match ?
                order by memories.importance desc, bm25(memories_fts)
                limit ?
                """,
                (query, limit),
            ).fetchall()
        return [
            MemoryEntry(
                id=row["id"],
                kind=row["kind"],
                text=row["text"],
                importance=row["importance"],
                source_ref=row["source_ref"],
            )
            for row in rows
        ]
```

- [ ] **Step 5: Run memory tests**

Run: `uv run pytest tests/test_memory_search.py -v`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/models.py src/hieronymus/memory.py tests/test_memory_search.py
git commit -m "feat: add fuzzy memory search"
```

## Task 6: CLI

**Files:**
- Create: `src/hieronymus/cli.py`
- Test: add CLI smoke tests if Click runner is available

- [ ] **Step 1: Implement CLI commands**

```python
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
def init_series(ctx: click.Context, slug: str, title: str, source_language: str, target_language: str) -> None:
    series = Registry(ctx.obj["config"]).create_series(
        slug=slug,
        title=title,
        source_language=source_language,
        target_language=target_language,
    )
    click.echo(json.dumps({"slug": series.slug, "database_path": str(series.database_path)}, ensure_ascii=False))


@main.command("propose-term")
@click.argument("series_slug")
@click.option("--category", required=True)
@click.option("--source", "source_text", required=True)
@click.option("--translation", required=True)
@click.option("--tag", "tags", multiple=True)
@click.pass_context
def propose_term(ctx: click.Context, series_slug: str, category: str, source_text: str, translation: str, tags: tuple[str, ...]) -> None:
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
    findings = Termbase(series.database_path).validate(
        raw_text=open(raw_file, encoding="utf-8").read(),
        translated_text=open(translated_file, encoding="utf-8").read(),
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
```

- [ ] **Step 2: Run CLI smoke commands**

Run:

```bash
uv run hieronymus --data-root /tmp/hieronymus-test init-series only-sense-online --title "Only Sense Online"
```

Expected: JSON with `slug` and `database_path`.

- [ ] **Step 3: Commit**

```bash
git add src/hieronymus/cli.py
git commit -m "feat: add Hieronymus CLI"
```

## Task 7: MCP Server

**Files:**
- Create: `src/hieronymus/mcp_server.py`

- [ ] **Step 1: Add MCP tool wrappers**

Implement tools with these names:

```text
hieronymus_termbase_contract
hieronymus_termbase_validate
hieronymus_termbase_propose
hieronymus_termbase_approve
hieronymus_memory_search
hieronymus_memory_add
```

Each MCP tool must call the same `Registry`, `Termbase`, and `MemoryStore` classes used by the CLI. Do not duplicate database logic in the MCP layer.

- [ ] **Step 2: Add stdio entrypoint**

Add a script entry to `pyproject.toml`:

```toml
[project.scripts]
hieronymus = "hieronymus.cli:main"
hieronymus-mcp = "hieronymus.mcp_server:main"
```

- [ ] **Step 3: Run import smoke test**

Run:

```bash
uv run python -c "from hieronymus.mcp_server import main; print(callable(main))"
```

Expected: `True`.

- [ ] **Step 4: Commit**

```bash
git add pyproject.toml src/hieronymus/mcp_server.py
git commit -m "feat: expose Hieronymus MCP tools"
```

## Task 8: Documentation and Translation Workspace Integration

**Files:**
- Create: `docs/usage.md`
- Create: `docs/translation-workspace-integration.md`

- [ ] **Step 1: Document CLI usage**

`docs/usage.md` must include:

```markdown
# Hieronymus Usage

## Data Root

Set `HIERONYMUS_DATA_ROOT` to the translation workspace memory directory:

```bash
export HIERONYMUS_DATA_ROOT=/home/inky/Yandex.Disk/Translation/.translation-memory
```

## Initialize a Series

```bash
hieronymus init-series only-sense-online --title "Only Sense Online" --source-language ja --target-language en
hieronymus init-series death-march --title "Death March to the Parallel World Rhapsody" --source-language ja --target-language en
```

## Validate a Chapter

```bash
hieronymus validate only-sense-online --raw-file only-sense-online/vol01/raw/chapter-002.xhtml --translated-file only-sense-online/vol01/translated/chapter-002.md
```
```

- [ ] **Step 2: Document agent workflow integration**

`docs/translation-workspace-integration.md` must state:

```markdown
# Translation Workspace Integration

Before translating a chapter, the orchestrator calls `hieronymus_termbase_contract` with the series slug, volume, and raw chapter text.

After translation, the orchestrator calls `hieronymus_termbase_validate` with the same raw chapter text and the translated chapter. Any high severity finding goes back to the Translator before Accuracy Review.

Fuzzy memories are advisory. Approved termbase entries are mandatory.
```

- [ ] **Step 3: Commit**

```bash
git add docs/usage.md docs/translation-workspace-integration.md
git commit -m "docs: describe Hieronymus usage and workflow integration"
```

## Task 9: Final Verification

**Files:**
- Modify only if verification finds issues.

- [ ] **Step 1: Run tests**

Run: `uv run pytest`

Expected: all tests pass.

- [ ] **Step 2: Run lint**

Run: `uv run ruff check .`

Expected: no violations.

- [ ] **Step 3: Run format check**

Run: `uv run ruff format --check .`

Expected: no formatting changes needed.

- [ ] **Step 4: Commit any verification fixes**

If fixes were required:

```bash
git add .
git commit -m "test: verify Hieronymus MVP"
```

If no fixes were required, do not create an empty commit.

## Self-Review

Spec coverage:

- Separate repo path: Task 1 and the already-created repository cover `~/Development/hieronymus`.
- Per-series SQLite: Task 2 creates registry and per-series databases.
- Strict termbase: Tasks 3 and 4 cover proposals, approval, contracts, aliases, tags, and validation.
- Fuzzy memories: Task 5 covers add/search with FTS5.
- CLI: Task 6 covers user-facing commands.
- MCP: Task 7 exposes the required agent tools.
- Translation workflow integration: Task 8 documents contract-before-translation and validate-before-review.
- Dreaming, embeddings, external API synthesis, and markdown exports are intentionally excluded from MVP per the design spec.

Placeholder scan:

- The plan contains no red-flag placeholder terms.
- Runtime choice is fixed for MVP: Python 3.12+, SQLite, CLI, MCP stdio.

Type consistency:

- `Registry`, `Termbase`, `MemoryStore`, `ContractTerm`, `ValidationFinding`, and `MemoryEntry` are introduced before use.
- CLI and MCP layers call core services instead of duplicating persistence logic.
