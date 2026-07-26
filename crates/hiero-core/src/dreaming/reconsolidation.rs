use async_trait::async_trait;
use chrono::Utc;
use icu_casemap::CaseMapper;
use sqlx::{Sqlite, SqlitePool, Transaction};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    db::CrystalRecord,
    domain::{Crystal, ScoreDelta, ShortTermMemory, apply_score_delta},
};

use super::{DreamPhase, DreamPhaseError};

pub const COMBINATION_TEXT_SIMILARITY_THRESHOLD: f64 = 0.80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconsolidationDecision {
    ReinforceInPlace,
    Supersede,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconsolidationOutcome {
    ReinforcedInPlace {
        crystal_id: i64,
    },
    Superseded {
        old_crystal_id: i64,
        new_crystal_id: i64,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct Reconsolidator {
    threshold: f64,
    current_cycle: i64,
}

impl Reconsolidator {
    #[must_use]
    pub const fn new(threshold: f64, current_cycle: i64) -> Self {
        Self {
            threshold,
            current_cycle,
        }
    }
}

#[async_trait]
impl DreamPhase for Reconsolidator {
    type Input = Vec<(ShortTermMemory, Crystal)>;
    type Output = Vec<ReconsolidationOutcome>;

    async fn run(
        &self,
        pool: &SqlitePool,
        input: Self::Input,
    ) -> Result<Self::Output, DreamPhaseError> {
        let mut outcomes = Vec::with_capacity(input.len());
        for (memory, source) in input {
            let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
            let (working_text, kind, source_crystal_id, archived_at): (
                String,
                String,
                Option<i64>,
                Option<String>,
            ) = sqlx::query_as(
                "SELECT text,kind,source_crystal_id,archived_at FROM short_term_memories WHERE id=?",
            )
            .bind(memory.id)
            .fetch_one(&mut *transaction)
            .await?;
            if source_crystal_id != Some(source.id) {
                return Err(DreamPhaseError::InvalidInput(
                    "working copy does not reference its supplied source crystal",
                ));
            }
            if kind != "working_copy" {
                return Err(DreamPhaseError::InvalidInput(
                    "source-linked memory is not a working copy",
                ));
            }
            let source = read_crystal(&mut transaction, source.id).await?;
            if archived_at.is_some() {
                let outcome =
                    completed_outcome(&mut transaction, &source, memory.id, self.current_cycle)
                        .await?;
                transaction.commit().await?;
                outcomes.push(outcome);
                continue;
            }
            if !matches!(source.status.as_str(), "active" | "candidate") {
                return Err(DreamPhaseError::InvalidInput(
                    "working-copy source is no longer active or candidate",
                ));
            }
            let outcome =
                match reconsolidation_decision(&working_text, &source.text, self.threshold) {
                    ReconsolidationDecision::ReinforceInPlace => {
                        reinforce_in_place(
                            &mut transaction,
                            &source,
                            memory.id,
                            self.current_cycle,
                        )
                        .await?;
                        ReconsolidationOutcome::ReinforcedInPlace {
                            crystal_id: source.id,
                        }
                    }
                    ReconsolidationDecision::Supersede => {
                        let new_id = supersede(
                            &mut transaction,
                            &source,
                            &working_text,
                            memory.id,
                            self.current_cycle,
                        )
                        .await?;
                        ReconsolidationOutcome::Superseded {
                            old_crystal_id: source.id,
                            new_crystal_id: new_id,
                        }
                    }
                };
            transaction.commit().await?;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }
}

async fn completed_outcome(
    transaction: &mut Transaction<'static, Sqlite>,
    source: &CrystalRecord,
    memory_id: i64,
    current_cycle: i64,
) -> Result<ReconsolidationOutcome, DreamPhaseError> {
    let reinforced: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1
            FROM memory_events
            WHERE crystal_id=?
              AND event_type='reconsolidated_in_place'
              AND applied=1
              AND cycle_id=?
              AND evidence=?
        )",
    )
    .bind(source.id)
    .bind(current_cycle)
    .bind(working_copy_evidence(memory_id))
    .fetch_one(&mut **transaction)
    .await?;
    if reinforced {
        return Ok(ReconsolidationOutcome::ReinforcedInPlace {
            crystal_id: source.id,
        });
    }
    if source.status == "superseded" {
        let replacement_id: Option<i64> = sqlx::query_scalar(
            "SELECT crystals.id
             FROM crystals
             JOIN crystal_sources
               ON crystal_sources.crystal_id=crystals.id
             WHERE crystals.supersedes_crystal_id=?
               AND crystal_sources.short_term_memory_id=?
             ORDER BY crystals.id
             LIMIT 1",
        )
        .bind(source.id)
        .bind(memory_id)
        .fetch_optional(&mut **transaction)
        .await?;
        let Some(new_crystal_id) = replacement_id else {
            return Err(DreamPhaseError::InvalidInput(
                "archived working copy has no completed supersession",
            ));
        };
        return Ok(ReconsolidationOutcome::Superseded {
            old_crystal_id: source.id,
            new_crystal_id,
        });
    }
    Err(DreamPhaseError::InvalidInput(
        "archived working copy has no completed reconsolidation",
    ))
}

fn working_copy_evidence(memory_id: i64) -> String {
    format!("working_copy:{memory_id}")
}

#[must_use]
pub fn reconsolidation_decision(
    working_copy_text: &str,
    source_text: &str,
    threshold: f64,
) -> ReconsolidationDecision {
    if diff_ratio(working_copy_text, source_text) < threshold {
        ReconsolidationDecision::ReinforceInPlace
    } else {
        ReconsolidationDecision::Supersede
    }
}

#[must_use]
pub fn diff_ratio(left: &str, right: &str) -> f64 {
    let left = tokens(left);
    let right = tokens(right);
    let denominator = left.len().max(right.len());
    if denominator == 0 {
        return 0.0;
    }
    levenshtein(&left, &right) as f64 / denominator as f64
}

#[must_use]
pub fn text_similarity(left: &str, right: &str) -> f64 {
    1.0 - diff_ratio(left, right)
}

fn tokens(text: &str) -> Vec<String> {
    let mapper = CaseMapper::new();
    text.unicode_words()
        .map(|token| mapper.fold_string(token).into_owned())
        .collect()
}

fn levenshtein(left: &[String], right: &[String]) -> usize {
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_token) in left.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_token) in right.iter().enumerate() {
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + usize::from(left_token != right_token));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

async fn read_crystal(
    transaction: &mut Transaction<'static, Sqlite>,
    id: i64,
) -> Result<CrystalRecord, sqlx::Error> {
    sqlx::query_as("SELECT * FROM crystals WHERE id = ?")
        .bind(id)
        .fetch_one(&mut **transaction)
        .await
}

async fn reinforce_in_place(
    transaction: &mut Transaction<'static, Sqlite>,
    source: &CrystalRecord,
    memory_id: i64,
    current_cycle: i64,
) -> Result<(), sqlx::Error> {
    let delta = ScoreDelta {
        strength: 0.02,
        confidence: 0.0,
    };
    let (strength, confidence, status) = apply_score_delta(source, delta);
    let now = Utc::now();
    sqlx::query("UPDATE crystals SET strength=?,confidence=?,status=?,last_reinforced_cycle=?,updated_at=? WHERE id=?")
        .bind(strength)
        .bind(confidence)
        .bind(status)
        .bind(current_cycle)
        .bind(now)
        .bind(source.id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT INTO memory_events(crystal_id,session_id,event_type,source_role,evidence,strength_delta,confidence_delta,applied,cycle_id,created_at) VALUES (?,NULL,'reconsolidated_in_place','system',?,0.02,0.0,1,?,?)")
        .bind(source.id)
        .bind(working_copy_evidence(memory_id))
        .bind(current_cycle)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    archive_memory(transaction, memory_id, now).await
}

async fn supersede(
    transaction: &mut Transaction<'static, Sqlite>,
    source: &CrystalRecord,
    working_text: &str,
    memory_id: i64,
    current_cycle: i64,
) -> Result<i64, sqlx::Error> {
    let now = Utc::now();
    let inserted = sqlx::query(
        "INSERT INTO crystals(crystal_type,text,title,scope_type,scope_key,series_slug,source_language,target_language,tags_json,strength,confidence,source_credibility,rule_intent,soft_origin,is_inferred,malformed_penalty,supersedes_crystal_id,status,created_cycle,last_activated_cycle,last_reinforced_cycle,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,'active',?,NULL,NULL,?,?)",
    )
    .bind(&source.crystal_type)
    .bind(working_text)
    .bind(&source.title)
    .bind(&source.scope_type)
    .bind(&source.scope_key)
    .bind(&source.series_slug)
    .bind(&source.source_language)
    .bind(&source.target_language)
    .bind(&source.tags_json)
    .bind(source.strength)
    .bind(source.confidence)
    .bind(&source.source_credibility)
    .bind(&source.rule_intent)
    .bind(&source.soft_origin)
    .bind(source.is_inferred)
    .bind(source.malformed_penalty)
    .bind(source.id)
    .bind(current_cycle)
    .bind(now)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    let new_id = inserted.last_insert_rowid();
    sqlx::query("INSERT INTO crystal_sources(crystal_id,short_term_memory_id) VALUES (?,?)")
        .bind(new_id)
        .bind(memory_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT INTO crystal_language_tags(crystal_id,language_tag) SELECT ?,language_tag FROM crystal_language_tags WHERE crystal_id=?")
        .bind(new_id).bind(source.id).execute(&mut **transaction).await?;
    sqlx::query("INSERT INTO crystal_story_scopes(crystal_id,scope,confidence,created_at) SELECT ?,scope,confidence,created_at FROM crystal_story_scopes WHERE crystal_id=?")
        .bind(new_id).bind(source.id).execute(&mut **transaction).await?;
    sqlx::query("INSERT INTO crystal_semantic_tags(crystal_id,tag,confidence,created_at) SELECT ?,tag,confidence,created_at FROM crystal_semantic_tags WHERE crystal_id=?")
        .bind(new_id).bind(source.id).execute(&mut **transaction).await?;
    sqlx::query("INSERT INTO crystal_concepts(crystal_id,concept_id,link_type,confidence,created_at) SELECT ?,concept_id,link_type,confidence,created_at FROM crystal_concepts WHERE crystal_id=?")
        .bind(new_id)
        .bind(source.id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("UPDATE crystals SET status='superseded',updated_at=? WHERE id=? AND status IN ('active','candidate')")
        .bind(now)
        .bind(source.id)
        .execute(&mut **transaction)
        .await?;
    archive_memory(transaction, memory_id, now).await?;
    Ok(new_id)
}

async fn archive_memory(
    transaction: &mut Transaction<'static, Sqlite>,
    memory_id: i64,
    now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE short_term_memories SET archived_at=COALESCE(archived_at,?) WHERE id=?")
        .bind(now)
        .bind(memory_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
