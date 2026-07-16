# Project-local Skills Installation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add local `hiero skills install` and `hiero skills uninstall` commands that install every Hieronymus skill under selected `.agents` and `.claude` project directories.

**Architecture:** Put project path validation, asset enumeration, plans, and mutation in a small `project_skills` module. Keep the Click group limited to selecting targets, displaying results, and calling the service. It must not alter the global agent/MCP installer.

**Tech Stack:** Python 3.12, Click, pytest, pathlib, `agent_assets.asset_map()`, `atomic_write_text()`.

## Global Constraints

- Install the entire `hieronymus-*` bundle with no per-skill selector.
- Targets are exactly `.agents/skills` and `.claude/skills` relative to the current directory.
- Owned skill directories overwrite without confirmation; unrelated skills and parents survive uninstall.
- `--target agents` and `--target claude` are repeatable. Non-TTY calls without one are usage errors.
- No MCP, manifest, or global configuration mutations.

---

### Task 1: Project skills service

**Files:**
- Create: `src/hieronymus/project_skills.py`
- Create: `tests/test_project_skills.py`

**Interfaces:**
- Consumes: `asset_map() -> dict[str, str]`.
- Produces: `ProjectSkillPlan`; `install_project_skills(workspace: Path, targets: tuple[str, ...], *, dry_run: bool = False)`; `uninstall_project_skills(...)`.

- [ ] **Step 1: Write failing service tests**

```python
def test_install_writes_every_skill_to_both_targets(tmp_path: Path) -> None:
    install_project_skills(tmp_path, ("agents", "claude"))
    assert (tmp_path / ".agents/skills/hieronymus-recall/SKILL.md").is_file()
    assert (tmp_path / ".claude/skills/hieronymus-recall/SKILL.md").is_file()

def test_overwrite_owned_skill_preserves_custom_skill(tmp_path: Path) -> None:
    owned = tmp_path / ".agents/skills/hieronymus-read/SKILL.md"
    custom = tmp_path / ".agents/skills/custom/SKILL.md"
    owned.parent.mkdir(parents=True); custom.parent.mkdir(parents=True)
    owned.write_text("old"); custom.write_text("custom")
    install_project_skills(tmp_path, ("agents",))
    assert owned.read_text() != "old"
    assert custom.read_text() == "custom"

def test_uninstall_removes_only_owned_skill_directories(tmp_path: Path) -> None:
    install_project_skills(tmp_path, ("agents",))
    custom = tmp_path / ".agents/skills/custom/SKILL.md"
    custom.parent.mkdir(parents=True); custom.write_text("custom")
    uninstall_project_skills(tmp_path, ("agents",))
    assert not (tmp_path / ".agents/skills/hieronymus-read").exists()
    assert custom.read_text() == "custom"
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_project_skills.py -v`

Expected: collection fails because `hieronymus.project_skills` is absent.

- [ ] **Step 3: Implement the service**

```python
def skill_assets() -> dict[str, str]:
    return {
        path.removeprefix("skills/"): text
        for path, text in asset_map().items()
        if path.startswith("skills/") and path.endswith("/SKILL.md")
    }

def install_project_skills(workspace: Path, targets: tuple[str, ...], *, dry_run: bool = False) -> ProjectSkillPlan:
    plan = plan_project_skills(workspace, "install", targets, dry_run=dry_run)
    if not dry_run:
        for root in target_roots(workspace, targets):
            for relative_path, text in skill_assets().items():
                atomic_write_text(root / relative_path, text)
    return plan
```

Validate target names and reject symlink workspace/target roots. Use `shutil.rmtree()` during uninstall only for a validated direct `hieronymus-*` child containing `SKILL.md`; do not delete `skills` or its parents. Add dry-run and duplicate target tests.

- [ ] **Step 4: Verify green**

Run: `uv run pytest tests/test_project_skills.py -v`

Expected: all service cases pass.

- [ ] **Step 5: Commit**

Run: `git add src/hieronymus/project_skills.py tests/test_project_skills.py && git commit -m "feat: add project-local skills bundle service"`

