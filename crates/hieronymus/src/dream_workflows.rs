//! Resolution of enabled dream workflow assignments to their configured
//! providers (ADR 0007): the configured ordered workflow assignments become
//! [`WorkflowChoice`] values, `provider.conf` defaults fill blank
//! assignments, and everything is fail-closed. An enabled choice with an
//! unknown provider, a blank model, or a missing required key never runs, and
//! a run whose required `coverage_audit` pass is disabled is rejected before
//! any input is processed.
//!
//! [`WorkflowResolver`] creates a fresh provider per selected workflow inside
//! the run (worker-local: provider instances are never stored on the service
//! across threads). Configured LLM providers are constructed from the
//! resolver's catalog snapshot with [`LlmDreamProvider::new`]; deterministic
//! providers stay explicit test and diagnostic injections
//! ([`WorkflowResolver::deterministic`], [`WorkflowResolver::serving`]).

use crate::dream_config::DreamConfig;
use crate::dream_providers::LlmDreamProvider;
use crate::dreaming::{DeterministicDreamProvider, DreamError, DreamProvider, ProviderIdentity};
use crate::provider_config::{ProviderCatalog, ProviderProfile};

/// One dream workflow assignment in resolution order (ADR 0007). On the
/// choices a resolver hands out, `provider` and `model` carry the values the
/// pass will run with (catalog defaults already applied); a blank value on an
/// enabled choice fails closed in [`enabled_choices`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowChoice {
    pub name: String,
    pub enabled: bool,
    pub provider: String,
    pub model: String,
}

/// Filter the enabled assignments in order and fail closed before execution:
/// a run without an enabled `coverage_audit` pass is rejected, and so is any
/// enabled choice left without a provider or model.
pub fn enabled_choices(choices: Vec<WorkflowChoice>) -> Result<Vec<WorkflowChoice>, String> {
    let selected: Vec<_> = choices
        .into_iter()
        .filter(|choice| choice.enabled)
        .collect();
    if !selected
        .iter()
        .any(|choice| choice.name == "coverage_audit")
    {
        return Err("coverage_audit must be enabled before processing memories".into());
    }
    if selected
        .iter()
        .any(|choice| choice.provider.trim().is_empty() || choice.model.trim().is_empty())
    {
        return Err("enabled workflows require a provider and model".into());
    }
    Ok(selected)
}

/// The explicit deterministic test seam: one provider factory serving a fixed
/// identity for every workflow, plus whether the served provider is the
/// deterministic one (a production config naming the `deterministic` profile
/// id only passes the gate behind this seam).
struct FixedProviderSource {
    factory: Box<dyn Fn() -> Box<dyn DreamProvider> + 'static>,
    identity: ProviderIdentity,
    is_deterministic: bool,
}

/// Serves one fresh provider per selected workflow. The production resolver
/// snapshots the `provider.conf` catalog and constructs [`LlmDreamProvider`]
/// clients from that snapshot; the deterministic seam is the explicit test
/// and diagnostic injection that serves one fixed provider for every
/// workflow.
pub struct WorkflowResolver {
    catalog: ProviderCatalog,
    fixed: Option<FixedProviderSource>,
}

impl std::fmt::Debug for WorkflowResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.fixed {
            Some(fixed) => formatter
                .debug_struct("WorkflowResolver")
                .field("provider", &fixed.identity.name)
                .finish_non_exhaustive(),
            None => formatter
                .debug_struct("WorkflowResolver")
                .field("catalog_providers", &self.catalog.providers.len())
                .finish_non_exhaustive(),
        }
    }
}

impl WorkflowResolver {
    /// Snapshot the loaded `provider.conf` catalog (ADR 0007): enabled
    /// workflow assignments resolve against this snapshot, never against the
    /// file again.
    pub fn from_catalog(catalog: ProviderCatalog) -> Self {
        Self {
            catalog,
            fixed: None,
        }
    }

