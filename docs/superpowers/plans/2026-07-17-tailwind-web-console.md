# Tailwind Web Console Implementation Plan

> [!CAUTION]
> **Superseded:** Follow the [Tailwind rewrite remediation plan](./2026-07-18-tailwind-rewrite-remediation.md) instead. The Bun-specific test commands and `mise`-based Hatch build hook below are historical implementation notes and must not be treated as current guidance.
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the frontend's legacy component CSS with a Tailwind CSS v4 editorial-workbench interface while preserving all current Svelte behavior and the persisted `data-theme` preference.

**Architecture:** Tailwind's Vite plugin scans the Svelte source and generates utility classes from a single `app.css` entry point. Runtime `--hiero-*` semantic variables remain owned by the existing `data-theme` selectors; `@theme` maps Tailwind utility names to those variables, and a selector-based `dark` custom variant honors the saved theme. A small CSS component layer remains only for selectors that cannot be expressed legibly in Svelte markup.

**Tech Stack:** Svelte 5 runes, TypeScript, Vite 6, Bun 1.3, Tailwind CSS v4, local Literata/Geist/Inconsolata fonts.

## Global Constraints

- Preserve routes, API calls, Svelte state, keyboard behavior, dialog focus behavior, and text content unless an accessibility correction is required.
- Use `@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));`; do not add `tailwind.config.js` or duplicate theme state.
- Keep `fonts.css` with local `@font-face` declarations and `font-display: swap`.
- Keep all runtime color values under `--hiero-*`; map `--color-*` Tailwind tokens to them in `@theme`.
- Use custom responsive tokens: `xs` 30rem, `sm` 45rem, `md` 60rem, `lg` 67.5rem; use `min-h-11` for interactive controls.
- Prefer inline Tailwind utilities. Restrict `@layer components` to form normalization, peer toggle styles, table-cell selection/hover styles, and dialog/backdrop styles.
- Preserve reduced-motion behavior with `motion-reduce:` utilities and a base reduced-motion rule for the named CSS animations.
- Do not add an icon package, a component library, backend changes, or API changes.

---

## File Structure

- Create: `frontend/src/web/app.css` — Tailwind import, selector dark variant, semantic theme values, `@theme` mappings, typography and animation utilities, small extracted component layer, base accessibility rules.
- Create: `frontend/src/web/app.test.ts` — Bun contract tests for the Tailwind entry stylesheet.
- Modify: `frontend/package.json` — Tailwind v4 build dependencies.
- Modify: `frontend/vite.config.ts` — register `@tailwindcss/vite` beside Svelte.
- Modify: `frontend/src/web/main.ts` — import `fonts.css` and `app.css` only.
- Modify: `frontend/src/web/App.svelte` — Tailwind page shell, navigation, provider page, table, responsive layout.
- Modify: `frontend/src/web/components/AdminDashboard.svelte` — Tailwind editorial dashboard and workflow status.
- Modify: `frontend/src/web/components/MemoryViews.svelte` — Tailwind memory-table and detail-panel layout.
- Modify: `frontend/src/web/components/DreamingEditor.svelte` — Tailwind settings/workflow form layout.
- Modify: `frontend/src/web/components/IngestEditor.svelte` — Tailwind settings form layout.
- Modify: `frontend/src/web/components/ReleaseEditor.svelte` — Tailwind release form layout.
- Modify: `frontend/src/web/components/ProviderEditor.svelte` — Tailwind native-dialog editor layout.
- Modify: `frontend/src/web/components/Toast.svelte` — Tailwind notification styling.
- Delete: `frontend/src/web/tokens.css`, `frontend/src/web/base.css`, `frontend/src/web/components.css` — superseded by `app.css`.

### Task 1: Add Tailwind v4 build wiring

**Files:**
- Modify: `frontend/package.json`
- Modify: `frontend/vite.config.ts`
- Create: `frontend/src/web/app.test.ts`

**Interfaces:**
- Consumes: Vite's `plugins` array and Bun's existing `bun test --pass-with-no-tests` command.
- Produces: Tailwind utility generation for all `.svelte` files and a test that guards the source integration.

- [ ] **Step 1: Write the failing build-wiring test**

