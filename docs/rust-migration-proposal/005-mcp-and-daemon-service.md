# Rust Migration Proposal: 005 - MCP Server & Daemon Service

Phase 5 of the Rust migration (depends on 001 for config/CLI, 003 for the domain stores every
route and tool calls, 004 for `DreamService` and the cross-process dream lock). Covers the
dual-transport MCP server, the unified Axum HTTP/WebSocket daemon, signal handling, and
frontend asset embedding.

**Normative override:** ADR 0015 owns the MCP protocol and transports. In
particular, `/api/mcp/{operation}` is a private Python bridge, not Streamable
HTTP, and is removed at Rust cutover. The compatibility manifest owns the live
tool registry and derives its count.

**Parity target:** every MCP tool in the authoritative registration snapshot, plus one new tool for the
reconsolidation feedback signal (003 §3); the full HTTP route surface from
`service_http.py`/`service_app.py`; the admin/config REST API from `tui_bridge/`, with
**request/response shapes pinned from the actual frontend client** (`frontend/src/web/lib/api.ts`)
rather than redesigned — the first draft invented a `/api/config/*` path scheme and flat
response structs that don't match what the shipping Svelte console actually calls, which would
have produced immediate 404s and `TypeError`s in the browser. **Legacy not carried forward:**
`service_state.py` (`server.json`, PID tracking, port allocation — there's no subprocess to
track), `service_discovery.py` (port health polling), `daemon_events.py`'s manual pub/sub
(replaced by Axum broadcast channels), the hand-rolled WebSocket frame-skipping loop that never
parsed control frames, the `?token=...` query-string auth leak into browser history/process
argv, and the dead `hieronymus_token` cookie fallback that nothing ever set. Auth is
header-based only, over a token file with `0600` permissions from the start (001 §3).

---

## 1. Multi-Transport MCP Server

1. **HTTP:** MCP revision `2026-07-28` Streamable HTTP mounted at `/mcp`, as
   pinned by ADR 0015.
2. **Stdio:** `hiero mcp` exposes newline-delimited MCP JSON-RPC and proxies the
   authenticated standard `/mcp` endpoint. It is a supported transport, not a
   private-operation shim marked for automatic removal.

```rust
pub fn register_tools(server: &mut McpServer, backend: Arc<dyn McpBackend>);

#[async_trait]
pub trait McpBackend: Send + Sync {
    async fn status(&self) -> Result<serde_json::Value>;
    // one method per tool group in §3 below; each thin-wraps a 003/004 store call
}

pub mod stdio_shim {
    pub async fn run(config: &HieronymusConfig) -> Result<()>;  // proxies to the running daemon over HTTP
}
```

---

## 2. Unified HTTP & WebSocket Daemon (Axum)

```rust
pub struct AppState { pub pool: SqlitePool, pub config: Arc<HieronymusConfig>, pub shutdown: tokio::sync::broadcast::Sender<()> }

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/admin", get(serve_index))
        .route("/config", get(serve_index))
        .nest_service("/assets", ServeDir::new(Asset::assets_root()))   // §5
        .nest("/api", api_routes())                                     // §4
        .route("/ws/admin", get(admin_ws_handler))                      // broadcast channel, replaces daemon_events polling
        .route("/health", get(health_handler))
        .route("/status", get(status_handler))
        .route("/shutdown", post(shutdown_handler))
        .nest("/mcp", mcp_routes())                                     // §1
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
```

**Binding & auth:** binds strictly to `127.0.0.1:9768` (override via `--port` /
`HIERONYMUS_PORT`, 001 §3); startup fails fast if the port is busy. Browser requests are
authorized via `Host`/`Origin` validation; MCP requests without an `Origin` header are accepted;
foreign origins are rejected at `/mcp`.

**Dream cycle awareness:** the daemon's background dreaming loop (004 §6) takes the same
cross-process file lock (004 §1) a CLI-invoked `hiero dream` would — the daemon holds no special
status here; it's just another process contending for the same lock file.

---

## 3. MCP Tools (Manifest Snapshot Plus One Proposed Addition)

