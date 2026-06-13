# OpenTUI Responsive Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `hiero admin` and `hiero config` usable at the 80x24 terminal floor and in narrow tmux splits by replacing fixed-width multi-pane rendering with tested responsive single-pane fallbacks.

**Architecture:** Add one small frontend layout classifier for terminal breakpoints, then have each OpenTUI screen choose between the existing wide layout and compact layouts based on `useTerminalDimensions()`. Keep backend contracts unchanged; all mutations stay behind the existing RPC clients and Python bridges.

**Tech Stack:** TypeScript, React 19, OpenTUI React 0.4.0, Bun 1.3.14 test runner, existing shared `frontend/src/test/opentuiHarness.tsx`.

---

## Current Code Map

- `frontend/src/admin/AdminScreen.tsx` renders a fixed `width={136}` shell with header, views, table, and detail panes in one row.
- `frontend/src/admin/AdminTable.tsx` hardcodes `width={48}` and `height={18}`.
- `frontend/src/admin/DetailPane.tsx` hardcodes `width={56}` and `height={14}`.
- `frontend/src/config/ConfigScreen.tsx` renders a fixed `width={100}` shell with provider and form columns.
- `frontend/src/config/ConfigForm.tsx` hardcodes `width={68}`.
- `frontend/src/test/opentuiHarness.tsx` already accepts renderer `{ width, height }` options and can render 80x24 and 60-column scenarios.
- `docs/roadmap.md` still lists responsive 80x24 and narrow tmux behavior as remaining OpenTUI work.

## Target Behavior

- Wide terminals keep the current multi-pane spatial layout.
- Standard 80x24 terminals show a compact layout without horizontal overflow:
  - `hiero admin`: one active pane at a time, selected with `Tab`/`Shift+Tab`; header and status are compressed.
  - `hiero config`: provider list and form become a single active pane layout; model suggestions and validation remain visible below the active pane when space allows.
- Narrow 60-column splits do not try to render wide panels. They show a clear compact or too-small state instead of broken columns.
- Below the minimum useful size, both screens render a short message that includes the current terminal size and the minimum target.
- Dialog, help, and command-palette overlays remain reachable and do not exceed compact viewport width.

## File Structure

- Create: `frontend/src/ui/responsive.ts`
  - Pure breakpoint helpers and shared dimensions.
- Modify: `frontend/src/admin/AdminScreen.tsx`
  - Use terminal dimensions, render wide/compact/too-small admin layouts.
- Modify: `frontend/src/admin/AdminTable.tsx`
  - Accept optional `width` and `height` props.
- Modify: `frontend/src/admin/DetailPane.tsx`
  - Accept optional `width` and `height` props.
- Modify: `frontend/src/admin/CommandPalette.tsx`
  - Accept optional width cap for compact mode if the fixed width overflows.
- Modify: `frontend/src/admin/HelpOverlay.tsx`
  - Accept optional width cap for compact mode if the fixed width overflows.
- Modify: `frontend/src/config/ConfigScreen.tsx`
  - Use terminal dimensions, render wide/compact/too-small config layouts.
- Modify: `frontend/src/config/ConfigForm.tsx`
  - Accept optional `width` prop.
- Modify: `frontend/src/admin/AdminScreen.test.tsx`
  - Add 80x24 and 60-column render tests.
- Modify: `frontend/src/config/ConfigScreen.test.tsx`
  - Add 80x24 and 60-column render tests.
- Modify: `docs/roadmap.md`
  - Move responsive OpenTUI floor behavior from remaining work to completed work after implementation.

---

### Task 1: Add Shared Breakpoint Classification

**Files:**
- Create: `frontend/src/ui/responsive.ts`
- Test: `frontend/src/ui/responsive.test.ts`

- [x] **Step 1: Write the failing tests**

Create `frontend/src/ui/responsive.test.ts`:

```ts
import { describe, expect, it } from "bun:test";
import { classifyTerminalLayout, panelWidth } from "./responsive.js";

describe("responsive layout helpers", () => {
  it("classifies wide, compact, narrow, and too-small terminal sizes", () => {
    expect(classifyTerminalLayout(160, 60)).toEqual({
      kind: "wide",
      width: 160,
      height: 60,
    });
    expect(classifyTerminalLayout(80, 24)).toEqual({
      kind: "compact",
      width: 80,
      height: 24,
    });
    expect(classifyTerminalLayout(60, 24)).toEqual({
      kind: "narrow",
      width: 60,
      height: 24,
    });
    expect(classifyTerminalLayout(49, 20)).toEqual({
      kind: "too-small",
      width: 49,
      height: 20,
    });
  });

  it("keeps panel content inside the terminal width after borders", () => {
    expect(panelWidth({ kind: "compact", width: 80, height: 24 }, 2)).toBe(76);
    expect(panelWidth({ kind: "narrow", width: 60, height: 24 }, 2)).toBe(56);
  });
});
```

- [x] **Step 2: Run the tests to verify they fail**

Run:

```bash
bun test --cwd frontend src/ui/responsive.test.ts
```

Expected: FAIL because `frontend/src/ui/responsive.ts` does not exist.

- [x] **Step 3: Add the implementation**

Create `frontend/src/ui/responsive.ts`:

```ts
export type TerminalLayoutKind = "wide" | "compact" | "narrow" | "too-small";

export type TerminalLayout = {
  kind: TerminalLayoutKind;
  width: number;
  height: number;
};

export const MIN_TERMINAL_WIDTH = 60;
export const MIN_TERMINAL_HEIGHT = 20;
export const MIN_COMPACT_WIDTH = 80;
export const MIN_COMPACT_HEIGHT = 24;
export const WIDE_WIDTH = 136;

export function classifyTerminalLayout(
  width: number,
  height: number,
): TerminalLayout {
  if (width < MIN_TERMINAL_WIDTH || height < MIN_TERMINAL_HEIGHT) {
    return { kind: "too-small", width, height };
  }
  if (width >= WIDE_WIDTH && height >= MIN_COMPACT_HEIGHT) {
    return { kind: "wide", width, height };
  }
  if (width >= MIN_COMPACT_WIDTH && height >= MIN_COMPACT_HEIGHT) {
    return { kind: "compact", width, height };
  }
  return { kind: "narrow", width, height };
}

export function panelWidth(layout: TerminalLayout, borderPadding = 2): number {
  return Math.max(20, layout.width - borderPadding * 2);
}

export function panelHeight(
  layout: TerminalLayout,
  reservedRows: number,
): number {
  return Math.max(4, layout.height - reservedRows);
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run:

```bash
bun test --cwd frontend src/ui/responsive.test.ts
```

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add frontend/src/ui/responsive.ts frontend/src/ui/responsive.test.ts
git commit -m "test: add opentui responsive layout helpers"
```

---

### Task 2: Make Config Screen Responsive

**Files:**
- Modify: `frontend/src/config/ConfigForm.tsx`
- Modify: `frontend/src/config/ConfigScreen.tsx`
- Test: `frontend/src/config/ConfigScreen.test.tsx`

- [x] **Step 1: Write compact config render tests**

In `frontend/src/config/ConfigScreen.test.tsx`, add this helper near `setupTest()`:

```ts
function setupSizedTest(width: number, height: number) {
  return createOpenTuiHarness({ width, height });
}
```

Then add these tests inside `describe("ConfigScreen", () => { ... })`:

```ts
it("renders config as a single active pane at 80x24", async () => {
  const { render, waitForFrame } = setupSizedTest(80, 24);

  await render(<ConfigScreen initial={payload()} client={undefined} />);

  const output = await waitForFrame((frame) =>
    frame.includes("Hieronymus Config"),
  );
  expect(output).toContain("Providers");
  expect(output).toContain("OpenAI compatible");
  expect(output).toContain("Tab pane");
  expect(output).not.toContain("/tmp/dream.conf | /tmp/ingest.conf | /tmp/release.conf");
});

it("renders a too-small config message below the minimum width", async () => {
  const { render, waitForFrame } = setupSizedTest(59, 20);

  await render(<ConfigScreen initial={payload()} client={undefined} />);

  const output = await waitForFrame((frame) =>
    frame.includes("Terminal too small"),
  );
  expect(output).toContain("59x20");
  expect(output).toContain("minimum 60x20");
});
```

- [x] **Step 2: Run the tests to verify they fail**

Run:

```bash
bun test --cwd frontend src/config/ConfigScreen.test.tsx
```