```ts
import { expect, test } from "bun:test";

test("the web stylesheet defines the data-theme Tailwind dark variant", async () => {
  const css = await Bun.file(new URL("./app.css", import.meta.url)).text();
  expect(css).toContain('@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));');
  expect(css).toContain('@import "tailwindcss";');
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bun test src/web/app.test.ts`

Expected: FAIL because `app.css` does not exist.

- [ ] **Step 3: Install and configure Tailwind**

Run: `bun add -D tailwindcss@^4 @tailwindcss/vite@^4 prettier-plugin-tailwindcss`

Then update `frontend/vite.config.ts`:

```ts
import tailwindcss from "@tailwindcss/vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  build: { outDir: "dist", emptyOutDir: true },
});
```

Create the minimal entry stylesheet:

```css
@import "tailwindcss";

@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `bun test src/web/app.test.ts`

Expected: PASS.

- [ ] **Step 5: Commit the isolated build integration**

```bash
git add frontend/package.json frontend/bun.lock frontend/vite.config.ts frontend/src/web/app.css frontend/src/web/app.test.ts
git commit -m "build: add Tailwind v4 frontend integration"
```

### Task 2: Establish semantic theme, typography, animation, and base rules

**Files:**
- Modify: `frontend/src/web/app.css`
- Modify: `frontend/src/web/app.test.ts`
- Modify: `frontend/src/web/main.ts`
- Modify: `frontend/src/web/fonts.css`

**Interfaces:**
- Consumes: the existing `data-theme` set by `frontend/index.html` and `createThemeToggle()`.
- Produces: `bg-root`, `bg-surface`, `text-primary`, `border-default`, `text-display`, `animate-toast-in`, and `dark:` utility behavior for every migrated component.

- [ ] **Step 1: Extend the failing CSS contract test**

```ts
test("the semantic theme preserves both runtime modes and editorial utilities", async () => {
  const css = await Bun.file(new URL("./app.css", import.meta.url)).text();
  for (const token of ["--hiero-bg-root", "--hiero-text-primary", "--hiero-danger", "--hiero-success"]) {
    expect(css).toContain(token);
  }
  expect(css).toContain("@utility text-display");
  expect(css).toContain("--animate-toast-in");
  expect(css).toContain("--breakpoint-sm: 45rem");
});
```

- [ ] **Step 2: Run the CSS contract test to verify it fails**

Run: `bun test src/web/app.test.ts`

Expected: FAIL because the semantic tokens and utilities have not been added.

- [ ] **Step 3: Implement the global Tailwind stylesheet and import it**

Use this structure in `app.css` (complete every listed semantic token for both themes):

```css
@import "tailwindcss";
@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));

@theme {
  --font-serif: "Literata", georgia, "Times New Roman", serif;
  --font-sans: "Geist", -apple-system, blinkmacsystemfont, "Segoe UI", sans-serif;
  --font-mono: "Inconsolata LGC", "Inconsolata", "Courier New", monospace;
  --color-root: var(--hiero-bg-root);
  --color-surface: var(--hiero-bg-surface);
  --color-raised: var(--hiero-bg-raised);
  --color-primary: var(--hiero-text-primary);
  --color-secondary: var(--hiero-text-secondary);
  --color-tertiary: var(--hiero-text-tertiary);
  --color-accent: var(--hiero-accent);
  --color-danger: var(--hiero-danger);
  --color-success: var(--hiero-success);
  --breakpoint-xs: 30rem;
  --breakpoint-sm: 45rem;
  --breakpoint-md: 60rem;
  --breakpoint-lg: 67.5rem;
}

