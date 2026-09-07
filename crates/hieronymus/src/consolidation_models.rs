//! Durable correction job identities and scheduler errors.
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationLease {
    pub decision_id: String,
    pub result_id: String,
    pub generation: u64,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub attempts: u64,
    pub expected_revision: u64,
    pub series_id: i64,
    pub provider_slot_id: String,
    /// Prepared bytes are replayed without another provider request.
    pub canonical_output: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Transient,
    Deterministic,
}
#[derive(Debug, thiserror::Error)]
pub enum ConsolidationError {
    #[error("consolidation storage: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("expired or superseded consolidation lease")]
    ExpiredLease,
    #[error("canonical consolidation idempotency conflict")]
    IdempotencyConflict,
    #[error("consolidation revision conflict")]
    RevisionConflict,
    #[error("consolidation invariant: {0}")]
    Invariant(String),
}

use crate::{
    authority_models::{EvidenceRef, OriginReceiptId, RenderingV1},
    story_applicability::ApplicabilityV1,
};
use serde::{Deserialize, Serialize};

/// Worker-enriched immutable output. Provider drafts cannot supply this origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsolidationResultV1 {
    pub version: u8,
    pub result_id: String,
    pub job_decision_id: String,
    pub generation: u64,
    pub expected_revision: u64,
    pub origin: OriginReceiptId,
    pub evidence_refs: Vec<EvidenceRef>,
    pub mutations: Vec<DerivedMutationV1>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum DerivedMutationV1 {
    LearnedRule {
        concept_id: i64,
        source_language: String,
        target_language: String,
        applicability: ApplicabilityV1,
        operation: Box<LearnedRuleOperationV1>,
    },
    ClaimLineage {
        input_claim_ids: Vec<i64>,
        output_claim_ids: Vec<i64>,
    },
}
/// Deliberately excludes correction/relevance and explicit authority operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum LearnedRuleOperationV1 {
    Activate {
        candidate_id: i64,
        candidate_revision: u64,
    },
    Replace {
        rule_id: i64,
        rule_revision: u64,
        rendering: RenderingV1,
    },
    Scope {
        rule_id: i64,
        rule_revision: u64,
        new_applicability: ApplicabilityV1,
    },
    Archive {
        rule_id: i64,
        rule_revision: u64,
    },
}
