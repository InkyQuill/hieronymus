use std::path::Path;

use chrono::Utc;
use rusqlite::Connection;

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::memory_models::{CrystalRecord, TranslationContext};
use crate::short_memory::search_expression;

pub const ALLOWED_CRYSTAL_TYPES: [&str; 7] = [
    "lesson",
    "rule",
    "thought",
    "observation",
    "concept_note",
    "concept",
    "erudition",
];
pub const ALLOWED_STATUSES: [&str; 5] =
    ["active", "candidate", "archived", "rejected", "superseded"];
const MAX_SEARCH_LIMIT: usize = 50;

/// Deterministic rule-crystal thresholds (Python `rule_crystals.py`): below
/// either threshold an active rule crystal stays advisory and is never
/// deterministically enforceable.
pub const DETERMINISTIC_RULE_CONFIDENCE_THRESHOLD: f64 = 0.8;
pub const DETERMINISTIC_RULE_STRENGTH_THRESHOLD: f64 = 0.8;

#[derive(Debug, thiserror::Error)]
pub enum CrystalError {
    #[error("unknown crystal: {0}")]
    UnknownCrystal(i64),
    #[error("text must not be empty")]
    EmptyText,
    #[error("unknown crystal_type: {0}")]
    UnknownCrystalType(String),
    #[error("unknown status: {0}")]
    UnknownStatus(String),
    #[error("crystal cannot supersede itself")]
    SelfSupersede,
    #[error("supersede crystals must be active or candidate")]
    SupersedeNotActive,
    #[error("supersede crystal {0} does not match")]
    SupersedeMismatch(String),
    #[error("crystal {0} is an active rule and cannot be superseded here (ADR 0011)")]
    SupersedeActiveRule(i64),
    #[error("limit must be at least 1")]
    LimitTooSmall,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
}

/// Weighted search score: negated bm25 (lower-is-better) plus strength and
/// confidence components plus a small series-scope bonus.
pub fn weighted_search_score(
    raw_bm25: f64,
    strength: f64,
    confidence: f64,
    scope_type: &str,
) -> f64 {
    let fts_component = (-raw_bm25).max(0.0);
    let scope_bonus = f64::from(scope_type == "series");
    fts_component + (strength * 0.35) + (confidence * 0.20) + (scope_bonus * 0.05)
}

/// The parsed canonical rule sentence; the serde projection is the
/// `parsed_rule` member of the `hieronymus_rule_crystal_validate` payload.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RuleCrystalParsed {
    pub source_text: String,
    pub canonical_translation: String,
    pub forbidden_variants: Vec<String>,
}

/// One rule-crystal validation report; the serde projection is the
/// `hieronymus_rule_crystal_validate` payload (the Python dict shape).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RuleCrystalValidation {
    pub crystal_id: i64,
    pub valid: bool,
    pub enforceable: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub parsed_rule: Option<RuleCrystalParsed>,
}

/// Parameters for [`CrystalStore::add_crystal`]; text and crystal_type are
/// required, everything else defaults like the Python keyword arguments.
#[derive(Debug, Clone)]
pub struct NewCrystal {
    pub crystal_type: String,
    pub text: String,
    pub title: String,
    pub strength: f64,
    pub confidence: f64,
    pub source_credibility: String,
    pub rule_intent: String,
    pub malformed_penalty: f64,
    pub supersedes_crystal_id: Option<i64>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub language_tags: Vec<String>,
    pub soft_origin: String,
    pub is_inferred: bool,
    pub concept_ids: Vec<i64>,
    pub status: String,
    pub source_memory_ids: Vec<i64>,
}

impl Default for NewCrystal {
    fn default() -> Self {
        Self {
            crystal_type: String::new(),
            text: String::new(),
            title: String::new(),
            strength: 0.5,
            confidence: 0.5,
            source_credibility: "observation".to_string(),
            rule_intent: String::new(),
            malformed_penalty: 0.0,
            supersedes_crystal_id: None,
            story_scopes: Vec::new(),
            semantic_tags: Vec::new(),
            language_tags: Vec::new(),
            soft_origin: String::new(),
            is_inferred: false,
            concept_ids: Vec::new(),
            status: "active".to_string(),
            source_memory_ids: Vec::new(),
        }
    }
}

