# Rust Qualification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce reproducible, reviewed qualification evidence for MCP transport, semantic native dependencies, frontend embedding, and legacy database import before any production Rust workspace or dependent implementation plan is started.

**Architecture:** First correct and re-review the frozen MCP 2026-07-28 authority against a byte-exact, commit-pinned copy of the official Draft 2020-12 schema, then commit that offline-validated oracle before any candidate is qualified. Four standalone Rust harness crates live under `qualification/harnesses/` and exercise only synthetic or frozen compatibility inputs in disposable work directories; native executions are explicit opt-in qualification jobs, while ordinary Python tests use fakes and the record gate is network-free. Python tooling validates a common evidence schema, renders canonical JSON into Markdown, fingerprints immutable risk-specific compatibility projections rather than mutable global ownership inventories, and computes one aggregate gate without rerunning heavy probes.

**Tech Stack:** Python 3.12 standard library, jsonschema 4.26.0 Draft 2020-12 validation, pytest, Ruff, Rust 1.96.0 on `x86_64-unknown-linux-gnu`, Cargo lockfiles, Bun 1.4.0, rmcp 3.1.4 candidate, LanceDB 0.37.1 candidate, ort 2.0.0-rc.13 candidate, rust-embed 8.12.0 candidate, rusqlite 0.40.2 candidate, SQLite FTS5.

**Spec:** `docs/superpowers/specs/2026-08-31-rust-migration-program-design.md`

## Global Constraints

- The documents under `docs/rust-migration-proposal/` remain useful analysis but are not normative.
- Produce qualification records for MCP transport, semantic native dependencies, frontend embedding, and legacy database import before writing the dependent implementation plan.
- This stage creates no production Rust workspace, production crate, daemon, migration runner, semantic index, or embedded-asset server; every Rust source file is a disposable qualification harness under `qualification/harnesses/`.
- The qualification target is exactly `x86_64-unknown-linux-gnu` with Rust `1.96.0 (ac68faa20 2026-05-25)`; a later target requires its own ADR and measured record.
- macOS and Windows are not supported by the initial cutover.
- FTS5-only operation is the required baseline for the initial Linux cutover.
- The MCP protocol revision is exactly `2026-07-28`; stdio is newline-delimited JSON-RPC, Streamable HTTP is `POST /mcp` with JSON or request-scoped SSE, and `/api/mcp/{operation}` remains an intentionally removed private Python bridge.
- MCP 2026-07-28 is stateless: there is no `initialize`, `notifications/initialized`, or transport session; every request carries exact reserved `params._meta["io.modelcontextprotocol/protocolVersion"]` and `params._meta["io.modelcontextprotocol/clientCapabilities"]`, canonical requests include SHOULD-level `params._meta["io.modelcontextprotocol/clientInfo"]` but its absence is accepted, and applicable HTTP requests carry `MCP-Protocol-Version` plus matching `Mcp-Method`. `Mcp-Name` is required only for named methods (`tools/call`, `resources/read`, and `prompts/get`): tools/list omits it, while tools/call carries `Mcp-Name: hieronymus_status`.
- The checked-in MCP authority is the byte-exact official 2026-07-28 Draft 2020-12 schema from upstream commit `271ecc9accafdd9b83a3c869fa67c22953b2af80`, SHA-256 `ef70b61f99b6d2e5e3b46863822eab08dff6a45bedc7a08914e0e5b133f40203`. Ordinary tests and both compatibility/qualification gates read only that fixture and never fetch or silently reformat it.
- Every canonical tools/list wire response contains exactly the required `cacheScope: "private"`, `ttlMs: 0`, `resultType: "complete"`, and `tools`; every tool uses wire key `inputSchema`, while semantic parity explicitly maps it to the current internal snapshot key `input_schema` without ever emitting `input_schema`. Every successful tools/call result also has `resultType: "complete"`. The compatibility authority is specifically configured to omit SHOULD-level `_meta["io.modelcontextprotocol/serverInfo"]` on every success because it is implementation-neutral and a self-reported package version would create a volatile contract unrelated to protocol behavior; the fixtures record and tests enforce that configured omission.
- A missing, unexpected, or body-mismatched `MCP-Protocol-Version`, `Mcp-Method`, or applicable `Mcp-Name` returns HTTP 400 with a JSON-RPC `HeaderMismatch` error (`-32020`). A coherent unsupported-version request carries the same unsupported value in the protocol header and body `_meta`, then returns HTTP 400 `UnsupportedProtocolVersionError` (`-32022`) with exact `requested` and `supported` data; header/body disagreement is never classified as unsupported version.
- SQLite remains authoritative for RAG sources, chunks, metadata, and semantic job state; embeddings and LanceDB tables are disposable derived artifacts.
- The semantic qualification must record exact crate versions/features, binary size, checksum-verified model load, a 10,000-chunk actual ANN index, checked series pre-filter-before-ANN plan/cardinality proof, zero cross-series hits, insert/search/delete, generation isolation, SQLite-durable lease/counter/cancellation recovery, zero SQLite-write-transaction spans across ONNX/LanceDB I/O, a complete 50-query run, and nonempty/isolation/rebuild-equivalent FTS fallback.
- Any failed semantic criterion selects `fts-only` for the initial Linux release; it never blocks the Rust workspace plan or the `x86_64-unknown-linux-gnu` release by itself.
- MCP, frontend embedding, or legacy database import failure blocks the Rust workspace/dependent plan named by the record. A failure cannot change protocol revision, remove a required transport, introduce filesystem `ServeDir`, require Bun at runtime, discard a supported database, or create a fresh sibling database beside legacy data.
- Release artifacts embed the built Svelte assets through `rust-embed`; Bun `1.4.0` is a build-time prerequisite only and is not an end-user runtime dependency.
- Existing supported databases are never opened for mutation without a successful preflight and recoverable backup.
- A failed upgrade does not leave a database marked as upgraded.
- Approved active terminology cannot be weakened by fuzzy recall or passive scoring.
- Normal mutations use one daemon-owned domain path across CLI, MCP, and web.
- Secret values do not appear in URLs, logs, discovery records, diagnostics, or frontend JSON; the `Secret<T>` type and redacted projections enforce this.
- Semantic retrieval failure degrades to FTS5 rather than failing recall.
- Every bounded background operation is resumable, auditable, or safely repeatable.
- Rust does not write source code into translation workspaces.
- Qualification inputs are checked-in synthetic fixtures or deterministic generated corpora. Harnesses must never enumerate `$HOME`, read the developer's real data root, dump the environment, or access `/home/inky/Yandex.Disk/Translation`; Rust/Cargo/Bun toolchain and package caches are the only permitted home-scoped reads and their absolute paths are normalized out of evidence.
- Canonical records contain no secrets, source-row text, hostnames, usernames, raw or percent-encoded absolute home paths, bearer or other authorization headers, cookies, launch grants, provider keys, passwords, private keys, or raw process logs. Structured redaction parses canonical JSON and recursively inspects every mapping key after exact snake/camel normalization, regardless of whether its value is a string, number, boolean, null, or scalar array; the same non-echoing policy scans completed Markdown. It covers exact password/private-key/auth/cookie, memory/source text, raw-log, stdout, and stderr names, and raw PEM private-key blocks.
- Network access is permitted only for Task 1's named official-spec review and one-time retrieval of its commit-pinned schema bytes, plus the explicit dependency-fetch, Bun-install, and checksum-verified model/runtime-acquisition steps. Ordinary compatibility tests, record validation, and every replay after acquisition run offline and make no network request.
- Recorded replay commands use one closed grammar of exact offline qualification/checker entry points and bounded Cargo/Bun forms. Shell expansion, substitution, globbing, redirection, control/metacharacters, URLs, and network-capable acquisition commands are invalid even when a shell could parse them.
- Ordinary `uv run pytest` tests inject bounded fake executables and never require Cargo, Rust artifacts, Bun packages, ONNX Runtime, a model, or a native-library cache. Live Rust/native qualification is explicit through `HIERONYMUS_QUALIFICATION_LIVE=1` commands and its dedicated workflow only.
- Every live run writes beneath `qualification/.artifacts/`, hashes frozen inputs before and after use, removes transient work/log/install directories on success or failure, and leaves only ignored caches plus reviewed records.
- Every Cargo build/test/Clippy command sets a risk-specific `CARGO_TARGET_DIR` beneath `qualification/.artifacts/cargo-target/`; crash probes disable core dumps and run in an owned process group that the runner terminates and reaps in `finally`.
- Tool discovery occurs against the original environment before HOME/XDG sanitization. It preserves the lexical cargo/rustup invocation paths separately from their resolved executable targets; every live runner invokes the preserved cargo shim, passes the resulting `ToolRoots` and bounded target to `safe_subprocess_env`, and uses that one safe environment for all Cargo/Bun/native children without serializing absolute tool paths.
- Each harness has its own `Cargo.toml` and committed `Cargo.lock`; direct risk dependencies are exact pins and all resolved versions/features are copied from `cargo metadata --locked` and `cargo tree -e features --locked` into the record.
- Each record's input digest covers `qualification/prerequisites.json`, `qualification/rust-toolchain.toml`, every common qualification module (`model.py`, `fingerprint.py`, `projections.py`, `redaction.py`, `validate.py`, `render.py`, `acquire.py`, `process.py`, and `clean.py`), its own Python runner, Cargo manifest/lockfile/source/tests, every directly consumed immutable compatibility snapshot/fixture including every actual HTTP route case, and risk-specific frontend/corpus inputs. The MCP record additionally covers the official schema bytes and pin metadata plus `qualification/compatibility/mcp-transport.json`; the database record covers `qualification/compatibility/legacy-database-import.json`. Neither record fingerprints mutable whole `compatibility/manifest.json` or `compatibility/snapshots/state.json`.
- Task 3 owns the two checked-in risk projections and derives them without semantic normalization from the current global manifest/state. Ordinary validation is offline and read-only: a selected source-field change or checked-in projection drift blocks the appropriate risk, while unrelated contract surfaces and generated test-node inventory changes are ignored. Later ledger-required regeneration of the global ownership files therefore does not stale an already measured record; changing a relevant projection changes its fingerprint and requires rerunning only that risk's live measurement.
- Pavel Obruchnikov `<me@inkyquill.net>` is the acceptance owner for all four records and the aggregate gate unless a compatibility-manifest entry explicitly delegates another named owner.

## Normative Inputs

- `https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/schema/2026-07-28/schema.json` (official MCP schema reviewed and pinned by Task 1; ordinary replay uses the checked-in exact bytes)
- `https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/transports/streamable-http.mdx` (official Streamable HTTP rules reviewed only by Task 1)
- `docs/adr/0013-semantic-index-and-platform-support.md`
- `docs/adr/0015-mcp-protocol-and-transport.md`
- `docs/adr/0010-data-locations-schema-ownership-and-upgrade.md`
- `docs/adr/0014-web-console-replaces-terminal-ui.md`
- `docs/superpowers/specs/2026-08-31-rust-semantic-rag-design.md`
- `docs/superpowers/specs/2026-08-31-rust-daemon-mcp-security-design.md`
- `docs/superpowers/specs/2026-08-31-rust-database-upgrade-design.md`
- `docs/superpowers/specs/2026-08-31-rust-distribution-cutover-design.md`
- `docs/superpowers/specs/2026-08-31-rust-compatibility-contracts-design.md`
- `compatibility/README.md`, mutable ownership source `compatibility/manifest.json`, mutable state source `compatibility/snapshots/state.json`, and the frozen fixtures named by each task. The two mutable sources are used only to validate Task 3's risk projections and are never record fingerprint inputs.

## Outcome Rules

| Risk | Passing decision | Failing decision | Aggregate consequence |
|---|---|---|---|
| MCP transport | `qualified` | `blocked` | Blocks `rust-workspace-and-contract-harness` and `rust-daemon-mcp-security`; ADR 0015 remains unchanged. |
| Semantic native dependencies | `semantic-enabled` | `fts-only` | Never blocks the aggregate gate; selects semantic-enabled or FTS5-only Linux mode. |
| Frontend embedding | `qualified` | `blocked` | Blocks `rust-workspace-and-contract-harness`, `rust-frontend`, and `rust-distribution-cutover`; ADR 0014 remains unchanged. |
| Legacy database import | `qualified` | `blocked` | Blocks `rust-workspace-and-contract-harness`, `rust-database-upgrade`, and `rust-distribution-cutover`; ADR 0010 and the database-upgrade spec remain unchanged. |

Missing, stale, malformed, partially executed, or unreviewed evidence is blocking. A blocking result is a valid qualification record but is not permission to write its dependent implementation plan.

## Execution And File Ownership Order

- Execute Tasks 1–21 in numeric order. Task 1's corrected compatibility oracle must be accepted as its own commit before Task 6 starts.
- Tasks 2–5 establish, in separate commits, the record model, validation/rendering/fingerprints and immutable compatibility projections, verified acquisitions, and sanitized process/cleanup boundary. Task 3 alone owns projection generation; Tasks 8 and 18 consume the appropriate checked-in projection, Task 19 validates both against their mutable sources, and later inventory regeneration must not rewrite projections for unrelated changes. Task 9 is the only later task allowed to extend `qualification/prerequisites.json`, `tools/qualification/acquire.py`, or `tests/qualification/test_acquire.py`.
- MCP Tasks 6–8, semantic Tasks 9–12, frontend Tasks 13–15, and database Tasks 16–18 each use manifest/build, behavior/recovery, then runner/evidence commits. Within a risk, later tasks modify only files explicitly handed off by the prior task.
- Task 19 owns dispatcher/checker/gate computation, Task 20 owns acceptance transitions and generated artifact refresh, and Task 21 alone owns contributor commands and CI workflows. Common-file ownership is sequential; do not implement tasks concurrently when they name the same file, and never fold an earlier review boundary into a later commit.

## File Map

- `qualification/README.md`: acquisition, offline replay, record refresh, cleanup, and decision rules.
- `qualification/prerequisites.json`: exact target/toolchain/Bun/model identity and allowed network acquisition sources.
- `qualification/rust-toolchain.toml`: qualification-only Rust 1.96.0 pin.
- `qualification/schemas/record.schema.json`: machine-readable contract for one risk record.
- `qualification/schemas/gate.schema.json`: machine-readable aggregate-gate contract.
- `qualification/fixtures/semantic-corpus.json`: deterministic 10,000-chunk/50-query corpus recipe.
- `qualification/compatibility/mcp-transport.json`: immutable canonical projection of only the MCP-relevant compatibility manifest contracts.
- `qualification/compatibility/legacy-database-import.json`: immutable canonical projection of only the database-relevant compatibility contracts and consumed database state.
- `qualification/harnesses/mcp-transport/`: standalone rmcp transport probe and lockfile.
- `qualification/harnesses/semantic-native/`: standalone ONNX/LanceDB/FTS probe and lockfile.
- `qualification/harnesses/frontend-embedding/`: standalone rust-embed probe and lockfile.
- `qualification/harnesses/legacy-database-import/`: standalone read-only SQLite import probe and lockfile.
- `tools/compatibility/inventory_mcp.py`: generator for the corrected official MCP 2026-07-28 protocol oracle.
- `tools/compatibility/inventory_http.py`: generator for MCP Host/auth/version/method/name HTTP route cases.
- `compatibility/authorities/mcp/2026-07-28/schema.json`: byte-exact, commit-pinned official Draft 2020-12 schema used offline.
- `compatibility/authorities/mcp/2026-07-28/schema.source.json`: immutable upstream URL/commit/SHA-256 metadata for the schema bytes.
- `tools/compatibility/mcp_schema.py`: offline pin verification and named official-schema definition validation.
- `qualification/records/*.json`: canonical machine evidence for four risks plus the aggregate gate.
- `docs/qualification/rust/*.md`: generated human-readable records; never hand-edited independently of JSON.
- `tools/qualification/model.py`: typed records, required-criterion sets, and consequence constants.
- `tools/qualification/fingerprint.py`: deterministic relative-path input hashing and stale-record detection.
- `tools/qualification/projections.py`: deterministic risk-projection builder plus offline, read-only drift validation against the current manifest/state sources.
- `tools/qualification/redaction.py`: canonical-record forbidden-data scan.
- `tools/qualification/validate.py`: network-free record/schema/invariant validation CLI.
- `tools/qualification/render.py`: deterministic Markdown renderer and drift checker.
- `tools/qualification/acquire.py`: explicit atomic semantic-model and ONNX-runtime acquisition with checksum and archive-policy verification.
- `tools/qualification/clean.py`: bounded cleanup for ignored qualification artifacts only.
- `tools/qualification/process.py`: owned process-group execution, core-dump suppression, timeouts, and bounded termination/reaping.
- `tests/qualification/fixtures/process-smoke/`: dependency-free locked Cargo project proving the sanitized Rust environment works offline.
- `tools/qualification/run_mcp.py`: MCP harness orchestration and record production.
- `tools/qualification/run_semantic.py`: semantic build/recovery/fallback orchestration and record production.
- `tools/qualification/run_frontend.py`: Bun build/rust-embed orchestration and record production.
- `tools/qualification/run_database.py`: read-only fixture import orchestration and record production.
- `tools/qualification/run.py`: one CLI dispatcher for individual or all live qualifications.
- `tools/qualification/check.py`: network-free schema, fingerprint, Markdown, redaction, and aggregate-gate validation.
- `tools/qualification/review.py`: review-only state transition and deterministic record/aggregate regeneration.
- `tests/qualification/factories.py`: deterministic valid/failed/stale/review-state record factories and bounded fake executable writer used only by tests.
- `tests/qualification/`: Python tests for record invariants, runners, cleanup safety, and gate truth table.
- `.github/workflows/pr.yml`: ordinary network-free qualification-record drift gate.
- `.github/workflows/rust-qualification-live.yml`: manually dispatched, pinned-action live evidence workflow; it never commits records.
- `.gitignore`: ignores model, work, install, log, and Cargo target artifacts under `qualification/.artifacts/`.

---

### Task 1: Correct And Re-Review The Official MCP 2026-07-28 Authority

**Complexity:** High, 3–4 hours.

**Files:**
- Modify: `pyproject.toml`
- Modify: `uv.lock`
- Create: `compatibility/authorities/mcp/2026-07-28/schema.json`
- Create: `compatibility/authorities/mcp/2026-07-28/schema.source.json`
- Create: `tools/compatibility/mcp_schema.py`
- Modify: `tools/compatibility/inventory_mcp.py`
- Modify: `tools/compatibility/inventory_http.py`
- Modify: `tools/compatibility/check.py`
- Modify: `tests/compatibility/test_mcp_inventory.py`
- Modify: `tests/compatibility/test_http_inventory.py`
- Modify: `tests/compatibility/test_check.py`
- Modify: `compatibility/fixtures/mcp/protocol.json`
- Modify: `compatibility/fixtures/http/route-cases.json`
- Modify: `compatibility/snapshots/state.json`
- Modify: `compatibility/manifest.json`
- Modify: `compatibility/fixtures/diagnostics/check-success.txt`

**Interfaces:**
- Consumes: the official MCP 2026-07-28 schema at upstream commit `271ecc9accafdd9b83a3c869fa67c22953b2af80`, the official Streamable HTTP rules, and ADR 0015's transport/auth/Host requirements.
- Produces immutable authority constants `OFFICIAL_SCHEMA_SOURCE: dict[str, object]` and `OFFICIAL_SCHEMA_SHA256: str`: upstream URL `https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/271ecc9accafdd9b83a3c869fa67c22953b2af80/schema/2026-07-28/schema.json`, SHA-256 `ef70b61f99b6d2e5e3b46863822eab08dff6a45bedc7a08914e0e5b133f40203`, and Draft 2020-12 dialect. Schema bytes are copied exactly once and are never generated, normalized, or fetched by tests/checks.
- Produces offline helpers in `tools.compatibility.mcp_schema`: `authority_issues(repo_root: Path) -> tuple[str, ...]` and `definition_issues(repo_root: Path, definition: Literal["ListToolsResultResponse", "CallToolResultResponse", "HeaderMismatchError", "UnsupportedProtocolVersionError"], instance: object) -> tuple[str, ...]`. Both verify the pin first; definition validation uses `jsonschema.Draft202012Validator` and deterministic sorted diagnostics.
- Produces: `_wire_tool(tool: dict[str, object]) -> dict[str, object]`, which maps current internal `input_schema` to wire `inputSchema` and emits only `name`, `description`, and `inputSchema` for the current snapshot shape.
- Produces constant: `RESPONSE_METADATA_RULES: dict[str, object]`, the exact configured serverInfo omission copied into both target fixtures.
- Produces: `_protocol_fixture(snapshot: dict[str, object]) -> dict[str, object]` whose target requests carry exact reserved `params._meta` keys, whose tools/list response has exact required `cacheScope: "private"`, `ttlMs: 0`, `resultType: "complete"`, and `tools`, whose every successful envelope has `resultType: "complete"`, and whose stdio/HTTP JSON/SSE variants contain no initialize/session fields or internal `input_schema` wire keys.
- Test helper: `_successful_result_envelopes(target: dict[str, object]) -> tuple[dict[str, object], ...]` returns tools/list and tools/call success envelopes from stdio, HTTP JSON, and HTTP SSE fixture branches.
- Produces validator: `_request_metadata_issues(request: dict[str, object]) -> tuple[str, ...]`, accepting absence of only clientInfo while rejecting missing/wrong required reserved keys and every direct legacy metadata field.
- Produces configured response metadata policy in both target fixtures: `io.modelcontextprotocol/serverInfo` has `configured: "omit"` and the exact implementation-neutral/volatile-version rationale below; every success omits `_meta` and tests prove this is the configured exception to the SHOULD.
- Produces: the `http.route.post.mcp` target cases in `_route_cases(snapshot: dict[str, object])`, with two successes and exactly eleven failures. Seven header failures return HTTP 400 `HeaderMismatch` (`-32020`); `protocol-version-header-mismatch` changes only the raw header and is classified as header mismatch. The distinct `unsupported-version` changes one semantic protocol-version field represented in both mirrored wire locations, keeps those values equal, and returns HTTP 400 `UnsupportedProtocolVersionError` (`-32022`) with exact requested/supported data. Every other failure mutates or omits one applicable raw request field.
- Produces constant: `HEADER_MISMATCH_MESSAGES: dict[str, str]`, the exact seven-case message mapping used by the generator and whole-body tests.
- Test helper: `_changed_request_leaf_paths(reference: dict[str, object], candidate: dict[str, object]) -> set[tuple[str, ...]]` reports exact changed/omitted request leaves for the one-raw-field invariant.
- Ownership: the two new pytest node ids are implementation-internal authority/gate checks, so `inventory_state --write` refreshes `compatibility/snapshots/state.json`, `compatibility/manifest.json`, and the diagnostic summary; no new public contract id is added.
- Produces: a separately reviewed compatibility commit that is an immutable prerequisite of Tasks 6–8; candidate qualification must not edit or normalize this oracle.

- [ ] **Step 1: Pin the exact official schema and its explicit validator dependency**

Add exact dev dependency `jsonschema==4.26.0` to `pyproject.toml`; it is already present transitively in the current lock, but Task 1 makes the compatibility validator's ownership direct. Refresh only dependency metadata offline:

```bash
uv lock --offline
```

Retrieve the schema only in this named authority step and preserve the response bytes exactly:

```bash
curl --fail --silent --show-error --location \
  https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/271ecc9accafdd9b83a3c869fa67c22953b2af80/schema/2026-07-28/schema.json \
  --output compatibility/authorities/mcp/2026-07-28/schema.json
sha256sum compatibility/authorities/mcp/2026-07-28/schema.json
```

Expected SHA-256: `ef70b61f99b6d2e5e3b46863822eab08dff6a45bedc7a08914e0e5b133f40203`. Create `schema.source.json` exactly as:

```json
{
  "schema_version": 1,
  "protocol_revision": "2026-07-28",
  "draft": "https://json-schema.org/draft/2020-12/schema",
  "upstream_commit": "271ecc9accafdd9b83a3c869fa67c22953b2af80",
  "upstream_url": "https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/271ecc9accafdd9b83a3c869fa67c22953b2af80/schema/2026-07-28/schema.json",
  "sha256": "ef70b61f99b6d2e5e3b46863822eab08dff6a45bedc7a08914e0e5b133f40203"
}
```

Never run a JSON formatter over `schema.json`; byte identity, not semantic reserialization, is the pin.

- [ ] **Step 2: Replace shallow fixture assertions with failing official-schema and exact-wire assertions**

