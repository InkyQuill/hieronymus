# TUI Design Improvements Spec

Scope: `frontend/src/admin/*`, `frontend/src/config/*`, `frontend/src/ui/*`.
Source of findings: design review of the OpenTUI admin/config screens
(persistent multi-panel layout, modal dialogs, config form).

This spec lists concrete, independently shippable fixes. Each item has a
problem statement, the fix, affected files, and acceptance criteria. Items are
grouped by priority tier; within a tier, order is not significant.

## Tier 1 — High impact, low cost

### 1.1 Fix mislabeled panel header in compact/narrow layout

**Problem**: In `AdminScreen.tsx` compact layout (`layout.kind !== "wide"`
branch), the panel header text is hardcoded to `"Detail Inspector"` for the
`helpOpen` and `commandsOpen` branches (lines ~857, ~866), but the same
hardcoded label also does not update when `activePanel === "views"` or
`"table"` is active without an overlay — the wide layout correctly shows
`Views` / `snapshot.view` / `Detail Inspector` per panel, but the compact
layout's overlay branches always claim to be the Detail Inspector even when
they're overlaying the Views or Table panel.

**Fix**: Compute the header label once from `activePanel` (or from
`helpOpen`/`commandsOpen` when those are active) and reuse it in all four
compact-layout branches, matching the wide-layout logic already used for
`Views` / `snapshot.view` / `Detail Inspector`.

**Files**: `frontend/src/admin/AdminScreen.tsx`

**Acceptance criteria**:
- Opening `?` help or `Ctrl+P` commands while `activePanel === "views"` shows
  "Views" (or equivalent) as the panel header, not "Detail Inspector".
- Existing compact-layout snapshot/interaction tests still pass; add a test
  asserting the header text matches `activePanel` when an overlay is open.

---

### 1.2 Replace text-only progress fields with gauge bars

**Problem**: `StatusPanels` in `AdminScreen.tsx` renders short-term drain and
dream progress as long concatenated strings (e.g. `Short-term pending 12 /
min 5 / max 30 urgent  drain 8/20 (40%) remaining 12`). This is slow to scan
and wraps unpredictably in narrower layouts.

**Fix**: Add a `Gauge` component (`label [████░░░░] value/max` using full
block characters, per the block-resolution building blocks pattern) and use
it for:
- short-term pending vs. min/max (`shortTermStatus.pending_count` /
  `max_pending_short_term_memories`)
- drain progress (`shortTermStatus.drain_progress`)
- dream progress (`dreamStatus.progress`)

Keep the existing textual detail (phase, run id, cycle id) as a trailing
label after the bar, not replacing it.

**Files**: new `frontend/src/ui/Gauge.tsx`, `frontend/src/admin/AdminScreen.tsx`
(`StatusPanels`), `frontend/src/config/ConfigScreen.tsx` if a comparable
progress value exists there.

**Acceptance criteria**:
- `Gauge` renders correctly at 0%, 50%, 100%, and when `max` is 0 (no
  division by zero).
- `Gauge` degrades to a plain `[value/max]` label when width is too small to
  render a bar (reuse `panelWidth`/layout classification, not a fixed
  threshold).
- Unit test covers bar fill rounding at boundary values (e.g. 1/3, 2/3).

---

### 1.3 Align `AdminTable` and `FocusableList` rows into columns

**Problem**: `AdminTable.tsx` renders `{row.label} [{row.status}]
{row.quality_label}` as one concatenated string with no column alignment.
Scanning status/quality across many rows requires reading each row
individually rather than visually scanning a column.

**Fix**: Compute per-column widths from the current `rows` (capped by
`width`), pad `label`, `status`, and `quality_label` to fixed widths, and
right-align or color-code the status/quality columns consistently. Truncate
overflowing labels with `…` rather than wrapping.

**Files**: `frontend/src/admin/AdminTable.tsx`

**Acceptance criteria**:
- Columns visually align across all rows at a given table width.
- Labels longer than the available column width are truncated with `…`, not
  wrapped or overflowing the box.
- Existing selection highlight (`>` marker + color) is preserved.

---

## Tier 2 — Medium impact

### 2.1 Introduce a semantic color/theme module

**Problem**: Named ANSI color strings (`"cyan"`, `"gray"`, `"red"`, `"green"`,
`"yellow"`) are hardcoded inline across `AdminScreen.tsx`, `AdminTable.tsx`,
`CommandPalette.tsx`, `DetailPane.tsx`, `HelpOverlay.tsx`, `dialogs.tsx`,
`ConfigScreen.tsx`, `ConfigForm.tsx`, and `StatusLine.tsx`. There is no single
place that defines what a color *means* (focus, error, success, muted), so
consistency depends on developers remembering to reuse the same string.

**Fix**: Add `frontend/src/ui/theme.ts` exporting semantic constants, e.g.:

```ts
export const theme = {
  accentPrimary: "cyan",
  accentMuted: "gray",
  statusError: "red",
  statusSuccess: "green",
  statusWarning: "yellow",
  fgDefault: undefined, // inherit terminal default
} as const;
```

Replace inline color strings with `theme.*` references incrementally,
starting with the files above. Do not introduce a runtime theme-switching
mechanism — this is a naming/consistency fix, not a new feature.

**Files**: new `frontend/src/ui/theme.ts`, then incremental replacement across
existing components.

**Acceptance criteria**:
- No new hardcoded color strings are introduced in touched files after this
  change; existing `fg="cyan"` etc. usages in touched files are replaced with
  `theme.accentPrimary` etc.
