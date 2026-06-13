# OpenTUI Admin Command Palette And Help Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert the admin TUI command palette from a static list into a keyboard-driven action picker and add a `?` help surface with context-aware footer hints.

**Architecture:** Python remains the source of admin domain metadata and exposes command descriptors next to existing view metadata. React/OpenTUI owns local focus, palette selection, and help rendering, but all mutations and inspections continue through `AdminBridge` RPC methods or existing dialog flows. The first implementation covers the current admin action surface without adding fuzzy search or responsive layout changes.

**Tech Stack:** Python 3.12, pytest, TypeScript, React 19, Bun 1.3, OpenTUI React, Zod, existing `AdminBridge`, `AdminScreen`, `CommandPalette`, `KeyHelp`, and `AdminScreen.test.tsx`.

---

## Current Code Map

- `src/hieronymus/admin.py`
  - Owns `ADMIN_VIEWS`, `ADMIN_VIEW_KEYS`, `ADMIN_VIEW_LABELS`, and `admin_view_options()`.
  - Does not expose command/action metadata yet.
- `src/hieronymus/tui_bridge/admin_api.py`
  - `AdminBridge.bootstrap()` already returns `view_keys`, `view_labels`, and `view_options`.
  - Action methods already exist for mutations and inspections: `reinforce_crystal`, `decay_crystal`, `delete_crystal`, `approve_proposal`, `reject_proposal`, `provenance`, `recall_reasons`, `run_manual_dreaming`, and `dream_review`.
- `frontend/src/rpc/schema.ts`
  - `AdminBootstrapSchema` parses view/status/config payloads.
  - No command descriptor schema exists.
- `frontend/src/admin/CommandPalette.tsx`
  - Static `COMMANDS` map renders text only.
  - It cannot select, disable, or execute actions.
- `frontend/src/admin/AdminScreen.tsx`
  - `ctrl+p` toggles a static palette.
  - Direct hotkeys already run actions or open dialogs.
  - `?` does not open help.
  - `KeyHelp` is static and not mode-aware.
- `frontend/src/admin/AdminScreen.test.tsx`
  - Has an OpenTUI test harness with `mockInput.pressKey()`, rendered-frame assertions, and fake RPC clients.
  - The installed OpenTUI test type is `pressKey(key, modifiers?)`, where modifiers can include `ctrl` and `shift`.
  - Existing tests cover hotkeys, direct mutations, view loading, and static command text.
- `docs/roadmap.md`
  - OpenTUI remaining work includes a real keyboard picker, `?` help, and context-aware footer hints.
- `docs/usage.md`
  - Documents a command palette that is broader than the current static implementation.

## Scope Boundary

This slice finishes the command palette and help-surface roadmap bullets. It does not implement fuzzy filtering, responsive 80x24 layout collapse, full markdown replacement, scrollbars, or React/OpenTUI warning cleanup. Those remain separate OpenTUI polish work.

---

### Task 1: Add Python-Owned Admin Command Metadata

**Files:**
- Modify: `src/hieronymus/admin.py`
- Modify: `src/hieronymus/tui_bridge/admin_api.py`
- Test: `tests/test_tui_bridge_admin.py`

- [ ] **Step 1: Write the backend contract test**

In `tests/test_tui_bridge_admin.py`, extend `test_admin_bootstrap_returns_views_stats_and_initial_snapshot()` with these assertions after the existing `view_options`/view-label assertions:

```python
    command_ids = [command["id"] for command in payload["command_options"]]
    assert "reinforce_crystal" in command_ids
    assert "run_manual_dreaming" in command_ids
    reinforce = next(
        command for command in payload["command_options"] if command["id"] == "reinforce_crystal"
    )
    assert reinforce == {
        "id": "reinforce_crystal",
        "label": "Reinforce Crystal",
        "hint": "Increase strength/confidence for the selected crystal or lesson.",
        "key": "+",
        "group": "Memory",
        "views": ["Crystals", "Lessons"],
        "requires_selection": True,
    }
```

- [ ] **Step 2: Run the backend contract test**

```bash
uv run pytest tests/test_tui_bridge_admin.py::test_admin_bootstrap_returns_views_stats_and_initial_snapshot -q
```

Expected: FAIL because `command_options` is not present.

- [ ] **Step 3: Add command metadata helpers**

