# Tailwind Rewrite Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish PR #21's Tailwind v4 rewrite by consolidating frontend packaging, replacing misleading source-string behavior tests with rendered component coverage, and cleaning migration-only CSS inconsistencies.

**Architecture:** Hatch remains the only frontend build entry point used by Python packaging, while the dedicated frontend CI job independently runs frontend quality gates. Vitest, jsdom, and Svelte Testing Library replace Bun-specific behavior tests; narrow filesystem-based source-contract tests remain for Tailwind configuration. Production styling changes are limited to dead-rule removal and semantic error-background normalization.

**Tech Stack:** Python 3.12, Hatchling custom build hook, uv, Bun 1.3.14, Svelte 5, Tailwind CSS v4, Vite 6, Vitest, jsdom, Svelte Testing Library, pytest, Ruff.

## Global Constraints

- Bun remains the package manager and package-script launcher; CI provisions Bun `1.3.14` with `oven-sh/setup-bun`.
- `hatch_build.py` invokes the `bun` executable from `$PATH`; it does not install or select Bun.
- `pyproject.toml` semantic-release `build_command` must be exactly `uv build`.
- Backend CI relies on a fresh runner with no restored `.venv`; do not add `--reinstall-package hieronymus` unless `.venv` caching is introduced.
- The frontend CI build and Hatch packaging build are intentionally separate checks even though both compile the frontend.
- Keep `.data-table` and `.editor-dialog`; remove only `.table-shell`, `.toggle-track`, and `.toggle-thumb`.
- Inline error alerts use `bg-[var(--hiero-danger-bg)] border-danger text-danger` in both themes.
- Do not address `remove_short_term_memory`, fetch timeouts, or any backend/security/database findings from `report.md`.
- Do not stage or commit the untracked `report.md` or `improvements.md` files.
- Follow TDD for behavior/configuration changes and commit after each task.

## File Map

- `hatch_build.py`: validate Bun availability and run the authoritative frontend package build.
- `pyproject.toml`: reduce semantic-release packaging to `uv build`.
- `install.sh`: remove the duplicate frontend build and stale TUI wording.
- `.github/workflows/pr.yml`: retain the frontend job; make backend `uv sync` the sole packaging build.
- `.github/workflows/release.yml`: do the same for release verification and release jobs.
- `tests/test_pr_workflow.py`: enforce fresh-runner Hatch ownership in PR CI.
- `tests/test_release_workflow.py`: enforce release CI, semantic-release, and build-hook contracts.
- `tests/test_release_scripts.py`: enforce installer delegation to `uv tool install`.
- `frontend/package.json` and `frontend/bun.lock`: add Vitest/jsdom/Testing Library and update scripts.
- `frontend/tsconfig.json`: browser-only production type scope.
- `frontend/tsconfig.test.json`: Vitest and Node test type scope.
- `frontend/vitest.config.ts`: Svelte/jsdom test configuration.
- `frontend/src/web/test/setup.ts`: DOM cleanup and dialog polyfill.
- `frontend/src/web/app.test.ts`: static Tailwind/configuration contracts using `node:fs/promises`.
- `frontend/src/web/lib/theme.svelte.test.ts`: Vitest coverage for theme persistence.
- `frontend/src/web/components/editors.test.ts`: rendered provider and dreaming editor interactions.
- `frontend/src/web/components/MemoryViews.test.ts`: rendered row keyboard selection and destructive confirmation.
- `frontend/src/web/app.css`: remove unused migration selectors.
- `frontend/src/web/App.svelte`, `AdminDashboard.svelte`, and `MemoryViews.svelte`: normalize inline error backgrounds.

---

### Task 1: Make missing-Bun package failures actionable

**Files:**
- Modify: `tests/test_release_workflow.py:247-256`
- Modify: `hatch_build.py:1-20`

**Interfaces:**
- Consumes: Bun `1.3.14` made available on `$PATH` by CI or local installation.
- Produces: `CustomBuildHook.initialize(...)` that raises `RuntimeError("Bun >=1.3 is required to build the Hieronymus web console; install it and ensure `bun` is on PATH")` before spawning a subprocess when Bun is absent.

- [ ] **Step 1: Tighten the build-hook contract test**

Add these assertions to `test_pyproject_configures_local_frontend_build_hook`:

```python
    assert "from shutil import which" in build_hook
    assert 'which("bun") is None' in build_hook
    assert "Bun >=1.3 is required to build the Hieronymus web console" in build_hook
    assert "install it and ensure `bun` is on PATH" in build_hook
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
uv run pytest tests/test_release_workflow.py::test_pyproject_configures_local_frontend_build_hook -q
```

Expected: FAIL because `hatch_build.py` does not import or call `shutil.which`.

- [ ] **Step 3: Add the fail-fast Bun check**

Update `hatch_build.py` to:

```python
from pathlib import Path
from shutil import which
from subprocess import run

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version: str, build_data: dict[str, object]) -> None:
        if which("bun") is None:
            raise RuntimeError(
                "Bun >=1.3 is required to build the Hieronymus web console; "
                "install it and ensure `bun` is on PATH"
            )

        frontend = Path(self.root) / "frontend"
        run(
            "bun install --frozen-lockfile".split(),
            check=True,
            cwd=frontend,
        )
        run(
            "bun run build".split(),
            check=True,
            cwd=frontend,
        )
```

- [ ] **Step 4: Run the focused test and verify GREEN**

Run:

```bash
uv run pytest tests/test_release_workflow.py::test_pyproject_configures_local_frontend_build_hook -q
```

Expected: `1 passed`.

- [ ] **Step 5: Commit**

```bash
git add hatch_build.py tests/test_release_workflow.py
git commit -m "fix: explain missing Bun during package builds"
```

---

### Task 2: Consolidate packaging around the Hatch hook

**Files:**
- Modify: `tests/test_pr_workflow.py:87-129`
- Modify: `tests/test_release_workflow.py:97-139,223-256`
- Modify: `tests/test_release_scripts.py:39-144,147-225`
- Modify: `.github/workflows/pr.yml:27-37`
- Modify: `.github/workflows/release.yml:27-37`
- Modify: `pyproject.toml:60-67`
- Modify: `install.sh:101-157,230-233`

**Interfaces:**
- Consumes: the fail-fast Hatch hook from Task 1.
- Produces: one frontend build per Python packaging operation; a separate frontend CI job still runs its own frontend-only build.

- [ ] **Step 1: Change workflow and release tests to describe the single build path**

In `test_pr_workflow_backend_job_runs_python_checks`, replace `backend_runs` expectations with:

```python
    backend_runs = [line for line in backend if line.startswith("      - run: ")]
    assert backend_runs == [
        "      - run: uv sync --dev",
        "      - run: uv run pytest",
        "      - run: uv run ruff check .",
        "      - run: uv run ruff format --check .",
    ]
    assert (
        "      # Fresh runners do not restore .venv, so uv sync installs the editable project "
        "and Hatch builds the frontend."
    ) in backend
    assert not any("--reinstall-package" in line for line in backend)
```

In `test_release_workflow_verify_job_uses_read_only_credentials`, replace `verify_runs` with:

```python
    verify_runs = [line for line in verify if line.startswith("      - run: ")]
    assert verify_runs == [
        "      - run: uv sync --dev",
        "      - run: uv run pytest",
        "      - run: uv run ruff check .",
        "      - run: uv run ruff format --check .",
    ]
    assert (
        "      # Fresh runners do not restore .venv, so uv sync installs the editable project "
        "and Hatch builds the frontend."
    ) in verify
    assert not any("--reinstall-package" in line for line in verify)
```

Change the semantic-release assertion to:

```python
    assert semantic_release["build_command"] == "uv build"
```

In `tests/test_release_scripts.py`, rename the stable installer test to
`test_install_script_delegates_frontend_build_to_tool_install_and_writes_stable_channel` and
replace its command assertions with:

```python
    assert f"uv:{ROOT}:tool install --force --reinstall {app_dir}" in commands
    assert not any(command.startswith("bun:") and "run build" in command for command in commands)
```

In the dev-channel test, replace the Bun build assertion with:

```python
    assert f"uv:{ROOT}:tool install --force --reinstall {app_dir}" in commands
    assert not any(command.startswith("bun:") and "run build" in command for command in commands)
```

Finally, change `test_install_script_uses_managed_github_checkout` to assert:

```python
    assert "build_frontend()" not in text
    assert "Building OpenTUI frontend" not in text
    assert "Hieronymus TUI" not in text
    assert "Hieronymus web console" in text
```

