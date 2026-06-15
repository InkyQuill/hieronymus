# Config Screen Grouped Form Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rework `hiero config` into one compact grouped editor with provider/API, dreaming, ingest, and release blocks, while fixing model input clipping and footer clarity.

**Architecture:** Keep Python config ownership unchanged. Add a frontend-only provider choice field to the React/OpenTUI form, group fields using the bridge-owned schema metadata, and sanitize local synthetic state before RPC calls. Preserve `config.select_provider` for provider changes and `config.update_draft` for normal field edits.

**Tech Stack:** React 19, TypeScript, OpenTUI React, Bun tests, Python bridge contracts, `uv`, `pytest`, `ruff`.

---

## File Structure

- Modify: `frontend/src/config/ConfigScreen.tsx`
  - Remove the separate provider panel from normal layouts.
  - Add frontend-only provider field handling.
  - Simplify header text.
  - Use `KeyHelp` in compact and wide layouts.
  - Route provider choice edits through `config.select_provider`.
- Modify: `frontend/src/config/ConfigForm.tsx`
  - Render fields grouped by `form_schema.groups`.
  - Show active field hints.
  - Give editable inputs a stable width.
- Modify: `frontend/src/config/ConfigScreen.test.tsx`
  - Update existing expectations that currently assert separate provider panel behavior.
  - Add coverage for grouped rendering, provider field behavior, compact header/footer, and model input leading characters.
- No Python config persistence files should change.

## Implementation Notes

Use these constants in `ConfigScreen.tsx`:

```ts
const SYNTHETIC_PROVIDER_KEY = "provider.__selected";
```

The synthetic provider field is frontend-only. It must never be sent inside the `provider` object for `config.update_draft`.

---

### Task 1: Add failing tests for grouped config presentation

**Files:**
- Modify: `frontend/src/config/ConfigScreen.test.tsx`

- [ ] **Step 1: Replace the compact layout presentation test**

Replace the existing test named `"renders config as a single active pane at 80x24"` with:

```ts
it("renders compact config as one grouped form at 80x24", async () => {
  const { render, waitForFrame } = setupSizedTest(80, 24);

  await render(<ConfigScreen initial={payload()} client={undefined} />);

  const output = await waitForFrame((frame) =>
    frame.includes("Provider/API | Dreaming | Ingest | Release"),
  );
  expect(output).toContain("Hieronymus Config");
  expect(output).toContain("Provider/API | Dreaming | Ingest | Release");
  expect(output).toContain("Provider/API");
  expect(output).toContain("OpenAI compatible");
  expect(output).toContain("Model");
  expect(output).not.toContain("Config files:");
  expect(output).not.toContain("compact 80x24");
  expect(output).not.toContain(
    "/tmp/dream.conf | /tmp/ingest.conf | /tmp/release.conf",
  );
});
```

- [ ] **Step 2: Replace the backend-owned section labels test**

Replace the existing test named `"renders backend-owned config section labels"` with:

```ts
it("renders grouped config blocks in bridge schema order", async () => {
  const { render, waitForFrame } = setupTest();

  await render(<ConfigScreen initial={payload()} client={undefined} />);

  const output = await waitForFrame((frame) =>
    frame.includes("Provider/API") && frame.includes("Dreaming"),
  );
  const providerIndex = output.indexOf("Provider/API");
  const dreamingIndex = output.indexOf("Dreaming");
  const ingestionIndex = output.indexOf("Ingestion");
  const updatesIndex = output.indexOf("Updates");

  expect(providerIndex).toBeGreaterThanOrEqual(0);
  expect(dreamingIndex).toBeGreaterThan(providerIndex);
  expect(ingestionIndex).toBeGreaterThan(dreamingIndex);
  expect(updatesIndex).toBeGreaterThan(ingestionIndex);
  expect(output).toContain("dream.conf");
  expect(output).toContain("ingest.conf");
  expect(output).toContain("release.conf");
});
```

- [ ] **Step 3: Replace the provider family selector test**