- `theme.ts` has no dependency on OpenTUI internals beyond the color type it
  already accepts (`string`), so it stays swappable later.

---

### 2.2 Render dialogs as an overlay instead of replacing the screen tree

**Problem**: In `AdminScreen.tsx`, when `dialog.kind !== "none"`, the entire
render tree is replaced by a centered `DialogOverlay` box (lines ~807–823).
The header, service status, stat panels, and status line are unmounted while
a dialog is open, so the user loses all surrounding context (which view was
active, service running state) for the duration of the dialog.

**Fix**: Render `DialogOverlay` as an absolutely positioned layer on top of
the existing screen tree (it already uses `position: "absolute"` internally
in `dialogs.tsx`) instead of gating the whole return value on `dialog.kind`.
Keep the backdrop dimmed (current black box is acceptable) but leave the
header/status/panels mounted underneath so state is not lost and closing the
dialog does not force a full re-render of the base layout.

**Files**: `frontend/src/admin/AdminScreen.tsx`, `frontend/src/admin/dialogs.tsx`

**Acceptance criteria**:
- Opening and closing a dialog does not change `activePanel`, `snapshot.view`,
  or scroll position of the table/detail panes.
- Header and status line remain present in the render tree (even if visually
  obscured by the modal) while a dialog is open — verify via test renderer
  snapshot that they are still mounted.

---

### 2.3 Deduplicate dialog field-focus navigation

**Problem**: `AddDialog`, `EditDialog`, `RenameDialog`, `MergeDialog`, and
`SplitDialog` in `dialogs.tsx` each reimplement `focusedIndex` state and
up/down navigation with slightly different clamping logic (`Math.max(0,
prev - 1)` / hardcoded `maxIndex` per dialog). This is six near-identical
implementations that must be kept in sync by hand.

**Fix**: Extract a `useFieldFocus(fieldCount: number)` hook returning
`{ focusedIndex, moveUp, moveDown }`, and use it in all five dialogs. Do not
change the visual layout or field order of any dialog — this is a
consistency/dedup fix, not a UX change.

**Files**: new `frontend/src/ui/useFieldFocus.ts`, `frontend/src/admin/dialogs.tsx`

**Acceptance criteria**:
- All five dialogs use the shared hook; no dialog reimplements clamped
  increment/decrement logic inline.
- Existing dialog keyboard tests (up/down/escape/enter behavior per dialog)
  continue to pass unmodified.

---

### 2.4 Add visible scroll indicator to `scrollbox` panels

**Problem**: `AdminTable` and `DetailPane` use `scrollbox` with a fixed
`height`, but there is no visible indicator of scroll position when content
exceeds the visible area. A user has no way to know whether a table/detail
pane has more rows below the fold.

**Fix**: Add a scrollbar indicator (OpenTUI's `scrollbar` component, per
`docs/opentui-conventions.md`'s note on visible scrollbar controls) to
`AdminTable` and `DetailPane` where content height exceeds the panel height.
For the memory editor textarea, this is explicitly deferred until OpenTUI
exposes a textarea scrollbar (per existing convention) — do not attempt it
here.

**Files**: `frontend/src/admin/AdminTable.tsx`, `frontend/src/admin/DetailPane.tsx`

**Acceptance criteria**:
- When row/content count exceeds visible height, a scrollbar or equivalent
  position indicator is visible.
- When content fits within the panel height, no scrollbar is rendered.

---

## Tier 3 — Polish, no functional risk

### 3.1 Strengthen header visual hierarchy

**Problem**: The Admin header (`Header` in `AdminScreen.tsx`) is two plain
`text` lines with no visual separation between branding, version, and live
service state. Service running/stopped state is only visible in the status
line at the bottom of the screen.

**Fix**: Add a right-aligned service status indicator (`● Service running` /
`○ Service stopped`, colored via `theme.statusSuccess` /
`theme.accentMuted`) on the same row as the version, so service state is
visible without scanning to the footer.

**Files**: `frontend/src/admin/AdminScreen.tsx` (`Header`)

**Acceptance criteria**:
- Service state indicator updates when `initial.service.running` /
  equivalent state changes (verify current data flow supports live updates;
  if `service.running` is bootstrap-only and never refreshed, note this as a
  follow-up rather than faking liveness).
- No additional height is added to the header (indicator shares the existing
  version row).

---

### 3.2 Differentiate disabled command-palette entries from selection dimming

**Problem**: In `CommandPalette.tsx`, disabled commands render `fg="gray"`
with an inline `(unavailable)` suffix — visually similar to unselected-but-
enabled rows, making it easy to miss why a command can't run.

**Fix**: Keep the command label at normal brightness for disabled commands,
but move the reason ("needs a selected row") to the hint line in
`theme.statusWarning` color when that command is highlighted, instead of an
inline gray suffix on every disabled row.

**Files**: `frontend/src/admin/CommandPalette.tsx`

**Acceptance criteria**:
- Disabled commands remain visually distinguishable from enabled ones (e.g.
  via a marker other than pure dimming) even in 16-color/NO_COLOR terminals.
- Reason for unavailability is shown when a disabled command is highlighted,
  not on every row.

---

## Explicitly out of scope

- Runtime-switchable themes (light/dark) — `theme.ts` centralizes constants
  only; no theme-switching UI.
- Mouse-driven panel focus — keyboard-first navigation is intentional per
  existing conventions.
- Any change to RPC methods, bridge boundaries, or backend-owned validation —
  all items above are frontend presentation changes only, per
  `docs/opentui-conventions.md`.
