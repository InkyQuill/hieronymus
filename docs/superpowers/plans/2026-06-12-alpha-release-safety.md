# Alpha Release Safety Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent GitHub Actions and release tooling from publishing a `1.x` Hieronymus release while the product is still alpha.

**Architecture:** Keep package metadata and tags SemVer-compatible as `0.x`, while human-facing CLI/TUI text displays an alpha marker. Add an explicit Python release guard, check the computed next semantic-release version before any side-effectful release command, then re-check metadata before publish.

**Tech Stack:** Python 3.12, Click, python-semantic-release, GitHub Actions, pytest, Ruff.

---

## Current Risk

- `.github/workflows/release.yml` does not hardcode `1.0.0`, but it runs `uv run semantic-release version` and then `uv run semantic-release publish` on every `main` push.
- `pyproject.toml` and `src/hieronymus/__init__.py` still contain `1.1.0`, so semantic-release currently continues the premature `1.x` line.
- `tag_format = "v{version}"` is correct because tags must stay SemVer-compatible, but a guard is needed so future release automation cannot publish `v1.0.0` or any other `v1.x` tag before Pavel approves a stable release.

## File Structure

- Modify `pyproject.toml`: set package version to `0.2.0`; keep `tag_format = "v{version}"`; keep stable SemVer tags.
- Modify `src/hieronymus/__init__.py`: set `__version__ = "0.2.0"`.
- Create `src/hieronymus/release_guard.py`: validate project metadata and computed release versions before publish.
- Modify `.github/workflows/release.yml`: run the metadata guard, check the computed next version with `semantic-release version --print`, then run normal release and re-check metadata before publish.
- Modify `src/hieronymus/presentation.py`: add display-version helpers that append `α` for `0.x`.
- Modify `src/hieronymus/admin.py`: send display version in the admin header.
- Modify `src/hieronymus/cli.py`: use display versions in human update output while keeping JSON raw.
- Modify `tests/test_release_workflow.py`: assert release metadata is `0.2.0`, semantic-release is guarded, and workflow order blocks publish.
- Create `tests/test_release_guard.py`: unit-test metadata and computed-version guards.
- Modify `tests/test_cli_service.py`: assert human-facing CLI output includes `α`.
- Modify `tests/test_admin_store.py`: assert admin header includes the alpha display version.

---

### Task 1: Pin Project Metadata Back To `0.2.0`

**Files:**
- Modify: `pyproject.toml`
- Modify: `src/hieronymus/__init__.py`
- Test: `tests/test_release_workflow.py`

- [ ] **Step 1: Write the failing metadata test**

Add this test to `tests/test_release_workflow.py` after `test_pyproject_configures_semantic_release`:

```python
def test_project_metadata_stays_on_alpha_version_line() -> None:
    pyproject_text = (ROOT / "pyproject.toml").read_text()
    pyproject = tomllib.loads(pyproject_text)
    init_text = (ROOT / "src" / "hieronymus" / "__init__.py").read_text()

    assert pyproject["project"]["version"] == "0.2.0"
    assert '__version__ = "0.2.0"' in init_text
    assert not pyproject["project"]["version"].startswith("1.")
    assert '"1.0.0"' not in init_text
    assert '"1.1.0"' not in init_text
```

- [ ] **Step 2: Run the focused test to verify it fails**

Run:

```bash
uv run pytest tests/test_release_workflow.py::test_project_metadata_stays_on_alpha_version_line -q
```

Expected: FAIL because the project metadata still says `1.1.0`.

- [ ] **Step 3: Update package metadata**

Change `pyproject.toml` line 3 to:

```toml
version = "0.2.0"
```

Change `src/hieronymus/__init__.py` to:

```python
"""Hieronymus translation memory."""

__all__ = ["__version__"]

__version__ = "0.2.0"
```

- [ ] **Step 4: Run the focused test to verify it passes**

Run:

```bash
uv run pytest tests/test_release_workflow.py::test_project_metadata_stays_on_alpha_version_line -q
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add pyproject.toml src/hieronymus/__init__.py tests/test_release_workflow.py
git commit -m "chore: reset version metadata to alpha line"
```

---

### Task 2: Add A Release Guard Module

**Files:**
- Create: `src/hieronymus/release_guard.py`
- Create: `tests/test_release_guard.py`

- [ ] **Step 1: Write failing guard tests**

