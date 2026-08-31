# Rust Migration Proposal: 002 - Database & FTS5 Integration

Phase 2 of the Rust migration (depends on 001 for `HieronymusConfig`). Covers the async SQLite
layer, embedded/versioned migrations, the full schema (pinned directly from the current
`src/hieronymus/migrations/global.sql`), trigger-owned FTS5 indexes, row model structs, and
canonical value helpers shared by every store in `hiero-core`.

**Parity target:** every table in the current `global.sql` except the ones listed as dropped
below — this schema is transcribed directly from the real file, not reconstructed from memory.
**Legacy not carried forward:** `strict_terms`/`strict_term_tags`/`strict_term_aliases`/
`strict_terms_fts` (superseded by rule-intent crystals, 003 §4), `ensure_column()` /
`ensure_global_compatibility_columns()` / `ensure_concept_facet_compatibility()` /
`ensure_concepts_allow_duplicate_names()` and `GLOBAL_COMPATIBILITY_COLUMNS` (Python's
runtime schema-patching layer — the Rust schema starts correct and upgrades only through
versioned migrations, never through patching at connection time).

---

## 1. Storage Libraries & Connection Setup

* **Database driver**: `sqlx` (features `"sqlite"`, `"runtime-tokio"`, `"migrate"`, `"macros"`).
* **Connection pooling**: `sqlx::SqlitePool`, shared via `Arc` across the HTTP daemon, the
  stdio MCP shim's direct-store CLI calls, and the background dreaming/indexing loops (004 §5).

SQLite defaults are not safe for concurrent access from multiple async tasks in the same
process — pragmas must be set explicitly on every connection the pool opens, not left to
driver/library defaults:

```rust
pub async fn connect(config: &HieronymusConfig) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(config.database_path())
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new().max_connections(8).connect_with(opts).await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    Ok(pool)
}
```

