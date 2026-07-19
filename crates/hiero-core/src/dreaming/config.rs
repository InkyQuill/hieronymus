use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize, Serializer};

use crate::{
    config::HieronymusConfig,
    provider::{PassName, ProviderError, secure_read_bounded, secure_write},
};

const MAX_CONFIG_BYTES: usize = 1024 * 1024;

const GENERAL_PROMPT: &str = "Use English as the primary searchable memory language. Preserve Japanese, Russian, and other languages only as terms, names, renderings, quoted evidence, or metadata. Long-term crystals must be 1-2 sentences. Short-term memories must be 1-6 sentences.";

#[derive(Debug, thiserror::Error)]
pub enum DreamConfigError {
    #[error("dream configuration is invalid: {0}")]
    Invalid(String),
    #[error("dream configuration contains invalid TOML")]
    InvalidToml(#[source] toml::de::Error),
    #[error("dream configuration could not be serialized")]
    Serialize(#[source] toml::ser::Error),
    #[error("dream configuration file operation failed")]
    File(#[source] ProviderError),
}

pub type Result<T> = std::result::Result<T, DreamConfigError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PhaseProfile {
    pub provider: String,
    pub model: String,
    pub enabled: bool,
    pub max_records_per_pass: usize,
}

impl Default for PhaseProfile {
    fn default() -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            enabled: false,
            max_records_per_pass: 500,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DreamConfig {
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
    pub reconsolidation_diff_threshold: f64,
    pub workflows: BTreeMap<PassName, PhaseProfile>,
}

impl Default for DreamConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            schedule_interval_minutes: 30,
            min_pending_short_term_memories: 20,
            max_pending_short_term_memories: 200,
            max_short_term_memories_per_cycle: 50,
            not_enough_memories_cycle_threshold: 5,
            max_changed_crystals_per_cycle: 200,
            max_related_concepts_per_cycle: 80,
            max_related_crystals_per_concept: 20,
            max_total_affected_crystals: 500,
            max_short_term_memories_per_run: 500,
            max_long_term_records_affected_per_run: 1_000,
            max_relation_records_per_pass: 1_000,
            general_prompt: GENERAL_PROMPT.into(),
            reconsolidation_diff_threshold: 0.20,
            workflows: PassName::ALL
                .into_iter()
                .map(|phase| (phase, PhaseProfile::default()))
                .collect(),
        }
    }
}

impl DreamConfig {
    pub fn load(config: &HieronymusConfig) -> Result<Self> {
        Self::load_from(config.dream_config_path())
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let Some(contents) =
            secure_read_bounded(path.as_ref(), MAX_CONFIG_BYTES).map_err(DreamConfigError::File)?
        else {
            return Self::default().validate();
        };
        let mut persisted: PersistedDreamConfig =
            toml::from_str(&contents).map_err(DreamConfigError::InvalidToml)?;
        let mut workflows = DreamConfig::default().workflows;
        workflows.append(&mut persisted.workflows);
        persisted.workflows = workflows;
        Self::from(persisted).validate()
    }

    pub fn save(&self, config: &HieronymusConfig) -> Result<()> {
        self.save_to(config.dream_config_path())
    }

    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate_ref()?;
        let contents = toml::to_string_pretty(&PersistedDreamConfig::from(self))
            .map_err(DreamConfigError::Serialize)?;
        secure_write(path.as_ref(), contents.as_bytes()).map_err(DreamConfigError::File)
    }

    pub fn validate(self) -> Result<Self> {
        self.validate_ref()?;
        Ok(self)
    }

    pub fn with_phase(mut self, phase: PassName, profile: PhaseProfile) -> Self {
        self.workflows.insert(phase, profile);
        self
    }

    pub fn with_workflow(self, name: &str, profile: PhaseProfile) -> Result<Self> {
        let phase = phase_from_name(name)?;
        Ok(self.with_phase(phase, profile))
    }