- [ ] **Step 2: Run the focused backend tests and verify RED**

Run:

```bash
uv run pytest tests/test_pr_workflow.py tests/test_release_workflow.py tests/test_release_scripts.py -q
```

Expected: FAIL on the old explicit Bun steps, semantic-release command, installer build calls, and TUI wording.

- [ ] **Step 3: Remove duplicated build recipes**

Set this exact value in `pyproject.toml`:

```toml
build_command = "uv build"
```

Delete the entire `build_frontend()` function and the `build_frontend` call from `install.sh`.
Replace both installer errors with:

```sh
echo "error: Bun >= 1.3 is required to build the Hieronymus web console" >&2
```

In the backend job of `.github/workflows/pr.yml`, replace the sync/build block with:

```yaml
      # Fresh runners do not restore .venv, so uv sync installs the editable project and Hatch builds the frontend.
      - run: uv sync --dev
      - run: uv run pytest
```

Make the same replacement in the `verify` job of `.github/workflows/release.yml`. Do not change
the dedicated frontend job or the release job's Bun setup.

- [ ] **Step 4: Run the focused backend tests and verify GREEN**

Run:

```bash
uv run pytest tests/test_pr_workflow.py tests/test_release_workflow.py tests/test_release_scripts.py -q
```

Expected: all selected tests pass.

- [ ] **Step 5: Verify one forced package build still compiles the frontend**

Run:

```bash
uv sync --dev --reinstall-package hieronymus
```

Expected: output contains `Building hieronymus`, frontend `vite build` succeeds, and Hieronymus is reinstalled.

- [ ] **Step 6: Commit**

```bash
git add pyproject.toml install.sh .github/workflows/pr.yml .github/workflows/release.yml tests/test_pr_workflow.py tests/test_release_workflow.py tests/test_release_scripts.py
git commit -m "build: make Hatch own frontend packaging"
```

---

### Task 3: Replace Bun-specific source tests with a Vitest harness

**Files:**
- Modify: `frontend/package.json`
- Modify: `frontend/bun.lock`
- Modify: `frontend/tsconfig.json`
- Create: `frontend/tsconfig.test.json`
- Create: `frontend/vitest.config.ts`
- Create: `frontend/src/web/test/setup.ts`
- Modify: `frontend/src/web/app.test.ts`
- Modify: `frontend/src/web/lib/theme.svelte.test.ts`

**Interfaces:**
- Consumes: Svelte 5 components and Vite configuration already in the frontend.
- Produces: `bun run test` -> `vitest run`; browser types isolated from Node/Vitest test types; filesystem source-contract helpers based on `node:fs/promises`.

- [ ] **Step 1: Convert the existing tests to Vitest imports and Node file access**

Replace `frontend/src/web/app.test.ts` with:

```typescript
import { access, readFile } from "node:fs/promises";
import { expect, test } from "vitest";

const webFile = (path: string) => new URL(path, import.meta.url);
const source = (path: string) => readFile(webFile(path), "utf8");

test("the web stylesheet configures Tailwind and the data-theme dark variant", async () => {
  const css = await source("./app.css");
  expect(css).toContain('@import "tailwindcss";');
  expect(css).toContain(
    '@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));',
  );
});

test("the semantic theme exposes required runtime tokens and utilities", async () => {
  const css = await source("./app.css");
  for (const token of [
    "--hiero-bg-root",
    "--hiero-text-primary",
    "--hiero-danger",
    "--hiero-success",
  ]) {
    expect(css).toContain(token);
  }
  expect(css).toContain("@utility text-display");
  expect(css).toContain("--color-strong: var(--hiero-border-strong)");
  expect(css).toContain("--animate-toast-in");
  expect(css).toContain("--breakpoint-sm: 45rem");
  expect(css).toMatch(/\[data-theme="light"\]\s*\{[\s\S]*?--hiero-bg-root:/);
  expect(css).toMatch(/\[data-theme="dark"\]\s*\{[\s\S]*?--hiero-bg-root:/);
});

test("the editor dialog source contract remains viewport-bounded and right-aligned", async () => {
  const css = await source("./app.css");
  expect(css).toMatch(
    /\.editor-dialog\s*\{[\s\S]*?right:\s*0;[\s\S]*?width:\s*min\(420px,\s*100%\);/,
  );
});

test("the web entry imports the Tailwind stylesheet", async () => {
  expect(await source("./main.ts")).toContain('import "./app.css";');
});

test("legacy stylesheets remain deleted", async () => {
  await expect(access(webFile("./base.css"))).rejects.toThrow();
  await expect(access(webFile("./tokens.css"))).rejects.toThrow();
  await expect(access(webFile("./components.css"))).rejects.toThrow();
});
```

