# Tailwind Rewrite Remediation Design

## Context

Pull request #21 migrates the local Svelte web console to Tailwind CSS v4 and adds a Hatch
build hook so packaged Python artifacts contain the compiled frontend. A repository review in
`report.md`, written against commit `11fdedc`, includes frontend, packaging, backend, security,
and maintenance findings. This remediation covers only findings introduced by, exposed by, or
necessary to complete the Tailwind rewrite.

The report predates commit `d72810c`, which removed the build hook's dependency on `mise`.
The missing-`mise` CI failure and the report's claim that the hook pins Bun independently are
therefore already resolved. The remaining duplicate-build problem is still current.

## Goals

- Make the Hatch hook the single packaging path that installs and builds frontend assets.
- Remove redundant frontend builds from release, installer, and backend verification flows.
- Replace source-string assertions that claim UI behavior with rendered component tests.
- Preserve narrow source-level tests where the source itself is the contract, such as Tailwind
  imports, theme declarations, and removal of legacy stylesheets.
- Remove CSS left unused by the Tailwind migration.
- Use one semantic danger-background treatment for error messages across the web console.
- Keep the change focused on the Tailwind rewrite and its packaging/test support.

## Non-Goals

- No backend, daemon, database, security, or general architecture findings from `report.md`.
- No new short-term-memory action or other resolution for the unreachable
  `remove_short_term_memory` constant.
- No API timeout behavior.
- No broader component redesign, visual refresh, or design-token change.
- No migration away from Bun as package manager or script launcher.
- No attempt to eliminate every use of custom CSS; `.data-table` and `.editor-dialog` remain
  because they are used and clearer as shared rules.

## Finding Disposition

### Included

1. **Duplicated frontend build recipes.** The new Hatch hook runs during `uv sync`, `uv build`,
   and `uv tool install`, while semantic-release, the shell installer, and backend CI also run
   explicit frontend builds. These duplicate the same work and can drift.
2. **Stale OpenTUI installer message.** The message lives in the redundant installer build
   helper and disappears when that helper is removed.
3. **Source-string tests presented as behavior tests.** The Tailwind migration introduced
   `app.test.ts`; several assertions prove only that class or markup strings occur in source.
4. **Dead Tailwind-migration CSS.** `.table-shell`, `.toggle-track`, and `.toggle-thumb` were
   introduced with `app.css` but are not referenced by any component.
5. **Inconsistent error backgrounds.** Tailwind conversion produced two alert treatments,
   `bg-raised` and `bg-[var(--hiero-danger-bg)]`, for the same error state.

### Included Only as Part of Test Modernization

The report flags `bun-types` in the shared TypeScript configuration. This is not independently
actionable: the current configuration includes Bun test files, so removing Bun types immediately
breaks type checking. The test migration will move browser component tests to Vitest with a DOM
environment and remove Bun-only source access from `app.test.ts`; TypeScript configuration and
test scripts can then stop exposing Bun globals to browser code. Bun remains the package manager
and command runner.

### Excluded

The unreachable `remove_short_term_memory` action and missing fetch timeout are real candidates
for separate frontend work, but they predate the Tailwind rewrite. All other report findings are
outside this PR's frontend styling/build scope.

## Build Architecture

`hatch_build.py` is the authoritative packaging integration:

1. CI provisions Bun `1.3.14` with `oven-sh/setup-bun`; the same version is declared in
   `frontend/package.json` for local tooling. The Hatch hook uses the `bun` executable from
   `$PATH` and does not read or install the declared package-manager version itself.
2. Any editable install or package build invokes the Hatch hook.
3. The hook runs the frozen frontend dependency install and Vite build once.
4. Hatch embeds `frontend/dist` in the Python artifact.

Consequences for callers:

- `pyproject.toml` changes semantic-release's `build_command` from
  `bun install --cwd frontend --frozen-lockfile && bun run --cwd frontend build && uv build`
  to exactly `uv build`; the Hatch hook owns the frontend build.
- `install.sh` runs `uv tool install` only after checkout/configuration; its separate
  `build_frontend` helper is removed. The two remaining “Hieronymus TUI” Bun error messages are
  changed to say that Bun `>=1.3` is required to build the Hieronymus web console as part of the
  same stale-migration cleanup.
