# Hieronymus Roadmap

Hieronymus is still an alpha local-first translation memory system. The current
codebase already contains the core memory graph, primitive MCP tools, local
service, React/OpenTUI management app, install/update flow, and dreaming
pipeline. This roadmap records what remains without turning those items into a
locked implementation plan.

## Product Surfaces

### Memory And Dreaming

Dreaming is the path from short-term observations to durable memory. It remains
bounded, auditable, and local-first.

Current baseline:

- `dream.conf` is the canonical configuration file for dreaming providers,
  workflows, prompts, thresholds, caps, and plaintext local API keys.
- The old provider settings file has been removed without migration because the
  project is pre-release.
- `ingest.conf` is the global data-root configuration file for ingestion policy,
  including direct short-term memory warning/rejection thresholds and
  Learn-style block splitting limits.
- Short-term memory ingestion supports separate sentence and symbol warning and
  rejection thresholds, with symbol thresholds disabled by default.
- Invalid configured provider workflows are rejected instead of silently falling
  back to deterministic dreaming.
- Provider-backed crystallization audit coverage verifies provider request and
  response summaries, parse warnings, selected memory IDs, and affected memory
  set payloads.

Remaining work:

- Add provider-backed dreaming smoke coverage that exercises multi-phase
  provider payloads through crystallization and maintenance paths.
- Extend dream audit coverage to maintenance decisions and multi-phase provider
  runs.

### MCP And Agent Integrations

MCP exposes primitives. Agent integrations and skills contain judgment-heavy
workflow behavior.

Remaining work:

- Replace stub output for integrations whose host protocol is known and ready.
- Keep reserved targets explicit when no host protocol is implemented.
- Add install/update tests for every integration that writes host configuration.
- Continue to keep Read, Learn, and Remember as agent skill workflows rather
  than MCP judgment wrappers.

### CLI And Service

The CLI is both a human tool and an automation surface. Human output should be
clear, while JSON output remains stable for scripts.

Remaining work:

- Document which commands intentionally access SQLite directly and why.
- Route agent-facing command/adaptor paths through local service discovery where
  that improves concurrency or deployment behavior.
- Preserve local-first operation and existing `--data-root` behavior.
- Improve CLI help quality with examples, clearer command grouping, and alpha
  status language.
- Keep machine-readable output behind `--json`; avoid parsing human CLI output
  from other components.

### OpenTUI Management App

The React/OpenTUI app on Bun is the first-class local configuration and memory
administration surface. It is real but early; remaining work is polish and
completeness, not another framework migration.

Completed baseline:

- `hiero config` receives Python-owned field schema payloads for labels, hints,
  groups, input types, choices, redaction behavior, defaults, and field-level
  validation metadata.
- Packaged `config` and `admin` startup have real-process smoke checks through
  `frontend/dist/main.js`, skipped cleanly when Bun, a POSIX PTY, or the built
  bundle is unavailable.
- CI builds the frontend bundle before backend pytest so packaged startup
  coverage stays exercised in workflow runs.

Remaining work:

- Extend `hiero config` beyond the current `dream.conf`, `ingest.conf`, and
  `release.conf` editor as future configuration files are added.
- Keep TUI admin and config mutations behind `AdminBridge` and `ConfigBridge`
  RPC handlers, existing domain stores, migrations, and audited Python methods.
- Ensure the frontend does not write SQLite, parse human CLI output, or
  duplicate migration/domain mutation logic.
- Convert the command palette from a static command list into a real keyboard
  picker with command names, keybindings, availability state, and bridge-backed
  execution.
- Add a `?` help surface and keep footer hints context-aware.
- Support keyboard-first navigation with consistent `q`, `Esc`, `/`, `Tab`,
  `Shift+Tab`, arrows, and `hjkl` where useful.
- Define responsive behavior at 80x24 and narrow tmux splits. Fixed wide
  layouts should collapse to a single-pane or drill-down fallback rather than
  rendering broken panels.
- Keep detail panes useful for dense inspection: fields, readable prose, JSON,
  diffs, provenance, recall reasons, and dream/audit output should render
  inside the admin interface.
- Replace the lightweight memory-detail markdown renderer with a full markdown
  renderer once OpenTUI markdown output is reliable in the test renderer.
- Enable visible scrollbars for multiline memory editors if OpenTUI exposes
  textarea scrollbar controls in a newer release.
- Clean up React `act(...)` warnings and OpenTUI `TerminalConsoleCache`
  listener warnings in `bun test` without globally muting stderr.

### Install, Release, And Quality

Managed installs and releases must not imply that Hieronymus has reached a
stable 1.x product line.

Completed:

- Remap the current version line from premature `1.x` to alpha `0.x`:
  `1.0.0` becomes `0.1.0`, `1.1.0` becomes `0.2.0`, and future development
  continues in `0.x` until a major release is explicitly approved.
- Show the Greek alpha marker in human-facing version prompts and headers, such
  as `v0.2.0α`. Keep package metadata, tags, and update comparisons
  SemVer-compatible.
- Delete premature local and GitHub tags `v1.0.0` and `v1.1.0`. Recreate
  equivalent `v0.1.0` and `v0.2.0` tags only if the release history needs those
  anchors.
- Update README and install/update wording so users understand the project is
  alpha software that can be used at their own risk.

Remaining work:

- Keep managed install, update, and release builds installing frontend
  dependencies and rebuilding `frontend/dist/main.js` before packaging or
  reinstalling.
- Fix command examples that use unsupported Bun flag order. Prefer
  `bun run --cwd frontend build` and `bun run --cwd frontend typecheck` for this
  Bun version.
- Keep `.superpowers/` ignored as temporary brainstorming companion state.
  Keep `.agents/` trackable because project-local agent skills and configuration
  may live there.

## ADR Follow-Up

The following decisions are recorded in ADRs:

- local plaintext configuration files and redaction boundaries;
- alpha versioning and release authority.

Future ADRs should be added only when a decision is hard to reverse, surprising
without context, and the result of a real trade-off.
