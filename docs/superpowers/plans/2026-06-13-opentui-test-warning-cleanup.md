# OpenTUI Test Warning Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the frontend OpenTUI test suite pass without React `act(...)` warnings or OpenTUI `TerminalConsoleCache` listener warnings, without muting stderr.

**Architecture:** Add a shared OpenTUI React test harness that owns `testRender`, keyboard input, frame waiting, and renderer destruction. Migrate App, config, and admin tests from local ad hoc render loops to that harness, then add a Python warning sentinel so the warning cleanup stays enforced from the main verification suite. This slice changes test infrastructure and roadmap docs only; it should not change production TUI behavior.

**Tech Stack:** Python 3.12, pytest, TypeScript, React 19, Bun 1.3.14, OpenTUI React 0.4.0, existing `@opentui/react/test-utils`.

---

## Current Code Map

- `frontend/src/app/App.test.tsx`
  - Defines a local `setupTest()` wrapper around `testRender()`.
  - Uses custom `waitFor()` that calls `renderOnce()` directly inside `act`.
  - Does not destroy the OpenTUI renderer after each test.
- `frontend/src/config/ConfigScreen.test.tsx`
  - Defines another local `setupTest()` with keyboard helpers.
  - Does not destroy the OpenTUI renderer after each test.
- `frontend/src/admin/AdminScreen.test.tsx`
  - Defines a third local `setupTest()` with a richer keyboard helper.
  - Uses a custom parsed Escape event and local frame polling.
  - Does not destroy the OpenTUI renderer after each test.
- `@opentui/react/test-utils`
  - `testRender(node, options)` creates a React root on a test renderer.
  - Its `onDestroy()` hook unmounts React and resets `IS_REACT_ACT_ENVIRONMENT`.
  - It returns `TestRendererSetup` with `flush()`, `waitFor()`, `waitForFrame()`, `captureCharFrame()`, `resize()`, and `renderer.destroy()`.
- `docs/roadmap.md`
  - OpenTUI remaining work still includes cleaning up React `act(...)` and OpenTUI `TerminalConsoleCache` warnings without globally muting stderr.

## Baseline

Current command:

```bash
bun --cwd frontend test
```

Current result: passes, but emits 42 warning lines matching:

```text
not wrapped in act
Possible EventTarget memory leak detected
TerminalConsoleCache
```

---

## Task 1: Add A Frontend Warning Sentinel

**Files:**
- Create: `tests/test_frontend_opentui_warnings.py`

- [ ] **Step 1: Write the failing warning sentinel**

Create `tests/test_frontend_opentui_warnings.py`:

```python
from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[1]
WARNING_MARKERS = (
    "not wrapped in act",
    "Possible EventTarget memory leak detected",
    "TerminalConsoleCache",
)


def test_frontend_opentui_tests_do_not_emit_lifecycle_warnings() -> None:
    if shutil.which("bun") is None:
        pytest.skip("Bun is not installed")

    result = subprocess.run(
        ["bun", "--cwd", "frontend", "test"],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
        timeout=30,
    )

    assert result.returncode == 0, result.stdout
    warning_lines = [
        line
        for line in result.stdout.splitlines()
        if any(marker in line for marker in WARNING_MARKERS)
    ]
    assert warning_lines == []
```

- [ ] **Step 2: Run the sentinel to verify it fails for the current reason**

Run:

```bash
uv run pytest tests/test_frontend_opentui_warnings.py -q
```

Expected: FAIL. The failure should show warning lines containing `not wrapped in act`, `Possible EventTarget memory leak detected`, or `TerminalConsoleCache`. If it fails because `bun` is missing, stop and report that local validation cannot cover this slice.

- [ ] **Step 3: Commit the failing sentinel**

```bash
git add tests/test_frontend_opentui_warnings.py
git commit -m "test: detect opentui frontend warnings"
```

---

## Task 2: Add A Shared OpenTUI Test Harness

**Files:**
- Create: `frontend/src/test/opentuiHarness.tsx`
- Test indirectly through: `frontend/src/app/App.test.tsx`

- [ ] **Step 1: Add the shared harness file**

Create `frontend/src/test/opentuiHarness.tsx`:

