use std::str::FromStr;

use chrono::DateTime;
use hiero_core::db::{DbError, migrate};
use sqlx::{
    Executor, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

async fn legacy_pool() -> SqlitePool {
    legacy_pool_from_schema(include_str!(
        "../../../src/hieronymus/migrations/global.sql"
    ))
    .await
}

async fn legacy_pool_from_schema(schema: &str) -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("memory URL should parse")
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("legacy fixture pool should connect");
    pool.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(schema.to_owned())))
        .await
        .expect("current Python global schema should install");
    pool
}

fn replace_table_definition(schema: &mut String, table: &str, replacement: &str) {
    let marker = format!("create table if not exists {table} (");
    let start = schema
        .find(&marker)
        .unwrap_or_else(|| panic!("current global schema should declare {table}"));
    let end = schema[start..]
        .find("\n);")
        .map(|offset| start + offset + 3)
        .unwrap_or_else(|| panic!("{table} declaration should terminate"));
    schema.replace_range(start..end, replacement);
}

async fn historical_compatibility_pool(
    crystals: &str,
    add_late_crystal_columns: bool,
) -> SqlitePool {
    historical_compatibility_pool_with_session_stage(
        crystals,
        add_late_crystal_columns,
        PYTHON_COMPATIBILITY_STAGE_THREE,
    )
    .await
}

async fn historical_compatibility_pool_with_session_stage(
    crystals: &str,
    add_late_crystal_columns: bool,
    session_stage: &str,
) -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("memory URL should parse")
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("historical fixture pool should connect");
    let mut schema = include_str!("../../../src/hieronymus/migrations/global.sql").to_owned();
    replace_table_definition(&mut schema, "task_sessions", HISTORICAL_TASK_SESSIONS);
    replace_table_definition(
        &mut schema,
        "short_term_memories",
        HISTORICAL_SHORT_TERM_MEMORIES,
    );
    replace_table_definition(&mut schema, "crystals", crystals);
    replace_table_definition(&mut schema, "concepts", HISTORICAL_CONCEPTS);
    replace_table_definition(&mut schema, "concept_facets", HISTORICAL_CONCEPT_FACETS);
    pool.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(schema)))
        .await
        .expect("historical global schema should install");

    pool.execute(sqlx::raw_sql(PYTHON_COMPATIBILITY_STAGE_ONE))
        .await
        .expect("first Python ALTER stage should install");
    pool.execute("PRAGMA foreign_keys = OFF")
        .await
        .expect("concept rebuild should disable FKs");
    pool.execute(sqlx::raw_sql(PYTHON_CONCEPT_REBUILD))
        .await
        .expect("Python concepts compatibility rebuild should install");
    pool.execute("PRAGMA foreign_keys = ON")
        .await
        .expect("concept rebuild should restore FKs");
    if add_late_crystal_columns {
        pool.execute(sqlx::raw_sql(PYTHON_COMPATIBILITY_STAGE_TWO))
            .await
            .expect("second Python ALTER stage should install");
    }
    pool.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(session_stage.to_owned())))
        .await
        .expect("session activity ALTER stage should install");
    pool
}

async fn pre_rebuild_concepts_pool(fresh_386_declaration: bool) -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("memory URL should parse")
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("pre-rebuild concepts fixture pool should connect");
    let mut schema = include_str!("../../../src/hieronymus/migrations/global.sql").to_owned();
    replace_table_definition(&mut schema, "task_sessions", HISTORICAL_TASK_SESSIONS);
    replace_table_definition(
        &mut schema,
        "short_term_memories",
        HISTORICAL_SHORT_TERM_MEMORIES,
    );
    replace_table_definition(&mut schema, "crystals", HISTORICAL_CRYSTALS_OLDEST);
    replace_table_definition(
        &mut schema,
        "concepts",
        if fresh_386_declaration {
            HISTORICAL_CONCEPTS_FRESH_386
        } else {
            HISTORICAL_CONCEPTS
        },
    );
    replace_table_definition(&mut schema, "concept_facets", HISTORICAL_CONCEPT_FACETS);
    pool.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(schema)))
        .await
        .expect("pre-rebuild Python schema should install");
    pool.execute(sqlx::raw_sql(if fresh_386_declaration {
        PYTHON_COMPATIBILITY_STAGE_ONE_WITH_EXISTING_CONCEPT_COLUMN
    } else {
        PYTHON_COMPATIBILITY_STAGE_ONE
    }))
    .await
    .expect("386d1e8 compatibility stage should install");
    pool.execute(sqlx::raw_sql(PYTHON_COMPATIBILITY_STAGE_TWO))
        .await
        .expect("late crystal compatibility stage should install");
    pool.execute(sqlx::raw_sql(PYTHON_COMPATIBILITY_STAGE_THREE))
        .await
        .expect("session compatibility stage should install");
    pool
}