Transcribed from `mcp_server.py`'s `@server.tool` registrations; the compatibility
manifest derives and reports the count,
grouped by backing store. `hieronymus_rule_crystals_*` now operate on crystals with non-empty
`rule_intent` (003 §2.4's `list_rule_intent`), not a `crystal_type == 'rule'` filter — same tool
names, corrected backing query per §3's rule-intent model in 003.

| Tool | Backing call |
|---|---|
| `hieronymus_status` | §2's `status_handler` logic + 001 §7 `run_doctor` |
| `hieronymus_series_create`, `hieronymus_series_init`, `hieronymus_series_list`, `hieronymus_series_set_language_tags` | 001 §6 `SeriesRegistry` |
| `hieronymus_concept_list`, `_get`, `_create`, `_update`, `_archive`, `_merge`, `_rename` | 003 §2.3 `ConceptStore` (`_rename` → `rename_concept`) |
| `hieronymus_concept_facet_add`, `_update`, `_list`, `_set_canonical` | 003 §2.3 `ConceptStore` facet methods |
| `hieronymus_concept_semantic_tags_set` | 003 §2.3 `ConceptStore` |
| `hieronymus_crystal_link_concept` | 003 §2.3 `ConceptStore::link_crystal` |
| `hieronymus_crystal_story_scopes_set`, `hieronymus_crystal_semantic_tags_set` | 003 §2.1 `CrystalStore` |
| `hieronymus_rule_crystals_list`, `_archive`, `_validate` | 003 §2.1/§2.4 (`list_rule_intent`, `archive`, `validate_rule`) |
| `hieronymus_termbase_contract`, `_validate`, `_propose`, `_approve` | 003 §2.4 `Termbase` |
| `hieronymus_memory_search` | 003 §3 `RecallService` |
| `hieronymus_memory_add` | 003 §2.2 `WorkspaceStore::add_short_term` |
| `hieronymus_session_start`, `_complete` | 003 §2.2 `WorkspaceStore` |
| `hieronymus_short_term_add`, `_add_batch` | 003 §2.2 `WorkspaceStore` |
| `hieronymus_recall` | 003 §3 `RecallService::recall` |
| `hieronymus_feedback` | 003 §2.5 `FeedbackStore::record` |
| `hieronymus_dream` | 004 §7 `DreamService::run_cycle` |
| `hieronymus_rag_import`, `_search` | 003 §4 `RagStore` |
| `hieronymus_concept_proposals_list` | 003 §2.3 `ConceptProposalStore::list_pending` |
| **`hieronymus_recall_feedback`** (new) | 003 §2.5 `FeedbackStore::record_recall_outcome` — schema description instructs the calling agent to report `{useful: [ids], miss: [ids]}` after acting on `hieronymus_recall` results |

Each tool's JSON schema mirrors the corresponding store method's input struct — tool schemas
are mechanically derived from the Rust types in 003/004, not independent design surface, except
`hieronymus_recall_feedback`'s description text, which is deliberately written to prompt the
calling agent's behavior (003 §3).

---

## 4. Admin & Config REST API

Pinned from `frontend/src/web/lib/api.ts`'s actual `fetch()` calls, not redesigned — the
existing Svelte console calls these exact paths and expects these exact response shapes today:

| Endpoint | Request | Response shape | Backing call |
|---|---|---|---|
| `GET /api/providers` | — | `{ providers: ProviderProfile[] }` | 004 §2 `ProviderCatalog::load` |
| `POST /api/providers` | `ProviderProfile` | `{ provider: ProviderProfile }` | `ProviderCatalog::save` |
| `DELETE /api/providers/{id}` | — | — | `ProviderCatalog` delete |
| `POST /api/providers/{id}/models` | — | `{ models: string[] }` | `ProviderRegistry::suggest_models` |
| `POST /api/providers/{id}/check` | `{ model: string }` | `{ ok: bool, detail: string }` | `ProviderRegistry::check` |
| `GET /api/settings/dream` | — | `{ dream: DreamConfig, providers: ProviderProfile[], model_cache: ModelCacheEntry[] }` | 004 §8 `DreamConfig::load` + provider/cache snapshots |
| `POST /api/settings/dream` | `{ dream: DreamConfig }` | `{ dream: DreamConfig }` | `DreamConfig::save` |
| `GET /api/settings/ingest` | — | `{ ingest: IngestConfig }` | `IngestConfig::load` |
| `POST /api/settings/ingest` | `{ ingest: IngestConfig }` | `{ ingest: IngestConfig }` | `IngestConfig::save` |
| `GET /api/settings/release` | — | `{ release: ReleaseInfo }` | 006 §3 release check |
| `POST /api/settings/release` | `{ release: ReleaseSettings }` | `{ release: ReleaseSettings }` | release config save |
| `GET /api/admin/dashboard` | — | `AdminDashboard` (summary counts) | cross-store summary query |
| `GET /api/admin/snapshot?view={view}&selected_id={id}` | — | `{ snapshot: AdminSnapshot }` | `AdminApi::snapshot(view, selected_id)` |
| `POST /api/admin/actions/{action}` | `{ id: i64, confirmed: Option<bool> }` | `{ result: { message: String }, snapshot: AdminSnapshot }` | `AdminApi::run_action(action, id, confirmed)` |
| `POST /api/admin/actions/run_manual_dreaming` | — | `{ result: { message: String }, snapshot: AdminSnapshot }` | 004 §7 `DreamService::run_cycle` |

```rust
pub struct AdminSnapshot { pub view: String, pub rows: Vec<serde_json::Value>, pub selected: Option<serde_json::Value>, pub detail: serde_json::Value }
pub struct AdminActionResult { pub result: ActionMessage, pub snapshot: AdminSnapshot }
pub struct ActionMessage { pub message: String }
```

Every admin action (including crystal reinforce/decay/archive from the web console) routes
through `run_action` → 003/004 store calls → `FeedbackStore::record`/`apply_score_delta` — there
is exactly one scoring code path shared by CLI, MCP, and web admin, unlike the current Python
codebase where `AdminBridge._record_immediate_feedback` (`admin.py`) duplicates
`scoring.py`'s delta table and misses the confidence-zero auto-archive check that `FeedbackStore`
applies. This is enforced structurally here, not by convention: `AdminApi::run_action`'s
implementation has no scoring logic of its own to duplicate — it can only call into 003 §2.5.

---

## 5. Signal Handling & Frontend Embedding

```rust
/// Resolves on Ctrl+C or SIGTERM (unix; falls back to a pending future elsewhere). On
/// resolution, build_router's graceful_shutdown waits for in-flight requests and the
/// dreaming/semantic-indexing loops (004 §6) to observe `shutdown` and exit.
async fn shutdown_signal();

#[derive(rust_embed::RustEmbed)]
#[folder = "frontend/dist/"]
struct Asset;

/// Serves the requested path from the embedded bundle; falls back to index.html for
/// client-side routing when the path isn't a known asset.
async fn serve_assets(path: axum::extract::Path<String>) -> impl axum::response::IntoResponse;
```

`HIERONYMUS_ASSETS_DIR` overrides the embedded bundle with a filesystem directory for frontend
development, replacing the current 11-directory ambiguous ancestor-walk `_web_asset_roots()`.
The Svelte 5 frontend itself is unchanged; §4's endpoints match what it already calls.