    pub(crate) fn validate_ref(&self) -> Result<()> {
        let positive = [
            (
                "schedule_interval_minutes",
                self.schedule_interval_minutes as u128,
            ),
            (
                "min_pending_short_term_memories",
                self.min_pending_short_term_memories as u128,
            ),
            (
                "max_pending_short_term_memories",
                self.max_pending_short_term_memories as u128,
            ),
            (
                "max_short_term_memories_per_cycle",
                self.max_short_term_memories_per_cycle as u128,
            ),
            (
                "not_enough_memories_cycle_threshold",
                self.not_enough_memories_cycle_threshold as u128,
            ),
            (
                "max_changed_crystals_per_cycle",
                self.max_changed_crystals_per_cycle as u128,
            ),
            (
                "max_related_concepts_per_cycle",
                self.max_related_concepts_per_cycle as u128,
            ),
            (
                "max_related_crystals_per_concept",
                self.max_related_crystals_per_concept as u128,
            ),
            (
                "max_total_affected_crystals",
                self.max_total_affected_crystals as u128,
            ),
            (
                "max_short_term_memories_per_run",
                self.max_short_term_memories_per_run as u128,
            ),
            (
                "max_long_term_records_affected_per_run",
                self.max_long_term_records_affected_per_run as u128,
            ),
            (
                "max_relation_records_per_pass",
                self.max_relation_records_per_pass as u128,
            ),
        ];
        if let Some((name, _)) = positive.into_iter().find(|(_, value)| *value == 0) {
            return Err(DreamConfigError::Invalid(format!(
                "{name} must be at least 1"
            )));
        }
        if self.max_pending_short_term_memories < self.min_pending_short_term_memories {
            return Err(DreamConfigError::Invalid(
                "max_pending_short_term_memories must be greater than or equal to min_pending_short_term_memories".into(),
            ));
        }
        if self.max_short_term_memories_per_cycle > self.max_pending_short_term_memories {
            return Err(DreamConfigError::Invalid(
                "max_short_term_memories_per_cycle must be less than or equal to max_pending_short_term_memories".into(),
            ));
        }
        if !self.reconsolidation_diff_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.reconsolidation_diff_threshold)
        {
            return Err(DreamConfigError::Invalid(
                "reconsolidation_diff_threshold must be finite and between 0 and 1".into(),
            ));
        }
        if self.workflows.len() != PassName::ALL.len()
            || PassName::ALL
                .iter()
                .any(|phase| !self.workflows.contains_key(phase))
        {
            return Err(DreamConfigError::Invalid(
                "workflows must contain exactly the seven Dream passes".into(),
            ));
        }
        for (phase, profile) in &self.workflows {
            if profile.max_records_per_pass == 0 {
                return Err(DreamConfigError::Invalid(format!(
                    "workflows.{}.max_records_per_pass must be at least 1",
                    phase.as_str()
                )));
            }
        }
        Ok(())
    }
}

impl Serialize for DreamConfig {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PersistedDreamConfig::from(self).serialize(serializer)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PersistedDreamConfig {
    dreaming: DreamingSettings,
    workflows: BTreeMap<PassName, PhaseProfile>,
}

impl Default for PersistedDreamConfig {
    fn default() -> Self {
        Self::from(&DreamConfig::default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DreamingSettings {
    enabled: bool,
    schedule_interval_minutes: u64,
    min_pending_short_term_memories: usize,
    max_pending_short_term_memories: usize,
    max_short_term_memories_per_cycle: usize,
    not_enough_memories_cycle_threshold: usize,
    max_changed_crystals_per_cycle: usize,
    max_related_concepts_per_cycle: usize,
    max_related_crystals_per_concept: usize,
    max_total_affected_crystals: usize,
    max_short_term_memories_per_run: usize,
    max_long_term_records_affected_per_run: usize,
    max_relation_records_per_pass: usize,
    general_prompt: String,
    reconsolidation_diff_threshold: f64,
}

impl Default for DreamingSettings {
    fn default() -> Self {
        Self::from(&DreamConfig::default())
    }
}

impl From<&DreamConfig> for DreamingSettings {
    fn from(value: &DreamConfig) -> Self {
        Self {
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
            max_long_term_records_affected_per_run: value.max_long_term_records_affected_per_run,
            max_relation_records_per_pass: value.max_relation_records_per_pass,
            general_prompt: value.general_prompt.clone(),
            reconsolidation_diff_threshold: value.reconsolidation_diff_threshold,
        }
    }
}

impl From<&DreamConfig> for PersistedDreamConfig {
    fn from(value: &DreamConfig) -> Self {
        Self {
            dreaming: DreamingSettings::from(value),
            workflows: value.workflows.clone(),
        }
    }
}

impl From<PersistedDreamConfig> for DreamConfig {
    fn from(value: PersistedDreamConfig) -> Self {
        let dreaming = value.dreaming;
        Self {
            enabled: dreaming.enabled,
            schedule_interval_minutes: dreaming.schedule_interval_minutes,
            min_pending_short_term_memories: dreaming.min_pending_short_term_memories,
            max_pending_short_term_memories: dreaming.max_pending_short_term_memories,
            max_short_term_memories_per_cycle: dreaming.max_short_term_memories_per_cycle,
            not_enough_memories_cycle_threshold: dreaming.not_enough_memories_cycle_threshold,
            max_changed_crystals_per_cycle: dreaming.max_changed_crystals_per_cycle,
            max_related_concepts_per_cycle: dreaming.max_related_concepts_per_cycle,
            max_related_crystals_per_concept: dreaming.max_related_crystals_per_concept,
            max_total_affected_crystals: dreaming.max_total_affected_crystals,
            max_short_term_memories_per_run: dreaming.max_short_term_memories_per_run,
            max_long_term_records_affected_per_run: dreaming.max_long_term_records_affected_per_run,
            max_relation_records_per_pass: dreaming.max_relation_records_per_pass,
            general_prompt: dreaming.general_prompt,
            reconsolidation_diff_threshold: dreaming.reconsolidation_diff_threshold,
            workflows: value.workflows,
        }
    }
}

fn phase_from_name(name: &str) -> Result<PassName> {
    PassName::ALL
        .into_iter()
        .find(|phase| phase.as_str() == name)
        .ok_or_else(|| DreamConfigError::Invalid(format!("unknown dream workflow phase: {name}")))
}
