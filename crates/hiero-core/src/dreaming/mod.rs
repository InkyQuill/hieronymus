//! Dream-cycle configuration and deterministic workflow resolution.

mod audit;
mod config;
mod lock;
mod workflows;

pub use audit::{
    DreamAuditEntry, DreamAuditError, DreamAuditStore, DreamPhaseRunRecord, DreamRunCompletion,
    PhaseRunStart,
};
pub use config::{DreamConfig, DreamConfigError, PhaseProfile};
pub use lock::{
    DreamCycleAlreadyRunning, DreamCycleGuard, DreamCyclePaths, DreamCycleState,
    acquire_dream_cycle_lock, dream_cycle_paths,
};
pub use workflows::{DreamPhase, WorkflowProfile, build_phase_prompt, resolve_workflows};
