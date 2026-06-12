# Hieronymus Management TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a keyboard-first admin TUI for inspecting, filtering, editing, reviewing, and controlling Hieronymus concepts, renderings, crystals, lessons, sessions, dream runs, proposals, audit history, service status, and statistics.

**Architecture:** Add a testable `AdminStore` facade over the existing global SQLite schema, then keep the Textual app as a thin presentation layer over that facade. The TUI provides dense navigation tables, detail panes, filter controls, command palette actions, modal edit/review dialogs, provenance/recall inspection, and operational status/statistics without becoming a separate source of truth.

**Tech Stack:** Python 3.12+, uv, SQLite/FTS5, pytest, ruff, Click CLI, Textual/Rich, existing Hieronymus config/service/dreaming stores.

---

## Ground Rules

- The TUI launches from `hiero admin`; `hiero admin --json` remains a non-interactive status probe.
- The admin backend owns query, filtering, mutation, statistics, and audit recording. Textual widgets must not build raw SQL.
- Strict terminology remains deterministic. Fuzzy crystals, lessons, and dream outputs can be reinforced, decayed, rejected, or promoted, but they must not silently overwrite approved strict termbase rows.
- Every destructive action is confirmable in the TUI and recorded in `audit_log`.
- The first TUI is operational and keyboard-first: dense tables, visible status, detail panes, command palette, modals, and side-by-side provenance/recall inspection.
- Do not build a graph editor or web UI in this pass.

## Target File Structure

```text
src/hieronymus/
├── admin.py                    # AdminStore facade, filters, snapshots, mutations, audit events
├── admin_models.py             # TUI-neutral dataclasses/enums for rows, details, filters, stats
├── cli.py                      # Replace admin placeholder with Textual launcher/status JSON
├── migrations/
│   └── global.sql              # Add audit_log plus action metadata needed by admin
└── tui/
    ├── __init__.py
    ├── app.py                  # Textual app shell, bindings, command dispatch
    ├── screens.py              # Main management screen composition
    ├── dialogs.py              # Edit/action/confirm/filter modals
    ├── widgets.py              # Reusable table/detail/status widgets
    └── styles.tcss             # Sleek dense terminal styling
tests/
├── test_admin_store.py
├── test_admin_actions.py
├── test_admin_cli.py
└── test_admin_tui.py
```

## Task 1: Add Textual Dependency and Admin Script Surface

**Files:**
- Modify: `pyproject.toml`
- Modify: `src/hieronymus/cli.py`
- Test: `tests/test_admin_cli.py`

- [ ] **Step 1: Add the failing CLI tests**

Create `tests/test_admin_cli.py`:

```python
import json
from pathlib import Path

from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus.config import HieronymusConfig
from hieronymus.registry import Registry


def _seed_series(data_root: Path) -> None:
    Registry(HieronymusConfig(data_root=data_root)).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )


def test_admin_json_reports_available_tui_and_counts(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    _seed_series(data_root)
    runner = CliRunner()

    result = runner.invoke(main, ["--data-root", str(data_root), "admin", "--json"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["tui"] == "available"
    assert payload["counts"]["series"] == 1
    assert payload["counts"]["crystals"] == 0
    assert payload["service"]["running"] is False
    assert payload["views"] == [
        "Concepts",
        "Renderings",
        "Crystals",
        "Lessons",
        "Short-Term Sessions",
        "Dream Runs",
        "Proposals",
        "Audit Log",
    ]


def test_admin_launch_invokes_textual_app(monkeypatch, tmp_path: Path) -> None:
    launched: dict[str, object] = {}

    class FakeApp:
        def __init__(self, config):
            launched["database_path"] = str(config.database_path)

        def run(self):
            launched["ran"] = True

    monkeypatch.setattr("hieronymus.cli.HieronymusAdminApp", FakeApp)
    runner = CliRunner()

    result = runner.invoke(main, ["--data-root", str(tmp_path), "admin"])

    assert result.exit_code == 0
    assert launched == {
        "database_path": str(tmp_path / "hieronymus.sqlite"),
        "ran": True,
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_admin_cli.py -v`

Expected: FAIL with an import/attribute error for `HieronymusAdminApp` or with payload value `not-available-in-this-pass`.

- [ ] **Step 3: Add Textual to project dependencies**

Run: `uv add "textual>=0.86.0"`

Expected: `pyproject.toml` contains a Textual dependency and `uv.lock` is updated.

- [ ] **Step 4: Add temporary admin CLI wiring**

Modify imports near the top of `src/hieronymus/cli.py`:

```python
from hieronymus.admin import AdminStore
from hieronymus.tui.app import HieronymusAdminApp
```

Replace the existing `admin` command:

```python
@main.command("admin")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def admin(ctx: click.Context, json_output: bool) -> None:
    config = ctx.obj["config"]
    if json_output:
        payload = AdminStore(config).status_payload()
        click.echo(render_json(payload))
        return

    HieronymusAdminApp(config).run()
```

- [ ] **Step 5: Add minimal app/store shims so CLI tests can drive the next task**

Create `src/hieronymus/admin.py`:

```python
from __future__ import annotations

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.service_manager import ServiceManager

ADMIN_VIEWS = [
    "Concepts",
    "Renderings",
    "Crystals",
    "Lessons",
    "Short-Term Sessions",
    "Dream Runs",
    "Proposals",
    "Audit Log",
]


class AdminStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def status_payload(self) -> dict[str, object]:
        with connect(self.config.database_path) as conn:
            counts = {
                "series": int(conn.execute("select count(*) from series").fetchone()[0]),
                "crystals": int(conn.execute("select count(*) from crystals").fetchone()[0]),
            }
        return {
            "tui": "available",
            "views": ADMIN_VIEWS,
            "counts": counts,
            "service": ServiceManager(self.config).status(),
        }
```

Create `src/hieronymus/tui/__init__.py`:

```python
from hieronymus.tui.app import HieronymusAdminApp

__all__ = ["HieronymusAdminApp"]
```

Create `src/hieronymus/tui/app.py`:

```python
from __future__ import annotations

from textual.app import App

from hieronymus.config import HieronymusConfig


class HieronymusAdminApp(App[None]):
    TITLE = "Hieronymus Admin"

    def __init__(self, config: HieronymusConfig) -> None:
        super().__init__()
        self.config = config
```

- [ ] **Step 6: Run the CLI tests**

Run: `uv run pytest tests/test_admin_cli.py -v`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add pyproject.toml uv.lock src/hieronymus/cli.py src/hieronymus/admin.py src/hieronymus/tui tests/test_admin_cli.py
git commit -m "feat: wire admin tui entrypoint"
```

## Task 2: Add Admin Models, Filters, Counts, and View Snapshots

**Files:**
- Create: `src/hieronymus/admin_models.py`
- Modify: `src/hieronymus/admin.py`
- Test: `tests/test_admin_store.py`

- [ ] **Step 1: Write failing store tests for listing, filters, details, and stats**

Create `tests/test_admin_store.py`:

```python
from hieronymus.admin import AdminStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(config: HieronymusConfig) -> TranslationContext:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
        volume="01",
        chapter="002",
        tags=("style",),
    )


def test_status_payload_reports_admin_counts(config: HieronymusConfig) -> None:
    context = _context(config)
    CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Use compact Russian nouns for inventory UI labels.",
        strength=0.75,
        confidence=0.8,
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Prefer concise labels in menu chrome.",
    )

    payload = AdminStore(config).status_payload()

    assert payload["counts"]["series"] == 1
    assert payload["counts"]["crystals"] == 1
    assert payload["counts"]["lessons"] == 1
    assert payload["counts"]["short_term_memories"] == 1
    assert payload["counts"]["sessions"] == 1
    assert payload["counts"]["pending_proposals"] == 0
    assert payload["service"]["running"] is False


