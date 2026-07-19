use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqliteConnection, SqlitePool};

use super::{
    DbError,
    models::{StrictTermAliasRow, StrictTermRow},
};

const MAX_SOURCE_ROWS: usize = 100_000;
const MAX_FIELD_BYTES: usize = 65_536;
const MAX_TAGS_PER_TERM: usize = 1_024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyConversionReport {
    pub source_rows: i64,
    pub converted_rows: i64,
    pub existing_rows: i64,
    pub dropped: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct ExpectedCrystal {
    source_id: String,
    text: String,
    title: String,
    scope_key: String,
    series_slug: String,
    source_language: String,
    target_language: String,
    tags_json: String,
    tags: Vec<String>,
    rule_intent: String,
    notes: String,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct TargetCrystal {
    id: i64,
    crystal_type: String,
    text: String,
    title: String,
    scope_type: String,
    scope_key: String,
    series_slug: String,
    source_language: String,
    target_language: String,
    tags_json: String,
    strength: f64,
    confidence: f64,
    source_credibility: String,
    rule_intent: String,
    soft_origin: Option<String>,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub async fn convert_legacy_strict_terms(
    pool: &SqlitePool,
) -> Result<LegacyConversionReport, DbError> {
    let mut connection = pool.acquire().await.map_err(conversion_error)?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map_err(conversion_error)?;

    match convert_in_transaction(&mut connection).await {
        Ok(report) => match sqlx::query("COMMIT").execute(&mut *connection).await {
            Ok(_) => Ok(report),
            Err(source) => {
                let error = conversion_error(source);
                rollback_after_error(&mut connection, error).await
            }
        },
        Err(error) => rollback_after_error(&mut connection, error).await,
    }
}

async fn convert_in_transaction(
    connection: &mut SqliteConnection,
) -> Result<LegacyConversionReport, DbError> {
    let objects = legacy_object_count(connection).await?;
    if objects == 0 {
        let artifacts = legacy_artifact_count(connection).await?;
        if artifacts == 0 {
            return Ok(LegacyConversionReport::default());
        }
        return Err(invalid(
            "schema",
            "legacy objects",
            format!("found {artifacts} orphan strict-term artifacts"),
        ));
    }
    if objects != 4 {
        return Err(DbError::InvalidLegacyTerm {
            source_id: "schema".to_owned(),
            field: "legacy objects",
            reason: format!("expected all four strict-term objects, found {objects}"),
        });
    }

    let terms = sqlx::query_as::<_, StrictTermRow>(
        "SELECT id, series_slug, source_language, target_language, category, source_text, canonical_translation, status, notes, created_at, updated_at FROM strict_terms ORDER BY id",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(conversion_error)?;
    if terms.len() > MAX_SOURCE_ROWS {
        return Err(invalid(
            "schema",
            "source rows",
            format!("exceeds {MAX_SOURCE_ROWS}"),
        ));
    }

    let source_rows = i64::try_from(terms.len())
        .map_err(|error| invalid("schema", "source rows", error.to_string()))?;
    let mut converted_rows = 0_i64;
    let mut existing_rows = 0_i64;
    let mut expected_rows = Vec::with_capacity(terms.len());

    for term in &terms {
        let aliases = sqlx::query_as::<_, StrictTermAliasRow>(
            "SELECT id, term_id, language, text, kind, case_sensitive FROM strict_term_aliases WHERE term_id = ? ORDER BY id",
        )
        .bind(term.id)
        .fetch_all(&mut *connection)
        .await
        .map_err(conversion_error)?;
        let tags: Vec<String> =
            sqlx::query_scalar("SELECT tag FROM strict_term_tags WHERE term_id = ? ORDER BY tag")
                .bind(term.id)
                .fetch_all(&mut *connection)
                .await
                .map_err(conversion_error)?;
        let expected = expected_crystal(term, &aliases, &tags)?;

        let ledger_target: Option<i64> = sqlx::query_scalar(
            "SELECT target_id FROM migration_ledger WHERE source_table = 'strict_terms' AND source_id = ? AND target_table = 'crystals'",
        )
        .bind(&expected.source_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(conversion_error)?;

        let target_id = if let Some(target_id) = ledger_target {
            ensure_equivalent(connection, target_id, &expected, true).await?;
            existing_rows += 1;
            target_id
        } else if let Some(target_id) = find_equivalent(connection, &expected).await? {
            insert_ledger(connection, &expected.source_id, target_id).await?;
            existing_rows += 1;
            target_id
        } else {
            let target_id = insert_crystal(connection, &expected).await?;
            insert_tags(connection, target_id, &expected).await?;
            insert_ledger(connection, &expected.source_id, target_id).await?;
            converted_rows += 1;
            target_id
        };
        ensure_equivalent(connection, target_id, &expected, true).await?;
        expected_rows.push((target_id, expected));
    }

    verify_parity(
        connection,
        source_rows,
        converted_rows,
        existing_rows,
        &expected_rows,
    )
    .await?;
    verify_integrity(connection).await?;
    drop_legacy_objects(connection).await?;
    verify_integrity(connection).await?;
    if legacy_artifact_count(connection).await? != 0 {
        return Err(invalid(
            "schema",
            "legacy objects",
            "objects remain after guarded drop",
        ));
    }

    Ok(LegacyConversionReport {
        source_rows,
        converted_rows,
        existing_rows,
        dropped: true,
    })
}

async fn rollback_after_error(
    connection: &mut SqliteConnection,
    error: DbError,
) -> Result<LegacyConversionReport, DbError> {
    if let Err(source) = sqlx::query("ROLLBACK").execute(&mut *connection).await {
        return Err(DbError::LegacyTermRollback {
            original: error.to_string(),
            source,
        });
    }
    Err(error)
}

fn expected_crystal(
    term: &StrictTermRow,
    aliases: &[StrictTermAliasRow],
    tags: &[String],
) -> Result<ExpectedCrystal, DbError> {
    let source_id = term.id.to_string();
    if term.id < 0 {
        return Err(invalid(&source_id, "id", "must be non-negative"));
    }
    for (field, value, allow_empty) in [
        ("series_slug", term.series_slug.as_str(), false),
        ("source_language", term.source_language.as_str(), false),
        ("target_language", term.target_language.as_str(), false),
        ("category", term.category.as_str(), false),
        ("source_text", term.source_text.as_str(), false),
        (
            "canonical_translation",
            term.canonical_translation.as_str(),
            false,
        ),
        ("notes", term.notes.as_str(), true),
    ] {
        validate_value(&source_id, field, value, allow_empty)?;
    }

    let status = match term.status.as_str() {
        "approved" | "active" => "active",
        "pending" => "candidate",
        "inactive" | "archived" => "archived",
        "rejected" => "rejected",
        "superseded" => "superseded",
        other => {
            return Err(invalid(
                &source_id,
                "status",
                format!("unsupported `{other}`"),
            ));
        }
    };

    let mut semantic_tags = BTreeSet::new();
    for tag in tags {
        validate_value(&source_id, "tag", tag, false)?;
        semantic_tags.insert(tag.clone());
    }
    for alias in aliases {
        if alias.term_id != term.id || alias.id < 0 {
            return Err(invalid(&source_id, "alias", "invalid alias identity"));
        }
        validate_value(&source_id, "alias language", &alias.language, false)?;
        validate_value(&source_id, "alias kind", &alias.kind, false)?;
        validate_value(&source_id, "alias text", &alias.text, false)?;
        let _case_sensitive = alias.case_sensitive;
        semantic_tags.insert(alias.text.clone());
    }
    if semantic_tags.len() > MAX_TAGS_PER_TERM {
        return Err(invalid(
            &source_id,
            "semantic tags",
            format!("exceeds {MAX_TAGS_PER_TERM}"),
        ));
    }
    let tags: Vec<String> = semantic_tags.into_iter().collect();
    let tags_json = serde_json::to_string(&tags)
        .map_err(|error| invalid(&source_id, "semantic tags", error.to_string()))?;
    let text = format!(
        "{} is translated as {}",
        term.source_text, term.canonical_translation
    );
    validate_value(&source_id, "rule text", &text, false)?;

    Ok(ExpectedCrystal {
        source_id,
        text,
        title: term.source_text.clone(),
        scope_key: format!("series:{}", term.series_slug),
        series_slug: term.series_slug.clone(),
        source_language: term.source_language.clone(),
        target_language: term.target_language.clone(),
        tags_json,
        tags,
        rule_intent: term.category.clone(),
        notes: term.notes.clone(),
        status: status.to_owned(),
        created_at: term.created_at,
        updated_at: term.updated_at,
    })
}

fn validate_value(
    source_id: &str,
    field: &'static str,
    value: &str,
    allow_empty: bool,
) -> Result<(), DbError> {
    if !allow_empty && value.is_empty() {
        return Err(invalid(source_id, field, "must not be empty"));
    }
    if value.len() > MAX_FIELD_BYTES {
        return Err(invalid(
            source_id,
            field,
            format!("exceeds {MAX_FIELD_BYTES} bytes"),
        ));
    }
    Ok(())
}

async fn find_equivalent(
    connection: &mut SqliteConnection,
    expected: &ExpectedCrystal,
) -> Result<Option<i64>, DbError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM crystals WHERE crystal_type = 'rule' AND text = ? AND title = ? AND scope_type = 'series' AND scope_key = ? AND series_slug = ? AND source_language = ? AND target_language = ? ORDER BY id",
    )
    .bind(&expected.text)
    .bind(&expected.title)
    .bind(&expected.scope_key)
    .bind(&expected.series_slug)
    .bind(&expected.source_language)
    .bind(&expected.target_language)
    .fetch_all(&mut *connection)
    .await
    .map_err(conversion_error)?;
    for id in ids {
        if ensure_equivalent(connection, id, expected, false)
            .await
            .is_ok()
        {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

async fn ensure_equivalent(
    connection: &mut SqliteConnection,
    target_id: i64,
    expected: &ExpectedCrystal,
    conflict: bool,
) -> Result<(), DbError> {
    let target = sqlx::query_as::<_, TargetCrystal>(
        "SELECT id, crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, soft_origin, status, created_at, updated_at FROM crystals WHERE id = ?",
    )
    .bind(target_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(conversion_error)?;
    let Some(target) = target else {
        return Err(conflict_error(
            expected,
            target_id,
            "ledger target does not exist",
        ));
    };
    let tags: Vec<String> = sqlx::query_scalar(
        "SELECT tag FROM crystal_semantic_tags WHERE crystal_id = ? ORDER BY tag",
    )
    .bind(target_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(conversion_error)?;
    let equivalent = target.id == target_id
        && target.crystal_type == "rule"
        && target.text == expected.text
        && target.title == expected.title
        && target.scope_type == "series"
        && target.scope_key == expected.scope_key
        && target.series_slug == expected.series_slug
        && target.source_language == expected.source_language
        && target.target_language == expected.target_language
        && target.tags_json == expected.tags_json
        && target.strength == 0.8
        && target.confidence == 0.95
        && target.source_credibility == "user_rule"
        && target.rule_intent == expected.rule_intent
        && target.soft_origin.as_deref() == Some(expected.notes.as_str())
        && target.status == expected.status
        && target.created_at == expected.created_at
        && target.updated_at == expected.updated_at
        && tags == expected.tags;
    if equivalent {
        Ok(())
    } else if conflict {
        Err(conflict_error(
            expected,
            target_id,
            "target fields or semantic tags differ",
        ))
    } else {
        Err(conflict_error(expected, target_id, "not equivalent"))
    }
}

async fn insert_crystal(
    connection: &mut SqliteConnection,
    expected: &ExpectedCrystal,
) -> Result<i64, DbError> {
    sqlx::query(
        "INSERT INTO crystals(crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, soft_origin, is_inferred, malformed_penalty, status, created_cycle, created_at, updated_at) VALUES ('rule', ?, ?, 'series', ?, ?, ?, ?, ?, 0.8, 0.95, 'user_rule', ?, ?, 0, 0.0, ?, 0, ?, ?)",
    )
    .bind(&expected.text)
    .bind(&expected.title)
    .bind(&expected.scope_key)
    .bind(&expected.series_slug)
    .bind(&expected.source_language)
    .bind(&expected.target_language)
    .bind(&expected.tags_json)
    .bind(&expected.rule_intent)
    .bind(&expected.notes)
    .bind(&expected.status)
    .bind(expected.created_at)
    .bind(expected.updated_at)
    .execute(&mut *connection)
    .await
    .map(|result| result.last_insert_rowid())
    .map_err(conversion_error)
}

async fn insert_tags(
    connection: &mut SqliteConnection,
    target_id: i64,
    expected: &ExpectedCrystal,
) -> Result<(), DbError> {
    for tag in &expected.tags {
        sqlx::query("INSERT INTO crystal_semantic_tags(crystal_id, tag, confidence, created_at) VALUES (?, ?, 0.95, ?)")
            .bind(target_id)
            .bind(tag)
            .bind(expected.created_at)
            .execute(&mut *connection)
            .await
            .map_err(conversion_error)?;
    }
    Ok(())
}

async fn insert_ledger(
    connection: &mut SqliteConnection,
    source_id: &str,
    target_id: i64,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO migration_ledger(source_table, source_id, target_table, target_id) VALUES ('strict_terms', ?, 'crystals', ?)")
        .bind(source_id)
        .bind(target_id)
        .execute(&mut *connection)
        .await
        .map_err(conversion_error)?;
    Ok(())
}

async fn verify_parity(
    connection: &mut SqliteConnection,
    source_rows: i64,
    converted_rows: i64,
    existing_rows: i64,
    expected_rows: &[(i64, ExpectedCrystal)],
) -> Result<(), DbError> {
    let ledger_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM migration_ledger WHERE source_table = 'strict_terms' AND target_table = 'crystals'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(conversion_error)?;
    if source_rows != converted_rows + existing_rows || ledger_rows != source_rows {
        return Err(DbError::LegacyTermCountMismatch {
            source_rows,
            converted_rows,
            existing_rows,
            ledger_rows,
        });
    }
    for (target_id, expected) in expected_rows {
        let ledger_target: Option<i64> = sqlx::query_scalar(
            "SELECT target_id FROM migration_ledger WHERE source_table = 'strict_terms' AND source_id = ? AND target_table = 'crystals'",
        )
        .bind(&expected.source_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(conversion_error)?;
        if ledger_target != Some(*target_id) {
            return Err(DbError::LegacyTermCountMismatch {
                source_rows,
                converted_rows,
                existing_rows,
                ledger_rows,
            });
        }
        ensure_equivalent(connection, *target_id, expected, true).await?;
    }
    Ok(())
}

async fn verify_integrity(connection: &mut SqliteConnection) -> Result<(), DbError> {
    let foreign_key_violations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pragma_foreign_key_check")
            .fetch_one(&mut *connection)
            .await
            .map_err(conversion_error)?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut *connection)
        .await
        .map_err(conversion_error)?;
    if foreign_key_violations != 0 || integrity != "ok" {
        return Err(invalid(
            "schema",
            "integrity",
            format!("foreign keys={foreign_key_violations}, integrity={integrity}"),
        ));
    }
    sqlx::query("INSERT INTO crystals_fts(crystals_fts, rank) VALUES ('integrity-check', 1)")
        .execute(&mut *connection)
        .await
        .map_err(conversion_error)?;
    Ok(())
}

async fn drop_legacy_objects(connection: &mut SqliteConnection) -> Result<(), DbError> {
    for statement in [
        "DROP TRIGGER IF EXISTS strict_terms_ai",
        "DROP TRIGGER IF EXISTS strict_terms_ad",
        "DROP TRIGGER IF EXISTS strict_terms_au",
        "DROP TABLE strict_terms_fts",
        "DROP TABLE strict_term_aliases",
        "DROP TABLE strict_term_tags",
        "DROP TABLE strict_terms",
    ] {
        sqlx::query(statement)
            .execute(&mut *connection)
            .await
            .map_err(conversion_error)?;
    }
    Ok(())
}

async fn legacy_object_count(connection: &mut SqliteConnection) -> Result<i64, DbError> {
    sqlx::query(
        "SELECT name FROM sqlite_schema WHERE name IN ('strict_terms', 'strict_term_tags', 'strict_term_aliases', 'strict_terms_fts') ORDER BY name",
    )
    .fetch_all(&mut *connection)
    .await
    .map(|rows| i64::try_from(rows.len()).expect("four schema rows fit in i64"))
    .map_err(conversion_error)
}

async fn legacy_artifact_count(connection: &mut SqliteConnection) -> Result<i64, DbError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE name IN ('strict_terms', 'strict_term_tags', 'strict_term_aliases', 'strict_terms_fts', 'strict_terms_ai', 'strict_terms_ad', 'strict_terms_au') OR name LIKE 'strict_terms_fts_%'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(conversion_error)
}

fn invalid(
    source_id: impl Into<String>,
    field: &'static str,
    reason: impl Into<String>,
) -> DbError {
    DbError::InvalidLegacyTerm {
        source_id: source_id.into(),
        field,
        reason: reason.into(),
    }
}

fn conflict_error(expected: &ExpectedCrystal, target_id: i64, reason: &str) -> DbError {
    DbError::LegacyTermConflict {
        source_id: expected.source_id.clone(),
        target_id,
        reason: reason.to_owned(),
    }
}

fn conversion_error(source: sqlx::Error) -> DbError {
    DbError::LegacyTermConversion { source }
}
