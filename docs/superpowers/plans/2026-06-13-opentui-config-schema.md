# OpenTUI Config Schema Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `hiero config` render Python-owned field labels, hints, grouping, redaction metadata, choices, defaults, and validation metadata instead of duplicating config semantics in React.

**Architecture:** `ConfigBridge` remains the mutation and validation boundary. Python emits a stable `form_schema` payload describing editable fields and field-level validation, while React/OpenTUI renders those field definitions against `form_values`. The frontend keeps keyboard behavior and local draft state, but no longer owns config labels, placeholders, choices, redaction semantics, or field grouping.

**Tech Stack:** Python 3.12, pytest, Pydantic-free dict payloads, TypeScript, React/OpenTUI on Bun, Zod runtime schemas.

---

## Current Code Map

- `src/hieronymus/tui_bridge/config_api.py`: builds `ConfigBridge` payloads, validates and saves `dream.conf`, `ingest.conf`, and `release.conf`, and currently returns `form_values` without a schema describing those values.
- `src/hieronymus/tui_bridge/config_state.py`: contains parsing helpers for config form values.
- `frontend/src/config/ConfigForm.tsx`: owns the hardcoded `fieldDefinitions` array with labels, keys, placeholders, input types, choices, and display order.
- `frontend/src/config/ConfigScreen.tsx`: imports `fieldDefinitions`, uses its length for navigation, and passes form values into `ConfigForm`.
- `frontend/src/rpc/schema.ts`: parses config bootstrap payloads but has no `form_schema` contract.
- `frontend/src/rpc/schema.test.ts`: covers config payload parsing.
- `frontend/src/config/ConfigScreen.test.tsx`: covers rendered config form behavior.
- `tests/test_tui_bridge_config.py`: covers `ConfigBridge` payloads, save/check behavior, redaction, and validation.
- `docs/roadmap.md`: tracks the OpenTUI item this plan targets.

---

### Task 1: Add Python-Owned Config Form Schema Payload

**Files:**
- Modify: `src/hieronymus/tui_bridge/config_api.py`
- Test: `tests/test_tui_bridge_config.py`

- [ ] **Step 1: Write failing backend schema test**

Add this test near the existing config bootstrap tests in `tests/test_tui_bridge_config.py`:

```python
def test_config_bootstrap_exposes_python_owned_form_schema(tmp_path: Path) -> None:
    payload = ConfigBridge(_config(tmp_path)).bootstrap({})

    schema = payload["form_schema"]
    assert [group["id"] for group in schema["groups"]] == [
        "provider",
        "dreaming",
        "release",
        "ingest",
    ]
    assert schema["groups"][0] == {
        "id": "provider",
        "label": "Provider",
        "description": "Connection settings for the selected dream provider.",
    }
    fields = {field["key"]: field for field in schema["fields"]}
    assert fields["provider.model"] == {
        "key": "provider.model",
        "group": "provider",
        "label": "Model",
        "hint": "Workflow model used by the selected provider.",
        "placeholder": "e.g. gpt-4.1-mini",
        "type": "text",
        "redacted": False,
    }
    assert fields["provider.api_key"] == {
        "key": "provider.api_key",
        "group": "provider",
        "label": "API Key",
        "hint": "Stored as plaintext in dream.conf and redacted in UI payloads.",
        "placeholder": "stored in dream.conf",
        "type": "secret",
        "redacted": True,
    }
    assert fields["release.update_channel"]["type"] == "choice"
    assert fields["release.update_channel"]["choices"] == ["stable", "dev"]
    assert fields["ingest.max_block_chars"]["default"] == "1200"
```

- [ ] **Step 2: Run the focused test to verify failure**

```bash
uv run pytest tests/test_tui_bridge_config.py::test_config_bootstrap_exposes_python_owned_form_schema -q
```

Expected: FAIL with `KeyError: 'form_schema'`.

- [ ] **Step 3: Add schema helper functions**

In `src/hieronymus/tui_bridge/config_api.py`, add these helpers below `REMOTE_PROVIDERS`:

```python
def _form_schema() -> dict[str, object]:
    return {
        "groups": [
            {
                "id": "provider",
                "label": "Provider",
                "description": "Connection settings for the selected dream provider.",
            },
            {
                "id": "dreaming",
                "label": "Dreaming",
                "description": "Autostart thresholds for turning short-term memory into durable memory.",
            },
            {
                "id": "release",
                "label": "Updates",
                "description": "Managed install update channel.",
            },
            {
                "id": "ingest",
                "label": "Ingestion",
                "description": "Limits for short-term memory and Learn ingestion.",
            },
        ],
        "fields": [
            _field(
                "provider.model",
                "provider",
                "Model",
                "text",
                hint="Workflow model used by the selected provider.",
                placeholder="e.g. gpt-4.1-mini",
            ),
            _field(
                "provider.api_key",
                "provider",
                "API Key",
                "secret",
                hint="Stored as plaintext in dream.conf and redacted in UI payloads.",
                placeholder="stored in dream.conf",
                redacted=True,
            ),
            _field(
                "provider.api_path",
                "provider",
                "API Path",
                "text",
                hint="Base URL for OpenAI-compatible, Gemini, or Anthropic gateways.",
                placeholder="e.g. https://api.openai.com/v1",
            ),
            _field(
                "provider.timeout_seconds",
                "provider",
                "Timeout (seconds)",
                "number",
                hint="Provider check and model-list timeout.",
                placeholder="e.g. 30",
                minimum=1,
            ),
            _field(
                "dreaming.autostart_enabled",
                "dreaming",
                "Autostart Enabled",
                "toggle",
                hint="Whether scheduled dreaming can run automatically.",
                choices=["yes", "no"],
                default="no",
            ),
            _field(
                "dreaming.min_interval_minutes",
                "dreaming",
                "Min Interval (minutes)",
                "number",
                hint="Minimum minutes between scheduled dream cycles.",
                placeholder="e.g. 30",
                minimum=1,
            ),
            _field(
                "dreaming.new_short_term_memory_threshold",
                "dreaming",
                "New Memory Threshold",
                "number",
                hint="Pending short-term memories required before scheduled dreaming runs.",
                placeholder="e.g. 25",
                minimum=1,
            ),
            _field(
                "release.update_channel",
                "release",
                "Update Channel",
                "choice",
                hint="Stable uses release tags; dev tracks the configured development target.",
                choices=["stable", "dev"],
                default="stable",
            ),
            _field(
                "ingest.warning_sentence_count",
                "ingest",
                "Memory Warn Sentences",
                "number",
                hint="Warn when direct short-term memory exceeds this sentence count.",
                placeholder="e.g. 6",
                default="6",
                minimum=1,
            ),
            _field(
                "ingest.rejection_sentence_count",
                "ingest",
                "Memory Reject Sentences",
                "number",
                hint="Reject direct short-term memory above this sentence count.",
                placeholder="e.g. 30",
                default="30",
                minimum=1,
            ),
            _field(
                "ingest.max_block_chars",
                "ingest",
                "Learn Block Characters",
                "number",
                hint="Maximum Learn block size before splitting.",
                placeholder="e.g. 1200",
                default="1200",
                minimum=1,
            ),
        ],
    }


def _field(
    key: str,
    group: str,
    label: str,
    field_type: str,
    *,
    hint: str,
    placeholder: str = "",
    choices: list[str] | None = None,
    default: str = "",
    minimum: int | None = None,
    redacted: bool = False,
) -> dict[str, object]:
    payload: dict[str, object] = {
        "key": key,
        "group": group,
        "label": label,
        "hint": hint,
        "placeholder": placeholder,
        "type": field_type,
        "redacted": redacted,
    }
    if choices is not None:
        payload["choices"] = choices
    if default:
        payload["default"] = default
    if minimum is not None:
        payload["minimum"] = minimum
    return payload
```

- [ ] **Step 4: Include `form_schema` in payload**

