# Service operations

The Rust daemon owns normal live database mutations, background processing and
MCP. The CLI and stdio adapter communicate with it. Exclusive migration/recovery
runs only with the daemon stopped; export is read-only. See
[ADR 0009](adr/0009-runtime-topology-and-daemon-lifecycle.md).

## Start and diagnose

```sh
hiero daemon
hiero service status --json
hiero status --json
hiero doctor --json
hiero semantic status --json
```

`hiero daemon` runs the server directly. A managed installation can instead use
`hiero service start`, `stop` and `status`. `hiero config` and `hiero admin` start
or discover the local server and open its web console. The tray is managed by the
server directly or through its helper; see [desktop registration](desktop-tray.md).
`hiero` and `hieronymus` are equivalent command names.

Automation should consume JSON and discovery, not parse human status text.
A configured provider or live process is not evidence that semantic retrieval
is ready. Errors and background warnings remain available in readable diagnostics.

## Files under the selected data root

[Usage](usage.md#data-and-privacy) owns platform defaults and override precedence.

| File/directory | Purpose |
| --- | --- |
| `hieronymus.sqlite` | Authoritative memory and domain data |
| `provider.conf` | Provider profiles, timeouts, defaults and API keys |
| `dream.conf` | Dream workflows, prompts, schedule and processing budgets |
| `comparison.conf` | Optional memory comparison assignments and timeout |
| `ingest.conf` / `relevance.conf` | Ingestion policy / optional Jev classification |
| `release.conf` / `web.conf` | Update channel / optional browser authentication |
| `llmcache.tmp` | Derived model-list hints, not configuration authority |
| `daemon.json` | Discovery and process identity; no bearer credential |
| `daemon.token`, `console.token`, `host-event.token` | Separate private local credentials |
| `agent-plugins/` | Installation-owned generated skills and host bundles |
| `backups/` / `logs/` | Recovery inputs / private readable diagnostics |

Do not copy tokens into plugin JSON, URLs or reports. Browser Host/Origin checks
remain active with authentication either on or off; MCP credentials remain required.
See [authentication and discovery](adr/0012-mcp-transport-authentication-and-discovery.md).

## Call tools and inspect processing

With the daemon running, `hiero tool-call` invokes the advertised MCP registry:

```sh
hiero tool-call hieronymus_series_list --args '{}' --json
hiero tool-call hieronymus_dream --args '{}' --json
```

`--start-daemon` is an explicit opt-in for tool calls. Inspect installed help and
MCP `tools/list` for current argument schemas. Do not guess record IDs or use
historical Python fixture schemas as the current tool registry.

Dream runs use configured workflows. The MCP call waits for the finished drain;
requests arriving during an active run share its result. Inspect phase results
and pending counts rather than treating an accepted request as completion.

## Export, upgrade and recovery

```sh
hiero export --output /absolute/path/memory.json --json
hiero migrate --dry-run --data-root /absolute/disposable/copy
```

Export reads SQLite without copying a live database file and writes to an explicit
destination. It is a content export, not a promise of full installation backup or
an automatic import roundtrip. For migration testing, use a copied data root.
Never run destructive recovery experiments on a working book's installation.

Migration classifies and validates the source, preserves an immutable backup,
and refuses unsafe or unsupported schemas. Recovery uses the current Rust runtime
and verified backup inputs; it does not reactivate Python. See
[ADR 0010](adr/0010-data-locations-schema-ownership-and-upgrade.md) and
[Distribution](distribution.md#install-update-and-recovery).
