# Hieronymus Production Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add GitHub-backed production install, uninstall, update-check, in-place update, and semantic-release scaffolding for Hieronymus.

**Architecture:** Keep release behavior in a focused `hieronymus.release` module and expose it through `hiero update`. Shell scripts handle first install/removal, while the Python module handles version/tag selection, managed checkout detection, and update execution. GitHub Actions owns semantic releases from `main`.

**Tech Stack:** Python 3.12, Click, uv, shell scripts, git, GitHub Actions, python-semantic-release, pytest, ruff.

---

## File Structure

- Create `src/hieronymus/release.py`: semantic tag parsing, latest-tag discovery, managed checkout detection, update check, and update execution.
- Modify `src/hieronymus/cli.py`: add `hiero update`, help text, JSON output.
- Create `install.sh`: idempotent managed checkout installer.
- Create `uninstall.sh`: removal script with `--keep-data` and `--purge-data`.
- Create `.github/workflows/release.yml`: test/lint/format and semantic-release on `main`.
- Modify `pyproject.toml`: semantic-release configuration and dev dependency.
- Modify `README.md` and `docs/usage.md`: install/update/uninstall docs.
- Create `tests/test_release.py`: pure release module tests.
- Modify `tests/test_cli_service.py`: update command and help tests.
- Create `tests/test_release_scripts.py`: installer/uninstaller script content and syntax tests.

## Task 1: GitHub Remote Setup

**Files:**
- Modify: local git config only

- [ ] **Step 1: Verify no origin remote exists**

Run:

```bash
git remote -v
```

Expected: no `origin` remote is printed.

- [ ] **Step 2: Add GitHub origin**

Run:

```bash
git remote add origin https://github.com/InkyQuill/hieronymus.git
```

Expected: command exits zero.

- [ ] **Step 3: Verify origin**

Run:

```bash
git remote -v
```

Expected output includes:

```text
origin  https://github.com/InkyQuill/hieronymus.git (fetch)
origin  https://github.com/InkyQuill/hieronymus.git (push)
```

- [ ] **Step 4: Commit**

No commit is needed because this only changes local git config.

## Task 2: Release Version and Tag Logic

**Files:**
- Create: `src/hieronymus/release.py`
- Create: `tests/test_release.py`

- [ ] **Step 1: Write failing tests for tag parsing and latest selection**

Create `tests/test_release.py`:

```python
from __future__ import annotations

from pathlib import Path

import pytest

from hieronymus.release import (
    MANAGED_APP_PATH,
    ReleaseTag,
    latest_stable_tag,
    managed_app_path,
    parse_release_tag,
)


def test_parse_release_tag_accepts_stable_semver() -> None:
    assert parse_release_tag("v1.2.3") == ReleaseTag("v1.2.3", (1, 2, 3))
    assert parse_release_tag("refs/tags/v10.20.30") == ReleaseTag("v10.20.30", (10, 20, 30))


@pytest.mark.parametrize("tag", ["1.2.3", "v1.2", "v1.2.3-rc.1", "not-a-tag"])
def test_parse_release_tag_rejects_non_stable_tags(tag: str) -> None:
    assert parse_release_tag(tag) is None


def test_latest_stable_tag_selects_highest_version() -> None:
    assert latest_stable_tag(["refs/tags/v0.2.0", "refs/tags/v0.10.0", "refs/tags/v0.9.9"]) == (
        "v0.10.0"
    )


def test_latest_stable_tag_returns_none_without_stable_tags() -> None:
    assert latest_stable_tag(["refs/tags/v0.2.0-rc.1", "refs/heads/main"]) is None


def test_managed_app_path_uses_home_by_default(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    assert managed_app_path() == tmp_path / ".local" / "share" / "hieronymus" / "app"
    assert MANAGED_APP_PATH == Path("~/.local/share/hieronymus/app")
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
uv run pytest tests/test_release.py -v
```

Expected: FAIL because `hieronymus.release` does not exist.

- [ ] **Step 3: Implement release tag helpers**

Create `src/hieronymus/release.py`:

```python
from __future__ import annotations

import re
import subprocess
import sys
from dataclasses import dataclass
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path

GITHUB_REPO_URL = "https://github.com/InkyQuill/hieronymus.git"
MANAGED_APP_PATH = Path("~/.local/share/hieronymus/app")
_TAG_RE = re.compile(r"(?:refs/tags/)?(v(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*))$")


@dataclass(frozen=True, order=True)
class ReleaseTag:
    name: str
    version: tuple[int, int, int]


@dataclass(frozen=True)
class UpdateStatus:
    current_version: str
    latest_version: str | None
    latest_tag: str | None
    update_available: bool
    managed_checkout: Path
    managed_install: bool
    target: str

    def as_dict(self) -> dict[str, object]:
        return {
            "current_version": self.current_version,
            "latest_version": self.latest_version,
            "latest_tag": self.latest_tag,
            "update_available": self.update_available,
            "managed_checkout": str(self.managed_checkout),
            "managed_install": self.managed_install,
            "target": self.target,
        }


def managed_app_path() -> Path:
    return MANAGED_APP_PATH.expanduser()


def package_version() -> str:
    try:
        return version("hieronymus")
    except PackageNotFoundError:
        from hieronymus import __version__

        return __version__


def parse_release_tag(raw_tag: str) -> ReleaseTag | None:
    tag = raw_tag.strip().removesuffix("^{}")
    match = _TAG_RE.fullmatch(tag)
    if match is None:
        return None
    version_tuple = (
        int(match.group("major")),
        int(match.group("minor")),
        int(match.group("patch")),
    )
    return ReleaseTag(match.group(1), version_tuple)


def latest_stable_tag(raw_tags: list[str]) -> str | None:
    parsed = [tag for raw_tag in raw_tags if (tag := parse_release_tag(raw_tag)) is not None]
    if not parsed:
        return None
    return max(parsed).name
```

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
uv run pytest tests/test_release.py -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/release.py tests/test_release.py
git commit -m "feat: add release version helpers"
```

## Task 3: Update Status and In-Place Update Engine

**Files:**
- Modify: `src/hieronymus/release.py`
- Modify: `tests/test_release.py`

- [ ] **Step 1: Add failing tests for update status and update execution**

Append to `tests/test_release.py`:

```python
from hieronymus.release import check_update, run_update


