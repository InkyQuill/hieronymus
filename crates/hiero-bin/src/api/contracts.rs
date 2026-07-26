use std::collections::BTreeMap;

use hiero_core::{
    dreaming::{DreamConfig, PhaseProfile},
    provider::{CredentialSource, PassName, ProviderCatalog, ProviderProfile},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderDraft {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    pub url: String,
    pub key: String,
    pub timeout_seconds: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SaveProviderRequest {
    pub provider: ProviderDraft,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProviderContract {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    pub url: String,
    pub key_configured: bool,
    pub model: String,
    pub timeout_seconds: f64,
}

impl ProviderContract {
    pub fn from_profile(profile: &ProviderProfile, catalog: &ProviderCatalog) -> Self {
        Self {
            id: profile.id().to_owned(),
            name: profile.name().to_owned(),
            provider_type: profile.provider_type().to_owned(),
            url: profile.base_url().to_owned(),
            key_configured: !matches!(profile.credential(), CredentialSource::None),
            model: if catalog.defaults().provider == profile.id() {
                catalog.defaults().model.clone()
            } else {
                String::new()
            },
            timeout_seconds: profile.timeout().map_or(0.0, |value| value.as_secs_f64()),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProvidersResponse {
    pub providers: Vec<ProviderContract>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProviderResponse {
    pub provider: ProviderContract,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelsResponse {
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderCheck {
    pub ok: bool,
    pub models: Vec<String>,
    pub source: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderCheckResponse {
    pub check: ProviderCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workflow {
    pub provider: String,
    pub model: String,
    pub enabled: bool,
    pub max_records_per_pass: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DreamingValues {
    pub enabled: bool,
    pub schedule_interval_minutes: u64,
    pub min_pending_short_term_memories: usize,
    pub max_pending_short_term_memories: usize,
    pub max_short_term_memories_per_cycle: usize,
    pub not_enough_memories_cycle_threshold: usize,
    pub max_changed_crystals_per_cycle: usize,
    pub max_related_concepts_per_cycle: usize,
    pub max_related_crystals_per_concept: usize,
    pub max_total_affected_crystals: usize,
    pub max_short_term_memories_per_run: usize,
    pub max_long_term_records_affected_per_run: usize,
    pub max_relation_records_per_pass: usize,
    pub general_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DreamSettings {
    pub dreaming: DreamingValues,
    pub workflows: BTreeMap<String, Workflow>,
}

impl From<&DreamConfig> for DreamSettings {
    fn from(value: &DreamConfig) -> Self {
        Self {
            dreaming: DreamingValues {
                enabled: value.enabled,
                schedule_interval_minutes: value.schedule_interval_minutes,
                min_pending_short_term_memories: value.min_pending_short_term_memories,
                max_pending_short_term_memories: value.max_pending_short_term_memories,
                max_short_term_memories_per_cycle: value.max_short_term_memories_per_cycle,
                not_enough_memories_cycle_threshold: value.not_enough_memories_cycle_threshold,
                max_changed_crystals_per_cycle: value.max_changed_crystals_per_cycle,
                max_related_concepts_per_cycle: value.max_related_concepts_per_cycle,
                max_related_crystals_per_concept: value.max_related_crystals_per_concept,
                max_total_affected_crystals: value.max_total_affected_crystals,
                max_short_term_memories_per_run: value.max_short_term_memories_per_run,
                max_long_term_records_affected_per_run: value
                    .max_long_term_records_affected_per_run,
                max_relation_records_per_pass: value.max_relation_records_per_pass,
                general_prompt: value.general_prompt.clone(),
            },
            workflows: value
                .workflows
                .iter()
                .map(|(name, workflow)| {
                    (
                        name.as_str().to_owned(),
                        Workflow {
                            provider: workflow.provider.clone(),
                            model: workflow.model.clone(),
                            enabled: workflow.enabled,
                            max_records_per_pass: workflow.max_records_per_pass,
                        },
                    )
                })
                .collect(),
        }
    }
}

impl TryFrom<DreamSettings> for DreamConfig {
    type Error = String;

    fn try_from(value: DreamSettings) -> Result<Self, Self::Error> {
        let mut config = DreamConfig {
            enabled: value.dreaming.enabled,
            schedule_interval_minutes: value.dreaming.schedule_interval_minutes,
            min_pending_short_term_memories: value.dreaming.min_pending_short_term_memories,
            max_pending_short_term_memories: value.dreaming.max_pending_short_term_memories,
            max_short_term_memories_per_cycle: value.dreaming.max_short_term_memories_per_cycle,
            not_enough_memories_cycle_threshold: value.dreaming.not_enough_memories_cycle_threshold,
            max_changed_crystals_per_cycle: value.dreaming.max_changed_crystals_per_cycle,
            max_related_concepts_per_cycle: value.dreaming.max_related_concepts_per_cycle,
            max_related_crystals_per_concept: value.dreaming.max_related_crystals_per_concept,
            max_total_affected_crystals: value.dreaming.max_total_affected_crystals,
            max_short_term_memories_per_run: value.dreaming.max_short_term_memories_per_run,
            max_long_term_records_affected_per_run: value
                .dreaming
                .max_long_term_records_affected_per_run,
            max_relation_records_per_pass: value.dreaming.max_relation_records_per_pass,
            general_prompt: value.dreaming.general_prompt,
            ..DreamConfig::default()
        };
        config.workflows.clear();
        for phase in PassName::ALL {
            let workflow = value
                .workflows
                .get(phase.as_str())
                .ok_or_else(|| format!("missing dream workflow: {}", phase.as_str()))?;
            config.workflows.insert(
                phase,
                PhaseProfile {
                    provider: workflow.provider.clone(),
                    model: workflow.model.clone(),
                    enabled: workflow.enabled,
                    max_records_per_pass: workflow.max_records_per_pass,
                },
            );
        }
        config.validate().map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DreamRequest {
    pub dream: DreamSettings,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DreamResponse {
    pub dream: DreamSettings,
    pub providers: Vec<ProviderContract>,
    pub model_cache: ModelCacheContract,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ModelCacheContract {
    pub providers: BTreeMap<String, CachedModels>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CachedModels {
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IngestRequest<T> {
    pub ingest: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSettings {
    pub update_channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseRequest {
    pub release: ReleaseSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdminActionRequest {
    pub id: i64,
    pub confirmed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ActionMessage {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AdminRow {
    pub id: Value,
    pub kind: String,
    pub label: String,
    pub status: String,
    pub scope: String,
    pub language_pair: String,
    pub quality_label: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AdminDetail {
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AdminSnapshot {
    pub view: String,
    pub rows: Vec<AdminRow>,
    pub selected: Option<AdminRow>,
    pub detail: AdminDetail,
}

impl AdminSnapshot {
    pub fn empty(view: impl Into<String>) -> Self {
        let view = view.into();
        Self {
            detail: AdminDetail {
                title: view.clone(),
                subtitle: "No record selected".into(),
                body: String::new(),
                fields: Vec::new(),
            },
            view,
            rows: Vec::new(),
            selected: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AdminSnapshotResponse {
    pub snapshot: AdminSnapshot,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AdminHeader {
    pub product: String,
    pub version: String,
    pub tagline: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AdminDashboard {
    pub header: AdminHeader,
    pub stats: BTreeMap<String, i64>,
    pub views: Vec<String>,
    pub short_term_status: BTreeMap<String, Value>,
    pub dream_status: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AdminActionResult {
    pub result: ActionMessage,
    pub snapshot: AdminSnapshot,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ManualDreamResponse {
    pub started: bool,
    pub status: String,
}
