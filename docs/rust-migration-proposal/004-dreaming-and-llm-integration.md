# Rust Migration Proposal: 004 - Dreaming & LLM Integration

Phase 4 of the Rust migration (depends on 001 for config/CLI, 002 for schema, 003 for
`CrystalStore`, `ConceptStore`, `FeedbackStore`, `WorkspaceStore::complete_inactive`). Covers
LLM provider integrations, cross-process dream-cycle locking, the background dreaming loop,
phase responsibility splitting, and bounded memory decay.

**Parity target:** `DreamService`'s full public API and every dreaming phase (crystallization,
concept extraction, consolidation, reinforcement, reconsolidation, link reinforcement,
maintenance, validation, persistence). **Legacy not carried forward:** `DreamService` as a
single ~150-method, 3,700-line god-class — the phases below are separate structs from the
start, not extracted after the fact; `dream_autostart.py` / `session_lifecycle.py` as separate
scheduler threads (replaced by the single interval loop in §5).

**Carried forward, corrected:** the first draft of this proposal replaced `dream_locks.py`'s
cross-process `fcntl` file locking with an in-process `tokio::sync::Mutex`, reasoning "there's
only one process now." That's wrong for this codebase's actual process model: 001 §4's CLI
boundary rules have `hiero dream` talk directly to `hiero-core` stores from its own OS process,
entirely separate from the daemon process running the background loop (§5) — exactly the two
concurrent writers `dream_locks.py` protects against today. §1 below specifies the real
cross-process lock.

---

## 1. Cross-Process Dream-Cycle Locking

Ported from `dream_locks.py`'s actual mechanism — a lock file plus a state JSON recording the
owner, not just an advisory lock:

```rust
pub struct DreamCyclePaths { pub lock_file: PathBuf, pub state_json: PathBuf }
pub fn dream_cycle_paths(config: &HieronymusConfig) -> DreamCyclePaths;  // under data_root (runtime state), not config_root — see 001 §3's directory split

pub struct DreamCycleState { pub owner: String, pub pid: u32, pub started_at: DateTime<Utc>, pub token: Uuid }

pub struct DreamCycleGuard { /* holds the open, locked file handle; releases the OS lock on Drop */ }

/// Acquires an exclusive flock() on lock_file (via the `fs2` crate, cross-platform: flock on
/// Unix, LockFileEx on Windows). `wait = false` uses a non-blocking attempt (matches
/// dream_locks.py's LOCK_EX | LOCK_NB); `wait = true` blocks until acquired. On success, writes
/// state_json with the caller's owner/pid/token. Returns DreamCycleAlreadyRunning(state) if
/// another process holds the lock and wait = false.
pub fn acquire_dream_cycle_lock(config: &HieronymusConfig, owner: &str, wait: bool) -> Result<DreamCycleGuard, DreamCycleAlreadyRunning>;

pub struct DreamCycleAlreadyRunning { pub state: Option<DreamCycleState> }
```

Both call sites take this lock exactly as `dream_locks.py` does today: the CLI's `hiero dream`
(001 §4) calls `acquire_dream_cycle_lock(&config, "cli", wait)`; the daemon's background loop
(§5) calls `acquire_dream_cycle_lock(&config, "autostart", wait = false)` and skips the tick if
already held. This is a real cross-process OS-level lock, not an in-process `Mutex` — the two
callers are, and remain, separate processes.

---

## 2. LLM Provider Integrations

| Provider | Library | Notes |
|---|---|---|
| OpenAI, Ollama (OpenAI-compatible) | `async-openai` | Same client for both, different `base_url` |
| Anthropic | `reqwest` + `serde`, hand-rolled client against `https://api.anthropic.com/v1/messages` | No official Rust SDK dependency |
| Gemini | `reqwest` + `serde`, hand-rolled client against `https://generativelanguage.googleapis.com/v1beta/models/...:generateContent` | |

