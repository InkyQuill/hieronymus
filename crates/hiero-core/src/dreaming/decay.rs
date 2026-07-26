use std::collections::BTreeSet;

use async_trait::async_trait;
use chrono::Utc;
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqlitePool};

use crate::{
    db::CrystalRecord,
    domain::{ScoreDelta, apply_score_delta},
};

use super::{DreamPhase, DreamPhaseError};

pub const STRENGTH_DECAY_PER_CYCLE: f64 = 0.03;
pub const CONFIDENCE_DECAY_AFTER_STRENGTH_BELOW: f64 = 0.20;
pub const CONFIDENCE_DECAY_PER_CYCLE: f64 = 0.01;
pub const DECAY_BATCH_SIZE: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecayScope {
    pub after_id: i64,
    pub current_cycle: i64,
    pub stale_before_cycle: i64,
    pub recalled_ids: Vec<i64>,
    pub linked_ids: Vec<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DecayManager;

#[derive(Debug, FromRow)]
struct DecayCandidate {
    id: i64,
}

impl DecayManager {
    pub async fn explain(
        pool: &SqlitePool,
        scope: &DecayScope,
    ) -> Result<Vec<String>, DreamPhaseError> {
        let protected = protected_ids(scope);
        let mut query = decay_query(scope, &protected, true);
        query
            .build()
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| row.try_get("detail").map_err(DreamPhaseError::from))
            .collect()
    }

    async fn select(pool: &SqlitePool, scope: &DecayScope) -> Result<Vec<i64>, DreamPhaseError> {
        let protected = protected_ids(scope);
        Ok(decay_query(scope, &protected, false)
            .build_query_as::<DecayCandidate>()
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|candidate| candidate.id)
            .collect())
    }
}

#[must_use]
pub fn decay_delta(crystal: &CrystalRecord) -> ScoreDelta {
    let post_decay_strength = (crystal.strength - STRENGTH_DECAY_PER_CYCLE).clamp(0.0, 1.0);
    ScoreDelta {
        strength: -STRENGTH_DECAY_PER_CYCLE,
        confidence: if post_decay_strength < CONFIDENCE_DECAY_AFTER_STRENGTH_BELOW {
            -CONFIDENCE_DECAY_PER_CYCLE
        } else {
            0.0
        },
    }
}

#[async_trait]
impl DreamPhase for DecayManager {
    type Input = DecayScope;
    type Output = Vec<i64>;

    async fn run(
        &self,
        pool: &SqlitePool,
        input: Self::Input,
    ) -> Result<Self::Output, DreamPhaseError> {
        let candidates = Self::select(pool, &input).await?;
        let protected = protected_ids(&input);
        let mut decayed = Vec::with_capacity(candidates.len());
        for id in candidates {
            let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
            let crystal: CrystalRecord = sqlx::query_as("SELECT * FROM crystals WHERE id=?")
                .bind(id)
                .fetch_one(&mut *transaction)
                .await?;
            if !is_eligible(&crystal, &input, &protected) {
                transaction.commit().await?;
                continue;
            }
            let delta = decay_delta(&crystal);
            let (strength, confidence, status) = apply_score_delta(&crystal, delta);
            let now = Utc::now();
            sqlx::query(
                "UPDATE crystals SET strength=?,confidence=?,status=?,updated_at=? WHERE id=?",
            )
            .bind(strength)
            .bind(confidence)
            .bind(&status)
            .bind(now)
            .bind(id)
            .execute(&mut *transaction)
            .await?;
            let strength_delta = strength - crystal.strength;
            let confidence_delta = confidence - crystal.confidence;
            if strength_delta != 0.0 || confidence_delta != 0.0 {
                sqlx::query("INSERT INTO memory_events(crystal_id,session_id,event_type,source_role,evidence,strength_delta,confidence_delta,applied,cycle_id,created_at) VALUES (?,NULL,'cycle_decay','system','cycle decay',?,?,1,?,?)")
                    .bind(id)
                    .bind(strength_delta)
                    .bind(confidence_delta)
                    .bind(input.current_cycle)
                    .bind(now)
                    .execute(&mut *transaction)
                    .await?;
                decayed.push(id);
            }
            transaction.commit().await?;
        }
        Ok(decayed)
    }
}

fn protected_ids(scope: &DecayScope) -> BTreeSet<i64> {
    scope
        .recalled_ids
        .iter()
        .chain(&scope.linked_ids)
        .copied()
        .collect()
}

fn effective_limit(limit: usize) -> i64 {
    if limit == 0 {
        DECAY_BATCH_SIZE as i64
    } else {
        limit.min(DECAY_BATCH_SIZE) as i64
    }
}

fn decay_query(
    scope: &DecayScope,
    protected: &BTreeSet<i64>,
    explain: bool,
) -> QueryBuilder<Sqlite> {
    let mut query = QueryBuilder::new(if explain {
        "EXPLAIN QUERY PLAN SELECT id FROM crystals WHERE status IN ('active','candidate') AND id > "
    } else {
        "SELECT id FROM crystals WHERE status IN ('active','candidate') AND id > "
    });
    query
        .push_bind(scope.after_id)
        .push(" AND created_cycle != ")
        .push_bind(scope.current_cycle)
        .push(" AND coalesce(last_activated_cycle,-1) != ")
        .push_bind(scope.current_cycle)
        .push(" AND coalesce(last_reinforced_cycle,0) < ")
        .push_bind(scope.stale_before_cycle);
    if !protected.is_empty() {
        query.push(" AND id NOT IN (");
        let mut separated = query.separated(",");
        for id in protected {
            separated.push_bind(*id);
        }
        separated.push_unseparated(")");
    }
    query
        .push(" ORDER BY id LIMIT ")
        .push_bind(effective_limit(scope.limit));
    query
}

fn is_eligible(crystal: &CrystalRecord, scope: &DecayScope, protected: &BTreeSet<i64>) -> bool {
    matches!(crystal.status.as_str(), "active" | "candidate")
        && crystal.id > scope.after_id
        && crystal.created_cycle != scope.current_cycle
        && crystal.last_activated_cycle != Some(scope.current_cycle)
        && crystal.last_reinforced_cycle.unwrap_or(0) < scope.stale_before_cycle
        && !protected.contains(&crystal.id)
}