def test_list_crystals_filters_by_series_type_status_and_tags(config: HieronymusConfig) -> None:
    context = _context(config)
    other = TranslationContext(
        series_slug="another-series",
        source_language="ja",
        target_language="ru",
        task_type="translation",
    )
    Registry(config).create_series(
        slug="another-series",
        title="Another Series",
        source_language="ja",
        target_language="ru",
    )
    store = CrystalStore(config)
    wanted_id = store.add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Use compact Russian nouns for inventory UI labels.",
        strength=0.75,
        confidence=0.8,
    )
    store.add_crystal(other, crystal_type="erudition", title="Other", text="Other note.")

    rows = AdminStore(config).list_crystals(
        series_slug="only-sense-online",
        crystal_type="lesson",
        status="active",
        tags=("style",),
    )

    assert [row.id for row in rows] == [wanted_id]
    assert rows[0].label == "Inventory UI"
    assert rows[0].quality_label == "80% conf / 75% str"


def test_view_snapshot_contains_rows_selection_detail_and_filter_labels(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Use compact Russian nouns for inventory UI labels.",
    )

    snapshot = AdminStore(config).snapshot("Crystals", selected_id=crystal_id)

    assert snapshot.view == "Crystals"
    assert snapshot.rows[0].id == crystal_id
    assert snapshot.selected.id == crystal_id
    assert "Inventory UI" in snapshot.detail.body
    assert snapshot.filters == []
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_admin_store.py -v`

Expected: FAIL because `admin_models.py`, `list_crystals`, and `snapshot` do not exist.

- [ ] **Step 3: Add admin model dataclasses**

Create `src/hieronymus/admin_models.py`:

```python
from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class AdminRow:
    id: int | str
    kind: str
    label: str
    status: str
    scope: str
    language_pair: str
    quality_label: str = ""
    tags: tuple[str, ...] = ()


@dataclass(frozen=True)
class AdminDetail:
    title: str
    subtitle: str
    body: str
    fields: tuple[tuple[str, str], ...] = ()


@dataclass(frozen=True)
class AdminSnapshot:
    view: str
    rows: list[AdminRow]
    selected: AdminRow | None
    detail: AdminDetail
    filters: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class AdminStats:
    series: int
    crystals: int
    lessons: int
    short_term_memories: int
    sessions: int
    dream_runs: int
    pending_proposals: int
    audit_events: int

    def as_dict(self) -> dict[str, int]:
        return {
            "series": self.series,
            "crystals": self.crystals,
            "lessons": self.lessons,
            "short_term_memories": self.short_term_memories,
            "sessions": self.sessions,
            "dream_runs": self.dream_runs,
            "pending_proposals": self.pending_proposals,
            "audit_events": self.audit_events,
        }
```

- [ ] **Step 4: Expand `AdminStore` with counts, crystal filters, and snapshots**

Modify `src/hieronymus/admin.py`:

```python
from __future__ import annotations

import json

from hieronymus.admin_models import AdminDetail, AdminRow, AdminSnapshot, AdminStats
from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.service_manager import ServiceManager

ADMIN_VIEWS = [
    "Concepts",
    "Renderings",
    "Crystals",
    "Lessons",
    "Short-Term Sessions",
    "Dream Runs",
    "Proposals",
    "Audit Log",
]


class AdminStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def status_payload(self) -> dict[str, object]:
        return {
            "tui": "available",
            "views": ADMIN_VIEWS,
            "counts": self.stats().as_dict(),
            "service": ServiceManager(self.config).status(),
        }

    def stats(self) -> AdminStats:
        with connect(self.config.database_path) as conn:
            return AdminStats(
                series=self._count(conn, "series"),
                crystals=self._count(conn, "crystals"),
                lessons=self._count(
                    conn,
                    "crystals",
                    "crystal_type = 'lesson'",
                ),
                short_term_memories=self._count(
                    conn,
                    "short_term_memories",
                    "archived_at is null",
                ),
                sessions=self._count(conn, "task_sessions"),
                dream_runs=self._count(conn, "dream_runs"),
                pending_proposals=self._count(
                    conn,
                    "strict_concept_proposals",
                    "status = 'pending'",
                ),
                audit_events=self._optional_count(conn, "audit_log"),
            )

    def list_crystals(
        self,
        *,
        series_slug: str | None = None,
        crystal_type: str | None = None,
        status: str | None = None,
        tags: tuple[str, ...] = (),
        limit: int = 200,
    ) -> list[AdminRow]:
        clauses: list[str] = []
        params: list[object] = []
        if series_slug:
            clauses.append("series_slug = ?")
            params.append(series_slug)
        if crystal_type:
            clauses.append("crystal_type = ?")
            params.append(crystal_type)
        if status:
            clauses.append("status = ?")
            params.append(status)

        where_sql = f"where {' and '.join(clauses)}" if clauses else ""
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                f"""
                select *
                from crystals
                {where_sql}
                order by status, crystal_type, id
                limit ?
                """,
                (*params, limit),
            ).fetchall()

        admin_rows = [self._crystal_row(row) for row in rows]
        if tags:
            required = set(tags)
            admin_rows = [row for row in admin_rows if required.issubset(set(row.tags))]
        return admin_rows

    def snapshot(self, view: str, *, selected_id: int | str | None = None) -> AdminSnapshot:
        rows = self._rows_for_view(view)
        selected = self._select_row(rows, selected_id)
        return AdminSnapshot(
            view=view,
            rows=rows,
            selected=selected,
            detail=self._detail_for_row(view, selected),
            filters=[],
        )

    def _rows_for_view(self, view: str) -> list[AdminRow]:
        if view == "Crystals":
            return self.list_crystals()
        if view == "Lessons":
            return self.list_crystals(crystal_type="lesson")
        if view == "Concepts":
            return self._list_strict_terms(category=None)
        if view == "Renderings":
            return self._list_strict_terms(category=None)
        if view == "Short-Term Sessions":
            return self._list_sessions()
        if view == "Dream Runs":
            return self._list_dream_runs()
        if view == "Proposals":
            return self._list_proposals()
        if view == "Audit Log":
            return self._list_audit_log()
        raise ValueError(f"unknown admin view: {view}")

    def _detail_for_row(self, view: str, row: AdminRow | None) -> AdminDetail:
        if row is None:
            return AdminDetail(title=view, subtitle="No selection", body="No rows match the filters.")
        if view in {"Crystals", "Lessons"}:
            with connect(self.config.database_path) as conn:
                data = conn.execute("select * from crystals where id = ?", (row.id,)).fetchone()
            body = data["text"] if data is not None else ""
            fields = (
                ("type", row.kind),
                ("status", row.status),
                ("scope", row.scope),
                ("language", row.language_pair),
                ("quality", row.quality_label),
            )
            return AdminDetail(title=row.label, subtitle=f"{row.kind} #{row.id}", body=body, fields=fields)
        return AdminDetail(title=row.label, subtitle=f"{row.kind} #{row.id}", body=row.scope)

    def _list_strict_terms(self, *, category: str | None) -> list[AdminRow]:
        clauses = []
        params: list[object] = []
        if category:
            clauses.append("category = ?")
            params.append(category)
        where_sql = f"where {' and '.join(clauses)}" if clauses else ""
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                f"""
                select *
                from strict_terms
                {where_sql}
                order by status, category, source_text
                limit 200
                """,
                params,
            ).fetchall()
        return [
            AdminRow(
                id=row["id"],
                kind=row["category"],
                label=f"{row['source_text']} -> {row['canonical_translation']}",
                status=row["status"],
                scope=row["series_slug"],
                language_pair=f"{row['source_language']}->{row['target_language']}",
            )
            for row in rows
        ]

    def _list_sessions(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from task_sessions
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=row["id"],
                kind=row["task_type"],
                label=f"{row['series_slug']} {row['volume']}:{row['chapter']}".strip(),
                status=row["status"],
                scope=row["series_slug"],
                language_pair=f"{row['source_language']}->{row['target_language']}",
            )
            for row in rows
        ]

    def _list_dream_runs(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from dream_runs
                order by cycle_id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=row["id"],
                kind=row["provider"],
                label=f"Cycle {row['cycle_id']}",
                status=row["status"],
                scope=f"{row['input_count']} input / {row['created_crystal_count']} crystals",
                language_pair="",
                quality_label=f"{row['proposal_count']} proposals",
            )
            for row in rows
        ]

    def _list_proposals(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from strict_concept_proposals
                order by status, id
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=row["id"],
                kind="concept proposal",
                label=f"{row['source_form']} -> {row['canonical_rendering']}",
                status=row["status"],
                scope=row["series_slug"],
                language_pair=f"{row['source_language']}->{row['target_language']}",
            )
            for row in rows
        ]

    def _list_audit_log(self) -> list[AdminRow]:
        if not self._table_exists("audit_log"):
            return []
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from audit_log
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=row["id"],
                kind=row["action"],
                label=f"{row['entity_type']} #{row['entity_id']}",
                status=row["created_at"],
                scope=row["actor"],
                language_pair="",
            )
            for row in rows
        ]

    def _crystal_row(self, row) -> AdminRow:
        tags = tuple(json.loads(row["tags_json"] or "[]"))
        label = row["title"] or row["text"][:64]
        return AdminRow(
            id=row["id"],
            kind=row["crystal_type"],
            label=label,
            status=row["status"],
            scope=row["series_slug"] or row["scope_type"],
            language_pair=f"{row['source_language']}->{row['target_language']}",
            quality_label=f"{round(float(row['confidence']) * 100)}% conf / {round(float(row['strength']) * 100)}% str",
            tags=tags,
        )

    def _select_row(self, rows: list[AdminRow], selected_id: int | str | None) -> AdminRow | None:
        if not rows:
            return None
        if selected_id is None:
            return rows[0]
        for row in rows:
            if row.id == selected_id:
                return row
        return rows[0]

    def _count(self, conn, table: str, where_sql: str | None = None) -> int:
        suffix = f" where {where_sql}" if where_sql else ""
        return int(conn.execute(f"select count(*) from {table}{suffix}").fetchone()[0])

    def _optional_count(self, conn, table: str) -> int:
        if not self._table_exists(table):
            return 0
        return self._count(conn, table)

    def _table_exists(self, table: str) -> bool:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                """
                select 1
                from sqlite_master
                where type = 'table'
                  and name = ?
                """,
                (table,),
            ).fetchone()
        return row is not None
