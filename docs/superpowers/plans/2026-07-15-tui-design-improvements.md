# TUI Design Improvements Implementation Plan

**Status:** Complete (2026-07-15). All tasks and final verification passed; review-driven deviations and hardening are recorded in the branch history.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Implement the fixes and improvements from `docs/tui-design-improvements.md` for the OpenTUI admin/config screens: a mislabeled-panel bug fix, progress gauges, aligned table columns, a non-destructive dialog overlay, deduplicated dialog focus logic, visible scrollbars, a header service indicator, clearer disabled-command styling, and a semantic color theme applied across the touched files.

**Architecture:** Each task is a self-contained change to the `frontend/src` OpenTUI React app (Bun + `@opentui/react`), following existing patterns: `bun:test` with the `createOpenTuiHarness` test-renderer wrapper (`frontend/src/test/opentuiHarness.tsx`), Zod-typed RPC payloads (`frontend/src/rpc/schema.ts`), and the existing `AdminScreen`/`ConfigScreen`/`dialogs` component structure. Tasks 1-8 ship working, independently testable UI improvements using the codebase's existing inline color literals (no new dependency between tasks). Task 9 is a pure consistency refactor that introduces a semantic color/theme module and retrofits it across every file touched by Tasks 1-8 plus the remaining files named in the spec, done last so it only has to sweep each file once.

**Tech Stack:** Bun, TypeScript, React 19, `@opentui/core` / `@opentui/react` 0.4.0, `bun:test`, Zod (already in use; no new dependencies).

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `frontend/src/admin/AdminScreen.tsx` | Modify | Panel header labels (Task 1), `StatusPanels` gauges (Task 2), dialog overlay mount point (Task 4), `Header` service indicator (Task 7), theme application (Task 9) |
| `frontend/src/admin/AdminScreen.test.tsx` | Modify | New/updated assertions for Tasks 1, 2, 4, 7 |
| `frontend/src/admin/AdminTable.tsx` | Modify | Column alignment (Task 3), scrollbar visibility (Task 6), theme application (Task 9) |
| `frontend/src/admin/AdminTable.test.tsx` | Create | Column alignment, truncation, selection marker, scrollbar tests |
| `frontend/src/admin/DetailPane.tsx` | Modify | Scrollbar visibility (Task 6), theme application (Task 9) |
| `frontend/src/admin/DetailPane.test.tsx` | Create | Scrollbar visibility test |
| `frontend/src/admin/CommandPalette.tsx` | Modify | Disabled-command styling (Task 8), theme application (Task 9) |
| `frontend/src/admin/CommandPalette.test.tsx` | Create | Disabled-marker and hint-line tests |
| `frontend/src/admin/dialogs.tsx` | Modify | Overlay centering/sizing (Task 4), shared field-focus hook (Task 5), theme application (Task 9) |
| `frontend/src/admin/HelpOverlay.tsx` | Modify | Theme application (Task 9) |
| `frontend/src/config/ConfigScreen.tsx` | Modify | Theme application (Task 9) |
| `frontend/src/config/ConfigForm.tsx` | Modify | Theme application (Task 9) |
| `frontend/src/ui/StatusLine.tsx` | Modify | Theme application (Task 9) |
| `frontend/src/ui/Gauge.tsx` | Create | Reusable `label [████░░░░] value/max` gauge bar (Task 2) |
| `frontend/src/ui/Gauge.test.tsx` | Create | Gauge fill/rounding/fallback tests |
| `frontend/src/ui/useFieldFocus.ts` | Create | Shared clamped focus-index hook for dialogs (Task 5) |
| `frontend/src/ui/useFieldFocus.test.tsx` | Create | Hook behavior tests via a probe component |
| `frontend/src/ui/theme.ts` | Create | Frozen semantic color constants (Task 9) |
| `frontend/src/ui/theme.test.ts` | Create | Theme constant/frozen-object tests |

---

## Task 1: Fix mislabeled panel header in compact/narrow layout

**Files:**
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Test: `frontend/src/admin/AdminScreen.test.tsx`

**Problem:** In the compact/narrow layout branch of `AdminScreen`, the panel header text is hardcoded to `"Detail Inspector"` whenever the help overlay or command palette is open, even if `activePanel` is `"views"` or `"table"`. The default `activePanel` on mount is `"views"`, so opening the command palette immediately after mount (no `Tab` needed) shows a "Detail Inspector" header while displaying the Views/Table overlay context.

- [x] **Step 1: Write the failing test**

Add this test inside the `describe("AdminScreen", ...)` block in `frontend/src/admin/AdminScreen.test.tsx`, right after the `"renders help in compact admin layout"` test (after line 289):

```tsx
  it("labels the compact overlay pane after the active panel, not always Detail Inspector", async () => {
    const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);
    await mockInput.press("p", { ctrl: true });

    const output = await waitForFrame((frame) =>
      frame.includes("Command Palette"),
    );
    expect(output).toContain("Views");
    expect(output).not.toContain("Detail Inspector");
  });
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx -t "labels the compact overlay pane"`
Expected: FAIL — `expect(output).not.toContain("Detail Inspector")` fails because the frame contains `"Detail Inspector"`.

- [x] **Step 3: Write minimal implementation**

In `frontend/src/admin/AdminScreen.tsx`, add a helper function near the other free functions at the bottom of the file (e.g. right after `serviceStatus`):

```tsx
function compactPanelLabel(
  activePanel: "views" | "table" | "detail",
  view: string,
): string {
  if (activePanel === "views") {
    return "Views";
  }
  if (activePanel === "table") {
    return view;
  }
  return "Detail Inspector";
}
```

Then, in the compact-layout render branch (`if (layout.kind !== "wide") { ... }`), replace the hardcoded `"Detail Inspector"` labels in the `helpOpen` and `commandsOpen` branches with the computed label. The branch currently reads:

```tsx
          {helpOpen ? (
            <>
              <text fg="cyan">Detail Inspector</text>
              <HelpOverlay
                commands={initial.command_options}
                view={snapshot.view}
                width={contentWidth}
              />
            </>
          ) : commandsOpen ? (
            <>
              <text fg="cyan">Detail Inspector</text>
              <CommandPalette
                commands={paletteCommands}
                selectedIndex={clampCommandIndex(selectedCommandIndex)}
                width={contentWidth}
              />
            </>
          ) : activePanel === "views" ? (
```

Change it to:

