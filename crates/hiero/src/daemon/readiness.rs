//! Daemon-lifetime readiness. Only current enabled assignments retain outcomes.
//! Credentials participate in private equality, never in identifiers or hashes.
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::dream_config::load_dream_config;
use hieronymus::dream_workflows::WorkflowResolver;
use hieronymus::provider_config::{ProviderCatalog, ProviderProfile, load_provider_catalog};
use hieronymus::provider_observation::{ProviderKey, ProviderObserver, ProviderOutcome};

use super::semantic_worker::RequiredSemanticState;
use crate::readiness::{ProviderCondition, ProviderReadiness, ReadinessLevel, ReadinessSummary};

#[derive(Clone, PartialEq)]
struct Assignment {
    capability: String,
    profile: String,
    model: String,
    configuration: Option<ProviderProfile>,
}

struct Entry {
    sequence: u64,
    outcome: Option<ProviderOutcome>,
    observed_at: Option<String>,
    capabilities: Vec<String>,
}

#[derive(Default)]
struct State {
    generation: u64,
    assignments: Vec<Assignment>,
    configuration_error: bool,
    entries: BTreeMap<ProviderKey, Entry>,
}

#[derive(Default)]
pub struct RuntimeReadiness {
    config: Option<HieronymusConfig>,
    stop: Option<Arc<AtomicBool>>,
    state: Mutex<State>,
}

impl RuntimeReadiness {
    pub fn for_config(config: HieronymusConfig, stop: Arc<AtomicBool>) -> Self {
        Self {
            config: Some(config),
            stop: Some(stop),
            state: Mutex::default(),
        }
    }

    /// Explicit seam for content-free observer contracts. In production the
    /// effective assignments are always reconciled from disk under this lock.
    pub fn activate(&self, keys: Vec<ProviderKey>) {
        let mut state = self.lock();
        state.entries.retain(|key, _| keys.contains(key));
        for key in keys {
            state.entries.entry(key).or_insert_with(empty_entry);
        }
    }

    /// Sample the same catalog that this resolver uses. Later edits cannot
    /// relabel an in-flight client: request-start reconciliation invalidates it.
    pub fn resolver(self: &Arc<Self>) -> WorkflowResolver {
        let mut state = self.lock();
        let catalog = self.reconcile(&mut state);
        let observer = Arc::new(ResolverObservation {
            runtime: Arc::clone(self),
            generation: state.generation,
            keys: state
                .entries
                .keys()
                .map(|key| ((key.profile.clone(), key.model.clone()), key.clone()))
                .collect(),
        });
        WorkflowResolver::from_catalog(catalog).with_observer(observer, state.generation)
    }

    pub fn snapshot(&self) -> ReadinessSummary {
        let mut state = self.lock();
        self.reconcile(&mut state);
        let mut reasons = Vec::new();
        if state.configuration_error {
            reasons.push("Enabled provider configuration is unavailable or incomplete".into());
        }
        let providers = state
            .entries
            .iter()
            .map(|(key, entry)| {
                let (condition, reason) = match entry.outcome {
                    None => (
                        ProviderCondition::Untested,
                        Some("External provider not yet verified".into()),
                    ),
                    Some(ProviderOutcome::Success) => (ProviderCondition::Healthy, None),
                    Some(outcome) => {
                        let reason = outcome_reason(outcome).to_string();
                        if !reasons.contains(&reason) {
                            reasons.push(reason.clone());
                        }
                        (ProviderCondition::Failed, Some(reason))
                    }
                };
                ProviderReadiness {
                    provider: redact(&key.profile, &state.assignments),
                    model: redact(&key.model, &state.assignments),
                    revision: key.revision,
                    condition,
                    reason,
                    observed_at: entry.observed_at.clone(),
                    capabilities: entry.capabilities.clone(),
                }
            })
            .collect();
        ReadinessSummary {
            level: if reasons.is_empty() {
                ReadinessLevel::Ready
            } else {
                ReadinessLevel::Degraded
            },
            reasons,
            providers,
        }
    }