Replace `frontend/src/web/lib/theme.svelte.test.ts` with:

```typescript
import { beforeEach, expect, test, vi } from "vitest";

beforeEach(() => {
  vi.resetModules();
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({ matches: false })),
  );
});

test("theme toggle applies and persists the selected theme", async () => {
  const { createThemeToggle } = await import("./theme.svelte");
  const theme = createThemeToggle();

  expect(theme.theme).toBe("dark");
  expect(document.documentElement.getAttribute("data-theme")).toBe("dark");

  theme.toggle();

  expect(theme.theme).toBe("light");
  expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  expect(localStorage.getItem("hiero-theme")).toBe("light");
});
```

- [ ] **Step 2: Run the frontend test command and verify RED**

Run:

```bash
bun run --cwd frontend test
```

Expected: FAIL because the package script still invokes `bun test` and `vitest` is not installed/configured.

- [ ] **Step 3: Add the test dependencies**

Run:

```bash
bun add --cwd frontend --dev vitest jsdom @testing-library/svelte @testing-library/user-event @types/node
```

Expected: `frontend/package.json` and `frontend/bun.lock` add the five development dependencies.

- [ ] **Step 4: Configure scripts and separate type scopes**

Set these scripts in `frontend/package.json`:

```json
"test": "vitest run",
"typecheck": "tsc -p tsconfig.json --noEmit && tsc -p tsconfig.test.json --noEmit"
```

Update `frontend/tsconfig.json` to:

```json
{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "rootDir": ".",
    "outDir": "dist",
    "types": ["vite/client"]
  },
  "include": ["src/web/**/*.ts", "src/web/**/*.svelte", "vite.config.ts"],
  "exclude": ["src/web/**/*.test.ts", "src/web/**/*.test.svelte.ts"]
}
```

Create `frontend/tsconfig.test.json`:

```json
{
  "extends": "./tsconfig.json",
  "compilerOptions": {
    "noEmit": true,
    "types": ["vite/client", "node", "vitest/globals"]
  },
  "include": [
    "src/web/**/*.test.ts",
    "src/web/**/*.test.svelte.ts",
    "src/web/**/*.svelte",
    "vitest.config.ts"
  ],
  "exclude": []
}
```

Create `frontend/vitest.config.ts`:

```typescript
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [svelte()],
  resolve: { conditions: ["browser"] },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/web/test/setup.ts"],
    clearMocks: true,
  },
});
```

Create `frontend/src/web/test/setup.ts`:

```typescript
import { cleanup } from "@testing-library/svelte";
import { afterEach, vi } from "vitest";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
});
```

- [ ] **Step 5: Run tests and both type scopes**

Run:

```bash
bun run --cwd frontend test
bun run --cwd frontend typecheck
```

Expected: the converted source-contract and theme tests pass; both TypeScript configurations pass without `bun-types`.

- [ ] **Step 6: Commit**

```bash
git add frontend/package.json frontend/bun.lock frontend/tsconfig.json frontend/tsconfig.test.json frontend/vitest.config.ts frontend/src/web/test/setup.ts frontend/src/web/app.test.ts frontend/src/web/lib/theme.svelte.test.ts
git commit -m "test: add rendered Svelte test harness"
```

---

### Task 4: Add rendered editor and memory-view behavior coverage

**Files:**
- Create: `frontend/src/web/components/editors.test.ts`
- Create: `frontend/src/web/components/MemoryViews.test.ts`
- Modify: `frontend/src/web/test/setup.ts`

**Interfaces:**
- Consumes: `render`, `screen`, `waitFor` from `@testing-library/svelte`; `userEvent`; real component prop and API types.
- Produces: behavior coverage for provider submission/close, dreaming state submission, keyboard row selection, and destructive confirmation.

- [ ] **Step 1: Add rendered editor tests**

