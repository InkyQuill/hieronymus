# Release Frontend Bundle Build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make release and managed update paths consistently install frontend dependencies and rebuild `frontend/dist/main.js` before tests, packaging, publishing, or reinstalling.

**Architecture:** Treat the OpenTUI bundle as a required build artifact for release-quality validation, not a best-effort local convenience. Keep the shell installer and Python managed updater behavior, but normalize Bun command order to the project-preferred `bun install --cwd frontend ...` and `bun run --cwd frontend build` forms. Update workflow tests and release/update tests so CI catches missing bundle builds, mutable release workflow actions, or command-order regressions.

**Tech Stack:** Python 3.12, pytest, GitHub Actions YAML, Bun 1.3.14, semantic-release, hatchling/uv build.

---

## Current Code Map

- `.github/workflows/pr.yml`: already installs Bun and builds `frontend/dist/main.js` before backend pytest.
- `.github/workflows/release.yml`: release workflow still uses mutable `actions/checkout@v4`, `astral-sh/setup-uv@v6`, and `actions/setup-python@v6` references. Its verify job runs `uv run pytest` before any frontend install/build, so packaged OpenTUI smoke tests can skip in release verification.
- `tests/test_release_workflow.py`: checks release workflow shape, Bun version, alpha guard order, and `pyproject.toml` semantic-release config. It currently accepts mutable release workflow action refs and the old semantic-release build command.
- `pyproject.toml`: semantic-release `build_command` is `bun --cwd frontend install --frozen-lockfile && bun --cwd frontend run build && uv build`, which is the unsupported flag order the roadmap says to avoid.
- `src/hieronymus/release.py`: `_build_frontend(checkout)` already installs dependencies and builds before `uv tool install --force`, but it does so by changing `cwd` to `checkout / "frontend"` and running `bun install --frozen-lockfile` plus `bun run build`. That works but does not lock in the preferred command shape.
- `tests/test_release.py`: verifies managed update fetch/checkout/build/reinstall command order.
- `install.sh`: managed installer already checks Bun, builds the frontend, then runs `uv tool install --force "$APP_DIR"`.
- `tests/test_release_scripts.py`: verifies `install.sh` builds before tool install.
- `docs/roadmap.md`: remaining release-quality work includes keeping managed install/update/release builds rebuilding the frontend bundle and fixing unsupported Bun flag-order examples.

---

### Task 1: Pin Release Workflow Actions And Build Frontend Before Verify Tests

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `tests/test_release_workflow.py`

- [ ] **Step 1: Write failing workflow tests**

In `tests/test_release_workflow.py`, add these constants below `EXPECTED_RELEASE_BUN_VERSION`:

```python
CHECKOUT_SHA = "34e114876b0b11c390a56381ad16ebd13914f8d5"
SETUP_UV_SHA = "d0d8abe699bfb85fec6de9f7adb5ae17292296ff"
SETUP_PYTHON_SHA = "a309ff8b426b58ec0e2a45f0f869d46889d02405"
SETUP_BUN_SHA = "0c5077e51419868618aeaa5fe8019c62421857d6"
```

Add this helper below `_step_value()`:

```python
def _uses_lines(lines: list[str]) -> list[str]:
    uses: list[str] = []
    for step in _step_blocks(lines):
        if any(line.strip().startswith("- uses:") for line in step):
            value = _step_value(step, "uses")
            if value is not None:
                uses.append(value)
    return uses
```

In `test_release_workflow_verify_job_uses_read_only_credentials()`, replace the checkout lookup and command assertions with:

```python
    assert _uses_lines(verify) == [
        f"actions/checkout@{CHECKOUT_SHA}",
        f"astral-sh/setup-uv@{SETUP_UV_SHA}",
        f"actions/setup-python@{SETUP_PYTHON_SHA}",
        f"oven-sh/setup-bun@{SETUP_BUN_SHA}",
    ]

    checkout = next(
        step
        for step in _step_blocks(verify)
        if _step_value(step, "uses") == f"actions/checkout@{CHECKOUT_SHA}"
    )
    assert "        with:" in checkout
    assert "          persist-credentials: false" in checkout
    assert not any(line.strip().startswith("token:") for line in checkout)

    bun_step = next(
        step
        for step in _step_blocks(verify)
        if _step_value(step, "uses") == f"oven-sh/setup-bun@{SETUP_BUN_SHA}"
    )
    assert _step_value(bun_step, "bun-version") == f'"{EXPECTED_RELEASE_BUN_VERSION}"'

    verify_runs = [line for line in verify if line.startswith("      - run: ")]
    assert verify_runs == [
        "      - run: uv sync --dev",
        "      - run: bun install --cwd frontend --frozen-lockfile",
        "      - run: bun run --cwd frontend build",
        "      - run: uv run pytest",
        "      - run: uv run ruff check .",
        "      - run: uv run ruff format --check .",
    ]
```

In `test_release_workflow_release_job_publishes_after_verification()`, replace the checkout lookup and Bun step search with exact action assertions:

