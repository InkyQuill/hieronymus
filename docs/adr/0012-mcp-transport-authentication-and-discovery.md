# Authenticate All Local Service Transports And Publish Discovery State

## Status

Proposed.

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
from discovery metadata, written atomically with user-only permissions, and
redacted from logs and diagnostics. Token rotation invalidates existing MCP and
WebSocket sessions.

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

The discovery record contains a protocol version. Clients reject incompatible
major versions and report the remediation command instead of guessing routes.
Port overrides are written into discovery/configuration; generated plugins must
not hard-code `9768`.

## Consequences

Authentication and discovery become cross-surface contracts rather than route
implementation details. Frontend integration tests must exercise the launch
grant, cookie, CSRF, WebSocket, and token-rotation flows.
