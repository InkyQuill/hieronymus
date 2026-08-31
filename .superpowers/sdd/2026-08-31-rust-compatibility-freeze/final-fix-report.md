# Rust compatibility freeze final fix report

Date: 2026-09-01

## Revision boundary

- Reviewed base: `f9ce7cc5e6fd1821fa1612681a8b36ea229aa2aa`
- Final implementation: `bec4b172af7a9719f1775b25701731ddd24fab28`
- Scope: one authorized whole-branch compatibility fix wave; no unrelated
  production behavior was changed.

## Final-review findings

### 1. HTTP current and target behavior

`tools/compatibility/inventory_http.py` and
`compatibility/fixtures/http/route-cases.json` now give every
`intentionally-change` route independently replayable `current` Python and
`target` ADR-backed outcomes. Target headers are selected from target auth,
never current auth. Target `/health` is unauthenticated and returns only
minimal liveness. The fixtures include launch-grant exchange, strict
`HttpOnly; SameSite=Strict` session cookies, CSRF issuance and validation,
Host/Origin failures, and WebSocket credential/resume/rotation cases from ADR
0012. Current response bodies remain derived from the Python handlers.

Evidence: `tests/compatibility/test_http_inventory.py`; focused HTTP suite
passed as part of the 74-test focused compatibility run.

### 2. MCP wire semantics

`tools/compatibility/inventory_mcp.py` derives all 39 tools from the FastMCP
registry. `compatibility/fixtures/mcp/protocol.json` records current Python and
ADR-backed target initialize/capability exchanges, protocol revision
`2026-07-28`, newline-delimited stdio, Streamable HTTP POST/JSON and SSE,
unsupported-version errors, transport registry identity, and the separately
classified private bridge removal. Each tool directory now contains a
tool-specific replayable error input and real MCP success/error result
envelopes. The deterministic in-memory MCP client/server session exercises the
real registry, validation, dispatch, and envelope conversion while a bounded
fake replaces only the network daemon hop.

Evidence: `tests/compatibility/test_mcp_inventory.py`; all 39 registry
contracts and per-tool wire fixtures passed focused and full-suite replay.

### 3. CLI behavior

`tools/compatibility/inventory_cli.py` retains the Click help/parameter
metadata and adds 56 behavior fixtures covering all four installed scripts and
all 52 discovered command/group invocations. The fixtures contain 157 callback
cases: 101 successes and 56 semantic failures. Every Click contract has a
non-help shipping-callback success, every JSON-capable command also has a JSON
success, and no behavioral failure relies on the invented guard option.
Service, update, doctor, and provider effects are deterministic fakes; public
stores and callbacks run below fresh redacted synthetic roots. The
`hieronymus-mcp` script replay uses the real in-memory MCP boundary rather than
a startup-only fake. Legacy installed-script routing to canonical Rust
invocations remains explicit.

Evidence: `tests/compatibility/test_cli_inventory.py`; focused CLI/MCP tests
passed, including callback, semantic-failure, isolation, and byte-comparison
guards.

### 4. Python and frontend ownership

`tools/compatibility/inventory_state.py` now maps every collected public
`tests/test_service_http.py` and `tests/test_agent_hooks.py` behavior to exact
contract IDs, with false-file-coverage tests. It collects all 16 Vitest cases
through Vitest's machine-readable read-only listing and stores them in the
strict, separate `frontend_test_ownership` namespace. Three cases map to exact
public request contracts; the other 13 carry test-specific internal reasons.
Missing Bun or Vitest dependencies is an explicit validation error, never a
silent omission.

Evidence: `tests/compatibility/test_state_inventory.py`,
`tests/compatibility/test_manifest.py`, and the manifest totals of 1,306
Python nodes plus 16 frontend nodes.

### 5. Database fixtures

The deterministic `minimal-python.sqlite` fixture is populated through public
stores/APIs with strict terms and aliases, concepts and facets, crystals,
sessions, short-term memory, feedback and activations, RAG, and migration
ledgers. Frozen representative row counts are respectively 1 strict term, 1
alias, 2 concepts, 3 facets, 2 crystals, 1 session, 1 short-term memory, 1
feedback event, 1 activation, 1 RAG source/chunk, and 4 migration-ledger rows.
Five additional SQLite fixtures cover supported legacy, empty, partial,
corrupt, and unknown schemas. Each variant records read-only integrity,
foreign-key, classification, and conversion-safety expectations; only current
and supported legacy are safe to convert.

Evidence: `tools/compatibility/inventory_state.py`,
`compatibility/snapshots/state.json`, and database replay tests in
`tests/compatibility/test_state_inventory.py`.

### 6. Data-root ownership

The dedicated `data-root.layout` contract owns every retained selected path.
It names Pavel Obruchnikov as acceptance owner, `data-config` as technical
owner, `hieronymus.config:HieronymusConfig` as the real Python entry point, the
state fixture, and `crates/hiero-config/tests/data_root_contract.rs::layout` as
the explicit Rust target. Every owned-path record links back to this contract.

Evidence: `compatibility/manifest.json`, `compatibility/snapshots/state.json`,
and `test_owned_data_root_paths_link_to_one_layout_contract`.

### 7. Release implementation state and report

The strict model and schema require `last_python_release`, nullable
`first_rust_release`, and `implementation_status`. Status/release consistency
is validated independently from disposition. Every current contract honestly
remains `outstanding` with no claimed Rust release. The canonical summary and
diagnostics derive and list implemented, changed, removed, and outstanding
contract IDs and counts without hardcoded totals. The current derived totals
are 155 contracts, 0 implemented, 24 changed, 1 removed, and 155 outstanding.
`compatibility/README.md` documents these semantics and the separate frontend
ownership namespace.

Evidence: `tools/compatibility/model.py`, `tools/compatibility/check.py`,
`compatibility/manifest.schema.json`,
`compatibility/fixtures/diagnostics/check-success.txt`, and model/check tests.

## Verification

- Focused HTTP/MCP/CLI/model compatibility suite: `74 passed`.
- Real-command canonical proof after test optimization: `2 passed` (one clean
  success and one sorted composite failure); helper/family/orphan coverage is
  retained without the former repeated full generations.
- Fresh complete generation comparison: 488 artifacts were byte-identical;
  aggregate SHA-256
  `fea7c76b35eec99183e9496e0fd7fe76ef9650d84ceb137fa1bdba190bfa4fd9`.
- Canonical read-only gate:
  `uv run --no-cache --no-sync python -B -m tools.compatibility.check` passed.
- Full Python suite from a clean test-process boundary:
  `uv run --no-cache --no-sync pytest -q -x` passed, `1306 passed` in
  `563.08s`.
- Frontend: `bun run test` passed, 4 files and 16 tests; `bun run typecheck`
  passed.
- `uv run --no-cache --no-sync ruff check .` passed.
- `uv run --no-cache --no-sync ruff format --check .` passed for 182 files.
- `git diff --check` passed.
- The incidental `uv.lock` package-version rewrite was restored; the
  implementation commit contains no lock-file change.

## Unresolved concerns

None. The first attempted full-suite run was discarded because dozens of stale
pytest-owned daemon processes from old temporary runs contaminated local
service tests. After terminating only those test processes, the clean full
suite passed completely. This was an environment cleanup, not a code change.