    /// Read a supplied worker snapshot only; no inference or acquisition here.
    pub fn snapshot_with_semantic(&self, semantic: &RequiredSemanticState) -> ReadinessSummary {
        let mut summary = self.snapshot();
        match semantic {
            RequiredSemanticState::Ready => {}
            RequiredSemanticState::Failed(_) => {
                summary.level = ReadinessLevel::Degraded;
                summary
                    .reasons
                    .push("Required semantic retrieval is unavailable".into());
            }
            RequiredSemanticState::Acquiring | RequiredSemanticState::Rebuilding => {
                if summary.level == ReadinessLevel::Ready {
                    summary.level = ReadinessLevel::Starting;
                }
                summary.reasons.push(
                    match semantic {
                        RequiredSemanticState::Acquiring => {
                            "Required semantic model is being acquired"
                        }
                        _ => "Required semantic index is being rebuilt",
                    }
                    .into(),
                );
            }
        }
        summary
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn reconcile(&self, state: &mut State) -> ProviderCatalog {
        let Some(config) = &self.config else {
            return ProviderCatalog::default();
        };
        let catalog = load_provider_catalog(config);
        let dream = load_dream_config(config);
        let mut configuration_error = dream.is_err();
        let mut assignments = Vec::new();
        if let Ok(dream) = &dream {
            for (capability, workflow) in &dream.workflows {
                if !workflow.enabled {
                    continue;
                }
                // A disabled scheduler still permits manual execution.
                let defaults = catalog.as_ref().ok().map(|catalog| &catalog.defaults);
                let profile = if workflow.provider.trim().is_empty() {
                    defaults
                        .map(|d| d.provider.trim().to_string())
                        .unwrap_or_default()
                } else {
                    workflow.provider.trim().to_string()
                };
                if profile == "deterministic" {
                    continue;
                }
                let model = if workflow.model.trim().is_empty() {
                    defaults
                        .map(|d| d.model.trim().to_string())
                        .unwrap_or_default()
                } else {
                    workflow.model.trim().to_string()
                };
                let configuration = catalog
                    .as_ref()
                    .ok()
                    .and_then(|c| c.providers.get(&profile))
                    .cloned();
                configuration_error |= profile.is_empty()
                    || model.is_empty()
                    || configuration
                        .as_ref()
                        .is_none_or(|p| p.provider_type() != "ollama" && p.key().is_blank());
                assignments.push(Assignment {
                    capability: capability.clone(),
                    profile,
                    model,
                    configuration,
                });
            }
        }
        if assignments != state.assignments || configuration_error != state.configuration_error {
            state.generation += 1;
            let mut next: BTreeMap<ProviderKey, Entry> = BTreeMap::new();
            for assignment in &assignments {
                if next
                    .keys()
                    .any(|key| key.profile == assignment.profile && key.model == assignment.model)
                {
                    continue;
                }
                let same_assignment = |other: &&Assignment| {
                    other.profile == assignment.profile && other.model == assignment.model
                };
                let previous: Vec<_> = state.assignments.iter().filter(same_assignment).collect();
                let current: Vec<_> = assignments.iter().filter(same_assignment).collect();
                let previous_key = if previous == current {
                    state
                        .entries
                        .keys()
                        .find(|key| {
                            key.profile == assignment.profile && key.model == assignment.model
                        })
                        .cloned()
                } else {
                    None
                };
                let key = previous_key.unwrap_or_else(|| ProviderKey {
                    profile: assignment.profile.clone(),
                    model: assignment.model.clone(),
                    revision: state.generation,
                });
                next.entry(key.clone()).or_insert_with(|| {
                    state.entries.remove(&key).unwrap_or_else(|| {
                        let mut entry = empty_entry();
                        entry.capabilities = current.iter().map(|a| a.capability.clone()).collect();
                        entry
                    })
                });
            }
            state.entries = next;
            state.assignments = assignments;
            state.configuration_error = configuration_error;
        }
        catalog.unwrap_or_default()
    }
}

/// A resolver lives only for one run. This bounded mapping pins each actual
/// client to its own effective configuration, even as another profile changes.
struct ResolverObservation {
    runtime: Arc<RuntimeReadiness>,
    generation: u64,
    keys: BTreeMap<(String, String), ProviderKey>,
}

impl ProviderObserver for ResolverObservation {
    fn starting(&self) {
        self.runtime.starting();
    }

    fn completed(&self, key: &ProviderKey, sequence: u64, outcome: ProviderOutcome) {
        if key.revision == self.generation
            && let Some(effective) = self.keys.get(&(key.profile.clone(), key.model.clone()))
        {
            self.runtime.completed(effective, sequence, outcome);
        }
    }
}

impl ProviderObserver for RuntimeReadiness {
    fn starting(&self) {
        self.reconcile(&mut self.lock());
    }

    fn completed(&self, key: &ProviderKey, sequence: u64, outcome: ProviderOutcome) {
        if self
            .stop
            .as_ref()
            .is_some_and(|stop| stop.load(Ordering::Acquire))
        {
            return;
        }
        let mut state = self.lock();
        self.reconcile(&mut state);
        if let Some(entry) = state.entries.get_mut(key)
            && sequence > entry.sequence
        {
            entry.sequence = sequence;
            entry.outcome = Some(outcome);
            entry.observed_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }
}

fn empty_entry() -> Entry {
    Entry {
        sequence: 0,
        outcome: None,
        observed_at: None,
        capabilities: Vec::new(),
    }
}

fn outcome_reason(outcome: ProviderOutcome) -> &'static str {
    match outcome {
        ProviderOutcome::Success => "Provider request succeeded",
        ProviderOutcome::Unavailable => "External provider is unavailable",
        ProviderOutcome::Authentication => "External provider authentication failed",
        ProviderOutcome::RateLimited => "External provider rate limit reached",
        ProviderOutcome::InvalidResponse => "External provider returned an invalid response",
    }
}

fn redact(text: &str, assignments: &[Assignment]) -> String {
    let secrets: Vec<_> = assignments
        .iter()
        .filter_map(|a| a.configuration.as_ref())
        .map(|p| p.key().expose_secret().as_str())
        .collect();
    hieronymus::secret::redact_values(text, &secrets)
}
