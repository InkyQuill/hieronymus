# Rust Migration Phase 005 Service and MCP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serve the web console, REST API, WebSocket events, Streamable HTTP MCP, stdio compatibility shim, and background workers from one Axum/Tokio daemon.

**Architecture:** One `AppState` contains cloned pool/config handles, typed services, broadcast events, and cancellation. Route handlers only validate/translate transport data and call Phase 001-004 services. `rmcp` 2.2 provides stateful Streamable HTTP MCP sessions; the stdio shim forwards operations through the proposal's `/api/mcp/{operation}` compatibility endpoint.

**Tech Stack:** Rust 2024, Axum 0.8, rmcp 2.2, Tokio, tower-http, rust-embed, reqwest/rustls.

## Global Constraints

- One binary/process owns HTTP, WebSocket, MCP, and background workers.
- Bind to `127.0.0.1`, default port 9768; fail fast on collision.
- Preserve all 39 existing MCP tools and add `hieronymus_recall_feedback`.
- Preserve the current frontend API paths and response shapes exactly.
- MCP clients without Origin are accepted; foreign browser origins/hosts are rejected.
- Graceful shutdown stops accepting requests, drains in-flight work, and cancels workers.

---

## File Map

- `hiero-bin/src/daemon.rs`: listener, lifecycle, shutdown, worker JoinSet.
- `hiero-bin/src/api/`: health/status/shutdown, provider/settings/admin handlers and contracts.
- `hiero-bin/src/mcp/`: backend, 40 tools, Streamable HTTP service, stdio shim.
- `hiero-bin/src/assets.rs`: embedded/fallback frontend serving.
- `tests/test_service.rs`, `tests/test_mcp.rs`: real-router contracts.

**Focused commands:** Task 1 `cargo test -p hiero-bin --test test_service router`; Task 2 `cargo test -p hiero-bin --test test_service api`; Task 3 `cargo test -p hiero-bin --test test_service assets && cargo test -p hiero-bin --test test_service events`; Task 4 `cargo test -p hiero-bin --test test_mcp http`; Task 5 `cargo test -p hiero-bin --test test_mcp stdio && cargo test -p hiero-bin --test test_service shutdown`. RED means the named contract fails for the absent behavior; GREEN means exit 0 with all named tests passed.

### Task 1: Build App State, Router, and Security Layers

**Files:** Create `daemon.rs`, `api/{mod.rs,system.rs,error.rs}`, `tests/test_service.rs`.

**Interfaces:** Produce `AppState`, `build_router(AppState) -> Router`, `serve(config, port, shutdown)`, and stable JSON error envelopes.

- [ ] Write real-router tests for every route in proposal 005 §2, loopback binding, occupied port, host/origin allow/reject matrix, body-size limit, request IDs, health/status, shutdown authorization, and sanitized internal errors.
- [ ] Run `cargo test -p hiero-bin --test test_service router`; expect RED.
- [ ] Implement middleware before business routes: trace/request ID, body limit, host/origin validation, then routing/state. Keep `/health` cheap; status may call doctor with a timeout.
- [ ] Run focused tests; expect GREEN. Commit `feat: add unified Axum daemon`.

### Task 2: Preserve Admin and Settings Contracts

**Files:** Create `api/{providers.rs,settings.rs,admin.rs,contracts.rs}`, extend `tests/test_service.rs`.

**Interfaces:** Produce typed request/response structs for every endpoint in proposal 005 §4 and an `AdminApi` that delegates all score changes to `FeedbackStore`.

- [ ] Translate `frontend/src/web/lib/api.ts` into request/response contract tests, including provider CRUD/check/models, dream/ingest/release settings, dashboard/snapshot, confirmation-required actions, and manual dreaming.
- [ ] Run focused tests; expect RED.
- [ ] Implement thin handlers and `IntoResponse` error mapping. `AdminApi::run_action` must contain no delta table or score arithmetic; assert this behavior through fake `FeedbackStore` calls.
- [ ] Run focused tests and `cd frontend && bun run typecheck && bun run test`; expect GREEN. Commit `feat: port web console API`.

### Task 3: Add WebSocket Events and Embedded Frontend

**Files:** Create `api/events.rs`, `assets.rs`; extend service tests; modify `build.rs`.

**Interfaces:** Produces `/ws/admin`, `Asset`, `serve_assets`, and `HIERONYMUS_ASSETS_DIR` override.

- [ ] Test WebSocket subscribe/event/lag/disconnect/shutdown and asset MIME/cache/fallback/traversal behavior with both embedded and temp override assets.
- [ ] Run focused tests; expect RED.
- [ ] Use a bounded `broadcast` channel; lag returns a resync-required event. Serve known assets with correct MIME/cache headers and unknown client routes with `index.html`; canonicalize override paths and reject escape.
- [ ] Make `build.rs` rerun only on frontend inputs and warn—not fail—when `dist` is absent. Run tests; expect GREEN. Commit `feat: embed console and stream admin events`.

### Task 4: Register and Test All MCP Tools

**Files:** Create `mcp/{mod.rs,backend.rs,tools.rs,http.rs}`, `tests/test_mcp.rs`.

**Interfaces:** Produce `McpBackend`, `register_tools`, rmcp `StreamableHttpService`, and the exact 40 tool names in proposal 005 §3.

- [ ] Add an expected-name set test with 40 unique names, schema snapshots for every tool, dispatch tests per backing store, MCP initialize/session/tool-call lifecycle, unknown tool, invalid payload, backend error, and origin validation.
- [ ] Run focused test; expect RED.
- [ ] Implement small tool handlers whose input structs are the same serde/schemars types used by core services. Use rmcp's `LocalSessionManager`; do not invent a parallel HTTP MCP protocol.
- [ ] Ensure rule tools query non-empty `rule_intent`, and recall feedback records `{useful,miss}` activation outcomes. Run tests; expect GREEN. Commit `feat: expose memory services over MCP`.

### Task 5: Implement Stdio Shim and Graceful Lifecycle

**Files:** Create `mcp/stdio.rs`; modify CLI dispatch, `daemon.rs`, `main.rs`; extend service/MCP tests.

**Interfaces:** Produces `stdio::run(&HieronymusConfig)`, signal future, coordinated shutdown, and no-subcommand start-or-status behavior.

- [ ] Spawn the binary in tests and exercise stdio initialize/tool-call/shutdown against a random-port daemon; test daemon unavailable, malformed daemon response, Ctrl+C, worker cancellation, and in-flight request drain.
- [ ] Run focused tests; expect RED.
- [ ] Keep the MCP session over stdio and forward each decoded operation to `/api/mcp/{operation}`, preserving schemas/results. Manage server and workers in `JoinSet`; cancellation propagates via token/broadcast, with bounded graceful timeout and surfaced task failures.
- [ ] Run all Phase 005 and workspace verification; expect GREEN. Commit `feat: complete single-process daemon and MCP shim`.

## Phase Acceptance

- [ ] Router contract tests cover every frontend fetch path and all 40 MCP tools.
- [ ] Foreign origins fail at MCP/admin routes while origin-less MCP clients work.
- [ ] Ctrl+C/SIGTERM leaves no listener, dream lock, or indexing task running.
- [ ] `rg -n 'ServiceManager|Popen|server.json|PID file|port allocation' crates` finds no legacy lifecycle design.