Create `frontend/src/web/components/editors.test.ts` with typed fixtures and callbacks:

```typescript
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import type { DreamSettings, ModelCache, ProviderDraft, ProviderProfile } from "../lib/types";
import DreamingEditor from "./DreamingEditor.svelte";
import ProviderEditor from "./ProviderEditor.svelte";

const provider = {
  id: "openai-main",
  name: "OpenAI Main",
  type: "openai",
  url: "https://api.openai.com/v1",
  key_configured: true,
  model: "gpt-5",
  timeout_seconds: 30,
} satisfies ProviderProfile;

const dream = {
  dreaming: {
    enabled: false,
    schedule_interval_minutes: 30,
    min_pending_short_term_memories: 20,
    max_pending_short_term_memories: 200,
    max_short_term_memories_per_cycle: 50,
    not_enough_memories_cycle_threshold: 5,
    max_changed_crystals_per_cycle: 200,
    max_related_concepts_per_cycle: 80,
    max_related_crystals_per_concept: 20,
    max_total_affected_crystals: 500,
    max_short_term_memories_per_run: 500,
    max_long_term_records_affected_per_run: 1000,
    max_relation_records_per_pass: 1000,
    general_prompt: "Keep evidence explicit.",
  },
  workflows: {
    concepts: {
      provider: "openai-main",
      model: "gpt-5",
      enabled: true,
      max_records_per_pass: 20,
    },
  },
} satisfies DreamSettings;

const modelCache = {
  providers: { "openai-main": { models: ["gpt-5"] } },
} satisfies ModelCache;

test("provider editor opens, submits edited fields, and closes", async () => {
  const user = userEvent.setup();
  const onSave = vi.fn<(draft: ProviderDraft) => void>();
  const onClose = vi.fn<() => void>();

  render(ProviderEditor, {
    props: {
      provider,
      models: [],
      onSave,
      onDelete: vi.fn(),
      onRefreshModels: vi.fn(),
      onCheck: vi.fn(),
      onClose,
    },
  });

  const dialog = await screen.findByRole("dialog", { name: "Edit OpenAI Main" });
  expect(dialog.hasAttribute("open")).toBe(true);
  const name = screen.getByLabelText("Display name");
  await user.clear(name);
  await user.type(name, "Primary OpenAI");
  await user.click(screen.getByRole("button", { name: "Save profile" }));
  expect(onSave).toHaveBeenCalledWith({
    id: "openai-main",
    name: "Primary OpenAI",
    type: "openai",
    url: "https://api.openai.com/v1",
    key: "",
    timeout_seconds: "30",
  });

  await user.click(screen.getByRole("button", { name: "Close editor" }));
  expect(onClose).toHaveBeenCalledOnce();
});

test("dreaming editor submits the toggled schedule state", async () => {
  const user = userEvent.setup();
  const onSave = vi.fn<(settings: DreamSettings) => void>();
  render(DreamingEditor, {
    props: { initial: dream, providers: [provider], modelCache, onSave },
  });

  await user.click(screen.getByRole("checkbox", { name: "Enable scheduled dreaming" }));
  await user.click(screen.getByRole("button", { name: "Save dreaming" }));

  expect(onSave).toHaveBeenCalledWith(
    expect.objectContaining({
      dreaming: expect.objectContaining({ enabled: true }),
    }),
  );
});
```

- [ ] **Step 2: Add rendered memory behavior tests with typed API mocks**

Create `frontend/src/web/components/MemoryViews.test.ts`:

```typescript
import { render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import { loadAdminSnapshot, runAdminAction } from "../lib/api";
import type { AdminActionResult, AdminDashboard, AdminSnapshot } from "../lib/types";
import MemoryViews from "./MemoryViews.svelte";

vi.mock("../lib/api", () => ({
  loadAdminSnapshot: vi.fn(),
  runAdminAction: vi.fn(),
}));

const loadSnapshotMock = vi.mocked(loadAdminSnapshot);
const runActionMock = vi.mocked(runAdminAction);

const dashboard = {
  header: { product: "Hieronymus", version: "0.4.0", tagline: "Translation memory" },
  stats: {},
  views: ["Crystals"],
  short_term_status: {},
  dream_status: {},
} satisfies AdminDashboard;

const row = {
  id: 7,
  kind: "crystal",
  label: "Crystal Alpha",
  status: "active",
  scope: "series",
  language_pair: "en-ru",
  quality_label: "strong",
  tags: [],
};

const listSnapshot = {
  snapshot: {
    view: "Crystals",
    rows: [row],
    selected: null,
    detail: { title: "Crystals", subtitle: "Choose a crystal", body: "", fields: [] },
  },
} satisfies AdminSnapshot;

const selectedSnapshot = {
  snapshot: {
    view: "Crystals",
    rows: [row],
    selected: row,
    detail: { title: "Crystal Alpha", subtitle: "Selected", body: "Evidence", fields: [] },
  },
} satisfies AdminSnapshot;

beforeEach(() => {
  loadSnapshotMock.mockReset();
  runActionMock.mockReset();
  loadSnapshotMock.mockResolvedValueOnce(listSnapshot).mockResolvedValue(selectedSnapshot);
});

test.each(["{Enter}", " "])("memory rows load by keyboard activation: %s", async (key) => {
  const user = userEvent.setup();
  render(MemoryViews, { props: { dashboard, onNotice: vi.fn() } });
  const memoryRow = await screen.findByRole("button", { name: /Crystal Alpha/ });
  memoryRow.focus();
  await user.keyboard(key);
  await waitFor(() => expect(loadSnapshotMock).toHaveBeenCalledWith("Crystals", 7));
});

test("destructive memory actions require confirmation and send the exact payload", async () => {
  const user = userEvent.setup();
  const actionResult = {
    result: { message: "Deleted Crystal Alpha." },
    snapshot: listSnapshot.snapshot,
  } satisfies AdminActionResult;
  runActionMock.mockResolvedValue(actionResult);

  render(MemoryViews, { props: { dashboard, onNotice: vi.fn() } });
  await user.click(await screen.findByRole("button", { name: /Crystal Alpha/ }));
  await screen.findByText("Evidence");
  await user.click(screen.getByRole("button", { name: "Delete" }));
  expect(runActionMock).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm Delete" }));
  expect(runActionMock).toHaveBeenCalledWith("delete_crystal", { id: 7, confirmed: true });
});
```

- [ ] **Step 3: Run rendered tests and verify RED**

Run:

```bash
bun run --cwd frontend test src/web/components/editors.test.ts src/web/components/MemoryViews.test.ts
```

Expected: FAIL because jsdom does not implement `HTMLDialogElement.showModal` for `ProviderEditor`.

- [ ] **Step 4: Add the dialog test-environment polyfill**

Append to `frontend/src/web/test/setup.ts`:

```typescript
if (!HTMLDialogElement.prototype.showModal) {
  HTMLDialogElement.prototype.showModal = function showModal() {
    this.open = true;
  };
}
```

- [ ] **Step 5: Run rendered tests and verify GREEN**

Run:

```bash
bun run --cwd frontend test src/web/components/editors.test.ts src/web/components/MemoryViews.test.ts
bun run --cwd frontend typecheck
```

Expected: all editor/memory tests and both type scopes pass. If a query does not match the
rendered accessible name exactly, inspect the rendered DOM and correct the query; do not fall back
to source-string assertions or test IDs.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/web/test/setup.ts frontend/src/web/components/editors.test.ts frontend/src/web/components/MemoryViews.test.ts
git commit -m "test: cover Tailwind console interactions"
```

---

### Task 5: Remove dead migration CSS and normalize error alerts

**Files:**
- Modify: `frontend/src/web/app.test.ts`
- Modify: `frontend/src/web/app.css:180-230`
- Modify: `frontend/src/web/App.svelte:243,271`
- Modify: `frontend/src/web/components/AdminDashboard.svelte:73`
- Modify: `frontend/src/web/components/MemoryViews.svelte:135`

**Interfaces:**
- Consumes: semantic `--hiero-danger-bg`, `border-danger`, and `text-danger` tokens.
- Produces: no unused migration selectors; all inline error alerts use the danger background.

- [ ] **Step 1: Add failing source-contract tests for the CSS cleanup**

Append to `frontend/src/web/app.test.ts`:

```typescript
test("the Tailwind stylesheet contains only used shared component selectors", async () => {
  const css = await source("./app.css");
  expect(css).not.toContain(".table-shell");
  expect(css).not.toContain(".toggle-track");
  expect(css).not.toContain(".toggle-thumb");
  expect(css).toContain(".data-table");
  expect(css).toContain(".editor-dialog");
});

