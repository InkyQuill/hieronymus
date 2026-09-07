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
    #[error("consolidation policy: {0}")]
    Policy(crate::authority_models::DecisionErrorV1),
    #[error("invalid consolidation draft: {0:?}")]
    Draft(DraftProblem),
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
    pub selected_claims: Vec<SelectedClaimV1>,
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

/// Worker-captured selection, bound to the original immutable claim observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedClaimV1 {
    pub claim_id: i64,
    pub revision: u64,
    pub evidence_id: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionReceiptV1 {
    pub result_id: String,
    pub job_decision_id: String,
    pub generation: u64,
    pub resulting_revision: u64,
    pub affected_rules: Vec<(i64, u64)>,
    pub affected_claims: Vec<(i64, u64)>,
    pub committed_at: String,
}
/// Stale is a successful state transition: callers must commit it for retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionOutcome {
    Complete {
        receipt: CompletionReceiptV1,
    },
    Stale {
        result_id: String,
        next_generation: u64,
    },
}

/// Typed draft faults are transient only before durable preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftProblem {
    DuplicateRuleTarget,
    OverlappingRuleOperations,
    LineageBounds,
    UnselectedLineage,
    SelfLineage,
    LineageCycle,
}

/// A trusted unresolved event retains context and reasons without pretending that
/// an operation, source occurrence or claim has already been resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedSignalV1 {
    pub version: u8,
    pub kind: UnresolvedSignalKind,
    pub decision_id: String,
    pub origin: OriginReceiptId,
    pub text: String,
    pub context: serde_json::Value,
    /// True only when context.text was identical to text and was omitted from
    /// canonical storage. False/absent preserves old rows and structured/null text.
    #[serde(default)]
    pub context_text_elided: bool,
    pub reasons: Vec<crate::authority_models::TentativeReason>,
    pub detail: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedSignalKind {
    UnresolvedSignal,
}

/// Total serialized worker projection budget, including its outer envelope.
pub const MAX_CORRECTION_CONTEXT_BYTES: usize = 512 * 1024;
/// Reserve 64 KiB for gathering and envelope overhead after the lossless signal.
pub const MAX_UNRESOLVED_SIGNAL_BYTES: usize = MAX_CORRECTION_CONTEXT_BYTES - 64 * 1024;

impl UnresolvedSignalV1 {
    /// Reconstruct the exact strict ingress context; never invent a text field
    /// for structured events or accept ambiguous duplicated/elided forms.
    pub fn submitted_context(&self) -> Option<serde_json::Value> {
        let mut context = self.context.clone();
        let fields = context.as_object_mut()?;
        if self.context_text_elided {
            if fields.contains_key("text") {
                return None;
            }
            fields.insert("text".into(), serde_json::Value::String(self.text.clone()));
        }
        Some(context)
    }
}
