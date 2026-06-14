# OpenTUI Conventions

Hieronymus uses the React/OpenTUI app as a local management surface over
Python-owned domain APIs. These conventions are standing rules for future TUI
work; they are not one-off roadmap deliverables.

## Bridge Boundaries

- `hiero admin` mutations must go through `AdminBridge` RPC handlers.
- `hiero config` mutations must go through `ConfigBridge` RPC handlers.
- Frontend code must not write SQLite, apply migrations, mutate domain stores,
  or reimplement audited Python methods.
- Frontend code must not parse human CLI output from `hiero` commands. It may
  use structured JSON-RPC payloads from the TUI bridge.
- Domain validation, config file persistence, migrations, release updates,
  memory mutations, dreaming operations, and audit recording remain backend
  responsibilities.

## Config Editing

- `hiero config` should render Python-owned schemas, labels, hints, defaults,
  choices, redaction state, and validation metadata.
- Adding a new configuration file should primarily extend `ConfigBridge`
  payloads and save/load handling. The frontend should remain schema-driven
  where practical instead of hardcoding domain rules.

## Admin Inspection

- Admin detail panes should remain useful for dense inspection: structured
  fields, readable prose, JSON, diffs, provenance, recall reasons, dream output,
  and audit output must render inside the admin interface.
- The frontend may choose presentation widgets for detail bodies, but the source
  data and mutation decisions stay backend-owned.

## Keyboard And Layout

- `q`, `Esc`, `/`, `Tab`, `Shift+Tab`, arrows, and `hjkl` should stay
  consistent where useful.
- Footer hints should reflect the active mode and avoid documenting inactive
  keys.
- Responsive behavior must preserve the 60-column minimum floor, the 80x24
  compact layout, and wide layouts that do not overflow fixed panes.
