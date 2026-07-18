use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use sqlx::{
    AssertSqlSafe, Connection, Executor, Row, SqliteConnection, SqlitePool,
    migrate::{Migrate, Migrator},
};

use super::DbError;

const SQLX_TABLE: &str = "_sqlx_migrations";
const SHADOW_PREFIX: &str = "__hiero_legacy_";

const LEGACY_TABLE_MAP: &[(&str, &str)] = &[
    ("series", "series"),
    ("series_language_tags", "series_language_tags"),
    ("task_sessions", "task_sessions"),
    ("task_session_language_tags", "task_session_language_tags"),
    ("task_session_story_scopes", "task_session_story_scopes"),
    ("task_session_semantic_tags", "task_session_semantic_tags"),
    ("short_term_memories", "short_term_memories"),
    (
        "short_term_memory_language_tags",
        "short_term_memory_language_tags",
    ),
    (
        "short_term_memory_story_scopes",
        "short_term_memory_story_scopes",
    ),
    (
        "short_term_memory_semantic_tags",
        "short_term_memory_semantic_tags",
    ),
    ("crystals", "crystals"),
    ("crystal_language_tags", "crystal_language_tags"),
    ("crystal_sources", "crystal_sources"),
    ("crystal_links", "crystal_links"),
    ("crystal_activations", "crystal_activations"),
    ("memory_events", "memory_events"),
    ("dream_runs", "dream_runs"),
    ("strict_concept_proposals", "concept_proposals"),
    ("audit_log", "audit_log"),
    ("concepts", "concepts"),
    ("concept_facets", "concept_facets"),
    ("concept_facet_language_tags", "concept_facet_language_tags"),
    ("concept_facet_story_scopes", "concept_facet_story_scopes"),
    ("concept_facet_semantic_tags", "concept_facet_semantic_tags"),
    ("concept_semantic_tags", "concept_semantic_tags"),
    ("concept_renames", "concept_renames"),
    ("crystal_concepts", "crystal_concepts"),
    ("crystal_story_scopes", "crystal_story_scopes"),
    ("crystal_semantic_tags", "crystal_semantic_tags"),
    ("dream_phase_runs", "dream_phase_runs"),
    ("dream_audit_entries", "dream_audit_entries"),
    ("memory_graph_migration_ledger", "migration_ledger"),
    ("rag_sources", "rag_sources"),
    ("rag_chunks", "rag_chunks"),
    ("rag_chunk_language_tags", "rag_chunk_language_tags"),
    ("rag_chunk_story_scopes", "rag_chunk_story_scopes"),
    ("rag_chunk_semantic_tags", "rag_chunk_semantic_tags"),
];

const STRICT_TABLES: &[&str] = &["strict_terms", "strict_term_tags", "strict_term_aliases"];

const EXPLICIT_COPY_TABLES: &[&str] = &[
    "task_sessions",
    "short_term_memories",
    "crystals",
    "crystal_activations",
    "concepts",
    "concept_facets",
];

const COMPATIBILITY_COLUMNS: &[(&str, &str, &str)] = &[
    (
        "task_sessions",
        "last_activity_at",
        "TEXT NOT NULL DEFAULT ''",
    ),
    ("short_term_memories", "source_credibility", "TEXT"),
    ("short_term_memories", "rule_intent", "TEXT"),
    ("short_term_memories", "soft_origin", "TEXT"),
    (
        "crystals",
        "source_credibility",
        "TEXT NOT NULL DEFAULT 'observation'",
    ),
    ("crystals", "rule_intent", "TEXT NOT NULL DEFAULT ''"),
    ("crystals", "malformed_penalty", "REAL NOT NULL DEFAULT 0"),
    ("crystals", "supersedes_crystal_id", "INTEGER"),
    ("crystals", "soft_origin", "TEXT"),
    ("crystals", "is_inferred", "INTEGER NOT NULL DEFAULT 0"),
    ("concepts", "merged_into_concept_id", "INTEGER"),
    (
        "concept_facets",
        "is_canonical",
        "INTEGER NOT NULL DEFAULT 0",
    ),
    ("concept_facets", "superseded_at", "TEXT"),
];

