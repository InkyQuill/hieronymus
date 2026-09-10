# Agent host acceptance — current qualification status

The historical P2 observations below describe its earlier binary and default protocol
settings. They are not the current capability finding. Task6 added mandatory MCP2026
discovery and trusted ingress; later native Claude2.1.241/Codex0.147.0 probes negotiated
2026-07-28 successfully. Task7 diagnostic probes observed genuine UserPromptSubmit
stdin on Claude2.1.241, Codex0.147.0 and zCode3.11.2 with supported plugin loading.
These probes do not establish full generated-candidate S1–S7 or semantic workflow
acceptance. That candidate qualification remains pending and must be recorded with
exact artifact/bundle/model identities. See [current wiring](agent-workflows.md).

The owner-authorized required matrix is now Claude/Codex/Pi. Generated Pi package and
synthetic extension tests are preparation only; installed Pi TUI/PTy evidence is still
required before native acceptance. All zCode observations below retain their original
labels and remain paused/unqualified rather than being credited to Pi.

## Historical P2 report (unchanged evidence)

# Agent host acceptance — P2, 2026-09-07

**Workflow acceptance is blocked. Bundle installation is not workflow support.**
The current application exposes stateless MCP revision `2026-07-28`. The
available Claude and Codex clients begin with `initialize`, which the current
stdio proxy forwards to a daemon that rejects that request. No compatibility
adapter was added and no older protocol was represented as the required revision.

## Real host observations

| Host | Native bundle installation | Actual Read/Learn/Remember/correction workflow |
| --- | --- | --- |
| Codex CLI 0.147.0 | Local marketplace added and generated `hieronymus@hieronymus-p2` installed, version 0.7.0 | Blocked: MCP startup failed, `handshaking with MCP server failed: connection closed: initialize response`. No source IDs or provenance returned. |
| Claude Code 2.1.241 | Generated Claude-format manifest validated; optional missing-author warning. Loaded with `--plugin-dir` and explicit MCP config. | Blocked: Hieronymus tools never became registered/callable. Host reported no fabricated results. |
| zCode 3.11.2 | Shares the Claude-format bundle; separate adapter was not invented. | Unverified. No isolated graphical workflow or authenticated isolated profile was established. |

The root preflight previously invoked `zcode --version`, which launches its GUI
and may write normal app caches/protocol metadata. That was not isolated acceptance.
P2 did not relaunch zCode. Read-only inspection of `/usr/lib/zcode/app.asar` found
`ZCODE_DESKTOP_HOME_DIR`, `ZCODE_DESKTOP_USER_DATA_DIR`,
`ZCODE_DESKTOP_SESSION_DATA_DIR`, and `ZCODE_HOME` configuration surfaces. Electron
user-data redirection alone does not demonstrate complete profile isolation or
working MCP access, so no host pass is inferred from these strings.

## Disposable setup and exact commands

The fixture workspace was `/tmp/hieronymus-p2-hosts-qcd29o93/workspace` with one
synthetic English `scene.txt`. Generated bundles were copied from the root's
preflight output to `claude-plugin` and `market/hieronymus` below the same root.
The local marketplace manifest in `market/.agents/plugins/marketplace.json` used
name `hieronymus-p2` and local source `./hieronymus`. Existing host authentication
files were copied directly into the disposable host homes with mode 0600, never
printed or committed. Authentication copies were removed after the probes.
No real plugin/MCP configuration or book contents were edited.

```sh
CODEX_HOME=/tmp/hieronymus-p2-hosts-qcd29o93/codex-home \
  codex plugin marketplace add /tmp/hieronymus-p2-hosts-qcd29o93/market --json
CODEX_HOME=/tmp/hieronymus-p2-hosts-qcd29o93/codex-home \
  codex plugin add hieronymus@hieronymus-p2 --json
claude plugin validate /tmp/hieronymus-p2-hosts-qcd29o93/claude-plugin
```

The built `hiero` had a disposable `bin/hieronymus-mcp` symlink, exercising normal
argv routing. A real daemon ran with `--data-root .../data --port 0`; explicit
checksum-verified original ONNX assets were staged there, and `hiero semantic
enable --runtime <repo>/qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so
--data-root .../data --json` returned `armed`, `ready`, and `runtime_verified:true`.
The handshake failure therefore does not establish a semantic startup failure.

Both host processes received `PATH=<temporary-root>/bin:$PATH` and
`HIERONYMUS_DATA_ROOT=<temporary-root>/data`:

```sh
CODEX_HOME=<temporary-root>/codex-home codex exec --skip-git-repo-check \
  --json -s workspace-write '<workflow prompt>'
CLAUDE_CONFIG_DIR=<temporary-root>/claude-home claude --print \
  --plugin-dir <temporary-root>/claude-plugin --setting-sources '' \
  --no-session-persistence --strict-mcp-config \
  --mcp-config <temporary-root>/claude-plugin/mcp/hieronymus.mcp.json \
  --allowedTools 'Read,mcp__hieronymus__*' --output-format json '<workflow prompt>'
```

The prompts explicitly requested actual plugin/MCP operations: Read `scene.txt`,
create a disposable series (en→ru), start a session, import the scene, learn the
1902 electrification note, recall it, semantic-search the physician paraphrase,
correct the year to 1903 through a supported correction operation, complete/start
another session and inspect retention. Both prohibited shell/credential reads
and simulated tool results. These steps remained blocked; no workflow pass is
claimed. Codex supplied the no-hooks test subject: its installed generated bundle
has no hooks declaration. That proves only the installed configuration, not that
the no-hooks memory workflow works.

Logs: `/tmp/hieronymus-p2-codex.jsonl`, `/tmp/hieronymus-p2-codex.stderr`,
`/tmp/hieronymus-p2-claude.json`, `/tmp/hieronymus-p2-claude.stderr`,
`/tmp/hieronymus-p2-host-enable.json`. These local probe logs are not release
artifacts or reproducible host credentials.

A direct native stdio probe sent ordinary JSON-RPC `initialize` envelopes with
`params.protocolVersion`, `capabilities:{}`, and `clientInfo`. Versions
`2025-03-26`, `2025-11-25`, and `2026-07-28` all returned `-32020`,
`Header mismatch: mirrored request metadata does not match`. Inspection of
`crates/hiero/src/daemon/protocol.rs` confirms an explicitly stateless contract
with no initialize; `crates/hiero/src/stdio.rs` forwards requests without a
legacy handshake bridge. Merely changing the version label does not fix this.

## Required unresolved behavior

P1 is an approved design, not implemented runtime. These named acceptance cases
remain **unresolved**, independently of the host handshake blocker:

- `immediate-factual-correction`: learning 1902 then an explicit correction to
  1903 must suppress/qualify obsolete 1902 on the next ordinary recall, including
  when the consolidation provider is unavailable.
- `immediate-rendering-correction`: an explicit scoped Russian rendering must
  reach the deterministic contract before dependent validation can succeed.
- `earlier-character-viewpoint`: a revelation at chapter 12 must not appear as
  known to the chapter-3 viewpoint character. Future evidence must be excluded
  or explicitly qualified, using manifest order rather than numeric guessing.
- `later-character-viewpoint`: the same source may apply after the revelation;
  a narrator's knowledge and a character's knowledge are separate gates.

The real retrieval fixture explicitly represents language and context; nonempty
viewpoint/story-position context fails rather than being silently ignored. Once
P1 runtime is implemented, add those public-MCP behavioral assertions before
claiming completion. No storage-only correction test substitutes for them.