In `ConfigBridge._payload()`, add this key next to `form_values`:

```python
"form_schema": _form_schema(),
```

- [ ] **Step 5: Run backend config tests**

```bash
uv run pytest tests/test_tui_bridge_config.py -q
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/tui_bridge/config_api.py tests/test_tui_bridge_config.py
git commit -m "feat: expose config form schema"
```

---

### Task 2: Parse Config Form Schema In Frontend RPC

**Files:**
- Modify: `frontend/src/rpc/schema.ts`
- Test: `frontend/src/rpc/schema.test.ts`

- [ ] **Step 1: Add failing schema test**

In `frontend/src/rpc/schema.test.ts`, add this helper below `configPaths()`:

```typescript
function configPayload(
  provider: "openai" | "gemini" | "anthropic" = "openai",
  overrides: Record<string, unknown> = {},
) {
  return {
    config_paths: configPaths(),
    provider_choices: [
      {
        name: provider,
        display_name:
          provider === "openai"
            ? "OpenAI compatible"
            : provider === "gemini"
              ? "Gemini"
              : "Anthropic",
        requires_api_key: true,
        supports_api_path: provider === "openai",
      },
    ],
    selected_provider: provider,
    draft: configDraft(provider),
    form_values: {
      provider: {
        model: provider === "gemini" ? "gemini-2.5-flash" : "gpt-4.1-mini",
      },
      dreaming: {},
      ingest: {},
      release: {},
    },
    validation: { ok: true, errors: [] },
    suggestions: {},
    detail: {},
    ...overrides,
  };
}
```

Then add:

```typescript
it("parses Python-owned config form schema", () => {
  const payload = ConfigBootstrapSchema.parse(configPayload("openai", {
    form_schema: {
      groups: [
        {
          id: "provider",
          label: "Provider",
          description: "Connection settings for the selected dream provider.",
        },
      ],
      fields: [
        {
          key: "provider.api_key",
          group: "provider",
          label: "API Key",
          hint: "Stored as plaintext in dream.conf and redacted in UI payloads.",
          placeholder: "stored in dream.conf",
          type: "secret",
          redacted: true,
        },
        {
          key: "release.update_channel",
          group: "release",
          label: "Update Channel",
          hint: "Managed install update channel.",
          placeholder: "",
          type: "choice",
          choices: ["stable", "dev"],
          default: "stable",
          redacted: false,
        },
      ],
    },
  }));

  expect(payload.form_schema.fields[0].type).toBe("secret");
  expect(payload.form_schema.fields[1].choices).toEqual(["stable", "dev"]);
});
```

- [ ] **Step 2: Run schema test to verify failure**

```bash
bun --cwd frontend test src/rpc/schema.test.ts
```

Expected: FAIL because `form_schema` is not parsed.

- [ ] **Step 3: Add Zod schema types**

In `frontend/src/rpc/schema.ts`, add before `ConfigBootstrapSchema`:

```typescript
const ConfigFieldTypeSchema = z.enum([
  "text",
  "secret",
  "number",
  "toggle",
  "choice",
]);

const ConfigFormGroupSchema = z
  .object({
    id: z.string(),
    label: z.string(),
    description: z.string().default(""),
  })
  .passthrough();

const ConfigFormFieldSchema = z
  .object({
    key: z.string(),
    group: z.string(),
    label: z.string(),
    hint: z.string().default(""),
    placeholder: z.string().default(""),
    type: ConfigFieldTypeSchema,
    choices: z.array(z.string()).default([]),
    default: z.string().default(""),
    minimum: z.number().optional(),
    redacted: z.boolean().default(false),
  })
  .passthrough();

const ConfigFormSchemaSchema = z.object({
  groups: z.array(ConfigFormGroupSchema),
  fields: z.array(ConfigFormFieldSchema),
});
```

Add to `ConfigBootstrapSchema`:

```typescript
form_schema: ConfigFormSchemaSchema.default({ groups: [], fields: [] }),
```

Export inferred types near the existing exports:

```typescript
export type ConfigFormField = z.infer<typeof ConfigFormFieldSchema>;
export type ConfigFormGroup = z.infer<typeof ConfigFormGroupSchema>;
```

- [ ] **Step 4: Run frontend schema tests and typecheck**

```bash
bun --cwd frontend test src/rpc/schema.test.ts
bun run --cwd frontend typecheck
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/rpc/schema.ts frontend/src/rpc/schema.test.ts
git commit -m "feat: parse config form schema"
```

---

### Task 3: Render Config Form From Server Schema

**Files:**
- Modify: `frontend/src/config/ConfigForm.tsx`
- Modify: `frontend/src/config/ConfigScreen.tsx`
- Test: `frontend/src/config/ConfigScreen.test.tsx`

- [ ] **Step 1: Add failing frontend render test**

In `frontend/src/config/ConfigScreen.test.tsx`, add this helper above `payload()`:

```typescript
function formSchema(fields = [
  {
    key: "provider.model",
    group: "provider",
    label: "Model",
    hint: "Model name used by the selected dream provider.",
    placeholder: "gpt-4.1-mini",
    type: "text" as const,
    choices: [],
    default: "",
    redacted: false,
  },
  {
    key: "provider.api_key",
    group: "provider",
    label: "API Key",
    hint: "Stored as plaintext in dream.conf and redacted in UI payloads.",
    placeholder: "stored in dream.conf",
    type: "secret" as const,
    choices: [],
    default: "",
    redacted: true,
  },
]) {
  return {
    groups: [
      {
        id: "provider",
        label: "Provider",
        description: "Connection settings for the selected dream provider.",
      },
    ],
    fields,
  };
}
```

In the existing `payload(selectedProvider: ProviderName = "openai")` helper, add this top-level property:

```typescript
form_schema: formSchema(),
```

Then add this test:

```typescript
it("renders config fields from backend schema", async () => {
  const { root, flush, captureCharFrame } = await setupTest();

  root.render(
    <ConfigScreen
      initial={{
        ...payload(),
        form_schema: formSchema([
          {
            key: "provider.model",
            group: "provider",
            label: "Backend Model Label",
            hint: "Backend-owned model hint.",
            placeholder: "backend placeholder",
            type: "text",
            choices: [],
            default: "",
            redacted: false,
          },
        ]),
      }}
      client={undefined}
    />,
  );
  await flush();

  expect(captureCharFrame()).toContain("Backend Model Label");
});
```

The assertion must fail before implementation because the current `ConfigForm` ignores `payload.form_schema.fields` and renders the local `fieldDefinitions` constant.

- [ ] **Step 2: Run frontend config test to verify failure**

```bash
bun --cwd frontend test src/config/ConfigScreen.test.tsx
```

Expected: FAIL because `ConfigForm` still uses static `fieldDefinitions`.

- [ ] **Step 3: Replace static field definitions prop**

In `frontend/src/config/ConfigForm.tsx`, remove the exported hardcoded `fieldDefinitions` array. Import `ConfigFormField` from `../rpc/schema.js` and change props:

```typescript
import type { ConfigFormField } from "../rpc/schema.js";

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
  onFieldChange: (key: string, value: string) => void;
  onSubmitField: () => void;
};
```

Change value resolution to use `fields`:

```typescript
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
  return { ...field, value };
});
```

Render `renderedFields` instead of `fields` from the old constant. For choices, use:

```tsx
{(field.choices.length ? field.choices : ["yes", "no"]).map((choice) => (
  <text key={choice} fg={field.value === choice ? "cyan" : "gray"}>
    [{choice}]{" "}
  </text>
))}
```

For editable fields, keep `<input />` for `text`, `secret`, and `number`.

- [ ] **Step 4: Pass schema fields from ConfigScreen**

In `frontend/src/config/ConfigScreen.tsx`, remove the `fieldDefinitions` import:

```typescript
import { ConfigForm, fieldDefinitions } from "./ConfigForm.js";
```

Replace with:

```typescript
import { ConfigForm } from "./ConfigForm.js";
```