```python
def test_official_mcp_schema_pin_and_target_envelopes() -> None:
    protocol = json.loads(
        (ROOT / "compatibility/fixtures/mcp/protocol.json").read_text(encoding="utf-8")
    )
    route_cases = json.loads(
        (ROOT / "compatibility/fixtures/http/route-cases.json").read_text(encoding="utf-8")
    )
    route_target = next(
        route["target"]
        for route in route_cases["routes"]
        if route["contract_id"] == "http.route.post.mcp"
    )
    source = json.loads(
        (ROOT / "compatibility/authorities/mcp/2026-07-28/schema.source.json")
        .read_text(encoding="utf-8")
    )
    schema_bytes = (
        ROOT / "compatibility/authorities/mcp/2026-07-28/schema.json"
    ).read_bytes()
    assert source == OFFICIAL_SCHEMA_SOURCE
    assert hashlib.sha256(schema_bytes).hexdigest() == OFFICIAL_SCHEMA_SHA256
    assert json.loads(schema_bytes)["$schema"] == (
        "https://json-schema.org/draft/2020-12/schema"
    )
    assert authority_issues(ROOT) == ()

    protocol_envelopes = _successful_result_envelopes(protocol["target"])
    route_envelopes = tuple(
        item["response"]["body"] for item in route_target["successes"]
    )
    assert len(protocol_envelopes) == 6
    assert len(route_envelopes) == 2
    for envelope in (*protocol_envelopes, *route_envelopes):
        definition = (
            "ListToolsResultResponse"
            if envelope["id"] == 1
            else "CallToolResultResponse"
        )
        assert definition_issues(ROOT, definition, envelope) == ()
        assert envelope["result"]["resultType"] == "complete"
        assert "_meta" not in envelope["result"]

    list_results = [
        envelope["result"]
        for envelope in (*protocol_envelopes, *route_envelopes)
        if envelope["id"] == 1
    ]
    assert len(list_results) == 4
    assert all(set(result) == {"cacheScope", "resultType", "tools", "ttlMs"} for result in list_results)
    assert all(result["cacheScope"] == "private" for result in list_results)
    assert all(result["ttlMs"] == 0 for result in list_results)
    assert all(
        set(tool) == {"description", "inputSchema", "name"}
        and "input_schema" not in tool
        for tool in protocol["target"]["tools_list"]["response"]["result"]["tools"]
    )

    snapshot_registry = {
        tool["name"]: tool["input_schema"] for tool in snapshot_mcp()["tools"]
    }
    target_registry = {
        tool["name"]: tool["inputSchema"]
        for tool in protocol["target"]["tools_list"]["response"]["result"]["tools"]
    }
    assert target_registry == snapshot_registry
    assert protocol["target"]["response_metadata_rules"] == RESPONSE_METADATA_RULES
    assert route_target["response_metadata_rules"] == RESPONSE_METADATA_RULES


def test_protocol_fixture_is_stateless_2026_07_28() -> None:
    protocol = json.loads(
        (ROOT / "compatibility/fixtures/mcp/protocol.json").read_text(encoding="utf-8")
    )
    target = protocol["target"]
    compact = json.dumps(protocol, sort_keys=True, separators=(",", ":"))
    assert '"initialize":' not in compact
    assert "notifications/initialized" not in compact
    assert "Mcp-Session-Id" not in compact
    assert {"initialize", "initialized", "session"}.isdisjoint(target)
    for request in target["requests"]:
        assert {"protocolVersion", "clientCapabilities", "clientInfo"}.isdisjoint(
            request["params"]
        )
        meta = request["params"]["_meta"]
        assert meta["io.modelcontextprotocol/protocolVersion"] == "2026-07-28"
        assert meta["io.modelcontextprotocol/clientCapabilities"] == {}
        assert meta["io.modelcontextprotocol/clientInfo"] == {
            "name": "compatibility-replay",
            "version": "1.0.0",
        }
    assert target["metadata_rules"] == {
        "required": [
            "io.modelcontextprotocol/protocolVersion",
            "io.modelcontextprotocol/clientCapabilities",
        ],
        "should": ["io.modelcontextprotocol/clientInfo"],
    }
    http_exchanges = {
        exchange["request"]["body"]["method"]: exchange
        for exchange in target["streamable_http"]["exchanges"]
    }
    assert set(http_exchanges) == {"tools/list", "tools/call"}
    list_headers = http_exchanges["tools/list"]["request"]["headers"]
    assert list_headers["Mcp-Method"] == "tools/list"
    assert "Mcp-Name" not in list_headers
    call_headers = http_exchanges["tools/call"]["request"]["headers"]
    assert call_headers["Mcp-Method"] == "tools/call"
    assert call_headers["Mcp-Name"] == "hieronymus_status"
    assert all(
        "Mcp-Session-Id" not in exchange["request"]["headers"]
        for exchange in http_exchanges.values()
    )
    without_client_info = json.loads(json.dumps(target["requests"][0]))
    del without_client_info["params"]["_meta"][
        "io.modelcontextprotocol/clientInfo"
    ]
    assert _request_metadata_issues(without_client_info) == ()
    missing_capabilities = json.loads(json.dumps(without_client_info))
    del missing_capabilities["params"]["_meta"][
        "io.modelcontextprotocol/clientCapabilities"
    ]
    assert _request_metadata_issues(missing_capabilities)
    direct_legacy = json.loads(json.dumps(without_client_info))
    direct_legacy["params"]["protocolVersion"] = "2026-07-28"
    assert _request_metadata_issues(direct_legacy)


def test_http_mcp_cases_cover_official_metadata_and_local_security() -> None:
    case = _route_cases_by_id()["http.route.post.mcp"]["target"]
    successes = {item["id"]: item["request"] for item in case["successes"]}
    tools_list = successes["tools-list"]
    assert tools_list["headers"]["Mcp-Method"] == "tools/list"
    assert "Mcp-Name" not in tools_list["headers"]
    assert tools_list["body"]["method"] == "tools/list"
    tools_call = successes["tools-call"]
    assert tools_call["headers"]["Mcp-Method"] == "tools/call"
    assert tools_call["headers"]["Mcp-Name"] == "hieronymus_status"
    assert tools_call["body"]["method"] == "tools/call"
    assert tools_call["body"]["params"]["name"] == "hieronymus_status"
    meta = tools_call["body"]["params"]["_meta"]
    assert meta["io.modelcontextprotocol/protocolVersion"] == "2026-07-28"
    assert meta["io.modelcontextprotocol/clientCapabilities"] == {}
    assert meta["io.modelcontextprotocol/clientInfo"] == {
        "name": "compatibility-replay",
        "version": "1.0.0",
    }
    failures = {failure["id"]: failure for failure in case["failures"]}
    assert len(failures) == 11
    assert set(failures) == {
        "invalid-host",
        "missing-bearer",
        "invalid-bearer",
        "missing-version",
        "protocol-version-header-mismatch",
        "unsupported-version",
        "missing-mcp-method",
        "wrong-mcp-method",
        "unexpected-mcp-name-tools-list",
        "missing-mcp-name-tools-call",
        "wrong-mcp-name-tools-call",
    }
    header_mismatch_ids = {
        "missing-version",
        "protocol-version-header-mismatch",
        "missing-mcp-method",
        "wrong-mcp-method",
        "unexpected-mcp-name-tools-list",
        "missing-mcp-name-tools-call",
        "wrong-mcp-name-tools-call",
    }
    for failure_id in header_mismatch_ids:
        failure = failures[failure_id]
        assert failure["response"]["status"] == 400
        assert failure["response"]["body"] == {
            "jsonrpc": "2.0",
            "id": failure["request"]["body"]["id"],
            "error": {
                "code": -32020,
                "message": HEADER_MISMATCH_MESSAGES[failure_id],
            },
        }
        assert definition_issues(
            ROOT, "HeaderMismatchError", failure["response"]["body"]
        ) == ()

    mismatch = failures["protocol-version-header-mismatch"]
    assert mismatch["request"]["headers"]["MCP-Protocol-Version"] == "2025-06-18"
    assert mismatch["request"]["body"]["params"]["_meta"][
        "io.modelcontextprotocol/protocolVersion"
    ] == "2026-07-28"
    unsupported = failures["unsupported-version"]
    assert unsupported["request"]["headers"]["MCP-Protocol-Version"] == "2025-06-18"
    assert unsupported["request"]["body"]["params"]["_meta"][
        "io.modelcontextprotocol/protocolVersion"
    ] == "2025-06-18"
    assert unsupported["response"] == {
        "status": 400,
        "headers": {"Content-Type": "application/json; charset=utf-8"},
        "body": {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32022,
                "message": "Unsupported protocol version: 2025-06-18",
                "data": {
                    "requested": "2025-06-18",
                    "supported": ["2026-07-28"],
                },
            },
        },
    }
    assert definition_issues(
        ROOT, "UnsupportedProtocolVersionError", unsupported["response"]["body"]
    ) == ()
    assert _changed_request_leaf_paths(tools_list, unsupported["request"]) == {
        ("body", "params", "_meta", "io.modelcontextprotocol/protocolVersion"),
        ("headers", "MCP-Protocol-Version"),
    }
    for failure_id, failure in failures.items():
        if failure_id == "unsupported-version":
            continue
        base = tools_call if failure_id.endswith("tools-call") else tools_list
        assert len(_changed_request_leaf_paths(base, failure["request"])) == 1
```

- [ ] **Step 3: Run the focused tests and verify RED against the invalid response/error oracle**

Run: `uv run pytest tests/compatibility/test_mcp_inventory.py::test_official_mcp_schema_pin_and_target_envelopes tests/compatibility/test_mcp_inventory.py::test_protocol_fixture_is_stateless_2026_07_28 tests/compatibility/test_http_inventory.py::test_http_mcp_cases_cover_official_metadata_and_local_security -v`

Expected: FAIL because all four frozen tools/list envelopes omit `cacheScope`/`ttlMs`, the 39 protocol tools emit internal `input_schema` instead of wire `inputSchema`, header failures are local non-JSON-RPC objects, the current `unsupported-version` is really a one-header mismatch, there is no distinct coherent unsupported-version case, and response-side serverInfo omission is not configured or tested.

- [ ] **Step 4: Implement offline Draft 2020-12 validation and corrected success wire objects**

`tools.compatibility.mcp_schema` reads only the two checked-in authority files. `authority_issues` requires the exact metadata object from Step 1, hashes raw `schema.json` bytes, parses them only after the hash matches, requires the exact `$schema` dialect, and runs `Draft202012Validator.check_schema`. `definition_issues` builds a validator schema by copying the full official schema and adding top-level `$ref: "#/$defs/<definition>"`; it sorts errors by absolute instance path, schema path, and message. Unknown definition names are rejected before validation. No helper imports an HTTP client or accepts a URL/path override.

The compatibility gate calls `authority_issues` before inventory generation and validates the generated in-memory protocol and route envelopes, not only checked-in fixture bytes: all four tools/list and four tools/call canonical envelopes use their named response definitions, the seven header error bodies use `HeaderMismatchError`, and the coherent unsupported body uses `UnsupportedProtocolVersionError`. Add `test_compatibility_gate_validates_official_mcp_schema_offline`; monkeypatch socket/URL open primitives to fail, wrap `_protocol_fixture` so one generated in-memory canonical list response loses `ttlMs`, run the MCP inventory validation, and require a deterministic official-schema failure rather than a fetch.

Define target request metadata exactly as:

```python
REQUEST_META = {
    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
    "io.modelcontextprotocol/clientCapabilities": {},
    "io.modelcontextprotocol/clientInfo": {
        "name": "compatibility-replay",
        "version": "1.0.0",
    },
}

tools_list_request = {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/list",
    "params": {"_meta": REQUEST_META},
}
tool_call_request = {
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
        "_meta": REQUEST_META,
        "name": "hieronymus_status",
        "arguments": {},
    },
}
```

Store those requests in list/call order and freeze `metadata_rules.required` to protocolVersion/clientCapabilities while `metadata_rules.should` contains clientInfo. Implement `_request_metadata_issues` from those exact rules. Canonical requests include clientInfo, but a request omitting only `io.modelcontextprotocol/clientInfo` remains valid; missing either required key or placing any metadata directly in `params` is invalid.

Map, do not rename in place, the internal registry:

```python
def _wire_tool(tool: dict[str, object]) -> dict[str, object]:
    return {
        "name": tool["name"],
        "description": tool["description"],
        "inputSchema": copy.deepcopy(tool["input_schema"]),
    }


tools_list_result = {
    "jsonrpc": "2.0",
    "id": 1,
    "result": {
        "cacheScope": "private",
        "ttlMs": 0,
        "resultType": "complete",
        "tools": [_wire_tool(tool) for tool in tools],
    },
}
```

The current Python response remains byte-faithful under `protocol["current"]`; only the target wire projection changes. Semantic parity compares current `inputSchema`, target `inputSchema`, and snapshot-internal `input_schema` by explicit comprehensions. Assert `input_schema` is absent from every target tool. Use `cacheScope: "private"` because the endpoint is bearer-scoped and `ttlMs: 0` so the compatibility oracle creates no stale-registry guarantee.

Record this exact configured omission in both target fixtures and require every success to omit `_meta`:

```python
RESPONSE_METADATA_RULES = {
    "io.modelcontextprotocol/serverInfo": {
        "configured": "omit",
        "rationale": (
            "The compatibility oracle is implementation-neutral; freezing self-reported "
            "package identity would create a volatile version contract unrelated to "
            "protocol behavior."
        ),
    }
}
```

Store both operations as newline-delimited stdio exchanges and as Streamable HTTP JSON/SSE exchanges; `_successful_result_envelopes(target)` enumerates all six successful envelopes. Both HTTP requests include Host, bearer, `MCP-Protocol-Version`, and matching `Mcp-Method`. The tools/list exchange omits `Mcp-Name`; tools/call includes `Mcp-Name: hieronymus_status`. The route-level tools/list success independently freezes the same four required fields, with an empty `tools` array, and tools/call remains a valid `CallToolResultResponse`. Delete handshake/session shapes from the entire protocol fixture. Qualification consumes only target.

- [ ] **Step 5: Implement exact HeaderMismatch and coherent unsupported-version route cases**

Keep Host and bearer failures under their existing local security contracts. Replace only the seven protocol-header failures with whole JSON-RPC bodies shaped as:

```python
{
    "jsonrpc": "2.0",
    "id": request_body["id"],
    "error": {"code": -32020, "message": HEADER_MISMATCH_MESSAGES[failure_id]},
}
```

Freeze these exact deterministic messages:

```python
HEADER_MISMATCH_MESSAGES = {
    "missing-version": "Header mismatch: required MCP-Protocol-Version header is missing",
    "protocol-version-header-mismatch": (
        "Header mismatch: MCP-Protocol-Version header value '2025-06-18' "
        "does not match body value '2026-07-28'"
    ),
    "missing-mcp-method": "Header mismatch: required Mcp-Method header is missing",
    "wrong-mcp-method": (
        "Header mismatch: Mcp-Method header value 'tools/call' "
        "does not match body value 'tools/list'"
    ),
    "unexpected-mcp-name-tools-list": (
        "Header mismatch: Mcp-Name header must be omitted for tools/list"
    ),
    "missing-mcp-name-tools-call": (
        "Header mismatch: required Mcp-Name header is missing for tools/call"
    ),
    "wrong-mcp-name-tools-call": (
        "Header mismatch: Mcp-Name header value 'hieronymus_recall' "
        "does not match body value 'hieronymus_status'"
    ),
}
```

Rename the old one-header `unsupported-version` mutation to `protocol-version-header-mismatch`; it changes only `headers["MCP-Protocol-Version"]` to `2025-06-18` and returns `-32020`. Add a new `unsupported-version` derived from tools/list whose header and `params._meta["io.modelcontextprotocol/protocolVersion"]` both equal `2025-06-18`. Its exact response is the object in Step 2: HTTP 400, JSON-RPC id `1`, code `-32022`, message `Unsupported protocol version: 2025-06-18`, and data `{"requested": "2025-06-18", "supported": ["2026-07-28"]}`.

Validation order is normative for Task 7: first compare required mirrored headers/body and emit `HeaderMismatch`; only after those values agree check server support and emit `UnsupportedProtocolVersionError`. The coherent unsupported case is the sole raw two-location exception because the two locations represent one semantic field. `_changed_request_leaf_paths` proves its exact pair and proves each of the other ten failures changes/omits exactly one applicable raw leaf relative to its tools/list or tools/call success.

- [ ] **Step 6: Regenerate ownership and prove the authority is complete and deterministic**

Run: `uv run python -m tools.compatibility.inventory_mcp --write`

Run: `uv run python -m tools.compatibility.inventory_http --write`

Run: `uv run python -m tools.compatibility.inventory_state --write`

Run: `uv run pytest tests/compatibility/test_mcp_inventory.py tests/compatibility/test_http_inventory.py tests/compatibility/test_check.py -v`

Expected: PASS without network; the authority pin/dialect/hash match, every generated and checked-in canonical envelope validates under its exact official definition, target requests use only exact reserved `_meta` keys, semantic registry parity maps internal `input_schema` to wire `inputSchema`, all four list results carry the exact four required fields, configured serverInfo omission is explicit on all eight successes, and the eleven HTTP failures have exact ids, mutations, status, bodies, and error classifications.

`inventory_state --write` adds exactly these two new implementation-internal ownership nodes and refreshes only the derived state snapshot/manifest/diagnostic bytes beyond the MCP/HTTP fixtures:

```text
tests/compatibility/test_mcp_inventory.py::test_official_mcp_schema_pin_and_target_envelopes
tests/compatibility/test_check.py::test_compatibility_gate_validates_official_mcp_schema_offline
```

The public contract count and contract ids remain unchanged; the schema authority itself is not a product surface contract.

- [ ] **Step 7: Run the offline compatibility gate before any candidate work**

Run: `uv run --no-cache --no-sync python -B -m tools.compatibility.check`

Expected: exit `0` with no network and no authority, official-schema, snapshot, fixture, manifest, ownership, or route-case drift. Corrupting the authority bytes, their metadata, a generated `inputSchema`, a required list field, a HeaderMismatch body, or unsupported-version data makes the gate fail deterministically.

- [ ] **Step 8: Commit the corrected oracle as the candidate-qualification prerequisite**

```bash
git add pyproject.toml uv.lock compatibility/authorities/mcp/2026-07-28/schema.json compatibility/authorities/mcp/2026-07-28/schema.source.json tools/compatibility/mcp_schema.py tools/compatibility/inventory_mcp.py tools/compatibility/inventory_http.py tools/compatibility/check.py tests/compatibility/test_mcp_inventory.py tests/compatibility/test_http_inventory.py tests/compatibility/test_check.py compatibility/fixtures/mcp/protocol.json compatibility/fixtures/http/route-cases.json compatibility/snapshots/state.json compatibility/manifest.json compatibility/fixtures/diagnostics/check-success.txt
git commit -m "fix: align MCP compatibility oracle with 2026-07-28"
```

Stop if this commit is not accepted. Tasks 6–8 consume the corrected commit, and Task 3 derives the immutable MCP compatibility projection from the accepted manifest entries before Task 8 fingerprints that projection, the authority bytes, pin metadata, schema-validated protocol/route fixtures, and internal snapshot used for semantic comparison. Task 8 never fingerprints the whole mutable manifest; candidate work must never fetch a schema, preserve `input_schema` on the wire, normalize the authority bytes, or replay the obsolete target fixture.

### Task 2: Common Qualification Record Schema And Decision Model

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `qualification/schemas/record.schema.json`
- Create: `tools/qualification/__init__.py`
- Create: `tools/qualification/model.py`
- Create: `tests/qualification/factories.py`
- Create: `tests/qualification/test_model.py`

**Interfaces:**
- Consumes: the four normative risk names, criterion sets, decisions, and accepted consequences in this plan.
- Produces: `load_record(path: Path) -> QualificationRecord`.
- Produces: `decision_for(risk: Risk, evidence: tuple[Evidence, ...]) -> Decision`, `status_for(evidence: tuple[Evidence, ...]) -> Literal["pass", "fail"]`, and `expected_consequence(risk: Risk, status: Literal["pass", "fail"]) -> str`.
- Test-only factory: `make_record(repo_root: Path, risk: Risk, *, failed: tuple[str, ...] = (), review_status: ReviewStatus = "pending") -> QualificationRecord`; Task 3 supplies its current input digest.
- Test-only factory: `accepted_records(repo_root: Path, *, semantic: Literal["semantic-enabled", "fts-only"]) -> dict[Risk, QualificationRecord]`, `accepted_failure(repo_root: Path, risk: Risk) -> QualificationRecord`, `pending_review(record: QualificationRecord) -> QualificationRecord`, `write_fake_executable(tmp_path: Path, *, failed_criteria: tuple[str, ...] = ()) -> Path`, and `fake_child_and_grandchild(tmp_path: Path) -> tuple[str, ...]`.

- [ ] **Step 1: Write failing record and decision tests**

```python
from pathlib import Path

from tests.qualification.factories import make_record
from tools.qualification.model import REQUIRED_CRITERIA, decision_for, status_for


ROOT = Path(__file__).resolve().parents[2]


def test_semantic_failure_is_complete_and_selects_fts_only() -> None:
    record = make_record(ROOT, "semantic-native", failed=("locked-native-build",))
    assert record.decision == "fts-only"
    assert decision_for(record.risk, record.evidence) == "fts-only"


def test_blocking_failure_cannot_change_the_contract() -> None:
    record = make_record(ROOT, "mcp-transport", failed=("locked-native-build",))
    assert status_for(record.evidence) == "fail"
    assert record.decision == "blocked"


def test_every_risk_has_exactly_one_ordered_criterion_set() -> None:
    assert tuple(REQUIRED_CRITERIA) == (
        "mcp-transport",
        "semantic-native",
        "frontend-embedding",
        "legacy-database-import",
    )
    assert len(REQUIRED_CRITERIA["mcp-transport"]) == 17
    assert all(len(criteria) == len(set(criteria)) for criteria in REQUIRED_CRITERIA.values())
```

- [ ] **Step 2: Run the tests and verify the missing-package failure**

Run: `uv run pytest tests/qualification/test_model.py -v`

Expected: FAIL during collection with `ModuleNotFoundError: No module named 'tools.qualification'`.

- [ ] **Step 3: Implement typed records and immutable risk rules**

Use these exact public types and constants in `tools/qualification/model.py`:

```python
from dataclasses import dataclass
from pathlib import Path
from typing import Literal


Risk = Literal[
    "mcp-transport",
    "semantic-native",
    "frontend-embedding",
    "legacy-database-import",
]
EvidenceStatus = Literal["pass", "fail", "not-run"]
Decision = Literal["qualified", "blocked", "semantic-enabled", "fts-only"]
ReviewStatus = Literal["pending", "accepted", "rejected"]
JsonScalar = str | int | float | bool | None
MeasurementValue = JsonScalar | tuple[JsonScalar, ...]


REQUIRED_CRITERIA: dict[Risk, tuple[str, ...]] = {
    "mcp-transport": (
        "locked-native-build",
        "protocol-2026-07-28",
        "no-handshake-or-session",
        "per-request-required-metadata",
        "unsupported-version-rejected",
        "stdio-newline-jsonrpc",
        "streamable-http-json",
        "streamable-http-sse",
        "http-method-name-headers",
        "http-host-auth-version-cases",
        "official-schema-envelopes",
        "header-mismatch-errors",
        "registry-identity",
        "result-error-identity",
        "result-type-required",
        "required-auth-metadata",
        "private-bridge-absent",
    ),
    "semantic-native": (
        "locked-native-build",
        "binary-size-recorded",
        "install-uninstall",
        "model-checksum-load",
        "ten-thousand-chunk-build",
        "ann-index-created",
        "series-prefilter-before-ann",
        "zero-cross-series-hits",
        "insert-search-delete",
        "generation-isolation",
        "durable-sqlite-job-state",
        "no-sqlite-write-across-native-io",
        "crash-recovery",
        "cancel-recovery",
        "fts-fallback",
        "fifty-query-run",
    ),
    "frontend-embedding": (
        "bun-version-and-frozen-build",
        "missing-bundle-rejected",
        "release-assets-embedded",
        "index-and-spa-fallback",
        "hashed-asset-and-mime",
        "missing-asset-404",
        "runtime-asset-root-inaccessible",
        "no-runtime-bun-node-python",
        "no-source-map-secret",
        "binary-size-recorded",
    ),
    "legacy-database-import": (
        "bundled-sqlite-fts5",
        "fixture-classification",
        "supported-current-read",
        "supported-legacy-read",
        "typed-row-accounting",
        "fts-query-equivalence",
        "ledger-preserved",
        "unsupported-fail-closed",
        "source-byte-identity",
        "no-sensitive-row-output",
    ),
}


FAILURE_CONSEQUENCES: dict[Risk, str] = {
    "mcp-transport": (
        "Block rust-workspace-and-contract-harness and rust-daemon-mcp-security; "
        "preserve MCP revision 2026-07-28, stdio, POST /mcp JSON/SSE, and removal "
        "of /api/mcp/{operation}."
    ),
    "semantic-native": (
        "Select FTS5-only for the initial x86_64-unknown-linux-gnu release; do not "
        "block rust-workspace-and-contract-harness or the Linux release."
    ),
    "frontend-embedding": (
        "Block rust-workspace-and-contract-harness, rust-frontend, and "
        "rust-distribution-cutover; preserve embedded Svelte assets and no Bun, "
        "Node, or Python runtime dependency."
    ),
    "legacy-database-import": (
        "Block rust-workspace-and-contract-harness, rust-database-upgrade, and "
        "rust-distribution-cutover; preserve every supported source schema and "
        "never create a fresh sibling database beside legacy data."
    ),
}


@dataclass(frozen=True)
class Evidence:
    criterion: str
    status: EvidenceStatus
    summary: str
    measurements: dict[str, MeasurementValue]
    not_run_reason: str | None = None


@dataclass(frozen=True)
class Environment:
    rustc: str
    cargo: str
    target: str
    os: str
    kernel: str
    architecture: str
    bun: str | None
    native_libraries: tuple[str, ...]


@dataclass(frozen=True)
class LockedDependency:
    name: str
    version: str
    source: str
    checksum: str | None
    features: tuple[str, ...]


@dataclass(frozen=True)
class CleanupEvidence:
    work_dir_removed: bool
    raw_logs_removed: bool
    install_dir_removed: bool
    source_inputs_unchanged: bool
    user_data_opened: bool
    core_dumps_disabled: bool
    owned_process_groups_reaped: bool


@dataclass(frozen=True)
class Review:
    owner: str
    status: ReviewStatus
    objective_evidence_reviewed: bool
    normative_constraints_preserved: bool


@dataclass(frozen=True)
class QualificationRecord:
    schema_version: int
    risk: Risk
    target: str
    status: Literal["pass", "fail"]
    decision: Decision
    acceptance_owner: str
    specs: tuple[str, ...]
    contract_ids: tuple[str, ...]
    input_paths: tuple[str, ...]
    input_digest: str
    commands: tuple[str, ...]
    environment: Environment
    dependencies: tuple[LockedDependency, ...]
    evidence: tuple[Evidence, ...]
    consequence: str
    cleanup: CleanupEvidence
    review: Review
```

Task 2 derives status and decision only from complete criterion evidence, enforces exact risk/decision/consequence combinations, rejects duplicate/unknown/missing criteria, and requires a reason for every `not-run`. Live runners create `Review(owner="Pavel Obruchnikov <me@inkyquill.net>", status="pending", objective_evidence_reviewed=False, normative_constraints_preserved=False)`; only Task 20 changes review fields. `record.schema.json` sets `additionalProperties: false` on every fixed object; measurements permit only named scalars or scalar arrays, never payload/log objects.

- [ ] **Step 4: Run the model tests GREEN**

Run: `uv run pytest tests/qualification/test_model.py -v`

Expected: PASS; every risk has exact criteria and deterministic pass/fail decision mapping.

- [ ] **Step 5: Commit the common record model**

```bash
git add qualification/schemas/record.schema.json tools/qualification/__init__.py tools/qualification/model.py tests/qualification/factories.py tests/qualification/test_model.py
git commit -m "test: define Rust qualification record model"
```

