# Rust Migration Phase 002 Database and FTS Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the authoritative SQLite schema, safe connection pool, versioned migrations, trigger-owned FTS5 indexes, row models, and legacy strict-term conversion boundary.

**Architecture:** `hiero-core::db` owns connection options and a single embedded `sqlx::Migrator`; stores receive cloned `SqlitePool` handles. Before `0001`, Rust classifies and losslessly baselines supported Python schemas in one `BEGIN IMMEDIATE` shadow-rebuild transaction, recording embedded migration metadata only after row-count/FK validation. Pure SQL migrations establish fresh schema, while the data-bearing strict-term conversion runs as an explicit Rust transaction before the final SQL drop migration.

**Tech Stack:** Rust 2024, SQLx 0.9 SQLite, SQLite FTS5/WAL, chrono, serde, tempfile.

## Global Constraints

- Preserve every `global.sql` table except `strict_terms`, `strict_term_tags`, `strict_term_aliases`, and `strict_terms_fts`.
- Fresh databases start at the final schema; existing Python databases upgrade without loss.
- Enable `foreign_keys = ON`, `recursive_triggers = ON`, and a five-second busy timeout on every connection; enable WAL for file databases. Recursive triggers are required so `INSERT OR REPLACE` fires FTS delete triggers.
- Every transaction that reads before writing begins with `BEGIN IMMEDIATE` semantics.
- Serialize the complete baseline/migration/conversion protocol. File databases hold one canonical-path-derived, crash-releasing cross-process advisory sidecar lock continuously from preflight through `0005`; memory databases hold a process-wide async mutex. Never rely on SQLx's no-op SQLite migration lock.
- FTS indexes are external-content tables maintained only by triggers; stores never dual-write FTS rows.
- Store timestamps as uniform RFC 3339 UTC text.
- Never run fresh-only `0001` directly against a Python database. Baseline it first, retain every strict-term object for Task 5, restore `foreign_keys = ON`, then let the single SQLx migrator continue.
- `migrations/0005_drop_strict_terms.sql` must never run until Rust conversion and parity checks commit successfully.

---

## File Map

- `crates/hiero-core/src/db/{mod.rs,error.rs,models.rs,legacy_terms.rs,test_support.rs}`: pool, typed rows, legacy converter, tests.
- `crates/hiero-core/src/values.rs`: timestamp and score helpers shared with stores.
- `migrations/0001_initial_schema.sql` through `0005_drop_strict_terms.sql`: embedded ordered schema changes.
- `crates/hiero-core/tests/{database,fts,legacy_terms}.rs`: black-box persistence contracts.

**Focused commands:** Task 1 `cargo test -p hiero-core --test database`; Task 2 and Task 4 `cargo test -p hiero-core --test schema`; Task 3 `cargo test -p hiero-core --test fts`; Task 5 `cargo test -p hiero-core --test legacy_terms`. RED means the named contract fails for the absent behavior; GREEN means exit 0 with all named tests passed.

### Task 1: Connect and Migrate Safely

**Files:** Create `crates/hiero-core/src/db/{mod.rs,error.rs,test_support.rs}`, `crates/hiero-core/tests/database.rs`; modify crate manifest and `lib.rs`.

**Interfaces:** Produces `pub async fn connect(config: &HieronymusConfig) -> Result<SqlitePool, DbError>`, `pub async fn connect_url(url: &str) -> Result<SqlitePool, DbError>`, and `pub async fn migrate(pool: &SqlitePool) -> Result<(), DbError>`.