```tsx
          {helpOpen ? (
            <>
              <text fg="cyan">
                {compactPanelLabel(activePanel, snapshot.view)}
              </text>
              <HelpOverlay
                commands={initial.command_options}
                view={snapshot.view}
                width={contentWidth}
              />
            </>
          ) : commandsOpen ? (
            <>
              <text fg="cyan">
                {compactPanelLabel(activePanel, snapshot.view)}
              </text>
              <CommandPalette
                commands={paletteCommands}
                selectedIndex={clampCommandIndex(selectedCommandIndex)}
                width={contentWidth}
              />
            </>
          ) : activePanel === "views" ? (
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx -t "labels the compact overlay pane"`
Expected: PASS

- [x] **Step 5: Run the full AdminScreen test file to check for regressions**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx`
Expected: All tests pass, including `"cycles compact admin panes with tab"`, `"renders command palette in compact admin layout"`, and `"renders help in compact admin layout"`.

- [x] **Step 6: Commit**

```bash
git add frontend/src/admin/AdminScreen.tsx frontend/src/admin/AdminScreen.test.tsx
git commit -m "fix: label compact admin overlay pane after the active panel"
```

---

## Task 2: Add progress gauges to short-term/drain/dream status

**Files:**
- Create: `frontend/src/ui/Gauge.tsx`
- Create: `frontend/src/ui/Gauge.test.tsx`
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Modify: `frontend/src/admin/AdminScreen.test.tsx`

**Problem:** `StatusPanels` in `AdminScreen.tsx` renders short-term pending/drain and dream progress as long concatenated text strings. There is no at-a-glance visual indicator of how full/complete these values are.

- [x] **Step 1: Write the failing Gauge unit tests**

Create `frontend/src/ui/Gauge.test.tsx`:

```tsx
import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { Gauge } from "./Gauge.js";

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("Gauge", () => {
  it("renders a half-filled bar at 50%", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Gauge label="Short-term" value={5} max={10} barWidth={10} />);

    const output = await waitForFrame((frame) => frame.includes("Short-term"));
    expect(output).toContain("Short-term [█████░░░░░] 5/10");
  });

  it("rounds a partial fill to the nearest block", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Gauge label="Drain" value={1} max={3} barWidth={9} />);

    const output = await waitForFrame((frame) => frame.includes("Drain"));
    expect(output).toContain("Drain [███░░░░░░] 1/3");
  });

  it("renders a full bar at 100%", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Gauge label="Dream" value={4} max={4} barWidth={4} />);

    const output = await waitForFrame((frame) => frame.includes("Dream"));
    expect(output).toContain("Dream [████] 4/4");
  });

  it("falls back to a plain fraction when max is zero", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Gauge label="Queue" value={0} max={0} barWidth={10} />);

    const output = await waitForFrame((frame) => frame.includes("Queue"));
    expect(output).toContain("Queue 0/0");
    expect(output).not.toContain("[");
  });

  it("falls back to a plain fraction when the width is too small for a bar", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(
      <Gauge label="Q" value={2} max={4} barWidth={10} width={10} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Q "));
    expect(output).toContain("Q 2/4");
    expect(output).not.toContain("[");
  });
});
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd frontend && bun test src/ui/Gauge.test.tsx`
Expected: FAIL — `Cannot find module './Gauge.js'` (or equivalent resolution error), since `Gauge.tsx` does not exist yet.

- [x] **Step 3: Write minimal implementation**

Create `frontend/src/ui/Gauge.tsx`:

```tsx
import React from "react";

const FILLED_BLOCK = "█";
const EMPTY_BLOCK = "░";
const DEFAULT_BAR_WIDTH = 10;
const MIN_BAR_RENDER_WIDTH = 16;

export type GaugeProps = {
  label: string;
  value: number;
  max: number;
  width?: number;
  barWidth?: number;
  fg?: string;
};

export function Gauge({
  label,
  value,
  max,
  width,
  barWidth = DEFAULT_BAR_WIDTH,
  fg,
}: GaugeProps) {
  const safeMax = Math.max(max, 0);
  const hasBarSpace = width === undefined || width >= MIN_BAR_RENDER_WIDTH;

  if (safeMax === 0 || !hasBarSpace) {
    return (
      <text fg={fg}>
        {label} {value}/{safeMax}
      </text>
    );
  }

  const ratio = Math.min(Math.max(value / safeMax, 0), 1);
  const filled = Math.round(ratio * barWidth);
  const bar =
    FILLED_BLOCK.repeat(filled) + EMPTY_BLOCK.repeat(barWidth - filled);

  return (
    <text fg={fg}>
      {label} [{bar}] {value}/{safeMax}
    </text>
  );
}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd frontend && bun test src/ui/Gauge.test.tsx`
Expected: PASS (5 tests)

- [x] **Step 5: Commit the Gauge component**

```bash
git add frontend/src/ui/Gauge.tsx frontend/src/ui/Gauge.test.tsx
git commit -m "feat: add Gauge bar component"
```

- [x] **Step 6: Write the failing AdminScreen integration test**

In `frontend/src/admin/AdminScreen.test.tsx`, add one assertion to the existing `"renders views, stats, table row, and detail"` test (after the line `expect(output).toContain("Dream DISABLED");`, around line 404):

```tsx
    expect(output).toContain("Short-term [░░░░░░░░░░] 0/200");
```

Then extend the existing `"reinforces the selected crystal and refreshes from nested snapshot"` test (ends around line 1041) by adding three assertions right after the existing `expect(output).toContain("progress 75%");` line:

```tsx
    expect(output).toContain("Short-term [░░░░░░░░░░] 3/200");
    expect(output).toContain("Drain [███████░░░] 7/10");
    expect(output).toContain("Dream [████████░░] 75/100");
