# Agent workflows

The Rust generator teaches automatic scoped memory maintenance: establish story context,
recall before chapter work, capture significant observations, validate terminology and
record correlated relevance feedback. Learned evidence can activate/revise terminology;
clear independently delivered corrections apply immediately. No author label or routine
approval queue confers authority. See [authority ingress](authority-ingress.md) for exact
public DTOs, current-selection rules and receipt dependencies.

`hiero plugins generate` (`--dry-run`, `--json`) deterministically writes eight skills,
MCP configuration and host manifests under `<data-root>/agent-plugins/`. It does not
edit host profiles or book files. The generated targets are codex, claude, gemini,
opencode, openclaw and pi; retained output is not proof of native support. The required
native matrix is Claude/Codex/Pi. zCode's prior shared-Claude results remain historical,
paused and unqualified.

The skills are hieronymus-bootstrap, recall, learn, read, remember, translate, review and
orchestrate. They preserve source language and explicit narrative scope, distinguish
current truth from research, and separate factual invalidation from relevance feedback.
RAG reads use the current results/non_current envelope and must report unavailable
semantics honestly. Ingest individual typed claims and real file evidence, not invented
source_role/user_rule provenance. Two independent aligned paragraph anchors and resolved
identity/scope can support a learned decision; ambiguous names remain tentative.

## Installed host wiring

The stable `hieronymus-mcp` command discovers the daemon; generated JSON contains no
port or credential. A custom installation sets `HIERONYMUS_DATA_ROOT` and places its
installed `bin` directory on PATH. Use each host's explicit MCP `2026-07-28` opt-in;
never downgrade silently. Claude requires launcher environment `MCP_SDK_GENERATION=v2`
and `MCP_PROTOCOL_NEGOTIATION=auto`. Codex requires `[features] mcp_2026_07_28=true`;
the generated Codex server environment supplies `CODEX_MCP_PROTOCOL_VERSION=2026-07-28`.
zCode exposes a supported per-server `protocolVersion=2026-07-28` override outside the
shared bundle; a disposable native zCode3.11.2 probe observed server/discover, tools/list, status and
series_list with this override. This is protocol evidence, not candidate workflow acceptance.
Historical P2 failures and newer qualification status are
tracked in [host acceptance](agent-host-acceptance.md).

Pi can install `<data-root>/agent-plugins/pi` through `pi install <path>` for its package
resources. In normal package loading the Hieronymus extension is passive: prompts continue
unchanged, packaged skills remain callable, and the separately installed `pi-mcp-adapter`
provides ordinary MCP context reads. No trusted correction receipt is minted because generic
extension discovery cannot prove that Hieronymus saw the original input. Use this isolated
launch only when trusted interactive correction delivery is required:

```sh
HIERONYMUS_PI_TRUSTED_LAUNCH=isolated-v1 pi \
  --no-extensions \
  -e <data-root>/agent-plugins/pi/extensions/hieronymus.ts \
  -e <installed-pi-mcp-adapter>/index.ts \
  --mcp-config <data-root>/agent-plugins/pi/mcp.json \
  --skill <data-root>/agent-plugins/pi/skills
```

Pi documents repeated `-e` arguments in load order; `--no-extensions` excludes ambient
handlers, making Hieronymus the first input handler and the installed adapter the only MCP
implementation. The generated `mcp.json` registers only `hieronymus-mcp` and pins
`protocolVersion` to `2026-07-28`; `pi-mcp-adapter` owns discovery, lazy lifecycle,
authoritative tool catalog, calls and error envelopes. The Hieronymus extension accepts
raw pre-expansion text only when Pi reports `source: interactive` under this isolated
launch. Ordinary text carries its correlated receipt through normal, steering and follow-up
queues. Idle slash skills/templates retain their raw command for native expansion and get
the correlated context at `before_agent_start`; slash commands during streaming must be
retried idle. RPC and extension input cannot mint a trusted delivery. With images attached,
only the exact text is trusted and the transformed context
explicitly excludes image content. Hook errors return Pi's handled result with a visible
diagnostic so the original prompt does not proceed.

Claude loads `<data-root>/agent-plugins/claude` with its supported `--plugin-dir` option.
The manifest explicitly references `hooks/hooks.json`. zCode's supported isolated
`plugins.dirs` setting can load that same Claude directory. A zCode launcher must set
`HIERONYMUS_AGENT_HOST=zcode`; the identical shared hook defaults to claude otherwise.
The installed handler validates the resulting supported host name. This is explicit
launcher identity, not a guess based on transcript paths or prompt text.

Codex uses its native local marketplace mechanism:

```sh
codex plugin marketplace add <data-root>/agent-plugins --json
codex plugin add hieronymus@hieronymus-local --json
```

The generated `.agents/plugins/marketplace.json` references `./codex`. The Codex manifest
references `hooks/hooks.codex.json`; optional hooks use the supported event-object,
nested hooks/type=command format. Trust only the inspected hook commands through the
host's supported trust interface. Codex 0.147 exposes hook key/currentHash in app-server
`hooks/list` and accepts each exact `hooks.state."<key>".trusted_hash` through
`config/batchWrite`; a changed hash needs a fresh decision. Do not use a blanket hook
trust bypass. Tool approval is separate: `--ask-for-approval never` does not itself
approve MCP tools. Disposable qualification must explicitly authorize the intended
server tools with the host's supported tool approval settings.

Hook enablement remains an optional host trust/configuration choice. Current generated
hooks subscribe to UserPromptSubmit, not an assumed cross-host SessionEnd event.
Older legacy session-start/session-end CLI entrypoints remain available independently.
Diagnostic native probes confirmed this hook shape on Claude 2.1.241, Codex 0.147.0 and
zCode 3.11.2; candidate installation and complete S1–S7 workflows are separate gates.

## First session and explicit selection

The first actual UserPromptSubmit without a binding returns read-only `binding_required`
context with the actual host/session identity. It explicitly says the prompt was **not
retained or applied** as a correction. It creates no domain session, origin or decision.
The model establishes the real MCP session, imports/chooses actual evidence or claims,
reads the authority revision and pipes those returned values to the installed
`hiero agent-hook bind-context` command as documented in authority-ingress.md. It must
not manually write host-context files, guess IDs or replay the initial prompt through
a fabricated event. A subsequent genuine host prompt can then apply immediately.

Every genuine invocation gets a fresh delivery UUID; identical text is not deduplication.
Exact retry uses `retry-delivery --delivery-id` and its frozen saved context. New source
selection or a later authority revision requires explicit rebinding from new observed
outputs. Existing in-flight events are never rebased. An unresolved authentic signal
increments revision and queues gathering but has no rule/claim effect; it cannot satisfy
required_decision_id. Applied/Replayed hook results supply that dependency for model
recall/contract/validation. Ordinary MCP cannot redeem host or console origins.

No generated file itself proves trusted ingress, semantic retrieval or installed host
acceptance. Native version, bundle hash, model, actual events and durable effects must
be recorded by the acceptance run. No plugin rewrites user host configuration, reports
into a book folder, or claims a pending consolidation completed.
