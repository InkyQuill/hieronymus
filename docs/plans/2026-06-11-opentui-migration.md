# OpenTUI Migration Implementation Plan

> **For Antigravity:** REQUIRED SUB-SKILL: Load executing-plans to implement this plan task-by-task.

**Goal:** Migrate Hieronymus TUI (config and admin tools) from React/Ink running on Node to React/OpenTUI running on Bun, upgrading design aesthetics, components, and adding rich interactive unicode loader animations.

**Architecture:** Port React JSX components from Ink to OpenTUI, using Bun as the runtime environment instead of Node. Replace custom input/scrolling implementations with OpenTUI's native input/scrollbox features, integrate `unicode-animations` for animated states, and update python subprocess launcher, installer checks & doctor verification.

**Tech Stack:** Python 3.12, Bun >=1.3, TypeScript, React 19, @opentui/core, @opentui/react, unicode-animations.

---

### Task 1: Package and TS Configuration Update

**Files:**
- Modify: `frontend/package.json`
- Modify: `frontend/tsconfig.json`

**Step 1: Write the package.json modifications**
Replace dependencies: removing `ink`, `ink-testing-library`, adding `@opentui/core`, `@opentui/react`, `unicode-animations`, and `@types/bun` as a dev dependency. Set build/test scripts for bun.

**Step 2: Write tsconfig.json modifications**
Update `jsxImportSource` to `@opentui/react`, `moduleResolution` to `bundler`, and specify `bun-types` in `types`.

**Step 3: Run package installation**
Run: `bun install` inside `frontend/`
Expected: Installs packages without error.

**Step 4: Commit**
```bash
git add frontend/package.json frontend/tsconfig.json
git commit -m "chore: update frontend dependencies and tsconfig for OpenTUI"
```

---

### Task 2: Port CLI, Doctor, and Installation Checks

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `src/hieronymus/doctor.py`
- Modify: `install.sh`
- Modify: `tests/test_cli_ink_tui.py` (rename to `tests/test_cli_opentui.py`)
- Modify: `tests/test_doctor_ink_runtime.py` (rename to `tests/test_doctor_opentui_runtime.py`)

**Step 1: Update CLI launcher**
Modify `src/hieronymus/cli.py` to run `bun` instead of `node` in `_launch_ink` and rename function to `_launch_opentui`.

**Step 2: Update Doctor verification**
Modify `src/hieronymus/doctor.py` to check for `bun` (and version >=1.3) instead of `node`/`pnpm`.

**Step 3: Update install.sh**
Add shell script warnings that check if `python3` is >= 3.12 and `bun` is >= 1.3, outputting helpful installation guides/suggestions if missing.

**Step 4: Rename and update tests**
Rename the python test files and update their assertions to mock and verify `bun` runtime checks.

**Step 5: Run tests to verify**
Run: `uv run pytest`
Expected: All Python tests pass.

**Step 6: Commit**
```bash
git add src/hieronymus/cli.py src/hieronymus/doctor.py install.sh tests/test_cli_opentui.py tests/test_doctor_opentui_runtime.py
git commit -m "feat: migrate backend launcher, installer checks, and doctor to Bun"
```

---

### Task 3: Port Entrypoint, Spinner Component, and App Layout

**Files:**
- Modify: `frontend/src/main.tsx`
- Modify: `frontend/src/app/App.tsx`
- Create: `frontend/src/ui/Spinner.tsx`

**Step 1: Port frontend entrypoint**
Modify `frontend/src/main.tsx` to use OpenTUI's `createCliRenderer` and `createRoot` instead of Ink's `render`.

**Step 2: Create reusable Spinner component**
Create `frontend/src/ui/Spinner.tsx` which imports `spinners` from `unicode-animations` and implements a simple frame interval loop to render loading icons (using e.g. `helix` or `pulse`).

**Step 3: Port main App wrapper**
Modify `frontend/src/app/App.tsx` to use lowercase OpenTUI JSX primitives (`box`, `text`) instead of capital Ink ones. Use `<Spinner>` for the initial bootstrapping screens.

**Step 4: Check build**
Run: `bun run build` inside `frontend/`
Expected: Compiles entrypoint & App to `dist/main.js` using `bun build`.

**Step 5: Commit**
```bash
git add frontend/src/main.tsx frontend/src/app/App.tsx frontend/src/ui/Spinner.tsx
git commit -m "refactor: port entrypoint, create unicode spinner, and update App root"
```

---

### Task 4: Port Config Panel and ConfigScreen

**Files:**
- Modify: `frontend/src/config/ConfigScreen.tsx`
- Modify: `frontend/src/config/ConfigForm.tsx`
- Modify: `frontend/src/config/ProviderSelector.tsx`
- Modify: `frontend/src/ui/StatusLine.tsx`

**Step 1: Replace custom input listeners**
Rewrite form controls in `ConfigForm.tsx` to use native `<input>` components. Remove raw stdin data handlers and replace with `useKeyboard` for navigation.

**Step 2: Add busy animations**
Add `<Spinner>` to the `StatusLine` component when the operation is busy or pending.

**Step 3: Upgrade visual styling**
Convert layouts to flexbox columns and rows with proper spacing, borders (`borderStyle="round"`), and active focus styling.

**Step 4: Run build and type check**
Run: `bun run build`
Expected: Successful build.

**Step 5: Commit**
```bash
git add frontend/src/config/ConfigScreen.tsx frontend/src/config/ConfigForm.tsx frontend/src/config/ProviderSelector.tsx frontend/src/ui/StatusLine.tsx
git commit -m "refactor: migrate configuration TUI screens to OpenTUI with busy spinner"
```

---

### Task 5: Port Admin Panel and AdminScreen

**Files:**
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Modify: `frontend/src/admin/AdminTable.tsx`
- Modify: `frontend/src/admin/DetailPane.tsx`
- Modify: `frontend/src/admin/dialogs.tsx`

**Step 1: Implement Scrollable Tables and Inspector**
Use OpenTUI's native `<scrollbox>` for `AdminTable` and `DetailPane` to support mouse scrolling.

**Step 2: Add spinner feedback in job status panels**
Add `<Spinner name="pulse">` to status panel execution blocks (e.g. dreaming progress, draining short-term memories) to denote active processing.

**Step 3: Implement Code/Diff Highlighting**
Integrate OpenTUI's native `<code>` and `<diff>` components in the inspector when viewing code or modifications.

**Step 4: Port dialog overlays**
Update dialog boxes (`dialogs.tsx`) to use absolute positioning layouts for centering.

**Step 5: Run typecheck and compile**
Run: `bun run build`
Expected: Successful compile to `frontend/dist/main.js`.

**Step 6: Commit**
```bash
git add frontend/src/admin/AdminScreen.tsx frontend/src/admin/AdminTable.tsx frontend/src/admin/DetailPane.tsx frontend/src/admin/dialogs.tsx
git commit -m "refactor: migrate admin TUI screens to OpenTUI with animated job statuses"
```

---

### Task 6: Shared UI Port and End-to-End Verification

**Files:**
- Modify: `frontend/src/ui/TextInput.tsx`
- Modify: `frontend/src/ui/FocusableList.tsx`
- Modify: `frontend/src/ui/KeyHelp.tsx`

**Step 1: Simplify Shared UI components**
Port or delete `TextInput.tsx` (redundant with native `<input>`), convert remaining UI files to OpenTUI.

**Step 2: Complete End-to-End Test Verification**
Run: `uv run pytest` and verify TUI works manually.

**Step 3: Commit**
```bash
git add frontend/src/ui/
git commit -m "refactor: port shared TUI elements and verify migration"
```
