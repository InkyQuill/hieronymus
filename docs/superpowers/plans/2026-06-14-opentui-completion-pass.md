# OpenTUI Completion Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the current OpenTUI roadmap slice in one pass: schema-extensible config editing, convention compliance checks, keyboard-first navigation/search, dense admin detail rendering, full markdown rendering, and multiline editor scrollbar detection.

**Architecture:** Keep frontend work presentation-only. Python `AdminBridge` and `ConfigBridge` continue to own domain mutation, persistence, validation, migrations, and audited methods. Add small frontend utilities for keyboard semantics and markdown rendering, and add backend/frontend tests that enforce the bridge-boundary conventions documented in `docs/opentui-conventions.md`.

**Tech Stack:** Python 3.12, pytest, TypeScript, React 19, OpenTUI React 0.4.0, Bun 1.3.14 test runner, existing OpenTUI renderer harness.

---

## Current Code Map

- `docs/opentui-conventions.md`
  - New convention document for bridge boundaries, config editing, admin inspection, keyboard, and layout.
- `docs/roadmap.md`
  - Roadmap now keeps deliverables only; standing “keep/ensure” rules moved to conventions.
- `src/hieronymus/tui_bridge/config_api.py`
  - `ConfigBridge` currently builds one hardcoded form schema for dream, ingest, and release settings.
  - Save/update/check methods already load and persist `dream.conf`, `ingest.conf`, and `release.conf`.
- `frontend/src/config/ConfigScreen.tsx`
  - Renders schema-driven fields from bridge payloads, but the schema is still shaped around fixed sections.
  - Supports arrows and basic hotkeys, but not `hjkl` or `/` search.
- `src/hieronymus/tui_bridge/admin_api.py`
  - `AdminBridge` owns snapshots, mutations, provenance, recall reasons, dream review, and manual dreaming.
- `frontend/src/admin/AdminScreen.tsx`
  - Calls backend JSON-RPC methods for snapshots and mutations.
  - Has command palette/help/responsive layouts.
  - Does not yet provide local `/` search or full `hjkl` panel navigation.
- `frontend/src/admin/DetailPane.tsx`
  - Detects JSON and diffs, otherwise uses a lightweight markdown renderer.
- `frontend/src/admin/dialogs.tsx`
  - Uses `TextAreaInput` for multiline memory text; visible scrollbar support depends on current OpenTUI component props.
- `frontend/src/ui/TextInput.tsx`
  - Wraps OpenTUI `input` and `textarea`.

## Compliance Baseline

Verified by inspection before writing this plan:

- `frontend/src` does not import SQLite or backend domain stores.
- Runtime frontend backend access is through `frontend/src/rpc/client.ts` and JSON-RPC methods such as `admin.snapshot`, `admin.*`, and `config.*`.
- `ConfigBridge` persists config files through Python config modules.
- `AdminBridge` performs admin mutations through Python stores and domain methods.

This plan adds tests to keep those conventions from regressing.

## File Structure

- Create: `tests/test_opentui_conventions.py`
  - Static convention checks for frontend/backend boundaries.
- Modify: `docs/opentui-conventions.md`
  - Add any conventions discovered during implementation.
- Modify: `src/hieronymus/tui_bridge/config_api.py`
  - Refactor config form schema into config sections so future config files can be added without touching frontend code.
- Modify: `frontend/src/rpc/schema.ts`
  - Parse section/group metadata needed by extensible config rendering.
- Modify: `frontend/src/rpc/schema.test.ts`
  - Schema coverage for section metadata passthrough/defaults.
- Modify: `frontend/src/config/ConfigScreen.tsx`
  - Render config sections from payload metadata, add keyboard navigation/search.
- Modify: `frontend/src/config/ConfigForm.tsx`
  - Render section/group labels from schema metadata where present.
- Modify: `frontend/src/config/ConfigScreen.test.tsx`
  - Config section, keyboard, and search tests.
- Create: `frontend/src/ui/keyboard.ts`
  - Shared semantic key helpers.
- Test: `frontend/src/ui/keyboard.test.ts`
  - Unit tests for key helper semantics.
- Create: `frontend/src/admin/markdown.tsx`
  - Fuller markdown block renderer for detail panes.
- Test: `frontend/src/admin/markdown.test.tsx`
  - Markdown rendering coverage through the OpenTUI harness.
- Modify: `frontend/src/admin/DetailPane.tsx`
  - Use markdown renderer, preserve JSON/diff/code paths.
- Modify: `frontend/src/admin/AdminScreen.tsx`
  - Add keyboard navigation/search and dense detail affordances.
- Modify: `frontend/src/admin/AdminScreen.test.tsx`
  - Admin keyboard/search/detail rendering tests.
- Modify: `frontend/src/admin/dialogs.tsx`
  - Detect/support visible textarea scrollbar props if OpenTUI exposes them; otherwise document unsupported state in code/tests.
