# Rust Migration Proposal: 006 - Testing & Deployment

Phase 6 of the Rust migration (depends on all of 001-005 — this phase verifies parity and
ships the binary; it defines no new domain code). Covers the test strategy, vector-index/RAG
testing, cross-compilation, and installer simplification.

**Parity target:** every `tests/test_*.py` file's coverage, reproduced in Rust against the same
fixtures and asserting the same behavior. **Legacy not carried forward:** the 240+-line
`install.sh` with Python/Node/Bun environment prerequisite checks (§4), and the frontend test
suite's grep-on-source-text pattern (`app.test.ts` asserting `expect(text).toContain(...)`
against raw `.svelte` file contents rather than rendered/mounted output) — the Rust migration is
the point at which frontend tests should switch to `@testing-library/svelte`-style
mount-and-interact assertions, since the backend rewrite touches the same API contracts those
tests would need to stay honest about.

---

## 1. Testing Strategy

### 1.1 Unit Tests

`#[cfg(test)] mod tests` blocks colocated with the logic they cover: RAG chunking boundaries
(003 §4), scoring calculations (003 §2.5), rule crystal parsing (003 §2.4), RRF rank fusion
(003 §5.4), score clamping/normalization (002 §6).

### 1.2 Integration Tests

Each test file gets an isolated in-memory database:

```rust
async fn setup_test_db() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    pool
}
```

| Test file | Covers | Python parity target |
|---|---|---|
| `test_crystals.rs` | `CrystalStore` CRUD, FTS search, supersede, link, scoring | `tests/test_crystals.py` |
| `test_memory.rs` | `WorkspaceStore` CRUD, FTS triggers, batch insert, working-copy dedup | `tests/test_workspace.py` |
| `test_concepts.rs` | Concept lifecycle, facets, linking, merging, `rename_concept` | `tests/test_concepts.py` |
| `test_recall.rs` | Multi-source recall, rule-intent/credibility boost, spreading activation, `crystal_activations` logging | `tests/test_recall.py` |
| `test_scoring.rs` | Immediate/passive deltas, `rule_intent` decay dampening (no immunity — see 003 §2.5), archival, `record_recall_outcome` | `tests/test_scoring.py` |
| `test_reconsolidation.rs` | `Reconsolidator`: diff-threshold reinforce-vs-supersede, concept-link inheritance, working-copy archival | none (new behavior — see `docs/superpowers/specs/2026-07-18-memory-reconsolidation-design.md`) |
| `test_link_reinforcement.rs` | `LinkReinforcer`: Hebbian strengthening, pairwise combination, `combined_into` events | none (new behavior — same design doc) |
| `test_rag.rs` | RAG import, FTS search, parsing, conversion | `tests/test_rag_store.py` |
| `test_semantic.rs` | LanceDB index, hybrid RRF, fake embeddings, fallback | `tests/test_hybrid_ranking.py` |
| `test_dreaming.rs` | Dream cycles, phase execution, crystallization, audit, cross-process lock contention | `tests/test_dreaming.py` |
| `test_rule_crystals.rs` | Rule-intent parsing, `list_rule_intent`, concept enrichment | `tests/test_termbase_validate.py` |
| `test_termbase.rs` | Propose/approve/contract/validate against rule-intent crystals only | `tests/test_termbase_contract.py` |
| `test_service.rs` | HTTP routes, MCP protocol, WebSocket events, `/api/admin`+`/api/settings` contract shapes | `tests/test_service_app.py` |
| `test_mcp.rs` | All 39 existing MCP tools + `hieronymus_recall_feedback` via HTTP transport | `tests/test_mcp_http.py` |
| `test_agent.rs` | Plugin installs, config generation, skill installation | `tests/test_agent_plugin_installers.py` |
| `test_cli.rs` | CLI command parsing, boundary rules, JSON output | `tests/test_cli.py` |
| `test_doctor.rs` | System diagnostics, checks, redaction | `tests/test_doctor.py` |

### 1.3 Mocking Strategy

- **LLM providers**: `MockDreamProvider` implements `DreamProvider` (004 §1), returns
  predetermined JSON. No network access in CI.
- **Embeddings**: `FakeEmbeddingProvider` implements `EmbeddingProvider` (003 §5.1), returns
  deterministic fixed vectors. No model download in CI.
- **HTTP daemon**: `axum::test` helpers against the real router from 005 §2 (no mock server).
- **LanceDB**: temporary directories per test. Contract tests run against both
  `FakeSemanticIndex` (in-memory `HashMap` implementing `SemanticIndex`, 003 §5.2) and the real
  `LanceDbIndex`, asserting identical behavior for the same inputs.

