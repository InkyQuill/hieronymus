use serde::{Deserialize, Serialize};

use crate::provider::CandidateCrystal;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcedCrystalCandidate {
    #[serde(flatten)]
    pub crystal: CandidateCrystal,
    pub source_memory_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationCandidate {
    pub source_id: i64,
    pub target_id: i64,
    pub relation: String,
    pub source_memory_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationsOutput {
    #[serde(default)]
    pub relations: Vec<RelationCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReinforcementCandidate {
    pub crystal_id: i64,
    pub strength_delta: f64,
    pub confidence_delta: f64,
    pub source_memory_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReinforcementOutput {
    #[serde(default)]
    pub reinforce: Vec<ReinforcementCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageAuditOutput {
    pub covered_memory_ids: Vec<i64>,
}

pub use crate::dreaming::phases::{CoverageAuditPhase, ReinforcementPhase, RelationsPhase};
