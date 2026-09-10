# Remediation, Stable Web MCP, And Semantic RAG Design

**Status:** Approved for implementation planning on 2026-07-18. Its local-auth
non-goal is superseded by ADR 0012; its “MCP v1/v2” package-generation language
and transport ambiguity are superseded by ADR 0015.

## Context

`report.md` and `improvements.md` describe a mixture of current defects, design debt,
already-completed Tailwind/build work, and longer-term retrieval improvements. The current
daemon also chooses a new loopback port on every start, while installed agent integrations use
a stdio MCP adapter over a private HTTP operation bridge.

This program addresses every finding against current `main`, consolidates the service around a
stable standard web MCP endpoint, adds local semantic RAG, and converts completed Superpowers
artifacts into durable project documentation. It is split into independently verifiable plans so
that correctness and migration work lands before broader architectural changes.

## Goals

- Fix the still-valid correctness, data-integrity, reliability, security-cleanup, configuration,
  frontend, and maintainability findings in the two reports.
- Run the daemon on one predictable port instead of allocating an ephemeral port on every start.
- Expose a standard MCP Streamable HTTP endpoint from the same process as the web console.
- Retain the current stdio MCP command for one compatibility release.
- Remove the legacy strict-term storage model after a lossless migration to rule crystals.
- Add local-by-default multilingual semantic retrieval with an authoritative SQLite corpus and a
  rebuildable LanceDB index.
- Distill fulfilled specs and plans into current documentation, then remove them from
  `docs/superpowers`.

## Non-Goals

- No authentication, authorization, TLS, or remote-deployment security layer in this program.
- No automatic fallback to another port when the configured port is occupied.
- No replacement of SQLite as the authoritative application and RAG corpus database.
- No Qdrant implementation; only a backend boundary that permits one later.
- No independent SDK-generation decision; ADR 0015 owns the exact MCP protocol
  revision and transports.
- No unrelated web-console redesign.

## Program Sequence

The implementation is divided into five plans:

1. Correctness and data integrity.
2. Stable daemon and standard web MCP.
3. Memory schema and internal boundary cleanup.
4. Local semantic RAG.
5. Documentation consolidation.

Plans 1 and 2 establish reliable storage and service lifecycle behavior. Plan 3 removes legacy
schema and naming debt. Plan 4 builds semantic retrieval on those stable boundaries. Plan 5
consolidates the final implemented architecture and removes fulfilled planning artifacts.

## Plan 1: Correctness And Data Integrity

### Canonical Feedback Scoring

`FeedbackStore` owns immediate feedback deltas, score clamping, confidence-based archival, rule
immunity, and delete-threshold archival. Admin feedback must call the same transaction-compatible
scoring operation instead of reproducing this logic in `AdminStore._record_immediate_feedback`.
CLI, MCP, and web-admin feedback must produce identical memory-event and crystal state changes.

### Atomic Proposal Approval

Proposal approval must parse and validate all JSON fields before mutation. It then starts a
`BEGIN IMMEDIATE` transaction, rereads the pending proposal, resolves or creates its concept,
writes facets/tags, updates proposal status, and writes audit data in one unit.

The transaction must prevent two approvers from independently observing the same pre-write state.
The second approver either observes the completed approval or receives a clear non-pending error.
Malformed proposal payloads and failed facet writes leave all related tables unchanged.

### Trigger-Owned FTS Synchronization

Fresh schema and upgrade migrations add insert, update, and delete triggers for
`short_term_memories_fts` and `crystals_fts`. Existing indexes are rebuilt once before manual FTS
writes are removed from Python services. Tests must cover raw SQL changes, application writes,
cascade deletion, and search results.

No new strict-term FTS triggers are added because Plan 3 removes the legacy strict-term tables.

### Shared Value Utilities

A small shared module owns:

- canonical UTC timestamps rendered with a `Z` suffix;
- score clamping to `[0.0, 1.0]`;
- normalized, deduplicated string tuples; and
- checked JSON-object decoding.

