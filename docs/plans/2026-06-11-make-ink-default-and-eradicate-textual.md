# Ink default TUI and Textual eradication Implementation Plan

> **For Antigravity:** REQUIRED SUB-SKILL: Load executing-plans to implement this plan task-by-task.

**Goal:** Eradicate the legacy Python Textual framework/TUI and make the TypeScript React/Ink interface the default and sole TUI for Hieronymus, removing the `HIERONYMUS_TUI` environment variable flags.

**Architecture:** Python CLI will launch the Node.js/Ink frontend directly via subprocess for interactive commands (`config` and `admin`), bypassing any environment checks. Textual dependencies and screen files will be deleted entirely from the repository.

**Tech Stack:** Python 3.12, uv, Node.js >=22, pytest

---

### Task 1: Remove textual dependency

**Files:**
- Modify: `pyproject.toml`
- Test: run `uv sync`

**Step 1: Write the failing test**
Run `uv run pip show textual` (assert it is installed). We will remove it.

**Step 2: Run test to verify it fails**
Run: `uv run pip show textual`
Expected: Output showing textual information (installed).

**Step 3: Write minimal implementation**
Edit `pyproject.toml` to remove `"textual>=0.86.0",` from dependencies.
Run `uv sync` to update `uv.lock`.

**Step 4: Run test to verify it passes**
Run: `uv run pip show textual`
Expected: Error indicating package not found.

**Step 5: Commit**
```bash
git add pyproject.toml uv.lock
git commit -m "chore: remove textual dependency"
```

---

### Task 2: Modify cli.py to launch Ink directly

**Files:**
- Modify: `src/hieronymus/cli.py`
- Test: `tests/test_cli_ink_tui.py`

**Step 1: Write the failing test**
Run pytest. At this point, the imports in `cli.py` would fail if textual package is uninstalled.

**Step 2: Run test to verify it fails**
Run: `uv run pytest tests/test_cli_ink_tui.py`
Expected: ImportError on `HieronymusConfigApp` or similar.

**Step 3: Write minimal implementation**
- Remove legacy TUI imports:
  ```python
  from hieronymus.tui.app import HieronymusAdminApp
  from hieronymus.tui.config_app import HieronymusConfigApp
  ```
- Remove `_tui_mode` helper function.
- Update `config_command` to:
  ```python
  @main.command("config", help="Open the configuration TUI.")
  @click.option("--json", "json_output", is_flag=True)
  @click.pass_context
  def config_command(ctx: click.Context, json_output: bool) -> None:
      config = ctx.obj["config"]
      if not json_output:
          _launch_ink("config", data_root=config.data_root)
          return
      # JSON logic remains...
  ```