`sqlx::migrate!`'s path argument resolves relative to the invoking crate's
`CARGO_MANIFEST_DIR` at compile time. Since only `hiero-bin` calls this (`hiero-core` exposes
`connect()`, `hiero-bin`'s `main.rs` is what actually runs it — see 001 §8), the path is fixed
at `"../../migrations"` from `crates/hiero-bin/`. If `hiero-core`'s own test suite also needs
embedded migrations (for its `#[cfg(test)]` integration tests, 006 §1.2), it embeds them
separately with its own relative path (`"../../migrations"` from `crates/hiero-core/`) — the
two crates each get their own `sqlx::migrate!` call pointed at the same physical directory, not
a shared macro invocation, since the macro is resolved at compile time per crate.

---

## 2. Full Schema

Transcribed from `global.sql` (current file, 514 lines), translated to the Rust migration's
`CREATE TABLE`/`CREATE VIRTUAL TABLE` statements verbatim in structure. Foreign keys to series
use `series_slug text references series(slug)` throughout — **not** a `series_id` integer FK;
only `series_language_tags` uses `series_id`. `concepts` has **no** direct series FK at all —
it scopes via `scope_type`/`scope_key` (a `global` scope has empty `scope_key`; any other scope
type requires a non-empty `scope_key`, enforced by a `CHECK` constraint).

```sql
-- Series
create table series (
  id integer primary key, slug text not null unique, title text not null,
  default_source_language text not null, default_target_language text not null,
  created_at text not null, updated_at text not null
);
create table series_language_tags (
  series_id integer not null references series(id) on delete cascade,
  language_tag text not null, created_at text not null default (datetime('now')),
  primary key (series_id, language_tag)
);

-- Sessions
create table task_sessions (
  id integer primary key, series_slug text not null references series(slug),
  source_language text not null, target_language text not null, task_type text not null,
  volume text not null default '', chapter text not null default '',
  status text not null, cycle_id integer,
  created_at text not null, last_activity_at text not null, completed_at text
);
create table task_session_language_tags (session_id integer not null references task_sessions(id) on delete cascade, language_tag text not null, primary key (session_id, language_tag));
create table task_session_story_scopes (session_id integer not null references task_sessions(id) on delete cascade, story_scope text not null, primary key (session_id, story_scope));
create table task_session_semantic_tags (session_id integer not null references task_sessions(id) on delete cascade, semantic_tag text not null, primary key (session_id, semantic_tag));

-- Short-term memory
create table short_term_memories (
  id integer primary key, session_id integer not null references task_sessions(id) on delete cascade,
  source_role text not null, kind text not null, text text not null,
  source_ref text not null default '', metadata_json text not null default '{}',
  source_credibility text, rule_intent text, soft_origin text,
  source_crystal_id integer references crystals(id) on delete set null,  -- NEW: reconsolidation working-copy marker, see 003 §3
  created_at text not null, archived_at text
);
create table short_term_memory_language_tags (memory_id integer not null references short_term_memories(id) on delete cascade, language_tag text not null, primary key (memory_id, language_tag));
create table short_term_memory_story_scopes (memory_id integer not null references short_term_memories(id) on delete cascade, story_scope text not null, primary key (memory_id, story_scope));
create table short_term_memory_semantic_tags (memory_id integer not null references short_term_memories(id) on delete cascade, semantic_tag text not null, primary key (memory_id, semantic_tag));
create virtual table short_term_memories_fts using fts5(text, content='short_term_memories', content_rowid='id');

-- Crystals (long-term memory; crystal_type in {lesson, rule, thought, observation, concept_note, concept, erudition})
create table crystals (
  id integer primary key, crystal_type text not null, text text not null, title text not null default '',
  scope_type text not null, scope_key text not null default '',
  series_slug text not null default '', source_language text not null default '', target_language text not null default '',
  tags_json text not null default '[]', strength real not null, confidence real not null,
  source_credibility text not null default 'observation', rule_intent text not null default '',
  soft_origin text, is_inferred integer not null default 0, malformed_penalty real not null default 0.0,
  supersedes_crystal_id integer references crystals(id) on delete set null,
  status text not null, created_cycle integer not null default 0,
  last_activated_cycle integer, last_reinforced_cycle integer,
  created_at text not null, updated_at text not null
);
create table crystal_language_tags (crystal_id integer not null references crystals(id) on delete cascade, language_tag text not null, primary key (crystal_id, language_tag));
create virtual table crystals_fts using fts5(title, text, content='crystals', content_rowid='id');
create table crystal_sources (crystal_id integer not null references crystals(id) on delete cascade, short_term_memory_id integer not null references short_term_memories(id) on delete cascade, primary key(crystal_id, short_term_memory_id));
create table crystal_links (source_crystal_id integer not null references crystals(id) on delete cascade, target_crystal_id integer not null references crystals(id) on delete cascade, link_type text not null, primary key(source_crystal_id, target_crystal_id, link_type));
create table crystal_activations (
  id integer primary key, crystal_id integer not null references crystals(id) on delete cascade,
  session_id integer not null references task_sessions(id) on delete cascade,
  recall_query text not null, rank integer not null, score real not null, reason text not null default '',
  outcome text,  -- NEW: 'useful' | 'miss' | NULL, set by FeedbackStore::record_recall_outcome, see 003 §3
  cycle_id integer, created_at text not null
);
create table crystal_concepts (crystal_id integer not null references crystals(id) on delete cascade, concept_id integer not null references concepts(id) on delete cascade, link_type text not null default 'mentions', confidence real not null default 0.2, created_at text not null, primary key(crystal_id, concept_id, link_type));
create table crystal_story_scopes (crystal_id integer not null references crystals(id) on delete cascade, scope text not null, confidence real not null default 0.2, created_at text not null, primary key(crystal_id, scope));
create table crystal_semantic_tags (crystal_id integer not null references crystals(id) on delete cascade, tag text not null, confidence real not null default 0.2, created_at text not null, primary key(crystal_id, tag));

-- Concepts (worldbuilding graph — scoped via scope_type/scope_key, no series FK)
create table concepts (
  id integer primary key, canonical_name text not null, description text not null default '',
  scope_type text not null default 'global', scope_key text not null default '',
  status text not null default 'candidate', confidence real not null default 0.2,
  merged_into_concept_id integer references concepts(id),
  created_at text not null, updated_at text not null,
  check ((scope_type = 'global' and scope_key = '') or (scope_type != 'global' and scope_key != ''))
);
create virtual table concepts_fts using fts5(canonical_name, description, content='concepts', content_rowid='id');
create table concept_facets (
  id integer primary key, concept_id integer not null references concepts(id) on delete cascade,
  language text not null default '', facet_type text not null, value text not null,
  source_crystal_id integer references crystals(id) on delete set null,
  confidence real not null default 0.2, is_canonical integer not null default 0, superseded_at text,
  created_at text not null, updated_at text not null
);
create virtual table concept_facet_fts using fts5(value, content='concept_facets', content_rowid='id');
create table concept_facet_language_tags (facet_id integer not null references concept_facets(id) on delete cascade, language_tag text not null, primary key (facet_id, language_tag));
create table concept_facet_story_scopes (facet_id integer not null references concept_facets(id) on delete cascade, story_scope text not null, primary key (facet_id, story_scope));
create table concept_facet_semantic_tags (facet_id integer not null references concept_facets(id) on delete cascade, semantic_tag text not null, primary key (facet_id, semantic_tag));
create table concept_semantic_tags (concept_id integer not null references concepts(id) on delete cascade, tag text not null, confidence real not null default 0.2, created_at text not null, primary key(concept_id, tag));
create table concept_renames (id integer primary key, concept_id integer not null references concepts(id) on delete cascade, old_name text not null, new_name text not null, reason text not null default '', dream_run_id integer references dream_runs(id) on delete set null, created_at text not null);
create table concept_proposals (  -- renamed from strict_concept_proposals: not strict-term-specific anymore
  id integer primary key, dream_run_id integer references dream_runs(id) on delete set null,
  series_slug text not null default '', source_language text not null, target_language text not null,
  concept_text text not null, source_form text not null, canonical_rendering text not null,
  approved_variants_json text not null default '[]', forbidden_variants_json text not null default '[]',
  rationale text not null default '', status text not null, created_at text not null, updated_at text not null
);

-- Events, dreaming, audit
create table memory_events (
  id integer primary key, crystal_id integer references crystals(id) on delete set null,
  session_id integer references task_sessions(id) on delete set null, event_type text not null,
  source_role text not null, evidence text not null default '',
  strength_delta real not null default 0, confidence_delta real not null default 0,
  applied integer not null default 0, cycle_id integer, created_at text not null
);
create table dream_runs (id integer primary key, cycle_id integer not null unique, status text not null, provider text not null, input_count integer not null default 0, created_crystal_count integer not null default 0, proposal_count integer not null default 0, error text not null default '', created_at text not null, completed_at text);
create table dream_phase_runs (id integer primary key, dream_run_id integer not null references dream_runs(id) on delete cascade, phase text not null, provider_profile text not null, provider_type text not null, model text not null, status text not null, input_count integer not null default 0, output_count integer not null default 0, error text not null default '', prompt_hash text not null default '', created_at text not null, completed_at text);
create table dream_audit_entries (id integer primary key, dream_run_id integer not null references dream_runs(id) on delete cascade, phase_run_id integer references dream_phase_runs(id) on delete set null, event_type text not null, severity text not null default 'info', summary text not null, payload_json text not null default '{}', created_at text not null);
create table audit_log (id integer primary key, actor text not null default 'admin', action text not null, entity_type text not null, entity_id text not null, note text not null default '', before_json text not null default '{}', after_json text not null default '{}', created_at text not null);
create table migration_ledger (source_table text not null, source_id text not null, target_table text not null, target_id integer not null, created_at text not null default (datetime('now')), primary key (source_table, source_id, target_table));

-- RAG
create table rag_sources (id integer primary key, series_slug text not null references series(slug) on delete cascade, source_ref text not null, source_type text not null, content_type text not null, checksum text not null, metadata_json text not null default '{}', created_at text not null, updated_at text not null, unique(id, series_slug), unique(series_slug, source_ref));
create table rag_chunks (
  id integer primary key, source_id integer not null references rag_sources(id) on delete cascade,
  series_slug text not null references series(slug) on delete cascade,
  chunk_kind text not null, text text not null, display_text text not null, location text not null default '',
  metadata_json text not null default '{}', created_at text not null,
  foreign key (source_id, series_slug) references rag_sources(id, series_slug) on delete cascade
);
create index rag_chunks_source_id_idx on rag_chunks(source_id);
create index rag_chunks_series_slug_idx on rag_chunks(series_slug);
create table rag_chunk_language_tags (chunk_id integer not null references rag_chunks(id) on delete cascade, language_tag text not null, primary key (chunk_id, language_tag));
create table rag_chunk_story_scopes (chunk_id integer not null references rag_chunks(id) on delete cascade, story_scope text not null, primary key (chunk_id, story_scope));
create table rag_chunk_semantic_tags (chunk_id integer not null references rag_chunks(id) on delete cascade, semantic_tag text not null, primary key (chunk_id, semantic_tag));
create virtual table rag_chunks_fts using fts5(text, display_text, location, content='rag_chunks', content_rowid='id');

-- Semantic index state (new, Plan 4 / 003 §5)
create table semantic_index_jobs (id integer primary key, status text not null default 'pending', generation_id text, created_at text not null, completed_at text);
create table semantic_chunk_state (chunk_id integer primary key references rag_chunks(id), checksum text not null, generation_id text not null, indexed_at text not null);

-- Compound index for bounded dream maintenance (004 §3)
create index idx_crystals_maintenance on crystals(status, crystal_type, last_reinforced_cycle, last_activated_cycle, id);
```

`strict_terms`, `strict_term_tags`, `strict_term_aliases`, `strict_terms_fts` are never created.

---

## 3. Trigger-Owned FTS5 Indexes

`concepts_fts`, `concept_facet_fts`, and `rag_chunks_fts` **already have** triggers in the
current Python schema — those are carried forward. `crystals_fts` and `short_term_memories_fts`
currently have **no** triggers (the actual bug Plan 1 fixes: `workspace.py`/`crystals.py`
insert into their FTS tables manually and never delete, so cascade deletes and admin edits
orphan entries). `strict_terms_fts` triggers aren't needed since the table is dropped.

All six trigger sets are scoped with `UPDATE OF <indexed columns>`, not a blanket
`AFTER UPDATE` — the current Python schema's three existing trigger sets fire on *any* column
update (including `crystals.strength`/`confidence`/`last_reinforced_cycle`, which change on
every reinforcement/decay pass), causing a wasteful full FTS delete+reinsert on writes that
never touch indexed text. This migration fixes that for all six tables, not just the two new
ones — carried-forward triggers get the same efficiency fix while they're being touched anyway:

```sql
CREATE TRIGGER crystals_ai AFTER INSERT ON crystals BEGIN
  INSERT INTO crystals_fts(rowid, title, text) VALUES (new.id, new.title, new.text);
END;
CREATE TRIGGER crystals_ad AFTER DELETE ON crystals BEGIN
  INSERT INTO crystals_fts(crystals_fts, rowid, title, text) VALUES ('delete', old.id, old.title, old.text);
END;
CREATE TRIGGER crystals_au AFTER UPDATE OF title, text ON crystals BEGIN
  INSERT INTO crystals_fts(crystals_fts, rowid, title, text) VALUES ('delete', old.id, old.title, old.text);
  INSERT INTO crystals_fts(rowid, title, text) VALUES (new.id, new.title, new.text);
END;

CREATE TRIGGER short_term_memories_ai AFTER INSERT ON short_term_memories BEGIN
  INSERT INTO short_term_memories_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER short_term_memories_ad AFTER DELETE ON short_term_memories BEGIN
  INSERT INTO short_term_memories_fts(short_term_memories_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER short_term_memories_au AFTER UPDATE OF text ON short_term_memories BEGIN
  INSERT INTO short_term_memories_fts(short_term_memories_fts, rowid, text) VALUES ('delete', old.id, old.text);
  INSERT INTO short_term_memories_fts(rowid, text) VALUES (new.id, new.text);
END;

-- concepts_ai/ad/au, concept_facets_ai/ad/au, rag_chunks_ai/ad/au: carried forward from the
-- current schema unchanged in insert/delete shape; their AU triggers narrow from blanket
-- `AFTER UPDATE` to `AFTER UPDATE OF canonical_name, description` / `AFTER UPDATE OF value` /
-- `AFTER UPDATE OF text, display_text, location` respectively.
```

---

## 4. Migration Sequence

| # | Migration | Effect |
|---|---|---|
| `0001` | `initial_schema.sql` | Creates all tables in §2 (minus `strict_terms*`), records baseline in `migration_ledger` |
| `0002` | `fts_triggers.sql` | Creates/narrows the §3 triggers, rebuilds FTS content from existing rows |
| `0003` | `compound_indexes.sql` | Adds `idx_crystals_maintenance` |
| `0004` | `semantic_index_state.sql` | Adds `semantic_index_jobs`, `semantic_chunk_state` |
| `0005` | `drop_strict_terms.sql` | Converts `strict_terms` rows to rule-intent crystals (see below), then drops all four legacy tables |

**Migration `0005` in detail.** `strict_terms` rows are *structured* data (`source_text`,
`canonical_translation`, `category`, `notes`, plus `strict_term_tags`/`strict_term_aliases`) —
not free text to run a pattern parser against. The conversion is a direct, programmatic mapping,
not a call through 003's `parse_rule` (that parser is for free-text rule statements written by
an agent, a different input entirely):

