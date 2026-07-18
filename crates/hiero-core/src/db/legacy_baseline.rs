use std::collections::HashSet;

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
    (
        "crystals",
        "supersedes_crystal_id",
        "INTEGER REFERENCES crystals(id)",
    ),
    ("crystals", "soft_origin", "TEXT"),
    ("crystals", "is_inferred", "INTEGER NOT NULL DEFAULT 0"),
    (
        "concepts",
        "merged_into_concept_id",
        "INTEGER REFERENCES concepts(id)",
    ),
    (
        "concept_facets",
        "is_canonical",
        "INTEGER NOT NULL DEFAULT 0",
    ),
    ("concept_facets", "superseded_at", "TEXT"),
];

const COMPATIBILITY_TABLES: &[&str] = &[
    "task_sessions",
    "short_term_memories",
    "crystals",
    "concepts",
    "concept_facets",
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
    let columns = sqlx::query("PRAGMA table_xinfo('_sqlx_migrations')")
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
                row.try_get::<i64, _>("hidden")?,
            ))
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(|source| DbError::LegacyInspection { source })?;
    let expected_columns = vec![
        (0, "version".to_owned(), "BIGINT".to_owned(), 0, None, 1, 0),
        (
            1,
            "description".to_owned(),
            "TEXT".to_owned(),
            1,
            None,
            0,
            0,
        ),
        (
            2,
            "installed_on".to_owned(),
            "TIMESTAMP".to_owned(),
            1,
            Some("CURRENT_TIMESTAMP".to_owned()),
            0,
            0,
        ),
        (3, "success".to_owned(), "BOOLEAN".to_owned(), 1, None, 0, 0),
        (4, "checksum".to_owned(), "BLOB".to_owned(), 1, None, 0, 0),
        (
            5,
            "execution_time".to_owned(),
            "BIGINT".to_owned(),
            1,
            None,
            0,
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

    let create_sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|source| DbError::LegacyInspection { source })?;
    const SQLX_CREATE_SQL: &str = r#"
        CREATE TABLE _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            success BOOLEAN NOT NULL,
            checksum BLOB NOT NULL,
            execution_time BIGINT NOT NULL
        )
    "#;
    if normalize_schema_sql(&create_sql) != normalize_schema_sql(SQLX_CREATE_SQL) {
        return Err(DbError::InvalidMigrationMetadata {
            reason: format!(
                "`_sqlx_migrations` table SQL differs from SQLx 0.9: `{}`",
                normalize_schema_sql(&create_sql)
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
    let mut seen = HashSet::new();
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
        if !seen.insert(version) {
            return Err(DbError::InvalidMigrationMetadata {
                reason: format!("migration version {version} appears more than once"),
            });
        }
        let expected = embedded
            .iter()
            .find(|migration| migration.version == version)
            .ok_or_else(|| DbError::InvalidMigrationMetadata {
                reason: format!(
                    "migration version {version} is unknown to this binary; use the binary that created it or restore a compatible database backup"
                ),
            })?;
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

    for table in COMPATIBILITY_TABLES {
        verify_compatibility_table(connection, &mut reference, table).await?;
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

type ColumnShape = (i64, String, String, i64, Option<String>, i64, i64);
type ForeignKeyShape = (String, String, String, String, String, String);

async fn verify_compatibility_table(
    actual: &mut SqliteConnection,
    reference: &mut SqliteConnection,
    table: &str,
) -> Result<(), DbError> {
    let actual_columns = table_columns(actual, table).await?;
    let expected_columns = table_columns(reference, table).await?;
    let actual_names = actual_columns
        .iter()
        .map(|column| column.1.clone())
        .collect::<Vec<_>>();
    let expected_names = expected_columns
        .iter()
        .map(|column| column.1.clone())
        .collect::<Vec<_>>();
    if !allowed_column_layouts(table, &expected_names).contains(&actual_names) {
        return Err(DbError::UnsupportedLegacySchema {
            reason: format!(
                "unsupported Python compatibility column order for {table}: {actual_names:?}"
            ),
        });
    }

    for actual_column in &actual_columns {
        let expected = expected_columns
            .iter()
            .find(|column| column.1 == actual_column.1)
            .ok_or_else(|| DbError::UnsupportedLegacySchema {
                reason: format!("unknown column {table}.{}", actual_column.1),
            })?;
        if !column_shape_is_supported(table, actual_column, expected) {
            return Err(DbError::UnsupportedLegacySchema {
                reason: format!(
                    "unsupported shape for {table}.{}: expected {expected:?}, found {actual_column:?}",
                    actual_column.1
                ),
            });
        }
    }

    verify_compatibility_foreign_keys(actual, reference, table, &actual_names).await?;
    verify_compatibility_table_constraints(actual, table, &actual_names).await
}

async fn table_columns(
    connection: &mut SqliteConnection,
    table: &str,
) -> Result<Vec<ColumnShape>, DbError> {
    sqlx::query(
        "SELECT cid, name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo(?) ORDER BY cid",
    )
    .bind(table)
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| DbError::LegacyInspection { source })?
    .into_iter()
    .map(|row| {
        Ok((
            row.try_get("cid")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("name")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("type")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("notnull")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("dflt_value")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("pk")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("hidden")
                .map_err(|source| DbError::LegacyInspection { source })?,
        ))
    })
    .collect()
}

fn allowed_column_layouts(table: &str, current: &[String]) -> Vec<Vec<String>> {
    let compatibility = COMPATIBILITY_COLUMNS
        .iter()
        .filter(|(candidate, _, _)| *candidate == table)
        .map(|(_, column, _)| (*column).to_owned())
        .collect::<Vec<_>>();
    let base = current
        .iter()
        .filter(|column| !compatibility.contains(column))
        .cloned()
        .collect::<Vec<_>>();
    let mut appended = base.clone();
    appended.extend(compatibility.clone());
    let mut layouts = vec![current.to_vec(), base.clone(), appended];
    if table == "crystals" {
        let mut first_stage = base.clone();
        first_stage.extend(["soft_origin".to_owned(), "is_inferred".to_owned()]);
        layouts.push(first_stage.clone());
        first_stage.extend([
            "source_credibility".to_owned(),
            "rule_intent".to_owned(),
            "malformed_penalty".to_owned(),
            "supersedes_crystal_id".to_owned(),
        ]);
        layouts.push(first_stage);
        let declared_late = current
            .iter()
            .filter(|column| column.as_str() != "soft_origin" && column.as_str() != "is_inferred")
            .cloned()
            .collect::<Vec<_>>();
        layouts.push(declared_late.clone());
        let mut declared_then_first_stage = declared_late;
        declared_then_first_stage.extend(["soft_origin".to_owned(), "is_inferred".to_owned()]);
        layouts.push(declared_then_first_stage);
    }
    layouts.sort();
    layouts.dedup();
    layouts
}

fn column_shape_is_supported(table: &str, actual: &ColumnShape, expected: &ColumnShape) -> bool {
    let same_except_cid = actual.1 == expected.1
        && actual.2 == expected.2
        && actual.3 == expected.3
        && actual.5 == expected.5
        && actual.6 == expected.6;
    if !same_except_cid {
        return false;
    }
    actual.4 == expected.4
        || (table == "task_sessions"
            && actual.1 == "last_activity_at"
            && actual.4.as_deref() == Some("''")
            && expected.4.is_none())
        || (table == "crystals"
            && actual.1 == "malformed_penalty"
            && actual.4.as_deref() == Some("0")
            && expected.4.as_deref() == Some("0.0"))
}

async fn verify_compatibility_foreign_keys(
    actual: &mut SqliteConnection,
    reference: &mut SqliteConnection,
    table: &str,
    actual_names: &[String],
) -> Result<(), DbError> {
    let actual_keys = table_foreign_keys(actual, table).await?;
    let expected_keys = table_foreign_keys(reference, table).await?;
    let compatibility = COMPATIBILITY_COLUMNS
        .iter()
        .filter(|(candidate, _, _)| *candidate == table)
        .map(|(_, column, _)| *column)
        .collect::<Vec<_>>();
    let actual_base = actual_keys
        .iter()
        .filter(|key| !compatibility.contains(&key.0.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let expected_base = expected_keys
        .iter()
        .filter(|key| !compatibility.contains(&key.0.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if actual_base != expected_base {
        return Err(DbError::UnsupportedLegacySchema {
            reason: format!(
                "foreign keys differ for {table}: {actual_base:?} != {expected_base:?}"
            ),
        });
    }
    for column in compatibility {
        let keys = actual_keys
            .iter()
            .filter(|key| key.0 == column)
            .collect::<Vec<_>>();
        if !actual_names.iter().any(|name| name == column) {
            if !keys.is_empty() {
                return Err(DbError::UnsupportedLegacySchema {
                    reason: format!("foreign key exists for missing {table}.{column}"),
                });
            }
            continue;
        }
        let valid = match (table, column, keys.as_slice()) {
            ("concepts", "merged_into_concept_id", [key]) => {
                key.1 == "id"
                    && key.2 == "concepts"
                    && key.3 == "NO ACTION"
                    && key.4 == "NO ACTION"
                    && key.5 == "NONE"
            }
            ("crystals", "supersedes_crystal_id", [key]) => {
                key.1 == "id"
                    && key.2 == "crystals"
                    && key.3 == "NO ACTION"
                    && matches!(key.4.as_str(), "NO ACTION" | "SET NULL")
                    && key.5 == "NONE"
            }
            ("concepts", "merged_into_concept_id", [])
            | ("crystals", "supersedes_crystal_id", []) => false,
            (_, _, []) => true,
            _ => false,
        };
        if !valid {
            return Err(DbError::UnsupportedLegacySchema {
                reason: format!("unsupported foreign key for {table}.{column}: {keys:?}"),
            });
        }
    }
    Ok(())
}

async fn table_foreign_keys(
    connection: &mut SqliteConnection,
    table: &str,
) -> Result<Vec<ForeignKeyShape>, DbError> {
    sqlx::query(
        "SELECT \"from\", \"to\", \"table\", on_update, on_delete, \"match\" FROM pragma_foreign_key_list(?) ORDER BY id, seq",
    )
    .bind(table)
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| DbError::LegacyInspection { source })?
    .into_iter()
    .map(|row| {
        Ok((
            row.try_get("from")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("to")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("table")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("on_update")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("on_delete")
                .map_err(|source| DbError::LegacyInspection { source })?,
            row.try_get("match")
                .map_err(|source| DbError::LegacyInspection { source })?,
        ))
    })
    .collect()
}

async fn verify_compatibility_table_constraints(
    connection: &mut SqliteConnection,
    table: &str,
    columns: &[String],
) -> Result<(), DbError> {
    let sql: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?")
            .bind(table)
            .fetch_one(&mut *connection)
            .await
            .map_err(|source| DbError::LegacyInspection { source })?;
    let compact = compact_schema_sql(&sql);
    if compact.contains("withoutrowid") || compact.ends_with("strict") {
        return Err(DbError::UnsupportedLegacySchema {
            reason: format!("unsupported table option on {table}"),
        });
    }
    let check_count = compact.matches("check(").count();
    if table == "concepts" {
        const SCOPE_CHECK: &str =
            "check((scope_type='global'andscope_key='')or(scope_type!='global'andscope_key!=''))";
        if check_count != 1 || !compact.contains(SCOPE_CHECK) {
            return Err(DbError::UnsupportedLegacySchema {
                reason: "concepts must retain the exact scope CHECK".to_owned(),
            });
        }
        let has_unique = compact.contains("unique(scope_type,scope_key,canonical_name)");
        let merged = columns
            .iter()
            .any(|column| column == "merged_into_concept_id");
        if has_unique
            && merged
            && columns
                .iter()
                .position(|column| column == "merged_into_concept_id")
                != columns.len().checked_sub(1)
        {
            return Err(DbError::UnsupportedLegacySchema {
                reason:
                    "concepts unique-name legacy shape has an impossible merged-column position"
                        .to_owned(),
            });
        }
        if compact.matches("unique(").count() != usize::from(has_unique) {
            return Err(DbError::UnsupportedLegacySchema {
                reason: "concepts has an unknown UNIQUE constraint".to_owned(),
            });
        }
    } else if check_count != 0 || compact.contains("unique(") {
        return Err(DbError::UnsupportedLegacySchema {
            reason: format!("{table} has an unknown table constraint"),
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
        let object_type: String = row
            .try_get("type")
            .map_err(|source| DbError::LegacyInspection { source })?;
        let name: String = row
            .try_get("name")
            .map_err(|source| DbError::LegacyInspection { source })?;
        let sql = row
            .try_get::<String, _>("sql")
            .map_err(|source| DbError::LegacyInspection { source })?;
        let normalized_sql = if object_type == "table"
            && COMPATIBILITY_TABLES.contains(&name.as_str())
        {
            "validated compatibility table".to_owned()
        } else {
            normalize_schema_sql(&sql)
        };
        Ok((
            object_type,
            name,
            row.try_get("tbl_name")
                .map_err(|source| DbError::LegacyInspection { source })?,
            normalized_sql,
        ))
    })
    .collect()
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
        .replace(" if not exists ", " ")
}

fn compact_schema_sql(sql: &str) -> String {
    normalize_schema_sql(sql)
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '"')
        .collect()
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

#[cfg(test)]
mod tests {
    use sqlx::{
        AssertSqlSafe, Connection, SqlSafeStr, SqliteConnection,
        migrate::{Migration, MigrationType, Migrator},
    };

    use super::validate_migration_metadata;

    fn migration(version: i64) -> Migration {
        Migration::new(
            version,
            format!("migration {version}").into(),
            MigrationType::Simple,
            AssertSqlSafe(format!(
                "CREATE TABLE migration_{version} (id INTEGER PRIMARY KEY)"
            ))
            .into_sql_str(),
            false,
        )
    }

    #[tokio::test]
    async fn stored_versions_remain_valid_when_future_migrator_inserts_lower_version() {
        let mut connection = SqliteConnection::connect("sqlite::memory:")
            .await
            .expect("future-migrator fixture should connect");
        let old = Migrator::with_migrations(vec![migration(1), migration(3), migration(4)]);
        old.run(&mut connection)
            .await
            .expect("old migration history should install");

        let future =
            Migrator::with_migrations(vec![migration(1), migration(2), migration(3), migration(4)]);
        validate_migration_metadata(&mut connection, &future)
            .await
            .expect("stored versions are a valid subset of the future migrator");
        future
            .run(&mut connection)
            .await
            .expect("future migrator should apply missing version 2");

        let versions: Vec<i64> = sqlx::query_scalar(
            "SELECT version FROM _sqlx_migrations WHERE success = 1 ORDER BY version",
        )
        .fetch_all(&mut connection)
        .await
        .expect("future history should read");
        assert_eq!(versions, [1, 2, 3, 4]);
        let version_two_table: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'migration_2'",
        )
        .fetch_one(&mut connection)
        .await
        .expect("version 2 table should inspect");
        assert_eq!(version_two_table, 1);
    }
}