- Update `admin` to:
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
      _launch_ink("admin", data_root=config.data_root)
  ```

**Step 4: Run test to verify it passes**
We will adjust the tests in Task 5 to pass. For now, running CLI manually will no longer attempt to load Textual.

**Step 5: Commit**
```bash
git add src/hieronymus/cli.py
git commit -m "refactor(cli): remove textual launch logic and imports"
```

---

### Task 3: Update doctor.py runtime check messages

**Files:**
- Modify: `src/hieronymus/doctor.py`
- Test: `tests/test_doctor.py` (or check check outputs)

**Step 1: Write the failing test**
Run doctor checks and inspect the output warnings mentioning `HIERONYMUS_TUI=ink`.

**Step 2: Run test to verify it fails**
Inspect `doctor.py` code for `HIERONYMUS_TUI=ink`.
Expected: Wording references the environment variable flag.

**Step 3: Write minimal implementation**
Modify warnings in `src/hieronymus/doctor.py` around lines 163, 184, 194 to mention the Hieronymus terminal user interface (TUI) without referencing `HIERONYMUS_TUI`.

**Step 4: Run test to verify it passes**
Run doctor command:
`uv run hiero doctor`
Expected: Output prints correct warning/info logs without environment variables.

**Step 5: Commit**
```bash
git add src/hieronymus/doctor.py
git commit -m "refactor(doctor): update node runtime warnings to refer to default TUI"
```

---

### Task 4: Delete legacy Textual TUI code

**Files:**
- Delete: `src/hieronymus/tui/` directory

**Step 1: Write the failing test**
None needed (deletion task).

**Step 2: Run test to verify it fails**
N/A

**Step 3: Write minimal implementation**
Delete the directory `src/hieronymus/tui/`.

**Step 4: Run test to verify it passes**
Confirm directory does not exist: `ls src/hieronymus/tui/`
Expected: No such file or directory.

**Step 5: Commit**
```bash
git rm -r src/hieronymus/tui/
git commit -m "refactor(tui): delete legacy Textual UI package"
```

---

### Task 5: Adapt CLI Ink TUI tests

**Files:**
- Modify: `tests/test_cli_ink_tui.py`

**Step 1: Write the failing test**
Run the existing CLI Ink tests:
`uv run pytest tests/test_cli_ink_tui.py`

**Step 2: Run test to verify it fails**
Expected: Tests fail because they assert Textual defaults, assert HIERONYMUS_TUI environment variables, and try to mock deleted App objects.

**Step 3: Write minimal implementation**
- Remove/update:
  - `test_cli_config_defaults_to_textual_when_tui_env_unset` (update to assert it launches Ink)
  - `test_cli_admin_defaults_to_textual_when_tui_env_unset` (update to assert it launches Ink)
  - `test_cli_textual_env_forces_textual_for_config_and_admin` (delete)
- Remove `monkeypatch.setenv("HIERONYMUS_TUI", "ink")` lines from remaining tests as they are now redundant.

**Step 4: Run test to verify it passes**
Run: `uv run pytest tests/test_cli_ink_tui.py`
Expected: All tests pass.

**Step 5: Commit**
```bash
git add tests/test_cli_ink_tui.py
git commit -m "test: update cli ink tui tests to verify default launcher behavior"
```

---

### Task 6: Delete legacy Textual test modules

**Files:**
- Delete: `tests/test_config_tui.py`, `tests/test_admin_tui.py`

**Step 1: Write the failing test**
None.

**Step 2: Run test to verify it fails**
N/A

**Step 3: Write minimal implementation**
Remove the test files:
- `tests/test_config_tui.py`
- `tests/test_admin_tui.py`

**Step 4: Run test to verify it passes**
Verify files are gone: `ls tests/test_*_tui.py`
Expected: Clean error or only `test_cli_ink_tui.py` remaining.

**Step 5: Commit**
```bash
git rm tests/test_config_tui.py tests/test_admin_tui.py
git commit -m "test: delete legacy Textual UI test modules"
```

---

### Task 7: Update other service/admin CLI tests

**Files:**
- Modify: `tests/test_admin_cli.py`, `tests/test_cli_service.py`

**Step 1: Write the failing test**
Run tests:
`uv run pytest tests/test_admin_cli.py tests/test_cli_service.py`

**Step 2: Run test to verify it fails**
Expected: Failure on `test_admin_launch_invokes_textual_app` and `test_config_launch_invokes_textual_app` due to missing `HieronymusAdminApp` and `HieronymusConfigApp` classes/imports.

**Step 3: Write minimal implementation**
- In `tests/test_admin_cli.py`, modify `test_admin_launch_invokes_textual_app` to check that `_launch_ink` is called instead of `HieronymusAdminApp.run`.
- In `tests/test_cli_service.py`, modify `test_config_launch_invokes_textual_app` to check that `_launch_ink` is called instead of `HieronymusConfigApp.run`.

**Step 4: Run test to verify it passes**
Run: `uv run pytest tests/test_admin_cli.py tests/test_cli_service.py`
Expected: PASS.

**Step 5: Commit**
```bash
git add tests/test_admin_cli.py tests/test_cli_service.py
git commit -m "test: update admin and config service CLI tests to assert Ink launch"
```

---

### Task 8: Update documentation and ADRs

**Files:**
- Modify: `README.md`, `docs/usage.md`, `docs/adr/0002-ink-react-tui-migration.md`

**Step 1: Write the failing test**
None.

**Step 2: Run test to verify it fails**
N/A

**Step 3: Write minimal implementation**
- Remove references to `HIERONYMUS_TUI=ink` or `HIERONYMUS_TUI=textual` from `README.md` and `docs/usage.md`.
- Update ADR `docs/adr/0002-ink-react-tui-migration.md` to note that the feature flag has been removed and Textual has been fully eradicated in the final migration stage.

**Step 4: Run test to verify it passes**
Verify that search for `HIERONYMUS_TUI` returns no references in docs.

**Step 5: Commit**
```bash
git add README.md docs/
git commit -m "docs: remove HIERONYMUS_TUI environment variable and update ADR"
```