const TIMESTAMP_COLUMNS: &[(&str, &[&str])] = &[
    ("series", &["created_at", "updated_at"]),
    ("series_language_tags", &["created_at"]),
    (
        "task_sessions",
        &["created_at", "last_activity_at", "completed_at"],
    ),
    ("short_term_memories", &["created_at", "archived_at"]),
    ("crystals", &["created_at", "updated_at"]),
    ("crystal_activations", &["created_at"]),
    ("memory_events", &["created_at"]),
    ("dream_runs", &["created_at", "completed_at"]),
    ("strict_concept_proposals", &["created_at", "updated_at"]),
    ("audit_log", &["created_at"]),
    ("concepts", &["created_at", "updated_at"]),
    (
        "concept_facets",
        &["superseded_at", "created_at", "updated_at"],
    ),
    ("concept_semantic_tags", &["created_at"]),
    ("concept_renames", &["created_at"]),
    ("crystal_concepts", &["created_at"]),
    ("crystal_story_scopes", &["created_at"]),
    ("crystal_semantic_tags", &["created_at"]),
    ("dream_phase_runs", &["created_at", "completed_at"]),
    ("dream_audit_entries", &["created_at"]),
    ("memory_graph_migration_ledger", &["created_at"]),
    ("rag_sources", &["created_at", "updated_at"]),
    ("rag_chunks", &["created_at"]),
    ("strict_terms", &["created_at", "updated_at"]),
];

pub(super) async fn prepare(pool: &SqlitePool, migrator: &Migrator) -> Result<(), DbError> {
    let mut connection = pool
        .acquire()
        .await
        .map_err(|source| DbError::LegacyInspection { source })?;
    if table_exists(&mut connection, SQLX_TABLE).await? {
        validate_migration_metadata(&mut connection, migrator).await?;
        return Ok(());
    }
    if user_table_count(&mut connection).await? == 0 {
        return Ok(());
    }

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *connection)
        .await
        .map_err(|source| DbError::LegacyUpgrade { source })?;

    let result = baseline_locked(&mut connection, migrator).await;
    let restore = restore_foreign_keys(&mut connection).await;
    if let Err(reason) = restore {
        connection.close().await.ok();
        return Err(DbError::ForeignKeyRestore { reason });
    }
    result
}

async fn baseline_locked(
    connection: &mut SqliteConnection,
    migrator: &Migrator,
) -> Result<(), DbError> {
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map_err(|source| DbError::LegacyUpgrade { source })?;
    let result = baseline_transaction(connection, migrator).await;
    match result {
        Ok(()) => sqlx::query("COMMIT")
            .execute(&mut *connection)
            .await
            .map(|_| ())
            .map_err(|source| DbError::LegacyUpgrade { source }),
        Err(error) => {
            let rollback = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            match rollback {
                Ok(_) => Err(error),
                Err(source) => Err(DbError::LegacyUpgrade { source }),
            }
        }
    }
}

async fn baseline_transaction(
    connection: &mut SqliteConnection,
    migrator: &Migrator,
) -> Result<(), DbError> {
    if table_exists(connection, SQLX_TABLE).await? {
        validate_migration_metadata(connection, migrator).await?;
        return Ok(());
    }
    verify_supported_legacy_schema(connection).await?;
    add_compatibility_columns(connection).await?;
    normalize_timestamps(connection).await?;
    remove_rebuildable_indexes(connection).await?;
    rename_to_shadows(connection).await?;

    connection
        .execute(sqlx::raw_sql(include_str!(
            "../../../../migrations/0001_initial_schema.sql"
        )))
        .await
        .map_err(|source| DbError::LegacyUpgrade { source })?;
    connection
        .execute(sqlx::raw_sql(STRICT_SCHEMA))
        .await
        .map_err(|source| DbError::LegacyUpgrade { source })?;
    copy_rows(connection).await?;
    verify_counts(connection).await?;
    drop_shadows(connection).await?;
    verify_foreign_keys(connection).await?;
    record_baseline(connection, migrator).await?;
    Ok(())
}