```rust
pub fn strict_term_to_crystal(term: &StrictTermRow, aliases: &[StrictTermAliasRow]) -> AddCrystalInput {
    AddCrystalInput {
        crystal_type: "rule".into(),
        title: term.source_text.clone(),
        text: format!("{} is translated as {}", term.source_text, term.canonical_translation),
        series_slug: term.series_slug.clone(),
        source_language: term.source_language.clone(),
        target_language: term.target_language.clone(),
        source_credibility: "user_rule".into(),
        rule_intent: term.category.clone(),  // e.g. "correction" — carries the term's category forward as the rule's intent label
        // aliases become crystal_semantic_tags rows; strict_term_tags become crystal_semantic_tags rows too
    }
}
```

Each converted row: (1) inserts the crystal, (2) inserts one `crystal_semantic_tags` row per
`strict_term_tags`/`strict_term_aliases` entry, (3) records a `migration_ledger` row
(`source_table = "strict_terms"`, `source_id = term.id`, `target_table = "crystals"`,
`target_id = new_crystal_id`) so the conversion is traceable and idempotent on a second run.
Only after every row converts successfully and the row counts match does the migration
`DROP TABLE` the four legacy tables — a conversion failure aborts the migration transaction
rather than partially dropping data.

Existing (Python-created) databases missing rows in `migration_ledger` for this migration are
detected by absence of a `source_table = "strict_terms"` row before `0005` runs; a fresh
database (or one already migrated) has none to convert and the migration is a no-op past the
`DROP TABLE` guard.