Expected: FAIL because the screen still renders fixed-width columns and does not show the compact footer wording or too-small message.

- [x] **Step 3: Let `ConfigForm` accept a width**

Modify `frontend/src/config/ConfigForm.tsx`:

```ts
type ConfigFormProps = {
  fields: ConfigFormField[];
  formValues: {
    provider: Record<string, string>;
    dreaming: Record<string, string>;
    ingest: Record<string, string>;
    release: Record<string, string>;
  };
  focusedFieldIndex: number;
  isEditing: boolean;
  focused?: boolean;
  width?: number;
  onFieldChange: (key: string, value: string) => void;
  onSubmitField: () => void;
};
```

Update the function signature and root box:

```tsx
export function ConfigForm({
  fields,
  formValues,
  focusedFieldIndex,
  isEditing,
  focused = true,
  width = 68,
  onFieldChange,
  onSubmitField,
}: ConfigFormProps) {
  // existing body stays the same
  return (
    <box flexDirection="column" width={width}>
      {/* existing children stay the same */}
    </box>
  );
}
```

- [x] **Step 4: Add compact config rendering**

Modify imports in `frontend/src/config/ConfigScreen.tsx`:

```ts
import { useKeyboard, useRenderer, useTerminalDimensions } from "@opentui/react";
import {
  classifyTerminalLayout,
  panelHeight,
  panelWidth,
} from "../ui/responsive.js";
```

Inside `ConfigScreen`, after `const renderer = useRenderer();`, add:

```ts
const dimensions = useTerminalDimensions();
const layout = classifyTerminalLayout(dimensions.width, dimensions.height);
const contentWidth = panelWidth(layout);
```

Before the existing return, add:

```tsx
if (layout.kind === "too-small") {
  return (
    <box flexDirection="column" width={dimensions.width}>
      <text>Terminal too small</text>
      <text fg="gray">
        {dimensions.width}x{dimensions.height}; minimum 60x20
      </text>
      <text fg="gray">Resize terminal to edit Hieronymus config.</text>
    </box>
  );
}

if (layout.kind !== "wide") {
  const compactHeight = panelHeight(layout, 8);
  return (
    <box flexDirection="column" width={dimensions.width}>
      <text>Hieronymus Config</text>
      <text fg="gray">
        {selectedProvider} · {layout.kind} {dimensions.width}x{dimensions.height}
      </text>

      <box
        flexDirection="column"
        marginTop={1}
        height={compactHeight}
        borderStyle="rounded"
        borderColor="cyan"
        paddingX={1}
      >
        {activePanel === "provider" ? (
          <>
            <text fg="cyan">Providers</text>
            <ProviderSelector
              choices={providerChoices}
              selected={selectedProvider}
              focused
              onSelect={selectProviderByIndex}
            />
          </>
        ) : (
          <>
            <text fg="cyan">Dreaming settings</text>
            <ConfigForm
              fields={formFields}
              formValues={localFormValues}
              focusedFieldIndex={focusedFieldIndex}
              isEditing={isEditing}
              focused
              width={contentWidth}
              onFieldChange={handleFieldChange}
              onSubmitField={submitField}
            />
          </>
        )}
      </box>

      <box marginTop={1} flexDirection="column">
        <text>
          Models: {suggestions.length > 0 ? suggestions.join(", ") : "-"}
        </text>
        {payload.validation.errors.slice(0, 2).map((error) => (
          <text key={error} fg="red">
            {error}
          </text>
        ))}
        {detailErrors.slice(0, 2).map((error) => (
          <text key={error} fg="red">
            {error}
          </text>
        ))}
      </box>

      <StatusLine message={status.message} error={status.error} busy={busy} />
      <KeyHelp
        keys={[
          "Tab pane",
          `${providerKeyRange(providerChoices)} provider`,
          "s save",
          "q quit",
        ]}
      />
    </box>
  );
}
```

Leave the existing wide return in place, but change its root width from `100` to `Math.min(100, dimensions.width)`.

- [x] **Step 5: Run config tests**

Run:

```bash
bun test --cwd frontend src/config/ConfigScreen.test.tsx
```

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add frontend/src/config/ConfigForm.tsx frontend/src/config/ConfigScreen.tsx frontend/src/config/ConfigScreen.test.tsx
git commit -m "feat: add compact config opentui layout"
```

---

### Task 3: Make Admin Screen Responsive

**Files:**
- Modify: `frontend/src/admin/AdminTable.tsx`
- Modify: `frontend/src/admin/DetailPane.tsx`
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Test: `frontend/src/admin/AdminScreen.test.tsx`

- [x] **Step 1: Write compact admin render tests**

In `frontend/src/admin/AdminScreen.test.tsx`, add this helper near `setupTest()`:

```ts
function setupSizedTest(width: number, height: number) {
  return createOpenTuiHarness({ width, height });
}
```

Then add these tests inside `describe("AdminScreen", () => { ... })`:

```ts
it("renders admin as a single active pane at 80x24", async () => {
  const { render, waitForFrame } = setupSizedTest(80, 24);

  await render(<AdminScreen initial={bootstrap()} client={undefined} />);

  const output = await waitForFrame((frame) => frame.includes("Views"));
  expect(output).toContain("H Hieronymus Admin 0.1.0");
  expect(output).toContain("Views");
  expect(output).toContain("Crystals");
  expect(output).toContain("Tab pane");
  expect(output).not.toContain("Detail Inspector");
});

it("cycles compact admin panes with tab", async () => {
  const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

  await render(<AdminScreen initial={bootstrap()} client={undefined} />);
  await mockInput.press("tab");

  const tableOutput = await waitForFrame((frame) =>
    frame.includes("Guild Ledger"),
  );
  expect(tableOutput).toContain("Crystals");
  expect(tableOutput).toContain("Guild Ledger");

  await mockInput.press("tab");
  const detailOutput = await waitForFrame((frame) =>
    frame.includes("Guild ledger detail marker."),
  );
  expect(detailOutput).toContain("Detail Inspector");
  expect(detailOutput).toContain("Guild ledger detail marker.");
});

