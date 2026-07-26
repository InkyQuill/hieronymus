use std::collections::BTreeMap;

use async_trait::async_trait;
use icu_casemap::CaseMapper;
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use crate::{db::ConceptRecord, domain::ConceptMergeProposalInput};

use super::{DreamPhase, DreamPhaseError};

const DUPLICATE_RATIONALE: &str = "Canonical name and scope are exact duplicates.";

#[derive(Debug, Clone, FromRow)]
struct ConsolidationKey {
    scope_type: String,
    scope_key: String,
    canonical_name_key: String,
}

#[derive(Debug, Clone, Copy)]
pub struct Consolidator {
    dream_run_id: i64,
    limit: usize,
}

impl Consolidator {
    #[must_use]
    pub const fn new(dream_run_id: i64, limit: usize) -> Self {
        Self {
            dream_run_id,
            limit,
        }
    }

    pub async fn scan(
        &self,
        pool: &SqlitePool,
    ) -> Result<Vec<ConceptMergeProposalInput>, DreamPhaseError> {
        if self.limit == 0 {
            return Ok(Vec::new());
        }
        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
        refresh_dirty_page(&mut transaction, self.limit).await?;

        let (mut key, mut source_after) = active_key(&mut transaction).await?;
        if key.is_none() {
            key = next_duplicate_key(&mut transaction).await?;
            source_after = 0;
            if key.is_none() {
                reset_key_cursor(&mut transaction).await?;
                key = next_duplicate_key(&mut transaction).await?;
            }
        }
        let Some(key) = key else {
            transaction.commit().await?;
            return Ok(Vec::new());
        };

        let target_id: Option<i64> = sqlx::query_scalar(
            "SELECT concept_id
             FROM concept_consolidation_keys
             WHERE scope_type=? AND scope_key=? AND canonical_name_key=?
             ORDER BY status_rank,confidence DESC,concept_id
             LIMIT 1",
        )
        .bind(&key.scope_type)
        .bind(&key.scope_key)
        .bind(&key.canonical_name_key)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(target_id) = target_id else {
            persist_emission(&mut transaction, &key, 0, false).await?;
            transaction.commit().await?;
            return Ok(Vec::new());
        };
        let source_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT k.concept_id
             FROM concept_consolidation_keys k
             WHERE k.scope_type=? AND k.scope_key=? AND k.canonical_name_key=?
               AND k.concept_id>? AND k.concept_id!=?
               AND NOT EXISTS(
                 SELECT 1 FROM concept_merge_proposals p
                 WHERE p.source_concept_id=k.concept_id
                   AND p.target_concept_id=? AND p.status='pending'
               )
             ORDER BY k.concept_id
             LIMIT ?",
        )
        .bind(&key.scope_type)
        .bind(&key.scope_key)
        .bind(&key.canonical_name_key)
        .bind(source_after)
        .bind(target_id)
        .bind(target_id)
        .bind(self.limit as i64)
        .fetch_all(&mut *transaction)
        .await?;
        let mut inserted = Vec::with_capacity(source_ids.len());
        for source_concept_id in source_ids {
            let proposal = ConceptMergeProposalInput {
                source_concept_id,
                target_concept_id: target_id,
                rationale: DUPLICATE_RATIONALE.into(),
                dream_run_id: self.dream_run_id,
            };
            let result = sqlx::query(
                "INSERT OR IGNORE INTO concept_merge_proposals(
                    source_concept_id,target_concept_id,rationale,status,dream_run_id,created_at,updated_at
                 ) VALUES (?,?,?,'pending',?,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)",
            )
            .bind(proposal.source_concept_id)
            .bind(proposal.target_concept_id)
            .bind(&proposal.rationale)
            .bind(proposal.dream_run_id)
            .execute(&mut *transaction)
            .await?;
            if result.rows_affected() == 1 {
                source_after = source_concept_id;
                inserted.push(proposal);
            }
        }
        let has_more =
            missing_source_after(&mut transaction, &key, target_id, source_after).await?;
        persist_emission(&mut transaction, &key, source_after, has_more).await?;
        transaction.commit().await?;
        Ok(inserted)
    }

    pub async fn maintenance_due(pool: &SqlitePool) -> Result<bool, DreamPhaseError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM concept_consolidation_dirty)
             OR EXISTS(
               SELECT 1 FROM concept_consolidation_emission
               WHERE active_scope_type IS NOT NULL
             )
             OR EXISTS(
               SELECT 1
               FROM concept_consolidation_keys source
               WHERE source.concept_id != (
                 SELECT target.concept_id
                 FROM concept_consolidation_keys target
                 WHERE target.scope_type=source.scope_type
                   AND target.scope_key=source.scope_key
                   AND target.canonical_name_key=source.canonical_name_key
                 ORDER BY target.status_rank,target.confidence DESC,target.concept_id
                 LIMIT 1
               )
                 AND EXISTS(
                   SELECT 1 FROM concept_consolidation_keys other
                   WHERE other.scope_type=source.scope_type
                     AND other.scope_key=source.scope_key
                     AND other.canonical_name_key=source.canonical_name_key
                     AND other.concept_id!=source.concept_id
                 )
                 AND NOT EXISTS(
                   SELECT 1 FROM concept_merge_proposals proposal
                   WHERE proposal.source_concept_id=source.concept_id
                     AND proposal.target_concept_id=(
                       SELECT target.concept_id
                       FROM concept_consolidation_keys target
                       WHERE target.scope_type=source.scope_type
                         AND target.scope_key=source.scope_key
                         AND target.canonical_name_key=source.canonical_name_key
                       ORDER BY target.status_rank,target.confidence DESC,target.concept_id
                       LIMIT 1
                     )
                     AND proposal.status='pending'
                 )
             )",
        )
        .fetch_one(pool)
        .await?)
    }

    async fn insert_proposals(
        &self,
        transaction: &mut Transaction<'static, Sqlite>,
        authoritative: Vec<ConceptRecord>,
    ) -> Result<Vec<ConceptMergeProposalInput>, DreamPhaseError> {
        let candidates = proposal_candidates(authoritative, self.dream_run_id);
        let mut inserted = Vec::new();
        for proposal in candidates {
            if inserted.len() == self.limit {
                break;
            }
            let result = sqlx::query(
                "INSERT OR IGNORE INTO concept_merge_proposals(
                    source_concept_id,target_concept_id,rationale,status,dream_run_id,created_at,updated_at
                 ) VALUES (?,?,?,'pending',?,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)",
            )
            .bind(proposal.source_concept_id)
            .bind(proposal.target_concept_id)
            .bind(&proposal.rationale)
            .bind(proposal.dream_run_id)
            .execute(&mut **transaction)
            .await?;
            if result.rows_affected() == 1 {
                inserted.push(proposal);
            }
        }
        Ok(inserted)
    }
}