In `src/hieronymus/admin.py`, add this constant after `ADMIN_LABEL_VIEW_KEYS`:

```python
ADMIN_COMMANDS = (
    {
        "id": "add_memory",
        "label": "Add Memory",
        "hint": "Create a new crystal in the current memory view.",
        "key": "a",
        "group": "Memory",
        "views": ("Crystals", "Lessons"),
        "requires_selection": False,
    },
    {
        "id": "edit_memory",
        "label": "Edit Memory",
        "hint": "Edit the selected crystal or lesson text.",
        "key": "e",
        "group": "Memory",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "delete_selected",
        "label": "Delete Selected",
        "hint": "Delete or archive the selected row after confirmation.",
        "key": "d",
        "group": "Memory",
        "views": ("Concepts", "Crystals", "Lessons", "Short-Term Sessions"),
        "requires_selection": True,
    },
    {
        "id": "merge_selected",
        "label": "Merge Selected",
        "hint": "Merge the selected concept or crystal into another item.",
        "key": "m",
        "group": "Memory",
        "views": ("Concepts", "Crystals"),
        "requires_selection": True,
    },
    {
        "id": "split_crystal",
        "label": "Split Crystal",
        "hint": "Split the selected crystal or lesson into two memories.",
        "key": "s",
        "group": "Memory",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "reinforce_crystal",
        "label": "Reinforce Crystal",
        "hint": "Increase strength/confidence for the selected crystal or lesson.",
        "key": "+",
        "group": "Memory",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "decay_crystal",
        "label": "Decay Crystal",
        "hint": "Decrease strength/confidence for the selected crystal or lesson.",
        "key": "-",
        "group": "Memory",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "approve_proposal",
        "label": "Approve Proposal",
        "hint": "Approve the selected compatibility proposal.",
        "key": "a",
        "group": "Proposals",
        "views": ("Proposals",),
        "requires_selection": True,
    },
    {
        "id": "reject_proposal",
        "label": "Reject Proposal",
        "hint": "Reject the selected compatibility proposal.",
        "key": "x",
        "group": "Proposals",
        "views": ("Proposals",),
        "requires_selection": True,
    },
    {
        "id": "inspect_provenance",
        "label": "Inspect Provenance",
        "hint": "Load provenance for the selected crystal or lesson.",
        "key": "p",
        "group": "Inspect",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "inspect_recall_reasons",
        "label": "Inspect Recall Reasons",
        "hint": "Load recall reason data for the selected crystal or lesson.",
        "key": "r",
        "group": "Inspect",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "run_manual_dreaming",
        "label": "Run Manual Dreaming",
        "hint": "Run dreaming manually and select the resulting dream run.",
        "key": "D",
        "group": "Dreaming",
        "views": ("Dream Runs",),
        "requires_selection": False,
    },
    {
        "id": "review_dream_output",
        "label": "Review Dream Output",
        "hint": "Load the review payload for the selected dream run.",
        "key": "enter",
        "group": "Dreaming",
        "views": ("Dream Runs",),
        "requires_selection": True,
    },
)
```

Below `admin_view_options()`, add:

```python
def admin_command_options() -> list[dict[str, object]]:
    return [
        {
            "id": str(command["id"]),
            "label": str(command["label"]),
            "hint": str(command["hint"]),
            "key": str(command["key"]),
            "group": str(command["group"]),
            "views": list(command["views"]),
            "requires_selection": bool(command["requires_selection"]),
        }
        for command in ADMIN_COMMANDS
    ]
```

- [ ] **Step 4: Include commands in bootstrap**

In `src/hieronymus/tui_bridge/admin_api.py`, import `admin_command_options` from `hieronymus.admin` and add this key to `AdminBridge.bootstrap()` next to `view_options`:

```python
            "command_options": admin_command_options(),
```

- [ ] **Step 5: Run backend tests**

```bash
uv run pytest tests/test_tui_bridge_admin.py::test_admin_bootstrap_returns_views_stats_and_initial_snapshot -q
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/admin.py src/hieronymus/tui_bridge/admin_api.py tests/test_tui_bridge_admin.py
git commit -m "feat: expose admin command metadata"
```

---

### Task 2: Parse Command Metadata In Frontend Runtime Schemas

**Files:**
- Modify: `frontend/src/rpc/schema.ts`
- Modify: `frontend/src/rpc/schema.test.ts`

