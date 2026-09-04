# Hieronymus Roadmap

Hieronymus is an alpha local-first translation memory system. The Python alpha
baseline below is closed and frozen as the behavioral reference for the Rust
rewrite; new feature work happens in Rust after the cutover.

## Rust Rewrite (active program)

Normative sources: the 2026-08-31 ADRs (0008–0015) and
`docs/superpowers/specs/2026-08-31-rust-*.md` (certification trimmed
2026-09-03). Process per owner direction: port tests from the current Python
behavior, adapt them as Rust tests against the frozen fixtures, implement until
green; commit per slice, plain diff review — no evidence records, gates, or
recorded attestations. Qualification stage (ADR 0013 spike, four measured
records, gate `qualified` / `semantic-enabled`) is complete and merged.

Planned slices, in order:

1. Workspace skeleton: virtual Cargo workspace, domain library + `hiero`
   binary, config files (`provider.conf`/`dream.conf`/`ingest.conf`/
   `release.conf`) with typed round-trip and `Secret<T>`, data-root handling,
   SQLite open/classify boundary.
2. Series/memory storage: schema creation at the current Rust schema,
   concepts/facets/crystals reads, short-term ingestion thresholds.
3. Terminology: `term_rules`/`term_rule_forms`, deterministic validation,
   recall contract section.
4. Memory/recall: ranked recall lanes, RRF fusion, feedback, working copies,
   reconsolidation.
5. RAG/semantic: FTS5 lane, import pipeline, LanceDB/ort semantic lane behind
   the qualified pins (LanceDB 0.37.1, ort 2.0.0-rc.13), generation lifecycle.
6. Dreaming: phase pipeline, bounded mutation, audit, provider workflows.
7. Daemon/transports: daemon lifecycle + discovery, MCP 2026-07-28 stdio +
   Streamable HTTP, REST/WebSocket routes, light local auth (ADR 0012
   amendment), embedded Svelte console.
8. Upgrade tooling before any destructive cutover: `hiero migrate` preflight,
   dry-run, typed conversion, backup/journal/promotion protocol.
9. Distribution: one binary + command links, installer, update flow, manual
   release rehearsal checklist, managed cutover.

Done so far: slices 1–2 (skeleton, config, series, sessions, short-term
memories, concepts/facets, crystals), slice 3 core (term_rules authority,
contract, context disambiguation), slice 4 core (FTS recall lane, RAG store,
rag recall lane with active-rule-protected merge), `hiero` version/classify
skeleton, CI workflows.

Deferred capability gaps to close before cutover:

- strict_terms → term_rules migration belongs to `hiero migrate` (slice 8).
- concept recall boosts (`recall_boosts_for_crystals`) port with the
  dreaming slice.

## Python Alpha Baseline (closed, behavioral reference)

Hieronymus is still an alpha local-first translation memory system. The current
codebase already contains the core memory graph, primitive MCP tools, local
service, React/OpenTUI management app, install/update flow, and dreaming
pipeline. The alpha baseline roadmap is closed; future work should start from a
new plan or ADR-backed decision when scope is approved.

> **Note (2026-09-03):** the OpenTUI management app described below is
> retired — [ADR 0014](adr/0014-web-console-replaces-terminal-ui.md) replaced
> the terminal UI with the Svelte web console. The section is kept as the
> behavioral history of the Python alpha.

## Product Surfaces

### Memory And Dreaming

Dreaming is the path from short-term observations to durable memory. It remains
bounded, auditable, and local-first.

Current baseline:

- `provider.conf` is the canonical configuration file for provider endpoint
  profiles and plaintext local API keys.
- `dream.conf` is the canonical configuration file for dreaming workflow
  assignments, prompts, thresholds, and caps.
- The old provider settings file has been removed without migration because the
  project is pre-release.
- `ingest.conf` is the global data-root configuration file for ingestion policy,
  including direct short-term memory warning/rejection thresholds and
  Learn-style block splitting limits.
- Short-term memory ingestion supports separate sentence and symbol warning and
  rejection thresholds, with symbol thresholds disabled by default.
- Invalid configured provider workflows are rejected instead of silently falling
  back to deterministic dreaming.
- Provider-backed audit coverage verifies crystallization request and response
  summaries, parse warnings, selected memory IDs, affected memory set payloads,
  multi-batch phase runs, and maintenance phase decisions in the same dream run.

Status: complete for the alpha baseline.

### MCP And Agent Integrations

Status: complete for the alpha integration baseline.

Completed:

- Claude Code, Codex, OpenCode, OpenClaw, and Gemini CLI have installable host configuration paths.
- Xiaomi MiMo, Pi, and Hermes remain explicit reserved targets because no safe noninteractive host configuration protocol is implemented.
- Install and reinstall coverage exists for every integration that writes host configuration.
- Read, Learn, and Remember remain agent skill workflows rather than MCP judgment wrappers.

### CLI And Service

The CLI is both a human tool and an automation surface. Human output should be
clear, while JSON output remains stable for scripts.

Completed baseline:

- Commands that intentionally access SQLite through Python domain stores are
  documented with command-level reasons.
- Agent-facing hook and MCP adapter status paths report local service discovery
  from configured runtime files instead of parsing human CLI output.
- Local-first operation and existing `--data-root` / `HIERONYMUS_DATA_ROOT`
  behavior remain the data-root selection contract.
- CLI help has clearer command grouping, examples, readable line lengths, and
  alpha status language.
- Machine-readable legacy automation command output is behind `--json`.

Status: complete for the alpha baseline.

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
- `hiero admin` receives Python-owned command metadata and renders a real
  keyboard command palette with availability state and bridge-backed execution.
- `hiero admin` has a `?` help surface and context-aware footer hints for
  normal, command-palette, and help modes.
- Frontend OpenTUI tests use a shared renderer harness that flushes React
  updates through `act(...)`, destroys renderers after each test, and keeps
  `bun test` free of React lifecycle and `TerminalConsoleCache` listener
  warnings.
- `hiero admin` and `hiero config` define responsive behavior for 80x24
  terminals and narrow tmux splits, using compact single-pane fallbacks instead
  of fixed-width broken panel layouts.
- `hiero config` exposes Python-owned section metadata for current and future
  configuration files while keeping the frontend schema-driven.
- `hiero admin` and `hiero config` support keyboard-first navigation with
  `hjkl`, `/` search, `Esc` cancellation, `Tab`/`Shift+Tab` focus movement, and
  footer hints aligned with the active mode.
- Admin detail panes render dense inspection bodies with structured fields,
  JSON, diffs, markdown blocks, provenance, recall reasons, dream output, and
  audit output through the admin interface.
- Multiline memory editor dialogs stay bounded in compact admin layouts. Visible
  textarea scrollbars remain dependent on a future OpenTUI textarea scrollbar
  API and are tracked in `docs/opentui-conventions.md`.

Status: complete for the alpha baseline.

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
- Managed install, managed update, PR verification, and release verification
  install frontend dependencies and rebuild `frontend/dist/main.js` before
  package validation, publishing, or reinstalling.
- Release build commands and active user-facing frontend build examples use
  the supported Bun command order for this Bun version.

Status: complete for the alpha baseline.

## ADR Follow-Up

The following decisions are recorded in ADRs:

- local plaintext configuration files and redaction boundaries;
- alpha versioning and release authority.

Future ADRs should be added only when a decision is hard to reverse, surprising
without context, and the result of a real trade-off.