Call sites migrate without changing their public contracts. The unnecessary catch-all following
the `JSONDecodeError` handler in the admin detail path is removed. Broad exception handlers that
form deliberate daemon, RPC, or provider isolation boundaries remain, but must log or return
appropriately redacted errors.

### Focused Interface Fixes

- `install.sh:has_tty()` requires terminal stdin and stdout as well as an accessible `/dev/tty`.
- The Short-Term Memory view exposes the existing `remove_short_term_memory` action with the
  existing destructive confirmation flow.
- The frontend API helper applies one bounded timeout, composes it with caller cancellation, and
  reports timeout separately from HTTP/API errors.

### Acceptance

- Bad proposal JSON and failed approval writes roll back completely.
- Concurrent proposal approval cannot create duplicate concepts or facets.
- All supported mutations keep FTS content synchronized.
- Admin and non-admin feedback have scoring/status parity.
- New timestamps have one canonical representation.
- Non-interactive installer tests never attempt to read `/dev/tty`.
- Short-term-memory removal is reachable in the web console.
- A stalled browser API request terminates predictably.

## Plan 2: Stable Daemon And Standard Web MCP

### Address And Exposure Model

The daemon defaults to `127.0.0.1:9768`. A service configuration value, environment override, or
explicit daemon argument may replace the port or bind address. The daemon never chooses another
port automatically. An occupied address is a startup error that reports the address and daemon
log location.

The service has no bearer token, credential file, query token, cookie token, or token field in
`server.json`. Loopback binding is the default protection. Choosing another interface, including
`0.0.0.0`, explicitly exposes the console, APIs, lifecycle controls, and MCP without
authentication; the CLI and documentation must warn about this.

Browser-originated requests retain basic Host and Origin validation. MCP requests without an
`Origin` header remain valid for native clients. Supplied MCP Origin headers are validated as
required by the Streamable HTTP transport specification.

### One ASGI Application

One Starlette/ASGI application owns:

```text
/admin, /config, /assets/*    browser console
/api/*                       browser admin/config API
/ws/admin                    browser event stream
/health, /status, /shutdown  lifecycle API
/mcp                         standard Streamable HTTP MCP
```

The official FastMCP Streamable HTTP application is configured with
`streamable_http_path="/"` and mounted at `/mcp`, so the public endpoint is exactly `/mcp` rather
than `/mcp/mcp`. Starlette routes replace `ThreadingHTTPServer`, and an ASGI WebSocket route
replaces manual handshake and frame handling. Static assets come from one packaged root with one
explicit development override.

MCP tool definitions are separated from transport registration. The daemon registers direct
domain handlers. The compatibility stdio command registers proxy handlers, avoiding recursive
daemon calls while preserving tool names, signatures, and results.

The private `/api/mcp/*` bridge remains only for the stdio compatibility release and is deleted
with that shim later. Agent installers use HTTP URL configuration where the client's documented
format supports it and retain `hieronymus-mcp` where it does not.

The dependency is bounded to `mcp>=1.27,<2`; MCP v2 requires a separate compatibility design.

### Lifecycle And Diagnostics

- `ServiceManager.start()` retains the child handle and directs output to a bounded daemon log.
- Early child exit is surfaced directly.
- Startup timeout terminates the process group, waits, escalates when required, and removes only
  matching discovery state.
- SIGINT, SIGTERM, HTTP shutdown, and normal teardown use one graceful-shutdown path.
- Scheduler and worker joins have deadlines.
- `stop()` waits for confirmed process exit and escalates within fixed bounds.
- `restart()` starts only after confirmed shutdown.
- Manual dreaming publishes progress events directly instead of polling SQLite every 200 ms.

### Acceptance

