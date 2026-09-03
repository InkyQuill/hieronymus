# Documentation Index

Status classification after the start of the Rust rewrite (2026-09-03).
The Rust migration is normative; Python-era documents are kept as the
behavioral reference being ported, then retired at cutover.

## Normative — drives the Rust rewrite

- `adr/` — decisions 0005–0015 are current (0002/0004 are retained history;
  0003 is a historical data-model reference; 0008 governs the rewrite;
  0012 as amended 2026-09-03 defines light local authentication).
- `superpowers/specs/2026-08-31-rust-*.md` — the nine normative migration
  specifications (certification trimmed 2026-09-03; see amendment notes).
- `roadmap.md` — active program: Rust rewrite slices.
- `adr/0008`, `adr/0013`, `qualification/records/` — cutover authority,
  semantic decision, accepted qualification evidence (archived, nothing
  further builds on the records).
- `../AGENTS.md` — working rules.

## Behavioral reference — describes the Python behavior being ported

Retire (archive or delete) at cutover, after the corresponding Rust slice
passes its ported tests:

- `current-baseline.md`, `usage.md`, `agent-workflows.md`,
  `service-toolkit.md`, `translation-workspace-integration.md`,
  `memory-dreaming.md`, `project-conventions.md`
- `superpowers/specs/2026-07-*.md` — Python implementation design history
- `tui-design-improvements.md` — untracked owner draft

## Historical — kept for context, not needed for work

- `adr/0002`, `adr/0004` — retired terminal-UI migrations
- `archive/` — retired documents (OpenTUI conventions)
- `rust-migration-proposal/` — superseded design input; see
  `rust-migration-proposal/README.md`
