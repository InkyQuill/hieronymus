# Authenticate All Local Service Transports And Publish Discovery State

## Status

Accepted on 2026-08-31. This ADR supersedes the “no authentication,
authorization, TLS, or remote-deployment security layer” non-goal in
`docs/superpowers/specs/2026-07-18-remediation-and-semantic-rag-design.md` for
the local daemon. Remote deployment and TLS remain non-goals.

> **Amendment (2026-09-03, owner):** local-first light authentication. The
> threat model for a local single-user tool is drive-by browser traffic against
> loopback (DNS rebinding) and accidental cross-process access — not
> authenticated adversaries. Kept: loopback-only bind; one static
> per-installation bearer token (CSPRNG, 0600) required by every non-static
> endpoint (`/mcp`, REST, WebSocket upgrade, shutdown) except a minimal
> unauthenticated `/health`; `Host`/`Origin` validation; `Secret<T>` redaction;
> the atomic non-secret discovery record. Simplified: token rotation rewrites
> the token and returns 401 to existing clients (reconnect and reread); no
> `credentials_rotated` ceremony and no idempotency-key retry policy attached
> to authentication. Browser console: one-time launch grant exchanged for a
> SameSite=Strict HttpOnly session cookie; mutating browser requests require a
> valid `Origin`/`Host` — the separate CSRF token layer is waived. WebSocket
> authentication is the session cookie checked at upgrade. `hiero mcp` reads
> the local token file directly; no credential-negotiation protocol.

> **Narrow authority amendment (2026-09-07, ADR0016):** ordinary MCP credentials
> establish agent authority only. Console launch grants now require the separate
> local `console.token`; `/authority/host-event` requires `host-event.token`.
> Both credentials reuse CSPRNG, atomic storage, 0600 permissions and redaction;
> neither is returned by MCP/status or embedded in generated bundles. Console
> fragment bootstrap, single-use grants, process-lifetime cookies and exact
> mutation Host/Origin checks remain. Same-account local shell access is trusted;
> this separation protects MCP authorship and context binding without a broker
> or claims against direct local credential access/database edits.

## Context

Loopback binding reduces exposure but does not authenticate local processes or
protect mutating browser endpoints by itself. `Host` and `Origin` validation
mitigate browser attacks but are not credentials. The Rust proposal mentions a
token file without defining how browser, MCP, WebSocket, shutdown, and stdio
clients use it.

## Decision

The daemon binds to loopback only by default and authenticates every non-static
endpoint with a per-installation bearer token. This includes REST, MCP,
WebSocket upgrade, status details, and shutdown. `/health` may return only a
minimal unauthenticated liveness response with no paths, versions, or user data.

The token is generated with a cryptographically secure RNG, stored separately
from discovery metadata, and written atomically with user-only permissions.
All secret-bearing domain values use a single `Secret<T>` newtype whose
`Debug`, `Display`, tracing-value, and serialization behavior is redacted by
default. Only the credential loader and outbound provider/auth header builders
may call an explicit `expose_secret()` method. Public API DTOs cannot contain
`Secret<T>` and must be constructed through redacting projection functions.

Token rotation sends an explicit `credentials_rotated` close/error to current
MCP and WebSocket sessions, then terminates them. Clients do not silently retry
mutations. They rediscover credentials, reauthenticate, and may retry only
operations declared idempotent or carrying an idempotency key.

Browser bootstrapping uses a short-lived, single-use launch grant created by
`hiero config` or `hiero admin`. The grant is exchanged over loopback for a
SameSite=Strict, HttpOnly session cookie. Tokens and grants never appear in URL
query strings. State-changing browser requests also require validated
`Host`/`Origin` and a CSRF token.

Native HTTP MCP clients read the endpoint and credential location from generated
host configuration. Where a host cannot supply authorization headers safely,
its plugin uses `hiero mcp`; the stdio adapter reads the local token and proxies
the authenticated MCP session. It starts the per-user daemon when absent and
returns a bounded diagnostic if startup or protocol negotiation fails.

The discovery record contains the application discovery version and ADR 0015's
exact MCP revision. Clients reject unsupported versions and report the
remediation command instead of guessing routes or treating the private Python
operation bridge as MCP.
Port overrides are written into discovery/configuration; generated plugins must
not hard-code `9768`.

## Consequences

Authentication and discovery become cross-surface contracts rather than route
implementation details. Frontend integration tests must exercise the launch
grant, cookie, CSRF, WebSocket, and token-rotation flows.