async fn validate_migration_metadata(
    connection: &mut SqliteConnection,
    migrator: &Migrator,
) -> Result<(), DbError> {
    let columns = sqlx::query("PRAGMA table_info('_sqlx_migrations')")
        .fetch_all(&mut *connection)
        .await
        .map_err(|source| DbError::LegacyInspection { source })?
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get::<i64, _>("cid")?,
                row.try_get::<String, _>("name")?,
                row.try_get::<String, _>("type")?,
                row.try_get::<i64, _>("notnull")?,
                row.try_get::<Option<String>, _>("dflt_value")?,
                row.try_get::<i64, _>("pk")?,
            ))
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(|source| DbError::LegacyInspection { source })?;
    let expected_columns = vec![
        (0, "version".to_owned(), "BIGINT".to_owned(), 0, None, 1),
        (1, "description".to_owned(), "TEXT".to_owned(), 1, None, 0),
        (
            2,
            "installed_on".to_owned(),
            "TIMESTAMP".to_owned(),
            1,
            Some("CURRENT_TIMESTAMP".to_owned()),
            0,
        ),
        (3, "success".to_owned(), "BOOLEAN".to_owned(), 1, None, 0),
        (4, "checksum".to_owned(), "BLOB".to_owned(), 1, None, 0),
        (
            5,
            "execution_time".to_owned(),
            "BIGINT".to_owned(),
            1,
            None,
            0,
        ),
    ];
    if columns != expected_columns {
        return Err(DbError::InvalidMigrationMetadata {
            reason: format!(
                "`_sqlx_migrations` does not have the SQLx 0.9 SQLite column contract; expected {expected_columns:?}, found {columns:?}"
            ),
        });
    }

    let indexes = sqlx::query("PRAGMA index_list('_sqlx_migrations')")
        .fetch_all(&mut *connection)
        .await
        .map_err(|source| DbError::LegacyInspection { source })?;
    if indexes.len() != 1
        || indexes[0]
            .try_get::<i64, _>("unique")
            .map_err(|source| DbError::LegacyInspection { source })?
            != 1
        || indexes[0]
            .try_get::<String, _>("origin")
            .map_err(|source| DbError::LegacyInspection { source })?
            != "pk"
        || indexes[0]
            .try_get::<i64, _>("partial")
            .map_err(|source| DbError::LegacyInspection { source })?
            != 0
    {
        return Err(DbError::InvalidMigrationMetadata {
            reason: "`_sqlx_migrations` must have only SQLx's version primary-key index".to_owned(),
        });
    }

    let stored = sqlx::query(
        "SELECT version, description, success, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| DbError::InvalidMigrationMetadata {
        reason: format!("cannot read migration history: {source}"),
    })?;
    let embedded = migrator.iter().collect::<Vec<_>>();
    if stored.len() > embedded.len() {
        return Err(DbError::InvalidMigrationMetadata {
            reason: format!(
                "history has {} row(s), but this binary embeds only {} migration(s); remove no rows until the database is inspected with the newer binary",
                stored.len(),
                embedded.len()
            ),
        });
    }
    for (position, row) in stored.iter().enumerate() {
        let version = row.try_get::<i64, _>("version").map_err(|source| {
            DbError::InvalidMigrationMetadata {
                reason: format!(
                    "cannot decode history row {} version: {source}",
                    position + 1
                ),
            }
        })?;
        let description = row.try_get::<String, _>("description").map_err(|source| {
            DbError::InvalidMigrationMetadata {
                reason: format!("cannot decode migration {version} description as TEXT: {source}"),
            }
        })?;
        let success = row.try_get::<i64, _>("success").map_err(|source| {
            DbError::InvalidMigrationMetadata {
                reason: format!("cannot decode migration {version} success flag: {source}"),
            }
        })?;
        let checksum = row.try_get::<Vec<u8>, _>("checksum").map_err(|source| {
            DbError::InvalidMigrationMetadata {
                reason: format!("cannot decode migration {version} checksum: {source}"),
            }
        })?;
        let expected = embedded[position];
        if version != expected.version {
            let known = embedded
                .iter()
                .any(|migration| migration.version == version);
            let reason = if known {
                format!(
                    "migration history is not an embedded prefix: expected version {} at position {}, found {version}; restore the missing migration record",
                    expected.version,
                    position + 1
                )
            } else {
                format!(
                    "migration version {version} is unknown to this binary; use the binary that created it or restore a compatible database backup"
                )
            };
            return Err(DbError::InvalidMigrationMetadata { reason });
        }
        if success != 1 {
            return Err(DbError::InvalidMigrationMetadata {
                reason: format!(
                    "migration {version} is dirty (`success` = {success}); inspect the partially applied migration before repairing its metadata"
                ),
            });
        }
        if description != expected.description.as_ref() {
            return Err(DbError::InvalidMigrationMetadata {
                reason: format!(
                    "migration {version} description mismatch: expected `{}`, found `{description}`",
                    expected.description
                ),
            });
        }
        if checksum != expected.checksum.as_ref() {
            return Err(DbError::InvalidMigrationMetadata {
                reason: format!(
                    "migration {version} checksum does not match the embedded migration; restore the matching migration or database"
                ),
            });
        }
    }
    Ok(())
}

