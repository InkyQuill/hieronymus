use std::{collections::BTreeSet, sync::OnceLock};

use chrono::Utc;
use regex::Regex;
use sqlx::{
    FromRow, QueryBuilder, Row, Sqlite, SqliteConnection, SqlitePool, pool::PoolConnection,
};

use crate::db::{CrystalLinkRecord, CrystalRecord, CrystalStatus, CrystalType};

use super::models::{AddCrystalInput, RuleFilter, TranslationContext, ValidationReport};

const MAX_SEARCH_LIMIT: usize = 50;
const CRYSTAL_COLUMNS: &str = "id, crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, soft_origin, is_inferred, malformed_penalty, supersedes_crystal_id, status, created_cycle, last_activated_cycle, last_reinforced_cycle, created_at, updated_at";
const SELECT_CRYSTAL_BY_ID: &str = "SELECT id, crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, soft_origin, is_inferred, malformed_penalty, supersedes_crystal_id, status, created_cycle, last_activated_cycle, last_reinforced_cycle, created_at, updated_at FROM crystals WHERE id = ?";
const SEARCH_CRYSTALS: &str = "SELECT crystals.id, crystals.crystal_type, crystals.text, crystals.title, crystals.scope_type, crystals.scope_key, crystals.series_slug, crystals.source_language, crystals.target_language, crystals.tags_json, crystals.strength, crystals.confidence, crystals.source_credibility, crystals.rule_intent, crystals.soft_origin, crystals.is_inferred, crystals.malformed_penalty, crystals.supersedes_crystal_id, crystals.status, crystals.created_cycle, crystals.last_activated_cycle, crystals.last_reinforced_cycle, crystals.created_at, crystals.updated_at, (max(-bm25(crystals_fts), 0.0) + crystals.strength * 0.35 + crystals.confidence * 0.20 + CASE WHEN crystals.scope_type = 'series' THEN 0.05 ELSE 0.0 END) AS search_score FROM crystals_fts JOIN crystals ON crystals.id = crystals_fts.rowid WHERE crystals_fts MATCH ? AND crystals.status IN ('active', 'candidate') AND ((crystals.scope_type = 'series' AND crystals.scope_key = ?) OR crystals.scope_type = 'global') AND (crystals.source_language = ? OR crystals.source_language = '') AND (crystals.target_language = ? OR crystals.target_language = '') ORDER BY search_score DESC, crystals.id ASC LIMIT ?";

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error("invalid crystal {field}: {reason}")]
    Validation { field: &'static str, reason: String },
    #[error("unknown crystal: {id}")]
    NotFound { id: i64 },
    #[error("crystal {operation} violated a database constraint: {message}")]
    Constraint {
        operation: &'static str,
        message: String,
        #[source]
        source: sqlx::Error,
    },
    #[error("crystal {operation} failed: {source}")]
    Database {
        operation: &'static str,
        #[source]
        source: sqlx::Error,
    },
    #[error("invalid raw FTS5 expression `{expression}`: {message}")]
    FtsExpression { expression: String, message: String },
    #[error("failed to serialize normalized crystal tags: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to roll back crystal {operation} after `{original}`: {source}")]
    Rollback {
        operation: &'static str,
        original: String,
        #[source]
        source: sqlx::Error,
    },
}

pub struct CrystalStore<'a> {
    pool: &'a SqlitePool,
}

impl<'a> CrystalStore<'a> {
    #[must_use]
    pub const fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn add(&self, input: AddCrystalInput) -> Result<i64> {
        let input = ValidatedInput::try_from(input)?;
        let mut connection = begin_immediate(self.pool, "add").await?;
        let result = add_in_transaction(&mut connection, &input).await;
        finish_write(&mut connection, "add", result).await
    }

    pub async fn get(&self, id: i64) -> Result<CrystalRecord> {
        sqlx::query_as::<_, CrystalRecord>(SELECT_CRYSTAL_BY_ID)
            .bind(id)
            .fetch_optional(self.pool)
            .await
            .map_err(|source| database("get", source))?
            .ok_or(StoreError::NotFound { id })
    }