### Task 3: Record Validation, Redaction, Fingerprints, And Rendering

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `tools/qualification/fingerprint.py`
- Create: `tools/qualification/projections.py`
- Create: `tools/qualification/redaction.py`
- Create: `tools/qualification/validate.py`
- Create: `tools/qualification/render.py`
- Create: `qualification/compatibility/mcp-transport.json`
- Create: `qualification/compatibility/legacy-database-import.json`
- Modify: `tests/qualification/factories.py`
- Create: `tests/qualification/test_fingerprint.py`
- Create: `tests/qualification/test_projections.py`
- Create: `tests/qualification/test_redaction.py`
- Create: `tests/qualification/test_render.py`
- Regenerate for Task 3 test-node ownership only: `compatibility/snapshots/state.json`
- Regenerate for Task 3 test-node ownership only: `compatibility/manifest.json`
- Regenerate for Task 3 test-node ownership only: `compatibility/fixtures/diagnostics/check-success.txt`

**Interfaces:**
- Consumes: Task 2 `QualificationRecord`, exact checked-in paths rooted at `repo_root`, and no implicit environment/user data.
- Produces: `fingerprint_inputs(repo_root: Path, paths: tuple[str, ...]) -> str`, SHA-256 over sorted relative paths plus bytes.
- Produces: `ProjectionRisk = Literal["mcp-transport", "legacy-database-import"]`, `build_projection(repo_root: Path, risk: ProjectionRisk) -> dict[str, object]`, `canonical_projection_bytes(projection: Mapping[str, object]) -> bytes`, and `projection_issues(repo_root: Path) -> dict[ProjectionRisk, tuple[str, ...]]` in `tools.qualification.projections`.
- Produces checked-in canonical projections `qualification/compatibility/mcp-transport.json` and `qualification/compatibility/legacy-database-import.json`; the former contains only the 42 relevant manifest contract entries, and the latter contains only the three relevant manifest contract entries plus the exact consumed database state fields.
- Produces deterministic CLI `python -m tools.qualification.projections --check`; it is offline and read-only. `--write` exists only for the explicit Task 3 generation step and atomically writes the two owned projection paths.
- Produces: `redaction_issues(serialized_record: str) -> list[str]` and `validate_record(record: QualificationRecord, repo_root: Path) -> list[str]`.
- Produces: `replay_commands_are_safe(value: object) -> bool`; it accepts only a nonempty tuple of unique commands from the exact offline replay matrix in Step 5.
- Produces: `render_record(record: QualificationRecord) -> str` with stable headings/table order.
- Produces deterministic CLIs `python -m tools.qualification.validate <record.json>` and `python -m tools.qualification.render --check <record.json> <record.md>`.

- [ ] **Step 1: Write failing projection, fingerprint, redaction, replay, and render tests**

```python
import json
from dataclasses import asdict, replace
from pathlib import Path, PurePosixPath

from tests.qualification.factories import make_record
from tools.qualification.fingerprint import COMMON_FINGERPRINT_INPUTS, fingerprint_inputs
from tools.qualification.projections import projection_issues
from tools.qualification.redaction import redaction_issues
from tools.qualification.render import render_record
from tools.qualification.validate import validate_record

def _seed_common_inputs(root: Path) -> None:
    for relative in COMMON_FINGERPRINT_INPUTS:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"qualification fixture: {relative}\n", encoding="utf-8")


def test_input_fingerprint_changes_with_fixture_bytes(tmp_path: Path) -> None:
    fixture = tmp_path / "fixture.txt"
    fixture.write_text("before", encoding="utf-8")
    before = fingerprint_inputs(tmp_path, ("fixture.txt",))
    fixture.write_text("after", encoding="utf-8")
    assert fingerprint_inputs(tmp_path, ("fixture.txt",)) != before


def test_record_rejects_secret_and_home_path(tmp_path: Path) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(tmp_path, "frontend-embedding")
    leaked = replace(
        record,
        commands=(*record.commands, "compat-secret-do-not-log /home/alice/private"),
    )
    serialized = json.dumps(asdict(leaked), sort_keys=True, separators=(",", ":"))
    assert redaction_issues(serialized) == [
        "record contains forbidden literal: compat-secret-do-not-log",
        "record contains an absolute home path",
    ]
    assert validate_record(leaked, tmp_path) == redaction_issues(serialized)


def test_render_is_deterministic(tmp_path: Path) -> None:
    _seed_common_inputs(tmp_path)
    record = make_record(tmp_path, "legacy-database-import")
    assert render_record(record) == render_record(record)
    assert "Legacy Database Import Qualification Record" in render_record(record)


def test_unrelated_inventory_changes_do_not_stale_projections(tmp_path: Path) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)
    manifest = read_json(tmp_path / "compatibility/manifest.json")
    manifest["test_ownership"].append(
        {
            "disposition": "implementation_internal",
            "node_id": "tests/qualification/test_later.py::test_unrelated",
            "reason": "later qualification inventory",
        }
    )
    write_json(tmp_path / "compatibility/manifest.json", manifest)
    state = read_json(tmp_path / "compatibility/snapshots/state.json")
    state["tests"]["node_ids"].append(
        "tests/qualification/test_later.py::test_unrelated"
    )
    write_json(tmp_path / "compatibility/snapshots/state.json", state)
    assert projection_issues(tmp_path) == {
        "mcp-transport": (),
        "legacy-database-import": (),
    }


def test_relevant_source_or_checked_in_projection_drift_is_blocking(
    tmp_path: Path,
) -> None:
    seed_projection_sources_and_checked_in_files(tmp_path)
    manifest = read_json(tmp_path / "compatibility/manifest.json")
    selected = next(
        item for item in manifest["contracts"] if item["id"] == "http.route.post.mcp"
    )
    selected["disposition"] = "remove"
    write_json(tmp_path / "compatibility/manifest.json", manifest)
    assert projection_issues(tmp_path)["mcp-transport"]
```

Define the private test helpers in `test_projections.py`: `read_json`/`write_json` use strict UTF-8 and canonical JSON bytes; `seed_projection_sources_and_checked_in_files` writes a minimal closed manifest with the exact 42 MCP and three database contracts, a state object with all 12 database fields plus unrelated `tests`/`config`, then writes both outputs from `build_projection`. The helper must call the production builder rather than maintain a second projection algorithm.

Add focused RED cases that mutate each selected MCP/database contract field, add/remove one `mcp.tool.*` id, mutate each projected database state field, reorder a source array, or corrupt/add a field to a checked-in projection; each must report deterministic drift for only the affected risk. Mutating an unrelated contract, `frontend_test_ownership`, `test_ownership`, `state["tests"]`, `state["config"]`, an unlisted `state["database"]` field, or another unrelated state surface must report no projection issue. Assert the projection checker performs no write, subprocess, socket, URL, or SQLite operation.

Add the exact remaining review regressions: a root spelled `alias/../chosen`; forced `os.dup` failure whose exception contains an absolute private path; a same-size in-place file rewrite during a multi-chunk read; structured `password`, `passWord`, `private_key`, `privateKey`, `auth`, `auth_header`, `authHeader`, `authorizationHeader`, `proxyAuthorization`, `cookieHeader`, `set_cookie`, `setCookie`, `memory_text`, `memoryText`, `source_text`, `sourceText`, `raw_log`, `rawLog`, `raw_logs`, `rawLogs`, `stdout`, and `stderr`, each with string, number, boolean, null, and scalar-array values at root and nested mapping/list depths; raw `-----BEGIN PRIVATE KEY-----`, `-----BEGIN RSA PRIVATE KEY-----`, `-----BEGIN EC PRIVATE KEY-----`, and `-----BEGIN OPENSSH PRIVATE KEY-----` blocks; `%2Fhome%2Falice%2Fprivate`, `%2FUsers%2Falice%2Fprivate`, `%2Froot%2Fprivate`, and `C%3A%5CUsers%5CAlice%5Cprivate`; `$OLDPWD/script`, `${PWD}/script`, `${INPUT:-/etc/passwd}`, globbing, substitution, redirection, every shell control/metacharacter, `curl`, `wget`, `cargo fetch`, `bun install`, `uv sync`, a URL token, and an unlisted executable; empty and duplicate replay-command tuples; every accepted/rejected row in the Step 5 replay matrix; every C0 control, DEL, every C1 control U+0080–U+009F; and Markdown `[label](target)` plus `![alt](target)` in every arbitrary rendered field. Diagnostics must never echo the matched value or underlying absolute path.

- [ ] **Step 2: Run tests and verify RED**

Run: `uv run pytest tests/qualification/test_fingerprint.py tests/qualification/test_projections.py tests/qualification/test_redaction.py tests/qualification/test_render.py -v`

Expected: FAIL importing the five Task 3 modules and both checked-in projections; after the pre-review implementation exists, the added regressions fail specifically on mutable-global staleness, noncanonical root handling, unnormalized `os.dup` failure, same-size mutation, residual structured/encoded secret forms, shell expansion/network commands, and active Markdown links/images.

- [ ] **Step 3: Build closed, source-derived compatibility projections**

`build_projection` reads the current `compatibility/manifest.json` and, only for the database projection, `compatibility/snapshots/state.json`. It requires unique string contract ids and copies the selected contract objects losslessly: no case folding, path rewriting, default insertion, list sorting, value coercion, or omission of a present contract field is allowed. Canonicalization sorts JSON object keys and the selected contract objects by exact `id`, but preserves every source array in its original order so an order change in a relevant field remains a semantic change. Serialize with UTF-8 `json.dumps(..., ensure_ascii=False, allow_nan=False, sort_keys=True, separators=(",", ":")) + "\n"`.

The MCP projection has exactly these top-level keys and selection rule:

```python
{
    "projection_version": 1,
    "risk": "mcp-transport",
    "contracts": sorted(
        full_source_contracts(
            exact_ids={
                "cli.script.hieronymus-mcp",
                "http.route.post.mcp",
                "http.route.post.api.mcp.operation",
            },
            id_prefix="mcp.tool.",
        ),
        key=lambda item: item["id"],
    ),
}
```

Require exactly 42 unique entries: the three exact ids and all 39 current `mcp.tool.*` entries. Each entry is the complete selected manifest contract object, including `adr` wherever present and the exact ordered `tests` file list; neither `frontend_test_ownership` nor `test_ownership` is copied. `full_source_contracts` is a private helper that indexes `manifest["contracts"]` by exact id, rejects duplicate/non-string ids or malformed entries, selects the three exact ids plus every prefix match for MCP (or the three exact database ids), deep-copies the complete selected objects, and rejects missing/unexpected selected ids before sorting.

The database projection has exactly these top-level keys and exact state-field allowlist:

```python
DATABASE_CONTRACT_IDS = (
    "database.schema.current",
    "database.migrations.current",
    "database.upgrade.preflight",
)
DATABASE_STATE_FIELDS = (
    "application_migration_ledgers",
    "columns",
    "fixture",
    "foreign_keys",
    "indexes",
    "migration_sources",
    "object_contracts",
    "representative_rows",
    "row_counts",
    "tables",
    "triggers",
    "variants",
)
{
    "projection_version": 1,
    "risk": "legacy-database-import",
    "contracts": sorted(full_source_contracts(DATABASE_CONTRACT_IDS), key=id_key),
    "database": {name: state["database"][name] for name in DATABASE_STATE_FIELDS},
}
```

Require the database source object to contain all 12 consumed fields with their source values unchanged. The checked-in projection's `database` object has exactly those 12 keys, but additional unlisted fields in source `state["database"]` are irrelevant and ignored. Task 17 must consume every projected field, including `object_contracts` ownership mapping, fixture identity, inventories, row counts/representative rows, migration sources/ledger, and variants. Do not copy `data_root`, `owned_paths`, `state["tests"]`, configuration, distribution, integrations, or any other state surface.

`projection_issues(repo_root)` rebuilds both expected projections in memory, validates the checked-in documents' exact closed shapes and canonical bytes, and returns a dict in MCP/database order. A relevant source field change or checked-in byte drift yields a fixed risk-prefixed issue; unrelated source changes yield no issue. All parse, type, missing-file, duplicate-id, and I/O failures become deterministic messages without source values or absolute paths. It never writes. The `--write` branch is separate, Task 3-only generation using atomic replacements after both documents validate.

- [ ] **Step 4: Implement exact risk policies and descriptor-stable fingerprints**

Use this exact common fingerprint set in `fingerprint.py`:

```python
COMMON_FINGERPRINT_INPUTS = (
    "qualification/prerequisites.json",
    "qualification/rust-toolchain.toml",
    "tools/qualification/model.py",
    "tools/qualification/fingerprint.py",
    "tools/qualification/projections.py",
    "tools/qualification/redaction.py",
    "tools/qualification/validate.py",
    "tools/qualification/render.py",
    "tools/qualification/acquire.py",
    "tools/qualification/process.py",
    "tools/qualification/clean.py",
)
```

`required_fingerprint_inputs(risk)` returns one exact literal common-plus-suffix tuple, and validation rejects omission, addition, duplication, or reordering before recomputing the digest. The four policies are:

- MCP, 180 total inputs: the 11 common files; `tools/qualification/run_mcp.py`; `qualification/harnesses/mcp-transport/Cargo.toml`, `Cargo.lock`, `src/main.rs`, `src/registry.rs`, `src/report.rs`, and `tests/transport.rs` (all abbreviated harness entries in this sentence are relative to `qualification/harnesses/mcp-transport/`); `qualification/compatibility/mcp-transport.json`; `compatibility/authorities/mcp/2026-07-28/schema.json` and `schema.source.json`; `compatibility/snapshots/mcp.json`; `compatibility/fixtures/mcp/protocol.json`; `compatibility/fixtures/http/route-cases.json`; and exactly `compatibility/fixtures/mcp/<name>/error.input.json`, `success.input.json`, `wire.error.json`, and `wire.success.json` for each literal tool name whose `mcp.tool.<name>` id is in the verified 39-tool projection. The 39 tool directories are direct children of `compatibility/fixtures/mcp/`; there is no intermediate `tools/` directory. The literal `_MCP_TOOL_NAMES` tuple and projected tool-id set must be equal.
- Semantic, 28 total inputs: the 11 common files; `tools/qualification/run_semantic.py`; `qualification/harnesses/semantic-native/Cargo.toml`, `Cargo.lock`, `src/lib.rs`, `src/corpus.rs`, `src/model.rs`, `src/index.rs`, `src/main.rs`, `src/scenario.rs`, `src/fts.rs`, `tests/corpus.rs`, `tests/index.rs`, `tests/recovery.rs`, and `tests/fts.rs` (all abbreviated harness entries in this sentence are relative to `qualification/harnesses/semantic-native/`); `qualification/fixtures/semantic-corpus.json`; `compatibility/fixtures/mcp/hieronymus_rag_search/success.input.json`; and `compatibility/fixtures/mcp/hieronymus_recall/success.input.json`.
- Frontend, 47 total inputs: the 11 common files; `tools/qualification/run_frontend.py`; `qualification/harnesses/frontend-embedding/Cargo.toml`, `Cargo.lock`, `build.rs`, `src/main.rs`, `src/assets.rs`, and `tests/assets.rs` (all abbreviated harness entries in this sentence are relative to `qualification/harnesses/frontend-embedding/`); `frontend/index.html`, `frontend/package.json`, `frontend/bun.lock`, `frontend/tsconfig.json`, and `frontend/vite.config.ts`; the exact 23 paths under `frontend/src/web/`: `App.svelte`, `app.css`, `app.test.ts`, `components/AdminDashboard.svelte`, `components/DreamingEditor.svelte`, `components/IngestEditor.svelte`, `components/MemoryViews.svelte`, `components/MemoryViews.test.ts`, `components/ProviderEditor.svelte`, `components/ReleaseEditor.svelte`, `components/Toast.svelte`, `components/editors.test.ts`, `fonts.css`, `fonts/geist.woff2`, `fonts/inconsolatalgc.woff2`, `fonts/literata.woff2`, `lib/admin-events.svelte.ts`, `lib/api.ts`, `lib/theme.svelte.test.ts`, `lib/theme.svelte.ts`, `lib/types.ts`, `main.ts`, and `test/setup.ts`; plus `compatibility/fixtures/http/route-cases.json`.
- Database, 26 total inputs: the 11 common files; `tools/qualification/run_database.py`; `qualification/harnesses/legacy-database-import/Cargo.toml`, `Cargo.lock`, `src/main.rs`, `src/classify.rs`, `src/probe_import.rs`, `src/report.rs`, and `tests/fixtures.rs` (all abbreviated harness entries in this sentence are relative to `qualification/harnesses/legacy-database-import/`); `qualification/compatibility/legacy-database-import.json`; and exactly `compatibility/fixtures/database/corrupt.sqlite`, `empty.sqlite`, `legacy-python.sqlite`, `minimal-python.sqlite`, `partial-python.sqlite`, and `unknown-schema.sqlite` (all abbreviated database entries in this sentence are relative to `compatibility/fixtures/database/`).

The whole mutable `compatibility/manifest.json` and `compatibility/snapshots/state.json` are absent from every risk suffix. Projection files and `projections.py` are covered, while implementation-internal test-node inventory is not. Task 3 updates `make_record` and its seeding helper to copy or create only paths already proven by the literal policy/manifest/inventory test below; a factory is forbidden to make a nonexistent policy path appear valid merely by synthesizing it. Its default command becomes an exact allowed `tools.qualification.validate` replay command rather than an ad-hoc executable name.

Add `test_mcp_literal_policy_matches_manifest_fixture_refs_and_real_inventory` in `tests/qualification/test_fingerprint.py`. It loads the real manifest, requires exactly 39 `mcp.tool.*` contracts, derives the exact tool names, and requires each contract's `fixture` to equal `compatibility/fixtures/mcp/<name>/success.input.json`. It then compares the 156 policy leaves to a `Path.iterdir()`/`find`-equivalent inventory of the four accepted leaf names under those exact direct-child directories:

```python
tool_contracts = tuple(
    item for item in manifest["contracts"] if item["id"].startswith("mcp.tool.")
)
assert len(tool_contracts) == 39
tool_names = tuple(sorted(item["id"].removeprefix("mcp.tool.") for item in tool_contracts))
assert tool_names == tuple(sorted(_MCP_TOOL_NAMES))
for item in tool_contracts:
    name = item["id"].removeprefix("mcp.tool.")
    assert item["fixture"] == f"compatibility/fixtures/mcp/{name}/success.input.json"
expected = {
    f"compatibility/fixtures/mcp/{name}/{leaf}"
    for name in tool_names
    for leaf in _MCP_TOOL_FIXTURE_LEAVES
}
actual = {
    path.relative_to(ROOT).as_posix()
    for directory in (ROOT / "compatibility/fixtures/mcp").iterdir()
    if directory.is_dir() and directory.name in tool_names
    for path in directory.iterdir()
    if path.name in _MCP_TOOL_FIXTURE_LEAVES
}
assert len(expected) == len(actual) == 156
assert set(MCP_TOOL_INPUT_WIRE_INPUTS) == expected == actual
assert len(required_fingerprint_inputs("mcp-transport")) == 180
assert len(required_fingerprint_inputs("semantic-native")) == 28
assert all(
    PurePosixPath(path).parts[3] != "tools"
    for path in MCP_TOOL_INPUT_WIRE_INPUTS
)
```

Run the equivalent repository audit with `find compatibility/fixtures/mcp -mindepth 2 -maxdepth 2 -type f` filtered to the four accepted leaf basenames and require exactly 156 paths. This test reads the real repository only; temporary-root factory tests consume its already verified literal tuple and separately prove that every required path exists before fingerprinting.

Before opening `/`, `_open_repository_root` requires `os.fspath(repo_root)` to be an already absolute, lexically canonical spelling: `normpath(raw) == raw`, with no `.`, `..`, repeated separator, or non-root trailing separator. It must reject `alias/../chosen` instead of applying `abspath` and silently selecting different bytes. Walk that exact spelling component-by-component with no-follow directory descriptors.

Normalize every `OSError` from `os.open`, `os.dup`, `os.fstat`, `os.read`, and descriptor cleanup into the existing deterministic `ValueError` boundary; no errno text or absolute path reaches validation or either CLI. Bind each input through the root descriptor, reject hard-link aliases, and compare before/after `st_dev`, `st_ino`, regular-file mode, `st_size`, `st_mtime_ns`, and `st_ctime_ns`, plus bytes read. Any difference reports that the relative input changed while being read, so a same-size in-place multi-chunk rewrite cannot yield a mixed-state digest.

- [ ] **Step 5: Close redaction, replay grammar, and literal Markdown rendering**

Validation still requires every Task 2 criterion exactly once, exact decision/consequence, exact risk inputs, all cleanup booleans except `user_data_opened` true, and `user_data_opened` false. For MCP/database records it also appends only that risk's `projection_issues(repo_root)` result.

When the supplied text is canonical record JSON, `redaction_issues` parses it strictly and recursively visits every mapping nested through mappings and arrays before applying the common raw-text scan; completed Markdown uses the same ordered raw-text rules after rendering. JSON parse failure at the record-validation boundary is a fixed validation issue, never a fallback that skips the structured walk. The implementation never relies on a regex that can see only string-valued fields. Normalize a mapping key only by removing ASCII `_` and folding ASCII letters to lowercase, then compare the entire result—not a substring or prefix—to a closed normalized-name set. The set covers all exact snake/camel spellings below:

| Issue class | Exact accepted spellings normalized for comparison |
|---|---|
| Authorization | `auth`, `auth_header`, `authHeader`, `authorization`, `authorization_header`, `authorizationHeader`, `proxy_authorization`, `proxyAuthorization` |
| Cookie | `cookie`, `cookie_header`, `cookieHeader`, `set_cookie`, `setCookie` |
| Password/private/token/provider | `password`, `pass_word`, `passWord`, `private_key`, `privateKey`, `access_token`, `accessToken`, `refresh_token`, `refreshToken`, `client_secret`, `clientSecret`, `api_key`, `apiKey`, `provider_key`, `providerKey`, `openai_api_key`, `openaiApiKey`, `anthropic_api_key`, `anthropicApiKey`, `gemini_api_key`, `geminiApiKey`, `secret`, `token`, `launch_grant`, `launchGrant` |
| Source text | `memory_text`, `memoryText`, `source_text`, `sourceText`, `source_row`, `sourceRow`, `row_text`, `rowText`, `chunk_text`, `chunkText`, `translation_text`, `translationText`, `note_text`, `noteText` |
| Raw process output | `raw_log`, `rawLog`, `raw_logs`, `rawLogs`, `stdout`, `stderr` |
| Host/user identity | `hostname`, `host_name`, `hostName`, `machine_name`, `machineName`, `host`, `username`, `user_name`, `userName`, `login_user`, `loginUser` |

For a forbidden normalized key, inspect the value regardless of whether it is a string, number, boolean, null, or scalar array. Only the exact string sentinels `none`, `<absent>`, and `<redacted>` (ASCII case-insensitive after trimming) are safe; an array is safe only when it is nonempty and every element is one of those string sentinels. Numbers, booleans, null, mixed arrays, objects, and all other strings report the fixed issue for that class. The completed Markdown then passes through the same ordered, non-echoing textual rules. Safe sentinel phrases such as `stderr: none;`, `raw log: <absent>.`, and table cell `| authorizationHeader | <redacted> |` remain allowed with ordinary terminal `.`, `,`, `;`, `:`, `!`, `?`, closing parenthesis, or Markdown table context; the punctuation is not captured as secret material.

Scan both raw text and one case-insensitive percent-decoded view for `/home/<user>`, `/Users/<user>`, `/root`, `C:\Users\<user>`, and `C:\Documents and Settings\<user>`; decoding is for detection only and never rewrites evidence. Add a raw-text PEM rule that recognizes `-----BEGIN PRIVATE KEY-----` and every BEGIN label ending in ` PRIVATE KEY`—including RSA, EC, DSA, ENCRYPTED, and OPENSSH. A BEGIN marker is sufficient even when the block is truncated; when an END marker is present its label must not be echoed. The rule returns only `record contains private-key material`. No redaction diagnostic may include the key name, matched value, PEM payload, or absolute path.

`replay_commands_are_safe` itself requires `type(value) is tuple`, `bool(value)`, every element to be a string, and `len(value) == len(set(value))` before testing membership; `validate_record` does not supply uniqueness as a separate compensating check. The allowlist is finite and derived from this exact matrix, with `<risk>` expanded only over the four Task 2 risks. `<status/assertions>` expands to `accepted/true/true` or to `rejected` with `false/false`, `true/false`, or `false/true`; `accepted` with a false assertion and `rejected/true/true` are invalid:

| Owner tasks | Exact offline command family admitted to a record |
|---|---|
| 8/12/15/18 | `HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/<risk> CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_<risk-module> --write`, using exact pairs `mcp-transport/run_mcp`, `semantic-native/run_semantic`, `frontend-embedding/run_frontend`, `legacy-database-import/run_database` |
| 12 and all record handoffs | `uv run python -m tools.qualification.validate qualification/records/<risk>.json` and `uv run python -m tools.qualification.render --check qualification/records/<risk>.json docs/qualification/rust/<risk>.md` |
| 19/21 | `HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run all --write` |
| 19/20/21 | `uv run python -m tools.qualification.check --record <risk>`, `uv run python -m tools.qualification.check --records-only`, and `uv run python -m tools.qualification.check --require-qualified` |
| 3/19/21 | `uv run python -m tools.qualification.projections --check`, `uv run --no-cache --no-sync python -B -m tools.qualification.projections --check`, `uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only`, and `uv run --no-cache --no-sync python -B -m tools.qualification.check --require-qualified` |
| 20/21 | `uv run python -m tools.qualification.review <risk> --status <status> --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed <bool> --normative-constraints-preserved <bool>` with only the four coherent status/assertion triples above |
| 12/21 cleanup | `uv run python -m tools.qualification.clean`, `uv run python -m tools.qualification.clean --apply`, and `uv run python -m tools.qualification.clean --apply --include-model` |
| 8 | The exact MCP `metadata`, `tree`, `fmt`, and `clippy` Cargo commands printed in Task 8 |
| 12 | The exact semantic `fmt` and `clippy` Cargo commands printed in Task 12 |
| 15 | The exact frontend `build`, `fmt`, and `clippy` Cargo commands printed in Task 15, plus Task 13's exact `unshare --user --map-root-user --net -- bun run --cwd frontend build -- --outDir ../qualification/.artifacts/frontend-dist/current --emptyOutDir` |
| 18 | The exact database `fmt` and `clippy` Cargo commands printed in Task 18 |