```rust
#[async_trait]
pub trait DreamProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn crystallize(&self, ctx: &TranslationContext, memories: &[ShortTermMemoryRecord]) -> Result<DreamOutput>;
    async fn run_pass(&self, pass: PassName, ctx: &TranslationContext, memories: &[ShortTermMemoryRecord]) -> Result<serde_json::Value>;
}

pub struct DreamOutput { pub crystals: Vec<CandidateCrystal>, pub concepts: Vec<ConceptCandidate> }
pub struct CandidateCrystal { pub crystal_type: String, pub title: String, pub text: String, pub source_credibility: String, pub rule_intent: String, pub confidence: f64 }
pub struct ConceptCandidate { pub canonical_name: String, pub facets: Vec<(String, String, String)> }  // (language, facet_type, value)

pub struct DeterministicProvider;  // non-LLM crystallization for rule-pattern-only input, implements DreamProvider

pub struct ProviderRegistry;
impl ProviderRegistry {
    pub fn resolve(&self, config: &ProviderCatalog, name: &str) -> Result<Box<dyn DreamProvider>>;  // resolves from provider.conf catalog + dream.conf workflow, per ADR 0007
    pub async fn check(&self, profile: &ProviderProfile) -> Result<ConnectionCheck>;                  // redacts API keys in output
    pub fn suggest_models(&self, provider: &str) -> Vec<String>;
}

pub struct ProviderProfile { pub id: String, pub provider: String, pub model: String, pub api_key: secrecy::SecretString, pub base_url: Option<String> }
pub struct ConnectionCheck { pub ok: bool, pub detail: String }

pub struct ModelCacheEntry { pub model: String, pub cached_at: DateTime<Utc>, pub ttl_hours: u64 }
pub struct ModelCache;
impl ModelCache {
    pub fn load(&self, path: &Path) -> Result<HashMap<String, ModelCacheEntry>>;  // TTL-based, path is llm-cache.json (001 §3)
    pub fn save(&self, path: &Path, entries: &HashMap<String, ModelCacheEntry>) -> Result<()>;  // atomic write via 001 §5's atomic_write_text
}
```

---

## 3. Split Phase Responsibilities

Each phase is a narrow struct implementing a shared trait, not a method on one god-class. Only
`Crystallizer` depends on an LLM (`DreamProvider`); `Reconsolidator` and `LinkReinforcer` are
purely algorithmic — this split matters because reconsolidation runs on every working copy every
cycle and shouldn't cost an LLM call per row:

```rust
#[async_trait]
pub trait DreamPhase: Send + Sync {
    type Input;
    type Output;
    async fn run(&self, pool: &SqlitePool, input: Self::Input) -> Result<Self::Output>;
}

pub struct Crystallizer<'a> { provider: &'a dyn DreamProvider }  // LLM-backed
impl<'a> DreamPhase for Crystallizer<'a> {
    type Input = (TranslationContext, Vec<ShortTermMemoryRecord>);
    type Output = DreamOutput;
}

pub struct Consolidator;   // concept-only: finds relationships between existing concepts, produces merge proposals — unchanged in scope from the first draft
impl DreamPhase for Consolidator {
    type Input = Vec<ConceptRecord>;
    type Output = Vec<CreateProposalInput>;  // 003 §2.3
}

pub struct ReinforcementManager;  // applies passive score deltas, extended per the reconsolidation design (see §4)
impl DreamPhase for ReinforcementManager {
    type Input = Vec<MemoryEventRecord>;  // 003 §2.5, includes recalled_again events
    type Output = Vec<i64>;               // crystal ids updated
}

pub struct DecayManager;   // see §5 below for its bounded query shape
impl DreamPhase for DecayManager {
    type Input = DecayScope;
    type Output = Vec<i64>;               // crystal ids decayed
}

pub struct Reconsolidator;  // NEW, purely algorithmic — no DreamProvider dependency; see §4
impl DreamPhase for Reconsolidator {
    type Input = Vec<(ShortTermMemoryRecord, CrystalRecord)>;  // (working copy, its source crystal)
    type Output = Vec<ReconsolidationOutcome>;
}

pub struct LinkReinforcer;  // NEW, purely algorithmic — no DreamProvider dependency; see §4
impl DreamPhase for LinkReinforcer {
    type Input = Vec<CrystalActivationRecord>;  // the cycle's activations, 003 §3
    type Output = Vec<LinkOutcome>;
}
```

`dreaming::validation` and `dreaming::persistence` are not phases themselves — they're called
by `Crystallizer`: `validation::normalize_candidate(candidate: &CandidateCrystal) -> Result<CandidateCrystal>`
and `persistence::write_candidates(tx: &mut Transaction, candidates: &[CandidateCrystal]) -> Result<Vec<i64>>`,
the latter always running inside the caller's transaction so a mid-batch failure can't leave an
orphaned crystal with no audit row.

---

## 4. Reconsolidation & Link Reinforcement

