use crate::provider::{PassName, ProviderCatalog};

use super::{DreamConfig, DreamConfigError, PhaseProfile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowProfile {
    pub phase: PassName,
    pub provider: String,
    pub model: String,
    pub max_records_per_pass: usize,
}

pub fn resolve_workflows(
    config: &DreamConfig,
    catalog: &ProviderCatalog,
) -> Result<Vec<WorkflowProfile>, DreamConfigError> {
    config.validate_ref()?;
    let mut resolved = Vec::new();
    for phase in PassName::ALL {
        let profile = &config.workflows[&phase];
        if !profile.enabled {
            continue;
        }
        resolved.push(resolve_phase(phase, profile, catalog)?);
    }
    Ok(resolved)
}

fn resolve_phase(
    phase: PassName,
    profile: &PhaseProfile,
    catalog: &ProviderCatalog,
) -> Result<WorkflowProfile, DreamConfigError> {
    let provider = non_empty(&profile.provider)
        .or_else(|| non_empty(&catalog.defaults().provider))
        .ok_or_else(|| {
            DreamConfigError::Invalid(format!(
                "enabled workflow must have a provider: {}",
                phase.as_str()
            ))
        })?;
    let model = non_empty(&profile.model)
        .or_else(|| non_empty(&catalog.defaults().model))
        .ok_or_else(|| {
            DreamConfigError::Invalid(format!(
                "enabled workflow must have a model: {}",
                phase.as_str()
            ))
        })?;
    if provider != "deterministic" && catalog.get(provider).is_none() {
        return Err(DreamConfigError::Invalid(format!(
            "provider profile missing: {provider}"
        )));
    }
    Ok(WorkflowProfile {
        phase,
        provider: provider.to_owned(),
        model: model.to_owned(),
        max_records_per_pass: profile.max_records_per_pass,
    })
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

pub fn build_phase_prompt(
    config: &DreamConfig,
    phase: PassName,
    input: &serde_json::Value,
) -> String {
    let phase_prompt = match phase {
        PassName::Concepts => {
            "Use English memory prose by default. Extract every supported concept and its advisory facets. Do not create translation rules. Every item must list source_memory_ids. Return JSON."
        }
        PassName::TerminologyCandidates => {
            "Use English memory prose by default. Extract advisory terminology candidates and source evidence. They must not impose strict validation. Return JSON."
        }
        PassName::RuleCrystals => {
            "Use English memory prose by default. Discover deterministic translation rules only when explicit user-rule evidence supports them. Every rule must list source_memory_ids. Return JSON."
        }
        PassName::KnowledgeCrystals => {
            "Use English memory prose by default. Extract factual, narrative, stylistic, character, world, and analytical knowledge as concise crystals with source_memory_ids. Return JSON."
        }
        PassName::Relations => {
            "Use English memory prose by default. Discover supported relations between concepts and crystals with source_memory_ids. Return JSON."
        }
        PassName::Reinforcement => {
            "Use English memory prose by default. Identify referenced long-term memory to reinforce, with source_memory_ids. Return reinforce as objects with crystal_id, strength_delta, and confidence_delta. Return JSON."
        }
        PassName::CoverageAudit => {
            "Use English memory prose by default. Account for every selected short-term memory ID. Return covered_memory_ids and source_memory_ids for every audit item. Return JSON."
        }
    };
    format!(
        "{}\n\n{phase_prompt}\n\nReturn a single JSON object.\n\n{}",
        config.general_prompt.trim(),
        serde_json::to_string(input).expect("serializing serde_json::Value is infallible")
    )
}
