# Admin memory views page

## Goal

Make memory exploration the primary admin workflow without making the overview
route load every memory view.

## Routes

- `/admin` is an Overview page. It shows local service status, dreaming status,
  aggregate memory statistics, and a link to the configuration console.
- `/admin/memory` is the Memory views page. It loads admin snapshots only after
  the page is opened and lets the user choose an existing backend view such as
  Crystals, Concepts, Renderings, and Audit Log.

## UI

The shared sidebar has separate `Overview` and `Memory views` entries. The
memory page dedicates its content area to the view selector, rows, and selected
detail, rather than nesting them in the overview status cards.

## Data flow and errors

`/admin` continues to request only `/api/admin/dashboard`. `/admin/memory`
requests the dashboard metadata first, then `/api/admin/snapshot?view=...` for
the selected view. Existing authenticated local-session handling is unchanged.
Snapshot failures remain visible inline and do not replace the rest of the page.

## Verification

Add HTTP routing coverage for `/admin/memory`, run Svelte compilation and
autofixer checks, then run the relevant Python tests and repository linting.