Implement the matrix as canonical raw command strings plus exact finite expansions, not as a permissive executable/flag parser. Ordinary entries split on exactly one ASCII space and allow only ASCII alphanumerics plus `_`, `-`, `.`, `/`, `:`, `+`, `,`, or `=`. The sole quote exception is the exact raw segment `--owner "Pavel Obruchnikov <me@inkyquill.net>"` in a canonical review command: parse it with `shlex.split(posix=True)`, require the resulting owner token to equal `Pavel Obruchnikov <me@inkyquill.net>`, require the exact flag order shown above, and require raw reserialization to equal the canonical matrix entry. This narrowly supports the named owner without allowing quotes elsewhere, unmatched quotes, general `<`/`>` tokens, backslashes, `%`, `$`, `~`, `*`, `?`, brackets/braces/parentheses, backticks, `!`, `#`, `;`, `&`, `|`, controls, substitution, expansion, globbing, redirection, or arbitrary shell syntax. URLs/URI schemes and all unlisted programs/subcommands remain invalid; this explicitly excludes `curl`, `wget`, `git`, `ssh`, `nc`, `cargo fetch`, `bun install`, `uv sync`, and `tools.qualification.acquire` from recorded offline replay.

Create one parameterized test matrix with every finite expansion above as an accepted case and one adjacent rejected mutation per row: empty tuple, duplicate command, changed risk/runner pairing, missing live/offline assignment, reordered assignment, noncanonical record path, projection `--write`, checker flag reordering/combination not listed, cleanup without `--apply` before `--include-model`, changed owner, unquoted owner, mismatched accepted/false or rejected/true assertions, Cargo `fetch`/extra flag/target mismatch, frontend output path without the exact `../`, and every shell/network mutation. A mechanical plan audit extracts every exact replay/cleanup/projection/review command printed in Tasks 8, 12, 15, 18, 19, 20, and 21 and requires it to appear in the accepted matrix; `PLANNED_REPLAY_COMMANDS` is the single source used by the validator and the test factory.

`render_record` prints title, decision, exact replay-command table, environment/dependency tables, one row per required criterion, consumed compatibility ids, input digest, cleanup assertions, and immutable consequence; it never includes raw stdout/stderr. One `_literal_markdown` function handles every arbitrary value in tables and bullets. It encodes every C0 code point U+0000–U+001F, DEL U+007F, every C1 code point U+0080–U+009F, ampersand, angle brackets, pipe, backslash, backtick, `!`, `[`, `]`, `(`, and `)` as fixed numeric entities so controls and link/image punctuation are always literal and cannot create an active destination. Dependency ordering is total, and the completed Markdown is redaction-scanned before return. `render --check` compares exact `read_bytes()` with UTF-8 rendered bytes.

- [ ] **Step 6: Generate projections, run GREEN, and verify irrelevant inventory stability**

Run: `uv run python -m tools.qualification.projections --write`

Run: `uv run python -m tools.compatibility.inventory_state --write`

Run: `uv run python -m tools.qualification.projections --check`

Run: `uv run pytest tests/qualification/test_fingerprint.py tests/qualification/test_projections.py tests/qualification/test_redaction.py tests/qualification/test_render.py -v`

Run: `uv run ruff check tools/qualification/fingerprint.py tools/qualification/projections.py tools/qualification/redaction.py tools/qualification/validate.py tools/qualification/render.py tests/qualification/test_fingerprint.py tests/qualification/test_projections.py tests/qualification/test_redaction.py tests/qualification/test_render.py`

Expected: all tests and Ruff pass; the two generated projections are canonical/current, relevant source changes fail, unrelated inventory/state changes do not alter projection bytes or record digests, same-size concurrent writes fail, the real manifest fixture refs, direct-child `find` inventory, literal 156-leaf MCP policy, 180-input MCP total, and 28-input semantic total agree, every structured scalar type/nested secret/PEM/encoded-home/shell/network/Markdown case is rejected without echo, safe sentinels remain accepted, every planned command matrix row is accepted with adjacent mutations rejected, and C0/DEL/C1 rendering is byte-deterministic.

Run the inventory generator a second time after the final Task 3 node set and require byte-identical global generated artifacts. Then modify only `test_ownership` and `state["tests"]` in a temporary copy, rerun projection checking and MCP/database record validation, and require both to remain current without rewriting either projection.

- [ ] **Step 7: Commit validation, projections, and rendering**

```bash
git add tools/qualification/fingerprint.py tools/qualification/projections.py tools/qualification/redaction.py tools/qualification/validate.py tools/qualification/render.py qualification/compatibility/mcp-transport.json qualification/compatibility/legacy-database-import.json tests/qualification/factories.py tests/qualification/test_fingerprint.py tests/qualification/test_projections.py tests/qualification/test_redaction.py tests/qualification/test_render.py compatibility/snapshots/state.json compatibility/manifest.json compatibility/fixtures/diagnostics/check-success.txt
git commit -m "test: validate and render Rust qualification records"
```

### Task 4: Qualification Prerequisites And Verified Acquisition

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `qualification/prerequisites.json`
- Create: `qualification/rust-toolchain.toml`
- Create: `tools/qualification/acquire.py`
- Create: `tests/qualification/test_acquire.py`

**Interfaces:**
- Consumes: exact Rust/Bun/model identities from Global Constraints and explicit network acquisition commands only.
- Produces: `acquire_semantic_model(repo_root: Path) -> Path`, atomic checksum-verified network acquisition.
- Produces CLI: `python -m tools.qualification.acquire semantic-model`; Task 9 later adds `onnx-runtime` without changing this contract.

- [ ] **Step 1: Write failing model acquisition tests**

Use a local fake HTTP redirector/downloader: accept the exact initial URL and allowed HTTPS host sequence, reject more than five hops, HTTP, userinfo, fragment, disallowed host, forwarded credentials/cookies, and checksum mismatch. Assert `.part` deletion on every failure and atomic promotion only on the exact model SHA-256.

- [ ] **Step 2: Run acquisition tests and verify RED**

Run: `uv run pytest tests/qualification/test_acquire.py -v`

Expected: FAIL importing `tools.qualification.acquire`.

- [ ] **Step 3: Define exact prerequisite/toolchain files**

Create `qualification/rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.96.0"
profile = "minimal"
targets = ["x86_64-unknown-linux-gnu"]
components = ["clippy", "rustfmt"]
```

Create `qualification/prerequisites.json` exactly as:

```json
{
  "schema_version": 1,
  "target": "x86_64-unknown-linux-gnu",
  "rust": {
    "version": "1.96.0",
    "commit": "ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96"
  },
  "bun": {
    "version": "1.3.14",
    "authority": "frontend/package.json"
  },
  "semantic_model": {
    "provider": "onnx-runtime",
    "repository": "sentence-transformers/all-MiniLM-L6-v2",
    "revision": "9a53d751e60e6dd34f2443711d44d5b09389f89a",
    "file": "onnx/model.onnx",
    "url": "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/9a53d751e60e6dd34f2443711d44d5b09389f89a/onnx/model.onnx",
    "sha256": "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452",
    "dimensions": 384,
    "normalization": "l2"
  },
  "network_policy": {
    "allowed_only_for": [
      "manual review of https://modelcontextprotocol.io/specification/2026-07-28",
      "cargo fetch --locked",
      "bun install --frozen-lockfile",
      "python -m tools.qualification.acquire semantic-model",
      "python -m tools.qualification.acquire onnx-runtime"
    ],
    "ordinary_replay": "offline"
  }
}
```

The model destination is `qualification/.artifacts/models/all-MiniLM-L6-v2/model.onnx` and is never committed.

- [ ] **Step 4: Implement checksum/redirect-safe model acquisition**

`acquire_semantic_model` streams to `model.onnx.part`, hashes while streaming, deletes mismatches, and calls `os.replace` only after exact SHA-256. Initial URL must equal prerequisites. At most five HTTPS hops may use exact hosts `huggingface.co`, `cdn-lfs.huggingface.co`, `cas-bridge.xethub.hf.co`, or `us.aws.cdn.hf.co`, with no userinfo/fragment. Signed query strings are never persisted/logged and credentials/cookies are never forwarded.

- [ ] **Step 5: Run acquisition tests GREEN and commit**

Run: `uv run pytest tests/qualification/test_acquire.py -v`

Expected: PASS without contacting the public network; redirect/checksum/partial-file cases are deterministic.

```bash
git add qualification/prerequisites.json qualification/rust-toolchain.toml tools/qualification/acquire.py tests/qualification/test_acquire.py
git commit -m "test: verify Rust qualification acquisitions"
```

### Task 5: Sanitized Process Environment And Bounded Cleanup

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `qualification/README.md`
- Create: `tools/qualification/process.py`
- Create: `tools/qualification/clean.py`
- Create: `tests/qualification/test_process.py`
- Create: `tests/qualification/test_clean.py`
- Create: `tests/qualification/fixtures/process-smoke/Cargo.toml`
- Create: `tests/qualification/fixtures/process-smoke/Cargo.lock`
- Create: `tests/qualification/fixtures/process-smoke/src/lib.rs`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: Task 4 toolchain pin and the caller's original environment only during tool-root discovery.
- Produces: `discover_tool_roots(original_env: Mapping[str, str]) -> ToolRoots`, where `ToolRoots(cargo_home: Path, rustup_home: Path, cargo_invocation: Path, cargo_resolved_target: Path, rustup_invocation: Path, rustup_resolved_target: Path)` contains canonical cache roots, lexical invocation/shim paths, and separately resolved executable targets. Invocation paths are never replaced by their resolved rustup target.
- Produces: `safe_subprocess_env(work_root: Path, *, cargo_offline: bool, tool_roots: ToolRoots, cargo_target_dir: Path) -> dict[str, str]`.
- Produces: `run_owned_process(argv: tuple[str, ...], *, cwd: Path, env: Mapping[str, str], timeout_seconds: int, no_progress_seconds: int) -> ProcessReceipt`, where `ProcessReceipt(exit_code: int | None, timed_out: bool, stdout_sha256: str, stderr_sha256: str, duration_ms: int, process_group_reaped: bool, core_dumps_disabled: bool)` stores no paths or raw output.
- Produces: `cleanup_targets(repo_root: Path, include_model: bool = False) -> tuple[Path, ...]` and CLI `python -m tools.qualification.clean [--apply] [--include-model]`.

- [ ] **Step 1: Write failing tool-root, Cargo-smoke, process-group, and cleanup tests**

```python
import json
import os
import subprocess
import sys
from dataclasses import asdict
from pathlib import Path

import pytest

from tests.qualification.factories import fake_child_and_grandchild
from tools.qualification.clean import cleanup_targets
from tools.qualification.process import (
    ToolRoots,
    discover_tool_roots,
    run_owned_process,
    safe_subprocess_env,
)


ROOT = Path(__file__).resolve().parents[2]


def _fake_tool_roots(tmp_path: Path) -> ToolRoots:
    cargo_home = tmp_path / "fake-cargo-home"
    rustup_home = tmp_path / "fake-rustup-home"
    bin_dir = cargo_home / "bin"
    bin_dir.mkdir(parents=True)
    rustup_home.mkdir()
    resolved_target = Path(sys.executable).resolve()
    cargo_invocation = bin_dir / "cargo"
    rustup_invocation = bin_dir / "rustup"
    cargo_invocation.symlink_to(resolved_target)
    rustup_invocation.symlink_to(resolved_target)
    return ToolRoots(
        cargo_home=cargo_home,
        rustup_home=rustup_home,
        cargo_invocation=cargo_invocation.absolute(),
        cargo_resolved_target=resolved_target,
        rustup_invocation=rustup_invocation.absolute(),
        rustup_resolved_target=resolved_target,
    )


def test_discovery_preserves_shim_paths_without_resolving_them(tmp_path: Path) -> None:
    expected = _fake_tool_roots(tmp_path)
    original_env = {
        "HOME": str(tmp_path / "original-home"),
        "CARGO_HOME": str(expected.cargo_home),
        "RUSTUP_HOME": str(expected.rustup_home),
        "PATH": str(expected.cargo_invocation.parent),
    }
    actual = discover_tool_roots(original_env)
    assert actual.cargo_invocation == expected.cargo_invocation
    assert actual.cargo_resolved_target == expected.cargo_resolved_target
    assert actual.rustup_invocation == expected.rustup_invocation
    assert actual.rustup_resolved_target == expected.rustup_resolved_target
    assert actual.cargo_invocation.name == "cargo"
    assert actual.cargo_invocation != actual.cargo_resolved_target


@pytest.mark.skipif(
    os.environ.get("HIERONYMUS_QUALIFICATION_LIVE") != "1",
    reason="requires the explicitly acquired Rust 1.96.0 toolchain",
)
def test_sanitized_env_keeps_discovered_rust_toolchain(tmp_path: Path) -> None:
    original_env = dict(os.environ)
    lexical_cargo_home = Path(
        original_env.get("CARGO_HOME", str(Path(original_env["HOME"]) / ".cargo"))
    ).absolute()
    roots = discover_tool_roots(original_env)
    env = safe_subprocess_env(
        tmp_path,
        cargo_offline=True,
        tool_roots=roots,
        cargo_target_dir=tmp_path / "cargo-target",
    )
    assert Path(env["CARGO_HOME"]).resolve() == roots.cargo_home
    assert Path(env["RUSTUP_HOME"]).resolve() == roots.rustup_home
    assert Path(env["HOME"]).is_relative_to(tmp_path)
    assert roots.cargo_invocation == lexical_cargo_home / "bin/cargo"
    assert roots.cargo_invocation.name == "cargo"
    assert roots.cargo_resolved_target == roots.cargo_invocation.resolve(strict=True)
    assert roots.rustup_invocation == lexical_cargo_home / "bin/rustup"
    assert roots.rustup_invocation.name == "rustup"
    assert roots.rustup_resolved_target == roots.rustup_invocation.resolve(strict=True)
    assert all(
        target.is_file() and os.access(target, os.X_OK)
        for target in (roots.cargo_resolved_target, roots.rustup_resolved_target)
    )
    version = subprocess.run(
        (str(roots.cargo_invocation), "+1.96.0", "--version"),
        env=env,
        check=True,
        capture_output=True, text=True,
    )
    assert version.stdout.startswith("cargo 1.96.0")
    assert "syncing" not in version.stderr.lower()
    assert "downloading" not in version.stderr.lower()
    metadata = subprocess.run(
        (
            str(roots.cargo_invocation), "+1.96.0", "metadata", "--offline", "--locked",
            "--no-deps", "--format-version", "1", "--manifest-path",
            str(ROOT / "tests/qualification/fixtures/process-smoke/Cargo.toml"),
        ),
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    assert json.loads(metadata.stdout)["packages"][0]["name"] == "qualification-process-smoke"
    assert "Updating" not in metadata.stderr


def test_owned_process_disables_core_and_reaps_group(tmp_path: Path) -> None:
    roots = _fake_tool_roots(tmp_path)
    receipt = run_owned_process(
        fake_child_and_grandchild(tmp_path), cwd=tmp_path,
        env=safe_subprocess_env(
            tmp_path,
            cargo_offline=True,
            tool_roots=roots,
            cargo_target_dir=tmp_path / "cargo-target",
        ),
        timeout_seconds=1, no_progress_seconds=1,
    )
    assert receipt.timed_out and receipt.core_dumps_disabled
    assert receipt.process_group_reaped and list(tmp_path.glob("core*")) == []
    serialized = json.dumps(asdict(receipt), sort_keys=True)
    assert all(str(path) not in serialized for path in asdict(roots).values())


def test_cleanup_never_targets_repository_or_model_by_default() -> None:
    targets = cleanup_targets(ROOT)
    assert ROOT not in targets
    assert ROOT / "qualification/.artifacts/models" not in targets
    assert all(path.is_relative_to(ROOT / "qualification/.artifacts") for path in targets)
```

- [ ] **Step 2: Run process/cleanup tests and verify RED**

Run: `uv run pytest tests/qualification/test_process.py tests/qualification/test_clean.py -v`

Expected: FAIL importing `tools.qualification.process` and `tools.qualification.clean`.

Run: `HIERONYMUS_QUALIFICATION_LIVE=1 uv run pytest tests/qualification/test_process.py::test_sanitized_env_keeps_discovered_rust_toolchain -v`

Expected: FAIL before implementation; this explicitly opted-in smoke is the only Python test in Task 5 that invokes the real Cargo toolchain.

- [ ] **Step 3: Implement pre-sanitization tool-root discovery**

Before replacing `HOME`, `discover_tool_roots` reads explicit original `CARGO_HOME`/`RUSTUP_HOME` when present; otherwise it derives `<original HOME>/.cargo` and `<original HOME>/.rustup`. It forms absolute lexical root paths first, discovers cargo and rustup at `<lexical cargo home>/bin/<name>` or through the original `PATH`, and only then canonicalizes the two cache-directory fields. For each tool it stores the absolute lexical invocation path without calling `resolve()`—so `~/.cargo/bin/cargo` remains the command path even when it is a rustup shim—and separately computes `resolve(strict=True)` into the matching resolved-target field. It requires both the invocation and resolved target to exist, the target to be a regular executable file, and rejects a missing, directory, or non-executable target. `safe_subprocess_env` then sets task-local `HOME`/TMP/XDG paths but explicitly exports the canonical discovered `CARGO_HOME`, `RUSTUP_HOME`, both invocation-parent directories at the front of `PATH`, `RUSTUP_AUTO_INSTALL=0`, `CARGO_NET_OFFLINE=true`, and the caller's resolved `cargo_target_dir`, which it rejects unless it is beneath `qualification/.artifacts/cargo-target/` in live use or the supplied pytest work root in tests. It removes credentials/proxies/provider variables. Records store only versions, invocation basenames, and digests—never the six absolute `ToolRoots` values.

- [ ] **Step 4: Add actual sanitized Cargo smoke coverage**

The live-gated test first requires `cargo_invocation == lexical_cargo_home / "bin/cargo"` and `rustup_invocation == lexical_cargo_home / "bin/rustup"`, using the value computed from the captured mapping in the test code above. It executes that preserved cargo shim—not `cargo_resolved_target`—for both `cargo +1.96.0 --version` and `cargo +1.96.0 metadata --offline --locked --no-deps --format-version 1` against a tiny checked-in test-only manifest at `tests/qualification/fixtures/process-smoke/Cargo.toml`; its committed `Cargo.lock` has no dependencies and `src/lib.rs` contains `pub fn smoke() {}`. `CARGO_NET_OFFLINE=true`, `RUSTUP_AUTO_INSTALL=0`, the dependency-free locked graph, and the assertions that rustup reports neither syncing nor downloading prove no sync/network path is used. Assert serialized receipts contain neither tool-root absolute path nor original home; do not persist environment values.

- [ ] **Step 5: Implement bounded process/cleanup behavior**

`run_owned_process` creates/reaps an owned group, applies `RLIMIT_CORE=(0,0)`, bounds output to digests/status/timings, and terminates/kills the group on timeout/failure. `cleanup_targets` permits only work/log/install/cargo-target/frontend-dist descendants under `qualification/.artifacts`; model deletion requires `--include-model`; symlinks, repository root, artifact root, home, and translation workspace are rejected.

- [ ] **Step 6: Add ignores/README, run GREEN, and commit**

Append `qualification/.artifacts/` and `qualification/harnesses/*/target/` to `.gitignore`. Document explicit acquisition, fake-only ordinary tests, bounded cleanup, generated Markdown, and opt-in live qualification.

Run: `uv run pytest tests/qualification/test_process.py tests/qualification/test_clean.py -v`

Expected: PASS with the real Cargo smoke skipped; fake-process, environment-policy, process-group, and cleanup tests need no Rust/native cache.

Run: `HIERONYMUS_QUALIFICATION_LIVE=1 uv run pytest tests/qualification/test_process.py::test_sanitized_env_keeps_discovered_rust_toolchain -v`

Expected: PASS; actual pinned Cargo version/metadata work under the sanitized environment without sync/network, and the serialized receipt contains no absolute tool roots.

```bash
git add .gitignore qualification/README.md tools/qualification/process.py tools/qualification/clean.py tests/qualification/test_process.py tests/qualification/test_clean.py tests/qualification/fixtures/process-smoke
git commit -m "test: bound Rust qualification processes"
```

### Task 6: MCP Candidate Manifest And Locked Build

**Complexity:** Low, 1–2 hours.

**Files:**
- Create: `qualification/harnesses/mcp-transport/Cargo.toml`
- Create: `qualification/harnesses/mcp-transport/Cargo.lock`
- Create: `qualification/harnesses/mcp-transport/src/main.rs` containing only `fn main() {}` until Task 7.

**Interfaces:**
- Consumes: accepted Task 1 oracle commit, including the offline official-schema pin and corrected eleven-case HTTP authority, plus the Rust 1.96.0 qualification toolchain.
- Produces: standalone exact candidate manifest and committed lockfile; no transport behavior or record.
- Build invariant: every command sets `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport` and never creates a repository-local `target/`.

- [ ] **Step 1: Verify RED before the standalone manifest exists**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu`

Expected: FAIL because `qualification/harnesses/mcp-transport/Cargo.toml` does not exist.

- [ ] **Step 2: Create the standalone candidate manifest**

Use this direct candidate boundary; the generated lockfile records every transitive version:

```toml
[package]
name = "hieronymus-mcp-transport-qualification"
version = "0.0.0"
edition = "2024"
rust-version = "1.96"
publish = false

[[bin]]
name = "mcp-transport"
path = "src/main.rs"