Add:

```typescript
const formFields = payload.form_schema.fields;
```

Replace:

```typescript
const focusedField = fieldDefinitions[focusedFieldIndex];
```

with:

```typescript
const focusedField = formFields[focusedFieldIndex];
```

Replace:

```typescript
Math.min(fieldDefinitions.length - 1, prev + 1)
```

with:

```typescript
Math.min(Math.max(formFields.length - 1, 0), prev + 1)
```

Guard edit entry when no fields exist:

```typescript
if (enter && formFields.length > 0) {
  setIsEditing(true);
  return;
}
```

Pass fields:

```tsx
<ConfigForm
  fields={formFields}
  formValues={localFormValues}
  focusedFieldIndex={focusedFieldIndex}
  isEditing={isEditing}
  focused={activePanel === "form"}
  onFieldChange={handleFieldChange}
  onSubmitField={submitField}
/>
```

When clamping focus after payload changes, add:

```typescript
useEffect(() => {
  setFocusedFieldIndex((index) => Math.min(index, Math.max(formFields.length - 1, 0)));
}, [formFields.length]);
```

This is the complete list of `fieldDefinitions` references to remove from `ConfigScreen.tsx`: the import line, `const focusedField = fieldDefinitions[focusedFieldIndex];`, `fieldDefinitions.length`, and the implicit `ConfigForm` call that currently lacks a `fields` prop.

- [ ] **Step 5: Run frontend tests**

```bash
bun --cwd frontend test src/config/ConfigScreen.test.tsx src/rpc/schema.test.ts
bun run --cwd frontend typecheck
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/config/ConfigForm.tsx frontend/src/config/ConfigScreen.tsx frontend/src/config/ConfigScreen.test.tsx
git commit -m "feat: render config form from schema"
```

---

### Task 4: Add Field-Level Validation Metadata

**Files:**
- Modify: `src/hieronymus/tui_bridge/config_api.py`
- Modify: `frontend/src/rpc/schema.ts`
- Test: `tests/test_tui_bridge_config.py`
- Test: `frontend/src/rpc/schema.test.ts`

- [ ] **Step 1: Write failing backend validation metadata test**

In `tests/test_tui_bridge_config.py`, add:

```python
def test_config_validation_includes_field_error_metadata(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))
    payload = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1-mini",
                "api_key": "",
                "api_path": "https://llm.example.test/v1",
                "timeout_seconds": "0",
            },
        }
    )

    assert payload["validation"]["ok"] is False
    assert payload["validation"]["field_errors"] == {
        "provider.timeout_seconds": ["providers.openai.timeout_seconds must be at least 1"]
    }
```

- [ ] **Step 2: Run backend test to verify failure**

```bash
uv run pytest tests/test_tui_bridge_config.py::test_config_validation_includes_field_error_metadata -q
```

Expected: FAIL because `field_errors` is absent.

- [ ] **Step 3: Add Python field error mapping**

In `src/hieronymus/tui_bridge/config_api.py`, change `_payload()` validation construction from:

```python
"validation": {"ok": not errors, "errors": errors},
```

to:

```python
"validation": {
    "ok": not errors,
    "errors": errors,
    "field_errors": _field_errors(errors, selected),
},
```

Add this helper near `_load_errors()`:

```python
def _field_errors(errors: list[str], selected: str) -> dict[str, list[str]]:
    mapping: dict[str, list[str]] = {}
    for error in errors:
        field = _field_for_error(error, selected)
        if field:
            mapping.setdefault(field, []).append(error)
    return mapping


def _field_for_error(error: str, selected: str) -> str:
    selected_provider_prefix = f"providers.{selected}."
    if error.startswith(selected_provider_prefix):
        provider_field = error.removeprefix(selected_provider_prefix).split(" ", 1)[0]
        return {
            "model": "provider.model",
            "endpoint": "provider.api_path",
            "timeout_seconds": "provider.timeout_seconds",
            "api_key": "provider.api_key",
        }.get(provider_field, "")
    if error.startswith("schedule_interval_minutes "):
        return "dreaming.min_interval_minutes"
    if error.startswith("min_pending_short_term_memories "):
        return "dreaming.new_short_term_memory_threshold"
    if error.startswith("short_memory.warning_sentence_count "):
        return "ingest.warning_sentence_count"
    if error.startswith("short_memory.rejection_sentence_count "):
        return "ingest.rejection_sentence_count"
    if error.startswith("learn.max_block_chars "):
        return "ingest.max_block_chars"
    if error.startswith("updates.channel "):
        return "release.update_channel"
    return ""
```

