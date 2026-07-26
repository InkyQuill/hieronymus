use std::collections::BTreeMap;

use async_trait::async_trait;
use icu_casemap::CaseMapper;
use sqlx::SqlitePool;

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
        let mut groups = BTreeMap::<(String, String, String), Vec<ConceptRecord>>::new();
        for concept in input.into_iter().filter(eligible) {
            groups
                .entry((
                    concept.scope_type.clone(),
                    concept.scope_key.clone(),
                    casefold(concept.canonical_name.trim()),
                ))
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
                dream_run_id: self.dream_run_id,
            }));
        }
        candidates.sort_by_key(|proposal| proposal.source_concept_id);
        candidates.truncate(self.limit);

        let mut inserted = Vec::new();
        for proposal in candidates {
            let result = sqlx::query(
                "INSERT OR IGNORE INTO concept_merge_proposals(
                    source_concept_id,target_concept_id,rationale,status,dream_run_id,created_at,updated_at
                 ) VALUES (?,?,?,'pending',?,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)",
            )
            .bind(proposal.source_concept_id)
            .bind(proposal.target_concept_id)
            .bind(&proposal.rationale)
            .bind(proposal.dream_run_id)
            .execute(pool)
            .await?;
            if result.rows_affected() == 1 {
                inserted.push(proposal);
            }
        }
        Ok(inserted)
    }
}

fn eligible(concept: &ConceptRecord) -> bool {
    concept.merged_into_concept_id.is_none()
        && matches!(concept.status.as_str(), "candidate" | "established")
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
