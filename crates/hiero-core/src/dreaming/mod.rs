//! Dream-cycle configuration and deterministic workflow resolution.

mod audit;
mod concepts;
mod config;
mod crystallize;
mod evidence;
mod lock;
pub(crate) mod parsing;
mod phases;
mod workflows;

pub use audit::{
    DreamAuditEntry, DreamAuditError, DreamAuditStore, DreamPhaseRunRecord, DreamRunCompletion,
    PhaseRunStart,
};
pub use concepts::{
    ConceptPhaseCandidate, ConceptsOutput, TerminologyCandidate, TerminologyCandidatesOutput,
};
pub use config::{DreamConfig, DreamConfigError, PhaseProfile};
pub use crystallize::{Crystallizer, KnowledgeCrystalsPhase, RuleCrystalsPhase};
pub use evidence::{
    CoverageAuditOutput, CoverageAuditPhase, ReinforcementCandidate, ReinforcementOutput,
    ReinforcementPhase, RelationCandidate, RelationsOutput, RelationsPhase,
    SourcedCrystalCandidate,
};
pub use lock::{
    DreamCycleAlreadyRunning, DreamCycleGuard, DreamCyclePaths, DreamCycleState,
    acquire_dream_cycle_lock, dream_cycle_paths,
};
pub use parsing::{
    MALFORMED_OUTPUT_PENALTY, MalformedPayloadError, parse_dream_output, parse_dream_output_async,
    parse_provider_output, strip_code_fences,
};
pub use phases::{
    CatalogDreamProviderResolver, ConceptsPhase, CrystalPhaseOutput, DreamPhase, DreamPhaseError,
    DreamProviderResolver, PhaseInput, PhaseOutput, RecoveryMetadata, TerminologyCandidatesPhase,
    execute_provider_passes,
};
pub use workflows::{WorkflowProfile, build_phase_prompt, resolve_workflows};