async fn verify_supported_legacy_schema(connection: &mut SqliteConnection) -> Result<(), DbError> {
    let mut missing = Vec::new();
    for (source, _) in LEGACY_TABLE_MAP {
        if !table_exists(connection, source).await? {
            missing.push(*source);
        }
    }
    for table in STRICT_TABLES {
        if !table_exists(connection, table).await? {
            missing.push(*table);
        }
    }
    if !table_exists(connection, "strict_terms_fts").await? {
        missing.push("strict_terms_fts");
    }
    if !missing.is_empty() {
        return Err(DbError::UnsupportedLegacySchema {
            reason: format!("missing required Python tables: {}", missing.join(", ")),
        });
    }

    let shadows: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND name LIKE '__hiero_legacy_%' ORDER BY name",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| DbError::LegacyInspection { source })?;
    if !shadows.is_empty() {
        return Err(DbError::UnsupportedLegacySchema {
            reason: format!(
                "reserved shadow tables already exist: {}",
                shadows.join(", ")
            ),
        });
    }
    let table_names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| DbError::LegacyInspection { source })?;
    let unknown = table_names
        .into_iter()
        .filter(|name| !is_known_legacy_table(name))
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(DbError::UnsupportedLegacySchema {
            reason: format!("unknown Python-era tables: {}", unknown.join(", ")),
        });
    }
    verify_legacy_object_manifest(connection).await?;
    Ok(())
}

async fn verify_legacy_object_manifest(connection: &mut SqliteConnection) -> Result<(), DbError> {
    let mut reference = SqliteConnection::connect("sqlite::memory:")
        .await
        .map_err(|source| DbError::LegacyInspection { source })?;
    reference
        .execute(sqlx::raw_sql(include_str!(
            "../../../../src/hieronymus/migrations/global.sql"
        )))
        .await
        .map_err(|source| DbError::LegacyInspection { source })?;

    for (table, column, _) in COMPATIBILITY_COLUMNS {
        if !column_exists(connection, table, column).await? {
            sqlx::query(AssertSqlSafe(format!(
                "ALTER TABLE {table} DROP COLUMN {column}"
            )))
            .execute(&mut reference)
            .await
            .map_err(|source| DbError::UnsupportedLegacySchema {
                reason: format!(
                    "cannot construct the known compatibility manifest without {table}.{column}: {source}"
                ),
            })?;
        }
    }

    let expected = object_manifest(&mut reference).await?;
    let actual = object_manifest(connection).await?;
    if actual != expected {
        let first_difference = actual
            .iter()
            .zip(expected.iter())
            .position(|(actual, expected)| actual != expected)
            .unwrap_or_else(|| actual.len().min(expected.len()));
        return Err(DbError::UnsupportedLegacySchema {
            reason: format!(
                "Python schema object manifest differs at position {first_difference}: expected {:?}, found {:?}; only documented compatibility columns may be absent",
                expected.get(first_difference),
                actual.get(first_difference)
            ),
        });
    }
    Ok(())
}