Replace the existing test named `"renders one provider family selector instead of provider rows"` with:

```ts
it("renders provider choice as the first Provider/API field", async () => {
  const { render, waitForFrame } = setupTest();

  await render(<ConfigScreen initial={payload()} client={undefined} />);

  const output = await waitForFrame((frame) =>
    frame.includes("> Provider: OpenAI compatible"),
  );
  expect(output).toContain("Provider/API");
  expect(output).toContain("> Provider: OpenAI compatible");
  expect(output).toContain("Model: gpt-4.1-mini");
  expect(output).not.toContain("Providers");
  expect(output).not.toContain("▶ OpenAI compatible");
});
```

- [ ] **Step 4: Add footer key clarity test**

Add this test near the compact footer test:

```ts
it("renders footer keys as bracketed key labels", async () => {
  const { render, waitForFrame } = setupSizedTest(80, 24);

  await render(<ConfigScreen initial={payload()} client={undefined} />);

  const output = await waitForFrame((frame) => frame.includes("[Enter] edit"));
  expect(output).toContain("[↑↓] field");
  expect(output).toContain("[Enter] edit");
  expect(output).toContain("[s] save");
  expect(output).toContain("[/] search");
  expect(output).toContain("[q] quit");
  expect(output).not.toContain("Tab pane / search");
});
```

- [ ] **Step 5: Run focused tests and verify they fail**

Run:

```bash
bun test --cwd frontend src/config/ConfigScreen.test.tsx
```

Expected: FAIL. Failures should mention missing grouped headings, existing `Providers` pane, old footer text, or old header labels.

- [ ] **Step 6: Commit failing tests**

```bash
git add frontend/src/config/ConfigScreen.test.tsx
git commit -m "test: cover grouped config screen layout"
```

---

### Task 2: Render grouped fields and active hints in ConfigForm

**Files:**
- Modify: `frontend/src/config/ConfigForm.tsx`
- Test: `frontend/src/config/ConfigScreen.test.tsx`

- [ ] **Step 1: Update ConfigForm props**

Change the imports and props at the top of `ConfigForm.tsx` to include group metadata:

```ts
import React from "react";
import type { ConfigFormField, ConfigFormGroup } from "../rpc/schema.js";

type ConfigFormProps = {
  groups: ConfigFormGroup[];
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
  visibleRows?: number;
  onFieldChange: (key: string, value: string) => void;
  onSubmitField: () => void;
};
```

- [ ] **Step 2: Add grouped rendering helpers**

Add these helper types and functions below the prop type:

```ts
type RenderedField = ConfigFormField & { value: string };

type RenderedGroup = {
  group: ConfigFormGroup;
  fields: Array<{ field: RenderedField; index: number }>;
};

function configFileLabel(group: ConfigFormGroup): string {
  if (group.section === "dream") return "dream.conf";
  if (group.section === "ingest") return "ingest.conf";
  if (group.section === "release") return "release.conf";
  return group.section || "";
}

function displayGroupLabel(group: ConfigFormGroup): string {
  if (group.id === "provider") return "Provider/API";
  if (group.id === "ingest") return "Ingestion";
  if (group.id === "release") return "Updates";
  return group.label;
}

function groupRenderedFields(
  groups: ConfigFormGroup[],
  fields: RenderedField[],
): RenderedGroup[] {
  const fieldsByGroup = new Map<string, Array<{ field: RenderedField; index: number }>>();
  fields.forEach((field, index) => {
    const list = fieldsByGroup.get(field.group) ?? [];
    list.push({ field, index });
    fieldsByGroup.set(field.group, list);
  });

  const orderedGroups = groups
    .map((group) => ({ group, fields: fieldsByGroup.get(group.id) ?? [] }))
    .filter((group) => group.fields.length > 0);

  const knownGroupIds = new Set(groups.map((group) => group.id));
  const orphanFields = fields
    .map((field, index) => ({ field, index }))
    .filter(({ field }) => !knownGroupIds.has(field.group));

  if (orphanFields.length > 0) {
    orderedGroups.push({
      group: {
        id: "other",
        section: "",
        label: "Other",
        description: "Additional configuration fields.",
      },
      fields: orphanFields,
    });
  }

  return orderedGroups;
}

function visibleIndexSet(
  fieldCount: number,
  focusedFieldIndex: number,
  visibleRows: number | undefined,
): Set<number> {
  const fieldWindow = getVisibleFieldWindow(fieldCount, focusedFieldIndex, visibleRows);
  return new Set(
    Array.from(
      { length: fieldWindow.end - fieldWindow.start },
      (_, index) => fieldWindow.start + index,
    ),
  );
}
```

