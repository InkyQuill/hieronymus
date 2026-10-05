# 0012 — Local authentication and discovery

Status: accepted 2026-08-31; private-ingress separation and optional browser
authentication amended 2026-09. Consolidated 2026-10-05.

## Context

Loopback binding does not authenticate local processes or prevent browser-origin
attacks. Discovery metadata, credentials and explicit-user correction origins
serve different purposes and must not be interchangeable.

## Decision

The daemon binds loopback by default. Discovery contains protocol/endpoint/process
identity, never credentials. Credential files are separate, atomically written,
owner-private and excluded from generated bundles, status, logs and grants.
Stale discovery requires authenticated probing and instance checks, not PID alone.

Ordinary MCP/native access uses the daemon credential. Console launch grants and
host-event ingress use separate credentials, not the MCP token. This protects
ordinary tool access from minting user origin; it is not isolation against another
process with the same OS-user filesystem/credential access. Agents must not
manufacture host events from quoted user text.

Browser authentication is off by default. `web.conf` can set
`authentication_required=true` for the existing launch-grant/session-cookie flow.
Default console corrections are attributed to the local desktop console, not an
identified authenticated person. Host and exact mutation Origin checks remain
active in both modes. WebSocket access follows the same configuration and guards.
`/health` exposes only minimal liveness, without paths, versions or user data.

When authentication is required, `hiero config`/`admin` mint a short-lived single-use
grant and open it in a URL fragment. The frontend removes the fragment before
exchange for a SameSite=Strict, HttpOnly daemon-lifetime cookie. Grants/tokens never
enter query strings or diagnostics. A read without Origin can be normal browser
navigation; an explicit foreign Origin is refused. Mutations, exchange and WebSocket
upgrade require the exact local Origin. No separate CSRF-token ceremony is required.

Secret-bearing values cross logging/audit/DTO boundaries only through redacting
projections. Explicit exposure is restricted to credential/config writers and outbound
authentication. Token rotation uses atomic replacement; clients reread after 401.
No extra credentials-rotated or authentication-specific idempotency ceremony is required.
Domain decisions retain their own replay/revision policy.

Generated MCP registration uses stable entry points and versioned discovery, with
no fixed port or baked bearer token. `hiero mcp` authenticates to the same `/mcp`
registry; [ADR 0015](0015-mcp-protocol-and-transport.md) owns protocol negotiation.
Unsupported versions fail explicitly rather than guessing a private bridge route.

## Consequences and limits

Native, browser and authority routes share the underlying domain checks while
retaining separate principals. [Authority ingress](../authority-ingress.md) owns
exact origin/selection/revision contracts. [Hook context](../agent-hook-context.md)
owns same-account lifecycle/prompt limitations. An event-shaped envelope or loaded
plugin is not cryptographic native-host attestation.

Verify cookie-free default access, optional authenticated launch/cookie/WebSocket,
foreign Host/Origin refusal and authenticated MCP separately. Credentials and
browser grants must remain absent from JSON/status/diagnostics on both success
and failure. These safeguards do not imply cross-platform or host acceptance.