- Modify: `frontend/src/ui/TextInput.tsx`
  - Pass textarea scrollbar props only if supported by local type/runtime.
- Modify: `docs/roadmap.md`
  - Move completed deliverables from remaining work to completed baseline.

---

### Task 1: Convention Compliance Tests

**Files:**
- Create: `tests/test_opentui_conventions.py`
- Modify: `docs/opentui-conventions.md`

- [ ] **Step 1: Write failing/static convention tests**

Create `tests/test_opentui_conventions.py`:

```python
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FRONTEND_SRC = ROOT / "frontend" / "src"


def _frontend_sources() -> list[Path]:
    return sorted(FRONTEND_SRC.rglob("*.ts")) + sorted(FRONTEND_SRC.rglob("*.tsx"))


def test_frontend_does_not_import_sqlite_or_domain_stores() -> None:
    forbidden = (
        "sqlite",
        "better-sqlite",
        "from \"../../src/",
        "from '../src/",
        "from \"../src/",
        "hieronymus.db",
    )
    offenders: list[str] = []
    for path in _frontend_sources():
        text = path.read_text(encoding="utf-8")
        if any(token in text for token in forbidden):
            offenders.append(str(path.relative_to(ROOT)))
    assert offenders == []


def test_frontend_does_not_parse_human_hiero_cli_output() -> None:
    offenders: list[str] = []
    for path in _frontend_sources():
        text = path.read_text(encoding="utf-8")
        if " h i e r o " in text.replace("\"", " ").replace("'", " "):
            offenders.append(str(path.relative_to(ROOT)))
        if "tui-bridge" in text and path.name != "client.ts":
            offenders.append(str(path.relative_to(ROOT)))
    assert offenders == []


def test_tui_mutation_methods_are_registered_on_python_bridges() -> None:
    server = (ROOT / "src" / "hieronymus" / "tui_bridge" / "server.py").read_text(
        encoding="utf-8",
    )
    assert "AdminBridge(config)" in server
    assert "ConfigBridge(config)" in server
    for method in (
        "admin.add_crystal",
        "admin.edit_crystal",
        "admin.delete_crystal",
        "admin.run_manual_dreaming",
        "config.update_draft",
        "config.save",
        "config.check_provider",
    ):
        assert method in server
```

- [ ] **Step 2: Run the convention tests**

Run:

```bash
uv run pytest tests/test_opentui_conventions.py -q
```

Expected: PASS if current code is compliant. If this fails, fix the convention test only when it is too broad; fix code only for a real convention violation.

- [ ] **Step 3: Update conventions if needed**

If the tests reveal a legitimate convention nuance, update `docs/opentui-conventions.md` with precise language. Do not add broad policy prose that is not enforceable or useful.

- [ ] **Step 4: Commit**

```bash
git add tests/test_opentui_conventions.py docs/opentui-conventions.md docs/roadmap.md
git commit -m "test: enforce opentui bridge conventions"
```

---

### Task 2: Extensible Config Sections

**Files:**
- Modify: `src/hieronymus/tui_bridge/config_api.py`
- Modify: `frontend/src/rpc/schema.ts`
- Modify: `frontend/src/rpc/schema.test.ts`
- Modify: `frontend/src/config/ConfigForm.tsx`
- Modify: `frontend/src/config/ConfigScreen.tsx`
- Test: `tests/test_tui_bridge_config.py`
- Test: `frontend/src/config/ConfigScreen.test.tsx`

- [ ] **Step 1: Add backend config section tests**

Add to `tests/test_tui_bridge_config.py`:

```python
def test_config_form_schema_groups_fields_by_config_file(tmp_path: Path) -> None:
    payload = ConfigBridge(_config(tmp_path)).bootstrap({})
    schema = payload["form_schema"]

    section_ids = [section["id"] for section in schema["sections"]]
    assert section_ids == ["dream", "ingest", "release"]

    field_sections = {field["key"]: field["section"] for field in schema["fields"]}
    assert field_sections["provider.model"] == "dream"
    assert field_sections["dreaming.autostart_enabled"] == "dream"
    assert field_sections["ingest.warning_sentence_count"] == "ingest"
    assert field_sections["release.update_channel"] == "release"
```

- [ ] **Step 2: Run backend config test and verify it fails**

Run:

```bash
uv run pytest tests/test_tui_bridge_config.py::test_config_form_schema_groups_fields_by_config_file -q
```

Expected: FAIL because `sections` and field `section` metadata do not exist yet.

- [ ] **Step 3: Add schema section metadata in Python**

In `src/hieronymus/tui_bridge/config_api.py`, update `_form_schema()` so it returns:

```python
    return {
        "sections": [
            {
                "id": "dream",
                "label": "dream.conf",
                "description": "Dreaming providers, workflows, and automation.",
                "path_key": "dream_config_path",
            },
            {
                "id": "ingest",
                "label": "ingest.conf",
                "description": "Ingest limits and learning safety thresholds.",
                "path_key": "ingest_config_path",
            },
            {
                "id": "release",
                "label": "release.conf",
                "description": "Managed install update channel settings.",
                "path_key": "release_config_path",
            },
        ],
        "groups": [
            # existing groups, each with a new "section" key
        ],
        "fields": [
            # existing fields, each with a new "section" key
        ],
    }
```

Set group/field sections as:

- provider and dreaming groups/fields: `"dream"`
- ingest group/fields: `"ingest"`
- release group/fields: `"release"`

- [ ] **Step 4: Add frontend schema parsing tests**

In `frontend/src/rpc/schema.test.ts`, add:

```ts
it("parses config form schema section metadata", () => {
  const payload = ConfigBootstrapSchema.parse({
    ...configPayload,
    form_schema: {
      sections: [
        {
          id: "dream",
          label: "dream.conf",
          description: "Dreaming settings.",
          path_key: "dream_config_path",
        },
      ],
      groups: [
        {
          id: "provider",
          section: "dream",
          label: "Provider",
          description: "Provider settings.",
        },
      ],
      fields: [
        {
          key: "provider.model",
          section: "dream",
          group: "provider",
          label: "Model",
          hint: "Model name.",
          placeholder: "gpt-4.1-mini",
          type: "text",
          choices: [],
          default: "",
          redacted: false,
        },
      ],
    },
  });

  expect(payload.form_schema.sections[0]?.id).toBe("dream");
  expect(payload.form_schema.groups[0]?.section).toBe("dream");
  expect(payload.form_schema.fields[0]?.section).toBe("dream");
});
```

- [ ] **Step 5: Update Zod schemas**

In `frontend/src/rpc/schema.ts`, add:

```ts
const ConfigFormSectionSchema = z
  .object({
    id: z.string(),
    label: z.string(),
    description: z.string().default(""),
    path_key: z.string().optional(),
  })
  .passthrough();
```

Add `section: z.string().optional()` to `ConfigFormGroupSchema` and `ConfigFormFieldSchema`.

Update `ConfigFormSchemaSchema`:

```ts
export const ConfigFormSchemaSchema = z
  .object({
    sections: z.array(ConfigFormSectionSchema).default([]),
    groups: z.array(ConfigFormGroupSchema).default([]),
    fields: z.array(ConfigFormFieldSchema).default([]),
  })
  .passthrough();
```

- [ ] **Step 6: Render config sections in frontend**

In `frontend/src/config/ConfigScreen.tsx`, derive:

```ts
const sections = payload.form_schema.sections;
const activeSection = sections.find((section) =>
  formFields.some((field) => field.section === section.id),
);
```

Render section labels near the title in wide and compact modes:

```tsx
{sections.length > 0 ? (
  <text fg="gray">
    {sections.map((section) => section.label).join(" · ")}
  </text>
) : null}
```

In `ConfigForm`, optionally render group/section metadata by matching `field.group` against `form_schema.groups` if that metadata is passed in. Keep the change minimal: the main requirement is that section metadata is parsed and visible, not a full tabbed config UI.

- [ ] **Step 7: Add frontend config section render test**

In `frontend/src/config/ConfigScreen.test.tsx`, add:

```tsx
it("renders backend-owned config section labels", async () => {
  const { render, waitForFrame } = setupTest();

  await render(
    <ConfigScreen
      initial={{
        ...payload(),
        form_schema: {
          ...formSchema(),
          sections: [
            {
              id: "dream",
              label: "dream.conf",
              description: "Dreaming settings.",
              path_key: "dream_config_path",
            },
            {
              id: "ingest",
              label: "ingest.conf",
              description: "Ingest settings.",
              path_key: "ingest_config_path",
            },
            {
              id: "release",
              label: "release.conf",
              description: "Release settings.",
              path_key: "release_config_path",
            },
          ],
        },
      }}
      client={undefined}
    />,
  );

  const output = await waitForFrame((frame) => frame.includes("dream.conf"));
  expect(output).toContain("dream.conf");
  expect(output).toContain("ingest.conf");
  expect(output).toContain("release.conf");
});
```

- [ ] **Step 8: Run focused config tests**

Run:

```bash
uv run pytest tests/test_tui_bridge_config.py::test_config_form_schema_groups_fields_by_config_file -q
bun test --cwd frontend src/rpc/schema.test.ts src/config/ConfigScreen.test.tsx
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/hieronymus/tui_bridge/config_api.py tests/test_tui_bridge_config.py frontend/src/rpc/schema.ts frontend/src/rpc/schema.test.ts frontend/src/config/ConfigForm.tsx frontend/src/config/ConfigScreen.tsx frontend/src/config/ConfigScreen.test.tsx
git commit -m "feat: make config schema section-aware"
```

---

### Task 3: Keyboard Helpers And Config Navigation/Search