```

- [x] **Step 7: Run test to verify it fails**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx -t "renders views, stats, table row, and detail"`
Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx -t "reinforces the selected crystal"`
Expected: Both FAIL — the gauge text is not present yet.

- [x] **Step 8: Write minimal implementation**

In `frontend/src/admin/AdminScreen.tsx`, add the import:

```tsx
import { Gauge } from "../ui/Gauge.js";
```

Replace the `StatusPanels` function body with:

```tsx
function StatusPanels({
  shortTermStatus,
  dreamStatus,
}: {
  shortTermStatus: AdminShortTermStatus;
  dreamStatus: AdminDreamStatus;
}) {
  const drain = shortTermStatus.drain_in_progress
    ? `  drain ${shortTermStatus.drain_completed}/${shortTermStatus.drain_total} (${formatPercent(
        shortTermStatus.drain_progress,
      )}) remaining ${shortTermStatus.drain_remaining}`
    : "";
  const dream = [
    `Dream ${dreamStatus.state}`,
    dreamStatus.current_phase ? `phase ${dreamStatus.current_phase}` : "",
    dreamStatus.progress > 0
      ? `progress ${formatPercent(dreamStatus.progress)}`
      : "",
    dreamStatus.run_id === null ? "" : `run ${dreamStatus.run_id}`,
    dreamStatus.cycle_id === null ? "" : `cycle ${dreamStatus.cycle_id}`,
  ]
    .filter(Boolean)
    .join("  ");

  return (
    <box flexDirection="column" marginTop={1}>
      <box flexDirection="row">
        {shortTermStatus.drain_in_progress && (
          <box marginRight={1}>
            <Spinner />
          </box>
        )}
        <text>
          Short-term pending {shortTermStatus.pending_count} / min{" "}
          {shortTermStatus.min_pending_short_term_memories} / max{" "}
          {shortTermStatus.max_pending_short_term_memories}
          {shortTermStatus.urgent ? " urgent" : ""}
          {drain}
        </text>
      </box>
      <Gauge
        label="Short-term"
        value={shortTermStatus.pending_count}
        max={shortTermStatus.max_pending_short_term_memories}
        fg={shortTermStatus.urgent ? "yellow" : "cyan"}
      />
      {shortTermStatus.drain_in_progress ? (
        <Gauge
          label="Drain"
          value={shortTermStatus.drain_completed}
          max={shortTermStatus.drain_total}
          fg="cyan"
        />
      ) : null}
      <box flexDirection="row" marginTop={0}>
        {dreamStatus.state !== "idle" && dreamStatus.state !== "DISABLED" && (
          <box marginRight={1}>
            <Spinner />
          </box>
        )}
        <text>{dream}</text>
      </box>
      {dreamStatus.progress > 0 ? (
        <Gauge
          label="Dream"
          value={Math.round(dreamStatus.progress * 100)}
          max={100}
          fg="cyan"
        />
      ) : null}
    </box>
  );
}
```

- [x] **Step 9: Run test to verify it passes**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx`
Expected: PASS — all tests including the two extended in Step 6.

- [x] **Step 10: Commit**

```bash
git add frontend/src/admin/AdminScreen.tsx frontend/src/admin/AdminScreen.test.tsx
git commit -m "feat: render short-term/drain/dream status as gauge bars"
```

---

## Task 3: Align AdminTable rows into fixed-width columns

**Files:**
- Modify: `frontend/src/admin/AdminTable.tsx`
- Create: `frontend/src/admin/AdminTable.test.tsx`

**Problem:** `AdminTable` renders `{label} [{status}] {quality_label}` as one concatenated string per row with no column alignment, making status/quality hard to scan across rows.

- [x] **Step 1: Write the failing test**

Create `frontend/src/admin/AdminTable.test.tsx`:

```tsx
import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
import type { AdminRow } from "../rpc/schema.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { AdminTable } from "./AdminTable.js";

function row(overrides: Partial<AdminRow> = {}): AdminRow {
  return {
    id: 1,
    kind: "concept",
    label: "Guild Ledger",
    status: "active",
    scope: "only-sense-online",
    language_pair: "ja -> ru",
    quality_label: "high",
    tags: [],
    ...overrides,
  };
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("AdminTable", () => {
  it("aligns the status column at the same position across rows regardless of label length", async () => {
    const rows = [
      row({ id: 1, label: "Short", status: "active", quality_label: "high" }),
      row({
        id: 2,
        label: "A much longer label value",
        status: "pending",
        quality_label: "medium",
      }),
    ];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 80,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={60} height={6} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Short"));
    const lines = output.split("\n").map((line) => line.trimEnd());
    const shortLine = lines.find((line) => line.includes("Short"));
    const longLine = lines.find((line) => line.includes("A much longer"));
    expect(shortLine).toBeDefined();
    expect(longLine).toBeDefined();
    const statusColumnStart = shortLine!.indexOf("active");
    expect(longLine!.indexOf("pending")).toBe(statusColumnStart);
  });

  it("truncates a label that exceeds the column width with an ellipsis", async () => {
    const rows = [
      row({
        id: 1,
        label: "A".repeat(80),
        status: "active",
        quality_label: "high",
      }),
    ];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={40} height={4} />,
    );

    const output = await waitForFrame((frame) => frame.includes("…"));
    expect(output).toContain("…");
    expect(output).not.toContain("A".repeat(80));
  });

  it("keeps the selection marker for the highlighted row", async () => {
    const rows = [row({ id: 1, label: "Guild Ledger" })];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={1} focused width={40} height={4} />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Guild Ledger"),
    );
    expect(output).toContain("> Guild Ledger");
  });
});
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd frontend && bun test src/admin/AdminTable.test.tsx`
Expected: FAIL — the first test fails because `indexOf("pending")` does not equal `indexOf("active")` under the current concatenated, unaligned format.

- [x] **Step 3: Write minimal implementation**

Replace `frontend/src/admin/AdminTable.tsx` with:

```tsx
import React from "react";
import type { AdminRow } from "../rpc/schema.js";

const MARKER_WIDTH = 2;
const COLUMN_GAP = 1;
const STATUS_COLUMN_WIDTH = 10;
const QUALITY_COLUMN_WIDTH = 10;
const MIN_LABEL_COLUMN_WIDTH = 4;
const ELLIPSIS = "…";

export function AdminTable({
  rows,
  selectedId,
  focused = true,
  width = 48,
  height = 18,
}: {
  rows: AdminRow[];
  selectedId: string | number | null;
  focused?: boolean;
  width?: number;
  height?: number;
}) {
  const labelWidth = Math.max(
    MIN_LABEL_COLUMN_WIDTH,
    width -
      MARKER_WIDTH -
      COLUMN_GAP * 2 -
      STATUS_COLUMN_WIDTH -
      QUALITY_COLUMN_WIDTH,
  );

  return (
    <scrollbox flexDirection="column" width={width} height={height}>
      {rows.map((row) => (
        <text
          key={String(row.id)}
          fg={row.id === selectedId ? (focused ? "cyan" : "white") : undefined}
        >
          {row.id === selectedId ? "> " : "  "}
          {padColumn(row.label, labelWidth)} {padColumn(
            row.status,
            STATUS_COLUMN_WIDTH,
          )} {padColumn(row.quality_label, QUALITY_COLUMN_WIDTH)}
        </text>
      ))}
    </scrollbox>
  );
}

function padColumn(value: string, columnWidth: number): string {
  if (value.length > columnWidth) {
    return columnWidth <= 1
      ? value.slice(0, columnWidth)
      : `${value.slice(0, columnWidth - 1)}${ELLIPSIS}`;
  }
  return value.padEnd(columnWidth, " ");
}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd frontend && bun test src/admin/AdminTable.test.tsx`
Expected: PASS (3 tests)

