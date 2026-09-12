# Agent workflows

The Rust generator teaches automatic scoped memory maintenance under the current user's
free-text project agreement: establish story context, recall before chapter work, capture
permitted significant observations, validate applicable terminology and record correlated
relevance feedback. With no special instruction, project files are primary and Hieronymus
is additional memory. Learned evidence can activate or revise terminology; clear
independently delivered corrections apply immediately. Neither a technical project binding,
an author label nor a routine approval queue confers authority or permission to write. See
[authority ingress](authority-ingress.md) for exact public DTOs, current-selection rules and
receipt dependencies.

`hiero plugins generate` (`--dry-run`, `--json`) deterministically writes eight skills,
MCP configuration and host manifests under `<data-root>/agent-plugins/`. It does not
edit host profiles or book files. The generated targets are codex, claude, gemini,
opencode, openclaw and pi; retained output is not proof of native support. The required
native matrix is Claude/Codex/Pi. zCode's prior shared-Claude results remain historical,
paused and unqualified.

The skills are hieronymus-bootstrap, recall, learn, read, remember, translate, review and
orchestrate. Every generated target carries a local
`hieronymus-bootstrap/resources/cws-project.md`; bootstrap links to it and the other skills
reuse bootstrap's project-context workflow by name. The resource is self-contained and
requires no CWS installation for supported project reading. The skills preserve source
language and explicit narrative scope, distinguish
current truth from research, and separate factual invalidation from relevance feedback.
RAG reads use the current results/non_current envelope and must report unavailable
semantics honestly. Ingest individual typed claims and real file evidence, not invented
source_role/user_rule provenance. Two independent aligned paragraph anchors and resolved
identity/scope can support a learned decision; ambiguous names remain tentative.

## Read-only CWS project inspection

The installed CLI can inspect the nearest Creative Writing Skills project without
starting a daemon, loading its configuration, installing CWS, or returning
manuscript and project-instruction contents:

```sh
hiero project-context --cwd /path/to/project --args '{"direction_id":null}' --json
```

The JSON projection always contains `version`, `status`, `root`, `schema_version`,
`instructions_path`, `binding`, `direction_id`, `source_language`, `target_language`,
and `diagnostics`; absent optional values stay `null`. Human output uses those keys in
the same order as `key: value` lines and writes `none` for null or empty diagnostics.
The caller reads the reported `AGENTS.md` separately together with current user
instructions and interprets their free-text agreement in the current task. The agreement
is not reduced to a mode, ranking or stored summary. Detection and binding do not decide
trust or authorize writes. A rule that remains active inside Hieronymus keeps that actual
status and provenance even when the agreement says project memory governs the current
work; applying the agreement does not internally invalidate the rule. Internal changes
still require the evidence and trusted-ingress routes documented above.

A new user instruction applies immediately in its stated scope. A local exception stays
local. When the user requests a durable agreement change, re-read and update the nearest
`AGENTS.md` through the recoverable project workflow while preserving unrelated and
independent conditions. A change in source preference does not prove records were
transferred and does not authorize a migration, deletion, or mirror.

Statuses `ready` and `unbound` exit 0. `ambiguous`, `unsupported`, and `not_found` exit
1; `invalid` exits 2. Diagnostics are stable technical strings:
`project_not_found`, `unsafe_path`, `invalid_manifest`, `invalid_binding`,
`unsupported_schema`, `unsupported_binding_version`, `unsupported_contract_version`,
`ambiguous_direction`, `ambiguous_volume`, `conflicting_direction`, `uncovered_edition`,
`unknown_direction`, and `filesystem_error`. Filesystem error details are not printed.
Contract version 1 supports project schemas 1 and 2 independently. Schema 2 resolves
explicit directions or uniquely identifying selected paths through actual manifests.
Several directions may use the same language and series. A direction without its own
map entry is `unbound`, even when the common-work slug is bound. Languages come from
the effective primary edition and direction, including per-volume source overrides.

For `ambiguous_volume`, select the relevant volume with `--cwd`. When all covered
volumes have the same effective editions, inspection can return their common context
without choosing a volume. Missing edition coverage fails with `uncovered_edition`.
The adapter preserves edition-qualified unit references and hashes for selected task
evidence; the ten-key CLI envelope remains unchanged. Skills carry
`cws:direction:<id>` in `story_scopes` for sessions and reads and in each applicable
claim's `scope_predicates`. Edition-specific evidence additionally carries
`cws:edition:<id>`. Volume/chapter and knowledge gates stay independent. Session
reload preserves predicates, contradictory direction selections fail, and omitted
languages use registry defaults while explicit nonempty overrides are normalized.

Generated indexes, `.creative-writing/`, translation lifecycle metadata, supplied originals and nested projects remain protected boundaries. The workflow
does not import whole projects, copy secrets or hidden text, require synchronized stores,
or recursively launch a second orchestrator.

Translation tasks retain strict external dependencies as
`provider`/`namespace`/`record_kind`/`record_id`/`revision`. The
Hieronymus namespace uses the observed ephemeral `status.instance_id` and actual
series. Capture or short-term responses without a public identity or revision use a
separate unverified marker; no revision is invented. Before status or acceptance, CWS
receives current strict observations through `--external-memory-observed`. Known
changed or missing references require review and take precedence; unavailable
observations and captured markers stay unknown. Historical unknown-at-acceptance is
preserved even if later matching strict observations establish current freshness. The
caller report is not authenticated remote verification, and every local source, review,
base, coverage and path guard remains in force.

An authorized transfer inventories only the selected records and provenance, uses public
reads and writes, retains returned IDs and revisions, and reconciles selected, written,
skipped, conflicting, pending and unresolved counts together with scopes and
dispositions. Partial completion and unresolved conflicts are reported explicitly.
Originals remain in place and no ongoing mirror is created unless the user requested
that work. A requested authoritative write that returns pending, rejected, tentative or
unresolved remains that exact result; it is not reported as a successful replacement.

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

Pi can install `<data-root>/agent-plugins/pi` as an ordinary package:

```sh
pi install npm:pi-mcp-adapter
pi install <data-root>/agent-plugins/pi
```

The first command installs Pi's separately packaged MCP prerequisite; restart Pi
after installation. The Hieronymus package exposes all eight skills. Its generated `mcp.json` registers only
`hieronymus-mcp` and pins
`protocolVersion` to `2026-07-28`; `pi-mcp-adapter` owns discovery, lazy lifecycle,
authoritative tool catalog, calls and error envelopes. Pi can use Hieronymus MCP tools and
skills after package installation. This passive package has no input hook and does not
recognize trusted corrections, mint Applied receipts, or block prompts.

Claude can install the generated local marketplace literally:

```sh
claude plugin marketplace add --scope local <data-root>/agent-plugins
claude plugin install --scope local hieronymus@hieronymus-local
```

The catalog at `.claude-plugin/marketplace.json` resolves `./claude` from the catalog root.
The Claude manifest explicitly references `hooks/hooks.json`. zCode's supported isolated
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