**Files:**
- Create: `frontend/src/ui/keyboard.ts`
- Test: `frontend/src/ui/keyboard.test.ts`
- Modify: `frontend/src/config/ConfigScreen.tsx`
- Test: `frontend/src/config/ConfigScreen.test.tsx`

- [ ] **Step 1: Add keyboard helper tests**

Create `frontend/src/ui/keyboard.test.ts`:

```ts
import { describe, expect, it } from "bun:test";
import {
  isConfirmKey,
  isDownKey,
  isEscapeKey,
  isLeftKey,
  isPrintableSearchKey,
  isRightKey,
  isUpKey,
} from "./keyboard.js";

describe("keyboard helpers", () => {
  it("maps arrows and vim keys to movement helpers", () => {
    expect(isUpKey("up")).toBe(true);
    expect(isUpKey("k")).toBe(true);
    expect(isDownKey("down")).toBe(true);
    expect(isDownKey("j")).toBe(true);
    expect(isLeftKey("left")).toBe(true);
    expect(isLeftKey("h")).toBe(true);
    expect(isRightKey("right")).toBe(true);
    expect(isRightKey("l")).toBe(true);
  });

  it("recognizes confirm and escape aliases", () => {
    expect(isConfirmKey("enter")).toBe(true);
    expect(isConfirmKey("return")).toBe(true);
    expect(isEscapeKey("escape")).toBe(true);
    expect(isEscapeKey("esc")).toBe(true);
  });

  it("accepts only printable single-character search input", () => {
    expect(isPrintableSearchKey("a")).toBe(true);
    expect(isPrintableSearchKey(" ")).toBe(true);
    expect(isPrintableSearchKey("/")).toBe(true);
    expect(isPrintableSearchKey("enter")).toBe(false);
    expect(isPrintableSearchKey("backspace")).toBe(false);
    expect(isPrintableSearchKey("")).toBe(false);
  });
});
```

- [ ] **Step 2: Run helper test and verify it fails**

Run:

```bash
bun test --cwd frontend src/ui/keyboard.test.ts
```

Expected: FAIL because `keyboard.ts` does not exist.

- [ ] **Step 3: Implement keyboard helper**

Create `frontend/src/ui/keyboard.ts`:

```ts
export function isUpKey(name: string | undefined): boolean {
  return name === "up" || name === "k";
}

export function isDownKey(name: string | undefined): boolean {
  return name === "down" || name === "j";
}

export function isLeftKey(name: string | undefined): boolean {
  return name === "left" || name === "h";
}

export function isRightKey(name: string | undefined): boolean {
  return name === "right" || name === "l";
}

export function isConfirmKey(name: string | undefined): boolean {
  return name === "enter" || name === "return";
}

export function isEscapeKey(name: string | undefined): boolean {
  return name === "escape" || name === "esc";
}

export function isPrintableSearchKey(name: string | undefined): boolean {
  return typeof name === "string" && name.length === 1;
}
```

- [ ] **Step 4: Add config navigation/search tests**

Add to `frontend/src/config/ConfigScreen.test.tsx`:

```tsx
it("supports vim movement keys in provider and form panels", async () => {
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  const client = fakeClient((method, params) => {
    calls.push({ method, params });
    return Promise.resolve(payload("gemini"));
  });
  const { render, mockInput, waitForFrame } = setupTest();

  await render(<ConfigScreen initial={payload()} client={client} />);
  await mockInput.press("j");
  await waitForFrame((frame) => frame.includes("Selected gemini"));
  expect(calls[0]).toMatchObject({
    method: "config.select_provider",
    params: { provider: "gemini" },
  });

  await mockInput.press("l");
  await mockInput.press("j");
  const output = await waitForFrame((frame) => frame.includes("> API Key:"));
  expect(output).toContain("> API Key:");
});

it("searches config form fields and jumps to the first match", async () => {
  const { render, mockInput, waitForFrame } = setupTest();

  await render(<ConfigScreen initial={payload()} client={undefined} />);
  await mockInput.press("l");
  await mockInput.type("/");
  await mockInput.type("timeout");
  let output = await waitForFrame((frame) => frame.includes("Search: timeout"));
  expect(output).toContain("Search: timeout");

  await mockInput.press("enter");
  output = await waitForFrame((frame) => frame.includes("> Timeout"));
  expect(output).toContain("> Timeout");
  expect(output).not.toContain("Search: timeout");
});
```

- [ ] **Step 5: Implement config keyboard/search**

In `frontend/src/config/ConfigScreen.tsx`, import the keyboard helpers and replace arrow-only checks with semantic checks. Add:

```ts
const [search, setSearch] = useState<{ open: boolean; query: string }>({
  open: false,
  query: "",
});
```

Add `closeSearch()` and `runConfigSearch()` helpers:

```ts
const closeSearch = () => setSearch({ open: false, query: "" });

const runConfigSearch = () => {
  const query = search.query.trim().toLowerCase();
  if (!query) {
    closeSearch();
    return;
  }
  if (activePanel === "provider") {
    const index = providerChoices.findIndex((provider) =>
      `${provider.display_name} ${provider.name}`.toLowerCase().includes(query),
    );
    if (index >= 0) {
      selectProviderByIndex(index);
    } else {
      setStatus({ message: `No provider match for ${search.query}`, error: true });
    }
    closeSearch();
    return;
  }
  const index = formFields.findIndex((field) =>
    `${field.label} ${field.key}`.toLowerCase().includes(query),
  );
  if (index >= 0) {
    setFocusedFieldIndex(index);
  } else {
    setStatus({ message: `No field match for ${search.query}`, error: true });
  }
  closeSearch();
};
```

Search mode should handle `Esc`, `Enter`, `Backspace`, and printable characters before normal hotkeys. Non-search mode should open search on `/`, use `h/l` for panel focus, and use `j/k` as up/down.

Render `Search: ${query}` above `StatusLine` in compact and wide mode. Update footer hints to mention `Tab/h/l pane`, `↑/↓/j/k move`, and `/ search`.

- [ ] **Step 6: Run config keyboard tests**

Run:

```bash
bun test --cwd frontend src/ui/keyboard.test.ts src/config/ConfigScreen.test.tsx
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/ui/keyboard.ts frontend/src/ui/keyboard.test.ts frontend/src/config/ConfigScreen.tsx frontend/src/config/ConfigScreen.test.tsx
git commit -m "feat: add config keyboard search"
```

---

### Task 4: Admin Keyboard Navigation/Search

**Files:**
- Modify: `frontend/src/admin/AdminScreen.tsx`
- Test: `frontend/src/admin/AdminScreen.test.tsx`

- [ ] **Step 1: Add failing admin navigation/search tests**

Add to `frontend/src/admin/AdminScreen.test.tsx`:

```tsx
it("supports vim movement keys for admin views and rows", async () => {
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  const base = bootstrap().snapshot;
  const snapshot = {
    ...base,
    rows: [base.rows[0], { ...base.rows[0], id: 2, label: "Second Crystal" }],
  };
  const client = fakeClient((method, params) => {
    calls.push({ method, params });
    return Promise.resolve({
      stats: bootstrap().stats,
      snapshot:
        params.selected_id === 2
          ? { ...snapshot, selected: snapshot.rows[1] }
          : snapshotForView("Renderings"),
    });
  });
  const { render, mockInput, waitFor } = setupTest();

  await render(
    <AdminScreen initial={{ ...bootstrap(), snapshot }} client={client} />,
  );

  await mockInput.press("j");
  await waitFor(async () => calls.length >= 1);
  expect(calls[0]).toMatchObject({
    method: "admin.snapshot",
    params: { view: "Renderings" },
  });

  await mockInput.press("l");
  await mockInput.press("j");
  await waitFor(async () => calls.length >= 2);
  expect(calls[1]).toMatchObject({
    method: "admin.snapshot",
    params: { view: "Crystals", selected_id: 2 },
  });
});

it("searches admin rows and selects the first match", async () => {
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  const base = bootstrap().snapshot;
  const snapshot = {
    ...base,
    rows: [base.rows[0], { ...base.rows[0], id: 2, label: "Recall Ledger" }],
  };
  const client = fakeClient((method, params) => {
    calls.push({ method, params });
    return Promise.resolve({
      stats: bootstrap().stats,
      snapshot: { ...snapshot, selected: snapshot.rows[1] },
    });
  });
  const { render, mockInput, waitForFrame, waitFor } = setupTest();

  await render(
    <AdminScreen initial={{ ...bootstrap(), snapshot }} client={client} />,
  );
  await mockInput.press("l");
  await mockInput.type("/");
  await mockInput.type("recall");
  const output = await waitForFrame((frame) => frame.includes("Search: recall"));
  expect(output).toContain("Search: recall");

  await mockInput.press("enter");
  await waitFor(async () => calls.length >= 1);
  expect(calls[0]).toEqual({
    method: "admin.snapshot",
    params: { view: "Crystals", selected_id: 2 },
  });
});
```

- [ ] **Step 2: Run admin tests and verify they fail**

Run:

```bash
bun test --cwd frontend src/admin/AdminScreen.test.tsx
```

Expected: FAIL because admin does not yet use shared `hjkl` movement or `/` search.

- [ ] **Step 3: Implement admin movement/search**

In `AdminScreen`, import keyboard helpers from `../ui/keyboard.js`, add search state:

```ts
const [search, setSearch] = useState<{ open: boolean; query: string }>({
  open: false,
  query: "",
});
```

Add panel helpers:

```ts
const focusPreviousPanel = () => { /* existing Shift+Tab order */ };
const focusNextPanel = () => { /* existing Tab order */ };
```

Add `closeSearch()` and `runAdminSearch()`:

```ts
const closeSearch = () => setSearch({ open: false, query: "" });

const runAdminSearch = () => {
  const query = search.query.trim().toLowerCase();
  if (!query) {
    closeSearch();
    return;
  }
  if (activePanel === "views") {
    const view = initial.views.find((candidate) =>
      candidate.toLowerCase().includes(query),
    );
    if (view) {
      loadView(view);
    } else {
      setStatus({ message: `No view match for ${search.query}`, error: true });
    }
    closeSearch();
    return;
  }
  const row = snapshot.rows.find((candidate) =>
    `${candidate.label} ${candidate.status} ${candidate.quality_label}`
      .toLowerCase()
      .includes(query),
  );
  if (row) {
    void runSnapshotOperation({
      client,
      method: "admin.snapshot",
      params: { view: snapshot.view, selected_id: row.id },
      successMessage: `Selected ${row.label}`,
      setSnapshot,
      setStats,
      setShortTermStatus,
      setDreamStatus,
      setConfigEditor,
      setStatus,
      operationInFlight,
    });
  } else {
    setStatus({ message: `No row match for ${search.query}`, error: true });
  }
  closeSearch();
};
```

Wire search mode before help/palette handling. Use `j/k` as up/down in views/table and `h/l` as previous/next panel. Keep command palette behavior unchanged.

- [ ] **Step 4: Render admin search and footer hints**

Render search text above `StatusLine` in compact and wide returns:

```tsx
{search.open ? <text fg="cyan">Search: {search.query || " "}</text> : null}
```

Update normal footer keys:

```ts
return [
  "Tab/h/l focus",
  `1-${viewKeyLimit} view`,
  "↑/↓/j/k move",
  "/ search",
  "Ctrl+P commands",
  "? help",
  "q quit",
];
```

Update compact inline footer similarly.

- [ ] **Step 5: Run admin tests**

Run:

```bash
bun test --cwd frontend src/admin/AdminScreen.test.tsx
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/admin/AdminScreen.tsx frontend/src/admin/AdminScreen.test.tsx
git commit -m "feat: add admin keyboard search"
```

---

### Task 5: Full Markdown Detail Renderer

**Files:**
- Create: `frontend/src/admin/markdown.tsx`
- Test: `frontend/src/admin/markdown.test.tsx`
- Modify: `frontend/src/admin/DetailPane.tsx`
- Test: `frontend/src/admin/AdminScreen.test.tsx`

- [ ] **Step 1: Add markdown renderer tests**

Create `frontend/src/admin/markdown.test.tsx`:

```tsx
import { afterEach, describe, expect, it } from "bun:test";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { renderMarkdownBlocks } from "./markdown.js";

function setupTest() {
  return createOpenTuiHarness({ width: 100, height: 30 });
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("renderMarkdownBlocks", () => {
  it("renders headings, lists, quotes, code fences, and emphasis", async () => {
    const { render, waitForFrame } = setupTest();

    await render(
      <box flexDirection="column">
        {renderMarkdownBlocks([
          "# Heading",
          "",
          "- First item",
          "1. Ordered item",
          "> Quoted note",
          "```json",
          "{\"ok\": true}",
          "```",
          "Plain **strong** text",
        ].join("\n"))}
      </box>,
    );

    const output = await waitForFrame((frame) => frame.includes("Heading"));
    expect(output).toContain("Heading");
    expect(output).toContain("- First item");
    expect(output).toContain("1. Ordered item");
    expect(output).toContain("> Quoted note");
    expect(output).toContain("{\"ok\": true}");
    expect(output).toContain("strong");
  });
});
```

- [ ] **Step 2: Run markdown test and verify it fails**

Run:

```bash
bun test --cwd frontend src/admin/markdown.test.tsx
```

Expected: FAIL because `markdown.tsx` does not exist.

- [ ] **Step 3: Implement markdown renderer**

Create `frontend/src/admin/markdown.tsx` with:

```tsx
import React from "react";
import { SyntaxStyle } from "@opentui/core";

export const markdownCodeStyle = SyntaxStyle.fromStyles({
  string: { fg: "#9ece6a" },
  number: { fg: "#ff9e64" },
  boolean: { fg: "#bb9af7" },
  property: { fg: "#7dcfff" },
  punctuation: { fg: "#a9b1d6" },
  keyword: { fg: "#bb9af7", bold: true },
});