async fn object_manifest(
    connection: &mut SqliteConnection,
) -> Result<Vec<(String, String, String, String)>, DbError> {
    sqlx::query(
        "SELECT type, name, tbl_name, coalesce(sql, '') AS sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| DbError::LegacyInspection { source })?
    .into_iter()
    .map(|row| {
        let sql = row
            .try_get::<String, _>("sql")
            .map_err(|source| DbError::LegacyInspection { source })?;
        Ok((
            row.try_get("type")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("name")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("tbl_name")
                .map_err(|source| DbError::LegacyInspection { source })?,
            normalize_schema_sql(&sql),
        ))
    })
    .collect()
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn is_known_legacy_table(name: &str) -> bool {
    LEGACY_TABLE_MAP.iter().any(|(source, _)| *source == name)
        || STRICT_TABLES.contains(&name)
        || [
            "short_term_memories_fts",
            "crystals_fts",
            "strict_terms_fts",
            "concepts_fts",
            "concept_facet_fts",
            "rag_chunks_fts",
        ]
        .iter()
        .any(|prefix| {
            name == *prefix
                || ["data", "idx", "content", "docsize", "config"]
                    .iter()
                    .any(|suffix| name == format!("{prefix}_{suffix}"))
        })
}

async fn add_compatibility_columns(connection: &mut SqliteConnection) -> Result<(), DbError> {
    for (table, column, definition) in COMPATIBILITY_COLUMNS {
        if !column_exists(connection, table, column).await? {
            execute_dynamic(
                connection,
                format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            )
            .await?;
        }
    }
    sqlx::query(
        "UPDATE task_sessions SET last_activity_at = created_at WHERE last_activity_at = ''",
    )
    .execute(&mut *connection)
    .await
    .map_err(|source| DbError::LegacyUpgrade { source })?;
    Ok(())
}

async fn normalize_timestamps(connection: &mut SqliteConnection) -> Result<(), DbError> {
    for (table, columns) in TIMESTAMP_COLUMNS {
        for column in *columns {
            let rows = sqlx::query(AssertSqlSafe(format!(
                "SELECT rowid AS legacy_rowid, {column} AS value FROM {table} WHERE {column} IS NOT NULL"
            )))
            .fetch_all(&mut *connection)
            .await
            .map_err(|source| DbError::LegacyUpgrade { source })?;
            for row in rows {
                let rowid: i64 = row
                    .try_get("legacy_rowid")
                    .map_err(|source| DbError::LegacyUpgrade { source })?;
                let value: String = row
                    .try_get("value")
                    .map_err(|source| DbError::LegacyUpgrade { source })?;
                let normalized =
                    normalize_timestamp(&value).ok_or_else(|| DbError::LegacyTimestamp {
                        table: (*table).to_owned(),
                        column: (*column).to_owned(),
                        rowid,
                        value: value.clone(),
                    })?;
                sqlx::query(AssertSqlSafe(format!(
                    "UPDATE {table} SET {column} = ? WHERE rowid = ?"
                )))
                .bind(normalized)
                .bind(rowid)
                .execute(&mut *connection)
                .await
                .map_err(|source| DbError::LegacyUpgrade { source })?;
            }
        }
    }
    Ok(())
}

fn normalize_timestamp(value: &str) -> Option<String> {
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        return Some(
            value
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::AutoSi, true),
        );
    }
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(value, format) {
            return Some(value.and_utc().to_rfc3339_opts(SecondsFormat::AutoSi, true));
        }
    }
    None
}

async fn remove_rebuildable_indexes(connection: &mut SqliteConnection) -> Result<(), DbError> {
    connection
        .execute(sqlx::raw_sql(
            r#"
            DROP TRIGGER IF EXISTS concepts_ai;
            DROP TRIGGER IF EXISTS concepts_ad;
            DROP TRIGGER IF EXISTS concepts_au;
            DROP TRIGGER IF EXISTS concept_facets_ai;
            DROP TRIGGER IF EXISTS concept_facets_ad;
            DROP TRIGGER IF EXISTS concept_facets_au;
            DROP TRIGGER IF EXISTS rag_chunks_ai;
            DROP TRIGGER IF EXISTS rag_chunks_ad;
            DROP TRIGGER IF EXISTS rag_chunks_au;
            DROP TABLE IF EXISTS short_term_memories_fts;
            DROP TABLE IF EXISTS crystals_fts;
            DROP TABLE IF EXISTS concepts_fts;
            DROP TABLE IF EXISTS concept_facet_fts;
            DROP TABLE IF EXISTS rag_chunks_fts;
            DROP INDEX IF EXISTS rag_chunks_source_id_idx;
            DROP INDEX IF EXISTS rag_chunks_series_slug_idx;
            "#,
        ))
        .await
        .map_err(|source| DbError::LegacyUpgrade { source })?;
    Ok(())
}

async fn rename_to_shadows(connection: &mut SqliteConnection) -> Result<(), DbError> {
    execute_dynamic(
        connection,
        "ALTER TABLE strict_terms_fts RENAME TO __hiero_legacy_strict_terms_fts".to_owned(),
    )
    .await?;
    for (source, _) in LEGACY_TABLE_MAP {
        execute_dynamic(
            connection,
            format!("ALTER TABLE {source} RENAME TO {SHADOW_PREFIX}{source}"),
        )
        .await?;
    }
    for table in STRICT_TABLES {
        execute_dynamic(
            connection,
            format!("ALTER TABLE {table} RENAME TO {SHADOW_PREFIX}{table}"),
        )
        .await?;
    }
    Ok(())
}