def test_check_update_reports_newer_tag(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("hieronymus.release.package_version", lambda: "0.1.0")
    monkeypatch.setattr("hieronymus.release.fetch_remote_tags", lambda: ["refs/tags/v0.2.0"])
    monkeypatch.setattr("hieronymus.release.is_managed_install", lambda _: True)

    status = check_update()

    assert status.current_version == "0.1.0"
    assert status.latest_version == "0.2.0"
    assert status.latest_tag == "v0.2.0"
    assert status.update_available is True
    assert status.managed_install is True


def test_check_update_handles_missing_tags(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("hieronymus.release.package_version", lambda: "0.1.0")
    monkeypatch.setattr("hieronymus.release.fetch_remote_tags", lambda: [])
    monkeypatch.setattr("hieronymus.release.is_managed_install", lambda _: False)

    status = check_update()

    assert status.latest_version is None
    assert status.latest_tag is None
    assert status.update_available is False
    assert status.managed_install is False


def test_run_update_rejects_unmanaged_install(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("hieronymus.release.is_managed_install", lambda _: False)

    with pytest.raises(RuntimeError, match="managed installer"):
        run_update()


def test_run_update_fetches_checkout_and_reinstalls(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    checkout = tmp_path / "app"
    checkout.mkdir()
    calls: list[list[str]] = []

    monkeypatch.setattr("hieronymus.release.managed_app_path", lambda: checkout)
    monkeypatch.setattr("hieronymus.release.is_managed_install", lambda _: True)
    monkeypatch.setattr("hieronymus.release.package_version", lambda: "0.1.0")
    monkeypatch.setattr("hieronymus.release.fetch_remote_tags", lambda: ["refs/tags/v0.2.0"])

    def fake_run(command: list[str], *, cwd: Path | None = None) -> None:
        calls.append(command if cwd is None else command + [f"cwd={cwd}"])

    monkeypatch.setattr("hieronymus.release._run", fake_run)

    status = run_update()

    assert status.latest_tag == "v0.2.0"
    assert ["git", "fetch", "--tags", "origin", "cwd=" + str(checkout)] in calls
    assert ["git", "checkout", "--force", "v0.2.0", "cwd=" + str(checkout)] in calls
    assert ["uv", "tool", "install", "--force", str(checkout)] in calls
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
uv run pytest tests/test_release.py -v
```

Expected: FAIL because update functions are not implemented.

- [ ] **Step 3: Implement update functions**

Append to `src/hieronymus/release.py`:

```python
def _version_tuple(version_text: str) -> tuple[int, int, int]:
    parsed = parse_release_tag(f"v{version_text}")
    return parsed.version if parsed is not None else (0, 0, 0)


def _tag_version(tag: str | None) -> str | None:
    parsed = parse_release_tag(tag or "")
    if parsed is None:
        return None
    return ".".join(str(part) for part in parsed.version)


def _run(command: list[str], *, cwd: Path | None = None) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def fetch_remote_tags(repo_url: str = GITHUB_REPO_URL) -> list[str]:
    result = subprocess.run(
        ["git", "ls-remote", "--tags", repo_url],
        check=True,
        text=True,
        capture_output=True,
    )
    tags = []
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) == 2:
            tags.append(parts[1])
    return tags


def is_managed_install(checkout: Path | None = None) -> bool:
    root = (checkout or managed_app_path()).resolve()
    try:
        executable = Path(sys.argv[0]).resolve()
    except OSError:
        return root.exists()
    return root.exists() and (root in executable.parents or root == Path.cwd().resolve())


def check_update(*, target: str = "latest") -> UpdateStatus:
    current = package_version()
    checkout = managed_app_path()
    if target == "main":
        latest_tag = "main"
        latest_version = None
        update_available = True
    else:
        latest_tag = latest_stable_tag(fetch_remote_tags())
        latest_version = _tag_version(latest_tag)
        update_available = (
            latest_version is not None and _version_tuple(latest_version) > _version_tuple(current)
        )
    return UpdateStatus(
        current_version=current,
        latest_version=latest_version,
        latest_tag=latest_tag,
        update_available=update_available,
        managed_checkout=checkout,
        managed_install=is_managed_install(checkout),
        target=target,
    )


def run_update(*, target: str = "latest") -> UpdateStatus:
    checkout = managed_app_path()
    if not is_managed_install(checkout):
        raise RuntimeError("Hieronymus was not installed through the managed installer")
    status = check_update(target=target)
    if not status.update_available and target != "main":
        return status
    checkout_target = "main" if target == "main" else status.latest_tag
    if checkout_target is None:
        return status
    _run(["git", "fetch", "--tags", "origin"], cwd=checkout)
    _run(["git", "checkout", "--force", checkout_target], cwd=checkout)
    _run(["uv", "tool", "install", "--force", str(checkout)])
    return check_update(target=target)
```

- [ ] **Step 4: Run release tests**

Run:

```bash
uv run pytest tests/test_release.py -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/release.py tests/test_release.py
git commit -m "feat: check and apply github updates"
```

## Task 4: CLI `hiero update`

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `tests/test_cli_service.py`

- [ ] **Step 1: Add failing CLI tests**

Append to `tests/test_cli_service.py`:

```python
def test_update_check_json_reports_status(tmp_path: Path) -> None:
    with patch("hieronymus.cli.check_update") as check_update:
        check_update.return_value.as_dict.return_value = {
            "current_version": "0.1.0",
            "latest_version": "0.2.0",
            "latest_tag": "v0.2.0",
            "update_available": True,
            "managed_checkout": str(tmp_path / "app"),
            "managed_install": True,
            "target": "latest",
        }
        result = CliRunner().invoke(main, ["update", "--check", "--json"])

    assert result.exit_code == 0
    assert json.loads(result.output)["latest_tag"] == "v0.2.0"


def test_update_runs_in_place_update() -> None:
    with patch("hieronymus.cli.run_update") as run_update:
        run_update.return_value.as_dict.return_value = {
            "current_version": "0.2.0",
            "latest_version": "0.2.0",
            "latest_tag": "v0.2.0",
            "update_available": False,
            "managed_checkout": "/home/user/.local/share/hieronymus/app",
            "managed_install": True,
            "target": "latest",
        }
        result = CliRunner().invoke(main, ["update"])

    assert result.exit_code == 0
    assert "Hieronymus is up to date" in result.output


def test_update_unmanaged_install_returns_clean_error() -> None:
    with patch("hieronymus.cli.run_update", side_effect=RuntimeError("managed installer")):
        result = CliRunner().invoke(main, ["update"])

    assert result.exit_code == 1
    assert "managed installer" in result.output
    assert "Traceback" not in result.output
```

Also update `test_cli_help_mentions_service_commands` to assert:

```python
assert "hiero update" in result.output
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_update_check_json_reports_status tests/test_cli_service.py::test_update_runs_in_place_update tests/test_cli_service.py::test_update_unmanaged_install_returns_clean_error -v
```

Expected: FAIL because `update` does not exist.

- [ ] **Step 3: Implement CLI command**

Modify imports in `src/hieronymus/cli.py`:

```python
from hieronymus.release import check_update, run_update
```

Add before `help_command`:

```python
@main.command("update")
@click.option("--check", "check_only", is_flag=True)
@click.option("--json", "as_json", is_flag=True)
@click.option("--target", type=click.Choice(["latest", "main"]), default="latest")
def update_command(check_only: bool, as_json: bool, target: str) -> None:
    try:
        status = check_update(target=target) if check_only else run_update(target=target)
    except RuntimeError as error:
        raise click.ClickException(str(error)) from error
    payload = status.as_dict()
    if as_json:
        click.echo(render_json(payload))
        return
    click.echo(render_greeting())
    click.echo()
    if payload["update_available"]:
        next_version = payload["latest_version"] or payload["latest_tag"]
        click.echo(f"Update available: {payload['current_version']} -> {next_version}")
    else:
        click.echo(f"Hieronymus is up to date: {payload['current_version']}")
    click.echo(f"managed checkout: {payload['managed_checkout']}")
```

Add to `help_command`:

```python
click.echo("  hiero update           Update managed installs in place")
```

- [ ] **Step 4: Run CLI tests**

Run:

```bash
uv run pytest tests/test_cli_service.py -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/cli.py tests/test_cli_service.py
git commit -m "feat: add hiero update command"
```

## Task 5: Install and Uninstall Scripts

**Files:**
- Create: `install.sh`
- Create: `uninstall.sh`
- Create: `tests/test_release_scripts.py`

- [ ] **Step 1: Write failing script tests**

Create `tests/test_release_scripts.py`:

```python
from __future__ import annotations

import subprocess
from pathlib import Path


def test_install_script_documents_managed_checkout_and_uv_install() -> None:
    script = Path("install.sh").read_text()

    assert "https://github.com/InkyQuill/hieronymus.git" in script
    assert "${HOME}/.local/share/hieronymus/app" in script
    assert "uv tool install --force" in script
    assert "git ls-remote --tags" in script


def test_uninstall_script_prompts_before_data_removal() -> None:
    script = Path("uninstall.sh").read_text()

    assert "uv tool uninstall hieronymus" in script
    assert "--keep-data" in script
    assert "--purge-data" in script
    assert "Remove Hieronymus settings and data" in script


def test_release_scripts_pass_shell_syntax_check() -> None:
    for script in ("install.sh", "uninstall.sh"):
        result = subprocess.run(["sh", "-n", script], check=False, text=True, capture_output=True)
        assert result.returncode == 0, result.stderr
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
uv run pytest tests/test_release_scripts.py -v
```

Expected: FAIL because scripts do not exist.

- [ ] **Step 3: Create `install.sh`**

Create `install.sh`:

```sh
#!/bin/sh
set -eu

REPO_URL="${HIERONYMUS_REPO_URL:-https://github.com/InkyQuill/hieronymus.git}"
APP_DIR="${HIERONYMUS_APP_DIR:-${HOME}/.local/share/hieronymus/app}"

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

latest_tag() {
  git ls-remote --tags "$REPO_URL" \
    | awk '{print $2}' \
    | sed 's#refs/tags/##; s#\\^{}##' \
    | grep -E '^v[0-9]+\\.[0-9]+\\.[0-9]+$' \
    | sort -V \
    | tail -n 1
}

need git

if ! command -v uv >/dev/null 2>&1; then
  echo "Installing uv..."
  curl -LsSf https://astral.sh/uv/install.sh | sh
  PATH="${HOME}/.local/bin:${PATH}"
fi

if [ -d "$APP_DIR/.git" ]; then
  git -C "$APP_DIR" fetch --tags origin
else
  mkdir -p "$(dirname "$APP_DIR")"
  git clone "$REPO_URL" "$APP_DIR"
fi

TAG="$(latest_tag || true)"
if [ -n "$TAG" ]; then
  git -C "$APP_DIR" checkout --force "$TAG"
else
  git -C "$APP_DIR" checkout --force main
fi

uv tool install --force "$APP_DIR"

echo "Hieronymus installed."
echo "Managed checkout: $APP_DIR"
if ! command -v hiero >/dev/null 2>&1; then
  echo "If 'hiero' is not found, add ${HOME}/.local/bin to PATH."
fi
```

- [ ] **Step 4: Create `uninstall.sh`**

Create `uninstall.sh`:

```sh
#!/bin/sh
set -eu

APP_DIR="${HIERONYMUS_APP_DIR:-${HOME}/.local/share/hieronymus/app}"
DATA_DIR="${HIERONYMUS_DATA_ROOT:-${HOME}/.config/hieronymus}"
MODE="prompt"

for arg in "$@"; do
  case "$arg" in
    --keep-data) MODE="keep" ;;
    --purge-data) MODE="purge" ;;
    *) echo "unknown option: $arg" >&2; exit 1 ;;
  esac