function inlineMarkdown(text: string, keyPrefix: string): React.ReactNode[] {
  const parts: React.ReactNode[] = [];
  const pattern = /(`([^`]+)`|<strong>(.*?)<\/strong>|\*\*([^*]+)\*\*|\*([^*]+)\*)/gi;
  let offset = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    if (match.index > offset) parts.push(text.slice(offset, match.index));
    const value = match[2] ?? match[3] ?? match[4] ?? match[5] ?? "";
    parts.push(
      <text key={`${keyPrefix}-${match.index}`} fg={match[2] ? "yellow" : undefined}>
        {value}
      </text>,
    );
    offset = match.index + match[0].length;
  }
  if (offset < text.length) parts.push(text.slice(offset));
  return parts;
}

export function renderMarkdownBlocks(markdown: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  const lines = markdown.split("\n");
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index] ?? "";
    const fence = line.match(/^```(\w+)?\s*$/);
    if (fence) {
      const body: string[] = [];
      index += 1;
      while (index < lines.length && !/^```\s*$/.test(lines[index] ?? "")) {
        body.push(lines[index] ?? "");
        index += 1;
      }
      nodes.push(
        <code
          key={`code-${index}`}
          content={body.join("\n")}
          filetype={fence[1] ?? "text"}
          syntaxStyle={markdownCodeStyle}
        />,
      );
      continue;
    }
    if (!line.trim()) {
      nodes.push(<text key={`blank-${index}`}> </text>);
      continue;
    }
    const heading = line.match(/^\s{0,3}#{1,6}\s+(.*)$/);
    if (heading) {
      nodes.push(
        <text key={`heading-${index}`} fg="cyan">
          {heading[1]}
        </text>,
      );
      continue;
    }
    const unordered = line.match(/^\s{0,3}[-*+]\s+(.*)$/);
    if (unordered) {
      nodes.push(<text key={`ul-${index}`}>- {inlineMarkdown(unordered[1] ?? "", `ul-${index}`)}</text>);
      continue;
    }
    const ordered = line.match(/^\s{0,3}(\d+)\.\s+(.*)$/);
    if (ordered) {
      nodes.push(<text key={`ol-${index}`}>{ordered[1]}. {inlineMarkdown(ordered[2] ?? "", `ol-${index}`)}</text>);
      continue;
    }
    const quote = line.match(/^\s{0,3}>\s?(.*)$/);
    if (quote) {
      nodes.push(<text key={`quote-${index}`} fg="gray">> {quote[1]}</text>);
      continue;
    }
    nodes.push(<text key={`p-${index}`}>{inlineMarkdown(line, `p-${index}`)}</text>);
  }
  return nodes;
}
```

Run Prettier after implementation; the long JSX lines above should be wrapped by Prettier.

- [ ] **Step 4: Replace lightweight renderer in DetailPane**

In `frontend/src/admin/DetailPane.tsx`:

- Import `renderMarkdownBlocks` and `markdownCodeStyle` from `./markdown.js`.
- Remove local `codeSyntaxStyle`, `renderInlineMarkdown`, and `renderMarkdownLines`.
- Keep `isDiff` and `isJson`.
- Use `markdownCodeStyle` for JSON and diff code styles.
- For prose bodies, return:

```tsx
return <box flexDirection="column">{renderMarkdownBlocks(detail.body)}</box>;
```

- [ ] **Step 5: Add admin detail markdown integration test**

In `frontend/src/admin/AdminScreen.test.tsx`, add:

```tsx
it("renders markdown detail blocks for dense inspection", async () => {
  const { render, waitForFrame } = setupTest();
  const initial = {
    ...bootstrap(),
    snapshot: {
      ...bootstrap().snapshot,
      detail: {
        title: "Markdown detail",
        subtitle: "Inspection",
        body: "# Recall Reasons\n\n- weighted match\n> audit note\n```json\n{\"score\": 1}\n```",
        fields: [["source", "test"]],
      },
    },
  };

  await render(<AdminScreen initial={initial} client={undefined} />);

  const output = await waitForFrame((frame) => frame.includes("Recall Reasons"));
  expect(output).toContain("weighted match");
  expect(output).toContain("> audit note");
  expect(output).toContain("\"score\"");
  expect(output).toContain("source: test");
});
```

- [ ] **Step 6: Run markdown/detail tests**

Run:

```bash
bun test --cwd frontend src/admin/markdown.test.tsx src/admin/AdminScreen.test.tsx
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/admin/markdown.tsx frontend/src/admin/markdown.test.tsx frontend/src/admin/DetailPane.tsx frontend/src/admin/AdminScreen.test.tsx
git commit -m "feat: render admin markdown details"
```

---

### Task 6: Multiline Editor Scrollbar Capability Check

**Files:**
- Modify: `frontend/src/ui/TextInput.tsx`
- Modify: `frontend/src/admin/dialogs.tsx`
- Test: `frontend/src/admin/AdminScreen.test.tsx`
- Modify: `docs/opentui-conventions.md`

- [ ] **Step 1: Inspect current OpenTUI textarea props**

Run:

```bash
rg -n "scrollbar|showScrollbar|scroll" frontend/node_modules/@opentui frontend/src/ui/TextInput.tsx frontend/src/admin/dialogs.tsx
```

Expected:

- If OpenTUI exposes a textarea scrollbar prop, use it.
- If not, document that visible multiline editor scrollbars remain unsupported by the current OpenTUI version and keep the UI stable.

- [ ] **Step 2: Add a regression test for multiline editor stability**

In `frontend/src/admin/AdminScreen.test.tsx`, add:

```tsx
it("keeps multiline memory editor bounded in the admin dialog", async () => {
  const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

  await render(<AdminScreen initial={bootstrap()} client={undefined} />);
  await mockInput.type("a");

  const output = await waitForFrame((frame) =>
    frame.includes("Add New Crystal / Lesson / Rule"),
  );
  expect(output).toContain("Text:");
  expect(output).toContain("Tags:");
  expect(output).toContain("Esc cancel");
});
```

- [ ] **Step 3: Apply scrollbar prop only if supported**

If the local OpenTUI textarea type supports a visible scrollbar prop, update `TextAreaInput` in `frontend/src/ui/TextInput.tsx`:

```tsx
<textarea
  value={value}
  onInput={onChange}
  placeholder={placeholder}
  focused={focused}
  width={width}
  height={height}
  showScrollbar={true}