- [ ] **Step 4: Add frontend schema coverage**

In `frontend/src/rpc/schema.ts`, change validation schema to include:

```typescript
field_errors: z.record(z.array(z.string())).default({}),
```

In `frontend/src/rpc/schema.test.ts`, add:

```typescript
it("parses config field validation errors", () => {
  const payload = ConfigBootstrapSchema.parse(configPayload("openai", {
    validation: {
      ok: false,
      errors: ["providers.openai.timeout_seconds must be at least 1"],
      field_errors: {
        "provider.timeout_seconds": ["providers.openai.timeout_seconds must be at least 1"],
      },
    },
  }));

  expect(payload.validation.field_errors["provider.timeout_seconds"]).toEqual([
    "providers.openai.timeout_seconds must be at least 1",
  ]);
});
```

- [ ] **Step 5: Run validation tests**

```bash
uv run pytest tests/test_tui_bridge_config.py -q
bun --cwd frontend test src/rpc/schema.test.ts
bun run --cwd frontend typecheck
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/tui_bridge/config_api.py frontend/src/rpc/schema.ts frontend/src/rpc/schema.test.ts tests/test_tui_bridge_config.py
git commit -m "feat: add config field validation metadata"
```

---

### Task 5: Update Roadmap And Verify

**Files:**
- Modify: `docs/roadmap.md`
- Test: backend and frontend verification commands

- [ ] **Step 1: Update roadmap**

In `docs/roadmap.md`, move this OpenTUI item out of remaining work:

```markdown
- Move config field labels, hints, grouping, defaults, field types, redaction
  behavior, and validation errors into Python-owned config schema payloads. The
  frontend should render those contracts rather than duplicating config
  semantics.
```

Add this to the OpenTUI baseline paragraph or a new completed bullet:

```markdown
- `hiero config` receives Python-owned field schema payloads for labels, hints,
  groups, input types, choices, redaction behavior, defaults, and field-level
  validation metadata.
```

- [ ] **Step 2: Run full verification**

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend format
bun run --cwd frontend typecheck
bun --cwd frontend test
bun run --cwd frontend build
git diff --check
```

Expected: all commands pass. Existing React `act(...)` and OpenTUI `TerminalConsoleCache` warnings may still appear in Bun tests unless addressed by a separate warning-cleanup slice.

- [ ] **Step 3: Commit docs**

```bash
git add docs/roadmap.md
git commit -m "docs: update opentui config schema status"
```

---

## Self-Review

Spec coverage:

- Python owns labels, hints, grouping, defaults, field types, choices, and redaction metadata through `form_schema`: Tasks 1 and 2.
- Frontend renders the Python-owned schema rather than hardcoded `fieldDefinitions`: Task 3.
- Validation metadata becomes field-addressable for frontend rendering and later UX polish: Task 4.
- Roadmap reflects completion and final verification runs: Task 5.

Placeholder scan:

- No placeholder markers, shortcut references, or undefined future helpers remain.
- All tasks include concrete files, code snippets, commands, expected failures, expected passes, and commit messages.

Type consistency:

- Backend key names are `form_schema.groups`, `form_schema.fields`, `field.key`, `field.group`, `field.label`, `field.hint`, `field.placeholder`, `field.type`, `field.choices`, `field.default`, `field.minimum`, and `field.redacted`.
- Frontend types use `ConfigFormField` and `ConfigFormGroup`, matching the Zod schema and React props.