const HISTORICAL_TASK_SESSIONS: &str = r#"create table if not exists task_sessions (
  id integer primary key,
  series_slug text not null references series(slug),
  source_language text not null,
  target_language text not null,
  task_type text not null,
  volume text not null default '',
  chapter text not null default '',
  status text not null,
  cycle_id integer,
  created_at text not null,
  completed_at text
);"#;

const HISTORICAL_SHORT_TERM_MEMORIES: &str = r#"create table if not exists short_term_memories (
  id integer primary key,
  session_id integer not null references task_sessions(id) on delete cascade,
  source_role text not null,
  kind text not null,
  text text not null,
  source_ref text not null default '',
  metadata_json text not null default '{}',
  created_at text not null,
  archived_at text
);"#;

const HISTORICAL_CRYSTALS_OLDEST: &str = r#"create table if not exists crystals (
  id integer primary key,
  crystal_type text not null,
  text text not null,
  title text not null default '',
  scope_type text not null,
  scope_key text not null default '',
  series_slug text not null default '',
  source_language text not null default '',
  target_language text not null default '',
  tags_json text not null default '[]',
  strength real not null,
  confidence real not null,
  status text not null,
  created_cycle integer not null default 0,
  last_activated_cycle integer,
  last_reinforced_cycle integer,
  created_at text not null,
  updated_at text not null
);"#;

const HISTORICAL_CRYSTALS_MID_AGE: &str = r#"create table if not exists crystals (
  id integer primary key,
  crystal_type text not null,
  text text not null,
  title text not null default '',
  scope_type text not null,
  scope_key text not null default '',
  series_slug text not null default '',
  source_language text not null default '',
  target_language text not null default '',
  tags_json text not null default '[]',
  strength real not null,
  confidence real not null,
  source_credibility text not null default 'observation',
  rule_intent text not null default '',
  malformed_penalty real not null default 0.0,
  supersedes_crystal_id integer references crystals(id) on delete set null,
  status text not null,
  created_cycle integer not null default 0,
  last_activated_cycle integer,
  last_reinforced_cycle integer,
  created_at text not null,
  updated_at text not null
);"#;

const HISTORICAL_CONCEPTS: &str = r#"create table if not exists concepts (
  id integer primary key,
  canonical_name text not null,
  description text not null default '',
  scope_type text not null default 'global',
  scope_key text not null default '',
  status text not null default 'vague',
  confidence real not null default 0.2,
  created_at text not null,
  updated_at text not null,
  check ((scope_type = 'global' and scope_key = '') or (scope_type != 'global' and scope_key != '')),
  unique(scope_type, scope_key, canonical_name)
);"#;

// Exact declaration introduced by 386d1e8 for databases created at that commit.
const HISTORICAL_CONCEPTS_FRESH_386: &str = r#"create table if not exists concepts (
  id integer primary key,
  canonical_name text not null,
  description text not null default '',
  scope_type text not null default 'global',
  scope_key text not null default '',
  status text not null default 'vague',
  confidence real not null default 0.2,
  merged_into_concept_id integer references concepts(id),
  created_at text not null,
  updated_at text not null,
  check ((scope_type = 'global' and scope_key = '') or (scope_type != 'global' and scope_key != '')),
  unique(scope_type, scope_key, canonical_name)
);"#;

const HISTORICAL_CONCEPT_FACETS: &str = r#"create table if not exists concept_facets (
  id integer primary key,
  concept_id integer not null references concepts(id) on delete cascade,
  language text not null default '',
  facet_type text not null,
  value text not null,
  source_crystal_id integer references crystals(id) on delete set null,
  confidence real not null default 0.2,
  created_at text not null,
  updated_at text not null
);"#;

const PYTHON_COMPATIBILITY_STAGE_ONE: &str = r#"
alter table concept_facets add column is_canonical integer not null default 0;
alter table concept_facets add column superseded_at text;
alter table concepts add column merged_into_concept_id integer references concepts(id);
alter table short_term_memories add column source_credibility text;
alter table short_term_memories add column rule_intent text;
alter table short_term_memories add column soft_origin text;
alter table crystals add column soft_origin text;
alter table crystals add column is_inferred integer not null default 0;
"#;

const PYTHON_COMPATIBILITY_STAGE_ONE_WITH_EXISTING_CONCEPT_COLUMN: &str = r#"
alter table concept_facets add column is_canonical integer not null default 0;
alter table concept_facets add column superseded_at text;
alter table short_term_memories add column source_credibility text;
alter table short_term_memories add column rule_intent text;
alter table short_term_memories add column soft_origin text;
alter table crystals add column soft_origin text;
alter table crystals add column is_inferred integer not null default 0;
"#;