Create `tests/test_release_guard.py`:

```python
from __future__ import annotations

from pathlib import Path

import pytest

from hieronymus.release_guard import ReleaseGuardError, validate_alpha_release_metadata


def write_project(root: Path, *, pyproject_version: str, module_version: str) -> None:
    (root / "src" / "hieronymus").mkdir(parents=True)
    (root / "pyproject.toml").write_text(
        f'[project]\nname = "hieronymus"\nversion = "{pyproject_version}"\n',
        encoding="utf-8",
    )
    (root / "src" / "hieronymus" / "__init__.py").write_text(
        f'"""Hieronymus translation memory."""\n\n__version__ = "{module_version}"\n',
        encoding="utf-8",
    )


def test_validate_alpha_release_metadata_accepts_zero_major_version(tmp_path: Path) -> None:
    write_project(tmp_path, pyproject_version="0.2.0", module_version="0.2.0")

    assert validate_alpha_release_metadata(tmp_path) == "0.2.0"


def test_validate_alpha_release_metadata_rejects_one_major_version(tmp_path: Path) -> None:
    write_project(tmp_path, pyproject_version="1.0.0", module_version="1.0.0")

    with pytest.raises(ReleaseGuardError, match="alpha releases must stay on 0.x"):
        validate_alpha_release_metadata(tmp_path)


def test_validate_alpha_release_metadata_rejects_mismatched_versions(tmp_path: Path) -> None:
    write_project(tmp_path, pyproject_version="0.2.0", module_version="0.3.0")

    with pytest.raises(ReleaseGuardError, match="version mismatch"):
        validate_alpha_release_metadata(tmp_path)
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
uv run pytest tests/test_release_guard.py -q
```

Expected: FAIL because `hieronymus.release_guard` does not exist.

- [ ] **Step 3: Implement the guard**

Create `src/hieronymus/release_guard.py`:

```python
from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

_MODULE_VERSION_RE = re.compile(r'^__version__ = "([^"]+)"$', re.MULTILINE)


class ReleaseGuardError(RuntimeError):
    pass


def _module_version(root: Path) -> str:
    init_path = root / "src" / "hieronymus" / "__init__.py"
    match = _MODULE_VERSION_RE.search(init_path.read_text(encoding="utf-8"))
    if match is None:
        raise ReleaseGuardError("src/hieronymus/__init__.py does not define __version__")
    return match.group(1)


def validate_alpha_release_metadata(root: Path) -> str:
    pyproject = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8"))
    project_version = str(pyproject["project"]["version"])
    module_version = _module_version(root)

    if project_version != module_version:
        raise ReleaseGuardError(
            f"version mismatch: pyproject.toml has {project_version}, "
            f"src/hieronymus/__init__.py has {module_version}"
        )
    if not project_version.startswith("0."):
        raise ReleaseGuardError(
            f"alpha releases must stay on 0.x until a stable release is approved; "
            f"found {project_version}"
        )
    return project_version


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args(argv)

    try:
        version = validate_alpha_release_metadata(args.root)
    except ReleaseGuardError as error:
        print(f"release guard failed: {error}", file=sys.stderr)
        return 1

    print(f"release guard passed: v{version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 4: Run guard tests to verify they pass**

Run:

```bash
uv run pytest tests/test_release_guard.py -q
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add src/hieronymus/release_guard.py tests/test_release_guard.py
git commit -m "test: guard alpha release metadata"
```

---

### Task 3: Guard GitHub Actions Before Publish

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `tests/test_release_workflow.py`
- Modify: `src/hieronymus/release_guard.py`
- Modify: `tests/test_release_guard.py`

- [ ] **Step 1: Write failing workflow tests**

Add release guard tests for computed versions to `tests/test_release_guard.py`:

```python
def test_validate_alpha_version_accepts_version_tag() -> None:
    assert validate_alpha_version("v0.3.0") == "0.3.0"


def test_validate_alpha_version_rejects_one_major_version() -> None:
    with pytest.raises(ReleaseGuardError, match="alpha releases must stay on 0.x"):
        validate_alpha_version("1.0.0")