- [ ] Write tests asserting `PRAGMA foreign_keys = 1`, `busy_timeout = 5000`, file DB `journal_mode = wal`, max eight concurrent acquisitions, migration idempotence, and a failed migration rollback.
- [ ] Run `cargo test -p hiero-core --test database`; expect RED.
- [ ] Define one `static MIGRATOR: Migrator = sqlx::migrate!("../../migrations")` in `hiero-core`; do not claim the macro path changes based on the calling crate. Use `SqliteConnectOptions::from_str`, `create_if_missing`, WAL only for non-memory URLs, `foreign_keys(true)`, and `busy_timeout(Duration::from_secs(5))`.
- [ ] Before calling `MIGRATOR.run`, classify empty/SQLx/Python/unknown state under `BEGIN IMMEDIATE`. Validate every existing `_sqlx_migrations` table through `table_xinfo` plus normalized `sqlite_schema` DDL, rejecting hidden/generated columns, table options, extra constraints, duplicates, dirty rows, unknown versions, or metadata/checksum mismatches. Match stored rows to embedded migrations by version; missing embedded versions remain pending and SQLx applies them without disturbing stored rows. Baseline only a Python family whose complete logical object manifest matches current `global.sql` or an exact normalized `CREATE TABLE`/autoindex state produced by the real ordered `db.py` compatibility ALTER/rebuild history, including both reachable `386d1e8` pre-rebuild concept layouts, historical defaults, FK actions, unique-index origins/columns, and collations; normalize SQL with quote-aware tokens so insignificant formatting and supported identifier quotes are canonical but string/blob literal bytes, operators, and constraint order remain exact; reject arbitrary subsets and malformed near-variants before mutation. Use fixed collision-checked shadows, lossless timestamp/status normalization and rename mappings, strict-term retention, row-count/FK validation, SQLx 0.9 embedded checksum metadata, rollback on failure, and verified FK restoration before returning the connection.
- [ ] Add `after_connect` assertions for required SQLite capabilities (`sqlite_compileoption_used('ENABLE_FTS5')` or a create/drop probe) and actionable `DbError::MissingFts5`.
- [ ] Run focused tests; expect GREEN. Commit `feat: add SQLite pool and migration runner`.

### Task 2: Install the Final Fresh Schema

**Files:** Create `migrations/0001_initial_schema.sql`, `migrations/0003_compound_indexes.sql`, `migrations/0004_semantic_index_state.sql`, `crates/hiero-core/tests/schema.rs`.

**Interfaces:** Produces the exact tables, foreign keys, checks, unique constraints, and indexes specified in proposal 002 §2.

- [ ] Write schema-introspection tests over `sqlite_schema`, `pragma_foreign_key_list`, `pragma_index_list`, and `pragma_table_info`. Assert all required tables exist, all four strict-term objects do not, `concepts` uses scope fields without series FK, RAG compound FK exists, and timestamps parse as RFC 3339.
- [ ] Run `cargo test -p hiero-core --test schema`; expect RED.
- [ ] Transcribe proposal 002 §2 into `0001`, correcting declaration order for readability and using SQLite `STRICT` on ordinary tables only after a compatibility test proves all declared types are supported. Use `INTEGER CHECK (... IN (0,1))` for booleans and explicit status/range checks where the Python schema already constrains values.
- [ ] Add the partial cursor/range index `idx_crystals_maintenance(id)` for the exact Rust id-cursor decay query (`status IN`, `id >`, current-cycle creation/activation exclusions, configurable reinforcement staleness threshold, active-rule exclusion, `ORDER BY id LIMIT`); verify boundary eligibility, natural named-index use, and absence of a temporary B-tree/full scan with `EXPLAIN QUERY PLAN`.
- [ ] Run focused tests; expect GREEN. Commit `feat: create authoritative Rust schema`.

### Task 3: Make FTS Trigger-Owned

**Files:** Create `migrations/0002_fts_triggers.sql`, `crates/hiero-core/tests/fts.rs`.

**Interfaces:** Produces trigger sets `{table}_ai`, `{table}_ad`, `{table}_au` for crystals, short-term memories, concepts, concept facets, and RAG chunks.