@utility text-display { font: 400 clamp(28px, 4vw, 36px) / 1.15 var(--font-serif); }
@utility text-h2 { font: 400 clamp(20px, 2.5vw, 24px) / 1.25 var(--font-serif); }
@utility text-h3 { font: 600 15px / 1.3 var(--font-sans); }
@utility text-body { font: 400 14px / 1.55 var(--font-sans); }
@utility text-body-sm { font: 400 13px / 1.5 var(--font-sans); }
@utility text-mono { font: 400 13px / 1.5 var(--font-mono); }
@utility text-eyebrow { font: 500 10px / 1 var(--font-sans); }
@utility text-caption { font: 400 11px / 1.4 var(--font-sans); }
```

Define the light and dark `--hiero-*` values from `tokens.css`, `@theme inline` keyframes for `fade-in`, `slide-in`, and `toast-in`, base `box-sizing`, root/body/background/font rules, focus-visible rings, and the reduced-motion override. Retain only `@layer components` selectors for `.table-shell`, `.data-table`, `.toggle-track`, `.toggle-thumb`, and `.editor-dialog`; use `@apply` only for stable repeated declarations.

Change `main.ts` to:

```ts
import { mount } from "svelte";
import App from "./App.svelte";
import "./fonts.css";
import "./app.css";