---

## 5. Row Model Structs

```rust
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct SeriesRecord { pub id: i64, pub slug: String, pub title: String, pub default_source_language: String, pub default_target_language: String, pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct TaskSessionRecord { pub id: i64, pub series_slug: String, pub source_language: String, pub target_language: String, pub task_type: String, pub volume: String, pub chapter: String, pub status: String, pub cycle_id: Option<i64>, pub created_at: DateTime<Utc>, pub last_activity_at: DateTime<Utc>, pub completed_at: Option<DateTime<Utc>> }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct ShortTermMemoryRecord { pub id: i64, pub session_id: i64, pub source_role: String, pub kind: String, pub text: String, pub source_ref: String, pub metadata_json: String, pub source_credibility: Option<String>, pub rule_intent: Option<String>, pub soft_origin: Option<String>, pub source_crystal_id: Option<i64>, pub created_at: DateTime<Utc>, pub archived_at: Option<DateTime<Utc>> }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct CrystalRecord {
    pub id: i64, pub crystal_type: String, pub text: String, pub title: String,
    pub scope_type: String, pub scope_key: String, pub series_slug: String,
    pub source_language: String, pub target_language: String, pub tags_json: String,
    pub strength: f64, pub confidence: f64,
    pub source_credibility: String, pub rule_intent: String, pub soft_origin: Option<String>,
    pub is_inferred: bool, pub malformed_penalty: f64, pub supersedes_crystal_id: Option<i64>,
    pub status: String, pub created_cycle: i64, pub last_activated_cycle: Option<i64>, pub last_reinforced_cycle: Option<i64>,
    pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrystalType { Lesson, Rule, Thought, Observation, ConceptNote, Concept, Erudition }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct CrystalActivationRecord { pub id: i64, pub crystal_id: i64, pub session_id: i64, pub recall_query: String, pub rank: i64, pub score: f64, pub reason: String, pub outcome: Option<String>, pub cycle_id: Option<i64>, pub created_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct CrystalLinkRecord { pub source_crystal_id: i64, pub target_crystal_id: i64, pub link_type: String }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct ConceptRecord { pub id: i64, pub canonical_name: String, pub description: String, pub scope_type: String, pub scope_key: String, pub status: String, pub confidence: f64, pub merged_into_concept_id: Option<i64>, pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct ConceptFacetRecord { pub id: i64, pub concept_id: i64, pub language: String, pub facet_type: String, pub value: String, pub source_crystal_id: Option<i64>, pub confidence: f64, pub is_canonical: bool, pub superseded_at: Option<DateTime<Utc>>, pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct ConceptProposalRecord { pub id: i64, pub dream_run_id: Option<i64>, pub series_slug: String, pub source_language: String, pub target_language: String, pub concept_text: String, pub source_form: String, pub canonical_rendering: String, pub approved_variants_json: String, pub forbidden_variants_json: String, pub rationale: String, pub status: String, pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct MemoryEventRecord { pub id: i64, pub crystal_id: Option<i64>, pub session_id: Option<i64>, pub event_type: String, pub source_role: String, pub evidence: String, pub strength_delta: f64, pub confidence_delta: f64, pub applied: bool, pub cycle_id: Option<i64>, pub created_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct RagChunkRecord { pub id: i64, pub source_id: i64, pub series_slug: String, pub chunk_kind: String, pub text: String, pub display_text: String, pub location: String, pub metadata_json: String, pub created_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct DreamRunRecord { pub id: i64, pub cycle_id: i64, pub status: String, pub provider: String, pub input_count: i64, pub created_crystal_count: i64, pub proposal_count: i64, pub error: String, pub created_at: DateTime<Utc>, pub completed_at: Option<DateTime<Utc>> }
```