const PYTHON_COMPATIBILITY_STAGE_TWO: &str = r#"
alter table crystals add column source_credibility text not null default 'observation';
alter table crystals add column rule_intent text not null default '';
alter table crystals add column malformed_penalty real not null default 0;
alter table crystals add column supersedes_crystal_id integer references crystals(id);
"#;

const PYTHON_COMPATIBILITY_STAGE_THREE: &str = r#"
alter table task_sessions add column last_activity_at text not null default '';
update task_sessions set last_activity_at = created_at where last_activity_at = '';
"#;

const PYTHON_CONCEPT_REBUILD: &str = r#"
drop trigger if exists concepts_ai;
drop trigger if exists concepts_ad;
drop trigger if exists concepts_au;
create table concepts_new (
  id integer primary key,
  canonical_name text not null,
  description text not null default '',
  scope_type text not null default 'global',
  scope_key text not null default '',
  status text not null default 'candidate',
  confidence real not null default 0.2,
  merged_into_concept_id integer references concepts_new(id),
  created_at text not null,
  updated_at text not null,
  check (
    (scope_type = 'global' and scope_key = '')
    or (scope_type != 'global' and scope_key != '')
  )
);
insert into concepts_new(id, canonical_name, description, scope_type, scope_key, status, confidence,
  merged_into_concept_id, created_at, updated_at)
select id, canonical_name, description, scope_type, scope_key, status, confidence,
  merged_into_concept_id, created_at, updated_at from concepts;
drop table concepts;
alter table concepts_new rename to concepts;
create trigger concepts_ai after insert on concepts begin
  insert into concepts_fts(rowid, canonical_name, description)
  values (new.id, new.canonical_name, new.description);
end;
create trigger concepts_ad after delete on concepts begin
  insert into concepts_fts(concepts_fts, rowid, canonical_name, description)
  values ('delete', old.id, old.canonical_name, old.description);
end;
create trigger concepts_au after update on concepts begin
  insert into concepts_fts(concepts_fts, rowid, canonical_name, description)
  values ('delete', old.id, old.canonical_name, old.description);
  insert into concepts_fts(rowid, canonical_name, description)
  values (new.id, new.canonical_name, new.description);
end;
insert into concepts_fts(concepts_fts) values ('rebuild');
"#;