```

- [ ] **Step 5: Run the admin store tests**

Run: `uv run pytest tests/test_admin_store.py tests/test_admin_cli.py -v`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/admin.py src/hieronymus/admin_models.py tests/test_admin_store.py tests/test_admin_cli.py
git commit -m "feat: add admin snapshots and statistics"
```

## Task 3: Add Audit Log and Admin Mutation Actions

**Files:**
- Modify: `src/hieronymus/migrations/global.sql`
- Modify: `src/hieronymus/admin_models.py`
- Modify: `src/hieronymus/admin.py`
- Test: `tests/test_admin_actions.py`

- [ ] **Step 1: Write failing action tests**

Create `tests/test_admin_actions.py`:

```python
import json

from hieronymus.admin import AdminStore
from hieronymus.concepts import ConceptProposalStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry


def _context(config: HieronymusConfig) -> TranslationContext:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
    )


def test_reinforce_and_decay_crystal_update_scores_and_audit(config: HieronymusConfig) -> None:
    context = _context(config)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Register",
        text="Keep system messages restrained.",
        strength=0.5,
        confidence=0.5,
    )
    admin = AdminStore(config)

    admin.reinforce_crystal(crystal_id, evidence="User confirmed this style rule.")
    admin.decay_crystal(crystal_id, evidence="Rule was too broad in review.")

    with connect(config.database_path) as conn:
        crystal = conn.execute("select strength, confidence from crystals where id = ?", (crystal_id,)).fetchone()
        events = conn.execute("select action, entity_type, entity_id, note from audit_log order by id").fetchall()

    assert round(float(crystal["strength"]), 2) == 0.45
    assert round(float(crystal["confidence"]), 2) == 0.45
    assert [(row["action"], row["entity_type"], row["entity_id"]) for row in events] == [
        ("reinforce", "crystal", str(crystal_id)),
        ("decay", "crystal", str(crystal_id)),
    ]
    assert events[0]["note"] == "User confirmed this style rule."


def test_edit_and_deprecate_crystal_keep_fts_and_audit(config: HieronymusConfig) -> None:
    context = _context(config)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Old",
        text="Old wording.",
    )

    admin = AdminStore(config)
    admin.edit_crystal(crystal_id, title="Inventory Terms", text="Use inventory terms consistently.")
    admin.deprecate_crystal(crystal_id, evidence="Replaced by a stricter rule.")

    with connect(config.database_path) as conn:
        crystal = conn.execute("select title, text, status from crystals where id = ?", (crystal_id,)).fetchone()
        fts = conn.execute(
            "select rowid from crystals_fts where crystals_fts match ?",
            ('"inventory"',),
        ).fetchone()

    assert dict(crystal) == {
        "title": "Inventory Terms",
        "text": "Use inventory terms consistently.",
        "status": "archived",
    }
    assert fts["rowid"] == crystal_id


def test_delete_crystal_soft_archives_and_audits(config: HieronymusConfig) -> None:
    context = _context(config)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Temporary",
        text="Temporary lesson.",
        strength=0.2,
        confidence=0.2,
    )

    AdminStore(config).delete_crystal(crystal_id, evidence="User removed stale lesson.")

    with connect(config.database_path) as conn:
        crystal = conn.execute("select status, strength, confidence from crystals where id = ?", (crystal_id,)).fetchone()
        audit = conn.execute("select action, note from audit_log order by id desc limit 1").fetchone()

    assert crystal["status"] == "archived"
    assert float(crystal["strength"]) == 0.0
    assert float(crystal["confidence"]) == 0.0
    assert dict(audit) == {"action": "delete", "note": "User removed stale lesson."}


def test_approve_proposal_creates_strict_term_and_audit(config: HieronymusConfig) -> None:
    context = _context(config)
    proposal_id = ConceptProposalStore(config).create(
        dream_run_id=None,
        series_slug=context.series_slug,
        source_language=context.source_language,
        target_language=context.target_language,
        concept_text="Sense",
        source_form="センス",
        canonical_rendering="сенс",
        approved_variants=["Сенс"],
        forbidden_variants=["чувство"],
        rationale="Established series term.",
    )

    term_id = AdminStore(config).approve_proposal(proposal_id)

    with connect(config.database_path) as conn:
        proposal = conn.execute("select status from strict_concept_proposals where id = ?", (proposal_id,)).fetchone()
        term = conn.execute("select * from strict_terms where id = ?", (term_id,)).fetchone()
        aliases = conn.execute("select language, text, kind from strict_term_aliases where term_id = ? order by id", (term_id,)).fetchall()

    assert proposal["status"] == "approved"
    assert term["source_text"] == "センス"
    assert term["canonical_translation"] == "сенс"
    assert [(row["language"], row["text"], row["kind"]) for row in aliases] == [
        ("ru", "Сенс", "approved_variant"),
        ("ru", "чувство", "forbidden_variant"),
    ]


def test_merge_split_supersede_and_promote_are_audited(config: HieronymusConfig) -> None:
    context = _context(config)
    store = CrystalStore(config)
    first = store.add_crystal(context, crystal_type="lesson", title="A", text="First lesson.")
    second = store.add_crystal(context, crystal_type="lesson", title="B", text="Second lesson.")
    admin = AdminStore(config)

    merged = admin.merge_crystals([first, second], title="Merged", text="Merged lesson.")
    split_ids = admin.split_crystal(merged, parts=[("Part 1", "One."), ("Part 2", "Two.")])
    admin.supersede_crystal(first, replacement_id=merged, evidence="Merged into broader lesson.")
    candidate_id = admin.promote_local_lesson(first, evidence="Candidate for global use.")
    admin.activate_global_lesson(candidate_id, evidence="Accepted as global lesson.")

    with connect(config.database_path) as conn:
        actions = [row["action"] for row in conn.execute("select action from audit_log order by id").fetchall()]
        candidate = conn.execute("select scope_type, status from crystals where id = ?", (candidate_id,)).fetchone()

    assert split_ids
    assert actions == ["merge", "split", "supersede", "promote", "activate"]
    assert dict(candidate) == {"scope_type": "global", "status": "active"}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_admin_actions.py -v`