test("inline error alerts use the semantic danger background", async () => {
  for (const path of [
    "./App.svelte",
    "./components/AdminDashboard.svelte",
    "./components/MemoryViews.svelte",
    "./components/DreamingEditor.svelte",
    "./components/IngestEditor.svelte",
    "./components/ProviderEditor.svelte",
    "./components/ReleaseEditor.svelte",
  ]) {
    const component = await source(path);
    const alerts = component.match(/<p class="[^"]*border-danger[^"]*text-danger[^"]*"/g) ?? [];
    expect(alerts.length, `${path} should expose an inline error alert`).toBeGreaterThan(0);
    for (const alert of alerts) {
      expect(alert).toContain("bg-[var(--hiero-danger-bg)]");
      expect(alert).not.toContain("bg-raised");
    }
  }
});
```

- [ ] **Step 2: Run the source-contract tests and verify RED**

Run:

```bash
bun run --cwd frontend test src/web/app.test.ts
```

Expected: FAIL because the three selectors still exist and App/AdminDashboard/MemoryViews still use `bg-raised` for errors.

- [ ] **Step 3: Remove only the dead selectors**

Delete the complete `.table-shell`, `.toggle-track`, and `.toggle-thumb` blocks from the
`@layer components` block in `frontend/src/web/app.css`. Leave these blocks unchanged:

```css
.data-table {
  width: 100%;
  margin: 0;
  border-collapse: collapse;
}

.editor-dialog {
  position: fixed;
  top: 0;
  right: 0;
  width: min(420px, 100%);
  height: 100dvh;
  margin: 0;
  padding: 24px;
  border: 0;
  border-left: 1px solid var(--hiero-border-default);
  background: var(--hiero-bg-root);
  box-shadow: -16px 0 40px rgb(0 0 0 / 15%);
}
```

- [ ] **Step 4: Normalize the four remaining error alerts**

In both error paragraphs in `App.svelte`, the error paragraph in `AdminDashboard.svelte`, and the
error paragraph at `MemoryViews.svelte:135`, replace only `bg-raised` with:

```text
bg-[var(--hiero-danger-bg)]
```

Do not replace `bg-raised` on buttons, tables, selected rows, or neutral loading states.

- [ ] **Step 5: Run frontend verification and verify GREEN**

Run:

```bash
bun run --cwd frontend format
bun run --cwd frontend typecheck
bun run --cwd frontend test
bun run --cwd frontend build
```

Expected: formatting, both type scopes, all Vitest tests, and the Vite production build pass.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/web/app.test.ts frontend/src/web/app.css frontend/src/web/App.svelte frontend/src/web/components/AdminDashboard.svelte frontend/src/web/components/MemoryViews.svelte
git commit -m "style: finish Tailwind migration cleanup"
```

---

### Task 6: Run the complete package and repository gate

**Files:**
- Verify only; modify files only if a check exposes a regression caused by Tasks 1-5.

**Interfaces:**
- Consumes: all deliverables from Tasks 1-5.
- Produces: fresh evidence that the frontend, editable package build, Python tests, and formatting gates pass together.

- [ ] **Step 1: Verify the locked frontend environment and all frontend gates**

Run:

```bash
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend format
bun run --cwd frontend typecheck
bun run --cwd frontend test
bun run --cwd frontend build
```

Expected: frozen install has no lockfile changes; format/typecheck/tests/build all pass.

- [ ] **Step 2: Force the exact editable package rebuild path**

Run:

```bash
uv sync --dev --reinstall-package hieronymus
```

Expected: Hatch invokes the frontend install/build and reinstalls `hieronymus` successfully.

- [ ] **Step 3: Run the repository-required Python gates**

Run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: all pytest tests pass; Ruff reports no lint or formatting violations.

- [ ] **Step 4: Inspect final scope**

Run:

```bash
git diff --check
git status --short
git log --oneline -6
```

Expected: no whitespace errors; only intended tracked changes/commits are present; `report.md` and
`improvements.md` remain untracked and unstaged.

- [ ] **Step 5: Push only after explicit user instruction**

Do not push automatically. When instructed, push the existing branch:

```bash
git push origin agent/tailwind-web-console
```