done

if command -v uv >/dev/null 2>&1; then
  uv tool uninstall hieronymus >/dev/null 2>&1 || true
fi

rm -rf "$APP_DIR"

if [ "$MODE" = "prompt" ]; then
  printf "Remove Hieronymus settings and data at %s? [y/N] " "$DATA_DIR"
  read -r answer
  case "$answer" in
    y|Y|yes|YES) MODE="purge" ;;
    *) MODE="keep" ;;
  esac
fi

if [ "$MODE" = "purge" ]; then
  rm -rf "$DATA_DIR"
  echo "Removed Hieronymus app, settings, and data."
else
  echo "Removed Hieronymus app. Kept settings and data at $DATA_DIR."
fi
```

- [ ] **Step 5: Mark scripts executable**

Run:

```bash
chmod +x install.sh uninstall.sh
```

- [ ] **Step 6: Run script tests**

Run:

```bash
uv run pytest tests/test_release_scripts.py -v
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add install.sh uninstall.sh tests/test_release_scripts.py
git commit -m "feat: add install and uninstall scripts"
```

## Task 6: Semantic Release Workflow

**Files:**
- Create: `.github/workflows/release.yml`
- Modify: `pyproject.toml`
- Modify: `uv.lock`
- Create: `tests/test_release_workflow.py`

- [ ] **Step 1: Add semantic-release dependency**

Run:

```bash
uv add --dev python-semantic-release
```

Expected: `pyproject.toml` and `uv.lock` update.

- [ ] **Step 2: Write failing workflow/config tests**

Create `tests/test_release_workflow.py`:

```python
from __future__ import annotations

