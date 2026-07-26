use std::collections::BTreeMap;

use async_trait::async_trait;
use icu_casemap::CaseMapper;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::{db::ConceptRecord, domain::ConceptMergeProposalInput};

use super::{DreamPhase, DreamPhaseError};

const DUPLICATE_RATIONALE: &str = "Canonical name and scope are exact duplicates.";

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
        let (after_id, restart_required): (i64, bool) = sqlx::query_as(
            "SELECT after_concept_id,restart_required
             FROM concept_consolidation_scan
             WHERE singleton=1",
        )
        .fetch_one(&mut *transaction)
        .await?;
        let mut page = select_page(&mut transaction, after_id, self.limit).await?;
        if page.is_empty() && after_id > 0 {
            page = select_page(&mut transaction, 0, self.limit).await?;
        }
        if page.is_empty() {
            transaction.commit().await?;
            return Ok(Vec::new());
        }

        refresh_keys(&mut transaction, &page).await?;
        let last_id = page.last().map_or(0, |concept| concept.id);
        let has_later: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM concepts
                WHERE id > ?
                  AND merged_into_concept_id IS NULL
                  AND status IN ('candidate','established')
            )",
        )
        .bind(last_id)
        .fetch_one(&mut *transaction)
        .await?;
        let inserted = if has_later || restart_required {
            Vec::new()
        } else {
            let authoritative = matching_authoritative(&mut transaction).await?;
            self.insert_proposals(&mut transaction, authoritative)
                .await?
        };
        sqlx::query(
            "UPDATE concept_consolidation_scan
             SET after_concept_id=?,
                 restart_required=CASE WHEN ? THEN restart_required ELSE 0 END,
                 updated_at=CURRENT_TIMESTAMP
             WHERE singleton=1",
        )
        .bind(if has_later { last_id } else { 0 })
        .bind(has_later)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(inserted)
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

async fn select_page(
    transaction: &mut Transaction<'static, Sqlite>,
    after_id: i64,
    limit: usize,
) -> Result<Vec<ConceptRecord>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM concepts
         WHERE id > ?
           AND merged_into_concept_id IS NULL
           AND status IN ('candidate','established')
         ORDER BY id
         LIMIT ?",
    )
    .bind(after_id)
    .bind(limit as i64)
    .fetch_all(&mut **transaction)
    .await
}

async fn refresh_keys(
    transaction: &mut Transaction<'static, Sqlite>,
    concepts: &[ConceptRecord],
) -> Result<(), sqlx::Error> {
    for concept in concepts {
        if eligible(concept) {
            sqlx::query(
                "INSERT INTO concept_consolidation_keys(
                    concept_id,scope_type,scope_key,canonical_name_key,updated_at
                 ) VALUES (?,?,?,?,CURRENT_TIMESTAMP)
                 ON CONFLICT(concept_id) DO UPDATE SET
                    scope_type=excluded.scope_type,
                    scope_key=excluded.scope_key,
                    canonical_name_key=excluded.canonical_name_key,
                    updated_at=excluded.updated_at",
            )
            .bind(concept.id)
            .bind(&concept.scope_type)
            .bind(&concept.scope_key)
            .bind(casefold(concept.canonical_name.trim()))
            .execute(&mut **transaction)
            .await?;
        } else {
            sqlx::query("DELETE FROM concept_consolidation_keys WHERE concept_id=?")
                .bind(concept.id)
                .execute(&mut **transaction)
                .await?;
        }
    }
    Ok(())
}

async fn matching_authoritative(
    transaction: &mut Transaction<'static, Sqlite>,
) -> Result<Vec<ConceptRecord>, sqlx::Error> {
    sqlx::query_as(
        "SELECT c.*
         FROM concepts c
         JOIN concept_consolidation_keys k ON k.concept_id=c.id
         WHERE c.merged_into_concept_id IS NULL
           AND c.status IN ('candidate','established')
           AND EXISTS(
             SELECT 1
             FROM concept_consolidation_keys other
             WHERE other.scope_type=k.scope_type
               AND other.scope_key=k.scope_key
               AND other.canonical_name_key=k.canonical_name_key
               AND other.concept_id!=k.concept_id
           )
         ORDER BY k.scope_type,k.scope_key,k.canonical_name_key,c.id",
    )
    .fetch_all(&mut **transaction)
    .await
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
