//! Dream-cycle configuration and deterministic workflow resolution.

mod audit;
mod concepts;
mod config;
mod crystallize;
mod decay;
mod evidence;
mod lock;
pub(crate) mod parsing;
mod phases;
mod reconsolidation;
mod reinforcement;
mod scheduler;
mod service;
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
pub use decay::{
    CONFIDENCE_DECAY_AFTER_STRENGTH_BELOW, CONFIDENCE_DECAY_PER_CYCLE, DECAY_BATCH_SIZE,
    DecayManager, DecayScope, STRENGTH_DECAY_PER_CYCLE, decay_delta,
};
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
pub use reconsolidation::{
    COMBINATION_TEXT_SIMILARITY_THRESHOLD, ReconsolidationDecision, ReconsolidationOutcome,
    Reconsolidator, diff_ratio, reconsolidation_decision, text_similarity,
};
pub use reinforcement::{
    LinkOutcome, LinkReinforcer, ReinforcementManager, select_survivor, useful_pairs,
};
pub use scheduler::run_background_loop;
pub use service::{
    CycleOptions, DreamService, DreamServiceError, MaintenancePayload, MaintenanceResult,
};
pub use workflows::{WorkflowProfile, build_phase_prompt, resolve_workflows};