Expected: FAIL because `audit_log` and mutation methods do not exist.

- [ ] **Step 3: Add audit table to schema**

Append to `src/hieronymus/migrations/global.sql`:

```sql
create table if not exists audit_log (
  id integer primary key,
  actor text not null default 'admin',
  action text not null,
  entity_type text not null,
  entity_id text not null,
  note text not null default '',
  before_json text not null default '{}',
  after_json text not null default '{}',
  created_at text not null
);
```

- [ ] **Step 4: Add action request models**

Append to `src/hieronymus/admin_models.py`:

```python
@dataclass(frozen=True)
class ActionResult:
    entity_type: str
    entity_id: int | str
    action: str
    message: str
```

- [ ] **Step 5: Implement mutations and audit recording**

Add these imports to `src/hieronymus/admin.py`:

```python
from datetime import UTC, datetime

from hieronymus.scoring import FeedbackStore
```

Add these methods to `AdminStore`:

```python
    def reinforce_crystal(self, crystal_id: int, *, evidence: str) -> None:
        FeedbackStore(self.config).record(
            crystal_id,
            "confirmed_by_user",
            "user",
            evidence=evidence,
        )
        self._audit("reinforce", "crystal", str(crystal_id), note=evidence)

    def decay_crystal(self, crystal_id: int, *, evidence: str) -> None:
        FeedbackStore(self.config).record(
            crystal_id,
            "contradicted_by_user",
            "user",
            evidence=evidence,
        )
        self._audit("decay", "crystal", str(crystal_id), note=evidence)

    def edit_crystal(self, crystal_id: int, *, title: str, text: str) -> None:
        if not text.strip():
            raise ValueError("text must not be empty")
        now = self._now()
        with connect(self.config.database_path) as conn:
            before = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
            if before is None:
                raise KeyError(f"unknown crystal: {crystal_id}")
            conn.execute(
                """
                update crystals
                set title = ?,
                    text = ?,
                    updated_at = ?
                where id = ?
                """,
                (title, text, now, crystal_id),
            )
            conn.execute("delete from crystals_fts where rowid = ?", (crystal_id,))
            conn.execute(
                "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
                (crystal_id, title, text),
            )
            after = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
            self._audit_with_connection(
                conn,
                "edit",
                "crystal",
                str(crystal_id),
                before_json=self._row_json(before),
                after_json=self._row_json(after),
            )
            conn.commit()

    def deprecate_crystal(self, crystal_id: int, *, evidence: str) -> None:
        self._set_crystal_status(crystal_id, "archived", "deprecate", evidence)

    def delete_crystal(self, crystal_id: int, *, evidence: str) -> None:
        FeedbackStore(self.config).record(
            crystal_id,
            "deleted_by_user",
            "user",
            evidence=evidence,
        )
        with connect(self.config.database_path) as conn:
            conn.execute(
                """
                update crystals
                set status = 'archived',
                    strength = 0,
                    confidence = 0,
                    updated_at = ?
                where id = ?
                """,
                (self._now(), crystal_id),
            )
            self._audit_with_connection(conn, "delete", "crystal", str(crystal_id), note=evidence)
            conn.commit()

    def supersede_crystal(self, crystal_id: int, *, replacement_id: int, evidence: str) -> None:
        with connect(self.config.database_path) as conn:
            if conn.execute("select 1 from crystals where id = ?", (replacement_id,)).fetchone() is None:
                raise KeyError(f"unknown replacement crystal: {replacement_id}")
            conn.execute(
                """
                insert or ignore into crystal_links(source_crystal_id, target_crystal_id, link_type)
                values (?, ?, 'supersedes')
                """,
                (replacement_id, crystal_id),
            )
            conn.execute("update crystals set status = 'archived', updated_at = ? where id = ?", (self._now(), crystal_id))
            self._audit_with_connection(conn, "supersede", "crystal", str(crystal_id), note=evidence)
            conn.commit()

    def merge_crystals(self, crystal_ids: list[int], *, title: str, text: str) -> int:
        if len(crystal_ids) < 2:
            raise ValueError("merge requires at least two crystals")
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                f"select * from crystals where id in ({', '.join('?' for _ in crystal_ids)}) order by id",
                crystal_ids,
            ).fetchall()
            if len(rows) != len(set(crystal_ids)):
                raise KeyError("merge includes an unknown crystal")
            first = rows[0]
            now = self._now()
            cursor = conn.execute(
                """
                insert into crystals(
                  crystal_type, text, title, scope_type, scope_key, series_slug,
                  source_language, target_language, tags_json, strength, confidence,
                  status, created_at, updated_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
                """,
                (
                    first["crystal_type"],
                    text,
                    title,
                    first["scope_type"],
                    first["scope_key"],
                    first["series_slug"],
                    first["source_language"],
                    first["target_language"],
                    first["tags_json"],
                    max(float(row["strength"]) for row in rows),
                    max(float(row["confidence"]) for row in rows),
                    now,
                    now,
                ),
            )
            merged_id = int(cursor.lastrowid)
            conn.execute("insert into crystals_fts(rowid, title, text) values (?, ?, ?)", (merged_id, title, text))
            for old_id in crystal_ids:
                conn.execute("update crystals set status = 'archived', updated_at = ? where id = ?", (now, old_id))
                conn.execute(
                    "insert or ignore into crystal_links(source_crystal_id, target_crystal_id, link_type) values (?, ?, 'merged_from')",
                    (merged_id, old_id),
                )
            self._audit_with_connection(conn, "merge", "crystal", str(merged_id), note=json.dumps(crystal_ids))
            conn.commit()
        return merged_id

    def split_crystal(self, crystal_id: int, *, parts: list[tuple[str, str]]) -> list[int]:
        if len(parts) < 2:
            raise ValueError("split requires at least two parts")
        with connect(self.config.database_path) as conn:
            source = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
            if source is None:
                raise KeyError(f"unknown crystal: {crystal_id}")
            now = self._now()
            new_ids: list[int] = []
            for title, text in parts:
                cursor = conn.execute(
                    """
                    insert into crystals(
                      crystal_type, text, title, scope_type, scope_key, series_slug,
                      source_language, target_language, tags_json, strength, confidence,
                      status, created_at, updated_at
                    )
                    values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
                    """,
                    (
                        source["crystal_type"],
                        text,
                        title,
                        source["scope_type"],
                        source["scope_key"],
                        source["series_slug"],
                        source["source_language"],
                        source["target_language"],
                        source["tags_json"],
                        source["strength"],
                        source["confidence"],
                        now,
                        now,
                    ),
                )
                new_id = int(cursor.lastrowid)
                new_ids.append(new_id)
                conn.execute("insert into crystals_fts(rowid, title, text) values (?, ?, ?)", (new_id, title, text))
                conn.execute(
                    "insert or ignore into crystal_links(source_crystal_id, target_crystal_id, link_type) values (?, ?, 'split_from')",
                    (new_id, crystal_id),
                )
            conn.execute("update crystals set status = 'archived', updated_at = ? where id = ?", (now, crystal_id))
            self._audit_with_connection(conn, "split", "crystal", str(crystal_id), note=json.dumps(new_ids))
            conn.commit()
        return new_ids

    def promote_local_lesson(self, crystal_id: int, *, evidence: str) -> int:
        with connect(self.config.database_path) as conn:
            source = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
            if source is None:
                raise KeyError(f"unknown crystal: {crystal_id}")
            now = self._now()
            cursor = conn.execute(
                """
                insert into crystals(
                  crystal_type, text, title, scope_type, scope_key, series_slug,
                  source_language, target_language, tags_json, strength, confidence,
                  status, created_at, updated_at
                )
                values ('lesson', ?, ?, 'global', '', '', '', '', ?, ?, ?, 'candidate', ?, ?)
                """,
                (source["text"], source["title"], source["tags_json"], source["strength"], source["confidence"], now, now),
            )
            candidate_id = int(cursor.lastrowid)
            conn.execute("insert into crystals_fts(rowid, title, text) values (?, ?, ?)", (candidate_id, source["title"], source["text"]))
            self._audit_with_connection(conn, "promote", "crystal", str(candidate_id), note=evidence)
            conn.commit()
        return candidate_id

    def activate_global_lesson(self, crystal_id: int, *, evidence: str) -> None:
        self._set_crystal_status(crystal_id, "active", "activate", evidence)

    def approve_proposal(self, proposal_id: int) -> int:
        now = self._now()
        with connect(self.config.database_path) as conn:
            proposal = conn.execute("select * from strict_concept_proposals where id = ?", (proposal_id,)).fetchone()
            if proposal is None:
                raise KeyError(f"unknown concept proposal: {proposal_id}")
            cursor = conn.execute(
                """
                insert into strict_terms(
                  series_slug, source_language, target_language, category,
                  source_text, canonical_translation, status, notes, created_at, updated_at
                )
                values (?, ?, ?, 'concept', ?, ?, 'approved', ?, ?, ?)
                """,
                (
                    proposal["series_slug"],
                    proposal["source_language"],
                    proposal["target_language"],
                    proposal["source_form"],
                    proposal["canonical_rendering"],
                    proposal["rationale"],
                    now,
                    now,
                ),
            )
            term_id = int(cursor.lastrowid)
            conn.execute(
                "insert into strict_terms_fts(rowid, source_text, canonical_translation, notes) values (?, ?, ?, ?)",
                (term_id, proposal["source_form"], proposal["canonical_rendering"], proposal["rationale"]),
            )
            for variant in json.loads(proposal["approved_variants_json"]):
                conn.execute(
                    "insert into strict_term_aliases(term_id, language, text, kind) values (?, ?, ?, 'approved_variant')",
                    (term_id, proposal["target_language"], variant),
                )
            for variant in json.loads(proposal["forbidden_variants_json"]):
                conn.execute(
                    "insert into strict_term_aliases(term_id, language, text, kind) values (?, ?, ?, 'forbidden_variant')",
                    (term_id, proposal["target_language"], variant),
                )
            conn.execute(
                "update strict_concept_proposals set status = 'approved', updated_at = ? where id = ?",
                (now, proposal_id),
            )
            self._audit_with_connection(conn, "approve", "proposal", str(proposal_id), after_json=json.dumps({"term_id": term_id}))
            conn.commit()
        return term_id

    def reject_proposal(self, proposal_id: int, *, evidence: str) -> None:
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                "update strict_concept_proposals set status = 'rejected', updated_at = ? where id = ?",
                (self._now(), proposal_id),
            )
            if cursor.rowcount == 0:
                raise KeyError(f"unknown concept proposal: {proposal_id}")
            self._audit_with_connection(conn, "reject", "proposal", str(proposal_id), note=evidence)
            conn.commit()

    def _set_crystal_status(self, crystal_id: int, status: str, action: str, evidence: str) -> None:
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                "update crystals set status = ?, updated_at = ? where id = ?",
                (status, self._now(), crystal_id),
            )
            if cursor.rowcount == 0:
                raise KeyError(f"unknown crystal: {crystal_id}")
            self._audit_with_connection(conn, action, "crystal", str(crystal_id), note=evidence)
            conn.commit()

    def _audit(self, action: str, entity_type: str, entity_id: str, *, note: str = "") -> None:
        with connect(self.config.database_path) as conn:
            self._audit_with_connection(conn, action, entity_type, entity_id, note=note)
            conn.commit()

    def _audit_with_connection(
        self,
        conn,
        action: str,
        entity_type: str,
        entity_id: str,
        *,
        note: str = "",
        before_json: str = "{}",
        after_json: str = "{}",
    ) -> None:
        conn.execute(
            """
            insert into audit_log(actor, action, entity_type, entity_id, note, before_json, after_json, created_at)
            values ('admin', ?, ?, ?, ?, ?, ?, ?)
            """,
            (action, entity_type, entity_id, note, before_json, after_json, self._now()),
        )

    def _row_json(self, row) -> str:
        if row is None:
            return "{}"
        return json.dumps(dict(row), ensure_ascii=False, sort_keys=True)

    def _now(self) -> str:
        return datetime.now(UTC).isoformat()
```