async fn copy_rows(connection: &mut SqliteConnection) -> Result<(), DbError> {
    for (source, target) in LEGACY_TABLE_MAP {
        if EXPLICIT_COPY_TABLES.contains(source) {
            continue;
        }
        execute_dynamic(
            connection,
            format!("INSERT INTO {target} SELECT * FROM {SHADOW_PREFIX}{source}"),
        )
        .await?;
    }
    connection
        .execute(sqlx::raw_sql(EXPLICIT_COPY_SQL))
        .await
        .map_err(|source| DbError::LegacyUpgrade { source })?;
    connection
        .execute(sqlx::raw_sql(
            r#"
            INSERT INTO strict_terms SELECT * FROM __hiero_legacy_strict_terms;
            INSERT INTO strict_term_tags SELECT * FROM __hiero_legacy_strict_term_tags;
            INSERT INTO strict_term_aliases SELECT * FROM __hiero_legacy_strict_term_aliases;
            INSERT INTO strict_terms_fts(strict_terms_fts) VALUES ('rebuild');
            "#,
        ))
        .await
        .map_err(|source| DbError::LegacyUpgrade { source })?;
    Ok(())
}

async fn verify_counts(connection: &mut SqliteConnection) -> Result<(), DbError> {
    for (source, target) in LEGACY_TABLE_MAP {
        compare_counts(connection, source, target).await?;
    }
    for table in STRICT_TABLES {
        compare_counts(connection, table, table).await?;
    }
    Ok(())
}

async fn compare_counts(
    connection: &mut SqliteConnection,
    source: &str,
    target: &str,
) -> Result<(), DbError> {
    let (legacy, current): (i64, i64) = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT (SELECT count(*) FROM {SHADOW_PREFIX}{source}), (SELECT count(*) FROM {target})"
    )))
    .fetch_one(&mut *connection)
    .await
    .map_err(|source| DbError::LegacyUpgrade { source })?;
    if legacy != current {
        return Err(DbError::UnsupportedLegacySchema {
            reason: format!("row-count mismatch for {source} -> {target}: {legacy} != {current}"),
        });
    }
    Ok(())
}

async fn drop_shadows(connection: &mut SqliteConnection) -> Result<(), DbError> {
    execute_dynamic(
        connection,
        "DROP TABLE __hiero_legacy_strict_terms_fts".to_owned(),
    )
    .await?;
    for (source, _) in LEGACY_TABLE_MAP.iter().rev() {
        execute_dynamic(connection, format!("DROP TABLE {SHADOW_PREFIX}{source}")).await?;
    }
    for table in STRICT_TABLES.iter().rev() {
        execute_dynamic(connection, format!("DROP TABLE {SHADOW_PREFIX}{table}")).await?;
    }
    Ok(())
}

async fn verify_foreign_keys(connection: &mut SqliteConnection) -> Result<(), DbError> {
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *connection)
        .await
        .map_err(|source| DbError::LegacyUpgrade { source })?;
    if !violations.is_empty() {
        return Err(DbError::UnsupportedLegacySchema {
            reason: format!(
                "foreign_key_check reported {} violation(s)",
                violations.len()
            ),
        });
    }
    Ok(())
}

async fn record_baseline(
    connection: &mut SqliteConnection,
    migrator: &Migrator,
) -> Result<(), DbError> {
    let migration = migrator
        .iter()
        .find(|migration| migration.version == 1)
        .ok_or_else(|| DbError::UnsupportedLegacySchema {
            reason: "embedded migration 0001 is missing".to_owned(),
        })?;
    connection
        .ensure_migrations_table(SQLX_TABLE)
        .await
        .map_err(|error| DbError::UnsupportedLegacySchema {
            reason: format!("cannot create SQLx migration metadata: {error}"),
        })?;
    sqlx::query(
        "INSERT INTO _sqlx_migrations(version, description, success, checksum, execution_time) VALUES (?, ?, 1, ?, 0)",
    )
    .bind(migration.version)
    .bind(migration.description.as_ref())
    .bind(migration.checksum.as_ref())
    .execute(&mut *connection)
    .await
    .map_err(|source| DbError::LegacyUpgrade { source })?;
    Ok(())
}