### Task 2: Click command group

**Files:**
- Modify: `src/hieronymus/cli.py`
- Create: `tests/test_cli_project_skills.py`

**Interfaces:**
- Consumes: Task 1 service functions.
- Produces: `hiero skills install` and `hiero skills uninstall`; repeatable `--target`, `--dry-run`, and `--yes`.

- [ ] **Step 1: Write failing CLI tests**

```python
def test_install_accepts_two_explicit_targets(runner: CliRunner) -> None:
    result = runner.invoke(main, ["skills", "install", "--target", "agents", "--target", "claude"])
    assert result.exit_code == 0
    assert Path(".agents/skills/hieronymus-read/SKILL.md").is_file()
    assert Path(".claude/skills/hieronymus-read/SKILL.md").is_file()

def test_noninteractive_install_without_target_is_usage_error(runner: CliRunner) -> None:
    result = runner.invoke(main, ["skills", "install"])
    assert result.exit_code == 2
    assert "supply at least one --target" in result.output
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_cli_project_skills.py -v`

Expected: fails because the `skills` group is unregistered.

- [ ] **Step 3: Implement target resolution and commands**

```python
@main.group("skills")
def skills_group() -> None:
    """Install the full Hieronymus workflow skill bundle in this project."""

@skills_group.command("install")
@click.option("--target", "targets", type=click.Choice(["agents", "claude"]), multiple=True)
@click.option("--dry-run", is_flag=True)
@click.option("--yes", "assume_yes", is_flag=True)
def skills_install(targets: tuple[str, ...], dry_run: bool, assume_yes: bool) -> None:
    selected = _project_skill_targets(targets, assume_yes=assume_yes)
    _echo_project_skill_plan(install_project_skills(Path.cwd(), selected, dry_run=dry_run))
```

Deduplicate targets in `agents`, `claude` order. If `click.get_text_stream("stdin").isatty()` is false and no target is supplied, raise `click.UsageError("supply at least one --target: agents or claude")`. In a TTY show a two-choice multi-select and let `--yes` accept it. Use the same selection helper for uninstall; test dry-run, overwrite-without-prompt, and preservation.

- [ ] **Step 4: Verify green**

Run: `uv run pytest tests/test_cli_project_skills.py -v`

Expected: all CLI cases pass.

- [ ] **Step 5: Commit**

Run: `git add src/hieronymus/cli.py tests/test_cli_project_skills.py && git commit -m "feat: add project skills install commands"`

### Task 3: Documentation and final verification

**Files:**
- Modify: `docs/agent-workflows.md`
- Modify: `docs/usage.md`
- Modify: `src/hieronymus/cli.py`
- Modify: `tests/test_cli_service.py`

**Interfaces:**
- Consumes: Task 2 command contract.
- Produces: documentation that distinguishes workspace-local skills from global `hiero install <agent>` integration.

- [ ] **Step 1: Write a failing documentation test**

```python
def test_agent_workflows_documents_project_local_skills() -> None:
    text = (ROOT / "docs" / "agent-workflows.md").read_text(encoding="utf-8")
    assert "hiero skills install --target agents --target claude" in text
    assert ".agents/skills" in text
    assert "does not register MCP" in text
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_cli_service.py::test_agent_workflows_documents_project_local_skills -v`

Expected: fails because local skill workflow is not documented.

- [ ] **Step 3: Document and update help**

Document install/uninstall, `--dry-run`, unconditional overwrite, safe ownership boundary, and that local skills do not register MCP. Add the exact install example to `hiero help` under Agent and automation.

- [ ] **Step 4: Verify and push**

Run targeted tests: `uv run pytest tests/test_project_skills.py tests/test_cli_project_skills.py tests/test_agent_assets.py tests/test_cli_service.py -v`.

Run repository checks: `uv run pytest`, `uv run ruff check .`, and `uv run ruff format --check .`.

Run final handoff: `git diff --check origin/main...HEAD`, `git status --short`, and `git push origin main`.

Expected: tests, lint, and formatting pass; only feature files are committed and pushed.
