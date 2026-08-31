# Rust Migration Proposal: 001 - Initial Setup

Phase 1 of migrating Hieronymus from Python to a single-binary Rust application. Covers migration
principles, the crate workspace, configuration/directory resolution, the full CLI surface,
system diagnostics, series registry, agent-plugin integration, and core dependencies.

This document, together with 002-006, is a complete, standalone specification set. Each phase
doc is implementable on its own once its stated dependencies (usually 001's types) exist —
none of them require cross-referencing an external "master" document.

---

## 1. Migration Principles

1. **Full parity, no legacy.** The Rust implementation natively implements every design
   decision from the July 18 superpowers program — no migration-on-read, no `strict_terms`
   tables, no compatibility rewrites. Fresh databases reflect the post-Plan-3 schema directly.
2. **Authoritative SQLite, rebuildable indexes.** SQLite is the single source of truth for all
   domain data. LanceDB and ONNX embedding models are derived/recoverable artifacts — deleting
   the LanceDB directory or model cache never loses data (see 003 §3).
3. **Deterministic terminology always wins.** Active rule crystals (`crystal_type = 'rule'`,
   `status = 'active'`) form a mandatory lane in recall that no amount of semantic evidence can
   override (see 003 §5, ADR 0003).
4. **One binary, one process.** The Rust binary serves HTTP, WebSocket, MCP, and runs
   background dreaming/indexing workers from a single Tokio runtime. No subprocess management,
   no PID files, no port allocation — the binary is the daemon (see 005).
5. **Stdio MCP is a compatibility shim only.** `hiero mcp` is a lightweight process that
   connects to the running daemon over HTTP (see 005 §1). Marked for removal in a future release.

### 1.1 What Changes From Python

| Python Pattern | Rust Equivalent | Rationale |
|---|---|---|
| Subprocess daemon management (`ServiceManager`, `Popen`) | Single-process Tokio runtime | Simpler lifecycle, no PID files, no port allocation |
| `click` CLI | `clap` derive macros | Compile-time arg parsing |
| `sqlite3.connect` + manual threading | `sqlx::SqlitePool` | Async connection pooling |
| Manual FTS insert/delete in application code | SQLite triggers (002 §3) | Eliminates index drift |
| `ThreadingHTTPServer` | `axum` on Tokio | All async, native WebSocket support |
| `json`/`toml`/`yaml`/`ini` parsing | `serde` + `serde_json` + `toml` | Unified serialization |
| `FastMCP` (Python SDK) | `rmcp` (Rust MCP SDK) | Native Rust, MCP v1 compatible |
| `fastembed` | `ort` (ONNX Runtime) + custom model loading | Lazy, ONNX-native |
| `lancedb` (Python) | `lancedb` (Rust crate) | Native LanceDB |
| `@dataclass(frozen=True)` | `#[derive(Debug, Clone, Serialize, Deserialize)]` structs | Equivalent safety, no `object.__setattr__` escape hatch needed |
| `re` module | `regex` crate | Same PCRE-adjacent power |
| `pulldown-cmark` (via markdownify) | `pulldown-cmark` crate | Same parser, native |
| `uv` package management | `cargo` | Standard Rust toolchain |
| `bun` frontend build | `rust-embed` + separate `bun run build` | Assets compiled into binary |
| `hatchling` build hook | `build.rs` | Cargo-native build scripts |
| `install.sh` (240+ lines) | `cargo-dist` / ~30-line fetch script | Dramatically simpler, see 006 §4 |

### 1.2 What Is Dropped Entirely

| Removed | Why |
|---|---|
| `service_manager.py` — subprocess lifecycle | Single-process daemon |
| `service_state.py` — `server.json`, PID tracking | No subprocess, no discovery state |
| `service_discovery.py` — port health polling | No subprocess to discover |
| `service_client.py` — HTTP daemon client | CLI talks to stores directly (§4 "CLI boundary rules") |
| `daemon_mcp_client.py` — ensure-daemon-running proxy | CLI uses direct store access; `hiero mcp` shim dials HTTP |
| `dream_locks.py` — fcntl file locking | `tokio::sync::Mutex` within the single process |
| `dream_autostart.py` / `session_lifecycle.py` — background threads | `tokio::time::interval` loop in the same runtime (004 §4) |
| `daemon_events.py` — manual pub/sub | Axum broadcast channels for WebSocket (005 §2) |
| `tui_bridge/` — stdio JSON-RPC protocol | Axum REST endpoints under `/api/admin`, `/api/config` (005 §5) |
| `release.py` / `release_config.py` / `release_guard.py` | `cargo-dist` binary download, GitHub Releases only, `cargo` build verification (006 §3) |
| `install.py` — Python-based plugin installer | Rust binary writes JSON/TOML configs directly (§5 below) |
| `agent_assets.py` — bundled skill strings | `rust-embed` assets |
| `memory_migration.py` — runtime compatibility rewrites | Clean schema from embedded migrations only; versioned upgrade path via `sqlx::migrate!` (002 §2, §5) |
| `strict_terms`, `strict_term_tags`, `strict_term_aliases`, `strict_terms_fts` tables | Rust schema never creates them (002 §2) |
| `hatch_build.py` — Python build hook | `build.rs` or external `bun run build` |
| `presentation.py` — CLI icons/colors | `clap` + `colored`/`indicatif` |
| `secrets.py` — API key redaction | `secrecy` crate or manual `Display` impls |

---

## 2. Crate Workspace Structure

Two-crate Cargo workspace: `hiero-core` is the domain library (no CLI/HTTP dependencies,
independently testable), `hiero-bin` is the binary that wires `hiero-core` to `clap`/`axum`/`rmcp`.

```
hieronymus/
├── Cargo.toml                     # Workspace root
├── build.rs                       # Compiles frontend assets if present
├── crates/
│   ├── hiero-core/                 # Domain library crate — see 002-004 for module contents
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs              # Re-exports all public modules
│   │       ├── config.rs           # HieronymusConfig, path resolution (§3 below)
│   │       ├── db/                 # Database layer — see 002
│   │       ├── domain/             # Domain models & stores — see 003
│   │       ├── rag/                # RAG pipeline — see 003 §4
│   │       ├── semantic/           # Semantic retrieval — see 003 §5
│   │       ├── dreaming/           # Dreaming engine — see 004
│   │       ├── provider/           # LLM providers — see 004 §1
│   │       ├── ingest/             # Ingestion config — see 003 §6
│   │       ├── agent/              # Agent integration — see §5 below
│   │       ├── registry.rs         # Series registry — see §6 below
│   │       ├── doctor.rs           # System diagnostics — see §7 below
│   │       └── values.rs           # Canonical helpers: UTC now, clamp, normalize — see 002 §4
│   │
│   └── hiero-bin/                  # Binary crate
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs             # Entrypoint (§8 below)
│           ├── cli/                # clap command definitions — see §4 below
│           ├── daemon.rs           # Axum server startup, routes, lifespan — see 005 §2
│           ├── mcp/                # MCP server interfaces — see 005 §1
│           └── api/                # REST API endpoints — see 005 §5
│
├── migrations/                     # Embedded SQL migrations — see 002
│
├── frontend/                       # Svelte 5 web console (unchanged)
│   └── dist/                       # Compiled by build.rs into binary
│
└── tests/                          # Integration tests — see 006 §1
```

Every store/service constructor in `hiero-core` takes `&HieronymusConfig` or `&SqlitePool`
(never owns them), so `hiero-bin` composes the object graph explicitly in `main.rs`.

---

## 3. Configuration & Directory Resolution

```rust
pub struct HieronymusConfig {
    pub data_root: PathBuf,
}

impl HieronymusConfig {
    /// Resolves data_root from (in order): explicit arg, HIERONYMUS_DATA_ROOT env var,
    /// XDG_DATA_HOME/hieronymus, default ~/.local/share/hieronymus.
    pub fn load(data_root: Option<PathBuf>) -> Result<Self>;

    pub fn database_path(&self) -> PathBuf;       // {data_root}/hieronymus.db
    pub fn config_root(&self) -> PathBuf;         // XDG_CONFIG_HOME/hieronymus, defaults ~/.config/hieronymus
    pub fn provider_config_path(&self) -> PathBuf;
    pub fn dream_config_path(&self) -> PathBuf;
    pub fn ingest_config_path(&self) -> PathBuf;
    pub fn semantic_config_path(&self) -> PathBuf;
    pub fn llm_cache_path(&self) -> PathBuf;      // llm-cache.json (renamed from llmcache.tmp)
    pub fn lancedb_dir(&self) -> PathBuf;
    pub fn models_dir(&self) -> PathBuf;
    pub fn daemon_log_path(&self) -> PathBuf;
    pub fn agent_plugins_dir(&self) -> PathBuf;
    pub fn dream_cycle_lock_path(&self) -> PathBuf;   // {data_root}/dream-cycle.lock — see 004 §1
    pub fn dream_cycle_state_path(&self) -> PathBuf;  // {data_root}/dream-cycle.json — see 004 §1
    /// Resolves the daemon's bind port: explicit --port arg, else HIERONYMUS_PORT env var,
    /// else 9768. No further fallback — an occupied port fails startup rather than silently
    /// trying another one (005 §2).
    pub fn resolve_port(&self, cli_arg: Option<u16>) -> u16;
}
```

**Directory standards honored:**
- Config files: `XDG_CONFIG_HOME/hieronymus`, defaulting to `~/.config/hieronymus`.
- Data/cache files (SQLite DB, logs, LanceDB, model cache, dream-cycle lock): `XDG_DATA_HOME/hieronymus`
  (`~/.local/share/hieronymus`), overridable via `HIERONYMUS_DATA_ROOT`.
- Daemon port: `--port` CLI arg, else `HIERONYMUS_PORT` env var, else `9768`.
- No `config_root` alias returning the same value as `data_root` (dropped Python dead code).

**Data root layout:**

```
~/.local/share/hieronymus/        (or $HIERONYMUS_DATA_ROOT)
├── hieronymus.db                 # SQLite database
├── lancedb/                      # Derived vector index (003 §3)
├── models/                       # ONNX model cache (003 §3)
├── daemon.log                    # Bounded log
├── dream-cycle.lock              # Cross-process dream lock (004 §1) — runtime state, not user config
├── dream-cycle.json              # Lock owner/pid/token (004 §1)
└── agent-plugins/                # Generated agent configs (not in git)

~/.config/hieronymus/             (or $XDG_CONFIG_HOME/hieronymus)
├── provider.conf                 # ADR 0007
├── dream.conf
├── ingest.conf
├── semantic.conf
├── llm-cache.json
└── auth-token                    # 0600 bearer credential — see 005 §2
```

### 3.1 Sub-configs

| Config | Struct | Methods | Notes |
|---|---|---|---|
| `provider.conf` | `ProviderCatalog` | `load()`, `save()`, profile CRUD | ADR 0007: separate from `dream.conf` |
| `dream.conf` | `DreamConfig { workflows: Vec<WorkflowProfile>, schedule_interval_minutes: u64, .. }` | `load()`, `save()`, `validate() -> Result<Self>` | No longer owns provider profiles |
| `ingest.conf` | `IngestConfig { short_memory_limits: ShortMemoryLimits, .. }` | `load()`, `save()` | |
| `semantic.conf` | `SemanticConfig { provider: String, model: String, dimensions: usize, .. }` | `load()`, `save()` | Defaults: `local`, `paraphrase-multilingual-MiniLM-L12-v2`, 384 |

---

## 4. Full CLI Schema (`clap`)

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "hiero")]
#[command(about = "Hieronymus local-first translation memory manager")]
pub struct Cli {
    #[arg(long)]
    pub data_root: Option<PathBuf>,
    #[arg(short, long)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the daemon (no-op if already running — the binary IS the daemon)
    Start { #[arg(long, default_value = "127.0.0.1")] host: String, #[arg(long, default_value_t = 9768)] port: u16 },
    /// Show daemon status
    Status { #[arg(long)] json: bool },
    /// Stop the daemon (HTTP POST /shutdown)
    Stop,
    /// Open the web console
    Config { #[arg(long)] json: bool },
    /// Run system diagnostics — see §7
    Doctor,

    // Series — see §6
    SeriesCreate { slug: String, title: String, source_language: String, target_language: String },
    SeriesList { #[arg(long)] json: bool },
    SeriesInit { slug: String },

    // Termbase — see 003 §3.4
    ProposeTerm { series_slug: String, category: String, source_text: String, canonical: String, #[arg(long)] tags: Vec<String>, #[arg(long)] notes: Option<String> },
    Approve { series_slug: String, term_id: i64 },
    Validate { series_slug: String, translated_text: String, #[arg(long)] raw_text: Option<String>, #[arg(long)] source_text: Option<String> },

    // Memory & recall — see 003
    Search { series_slug: String, query: String, #[arg(long, default_value_t = 5)] limit: usize },
    Remember { series_slug: String, kind: String, text: String },
    RememberShort { session_id: i64, kind: String, text: String },
    Forget { crystal_id: i64 },
    SessionStart { series_slug: String },
    SessionComplete { session_id: i64 },
    Recall { session_id: i64, series_slug: String, query: String, #[arg(long, default_value_t = 10)] limit: usize },

    // RAG — see 003 §4
    Rag { #[command(subcommand)] cmd: RagCommand },
    Feedback { session_id: i64, correction_text: String },
    Dream { #[arg(long)] provider: Option<String>, #[arg(long)] wait: bool },

    // Concepts — see 003 §3.3
    ConceptList { series_slug: String },
    ConceptCreate { series_slug: String, name: String },
    ConceptUpdate { concept_id: i64, #[arg(long)] name: Option<String> },
    ConceptArchive { concept_id: i64 },
    ConceptMerge { source_id: i64, target_id: i64, reason: String },
    ConceptRename { concept_id: i64, new_name: String },
    ConceptFacetAdd { concept_id: i64, kind: String, text: String },
    ConceptFacetUpdate { facet_id: i64, text: String },
    ConceptFacetList { concept_id: i64 },
    ConceptFacetSetCanonical { facet_id: i64 },
    ConceptSemanticTagsSet { concept_id: i64, tags: Vec<String> },
    CrystalValidate { crystal_id: i64 },
    ConceptProposalsList { series_slug: String },

    // Agent/Skills — see §5
    Install { #[arg(long)] app: Option<String>, #[arg(long)] dry_run: bool },
    SkillsInstall { targets: Vec<String>, #[arg(long)] dry_run: bool },
    SkillsUninstall { targets: Vec<String>, #[arg(long)] dry_run: bool },

    // Update — see 006 §3
    Update { #[arg(long)] check_only: bool },

    /// Stdio MCP compatibility shim — see 005 §1
    Mcp,
}

#[derive(Subcommand)]
pub enum RagCommand {
    Import { series_slug: String, path: PathBuf, #[arg(long)] source_type: Option<String> },
    Search { series_slug: String, query: String, #[arg(long, default_value_t = 10)] limit: usize },
    IndexStatus,
    IndexRebuild,
    IndexCancel,
}
```

**CLI boundary rules:** `search`, `remember`, `recall`, `rag-*`, `dream`, `doctor`, `validate`,
`propose-term`, `approve`, `session-*`, `concept-*` talk directly to `hiero-core` stores using a
short-lived `SqlitePool`. Only `status`, `stop`, `config` make HTTP calls to a running daemon.
`hiero` with no subcommand starts the daemon if not running, otherwise prints status.

---

## 5. Agent Integration

```rust
#[async_trait]
pub trait AgentPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn detect(&self) -> AgentAvailability;
    fn install_plan(&self, ctx: &ProjectAgentContext) -> Result<InstallPlan>;
    fn apply(&self, plan: &InstallPlan, dry_run: bool) -> Result<()>;
}

pub struct AgentAvailability { pub installed: bool, pub detect_paths: Vec<PathBuf>, pub config_paths: Vec<PathBuf> }
pub struct InstallStep { pub description: String, pub path: PathBuf }
pub struct InstallPlan { pub steps: Vec<InstallStep> }

pub fn agent_plugins() -> Vec<Box<dyn AgentPlugin>>;
pub fn resolve_plugin(name: &str) -> Option<Box<dyn AgentPlugin>>;

// Helpers shared by all plugin implementations:
pub fn load_json_object(path: &Path) -> Result<Map<String, Value>>;
pub fn patch_json_config(path: &Path, patch: impl FnOnce(&mut Map<String, Value>)) -> Result<()>;
pub fn load_toml_object(path: &Path) -> Result<toml::Table>;
pub fn patch_toml_config(path: &Path, patch: impl FnOnce(&mut toml::Table)) -> Result<()>;
pub fn atomic_write_text(path: &Path, contents: &str) -> Result<()>;  // 0600 permissions
```

Plugin implementations: `claude.rs`, `codex.rs` write HTTP MCP URL config
(`http://127.0.0.1:9768/mcp`) directly. `gemini.rs`, `opencode.rs`, `openclaw.rs` write a
`hieronymus-mcp` stdio command config (compatibility shim, see 005 §1).

```rust
pub fn discover_context(cwd: &Path) -> Option<ProjectAgentContext>;  // walks up for .hieronymus.json

pub struct SkillPlan { pub installed: Vec<PathBuf>, pub skipped: Vec<PathBuf> }
pub fn install_skills(workspace: &Path, targets: &[String], dry_run: bool) -> Result<SkillPlan>;
pub fn uninstall_skills(workspace: &Path, targets: &[String], dry_run: bool) -> Result<SkillPlan>;
pub fn skill_assets() -> HashMap<String, String>;  // bundled via rust-embed
```

`hiero agent-hook session-start` / `session-end` subcommands (folded into the main CLI, not a
separate binary) call `discover_context()` and `WorkspaceStore::start_session`/`complete_session`
(003 §3.2).

---

## 6. Series Registry

```rust
pub struct SeriesRegistry<'a> { pool: &'a SqlitePool }

impl<'a> SeriesRegistry<'a> {
    pub async fn create(&self, slug: &str, title: &str, source_lang: &str, target_lang: &str) -> Result<SeriesRecord>;
    pub async fn list(&self) -> Result<Vec<SeriesRecord>>;
    pub async fn get(&self, slug: &str) -> Result<SeriesRecord>;
    pub async fn init(&self, slug: &str) -> Result<()>;  // seeds default language/story-scope tags
}
```

---

## 7. Doctor (System Diagnostics)

```rust
pub struct DoctorReport { pub checks: Vec<DoctorCheck> }
pub struct DoctorCheck { pub name: String, pub status: CheckStatus, pub detail: String }
pub enum CheckStatus { Ok, Warn, Fail }

pub async fn run_doctor(config: &HieronymusConfig, pool: &SqlitePool) -> Result<DoctorReport>;
```

Checks performed: database schema/migration-ledger validity (002 §5), FTS integrity against
trigger-owned indexes (002 §3), agent plugin detection (§5), provider connections with API-key
redaction (004 §1), RAG source/chunk counts, semantic index health without triggering a model
download (003 §3), config file TOML validity, Rust toolchain / Bun (dev-only) presence.

---

## 8. Entrypoint & Dependency Composition

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = HieronymusConfig::load(cli.data_root)?;
    match cli.command {
        Commands::Start { host, port } => daemon::run(config, host, port).await,   // see 005 §2
        Commands::Status { json } | Commands::Stop | Commands::Config { json } => {
            daemon::client_command(&config, cli.command).await                     // HTTP to running daemon
        }
        Commands::Mcp => mcp::stdio_shim::run(&config).await,                      // see 005 §1
        other => cli::dispatch_direct(&config, other).await,                       // direct store access
    }
}
```

`daemon::run`, `mcp::stdio_shim::run`, and `cli::dispatch_direct` are specified in 005 and the
relevant store sections of 003/004 — this file only fixes their signatures and call sites.

---

## 9. Dependencies

```toml
# crates/hiero-core/Cargo.toml
[dependencies]
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "migrate", "macros"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
toml = "0.8"
regex = "1"
chrono = { version = "0.4", features = ["serde"] }
async-trait = "0.1"
thiserror = "2"
pulldown-cmark = "0.12"
csv = "1"
lancedb = "0.17"          # 003 §3: semantic index
ort = "2"                 # 003 §3: ONNX embeddings
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
async-openai = "0.27"     # 004 §1: OpenAI-compatible providers
secrecy = "0.10"          # API key redaction
uuid = { version = "1", features = ["v4"] }
sha2 = "0.10"             # checksums for cache identity, vector identity

# crates/hiero-bin/Cargo.toml
[dependencies]
hiero-core = { path = "../hiero-core" }
clap = { version = "4", features = ["derive"] }
axum = { version = "0.8", features = ["ws"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["trace", "cors", "fs"] }
tokio = { version = "1", features = ["full"] }
rust-embed = "8"
mime_guess = "2"
rmcp = "0.4"              # 005 §1: Rust MCP SDK
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
indicatif = "0.17"        # progress bars for CLI
colored = "2"             # terminal colors
```