- Every ordinary restart returns to the configured port.
- Port conflict never causes silent fallback.
- A standard MCP SDK client can initialize, list tools, and call tools over `/mcp`.
- Supplied invalid Origins are rejected; native no-Origin clients work.
- The stdio command remains contract-compatible for one release.
- Startup failures leave no orphan daemon and preserve useful logs.
- SIGTERM and HTTP shutdown remove matching state and stop workers within deadlines.
- Stop/restart report their verified outcome.
- Real WebSocket clients receive events and disconnect cleanly.
- Installed wheels serve assets only from the packaged distribution.

## Plan 3: Memory Schema And Internal Boundary Cleanup

### Versioned Schema Evolution

Introduce a `schema_migrations` ledger and ordered upgrade migrations. `global.sql` describes a
fresh database; upgrades no longer depend on an expanding set of startup compatibility rewrites.

### Strict-Term Retirement

Fresh databases omit `strict_terms`, `strict_term_tags`, `strict_term_aliases`, and
`strict_terms_fts`. Existing databases are upgraded as follows:

1. Write a timestamped JSON backup of every legacy term and associated row.
2. Convert approved/active terms into rule crystals, concepts, facets, variants, semantic tags,
   and provenance records.
3. Verify ledger coverage and semantic parity for terminology validation and recall.
4. Stop with the exact blocking term if an approved rule cannot be represented safely.
5. Preserve rejected/inactive history in the backup and audit trail.
6. Drop legacy tables only after all required checks pass.

Termbase imports and admin rendering views then read and write rule crystals directly. Removing
the legacy rendering query also removes its current N+1 tag lookup.

### Naming And Configuration

- Rename `tui_bridge` to a package describing its admin/config console API role.
- Delete its unused custom stdio loop.
- Use `data_root` as the sole internal root name and remove the aliasing `config_root` property.
- Resolve the default root through `XDG_CONFIG_HOME`; retain precedence for explicit CLI paths and
  `HIERONYMUS_DATA_ROOT`.
- Rename `llmcache.tmp` to `llm-cache.json`, atomically adopting the old file only when the new
  path is absent.

Frozen dataclass normalization remains unchanged: `object.__setattr__` in `__post_init__` is the
intended way to establish normalized values in a frozen instance.

### Dreaming Boundaries And Decay

Keep `DreamService` as the public facade while extracting phase orchestration,
normalization/validation, persistence, and maintenance collaborators. Outputs, provider prompts,
transactions, and scoring semantics remain stable.

The existing changed-crystal cap is retained. Replace the full candidate count plus offset query
with indexed, bounded selection and `limit + 1` detection. Composite indexes cover status,
crystal type, reinforcement/activation recency, and deterministic selection. Active rule crystals
remain immune.

### Acceptance

- Representative legacy databases upgrade without terminology loss.
- Upgrade interruption rolls back database mutation and leaves the backup usable.
- Blocking approved rules prevent table removal.
- Fresh databases contain no strict-term tables.
- Rule-crystal validation and recall match legacy behavior.
- Renamed internal packages leave no stale public command or import contract.
- Existing cache files are adopted once without overwrite.
- Query plans demonstrate bounded indexed decay selection.
- Dream contract tests remain unchanged.

## Plan 4: Local Semantic RAG

### Storage Model

SQLite remains authoritative for RAG sources, chunks, metadata, and FTS. LanceDB is a derived
index under the data root containing chunk IDs, vectors, source checksums, model identity, index
generation, and filterable scope metadata. Deleting the derived directory must not lose source
data; `hiero rag reindex` reconstructs it from SQLite.

A narrow `SemanticIndex` interface covers rebuild, bounded upsert/delete, search, health, and
generation metadata. LanceDB is the only implementation in this program. The interface may gain a
Qdrant implementation later without changing `RagStore` or recall contracts.

### Embeddings

The default provider is local FastEmbed using
`sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2`, a 384-dimensional multilingual
model. It downloads lazily with visible progress and is never fetched during installation or
ordinary startup. Offline or unavailable models leave FTS operational.