### 1.4 Parity Validation

Integration tests ingest fixed sample datasets (former `strict_terms` fixtures, known
translation contexts) and assert:
1. Rule crystal extraction matches the Python reference output exactly.
2. Recall ranking matches Python's within a documented tolerance (RRF/BM25 floating-point drift).
3. Termbase validation reports identical findings for the same input.
4. Concept graph construction matches the expected structure after a dream cycle.

---

## 2. Vector Index & RAG Testing

- **Local index validation**: build a temporary LanceDB directory, insert sample RAG chunks,
  query, assert expected nearest-neighbors.
- **Fallback verification**: disable LanceDB or mock a missing ONNX model file, assert
  retrieval falls back to FTS5-only without erroring (003 §5.4 invariant).
- **Rank fusion verification**: assert `reciprocal_rank_fusion` (003 §5.4) merges candidates
  correctly under varying FTS/semantic scoring distributions.

---

## 3. Compilation & Release Pipelines

* **Build tool**: **native runners per target**, not cross-compilation. `ort` (bundles/links
  ONNX Runtime, a C++ library) and `lancedb` (Apache Arrow + the native Lance engine) are both
  native-dependency crates that routinely fail to cross-compile from a single Linux runner via
  `cross` — missing target-arch C/C++ toolchains and prebuilt native libraries are a recurring
  failure mode for exactly this crate pair, not a hypothetical risk. CI runs the release build
  on GitHub Actions' native `ubuntu-latest`, `macos-latest` (covers both Intel and
  `aarch64-apple-darwin` via Xcode's universal toolchain), and `windows-latest` runners, one job
  per target platform, rather than cross-compiling all four from one job.
* **Target platforms**: `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`,
  `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
* **Static linking**: `rustls-tls` feature everywhere `reqwest` is used — no dependency on
  local OpenSSL configuration.
* **One build recipe.** Python currently has four independent frontend-build recipes (Hatch
  build hook, `python-semantic-release`'s `build_command`, `install.sh`, CI) with inconsistent
  Bun version pins, causing redundant double/triple builds during release. Rust replaces all
  four with one: `build.rs` invokes `bun run --cwd frontend build` (pinned Bun version recorded
  once, in the workspace `Cargo.toml` or a `.bun-version` file `build.rs` reads), and
  `cargo build --release` is the only release command — no separate `uv build`/`bun`
  orchestration layer to keep in sync.

```bash
# Development
cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check

# Frontend integration
cd frontend && bun run build      # produces frontend/dist/
cargo build --release             # embeds frontend/dist/ via rust-embed (005 §6)

# Cross-compilation
cargo dist build --target x86_64-unknown-linux-gnu
cargo dist build --target x86_64-apple-darwin
cargo dist build --target aarch64-apple-darwin
cargo dist build --target x86_64-pc-windows-msvc
```

`build.rs`: if `frontend/dist/` is present, compiles it into the binary via `rust-embed`; if
absent (library-only or frontend-less CI builds), emits a warning but succeeds. The release
pipeline always builds with the frontend present.

---

## 4. Installer Simplification

Single-binary distribution shrinks `install.sh` to roughly 30 lines with no interactive-TTY
edge cases to get wrong (Python's `has_tty()` checked `/dev/tty` readability, which stayed true
under `pytest` subprocess capture and could hang automated runs — moot once there's no
interactive installer prompt to gate):

1. Detect target OS/CPU architecture.
2. Download `hiero-{version}-{target}.tar.gz` from GitHub Releases.
3. Verify checksum.
4. Extract binary to `~/.local/bin/hiero`.
5. Run `hiero doctor` (001 §7) to confirm the install is healthy.

No Python, Node, Bun, or other runtime prerequisite checks — "if it installs, it runs."

---

## 5. Open Questions & Future Work

| Question | Scope |
|---|---|
| MCP SDK v2 migration | Current design targets MCP v1 via `rmcp` (005 §1); revisit when v2 stabilizes |
| ONNX model distribution | The multilingual embedding model is ~470MB (003 §5.1); consider bundling a smaller default or documenting the first-run download clearly |
| Agent skill bundling | Currently `rust-embed` for skill assets (001 §5); may move to a standalone agent-skills repository if assets grow large enough to bloat the binary |
| Windows support | `cargo-dist` covers the target, but the daemon/signal-handling path (005 §4) is unix-tested first; needs explicit Windows CI verification before calling it supported |