```python
    assert _uses_lines(release) == [
        f"actions/checkout@{CHECKOUT_SHA}",
        f"astral-sh/setup-uv@{SETUP_UV_SHA}",
        f"actions/setup-python@{SETUP_PYTHON_SHA}",
        f"oven-sh/setup-bun@{SETUP_BUN_SHA}",
    ]

    checkout = next(
        step
        for step in _step_blocks(release)
        if _step_value(step, "uses") == f"actions/checkout@{CHECKOUT_SHA}"
    )
    assert "        with:" in checkout
    assert "          fetch-depth: 0" in checkout
    assert "          token: ${{ secrets.GITHUB_TOKEN }}" in checkout

    bun_step = next(
        step
        for step in _step_blocks(release)
        if _step_value(step, "uses") == f"oven-sh/setup-bun@{SETUP_BUN_SHA}"
    )
    assert _step_value(bun_step, "bun-version") == f'"{EXPECTED_RELEASE_BUN_VERSION}"'
```

Keep the existing semantic-release version/publish and alpha guard assertions.

Also add this run-order assertion near the existing `expected_runs` membership checks in
`test_release_workflow_release_job_publishes_after_verification()`:

```python
    release_runs = [line for line in release if line.startswith("      - run: ")]
    assert release_runs == [
        "      - run: uv sync --dev",
        "      - run: uv run python -m hieronymus.release_guard",
        "      - run: uv run semantic-release version",
        "      - run: uv run python -m hieronymus.release_guard",
        "      - run: uv run semantic-release publish",
    ]
```

This keeps the release job ordered around alpha guards while still allowing the multiline
`Check next release version` step to be checked by the existing dedicated test.

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
uv run pytest tests/test_release_workflow.py -q
```

Expected: FAIL because `.github/workflows/release.yml` still uses mutable action refs and verify does not install/build the frontend bundle before pytest.

- [ ] **Step 3: Update release workflow**

In `.github/workflows/release.yml`, replace verify job actions with pinned refs and add Bun setup plus frontend install/build before pytest:

```yaml
      - uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5
        with:
          persist-credentials: false

      - uses: astral-sh/setup-uv@d0d8abe699bfb85fec6de9f7adb5ae17292296ff

      - uses: actions/setup-python@a309ff8b426b58ec0e2a45f0f869d46889d02405
        with:
          python-version: "3.12"

      # oven-sh/setup-bun v2 pinned to commit SHA for reproducibility.
      - uses: oven-sh/setup-bun@0c5077e51419868618aeaa5fe8019c62421857d6
        with:
          bun-version: "1.3.14"

      - run: uv sync --dev
      - run: bun install --cwd frontend --frozen-lockfile
      - run: bun run --cwd frontend build
      - run: uv run pytest
      - run: uv run ruff check .
      - run: uv run ruff format --check .
```

In the release job, replace mutable refs with pinned refs:

```yaml
      - uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5
        with:
          fetch-depth: 0
          token: ${{ secrets.GITHUB_TOKEN }}

      - uses: astral-sh/setup-uv@d0d8abe699bfb85fec6de9f7adb5ae17292296ff

      - uses: actions/setup-python@a309ff8b426b58ec0e2a45f0f869d46889d02405
        with:
          python-version: "3.12"
```

Keep the existing pinned `oven-sh/setup-bun` release job step.

- [ ] **Step 4: Run workflow tests**

Run:

```bash
uv run pytest tests/test_release_workflow.py -q
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/release.yml tests/test_release_workflow.py
git commit -m "ci: build frontend bundle in release verification"
```

---

### Task 2: Normalize Release Build Commands

**Files:**
- Modify: `pyproject.toml`
- Modify: `src/hieronymus/release.py`
- Modify: `tests/test_release.py`
- Modify: `tests/test_release_workflow.py`

- [ ] **Step 1: Write failing command-shape assertions**

In `tests/test_release.py`, update the expected commands in `test_run_update_fetches_checks_out_and_reinstalls_managed_install()`:

```python
        (["bun", "install", "--cwd", "frontend", "--frozen-lockfile"], checkout),
        (["bun", "run", "--cwd", "frontend", "build"], checkout),
```

In `test_run_update_fetches_origin_main_before_checkout()`, update the expected commands the same way:

```python
        (["bun", "install", "--cwd", "frontend", "--frozen-lockfile"], checkout),
        (["bun", "run", "--cwd", "frontend", "build"], checkout),
```

In `tests/test_release_workflow.py`, update the semantic-release build command assertion to:

```python
    assert semantic_release["build_command"] == (
        "bun install --cwd frontend --frozen-lockfile && "
        "bun run --cwd frontend build && "
        "uv build"
    )
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
uv run pytest tests/test_release.py::test_run_update_fetches_checks_out_and_reinstalls_managed_install tests/test_release.py::test_run_update_fetches_origin_main_before_checkout tests/test_release_workflow.py::test_pyproject_configures_semantic_release -q
```

Expected: FAIL because `release.py` and `pyproject.toml` still use the old command shapes.

- [ ] **Step 3: Update Python managed update build command**

In `src/hieronymus/release.py`, replace `_build_frontend()` with:

```python
def _build_frontend(checkout: Path) -> None:
    _run(["bun", "install", "--cwd", "frontend", "--frozen-lockfile"], cwd=checkout)
    _run(["bun", "run", "--cwd", "frontend", "build"], cwd=checkout)