- [ ] **Step 3: Replace the ConfigForm component body**

Replace the body of `ConfigForm` with:

```tsx
export function ConfigForm({
  groups,
  fields,
  formValues,
  focusedFieldIndex,
  isEditing,
  focused = true,
  width = 68,
  visibleRows,
  onFieldChange,
  onSubmitField,
}: ConfigFormProps) {
  const provider = formValues.provider;
  const dreaming = formValues.dreaming;
  const ingest = formValues.ingest;
  const release = formValues.release;

  const renderedFields = fields.map((field) => {
    let value = "";
    if (field.key.startsWith("provider.")) {
      value = provider[field.key.slice(9)] || "";
    } else if (field.key.startsWith("dreaming.")) {
      value = dreaming[field.key.slice(9)] || "";
    } else if (field.key.startsWith("ingest.")) {
      value = ingest[field.key.slice(7)] || "";
    } else if (field.key.startsWith("release.")) {
      value = release[field.key.slice(8)] || "";
    }
    if ((field.type === "toggle" || field.type === "choice") && !value) {
      value = field.default || field.choices[0] || "";
    }
    return {
      ...field,
      value,
    };
  });

  const visibleIndexes = visibleIndexSet(
    renderedFields.length,
    focusedFieldIndex,
    visibleRows,
  );
  const groupedFields = groupRenderedFields(groups, renderedFields);
  const activeField = renderedFields[focusedFieldIndex];
  const inputWidth = Math.max(12, width - 34);

  return (
    <box flexDirection="column" width={width}>
      {groupedFields.map(({ group, fields: groupFields }) => {
        const visibleGroupFields = groupFields.filter(({ index }) =>
          visibleIndexes.has(index),
        );
        const isActiveGroup = groupFields.some(
          ({ index }) => focused && focusedFieldIndex === index,
        );
        if (visibleGroupFields.length === 0) {
          return null;
        }

        return (
          <box
            key={group.id}
            flexDirection="column"
            marginTop={1}
            borderStyle="rounded"
            borderColor={isActiveGroup ? "cyan" : "gray"}
            paddingX={1}
          >
            <box flexDirection="row">
              <text fg={isActiveGroup ? "cyan" : undefined}>
                {displayGroupLabel(group)}
              </text>
              {configFileLabel(group) ? (
                <text fg="gray"> · {configFileLabel(group)}</text>
              ) : null}
            </box>
            {group.description ? <text fg="gray">{group.description}</text> : null}
            <box flexDirection="column" marginTop={1}>
              {visibleGroupFields.map(({ field, index }) => {
                const isFieldFocused = focused && focusedFieldIndex === index;
                const labelColor = isFieldFocused ? "cyan" : "gray";

                return (
                  <box key={field.key} flexDirection="row">
                    <text fg={labelColor}>
                      {isFieldFocused ? "> " : "  "}
                      {field.label}:{" "}
                    </text>

                    {field.type === "toggle" || field.type === "choice" ? (
                      <box flexDirection="row">
                        {isFieldFocused && isEditing ? (
                          <box flexDirection="row">
                            {(field.choices.length
                              ? field.choices
                              : ["yes", "no"]
                            ).map((choice) => (
                              <text
                                key={choice}
                                fg={field.value === choice ? "cyan" : "gray"}
                              >
                                [{choice}]{" "}
                              </text>
                            ))}
                          </box>
                        ) : (
                          <text fg={isFieldFocused ? "cyan" : undefined}>
                            {field.value}
                          </text>
                        )}
                      </box>
                    ) : isFieldFocused && isEditing ? (
                      <input
                        value={field.value}
                        width={inputWidth}
                        onInput={(val) => onFieldChange(field.key, val)}
                        onSubmit={() => onSubmitField()}
                        focused={true}
                        placeholder={field.placeholder}
                      />
                    ) : (
                      <text fg={isFieldFocused ? "cyan" : undefined}>
                        {field.value || field.placeholder || " "}
                      </text>
                    )}
                  </box>
                );
              })}
            </box>
            {isActiveGroup && activeField?.hint ? (
              <text fg="gray">{activeField.hint}</text>
            ) : null}
          </box>
        );
      })}
    </box>
  );
}
```