- [ ] **Step 1: Write the frontend schema test**

In `frontend/src/rpc/schema.test.ts`, inside `it("parses admin bootstrap status and config contracts", ...)`, add this property to the parsed payload:

```ts
      command_options: [
        {
          id: "reinforce_crystal",
          label: "Reinforce Crystal",
          hint: "Increase strength/confidence for the selected crystal or lesson.",
          key: "+",
          group: "Memory",
          views: ["Crystals", "Lessons"],
          requires_selection: true,
        },
      ],
```

Then add this assertion after the existing config warning assertions:

```ts
    expect(payload.command_options[0]).toMatchObject({
      id: "reinforce_crystal",
      key: "+",
      views: ["Crystals", "Lessons"],
      requires_selection: true,
    });
```

- [ ] **Step 2: Run the schema test**

```bash
bun --cwd frontend test src/rpc/schema.test.ts
```

Expected: FAIL because `command_options` is not parsed.

- [ ] **Step 3: Add command schemas**

In `frontend/src/rpc/schema.ts`, add this schema after `AdminHeaderSchema`:

```ts
export const AdminCommandSchema = z
  .object({
    id: z.string(),
    label: z.string(),
    hint: z.string(),
    key: z.string(),
    group: z.string(),
    views: z.array(z.string()),
    requires_selection: z.boolean(),
  })
  .passthrough();
```

In `AdminBootstrapSchema`, add:

```ts
  command_options: z.array(AdminCommandSchema).default([]),
```

Near the existing exported types, add:

```ts
export type AdminCommand = z.infer<typeof AdminCommandSchema>;
```

- [ ] **Step 4: Run the schema test**

```bash
bun --cwd frontend test src/rpc/schema.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/rpc/schema.ts frontend/src/rpc/schema.test.ts
git commit -m "feat: parse admin command metadata"
```

---

### Task 3: Build The Keyboard-Driven Command Palette

**Files:**
- Modify: `frontend/src/admin/CommandPalette.tsx`
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Modify: `frontend/src/admin/AdminScreen.test.tsx`

- [ ] **Step 1: Replace the static palette component**

Replace `frontend/src/admin/CommandPalette.tsx` with:

```tsx
import React from "react";
import type { AdminCommand } from "../rpc/schema.js";

export function commandsForView(
  commands: AdminCommand[],
  view: string,
  hasSelection: boolean,
): Array<AdminCommand & { disabled: boolean }> {
  return commands
    .filter((command) => command.views.includes(view))
    .map((command) => ({
      ...command,
      disabled: command.requires_selection && !hasSelection,
    }));
}

export function CommandPalette({
  commands,
  selectedIndex,
}: {
  commands: Array<AdminCommand & { disabled: boolean }>;
  selectedIndex: number;
}) {
  return (
    <box
      flexDirection="column"
      borderStyle="rounded"
      borderColor="cyan"
      paddingX={1}
      paddingY={1}
    >
      <text fg="cyan">Command Palette</text>
      {commands.length === 0 ? <text fg="gray">No commands for this view</text> : null}
      {commands.map((command, index) => (
        <text
          key={command.id}
          fg={command.disabled ? "gray" : index === selectedIndex ? "cyan" : undefined}
        >
          {index === selectedIndex ? "> " : "  "}
          {command.label} [{command.key}] {command.disabled ? "(unavailable)" : ""}
        </text>
      ))}
      {commands[selectedIndex] ? (
        <text fg="gray">{commands[selectedIndex].hint}</text>
      ) : null}
      <text fg="gray">Enter run  Esc close  ↑/↓ or j/k move</text>
    </box>
  );
}
```

- [ ] **Step 2: Add test payload command options**

In `frontend/src/admin/AdminScreen.test.tsx`, add `command_options` to `bootstrap()` next to `view_options`/`default_view` data:

```ts
    command_options: [
      {
        id: "add_memory",
        label: "Add Memory",
        hint: "Create a new crystal in the current memory view.",
        key: "a",
        group: "Memory",
        views: ["Crystals", "Lessons"],
        requires_selection: false,
      },
      {
        id: "edit_memory",
        label: "Edit Memory",
        hint: "Edit the selected crystal or lesson text.",
        key: "e",
        group: "Memory",
        views: ["Crystals", "Lessons"],
        requires_selection: true,
      },
      {
        id: "reinforce_crystal",
        label: "Reinforce Crystal",
        hint: "Increase strength/confidence for the selected crystal or lesson.",
        key: "+",
        group: "Memory",
        views: ["Crystals", "Lessons"],
        requires_selection: true,
      },
      {
        id: "decay_crystal",
        label: "Decay Crystal",
        hint: "Decrease strength/confidence for the selected crystal or lesson.",
        key: "-",
        group: "Memory",
        views: ["Crystals", "Lessons"],
        requires_selection: true,
      },
      {
        id: "approve_proposal",
        label: "Approve Proposal",
        hint: "Approve the selected compatibility proposal.",
        key: "a",
        group: "Proposals",
        views: ["Proposals"],
        requires_selection: true,
      },
      {
        id: "run_manual_dreaming",
        label: "Run Manual Dreaming",
        hint: "Run dreaming manually and select the resulting dream run.",
        key: "D",
        group: "Dreaming",
        views: ["Dream Runs"],
        requires_selection: false,
      },
    ],
```

- [ ] **Step 3: Expose direct key presses in the test harness**

In `frontend/src/admin/AdminScreen.test.tsx`, update the `input` object inside `setupTest()` so it has both `type()` and `press()`:

```ts
  const input = {
    type: async (value: string) => {
      const current = await ensureSetup();
      for (const key of value) {
        act(() => {
          current.mockInput.pressKey(key);
        });
      }
      await flush();
    },
    press: async (name: string, options: { ctrl?: boolean; shift?: boolean } = {}) => {
      const current = await ensureSetup();
      act(() => {
        current.mockInput.pressKey(name, options);
      });
      await flush();
    },
  };
```

- [ ] **Step 4: Write palette interaction tests**

In `frontend/src/admin/AdminScreen.test.tsx`, replace `it("shows crystal commands for crystal view", ...)` with:

```tsx
  it("opens a keyboard command palette with context commands", async () => {
    const { root, flush, captureCharFrame } = await setupTest();

    root.render(
      <AdminScreen initial={bootstrap()} client={undefined} showCommands />,
    );
    await flush();

    const output = captureCharFrame();
    expect(output).toContain("Command Palette");
    expect(output).toContain("> Add Memory [a]");
    expect(output).toContain("Reinforce Crystal [+]");
    expect(output).not.toContain("Approve Proposal");
  });
```

Add this new test after it:

```tsx
  it("runs selected command palette actions through existing RPC handlers", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: {
          ...bootstrap().snapshot,
          detail: {
            ...bootstrap().snapshot.detail,
            body: "Palette reinforcement marker.",
          },
        },
      });
    });
    const { root, mockInput, flush, captureCharFrame, waitFor } =
      await setupTest();

    root.render(<AdminScreen initial={bootstrap()} client={client} />);
    await flush();

    await mockInput.press("p", { ctrl: true });
    await mockInput.press("j");
    await mockInput.press("j");
    await mockInput.press("enter");

    await waitFor(async () => captureCharFrame().includes("Palette reinforcement marker."));

    expect(calls).toEqual([
      {
        method: "admin.reinforce_crystal",
        params: { id: 1, view: "Crystals" },
      },
    ]);
  });
```

- [ ] **Step 5: Add AdminScreen command execution helpers**

In `frontend/src/admin/AdminScreen.tsx`, update imports:

```ts
import { CommandPalette, commandsForView } from "./CommandPalette.js";
```

Also import `AdminCommand` from `../rpc/schema.js` with the other type imports:

```ts
  type AdminCommand,
```

After `const viewKeyLimit = ...`, add:

```ts
  const paletteCommands = commandsForView(
    initial.command_options,
    snapshot.view,
    Boolean(snapshot.selected),
  );
  const [selectedCommandIndex, setSelectedCommandIndex] = useState(0);
```

Add this helper inside `AdminScreen`, above `handleInput`:

```ts
  const clampCommandIndex = (index: number) =>
    Math.min(Math.max(index, 0), Math.max(paletteCommands.length - 1, 0));
```

Add this helper above `handleInput`:

```ts
  const executeCommand = (command: AdminCommand | undefined) => {
    if (!command) {
      return;
    }
    if (command.requires_selection && !snapshot.selected) {
      setStatus({ message: `${command.label} needs a selected row`, error: true });
      return;
    }
    setCommandsOpen(false);
    if (command.id === "add_memory") {
      setDialog({ kind: "add", error: "" });
      return;
    }
    if (command.id === "edit_memory") {
      handleInput("e");
      return;
    }
    if (command.id === "delete_selected") {
      handleInput("d");
      return;
    }
    if (command.id === "merge_selected") {
      handleInput("m");
      return;
    }
    if (command.id === "split_crystal") {
      handleInput("s");
      return;
    }
    if (command.id === "reinforce_crystal") {
      handleInput("+");
      return;
    }
    if (command.id === "decay_crystal") {
      handleInput("-");
      return;
    }
    if (command.id === "approve_proposal") {
      runSelectedSnapshotCommand("admin.approve_proposal", "Approved proposal");
      return;
    }
    if (command.id === "reject_proposal") {
      runSelectedSnapshotCommand("admin.reject_proposal", "Rejected proposal");
      return;
    }
    if (command.id === "run_manual_dreaming") {
      runSnapshotCommand("admin.run_manual_dreaming", {}, "Ran manual dreaming");
      return;
    }
    if (command.id === "review_dream_output") {
      runSelectedSnapshotCommand("admin.dream_review", "Loaded dream review");
      return;
    }
    if (command.id === "inspect_provenance") {
      runInspectionCommand("admin.provenance", "Loaded provenance");
      return;
    }
    if (command.id === "inspect_recall_reasons") {
      runInspectionCommand("admin.recall_reasons", "Loaded recall reasons");
      return;
    }
  };
```

Add these helpers above `executeCommand()`:

```ts
  const runSnapshotCommand = (
    method: string,
    params: Record<string, unknown>,
    successMessage: string,
  ) => {
    if (!client || operationInFlight.current) {
      return;
    }
    void runSnapshotOperation({
      client,
      method,
      params: { ...params, view: snapshot.view, filters: snapshot.filters },
      successMessage,
      setSnapshot,
      setStats,
      setShortTermStatus,
      setDreamStatus,
      setConfigEditor,
      setStatus,
      operationInFlight,
    });
  };

  const runSelectedSnapshotCommand = (method: string, successMessage: string) => {
    const selectedId = snapshot.selected?.id;
    if (selectedId === undefined) {
      setStatus({ message: "No row selected", error: true });
      return;
    }
    runSnapshotCommand(method, { id: selectedId }, successMessage);
  };

  const runInspectionCommand = (method: string, successMessage: string) => {
    const selectedId = snapshot.selected?.id;
    if (!client || operationInFlight.current || selectedId === undefined) {
      return;
    }
    operationInFlight.current = true;
    setStatus({ message: `Working: ${successMessage}`, error: false });
    void client
      .request(method, { id: selectedId })
      .then((response) => {
        const detail = inspectionDetail(method, response);
        setSnapshot((current) => ({
          ...current,
          detail,
        }));
        setStatus({ message: successMessage, error: false });
      })
      .catch((error) => {
        setStatus({
          message: error instanceof Error ? error.message : String(error),
          error: true,
        });
      })
      .finally(() => {
        operationInFlight.current = false;
      });
  };
```

Add this file-level helper below `runSnapshotOperation()`:

```ts
function inspectionDetail(method: string, response: Record<string, unknown>) {
  if (method === "admin.provenance") {
    return {
      title: "Provenance",
      subtitle: "admin.provenance",
      body: JSON.stringify(response.provenance ?? {}, null, 2),
      fields: [],
    };
  }
  return {
    title: "Recall Reasons",
    subtitle: "admin.recall_reasons",
    body: JSON.stringify(response.reasons ?? [], null, 2),
    fields: [],
  };
}
```

- [ ] **Step 6: Route palette keyboard input**

In `useKeyboard()`, before panel navigation, add:

```ts
    if (commandsOpen) {
      if (key.name === "escape") {
        setCommandsOpen(false);
        return;
      }
      if (key.name === "down" || key.name === "j") {
        setSelectedCommandIndex((index) => clampCommandIndex(index + 1));
        return;
      }
      if (key.name === "up" || key.name === "k") {
        setSelectedCommandIndex((index) => clampCommandIndex(index - 1));
        return;
      }
      if (key.name === "enter" || key.name === "return") {
        const command = paletteCommands[clampCommandIndex(selectedCommandIndex)];
        executeCommand(command);
        return;
      }
    }
```