impl NewCrystal {
    pub fn new(crystal_type: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            crystal_type: crystal_type.into(),
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn strength(mut self, strength: f64) -> Self {
        self.strength = strength;
        self
    }

    pub fn confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn source_credibility(mut self, source_credibility: impl Into<String>) -> Self {
        self.source_credibility = source_credibility.into();
        self
    }

    pub fn rule_intent(mut self, rule_intent: impl Into<String>) -> Self {
        self.rule_intent = rule_intent.into();
        self
    }
}

/// Crystal store: long-term advisory memory over the data-root database.
pub struct CrystalStore {
    config: HieronymusConfig,
}

impl CrystalStore {
    pub fn open(config: &HieronymusConfig) -> Result<Self, CrystalError> {
        open_migrated(&config.database_path())?;
        Ok(Self {
            config: config.clone(),
        })
    }

    pub fn config(&self) -> &HieronymusConfig {
        &self.config
    }

    /// Insert a crystal bound to the context's series scope, with FTS row,
    /// typed side tables, concept links, and source-memory links.
    pub fn add_crystal(
        &self,
        context: &TranslationContext,
        crystal_type: &str,
        new: &NewCrystal,
    ) -> Result<i64, CrystalError> {
        validate_crystal_type(crystal_type)?;
        validate_status(&new.status)?;
        if new.text.trim().is_empty() {
            return Err(CrystalError::EmptyText);
        }

        let clamped_strength = clamp_score(new.strength);
        let clamped_confidence = clamp_score(new.confidence);
        let clean_story_scopes = clean_text_tuple(new.story_scopes.iter().map(String::as_str));
        let clean_semantic_tags = clean_text_tuple(new.semantic_tags.iter().map(String::as_str));
        let clean_language_tags = clean_text_tuple(new.language_tags.iter().map(String::as_str));
        let clean_concept_ids = clean_int_tuple(new.concept_ids.iter().copied());
        let soft_origin = new.soft_origin.trim().to_string();
        let legacy_tags: Vec<String> = if clean_semantic_tags.is_empty() {
            context.tags.clone()
        } else {
            clean_semantic_tags.clone()
        };
        let tags_json = serde_json::to_string(&legacy_tags)?;
        let now = now_iso8601();

        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "insert into crystals(
               crystal_type, text, title, scope_type, scope_key, series_slug,
               source_language, target_language, tags_json, strength, confidence,
               source_credibility, rule_intent, soft_origin, is_inferred,
               malformed_penalty, supersedes_crystal_id, status, created_at, updated_at
             )
             values (?1, ?2, ?3, 'series', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19)",
            rusqlite::params![
                crystal_type,
                new.text,
                new.title,
                context.scope_key(),
                context.series_slug,
                context.source_language,
                context.target_language,
                tags_json,
                clamped_strength,
                clamped_confidence,
                new.source_credibility,
                new.rule_intent,
                soft_origin,
                new.is_inferred as i64,
                new.malformed_penalty,
                new.supersedes_crystal_id,
                new.status,
                now,
                now,
            ],
        )?;
        let crystal_id = transaction.last_insert_rowid();
        transaction.execute(
            "insert into crystals_fts(rowid, title, text) values (?1, ?2, ?3)",
            rusqlite::params![crystal_id, new.title, new.text],
        )?;
        for story_scope in &clean_story_scopes {
            transaction.execute(
                "insert into crystal_story_scopes(crystal_id, scope, confidence, created_at)
                 values (?1, ?2, ?3, ?4)",
                rusqlite::params![crystal_id, story_scope, clamped_confidence, now],
            )?;
        }
        for semantic_tag in &clean_semantic_tags {
            transaction.execute(
                "insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
                 values (?1, ?2, ?3, ?4)",
                rusqlite::params![crystal_id, semantic_tag, clamped_confidence, now],
            )?;
        }
        for language_tag in &clean_language_tags {
            transaction.execute(
                "insert into crystal_language_tags(crystal_id, language_tag)
                 values (?1, ?2)",
                rusqlite::params![crystal_id, language_tag],
            )?;
        }
        for concept_id in &clean_concept_ids {
            transaction.execute(
                "insert into crystal_concepts(
                   crystal_id, concept_id, link_type, confidence, created_at
                 )
                 values (?1, ?2, 'mentions', ?3, ?4)",
                rusqlite::params![crystal_id, concept_id, clamped_confidence, now],
            )?;
        }
        for memory_id in &new.source_memory_ids {
            transaction.execute(
                "insert into crystal_sources(crystal_id, short_term_memory_id)
                 values (?1, ?2)",
                rusqlite::params![crystal_id, memory_id],
            )?;
        }
        transaction.commit()?;
        Ok(crystal_id)
    }

    pub fn get(&self, crystal_id: i64) -> Result<CrystalRecord, CrystalError> {
        let connection = self.connection()?;
        let record = hydrate_crystal(&connection, crystal_id)?;
        Ok(record)
    }

    /// Rule-type crystals newest-first with optional status and series filter.
    pub fn list_rule_crystals(
        &self,
        status: Option<&str>,
        series_slug: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CrystalRecord>, CrystalError> {
        if limit == 0 {
            return Err(CrystalError::LimitTooSmall);
        }
        if let Some(status) = status {
            validate_status(status)?;
        }
        let bounded_limit = limit.min(MAX_SEARCH_LIMIT) as i64;
        let connection = self.connection()?;
        let ids: Vec<i64> = {
            let mut statement = connection.prepare(
                "select id from crystals where crystal_type = 'rule'
                 and (?1 is null or status = ?1)
                 and (?2 is null or series_slug = ?2)
                 order by id desc limit ?3",
            )?;
            let rows = statement.query_map(
                rusqlite::params![status, series_slug, bounded_limit],
                |row| row.get(0),
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            records.push(hydrate_crystal(&connection, id)?);
        }
        Ok(records)
    }

    /// Archive a rule crystal; non-rule crystals are rejected. Standalone
    /// maintenance entry (Python parity); the audited rule lifecycle uses
    /// `archive_rule_crystal_in_transaction` so authority and projection
    /// archive in ONE transaction.
    pub fn archive_rule_crystal(&self, crystal_id: i64) -> Result<CrystalRecord, CrystalError> {
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        archive_rule_crystal_in_transaction(&transaction, crystal_id, &now)?;
        transaction.commit()?;
        self.get(crystal_id)
    }

    /// Validate rule-crystal shape and deterministic enforceability (Python
    /// `CrystalStore.validate_rule_crystal`): the canonical rule sentence
    /// must round-trip, and an active, concept-linked crystal above both
    /// deterministic thresholds is enforceable. Read-only — the projection
    /// is advisory; enforcement reads the structured authority.
    pub fn validate_rule_crystal(
        &self,
        crystal_id: i64,
    ) -> Result<RuleCrystalValidation, CrystalError> {
        let crystal = self.get(crystal_id)?;
        let connection = self.connection()?;
        let active_concept_ids: Vec<i64> = {
            let mut statement = connection.prepare(
                "select distinct cc.concept_id
                 from crystal_concepts cc
                 join concepts c on c.id = cc.concept_id
                 where cc.crystal_id = ?1
                   and c.status not in ('archived', 'merged')
                 order by cc.concept_id",
            )?;
            let rows = statement.query_map([crystal_id], |row| row.get::<_, i64>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        let mut errors: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut parsed_rule: Option<RuleCrystalParsed> = None;

        if crystal.crystal_type != "rule" {
            errors.push("crystal_type must be 'rule'".to_string());
        }
        match crate::terminology::parse_rule_crystal(&crystal.text) {
            None => errors.push(
                "rule text must match '<source> is translated as <target>[, not <forbidden>].'"
                    .to_string(),
            ),
            Some((source_text, canonical_translation, forbidden_variants)) => {
                parsed_rule = Some(RuleCrystalParsed {
                    source_text,
                    canonical_translation,
                    forbidden_variants,
                });
            }
        }

        if crystal.status != "active" {
            warnings.push("rule crystal is not active".to_string());
        }
        if crystal.confidence < DETERMINISTIC_RULE_CONFIDENCE_THRESHOLD {
            warnings.push("confidence is below deterministic validation threshold".to_string());
        }
        if crystal.strength < DETERMINISTIC_RULE_STRENGTH_THRESHOLD {
            warnings.push("strength is below deterministic validation threshold".to_string());
        }
        if crystal.concept_ids.is_empty() {
            warnings.push("rule crystal is not linked to a concept".to_string());
        } else if active_concept_ids.is_empty() {
            warnings.push("rule crystal is not linked to an active concept".to_string());
        }

        let valid = errors.is_empty();
        let enforceable = valid
            && crystal.status == "active"
            && crystal.confidence >= DETERMINISTIC_RULE_CONFIDENCE_THRESHOLD
            && crystal.strength >= DETERMINISTIC_RULE_STRENGTH_THRESHOLD
            && !active_concept_ids.is_empty();
        Ok(RuleCrystalValidation {
            crystal_id: crystal.id,
            valid,
            enforceable,
            errors,
            warnings,
            parsed_rule,
        })
    }

    pub fn set_story_scopes(
        &self,
        crystal_id: i64,
        story_scopes: &[String],
        confidence: f64,
    ) -> Result<CrystalRecord, CrystalError> {
        hydrate_crystal(&self.connection()?, crystal_id)?;
        self.set_crystal_side_table(
            crystal_id,
            "crystal_story_scopes",
            "scope",
            story_scopes,
            confidence,
        )?;
        self.get(crystal_id)
    }

    pub fn set_semantic_tags(
        &self,
        crystal_id: i64,
        semantic_tags: &[String],
        confidence: f64,
    ) -> Result<CrystalRecord, CrystalError> {
        hydrate_crystal(&self.connection()?, crystal_id)?;
        self.set_crystal_side_table(
            crystal_id,
            "crystal_semantic_tags",
            "tag",
            semantic_tags,
            confidence,
        )?;
        self.get(crystal_id)
    }

    fn set_crystal_side_table(
        &self,
        crystal_id: i64,
        table: &str,
        column: &str,
        values: &[String],
        confidence: f64,
    ) -> Result<(), CrystalError> {
        let clean = clean_text_tuple(values.iter().map(String::as_str));
        let clamped = clamp_score(confidence);
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            &format!("delete from {table} where crystal_id = ?1"),
            [crystal_id],
        )?;
        for value in &clean {
            transaction.execute(
                &format!(
                    "insert into {table}(crystal_id, {column}, confidence, created_at)
                     values (?1, ?2, ?3, ?4)"
                ),
                rusqlite::params![crystal_id, value, clamped, now],
            )?;
        }
        transaction.execute(
            "update crystals set updated_at = ?1 where id = ?2",
            rusqlite::params![now, crystal_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Bounded maintenance ordering: low confidence first, skipping active
    /// rule crystals (deterministic authority, ADR 0011).
    pub fn low_confidence_first(
        &self,
        crystal_ids: &[i64],
        limit: usize,
    ) -> Result<Vec<i64>, CrystalError> {
        if limit == 0 || crystal_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut unique = crystal_ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        let connection = self.connection()?;
        let ids: Vec<i64> = {
            let mut statement = connection.prepare(
                "select id from crystals
                 where id in (select value from json_each(?1))
                   and not (crystal_type = 'rule' and status = 'active')
                 order by confidence asc, strength asc, id asc
                 limit ?2",
            )?;
            let rows = statement.query_map(
                rusqlite::params![serde_json::to_string(&unique)?, limit as i64],
                |row| row.get(0),
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        Ok(ids)
    }

    /// Series/language-bounded FTS search with weighted scoring: exact order
    /// (score desc, id) and active/candidate statuses only.
    pub fn search_scored(
        &self,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(CrystalRecord, f64)>, CrystalError> {
        if limit == 0 {
            return Err(CrystalError::LimitTooSmall);
        }
        let expression = search_expression(query);
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let bounded_limit = limit.min(MAX_SEARCH_LIMIT) as i64;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select crystals.id, bm25(crystals_fts) as raw_bm25,
                    crystals.strength, crystals.confidence, crystals.scope_type
             from crystals_fts
             join crystals on crystals.id = crystals_fts.rowid
             where crystals_fts match ?1
               and crystals.status in ('active', 'candidate')
               and (
                 (crystals.scope_type = 'series' and crystals.scope_key = ?2)
                 or crystals.scope_type = 'global'
               )
               and (crystals.source_language = ?3 or crystals.source_language = '')
               and (crystals.target_language = ?4 or crystals.target_language = '')
             limit ?5",
        )?;
        let scored: Vec<(i64, f64)> = statement
            .query_map(
                rusqlite::params![
                    expression,
                    context.scope_key(),
                    context.source_language,
                    context.target_language,
                    bounded_limit
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;

        let mut scored_records: Vec<(CrystalRecord, f64)> = scored
            .into_iter()
            .map(|(id, raw_bm25)| {
                let record = hydrate_crystal(&connection, id)?;
                let score = weighted_search_score(
                    raw_bm25,
                    record.strength,
                    record.confidence,
                    &record.scope_type,
                );
                Ok((record, score))
            })
            .collect::<Result<Vec<_>, CrystalError>>()?;
        scored_records.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(left.0.id.cmp(&right.0.id))
        });
        Ok(scored_records)
    }

    pub fn search(
        &self,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CrystalRecord>, CrystalError> {
        Ok(self
            .search_scored(context, query, limit)?
            .into_iter()
            .map(|(record, _score)| record)
            .collect())
    }

    /// Legacy fallback search (Python `MemoryStore._fallback_search`, the
    /// long-term half): plain bm25 FTS over `active` crystals with
    /// series/global scope and language filters — no graded boosts, no
    /// candidate-status crystals.
    pub fn search_active(
        &self,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CrystalRecord>, CrystalError> {
        if limit == 0 {
            return Err(CrystalError::LimitTooSmall);
        }
        let expression = search_expression(query);
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let bounded_limit = limit.min(MAX_SEARCH_LIMIT) as i64;
        let connection = self.connection()?;
        let ids: Vec<i64> = {
            let mut statement = connection.prepare(
                "select crystals.id
                 from crystals_fts
                 join crystals on crystals.id = crystals_fts.rowid
                 where crystals_fts match ?1
                   and crystals.status = 'active'
                   and (
                     (crystals.scope_type = 'series' and crystals.scope_key = ?2)
                     or crystals.scope_type = 'global'
                   )
                   and (crystals.source_language = ?3 or crystals.source_language = '')
                   and (crystals.target_language = ?4 or crystals.target_language = '')
                 order by bm25(crystals_fts), crystals.id
                 limit ?5",
            )?;
            let rows = statement.query_map(
                rusqlite::params![
                    expression,
                    context.scope_key(),
                    context.source_language,
                    context.target_language,
                    bounded_limit
                ],
                |row| row.get(0),
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            records.push(hydrate_crystal(&connection, id)?);
        }
        Ok(records)
    }

    /// Bounded listing of active/candidate crystals for metadata-only recall
    /// candidates, newest first.
    pub fn list_all_candidates(&self, limit: usize) -> Result<Vec<CrystalRecord>, CrystalError> {
        let connection = self.connection()?;
        let ids: Vec<i64> = {
            let mut statement = connection.prepare(
                "select id from crystals
                 where status in ('active', 'candidate')
                 order by id desc limit ?1",
            )?;
            let rows = statement.query_map([limit as i64], |row| row.get(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            records.push(hydrate_crystal(&connection, id)?);
        }
        Ok(records)
    }

    /// Supersede an old crystal with a new same-shape one: the old record
    /// becomes `superseded`, the new one points at it, and a memory event is
    /// recorded for the audit trail. Active rule crystals are rejected in
    /// either position (ADR 0011).
    pub fn supersede(
        &self,
        old_crystal_id: i64,
        new_crystal_id: i64,
        reason: &str,
        cycle_id: i64,
    ) -> Result<(), CrystalError> {
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        supersede_in_transaction(
            &transaction,
            old_crystal_id,
            new_crystal_id,
            reason,
            cycle_id,
            &now,
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn connection(&self) -> Result<Connection, CrystalError> {
        let path: std::path::PathBuf = self.config.database_path();
        Ok(open_migrated(Path::new(&path))?)
    }
}

struct CrystalRow {
    status: String,
    series_slug: String,
    source_language: String,
    target_language: String,
    crystal_type: String,
    scope_type: String,
    scope_key: String,
}

fn crystal_row(connection: &Connection, crystal_id: i64) -> Result<CrystalRow, CrystalError> {
    connection
        .query_row(
            "select status, series_slug, source_language, target_language,
                    crystal_type, scope_type, scope_key
             from crystals where id = ?1",
            [crystal_id],
            |row| {
                Ok(CrystalRow {
                    status: row.get(0)?,
                    series_slug: row.get(1)?,
                    source_language: row.get(2)?,
                    target_language: row.get(3)?,
                    crystal_type: row.get(4)?,
                    scope_type: row.get(5)?,
                    scope_key: row.get(6)?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => CrystalError::UnknownCrystal(crystal_id),
            other => other.into(),
        })
}

/// The ADR 0011 / slice-5 protection predicate: active structured rule
/// authority never decays passively, never supersedes, never combines.
pub(crate) fn is_active_rule(crystal_type: &str, status: &str) -> bool {
    crystal_type == "rule" && status == "active"
}

fn validate_supersede_rows(old_row: &CrystalRow, new_row: &CrystalRow) -> Result<(), CrystalError> {
    for row in [old_row, new_row] {
        if !["active", "candidate"].contains(&row.status.as_str()) {
            return Err(CrystalError::SupersedeNotActive);
        }
    }
    for (column, old_value, new_value) in [
        ("series_slug", &old_row.series_slug, &new_row.series_slug),
        (
            "source_language",
            &old_row.source_language,
            &new_row.source_language,
        ),
        (
            "target_language",
            &old_row.target_language,
            &new_row.target_language,
        ),
        ("crystal_type", &old_row.crystal_type, &new_row.crystal_type),
        ("scope_type", &old_row.scope_type, &new_row.scope_type),
        ("scope_key", &old_row.scope_key, &new_row.scope_key),
    ] {
        if old_value != new_value {
            return Err(CrystalError::SupersedeMismatch(column.to_string()));
        }
    }
    Ok(())
}

/// The transaction-aware supersede primitive (Python
/// `_supersede_with_connection`): the dream graph applies provider supersede
/// actions through this inside the persistence transaction, so a failing
/// action rolls the whole batch back. Shape and status validation stay
/// fail-loud, and the ADR 0011 guard is enforced here rather than only at
/// the dream call site: an active rule crystal can never be a supersede
/// source or target through this primitive (only an authenticated user may
/// replace approved authority).
pub(crate) fn supersede_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    old_crystal_id: i64,
    new_crystal_id: i64,
    reason: &str,
    cycle_id: i64,
    now: &str,
) -> Result<(), CrystalError> {
    if old_crystal_id == new_crystal_id {
        return Err(CrystalError::SelfSupersede);
    }
    let old_row = crystal_row(transaction, old_crystal_id)?;
    let new_row = crystal_row(transaction, new_crystal_id)?;
    // ADR 0011: active rule authority never moves through a supersede, in
    // either direction, so the primitive refuses before any row mutates.
    if is_active_rule(&old_row.crystal_type, &old_row.status) {
        return Err(CrystalError::SupersedeActiveRule(old_crystal_id));
    }
    if is_active_rule(&new_row.crystal_type, &new_row.status) {
        return Err(CrystalError::SupersedeActiveRule(new_crystal_id));
    }
    validate_supersede_rows(&old_row, &new_row)?;
    transaction.execute(
        "update crystals set status = 'superseded', updated_at = ?1 where id = ?2",
        rusqlite::params![now, old_crystal_id],
    )?;
    transaction.execute(
        "update crystals set supersedes_crystal_id = ?1, updated_at = ?2 where id = ?3",
        rusqlite::params![old_crystal_id, now, new_crystal_id],
    )?;
    transaction.execute(
        "insert into memory_events(
           crystal_id, session_id, event_type, source_role, evidence,
           strength_delta, confidence_delta, applied, cycle_id, created_at
         )
         values (?1, null, 'supersede', 'system', ?2, 0, 0, 1, ?3, ?4)",
        rusqlite::params![old_crystal_id, reason, cycle_id, now],
    )?;
    Ok(())
}

fn validate_crystal_type(crystal_type: &str) -> Result<(), CrystalError> {
    if ALLOWED_CRYSTAL_TYPES.contains(&crystal_type) {
        Ok(())
    } else {
        Err(CrystalError::UnknownCrystalType(crystal_type.to_string()))
    }
}

/// Archive one rule crystal inside an existing transaction. The audited rule
/// lifecycle (`Termbase::apply_action`) shares this primitive so a linked
/// rule's authority row and its advisory projection always transition in ONE
/// SQLite transaction — a projection failure rolls the authority back, and a
/// committed archive never leaves an active projection behind. Non-rule
/// crystals and unknown ids are rejected (fail loud, never guess).
pub(crate) fn archive_rule_crystal_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    crystal_id: i64,
    now: &str,
) -> Result<(), CrystalError> {
    let crystal_type: String = transaction
        .query_row(
            "select crystal_type from crystals where id = ?1",
            [crystal_id],
            |row| row.get(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => CrystalError::UnknownCrystal(crystal_id),
            other => other.into(),
        })?;
    if crystal_type != "rule" {
        return Err(CrystalError::Invalid(
            "crystal is not a rule crystal".to_string(),
        ));
    }
    let updated = transaction.execute(
        "update crystals set status = 'archived', updated_at = ?1 where id = ?2",
        rusqlite::params![now, crystal_id],
    )?;
    if updated == 0 {
        return Err(CrystalError::UnknownCrystal(crystal_id));
    }
    Ok(())
}

fn validate_status(status: &str) -> Result<(), CrystalError> {
    if ALLOWED_STATUSES.contains(&status) {
        Ok(())
    } else {
        Err(CrystalError::UnknownStatus(status.to_string()))
    }
}

fn clamp_score(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn clean_text_tuple<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut cleaned: Vec<String> = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    cleaned.sort();
    cleaned.dedup();
    cleaned
}

fn clean_int_tuple(values: impl Iterator<Item = i64>) -> Vec<i64> {
    let mut cleaned: Vec<i64> = values.collect();
    cleaned.sort_unstable();
    cleaned.dedup();
    cleaned
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339()
}

fn hydrate_crystal(
    connection: &Connection,
    crystal_id: i64,
) -> Result<CrystalRecord, CrystalError> {
    let record = connection
        .query_row(
            "select id, crystal_type, text, title, scope_type, scope_key, series_slug,
                    source_language, target_language, strength, confidence,
                    source_credibility, rule_intent, malformed_penalty,
                    supersedes_crystal_id, status, soft_origin, is_inferred
             from crystals where id = ?1",
            [crystal_id],
            |row| {
                Ok(CrystalRecord {
                    id: row.get(0)?,
                    crystal_type: row.get(1)?,
                    text: row.get(2)?,
                    title: row.get(3)?,
                    scope_type: row.get(4)?,
                    scope_key: row.get(5)?,
                    series_slug: row.get(6)?,
                    source_language: row.get(7)?,
                    target_language: row.get(8)?,
                    strength: row.get(9)?,
                    confidence: row.get(10)?,
                    source_credibility: row.get(11)?,
                    rule_intent: row.get(12)?,
                    malformed_penalty: row.get(13)?,
                    supersedes_crystal_id: row.get(14)?,
                    status: row.get(15)?,
                    language_tags: Vec::new(),
                    story_scopes: Vec::new(),
                    semantic_tags: Vec::new(),
                    soft_origin: row.get::<_, Option<String>>(16)?.unwrap_or_default(),
                    is_inferred: row.get::<_, i64>(17)? != 0,
                    concept_ids: Vec::new(),
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => CrystalError::UnknownCrystal(crystal_id),
            other => other.into(),
        })?;
    hydrate_side_tables(connection, record)
}

fn hydrate_side_tables(
    connection: &Connection,
    mut record: CrystalRecord,
) -> Result<CrystalRecord, CrystalError> {
    record.language_tags = text_map(
        connection,
        "crystal_language_tags",
        "language_tag",
        record.id,
    )?;
    record.story_scopes = text_map(connection, "crystal_story_scopes", "scope", record.id)?;
    record.semantic_tags = text_map(connection, "crystal_semantic_tags", "tag", record.id)?;
    let mut statement = connection.prepare(
        "select distinct concept_id from crystal_concepts
         where crystal_id = ?1 order by concept_id",
    )?;
    let rows = statement.query_map([record.id], |row| row.get::<_, i64>(0))?;
    record.concept_ids = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(record)
}

fn text_map(
    connection: &Connection,
    table: &str,
    value_column: &str,
    crystal_id: i64,
) -> Result<Vec<String>, CrystalError> {
    let mut statement = connection.prepare(&format!(
        "select {value_column} from {table} where crystal_id = ?1 order by {value_column}"
    ))?;
    let rows = statement.query_map([crystal_id], |row| row.get::<_, String>(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