it("renders a too-small admin message below the minimum width", async () => {
  const { render, waitForFrame } = setupSizedTest(59, 20);

  await render(<AdminScreen initial={bootstrap()} client={undefined} />);

  const output = await waitForFrame((frame) =>
    frame.includes("Terminal too small"),
  );
  expect(output).toContain("59x20");
  expect(output).toContain("minimum 60x20");
});
```

- [x] **Step 2: Run the tests to verify they fail**

Run:

```bash
bun test --cwd frontend src/admin/AdminScreen.test.tsx
```

Expected: FAIL because the screen still renders all fixed-width panes at 80 columns and lacks the too-small message.

- [x] **Step 3: Make table and detail panes dimension-aware**

Modify `frontend/src/admin/AdminTable.tsx`:

```tsx
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
  return (
    <scrollbox flexDirection="column" width={width} height={height}>
      {/* existing rows stay the same */}
    </scrollbox>
  );
}
```

Modify `frontend/src/admin/DetailPane.tsx`:

```tsx
export function DetailPane({
  detail,
  width = 56,
  height = 14,
}: {
  detail: AdminSnapshot["detail"];
  width?: number;
  height?: number;
}) {
  // existing renderBody stays the same
  return (
    <scrollbox flexDirection="column" width={width} height={height}>
      {/* existing children stay the same */}
    </scrollbox>
  );
}
```

- [x] **Step 4: Add compact admin rendering**

Modify imports in `frontend/src/admin/AdminScreen.tsx`:

```ts
import { useKeyboard, useRenderer, useTerminalDimensions } from "@opentui/react";
import {
  classifyTerminalLayout,
  panelHeight,
  panelWidth,
} from "../ui/responsive.js";
```

Inside `AdminScreen`, after `const renderer = useRenderer();`, add:

```ts
const dimensions = useTerminalDimensions();
const layout = classifyTerminalLayout(dimensions.width, dimensions.height);
const contentWidth = panelWidth(layout);
```

Before the dialog return, add:

```tsx
if (layout.kind === "too-small") {
  return (
    <box flexDirection="column" width={dimensions.width}>
      <text>Terminal too small</text>
      <text fg="gray">
        {dimensions.width}x{dimensions.height}; minimum 60x20
      </text>
      <text fg="gray">Resize terminal to inspect Hieronymus memory.</text>
    </box>
  );
}
```

Replace the dialog wrapper width/height with responsive values:

```tsx
width={Math.min(136, dimensions.width)}
height={Math.min(20, dimensions.height)}
```

Before the existing wide return, add:

```tsx
if (layout.kind !== "wide") {
  const compactHeight = panelHeight(layout, 9);
  return (
    <box flexDirection="column" width={dimensions.width}>
      <box flexDirection="column" borderStyle="rounded" borderColor="gray" paddingX={1}>
        <Header header={initial.header} />
        <text>{formatStats(stats)}</text>
        <StatusPanels
          shortTermStatus={shortTermStatus}
          dreamStatus={dreamStatus}
        />
      </box>

      <box
        flexDirection="column"
        marginTop={1}
        height={compactHeight}
        borderStyle="rounded"
        borderColor="cyan"
        paddingX={1}
      >
        {activePanel === "views" ? (
          <>
            <text fg="cyan">Views</text>
            <FocusableList
              items={initial.views}
              selectedIndex={selectedViewIndex}
              label={(view) => view}
              focused
            />
          </>
        ) : activePanel === "table" ? (
          <>
            <text fg="cyan">{snapshot.view}</text>
            <AdminTable
              rows={snapshot.rows}
              selectedId={snapshot.selected?.id ?? null}
              focused
              width={contentWidth}
              height={compactHeight - 2}
            />
          </>
        ) : (
          <>
            <text fg="cyan">Detail Inspector</text>
            {helpOpen ? (
              <HelpOverlay
                commands={initial.command_options}
                view={snapshot.view}
              />
            ) : commandsOpen ? (
              <CommandPalette
                commands={paletteCommands}
                selectedIndex={clampCommandIndex(selectedCommandIndex)}
              />
            ) : (
              <DetailPane
                detail={snapshot.detail}
                width={contentWidth}
                height={compactHeight - 2}
              />
            )}
          </>
        )}
      </box>

      <StatusLine message={status.message} error={status.error} />
      <KeyHelp keys={footerKeys({ commandsOpen, helpOpen, viewKeyLimit }).map((key) => key === "Tab focus" ? "Tab pane" : key)} />
    </box>
  );
}
```

Leave the existing wide return in place, but change its root width from `136` to `Math.min(136, dimensions.width)`.

- [x] **Step 5: Run admin tests**

Run:

```bash
bun test --cwd frontend src/admin/AdminScreen.test.tsx
```

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add frontend/src/admin/AdminTable.tsx frontend/src/admin/DetailPane.tsx frontend/src/admin/AdminScreen.tsx frontend/src/admin/AdminScreen.test.tsx
git commit -m "feat: add compact admin opentui layout"
```

---

### Task 4: Keep Compact Overlays Inside the Viewport

**Files:**
- Modify: `frontend/src/admin/CommandPalette.tsx`
- Modify: `frontend/src/admin/HelpOverlay.tsx`
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Test: `frontend/src/admin/AdminScreen.test.tsx`

- [x] **Step 1: Write compact overlay tests**

Add these tests to `frontend/src/admin/AdminScreen.test.tsx`:

```ts
it("renders command palette in compact admin layout", async () => {
  const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

  await render(<AdminScreen initial={bootstrap()} client={undefined} />);
  await mockInput.press("p", { ctrl: true });
  await mockInput.press("tab");
  await mockInput.press("tab");

  const output = await waitForFrame((frame) =>
    frame.includes("Command Palette"),
  );
  expect(output).toContain("Command Palette");
  expect(output).toContain("Enter run Esc close");
});

it("renders help in compact admin layout", async () => {
  const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

  await render(<AdminScreen initial={bootstrap()} client={undefined} />);
  await mockInput.type("?");
  await mockInput.press("tab");
  await mockInput.press("tab");

  const output = await waitForFrame((frame) => frame.includes("Help"));
  expect(output).toContain("Help");
  expect(output).toContain("Esc/? close");
});
```

- [x] **Step 2: Run the tests**

Run:

```bash
bun test --cwd frontend src/admin/AdminScreen.test.tsx
```

Expected: FAIL if overlays exceed compact width or cannot be reached in compact mode.

- [x] **Step 3: Add optional width props to overlays**

Modify `frontend/src/admin/CommandPalette.tsx` props:

```tsx
export function CommandPalette({
  commands,
  selectedIndex,
  width = 54,
}: {
  commands: Array<AdminCommand & { disabled?: boolean }>;
  selectedIndex: number;
  width?: number;
}) {
  const height = Math.min(Math.max(commands.length + 4, 6), 14);
  return (
    <box
      flexDirection="column"
      borderStyle="rounded"
      borderColor="cyan"
      width={width}
      height={height}
      paddingX={1}
    >
      {/* existing children stay the same */}
    </box>
  );
}
```

Modify `frontend/src/admin/HelpOverlay.tsx` props:

```tsx
export function HelpOverlay({
  commands,
  view,
  width = 58,
}: {
  commands: AdminCommand[];
  view: string;
  width?: number;
}) {
  const visibleCommands = commands.filter((command) =>
    command.views.includes(view),
  );
  return (
    <box flexDirection="column" width={width}>
      {/* existing children stay the same */}
    </box>
  );
}
```

- [x] **Step 4: Pass compact widths from admin**

In compact branches of `frontend/src/admin/AdminScreen.tsx`, pass `width={contentWidth}` to `HelpOverlay` and `CommandPalette`.

```tsx
<HelpOverlay
  commands={initial.command_options}
  view={snapshot.view}
  width={contentWidth}
/>
```

```tsx
<CommandPalette
  commands={paletteCommands}
  selectedIndex={clampCommandIndex(selectedCommandIndex)}
  width={contentWidth}
/>
```

Keep wide mode behavior unchanged.

- [x] **Step 5: Run admin tests**

Run:

```bash
bun test --cwd frontend src/admin/AdminScreen.test.tsx
```

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add frontend/src/admin/CommandPalette.tsx frontend/src/admin/HelpOverlay.tsx frontend/src/admin/AdminScreen.tsx frontend/src/admin/AdminScreen.test.tsx
git commit -m "test: cover compact admin overlays"
```

---

### Task 5: Register Roadmap Completion and Verify

**Files:**
- Modify: `docs/roadmap.md`

- [x] **Step 1: Update roadmap completed work**

In `docs/roadmap.md`, add this bullet to the OpenTUI `Completed baseline:` list:

```md
- `hiero admin` and `hiero config` define responsive behavior for 80x24
  terminals and narrow tmux splits, using compact single-pane fallbacks instead
  of fixed-width broken panel layouts.
```

Remove this bullet from OpenTUI `Remaining work:`:

```md
- Define responsive behavior at 80x24 and narrow tmux splits. Fixed wide
  layouts should collapse to a single-pane or drill-down fallback rather than
  rendering broken panels.
```

- [x] **Step 2: Run focused frontend tests**

Run:

```bash
bun test --cwd frontend src/ui/responsive.test.ts src/config/ConfigScreen.test.tsx src/admin/AdminScreen.test.tsx
```

Expected: PASS.

- [x] **Step 3: Run full verification**

Run:

```bash
bun test --cwd frontend
bun run --cwd frontend typecheck
bun run --cwd frontend build
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: all commands pass.

- [x] **Step 4: Commit**

```bash
git add docs/roadmap.md
git commit -m "docs: register opentui responsive layout completion"
```

---

## Implementation Notes

- Keep the first implementation conservative. Do not redesign admin workflows or introduce new backend RPC methods.
- Do not add frontend domain mutation logic. The compact views must call the same handlers as wide mode.
- Use `useTerminalDimensions()` in screens, not global process width reads, so resize events re-render naturally through OpenTUI React.
- Preserve existing wide-mode tests and behavior.
- If a rendered text line overflows in tests, prefer truncating secondary context such as path lists or status summaries over hiding primary controls.
- The `narrow` layout may share the compact layout. The important rule is that it does not attempt the fixed wide layout.

## Self-Review

- Spec coverage: The plan addresses 80x24, narrow split behavior, too-small fallback, admin/config screens, overlay width, tests, and roadmap registration.
- Placeholder scan: No task contains TBD or open-ended implementation placeholders; every code step includes concrete snippets and commands.
- Type consistency: The plan introduces `TerminalLayout`, `classifyTerminalLayout`, `panelWidth`, and `panelHeight` once and uses the same names in later tasks.
