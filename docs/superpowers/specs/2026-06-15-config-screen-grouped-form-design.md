# hiero config Grouped Form Design

## Context

User testing of `hiero config` found these issues:

- Text entry in the Model field can hide the first typed characters.
- Configuration is presented as one flat list without enough explanation.
- "Config files" is unclear as a top-level status line.
- Terminal size is shown during normal use even though it is not actionable.
- Provider selection feels disconnected on a separate tab/pane.
- Footer help does not visually distinguish keys from descriptions.
- Provider/API access, dreaming, ingest, and release settings are mixed together.

The existing bridge already exposes form metadata through `form_schema.sections`,
`form_schema.groups`, and `form_schema.fields`. The main problem is the
React/OpenTUI presentation, not the persistence model.

## Goal

Make `hiero config` a compact grouped editor:

- Keep one overview screen instead of a wizard.
- Show configuration settings grouped in this order:
  1. Provider and API access
  2. Dreaming
  3. Ingestion
  4. Updates
- Explain the active group and active field without adding onboarding screens.
- Remove non-actionable header noise.
- Preserve fast repeated editing with keyboard navigation.

## Non-Goals

- Do not redesign the Python config file format.
- Do not move runtime databases or book project state.
- Do not introduce fuzzy or semantic behavior into strict configuration rules.
- Do not build a first-run onboarding wizard.
- Do not implement a graphical browser UI for configuration.

## Proposed Approach

Use the existing bridge contract and improve the TUI structure.

`ConfigScreen` should render a single configuration editor. The separate
provider panel should disappear from the primary layout. Provider choice should
be represented as the first editable field in the Provider/API group, backed by
the existing `provider_choices` and `selected_provider` data. If the existing
schema cannot express that cleanly, add one synthetic frontend field for
provider selection rather than changing persisted config shape.

`ConfigForm` should group fields by `form_schema.groups`, preserving the group
order from the bridge. Each group should show:

- A concise group title.
- The owning config file label when useful, such as `dream.conf`, `ingest.conf`,
  or `release.conf`.
- Fields belonging to the group.
- One active-field hint line for the focused field.

The active group should be visually distinct. Inactive groups should remain
visible where terminal space allows. Compact terminals may window the visible
field range, but the current group heading and active hint must stay visible.

## Header

The normal header should show:

- `Hieronymus Config`
- A compact group breadcrumb: `Provider/API | Dreaming | Ingest | Release`

The header should not show terminal size in normal layouts. Terminal dimensions
belong only on the too-small screen, where they explain why editing is blocked.

Raw config paths should not be shown as a long top-level status line. File
ownership can appear near group titles instead:

- Provider/API: `dream.conf`
- Dreaming: `dream.conf`
- Ingestion: `ingest.conf`
- Updates: `release.conf`

## Editing Behavior

Navigation should remain keyboard-first:

- Up/down moves between fields.
- Enter starts or confirms editing.
- Escape cancels an active edit.
- Search remains available.
- Save, reload, provider check, and quit remain global commands.

Provider selection should behave like a choice field in the Provider/API group.
Changing it should continue to call `config.select_provider` so provider defaults,
model suggestions, and redacted secret handling remain Python-owned.

Text, number, and secret inputs must use a stable visible width based on the
available row width after the label. The Model field must preserve leading
characters visually while editing and after submission.

## Footer

Footer help should distinguish keys from explanations. Use a compact repeated
pattern such as:

`[↑↓] field  [Enter] edit  [s] save  [/] search  [c] check  [q] quit`

Keys should be visually highlighted differently from descriptive words. The
compact footer should avoid ambiguous phrases such as `Tab pane / search`.

## Data Flow

The data flow remains:

1. Python `ConfigBridge.bootstrap` emits provider choices, selected provider,
   form values, form schema, validation, suggestions, and detail payloads.
2. React keeps local draft state for responsive text editing.
3. Field submission calls `config.update_draft`.
4. Provider choice calls `config.select_provider`.
5. Save calls `config.save`.

No frontend code should write config files directly.

## Error Handling

Validation errors remain sourced from the bridge. The grouped form should show
errors close to the current editing context where practical:

- Global validation errors can remain near the status area.
- Field-specific errors should be associated with the corresponding field if the
  bridge payload contains enough information.
- Provider check errors stay visible below or near the Provider/API group.

The UI should continue to redact configured secrets in all displayed payloads.

## Testing

Add or update frontend OpenTUI tests to verify:

- The provider selector is not rendered as a separate pane in the normal layout.
- Groups render in the expected order.
- Provider choice appears in the Provider/API group.
- Normal compact layout does not show terminal dimensions.
- The footer visually separates keys from descriptions.
- Editing the Model field preserves leading characters.
- Group headings and active hints remain visible in compact terminal layouts.

Run full project verification before claiming implementation complete:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```