- [ ] Write tests that insert/update/delete/replace each content row through only its base table, then query FTS and run FTS5 `integrity-check`. Assert metadata-only non-ID updates do not change FTS content, primary-key updates move the external-content rowid, and direct/multiple-path cascade deletes leave no orphan rowids.
- [ ] Run `cargo test -p hiero-core --test fts`; expect RED.
- [ ] Implement external-content FTS5 tables and insert/delete/update triggers. Narrow updates to `id,title,text`; `id,text`; `id,canonical_name,description`; `id,value`; and `id,text,display_text,location` respectively. Finish migration with each FTS table's `rebuild` command.
- [ ] Delete no Python FTS code yet; Phase 006 removes Python only after parity. Run focused tests; expect GREEN. Commit `feat: maintain FTS indexes with triggers`.

### Task 4: Add Typed Row Models and Conversion Types

**Files:** Create `crates/hiero-core/src/db/models.rs`; modify `db/mod.rs`; add model decode tests to `tests/schema.rs`.

**Interfaces:** Produces every record in proposal 002 §5 plus private `StrictTermRow`, `StrictTermAliasRow`, and typed enums for closed status/type fields where callers branch.

- [ ] Write one fixture/decode test per model, including nullable timestamps, booleans, JSON strings, malformed timestamps, and unknown closed enum values.
- [ ] Run `cargo test -p hiero-core --test schema models`; expect RED.
- [ ] Implement `FromRow` records with `DateTime<Utc>` and deliberate serde exposure. Add `TryFrom<String>` only for enums that consumers discriminate; keep forward-compatible persisted labels as strings elsewhere.
- [ ] Run focused tests and docs; expect GREEN. Commit `feat: add typed database records`.

### Task 5: Convert and Drop Legacy Strict Terms Transactionally

**Files:** Create `crates/hiero-core/src/db/legacy_terms.rs`, `crates/hiero-core/tests/legacy_terms.rs`, `migrations/0005_drop_strict_terms.sql`; modify migration orchestration in `db/mod.rs`.

**Interfaces:** Produces `pub async fn convert_legacy_strict_terms(pool: &SqlitePool) -> Result<LegacyConversionReport, DbError>` and `LegacyConversionReport { source_rows, converted_rows, existing_rows, dropped }`.

- [ ] Build a Python-era fixture with active/inactive terms, tags, aliases, duplicate ledger entries, an injected invalid row, and no legacy tables. Assert direct structured mapping, semantic-tag union/deduplication, traceable ledger rows, idempotence, rollback on mismatch, and drop only after exact source/target count parity.
- [ ] Run `cargo test -p hiero-core --test legacy_terms`; expect RED.
- [ ] In one acquired connection, execute `BEGIN IMMEDIATE`, consume the strict-term objects retained by the pre-0001 baseline, read structured rows, insert rule crystals and semantic tags with bound parameters, record ledger rows, verify exact counts and every converter-owned target/tag field (including NULL-vs-zero lifecycle fields, tag confidence, and timestamps), then drop legacy FTS/triggers/child/parent objects in safe order and commit. Do not call the free-text `parse_rule` path.
- [ ] Keep `0005_drop_strict_terms.sql` limited to guarded DDL for fresh/empty cases. Use the one embedded migrator as `run_to(4)`, invoke the Rust converter, then `run()` the same manifest through `0005`; document why a plain SQLx migration cannot call typed Rust and why this preserves the embedded checksums without filesystem or duplicated-manifest dependencies.
- [ ] Run focused tests against both fresh and copied Python schemas; expect GREEN. Commit `feat: retire legacy strict terms safely`.

## Phase Acceptance

- [ ] `cargo test -p hiero-core --test database --test schema --test fts --test legacy_terms` passes.
- [ ] `PRAGMA foreign_key_check`, all FTS orphan queries, and `PRAGMA integrity_check` return clean results.
- [ ] `rg -n 'INSERT INTO .*_fts|DELETE FROM .*_fts' crates --glob '*.rs'` finds no store-owned FTS writes.
- [ ] Migration tests prove a legacy conversion failure retains every legacy table and row.