async fn schema_snapshot(pool: &SqlitePool) -> Vec<(String, String, String, String)> {
    sqlx::query_as(
        "SELECT type, name, tbl_name, coalesce(sql, '') FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .fetch_all(pool)
    .await
    .expect("legacy schema should inspect")
}

async fn assert_unknown_shape_is_unchanged(pool: &SqlitePool) {
    let before = schema_snapshot(pool).await;
    let error = migrate(pool)
        .await
        .expect_err("altered full-family Python schema must be rejected");
    assert!(
        matches!(error, DbError::UnsupportedLegacySchema { .. }),
        "unexpected error: {error}"
    );
    assert_eq!(schema_snapshot(pool).await, before);
    let migration_metadata: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name = '_sqlx_migrations'")
            .fetch_one(pool)
            .await
            .expect("metadata absence should inspect");
    let shadows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name LIKE '__hiero_legacy_%'")
            .fetch_one(pool)
            .await
            .expect("shadow absence should inspect");
    assert_eq!((migration_metadata, shadows), (0, 0));
}

async fn seed_every_legacy_table(pool: &SqlitePool) {
    pool.execute(sqlx::raw_sql(
        r#"
        INSERT INTO series VALUES (1, 'legacy', 'Legacy', 'en', 'ru', '2026-07-18T10:11:12.123456+00:00', '2026-07-18 10:11:13');
        INSERT INTO series_language_tags(series_id, language_tag) VALUES (1, 'en');
        INSERT INTO task_sessions VALUES (1, 'legacy', 'en', 'ru', 'translation', '1', '2', 'completed', 4, '2026-07-18T10:11:12Z', '2026-07-18 10:11:13', '2026-07-18T10:11:14+00:00');
        INSERT INTO task_session_language_tags VALUES (1, 'en');
        INSERT INTO task_session_story_scopes VALUES (1, 'chapter:2');
        INSERT INTO task_session_semantic_tags VALUES (1, 'legacy');

        INSERT INTO crystals(id, crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, soft_origin, is_inferred, malformed_penalty, supersedes_crystal_id, status, created_cycle, last_activated_cycle, last_reinforced_cycle, created_at, updated_at)
        VALUES (1, 'rule', 'legacy rule', 'Rule', 'series', 'legacy', 'legacy', 'en', 'ru', '["legacy"]', 0.8, 0.9, 'user_rule', 'correction', NULL, 0, 0.0, NULL, 'active', 1, 2, 3, '2026-07-18 10:11:12.250', '2026-07-18T10:11:13.250000+00:00');
        INSERT INTO crystal_language_tags VALUES (1, 'en');

        INSERT INTO short_term_memories(id, session_id, source_role, kind, text, source_ref, metadata_json, source_credibility, rule_intent, soft_origin, created_at, archived_at)
        VALUES (1, 1, 'translator', 'lesson', 'legacy memory', 'chapter:2', '{}', 'expert', 'note', NULL, '2026-07-18T10:11:12Z', NULL);
        INSERT INTO short_term_memory_language_tags VALUES (1, 'en');
        INSERT INTO short_term_memory_story_scopes VALUES (1, 'chapter:2');
        INSERT INTO short_term_memory_semantic_tags VALUES (1, 'legacy');
        INSERT INTO crystal_sources VALUES (1, 1);
        INSERT INTO crystal_links VALUES (1, 1, 'related');
        INSERT INTO crystal_activations(id, crystal_id, session_id, recall_query, rank, score, reason, cycle_id, created_at)
        VALUES (1, 1, 1, 'legacy', 1, 0.8, 'fixture', 4, '2026-07-18 10:11:12');

        INSERT INTO dream_runs VALUES (1, 4, 'completed', 'fixture', 1, 1, 1, '', '2026-07-18T10:11:12+00:00', '2026-07-18 10:11:13');
        INSERT INTO concepts VALUES (1, 'Legacy concept', 'fixture', 'series', 'legacy', 'solid', 0.9, NULL, '2026-07-18 10:11:12', '2026-07-18T10:11:13+00:00');
        INSERT INTO concept_facets VALUES (1, 1, 'en', 'name', 'Legacy concept', 1, 0.9, 1, NULL, '2026-07-18T10:11:12Z', '2026-07-18T10:11:13Z');
        INSERT INTO concept_facet_language_tags VALUES (1, 'en');
        INSERT INTO concept_facet_story_scopes VALUES (1, 'chapter:2');
        INSERT INTO concept_facet_semantic_tags VALUES (1, 'legacy');
        INSERT INTO concept_semantic_tags VALUES (1, 'legacy', 0.9, '2026-07-18 10:11:12');
        INSERT INTO concept_renames VALUES (1, 1, 'Old', 'Legacy concept', 'fixture', 1, '2026-07-18T10:11:12+00:00');
        INSERT INTO strict_concept_proposals VALUES (1, 1, 'legacy', 'en', 'ru', 'Proposal', 'Proposal', 'Предложение', '[]', '[]', '', 'pending', '2026-07-18 10:11:12', '2026-07-18T10:11:13+00:00');
        INSERT INTO crystal_concepts VALUES (1, 1, 'mentions', 0.9, '2026-07-18T10:11:12Z');
        INSERT INTO crystal_story_scopes VALUES (1, 'chapter:2', 0.9, '2026-07-18 10:11:12');
        INSERT INTO crystal_semantic_tags VALUES (1, 'legacy', 0.9, '2026-07-18T10:11:12+00:00');

        INSERT INTO memory_events VALUES (1, 1, 1, 'recalled', 'system', 'fixture', 0.1, 0.1, 1, 4, '2026-07-18 10:11:12');
        INSERT INTO dream_phase_runs VALUES (1, 1, 'extract', 'default', 'fixture', 'fixture', 'completed', 1, 1, '', '', '2026-07-18T10:11:12Z', '2026-07-18T10:11:13Z');
        INSERT INTO dream_audit_entries VALUES (1, 1, 1, 'completed', 'info', 'fixture', '{}', '2026-07-18 10:11:12');
        INSERT INTO audit_log VALUES (1, 'admin', 'seed', 'fixture', '1', '', '{}', '{}', '2026-07-18T10:11:12+00:00');
        INSERT INTO memory_graph_migration_ledger(source_table, source_id, target_table, target_id) VALUES ('fixture', '1', 'crystals', 1);

        INSERT INTO rag_sources VALUES (1, 'legacy', 'book.md', 'text', 'markdown', 'abc', '{}', '2026-07-18 10:11:12', '2026-07-18T10:11:13+00:00');
        INSERT INTO rag_chunks VALUES (1, 1, 'legacy', 'paragraph', 'chunk', 'Chunk', 'p1', '{}', '2026-07-18T10:11:12Z');
        INSERT INTO rag_chunk_language_tags VALUES (1, 'en');
        INSERT INTO rag_chunk_story_scopes VALUES (1, 'chapter:2');
        INSERT INTO rag_chunk_semantic_tags VALUES (1, 'legacy');

        INSERT INTO strict_terms VALUES (1, 'legacy', 'en', 'ru', 'correction', 'Source', 'Канон', 'approved', 'keep me', '2026-07-18 10:11:12', '2026-07-18T10:11:13+00:00');
        INSERT INTO strict_term_tags VALUES (1, 'term-tag');
        INSERT INTO strict_term_aliases VALUES (1, 1, 'ru', 'Вариант', 'approved', 1);
        INSERT INTO strict_terms_fts(rowid, source_text, canonical_translation, notes) VALUES (1, 'Source', 'Канон', 'keep me');
        "#,
    ))
    .await
    .expect("representative legacy rows should insert");
}

async fn seed_historical_compatibility_rows(pool: &SqlitePool) {
    pool.execute(sqlx::raw_sql(
        r#"
        INSERT INTO series(id, slug, title, default_source_language, default_target_language, created_at, updated_at)
        VALUES (1, 'legacy', 'Legacy', 'en', 'ru', '2026-07-18 10:11:12', '2026-07-18 10:11:13');
        INSERT INTO task_sessions(id, series_slug, source_language, target_language, task_type, volume, chapter, status, cycle_id, created_at, last_activity_at, completed_at)
        VALUES (1, 'legacy', 'en', 'ru', 'translation', '1', '2', 'completed', 4, '2026-07-18 10:11:12', '2026-07-18 10:11:13', '2026-07-18 10:11:14');
        INSERT INTO short_term_memories(id, session_id, source_role, kind, text, source_ref, metadata_json, source_credibility, rule_intent, soft_origin, created_at, archived_at)
        VALUES (1, 1, 'translator', 'lesson', 'legacy memory', 'chapter:2', '{}', 'expert', 'note', 'historical', '2026-07-18 10:11:12', NULL);
        INSERT INTO crystals(id, crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, soft_origin, is_inferred, malformed_penalty, supersedes_crystal_id, status, created_cycle, last_activated_cycle, last_reinforced_cycle, created_at, updated_at)
        VALUES (1, 'observation', 'legacy crystal', 'Crystal', 'series', 'legacy', 'legacy', 'en', 'ru', '[]', 0.8, 0.9, 'user_rule', 'correction', 'historical', 1, 0.25, NULL, 'active', 1, 2, 3, '2026-07-18 10:11:12', '2026-07-18 10:11:13');
        INSERT INTO concepts(id, canonical_name, description, scope_type, scope_key, status, confidence, merged_into_concept_id, created_at, updated_at)
        VALUES (1, 'Legacy concept', 'fixture', 'series', 'legacy', 'vague', 0.9, NULL, '2026-07-18 10:11:12', '2026-07-18 10:11:13');
        INSERT INTO concept_facets(id, concept_id, language, facet_type, value, source_crystal_id, confidence, is_canonical, superseded_at, created_at, updated_at)
        VALUES (1, 1, 'en', 'name', 'Legacy concept', 1, 0.9, 1, NULL, '2026-07-18 10:11:12', '2026-07-18 10:11:13');
        "#,
    ))
    .await
    .expect("historical compatibility rows should insert");
}

async fn assert_historical_compatibility_baselines(crystals: &str, add_late_crystal_columns: bool) {
    let pool = historical_compatibility_pool(crystals, add_late_crystal_columns).await;
    seed_historical_compatibility_rows(&pool).await;
    migrate(&pool)
        .await
        .expect("real Python ALTER variant should baseline");
    migrate(&pool)
        .await
        .expect("historical compatibility baseline should be idempotent");

    let crystal: (String, String, i64, f64) = sqlx::query_as(
        "SELECT source_credibility, soft_origin, is_inferred, malformed_penalty FROM crystals WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("historical crystal should survive");
    assert_eq!(
        crystal,
        ("user_rule".to_owned(), "historical".to_owned(), 1, 0.25)
    );
    let session_activity: String =
        sqlx::query_scalar("SELECT last_activity_at FROM task_sessions WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("session activity should survive");
    assert_eq!(session_activity, "2026-07-18T10:11:13Z");
    let supersedes_delete: String = sqlx::query_scalar(
        "SELECT on_delete FROM pragma_foreign_key_list('crystals') WHERE \"from\" = 'supersedes_crystal_id'",
    )
    .fetch_one(&pool)
    .await
    .expect("final supersedes FK should inspect");
    assert_eq!(supersedes_delete, "SET NULL");
    let concept: (String, String, String, String) = sqlx::query_as(
        "SELECT canonical_name, status, created_at, updated_at FROM concepts WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("historical concept should survive");
    assert_eq!(
        concept,
        (
            "Legacy concept".to_owned(),
            "candidate".to_owned(),
            "2026-07-18T10:11:12Z".to_owned(),
            "2026-07-18T10:11:13Z".to_owned(),
        )
    );
}

#[tokio::test]
async fn oldest_python_base_with_every_real_alter_stage_baselines_losslessly() {
    assert_historical_compatibility_baselines(HISTORICAL_CRYSTALS_OLDEST, true).await;
}

#[tokio::test]
async fn mixed_age_python_schema_with_declared_and_appended_columns_baselines_losslessly() {
    assert_historical_compatibility_baselines(HISTORICAL_CRYSTALS_MID_AGE, false).await;
}

#[tokio::test]
async fn fresh_386_concepts_with_in_place_merged_column_baseline_losslessly() {
    let pool = pre_rebuild_concepts_pool(true).await;
    seed_historical_compatibility_rows(&pool).await;
    migrate(&pool)
        .await
        .expect("fresh 386d1e8 concepts declaration should baseline");
    migrate(&pool)
        .await
        .expect("fresh 386d1e8 baseline should be idempotent");

    let concept: (String, String, String, String) = sqlx::query_as(
        "SELECT canonical_name, status, created_at, updated_at FROM concepts WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("fresh 386d1e8 concept should survive");
    assert_eq!(
        concept,
        (
            "Legacy concept".to_owned(),
            "candidate".to_owned(),
            "2026-07-18T10:11:12Z".to_owned(),
            "2026-07-18T10:11:13Z".to_owned(),
        )
    );
}

#[tokio::test]
async fn upgraded_386_concepts_with_appended_merged_column_baseline_losslessly() {
    let pool = pre_rebuild_concepts_pool(false).await;
    seed_historical_compatibility_rows(&pool).await;
    migrate(&pool)
        .await
        .expect("386d1e8 appended concepts column should baseline");
    migrate(&pool)
        .await
        .expect("appended 386d1e8 baseline should be idempotent");

    let concept: (String, String, String, String) = sqlx::query_as(
        "SELECT canonical_name, status, created_at, updated_at FROM concepts WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("upgraded 386d1e8 concept should survive");
    assert_eq!(
        concept,
        (
            "Legacy concept".to_owned(),
            "candidate".to_owned(),
            "2026-07-18T10:11:12Z".to_owned(),
            "2026-07-18T10:11:13Z".to_owned(),
        )
    );
}

#[tokio::test]
async fn near_python_variant_with_wrong_compatibility_default_is_rejected_before_mutation() {
    let malformed_session_stage = PYTHON_COMPATIBILITY_STAGE_THREE.replace(
        "last_activity_at text not null default ''",
        "last_activity_at text not null default 'unknown'",
    );
    let pool = historical_compatibility_pool_with_session_stage(
        HISTORICAL_CRYSTALS_OLDEST,
        true,
        &malformed_session_stage,
    )
    .await;

    assert_unknown_shape_is_unchanged(&pool).await;
}

async fn concept_facet_value_variant(replacement: &str) -> SqlitePool {
    let schema = include_str!("../../../src/hieronymus/migrations/global.sql").replacen(
        "value text not null,",
        replacement,
        1,
    );
    legacy_pool_from_schema(&schema).await
}

#[tokio::test]
async fn compatibility_column_unique_clause_is_rejected_before_mutation() {
    let pool = concept_facet_value_variant("value text not null unique,").await;
    assert_unknown_shape_is_unchanged(&pool).await;
}

#[tokio::test]
async fn compatibility_column_collation_is_rejected_before_mutation() {
    let pool = concept_facet_value_variant("value text not null collate nocase,").await;
    assert_unknown_shape_is_unchanged(&pool).await;
}

#[tokio::test]
async fn compatibility_column_check_is_rejected_before_mutation() {
    let pool = concept_facet_value_variant("value text not null check(length(value) > 0),").await;
    assert_unknown_shape_is_unchanged(&pool).await;
}

#[tokio::test]
async fn current_python_schema_is_baselined_losslessly_and_idempotently() {
    let pool = legacy_pool().await;
    seed_every_legacy_table(&pool).await;

    migrate(&pool)
        .await
        .expect("supported Python DB should upgrade");
    migrate(&pool)
        .await
        .expect("legacy baseline should be idempotent");

    for table in [
        "series",
        "series_language_tags",
        "task_sessions",
        "task_session_language_tags",
        "task_session_story_scopes",
        "task_session_semantic_tags",
        "short_term_memories",
        "short_term_memory_language_tags",
        "short_term_memory_story_scopes",
        "short_term_memory_semantic_tags",
        "crystals",
        "crystal_language_tags",
        "crystal_sources",
        "crystal_links",
        "crystal_activations",
        "memory_events",
        "dream_runs",
        "concept_proposals",
        "audit_log",
        "concepts",
        "concept_facets",
        "concept_facet_language_tags",
        "concept_facet_story_scopes",
        "concept_facet_semantic_tags",
        "concept_semantic_tags",
        "concept_renames",
        "crystal_concepts",
        "crystal_story_scopes",
        "crystal_semantic_tags",
        "dream_phase_runs",
        "dream_audit_entries",
        "migration_ledger",
        "rag_sources",
        "rag_chunks",
        "rag_chunk_language_tags",
        "rag_chunk_story_scopes",
        "rag_chunk_semantic_tags",
        "strict_terms",
        "strict_term_tags",
        "strict_term_aliases",
    ] {
        let count: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&pool)
                .await
                .expect("preserved table should be readable");
        assert_eq!(count, 1, "row loss in {table}");
    }
    let strict_fts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM strict_terms_fts WHERE strict_terms_fts MATCH 'Source'",
    )
    .fetch_one(&pool)
    .await
    .expect("strict-term FTS should remain queryable");
    assert_eq!(strict_fts, 1);
    let concept_status: String = sqlx::query_scalar("SELECT status FROM concepts WHERE id = 1")
        .fetch_one(&pool)
        .await
        .expect("concept should survive");
    assert_eq!(concept_status, "established");
    let (source_crystal_id, outcome): (Option<i64>, Option<String>) = sqlx::query_as(
        "SELECT short_term_memories.source_crystal_id, crystal_activations.outcome FROM short_term_memories CROSS JOIN crystal_activations",
    )
    .fetch_one(&pool).await.expect("new nullable compatibility columns should read");
    assert_eq!((source_crystal_id, outcome), (None, None));

    let timestamps = sqlx::query_scalar::<_, String>(
        "SELECT created_at FROM series UNION ALL SELECT updated_at FROM crystals UNION ALL SELECT created_at FROM migration_ledger",
    ).fetch_all(&pool).await.expect("timestamps should be readable");
    for value in timestamps {
        DateTime::parse_from_rfc3339(&value).expect("legacy timestamps must normalize to RFC3339");
    }
    let versions: Vec<i64> = sqlx::query_scalar(
        "SELECT version FROM _sqlx_migrations WHERE success = 1 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("migration records should exist");
    assert_eq!(versions, vec![1, 3, 4]);
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .expect("FK pragma should read");
    assert_eq!(fk, 1);
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .expect("FK check should run");
    assert!(violations.is_empty());
}

#[tokio::test]
async fn invalid_legacy_timestamp_rolls_back_and_restores_foreign_keys() {
    let pool = legacy_pool().await;
    seed_every_legacy_table(&pool).await;
    sqlx::query("UPDATE series SET created_at = 'not-a-timestamp' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("failure fixture should update");

    let error = migrate(&pool)
        .await
        .expect_err("invalid timestamp must abort baseline");
    assert!(matches!(error, DbError::LegacyTimestamp { .. }));
    let old_name: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE name = 'strict_concept_proposals'",
    )
    .fetch_one(&pool)
    .await
    .expect("legacy schema should remain");
    let new_name: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name = 'concept_proposals'")
            .fetch_one(&pool)
            .await
            .expect("new schema absence should read");
    assert_eq!((old_name, new_name), (1, 0));
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .expect("FK pragma should read");
    assert_eq!(fk, 1);
}

#[tokio::test]
async fn constraint_failure_after_shadow_rename_rolls_back_every_object_and_row() {
    let pool = legacy_pool().await;
    seed_every_legacy_table(&pool).await;
    sqlx::query("UPDATE concepts SET status = 'invented' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("failure fixture should update");

    migrate(&pool)
        .await
        .expect_err("final-schema CHECK failure must roll back baseline");
    let legacy: i64 = sqlx::query_scalar("SELECT count(*) FROM strict_concept_proposals")
        .fetch_one(&pool)
        .await
        .expect("legacy proposal should remain");
    let strict: i64 = sqlx::query_scalar("SELECT count(*) FROM strict_terms")
        .fetch_one(&pool)
        .await
        .expect("strict term should remain");
    let shadows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name LIKE '__hiero_legacy_%'")
            .fetch_one(&pool)
            .await
            .expect("shadow absence should read");
    assert_eq!((legacy, strict, shadows), (1, 1, 0));
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .expect("FK pragma should read");
    assert_eq!(fk, 1);
}

#[tokio::test]
async fn unknown_partial_schema_returns_a_typed_error_without_mutation() {
    let options = SqliteConnectOptions::from_str("sqlite::memory:").expect("URL should parse");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("pool should connect");
    sqlx::query("CREATE TABLE series (id INTEGER PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("partial fixture should install");

    let error = migrate(&pool)
        .await
        .expect_err("partial schema must be rejected");
    assert!(matches!(error, DbError::UnsupportedLegacySchema { .. }));
    let sql: String = sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE name = 'series'")
        .fetch_one(&pool)
        .await
        .expect("partial table should remain");
    assert_eq!(sql, "CREATE TABLE series (id INTEGER PRIMARY KEY)");
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .expect("FK pragma should read");
    assert_eq!(fk, 1);
}

#[tokio::test]
async fn synthetic_missing_compatibility_combination_is_rejected_before_mutation() {
    let pool = legacy_pool().await;
    pool.execute(sqlx::raw_sql(
        r#"
        ALTER TABLE task_sessions DROP COLUMN last_activity_at;
        ALTER TABLE short_term_memories DROP COLUMN source_credibility;
        ALTER TABLE short_term_memories DROP COLUMN rule_intent;
        ALTER TABLE short_term_memories DROP COLUMN soft_origin;
        ALTER TABLE crystals DROP COLUMN source_credibility;
        ALTER TABLE crystals DROP COLUMN rule_intent;
        ALTER TABLE crystals DROP COLUMN malformed_penalty;
        ALTER TABLE crystals DROP COLUMN supersedes_crystal_id;
        ALTER TABLE crystals DROP COLUMN soft_origin;
        ALTER TABLE crystals DROP COLUMN is_inferred;
        ALTER TABLE concepts DROP COLUMN merged_into_concept_id;
        ALTER TABLE concept_facets DROP COLUMN is_canonical;
        ALTER TABLE concept_facets DROP COLUMN superseded_at;
        "#,
    ))
    .await
    .expect("known historical columns should be removable for the fixture");

    assert_unknown_shape_is_unchanged(&pool).await;
}

#[tokio::test]
async fn reserved_shadow_collision_is_rejected_before_any_rename() {
    let pool = legacy_pool().await;
    sqlx::query("CREATE TABLE __hiero_legacy_series (id INTEGER PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("collision fixture should install");

    let error = migrate(&pool)
        .await
        .expect_err("shadow collision must be rejected");
    assert!(matches!(error, DbError::UnsupportedLegacySchema { .. }));
    let original: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name = 'series'")
            .fetch_one(&pool)
            .await
            .expect("original schema should remain");
    assert_eq!(original, 1);
}

#[tokio::test]
async fn full_legacy_family_missing_noncompat_column_is_rejected_before_mutation() {
    let pool = legacy_pool().await;
    sqlx::query("ALTER TABLE audit_log DROP COLUMN note")
        .execute(&pool)
        .await
        .expect("missing-column fixture should install");
    assert_unknown_shape_is_unchanged(&pool).await;
}

#[tokio::test]
async fn full_legacy_family_with_extra_column_is_rejected_before_mutation() {
    let pool = legacy_pool().await;
    sqlx::query("ALTER TABLE audit_log ADD COLUMN intruder TEXT")
        .execute(&pool)
        .await
        .expect("extra-column fixture should install");
    assert_unknown_shape_is_unchanged(&pool).await;
}

#[tokio::test]
async fn full_legacy_family_with_wrong_column_type_is_rejected_before_mutation() {
    let pool = legacy_pool().await;
    pool.execute(sqlx::raw_sql(
        r#"
        DROP TABLE audit_log;
        CREATE TABLE audit_log (
          id integer primary key,
          actor integer not null default 'admin',
          action text not null,
          entity_type text not null,
          entity_id text not null,
          note text not null default '',
          before_json text not null default '{}',
          after_json text not null default '{}',
          created_at text not null
        );
        "#,
    ))
    .await
    .expect("wrong-type fixture should install");
    assert_unknown_shape_is_unchanged(&pool).await;
}

#[tokio::test]
async fn full_legacy_family_with_altered_constraint_is_rejected_before_mutation() {
    let pool = legacy_pool().await;
    pool.execute(sqlx::raw_sql(
        r#"
        DROP TABLE audit_log;
        CREATE TABLE audit_log (
          id integer primary key,
          actor text not null default 'admin' check(actor <> ''),
          action text not null,
          entity_type text not null,
          entity_id text not null,
          note text not null default '',
          before_json text not null default '{}',
          after_json text not null default '{}',
          created_at text not null
        );
        "#,
    ))
    .await
    .expect("altered-constraint fixture should install");
    assert_unknown_shape_is_unchanged(&pool).await;
}

#[tokio::test]
async fn full_legacy_family_with_altered_foreign_key_is_rejected_before_mutation() {
    let pool = legacy_pool().await;
    pool.execute(sqlx::raw_sql(
        r#"
        DROP TABLE crystal_sources;
        CREATE TABLE crystal_sources (
          crystal_id integer not null references crystals(id) on delete cascade,
          short_term_memory_id integer not null references short_term_memories(id) on delete set null,
          primary key(crystal_id, short_term_memory_id)
        );
        "#,
    ))
    .await
    .expect("altered-FK fixture should install");
    assert_unknown_shape_is_unchanged(&pool).await;
}