- [ ] **Step 4: Run focused tests and verify they still fail at call sites**

Run:

```bash
bun test --cwd frontend src/config/ConfigScreen.test.tsx
```

Expected: FAIL with TypeScript/runtime errors about missing `groups` prop or old provider panel behavior.

- [ ] **Step 5: Commit grouped ConfigForm**

```bash
git add frontend/src/config/ConfigForm.tsx
git commit -m "feat: group config form fields"
```

---

### Task 3: Collapse ConfigScreen to one grouped editor

**Files:**
- Modify: `frontend/src/config/ConfigScreen.tsx`
- Test: `frontend/src/config/ConfigScreen.test.tsx`

- [ ] **Step 1: Remove ProviderSelector import and active panel state**

In `ConfigScreen.tsx`, remove:

```ts
import { ProviderSelector } from "./ProviderSelector.js";
```

Change:

```ts
const [activePanel, setActivePanel] = useState<"provider" | "form">(
  "provider",
);
```

to:

```ts
const [focusedFieldIndex, setFocusedFieldIndex] = useState(0);
```

Remove the old duplicate `focusedFieldIndex` declaration, `switchPanel`,
`focusProviderPanel`, and `focusFormPanel`.

- [ ] **Step 2: Add provider display helpers**

Add these helpers near the existing helper functions:

```ts
const SYNTHETIC_PROVIDER_KEY = "provider.__selected";

function providerDisplayName(
  providerChoices: ConfigBootstrap["provider_choices"],
  provider: ProviderName,
): string {
  return (
    providerChoices.find((choice) => choice.name === provider)?.display_name ??
    provider
  );
}

function providerNameForDisplay(
  providerChoices: ConfigBootstrap["provider_choices"],
  displayName: string,
): ProviderName | undefined {
  return providerChoices.find((choice) => choice.display_name === displayName)
    ?.name;
}

function configFieldsWithProviderChoice(
  payload: ConfigBootstrap,
): ConfigFormField[] {
  const providerChoiceField: ConfigFormField = {
    key: SYNTHETIC_PROVIDER_KEY,
    group: "provider",
    section: "dream",
    label: "Provider",
    hint: "Provider family used for dreaming workflows and model checks.",
    placeholder: "",
    type: "choice",
    choices: payload.provider_choices.map((choice) => choice.display_name),
    default: providerDisplayName(payload.provider_choices, payload.selected_provider),
    redacted: false,
  };

  return [
    providerChoiceField,
    ...payload.form_schema.fields.filter(
      (field) => field.key !== SYNTHETIC_PROVIDER_KEY,
    ),
  ];
}

function formValuesWithSelectedProvider(
  values: ConfigFormValues,
  payload: ConfigBootstrap,
): ConfigFormValues {
  return {
    ...values,
    provider: {
      ...values.provider,
      __selected: providerDisplayName(
        payload.provider_choices,
        payload.selected_provider,
      ),
    },
  };
}

function draftFormValues(values: ConfigFormValues): ConfigFormValues {
  const { __selected, ...provider } = values.provider;
  void __selected;
  return {
    ...values,
    provider,
  };
}
```