```

Add this workflow test to `tests/test_release_workflow.py`:

```python
def test_release_workflow_guards_alpha_version_before_publish() -> None:
    lines = _workflow_lines()
    release = _block_after(lines, _find_line(lines, "  release:"))

    guard_command = "      - run: uv run python -m hieronymus.release_guard"
    computed_guard_name = "      - name: Check next release version"
    version_command = "      - run: uv run semantic-release version"
    publish_command = "      - run: uv run semantic-release publish"

    guard_indexes = [index for index, line in enumerate(release) if line == guard_command]
    computed_guard_index = release.index(computed_guard_name)
    version_index = release.index(version_command)
    publish_index = release.index(publish_command)

    assert len(guard_indexes) == 2
    assert guard_indexes[0] < computed_guard_index < version_index
    assert version_index < guard_indexes[1] < publish_index

    computed_guard = _step_block(release, computed_guard_name)
    assert 'NEXT_VERSION="$(uv run semantic-release version --print)"' in computed_guard
    assert 'uv run python -m hieronymus.release_guard --version "$NEXT_VERSION"' in computed_guard
```

- [ ] **Step 2: Run the focused test to verify it fails**

Run:

```bash
uv run pytest tests/test_release_workflow.py::test_release_workflow_guards_alpha_version_before_publish -q
```

Expected: FAIL because the guard cannot validate computed versions and the workflow does not check `semantic-release version --print`.

- [ ] **Step 3: Add computed-version validation**

Extend `src/hieronymus/release_guard.py`:

```python
def validate_alpha_version(version_text: str) -> str:
    normalized = version_text.strip().removeprefix("v")
    if not normalized.startswith("0."):
        raise ReleaseGuardError(
            f"alpha releases must stay on 0.x until a stable release is approved; "
            f"found {normalized}"
        )
    return normalized
```

Add `--version VERSION` to `main()`. When supplied, validate that version instead of project metadata and print `release guard passed: v{normalized_version}`.

- [ ] **Step 4: Add guard steps to the release workflow**

Change the release job in `.github/workflows/release.yml` so the end of the job is:

```yaml
      - run: uv sync --dev

      - run: uv run python -m hieronymus.release_guard

      - name: Check next release version
        run: |
          NEXT_VERSION="$(uv run semantic-release version --print)"
          uv run python -m hieronymus.release_guard --version "$NEXT_VERSION"
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}

      - run: uv run semantic-release version
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}

      - run: uv run python -m hieronymus.release_guard

      - run: uv run semantic-release publish
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

- [ ] **Step 5: Run focused release tests**

Run:

```bash
uv run pytest tests/test_release_guard.py tests/test_release_workflow.py -q
```

Expected: PASS.

- [ ] **Step 6: Commit**

Run:

```bash
git add .github/workflows/release.yml src/hieronymus/release_guard.py tests/test_release_guard.py tests/test_release_workflow.py
git commit -m "ci: guard computed release version before publishing"
```

---

### Task 4: Display Alpha Marker In Human-Facing Version Text

**Files:**
- Modify: `src/hieronymus/presentation.py`
- Modify: `src/hieronymus/admin.py`
- Modify: `src/hieronymus/cli.py`
- Modify: `tests/test_cli_service.py`
- Modify: `tests/test_admin_store.py`

- [ ] **Step 1: Write failing CLI display tests**

Change `test_render_greeting_contains_identity_and_tagline` in `tests/test_cli_service.py` to:

```python
def test_render_greeting_contains_identity_tagline_and_alpha_marker() -> None:
    rendered = render_greeting("0.2.0")

    assert rendered == f"{GREETING_ICON} Hieronymus v0.2.0α\nRemembers things for you."
```

Add direct display-version coverage:

```python
def test_display_version_marks_alpha_versions() -> None:
    assert display_version("0.2.0") == "v0.2.0α"


def test_display_version_leaves_stable_versions_unmarked() -> None:
    assert display_version("1.0.0") == "v1.0.0"
```

Change the human update assertions in `tests/test_cli_service.py`:

```python
assert "Hieronymus is up to date: v0.2.0α" in result.output
assert "No update available: v0.2.0α" in result.output
assert "Update available: v0.1.0α -> v0.2.0α" in result.output
assert "Updated Hieronymus: v0.1.0α -> v0.2.0α" in result.output
```

- [ ] **Step 2: Write failing admin header test**

In `tests/test_admin_store.py`, update the header assertion in `test_status_payload_includes_dashboard_contract` or the nearest existing header payload test to prove the admin header uses the display-version provider without depending on the currently installed package version:

```python
monkeypatch.setattr("hieronymus.admin.package_display_version", lambda: "v0.2.0α")
assert payload["header"]["version"] == "v0.2.0α"
```

- [ ] **Step 3: Run focused tests to verify they fail**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_render_greeting_contains_identity_tagline_and_alpha_marker tests/test_cli_service.py::test_update_human_runs_update_and_prints_up_to_date tests/test_cli_service.py::test_update_check_human_prints_no_update_available tests/test_cli_service.py::test_update_check_human_prints_update_available tests/test_cli_service.py::test_update_human_prints_updated_after_applied_update tests/test_admin_store.py -q
```

Expected: FAIL because human-facing versions still render as raw `0.x` strings.

- [ ] **Step 4: Add display helpers**

Modify `src/hieronymus/presentation.py`:

```python
def display_version(raw_version: str) -> str:
    if raw_version.startswith("0."):
        return f"v{raw_version}α"
    return f"v{raw_version}"


def package_display_version() -> str:
    return display_version(package_version())
```

Change `render_greeting` to:

```python
def render_greeting(app_version: str | None = None) -> str:
    resolved_version = app_version if app_version is not None else package_version()
    return f"{GREETING_ICON} Hieronymus {display_version(resolved_version)}\n{TAGLINE}"
```

- [ ] **Step 5: Use display versions in admin and CLI human output**

Change the import in `src/hieronymus/admin.py`:

```python
from hieronymus.presentation import GREETING_ICON, TAGLINE, package_display_version
```

Change the header payload version:

```python
"version": package_display_version(),
```

Change the import in `src/hieronymus/cli.py`:

```python
from hieronymus.presentation import GUIDE_ICON, display_version, render_greeting, render_json
```

In `update_command`, before printing human output, add:

```python
    current_display = display_version(status.current_version)
```

Change the human update messages to:

```python
        before_display = display_version(before_status.current_version)
        click.echo(f"Updated Hieronymus: {before_display} -> {current_display}")
```

```python
        update_target = status.latest_version or status.latest_tag or "unknown"
        update_display = display_version(update_target) if update_target != "unknown" else update_target
        click.echo(f"Update available: {current_display} -> {update_display}")
```

```python
        click.echo(f"No update available: {current_display}")
```

```python
        click.echo(f"Hieronymus is up to date: {current_display}")
```

- [ ] **Step 6: Run focused tests to verify they pass**

Run:

```bash
uv run pytest tests/test_cli_service.py tests/test_admin_store.py -q
```

Expected: PASS.

- [ ] **Step 7: Commit**

Run:

```bash
git add src/hieronymus/presentation.py src/hieronymus/admin.py src/hieronymus/cli.py tests/test_cli_service.py tests/test_admin_store.py
git commit -m "feat: show alpha marker in human version text"
```

---

### Task 5: Full Verification

**Files:**
- Verify the repository state.

- [ ] **Step 1: Run Python verification**

Run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: all commands pass.

- [ ] **Step 2: Run release guard directly**

Run:

```bash
uv run python -m hieronymus.release_guard
```

Expected:

```text
release guard passed: v0.2.0
```

- [ ] **Step 3: Confirm no tracked `1.0.0` or `1.1.0` release metadata remains**

Run:

```bash
rg -n 'version = "1\\.|__version__ = "1\\.|v1\\.0\\.0|v1\\.1\\.0|1\\.0\\.0|1\\.1\\.0' pyproject.toml src tests .github docs README.md
```

Expected: no matches except historical ADR/roadmap text that explicitly describes the old premature versions. If this command reports active metadata or workflow matches, fix those lines before final commit.

- [ ] **Step 4: Commit verification-only fixes if needed**

If Task 5 Step 3 found active metadata or workflow matches and they were fixed, run:

```bash
git add pyproject.toml src tests .github docs README.md
git commit -m "chore: finish alpha release safety cleanup"
```

If no files changed during Task 5, do not create a commit.

---

## Self-Review

- Spec coverage: This plan covers ADR 0006 release authority, `1.x -> 0.x` remapping, SemVer-compatible package metadata/tags, Greek `α` for human-facing versions, and GitHub Actions guardrails before publish.
- Placeholder scan: The plan contains concrete paths, commands, expected outcomes, and code snippets for each implementation task.
- Type consistency: `display_version`, `package_display_version`, `validate_alpha_release_metadata`, and `ReleaseGuardError` are introduced before use and referenced with consistent names.