mount(App, { target: document.getElementById("app")! });
```

- [ ] **Step 4: Run the CSS test and static checks**

Run: `bun test src/web/app.test.ts && bun run typecheck && bun run format`

Expected: PASS, TypeScript exits 0, and Prettier reports matching files.

- [ ] **Step 5: Commit the global visual contract**

```bash
git add frontend/src/web/app.css frontend/src/web/app.test.ts frontend/src/web/main.ts frontend/src/web/fonts.css
git commit -m "feat: define Tailwind web console theme"
```

### Task 3: Migrate the app shell and provider page

**Files:**
- Modify: `frontend/src/web/App.svelte`

**Interfaces:**
- Consumes: existing `section`, `busy`, `error`, `themeToggle`, provider actions, and child component callback props.
- Produces: the responsive page shell and provider table using Tailwind utility classes without altering event handlers or data loading.

- [ ] **Step 1: Add a failing source-contract test for the Tailwind app shell**

```ts
test("the app shell uses the semantic Tailwind surface utilities", async () => {
  const app = await Bun.file(new URL("./App.svelte", import.meta.url)).text();
  expect(app).toContain("min-h-dvh");
  expect(app).toContain("bg-root");
  expect(app).toContain("border-default");
});
```

- [ ] **Step 2: Run the test to verify it fails before shell migration**

Run: `bun test src/web/app.test.ts`

Expected: FAIL because the app shell does not yet use semantic Tailwind utilities.

- [ ] **Step 3: Convert `App.svelte` markup to Tailwind utilities**

Use a `min-h-dvh bg-root text-primary font-sans` shell; a sticky `border-b border-default bg-surface` header; `mx-auto w-full max-w-[90rem] px-4 py-6 sm:px-8 lg:px-12` workspace spacing; and a `grid gap-8 md:grid-cols-[minmax(14rem,18rem)_minmax(0,1fr)]` editorial layout where the lead/stage pattern appears. Convert navigation links, tabs, buttons, provider table, empty/loading/error states, and modal mounting. Preserve every current `onclick`, `onkeydown`, aria attribute, and Svelte conditional.

- [ ] **Step 4: Run Svelte checks**

Run: `bun run typecheck && bun run format`

Expected: both commands exit 0.

- [ ] **Step 5: Commit the shell migration**

```bash
git add frontend/src/web/App.svelte
git commit -m "refactor: migrate web app shell to Tailwind"
```

### Task 4: Migrate dashboard and memory administration views

**Files:**
- Modify: `frontend/src/web/components/AdminDashboard.svelte`
- Modify: `frontend/src/web/components/MemoryViews.svelte`

**Interfaces:**
- Consumes: unchanged `AdminDashboard`, `AdminSnapshot`, action callbacks, and row keyboard handlers.
- Produces: responsive dashboard cards, workflow progress, scrollable memory tables, and readable detail panels.

- [ ] **Step 1: Write a failing markup contract test**

```ts
test("memory rows keep keyboard activation after Tailwind migration", async () => {
  const memoryView = await Bun.file(new URL("./components/MemoryViews.svelte", import.meta.url)).text();
  expect(memoryView).toContain('event.key === "Enter"');
  expect(memoryView).toContain("event.preventDefault()");
  expect(memoryView).toContain("overflow-x-auto");
});
```

- [ ] **Step 2: Run the test to verify the layout requirement fails**

Run: `bun test src/web/app.test.ts`

Expected: FAIL because `overflow-x-auto` is not yet present.

- [ ] **Step 3: Convert the two views without changing state or actions**

Use `grid gap-3 sm:grid-cols-2 xl:grid-cols-4` for stat cards, `grid grid-cols-2 gap-2 sm:grid-cols-7` for workflow steps, and `overflow-x-auto rounded-md border border-default` around data tables. Use Tailwind group/selector variants or the `.data-table` component layer for `tbody tr:hover td`, selected cell backgrounds, and first-cell accent borders. Make the memory split `grid gap-6 lg:grid-cols-[minmax(0,1.45fr)_minmax(20rem,.8fr)]` and stack it below `lg`.

- [ ] **Step 4: Run the component test and checks**

Run: `bun test src/web/app.test.ts && bun run typecheck && bun run format`

Expected: PASS and clean checks.

- [ ] **Step 5: Commit the administration views**

```bash
git add frontend/src/web/components/AdminDashboard.svelte frontend/src/web/components/MemoryViews.svelte frontend/src/web/app.test.ts
git commit -m "refactor: migrate admin views to Tailwind"
```

### Task 5: Migrate settings editors, dialog, and notification surfaces

**Files:**
- Modify: `frontend/src/web/components/DreamingEditor.svelte`
- Modify: `frontend/src/web/components/IngestEditor.svelte`
- Modify: `frontend/src/web/components/ReleaseEditor.svelte`
- Modify: `frontend/src/web/components/ProviderEditor.svelte`
- Modify: `frontend/src/web/components/Toast.svelte`

**Interfaces:**
- Consumes: each component's existing typed props, `$state` drafts, native form bindings, native dialog lifecycle, and callback props.
- Produces: a consistent Tailwind form system with peer-driven toggles, 44px controls, editor dialog, and status toast.

- [ ] **Step 1: Write a failing source-contract test for accessibility-critical form patterns**

```ts
test("editor controls preserve native dialog and accessible toggle markup", async () => {
  const providerEditor = await Bun.file(new URL("./components/ProviderEditor.svelte", import.meta.url)).text();
  const dreamingEditor = await Bun.file(new URL("./components/DreamingEditor.svelte", import.meta.url)).text();
  expect(providerEditor).toContain("<dialog");
  expect(providerEditor).toContain("aria-labelledby");
  expect(dreamingEditor).toContain("peer");
  expect(dreamingEditor).toContain("min-h-11");
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bun test src/web/app.test.ts`

Expected: FAIL because the migrated utility classes are absent.

- [ ] **Step 3: Migrate each editor to shared Tailwind conventions**

Apply `grid gap-4` forms, `text-caption text-secondary` labels, `min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary` controls, and `focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40` focus behavior. Restructure toggle inputs only as needed for `peer sr-only` plus `peer-checked:` track/thumb utilities. Retain `fieldset`, `legend`, labels, inputs, and all current `bind:`/callback behavior. Style the dialog using the narrow `.editor-dialog` CSS layer and Tailwind layout utilities; retain `showModal`, `oncancel`, Escape, and focus restoration. Give the toast `animate-toast-in motion-reduce:animate-none` and tone-specific semantic colors.

- [ ] **Step 4: Run the form contract test and static checks**

Run: `bun test src/web/app.test.ts && bun run typecheck && bun run format`

Expected: PASS and clean checks.

- [ ] **Step 5: Commit editor and notification migration**

```bash
git add frontend/src/web/components/DreamingEditor.svelte frontend/src/web/components/IngestEditor.svelte frontend/src/web/components/ReleaseEditor.svelte frontend/src/web/components/ProviderEditor.svelte frontend/src/web/components/Toast.svelte frontend/src/web/app.test.ts
git commit -m "refactor: migrate settings editors to Tailwind"
```

### Task 6: Remove legacy styles and perform full frontend visual verification

**Files:**
- Delete: `frontend/src/web/tokens.css`
- Delete: `frontend/src/web/base.css`
- Delete: `frontend/src/web/components.css`
- Modify: `frontend/src/web/app.test.ts`

**Interfaces:**
- Consumes: all Tailwind utility migrations from Tasks 2–5.
- Produces: one authoritative Tailwind stylesheet and a distributable `frontend/dist` build.

- [ ] **Step 1: Activate the failing no-legacy-import test from Task 3 and add file-removal assertions**

```ts
test("the legacy stylesheets have been retired", async () => {
  for (const filename of ["tokens.css", "base.css", "components.css"]) {
    expect(await Bun.file(new URL(`./${filename}`, import.meta.url)).exists()).toBe(false);
  }
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bun test src/web/app.test.ts`

Expected: FAIL because the legacy stylesheets still exist.

- [ ] **Step 3: Delete the old stylesheets and complete source cleanup**

Delete the three legacy CSS files. Confirm `main.ts` imports only `fonts.css` and `app.css`; no `.svelte` file retains legacy `class:` directives for the removed CSS classes. Keep `fonts.css` only for font faces.

- [ ] **Step 4: Run frontend validation**

Run: `bun test && bun run typecheck && bun run format && bun run build`

Expected: all commands exit 0 and `frontend/dist` contains the Vite output.

- [ ] **Step 5: Inspect rendered routes and interaction states**

Run: `bun run dev -- --host 127.0.0.1`

Check `/admin`, `/admin/memory`, `/config`, `/config/dreaming`, `/config/ingest`, and `/config/release` at 375px, 768px, 1024px, and 1440px in both themes. Confirm no horizontal page overflow, table wrappers scroll rather than clip, focus rings remain visible, Space/Enter row activation remains correct, the provider dialog traps focus and restores it on close, and reduced motion suppresses entry animations.

- [ ] **Step 6: Run repository packaging checks**

Run: `uv run pytest && uv run ruff check . && uv run ruff format --check . && uv build`

Expected: test suite, Ruff checks, and package build complete successfully; `uv build` includes `frontend/dist` through Hatch's configured build command.

- [ ] **Step 7: Commit the cleanup**

```bash
git add -A frontend/src/web frontend/dist frontend/package.json frontend/bun.lock
git commit -m "refactor: complete Tailwind web console migration"
```

### Task 7: Build and package frontend assets from a clean checkout

**Files:**
- Create: `hatch_build.py`
- Modify: `pyproject.toml`
- Modify: focused packaging contract test under `tests/`

**Interfaces:**
- Consumes: the Vite build configured in `frontend/package.json` and the existing Hatch source mapping for `frontend/dist`.
- Produces: wheels and source distributions whose frontend assets are generated during `uv build`, even when `frontend/dist` is absent before the build.

- [ ] **Step 1: Add a failing packaging-contract test**

Add a focused test that asserts the package build configuration invokes a project-local Hatch build hook responsible for the frontend build. Keep the test deterministic; artifact-level verification remains part of Step 4.

- [ ] **Step 2: Verify the test fails**

Run: `uv run pytest tests/test_release_workflow.py -q`

Expected: FAIL because no Hatch build hook is configured.

- [ ] **Step 3: Generate the Vite bundle from a Hatch build hook**

Add a project-local `CustomBuildHook` that runs `mise exec bun@1.3.14 -- bun install --frozen-lockfile` and `mise exec bun@1.3.14 -- bun run build` in `frontend/`. Configure it for Hatch builds in `pyproject.toml`. Preserve the existing wheel source mapping and artifact rules; a normal `uv build` must now work without a pre-existing `frontend/dist` directory.

- [ ] **Step 4: Verify clean artifacts contain the bundle**

Build from a clean detached worktree with no ignored `frontend/dist`, then inspect the wheel and sdist contents. Confirm they both contain the Vite `index.html` and generated assets.

- [ ] **Step 5: Commit the packaging fix**

```bash
git add hatch_build.py pyproject.toml tests/
git commit -m "build: bundle frontend assets during package builds"
```

## Plan Self-Review

- Spec coverage: Task 1 covers Bun/Vite/Tailwind wiring; Task 2 covers selector dark mode, all semantic runtime token families, typography, breakpoints, animation, and reduced motion; Tasks 3–5 migrate each listed interface while preserving behavior; Task 6 removes legacy CSS, checks responsive and accessibility states, and verifies the package build coupling.
- Placeholder scan: no unresolved placeholders or unspecified test commands remain.
- Type consistency: the plan introduces no runtime TypeScript interfaces or backend APIs; it preserves existing typed props and callback signatures.