- Backend PR/release verification keeps Bun setup before `uv sync`, but removes the explicit
  install/build steps after `uv sync`. Current GitHub jobs start from a fresh runner and do not
  restore `.venv`, so the editable project must be installed and the Hatch hook fires. The uv
  package cache does not make an editable install appear in a new virtual environment. CI does
  not add `--reinstall-package hieronymus`; if `.venv` caching is introduced later, that decision
  must be revisited and documented in the workflow.
- The dedicated frontend CI job retains install, formatting, type checking, tests, and build.
  It validates frontend development independently from Python packaging. This intentionally
  compiles the frontend once in backend packaging verification and once in frontend verification:
  the first proves the wheel integration, while the second proves frontend-only quality gates.

Tests around workflow and release configuration must assert this ownership explicitly so a
second build recipe cannot be added unnoticed.

## Frontend Test Architecture

Use `vitest`, `jsdom`, `@testing-library/svelte`, and `@testing-library/user-event` as frontend
development dependencies. Add `vitest.config.ts` with the existing Svelte Vite plugin, a `jsdom`
test environment, and a setup file that performs Testing Library cleanup. The test suite should
render components, interact with accessible controls, and assert visible state or callbacks
rather than search source files for markup.

Keep browser-source and test types separate. `tsconfig.json` covers production browser files with
`vite/client` types and excludes test files. A test TypeScript configuration extends it with
Vitest and Node types for test and configuration files. The `typecheck` script runs both configs,
and the `test` script runs `vitest run`. Bun still executes these package scripts.

The first behavior coverage remains deliberately focused on migration-sensitive paths:

- theme toggle changes the document theme and persists the choice;
- provider editor opens as a dialog, initializes its fields, submits edited data, and invokes
  close behavior;
- dreaming toggles and save controls update/submit current state;
- memory rows support mouse and Enter/Space selection, and destructive actions require
  confirmation before dispatch;
- error states render with the shared semantic danger treatment where class-level coverage is
  still warranted.

Static contract tests remain appropriate for facts that are inherently source/configuration
contracts: importing Tailwind, defining the data-theme dark variant, exposing required semantic
tokens/utilities, constraining the editor dialog rule, importing `app.css`, and deleting legacy
stylesheet files. These tests must be named as configuration or source-contract tests rather than
behavior tests. They use `node:fs/promises` (`readFile` and `access`) instead of `Bun.file`, so
removing Bun globals from the test type environment does not remove this coverage.

Network calls used by mounted components are mocked at the API-module boundary. Mock functions
are typed from the real API exports, fixtures use `satisfies` against the real response types, and
tests assert exact request arguments or callback payloads. Tests must assert component behavior,
not mock call counts alone.

## CSS Remediation

Remove `.table-shell`, `.toggle-track`, and `.toggle-thumb` from `app.css`. Keep `.data-table`
and `.editor-dialog` because current Svelte templates reference them.

All inline error alerts use `bg-[var(--hiero-danger-bg)]` together with `border-danger` and
`text-danger`. This retains the existing semantic CSS variable in both light and dark themes and
avoids introducing another component abstraction solely to deduplicate a class list.

## Error Handling

This remediation does not change request or domain error propagation. It changes only how
existing error strings are rendered and tested. Component tests cover rejected API calls where
needed to prove the error remains visible after the test harness changes.

Build failures remain fail-fast: a failed frozen Bun install or Vite build makes the Hatch build
fail, which in turn fails `uv sync`, `uv build`, or `uv tool install`. Before starting subprocesses,
the hook checks `shutil.which("bun")` and raises a concise `RuntimeError` explaining that Bun
`>=1.3` must be installed and available on `$PATH`; source installs must not fail with only a raw
`FileNotFoundError` traceback.

## Verification

Frontend verification:

```bash
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend format
bun run --cwd frontend typecheck
bun run --cwd frontend test
bun run --cwd frontend build
```

Packaging and repository verification:

```bash
uv sync --dev --reinstall-package hieronymus
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

The package-build verification must exercise the Hatch hook, not rely on a previously generated
`frontend/dist` directory alone.
