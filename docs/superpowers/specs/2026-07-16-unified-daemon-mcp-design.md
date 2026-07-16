# Unified Daemon and MCP Design

## Goal

Make one local Hieronymus daemon the sole owner of SQLite-backed domain work.
The web console, dreaming scheduler, CLI lifecycle commands, and stdio MCP
adapter must discover and use the same daemon for one data root.

## Current Problem

`hiero config` and `hiero admin` already call `ServiceManager.ensure_running()`
and open the daemon's loopback web server. The stdio MCP server instead creates
Python domain stores in its own process (`stdio-direct-store`). This gives two
processes authority to read and write the same SQLite files and makes daemon
health unrelated to MCP activity.

## Architecture

The daemon remains a loopback-only HTTP server authenticated by the random
token in `server.json`. Add an internal, explicit MCP operation registry behind
`POST /api/mcp/<operation>`. Each operation has a fixed handler and a JSON
input/output contract. The route never resolves handler names from request
input and rejects unknown operations.

`mcp_server.py` becomes a thin stdio-to-daemon adapter:

1. Load the normal `HieronymusConfig` for the invoking environment.
2. Call `ServiceManager(config).ensure_running()`.
3. Read the resulting state and call the authenticated loopback MCP endpoint.
4. Return the daemon payload unchanged except for transport errors, which are
   converted to a clear MCP-visible error.

The existing MCP tool names, argument names, return payload shapes, and
deterministic terminology precedence remain unchanged. The direct-store MCP
adapter and its status label are removed.

## Operation Boundaries

The migration covers every existing `@server.tool()` operation in
`mcp_server.py`, grouped by domain rather than by UI route:

- status and series;
- concepts, facets, semantic tags, and links;
- crystals, rule crystals, terminology validation and proposal approval;
- memory and RAG import/search;
- sessions, short-term memory, recall, and feedback;
- dreaming and concept proposals.

Handlers may reuse the present domain services and serializers inside the
daemon. They must not reimplement domain rules in HTTP routing code.

## Lifecycle and Diagnostics

The default data root is `~/.config/hieronymus` unless `--data-root` or
`HIERONYMUS_DATA_ROOT` explicitly selects another root. All daemon consumers
must use the same resolved config for their current invocation.

MCP auto-starts the daemon if it is absent, and reuses a healthy state if it is
present. The daemon process owns the random port, token, web console, dreaming
scheduler, and domain mutation execution.

`doctor` continues to be read-only by default. It reports one of:

- no daemon state / daemon stopped;
- stale or unreachable state;
- running daemon, including PID, loopback port, and data root.

It must not label a successful health check as an autofix.

## Security and Failure Rules

- Bind only to `127.0.0.1`.
- Require the existing token for every `/api/mcp/` request.
- Accept only JSON objects up to the existing request size bound.
- Reject unknown operations with a 404 JSON error.
- Convert invalid domain parameters to 400 JSON errors.
- On MCP client transport failure after `ensure_running()`, raise an actionable
  error; never fall back to direct SQLite access.

## Verification

- Unit-test the allow-list and authorization of MCP operation routes.
- Unit-test `DaemonMcpClient` auto-start/reuse/error paths with fake lifecycle
  and service clients.
- Move existing MCP contract tests to exercise the daemon boundary and compare
  their existing payloads.
- Add an integration test proving MCP, config/admin, and doctor report one
  PID, port, and data root.
- Build and test-install the wheel; stop the daemon, invoke an MCP operation,
  then verify that config/admin reuse the started daemon.
