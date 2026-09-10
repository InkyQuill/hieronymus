# Practical release acceptance — 2026-09-10

The owner selected seven ordinary-product checks and deferred the full native host matrix
until the week of September 14. See [deferred qualification](deferred/2026-09-10-native-host-qualification.md).
No public release, marketplace publication, or full native correction qualification is claimed.

## Source and artifact identity

The practical baseline is `63f4535`; the observed long-running MCP client defect is repaired
in `d9c3863`. Its release archive and public installation passed.

Final archive SHA-256: `6b163d16ac4cba0e58542d91b58fa189fca220364e297e6d52e0042753f8538e`.
Final native SHA-256: `7e43098df16881baacd123da06e754d89cc989e7f952a33045d277790d8f9d78`.

Baseline archive SHA-256: `cc74a34d525405635345be9159c2b239f52fcd35dd742ca539c3286e7f8715a3`.
Baseline native SHA-256: `88643d85a15c47aced565d2644dd2b46b3b80b0aa440b1efc98bd0f92fc7436c`.
Both identify version 0.8.0, schema 5, MCP revision 2026-07-28.

## Practical observations

| Owner check | Actual evidence |
| --- | --- |
| MCP builds | Release build, embedded frontend checks, real bundled-model inference, archive checksum/round-trip and public installer passed. |
| Tools work | Pi discovered 43 tools and performed actual status, series creation, source import, session, memory and RAG calls. Claude and Codex called status and series_list. This is not a claim that every tool was exercised with every host/model. |
| Agent saves information | Pi 0.85.1 with DeepSeek-v4-flash saved memory 1 in series 1, then completed session 1. Independent read-only SQLite inspection confirmed the row and session status. |
| Agent uses RAG | Pi imported the three-paragraph fictional lighthouse fixture and retrieved paragraph1 for the key question. The returned rank reason was `rag semantic match`; unknown chronology stayed labelled source-inspection/non_current evidence. |
| Ollama embeddings | The installed daemon used nomic-embed-text:latest, 768 dimensions, immutable digest `0a109f422b47e3a30ba2b10eca18548e944e8a23073ee3f3e947efcf3c45e59f`, an active intact generation, and exact-text processing with truncation disabled. Readiness and index identity survived a daemon restart. |
| Dream | OpenAI-compatible DeepSeek-v4-flash completed cycle 2 and persisted three crystals from the completed-session memory. Native Ollama qwen3.5 exhausted three 120s attempts and is not accepted. On the repaired installed artifact, Pi saved memory 2 in a new session, completed it and called Dream through MCP. Cycle 3 returned `completed`, input_count 1 and created_crystal_count 5 after 24.1s; read-only DB inspection confirmed the same five new crystals. |
| Marketplace/package installation and skills | Generated local marketplaces installed successfully into Claude 2.1.241 and Codex 0.147.0. Both loaded the installed recall skill and made actual MCP calls. Pi installed the generated package using its ordinary package manager and the already installed pi-mcp-adapter 2.32.1, invoked the read skill, and performed the save/RAG workflow. |

The native application is installed first with the public release installer. Agent
marketplace/package installation then supplies MCP registration and skills; it does not
automatically download the native daemon. The ordinary installation prerequisite is
documented in [agent workflows](agent-workflows.md).

## Observed failures and corrections

- The generated bootstrap previously named nonexistent `CurrentKnowledge`; it now names
  `Current` and explains ordinary `OmniscientResearch` when chronology is absent.
- Pi initially tried to store research mode on a session and omitted claim applicability.
  MCP rejected both malformed calls. The agent inspected schemas and corrected them without
  inventing story coordinates. Transient provider connection errors were retried.
- The initial Claude smoke used an unsuitable broad tool-approval pattern and inherited
  stdin. The successful retry used closed stdin and the two exact approved MCP tool names.
- The successful Codex custom-installation smoke used closed stdin, an absolute installed
  MCP command, an explicit `HIERONYMUS_DATA_ROOT`, protocol opt-in and the supported
  `default_tools_approval_mode="approve"` setting in its disposable profile. The initial
  incomplete launcher attempt is retained separately.
- Both initial Dream tool-call attempts hit the shared client's unconditional ten-second
  deadline even while the supervised worker continued. `d9c3863` allows bounded twenty-minute
  `tools/call` forwarding, retaining ten-second control requests, two-second discovery and
  immediate socket cancellation. A real mock response delayed 10.5s failed before and passed
  after this change. This does not override an agent host's own tool timeout.

## Verification and evidence

The pre-timeout baseline passed 1312 Rust tests with12 explicitly ignored artifact/model tests.
The final guidance changes passed 13 focused checks, formatting, all-feature Clippy and
rustdoc with warnings denied. Frontend typecheck, 76 tests and production build passed;
frontend source did not change afterward. The final test-only changes `aa9a7a8`, `fa45623` and `1ab5a36` remove two fixture
races: authority setup competing with daemon recovery, and writing stdin after an expected
startup rejection. They preserve business assertions, rejection checks and live daemon
coverage; no SQL retry or runtime behavior was changed. Prior failed runs are retained.
Final `cargo test --all-features --locked --no-fail-fast` on `1ab5a36` passed:
**1313 passed, 0 failed, 12 explicitly ignored, across 89 target summaries**.
Formatting and all-target/all-feature Clippy passed on the final test changes; rustdoc
with warnings denied passed on the unchanged production tree. Frontend remained unchanged
and its typecheck, 76 tests and build passed. The final commits after `d9c3863` change only
test fixtures and this verification documentation, so the identified release binary
remains the verified production artifact.


Local raw transcripts, read-only DB snapshots, hashes and installer logs are retained under
`.superpowers/sdd/2026-09-06-autonomous-authority-implementation/practical-acceptance/`.
`pi-session.jsonl`, `pi-rag-result.json`, `persistence-evidence.json`, `semantic-ready.json`,
`semantic-restart.json`, `claude-skill-retry.jsonl`, `codex-skill-retry.jsonl`,
`dream-deepseek-persisted.json`, `dream-crystals.json`, `pi-dream-d9c3863.jsonl`,
`pi-dream-result.json` and `dream-d9c3863-persisted.json` distinguish model prose from actual
calls and durable effects. Failed setup/provider attempts are retained, not relabelled passed.

All installation profiles and the literary corpus are disposable. No real book or user
runtime database was changed. Native authentication copies were removed after their process;
temporary provider secrets are removed when the daemon stops. The repository `.env` remains
untracked and ignored. No credentials are included in this report or committed evidence.