```tsx
import React from "react";
import { act } from "react";
import type { ParsedKey } from "@opentui/core";
import type {
  TestRendererOptions,
  TestRendererSetup,
} from "@opentui/core/testing";
import { testRender } from "@opentui/react/test-utils";

type KeyOptions = {
  ctrl?: boolean;
  shift?: boolean;
};

type Harness = {
  render: (node: React.ReactNode) => Promise<void>;
  flush: () => Promise<void>;
  waitFor: (
    predicate: () => boolean | Promise<boolean>,
    maxPasses?: number,
  ) => Promise<void>;
  waitForFrame: (
    predicate: (frame: string) => boolean | Promise<boolean>,
    maxPasses?: number,
  ) => Promise<string>;
  captureCharFrame: () => string;
  mockInput: {
    press: (name: string, options?: KeyOptions) => Promise<void>;
    type: (value: string) => Promise<void>;
  };
  cleanup: () => Promise<void>;
};

const activeHarnesses = new Set<Harness>();

export async function cleanupOpenTuiHarnesses(): Promise<void> {
  const harnesses = Array.from(activeHarnesses);
  activeHarnesses.clear();
  for (const harness of harnesses) {
    await harness.cleanup();
  }
}

export function createOpenTuiHarness(
  options: TestRendererOptions,
): Harness {
  let setup: TestRendererSetup | null = null;

  const ensureSetup = () => {
    if (!setup) {
      throw new Error("OpenTUI test harness has not rendered yet");
    }
    return setup;
  };

  const render = async (node: React.ReactNode) => {
    if (setup) {
      await cleanup();
    }
    setup = await testRender(node, options);
    activeHarnesses.add(harness);
    await flush();
  };

  const flush = async () => {
    const current = ensureSetup();
    await act(async () => {
      await current.flush();
    });
  };

  const waitFor = async (
    predicate: () => boolean | Promise<boolean>,
    maxPasses = 25,
  ) => {
    const current = ensureSetup();
    await act(async () => {
      await current.waitFor(predicate, { maxPasses });
    });
  };

  const waitForFrame = async (
    predicate: (frame: string) => boolean | Promise<boolean>,
    maxPasses = 25,
  ) => {
    const current = ensureSetup();
    let frame = "";
    await act(async () => {
      frame = await current.waitForFrame(predicate, { maxPasses });
    });
    return frame;
  };

  const press = async (name: string, options: KeyOptions = {}) => {
    const current = ensureSetup();
    await act(async () => {
      if (name === "enter") {
        current.mockInput.pressEnter(options);
      } else if (name === "tab") {
        current.mockInput.pressTab(options);
      } else if (name === "backspace") {
        current.mockInput.pressBackspace();
      } else if (name === "escape") {
        const escapeKey: ParsedKey = {
          name: "escape",
          ctrl: options.ctrl ?? false,
          meta: false,
          shift: options.shift ?? false,
          option: false,
          sequence: "\x1B",
          number: false,
          raw: "\x1B",
          eventType: "press",
          source: "raw",
        };
        current.renderer.keyInput.processParsedKey(escapeKey);
      } else if (
        name === "up" ||
        name === "down" ||
        name === "left" ||
        name === "right"
      ) {
        current.mockInput.pressArrow(name, options);
      } else {
        current.mockInput.pressKey(name, options);
      }
    });
    await flush();
  };

  const type = async (value: string) => {
    const current = ensureSetup();
    await act(async () => {
      for (const key of value) {
        current.mockInput.pressKey(key);
      }
    });
    await flush();
  };

  const captureCharFrame = () => setup?.captureCharFrame() ?? "";

  const cleanup = async () => {
    if (!setup) {
      return;
    }
    const current = setup;
    setup = null;
    activeHarnesses.delete(harness);
    await act(async () => {
      current.renderer.destroy();
      await Promise.resolve();
    });
  };

  const harness: Harness = {
    render,
    flush,
    waitFor,
    waitForFrame,
    captureCharFrame,
    mockInput: { press, type },
    cleanup,
  };

  return harness;
}
```

- [ ] **Step 2: Run typecheck to catch harness type issues**

Run:

```bash
bun run --cwd frontend typecheck
```

