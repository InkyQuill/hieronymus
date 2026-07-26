use async_trait::async_trait;
use serde::de::DeserializeOwned;
use sqlx::SqlitePool;

use crate::{
    domain::{ShortTermMemory, TranslationContext},
    provider::{DreamProvider, PassName, ProviderError},
};

use super::{
    ConceptsOutput, CoverageAuditOutput, ReinforcementOutput, RelationsOutput,
    SourcedCrystalCandidate, TerminologyCandidatesOutput, WorkflowProfile,
};

#[async_trait]
pub trait DreamPhase: Send + Sync {
    type Input;
    type Output;

    async fn run(
        &self,
        pool: &SqlitePool,
        input: Self::Input,
    ) -> Result<Self::Output, DreamPhaseError>;
}

#[derive(Debug, thiserror::Error)]
pub enum DreamPhaseError {
    #[error("dream provider failed")]
    Provider(#[source] ProviderError),
    #[error("dream phase output schema is invalid")]
    InvalidSchema,
    #[error("dream phase output exceeds max_records_per_pass")]
    OutputLimit,
}

#[derive(Debug, Clone)]
pub struct PhaseInput {
    pub context: TranslationContext,
    pub memories: Vec<ShortTermMemory>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrystalPhaseOutput {
    #[serde(default)]
    pub crystals: Vec<SourcedCrystalCandidate>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhaseOutput {
    Concepts(ConceptsOutput),
    TerminologyCandidates(TerminologyCandidatesOutput),
    RuleCrystals(CrystalPhaseOutput),
    KnowledgeCrystals(CrystalPhaseOutput),
    Relations(RelationsOutput),
    Reinforcement(ReinforcementOutput),
    CoverageAudit(CoverageAuditOutput),
}

macro_rules! provider_phase {
    ($name:ident, $pass:expr, $output:ty) => {
        pub struct $name<'a> {
            provider: &'a dyn DreamProvider,
        }

        impl<'a> $name<'a> {
            #[must_use]
            pub const fn new(provider: &'a dyn DreamProvider) -> Self {
                Self { provider }
            }
        }

        #[async_trait]
        impl DreamPhase for $name<'_> {
            type Input = PhaseInput;
            type Output = $output;

            async fn run(
                &self,
                _pool: &SqlitePool,
                input: Self::Input,
            ) -> Result<Self::Output, DreamPhaseError> {
                let value = self
                    .provider
                    .run_pass($pass, &input.context, &input.memories)
                    .await
                    .map_err(DreamPhaseError::Provider)?;
                decode(value)
            }
        }
    };
}

provider_phase!(ConceptsPhase, PassName::Concepts, ConceptsOutput);
provider_phase!(
    TerminologyCandidatesPhase,
    PassName::TerminologyCandidates,
    TerminologyCandidatesOutput
);
provider_phase!(
    RuleCrystalsPhase,
    PassName::RuleCrystals,
    CrystalPhaseOutput
);
provider_phase!(
    KnowledgeCrystalsPhase,
    PassName::KnowledgeCrystals,
    CrystalPhaseOutput
);
provider_phase!(RelationsPhase, PassName::Relations, RelationsOutput);
provider_phase!(
    ReinforcementPhase,
    PassName::Reinforcement,
    ReinforcementOutput
);
provider_phase!(
    CoverageAuditPhase,
    PassName::CoverageAudit,
    CoverageAuditOutput
);

pub async fn execute_provider_passes(
    pool: &SqlitePool,
    provider: &dyn DreamProvider,
    workflow: &[WorkflowProfile],
    context: TranslationContext,
    memories: Vec<ShortTermMemory>,
) -> Result<Vec<PhaseOutput>, DreamPhaseError> {
    let input = PhaseInput { context, memories };
    let mut outputs = Vec::with_capacity(workflow.len());
    for profile in workflow {
        let output = match profile.phase {
            PassName::Concepts => PhaseOutput::Concepts(
                ConceptsPhase::new(provider)
                    .run(pool, input.clone())
                    .await?,
            ),
            PassName::TerminologyCandidates => {
                let mut output = TerminologyCandidatesPhase::new(provider)
                    .run(pool, input.clone())
                    .await?;
                output.deduplicate();
                PhaseOutput::TerminologyCandidates(output)
            }
            PassName::RuleCrystals => PhaseOutput::RuleCrystals(
                RuleCrystalsPhase::new(provider)
                    .run(pool, input.clone())
                    .await?,
            ),
            PassName::KnowledgeCrystals => PhaseOutput::KnowledgeCrystals(
                KnowledgeCrystalsPhase::new(provider)
                    .run(pool, input.clone())
                    .await?,
            ),
            PassName::Relations => PhaseOutput::Relations(
                RelationsPhase::new(provider)
                    .run(pool, input.clone())
                    .await?,
            ),
            PassName::Reinforcement => PhaseOutput::Reinforcement(
                ReinforcementPhase::new(provider)
                    .run(pool, input.clone())
                    .await?,
            ),
            PassName::CoverageAudit => PhaseOutput::CoverageAudit(
                CoverageAuditPhase::new(provider)
                    .run(pool, input.clone())
                    .await?,
            ),
        };
        if output_count(&output) > profile.max_records_per_pass {
            return Err(DreamPhaseError::OutputLimit);
        }
        outputs.push(output);
    }
    Ok(outputs)
}

fn output_count(output: &PhaseOutput) -> usize {
    match output {
        PhaseOutput::Concepts(value) => value.concepts.len(),
        PhaseOutput::TerminologyCandidates(value) => value.concept_proposals.len(),
        PhaseOutput::RuleCrystals(value) | PhaseOutput::KnowledgeCrystals(value) => {
            value.crystals.len()
        }
        PhaseOutput::Relations(value) => value.relations.len(),
        PhaseOutput::Reinforcement(value) => value.reinforce.len(),
        PhaseOutput::CoverageAudit(value) => value.covered_memory_ids.len(),
    }
}

fn decode<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, DreamPhaseError> {
    serde_json::from_value(value).map_err(|_| DreamPhaseError::InvalidSchema)
}