    /// Explicit deterministic injection (tests and diagnostics): serves a
    /// fresh [`DeterministicDreamProvider`] for every workflow so the full
    /// seven-pass pipeline stays exercisable without a provider catalog.
    pub fn deterministic() -> Self {
        Self::serving(|| Box::new(DeterministicDreamProvider))
    }

    /// Explicit injection of a custom provider (tests): a fresh instance from
    /// `factory` is created for every selected workflow. The factory is
    /// sampled once here to pin the identity written into run, phase, and
    /// audit records and to drive the fail-closed gate (a config-enabled
    /// workflow assigned to a configured provider still refuses
    /// substitution).
    pub fn serving(factory: impl Fn() -> Box<dyn DreamProvider> + 'static) -> Self {
        let provider = factory();
        let identity = ProviderIdentity {
            profile: provider.profile_name().to_string(),
            name: provider.name().to_string(),
            model: provider.model().to_string(),
            endpoint: provider.endpoint().to_string(),
        };
        let is_deterministic = provider.is_deterministic();
        Self {
            catalog: ProviderCatalog::default(),
            fixed: Some(FixedProviderSource {
                factory: Box::new(factory),
                identity,
                is_deterministic,
            }),
        }
    }

    /// Create the fresh provider that runs one selected workflow (worker
    /// local by construction: the instance lives only inside the pass that
    /// asked for it).
    pub fn provider(&self, choice: &WorkflowChoice) -> Result<Box<dyn DreamProvider>, DreamError> {
        let Some(fixed) = &self.fixed else {
            if choice.provider == "deterministic" {
                return Err(DreamError::InvalidWorkflow(format!(
                    "workflow {} is declared deterministic; only the deterministic \
                     provider may run it",
                    choice.name
                )));
            }
            let profile = self.profile(choice)?;
            let provider =
                LlmDreamProvider::new(choice.provider.clone(), profile, choice.model.clone())?;
            return Ok(Box::new(provider));
        };
        Ok((fixed.factory)())
    }

    /// The identity the resolver serves for one choice: what run, phase, and
    /// audit records write — the actual resolved profile id and model, never
    /// a single injected provider's identity for every phase.
    pub(crate) fn identity(&self, choice: &WorkflowChoice) -> Result<ProviderIdentity, DreamError> {
        match &self.fixed {
            Some(fixed) => Ok(fixed.identity.clone()),
            None => {
                let profile = self.profile(choice)?;
                Ok(ProviderIdentity {
                    profile: choice.provider.clone(),
                    name: profile.provider_type().to_string(),
                    model: choice.model.clone(),
                    endpoint: profile.url().to_string(),
                })
            }
        }
    }

    /// The run-level provider summary (the `dream_runs.provider` label): the
    /// first enabled workflow's wire provider name; the fixed seam always
    /// reports its injected provider. Empty when no workflow is enabled —
    /// such a run fails the coverage gate before any pass.
    pub(crate) fn run_provider_name(&self, choices: &[WorkflowChoice]) -> String {
        match &self.fixed {
            Some(fixed) => fixed.identity.name.clone(),
            None => choices
                .iter()
                .filter(|choice| choice.enabled)
                .find_map(|choice| self.identity(choice).ok().map(|identity| identity.name))
                .unwrap_or_default(),
        }
    }

