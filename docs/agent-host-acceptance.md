# Checking an installed writing agent

Connecting MCP, installing skills and loading hooks are independent outcomes.
A generated bundle, native stdio handshake or another host's transcript does not
prove that an installed writing agent remembers and corrects a story usefully.

## Scope and known limits

The current required workflow matrix is Claude/Codex/Pi. zCode shares the
Claude-format bundle, but its evidence stays separate and paused/unqualified
unless actually tested. Pi supplies skills and MCP context through its passive
package/adapter; active trusted correction ingress is deferred.

Historical Claude/Codex probes demonstrated modern protocol negotiation and
actual prompt stdin for particular versions. They do not qualify a new bundle,
new SessionStart/SessionEnd hooks or the complete story workflow. Old P2 handshake
failures are historical: the stdio initialization bridge is now documented in
[ADR 0015](adr/0015-mcp-protocol-and-transport.md).

This consolidation runs no native hosts and establishes no new host pass.
Use actual current run/issue evidence before stating support for a specific
version. Missing native evidence is an advisory qualification gap under AGENTS.md,
not a reason to weaken authority checks or require an author review queue.

## Set up a disposable workflow

Use a synthetic writing project, private disposable host/configuration homes and
a disposable Hieronymus root. Load the actual packaged bundle and its MCP
registration through the host's native mechanism. Do not alter the user's normal
host configuration, copy secrets into logs or invoke fabricated host events.
Record application/host/model versions and exact bundle/artifact hashes.

Verify MCP status/tool calls, skill discovery and actual native hook loading
separately. The installed `hieronymus-doctor` workflow uses
[session health](../crates/hiero/resources/agent-health.md) for current checks.
Do not create a memory task merely to test setup.

## Observe useful behavior

| Scenario | Evidence to inspect |
| --- | --- |
| Ordinary read/learn/recall | Source import, scoped observation IDs and returned evidence across tasks |
| Task completion | The task completes its own exact session; nested workflows leave the caller active |
| Explicit rendering correction | Genuine supported ingress, applied receipt and next deterministic validation |
| Factual invalidation/qualification | Exact claim/scope, next recall and retained effect after restart |
| Negative usefulness feedback | One correlated/idempotent score effect, no rule or factual invalidation |
| Earlier/later viewpoint | Evidence-backed order; later knowledge does not become an earlier character's truth |
| Provider outage/recovery | Durable input/correction, visible unfinished work and real later completion |
| Resume/termination | Actual native events, activity guard, pending retry and no premature session closure |

Include ambiguous identity, stale revisions, cross-session/series negatives,
ordinary-origin rejection, identical-prompt/new-delivery identity and exact retry.
Accepted/tentative signals do not satisfy `required_decision_id`. Relevant unbound
prompts are not retained/applied merely because binding instructions were returned.

Hooks are a same-account local interface; native event shape is not cryptographic
host provenance. No same-user token, parent PID or process-name shortcut proves a
stronger boundary. [Hook context](agent-hook-context.md) owns binding, pause/retry
and lifecycle contracts; [authority ingress](authority-ingress.md) owns receipts.

## Keep the result usable

Save concise transcripts/logs and the observed effect in an issue/PR or run artifact,
with tested identities and cleanup status. Preserve failed/unavailable steps as such.
Do not transfer a zCode result to Pi, a transport pass to semantic accuracy, or a
synthetic provider result to a commercial model. Reuse an existing issue for the
same reproducible problem. The [roadmap](roadmap.md) keeps follow-up scopes;
completed matrices and old timestamps stay in Git/local history.