A separate embedding-provider protocol permits optional configured remote implementations without
coupling embeddings to chat-provider behavior. Provider, model, revision, dimensions, and chunk
checksum form the vector identity; changing any of them marks affected vectors stale.

### Index Lifecycle

- RAG import commits SQLite content before scheduling index work.
- The daemon processes indexing in bounded batches.
- Search admits only vectors matching current chunk checksums and the active index generation.
- Missing, corrupt, or stale indexes degrade to FTS and report their state through status and
  doctor output.
- CLI and web controls expose status, pending count, rebuild, and cancellation.
- Atomic generation switching prevents partially rebuilt indexes from becoming active.

### Hybrid Retrieval

1. Query SQLite FTS5 and LanceDB vector search independently with expanded candidate budgets.
2. Apply series, language, story-scope, and semantic-tag filters to both lanes.
3. Merge ranks with reciprocal-rank fusion rather than comparing BM25 and vector distances.
4. Apply existing metadata and glossary boosts after fusion.
5. Return lexical rank, vector rank, fusion contribution, model, and fallback state in rank
   explanations.
6. Keep active rule crystals in their mandatory lane. Semantic evidence remains advisory and
   cannot override deterministic terminology constraints.

### Acceptance

- Deterministic fake-embedding tests require no network or model download.
- Multilingual paraphrase and cross-language fixtures improve over lexical-only retrieval.
- Stale generations and checksums are excluded.
- Rebuild is crash-safe and atomically activated.
- Source deletion synchronizes derived vectors.
- Missing models and corrupt indexes fall back to FTS.
- Rank explanations expose both retrieval lanes.
- Conflicting semantic evidence cannot outrank an active rule crystal.

## Plan 5: Documentation Consolidation

Completed specs and plans are not archived verbatim. Their durable decisions are rewritten into
current documentation, while obsolete steps and superseded filenames are discarded.

The target documentation set is:

- `docs/architecture.md`
- `docs/service-and-mcp.md`
- `docs/web-console.md`
- `docs/memory-model.md`
- `docs/rag.md`
- `docs/providers.md`
- `docs/installation-and-updates.md`
- `docs/agent-integration.md`
- `docs/release-process.md`
- `docs/review-disposition-2026-07-18.md`

Existing documents are merged or renamed rather than duplicated. A temporary traceability table
maps every fulfilled Superpowers artifact to durable sections and implementation evidence. A
fulfilled artifact is deleted only after that mapping is complete.

`report.md` and `improvements.md` remain untouched unless the user separately requests their
removal. The review-disposition document records every finding as planned, resolved, partially
stale, or intentionally unchanged.

After the consolidation plan runs, `docs/superpowers` contains only this approved design and the
unimplemented plans derived from it. After each remaining plan is implemented and its durable docs
are updated, its completed planning artifacts are removed. At program completion the directory is
empty or contains only genuinely pending work.

Documentation validation checks internal links, commands, paths, port examples, MCP connection
examples, schema terminology, package version references, and contradictions against current code
and tests.

## Finding Disposition

### `report.md`