from pathlib import Path


def test_release_workflow_runs_checks_and_semantic_release() -> None:
    workflow = Path(".github/workflows/release.yml").read_text()

    assert "on:" in workflow
    assert "branches: [main]" in workflow
    assert "uv run pytest" in workflow
    assert "uv run ruff check ." in workflow
    assert "uv run ruff format --check ." in workflow
    assert "semantic-release version" in workflow
    assert "semantic-release publish" in workflow


def test_pyproject_configures_semantic_release() -> None:
    pyproject = Path("pyproject.toml").read_text()

    assert "[tool.semantic_release]" in pyproject
    assert 'version_toml = ["pyproject.toml:project.version"]' in pyproject
    assert 'version_variables = ["src/hieronymus/__init__.py:__version__"]' in pyproject
    assert "python-semantic-release" in pyproject
```

- [ ] **Step 3: Run tests to verify they fail**

Run:

```bash
uv run pytest tests/test_release_workflow.py -v
```

Expected: FAIL because workflow and config do not exist.

- [ ] **Step 4: Add semantic-release config**

Append to `pyproject.toml`:

```toml
[tool.semantic_release]
version_toml = ["pyproject.toml:project.version"]
version_variables = ["src/hieronymus/__init__.py:__version__"]
branch = "main"
tag_format = "v{version}"
build_command = "uv build"
changelog_file = "CHANGELOG.md"
commit_message = "chore(release): v{version}"
```

- [ ] **Step 5: Add release workflow**

Create `.github/workflows/release.yml`:

```yaml
name: release