    pub async fn list_rule_intent(&self, filter: RuleFilter) -> Result<Vec<CrystalRecord>> {
        let limit = bounded_limit(filter.limit)?;
        if let Some(status) = filter.status.as_deref() {
            validate_status(status)?;
        }
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {CRYSTAL_COLUMNS} FROM crystals WHERE trim(rule_intent) <> ''"
        ));
        if let Some(status) = filter.status {
            query.push(" AND status = ").push_bind(status);
        }
        if let Some(series_slug) = filter.series_slug {
            query.push(" AND series_slug = ").push_bind(series_slug);
        }
        query
            .push(" ORDER BY id DESC LIMIT ")
            .push_bind(limit as i64);
        query
            .build_query_as::<CrystalRecord>()
            .fetch_all(self.pool)
            .await
            .map_err(|source| database("list rule-intent crystals", source))
    }

    pub async fn archive(&self, id: i64) -> Result<CrystalRecord> {
        let mut connection = begin_immediate(self.pool, "archive").await?;
        let result = async {
            get_on(&mut connection, id, "archive").await?;
            sqlx::query(
                "UPDATE crystals SET status = 'archived', updated_at = ? WHERE id = ? AND status <> 'archived'",
            )
            .bind(Utc::now())
            .bind(id)
            .execute(&mut *connection)
            .await
            .map_err(|source| database("archive", source))?;
            get_on(&mut connection, id, "archive").await
        }
        .await;
        finish_write(&mut connection, "archive", result).await
    }

    pub async fn supersede(&self, old_id: i64, mut new: AddCrystalInput) -> Result<CrystalRecord> {
        if let Some(requested) = new.supersedes_crystal_id
            && requested != old_id
        {
            return Err(invalid(
                "supersedes_crystal_id",
                format!("must be {old_id} for this supersession, got {requested}"),
            ));
        }
        new.supersedes_crystal_id = Some(old_id);
        let new = ValidatedInput::try_from(new)?;
        if !matches!(new.status.as_str(), "active" | "candidate") {
            return Err(invalid(
                "status",
                "new superseding crystal must be active or candidate",
            ));
        }

        let mut connection = begin_immediate(self.pool, "supersede").await?;
        let result = async {
            let old = get_on(&mut connection, old_id, "supersede").await?;
            if !matches!(old.status.as_str(), "active" | "candidate") {
                return Err(invalid(
                    "status",
                    "old superseded crystal must be active or candidate",
                ));
            }
            for (field, old_value, new_value) in [
                ("crystal_type", old.crystal_type.as_str(), new.crystal_type.as_str()),
                ("scope_type", old.scope_type.as_str(), new.scope_type.as_str()),
                ("scope_key", old.scope_key.as_str(), new.scope_key.as_str()),
                ("series_slug", old.series_slug.as_str(), new.series_slug.as_str()),
                (
                    "source_language",
                    old.source_language.as_str(),
                    new.source_language.as_str(),
                ),
                (
                    "target_language",
                    old.target_language.as_str(),
                    new.target_language.as_str(),
                ),
            ] {
                if old_value != new_value {
                    return Err(invalid(field, "does not match the superseded crystal"));
                }
            }

            let new_id = add_in_transaction(&mut connection, &new).await?;
            let updated = sqlx::query(
                "UPDATE crystals SET status = 'superseded', updated_at = ? WHERE id = ? AND status IN ('active', 'candidate')",
            )
            .bind(Utc::now())
            .bind(old_id)
            .execute(&mut *connection)
            .await
            .map_err(|source| database("supersede", source))?;
            if updated.rows_affected() != 1 {
                return Err(invalid("status", "superseded crystal changed concurrently"));
            }
            get_on(&mut connection, new_id, "supersede").await
        }
        .await;
        finish_write(&mut connection, "supersede", result).await
    }

    pub async fn link(
        &self,
        source_id: i64,
        target_id: i64,
        link_type: &str,
    ) -> Result<CrystalLinkRecord> {
        if source_id == target_id {
            return Err(invalid("link", "a crystal cannot link to itself"));
        }
        let link_type = link_type.trim();
        if link_type.is_empty() {
            return Err(invalid("link_type", "must not be empty"));
        }
        let mut connection = begin_immediate(self.pool, "link").await?;
        let result = async {
            get_on(&mut connection, source_id, "link").await?;
            get_on(&mut connection, target_id, "link").await?;
            sqlx::query("INSERT OR IGNORE INTO crystal_links(source_crystal_id, target_crystal_id, link_type) VALUES (?, ?, ?)")
                .bind(source_id)
                .bind(target_id)
                .bind(link_type)
                .execute(&mut *connection)
                .await
                .map_err(|source| database("link", source))?;
            Ok(CrystalLinkRecord {
                source_crystal_id: source_id,
                target_crystal_id: target_id,
                link_type: link_type.to_owned(),
            })
        }
        .await;
        finish_write(&mut connection, "link", result).await
    }

    pub async fn linked(&self, crystal_id: i64) -> Result<Vec<(CrystalLinkRecord, f64)>> {
        self.get(crystal_id).await?;
        sqlx::query_as::<_, CrystalLinkRecord>(
            "SELECT source_crystal_id, target_crystal_id, link_type FROM crystal_links WHERE source_crystal_id = ? OR target_crystal_id = ? ORDER BY CASE WHEN source_crystal_id = ? THEN target_crystal_id ELSE source_crystal_id END, source_crystal_id, target_crystal_id, link_type",
        )
        .bind(crystal_id)
        .bind(crystal_id)
        .bind(crystal_id)
        .fetch_all(self.pool)
        .await
        .map(|links| links.into_iter().map(|link| (link, 1.0)).collect())
        .map_err(|source| database("list linked crystals", source))
    }

    pub async fn validate_rule(&self, id: i64) -> Result<ValidationReport> {
        let crystal = self.get(id).await?;
        let mut findings = Vec::new();
        if crystal.rule_intent.trim().is_empty() {
            findings.push("rule_intent must not be empty".to_owned());
        }
        if crystal.status != "active" {
            findings.push("rule-intent crystal must be active".to_owned());
        }
        Ok(ValidationReport {
            ok: findings.is_empty(),
            findings,
        })
    }

    pub async fn set_story_scopes(&self, id: i64, scopes: &[String]) -> Result<CrystalRecord> {
        let scopes = normalize_texts(scopes, false);
        let mut connection = begin_immediate(self.pool, "set story scopes").await?;
        let result = async {
            let crystal = get_on(&mut connection, id, "set story scopes").await?;
            sqlx::query("DELETE FROM crystal_story_scopes WHERE crystal_id = ?")
                .bind(id)
                .execute(&mut *connection)
                .await
                .map_err(|source| database("set story scopes", source))?;
            let now = Utc::now();
            for scope in &scopes {
                sqlx::query("INSERT INTO crystal_story_scopes(crystal_id, scope, confidence, created_at) VALUES (?, ?, ?, ?)")
                    .bind(id)
                    .bind(scope)
                    .bind(crystal.confidence)
                    .bind(now)
                    .execute(&mut *connection)
                    .await
                    .map_err(|source| database("set story scopes", source))?;
            }
            touch_and_get(&mut connection, id, "set story scopes").await
        }
        .await;
        finish_write(&mut connection, "set story scopes", result).await
    }

    pub async fn set_semantic_tags(&self, id: i64, tags: &[String]) -> Result<CrystalRecord> {
        let tags = normalize_texts(tags, false);
        let tags_json =
            serde_json::to_string(&tags).map_err(|source| StoreError::Json { source })?;
        let mut connection = begin_immediate(self.pool, "set semantic tags").await?;
        let result = async {
            let crystal = get_on(&mut connection, id, "set semantic tags").await?;
            sqlx::query("DELETE FROM crystal_semantic_tags WHERE crystal_id = ?")
                .bind(id)
                .execute(&mut *connection)
                .await
                .map_err(|source| database("set semantic tags", source))?;
            let now = Utc::now();
            for tag in &tags {
                sqlx::query("INSERT INTO crystal_semantic_tags(crystal_id, tag, confidence, created_at) VALUES (?, ?, ?, ?)")
                    .bind(id)
                    .bind(tag)
                    .bind(crystal.confidence)
                    .bind(now)
                    .execute(&mut *connection)
                    .await
                    .map_err(|source| database("set semantic tags", source))?;
            }
            sqlx::query("UPDATE crystals SET tags_json = ?, updated_at = ? WHERE id = ?")
                .bind(tags_json)
                .bind(now)
                .bind(id)
                .execute(&mut *connection)
                .await
                .map_err(|source| database("set semantic tags", source))?;
            get_on(&mut connection, id, "set semantic tags").await
        }
        .await;
        finish_write(&mut connection, "set semantic tags", result).await
    }

    pub async fn lowest_confidence(&self, ids: &[i64], limit: usize) -> Result<Vec<i64>> {
        if ids.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let ids: BTreeSet<i64> = ids.iter().copied().collect();
        let mut query = QueryBuilder::<Sqlite>::new("SELECT id FROM crystals WHERE id IN (");
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(") AND NOT (status = 'active' AND trim(rule_intent) <> '') ORDER BY confidence ASC, strength ASC, id ASC LIMIT ");
        query.push_bind(limit.min(MAX_SEARCH_LIMIT) as i64);
        query
            .build_query_scalar::<i64>()
            .fetch_all(self.pool)
            .await
            .map_err(|source| database("select lowest-confidence crystals", source))
    }

    pub async fn search(
        &self,
        ctx: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CrystalRecord>> {
        Ok(self
            .search_scored(ctx, query, limit)
            .await?
            .into_iter()
            .map(|(crystal, _)| crystal)
            .collect())
    }

    pub async fn search_scored(
        &self,
        ctx: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(CrystalRecord, f64)>> {
        let expression = search_expression(query);
        if expression.is_empty() {
            bounded_limit(limit)?;
            return Ok(Vec::new());
        }
        self.search_expression(ctx, &expression, limit).await
    }

    /// Executes an intentional raw FTS5 expression. Use [`Self::search`] for untrusted plain text.
    pub async fn search_expression(
        &self,
        ctx: &TranslationContext,
        expression: &str,
        limit: usize,
    ) -> Result<Vec<(CrystalRecord, f64)>> {
        let limit = bounded_limit(limit)?;
        if expression.trim().is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(SEARCH_CRYSTALS)
            .bind(expression)
            .bind(&ctx.scope_key)
            .bind(&ctx.source_language)
            .bind(&ctx.target_language)
            .bind(limit as i64)
            .fetch_all(self.pool)
            .await
            .map_err(|source| fts_error(expression, source))?;
        rows.into_iter()
            .map(|row| {
                let score = row
                    .try_get::<f64, _>("search_score")
                    .map_err(|source| database("decode crystal search score", source))?;
                let crystal = CrystalRecord::from_row(&row)
                    .map_err(|source| database("decode crystal search result", source))?;
                Ok((crystal, score))
            })
            .collect()
    }
}

#[must_use]
pub fn search_expression(query: &str) -> String {
    static TOKEN_RE: OnceLock<Regex> = OnceLock::new();
    let token_re = TOKEN_RE.get_or_init(|| Regex::new(r"[\p{L}\p{N}_]+").expect("constant regex"));
    // Keep FTS operators from recovering their special meaning after token quoting.
    token_re
        .find_iter(query)
        .map(|capture| capture.as_str())
        .filter(|token| {
            !matches!(
                token.to_ascii_lowercase().as_str(),
                "and" | "or" | "not" | "near"
            )
        })
        .map(|token| format!("\"{token}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

struct ValidatedInput {
    crystal_type: String,
    title: String,
    text: String,
    scope_type: String,
    scope_key: String,
    series_slug: String,
    source_language: String,
    target_language: String,
    source_credibility: String,
    rule_intent: String,
    strength: f64,
    confidence: f64,
    tags: Vec<String>,
    language_tags: Vec<String>,
    story_scopes: Vec<String>,
    semantic_tags: Vec<String>,
    soft_origin: Option<String>,
    is_inferred: bool,
    malformed_penalty: f64,
    supersedes_crystal_id: Option<i64>,
    status: String,
}

impl TryFrom<AddCrystalInput> for ValidatedInput {
    type Error = StoreError;

    fn try_from(input: AddCrystalInput) -> Result<Self> {
        input
            .crystal_type
            .parse::<CrystalType>()
            .map_err(|error| invalid("crystal_type", error.to_string()))?;
        validate_status(&input.status)?;
        validate_score("strength", input.strength)?;
        validate_score("confidence", input.confidence)?;
        if !input.malformed_penalty.is_finite() || input.malformed_penalty < 0.0 {
            return Err(invalid(
                "malformed_penalty",
                "must be finite and at least 0",
            ));
        }
        let text = input.text.trim().to_owned();
        if text.is_empty() {
            return Err(invalid("text", "must not be empty"));
        }
        let scope_type = input.scope_type.trim().to_owned();
        let scope_key = input.scope_key.trim().to_owned();
        if scope_type.is_empty() {
            return Err(invalid("scope_type", "must not be empty"));
        }
        if (scope_type == "global") != scope_key.is_empty() {
            return Err(invalid(
                "scope_key",
                "must be empty for global scope and non-empty otherwise",
            ));
        }
        if !crate::values::SOURCE_CREDIBILITY_CONFIDENCE
            .contains_key(input.source_credibility.trim())
        {
            return Err(invalid(
                "source_credibility",
                format!("unknown label: {}", input.source_credibility),
            ));
        }

        Ok(Self {
            crystal_type: input.crystal_type,
            title: input.title.trim().to_owned(),
            text,
            scope_type,
            scope_key,
            series_slug: input.series_slug.trim().to_owned(),
            source_language: input.source_language.trim().to_lowercase(),
            target_language: input.target_language.trim().to_lowercase(),
            source_credibility: input.source_credibility.trim().to_owned(),
            rule_intent: input.rule_intent.trim().to_owned(),
            strength: input.strength,
            confidence: input.confidence,
            tags: normalize_texts(&input.tags, false),
            language_tags: normalize_texts(&input.language_tags, true),
            story_scopes: normalize_texts(&input.story_scopes, false),
            semantic_tags: normalize_texts(&input.semantic_tags, false),
            soft_origin: input.soft_origin.and_then(|value| {
                let value = value.trim().to_owned();
                (!value.is_empty()).then_some(value)
            }),
            is_inferred: input.is_inferred,
            malformed_penalty: input.malformed_penalty,
            supersedes_crystal_id: input.supersedes_crystal_id,
            status: input.status,
        })
    }
}

async fn add_in_transaction(
    connection: &mut SqliteConnection,
    input: &ValidatedInput,
) -> Result<i64> {
    let tags = if input.semantic_tags.is_empty() {
        &input.tags
    } else {
        &input.semantic_tags
    };
    let tags_json = serde_json::to_string(tags).map_err(|source| StoreError::Json { source })?;
    let now = Utc::now();
    let result = sqlx::query(
        "INSERT INTO crystals(crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, soft_origin, is_inferred, malformed_penalty, supersedes_crystal_id, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.crystal_type)
    .bind(&input.text)
    .bind(&input.title)
    .bind(&input.scope_type)
    .bind(&input.scope_key)
    .bind(&input.series_slug)
    .bind(&input.source_language)
    .bind(&input.target_language)
    .bind(tags_json)
    .bind(input.strength)
    .bind(input.confidence)
    .bind(&input.source_credibility)
    .bind(&input.rule_intent)
    .bind(&input.soft_origin)
    .bind(input.is_inferred)
    .bind(input.malformed_penalty)
    .bind(input.supersedes_crystal_id)
    .bind(&input.status)
    .bind(now)
    .bind(now)
    .execute(&mut *connection)
    .await
    .map_err(|source| database("add", source))?;
    let id = result.last_insert_rowid();
    for language_tag in &input.language_tags {
        sqlx::query("INSERT INTO crystal_language_tags(crystal_id, language_tag) VALUES (?, ?)")
            .bind(id)
            .bind(language_tag)
            .execute(&mut *connection)
            .await
            .map_err(|source| database("add language tag", source))?;
    }
    for scope in &input.story_scopes {
        sqlx::query("INSERT INTO crystal_story_scopes(crystal_id, scope, confidence, created_at) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind(scope)
            .bind(input.confidence)
            .bind(now)
            .execute(&mut *connection)
            .await
            .map_err(|source| database("add story scope", source))?;
    }
    for tag in &input.semantic_tags {
        sqlx::query("INSERT INTO crystal_semantic_tags(crystal_id, tag, confidence, created_at) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind(tag)
            .bind(input.confidence)
            .bind(now)
            .execute(&mut *connection)
            .await
            .map_err(|source| database("add semantic tag", source))?;
    }
    Ok(id)
}

async fn begin_immediate(
    pool: &SqlitePool,
    operation: &'static str,
) -> Result<PoolConnection<Sqlite>> {
    let mut connection = pool
        .acquire()
        .await
        .map_err(|source| database(operation, source))?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map_err(|source| database(operation, source))?;
    Ok(connection)
}

async fn finish_write<T>(
    connection: &mut SqliteConnection,
    operation: &'static str,
    result: Result<T>,
) -> Result<T> {
    match result {
        Ok(value) => sqlx::query("COMMIT")
            .execute(&mut *connection)
            .await
            .map(|_| value)
            .map_err(|source| database(operation, source)),
        Err(error) => match sqlx::query("ROLLBACK").execute(&mut *connection).await {
            Ok(_) => Err(error),
            Err(source) => Err(StoreError::Rollback {
                operation,
                original: error.to_string(),
                source,
            }),
        },
    }
}

async fn get_on(
    connection: &mut SqliteConnection,
    id: i64,
    operation: &'static str,
) -> Result<CrystalRecord> {
    sqlx::query_as::<_, CrystalRecord>(SELECT_CRYSTAL_BY_ID)
        .bind(id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|source| database(operation, source))?
        .ok_or(StoreError::NotFound { id })
}

async fn touch_and_get(
    connection: &mut SqliteConnection,
    id: i64,
    operation: &'static str,
) -> Result<CrystalRecord> {
    sqlx::query("UPDATE crystals SET updated_at = ? WHERE id = ?")
        .bind(Utc::now())
        .bind(id)
        .execute(&mut *connection)
        .await
        .map_err(|source| database(operation, source))?;
    get_on(connection, id, operation).await
}

fn bounded_limit(limit: usize) -> Result<usize> {
    if limit == 0 {
        Err(invalid("limit", "must be at least 1"))
    } else {
        Ok(limit.min(MAX_SEARCH_LIMIT))
    }
}

fn validate_score(field: &'static str, score: f64) -> Result<()> {
    if score.is_finite() && (0.0..=1.0).contains(&score) {
        Ok(())
    } else {
        Err(invalid(field, "must be finite and between 0 and 1"))
    }
}

fn validate_status(status: &str) -> Result<()> {
    status
        .parse::<CrystalStatus>()
        .map(|_| ())
        .map_err(|error| invalid("status", error.to_string()))
}

fn normalize_texts(values: &[String], lowercase: bool) -> Vec<String> {
    let mut values: Vec<String> = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| {
            if lowercase {
                value.to_lowercase()
            } else {
                value.to_owned()
            }
        })
        .collect();
    values.sort();
    values.dedup();
    values
}

fn invalid(field: &'static str, reason: impl Into<String>) -> StoreError {
    StoreError::Validation {
        field,
        reason: reason.into(),
    }
}

fn database(operation: &'static str, source: sqlx::Error) -> StoreError {
    let constraint = source.as_database_error().is_some_and(|error| {
        error.is_check_violation()
            || error.is_foreign_key_violation()
            || error.is_unique_violation()
    });
    if constraint {
        StoreError::Constraint {
            operation,
            message: source.to_string(),
            source,
        }
    } else {
        StoreError::Database { operation, source }
    }
}

fn fts_error(expression: &str, source: sqlx::Error) -> StoreError {
    let message = source.to_string();
    if message.contains("fts5")
        || message.contains("syntax error")
        || message.contains("unterminated")
    {
        StoreError::FtsExpression {
            expression: expression.to_owned(),
            message,
        }
    } else {
        database("search", source)
    }
}