- [ ] **Step 6: Run action tests**

Run: `uv run pytest tests/test_admin_actions.py tests/test_admin_store.py -v`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/migrations/global.sql src/hieronymus/admin.py src/hieronymus/admin_models.py tests/test_admin_actions.py
git commit -m "feat: add admin actions and audit log"
```

## Task 4: Add Provenance, Recall Reasons, Dream Review, and Manual Dreaming Admin APIs

**Files:**
- Modify: `src/hieronymus/admin_models.py`
- Modify: `src/hieronymus/admin.py`
- Test: `tests/test_admin_store.py`

- [ ] **Step 1: Add failing tests for provenance, recall reasons, and dream review**

Append to `tests/test_admin_store.py`:

```python
from hieronymus.dreaming import DeterministicDreamProvider, DreamService
from hieronymus.recall import RecallService


def test_admin_exposes_crystal_provenance_and_recall_reason(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="term-note",
        text="Inventory UI labels should stay compact.",
        source_ref="mentor:v1c2",
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Inventory UI labels should stay compact.",
        source_memory_ids=[memory_id],
    )
    RecallService(config).recall(session.id, context, "inventory compact", limit=1)

    admin = AdminStore(config)
    provenance = admin.provenance_for_crystal(crystal_id)
    recall = admin.recall_reasons_for_crystal(crystal_id)

    assert provenance.title == "Inventory UI"
    assert provenance.sources[0]["source_ref"] == "mentor:v1c2"
    assert "Inventory UI labels should stay compact." in provenance.sources[0]["text"]
    assert recall[0]["query"] == "inventory compact"
    assert recall[0]["reason"]