Full behavioral spec: `docs/superpowers/specs/2026-07-18-memory-reconsolidation-design.md`.

**`Reconsolidator`.** For every non-archived working-copy short-term memory
(`source_crystal_id IS NOT NULL`), computes a token-level diff ratio against its source
crystal's current text:

```rust
pub enum ReconsolidationOutcome {
    ReinforcedInPlace { crystal_id: i64 },
    Superseded { old_crystal_id: i64, new_crystal_id: i64 },
}
pub fn diff_ratio(working_copy_text: &str, source_text: &str) -> f64;  // token-level edit distance / token count
```

Below `DreamConfig::reconsolidation_diff_threshold` (default `0.20`): reinforce the original
crystal in place. At or above: call `CrystalStore::supersede` (003 §2.1) with the working
copy's text, which inserts a new crystal (`supersedes_crystal_id = old.id`) and flips the
original to `status = 'superseded'`; the new crystal's `crystal_concepts` rows are copied from
the original (not moved). Either outcome ends with `WorkspaceStore::archive` (003 §2.2) on the
working-copy row — reusing the existing `archived_at` field that already excludes processed
short-term memories from dreaming input.

**`LinkReinforcer`.** Reads the cycle's `crystal_activations` rows (`outcome IS NOT NULL`):

```rust
pub enum LinkOutcome {
    Strengthened { source_id: i64, target_id: i64 },
    Combined { survivor_id: i64, absorbed_id: i64 },
}
pub fn similarity(a: &CrystalRecord, b: &CrystalRecord, a_concepts: &[i64], b_concepts: &[i64]) -> f64;  // shared crystal_concepts overlap + text similarity
```

- **Hebbian strengthening**: crystals with `outcome = 'useful'` in the same session are
  co-activation evidence; `CrystalStore::link(source_id, target_id, "related")` (insert-or-ignore,
  so repeated co-activation is idempotent at the row level — "strengthening" is currently a
  matter of the link existing plus activation frequency read at query time, since `crystal_links`
  has no weight column, 003 §2.1).
- **Pairwise combination**: among co-activated `useful` pairs, if `similarity(...)` exceeds a
  threshold, pick a survivor (higher `SOURCE_CREDIBILITY_CONFIDENCE` weight, tie-broken by
  higher `strength`), union the other's `crystal_concepts` and `crystal_links` rows onto the
  survivor, set the absorbed crystal's `status = 'superseded'`, and record a `memory_events` row
  (`event_type = "combined_into"`, `evidence = survivor_id.to_string()`) on the absorbed
  crystal. `supersedes_crystal_id` isn't reused here — that slot is `Reconsolidator`'s
  "new crystal replaces its own prior revision" relationship; combination merges two
  independently-existing crystals, a different relationship, recorded via the same
  event-sourced pattern as `recalled_again`/`recalled_useful`/`recalled_miss`. Combination is
  pairwise only — an N-way-similar cluster resolves over multiple dream cycles.

---

## 5. Bounded & Indexed Memory Decay

Ambient decay in Python scans every eligible crystal on each dream run, which doesn't scale past
a few thousand crystals. In Rust:
- `idx_crystals_maintenance` (002 §2) covers `(status, crystal_type, last_reinforced_cycle, last_activated_cycle, id)`.
- `DecayManager` queries only a **bounded set**: crystals recalled or linked during the current
  cycle's context, plus crystals whose `last_reinforced_cycle` is older than a configurable
  staleness window — never a full table scan.
- Decay deltas run through `FeedbackStore::apply_score_delta` (003 §2.5), which applies the
  `rule_intent`/`source_credibility` dampening described there — no separate rule-immunity
  branch here.

```rust
pub struct DecayScope { pub recalled_ids: Vec<i64>, pub linked_ids: Vec<i64>, pub limit: usize }
pub fn select_decay_candidates(scope: &DecayScope) -> Vec<i64>;  // indexed, bounded to limit+1
```

---

## 6. Background Dreaming Loop

```rust
/// Runs on the daemon's Tokio runtime (005 §2), replacing DreamAutostart + SessionLifecycle
/// threads. Each tick: (1) completes stale sessions via 003 §2.2's complete_stale_sessions,
/// (2) attempts acquire_dream_cycle_lock(&config, "autostart", wait = false) — skips the tick
/// if another process (e.g. a concurrent `hiero dream` CLI invocation) holds it, (3) runs
/// DreamService::run_due() under the held lock if the pending-memory threshold is met. Exits
/// when `shutdown` fires.
pub async fn dreaming_loop(
    pool: SqlitePool,
    config: Arc<HieronymusConfig>,
    dream_config: DreamConfig,
    shutdown: tokio::sync::broadcast::Receiver<()>,
);
```

