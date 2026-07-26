use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use chrono::Utc;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::{
    db::{CrystalActivationRecord, CrystalRecord, MemoryEventRecord},
    domain::{PASSIVE_EVENT_DELTAS, ScoreDelta, apply_score_delta},
    values::SOURCE_CREDIBILITY_CONFIDENCE,
};

use super::{COMBINATION_TEXT_SIMILARITY_THRESHOLD, DreamPhase, DreamPhaseError, text_similarity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkOutcome {
    Strengthened { source_id: i64, target_id: i64 },
    Combined { survivor_id: i64, absorbed_id: i64 },
}

#[derive(Debug, Clone, Copy)]
pub struct LinkReinforcer {
    current_cycle: i64,
}

impl LinkReinforcer {
    #[must_use]
    pub const fn new(current_cycle: i64) -> Self {
        Self { current_cycle }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ReinforcementManager {
    current_cycle: i64,
}

impl ReinforcementManager {
    #[must_use]
    pub const fn new(current_cycle: i64) -> Self {
        Self { current_cycle }
    }
}

#[must_use]
pub fn useful_pairs(activations: &[CrystalActivationRecord]) -> Vec<(i64, i64)> {
    let mut by_session = BTreeMap::<i64, BTreeSet<i64>>::new();
    for activation in activations {
        if activation.outcome.as_deref() == Some("useful") {
            by_session
                .entry(activation.session_id)
                .or_default()
                .insert(activation.crystal_id);
        }
    }
    let mut pairs = BTreeSet::new();
    for ids in by_session.values() {
        let ids: Vec<i64> = ids.iter().copied().collect();
        for left in 0..ids.len() {
            for right in left + 1..ids.len() {
                pairs.insert((ids[left], ids[right]));
            }
        }
    }
    pairs.into_iter().collect()
}

#[must_use]
pub fn select_survivor(left: &CrystalRecord, right: &CrystalRecord) -> (i64, i64) {
    let left_weight = credibility(left);
    let right_weight = credibility(right);
    if left_weight > right_weight
        || (left_weight == right_weight
            && (left.strength > right.strength
                || (left.strength == right.strength && left.id < right.id)))
    {
        (left.id, right.id)
    } else {
        (right.id, left.id)
    }
}

#[async_trait]
impl DreamPhase for LinkReinforcer {
    type Input = Vec<CrystalActivationRecord>;
    type Output = Vec<LinkOutcome>;

    async fn run(
        &self,
        pool: &SqlitePool,
        input: Self::Input,
    ) -> Result<Self::Output, DreamPhaseError> {
        let mut combined = BTreeSet::new();
        let mut outcomes = Vec::new();
        for (source_id, target_id) in useful_pairs(&input) {
            let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
            let source = read_crystal(&mut transaction, source_id).await?;
            let target = read_crystal(&mut transaction, target_id).await?;
            if !eligible(&source) || !eligible(&target) {
                transaction.commit().await?;
                continue;
            }
            sqlx::query("INSERT OR IGNORE INTO crystal_links(source_crystal_id,target_crystal_id,link_type) VALUES (?,?,'related')")
                .bind(source_id)
                .bind(target_id)
                .execute(&mut *transaction)
                .await?;
            let shared_concept: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM crystal_concepts a JOIN crystal_concepts b ON b.concept_id=a.concept_id WHERE a.crystal_id=? AND b.crystal_id=?)")
                .bind(source_id)
                .bind(target_id)
                .fetch_one(&mut *transaction)
                .await?;
            let should_combine = shared_concept
                || text_similarity(&source.text, &target.text)
                    > COMBINATION_TEXT_SIMILARITY_THRESHOLD;
            let outcome = if should_combine
                && !combined.contains(&source_id)
                && !combined.contains(&target_id)
            {
                let (survivor_id, absorbed_id) = select_survivor(&source, &target);
                combine(
                    &mut transaction,
                    survivor_id,
                    absorbed_id,
                    self.current_cycle,
                )
                .await?;
                combined.insert(survivor_id);
                combined.insert(absorbed_id);
                LinkOutcome::Combined {
                    survivor_id,
                    absorbed_id,
                }
            } else {
                LinkOutcome::Strengthened {
                    source_id,
                    target_id,
                }
            };
            transaction.commit().await?;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }
}

#[async_trait]
impl DreamPhase for ReinforcementManager {
    type Input = Vec<MemoryEventRecord>;
    type Output = Vec<i64>;

    async fn run(
        &self,
        pool: &SqlitePool,
        input: Self::Input,
    ) -> Result<Self::Output, DreamPhaseError> {
        let mut updated = BTreeSet::new();
        for event in input {
            let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
            let stored: MemoryEventRecord =
                sqlx::query_as("SELECT * FROM memory_events WHERE id=?")
                    .bind(event.id)
                    .fetch_one(&mut *transaction)
                    .await?;
            if stored.applied {
                transaction.commit().await?;
                continue;
            }
            let Some(&(strength_delta, confidence_delta)) =
                PASSIVE_EVENT_DELTAS.get(stored.event_type.as_str())
            else {
                transaction.commit().await?;
                continue;
            };
            let Some(crystal_id) = stored.crystal_id else {
                transaction.commit().await?;
                continue;
            };
            let crystal = read_crystal(&mut transaction, crystal_id).await?;
            let delta = ScoreDelta {
                strength: strength_delta,
                confidence: confidence_delta,
            };
            let (strength, confidence, status) = apply_score_delta(&crystal, delta);
            let last_reinforced_cycle = if strength_delta > 0.0 {
                Some(self.current_cycle)
            } else {
                crystal.last_reinforced_cycle
            };
            sqlx::query("UPDATE crystals SET strength=?,confidence=?,status=?,last_reinforced_cycle=?,updated_at=? WHERE id=?")
                .bind(strength)
                .bind(confidence)
                .bind(status)
                .bind(last_reinforced_cycle)
                .bind(Utc::now())
                .bind(crystal_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("UPDATE memory_events SET applied=1,cycle_id=? WHERE id=? AND applied=0")
                .bind(self.current_cycle)
                .bind(event.id)
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            updated.insert(crystal_id);
        }
        Ok(updated.into_iter().collect())
    }
}

async fn read_crystal(
    transaction: &mut Transaction<'static, Sqlite>,
    id: i64,
) -> Result<CrystalRecord, sqlx::Error> {
    sqlx::query_as("SELECT * FROM crystals WHERE id=?")
        .bind(id)
        .fetch_one(&mut **transaction)
        .await
}

fn eligible(crystal: &CrystalRecord) -> bool {
    matches!(crystal.status.as_str(), "active" | "candidate")
}

fn credibility(crystal: &CrystalRecord) -> f64 {
    SOURCE_CREDIBILITY_CONFIDENCE
        .get(crystal.source_credibility.as_str())
        .copied()
        .unwrap_or(0.35)
}

async fn combine(
    transaction: &mut Transaction<'static, Sqlite>,
    survivor_id: i64,
    absorbed_id: i64,
    current_cycle: i64,
) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    sqlx::query("INSERT INTO crystal_concepts(crystal_id,concept_id,link_type,confidence,created_at) SELECT ?,concept_id,link_type,confidence,created_at FROM crystal_concepts WHERE crystal_id=? ON CONFLICT(crystal_id,concept_id,link_type) DO UPDATE SET confidence=max(crystal_concepts.confidence,excluded.confidence)")
        .bind(survivor_id)
        .bind(absorbed_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO crystal_links(source_crystal_id,target_crystal_id,link_type) SELECT ?,target_crystal_id,link_type FROM crystal_links WHERE source_crystal_id=? AND target_crystal_id NOT IN (?,?)")
        .bind(survivor_id)
        .bind(absorbed_id)
        .bind(survivor_id)
        .bind(absorbed_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO crystal_links(source_crystal_id,target_crystal_id,link_type) SELECT source_crystal_id,?,link_type FROM crystal_links WHERE target_crystal_id=? AND source_crystal_id NOT IN (?,?)")
        .bind(survivor_id)
        .bind(absorbed_id)
        .bind(survivor_id)
        .bind(absorbed_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("UPDATE crystals SET status='superseded',updated_at=? WHERE id=? AND status IN ('active','candidate')")
        .bind(now)
        .bind(absorbed_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT INTO memory_events(crystal_id,session_id,event_type,source_role,evidence,strength_delta,confidence_delta,applied,cycle_id,created_at) VALUES (?,NULL,'combined_into','system',?,0.0,0.0,1,?,?)")
        .bind(absorbed_id)
        .bind(survivor_id.to_string())
        .bind(current_cycle)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