on:
  push:
    branches: [main]

permissions:
  contents: write

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
          token: ${{ secrets.GITHUB_TOKEN }}
      - uses: astral-sh/setup-uv@v5
      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"
      - run: uv sync --dev
      - run: uv run pytest
      - run: uv run ruff check .
      - run: uv run ruff format --check .
      - run: uv run semantic-release version
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      - run: uv run semantic-release publish
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

- [ ] **Step 6: Run workflow tests**

Run:

```bash
uv run pytest tests/test_release_workflow.py -v
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add pyproject.toml uv.lock .github/workflows/release.yml tests/test_release_workflow.py
git commit -m "ci: add semantic release workflow"
```

## Task 7: Documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/usage.md`
- Modify: `tests/test_cli_service.py`

- [ ] **Step 1: Add docs assertions**

Append to `tests/test_cli_service.py`:

```python
def test_readme_documents_production_install_update_and_uninstall() -> None:
    readme = Path("README.md").read_text()

    assert "https://github.com/InkyQuill/hieronymus" in readme
    assert (
        "curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/install.sh | sh"
        in readme
    )
    assert "hiero update" in readme
    assert "uninstall.sh" in readme
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_readme_documents_production_install_update_and_uninstall -v
```

Expected: FAIL because README does not include release docs.

- [ ] **Step 3: Update README**

Add after the opening description in `README.md`:

````markdown
## Install

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/install.sh | sh
```

The installer keeps a managed checkout at `~/.local/share/hieronymus/app` and installs the
`hiero` and `hieronymus` console commands through `uv tool install`.

Update in place:

```bash
hiero update
```

Remove the app:

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/uninstall.sh | sh
```

The uninstaller asks before removing settings and data under `~/.config/hieronymus`.

Repository: <https://github.com/InkyQuill/hieronymus>
````

- [ ] **Step 4: Update usage docs**

Add an `Installation and Updates` section to `docs/usage.md` with the same install, update,
`hiero update --check`, and uninstall commands. Include this warning:

```markdown
The uninstall script only removes Hieronymus-owned install and config/data paths. It does not remove
translation workspace directories.
```

- [ ] **Step 5: Run docs test**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_readme_documents_production_install_update_and_uninstall -v
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add README.md docs/usage.md tests/test_cli_service.py
git commit -m "docs: document install update and uninstall"
```

## Task 8: Final Verification and GitHub Push

**Files:**
- Modify: remote repository state

- [ ] **Step 1: Run full verification**

Run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected:

- all tests pass;
- Ruff reports no lint failures;
- format check reports all files formatted.

- [ ] **Step 2: Check final status**

Run:

```bash
git status --short
```

Expected: only unrelated pre-existing untracked files are present, or no output.

- [ ] **Step 3: Push `main` to GitHub**

Run:

```bash
git push -u origin main
```

Expected: the empty GitHub repository receives the `main` branch. If GitHub rejects the push because
the remote has new files, fetch and inspect before retrying.

- [ ] **Step 4: Verify remote branch**

Run:

```bash
git ls-remote --heads origin main
```

Expected: output includes a commit hash and `refs/heads/main`.

- [ ] **Step 5: Commit**

No commit is needed because pushing changes remote state only.

## Plan Self-Review

- Spec coverage: GitHub remote, install script, uninstall script, `hiero update`, tag-based update
  checks, semantic-release tagging, docs, and final verification are covered.
- Placeholder scan: no incomplete-work markers or deferred implementation language is intentionally
  present.
- Type consistency: `ReleaseTag`, `UpdateStatus`, `check_update`, and `run_update` are introduced
  before CLI use.
