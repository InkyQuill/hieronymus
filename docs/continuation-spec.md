# Hieronymus Continuation Spec

This is the only active continuation spec after the multilingual memory and
OpenTUI migration work. Older execution plans were removed because they describe
work that has already landed or was superseded by the Bun/OpenTUI path. The
durable product vision from those plans now lives in
`docs/adr/0005-product-vision.md`.

## Current Baseline

- The interactive TUI is React/OpenTUI on Bun. `hiero config` and `hiero admin`
  launch the Bun frontend and bridge back into the same Python environment.
- Managed install, update, and release builds install frontend dependencies and
  rebuild `frontend/dist/main.js` before packaging or reinstalling.
- The memory model has language-neutral series, short-term metadata rows,
  concepts, facets, active rule crystals, enriched recall, dreaming audit
  records, and primitive MCP/admin operations.
- `hieronymus_read` and `hieronymus_learn` are no longer MCP judgment tools.
  Read/Learn/Remember behavior lives in agent skill workflows and writes
  short-term memory through primitives.

## Remaining Work

### 1. OpenTUI Runtime Polish

The frontend tests pass, but `bun test` still emits React `act(...)` warnings and
OpenTUI `TerminalConsoleCache` listener warnings.

Acceptance:

- `bun test` exits successfully without React `act(...)` warnings.
- Repeated OpenTUI test renders do not emit listener-leak warnings.
- The fix does not hide real React state-update failures by globally muting
  stderr.

### 2. Real TUI Smoke Coverage

Current coverage exercises the bridge and OpenTUI test renderer. Add a narrow
real-process smoke check for the packaged frontend path so install/build
regressions are caught before release.

Acceptance:

- A smoke test starts `bun frontend/dist/main.js config --bridge-command ...`
  against a temporary data root and verifies the first frame or bootstrap path
  without requiring a human terminal session.
- A matching admin smoke checks `admin` startup.
- Smoke tests are skipped cleanly when the environment cannot provide a PTY or
  Bun runtime.

### 3. Daemon-Backed Command Path

The service toolkit still notes that some commands access storage directly.
Move agent-facing command/adaptor paths toward service discovery where that
improves concurrency and deployment behavior.

Acceptance:

- Document which commands remain intentionally direct-to-SQL and why.
- For commands that should use the local daemon, route through the service
  client with clear fallback behavior when the daemon is unavailable.
- Preserve local-first operation and existing `--data-root` behavior.

### 4. Deferred Agent Integrations

Several agent integration targets remain reserved or stubbed.

Acceptance:

- Replace stub output for any integration that is ready to support a real host
  protocol.
- Keep reserved targets explicit when no host protocol is implemented.
- Add install/update tests for every integration that writes host configuration.

### 5. Provider-Backed Dreaming Smoke

The deterministic and parser paths are well covered. Remote/provider-backed
phase behavior still needs targeted smoke coverage before enabling more
workflow automation by default.

Acceptance:

- Add a fixture provider that exercises multi-phase provider payloads through
  crystallization and maintenance paths.
- Verify audit records include provider request/response payloads, affected
  memory set summaries, parse warnings, and maintenance decisions.
- Keep deterministic fallback explicit; it must not silently replace invalid
  configured provider workflows.