```

This preserves behavior but records commands from the managed checkout root, matching the command style used by CI and docs.

- [ ] **Step 4: Update semantic-release build command**

In `pyproject.toml`, change:

```toml
build_command = "bun --cwd frontend install --frozen-lockfile && bun --cwd frontend run build && uv build"
```

to:

```toml
build_command = "bun install --cwd frontend --frozen-lockfile && bun run --cwd frontend build && uv build"
```

- [ ] **Step 5: Run focused release tests**

Run:

```bash
uv run pytest tests/test_release.py tests/test_release_workflow.py -q
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add pyproject.toml src/hieronymus/release.py tests/test_release.py tests/test_release_workflow.py
git commit -m "build: normalize frontend bundle commands"
```

---

### Task 3: Update Active Docs And Roadmap

**Files:**
- Modify: `README.md`
- Modify: `docs/roadmap.md`

- [ ] **Step 1: Update active README command example**

In `README.md`, replace the frontend development command block:

```bash
bun install --cwd frontend --frozen-lockfile
bun --cwd frontend test
bun run --cwd frontend build
```

with:

```bash
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend build
bun --cwd frontend test
```

This keeps the currently working Bun test command and puts the required build command in the preferred project form.

- [ ] **Step 2: Update roadmap**

In `docs/roadmap.md`, move these remaining-work items out of `Install, Release, And Quality` remaining work:

```markdown
- Keep managed install, update, and release builds installing frontend
  dependencies and rebuilding `frontend/dist/main.js` before packaging or
  reinstalling.
- Fix command examples that use unsupported Bun flag order. Prefer
  `bun run --cwd frontend build` and `bun run --cwd frontend typecheck` for this
  Bun version.
```

Add these bullets to the `Completed:` list in the same section:

```markdown
- Managed install, managed update, PR verification, and release verification
  install frontend dependencies and rebuild `frontend/dist/main.js` before
  package validation, publishing, or reinstalling.
- Release build commands and active user-facing frontend build examples use
  the supported Bun command order for this Bun version.
```

Keep the `.superpowers/` / `.agents/` remaining-work bullet in place.

- [ ] **Step 3: Run docs grep checks**

Run:

```bash
rg -n "bun --cwd frontend (install|run build|typecheck)" README.md docs/usage.md pyproject.toml .github/workflows src tests
rg -n "bun run --cwd frontend build" README.md docs/usage.md pyproject.toml .github/workflows/release.yml .github/workflows/pr.yml
```

Expected: first command prints no matches. Second command includes active build examples/configuration in README, docs usage, pyproject, PR workflow, and release workflow. The pyproject match is inside the semantic-release `build_command` string, not a shell step.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/roadmap.md
git commit -m "docs: register frontend bundle release hygiene"
```

---

### Task 4: Final Verification

**Files:**
- No source file changes expected unless verification exposes a defect.

- [ ] **Step 1: Run backend verification**

Run:

```bash
bun run --cwd frontend build
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: PASS.

- [ ] **Step 2: Run frontend verification**

Run:

```bash
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend format
bun run --cwd frontend typecheck
bun --cwd frontend test
bun run --cwd frontend build
```

Expected: PASS. Existing React `act(...)` and OpenTUI `TerminalConsoleCache` warnings may still appear in `bun --cwd frontend test`; this plan does not address that separate roadmap item.

- [ ] **Step 3: Run final diff check**

Run:

```bash
git diff --check
git status --short --branch
```

Expected: `git diff --check` prints nothing. `git status --short --branch` shows a clean branch.

---

## Self-Review

Spec coverage:

- Managed release verification builds `frontend/dist/main.js` before package-quality tests: Task 1.
- Release workflow actions are immutable like PR workflow actions: Task 1.
- Managed update rebuild command style is normalized without changing behavior: Task 2.
- Semantic-release packaging installs dependencies and rebuilds the bundle before `uv build`: Task 2.
- Active user-facing frontend build examples avoid unsupported Bun build/typecheck flag order: Task 3.
- Roadmap records completed release/install frontend bundle hygiene: Task 3.
- Full backend and frontend verification: Task 4.

Red-flag scan:

- No planning markers, shortcut references, or undefined helper names remain.
- Every code or config change step includes the exact target content and the command that validates it.

Type consistency:

- Workflow SHA constants match the existing PR workflow pins.
- Preferred build commands are consistently `bun install --cwd frontend --frozen-lockfile` and `bun run --cwd frontend build`.
- Managed update test expectations match `_build_frontend(checkout)` command lists exactly.