Update the `ctrl+p` branch in `handleInput()`:

```ts
    if (ctrl && input === "p") {
      setSelectedCommandIndex(0);
      setCommandsOpen((open) => !open);
      return;
    }
```

Update the render call:

```tsx
          {commandsOpen ? (
            <CommandPalette
              commands={paletteCommands}
              selectedIndex={clampCommandIndex(selectedCommandIndex)}
            />
          ) : null}
```

- [ ] **Step 7: Run frontend admin tests**

```bash
bun --cwd frontend test src/admin/AdminScreen.test.tsx
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/admin/AdminScreen.tsx frontend/src/admin/AdminScreen.test.tsx frontend/src/admin/CommandPalette.tsx
git commit -m "feat: add executable admin command palette"
```

---

### Task 4: Add `?` Help Surface And Context-Aware Footer Hints

**Files:**
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Modify: `frontend/src/admin/AdminScreen.test.tsx`
- Create: `frontend/src/admin/HelpOverlay.tsx`

- [ ] **Step 1: Add the help overlay component**

Create `frontend/src/admin/HelpOverlay.tsx`:

```tsx
import React from "react";
import type { AdminCommand } from "../rpc/schema.js";

export function HelpOverlay({
  commands,
  view,
}: {
  commands: AdminCommand[];
  view: string;
}) {
  const visibleCommands = commands.filter((command) =>
    command.views.includes(view),
  );
  return (
    <box
      flexDirection="column"
      borderStyle="rounded"
      borderColor="cyan"
      paddingX={1}
      paddingY={1}
      width={58}
    >
      <text fg="cyan">Help</text>
      <text>Navigation</text>
      <text fg="gray">Tab/Shift+Tab focus panels  ↑/↓ or j/k move  1-9 switch views</text>
      <text fg="gray">Ctrl+P command palette  ? help  Esc close  q quit</text>
      <text>Commands for {view}</text>
      {visibleCommands.length === 0 ? <text fg="gray">No commands for this view</text> : null}
      {visibleCommands.map((command) => (
        <text key={command.id}>
          [{command.key}] {command.label} - {command.hint}
        </text>
      ))}
    </box>
  );
}
```

- [ ] **Step 2: Add the help test**

In `frontend/src/admin/AdminScreen.test.tsx`, add:

```tsx
  it("opens contextual help with question mark and closes with escape", async () => {
    const { root, mockInput, flush, captureCharFrame, waitFor } =
      await setupTest();

    root.render(<AdminScreen initial={bootstrap()} client={undefined} />);
    await flush();

    await mockInput.type("?");
    await waitFor(async () => captureCharFrame().includes("Help"));

    let output = captureCharFrame();
    expect(output).toContain("Ctrl+P command palette");
    expect(output).toContain("[+] Reinforce Crystal");
    expect(output).not.toContain("Approve Proposal");

    await mockInput.press("escape");
    await waitFor(async () => !captureCharFrame().includes("Help"));
  });
```

- [ ] **Step 3: Wire help state and mode-aware footer**

In `frontend/src/admin/AdminScreen.tsx`, import:

```ts
import { HelpOverlay } from "./HelpOverlay.js";
```

Add state:

```ts
  const [helpOpen, setHelpOpen] = useState(false);
```

In `useKeyboard()`, before the `commandsOpen` block, add:

```ts
    if (helpOpen) {
      if (key.name === "escape" || key.name === "?") {
        setHelpOpen(false);
      }
      return;
    }

    if (key.name === "?") {
      setHelpOpen(true);
      setCommandsOpen(false);
      return;
    }
```

In the `ctrl+p` branch, close help:

```ts
      setHelpOpen(false);
```

In the right detail pane render, add:

```tsx
          {helpOpen ? (
            <HelpOverlay commands={initial.command_options} view={snapshot.view} />
          ) : null}
```

Replace the `KeyHelp` keys array with a mode-aware helper:

```tsx
      <KeyHelp keys={footerKeys({ commandsOpen, helpOpen, viewKeyLimit })} />
```

Add this file-level helper:

```ts
function footerKeys({
  commandsOpen,
  helpOpen,
  viewKeyLimit,
}: {
  commandsOpen: boolean;
  helpOpen: boolean;
  viewKeyLimit: number;
}) {
  if (helpOpen) {
    return ["Esc close help", "q quit"];
  }
  if (commandsOpen) {
    return ["↑/↓ move", "Enter run", "Esc close", "? help"];
  }
  return [
    "Tab focus",
    `1-${viewKeyLimit} view`,
    "↑/↓ or j/k move",
    "Ctrl+P commands",
    "? help",
    "q quit",
  ];
}
```

- [ ] **Step 4: Run frontend admin tests**

```bash
bun --cwd frontend test src/admin/AdminScreen.test.tsx
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/admin/AdminScreen.tsx frontend/src/admin/AdminScreen.test.tsx frontend/src/admin/HelpOverlay.tsx
git commit -m "feat: add admin help overlay"
```

---

### Task 5: Update Docs And Roadmap

**Files:**
- Modify: `docs/roadmap.md`
- Modify: `docs/usage.md`

- [ ] **Step 1: Update roadmap OpenTUI completed baseline**

In `docs/roadmap.md`, add these bullets to the OpenTUI Management App `Completed baseline:` list:

```markdown
- `hiero admin` receives Python-owned command metadata and renders a real
  keyboard command palette with availability state and bridge-backed execution.
- `hiero admin` has a `?` help surface and context-aware footer hints for
  normal, command-palette, and help modes.
```

- [ ] **Step 2: Remove completed OpenTUI remaining bullets**

In the OpenTUI Management App `Remaining work:` list, remove:

```markdown
- Convert the command palette from a static command list into a real keyboard
  picker with command names, keybindings, availability state, and bridge-backed
  execution.
- Add a `?` help surface and keep footer hints context-aware.
```

Leave the other OpenTUI remaining bullets in place.

- [ ] **Step 3: Update usage docs**

In `docs/usage.md`, replace the Admin TUI key list with:

```markdown
### Admin TUI Keys

- `Tab` / `Shift+Tab`: cycle focus between views, table, and detail
- `1`-`9`: switch views
- `↑` / `↓` or `j` / `k`: move within the focused list or command palette
- `ctrl+p`: open the command palette
- `?`: open contextual help
- `Enter`: run the selected command while the command palette is open
- `Esc`: close help, command palette, or dialogs
- `q`: quit
```

Update the command palette paragraph so it says:

```markdown
The command palette is backed by the same Python command metadata that powers
the bridge. It shows only commands relevant to the current view, marks commands
that need a selected row as unavailable, and executes mutations or inspections
through `AdminBridge` RPC methods rather than parsing CLI output.
```

- [ ] **Step 4: Commit docs**

```bash
git add docs/roadmap.md docs/usage.md
git commit -m "docs: register admin command palette help"
```

---

### Task 6: Final Verification And PR

**Files:**
- Verify only

- [ ] **Step 1: Run targeted Python and frontend tests**

```bash
uv run pytest tests/test_tui_bridge_admin.py -q
bun --cwd frontend test src/rpc/schema.test.ts src/admin/AdminScreen.test.tsx
```

Expected: PASS.

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

Expected: PASS. Existing React/OpenTUI warnings may still appear in Bun tests; do not hide or globally mute them in this slice.

- [ ] **Step 3: Push and open PR**

```bash
git push -u origin plan/opentui-command-palette-help
gh pr create --fill
```

Expected: GitHub opens a PR from `plan/opentui-command-palette-help` into `main`.

---

## Self-Review

Spec coverage:

- Real command palette: Tasks 1-3 replace static strings with Python-owned command descriptors, Zod parsing, keyboard selection, availability state, and execution.
- Bridge-backed execution: Task 3 routes mutations/inspections through existing `AdminBridge` RPC methods or existing dialog flows.
- `?` help surface and context-aware footer: Task 4 adds both and tests mode changes.
- Roadmap/docs registration: Task 5 moves only completed OpenTUI bullets and updates usage keys.
- No unrelated OpenTUI work: responsive collapse, markdown replacement, scrollbar support, and warning cleanup remain outside this slice.

Completion-marker scan:

- No deferred-work markers are present.
- Every task lists exact files, code snippets, commands, and expected outcomes.

Type consistency:

- Backend command payload keys are `id`, `label`, `hint`, `key`, `group`, `views`, and `requires_selection`.
- Frontend `AdminCommandSchema` and `AdminCommand` use the same field names.
- Palette filtering uses `command.views.includes(snapshot.view)` and `command.requires_selection`.