[dependencies]
anyhow = "1"
rmcp = { version = "=3.1.4", default-features = false, features = [
  "client",
  "server",
  "macros",
  "schemars",
  "transport-child-process",
  "transport-io",
  "transport-streamable-http-client-reqwest",
  "transport-streamable-http-server",
] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
tokio = { version = "1", features = ["io-util", "macros", "net", "process", "rt-multi-thread", "sync", "time"] }

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Acquire, lock, and prove the empty candidate builds offline**

Networked prerequisite:

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/mcp-transport/Cargo.toml
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport cargo +1.96.0 fetch --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked
```

Offline build:

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu
```

Expected: PASS without network access; `git status --short` shows no unbounded `target/` tree.

- [ ] **Step 4: Commit the independently reviewable candidate boundary**

```bash
git add qualification/harnesses/mcp-transport/Cargo.toml qualification/harnesses/mcp-transport/Cargo.lock qualification/harnesses/mcp-transport/src/main.rs
git commit -m "test: lock MCP qualification candidate"
```

### Task 7: MCP Stateless Transport Behavior

**Complexity:** High, 3–4 hours.

**Files:**
- Modify: `qualification/harnesses/mcp-transport/src/main.rs`
- Create: `qualification/harnesses/mcp-transport/src/registry.rs`
- Create: `qualification/harnesses/mcp-transport/src/report.rs`
- Create: `qualification/harnesses/mcp-transport/tests/transport.rs`

**Interfaces:**
- Consumes: Task 1's accepted, official-schema-validated `compatibility/fixtures/mcp/protocol.json`, `compatibility/fixtures/http/route-cases.json`, `compatibility/snapshots/mcp.json`, and Task 6 lockfile. The immutable schema authority remains a Task 1/Task 8 validation and fingerprint input; the Rust candidate does not fetch or reinterpret it.
- Produces Rust CLI: `mcp-transport stdio --registry <path> --protocol <path>`.
- Produces Rust CLI: `mcp-transport http --registry <path> --protocol <path> --route-cases <path> --bind 127.0.0.1:0 --ready-file <path>`.
- Produces bounded JSON evidence for every MCP criterion; it never performs or accepts initialize/session behavior.

- [ ] **Step 1: Write failing stateless stdio/HTTP behavior tests**

In `tests/transport.rs`, load only `protocol["target"]`; assert tools/list and tools/call work without handshake, require `_meta` protocolVersion/clientCapabilities, accept canonical clientInfo and a clone omitting only clientInfo, reject direct `params.protocolVersion`/`params.clientCapabilities`/`params.clientInfo`, reject missing/wrong required reserved keys, and reject handshake/session headers. Assert every successful tools/list/tools/call response has exactly `resultType: "complete"` over stdio, HTTP JSON, and HTTP SSE and omits `_meta` under the exact configured serverInfo policy. For every tools/list transport, require `cacheScope: "private"`, `ttlMs: 0`, and tools using `inputSchema` with no `input_schema`; compare registry semantics by explicitly mapping target `inputSchema` to snapshot internal `input_schema`.

For HTTP, require tools/list with `Mcp-Method: tools/list` and no `Mcp-Name` to pass; require tools/call with `Mcp-Method: tools/call` and `Mcp-Name: hieronymus_status` to pass. Replay the exact two successes and eleven failures from Task 1. Assert the seven header validation cases are HTTP 400 JSON-RPC `HeaderMismatch` (`-32020`) envelopes with exact ids/messages; assert `protocol-version-header-mismatch` changes only the raw protocol header and leaves body `_meta` at `2026-07-28`. Assert the distinct coherent `unsupported-version` request carries `2025-06-18` in both mirrored locations and returns HTTP 400 code `-32022` with exact requested/supported data. Prove that coherent unsupported version is the sole two-raw-leaf exception and each other failure mutates/omits one applicable raw field.

- [ ] **Step 2: Run behavior tests and verify RED**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu --test transport`

Expected: FAIL because the empty binary exposes no transport.

- [ ] **Step 3: Implement the frozen registry and stateless transports**

`registry.rs` loads immutable `ToolDefinition` values and canned success/error fixture results only. It has no domain store or SQLite access. Its deserializer accepts the official target wire key `inputSchema`; any adapter to a Rust-internal snake_case field is private and serialization must return `inputSchema`. If rmcp cannot represent the exact official shape, report failure; do not introduce handshake state, emit `input_schema`, or alter fixtures.

```rust
pub const PROTOCOL_REVISION: &str = "2026-07-28";

pub trait RegistryProbe: Sized {
    fn load(snapshot: &Path) -> anyhow::Result<Self>;
    fn list_tools(&self) -> &[ToolDefinition];
    fn fixture_result(&self, name: &str, arguments: &Value) -> anyhow::Result<Value>;
}
```

Every request validates `params._meta["io.modelcontextprotocol/protocolVersion"]` and required `params._meta["io.modelcontextprotocol/clientCapabilities"]`; `io.modelcontextprotocol/clientInfo` is validated when present but absence is accepted. Direct legacy metadata fields are rejected. HTTP first validates required mirrored headers against the body: any missing, unexpected, or mismatched protocol, method, or applicable name returns the frozen HTTP 400 `HeaderMismatch` JSON-RPC body. Only after the protocol header and body value agree does it test support and return the frozen HTTP 400 `UnsupportedProtocolVersionError` body. Its method-aware name validator requires a nonempty `Mcp-Name` only for `tools/call`, `resources/read`, and `prompts/get`; for the qualified tools/call fixture it additionally matches the header to `params.name`. It accepts tools/list only without `Mcp-Name` and rejects unexpected or mismatched names.

Every successful tools/list result sets `cacheScope` exactly to `private`, `ttlMs` to `0`, `resultType` to `complete`, and returns official `Tool` objects with `inputSchema`; every successful tools/call result sets `resultType` to `complete`. Every success deliberately omits result `_meta` under Task 1's exact configured serverInfo omission and rationale; candidate version/package data must not appear. Stdio emits one response JSON object plus `\n`; HTTP binds only loopback and serves only `POST /mcp` as JSON or request-scoped SSE.

- [ ] **Step 4: Implement bounded reports and pass the offline behavior suite**

`report.rs` emits one bounded JSON object containing protocol version, transport, request/response SHA-256 values, content types, registry digest, stdout framing counts, and exit status. It never emits the bearer literal, headers, raw payload text, or absolute paths.

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu
```

Expected: the test command completes without network access and proves exact official success objects, internal-to-wire schema mapping, configured serverInfo omission, HeaderMismatch versus unsupported-version classification, exact-match, and mismatch-reporting paths. Candidate protocol/transport support is decided only by the live fixture replay and becomes an honest qualified or blocking record.

- [ ] **Step 5: Commit transport behavior without records**

```bash
git add qualification/harnesses/mcp-transport/src qualification/harnesses/mcp-transport/tests/transport.rs
git commit -m "test: prove stateless MCP transport behavior"
```

### Task 8: MCP Runner And Qualification Evidence

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `tools/qualification/run_mcp.py`
- Create: `tests/qualification/test_run_mcp.py`
- Create: `qualification/records/mcp-transport.json`
- Create: `docs/qualification/rust/mcp-transport.md`

**Interfaces:**
- Consumes: Task 5 `ToolRoots`, `discover_tool_roots`, `safe_subprocess_env`, and `run_owned_process`; Tasks 1, 3, 6, and 7; `qualification/compatibility/mcp-transport.json`; `compatibility/authorities/mcp/2026-07-28/schema.json` and its `schema.source.json`; `compatibility/snapshots/mcp.json`; the corrected protocol/route fixtures; exactly the four `compatibility/fixtures/mcp/<name>/{error.input.json,success.input.json,wire.error.json,wire.success.json}` leaves in each of the 39 direct-child tool directories; and only the projected contract ids `cli.script.hieronymus-mcp`, `http.route.post.mcp`, `http.route.post.api.mcp.operation`, and every `mcp.tool.*`. No intermediate grouping directory is accepted. The runner never consumes or fingerprints the whole mutable manifest.
- Produces Python: `run(repo_root: Path, work_root: Path, *, executable: Path) -> QualificationRecord` for fake-injected unit tests and `run_live(repo_root: Path, work_root: Path, *, original_env: Mapping[str, str]) -> QualificationRecord` for the opt-in CLI; `run_live` rejects missing `HIERONYMUS_QUALIFICATION_LIVE=1` before Cargo execution.
- Produces private handoff: `_live_process_context(repo_root: Path, work_root: Path, original_env: Mapping[str, str]) -> tuple[ToolRoots, Path, dict[str, str]]`, returning discovered tool roots, `qualification/.artifacts/cargo-target/mcp-transport`, and the sanitized child environment in that order.
- Produces canonical `qualified` only when all seventeen MCP criteria pass, including `official-schema-envelopes` and `header-mismatch-errors`; otherwise exact Task 2 `blocked` consequence.

- [ ] **Step 1: Write failing fake-only runner tests**

```python
def test_mcp_runner_consumes_corrected_oracle(tmp_path: Path) -> None:
    executable = write_fake_executable(tmp_path)
    record = run(ROOT, tmp_path, executable=executable)
    assert "compatibility/authorities/mcp/2026-07-28/schema.json" in record.input_paths
    assert "compatibility/authorities/mcp/2026-07-28/schema.source.json" in record.input_paths
    assert "qualification/compatibility/mcp-transport.json" in record.input_paths
    assert "tools/qualification/projections.py" in record.input_paths
    assert "compatibility/manifest.json" not in record.input_paths
    assert "compatibility/snapshots/state.json" not in record.input_paths
    assert "compatibility/snapshots/mcp.json" in record.input_paths
    assert "compatibility/fixtures/mcp/protocol.json" in record.input_paths
    assert "compatibility/fixtures/http/route-cases.json" in record.input_paths
    assert "compatibility/fixtures/mcp/hieronymus_status/success.input.json" in record.input_paths
    assert len(record.input_paths) == 180
    assert sum(
        path.startswith("compatibility/fixtures/mcp/")
        and path.endswith(
            ("error.input.json", "success.input.json", "wire.error.json", "wire.success.json")
        )
        for path in record.input_paths
    ) == 156
    assert all(
        PurePosixPath(path).parts[3] != "tools"
        for path in record.input_paths
        if path.startswith("compatibility/fixtures/mcp/hieronymus_")
    )
    assert {item.criterion for item in record.evidence} == set(
        REQUIRED_CRITERIA["mcp-transport"]
    )
    assert len(record.evidence) == 17
    assert projection_issues(ROOT)["mcp-transport"] == ()


def test_mcp_failure_preserves_adr_0015(tmp_path: Path) -> None:
    executable = write_fake_executable(
        tmp_path, failed_criteria=("official-schema-envelopes",)
    )
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "blocked"
    assert record.consequence == FAILURE_CONSEQUENCES["mcp-transport"]
```

Add `test_mcp_live_context_discovers_before_sanitizing`: monkeypatch `run_mcp.discover_tool_roots` and `run_mcp.safe_subprocess_env`, call `_live_process_context(ROOT, tmp_path, original_env)`, and assert the call order is exactly `("discover", "sanitize")`, the identical returned `ToolRoots` is passed to sanitization, `cargo_offline` is true, and `cargo_target_dir == ROOT / "qualification/.artifacts/cargo-target/mcp-transport"`.

Add `test_mcp_live_children_reuse_safe_environment`: inject a fixture-backed `run_owned_process` spy into `run_live`, return successful receipts for the bounded Cargo and transport calls, and assert every captured call receives the identical environment object returned by `safe_subprocess_env`, every Cargo argv starts with `str(tool_roots.cargo_invocation)`, and no call receives the original environment mapping.

- [ ] **Step 2: Run the unit tests and verify RED without invoking Cargo**

Run: `uv run pytest tests/qualification/test_run_mcp.py -v`

Expected: FAIL importing `tools.qualification.run_mcp`; no Cargo command runs.

- [ ] **Step 3: Implement the fake-injectable live runner**

`run_mcp.run_live` copies the caller-supplied `original_env`, calls `discover_tool_roots` before any HOME/XDG rewrite, derives the exact MCP Cargo target path, and calls `safe_subprocess_env(work_root, cargo_offline=True, tool_roots=tool_roots, cargo_target_dir=cargo_target_dir)`. The module CLI supplies `dict(os.environ)` to `run_live`. `run_mcp.run` first requires `projection_issues(repo_root)["mcp-transport"] == ()`, loads contract ids/fields only from the verified MCP projection, calls Task 1's offline `authority_issues`, and validates every candidate/fixture success or protocol error through `definition_issues`; it never accepts a URL or opens the network. It then uses `qualification/.artifacts/work/mcp-transport`, starts each transport with a 20-second ready/response timeout, replays every target transport case, compares tool lists by canonical JSON digest after explicitly mapping wire `inputSchema` to snapshot internal `input_schema`, compares one success and one error tool call across transports, verifies `/api/mcp/fixture` is absent, and gathers locked dependencies with:

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 metadata --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --format-version 1
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 tree --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked -e features
```

Every Cargo argv begins with `str(tool_roots.cargo_invocation)`, never the resolved rustup target or a bare `cargo`. Every Cargo and MCP harness child goes through `run_owned_process` with the same sanitized environment and risk-specific target. The runner deletes ready files/logs/target output in `finally`, verifies all immutable compatibility inputs are byte-identical, and records the corrected oracle commit/digest. The digest includes `projections.py`, the canonical MCP projection, raw schema bytes, pin metadata, MCP snapshot, protocol, route cases, and the exact 156 policy leaves under `compatibility/fixtures/mcp/<name>/`; it excludes the whole mutable manifest/state sources. Before execution, it reruns Task 3's manifest-fixture/path-inventory equality and refuses any missing leaf, extra policy path, synthesized factory-only path, or intermediate grouping directory. It requires the projection's exact 42 contract ids/fields, exact `cacheScope`, `ttlMs`, `resultType`, `inputSchema`, configured serverInfo omission, two success/eleven failure ids, all seven `-32020` bodies, the one coherent `-32022` body/data, and the exact raw-leaf mutation invariant. It compares exact route-case status/body digests and never claims generic Host/auth routing beyond the frozen `POST /mcp` cases. It records Cargo/Rust versions and basenames only; no `ToolRoots` path enters JSON, Markdown, or raw-log output.

Run: `HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_mcp --write`

Expected: exit `0` means a complete, sanitized record was written. The record itself says `qualified` or `blocked`; a blocking candidate is retained rather than rewritten to pass.

- [ ] **Step 4: Verify unit tests, opt-in live evidence, and formatting**

Run: `uv run pytest tests/qualification/test_run_mcp.py tests/qualification/test_model.py tests/qualification/test_render.py -v`

Run: `uv run ruff check tools/qualification/run_mcp.py tests/qualification/test_run_mcp.py`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --check`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: tests, Ruff, rustfmt, and Clippy pass; record validation reports all seventeen criteria, current MCP projection, no official-schema error, stale authority/input, secret, or path leak. A temporary unrelated `test_ownership`/`state["tests"]` regeneration leaves the measured digest unchanged, while changing a projected MCP contract field blocks before record write.

- [ ] **Step 5: Commit the MCP qualification record**

```bash
git add tools/qualification/run_mcp.py tests/qualification/test_run_mcp.py qualification/records/mcp-transport.json docs/qualification/rust/mcp-transport.md
git commit -m "test: qualify MCP transport candidate"
```

### Task 9: Semantic Acquisition, Corpus, And Locked Candidate Build

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/fixtures/semantic-corpus.json`
- Create: `qualification/harnesses/semantic-native/Cargo.toml`
- Create: `qualification/harnesses/semantic-native/Cargo.lock`
- Create: `qualification/harnesses/semantic-native/src/lib.rs`
- Create: `qualification/harnesses/semantic-native/src/corpus.rs`
- Create: `qualification/harnesses/semantic-native/tests/corpus.rs`
- Modify: `qualification/prerequisites.json`
- Modify: `tools/qualification/acquire.py`
- Modify: `tests/qualification/test_acquire.py`

**Interfaces:**
- Consumes: corrected Task 1 MCP RAG/recall success inputs at `compatibility/fixtures/mcp/hieronymus_rag_search/success.input.json` and `compatibility/fixtures/mcp/hieronymus_recall/success.input.json` as fixed query seeds, plus the Task 4 acquisition boundary.
- Produces: `generate_corpus(spec: &CorpusSpec) -> anyhow::Result<(Vec<Chunk>, Vec<Query>)>` with exactly 10,000 chunks across 100 series and 50 queries.
- Produces: verified model/runtime directories plus standalone exact candidate manifest/lockfile; no semantic decision or record.

- [ ] **Step 1: Write failing deterministic-corpus and safe-acquisition tests**

```rust
#[test]
fn corpus_is_exactly_ten_thousand_chunks_and_fifty_queries() -> anyhow::Result<()> {
    let spec = CorpusSpec::load(fixture("qualification/fixtures/semantic-corpus.json"))?;
    let (chunks, queries) = generate_corpus(&spec)?;
    assert_eq!(chunks.len(), 10_000);
    assert_eq!(queries.len(), 50);
    assert_eq!(chunks.iter().map(|c| &c.series_slug).collect::<BTreeSet<_>>().len(), 100);
    let (chunks_again, queries_again) = generate_corpus(&spec)?;
    assert_eq!(
        corpus_digest(&chunks, &queries),
        corpus_digest(&chunks_again, &queries_again),
    );
    Ok(())
}

```

In `tests/qualification/test_acquire.py`, use an in-memory ONNX archive with the official top directory and relative library symlink; assert the validated chain resolves within the extraction root. Add RED cases for `../` traversal, absolute links, link cycles, links escaping the top directory, and checksum mismatch. Add one accepted signed redirect to exact host `us.aws.cdn.hf.co` and one rejected redirect to `us.aws.cdn.hf.co.attacker.invalid`.

- [ ] **Step 2: Run the new acquisition tests RED before implementation**

Run: `uv run pytest tests/qualification/test_acquire.py -v`

Expected: FAIL on missing ONNX runtime acquisition/extraction behavior; redirect, checksum, traversal, relative-symlink, cycle, and escape cases all execute without public network access.

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --no-default-features`

Expected: FAIL because `qualification/harnesses/semantic-native/Cargo.toml` does not exist.

- [ ] **Step 3: Define the exact deterministic corpus recipe**

Create `qualification/fixtures/semantic-corpus.json`:

```json
{
  "schema_version": 1,
  "seed": "hieronymus-semantic-qualification-v1",
  "series_count": 100,
  "chunks_per_series": 100,
  "query_count": 50,
  "embedding_dimensions": 384,
  "generation_ids": ["generation-a", "generation-b"],
  "series_slug_format": "qualification-series-{index:03}",
  "chunk_text_format": "synthetic literary memory {series_index:03}/{chunk_index:03}",
  "query_selection": "queries 0..49 target series 0..49 and chunk index (query_index * 17) % 100"
}
```

Corpus generation uses SHA-256 of `seed || series_index_be || chunk_index_be` for deterministic synthetic token ids, checksum, and fake vector values. Every query carries an explicit eligible `series_slug`; generated decoys reuse token patterns in other series so post-filter over-fetch loses results and the test detects it. The semantic record stores the resulting corpus digest; input fingerprinting includes both this recipe and `corpus.rs`, so a generator change makes the record stale.

- [ ] **Step 4: Add the exact native runtime prerequisite and atomic acquisition**

Extend `qualification/prerequisites.json` with:

```json
"onnx_runtime": {
  "version": "1.28.0",
  "target": "linux-x64",
  "url": "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-linux-x64-1.28.0.tgz",
  "sha256": "a3e1b79d7bb1bf09696ce675f49e4064e6c81f6202b8225624fff0e93f8d6407",
  "library": "lib/libonnxruntime.so"
}
```

Extend `tools.qualification.acquire` with `onnx-runtime`. It downloads to `qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0.tgz.part`, permits only HTTPS redirects through `github.com`, `release-assets.githubusercontent.com`, and `objects.githubusercontent.com`, and verifies SHA-256 before extraction. Require exactly one archive top directory named `onnxruntime-linux-x64-1.28.0`; strip only that component. Extract regular files/directories first, then create relative symlinks only after lexically resolving every hop, rejecting absolute targets, `..` escape, cycles, dangling links, device entries, hard links, and any final target outside the extraction root. Validate the required `lib/libonnxruntime.so` chain and atomically rename the completed directory.

- [ ] **Step 5: Run the complete acquisition suite GREEN**

Run: `uv run pytest tests/qualification/test_acquire.py -v`

Expected: PASS; allowed/disallowed redirects, checksum mismatch, traversal, official top directory, valid relative symlink chain, cycle, dangling/absolute/escaping link, device, and hard-link cases are all proven before acquisition code is committed.

- [ ] **Step 6: Create the feature-separated candidate manifest**

```toml
[package]
name = "hieronymus-semantic-native-qualification"
version = "0.0.0"
edition = "2024"
rust-version = "1.96"
publish = false

[features]
default = ["semantic-native"]
semantic-native = [
  "dep:arrow-array",
  "dep:arrow-schema",
  "dep:lancedb",
  "dep:ort",
]

[dependencies]
anyhow = "1"
arrow-array = { version = "=58.3.0", optional = true }
arrow-schema = { version = "=58.3.0", optional = true }
futures = "0.3"
lancedb = { version = "=0.37.1", default-features = false, optional = true }
ort = { version = "=2.0.0-rc.13", default-features = false, features = ["api-28", "load-dynamic", "ndarray", "std", "tracing"], optional = true }
rusqlite = { version = "=0.40.2", default-features = false, features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
tempfile = "3"
tokio = { version = "1", features = ["macros", "process", "rt-multi-thread", "sync", "time"] }

[dev-dependencies]
proptest = "1"
```

`--no-default-features` compiles the FTS path without LanceDB, Arrow, ndarray, or ort references. Avoid a direct `ndarray` dependency: use the `ort` rc.13 re-export/input macros so no cross-version array values cross the public boundary. The native runtime is loaded explicitly from the verified acquisition path; no build script downloads ONNX Runtime.

- [ ] **Step 7: Acquire dependencies once and prove the candidate builds offline**

Networked acquisition prerequisites:

```bash
uv run python -m tools.qualification.acquire semantic-model
uv run python -m tools.qualification.acquire onnx-runtime
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/semantic-native/Cargo.toml
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native cargo +1.96.0 fetch --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked
```

Offline replay:

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --no-default-features --target x86_64-unknown-linux-gnu
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu
```

Expected: both feature boundaries compile without network access and acquisition tests prove the exact redirect/extraction policy.

- [ ] **Step 8: Commit the acquisition and candidate boundary**

```bash
git add qualification/fixtures/semantic-corpus.json qualification/harnesses/semantic-native/Cargo.toml qualification/harnesses/semantic-native/Cargo.lock qualification/harnesses/semantic-native/src/lib.rs qualification/harnesses/semantic-native/src/corpus.rs qualification/harnesses/semantic-native/tests/corpus.rs qualification/prerequisites.json tools/qualification/acquire.py tests/qualification/test_acquire.py
git commit -m "test: lock semantic native qualification candidate"
```

### Task 10: Semantic ANN Index And Pre-Filter Proof

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/harnesses/semantic-native/src/model.rs`
- Create: `qualification/harnesses/semantic-native/src/index.rs`
- Create: `qualification/harnesses/semantic-native/tests/index.rs`

**Interfaces:**
- Consumes: Task 9 model/runtime, deterministic corpus, candidate lockfile, and LanceDB candidate.
- Produces: `OnnxEmbeddingProvider::load(runtime: &Path, model: &Path, expected_sha256: &str) -> anyhow::Result<Self>`.
- Produces: `async fn GenerationIndex::create(root: &Path, identity: ModelIdentity, generation: &str) -> anyhow::Result<Self>`, `create_ann_index(&mut self)`, `explain_prefiltered_search(&self, series_slug: &str, vector: &[f32], limit: usize)`, and append/search/delete/verify/activate methods.
- Proof rule: `series-prefilter-before-ann` passes only with a created ANN index, an adversarial cardinality/top-k result, and checked explain/analyze output showing the `series_slug` predicate below ANN nearest-neighbor execution; flat scan or post-filter/over-fetch is a failure.
- Test helpers in `tests/index.rs`: `populated_adversarial_index() -> anyhow::Result<GenerationIndex>` creates eligible ids `1..=10` plus 100 closer ineligible decoys; `needle() -> Vec<f32>` returns the fixed 384-float query; `expected_eligible_ids() -> Vec<i64>` returns `1..=10`.

- [ ] **Step 1: Write failing ANN creation and adversarial pre-filter tests**

```rust
#[tokio::test]
async fn search_uses_series_prefilter_before_ann() -> anyhow::Result<()> {
    let mut index = populated_adversarial_index().await?;
    index.create_ann_index().await?;
    let plan = index.explain_prefiltered_search("eligible", &needle(), 10).await?;
    assert!(plan.ann_index_used);
    assert!(plan.series_predicate_below_ann);
    assert_eq!(plan.eligible_cardinality, 10);
    let hits = index.search("eligible", &needle(), 10).await?;
    assert_eq!(hits.len(), 10);
    assert_eq!(hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(), expected_eligible_ids());
    Ok(())
}
```

Populate 10 eligible vectors whose distances rank below 100 closer ineligible decoys. Set `limit=10` and the candidate's ANN probe parameter low enough that post-filtering a global top-k returns fewer than 10; require all 10 exact eligible ids. The checked explain/analyze plan stores normalized operator/predicate/index names and cardinalities, never raw vectors.

- [ ] **Step 2: Run the ANN test and verify RED**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu --test index`

Expected: FAIL because the ANN index and explainable pre-filter path do not exist.

- [ ] **Step 3: Implement the model and ANN boundary**

`OnnxEmbeddingProvider` verifies checksum, loads the verified runtime, mean-pools, L2-normalizes, and returns an owned `Vec<f32>`; all ndarray values remain internal to ort's re-export. `GenerationIndex` stores chunk id, series slug, checksum, generation, and 384-float vector, creates a cosine IVF-PQ ANN index on the vector column with 16 partitions, waits until index statistics report all 10,000 rows indexed, and issues the series predicate as part of the ANN query. Evidence records index type/name, indexed-row count, distance metric, normalized plan operators, predicate placement, eligible cardinality, requested top-k, and returned count. It rejects any plan lacking both ANN index use and predicate pushdown.

- [ ] **Step 4: Prove insert/search/delete and generation isolation**

Add tests that insert a synthetic eligible chunk, find it through ANN, delete it, and prove absence after index refresh; create an incomplete second generation and prove all searches stay on the active first generation. Run the complete 10,000-chunk/50-query corpus twice and require identical corpus/index-input digests.

- [ ] **Step 5: Run offline tests and commit ANN behavior**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu --test index`

Expected: PASS with an actual ANN index, exact adversarial eligible ids/cardinality, and normalized explain/analyze proof; disabling index creation or moving the predicate to post-filter makes the test fail.

```bash
git add qualification/harnesses/semantic-native/src/model.rs qualification/harnesses/semantic-native/src/index.rs qualification/harnesses/semantic-native/tests/index.rs
git commit -m "test: prove semantic ANN prefiltering"
```

### Task 11: Semantic Durable Recovery And FTS Fallback

**Complexity:** High, 3–4 hours.

**Files:**
- Create: `qualification/harnesses/semantic-native/src/main.rs`
- Create: `qualification/harnesses/semantic-native/src/scenario.rs`
- Create: `qualification/harnesses/semantic-native/src/fts.rs`
- Create: `qualification/harnesses/semantic-native/tests/recovery.rs`
- Create: `qualification/harnesses/semantic-native/tests/fts.rs`

**Interfaces:**
- Consumes: Tasks 9–10 model/runtime, corpus, lockfile, and ANN index behavior.
- Produces Rust CLI: `semantic-native scenario --work-dir <path> --state-db <path> --mode complete|crash|cancel --generation <id> --stop-after <count>`.
- Produces Rust CLI: `semantic-native query --work-dir <path> --series <slug> --query-index <0..49>`.
- Produces Rust CLI without native features: `semantic-native fts-fallback --work-dir <path> --series <slug> --query-index <0..49>`.
- Produces: durable SQLite job/generation state with leases, counters, cancellation, and recovery; filesystem progress is forbidden.
- Test helpers in `tests/recovery.rs`: `run_complete`, `run_crash`, `run_cancel`, `resume`, `read_active_generation`, `written_count`, `generation_status`, and `run_complete_with_probe` operate only beneath a supplied temp directory; crash uses a core-disabled owned child process group. `TransactionProbe::native_io_while_write_transaction() -> usize` exposes the violation counter.
- Test helpers in `tests/fts.rs`: `expected_fts_id_digests`, `run_all_fts_queries`, `digest_receipts`, and `delete_and_rebuild_fts` consume the deterministic corpus recipe and operate only on a temporary SQLite database.

- [ ] **Step 1: Write failing durable recovery, transaction-boundary, and FTS tests**

```rust
#[tokio::test]
async fn crash_leaves_previous_generation_active_and_resume_is_exact() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    run_complete(root.path(), "generation-a").await?;
    let crash = run_crash(root.path(), "generation-b", 4_200).await;
    assert!(crash.is_err());
    assert_eq!(read_active_generation(root.path())?, "generation-a");
    resume(root.path(), "generation-b").await?;
    assert_eq!(written_count(root.path(), "generation-b")?, 10_000);
    Ok(())
}

#[tokio::test]
async fn cancellation_never_activates_cancelled_generation() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    run_complete(root.path(), "generation-a").await?;
    run_cancel(root.path(), "generation-b", 3_200).await?;
    assert_eq!(read_active_generation(root.path())?, "generation-a");
    assert_eq!(generation_status(root.path(), "generation-b")?, "cancelled");
    Ok(())
}
#[tokio::test]
async fn native_io_never_runs_inside_sqlite_write_transaction() -> anyhow::Result<()> {
    let probe = TransactionProbe::new();
    run_complete_with_probe(&probe).await?;
    assert_eq!(probe.native_io_while_write_transaction(), 0);
    Ok(())
}

#[test]
fn fts_fallback_is_nonempty_isolated_and_rebuild_equivalent() -> anyhow::Result<()> {
    let expected = expected_fts_id_digests();
    let before = run_all_fts_queries()?;
    assert!(before.iter().all(|receipt| receipt.eligible_count > 0));
    assert!(before.iter().all(|receipt| receipt.cross_series_count == 0));
    assert_eq!(digest_receipts(&before), expected);
    delete_and_rebuild_fts()?;
    assert_eq!(digest_receipts(&run_all_fts_queries()?), expected);
    Ok(())
}
```

- [ ] **Step 2: Run the focused tests and verify missing scenario/runner failures**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu --test recovery --test fts`

Expected: FAIL because the recovery test target and scenario module do not exist.

- [ ] **Step 3: Implement SQLite-owned job/generation state and transaction-free native I/O**

Create these qualification-only SQLite tables in `state-db`:

```sql
create table semantic_generations (
    generation_id text primary key,
    status text not null check (status in ('building','ready','active','superseded','cancelled','failed')),
    model_digest text not null,
    expected_count integer not null,
    written_count integer not null default 0,
    active integer not null default 0 check (active in (0,1))
);
create unique index one_active_semantic_generation
    on semantic_generations(active) where active = 1;
create table semantic_jobs (
    job_id text primary key,
    generation_id text not null references semantic_generations(generation_id),
    status text not null check (status in ('queued','running','cancel_requested','cancelled','complete','failed')),
    next_batch integer not null,
    completed_batches integer not null,
    lease_owner text,
    lease_expires_unix_ms integer,
    cancel_requested integer not null default 0 check (cancel_requested in (0,1))
);
```

For each batch: claim/renew lease and read cursor in a short committed SQLite write transaction; close the transaction; run ONNX and LanceDB I/O; then open a new short transaction to compare lease owner, persist written counters/checksums, and advance `next_batch`. `TransactionProbe` wraps every SQLite write transaction and every native call; entering ONNX/LanceDB with write depth nonzero is a hard failure and increments a recorded violation counter. No filesystem progress/manifest controls recovery or activation.

`--mode crash --stop-after 4200` aborts only after the batch receipt is committed. Resume takes an expired lease, verifies persisted counters against LanceDB ids/checksums, and continues exactly once. Cancellation is a committed request flag observed between batches; it closes handles, commits `cancelled`, clears the lease, and never activates that generation. Activation is one SQLite transaction after 10,000 rows, ANN/index verification, and sample queries succeed.

The CLI rejects work directories outside `qualification/.artifacts/work/semantic-native`, symlinks, model/runtime checksum mismatch, mixed dimensions, mixed generation ids, and non-loopback acquisition URLs before writing.

- [ ] **Step 4: Implement and prove the strengthened FTS fallback**

Create a disposable external-content FTS5 table over generated SQLite rows. `expected_fts_id_digests()` independently derives each query's eligible ids from the deterministic corpus recipe before opening SQLite, requires at least one expected id for every query, and returns 50 ordered SHA-256 digests. FTS search includes `series_slug` in the SQLite query, requires the same nonempty digests with zero cross-series ids, then deletes/rebuilds the FTS index and requires identical digests again. Compile this path with `--no-default-features`; model/runtime/index paths are absent and a download probe must remain untouched.

- [ ] **Step 5: Run offline recovery/fallback tests and commit behavior**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --features semantic-native --target x86_64-unknown-linux-gnu --test recovery`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --no-default-features --target x86_64-unknown-linux-gnu --test fts`

Expected: durable lease/counter recovery, cancellation, active-generation isolation, zero native-I/O transaction violations, exact nonempty FTS id digests, series isolation, and rebuild equivalence all pass.

```bash
git add qualification/harnesses/semantic-native/src/main.rs qualification/harnesses/semantic-native/src/scenario.rs qualification/harnesses/semantic-native/src/fts.rs qualification/harnesses/semantic-native/tests/recovery.rs qualification/harnesses/semantic-native/tests/fts.rs
git commit -m "test: prove semantic recovery and FTS fallback"
```

### Task 12: Semantic Runner And Measured Evidence

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `tools/qualification/run_semantic.py`
- Create: `tests/qualification/test_run_semantic.py`
- Create: `qualification/records/semantic-native.json`
- Create: `docs/qualification/rust/semantic-native.md`

**Interfaces:**
- Consumes: Task 5 `ToolRoots`, `discover_tool_roots`, `safe_subprocess_env`, and `run_owned_process`; Tasks 9–11; and the exact frozen seed fixtures `compatibility/fixtures/mcp/hieronymus_rag_search/success.input.json` and `compatibility/fixtures/mcp/hieronymus_recall/success.input.json`. These are direct children of the MCP fixture root with no intermediate grouping directory.
- Produces: `run(repo_root: Path, work_root: Path, *, executable: Path) -> QualificationRecord` for fake-injected pytest and `run_live(repo_root: Path, work_root: Path, *, original_env: Mapping[str, str]) -> QualificationRecord` for the opt-in CLI; `run_live` rejects missing `HIERONYMUS_QUALIFICATION_LIVE=1`.
- Produces private handoff: `_live_process_context(repo_root: Path, work_root: Path, original_env: Mapping[str, str]) -> tuple[ToolRoots, Path, dict[str, str]]`, returning discovered tool roots, `qualification/.artifacts/cargo-target/semantic-native`, and the sanitized child environment in that order.
- Produces: `semantic-enabled` only when all sixteen semantic criteria pass; any other complete result is `fts-only` and never blocks Linux release.

- [ ] **Step 1: Write failing fake-only decision and evidence tests**

```python
def test_any_semantic_failure_selects_fts_only(tmp_path: Path) -> None:
    executable = write_fake_executable(
        tmp_path, failed_criteria=("series-prefilter-before-ann",)
    )
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "fts-only"
    assert "do not block" in record.consequence


def test_prefilter_and_recovery_evidence_are_required(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=write_fake_executable(tmp_path))
    criteria = {item.criterion for item in record.evidence}
    assert "series-prefilter-before-ann" in criteria
    assert "no-sqlite-write-across-native-io" in criteria
    assert "compatibility/fixtures/mcp/hieronymus_rag_search/success.input.json" in record.input_paths
    assert "compatibility/fixtures/mcp/hieronymus_recall/success.input.json" in record.input_paths
    assert len(record.input_paths) == 28
    assert all(
        PurePosixPath(path).parts[3] != "tools"
        for path in record.input_paths
        if path.startswith("compatibility/fixtures/mcp/hieronymus_")
    )
```

Add `test_semantic_live_context_discovers_before_sanitizing`: monkeypatch `run_semantic.discover_tool_roots` and `run_semantic.safe_subprocess_env`, call `_live_process_context(ROOT, tmp_path, original_env)`, and assert call order `("discover", "sanitize")`, identity of the passed `ToolRoots`, `cargo_offline is True`, and target `ROOT / "qualification/.artifacts/cargo-target/semantic-native"`.

Add `test_semantic_live_children_reuse_safe_environment`: inject a criterion-fixture-backed `run_owned_process` spy into `run_live`; assert every Cargo, ONNX, LanceDB, SQLite, crash, cancellation, FTS, `ldd`, and installed-binary call receives the identical environment object returned by `safe_subprocess_env`, every Cargo argv starts with `str(tool_roots.cargo_invocation)`, and no call receives the original environment mapping.

- [ ] **Step 2: Run Python tests and verify RED without native prerequisites**

Run: `uv run pytest tests/qualification/test_run_semantic.py -v`

Expected: FAIL importing `tools.qualification.run_semantic`; no Rust/native process runs.

- [ ] **Step 3: Implement the full measured runner**

`run_semantic.run_live` copies the caller-supplied `original_env`, calls `discover_tool_roots` before any sanitization, then passes the returned object and exact semantic Cargo target to `safe_subprocess_env`; its module CLI supplies `dict(os.environ)`. `run_semantic.run` reuses the existing sanitized `CARGO_TARGET_DIR` at `qualification/.artifacts/cargo-target/semantic-native` without deleting or recreating it, and passes that exact directory to every child. Basic-metrics amendment (2026-09-02, owner ruling): because the semantic candidate is unreleased and the record will be regenerated at cutover, `locked-native-build` is proven as a successful `--release --locked` incremental build over the warm cache; Task 9's clean offline `cargo check --locked` evidence remains the clean-build authority, and no test may assert delete-recreate clean-build behavior. It then executes the following exact evidence sequence against fresh disposable paths:

1. Build the semantic binary with `--release --locked --features semantic-native` and record binary/library sizes plus `ldd` basenames.
2. Copy the binary, `libonnxruntime.so`, model, and an empty index root beneath `qualification/.artifacts/install/semantic-native`; run one query; remove the install directory; assert it no longer exists.
3. Verify model checksum before load and record 384 dimensions plus normalized output digest.
4. Build 10,000 chunks for `generation-a`, create and await the ANN index, and record normalized explain/analyze evidence.
5. Run the adversarial top-k/cardinality case and all 50 series-scoped queries; require exact eligible ids, pre-filter-before-ANN proof, and zero cross-series hits.
6. Insert one synthetic chunk, find it through ANN, delete it, and prove it is absent after refresh.
7. Create incomplete `generation-b`; prove searches remain on `generation-a`.
8. Record SQLite job/generation rows, leases, batch counters, and exactly zero native-I/O-inside-write-transaction violations.
9. With core dumps disabled and an owned process group, abort `generation-b` at 4,200, expire/take its lease, resume to 10,000, verify, activate once, and prove no duplicate ids.
10. Cancel a new generation at 3,200 through durable SQLite state and prove `generation-b` remains active.
11. Build the FTS binary with `--release --locked --no-default-features`, omit model/index paths, and prove all 50 nonempty expected eligible-id digests, series isolation, delete/rebuild equivalence, and no download attempt.

Every Cargo argv begins with `str(tool_roots.cargo_invocation)`. Every Cargo, ONNX, LanceDB scenario, crash, cancellation, FTS, `ldd`, and installed-binary child uses `run_owned_process` with the same sanitized environment and risk-specific target, has a 20-minute total timeout and a 2-minute no-progress timeout, has `RLIMIT_CORE=0`, and is terminated/reaped by owned process group in `finally`. A build or prerequisite failure marks itself `fail`; causally dependent criteria become `not-run` with the exact failing criterion in `not_run_reason`. The record remains complete and selects `fts-only`. Environment evidence stores only versions, basenames, and digests; all six absolute `ToolRoots` fields are forbidden by redaction.

- [ ] **Step 4: Run the opt-in semantic qualification and write both records**

Run:

```bash
HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_semantic --write
```

Expected: exit `0` after atomically writing a complete `qualification/records/semantic-native.json` and matching Markdown. The JSON decision is exactly `semantic-enabled` when all criteria pass or `fts-only` when any criterion fails. No outcome from this command is `blocked`.

- [ ] **Step 5: Verify cleanup and offline reproducibility**

Run: `uv run python -m tools.qualification.clean`

Expected: dry-run lists only work/log/install/Cargo-target/frontend-dist descendants and does not list the acquired model/runtime directory.

Run: `uv run python -m tools.qualification.clean --apply`

Expected: transient paths are absent; model/runtime acquisitions remain.

Run: `uv run python -m tools.qualification.validate qualification/records/semantic-native.json`

Run: `uv run python -m tools.qualification.render --check qualification/records/semantic-native.json docs/qualification/rust/semantic-native.md`

Expected: both commands pass without running Cargo, opening a network connection, or requiring transient artifacts; input fingerprints, redaction, evidence completeness, and generated Markdown match.

- [ ] **Step 6: Run focused code-quality checks**

Run: `uv run pytest tests/qualification/test_run_semantic.py tests/qualification/test_model.py tests/qualification/test_clean.py -v`

Run: `uv run ruff check tools/qualification/run_semantic.py tests/qualification/test_run_semantic.py`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/semantic-native/Cargo.toml --check`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --all-targets --features semantic-native --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: Python and Rust checks pass. Record validation accepts either measured semantic decision and rejects missing or leaked evidence.

- [ ] **Step 7: Commit the semantic qualification record**

```bash
git add tools/qualification/run_semantic.py tests/qualification/test_run_semantic.py qualification/records/semantic-native.json docs/qualification/rust/semantic-native.md
git commit -m "test: record semantic native qualification"
```

### Task 13: Frontend Candidate Manifest And Reproducible Bundle Build

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `qualification/harnesses/frontend-embedding/Cargo.toml`
- Create: `qualification/harnesses/frontend-embedding/Cargo.lock`
- Create: `qualification/harnesses/frontend-embedding/build.rs`
- Create: `qualification/harnesses/frontend-embedding/src/main.rs` containing only `fn main() {}` until Task 14.

**Interfaces:**
- Consumes: `frontend/package.json`, `frontend/bun.lock`, `frontend/vite.config.ts`, and all `frontend/src/**`.
- Produces: a Svelte bundle at `qualification/.artifacts/frontend-dist/current` and standalone rust-embed manifest/lockfile.
- Build invariant: the RustEmbed folder is manifest-relative `../../.artifacts/frontend-dist/current/`; `build.rs` resolves it from `CARGO_MANIFEST_DIR`, and every Cargo command sets `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding`.

- [ ] **Step 1: Verify RED for the missing candidate manifest and bundle**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked --release --target x86_64-unknown-linux-gnu`

Expected: FAIL because the frontend-embedding manifest does not exist.

- [ ] **Step 2: Create the standalone rust-embed candidate manifest**

```toml
[package]
name = "hieronymus-frontend-embedding-qualification"
version = "0.0.0"
edition = "2024"
rust-version = "1.96"
publish = false
build = "build.rs"

[[bin]]
name = "frontend-embedding"
path = "src/main.rs"

[dependencies]
anyhow = "1"
mime_guess = "2"
percent-encoding = "2"
rust-embed = { version = "=8.12.0", features = ["debug-embed", "deterministic-timestamps", "interpolate-folder-path"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"

[dev-dependencies]
tempfile = "3"
```

`build.rs` resolves `Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.artifacts/frontend-dist/current")`, canonicalizes it, requires `index.html` plus at least one `assets/` file, rejects symlinks escaping that root, and emits only exact `cargo:rerun-if-changed` paths. The runner, not the build script, builds Svelte before Cargo; no environment interpolation selects another asset root.

- [ ] **Step 3: Acquire Bun/Cargo dependencies and build without replay network**

One-time networked acquisition:

```bash
bun install --cwd frontend --frozen-lockfile
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding cargo +1.96.0 fetch --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked
```

Do not invoke a nonexistent Bun offline-install flag. For replay, reuse the lock-matched hydrated `frontend/node_modules`, clear `qualification/.artifacts/frontend-dist/current`, and execute the build in a Linux network namespace:

```bash
unshare --user --map-root-user --net -- bun run --cwd frontend build -- --outDir ../qualification/.artifacts/frontend-dist/current --emptyOutDir
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 build --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --target x86_64-unknown-linux-gnu
```

Expected: Bun reports `1.4.0`; `bun.lock` is unchanged; the build succeeds with no network namespace interface; exact file/byte digests are stable across two clean builds. If unprivileged network namespaces are unavailable, record the frontend build criterion failed—do not claim offline replay.

- [ ] **Step 4: Commit the independently reviewable build boundary**

```bash
git add qualification/harnesses/frontend-embedding/Cargo.toml qualification/harnesses/frontend-embedding/Cargo.lock qualification/harnesses/frontend-embedding/build.rs qualification/harnesses/frontend-embedding/src/main.rs
git commit -m "test: lock frontend embedding candidate"
```

### Task 14: Embedded Asset Resolution And Runtime Independence

**Complexity:** Medium, 2–3 hours.

**Files:**
- Modify: `qualification/harnesses/frontend-embedding/src/main.rs`
- Create: `qualification/harnesses/frontend-embedding/src/assets.rs`
- Create: `qualification/harnesses/frontend-embedding/tests/assets.rs`

**Interfaces:**
- Consumes: Task 13's canonical bundle and locked candidate.
- Produces: `AssetResponse resolve_asset(request_path: &str)` and CLI `frontend-embedding manifest|get --path <request-path>`.
- Ownership boundary: proves embedded path resolution, MIME/digests, SPA fallback, and filesystem independence only; it does not claim Host validation, bearer/session auth, CSRF, or full HTTP routing.

- [ ] **Step 1: Write failing embedded-resolution tests**

```rust
#[test]
fn embedded_index_and_spa_paths_resolve() {
    assert_eq!(resolve_asset("/").status, 200);
    assert!(resolve_asset("/admin/fixture").fallback);
    assert!(resolve_asset("/config/fixture").fallback);
}

#[test]
fn missing_real_asset_is_not_an_index_fallback() {
    let response = resolve_asset("/assets/qualification-missing.js");
    assert_eq!(response.status, 404);
    assert!(!response.fallback);
}
```

- [ ] **Step 2: Run the asset tests and verify RED**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked --release --target x86_64-unknown-linux-gnu --test assets`

Expected: FAIL because the resolver does not exist.

- [ ] **Step 3: Implement the embedded resolver with exact fallback rules**

```rust
#[derive(rust_embed::RustEmbed)]
#[folder = "../../.artifacts/frontend-dist/current/"]
struct FrontendAssets;

use sha2::Digest;

#[derive(Debug, Serialize)]
pub struct AssetResponse {
    pub status: u16,
    pub content_type: String,
    pub body_sha256: String,
    pub fallback: bool,
}

fn response(status: u16, key: &str, bytes: &[u8], fallback: bool) -> AssetResponse {
    AssetResponse {
        status,
        content_type: if key.ends_with(".html") {
            "text/html; charset=utf-8".to_owned()
        } else {
            mime_guess::from_path(key).first_or_octet_stream().to_string()
        },
        body_sha256: format!("{:x}", sha2::Sha256::digest(bytes)),
        fallback,
    }
}

fn empty_response(status: u16) -> AssetResponse {
    response(status, "response.json", b"", false)
}

pub fn resolve_asset(request_path: &str) -> AssetResponse {
    let lowercase = request_path.to_ascii_lowercase();
    if lowercase.contains("%2f") || lowercase.contains("%5c") || lowercase.contains("%00") {
        return empty_response(400);
    }
    let decoded = match percent_encoding::percent_decode_str(request_path).decode_utf8() {
        Ok(path) => path,
        Err(_) => return empty_response(400),
    };
    if decoded.contains('\\')
        || decoded.contains('\0')
        || decoded.split('/').any(|part| part == "..")
    {
        return empty_response(400);
    }
    let key = decoded.trim_start_matches('/');
    let key = if key.is_empty() { "index.html" } else { key };
    if let Some(asset) = FrontendAssets::get(key) {
        return response(200, key, asset.data.as_ref(), false);
    }
    if key.starts_with("assets/") {
        return empty_response(404);
    }
    let index = FrontendAssets::get("index.html").expect("build.rs verified index.html");
    response(200, "index.html", index.data.as_ref(), true)
}
```

The exact rules are: normalize one leading slash; reject `..`, backslash, percent-decoded separators, and NUL with 400; serve a present embedded file with its MIME type; return 404 for a missing path beneath `/assets/`; otherwise serve embedded `index.html`. The CLI returns only status, MIME, body digest, byte length, and fallback; it never prints bodies or claims HTTP security behavior.

- [ ] **Step 4: Prove filesystem/runtime independence under inaccessible asset roots**

After building, copy only the release binary into `qualification/.artifacts/install/frontend-embedding/bin`; move `qualification/.artifacts/frontend-dist/current` to `qualification/.artifacts/frontend-dist/quarantine/current`, then set the runner-owned quarantine directory to mode `000`. Execute every path probe under `strace -f -e trace=%file`; require the same response digests and no attempted open/stat/readlink beneath the original or quarantined asset root. Restore permissions and the asset root in `finally`, then remove the install/quarantine directories. Also require executable dependencies/process tree to contain no Bun, Node, or Python.

- [ ] **Step 5: Run behavior tests and commit**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked --release --target x86_64-unknown-linux-gnu --test assets`

Expected: PASS; missing bundle at compile time fails, embedded path behavior matches, and runtime asset-root access is unnecessary.

```bash
git add qualification/harnesses/frontend-embedding/src qualification/harnesses/frontend-embedding/tests/assets.rs
git commit -m "test: prove embedded frontend asset behavior"
```

### Task 15: Frontend Runner And Qualification Evidence

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `tools/qualification/run_frontend.py`
- Create: `tests/qualification/test_run_frontend.py`
- Create: `qualification/records/frontend-embedding.json`
- Create: `docs/qualification/rust/frontend-embedding.md`

**Interfaces:**
- Consumes: Task 5 `ToolRoots`, `discover_tool_roots`, `safe_subprocess_env`, and `run_owned_process`; Tasks 13–14; frontend source/lock/build config; and only the path/status/MIME/body shape from manifest ids `frontend.route.get.root`, `frontend.route.get.admin`, `frontend.route.get.admin.path`, `frontend.route.get.assets.path`, `frontend.route.get.config`, and `frontend.route.get.config.path` in `route-cases.json`.
- Produces: `run(repo_root: Path, work_root: Path, *, executable: Path) -> QualificationRecord` for fake-injected pytest and `run_live(repo_root: Path, work_root: Path, *, original_env: Mapping[str, str]) -> QualificationRecord` for the opt-in CLI; `run_live` rejects missing `HIERONYMUS_QUALIFICATION_LIVE=1`.
- Produces private handoff: `_live_process_context(repo_root: Path, work_root: Path, original_env: Mapping[str, str]) -> tuple[ToolRoots, Path, Path, dict[str, str]]`, returning discovered tool roots, lexical Bun invocation, `qualification/.artifacts/cargo-target/frontend-embedding`, and the sanitized child environment in that order.
- Produces private lookup: `_discover_bun_invocation(original_env: Mapping[str, str]) -> Path`, preserving the absolute lexical Bun invocation from the original PATH while validating its separately resolved target as a regular executable; neither path enters evidence.
- Produces: `qualified` only when every frontend criterion passes; otherwise exact Task 2 blocking consequence.

- [ ] **Step 1: Write failing fake-only runner tests**

```python
def test_frontend_runner_owns_embedded_path_contracts_only(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=write_fake_executable(tmp_path))
    assert set(record.contract_ids) == {
        "frontend.route.get.root",
        "frontend.route.get.admin",
        "frontend.route.get.admin.path",
        "frontend.route.get.assets.path",
        "frontend.route.get.config",
        "frontend.route.get.config.path",
    }
    assert "compatibility/fixtures/http/route-cases.json" in record.input_paths
    assert not {"http-host-validation", "browser-auth", "csrf"} & {
        item.criterion for item in record.evidence
    }


def test_frontend_failure_does_not_select_serve_dir(tmp_path: Path) -> None:
    executable = write_fake_executable(
        tmp_path, failed_criteria=("runtime-asset-root-inaccessible",)
    )
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "blocked"
    assert "embedded Svelte assets" in record.consequence
```

Add `test_frontend_live_context_discovers_before_sanitizing`: monkeypatch `run_frontend.discover_tool_roots`, `run_frontend._discover_bun_invocation`, and `run_frontend.safe_subprocess_env`, call `_live_process_context(ROOT, tmp_path, original_env)`, and assert call order `("discover-tools", "discover-bun", "sanitize")`, identity of the passed `ToolRoots`, preservation of the returned lexical Bun path, `cargo_offline is True`, and target `ROOT / "qualification/.artifacts/cargo-target/frontend-embedding"`.

Add `test_frontend_live_children_reuse_safe_environment`: inject a fixture-backed `run_owned_process` spy into `run_live`; assert every Bun build, Cargo, embedded-binary, `strace`, `ldd`, and process-tree call receives the identical environment object returned by `safe_subprocess_env`, Bun argv starts with the lexical path returned by `_discover_bun_invocation`, every Cargo argv starts with `str(tool_roots.cargo_invocation)`, and no call receives the original environment mapping.

- [ ] **Step 2: Run Python tests and verify RED without Bun or Cargo**

Run: `uv run pytest tests/qualification/test_run_frontend.py -v`

Expected: FAIL importing `tools.qualification.run_frontend`; no Bun/Cargo/native command runs.

- [ ] **Step 3: Implement the live embedding runner and objective criteria**

`run_frontend.run_live` copies the caller-supplied `original_env`, calls `discover_tool_roots` and `_discover_bun_invocation` before any sanitization, then passes the returned `ToolRoots` and exact frontend Cargo target to `safe_subprocess_env`; its module CLI supplies `dict(os.environ)`. `run_frontend.run` runs Task 13's network-isolated Bun build directly into the canonical artifact directory, fingerprints it, and builds release mode with:

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 build --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --target x86_64-unknown-linux-gnu
```

It then:

1. Records Bun version, frozen lock digest, bundle digest, and successful build.
2. Renames the canonical asset directory away, builds in fresh bounded `qualification/.artifacts/cargo-target/frontend-embedding-missing`, requires a stable compile-time missing-bundle failure, removes that target, then restores the asset root in `finally`.
3. Compares the embedded manifest's relative files, byte lengths, and SHA-256 values to the copied Vite output.
4. Replays only the six routes' embedded path/status/MIME/body expectations; excludes Host/auth/CSRF/router ownership from the record.
5. Performs Task 14's rename/permission-denial plus `strace` open proof with only the release binary present.
6. Requires `ldd` basenames and process tree to show no Bun, Node, or Python runtime.
7. Rejects any `.map` file and scans embedded bytes for `compat-secret-do-not-log`, `Authorization: Bearer`, `provider_key`, `/home/`, and `Yandex.Disk`.
8. Records release binary byte size without imposing an unapproved size threshold.
9. Runs Clippy with `-D warnings`, the canonical asset root, Cargo offline, and the bounded target before removing the asset root.

Every Cargo argv begins with `str(tool_roots.cargo_invocation)`. The Bun build, Cargo, embedded binary, `strace`, `ldd`, and process-tree probes all use `run_owned_process` with the same sanitized environment and risk-specific target; Bun is invoked by its absolute executable path discovered from the original PATH before sanitization, but that path is never serialized. The runner restores all renamed/permission-denied paths, then removes copied bundle, install directory, traces, and Cargo target in `finally`; it preserves only the canonical record and ignored Bun cache/node_modules. Evidence stores only versions, basenames, sizes, and digests—never any absolute tool root or invocation path.

- [ ] **Step 4: Run opt-in qualification and focused verification**

Run: `HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_frontend --write`

Expected: writes a complete `frontend-embedding.json` and generated Markdown. It records `qualified` or an honest `blocked` result; it never records filesystem serving or a runtime language dependency as an alternative.

Run: `uv run pytest tests/qualification/test_run_frontend.py tests/qualification/test_model.py tests/qualification/test_render.py -v`

Run: `uv run ruff check tools/qualification/run_frontend.py tests/qualification/test_run_frontend.py`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --check`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: Python tests, rustfmt, and the exact bounded Clippy command pass before cleanup; record validation reports exact path-contract ownership, complete criteria, and no sensitive data.

- [ ] **Step 5: Commit the frontend qualification record**

```bash
git add tools/qualification/run_frontend.py tests/qualification/test_run_frontend.py qualification/records/frontend-embedding.json docs/qualification/rust/frontend-embedding.md
git commit -m "test: qualify frontend asset embedding"
```

### Task 16: Legacy Database Candidate Manifest And Locked Build

**Complexity:** Low, 1–2 hours.

**Files:**
- Create: `qualification/harnesses/legacy-database-import/Cargo.toml`
- Create: `qualification/harnesses/legacy-database-import/Cargo.lock`
- Create: `qualification/harnesses/legacy-database-import/src/main.rs` containing only `fn main() {}` until Task 17.

**Interfaces:**
- Consumes: Rust 1.96.0 and rusqlite 0.40.2 candidate.
- Produces: standalone bundled-SQLite manifest and committed lockfile; no database behavior or evidence.
- Build invariant: every Cargo command sets `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import`.

- [ ] **Step 1: Verify RED before the candidate manifest exists**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --target x86_64-unknown-linux-gnu`

Expected: FAIL because the legacy-database-import manifest does not exist.

- [ ] **Step 2: Create the minimal bundled-SQLite candidate manifest**

```toml
[package]
name = "hieronymus-legacy-database-import-qualification"
version = "0.0.0"
edition = "2024"
rust-version = "1.96"
publish = false

[[bin]]
name = "legacy-database-import"
path = "src/main.rs"

[dependencies]
anyhow = "1"
rusqlite = { version = "=0.40.2", default-features = false, features = ["bundled", "backup", "serde_json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"

[dev-dependencies]
tempfile = "3"
```

The candidate deliberately uses bundled SQLite so the probe measures FTS5/file compatibility without silently depending on the workstation's SQLite library.

- [ ] **Step 3: Acquire, lock, and prove offline build**

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import cargo +1.96.0 generate-lockfile --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import cargo +1.96.0 fetch --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 check --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --target x86_64-unknown-linux-gnu
```

Expected: PASS without network access and no repository-local target directory.

- [ ] **Step 4: Commit the candidate boundary**

```bash
git add qualification/harnesses/legacy-database-import/Cargo.toml qualification/harnesses/legacy-database-import/Cargo.lock qualification/harnesses/legacy-database-import/src/main.rs
git commit -m "test: lock database import qualification candidate"
```

### Task 17: Read-Only Database Fixture Behavior

**Complexity:** High, 3–4 hours.

**Files:**
- Modify: `qualification/harnesses/legacy-database-import/src/main.rs`
- Create: `qualification/harnesses/legacy-database-import/src/classify.rs`
- Create: `qualification/harnesses/legacy-database-import/src/probe_import.rs`
- Create: `qualification/harnesses/legacy-database-import/src/report.rs`
- Create: `qualification/harnesses/legacy-database-import/tests/fixtures.rs`

**Interfaces:**
- Consumes: Task 3's verified `qualification/compatibility/legacy-database-import.json` and exactly the six frozen SQLite files under `compatibility/fixtures/database/`. It never reads schema/migration expectations directly from the whole mutable state snapshot.
- Produces: `classify_read_only(source: &Path, fixture_root: &Path, projection_contract: &Path) -> anyhow::Result<Classification>` and `probe_import(source: &Path, target: &Path, fixture_root: &Path, work_root: &Path, expected: &DatabaseContract) -> anyhow::Result<ProbeReceipt>`.
- Produces CLI: `legacy-database-import classify --fixture-root compatibility/fixtures/database --source-name <basename> --contract qualification/compatibility/legacy-database-import.json` and `probe-import ... --work-root <risk-work-root> --target-name <basename>`; arbitrary source/target/contract paths are not accepted.
- Test helpers in `tests/fixtures.rs`: `expected_cases()` returns `minimal-python.sqlite/supported-python/true`, `legacy-python.sqlite/supported-legacy-python/true`, `empty.sqlite/empty/false`, `partial-python.sqlite/partial-python/false`, `corrupt.sqlite/corrupt/false`, and `unknown-schema.sqlite/unknown-schema/false`; `fixture`, `fixture_root`, and `projection_contract` resolve checked-in inputs; `assert_rejected_source`, `assert_rejected_target`, and `assert_rejected_contract` invoke the CLI and require exit code `2` before SQLite opens.

- [ ] **Step 1: Write failing fixture-matrix and path-boundary tests**

```rust
#[test]
fn frozen_fixture_matrix_is_read_only() -> anyhow::Result<()> {
    for (name, classification, safe) in expected_cases() {
        let before = sha256(fixture(name))?;
        let actual = classify_read_only(&fixture(name), &fixture_root(), &projection_contract())?;
        assert_eq!((actual.name.as_str(), actual.safe_to_convert), (classification, safe));
        assert_eq!(sha256(fixture(name))?, before);
    }
    Ok(())
}

#[test]
fn cli_rejects_unowned_source_target_or_contract() {
    assert_rejected_source("../../user.sqlite");
    assert_rejected_target("../../sibling.sqlite");
    assert_rejected_contract("compatibility/snapshots/state.json");
}
```

- [ ] **Step 2: Run fixture tests and verify RED**

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --target x86_64-unknown-linux-gnu --test fixtures`

Expected: FAIL because classification/import and CLI root enforcement do not exist.

- [ ] **Step 3: Implement bounded classification and neutral probe import**

Open source fixtures with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`, immediately set `PRAGMA query_only=ON`, and never issue a source transaction or write pragma. Parse the closed Task 3 database projection, require its exact three contract ids and 12-field `database` object, and use every projected field: `object_contracts` maps each expectation to its contract id; `fixture` identifies the current fixture; `tables`, `columns`, `indexes`, `triggers`, `foreign_keys`, `row_counts`, and `representative_rows` define current schema/content checks; `migration_sources` and `application_migration_ledgers` define migration checks; and `variants` defines classification/preflight expectations. No direct fallback to `compatibility/snapshots/state.json` is allowed. It is a qualification classifier, not the production `StateClassifier`.

Canonicalize `fixture_root` and require it equals `repo_root/compatibility/fixtures/database`; accept `source-name` only when it is one of the six frozen basenames and its resolved path is a non-symlink direct child. Canonicalize `work_root` and require it is a non-symlink descendant of `qualification/.artifacts/work/legacy-database-import`; construct `target-name` beneath it and reject separators, `..`, existing symlinks, or any target outside that root.

For the two supported fixtures only, `probe_import` creates a new disposable target with this neutral schema:

```sql
create table probe_rows (
    source_table text not null,
    source_id text not null,
    payload_sha256 text not null,
    field_count integer not null,
    primary key (source_table, source_id)
);
create table probe_ledger (
    source_table text not null,
    source_id text not null,
    outcome text not null check (outcome in ('read', 'skipped', 'blocking')),
    reason_code text not null,
    primary key (source_table, source_id)
);
```

Read typed integers, reals, text, blobs, booleans, JSON, and documented timestamps; canonicalize each row in memory; store only its digest and field count. Every source row has exactly one ledger outcome. The probe explicitly covers series, sessions, strict terms/aliases/tags, concepts/facets, memories/crystals/links, RAG sources/chunks/tags/scopes, events/audit, and `memory_graph_migration_ledger`. It never attempts the production target rule schema or one-way cutover protocol.

- [ ] **Step 4: Implement objective FTS, ledger, and byte-identity checks**

For `minimal-python.sqlite`, compare table/column/index/trigger/foreign-key inventories, row counts, and representative-row digests with the verified projection; verify its migration-source and application-ledger expectations; run `PRAGMA integrity_check`, `PRAGMA foreign_key_check`, and one exact FTS query for strict terms, memories, concepts, crystals, and RAG chunks. For `legacy-python.sqlite`, require the projected legacy variant identity/fingerprint and full typed ledger accounting even when fewer tables are present.

Before and after every classify/import attempt, hash the source fixture and require equality. Hash every file in the fixture directory to prove no journal, WAL, sibling database, backup, or target appeared next to it. Unsupported/corrupt/partial/empty cases must fail before target creation with their frozen classification. Reports include only classification, counts, schema/object digests, FTS result id digests, ledger outcome counts, error code, and source byte-identity boolean.

- [ ] **Step 5: Run all fixture tests offline and commit behavior**

```bash
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 test --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --target x86_64-unknown-linux-gnu
```

Expected: bundled SQLite reports FTS5 enabled; the supported current and legacy cases produce exact neutral receipts; four non-convertible cases fail closed; all source and sibling-directory digests remain unchanged.

```bash
git add qualification/harnesses/legacy-database-import/src qualification/harnesses/legacy-database-import/tests/fixtures.rs
git commit -m "test: prove read-only database fixture import"
```

### Task 18: Database Runner And Qualification Evidence

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `tools/qualification/run_database.py`
- Create: `tests/qualification/test_run_database.py`
- Create: `qualification/records/legacy-database-import.json`
- Create: `docs/qualification/rust/legacy-database-import.md`

**Interfaces:**
- Consumes: Task 5 `ToolRoots`, `discover_tool_roots`, `safe_subprocess_env`, and `run_owned_process`; Tasks 3 and 16–17; `qualification/compatibility/legacy-database-import.json`; all six frozen database fixtures; and only the projected manifest ids `database.schema.current`, `database.migrations.current`, and `database.upgrade.preflight`. It never consumes or fingerprints the whole mutable manifest/state sources.
- Produces: `run(repo_root: Path, work_root: Path, *, executable: Path) -> QualificationRecord` for fake-injected unit tests and `run_live(repo_root: Path, work_root: Path, *, original_env: Mapping[str, str]) -> QualificationRecord` for the opt-in CLI; `run_live` rejects missing `HIERONYMUS_QUALIFICATION_LIVE=1`.
- Produces private handoff: `_live_process_context(repo_root: Path, work_root: Path, original_env: Mapping[str, str]) -> tuple[ToolRoots, Path, dict[str, str]]`, returning discovered tool roots, `qualification/.artifacts/cargo-target/legacy-database-import`, and the sanitized child environment in that order.
- Produces: `qualified` only when every database criterion passes; otherwise exact Task 2 blocking consequence.

- [ ] **Step 1: Write failing fake-only runner tests**

```python
def test_database_runner_uses_only_frozen_fixture_root(tmp_path: Path) -> None:
    record = run(ROOT, tmp_path, executable=write_fake_executable(tmp_path))
    assert "qualification/compatibility/legacy-database-import.json" in record.input_paths
    assert "tools/qualification/projections.py" in record.input_paths
    assert "compatibility/manifest.json" not in record.input_paths
    assert "compatibility/snapshots/state.json" not in record.input_paths
    assert all(
        not path.endswith(".sqlite")
        or path.startswith("compatibility/fixtures/database/")
        for path in record.input_paths
    )
    assert projection_issues(ROOT)["legacy-database-import"] == ()


def test_database_failure_preserves_data_disposition(tmp_path: Path) -> None:
    executable = write_fake_executable(
        tmp_path, failed_criteria=("source-byte-identity",)
    )
    record = run(ROOT, tmp_path, executable=executable)
    assert record.decision == "blocked"
    assert "fresh sibling database" in record.consequence
```

Add `test_database_live_context_discovers_before_sanitizing`: monkeypatch `run_database.discover_tool_roots` and `run_database.safe_subprocess_env`, call `_live_process_context(ROOT, tmp_path, original_env)`, and assert call order `("discover", "sanitize")`, identity of the passed `ToolRoots`, `cargo_offline is True`, and target `ROOT / "qualification/.artifacts/cargo-target/legacy-database-import"`.

Add `test_database_live_children_reuse_safe_environment`: inject a fixture-backed `run_owned_process` spy into `run_live`; assert every Cargo, SQLite harness, and inspection call receives the identical environment object returned by `safe_subprocess_env`, every Cargo argv starts with `str(tool_roots.cargo_invocation)`, and no call receives the original environment mapping.

- [ ] **Step 2: Run Python tests and verify RED without Cargo**

Run: `uv run pytest tests/qualification/test_run_database.py -v`

Expected: FAIL importing `tools.qualification.run_database`; no Rust process runs.

- [ ] **Step 3: Implement the bounded runner and write the live record**

`run_database.run_live` copies the caller-supplied `original_env`, calls `discover_tool_roots` before any sanitization, then passes the returned object and exact database Cargo target to `safe_subprocess_env`; its module CLI supplies `dict(os.environ)`. Before any Cargo or SQLite child, the runner requires `projection_issues(repo_root)["legacy-database-import"] == ()`, loads exact contract ids from that projection, and passes only the exact frozen fixture root, verified projection path, and risk work root to Task 17's CLI—never an arbitrary source/target/contract path. Every Cargo argv begins with `str(tool_roots.cargo_invocation)`, and every Cargo, SQLite harness, and inspection child uses `run_owned_process` with the same sanitized environment and risk-specific target. It hashes every immutable fixture/input before and after, removes target databases/traces/Cargo target in `finally`, and records only classifications/counts/digests/error codes; no absolute tool-root or invocation path is serialized. The record digest includes `projections.py`, the canonical database projection, harness/runner files, and six fixtures, and excludes the whole mutable manifest/state sources.

Run: `HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_database --write`

Expected: writes a complete JSON/Markdown pair with `qualified` or `blocked`. No source-row value, memory text, note, provider value, absolute path, or raw SQLite error dump enters either record.

Run: `uv run pytest tests/qualification/test_run_database.py tests/qualification/test_model.py tests/qualification/test_render.py -v`

Run: `uv run ruff check tools/qualification/run_database.py tests/qualification/test_run_database.py`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --check`

Run: `CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`

Expected: all checks pass; record validation proves the exact current database projection, fixture/contract ownership, complete criteria, accepted consequence, and source-byte identity. A temporary unrelated `test_ownership`/`state["tests"]` regeneration leaves the measured digest unchanged, while changing any selected contract or projected database field blocks before record write.

- [ ] **Step 4: Commit the database qualification record**

```bash
git add tools/qualification/run_database.py tests/qualification/test_run_database.py qualification/records/legacy-database-import.json docs/qualification/rust/legacy-database-import.md
git commit -m "test: qualify legacy database import"
```

### Task 19: Dispatcher, Record Checker, And Aggregate Gate

**Complexity:** Medium, 2–3 hours.

**Files:**
- Create: `tools/qualification/run.py`
- Create: `tools/qualification/check.py`
- Create: `tests/qualification/test_run.py`
- Create: `tests/qualification/test_gate.py`
- Create: `qualification/records/aggregate.json`
- Create: `docs/qualification/rust/gate.md`
- Create: `qualification/schemas/gate.schema.json`

**Interfaces:**
- Consumes: all four canonical records, their rendered Markdown, schemas, prerequisites, harness sources/lockfiles, frozen input fingerprints, both Task 3 compatibility projections, and `projection_issues` for read-only comparison with current mutable manifest/state sources.
- Produces: `run_one_live(risk: Risk, repo_root: Path, work_root: Path, *, original_env: Mapping[str, str]) -> QualificationRecord` and opt-in CLI `python -m tools.qualification.run <risk|all> --write`; the CLI snapshots `dict(os.environ)` once before dispatch and never sanitizes or serializes that mapping itself.
- Produces: `compute_gate(records: Mapping[Risk, QualificationRecord], repo_root: Path) -> GateRecord`.
- Produces: `validation_and_review_issues(records: Mapping[Risk, QualificationRecord], repo_root: Path) -> dict[Risk, tuple[str, ...]]` and `record_digests(records: Mapping[Risk, QualificationRecord]) -> dict[Risk, str]`.
- Produces: CLI `python -m tools.qualification.check [--record <risk>] [--records-only] [--require-qualified]`.
- Produces: canonical aggregate JSON and generated `docs/qualification/rust/gate.md`.

- [ ] **Step 1: Write the failing aggregate truth-table and offline-check tests**

```python
def test_all_pass_enables_semantic() -> None:
    gate = compute_gate(accepted_records(ROOT, semantic="semantic-enabled"), ROOT)
    assert gate.status == "qualified"
    assert gate.release_mode == "semantic-enabled"
    assert gate.blocked_plans == ()


def test_semantic_failure_selects_fts_only_and_still_qualifies() -> None:
    gate = compute_gate(accepted_records(ROOT, semantic="fts-only"), ROOT)
    assert gate.status == "qualified"
    assert gate.release_mode == "fts-only"
    assert gate.blocked_plans == ()


@pytest.mark.parametrize(
    ("risk", "blocked_plan"),
    [
        ("mcp-transport", "rust-daemon-mcp-security"),
        ("frontend-embedding", "rust-frontend"),
        ("legacy-database-import", "rust-database-upgrade"),
    ],
)
def test_non_semantic_failure_blocks_without_changing_specs(
    risk: Risk, blocked_plan: str
) -> None:
    records = accepted_records(ROOT, semantic="semantic-enabled")
    records[risk] = accepted_failure(ROOT, risk)
    gate = compute_gate(records, ROOT)
    assert gate.status == "blocked"
    assert "rust-workspace-and-contract-harness" in gate.blocked_plans
    assert blocked_plan in gate.blocked_plans


def test_pending_review_is_blocking() -> None:
    pending = accepted_records(ROOT, semantic="fts-only")
    pending["mcp-transport"] = pending_review(pending["mcp-transport"])
    assert compute_gate(pending, ROOT).status == "blocked"


def test_stale_input_is_blocking() -> None:
    stale = accepted_records(ROOT, semantic="fts-only")
    stale["frontend-embedding"] = replace(
        stale["frontend-embedding"], input_digest="0" * 64
    )
    assert compute_gate(stale, ROOT).status == "blocked"


def test_records_only_check_never_invokes_live_runners(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("socket.create_connection", fail_if_called)
    monkeypatch.setattr("subprocess.run", fail_if_called)
    result = invoke_check("--records-only")
    assert result.exit_code == 0


def test_unrelated_global_inventory_regeneration_keeps_measured_records_current(
    temp_repository: Path,
) -> None:
    seed_accepted_records_and_projections(temp_repository)
    append_unrelated_test_ownership(temp_repository)
    append_unrelated_state_test_node(temp_repository)
    before = load_record_digests(temp_repository)
    result = invoke_check("--records-only", cwd=temp_repository)
    assert result.exit_code == 0
    assert load_record_digests(temp_repository) == before
```

Define these private `test_gate.py` helpers over the existing temporary-repository/factory support: `seed_accepted_records_and_projections` writes the production-built canonical projections and four accepted records/Markdown/aggregate; the two append helpers modify only their named unrelated arrays with canonical JSON; and `load_record_digests` loads the four records and returns only their stored `input_digest` values. They must reuse Task 3 and Task 19 production serializers rather than duplicate them.

Add the paired failure test: mutate one selected MCP contract and one projected database state field in separate cases; `--records-only` must report the appropriate projection drift and block without rewriting the projection, risk record, Markdown, or aggregate.

Add `test_dispatcher_passes_original_environment_to_runner`: install one fake `LiveRunner`, call `run_one_live` with a sentinel mapping containing the live flag, and assert the fake receives the identical mapping as keyword-only `original_env` and a risk-specific work root. This test invokes neither discovery nor sanitization.

- [ ] **Step 2: Run the tests and verify the missing aggregate modules**

Run: `uv run pytest tests/qualification/test_run.py tests/qualification/test_gate.py -v`

Expected: FAIL importing `tools.qualification.run` and `tools.qualification.check`.

- [ ] **Step 3: Implement one dispatcher and non-short-circuiting live execution**

```python
from collections.abc import Mapping
from typing import Protocol


class LiveRunner(Protocol):
    def __call__(
        self,
        repo_root: Path,
        work_root: Path,
        *,
        original_env: Mapping[str, str],
    ) -> QualificationRecord: ...


LIVE_RUNNERS: dict[Risk, LiveRunner] = {
    "mcp-transport": run_mcp.run_live,
    "semantic-native": run_semantic.run_live,
    "frontend-embedding": run_frontend.run_live,
    "legacy-database-import": run_database.run_live,
}


def run_one_live(
    risk: Risk,
    repo_root: Path,
    work_root: Path,
    *,
    original_env: Mapping[str, str],
) -> QualificationRecord:
    if original_env.get("HIERONYMUS_QUALIFICATION_LIVE") != "1":
        raise RuntimeError("live qualification requires HIERONYMUS_QUALIFICATION_LIVE=1")
    return LIVE_RUNNERS[risk](
        repo_root,
        work_root / risk,
        original_env=original_env,
    )
```

The dispatcher snapshots `original_env = dict(os.environ)` once, passes it unchanged to every `run_one_live` call, and never places it in a record. Each risk runner independently performs Task 5 discovery before sanitization and uses its own target. `all` executes risks in the dictionary order above, gives each a distinct work root, writes every complete result even after a failure, then recomputes/writes the blocked pending-review aggregate JSON/Markdown so the record set stays internally consistent. It returns success when all four records are complete and valid. Risk decisions are evaluated only by `check --require-qualified`; this prevents a valid blocking record from being discarded.

- [ ] **Step 4: Implement the exact aggregate decision function**

```python
BLOCKING_PLANS: dict[Risk, tuple[str, ...]] = {
    "mcp-transport": (
        "rust-workspace-and-contract-harness",
        "rust-daemon-mcp-security",
    ),
    "semantic-native": (),
    "frontend-embedding": (
        "rust-workspace-and-contract-harness",
        "rust-frontend",
        "rust-distribution-cutover",
    ),
    "legacy-database-import": (
        "rust-workspace-and-contract-harness",
        "rust-database-upgrade",
        "rust-distribution-cutover",
    ),
}


@dataclass(frozen=True)
class GateRecord:
    schema_version: int
    target: str
    status: Literal["qualified", "blocked"]
    release_mode: Literal["semantic-enabled", "fts-only"] | None
    blocked_plans: tuple[str, ...]
    issues: tuple[str, ...]
    record_digests: dict[Risk, str]
    acceptance_owner: str


def compute_gate(
    records: Mapping[Risk, QualificationRecord],
    repo_root: Path,
) -> GateRecord:
    risk_issues = validation_and_review_issues(records, repo_root)
    issues = tuple(
        sorted(
            f"{risk}: {issue}"
            for risk, found in risk_issues.items()
            for issue in found
        )
    )
    blocked: set[str] = set()
    for risk in REQUIRED_CRITERIA:
        record = records.get(risk)
        if risk_issues[risk]:
            if risk == "semantic-native":
                blocked.add("rust-workspace-and-contract-harness")
            else:
                blocked.update(BLOCKING_PLANS[risk])
        elif risk != "semantic-native" and record is not None and record.decision != "qualified":
            blocked.update(BLOCKING_PLANS[risk])
    qualified = not issues and not blocked
    semantic = records.get("semantic-native")
    if qualified:
        assert semantic is not None
        assert semantic.decision in ("semantic-enabled", "fts-only")
        release_mode = semantic.decision
    else:
        release_mode = None
    return GateRecord(
        schema_version=1,
        target="x86_64-unknown-linux-gnu",
        status="qualified" if qualified else "blocked",
        release_mode=release_mode,
        blocked_plans=tuple(sorted(blocked)),
        issues=issues,
        record_digests=record_digests(records),
        acceptance_owner="Pavel Obruchnikov <me@inkyquill.net>",
    )
```

`validation_and_review_issues` first calls `projection_issues(repo_root)` once and attaches MCP/database projection findings only to their matching risks. It treats relevant projection drift, missing/extra risk, schema error, stale immutable input digest, Markdown drift, redaction failure, incomplete evidence, consequence mismatch, `review.status != "accepted"`, or false review assertions as blocking. Unrelated global manifest/state fields are outside the projections and are not staleness. A reviewed semantic `fts-only` record is not an issue.

- [ ] **Step 5: Implement network-free drift validation and initial aggregate rendering**

`check` loads schemas and records, rebuilds both compatibility projections in memory from only their selected source subtrees, compares them with checked-in canonical bytes, recomputes every immutable input fingerprint, validates redaction/review/consequence rules, regenerates four Markdown strings in memory, computes the gate, and compares canonical JSON/Markdown bytes. It imports no runner module in `--records-only` mode, performs no writes, and opens no subprocess, socket, SQLite connection, Cargo cache, model, or frontend bundle; SQLite fixture files are read only as bytes for SHA-256. Global ownership files may have later unrelated inventory bytes, but selected projection equality and record digests remain current.

`--record semantic-native` validates one risk but does not claim aggregate qualification. `--records-only` exits `0` for an internally consistent blocked gate. `--require-qualified` exits `1` and prints sorted issue/blocked-plan ids unless the gate is exactly `qualified`; when qualified, it prints exactly one of:

```text
Rust qualification gate: qualified (semantic-enabled)
Rust qualification gate: qualified (fts-only)
```

Write the pending-review aggregate to `qualification/records/aggregate.json` and generate `docs/qualification/rust/gate.md` from the four as-measured records. The aggregate is expected to remain blocked until Task 20 records the named owner's review.

- [ ] **Step 6: Run the focused gate tests and commit**

Run: `uv run pytest tests/qualification/test_run.py tests/qualification/test_gate.py -v`

Run: `uv run ruff check tools/qualification/run.py tools/qualification/check.py tests/qualification/test_run.py tests/qualification/test_gate.py`

Expected: PASS; `--records-only` cannot invoke a live runner or network/subprocess primitive, verifies both projections current, ignores unrelated generated ownership inventory changes, blocks relevant projection changes, and distinguishes reviewed `fts-only` from missing/unreviewed semantic evidence.

```bash
git add tools/qualification/run.py tools/qualification/check.py tests/qualification/test_run.py tests/qualification/test_gate.py qualification/schemas/gate.schema.json qualification/records/aggregate.json docs/qualification/rust/gate.md
git commit -m "test: compute Rust qualification gate"
```

### Task 20: Acceptance Review And Artifact Regeneration

**Complexity:** Small, 1–2 hours plus named-owner review.

**Files:**
- Create: `tools/qualification/review.py`
- Create: `tests/qualification/test_review.py`
- Modify after acceptance review: `qualification/records/mcp-transport.json`
- Modify after acceptance review: `qualification/records/semantic-native.json`
- Modify after acceptance review: `qualification/records/frontend-embedding.json`
- Modify after acceptance review: `qualification/records/legacy-database-import.json`
- Regenerate: `docs/qualification/rust/mcp-transport.md`
- Regenerate: `docs/qualification/rust/semantic-native.md`
- Regenerate: `docs/qualification/rust/frontend-embedding.md`
- Regenerate: `docs/qualification/rust/legacy-database-import.md`
- Regenerate: `qualification/records/aggregate.json`
- Regenerate: `docs/qualification/rust/gate.md`

**Interfaces:**
- Consumes: Task 19's checker/gate, Task 3's read-only projection validation, and the four immutable measured records from Tasks 8, 12, 15, and 18. Later unrelated regeneration of global manifest/state ownership sources is permitted and does not require rerunning a live measurement.
- Produces: `review_record(record: QualificationRecord, *, owner: str, status: Literal["accepted", "rejected"], objective_evidence_reviewed: bool, normative_constraints_preserved: bool) -> QualificationRecord`; this function may change only `review`.
- Produces CLI: `python -m tools.qualification.review <risk> --status <accepted|rejected> --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed <true|false> --normative-constraints-preserved <true|false>`; it rewrites one review block and regenerates that Markdown plus the aggregate JSON/Markdown with atomic per-file replacement.

- [ ] **Step 1: Write failing review-transition and regeneration tests**

Test that acceptance is rejected unless the owner is exact and both assertions are true; rejection preserves whichever assertion is false. Require each of the eight exact accepted/rejected commands printed in Step 3 to satisfy `replay_commands_are_safe((command,))`, parse the quoted owner through `shlex.split` to the single exact token `Pavel Obruchnikov <me@inkyquill.net>`, and reject unquoted/changed owners, reordered flags, status/assertion mismatches, empty tuples, and duplicate commands. Snapshot every non-review JSON path before and after `review_record` and require equality. Exercise the CLI in a temporary repository, require atomic per-file replacement of exactly one risk JSON/Markdown plus aggregate JSON/Markdown, and prove a rejected or partially reviewed record leaves the aggregate blocked. After seeding valid measured records/projections, mutate only unrelated manifest `test_ownership` and state `tests` inventory and prove MCP/database review still succeeds without changing their input digests. In paired cases, mutate a selected MCP contract or projected database field and prove review refuses before any replacement.

Run: `uv run pytest tests/qualification/test_review.py -v`

Expected: FAIL importing `tools.qualification.review`.

- [ ] **Step 2: Perform the named acceptance-owner review without altering evidence**

Every live runner writes this exact review block initially:

```json
{
  "owner": "Pavel Obruchnikov <me@inkyquill.net>",
  "status": "pending",
  "objective_evidence_reviewed": false,
  "normative_constraints_preserved": false
}
```

Pavel reviews the canonical evidence, input/lockfile digests, human rendering, and consequence. If the record accurately reflects the observed pass or failure without weakening a normative requirement, select this review value for the Step 4 CLI; do not hand-edit the record:

```json
{
  "owner": "Pavel Obruchnikov <me@inkyquill.net>",
  "status": "accepted",
  "objective_evidence_reviewed": true,
  "normative_constraints_preserved": true
}
```

If either assertion is false, select `status="rejected"` and keep the false assertion false. Step 4 regenerates Markdown and leaves the aggregate gate blocked. Never edit evidence status, measurements, input digest, or consequence during review; rerun the owning harness to change measured evidence.

- [ ] **Step 3: Implement the review-only transition and atomic regeneration**

`review_record` first validates the immutable measured record and exact owner, then copies only the `Review` dataclass. The CLI also requires the appropriate projection current through Task 19's single validation path. It accepts only with both assertions true; it rejects when either is false. It refuses `pending`, malformed records, stale immutable input digests, and relevant projection drift, but does not treat unrelated later global inventory bytes as staleness. It stages the reviewed risk JSON/Markdown plus aggregate JSON/Markdown in their own directories, validates all four replacement files in memory, and calls `os.replace` for each file only after every byte is valid. A crash between replacements leaves detectable drift and rerunning the same command converges to the same bytes. It never imports a live runner or changes evidence, decisions, consequences, commands, fingerprints, or measurements. If a relevant source contract/state field changes, Task 3 must regenerate its projection and the owning Task 8 or 18 live harness must create a new measured record before review; Task 20 cannot refresh that digest.

For each rejected record, use its exact risk-specific command below after implementation; both assertions remain conservatively false and no claim of partial acceptance is made:

```bash
uv run python -m tools.qualification.review mcp-transport --status rejected --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed false --normative-constraints-preserved false
uv run python -m tools.qualification.review semantic-native --status rejected --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed false --normative-constraints-preserved false
uv run python -m tools.qualification.review frontend-embedding --status rejected --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed false --normative-constraints-preserved false
uv run python -m tools.qualification.review legacy-database-import --status rejected --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed false --normative-constraints-preserved false
```

For each accepted record, use its exact risk-specific command:

```bash
uv run python -m tools.qualification.review mcp-transport --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true
uv run python -m tools.qualification.review semantic-native --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true
uv run python -m tools.qualification.review frontend-embedding --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true
uv run python -m tools.qualification.review legacy-database-import --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true
```

- [ ] **Step 4: Record the review outcomes, regenerate, verify, and commit**

Run the CLI once for each risk with the actual named-owner outcome and assertion values established in Step 2. Then run:

Run: `uv run pytest tests/qualification/test_review.py tests/qualification/test_gate.py -v`

Run: `uv run python -m tools.qualification.check --records-only`

Run: `uv run ruff check tools/qualification/review.py tests/qualification/test_review.py`

Expected: review tests pass; record validation exits `0` for either a consistent qualified or consistent blocked gate; unrelated global ownership regeneration is accepted; relevant projection drift is rejected; and every non-review record path is byte-equivalent to its immutable measured Task 8, 12, 15, or 18 input.

```bash
git add tools/qualification/review.py tests/qualification/test_review.py qualification/records docs/qualification/rust
git commit -m "docs: record Rust qualification review"
```

### Task 21: Qualification Contributor Commands And CI Workflows

**Complexity:** Small, 1–2 hours.

**Files:**
- Modify: `qualification/README.md`
- Modify: `.github/workflows/pr.yml`
- Create: `.github/workflows/rust-qualification-live.yml`
- Modify: `tests/test_pr_workflow.py`
- Create: `tests/test_rust_qualification_workflow.py`

**Interfaces:**
- Consumes: Tasks 19–20's network-free checker/review gate and all live runner commands.
- Produces: ordinary PR validation with no Cargo/native acquisition and a manual-only live workflow whose every `uses:` ref is a full 40-character commit SHA.
- Security invariant: every checkout sets `persist-credentials: false`; the Rust action is commit-pinned and receives `toolchain: 1.96.0` as explicit input; symbolic channel and major-version action refs are forbidden.

- [ ] **Step 1: Write failing workflow-policy tests**

Extend the existing line-oriented helpers in `tests/test_pr_workflow.py`. In `tests/test_rust_qualification_workflow.py`, assert the only trigger is `workflow_dispatch`, job permissions are `contents: read`, checkout disables credential persistence, every action suffix matches `[0-9a-f]{40}`, the toolchain input is exact, acquisition precedes live execution, `CARGO_NET_OFFLINE=true` is set only after acquisition, and no step commits/pushes. Require these exact action pins:

```python
CHECKOUT_SHA = "34e114876b0b11c390a56381ad16ebd13914f8d5"
SETUP_UV_SHA = "d0d8abe699bfb85fec6de9f7adb5ae17292296ff"
SETUP_BUN_SHA = "0c5077e51419868618aeaa5fe8019c62421857d6"
RUST_TOOLCHAIN_SHA = "4360b52568e2003a75bf9bc1d59f33a8e3fc893c"
UPLOAD_ARTIFACT_SHA = "ea165f8d65b6e75b540449e92b4886f43607fa02"
```

Run: `uv run pytest tests/test_pr_workflow.py tests/test_rust_qualification_workflow.py -v`

Expected: FAIL because the PR drift step and live workflow do not exist.

- [ ] **Step 2: Add contributor commands and the ordinary CI record gate**

Document exactly these workflows in `qualification/README.md`:

```bash
# One-time networked acquisitions
uv run python -m tools.qualification.acquire semantic-model
uv run python -m tools.qualification.acquire onnx-runtime
bun install --cwd frontend --frozen-lockfile
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport cargo +1.96.0 fetch --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native cargo +1.96.0 fetch --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding cargo +1.96.0 fetch --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked
CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import cargo +1.96.0 fetch --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked

# Explicit live qualification; writes measured records with pending review
HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run all --write

# Named-owner commands when all four measured records are accepted; use Task 20's rejected branch otherwise
uv run python -m tools.qualification.review mcp-transport --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true
uv run python -m tools.qualification.review semantic-native --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true
uv run python -m tools.qualification.review frontend-embedding --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true
uv run python -m tools.qualification.review legacy-database-import --status accepted --owner "Pavel Obruchnikov <me@inkyquill.net>" --objective-evidence-reviewed true --normative-constraints-preserved true

# Ordinary network-free validation
uv run --no-cache --no-sync python -B -m tools.qualification.projections --check
uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only

# Required before any dependent Rust implementation plan
uv run --no-cache --no-sync python -B -m tools.qualification.check --require-qualified

# Bounded cleanup; model/runtime deletion is separately explicit
uv run python -m tools.qualification.clean
uv run python -m tools.qualification.clean --apply
uv run python -m tools.qualification.clean --apply --include-model
```

Append this step to the existing Python PR job in `.github/workflows/pr.yml` without adding Rust/Bun/cache setup:

```yaml
- name: Validate Rust qualification records
  env:
    HIERONYMUS_QUALIFICATION_LIVE: "0"
  run: |
    uv run --no-cache --no-sync pytest tests/qualification
    uv run --no-cache --no-sync python -B -m tools.qualification.projections --check
    uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only
```

Every runner test injects a fake executable, so this job needs no Cargo cache, Bun install, ONNX Runtime, model, or network. It validates durable records and allows an honest blocking record to merge. Any workflow that generates a dependent Rust plan must run `--require-qualified` first.

Create `.github/workflows/rust-qualification-live.yml` exactly as an explicit manual job:

```yaml
name: Rust qualification live
on:
  workflow_dispatch:

jobs:
  qualify:
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5
        with:
          persist-credentials: false
      - uses: astral-sh/setup-uv@d0d8abe699bfb85fec6de9f7adb5ae17292296ff
      - uses: oven-sh/setup-bun@0c5077e51419868618aeaa5fe8019c62421857d6
        with:
          bun-version: "1.4.0"
      - uses: dtolnay/rust-toolchain@4360b52568e2003a75bf9bc1d59f33a8e3fc893c
        with:
          toolchain: "1.96.0"
          targets: x86_64-unknown-linux-gnu
          components: clippy,rustfmt
      - name: Acquire locked prerequisites
        run: |
          uv sync --frozen
          uv run python -m tools.qualification.acquire semantic-model
          uv run python -m tools.qualification.acquire onnx-runtime
          bun install --cwd frontend --frozen-lockfile
          CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport cargo +1.96.0 fetch --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked
          CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native cargo +1.96.0 fetch --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked
          CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding cargo +1.96.0 fetch --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --locked
          CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import cargo +1.96.0 fetch --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked
      - name: Run isolated live qualification
        env:
          HIERONYMUS_QUALIFICATION_LIVE: "1"
          CARGO_NET_OFFLINE: "true"
        run: uv run python -m tools.qualification.run all --write
      - name: Validate sanitized records
        run: uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only
      - uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02
        with:
          name: rust-qualification-records
          path: |
            qualification/records/*.json
            docs/qualification/rust/*.md
```

The workflow never commits records. Each runner supplies its bounded risk-specific `CARGO_TARGET_DIR`; `run_owned_process` disables cores/reaps groups. A scheduled or pull-request trigger is forbidden.

- [ ] **Step 3: Run workflow tests and complete qualification-stage verification**

Run: `uv run pytest tests/test_pr_workflow.py tests/test_rust_qualification_workflow.py -v`

Expected: PASS; all action refs are full SHAs, checkout credentials are disabled, and the live workflow is manual-only.

Run: `uv run --no-cache --no-sync python -B -m tools.compatibility.check`

Expected: exit `0`; the frozen compatibility boundary has not drifted.

Run: `uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only`

Expected: exit `0`; both compatibility projections are current, and records, immutable fingerprints, redaction, reviews, Markdown, and checked-in aggregate gate are internally consistent without network access. Unrelated later global ownership inventory bytes do not require live remeasurement.

Run: `uv run --no-cache --no-sync python -B -m tools.qualification.check --require-qualified`

Expected for permission to write the next Rust workspace/dependent plan: exit `0` with exactly one qualified mode line. If it exits `1`, preserve the blocking aggregate record and stop before writing a dependent plan.

Run: `env -u HIERONYMUS_QUALIFICATION_LIVE uv run --no-cache --no-sync pytest`

Run: `uv run --no-cache --no-sync ruff check .`

Run: `uv run --no-cache --no-sync ruff format --check .`

Expected: the full Python suite and required project checks pass with live qualification disabled; runner tests prove fake injection and fail if Cargo/Bun/native execution is attempted.

Run: `git diff --exit-code -- uv.lock frontend/bun.lock`

Expected: ordinary verification changed neither dependency lockfile. Rust/Bun/native harness and Clippy commands run only in the explicit live workflow or the live commands in Tasks 5–18, always with the risk-specific bounded Cargo target directory.

- [ ] **Step 4: Verify cleanup, repository scope, final diff, and commit**

Run: `uv run python -m tools.qualification.clean --apply`

Expected: transient qualification work/install/log/target/bundle directories are removed; verified model/runtime acquisitions remain ignored and no tracked record changes.

Run: `git diff --check`

Run: `git status --short`

Expected: no whitespace errors; only files enumerated by this plan plus any pre-existing unrelated user changes appear. No production Rust path, translation workspace, runtime database, model binary, raw log, node_modules, frontend dist, or user-owned untracked file is staged.

```bash
git add qualification/README.md .github/workflows/pr.yml .github/workflows/rust-qualification-live.yml tests/test_pr_workflow.py tests/test_rust_qualification_workflow.py
git commit -m "ci: verify Rust qualification records"
```

## Self-Review Record

- Spec coverage: Task 1 pins the byte-exact official Draft 2020-12 schema, corrects and separately commits valid stateless tools/list/tools/call envelopes, explicitly configures serverInfo omission, and distinguishes seven HeaderMismatch cases from one coherent unsupported-version case while preserving method-aware name applicability. Tasks 2–5 establish the record, immutable risk-specific compatibility projections, validation, acquisition, cargo-shim-preserving discovery, and bounded-process foundations. Tasks 6–8 qualify all seventeen MCP criteria against the current verified 42-contract projection: exact metadata/headers/required list fields/wire key/transports/registry/error parity, configured response metadata, schema-pinned offline validation, and private-bridge absence. Tasks 9–12 own actual ANN creation, checked pre-filter plan/cardinality proof, SQLite-durable recovery/no-write-transaction-native-I/O proof, strengthened FTS, and FTS-only selection. Tasks 13–15 own manifest-correct Svelte embedding and traced runtime independence without claiming HTTP security ownership. Tasks 16–18 consume the verified three-contract/12-state-field database projection and own frozen-root classification/import, typed accounting, FTS/ledger proof, fail-closed behavior, and source immutability. Tasks 19–21 own projection-current validation, the aggregate gate, named-owner review/regeneration, reproducibility commands, and pinned CI.
- Normative consequence coverage: semantic failure has exactly one accepted non-blocking result, `fts-only`; MCP/frontend/database failure blocks named dependent plans and never edits fixtures or specifications to turn a failure into a pass.
- Network coverage: Task 1 alone retrieves the commit-pinned official schema once and verifies exact raw bytes; its tests, compatibility gate, Task 8 runner, and qualification gate use only the checked-in authority. Other acquisition commands are named and checksum/frozen-lock constrained; Hugging Face redirects include the observed exact CDN host under hop validation; Bun replay uses an isolated network namespace rather than a nonexistent offline-install flag; ordinary pytest uses only bounded fake child executables, while record checks use no subprocess, socket, Rust/native cache, or network. The one real Cargo environment smoke is explicitly live-gated.
- Sensitive-data coverage: inputs are synthetic/frozen, work roots are bounded, reports contain only digests/counts/basenames, and source database bytes are verified unchanged. Canonical JSON is parsed and recursively inspected by exact normalized snake/camel key over every scalar type/array, while completed Markdown uses the same non-echoing issue order; password/private-key/auth/cookie, provider/token/host/user identity, memory/source text, raw log/stdout/stderr, raw PEM private-key markers, and raw/percent-encoded home paths are rejected. Exact safe sentinels remain allowed in ordinary punctuation/table context.
- Cleanup coverage: all transient output and every Cargo target are under one ignored exact root; core dumps are disabled; owned process groups are reaped; cleanup targets are enumerated/tested; model/runtime removal needs a separate flag; and no recursive operation can target the repository, home, translation workspace, or user data.
- Ownership coverage: Task 1 owns the schema authority/module plus two implementation-internal test nodes and regenerates their state-snapshot/manifest/diagnostic ownership without adding public contract ids. Task 3 alone owns `projections.py` and both checked-in projections; ledger-required later global inventory regeneration may not rewrite them for unrelated changes. Tasks 2–5 otherwise split common model, validation/rendering, acquisition, and process/cleanup ownership; Task 9 alone extends acquisition inputs; every risk is split into separately committed manifest/build, behavior/recovery, and runner/evidence reviews; Tasks 19–21 separately own gate computation, review regeneration, and workflows.
- Projection/staleness coverage: MCP and database records fingerprint their immutable projections and builder/validation code, never whole mutable ownership sources. Tasks 8/18 validate the appropriate projection before measurement, Task 19 validates both on every gate check, and Task 20 may review unchanged measured records after unrelated global inventory regeneration. A relevant selected source change blocks, requires projection regeneration, changes that risk's digest, and therefore requires a new owning live measurement.
- Type/signature consistency: Task 1's four allowed schema-definition names match every Task 7/8 validation call, internal `input_schema` is mapped only at the explicit target-wire boundary, and the MCP record has exactly seventeen named criteria. `projection_issues(repo_root)` returns exact MCP/database keys consumed consistently by validation and Tasks 8, 18, 19, and 20. Task 3 proves the 39 direct-child MCP directories, 156 policy leaves, 180-input MCP policy, and 28-input semantic policy against real manifest fixture refs and filesystem inventory. `replay_commands_are_safe` owns nonempty/unique enforcement and admits only the finite Task 8/12/15/18/19/20/21 matrix, including exact quoted owner/status/boolean forms, projections, bounded cleanup, checker, runner, Cargo, and isolated frontend commands. Fake-injected `run(..., executable: Path)` and guarded `run_live(..., original_env: Mapping[str, str])` are distinct for all four runners; every live context returns Task 5's exact `ToolRoots`, target, and safe environment; risk/criterion ids come from `REQUIRED_CRITERIA`; every record uses Task 2's exact types; Task 19 passes one unsanitized environment snapshot only to `run_live` and its records-only path imports no runner; Task 20 can replace only `Review`.
- Production-scope check: the file map contains no production Rust workspace or crate path, and no task changes Python runtime behavior or starts a dependent implementation plan.

Before accepting this plan, run:

```bash
rg -n '\b(T[B]D|T[O]DO)\b|implement la[t]er|fill in deta[i]ls|Similar to Tas[k]' docs/superpowers/plans/2026-09-01-rust-qualification.md
test "$(find compatibility/fixtures/mcp -mindepth 1 -maxdepth 1 -type d -name 'hieronymus_*' | wc -l)" -eq 39
test "$(find compatibility/fixtures/mcp -mindepth 2 -maxdepth 2 -type f \( -name 'error.input.json' -o -name 'success.input.json' -o -name 'wire.error.json' -o -name 'wire.success.json' \) | wc -l)" -eq 156
python - <<'PY'
import re
import json
from pathlib import Path

text = Path("docs/superpowers/plans/2026-09-01-rust-qualification.md").read_text()
tasks = [int(value) for value in re.findall(r"^### Task (\d+):", text, re.MULTILINE)]
assert tasks == list(range(1, 22)), tasks
assert sum(line.startswith("```") for line in text.splitlines()) % 2 == 0
assert all(1 <= int(value) <= 21 for value in re.findall(r"\bTasks? (\d+)", text))
action_refs = re.findall(r"^\s+- uses: [^@\s]+@([^\s]+)$", text, re.MULTILINE)
assert action_refs and all(re.fullmatch(r"[0-9a-f]{40}", ref) for ref in action_refs)
for required in (
    "ef70b61f99b6d2e5e3b46863822eab08dff6a45bedc7a08914e0e5b133f40203",
    '"cacheScope": "private"',
    '"ttlMs": 0',
    '"inputSchema"',
    '"protocol-version-header-mismatch"',
    '"code": -32020',
    '"code": -32022',
    '"configured": "omit"',
    "assert len(failures) == 11",
    "assert len(record.evidence) == 17",
    "qualification/compatibility/mcp-transport.json",
    "qualification/compatibility/legacy-database-import.json",
    "tools/qualification/projections.py",
    "projection_issues(repo_root",
    '"application_migration_ledgers"',
    '"variants"',
    "st_mtime_ns",
    "st_ctime_ns",
    "authorizationHeader",
    "privateKey",
    "memoryText",
    "sourceText",
    "rawLog",
    "rawLogs",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "every C1 code point U+0080–U+009F",
    "len(value) == len(set(value))",
    "PLANNED_REPLAY_COMMANDS",
    "%2Fhome%2Falice%2Fprivate",
    "${INPUT:-/etc/passwd}",
    "![alt](target)",
    "compatibility/fixtures/mcp/<name>/error.input.json",
    "compatibility/fixtures/mcp/hieronymus_rag_search/success.input.json",
    "compatibility/fixtures/mcp/hieronymus_recall/success.input.json",
    "assert len(required_fingerprint_inputs(\"mcp-transport\")) == 180",
    "assert len(required_fingerprint_inputs(\"semantic-native\")) == 28",
):
    assert required in text, required
assert "compatibility/fixtures/mcp/" + "tools/" not in text
assert 'tool["input_schema"]\n        for tool in target' not in text
assert not re.search(
    r'^\s*assert "compatibility/manifest\.json" in record\.input_paths\s*$',
    text,
    re.MULTILINE,
)
assert not re.search(
    r'^\s*assert "compatibility/snapshots/state\.json" in record\.input_paths\s*$',
    text,
    re.MULTILINE,
)
for line_number, line in enumerate(text.splitlines(), start=1):
    if not re.search(
        r"compatibility/(?:manifest\.json|snapshots/state\.json)", line
    ) or not re.search(r"fingerprint|input_paths|digest includes", line, re.I):
        continue
    assert re.search(
        r"never|neither|not in|absent|exclude|mutable|source|projection",
        line,
        re.I,
    ), (line_number, line)

manifest = json.loads(Path("compatibility/manifest.json").read_text(encoding="utf-8"))
tool_contracts = tuple(
    item for item in manifest["contracts"] if item["id"].startswith("mcp.tool.")
)
assert len(tool_contracts) == 39
tool_names = {item["id"].removeprefix("mcp.tool.") for item in tool_contracts}
for item in tool_contracts:
    name = item["id"].removeprefix("mcp.tool.")
    assert item["fixture"] == f"compatibility/fixtures/mcp/{name}/success.input.json"
leaves = {"error.input.json", "success.input.json", "wire.error.json", "wire.success.json"}
actual = {
    path.as_posix()
    for directory in Path("compatibility/fixtures/mcp").iterdir()
    if directory.is_dir() and directory.name in tool_names
    for path in directory.iterdir()
    if path.name in leaves
}
expected = {
    f"compatibility/fixtures/mcp/{name}/{leaf}"
    for name in tool_names
    for leaf in leaves
}
assert len(actual) == len(expected) == 156
assert actual == expected

planned_replay_commands = (
    "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_mcp --write",
    "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_semantic --write",
    "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_frontend --write",
    "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_database --write",
    "HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run all --write",
    "uv run --no-cache --no-sync python -B -m tools.qualification.projections --check",
    "uv run --no-cache --no-sync python -B -m tools.qualification.check --records-only",
    "uv run --no-cache --no-sync python -B -m tools.qualification.check --require-qualified",
    "uv run python -m tools.qualification.clean",
    "uv run python -m tools.qualification.clean --apply",
    "uv run python -m tools.qualification.clean --apply --include-model",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 metadata --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --format-version 1",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 tree --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked -e features",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --check",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/mcp-transport CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/mcp-transport/Cargo.toml --locked --target x86_64-unknown-linux-gnu -- -D warnings",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/semantic-native/Cargo.toml --check",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/semantic-native CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/semantic-native/Cargo.toml --locked --all-targets --features semantic-native --target x86_64-unknown-linux-gnu -- -D warnings",
    "unshare --user --map-root-user --net -- bun run --cwd frontend build -- --outDir ../qualification/.artifacts/frontend-dist/current --emptyOutDir",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 build --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --target x86_64-unknown-linux-gnu",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --check",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --check",
    "CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings",
)
for command in planned_replay_commands:
    assert command in text, command
for template in (
    "uv run python -m tools.qualification.validate qualification/records/<risk>.json",
    "uv run python -m tools.qualification.render --check qualification/records/<risk>.json docs/qualification/rust/<risk>.md",
    "uv run python -m tools.qualification.check --record <risk>",
):
    assert template in text, template
for risk in ("mcp-transport", "semantic-native", "frontend-embedding", "legacy-database-import"):
    for status, objective, normative in (
        ("accepted", "true", "true"),
        ("rejected", "false", "false"),
    ):
        command = (
            f'uv run python -m tools.qualification.review {risk} --status {status} '
            '--owner "Pavel Obruchnikov <me@inkyquill.net>" '
            f'--objective-evidence-reviewed {objective} '
            f'--normative-constraints-preserved {normative}'
        )
        assert command in text, command
PY
git diff --check -- docs/superpowers/plans/2026-09-01-rust-qualification.md
git diff -- docs/superpowers/plans/2026-09-01-rust-qualification.md
test "$(git diff --name-only)" = "docs/superpowers/plans/2026-09-01-rust-qualification.md"
git status --short
```

Expected: the red-flag scan prints nothing; tasks remain exactly 1–21; task references, fences, action pins, official schema pin, exact list fields/wire key, error codes/case counts, configured serverInfo omission, projection paths/API/state fields, recursive structured/PEM redaction, C0/DEL/C1 encoding, and seventeen-criterion handoff pass. The real 39-directory/156-leaf MCP inventory and manifest refs match the direct-child policy, the MCP/semantic totals remain 180/28, no obsolete intermediate fixture hierarchy remains, every exact downstream replay/projection/check/cleanup/review command is represented in the accepted matrix, and no positive whole-manifest/state fingerprint statement remains. Diff check passes and the changed-files audit contains only this plan. If a pre-existing unrelated change exists, use the path-scoped diff/status audit instead and leave it unstaged and untouched.

## Execution Handoff

Execute Tasks 1–21 only through the required sub-skill named in the header. Task 1's corrected compatibility commit—including the exact schema pin, valid response wire objects, configured serverInfo omission, and corrected HeaderMismatch/unsupported-version split—is a hard prerequisite and must pass independent review before candidate qualification. Task 3 then creates the two immutable risk projections and proves its literal direct-child MCP fixture policy against the real 39 manifest refs and 156-leaf filesystem inventory; factories cannot manufacture missing policy paths. Tasks 7–8 consume but never rewrite the MCP authority, Tasks 8/18 validate and fingerprint only their appropriate projection, Task 12 consumes the two direct-child semantic seeds, and Tasks 19–20 enforce projection currency without treating unrelated global inventory regeneration as measured-record staleness. Every stored command must belong to Task 3's finite matrix, including exact quoted-owner review/status/assertion forms and bounded cleanup/projection/check commands. After Task 21, a qualified aggregate permits the separate Rust workspace/contract-harness plan; a blocked aggregate is the durable stage result and requires a new candidate qualification run or an ADR-backed specification change before dependent planning.