    /// Translate the configured ordered workflow assignments into choices
    /// (ADR 0007 resolution: explicit assignment, then catalog defaults) and
    /// run the fail-closed gate over every enabled assignment: an unknown
    /// provider, a blank provider/model, or a missing required key never
    /// opens the service. Disabled assignments are skipped, never errors.
    /// The deterministic seam claims every workflow (the injected provider
    /// explicitly serves the whole pipeline), but it still refuses to
    /// substitute for a config-enabled assignment that names a configured
    /// provider.
    pub(crate) fn translate_choices(
        &self,
        dream_config: &DreamConfig,
    ) -> Result<Vec<WorkflowChoice>, DreamError> {
        let mut choices: Vec<WorkflowChoice> = dream_config
            .workflows
            .iter()
            .map(|(name, workflow)| WorkflowChoice {
                name: name.clone(),
                enabled: workflow.enabled,
                provider: workflow.provider.clone(),
                model: workflow.model.clone(),
            })
            .collect();
        match &self.fixed {
            Some(fixed) => {
                for choice in &choices {
                    if !choice.enabled {
                        continue;
                    }
                    let (provider_id, model) = resolved_assignment(choice, &self.catalog);
                    validate_enabled(choice, &provider_id, &model, &self.catalog, Some(fixed))?;
                }
                for choice in &mut choices {
                    choice.enabled = true;
                    choice.provider = fixed.identity.name.clone();
                    choice.model = fixed.identity.model.clone();
                }
            }
            None => {
                for choice in &mut choices {
                    if !choice.enabled {
                        continue;
                    }
                    let (provider_id, model) = resolved_assignment(choice, &self.catalog);
                    choice.provider = provider_id;
                    choice.model = model;
                    validate_enabled(choice, &choice.provider, &choice.model, &self.catalog, None)?;
                }
            }
        }
        Ok(choices)
    }

    fn profile(&self, choice: &WorkflowChoice) -> Result<ProviderProfile, DreamError> {
        self.catalog
            .providers
            .get(&choice.provider)
            .cloned()
            .ok_or_else(|| {
                DreamError::InvalidWorkflow(format!(
                    "workflow {}: provider profile missing: {}",
                    choice.name, choice.provider
                ))
            })
    }
}

/// ADR 0007 resolution for one assignment: the explicit provider/model, else
/// the catalog defaults.
fn resolved_assignment(choice: &WorkflowChoice, catalog: &ProviderCatalog) -> (String, String) {
    let provider = if choice.provider.trim().is_empty() {
        catalog.defaults.provider.trim()
    } else {
        choice.provider.trim()
    };
    let model = if choice.model.trim().is_empty() {
        catalog.defaults.model.trim()
    } else {
        choice.model.trim()
    };
    (provider.to_string(), model.to_string())
}

/// The open-time fail-closed gate for one enabled assignment. The error text
/// names the workflow and the problem and carries no secret material, so it
/// is safe for run records and audit surfaces.
fn validate_enabled(
    choice: &WorkflowChoice,
    provider_id: &str,
    model: &str,
    catalog: &ProviderCatalog,
    fixed: Option<&FixedProviderSource>,
) -> Result<(), DreamError> {
    if provider_id.is_empty() {
        return Err(DreamError::InvalidWorkflow(format!(
            "workflow {}: enabled workflow must have a provider",
            choice.name
        )));
    }
    if model.is_empty() {
        return Err(DreamError::InvalidWorkflow(format!(
            "workflow {}: enabled workflow must have a model",
            choice.name
        )));
    }
    if provider_id == "deterministic" {
        if fixed.is_some_and(|fixed| fixed.is_deterministic) {
            return Ok(());
        }
        return Err(DreamError::InvalidWorkflow(format!(
            "workflow {} is declared deterministic; only the deterministic \
             provider may run it",
            choice.name
        )));
    }
    if fixed.is_some() {
        return Err(DreamError::InvalidWorkflow(format!(
            "workflow {} requires configured provider {provider_id}; the \
             deterministic provider never substitutes for a configured LLM workflow",
            choice.name
        )));
    }
    let Some(profile) = catalog.providers.get(provider_id) else {
        return Err(DreamError::InvalidWorkflow(format!(
            "workflow {}: provider profile missing: {provider_id}",
            choice.name
        )));
    };
    // Python `_provider_from_profile`: only ollama runs without a key.
    if profile.provider_type() != "ollama" && profile.key().is_blank() {
        return Err(DreamError::InvalidWorkflow(format!(
            "workflow {}: API key missing for provider profile: {provider_id}",
            choice.name
        )));
    }
    Ok(())
}