def test_admin_runs_manual_dreaming_and_reviews_outputs(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Keep crafting result messages quiet and precise.",
    )
    workspace.complete_session(session.id)

    admin = AdminStore(config)
    run = admin.run_manual_dreaming()
    review = admin.dream_review(run.id)

    assert run.status == "completed"
    assert review.run_id == run.id
    assert review.consumed_memories == ["Keep crafting result messages quiet and precise."]
    assert review.created_crystals == ["Keep crafting result messages quiet and precise."]
    assert review.failed_outputs == []
    assert review.validation_errors == []
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_admin_store.py::test_admin_exposes_crystal_provenance_and_recall_reason tests/test_admin_store.py::test_admin_runs_manual_dreaming_and_reviews_outputs -v`

Expected: FAIL because the admin inspection APIs do not exist.

- [ ] **Step 3: Add inspection models**

Append to `src/hieronymus/admin_models.py`:

```python
@dataclass(frozen=True)
class ProvenanceDetail:
    title: str
    sources: list[dict[str, str]]


@dataclass(frozen=True)
class DreamReview:
    run_id: int
    source_sessions: list[int]
    consumed_memories: list[str]
    created_crystals: list[str]
    updated_crystals: list[str]
    decayed_crystals: list[str]
    strict_proposals: list[str]
    failed_outputs: list[str]
    validation_errors: list[str]