- [ ] **Step 3: Use display fields and synthetic provider values**

Change:

```ts
const formFields = payload.form_schema.fields;
```

to:

```ts
const formFields = configFieldsWithProviderChoice(payload);
```

Change the local form initialization and sync effect to call `formValuesWithSelectedProvider`:

```ts
const [localFormValues, setLocalFormValues] = useState(() =>
  formValuesWithSelectedProvider(
    {
      provider: { ...payload.form_values.provider },
      dreaming: { ...payload.form_values.dreaming },
      ingest: { ...payload.form_values.ingest },
      release: { ...payload.form_values.release },
    },
    payload,
  ),
);

useEffect(() => {
  setLocalFormValues(
    formValuesWithSelectedProvider(
      {
        provider: { ...payload.form_values.provider },
        dreaming: { ...payload.form_values.dreaming },
        ingest: { ...payload.form_values.ingest },
        release: { ...payload.form_values.release },
      },
      payload,
    ),
  );
}, [payload]);
```

- [ ] **Step 4: Sanitize update_draft params**

In `submitField`, add:

```ts
const draftValues = draftFormValues(formValues);
```

and change the params to use `draftValues.provider`, `draftValues.dreaming`, `draftValues.ingest`, and `draftValues.release`.

- [ ] **Step 5: Route synthetic provider choice through select_provider**

In the editing branch for toggle/choice fields, update the Enter handling:

```ts
} else if (enter) {
  const currentVal = effectiveValueForField(localFormValues, focusedField);
  if (focusedField.key === SYNTHETIC_PROVIDER_KEY) {
    const provider = providerNameForDisplay(providerChoices, currentVal);
    setIsEditing(false);
    if (provider) {
      selectProviderByIndex(
        Math.max(
          providerChoices.findIndex((choice) => choice.name === provider),
          0,
        ),
      );
    }
    return;
  }
  submitField(withFieldValue(localFormValues, focusedField.key, currentVal));
}
```

- [ ] **Step 6: Simplify navigation**

Replace the panel navigation block with:

```ts
if (up) {
  setFocusedFieldIndex((prev) => Math.max(0, prev - 1));
  return;
}
if (down) {
  setFocusedFieldIndex((prev) =>
    Math.min(Math.max(formFields.length - 1, 0), prev + 1),
  );
  return;
}
if (enter && formFields.length > 0) {
  setIsEditing(true);
  return;
}
```

Keep numeric provider shortcuts as global hotkeys after normal field navigation.

- [ ] **Step 7: Replace compact and wide layout bodies**

Both compact and wide layouts should render:

```tsx
<text>Hieronymus Config</text>
<text fg="gray">Provider/API | Dreaming | Ingest | Release</text>
```

and a single bordered form panel:

```tsx
<box
  flexDirection="column"
  marginTop={1}
  height={compactHeight}
  borderStyle="rounded"
  borderColor="cyan"
  paddingX={1}
>
  <ConfigForm
    groups={payload.form_schema.groups}
    fields={formFields}
    formValues={localFormValues}
    focusedFieldIndex={focusedFieldIndex}
    isEditing={isEditing}
    focused
    width={contentWidth}
    visibleRows={compactVisibleFormRows}
    onFieldChange={handleFieldChange}
    onSubmitField={submitField}
  />
</box>
```

For the wide layout, use `height={panelHeight(layout, 10)}`, `width={Math.min(98, dimensions.width - 2)}`, and the same `ConfigForm` props with `visibleRows={panelHeight(layout, 16)}`.

- [ ] **Step 8: Use KeyHelp in compact and wide layouts**

Use this footer in both layouts:

```tsx
<KeyHelp
  keys={[
    "↑↓ field",
    "Enter edit",
    "/ search",
    `${providerKeyRange(providerChoices)} provider`,
    "s save",
    "r reload",
    "c check",
    "q quit",
  ]}
/>
```

