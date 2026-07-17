# Daemon Session Lifecycle and Live Dreaming Design

## Goal

Make task sessions safe to leave behind, let a local administrator start and
observe Dreaming from the web application, and keep the whole experience on
the one loopback daemon address without browser token handling.

## Scope

This replaces the operational requirement for an agent to always close a
session manually. It does not change the semantic meaning of short-term
memory, Dreaming passes, or MCP authentication.

## Session lifecycle

`task_sessions` remain the operational boundary for a unit of agent work. A
session owns its short-term memories and context; an active session is not
eligible for Dreaming. Every short-term-memory write updates the session's
activity timestamp.

The daemon scheduler runs the lifecycle check at its existing one-minute
cadence. It closes each active session with no activity for 30 minutes. A
manual administrator action can close a selected active session immediately.
Both routes use the same idempotent completion operation; a completed,
dreamed, or missing session produces a clear no-op/error response rather than
mutating another record.

After either automatic or manual closure, the daemon evaluates the existing
`min_pending_short_term_memories` setting across all completed, unarchived
short-term memories. If the threshold is met and no dream cycle is active, it
starts one Dreaming run immediately. It does not introduce a second threshold
or wait for the regular schedule interval. Explicit manual Dreaming always
bypasses the minimum threshold.

## One daemon and local access

The existing loopback HTTP daemon remains the only process and address for the
web app, admin APIs, WebSocket endpoint, MCP bridge, and scheduler. Browser
admin and configuration routes do not use a token, but loopback binding alone
is not authorization: a hostile website could otherwise issue same-browser
requests to the local service.

Static app navigation and assets remain available without an Origin header.
Every browser admin/configuration API request and the `/ws/admin` upgrade must
have an `Origin` equal to the daemon's own local HTTP origin (scheme, loopback
host, and bound port); a missing or mismatched Origin is rejected. This blocks
cross-origin fetch, DNS rebinding, CSRF, and WebSocket connections. The server
remains bound to loopback only. Internal `/api/mcp/*`, daemon lifecycle, and
other non-browser service routes continue to require the daemon token.

## Live Dreaming transport

The daemon exposes one WebSocket endpoint, `/ws/admin`. It is local-UI access
and therefore has the same no-token rule as the admin routes. A small in-process
event hub fan-outs JSON events to connected browsers; it does not persist an
event queue and reconnecting clients reload their dashboard/snapshot normally.

Events have a stable `type`, timestamp, and payload. The initial event set is:

- `session_closed` — session ID and whether closure was automatic;
- `dream_started` — run ID, cycle ID, trigger (`manual`, `threshold`, or
  scheduled), provider, and pending input count;
- `dream_phase_progress` — run/cycle IDs, phase name, completed phase count,
  total phase count, and current phase status;
- `dream_completed` — run/cycle IDs and resulting input/crystal/proposal
  counts;
- `dream_failed` — run/cycle IDs and a safe error message.

The HTTP request that starts a manual run returns immediately with the run
identifier and `running` status. If a run is already active, it returns HTTP
200 with that existing run and `started: false`; it never creates a duplicate.
The UI disables duplicate starts while a run is active. Completion/failure is
published from the Dreaming worker, so the request thread never waits for an
LLM call.

## Web UX

Overview shows current Dreaming state, a compact phase progress indicator when
running, and a prominent `Run Dreaming now` action. The Dreaming configuration
page has the same action and live status, so users do not need to navigate away
from settings. A successful request gets an immediate toast; a completed or
failed run gets a follow-up toast from the WebSocket event.

`Short-Term Sessions` exposes `Close session` only for active sessions. The
action asks for confirmation, updates the selected snapshot, and lets the live
event refresh related dashboard and memory data. `Short-Term Memory` remains a
read-first view; it is not used as a proxy for closing a whole session.

All mounted admin pages maintain one WebSocket connection. On each relevant
event they reload their dashboard; Memory Views also reloads the current
snapshot. A successful reconnect immediately reloads the dashboard and current
snapshot before waiting for another event, closing the missed-event gap. If the
socket disconnects, the UI shows a non-blocking reconnecting state and retries
with bounded backoff. API mutations still display explicit error toasts.

## Error handling and safety

Only one Dreaming cycle may execute, using the existing dream lock. A manual
start while one is active returns its run information rather than creating a
second cycle. Scheduler failures are logged and published as `dream_failed` if
a run was started. Closing sessions and starting threshold Dreaming are
independently retry-safe: closure commits before launch, and a subsequent
scheduler tick may launch an eligible run if the first launch failed.

## Validation

Backend tests cover activity timestamp updates, timeout closure, manual close,
threshold-based launch after either close mode, minimum bypass for manual
launch, duplicate-run protection, route Origin/token authorization boundaries,
and emitted event payloads.
Frontend tests cover run/close controls, live progress, reconnect behaviour,
and refreshes caused by WebSocket events. Browser-level tests verify a local
admin page works without a token and reflects a completed manual run.