| Finding | Current disposition |
| --- | --- |
| 1.1 duplicated crystal archival/scoring | Valid; Plan 1 canonicalizes feedback scoring. |
| 2.1 token in browser URL | Valid; Plan 2 removes all service tokens and opens clean URLs. |
| 2.2 dead cookie token | Valid; removed with token machinery in Plan 2. |
| 2.3 permissive token-state file | Superseded; Plan 2 removes the secret entirely. |
| 2.4 hand-written WebSocket | Valid; replaced by ASGI WebSocket in Plan 2. |
| 3 duplicated frontend build recipes | Resolved by the merged Tailwind remediation; no work. |
| 3.1 stale OpenTUI installer text | Resolved on current `main`; no work. |
| 1.2 orphan daemon after startup timeout | Valid; Plan 2 owns and terminates failed children. |
| 1.3 partial proposal approval | Valid; Plan 1 validates first and uses one transaction. |
| 1.4 proposal concept TOCTOU | Valid; Plan 1 starts a write transaction before resolution. |
| 1.5 missing FTS delete/update triggers | Valid; Plan 1 adds short-term/crystal triggers; Plan 3 removes strict terms. |
| 1.6 duplicated `_now()` | Valid; Plan 1 centralizes canonical timestamps. |
| 1.7 duplicated `_clamp_score` | Valid; Plan 1 centralizes score clamping. |
| 1.8 admin N+1 term tags | Valid but removed with the legacy term view in Plan 3. |
| 1.9 broad admin exception | Valid at the cited detail path; narrowed in Plan 1. Boundary catches remain. |
| 4 frontend tests do not test behavior | Resolved by current Vitest rendered-component suites; static contract tests remain intentionally. |
| 5.1 daemon logs discarded | Valid; Plan 2 writes bounded diagnostic logs. |
| 5.2 restart ignores failed stop | Valid; Plan 2 verifies shutdown before start. |
| 5.3 no SIGTERM handling | Valid; Plan 2 unifies graceful signal shutdown. |
| 5.4 unbounded scheduler join | Valid; Plan 2 adds deadlines. |
| 5.5 200 ms progress polling | Valid; Plan 2 uses direct events. |
| 6.1 `config_root` aliases `data_root` | Valid naming debt; Plan 3 keeps only `data_root`. |
| 6.2 ignores `XDG_CONFIG_HOME` | Valid; Plan 3 honors it. |
| 6.3 misleading `llmcache.tmp` | Valid; Plan 3 adopts `llm-cache.json`. |
| 6.4 misleading `tui_bridge` package | Valid; Plan 3 renames it for console APIs. |
| 6.5 dead `run_stdio` | Valid; Plan 3 deletes it. |
| 7.1 timestamp inconsistency | Valid; Plan 1 standardizes UTC `Z`. |
| 7.2 frozen dataclasses use `object.__setattr__` | Intentionally unchanged; this is idiomatic `__post_init__` normalization. |
| 7.3 duplicated JSON-object helpers | Valid; Plan 1 shares checked decoding. |
| 7.4 duplicated tuple normalization | Valid; Plan 1 shares normalization. |
| 8.1 browser project includes `bun-types` | Resolved on current `main`; no work. |
| 8.2 unreachable short-term removal | Valid; Plan 1 exposes it in the correct view. |
| 8.3 dead Tailwind CSS classes | Resolved on current `main`; no work. |
| 8.4 no frontend fetch timeout | Valid; Plan 1 adds bounded cancellation. |
| 8.5 inconsistent error styling | Resolved on current `main`; no work. |
| 9.1 oversized `DreamService` | Valid maintainability debt; Plan 3 extracts collaborators. |
| 9.2 ambiguous asset roots | Valid; Plan 2 uses one packaged root and one explicit dev override. |

### `improvements.md`

| Finding | Current disposition |
| --- | --- |
| 2.1 missing FTS triggers/manual writes | Valid; Plan 1 makes triggers authoritative. |
| 2.2 installer TTY detection | Valid; Plan 1 requires terminal stdin/stdout. |
| 2.3 `strict_terms` overlaps rule crystals | Valid; Plan 3 migrates and removes legacy storage. |
| 3.1 semantic vector RAG | Planned in Plan 4 with local FastEmbed and LanceDB. |
| 3.2 unbounded dreaming decay | Partially stale because a changed-crystal cap exists; Plan 3 removes remaining full scans and adds indexes. |

## Cross-Plan Verification

Each implementation plan uses focused red-green tests and finishes with the project-required
checks:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Frontend-changing plans also run formatting, type checking, Vitest, and a production build from
`frontend/`. Packaging or service-changing plans additionally build a wheel, test-install it in an
isolated environment, and probe the installed daemon rather than the source checkout.
