//! Dream-cycle configuration and deterministic workflow resolution.

mod config;
mod workflows;

pub use config::{DreamConfig, DreamConfigError, PhaseProfile};
pub use workflows::{DreamPhase, WorkflowProfile, build_phase_prompt, resolve_workflows};
