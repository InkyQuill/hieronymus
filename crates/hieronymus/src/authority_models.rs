//! Trusted domain protocol. Requests deliberately implement Serialize only:
//! transports must enrich a separate draft, never deserialize actor claims.
use crate::story_applicability::ApplicabilityV1;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OriginReceiptId(pub String);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    ExplicitUser,
    Agent,
    Dream,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    Learned,
    ExplicitUser,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    SourcePassage,
    AlignedRendering,
    Observation,
    UserEvent,
}
impl EvidenceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SourcePassage => "source_passage",
            Self::AlignedRendering => "aligned_rendering",
            Self::Observation => "observation",
            Self::UserEvent => "user_event",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub kind: EvidenceKind,
    pub id: i64,
    pub content_hash: String,
    pub span_start: usize,
    pub span_end: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderingV1 {
    pub source_forms: Vec<String>,
    pub canonical: String,
    pub approved_variants: Vec<String>,
    pub forbidden_variants: Vec<String>,
    pub case_sensitive: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum FactEffect {
    Invalidate,
    Qualify { qualification: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CorrectionIntentV1 {
    Fact {
        claim_id: i64,
        claim_revision: u64,
        effect: FactEffect,
    },
    Relevance {
        recall_id: String,
        useful: Vec<i64>,
        missed: Vec<i64>,
    },
    Rendering {
        replaces: Option<(i64, u64)>,
        value: RenderingV1,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum OperationV1 {
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
    Correct {
        intent: CorrectionIntentV1,
    },
}
#[derive(Debug, Clone, Serialize)]
pub struct DecisionRequestV1 {
    pub version: u8,
    pub decision_id: String,
    pub expected_revision: u64,
    pub actor_kind: ActorKind,
    pub origin: OriginReceiptId,
    pub evidence_refs: Vec<EvidenceRef>,
    pub series_id: i64,
    pub concept_id: Option<i64>,
    pub source_language: String,
    pub target_language: Option<String>,
    pub applicability: ApplicabilityV1,
    pub operation: OperationV1,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TentativeReason {
    AmbiguousIdentity,
    AmbiguousIntent,
    InsufficientEvidence,
    UnknownOrder,
    ConflictingEvidence,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionReceiptV1 {
    pub decision_id: String,
    pub resulting_revision: u64,
    pub affected_rules: Vec<(i64, u64)>,
    pub affected_claims: Vec<(i64, u64)>,
    pub effective_applicability: ApplicabilityV1,
    pub effect: String,
    pub consolidation_job_id: String,
    pub origin: OriginReceiptId,
    pub committed_at: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionResultV1 {
    Applied {
        receipt: DecisionReceiptV1,
    },
    Replayed {
        receipt: DecisionReceiptV1,
    },
    Tentative {
        receipt: DecisionReceiptV1,
        reasons: Vec<TentativeReason>,
    },
}
impl DecisionResultV1 {
    pub fn receipt(&self) -> &DecisionReceiptV1 {
        match self {
            Self::Applied { receipt }
            | Self::Replayed { receipt }
            | Self::Tentative { receipt, .. } => receipt,
        }
    }
}
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecisionErrorV1 {
    #[error("unsupported decision version")]
    UnsupportedVersion,
    #[error("invalid decision request")]
    InvalidRequest,
    #[error("unverified origin")]
    UnverifiedOrigin,
    #[error("origin does not authorize this operation")]
    OriginMismatch,
    #[error("immutable evidence mismatch")]
    EvidenceMismatch,
    #[error("unknown target")]
    UnknownTarget,
    #[error("language mismatch")]
    LanguageMismatch,
    #[error("applicability conflict")]
    ApplicabilityConflict,
    #[error("authority conflict")]
    AuthorityConflict,
    #[error("revision conflict: current revision {current_revision}")]
    RevisionConflict { current_revision: u64 },
    #[error("idempotency conflict")]
    IdempotencyConflict,
    #[error("storage unavailable")]
    StorageUnavailable,
}
impl From<rusqlite::Error> for DecisionErrorV1 {
    fn from(_: rusqlite::Error) -> Self {
        Self::StorageUnavailable
    }
}
impl From<serde_json::Error> for DecisionErrorV1 {
    fn from(_: serde_json::Error) -> Self {
        Self::InvalidRequest
    }
}