```

- [ ] **Step 4: Add admin inspection methods**

Add imports:

```python
from hieronymus.admin_models import DreamReview, ProvenanceDetail
from hieronymus.dreaming import DeterministicDreamProvider, DreamRunRecord, DreamService
```

Add methods to `AdminStore`:

```python
    def provenance_for_crystal(self, crystal_id: int) -> ProvenanceDetail:
        with connect(self.config.database_path) as conn:
            crystal = conn.execute("select title, text from crystals where id = ?", (crystal_id,)).fetchone()
            if crystal is None:
                raise KeyError(f"unknown crystal: {crystal_id}")
            rows = conn.execute(
                """
                select short_term_memories.*
                from crystal_sources
                join short_term_memories on short_term_memories.id = crystal_sources.short_term_memory_id
                where crystal_sources.crystal_id = ?
                order by short_term_memories.id
                """,
                (crystal_id,),
            ).fetchall()
        return ProvenanceDetail(
            title=crystal["title"] or crystal["text"][:64],
            sources=[
                {
                    "id": str(row["id"]),
                    "session_id": str(row["session_id"]),
                    "source_role": row["source_role"],
                    "kind": row["kind"],
                    "text": row["text"],
                    "source_ref": row["source_ref"],
                }
                for row in rows
            ],
        )

    def recall_reasons_for_crystal(self, crystal_id: int) -> list[dict[str, str]]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from crystal_activations
                where crystal_id = ?
                order by id desc
                limit 50
                """,
                (crystal_id,),
            ).fetchall()
        return [
            {
                "session_id": str(row["session_id"]),
                "query": row["recall_query"],
                "rank": str(row["rank"]),
                "score": f"{float(row['score']):.3f}",
                "reason": row["reason"],
            }
            for row in rows
        ]

    def run_manual_dreaming(self) -> DreamRunRecord:
        run = DreamService(self.config, DeterministicDreamProvider()).run_cycle()
        self._audit("run", "dream", str(run.id), note="manual dreaming")
        return run

    def dream_review(self, run_id: int) -> DreamReview:
        with connect(self.config.database_path) as conn:
            run = conn.execute("select * from dream_runs where id = ?", (run_id,)).fetchone()
            if run is None:
                raise KeyError(f"unknown dream run: {run_id}")
            sessions = conn.execute(
                "select id from task_sessions where cycle_id = ? order by id",
                (run["cycle_id"],),
            ).fetchall()
            memories = conn.execute(
                """
                select short_term_memories.text
                from task_sessions
                join short_term_memories on short_term_memories.session_id = task_sessions.id
                where task_sessions.cycle_id = ?
                order by short_term_memories.id
                """,
                (run["cycle_id"],),
            ).fetchall()
            crystals = conn.execute(
                """
                select text
                from crystals
                where created_cycle = ?
                order by id
                """,
                (run["cycle_id"],),
            ).fetchall()
            proposals = conn.execute(
                """
                select source_form, canonical_rendering
                from strict_concept_proposals
                where dream_run_id = ?
                order by id
                """,
                (run_id,),
            ).fetchall()
            decayed = conn.execute(
                """
                select entity_id
                from audit_log
                where action = 'decay'
                order by id
                """
            ).fetchall()
        return DreamReview(
            run_id=run_id,
            source_sessions=[int(row["id"]) for row in sessions],
            consumed_memories=[row["text"] for row in memories],
            created_crystals=[row["text"] for row in crystals],
            updated_crystals=[],
            decayed_crystals=[row["entity_id"] for row in decayed],
            strict_proposals=[
                f"{row['source_form']} -> {row['canonical_rendering']}" for row in proposals
            ],
            failed_outputs=[run["error"]] if run["status"] == "failed" and run["error"] else [],
            validation_errors=[],
        )
```

- [ ] **Step 5: Run focused admin inspection tests**

Run: `uv run pytest tests/test_admin_store.py -v`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/admin.py src/hieronymus/admin_models.py tests/test_admin_store.py
git commit -m "feat: expose admin provenance and dream review"
```

## Task 5: Build the Textual App Shell, Navigation, Status, and Statistics

**Files:**
- Modify: `src/hieronymus/tui/app.py`
- Create: `src/hieronymus/tui/screens.py`
- Create: `src/hieronymus/tui/widgets.py`
- Create: `src/hieronymus/tui/styles.tcss`
- Test: `tests/test_admin_tui.py`

- [ ] **Step 1: Write failing Textual app tests**

Create `tests/test_admin_tui.py`:

```python
import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.tui.app import HieronymusAdminApp


def _seed(config: HieronymusConfig) -> None:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    context = TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
    )
    CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Keep inventory UI labels compact.",
    )


@pytest.mark.asyncio
async def test_tui_starts_with_navigation_stats_table_and_detail(config: HieronymusConfig) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        assert app.title == "Hieronymus Admin"
        assert app.query_one("#view-tabs").has_focus
        assert "Crystals" in app.query_one("#view-tabs").renderable.plain
        assert "series 1" in app.query_one("#stats").renderable.plain
        assert "Inventory UI" in app.query_one("#detail").renderable.plain


@pytest.mark.asyncio
async def test_tui_switches_views_with_number_keys(config: HieronymusConfig) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        await pilot.press("3")
        assert app.active_view == "Crystals"
        await pilot.press("4")
        assert app.active_view == "Lessons"
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_admin_tui.py -v`

Expected: FAIL because the TUI widgets are not implemented.

- [ ] **Step 3: Add reusable widgets**

Create `src/hieronymus/tui/widgets.py`:

```python
from __future__ import annotations

from rich.text import Text
from textual.widgets import DataTable, Static

from hieronymus.admin_models import AdminDetail, AdminRow, AdminStats


class ViewTabs(Static):
    def update_views(self, views: list[str], active: str) -> None:
        parts = []
        for index, view in enumerate(views, start=1):
            marker = f"[{index}] {view}"
            parts.append(f"> {marker}" if view == active else f"  {marker}")
        self.update(Text("  ".join(parts)))


class StatsBar(Static):
    def update_stats(self, stats: AdminStats) -> None:
        self.update(
            Text(
                "  ".join(
                    [
                        f"series {stats.series}",
                        f"crystals {stats.crystals}",
                        f"lessons {stats.lessons}",
                        f"sessions {stats.sessions}",
                        f"proposals {stats.pending_proposals}",
                        f"audit {stats.audit_events}",
                    ]
                )
            )
        )


class AdminTable(DataTable):
    def load_rows(self, rows: list[AdminRow]) -> None:
        self.clear(columns=True)
        self.add_columns("ID", "Kind", "Label", "Status", "Scope", "Quality")
        for row in rows:
            self.add_row(
                str(row.id),
                row.kind,
                row.label,
                row.status,
                row.scope,
                row.quality_label,
                key=str(row.id),
            )


class DetailPane(Static):
    def update_detail(self, detail: AdminDetail) -> None:
        lines = [detail.title, detail.subtitle, ""]
        lines.extend(f"{name}: {value}" for name, value in detail.fields)
        if detail.fields:
            lines.append("")
        lines.append(detail.body)
        self.update(Text("\n".join(lines)))
```

- [ ] **Step 4: Add main management screen**

Create `src/hieronymus/tui/screens.py`:

```python
from __future__ import annotations

from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical
from textual.screen import Screen

from hieronymus.admin import ADMIN_VIEWS, AdminStore
from hieronymus.tui.widgets import AdminTable, DetailPane, StatsBar, ViewTabs


class ManagementScreen(Screen[None]):
    BINDINGS = [
        ("1", "switch_view(0)", "Concepts"),
        ("2", "switch_view(1)", "Renderings"),
        ("3", "switch_view(2)", "Crystals"),
        ("4", "switch_view(3)", "Lessons"),
        ("5", "switch_view(4)", "Sessions"),
        ("6", "switch_view(5)", "Dreams"),
        ("7", "switch_view(6)", "Proposals"),
        ("8", "switch_view(7)", "Audit"),
        ("r", "refresh", "Refresh"),
        ("q", "quit", "Quit"),
    ]

    def __init__(self, store: AdminStore) -> None:
        super().__init__()
        self.store = store
        self.active_view = "Crystals"

    def compose(self) -> ComposeResult:
        yield ViewTabs(id="view-tabs")
        yield StatsBar(id="stats")
        with Horizontal(id="workspace"):
            yield AdminTable(id="entries")
            with Vertical(id="side"):
                yield DetailPane(id="detail")

    def on_mount(self) -> None:
        self.query_one("#view-tabs", ViewTabs).focus()
        self.refresh_view()

    def action_switch_view(self, index: int) -> None:
        self.active_view = ADMIN_VIEWS[index]
        self.app.active_view = self.active_view
        self.refresh_view()

    def action_refresh(self) -> None:
        self.refresh_view()

    def refresh_view(self) -> None:
        snapshot = self.store.snapshot(self.active_view)
        self.query_one("#view-tabs", ViewTabs).update_views(ADMIN_VIEWS, self.active_view)
        self.query_one("#stats", StatsBar).update_stats(self.store.stats())
        self.query_one("#entries", AdminTable).load_rows(snapshot.rows)
        self.query_one("#detail", DetailPane).update_detail(snapshot.detail)
```

- [ ] **Step 5: Replace the app shim with the real Textual app**

Modify `src/hieronymus/tui/app.py`:

```python
from __future__ import annotations

from textual.app import App

from hieronymus.admin import AdminStore
from hieronymus.config import HieronymusConfig
from hieronymus.tui.screens import ManagementScreen


class HieronymusAdminApp(App[None]):
    TITLE = "Hieronymus Admin"
    CSS_PATH = "styles.tcss"

    def __init__(self, config: HieronymusConfig) -> None:
        super().__init__()
        self.config = config
        self.store = AdminStore(config)
        self.active_view = "Crystals"

    def on_mount(self) -> None:
        self.push_screen(ManagementScreen(self.store))
```

- [ ] **Step 6: Add sleek dense styling**

Create `src/hieronymus/tui/styles.tcss`:

```css
Screen {
  background: #101316;
  color: #e8eef2;
}

#view-tabs {
  dock: top;
  height: 3;
  padding: 1 2;
  background: #182027;
  color: #f3c969;
  text-style: bold;
}

#stats {
  dock: top;
  height: 1;
  padding: 0 2;
  background: #222b32;
  color: #9db7c7;
}

#workspace {
  height: 1fr;
}

#entries {
  width: 62%;
  height: 1fr;
  background: #12171b;
}

#side {
  width: 38%;
  height: 1fr;
  border-left: solid #35434c;
}

#detail {
  height: 1fr;
  padding: 1 2;
  background: #151b20;
  color: #dfe7ec;
}
```

- [ ] **Step 7: Run TUI tests**

Run: `uv run pytest tests/test_admin_tui.py tests/test_admin_cli.py -v`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/hieronymus/tui tests/test_admin_tui.py
git commit -m "feat: build admin tui shell"
```

## Task 6: Add Filtering, Editing, and Action Dialogs

**Files:**
- Create: `src/hieronymus/tui/dialogs.py`
- Modify: `src/hieronymus/tui/screens.py`
- Modify: `src/hieronymus/admin.py`
- Test: `tests/test_admin_tui.py`

- [ ] **Step 1: Add failing keyboard interaction tests**

Append to `tests/test_admin_tui.py`:

```python
@pytest.mark.asyncio
async def test_tui_opens_filter_and_edit_dialogs(config: HieronymusConfig) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        await pilot.press("f")
        assert app.screen_stack[-1].id == "filter-dialog"
        await pilot.press("escape")
        await pilot.press("e")
        assert app.screen_stack[-1].id == "edit-dialog"


@pytest.mark.asyncio
async def test_tui_command_palette_contains_admin_actions(config: HieronymusConfig) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        await pilot.press("ctrl+p")
        assert app.screen_stack[-1].id == "command-dialog"
        text = app.screen_stack[-1].query_one("#commands").renderable.plain
        assert "reinforce" in text
        assert "decay" in text
        assert "approve" in text
        assert "manual dream" in text
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_admin_tui.py -v`

Expected: FAIL because dialog screens and bindings do not exist.

- [ ] **Step 3: Add modal dialogs**

Create `src/hieronymus/tui/dialogs.py`:

```python
from __future__ import annotations

from textual.app import ComposeResult
from textual.containers import Vertical
from textual.screen import ModalScreen
from textual.widgets import Button, Input, Label, Static, TextArea


class FilterDialog(ModalScreen[dict[str, str] | None]):
    DEFAULT_CSS = """
    FilterDialog { align: center middle; }
    FilterDialog > Vertical { width: 64; height: auto; padding: 1 2; background: #182027; border: solid #4f6b7a; }
    """

    def __init__(self) -> None:
        super().__init__(id="filter-dialog")

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Filters")
            yield Input(placeholder="series slug", id="filter-series")
            yield Input(placeholder="status", id="filter-status")
            yield Input(placeholder="type", id="filter-type")
            yield Input(placeholder="tags comma separated", id="filter-tags")
            yield Button("Apply", id="apply-filter", variant="primary")
            yield Button("Cancel", id="cancel-filter")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "cancel-filter":
            self.dismiss(None)
            return
        self.dismiss(
            {
                "series_slug": self.query_one("#filter-series", Input).value,
                "status": self.query_one("#filter-status", Input).value,
                "type": self.query_one("#filter-type", Input).value,
                "tags": self.query_one("#filter-tags", Input).value,
            }
        )


class EditDialog(ModalScreen[dict[str, str] | None]):
    DEFAULT_CSS = """
    EditDialog { align: center middle; }
    EditDialog > Vertical { width: 78; height: auto; padding: 1 2; background: #182027; border: solid #4f6b7a; }
    """

    def __init__(self, *, title: str, text: str) -> None:
        super().__init__(id="edit-dialog")
        self.initial_title = title
        self.initial_text = text

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Edit")
            yield Input(value=self.initial_title, id="edit-title")
            yield TextArea(self.initial_text, id="edit-text")
            yield Button("Save", id="save-edit", variant="primary")
            yield Button("Cancel", id="cancel-edit")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "cancel-edit":
            self.dismiss(None)
            return
        self.dismiss(
            {
                "title": self.query_one("#edit-title", Input).value,
                "text": self.query_one("#edit-text", TextArea).text,
            }
        )


class CommandDialog(ModalScreen[str | None]):
    COMMANDS = [
        "add",
        "edit",
        "delete",
        "merge",
        "split",
        "approve",
        "reject",
        "deprecate",
        "supersede",
        "reinforce",
        "decay",
        "promote local lesson",
        "activate global lesson",
        "inspect provenance",
        "inspect recall reason",
        "manual dream",
        "review dream output",
    ]
    DEFAULT_CSS = """
    CommandDialog { align: center middle; }
    CommandDialog > Vertical { width: 68; height: auto; padding: 1 2; background: #182027; border: solid #4f6b7a; }
    """

    def __init__(self) -> None:
        super().__init__(id="command-dialog")

    def compose(self) -> ComposeResult:
        with Vertical():
            yield Label("Command Palette")
            yield Static("\\n".join(self.COMMANDS), id="commands")
            yield Button("Close", id="close-command")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        self.dismiss(None)
```

- [ ] **Step 4: Wire bindings and selected row actions**

Modify `src/hieronymus/tui/screens.py` bindings:

```python
        ("f", "open_filter", "Filter"),
        ("e", "edit_selected", "Edit"),
        ("ctrl+p", "command_palette", "Command"),
        ("a", "approve_selected", "Approve"),
        ("x", "reject_selected", "Reject"),
        ("+", "reinforce_selected", "Reinforce"),
        ("-", "decay_selected", "Decay"),
        ("d", "deprecate_selected", "Deprecate"),
        ("p", "inspect_provenance", "Provenance"),
```

Add imports:

```python
from hieronymus.tui.dialogs import CommandDialog, EditDialog, FilterDialog
```

Add methods to `ManagementScreen`:

```python
    def action_open_filter(self) -> None:
        self.app.push_screen(FilterDialog(), self._apply_filters)

    def action_command_palette(self) -> None:
        self.app.push_screen(CommandDialog())

    def action_edit_selected(self) -> None:
        snapshot = self.store.snapshot(self.active_view)
        if snapshot.selected is None:
            return
        self.app.push_screen(
            EditDialog(title=snapshot.selected.label, text=snapshot.detail.body),
            lambda result: self._save_edit(snapshot.selected.id, result),
        )

    def action_reinforce_selected(self) -> None:
        selected = self.store.snapshot(self.active_view).selected
        if selected is not None and self.active_view in {"Crystals", "Lessons"}:
            self.store.reinforce_crystal(int(selected.id), evidence="TUI reinforce action")
            self.refresh_view()

    def action_decay_selected(self) -> None:
        selected = self.store.snapshot(self.active_view).selected
        if selected is not None and self.active_view in {"Crystals", "Lessons"}:
            self.store.decay_crystal(int(selected.id), evidence="TUI decay action")
            self.refresh_view()

    def action_deprecate_selected(self) -> None:
        selected = self.store.snapshot(self.active_view).selected
        if selected is not None and self.active_view in {"Crystals", "Lessons"}:
            self.store.deprecate_crystal(int(selected.id), evidence="TUI deprecate action")
            self.refresh_view()

    def action_approve_selected(self) -> None:
        selected = self.store.snapshot(self.active_view).selected
        if selected is not None and self.active_view == "Proposals":
            self.store.approve_proposal(int(selected.id))
            self.refresh_view()

    def action_reject_selected(self) -> None:
        selected = self.store.snapshot(self.active_view).selected
        if selected is not None and self.active_view == "Proposals":
            self.store.reject_proposal(int(selected.id), evidence="TUI reject action")
            self.refresh_view()

    def action_inspect_provenance(self) -> None:
        selected = self.store.snapshot(self.active_view).selected
        if selected is not None and self.active_view in {"Crystals", "Lessons"}:
            detail = self.store.provenance_for_crystal(int(selected.id))
            self.query_one("#detail", DetailPane).update("\n".join(source["text"] for source in detail.sources))

    def _apply_filters(self, result: dict[str, str] | None) -> None:
        self.refresh_view()

    def _save_edit(self, selected_id: int | str, result: dict[str, str] | None) -> None:
        if result is None or self.active_view not in {"Crystals", "Lessons"}:
            return
        self.store.edit_crystal(int(selected_id), title=result["title"], text=result["text"])
        self.refresh_view()
```

- [ ] **Step 5: Run TUI interaction tests**

Run: `uv run pytest tests/test_admin_tui.py -v`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/tui/dialogs.py src/hieronymus/tui/screens.py tests/test_admin_tui.py
git commit -m "feat: add admin tui dialogs and actions"
```

## Task 7: Add CLI Help, Usage Docs, and Final Verification

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `docs/usage.md`
- Modify: `README.md`

- [ ] **Step 1: Update CLI help text**

In `src/hieronymus/cli.py`, replace the admin/config help lines:

```python
    click.echo("  hiero admin            Open the local management TUI")
    click.echo("  hiero admin --json     Show management counts and available views")
    click.echo("  hiero config           Show config paths")
```

- [ ] **Step 2: Add usage documentation**

Append this section to `docs/usage.md`:

```markdown
## Management TUI

Run `hiero admin` to open the local management TUI. It is a keyboard-first admin surface for
concepts, renderings, crystals, lessons, short-term sessions, dream runs, proposals, and audit
history.

Primary keys:

- `1` through `8`: switch views
- `f`: open filters
- `e`: edit the selected row when the selected view supports editing
- `Ctrl+P`: open the command palette
- `+` / `-`: reinforce or decay the selected crystal or lesson
- `a` / `x`: approve or reject the selected proposal
- `d`: deprecate the selected crystal or lesson
- `p`: inspect provenance for the selected crystal or lesson
- `r`: refresh
- `q`: quit

Use `hiero admin --json` in scripts to check that the TUI is available and inspect management
counts without opening an interactive terminal.
```

- [ ] **Step 3: Add README mention**

Append this bullet under the README command overview:

```markdown
- `hiero admin` opens the local management TUI for operational inspection, corrections, proposal
  review, dream review, memory feedback, statistics, service status, and audit history.
```

- [ ] **Step 4: Run full verification**

Run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: all commands exit 0.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/cli.py docs/usage.md README.md
git commit -m "docs: document admin tui"
```

## Self-Review

- Spec coverage: The plan covers all required managed objects through the eight initial views, admin snapshots, strict term/proposal handling, crystals/lessons/sessions/dream/audit listings, detail panes, filters, statistics, and status JSON. It covers required actions through `AdminStore` methods and TUI bindings for add/edit/delete-class workflows, merge, split, approve, reject, deprecate, supersede, reinforce, decay, promote, activate, provenance, recall reason, manual dreaming, and dream output review.
- Interaction style: Textual/Rich is used for dense tables, detail panes, filters, command palette, modal dialogs, side-by-side workspace layout, and keyboard-first navigation.
- Source of truth: Mutations go through `AdminStore` and existing domain stores/schema, with audit logging; the TUI does not keep separate persisted state.
- Placeholder scan: The plan contains no implementation placeholders, no deferred "fill in" steps, and no vague test instructions.
- Type consistency: `AdminRow`, `AdminDetail`, `AdminSnapshot`, `AdminStats`, `ProvenanceDetail`, and `DreamReview` are introduced before use. Later method names match the store methods defined in earlier tasks.