---

## 7. `DreamService` Public API

```rust
pub struct CycleOptions { pub owner: String, pub wait: bool, pub skip_when_locked: bool, pub trigger_type: String, pub ignore_minimum: bool }
pub struct MaintenancePayload { pub reinforce: Vec<i64>, pub decay: Vec<i64>, pub combine: Vec<(i64, i64)>, pub supersede: Vec<(i64, i64)> }
pub struct MaintenanceResult { pub reinforced: usize, pub decayed: usize, pub combined: usize, pub superseded: usize }

pub struct DreamService<'a> { pool: &'a SqlitePool, config: &'a HieronymusConfig, dream_config: &'a DreamConfig }
impl<'a> DreamService<'a> {
    pub async fn run_cycle(&self, opts: CycleOptions) -> Result<DreamRunRecord>;         // acquires the cross-process lock itself (§1), one batch of pending memories
    pub async fn run_all(&self, opts: CycleOptions) -> Result<DreamRunRecord>;           // repeats run_cycle until no pending memories remain
    pub async fn run_due(&self) -> Result<Option<DreamRunRecord>>;                       // called by §6's loop under an already-held lock; no-op if threshold unmet
    pub async fn decay_candidates(&self, ids: &[i64], reason: &str, deltas: ScoreDelta) -> Result<Vec<i64>>;
    pub async fn apply_maintenance(&self, payload: &MaintenancePayload, cycle_id: i64) -> Result<MaintenanceResult>;
}
```

`DreamAuditStore` (`dreaming::audit`) records every phase transition to `dream_audit_entries`
(002 §2) — `pub async fn record(&self, run_id: i64, phase_run_id: Option<i64>, event_type: &str, summary: &str, payload: &serde_json::Value) -> Result<i64>`.

---

## 8. `DreamConfig` and Workflow Resolution

Fields transcribed from the current `dream_config.py:34-48` dataclass, plus one new field for
reconsolidation:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowProfile { pub provider: String, pub model: String, pub enabled: bool, pub max_records_per_pass: usize }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamConfig {
    pub enabled: bool,
    pub schedule_interval_minutes: u64,
    pub min_pending_short_term_memories: usize,
    pub max_pending_short_term_memories: usize,
    pub max_short_term_memories_per_cycle: usize,
    pub not_enough_memories_cycle_threshold: usize,
    pub max_changed_crystals_per_cycle: usize,
    pub max_related_concepts_per_cycle: usize,
    pub max_related_crystals_per_concept: usize,
    pub max_total_affected_crystals: usize,
    pub max_short_term_memories_per_run: usize,
    pub max_long_term_records_affected_per_run: usize,
    pub max_relation_records_per_pass: usize,
    pub general_prompt: String,
    pub reconsolidation_diff_threshold: f64,  // NEW, default 0.20 — see §4
    pub workflows: HashMap<String, WorkflowProfile>,  // keyed by pass name, matching with_workflow(name, ...)'s shape
}
impl DreamConfig {
    pub fn load(config: &HieronymusConfig) -> Result<Self>;
    pub fn save(&self, config: &HieronymusConfig) -> Result<()>;
    pub fn validate(self) -> Result<Self>;
    pub fn with_workflow(self, name: &str, workflow: WorkflowProfile) -> Self;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassName { Concepts, TerminologyCandidates, RuleCrystals, KnowledgeCrystals, Relations, Reinforcement, CoverageAudit }

pub fn resolve_workflows(config: &DreamConfig) -> Vec<(PassName, WorkflowProfile)>;
pub fn build_prompt(config: &DreamConfig, phase: PassName, input: &serde_json::Value) -> String;
```

---

## 9. LLM JSON Parsing & Recovery

Crystallization requires the LLM to return structured JSON matching `DreamOutput`.
`serde_json::from_str` deserializes candidates and concepts; markdown code-fence stripping runs
before parsing. Malformed payloads accrue a penalty written to `crystals.malformed_penalty`
(002 §2) rather than being silently dropped:

```rust
pub fn strip_code_fences(raw: &str) -> &str;
pub fn parse_dream_output(raw: &str) -> Result<DreamOutput, MalformedPayloadError>;
```
