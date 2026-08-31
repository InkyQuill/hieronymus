# Pin The MCP Protocol And Standard Transports

## Status

Proposed. On acceptance, this ADR supersedes uses of the package-generation
labels “MCP v1,” “MCP SDK v2,” and any description of
`/api/mcp/{operation}` as Streamable HTTP in earlier plans and Rust proposal
documents. It narrows ADR 0012's versioned-discovery decision with an exact
wire protocol.

## Context

The Python application exposes standard MCP over stdio, but its daemon also has
a private operation-style HTTP bridge at `/api/mcp/{operation}`. That bridge is
not the JSON-RPC Streamable HTTP transport. Earlier documents conflate SDK
package generations, the private bridge, and MCP protocol revisions, making
transport parity impossible to test.

## Decision

The Rust cutover targets the official MCP protocol revision `2026-07-28` as
defined by the [MCP specification](https://modelcontextprotocol.io/specification/2026-07-28).
The compatibility manifest pins the protocol revision, JSON-RPC schemas,
capabilities, tool registry, error mapping, and transport behavior.

The supported transports are:

- **stdio:** `hiero mcp` exchanges newline-delimited JSON-RPC messages on stdin
  and stdout and writes diagnostics only to stderr;
- **Streamable HTTP:** the daemon accepts JSON-RPC HTTP POST requests at the
  single `/mcp` endpoint and returns either one JSON response or a
  request-scoped SSE stream as defined by revision `2026-07-28`.

Both transports expose the same registry and protocol semantics. `hiero mcp`
is an authenticated adapter to the daemon's `/mcp` endpoint; it does not open
SQLite, translate calls into private operation URLs, or invent another schema.
HTTP clients send the required `MCP-Protocol-Version` metadata and normal local
service credentials. Unsupported revisions fail negotiation with a stable
diagnostic; the server does not guess or silently downgrade.

The Python-only `/api/mcp/{operation}` surface is classified as an internal
compatibility bridge, not an MCP transport. It is recorded in the manifest as
intentionally removed at Rust cutover by this ADR. The Python reference may
remain available to the differential harness, but generated integrations and
the Rust daemon use only standard stdio or `/mcp`.

## Consequences

MCP parity means semantic parity for the same registry plus conformance to an
exact protocol revision, not path parity with the private Python bridge. A
future protocol revision, compatibility window, or transport removal requires
an ADR and fixtures; an SDK crate upgrade alone cannot change the contract.