#[async_trait]
impl DreamPhase for Consolidator {
    type Input = Vec<ConceptRecord>;
    type Output = Vec<ConceptMergeProposalInput>;

    async fn run(
        &self,
        pool: &SqlitePool,
        input: Self::Input,
    ) -> Result<Self::Output, DreamPhaseError> {
        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
        let mut authoritative = Vec::with_capacity(input.len());
        for concept in input {
            let stored = sqlx::query_as::<_, ConceptRecord>("SELECT * FROM concepts WHERE id=?")
                .bind(concept.id)
                .fetch_optional(&mut *transaction)
                .await?;
            if let Some(stored) = stored {
                authoritative.push(stored);
            }
        }
        refresh_keys(&mut transaction, &authoritative).await?;
        let inserted = self
            .insert_proposals(&mut transaction, authoritative)
            .await?;
        transaction.commit().await?;
        Ok(inserted)
    }
}

async fn refresh_dirty_page(
    transaction: &mut Transaction<'static, Sqlite>,
    limit: usize,
) -> Result<(), sqlx::Error> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT concept_id FROM concept_consolidation_dirty
         ORDER BY queue_id
         LIMIT ?",
    )
    .bind(limit as i64)
    .fetch_all(&mut **transaction)
    .await?;
    for id in ids {
        let concept = sqlx::query_as::<_, ConceptRecord>("SELECT * FROM concepts WHERE id=?")
            .bind(id)
            .fetch_optional(&mut **transaction)
            .await?;
        if let Some(concept) = concept {
            refresh_keys(transaction, std::slice::from_ref(&concept)).await?;
        } else {
            sqlx::query("DELETE FROM concept_consolidation_keys WHERE concept_id=?")
                .bind(id)
                .execute(&mut **transaction)
                .await?;
        }
        sqlx::query("DELETE FROM concept_consolidation_dirty WHERE concept_id=?")
            .bind(id)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

async fn refresh_keys(
    transaction: &mut Transaction<'static, Sqlite>,
    concepts: &[ConceptRecord],
) -> Result<(), sqlx::Error> {
    for concept in concepts {
        reset_active_group_for_concept(transaction, concept.id).await?;
        if eligible(concept) {
            sqlx::query(
                "INSERT INTO concept_consolidation_keys(
                    concept_id,scope_type,scope_key,canonical_name_key,status_rank,confidence,updated_at
                 ) VALUES (?,?,?,?,?,?,CURRENT_TIMESTAMP)
                  ON CONFLICT(concept_id) DO UPDATE SET
                    scope_type=excluded.scope_type,
                    scope_key=excluded.scope_key,
                    canonical_name_key=excluded.canonical_name_key,
                    status_rank=excluded.status_rank,
                    confidence=excluded.confidence,
                    updated_at=excluded.updated_at",
            )
            .bind(concept.id)
            .bind(&concept.scope_type)
            .bind(&concept.scope_key)
            .bind(casefold(concept.canonical_name.trim()))
            .bind(status_rank(&concept.status))
            .bind(concept.confidence)
            .execute(&mut **transaction)
            .await?;
            reset_active_group_for_concept(transaction, concept.id).await?;
        } else {
            sqlx::query("DELETE FROM concept_consolidation_keys WHERE concept_id=?")
                .bind(concept.id)
                .execute(&mut **transaction)
                .await?;
        }
    }
    Ok(())
}