Expected: PASS. If TypeScript reports that `pressBackspace`, `pressTab`, or `renderer.destroy()` has a different type shape, inspect `frontend/node_modules/@opentui/core/testing/test-renderer.d.ts` and `frontend/node_modules/@opentui/react/src/test-utils.d.ts`, then adjust only the harness.

- [ ] **Step 3: Do not commit yet**

This harness is unused until Task 3. Leave it staged or unstaged for the Task 3 commit.

---

## Task 3: Migrate App Tests To The Shared Harness

**Files:**
- Modify: `frontend/src/app/App.test.tsx`
- Create if not already created: `frontend/src/test/opentuiHarness.tsx`

- [ ] **Step 1: Replace App test imports and setup**

In `frontend/src/app/App.test.tsx`, remove these imports:

```tsx
import React from "react";
import { act } from "react";
import { testRender } from "@opentui/react/test-utils";
```

Add this import after the existing local imports:

```tsx
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
```

Change the Bun test import from:

```tsx
import { describe, expect, it } from "bun:test";
```

to:

```tsx
import { afterEach, describe, expect, it } from "bun:test";
```

Delete the local `setupTest()` function.

Add this helper near the top of the file:

```tsx
function setupTest() {
  return createOpenTuiHarness({ width: 160, height: 60 });
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});
```

- [ ] **Step 2: Update App test render calls**

In both App tests, replace:

```tsx
const { root, flush, captureCharFrame, waitFor } = await setupTest();
root.render(<App mode="admin" client={fakeClient()} />);
await flush();
```

with:

```tsx
const { render, captureCharFrame, waitForFrame } = setupTest();
await render(<App mode="admin" client={fakeClient()} />);
```

For the rejection test, keep the same replacement but with the existing `fakeClient(() => Promise.reject("offline"))` node.

Replace frame polling blocks like:

```tsx
await waitFor(async () => {
  const frame = captureCharFrame();
  return frame.includes("Hieronymus Admin");
});
```

with:

```tsx
await waitForFrame((frame) => frame.includes("Hieronymus Admin"));
```

For the `"offline"` test, use:

```tsx
await waitForFrame((frame) => frame.includes("offline"));
```

- [ ] **Step 3: Run App tests**

Run:

```bash
bun --cwd frontend test src/app/App.test.tsx
```

Expected: PASS with no `not wrapped in act`, `TerminalConsoleCache`, or `Possible EventTarget memory leak detected` lines.

- [ ] **Step 4: Commit App harness migration**

```bash
git add frontend/src/test/opentuiHarness.tsx frontend/src/app/App.test.tsx
git commit -m "test: add opentui react harness"
```

---

## Task 4: Migrate ConfigScreen Tests To The Shared Harness

**Files:**
- Modify: `frontend/src/config/ConfigScreen.test.tsx`
- Modify: `frontend/src/test/opentuiHarness.tsx` only if a missing helper is discovered

- [ ] **Step 1: Replace ConfigScreen test imports and setup**

In `frontend/src/config/ConfigScreen.test.tsx`, remove these imports:

```tsx
import React from "react";
import { act } from "react";
import { testRender } from "@opentui/react/test-utils";
```

Change the Bun test import from:

```tsx
import { describe, expect, it } from "bun:test";
```

to:

```tsx
import { afterEach, describe, expect, it } from "bun:test";
```

Add:

```tsx
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
```

Delete the local `setupTest()` function.

Add:

```tsx
function setupTest() {
  return createOpenTuiHarness({ width: 120, height: 36 });
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});
```

- [ ] **Step 2: Update ConfigScreen render calls**

For each test, replace this pattern:

```tsx
const { root, mockInput, flush, captureCharFrame, waitFor } =
  await setupTest();

root.render(<ConfigScreen initial={payload()} client={client} />);
await flush();
```

with:

```tsx
const { render, mockInput, captureCharFrame, waitForFrame } = setupTest();

await render(<ConfigScreen initial={payload()} client={client} />);
```

Use `waitForFrame((frame) => frame.includes("..."))` wherever the test currently calls `waitFor()` only to inspect rendered text. Keep `waitFor()` only for predicates that inspect non-frame state such as call counts.

- [ ] **Step 3: Preserve existing keyboard assertions**

Do not change expected UI behavior. Existing calls such as:

```tsx
await mockInput.press("tab");
await mockInput.press("down");
await mockInput.type("gpt-4.1");
```