**Timestamp handling.** Application code writes `datetime.now(UTC).isoformat()` almost
everywhere (`crystals.py:28`, `recall.py:88`, `concepts.py:38`, etc. — e.g.
`2026-07-18T12:34:56.789012+00:00`), but two places use SQLite's `datetime('now')` column
default (`series_language_tags.created_at`, `migration_ledger.created_at` — format
`2026-07-18 12:34:56`, no timezone, no fractional seconds). These are genuinely different
formats sharing a schema. Migration `0001` normalizes both `datetime('now')` defaults to the
same `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')` shape the rest of the schema uses, so every
timestamp column in the Rust schema is uniform RFC 3339 going forward and `sqlx`'s `chrono`
decoding (which expects one consistent format per column) never has to special-case these two
columns.

---

## 6. Canonical Value Helpers

Every duplicate-in-Python helper (`_now()` defined 12×, `_clamp_score` defined 4×) collapses to
one shared module:

```rust
pub fn utc_now() -> DateTime<Utc>;                                   // always serializes RFC 3339 with Z suffix
pub fn clamp_score(value: f64) -> f64;                                // clamp to [0.0, 1.0]
pub fn normalize_tuple<T: Eq + Hash + Clone>(items: &[T]) -> Vec<T>;  // dedup, preserve order
pub fn json_object(value: &Value) -> Result<Map<String, Value>>;      // reject arrays/scalars

/// Ported verbatim from dreaming.py:26-33 — label -> confidence weight, reused by both
/// crystallization (dream-time confidence assignment) and recall (score boost), see 003 §3.
pub const SOURCE_CREDIBILITY_CONFIDENCE: phf::Map<&str, f64> = phf::phf_map! {
    "rumor" => 0.15, "thought" => 0.2, "observation" => 0.35, "source_text" => 0.7,
    "user_suggestion" => 0.8, "expert" => 0.85, "user_rule" => 0.95,
};
```