- [ ] **Step 9: Run focused tests**

Run:

```bash
bun test --cwd frontend src/config/ConfigScreen.test.tsx
```

Expected: Some old navigation tests may still fail because they expect tabs/panels. Grouped presentation tests should pass.

- [ ] **Step 10: Commit one-editor ConfigScreen**

```bash
git add frontend/src/config/ConfigScreen.tsx frontend/src/config/ConfigScreen.test.tsx
git commit -m "feat: render config as grouped editor"
```

---

### Task 4: Update keyboard behavior tests for one editor

**Files:**
- Modify: `frontend/src/config/ConfigScreen.test.tsx`

- [ ] **Step 1: Replace provider selection RPC test expectations**

In `"selects a provider through the configured RPC"`, replace the interaction with:

```ts
await render(<ConfigScreen initial={payload()} client={client} />);

await mockInput.press("enter");
await mockInput.press("right");
await mockInput.press("enter");

await waitForFrame((frame) => frame.includes("Selected gemini"));
```

Replace final frame expectations with:

```ts
const output = captureCharFrame();
expect(output).toContain("Provider: Gemini");
expect(output).toContain("gemini-2.5-flash");
```

- [ ] **Step 2: Replace provider j/k navigation test**

Rename `"moves through providers with j and k like arrow keys"` to `"changes provider choice while editing the provider field"` and use:

```ts
await render(<ConfigScreen initial={payload()} client={client} />);

await mockInput.press("enter");
await mockInput.press("right");
await mockInput.press("enter");
await waitForFrame((frame) => frame.includes("Selected gemini"));

await mockInput.press("enter");
await mockInput.press("left");
await mockInput.press("enter");
await waitForFrame((frame) => frame.includes("Selected openai"));
```

Keep:

```ts
expect(calls.map((call) => call.params.provider)).toEqual([
  "gemini",
  "openai",
]);
```

- [ ] **Step 3: Replace form navigation test startup**

In `"moves through form fields with j and k like arrow keys"`, remove `await mockInput.press("tab");` and change expectations so the first `j` reaches Model after Provider:

```ts
await mockInput.type("j");

let output = await waitForFrame((frame) => frame.includes("> Model"));
expect(output).toContain("> Model");

await mockInput.type("k");

output = await waitForFrame((frame) => frame.includes("> Provider"));
expect(output).toContain("> Provider");
```

- [ ] **Step 4: Remove panel switching test**

Delete the test named `"moves between config panels with h and l"` because there is no second panel in the approved design.

- [ ] **Step 5: Update search-mode tab test**

In `"keeps tab inside active config search mode"`, replace the last assertions with:

```ts
expect(output).toContain("Search: mod");
expect(output).toContain("> Provider");
expect(output).not.toContain("> Model");
```

- [ ] **Step 6: Update form edit tests that press Tab before editing**

In `"updates schema-driven number, toggle, and choice fields from the form panel"`, replace the first navigation sequence:

```ts
await mockInput.press("tab");
await mockInput.press("enter");
```

with:

```ts
await mockInput.press("down");
await mockInput.press("enter");
```

In `"submits schema-effective choice defaults from the form panel"`, replace:

```ts
await mockInput.press("tab");
await mockInput.press("enter");
await mockInput.press("enter");
```

with:

```ts
await mockInput.press("down");
await mockInput.press("enter");
await mockInput.press("enter");
```

- [ ] **Step 7: Run focused tests**

Run:

```bash
bun test --cwd frontend src/config/ConfigScreen.test.tsx
```

Expected: PASS for `ConfigScreen.test.tsx`.

- [ ] **Step 8: Commit keyboard test updates**

```bash
git add frontend/src/config/ConfigScreen.test.tsx
git commit -m "test: update config keyboard flow"
```

---

### Task 5: Add regression test for model leading characters

**Files:**
- Modify: `frontend/src/config/ConfigScreen.test.tsx`
- Modify: `frontend/src/config/ConfigForm.tsx`