async fn restore_foreign_keys(connection: &mut SqliteConnection) -> Result<(), String> {
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;
    let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;
    if enabled != 1 {
        return Err(format!("PRAGMA foreign_keys returned {enabled}"));
    }
    Ok(())
}

async fn table_exists(connection: &mut SqliteConnection, table: &str) -> Result<bool, DbError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_one(&mut *connection)
    .await
    .map(|count| count == 1)
    .map_err(|source| DbError::LegacyInspection { source })
}

async fn column_exists(
    connection: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<bool, DbError> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pragma_table_info(?) WHERE name = ?")
        .bind(table)
        .bind(column)
        .fetch_one(&mut *connection)
        .await
        .map(|count| count == 1)
        .map_err(|source| DbError::LegacyInspection { source })
}

async fn user_table_count(connection: &mut SqliteConnection) -> Result<i64, DbError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|source| DbError::LegacyInspection { source })
}

async fn execute_dynamic(
    connection: &mut SqliteConnection,
    statement: String,
) -> Result<(), DbError> {
    sqlx::query(AssertSqlSafe(statement))
        .execute(&mut *connection)
        .await
        .map(|_| ())
        .map_err(|source| DbError::LegacyUpgrade { source })
}

const STRICT_SCHEMA: &str = r#"
CREATE TABLE strict_terms (
  id INTEGER PRIMARY KEY,
  series_slug TEXT NOT NULL REFERENCES series(slug),
  source_language TEXT NOT NULL,
  target_language TEXT NOT NULL,
  category TEXT NOT NULL,
  source_text TEXT NOT NULL,
  canonical_translation TEXT NOT NULL,
  status TEXT NOT NULL,
  notes TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;
CREATE TABLE strict_term_tags (
  term_id INTEGER NOT NULL REFERENCES strict_terms(id) ON DELETE CASCADE,
  tag TEXT NOT NULL,
  PRIMARY KEY(term_id, tag)
) STRICT;
CREATE TABLE strict_term_aliases (
  id INTEGER PRIMARY KEY,
  term_id INTEGER NOT NULL REFERENCES strict_terms(id) ON DELETE CASCADE,
  language TEXT NOT NULL,
  text TEXT NOT NULL,
  kind TEXT NOT NULL,
  case_sensitive INTEGER NOT NULL DEFAULT 1
) STRICT;
CREATE VIRTUAL TABLE strict_terms_fts USING fts5(
  source_text,
  canonical_translation,
  notes,
  content='strict_terms',
  content_rowid='id'
);
"#;

const EXPLICIT_COPY_SQL: &str = r#"
INSERT INTO task_sessions
SELECT id, series_slug, source_language, target_language, task_type, volume, chapter,
       status, cycle_id, created_at, last_activity_at, completed_at
FROM __hiero_legacy_task_sessions;

INSERT INTO short_term_memories
SELECT id, session_id, source_role, kind, text, source_ref, metadata_json,
       source_credibility, rule_intent, soft_origin, NULL, created_at, archived_at
FROM __hiero_legacy_short_term_memories;

INSERT INTO crystals
SELECT id, crystal_type, text, title, scope_type, scope_key, series_slug,
       source_language, target_language, tags_json, strength, confidence,
       source_credibility, rule_intent, soft_origin, is_inferred,
       malformed_penalty, supersedes_crystal_id, status, created_cycle,
       last_activated_cycle, last_reinforced_cycle, created_at, updated_at
FROM __hiero_legacy_crystals;

INSERT INTO crystal_activations
SELECT id, crystal_id, session_id, recall_query, rank, score, reason, NULL, cycle_id, created_at
FROM __hiero_legacy_crystal_activations;

INSERT INTO concepts
SELECT id, canonical_name, description, scope_type, scope_key,
       CASE status WHEN 'vague' THEN 'candidate' WHEN 'solid' THEN 'established' ELSE status END,
       confidence, merged_into_concept_id, created_at, updated_at
FROM __hiero_legacy_concepts;

INSERT INTO concept_facets
SELECT id, concept_id, language, facet_type, value, source_crystal_id,
       confidence, is_canonical, superseded_at, created_at, updated_at
FROM __hiero_legacy_concept_facets;
"#;