async fn reset_active_group_for_concept(
    transaction: &mut Transaction<'static, Sqlite>,
    concept_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE concept_consolidation_emission
         SET source_after_concept_id=0,updated_at=CURRENT_TIMESTAMP
         WHERE singleton=1
           AND EXISTS(
             SELECT 1 FROM concept_consolidation_keys k
             WHERE k.concept_id=?
               AND k.scope_type=active_scope_type
               AND k.scope_key=active_scope_key
               AND k.canonical_name_key=active_canonical_name_key
           )",
    )
    .bind(concept_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn active_key(
    transaction: &mut Transaction<'static, Sqlite>,
) -> Result<(Option<ConsolidationKey>, i64), sqlx::Error> {
    let row: Option<(String, String, String, i64)> = sqlx::query_as(
        "SELECT active_scope_type,active_scope_key,active_canonical_name_key,
                source_after_concept_id
         FROM concept_consolidation_emission
         WHERE singleton=1
           AND active_scope_type IS NOT NULL
           AND active_scope_key IS NOT NULL
           AND active_canonical_name_key IS NOT NULL
           AND NOT EXISTS(
             SELECT 1
             FROM concept_consolidation_keys k
             JOIN concept_consolidation_dirty dirty ON dirty.concept_id=k.concept_id
             WHERE k.scope_type=active_scope_type
               AND k.scope_key=active_scope_key
               AND k.canonical_name_key=active_canonical_name_key
           )
           AND NOT EXISTS(
             SELECT 1 FROM concept_consolidation_dirty dirty
             WHERE dirty.scope_type=active_scope_type
               AND dirty.scope_key=active_scope_key
               AND dirty.canonical_name COLLATE hiero_casefold
                   =active_canonical_name_key
           )",
    )
    .fetch_optional(&mut **transaction)
    .await?;
    Ok(row.map_or((None, 0), |row| {
        (
            Some(ConsolidationKey {
                scope_type: row.0,
                scope_key: row.1,
                canonical_name_key: row.2,
            }),
            row.3,
        )
    }))
}

async fn next_duplicate_key(
    transaction: &mut Transaction<'static, Sqlite>,
) -> Result<Option<ConsolidationKey>, sqlx::Error> {
    sqlx::query_as(
        "SELECT k.scope_type,k.scope_key,k.canonical_name_key
         FROM concept_consolidation_keys k
         JOIN concept_consolidation_emission e ON e.singleton=1
         WHERE (
           e.has_after_key=0
           OR k.scope_type>e.after_scope_type
           OR (k.scope_type=e.after_scope_type AND k.scope_key>e.after_scope_key)
           OR (k.scope_type=e.after_scope_type AND k.scope_key=e.after_scope_key
               AND k.canonical_name_key>e.after_canonical_name_key)
         )
           AND EXISTS(
             SELECT 1 FROM concept_consolidation_keys other
             WHERE other.scope_type=k.scope_type
               AND other.scope_key=k.scope_key
               AND other.canonical_name_key=k.canonical_name_key
               AND other.concept_id!=k.concept_id
           )
           AND NOT EXISTS(
             SELECT 1
             FROM concept_consolidation_keys member
             JOIN concept_consolidation_dirty dirty ON dirty.concept_id=member.concept_id
             WHERE member.scope_type=k.scope_type
               AND member.scope_key=k.scope_key
               AND member.canonical_name_key=k.canonical_name_key
           )
           AND NOT EXISTS(
             SELECT 1 FROM concept_consolidation_dirty dirty
             WHERE dirty.scope_type=k.scope_type
               AND dirty.scope_key=k.scope_key
               AND dirty.canonical_name COLLATE hiero_casefold
                   =k.canonical_name_key
           )
         ORDER BY k.scope_type,k.scope_key,k.canonical_name_key,k.concept_id
         LIMIT 1",
    )
    .fetch_optional(&mut **transaction)
    .await
}

async fn reset_key_cursor(
    transaction: &mut Transaction<'static, Sqlite>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE concept_consolidation_emission
         SET has_after_key=0,after_scope_type='',after_scope_key='',
             after_canonical_name_key='',updated_at=CURRENT_TIMESTAMP
         WHERE singleton=1",
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn missing_source_after(
    transaction: &mut Transaction<'static, Sqlite>,
    key: &ConsolidationKey,
    target_id: i64,
    source_after: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM concept_consolidation_keys k
           WHERE k.scope_type=? AND k.scope_key=? AND k.canonical_name_key=?
             AND k.concept_id>? AND k.concept_id!=?
             AND NOT EXISTS(
               SELECT 1 FROM concept_merge_proposals p
               WHERE p.source_concept_id=k.concept_id
                 AND p.target_concept_id=? AND p.status='pending'
             )
         )",
    )
    .bind(&key.scope_type)
    .bind(&key.scope_key)
    .bind(&key.canonical_name_key)
    .bind(source_after)
    .bind(target_id)
    .bind(target_id)
    .fetch_one(&mut **transaction)
    .await
}

