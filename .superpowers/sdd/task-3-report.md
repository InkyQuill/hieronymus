# Task 3 report: Tailwind app shell migration

## Delivered

- Added a source-contract test for the required semantic Tailwind shell utilities.
- Migrated `frontend/src/web/App.svelte` to the approved responsive shell:
  - `min-h-dvh bg-root text-primary font-sans` root.
  - Sticky semantic header and responsive `max-w-[90rem]` workspace spacing.
  - Editorial providers layout with an aside/stage grid and utility-styled table states.
  - Utility-styled configuration navigation and loading/error states across every route branch.
- Preserved all route conditions, state and fetch/action functions, `onclick` callbacks,
  provider-row `onkeydown` behavior, ARIA labels, keyed editor mounting, and toast/modal
  mounting behavior.

## TDD evidence

1. Added `the app shell uses the semantic Tailwind surface utilities` to
   `frontend/src/web/app.test.ts`.
2. Ran `bun test src/web/app.test.ts`; it failed as expected because `App.svelte`
   did not contain `min-h-dvh`.
3. Migrated the markup and reran the focused test successfully.

## Verification

- `bun test src/web/app.test.ts` — 5 passing.
- `bun test` — 6 passing.
- `bun run typecheck` — passing.
- `bun run format` — passing.
- `git diff --check` — passing.

## Self-review

- The migration is scoped to `App.svelte` plus the required source-contract test.
- No child components or legacy CSS files were changed or removed.
- Existing accessibility semantics and keyboard behavior in the interactive provider rows remain unchanged.