- [x] **Step 5: Run the AdminScreen test file to check for regressions**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx`
Expected: PASS — no test asserts the old `[status]` bracket format (verified: only `Guild Ledger`, row labels, and `quality_label` values like `"reinforced"`/`"draft"` are checked as substrings, which remain present under column formatting).

- [x] **Step 6: Commit**

```bash
git add frontend/src/admin/AdminTable.tsx frontend/src/admin/AdminTable.test.tsx
git commit -m "feat: align AdminTable rows into fixed-width columns"
```

---

## Task 4: Render dialogs as a non-destructive overlay

**Files:**
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Modify: `frontend/src/admin/dialogs.tsx`
- Modify: `frontend/src/admin/AdminScreen.test.tsx`

**Problem:** When a dialog is open, `AdminScreen` returns a completely different render tree (`if (dialog.kind !== "none") { return (...) }`), so the header, stat panels, and status line are unmounted while the dialog is shown. Verified via a live probe (see Task 6) that OpenTUI's `scrollbox`/box compositing does support `position: "absolute"` overlays with computed offsets, so the dialog can be layered on top of the existing tree instead of replacing it.

- [x] **Step 1: Write the failing test**

Add this test inside the `describe("AdminScreen", ...)` block in `frontend/src/admin/AdminScreen.test.tsx`, after the `"reinforces the selected crystal and refreshes from nested snapshot"` test:

```tsx
  it("keeps the admin header visible behind an open add-memory dialog", async () => {
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);
    await mockInput.type("a");

    const output = await waitForFrame((frame) =>
      frame.includes("Add New Crystal / Lesson / Rule"),
    );

    expect(output).toContain("Add New Crystal / Lesson / Rule");
    expect(output).toContain("H Hieronymus Admin 0.1.0");
  });
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx -t "keeps the admin header visible"`
Expected: FAIL — `expect(output).toContain("H Hieronymus Admin 0.1.0")` fails because the dialog branch fully replaces the header.

- [x] **Step 3: Write minimal implementation — update `DialogOverlay` sizing/positioning**

In `frontend/src/admin/dialogs.tsx`, add the `useTerminalDimensions` import:

```tsx
import { useKeyboard, useTerminalDimensions } from "@opentui/react";
```

Replace the start of `DialogOverlay` (currently):

```tsx
export function DialogOverlay({ state, onClose, onSubmit }: DialogProps) {
  if (state.kind === "none") {
    return null;
  }

  // Common styles
  const overlayStyle: any = {
    position: "absolute",
    top: 0,
    left: 0,
    width: "100%",
    height: "100%",
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "#000000",
  };
```

with:

```tsx
export function DialogOverlay({ state, onClose, onSubmit }: DialogProps) {
  const dimensions = useTerminalDimensions();

  if (state.kind === "none") {
    return null;
  }

  const modalWidth = Math.min(136, dimensions.width);
  const modalHeight = Math.min(24, dimensions.height);
  const top = Math.max(0, Math.floor((dimensions.height - modalHeight) / 2));
  const left = Math.max(0, Math.floor((dimensions.width - modalWidth) / 2));

  // Common styles
  const overlayStyle: any = {
    position: "absolute",
    top,
    left,
    width: modalWidth,
    height: modalHeight,
    alignItems: "center",
    justifyContent: "center",
  };
```

(The hook call is moved above the `state.kind === "none"` early return because React hooks must run unconditionally on every render. The opaque `backgroundColor: "#000000"` is dropped so content outside the modal's bounded footprint remains visible; the modal card itself still has its own `backgroundColor: "#141414"` via `modalStyle`.)

- [x] **Step 4: Write minimal implementation — mount `DialogOverlay` inside the main tree**

In `frontend/src/admin/AdminScreen.tsx`, delete the standalone early-return block:

```tsx
  if (dialog.kind !== "none") {
    return (
      <box
        flexDirection="column"
        width={Math.min(136, dimensions.width)}
        height={Math.min(20, dimensions.height)}
        alignItems="center"
        justifyContent="center"
      >
        <DialogOverlay
          state={dialog}
          onClose={() => setDialog(closedDialog)}
          onSubmit={handleDialogSubmit}
        />
      </box>
    );
  }
```

Then give both root layout boxes an explicit `height` (needed so nested `position: "absolute"` children resolve correctly against a sized ancestor), and append `<DialogOverlay />` as the last child of each.

Compact-layout root box currently starts:

```tsx
    return (
      <box flexDirection="column" width={dimensions.width}>
```

Change to:

```tsx
    return (
      <box
        flexDirection="column"
        width={dimensions.width}
        height={dimensions.height}
      >
```

and add, right before the closing `</box>` of that same return block (after `<StatusLine message={status.message} error={status.error} />`):

```tsx
        <DialogOverlay
          state={dialog}
          onClose={() => setDialog(closedDialog)}
          onSubmit={handleDialogSubmit}
        />
```

Wide-layout root box currently starts:

```tsx
  return (
    <box flexDirection="column" width={Math.min(136, dimensions.width)}>
```

Change to:

```tsx
  return (
    <box
      flexDirection="column"
      width={Math.min(136, dimensions.width)}
      height={dimensions.height}
    >
```

and add, right before the closing `</box>` of that return block (after `<KeyHelp ... />`):

```tsx
      <DialogOverlay
        state={dialog}
        onClose={() => setDialog(closedDialog)}
        onSubmit={handleDialogSubmit}
      />
```

- [x] **Step 5: Run test to verify it passes**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx -t "keeps the admin header visible"`
Expected: PASS

- [x] **Step 6: Run the full AdminScreen test file to check for regressions**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx`
Expected: PASS — including the `"does not open hidden dialogs below the minimum terminal size"` test (still guarded by the existing `useEffect` that force-closes dialogs when `layout.kind === "too-small"`) and all dialog interaction tests (Add/Edit/Merge/Split/Delete/Rename keyboard flows).

- [x] **Step 7: Commit**

```bash
git add frontend/src/admin/AdminScreen.tsx frontend/src/admin/dialogs.tsx frontend/src/admin/AdminScreen.test.tsx
git commit -m "fix: render admin dialogs as a centered overlay instead of replacing the screen"
```

---

## Task 5: Deduplicate dialog field-focus navigation

**Files:**
- Create: `frontend/src/ui/useFieldFocus.ts`
- Create: `frontend/src/ui/useFieldFocus.test.tsx`
- Modify: `frontend/src/admin/dialogs.tsx`

**Problem:** `AddDialog`, `EditDialog`, `MergeDialog`, and `SplitDialog` each reimplement clamped up/down focus-index navigation with slightly different inline `Math.max`/`Math.min` logic.

- [x] **Step 1: Write the failing hook test**

Create `frontend/src/ui/useFieldFocus.test.tsx`:

```tsx
import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
import { useKeyboard } from "@opentui/react";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { useFieldFocus } from "./useFieldFocus.js";

function Probe({ fieldCount }: { fieldCount: number }) {
  const { focusedIndex, moveUp, moveDown } = useFieldFocus(fieldCount);
  useKeyboard((key) => {
    if (key.name === "down") {
      moveDown();
    } else if (key.name === "up") {
      moveUp();
    }
  });
  return <text>focused {focusedIndex}</text>;
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("useFieldFocus", () => {
  it("starts at index 0", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Probe fieldCount={3} />);

    const output = await waitForFrame((frame) => frame.includes("focused"));
    expect(output).toContain("focused 0");
  });

  it("moves down and clamps at fieldCount - 1", async () => {
    const { render, mockInput, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Probe fieldCount={2} />);
    await mockInput.press("down");
    await mockInput.press("down");
    await mockInput.press("down");

    const output = await waitForFrame((frame) => frame.includes("focused 1"));
    expect(output).toContain("focused 1");
  });

  it("moves up and clamps at 0", async () => {
    const { render, mockInput, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Probe fieldCount={3} />);
    await mockInput.press("down");
    await mockInput.press("up");
    await mockInput.press("up");

    const output = await waitForFrame((frame) => frame.includes("focused 0"));
    expect(output).toContain("focused 0");
  });
});
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd frontend && bun test src/ui/useFieldFocus.test.tsx`
Expected: FAIL — `Cannot find module './useFieldFocus.js'`.

- [x] **Step 3: Write minimal implementation**

Create `frontend/src/ui/useFieldFocus.ts`:

```ts
import { useState } from "react";

export type FieldFocus = {
  focusedIndex: number;
  moveUp: () => void;
  moveDown: () => void;
  setFocusedIndex: (index: number) => void;
};

export function useFieldFocus(fieldCount: number): FieldFocus {
  const [focusedIndex, setFocusedIndex] = useState(0);

  const moveUp = () => {
    setFocusedIndex((current) => Math.max(0, current - 1));
  };

  const moveDown = () => {
    setFocusedIndex((current) =>
      Math.min(Math.max(fieldCount - 1, 0), current + 1),
    );
  };

  return { focusedIndex, moveUp, moveDown, setFocusedIndex };
}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd frontend && bun test src/ui/useFieldFocus.test.tsx`
Expected: PASS (3 tests)

- [x] **Step 5: Commit the hook**

```bash
git add frontend/src/ui/useFieldFocus.ts frontend/src/ui/useFieldFocus.test.tsx
git commit -m "feat: add useFieldFocus hook for dialog field navigation"
```

- [x] **Step 6: Apply the hook to `AddDialog`**

In `frontend/src/admin/dialogs.tsx`, add the import:

```tsx
import { useFieldFocus } from "../ui/useFieldFocus.js";
```

In `AddDialog`, replace:

```tsx
  const [focusedIndex, setFocusedIndex] = useState(0); // 0 = type, 1 = title, 2 = text, 3 = tags
```

with:

```tsx
  const { focusedIndex, moveUp, moveDown } = useFieldFocus(4); // 0 = type, 1 = title, 2 = text, 3 = tags
```

and replace the keyboard handler's navigation branch:

```tsx
    if (key.name === "up") {
      setFocusedIndex((prev) => Math.max(0, prev - 1));
    } else if (key.name === "down") {
      setFocusedIndex((prev) => Math.min(3, prev + 1));
    } else if (focusedIndex === 0) {
```

with:

```tsx
    if (key.name === "up") {
      moveUp();
    } else if (key.name === "down") {
      moveDown();
    } else if (focusedIndex === 0) {
```

- [x] **Step 7: Apply the hook to `EditDialog`**

Replace:

```tsx
  const [title, setTitle] = useState(state.initialTitle || "");
  const [text, setText] = useState(state.initialText || "");
  const [focusedIndex, setFocusedIndex] = useState(0); // 0 = title, 1 = text
```

with:

```tsx
  const [title, setTitle] = useState(state.initialTitle || "");
  const [text, setText] = useState(state.initialText || "");
  const { focusedIndex, moveUp, moveDown } = useFieldFocus(2); // 0 = title, 1 = text
```

and replace:

```tsx
    if (key.name === "up") {
      setFocusedIndex(0);
    } else if (key.name === "down") {
      setFocusedIndex(1);
    }
```

with:

```tsx
    if (key.name === "up") {
      moveUp();
    } else if (key.name === "down") {
      moveDown();
    }
```

- [x] **Step 8: Apply the hook to `MergeDialog`**

Replace:

```tsx
  const [focusedIndex, setFocusedIndex] = useState(0);
  const [localError, setLocalError] = useState("");

  const maxIndex = isConcept ? 1 : 2; // concept: 0=targetId, 1=evidence. crystal: 0=targetId, 1=title, 2=text.
```

with:

```tsx
  const maxIndex = isConcept ? 1 : 2; // concept: 0=targetId, 1=evidence. crystal: 0=targetId, 1=title, 2=text.
  const { focusedIndex, moveUp, moveDown } = useFieldFocus(maxIndex + 1);
  const [localError, setLocalError] = useState("");
```

and replace:

```tsx
    if (key.name === "up") {
      setFocusedIndex((prev) => Math.max(0, prev - 1));
    } else if (key.name === "down") {
      setFocusedIndex((prev) => Math.min(maxIndex, prev + 1));
    }
```

with:

```tsx
    if (key.name === "up") {
      moveUp();
    } else if (key.name === "down") {
      moveDown();
    }
```

- [x] **Step 9: Apply the hook to `SplitDialog`**

Replace:

```tsx
  const [focusedIndex, setFocusedIndex] = useState(0); // 0=p1 title, 1=p1 text, 2=p2 title, 3=p2 text
```

with:

```tsx
  const { focusedIndex, moveUp, moveDown } = useFieldFocus(4); // 0=p1 title, 1=p1 text, 2=p2 title, 3=p2 text
```

and replace:

```tsx
    if (key.name === "up") {
      setFocusedIndex((prev) => Math.max(0, prev - 1));
    } else if (key.name === "down") {
      setFocusedIndex((prev) => Math.min(3, prev + 1));
    }
```

with:

```tsx
    if (key.name === "up") {
      moveUp();
    } else if (key.name === "down") {
      moveDown();
    }
```

- [x] **Step 10: Run the full AdminScreen test file to confirm dialog behavior is unchanged**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx`
Expected: PASS — all existing Add/Edit/Merge/Split dialog keyboard tests pass unmodified, since `useFieldFocus`'s clamped increment/decrement is behaviorally identical to the inline logic it replaces.

- [x] **Step 11: Commit**

```bash
git add frontend/src/admin/dialogs.tsx
git commit -m "refactor: dedupe dialog field-focus navigation with useFieldFocus"
```

---

## Task 6: Add visible scrollbar indicators to AdminTable and DetailPane

**Files:**
- Modify: `frontend/src/admin/AdminTable.tsx`
- Modify: `frontend/src/admin/AdminTable.test.tsx`
- Modify: `frontend/src/admin/DetailPane.tsx`
- Create: `frontend/src/admin/DetailPane.test.tsx`

**Problem:** `scrollbox` renders only a single `▀` thumb glyph by default when content overflows (verified via a live probe render: default `scrollbox` output showed a lone `▀` character in the last column and nothing else; no up/down arrows). This is easy to miss. Setting `scrollbarOptions: { showArrows: true }` renders `▲`/`▼` arrow glyphs in addition to the thumb (also verified via a live probe render), which is a much clearer affordance — and both glyphs are absent entirely when content fits without overflow.

- [x] **Step 1: Write the failing AdminTable scrollbar test**

Add this test to the `describe("AdminTable", ...)` block in `frontend/src/admin/AdminTable.test.tsx`:

```tsx
  it("shows scrollbar arrows when rows overflow the visible height", async () => {
    const rows = Array.from({ length: 30 }, (_, index) =>
      row({ id: index, label: `Row ${index}` }),
    );
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={40} height={5} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Row 0"));
    expect(output).toContain("▲");
  });

  it("shows no scrollbar arrows when rows fit within the visible height", async () => {
    const rows = [row({ id: 1, label: "Only Row" })];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={40} height={5} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Only Row"));
    expect(output).not.toContain("▲");
  });
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd frontend && bun test src/admin/AdminTable.test.tsx -t "shows scrollbar arrows"`
Expected: FAIL — `expect(output).toContain("▲")` fails because `AdminTable`'s `scrollbox` has no `scrollbarOptions` set.

- [x] **Step 3: Write minimal implementation**

In `frontend/src/admin/AdminTable.tsx`, update the `scrollbox` element:

```tsx
    <scrollbox
      flexDirection="column"
      width={width}
      height={height}
      style={{ scrollbarOptions: { showArrows: true } }}
    >
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd frontend && bun test src/admin/AdminTable.test.tsx`
Expected: PASS (5 tests)

- [x] **Step 5: Commit AdminTable scrollbar change**

```bash
git add frontend/src/admin/AdminTable.tsx frontend/src/admin/AdminTable.test.tsx
git commit -m "feat: show scrollbar arrows on AdminTable when rows overflow"
```

- [x] **Step 6: Write the failing DetailPane scrollbar test**

Create `frontend/src/admin/DetailPane.test.tsx`:

```tsx
import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
import type { AdminDetail } from "../rpc/schema.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { DetailPane } from "./DetailPane.js";

function detail(overrides: Partial<AdminDetail> = {}): AdminDetail {
  return {
    title: "Guild Ledger",
    subtitle: "concept",
    body: "Guild ledger detail marker.",
    fields: [],
    ...overrides,
  };
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("DetailPane", () => {
  it("shows scrollbar arrows when the body overflows the visible height", async () => {
    const longBody = Array.from({ length: 30 }, (_, i) => `Line ${i}`).join(
      "\n",
    );
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <DetailPane detail={detail({ body: longBody })} width={40} height={5} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Line 0"));
    expect(output).toContain("▲");
  });

  it("shows no scrollbar arrows when the body fits within the visible height", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <DetailPane
        detail={detail({ body: "Short body." })}
        width={40}
        height={5}
      />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Short body."),
    );
    expect(output).not.toContain("▲");
  });
});
```

- [x] **Step 7: Run test to verify it fails**

Run: `cd frontend && bun test src/admin/DetailPane.test.tsx -t "shows scrollbar arrows"`
Expected: FAIL — `DetailPane`'s `scrollbox` has no `scrollbarOptions` set yet.

- [x] **Step 8: Write minimal implementation**

In `frontend/src/admin/DetailPane.tsx`, update the `scrollbox` element:

```tsx
    <scrollbox
      flexDirection="column"
      width={width}
      height={height}
      style={{ scrollbarOptions: { showArrows: true } }}
    >
```

- [x] **Step 9: Run test to verify it passes**

Run: `cd frontend && bun test src/admin/DetailPane.test.tsx`
Expected: PASS (2 tests)

- [x] **Step 10: Commit**

```bash
git add frontend/src/admin/DetailPane.tsx frontend/src/admin/DetailPane.test.tsx
git commit -m "feat: show scrollbar arrows on DetailPane when content overflows"
```

---

## Task 7: Strengthen header visual hierarchy with a service status indicator

**Files:**
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Modify: `frontend/src/admin/AdminScreen.test.tsx`

**Problem:** The wide-layout `Header` component shows product/version/tagline but no service running/stopped state; that state is only visible transiently in the status line at the bottom.

- [x] **Step 1: Write the failing test**

Add one assertion to the existing `"renders views, stats, table row, and detail"` test in `frontend/src/admin/AdminScreen.test.tsx` (bootstrap's `service.running` is `false`), right after `expect(output).toContain("H Hieronymus Admin 0.1.0");`:

```tsx
    expect(output).toContain("○ Service stopped");
```

Then add a new test after it, inside the `describe("AdminScreen", ...)` block:

```tsx
  it("shows a live service status indicator in the header", async () => {
    const { render, waitForFrame } = setupTest();

    await render(
      <AdminScreen
        initial={{ ...bootstrap(), service: { running: true } }}
        client={undefined}
      />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Hieronymus Admin"),
    );
    expect(output).toContain("● Service running");
  });
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx -t "service status indicator"`
Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx -t "renders views, stats, table row, and detail"`
Expected: Both FAIL — neither `"○ Service stopped"` nor `"● Service running"` is rendered yet.

- [x] **Step 3: Write minimal implementation**

In `frontend/src/admin/AdminScreen.tsx`, replace the `Header` function:

```tsx
function Header({ header }: { header: AdminHeader }) {
  return (
    <>
      <text>
        {header.logo.text} {header.product} Admin {header.version}
      </text>
      <text fg="gray">{header.tagline}</text>
    </>
  );
}
```

with:

```tsx
function Header({
  header,
  serviceRunning,
}: {
  header: AdminHeader;
  serviceRunning: boolean;
}) {
  return (
    <>
      <box flexDirection="row" justifyContent="space-between">
        <text>
          {header.logo.text} {header.product} Admin {header.version}
        </text>
        <text fg={serviceRunning ? "green" : "gray"}>
          {serviceRunning ? "● Service running" : "○ Service stopped"}
        </text>
      </box>
      <text fg="gray">{header.tagline}</text>
    </>
  );
}
```

Then update the call site in the wide-layout branch:

```tsx
        <Header header={initial.header} />
```

to:

```tsx
        <Header header={initial.header} serviceRunning={initial.service.running} />
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx`
Expected: PASS — all tests including the two updated/added in Step 1.

- [x] **Step 5: Commit**

```bash
git add frontend/src/admin/AdminScreen.tsx frontend/src/admin/AdminScreen.test.tsx
git commit -m "feat: show live service status indicator in the admin header"
```

---

## Task 8: Differentiate disabled command-palette entries without relying on dimming alone

**Files:**
- Modify: `frontend/src/admin/CommandPalette.tsx`
- Create: `frontend/src/admin/CommandPalette.test.tsx`

**Problem:** Disabled commands render at `fg="gray"` with an inline `(unavailable)` suffix, which is easy to miss (dimmed text next to unselected-but-enabled rows looks similar) and relies on color+text glued onto every row instead of a distinct marker plus a highlighted-state reason.

- [x] **Step 1: Write the failing test**

Create `frontend/src/admin/CommandPalette.test.tsx`:

```tsx
import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
import type { AdminCommand } from "../rpc/schema.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { CommandPalette } from "./CommandPalette.js";

function command(
  overrides: Partial<AdminCommand & { disabled: boolean }> = {},
): AdminCommand & { disabled: boolean } {
  return {
    id: "edit_memory",
    label: "Edit Memory",
    hint: "Edit the selected crystal or lesson text.",
    key: "e",
    group: "Memory",
    views: ["Crystals"],
    requires_selection: true,
    disabled: false,
    ...overrides,
  };
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("CommandPalette", () => {
  it("marks a disabled command with a non-color marker instead of an inline suffix", async () => {
    const commands = [
      command({ id: "add_memory", label: "Add Memory", disabled: false }),
      command({ disabled: true }),
    ];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 12,
    });

    await render(<CommandPalette commands={commands} selectedIndex={0} />);

    const output = await waitForFrame((frame) =>
      frame.includes("Edit Memory"),
    );
    expect(output).toContain("✕ Edit Memory");
    expect(output).not.toContain("(unavailable)");
  });

  it("shows the disabled reason on the hint line only when the disabled command is highlighted", async () => {
    const commands = [command({ disabled: true })];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 12,
    });

    await render(<CommandPalette commands={commands} selectedIndex={0} />);

    const output = await waitForFrame((frame) =>
      frame.includes("Edit Memory"),
    );
    expect(output).toContain("Edit Memory needs a selected row");
  });

  it("shows the normal hint when the highlighted command is enabled", async () => {
    const commands = [command({ disabled: false })];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 12,
    });

    await render(<CommandPalette commands={commands} selectedIndex={0} />);

    const output = await waitForFrame((frame) =>
      frame.includes("Edit Memory"),
    );
    expect(output).toContain("Edit the selected crystal or lesson text.");
  });
});
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd frontend && bun test src/admin/CommandPalette.test.tsx`
Expected: FAIL — the current output contains `"(unavailable)"` and no `"✕ Edit Memory"` or `"needs a selected row"` text.

- [x] **Step 3: Write minimal implementation**

In `frontend/src/admin/CommandPalette.tsx`, replace the command rows and hint block:

```tsx
      {commands.map((command, index) => (
        <box key={command.id} height={1}>
          <text
            fg={
              command.disabled
                ? "gray"
                : index === selectedIndex
                  ? "cyan"
                  : undefined
            }
          >
            {index === selectedIndex ? "> " : "  "}
            {command.label} [{command.key}]{" "}
            {command.disabled ? "(unavailable)" : ""}
          </text>
        </box>
      ))}
      {commands[selectedIndex] ? (
        <box height={1}>
          <text fg="gray">{commands[selectedIndex].hint}</text>
        </box>
      ) : null}
```

with:

```tsx
      {commands.map((command, index) => (
        <box key={command.id} height={1}>
          <text
            fg={
              index === selectedIndex
                ? command.disabled
                  ? "yellow"
                  : "cyan"
                : undefined
            }
          >
            {index === selectedIndex ? "> " : "  "}
            {command.disabled ? "✕ " : "  "}
            {command.label} [{command.key}]
          </text>
        </box>
      ))}
      {commands[selectedIndex] ? (
        <box height={1}>
          <text fg={commands[selectedIndex].disabled ? "yellow" : "gray"}>
            {commands[selectedIndex].disabled
              ? `${commands[selectedIndex].label} needs a selected row`
              : commands[selectedIndex].hint}
          </text>
        </box>
      ) : null}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd frontend && bun test src/admin/CommandPalette.test.tsx`
Expected: PASS (3 tests)

- [x] **Step 5: Run the full AdminScreen test file to check for regressions**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx`
Expected: PASS — no existing test asserts the removed `"(unavailable)"` string (verified by inspection of `AdminScreen.test.tsx`).

- [x] **Step 6: Commit**

```bash
git add frontend/src/admin/CommandPalette.tsx frontend/src/admin/CommandPalette.test.tsx
git commit -m "feat: mark disabled command-palette entries with a symbol instead of dimming alone"
```

---

## Task 9: Introduce a semantic color theme and apply it across touched files

**Files:**
- Create: `frontend/src/ui/theme.ts`
- Create: `frontend/src/ui/theme.test.ts`
- Modify: `frontend/src/ui/StatusLine.tsx`
- Modify: `frontend/src/admin/CommandPalette.tsx`
- Modify: `frontend/src/admin/AdminTable.tsx`
- Modify: `frontend/src/admin/DetailPane.tsx`
- Modify: `frontend/src/admin/HelpOverlay.tsx`
- Modify: `frontend/src/admin/dialogs.tsx`
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Modify: `frontend/src/config/ConfigScreen.tsx`
- Modify: `frontend/src/config/ConfigForm.tsx`

**Problem:** Named color strings (`"cyan"`, `"gray"`, `"red"`, `"green"`, `"yellow"`) are hardcoded inline throughout the codebase, including in the code just added by Tasks 1-8. There is no single place defining what a color means.

- [x] **Step 1: Write the failing theme test**

Create `frontend/src/ui/theme.test.ts`:

```ts
import { describe, expect, it } from "bun:test";
import { theme } from "./theme.js";

describe("theme", () => {
  it("exposes the semantic color slots used across the admin and config screens", () => {
    expect(theme.accentPrimary).toBe("cyan");
    expect(theme.accentMuted).toBe("gray");
    expect(theme.statusError).toBe("red");
    expect(theme.statusSuccess).toBe("green");
    expect(theme.statusWarning).toBe("yellow");
  });

  it("freezes the theme object so slots cannot be mutated at runtime", () => {
    expect(Object.isFrozen(theme)).toBe(true);
  });
});
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd frontend && bun test src/ui/theme.test.ts`
Expected: FAIL — `Cannot find module './theme.js'`.

- [x] **Step 3: Write minimal implementation**

Create `frontend/src/ui/theme.ts`:

```ts
export const theme = Object.freeze({
  accentPrimary: "cyan",
  accentMuted: "gray",
  statusError: "red",
  statusSuccess: "green",
  statusWarning: "yellow",
});
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd frontend && bun test src/ui/theme.test.ts`
Expected: PASS (2 tests)

- [x] **Step 5: Commit the theme module**

```bash
git add frontend/src/ui/theme.ts frontend/src/ui/theme.test.ts
git commit -m "feat: add semantic color theme module"
```

- [x] **Step 6: Apply the theme to `StatusLine.tsx` as the worked example**

In `frontend/src/ui/StatusLine.tsx`, add the import:

```tsx
import { theme } from "./theme.js";
```

Replace:

```tsx
  const fg = error ? "red" : busy && pulse > 0.5 ? "cyan" : "green";
```

with:

```tsx
  const fg = error
    ? theme.statusError
    : busy && pulse > 0.5
      ? theme.accentPrimary
      : theme.statusSuccess;
```

Verify no literal color strings remain:

Run: `grep -n 'fg="cyan"\|fg="gray"\|fg="red"\|fg="green"\|fg="yellow"\|borderColor="cyan"\|borderColor="gray"\|borderColor="red"\|"cyan"\|"gray"\|"red"\|"green"\|"yellow"' frontend/src/ui/StatusLine.tsx`
Expected: no output.

- [x] **Step 7: Run the StatusLine-dependent tests to check for regressions**

Run: `cd frontend && bun test src/admin/AdminScreen.test.tsx src/config/ConfigScreen.test.tsx`
Expected: PASS — `theme.statusError`/`theme.accentPrimary`/`theme.statusSuccess` resolve to the same string values (`"red"`/`"cyan"`/`"green"`) as before, so rendered output is unchanged.

- [x] **Step 8: Commit**

```bash
git add frontend/src/ui/StatusLine.tsx
git commit -m "refactor: use theme constants in StatusLine"
```

- [x] **Step 9: Apply the same substitution pattern to the remaining files**

For each file below, add `import { theme } from "../ui/theme.js";` (or `"./theme.js"` for files already inside `frontend/src/ui/`), then apply this exact, deterministic substitution to every occurrence in the file:

| Literal found in the file | Replace with |
|---|---|
| `fg="cyan"` (plain string attribute) | `fg={theme.accentPrimary}` |
| `fg="gray"` | `fg={theme.accentMuted}` |
| `fg="red"` | `fg={theme.statusError}` |
| `fg="green"` | `fg={theme.statusSuccess}` |
| `fg="yellow"` | `fg={theme.statusWarning}` |
| `borderColor="cyan"` | `borderColor={theme.accentPrimary}` |
| `borderColor="gray"` | `borderColor={theme.accentMuted}` |
| `borderColor="red"` | `borderColor={theme.statusError}` |
| Bare `"cyan"` inside an existing `{...}` expression (e.g. `fg={cond ? "cyan" : "gray"}`) | `theme.accentPrimary` (keep the surrounding expression syntax unchanged) |
| Bare `"gray"` inside an existing `{...}` expression | `theme.accentMuted` |
| Bare `"red"` inside an existing `{...}` expression | `theme.statusError` |
| Bare `"green"` inside an existing `{...}` expression | `theme.statusSuccess` |
| Bare `"yellow"` inside an existing `{...}` expression | `theme.statusWarning` |

Apply this to, in order:

1. `frontend/src/admin/CommandPalette.tsx`
2. `frontend/src/admin/AdminTable.tsx`
3. `frontend/src/admin/DetailPane.tsx`
4. `frontend/src/admin/HelpOverlay.tsx`
5. `frontend/src/admin/dialogs.tsx`
6. `frontend/src/admin/AdminScreen.tsx`
7. `frontend/src/config/ConfigScreen.tsx`
8. `frontend/src/config/ConfigForm.tsx`

After each file, verify completeness with:

Run: `grep -n 'fg="cyan"\|fg="gray"\|fg="red"\|fg="green"\|fg="yellow"\|borderColor="cyan"\|borderColor="gray"\|borderColor="red"' <file>`
Expected: no output (the grep exits with status 1).

Note: `AdminScreen.tsx`'s `Gauge` integration (Task 2) passes `fg="yellow"`/`fg="cyan"` as literal strings to the `Gauge` component's `fg` prop — these are matched by the same grep pattern's `fg="cyan"` and `fg="yellow"` cases and must be converted the same way (`fg={theme.accentPrimary}`, `fg={theme.statusWarning}`).

- [x] **Step 10: Run the full frontend test suite**

Run: `cd frontend && bun test`
Expected: PASS — every test file passes. Since `theme.ts` maps each slot to the exact same string value the code used before (`accentPrimary` → `"cyan"`, etc.), no rendered output changes, so no test assertions need to change.

- [x] **Step 11: Run typecheck**

Run: `cd frontend && bun run typecheck`
Expected: no errors — `fg`/`borderColor` props accept `string`, and `theme.*` values are typed as string literals from the frozen `theme` object.

- [x] **Step 12: Commit**

```bash
git add frontend/src/admin/CommandPalette.tsx frontend/src/admin/AdminTable.tsx frontend/src/admin/DetailPane.tsx frontend/src/admin/HelpOverlay.tsx frontend/src/admin/dialogs.tsx frontend/src/admin/AdminScreen.tsx frontend/src/config/ConfigScreen.tsx frontend/src/config/ConfigForm.tsx
git commit -m "refactor: use theme constants instead of hardcoded color literals"
```

---

## Final Verification

- [x] **Run the entire frontend test suite one more time**

Run: `cd frontend && bun test`
Expected: PASS — all test files, including the five new ones (`Gauge.test.tsx`, `AdminTable.test.tsx`, `DetailPane.test.tsx`, `CommandPalette.test.tsx`, `useFieldFocus.test.tsx`, `theme.test.ts`) and all modified ones (`AdminScreen.test.tsx`).

- [x] **Run typecheck and format check**

Run: `cd frontend && bun run typecheck && bun run format`
Expected: no errors.