async fn persist_emission(
    transaction: &mut Transaction<'static, Sqlite>,
    key: &ConsolidationKey,
    source_after: i64,
    has_more: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE concept_consolidation_emission
         SET has_after_key=?,
             after_scope_type=?,after_scope_key=?,after_canonical_name_key=?,
             active_scope_type=?,active_scope_key=?,active_canonical_name_key=?,
             source_after_concept_id=?,updated_at=CURRENT_TIMESTAMP
         WHERE singleton=1",
    )
    .bind(!has_more)
    .bind(&key.scope_type)
    .bind(&key.scope_key)
    .bind(&key.canonical_name_key)
    .bind(has_more.then_some(&key.scope_type))
    .bind(has_more.then_some(&key.scope_key))
    .bind(has_more.then_some(&key.canonical_name_key))
    .bind(if has_more { source_after } else { 0 })
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn proposal_candidates(
    authoritative: Vec<ConceptRecord>,
    dream_run_id: i64,
) -> Vec<ConceptMergeProposalInput> {
    let mut groups = BTreeMap::<(String, String, String), Vec<ConceptRecord>>::new();
    for concept in authoritative.into_iter().filter(eligible) {
        groups
            .entry(concept_key(&concept))
            .or_default()
            .push(concept);
    }
    let mut candidates = Vec::new();
    for mut group in groups.into_values().filter(|group| group.len() > 1) {
        group.sort_by(target_order);
        let target_id = group[0].id;
        let mut sources = group.into_iter().skip(1).collect::<Vec<_>>();
        sources.sort_by_key(|source| source.id);
        candidates.extend(sources.into_iter().map(|source| ConceptMergeProposalInput {
            source_concept_id: source.id,
            target_concept_id: target_id,
            rationale: DUPLICATE_RATIONALE.into(),
            dream_run_id,
        }));
    }
    candidates.sort_by_key(|proposal| proposal.source_concept_id);
    candidates
}

fn eligible(concept: &ConceptRecord) -> bool {
    concept.merged_into_concept_id.is_none()
        && matches!(concept.status.as_str(), "candidate" | "established")
}

fn concept_key(concept: &ConceptRecord) -> (String, String, String) {
    (
        concept.scope_type.clone(),
        concept.scope_key.clone(),
        casefold(concept.canonical_name.trim()),
    )
}

fn target_order(left: &ConceptRecord, right: &ConceptRecord) -> std::cmp::Ordering {
    status_rank(&left.status)
        .cmp(&status_rank(&right.status))
        .then_with(|| right.confidence.total_cmp(&left.confidence))
        .then_with(|| left.id.cmp(&right.id))
}

fn status_rank(status: &str) -> u8 {
    u8::from(status != "established")
}

fn casefold(value: &str) -> String {
    CaseMapper::new().fold_string(value).into_owned()
}
