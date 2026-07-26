use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use sqlx::SqlitePool;

use crate::{
    domain::{ShortTermMemory, TranslationContext, apply_malformed_confidence_penalty},
    provider::{
        DreamProvider, PassName, ProviderCatalog, ProviderError, ProviderPassOutput,
        ProviderRegistry,
    },
};

use super::{
    ConceptsOutput, CoverageAuditOutput, MALFORMED_OUTPUT_PENALTY, ReinforcementOutput,
    RelationsOutput, SourcedCrystalCandidate, TerminologyCandidatesOutput, WorkflowProfile,
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
#[non_exhaustive]
pub enum DreamPhaseError {
    #[error("dream provider failed")]
    Provider(#[source] ProviderError),
    #[error("dream phase output schema is invalid")]
    InvalidSchema,
    #[error("dream phase output exceeds max_records_per_pass")]
    OutputLimit,
    #[error("dream workflow provider could not be resolved")]
    ProviderResolution,
    #[error("dream phase database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("dream phase input is invalid: {0}")]
    InvalidInput(&'static str),
}

pub trait DreamProviderResolver: Send + Sync {
    fn resolve(
        &self,
        workflow: &WorkflowProfile,
    ) -> Result<Arc<dyn DreamProvider>, DreamPhaseError>;
}

impl<F> DreamProviderResolver for F
where
    F: Fn(&WorkflowProfile) -> Result<Arc<dyn DreamProvider>, DreamPhaseError> + Send + Sync,
{
    fn resolve(
        &self,
        workflow: &WorkflowProfile,
    ) -> Result<Arc<dyn DreamProvider>, DreamPhaseError> {
        self(workflow)
    }
}

pub struct CatalogDreamProviderResolver<'a> {
    registry: &'a ProviderRegistry,
    catalog: &'a ProviderCatalog,
}

impl<'a> CatalogDreamProviderResolver<'a> {
    #[must_use]
    pub const fn new(registry: &'a ProviderRegistry, catalog: &'a ProviderCatalog) -> Self {
        Self { registry, catalog }
    }
}

impl DreamProviderResolver for CatalogDreamProviderResolver<'_> {
    fn resolve(
        &self,
        workflow: &WorkflowProfile,
    ) -> Result<Arc<dyn DreamProvider>, DreamPhaseError> {
        self.registry
            .resolve(self.catalog, &workflow.provider, &workflow.model)
            .map(Arc::from)
            .map_err(DreamPhaseError::Provider)
    }
}

#[derive(Debug, Clone)]
pub struct PhaseInput {
    pub context: TranslationContext,
    pub memories: Vec<ShortTermMemory>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RecoveryMetadata {
    pub recovered: bool,
    pub malformed_penalty: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrystalPhaseOutput {
    #[serde(default)]
    pub crystals: Vec<SourcedCrystalCandidate>,
    #[serde(skip)]
    pub recovery: RecoveryMetadata,
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
                let parsed = self
                    .provider
                    .run_pass($pass, &input.context, &input.memories)
                    .await
                    .map_err(DreamPhaseError::Provider)?;
                decode_and_validate(parsed, &input.memories)
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
    resolver: &dyn DreamProviderResolver,
    workflow: &[WorkflowProfile],
    context: TranslationContext,
    memories: Vec<ShortTermMemory>,
) -> Result<Vec<PhaseOutput>, DreamPhaseError> {
    let input = PhaseInput { context, memories };
    let mut outputs = Vec::with_capacity(workflow.len());
    for profile in workflow {
        let provider = resolver.resolve(profile)?;
        let mut output = match profile.phase {
            PassName::Concepts => PhaseOutput::Concepts(
                ConceptsPhase::new(provider.as_ref())
                    .run(pool, input.clone())
                    .await?,
            ),
            PassName::TerminologyCandidates => {
                let output = TerminologyCandidatesPhase::new(provider.as_ref())
                    .run(pool, input.clone())
                    .await?;
                PhaseOutput::TerminologyCandidates(output)
            }
            PassName::RuleCrystals => PhaseOutput::RuleCrystals(
                RuleCrystalsPhase::new(provider.as_ref())
                    .run(pool, input.clone())
                    .await?,
            ),
            PassName::KnowledgeCrystals => PhaseOutput::KnowledgeCrystals(
                KnowledgeCrystalsPhase::new(provider.as_ref())
                    .run(pool, input.clone())
                    .await?,
            ),
            PassName::Relations => PhaseOutput::Relations(
                RelationsPhase::new(provider.as_ref())
                    .run(pool, input.clone())
                    .await?,
            ),
            PassName::Reinforcement => PhaseOutput::Reinforcement(
                ReinforcementPhase::new(provider.as_ref())
                    .run(pool, input.clone())
                    .await?,
            ),
            PassName::CoverageAudit => PhaseOutput::CoverageAudit(
                CoverageAuditPhase::new(provider.as_ref())
                    .run(pool, input.clone())
                    .await?,
            ),
        };
        if output_count(&output) > profile.max_records_per_pass {
            return Err(DreamPhaseError::OutputLimit);
        }
        if let PhaseOutput::TerminologyCandidates(value) = &mut output {
            value.deduplicate();
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

trait ValidatedPhaseOutput: DeserializeOwned {
    fn is_valid(&self, allowed_memory_ids: &HashSet<i64>) -> bool;
    fn apply_recovery(&mut self, recovery: RecoveryMetadata);
}

fn decode_and_validate<T: ValidatedPhaseOutput>(
    parsed: ProviderPassOutput,
    memories: &[ShortTermMemory],
) -> Result<T, DreamPhaseError> {
    let recovery = match (parsed.recovered, parsed.malformed_penalty) {
        (false, 0.0) => RecoveryMetadata::default(),
        (true, MALFORMED_OUTPUT_PENALTY) => RecoveryMetadata {
            recovered: true,
            malformed_penalty: MALFORMED_OUTPUT_PENALTY,
        },
        _ => return Err(DreamPhaseError::InvalidSchema),
    };
    let mut output: T =
        serde_json::from_value(parsed.value).map_err(|_| DreamPhaseError::InvalidSchema)?;
    let allowed_memory_ids = memories.iter().map(|memory| memory.id).collect();
    if !output.is_valid(&allowed_memory_ids) {
        return Err(DreamPhaseError::InvalidSchema);
    }
    output.apply_recovery(recovery);
    Ok(output)
}

fn valid_source_ids(ids: &[i64], allowed_memory_ids: &HashSet<i64>) -> bool {
    !ids.is_empty()
        && ids
            .iter()
            .all(|id| *id > 0 && allowed_memory_ids.contains(id))
}

impl ValidatedPhaseOutput for ConceptsOutput {
    fn is_valid(&self, allowed_memory_ids: &HashSet<i64>) -> bool {
        self.concepts.iter().all(|candidate| {
            !candidate.concept.canonical_name.trim().is_empty()
                && valid_source_ids(&candidate.source_memory_ids, allowed_memory_ids)
                && candidate
                    .concept
                    .facets
                    .iter()
                    .all(|(kind, value, language)| {
                        !kind.trim().is_empty()
                            && !value.trim().is_empty()
                            && !language.trim().is_empty()
                    })
        })
    }

    fn apply_recovery(&mut self, recovery: RecoveryMetadata) {
        self.recovery = recovery;
    }
}

impl ValidatedPhaseOutput for TerminologyCandidatesOutput {
    fn is_valid(&self, allowed_memory_ids: &HashSet<i64>) -> bool {
        self.concept_proposals.iter().all(|candidate| {
            !candidate.concept_text.trim().is_empty()
                && !candidate.source_form.trim().is_empty()
                && !candidate.canonical_rendering.trim().is_empty()
                && valid_source_ids(&candidate.source_memory_ids, allowed_memory_ids)
        })
    }

    fn apply_recovery(&mut self, recovery: RecoveryMetadata) {
        self.recovery = recovery;
    }
}

impl ValidatedPhaseOutput for CrystalPhaseOutput {
    fn is_valid(&self, allowed_memory_ids: &HashSet<i64>) -> bool {
        self.crystals.iter().all(|candidate| {
            !candidate.crystal.crystal_type.trim().is_empty()
                && !candidate.crystal.title.trim().is_empty()
                && !candidate.crystal.text.trim().is_empty()
                && !candidate.crystal.source_credibility.trim().is_empty()
                && candidate.crystal.confidence.is_finite()
                && (0.0..=1.0).contains(&candidate.crystal.confidence)
                && valid_source_ids(&candidate.source_memory_ids, allowed_memory_ids)
        })
    }

    fn apply_recovery(&mut self, recovery: RecoveryMetadata) {
        self.recovery = recovery;
        if recovery.recovered {
            for candidate in &mut self.crystals {
                candidate.crystal.confidence = apply_malformed_confidence_penalty(
                    candidate.crystal.confidence,
                    recovery.malformed_penalty,
                );
                candidate.crystal.malformed_penalty += recovery.malformed_penalty;
            }
        }
    }
}

impl ValidatedPhaseOutput for RelationsOutput {
    fn is_valid(&self, allowed_memory_ids: &HashSet<i64>) -> bool {
        self.relations.iter().all(|candidate| {
            candidate.source_id > 0
                && candidate.target_id > 0
                && candidate.source_id != candidate.target_id
                && !candidate.relation.trim().is_empty()
                && valid_source_ids(&candidate.source_memory_ids, allowed_memory_ids)
        })
    }

    fn apply_recovery(&mut self, recovery: RecoveryMetadata) {
        self.recovery = recovery;
    }
}

impl ValidatedPhaseOutput for ReinforcementOutput {
    fn is_valid(&self, allowed_memory_ids: &HashSet<i64>) -> bool {
        self.reinforce.iter().all(|candidate| {
            candidate.crystal_id > 0
                && candidate.strength_delta.is_finite()
                && candidate.confidence_delta.is_finite()
                && (-1.0..=1.0).contains(&candidate.strength_delta)
                && (-1.0..=1.0).contains(&candidate.confidence_delta)
                && valid_source_ids(&candidate.source_memory_ids, allowed_memory_ids)
        })
    }

    fn apply_recovery(&mut self, recovery: RecoveryMetadata) {
        self.recovery = recovery;
    }
}

impl ValidatedPhaseOutput for CoverageAuditOutput {
    fn is_valid(&self, allowed_memory_ids: &HashSet<i64>) -> bool {
        valid_source_ids(&self.covered_memory_ids, allowed_memory_ids)
    }

    fn apply_recovery(&mut self, recovery: RecoveryMetadata) {
        self.recovery = recovery;
    }
}