should continue to work through the shared harness.

- [ ] **Step 4: Run ConfigScreen tests**

Run:

```bash
bun --cwd frontend test src/config/ConfigScreen.test.tsx
```

Expected: PASS with no `not wrapped in act`, `TerminalConsoleCache`, or `Possible EventTarget memory leak detected` lines.

- [ ] **Step 5: Commit ConfigScreen migration**

```bash
git add frontend/src/config/ConfigScreen.test.tsx frontend/src/test/opentuiHarness.tsx
git commit -m "test: use opentui harness for config screen"
```

---

## Task 5: Migrate AdminScreen Tests To The Shared Harness

**Files:**
- Modify: `frontend/src/admin/AdminScreen.test.tsx`
- Modify: `frontend/src/test/opentuiHarness.tsx` only if a missing helper is discovered

- [ ] **Step 1: Replace AdminScreen test imports and setup**

In `frontend/src/admin/AdminScreen.test.tsx`, remove these imports:

```tsx
import React from "react";
import { act } from "react";
import type { ParsedKey } from "@opentui/core";
import { testRender } from "@opentui/react/test-utils";
```

Change:

```tsx
import { describe, expect, it } from "bun:test";
```

to:

```tsx
import { afterEach, describe, expect, it } from "bun:test";
```

Add:

```tsx
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
```

Delete the local `setupTest()` function.

Add:

```tsx
function setupTest() {
  return createOpenTuiHarness({ width: 160, height: 60 });
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});
```

- [ ] **Step 2: Update AdminScreen render calls**

For each test, replace:

```tsx
const { root, mockInput, flush, captureCharFrame, waitFor } =
  await setupTest();

root.render(<AdminScreen initial={bootstrap()} client={client} />);
await flush();
```

with:

```tsx
const { render, mockInput, captureCharFrame, waitFor, waitForFrame } =
  setupTest();

await render(<AdminScreen initial={bootstrap()} client={client} />);
```

For tests that do not use `waitFor`, omit it from destructuring. For tests that only wait for text, use `waitForFrame((frame) => frame.includes("..."))`.

- [ ] **Step 3: Keep existing input semantics**

The shared harness must preserve all current Admin test input forms:

```tsx
await mockInput.type("?");
await mockInput.type("+");
await mockInput.press("p", { ctrl: true });
await mockInput.press("enter");
await mockInput.press("escape");
await mockInput.press("tab");
await mockInput.press("down");
```

Do not change production `AdminScreen.tsx` behavior in this task.

- [ ] **Step 4: Run AdminScreen tests**

Run:

```bash
bun --cwd frontend test src/admin/AdminScreen.test.tsx
```

Expected: PASS with no `not wrapped in act`, `TerminalConsoleCache`, or `Possible EventTarget memory leak detected` lines.

- [ ] **Step 5: Commit AdminScreen migration**

```bash
git add frontend/src/admin/AdminScreen.test.tsx frontend/src/test/opentuiHarness.tsx
git commit -m "test: use opentui harness for admin screen"
```

---

## Task 6: Make The Warning Sentinel Pass

**Files:**
- Modify: `frontend/src/test/opentuiHarness.tsx` if cleanup still leaks warnings
- Modify: migrated test files only if they still use stale polling patterns

- [ ] **Step 1: Run the warning sentinel**

Run:

```bash
uv run pytest tests/test_frontend_opentui_warnings.py -q
```

Expected after Tasks 3-5: PASS.

- [ ] **Step 2: If React act warnings remain, fix stale direct polling**

Search:

```bash
rg -n "renderOnce\\(|current\\.flush\\(|testRender\\(|await setupTest\\(|root\\.render" frontend/src
```

Expected after migration: no matches in `frontend/src/app/*.test.tsx`, `frontend/src/config/*.test.tsx`, or `frontend/src/admin/*.test.tsx` except imports/usages inside `frontend/src/test/opentuiHarness.tsx`.

If a test still uses direct `renderOnce()` or local `testRender()`, migrate that test to the shared harness before continuing.

- [ ] **Step 3: If TerminalConsoleCache warnings remain, enforce cleanup**

Inspect `frontend/src/test/opentuiHarness.tsx`. The cleanup path must call:

```tsx
current.renderer.destroy();
```

inside `act`, and each test file must have:

```tsx
afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});
```

If a test creates multiple harnesses, `cleanupOpenTuiHarnesses()` must destroy all of them through the `activeHarnesses` registry.

- [ ] **Step 4: Commit sentinel pass fixes**

If Task 6 required code changes:

```bash
git add frontend/src/test/opentuiHarness.tsx frontend/src/app/App.test.tsx frontend/src/config/ConfigScreen.test.tsx frontend/src/admin/AdminScreen.test.tsx
git commit -m "test: silence opentui lifecycle warnings"
```

If no code changes were required, skip this commit.

---

## Task 7: Register The Completed Roadmap Work

**Files:**
- Modify: `docs/roadmap.md`

- [ ] **Step 1: Update the OpenTUI completed baseline**

In `docs/roadmap.md`, add this bullet to the OpenTUI Management App `Completed baseline:` list:

```md
- Frontend OpenTUI tests use a shared renderer harness that flushes React updates
  through `act(...)`, destroys renderers after each test, and keeps `bun test`
  free of React lifecycle and `TerminalConsoleCache` listener warnings.
```

- [ ] **Step 2: Remove the completed remaining-work bullet**

In `docs/roadmap.md`, remove this bullet from OpenTUI `Remaining work:`:

```md
- Clean up React `act(...)` warnings and OpenTUI `TerminalConsoleCache`
  listener warnings in `bun test` without globally muting stderr.
```

Leave unrelated remaining work in place.

- [ ] **Step 3: Run docs diff check**

Run:

```bash
git diff --check docs/roadmap.md
```

Expected: PASS with no output.

- [ ] **Step 4: Commit roadmap update**

```bash
git add docs/roadmap.md
git commit -m "docs: register opentui warning cleanup"
```

---

## Task 8: Final Verification

**Files:**
- No source edits expected.

- [ ] **Step 1: Run targeted verification**

```bash
uv run pytest tests/test_frontend_opentui_warnings.py tests/test_tui_bridge_admin.py -q
bun --cwd frontend test src/app/App.test.tsx src/config/ConfigScreen.test.tsx src/admin/AdminScreen.test.tsx
```

Expected: PASS. The Bun output must not contain `not wrapped in act`, `Possible EventTarget memory leak detected`, or `TerminalConsoleCache`.

- [ ] **Step 2: Run full verification**

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
bun run --cwd frontend format
bun run --cwd frontend typecheck
bun --cwd frontend test
bun run --cwd frontend build
git diff --check
```

Expected: all commands pass. The full `bun --cwd frontend test` output must not contain the warning markers listed in Task 1.

- [ ] **Step 3: Push and open PR**

```bash
git status --short
git push -u origin plan/opentui-test-warning-cleanup
gh pr create --base main --head plan/opentui-test-warning-cleanup --title "Clean up OpenTUI test warnings" --body "$(cat <<'EOF'
## Summary
- Add a shared OpenTUI React test harness with act-aware input, frame waiting, and renderer cleanup.
- Migrate App, config, and admin frontend tests to the shared harness.
- Add a warning sentinel and update the roadmap once `bun test` is clean.

## Test Plan
- [ ] `uv run pytest`
- [ ] `uv run ruff check .`
- [ ] `uv run ruff format --check .`
- [ ] `bun run --cwd frontend format`
- [ ] `bun run --cwd frontend typecheck`
- [ ] `bun --cwd frontend test`
- [ ] `bun run --cwd frontend build`
- [ ] `git diff --check`
EOF
)"
```

Expected: GitHub opens a PR from `plan/opentui-test-warning-cleanup` into `main`.

---

## Self-Review Notes

- Spec coverage: this plan covers the roadmap warning-cleanup item directly and includes docs registration after the sentinel passes.
- Placeholder scan: no unfinished placeholder markers or undefined follow-up steps are left in the plan.
- Type consistency: the shared harness exposes `render`, `flush`, `waitFor`, `waitForFrame`, `captureCharFrame`, `mockInput.press`, `mockInput.type`, and `cleanup`; later tasks use only those names.
- Scope boundary: this plan intentionally avoids responsive layout, markdown rendering, scrollbars, and production TUI behavior changes.