/>
```

If the prop is not supported, do not add an unknown prop. Instead add a concise code comment near the textarea explaining:

```ts
// OpenTUI 0.4.0 textarea does not expose visible scrollbar controls.
```

Update `docs/opentui-conventions.md` under Admin Inspection:

```md
- Multiline editor scrollbars should be enabled once the installed OpenTUI
  textarea component exposes a stable visible-scrollbar prop. Until then,
  multiline dialogs must remain bounded and keyboard usable.
```

- [ ] **Step 4: Run focused admin tests and typecheck**

Run:

```bash
bun test --cwd frontend src/admin/AdminScreen.test.tsx
bun run --cwd frontend typecheck
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/ui/TextInput.tsx frontend/src/admin/dialogs.tsx frontend/src/admin/AdminScreen.test.tsx docs/opentui-conventions.md
git commit -m "test: cover multiline editor bounds"
```

---

### Task 7: Roadmap Registration And Verification

**Files:**
- Modify: `docs/roadmap.md`
- Modify: `docs/superpowers/plans/2026-06-14-opentui-completion-pass.md`

- [ ] **Step 1: Update roadmap completed work**

In `docs/roadmap.md`, add bullets to OpenTUI `Completed baseline:`:

```md
- `hiero config` exposes section metadata for current and future configuration
  files while keeping rendering schema-driven.
- `hiero admin` and `hiero config` support keyboard-first navigation with
  `hjkl`, `/` search, `Esc` cancellation, and footer hints aligned with the
  active mode.
- Admin detail panes render dense inspection bodies with structured fields,
  JSON, diffs, markdown blocks, provenance, recall reasons, dream output, and
  audit output through the admin interface.
```

Remove completed bullets from OpenTUI `Remaining work`:

```md
- Extend `hiero config` beyond the current `dream.conf`, `ingest.conf`, and
  `release.conf` editor as future configuration files are added.
- Support keyboard-first navigation with consistent `q`, `Esc`, `/`, `Tab`,
  `Shift+Tab`, arrows, and `hjkl` where useful.
- Replace the lightweight memory-detail markdown renderer with a full markdown
  renderer once OpenTUI markdown output is reliable in the test renderer.
- Enable visible scrollbars for multiline memory editors if OpenTUI exposes
  textarea scrollbar controls in a newer release.
```

If Task 6 found scrollbars unsupported, do not claim visible scrollbar completion. Instead remove the roadmap item because it is now captured as a convention/dependency note in `docs/opentui-conventions.md`.

- [ ] **Step 2: Run focused tests**

Run:

```bash
uv run pytest tests/test_opentui_conventions.py tests/test_tui_bridge_config.py::test_config_form_schema_groups_fields_by_config_file -q
bun test --cwd frontend src/ui/keyboard.test.ts src/rpc/schema.test.ts src/config/ConfigScreen.test.tsx src/admin/markdown.test.tsx src/admin/AdminScreen.test.tsx
```

Expected: PASS.

- [ ] **Step 3: Run full verification**

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

- [ ] **Step 4: Commit**

```bash
git add docs/roadmap.md docs/superpowers/plans/2026-06-14-opentui-completion-pass.md
git commit -m "docs: register opentui completion pass"
```

---

## Implementation Notes

- Keep this pass frontend/bridge-oriented. Do not introduce new databases, new backend mutation surfaces, or new frontend persistence.
- If a planned detail renderer feature is blocked by OpenTUI test renderer behavior, preserve the best reliable renderer and document the exact blocker in `docs/opentui-conventions.md`.
- If multiline scrollbar props are unavailable in OpenTUI 0.4.0, do not pass unknown props. Document the dependency and keep bounded editor tests.
- Keep commits task-sized. Review after each task before moving on.

## Self-Review

- Spec coverage: The plan covers all user-listed OpenTUI roadmap items, moves standing “keep/ensure” rules into conventions, and includes compliance checks for current code.
- Placeholder scan: No task uses TBD or open-ended placeholders; external capability uncertainty for textarea scrollbars is handled with an explicit inspect-and-branch task.
- Type consistency: Shared helper names and config schema metadata names are defined before later tasks use them.
