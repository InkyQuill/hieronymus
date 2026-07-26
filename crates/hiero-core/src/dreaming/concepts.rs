use serde::{Deserialize, Serialize};

use crate::provider::ConceptCandidate;

use super::RecoveryMetadata;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptPhaseCandidate {
    #[serde(flatten)]
    pub concept: ConceptCandidate,
    pub source_memory_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptsOutput {
    #[serde(default)]
    pub concepts: Vec<ConceptPhaseCandidate>,
    #[serde(skip)]
    pub recovery: RecoveryMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCandidate {
    pub concept_text: String,
    pub source_form: String,
    pub canonical_rendering: String,
    pub source_memory_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCandidatesOutput {
    #[serde(default)]
    pub concept_proposals: Vec<TerminologyCandidate>,
    #[serde(skip)]
    pub recovery: RecoveryMetadata,
}

impl TerminologyCandidatesOutput {
    pub(crate) fn deduplicate(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.concept_proposals.retain(|proposal| {
            seen.insert((
                proposal.concept_text.to_lowercase(),
                proposal.source_form.to_lowercase(),
                proposal.canonical_rendering.to_lowercase(),
            ))
        });
    }
}
