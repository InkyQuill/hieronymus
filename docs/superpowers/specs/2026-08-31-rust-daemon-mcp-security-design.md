# Rust Daemon, MCP, And Security Design

**Status:** Accepted on 2026-08-31.

## Goal

Provide a discoverable local daemon whose CLI, MCP, WebSocket, and browser
surfaces share authentication, domain behavior, and graceful lifecycle.

## Lifecycle And Discovery

`hiero daemon` runs in the foreground. `hiero start`, `stop`, `restart`, and
`status` operate through the platform service integration and versioned
discovery record. Startup acquires data-root ownership, opens and verifies the
database, binds loopback, writes discovery atomically, then reports ready.

Startup fails without changing discovery if schema verification, authentication
material, or binding fails. An occupied configured port is an error; no silent
port scan occurs. A chosen override is persisted/discovered so plugins never
hard-code the default port.

Before binding or publishing discovery, startup runs the shared bounded
`StateClassifier`, which reads only version markers, required-file presence,
and cutover-journal state. It does not execute typed conversion or full
integrity/index verification. It starts only for the current supported
Rust schema, current config versions, and complete/absent journal. A legacy
Python schema exits with `migration_required`; legacy config exits with
`config_migration_required`; a committed database awaiting config promotion
exits with `config_promotion_required`. Stable diagnostics include the exact
migration/resume command. Newer, unknown, corrupt, and other partial states fail
closed. Daemon startup never performs schema or config migration.

Graceful shutdown stops admission, closes MCP/WebSocket sessions, signals
workers, waits for bounded work, rolls back unfinished transactions, closes the
semantic index, removes matching discovery state, and releases ownership.

## Authentication

All service endpoints except minimal `/health` require authentication. Native
clients use `Authorization: Bearer`. Browser entry uses a single-use launch
grant exchanged for an HttpOnly, SameSite=Strict session plus CSRF token.
Mutations require the session and CSRF header. Host and Origin validation are
additional browser defenses, not replacements for credentials.

WebSocket authentication occurs during upgrade and inherits token rotation and
session expiry. `/shutdown`, status details, admin, config, MCP, and stdio proxy
operations use the same policy. Logs redact authorization, cookies, API keys,
launch grants, and query strings.

Credentials and provider keys use the shared `Secret<T>` type. Its `Debug`,
`Display`, tracing, and serialization forms are redacted; public DTO types cannot
contain it. Only credential loaders and outbound header builders can call
`expose_secret()`. Tests send sentinel secrets through every error/log/audit/DTO
path and fail if the literal sentinel appears.

Token rotation emits `credentials_rotated`, hard-closes MCP and WebSocket
sessions, and requires rediscovery plus reauthentication. Clients never silently
replay mutations. Automatic retry is limited to declared idempotent reads or
requests carrying an accepted idempotency key.

## MCP

One tool registry owns tool names, descriptions, JSON schemas, and result/error
mapping. Per ADR 0015, the exact MCP revision is `2026-07-28`. Streamable HTTP
uses JSON-RPC HTTP POST at `/mcp` with JSON or request-scoped SSE responses;
stdio uses newline-delimited JSON-RPC. Both expose this same registry. The stdio
adapter does not duplicate domain schemas and does not access SQLite. It
discovers, starts if allowed, authenticates, negotiates the pinned protocol,
and proxies `/mcp` with bounded reconnect behavior.

The compatibility manifest owns the current registry snapshot and derives its
tool count. New recall-feedback behavior is added as a versioned contract using
`recall_id`, activation ids, and idempotency key. The Python private operation
bridge is not MCP and is intentionally removed by ADR 0015. Removal of stdio
requires a later ADR and host-support evidence.

## HTTP And Frontend Contracts

Routes are generated or tested against the actual TypeScript client types. The
contract inventory fixes method, path, auth, request envelope, response
envelope, status codes, and error body. In particular, provider save/check/model
and manual dreaming must preserve the shipping frontend shapes unless an
accepted contract change updates both sides atomically.

The manifest contains a concrete entry for every current route in the route
families listed by the compatibility spec, including health/status/shutdown,
providers, settings, admin actions/snapshots, admin WebSocket, static SPA
routes/assets, the removed Python operation bridge, and the new `/mcp` endpoint.

Static assets are served from an embedded asset abstraction using `rust-embed`
lookup/iteration, not a filesystem `ServeDir`. Development may use an explicit
filesystem override. Unknown client-side routes fall back to embedded
`index.html`; missing actual assets return 404.

WebSocket messages have a version, event id, event type, and payload. Clients
resume from the last event id when retained; otherwise the server instructs a
snapshot refresh. Lag does not silently produce a partial admin view.

## Service Installation

Service definitions use the absolute installed binary and explicit data root.
They set restart/backoff and log policies appropriate to the platform. Install,
upgrade, and uninstall are idempotent and do not remove user data. `doctor`
reports service definition, discovery, auth permissions, protocol compatibility,
and database/index health without triggering downloads.

## Acceptance Criteria

- Every current frontend request matches a tested Rust route contract.
- Unauthenticated and cross-origin mutation attempts fail.
- Tokens/grants do not appear in URLs or logs.
- Sentinel secret tests cover logs, errors, audit, diagnostics, MCP/HTTP JSON,
  WebSocket events, and frontend payloads through one `Secret<T>` mechanism.
- Stdio and HTTP MCP expose identical registered schemas and results.
- Non-default ports and token rotation propagate through discovery/plugins.
- Kill/restart/stale-discovery tests recover without concurrent daemon writers.