- [ ] **Step 1: Add focused model editing test**

Add this test near the schema-driven field edit tests:

```ts
it("preserves leading characters when editing the model field", async () => {
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  const client = fakeClient((method, params) => {
    calls.push({ method, params });
    return Promise.resolve({
      ...payload(),
      form_values: {
        ...payload().form_values,
        provider: {
          ...payload().form_values.provider,
          model: "deepseek",
        },
      },
    });
  });
  const { render, mockInput, waitFor, waitForFrame } = setupTest();

  await render(<ConfigScreen initial={payload()} client={client} />);

  await mockInput.press("down");
  await mockInput.press("enter");
  for (let index = 0; index < "gpt-4.1-mini".length; index += 1) {
    await mockInput.press("backspace");
  }
  await mockInput.type("deepseek");
  await mockInput.press("enter");

  await waitFor(async () => calls.length >= 1);
  const output = await waitForFrame((frame) => frame.includes("deepseek"));

  expect(calls[0]?.params.provider).toMatchObject({ model: "deepseek" });
  expect(output).toContain("Model: deepseek");
  expect(output).not.toContain("Model: epseek");
});
```

- [ ] **Step 2: Run the regression test**

Run:

```bash
bun test --cwd frontend src/config/ConfigScreen.test.tsx --timeout 10000
```

Expected: FAIL if the OpenTUI input still clips leading characters or if navigation lands on the wrong field.

- [ ] **Step 3: Ensure ConfigForm input has stable width**

Confirm the text/number/secret editing branch in `ConfigForm.tsx` includes:

```tsx
<input
  value={field.value}
  width={inputWidth}
  onInput={(val) => onFieldChange(field.key, val)}
  onSubmit={() => onSubmitField()}
  focused={true}
  placeholder={field.placeholder}
/>
```

If the test still fails visually, replace the `<input>` with the existing `TextInput` wrapper only after adding a `width` prop to `frontend/src/ui/TextInput.tsx`.

- [ ] **Step 4: Run focused tests**

Run:

```bash
bun test --cwd frontend src/config/ConfigScreen.test.tsx
```

Expected: PASS.

- [ ] **Step 5: Commit model input regression coverage**

```bash
git add frontend/src/config/ConfigScreen.test.tsx frontend/src/config/ConfigForm.tsx frontend/src/ui/TextInput.tsx
git commit -m "fix: preserve config model input text"
```

---

### Task 6: Run full verification

**Files:**
- No planned source edits.

- [ ] **Step 1: Run frontend typecheck**

Run:

```bash
bun run --cwd frontend typecheck
```

Expected: PASS.

- [ ] **Step 2: Run frontend tests**

Run:

```bash
bun test --cwd frontend
```

Expected: PASS.

- [ ] **Step 3: Run project tests**

Run:

```bash
uv run pytest
```

Expected: PASS.

- [ ] **Step 4: Run ruff check**

Run:

```bash
uv run ruff check .
```

Expected: PASS.

- [ ] **Step 5: Run ruff format check**

Run:

```bash
uv run ruff format --check .
```

Expected: PASS.

- [ ] **Step 6: Inspect final git status**

Run:

```bash
git status --short
```

Expected: clean working tree after task commits, or only intentional uncommitted changes if execution mode deliberately defers commits.

## Self-Review

- Spec coverage: The plan covers grouped overview layout, removal of normal terminal dimensions, removal of raw top-level config paths, provider as an inline field, active hints, footer key clarity, model input clipping, bridge-owned data flow, provider selection through `config.select_provider`, draft updates through `config.update_draft`, and verification commands.
- Placeholder scan: No unfinished-marker language or unspecified "add tests" steps remain.
- Type consistency: The synthetic provider key is consistently `provider.__selected`; the local storage slot is consistently `provider.__selected` via `key.slice(9)`. `draftFormValues` strips `__selected` before draft update RPC calls.
