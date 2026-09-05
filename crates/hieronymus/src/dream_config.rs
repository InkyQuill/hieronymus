use indexmap::IndexMap;
use toml::Table;

use crate::atomic::atomic_write_text;
use crate::data_root::HieronymusConfig;

pub const DREAM_WORKFLOW_NAMES: [&str; 7] = [
    "concepts",
    "terminology_candidates",
    "rule_crystals",
    "knowledge_crystals",
    "relations",
    "reinforcement",
    "coverage_audit",
];

pub const DREAMING_FIELDS: [&str; 15] = [
    "enabled",
    "schedule_interval_minutes",
    "min_pending_short_term_memories",
    "max_pending_short_term_memories",
    "max_short_term_memories_per_cycle",
    "not_enough_memories_cycle_threshold",
    "max_changed_crystals_per_cycle",
    "max_related_concepts_per_cycle",
    "max_related_crystals_per_concept",
    "max_total_affected_crystals",
    "max_short_term_memories_per_run",
    "max_long_term_records_affected_per_run",
    "max_relation_records_per_pass",
    "reconsolidation_diff_threshold",
    "general_prompt",
];

const WORKFLOW_FIELDS: [&str; 4] = ["provider", "model", "enabled", "max_records_per_pass"];

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct DreamConfigError {
    message: String,
}

impl DreamConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// One dream workflow assignment (ADR 0007): which provider/model runs the
/// pass. Provider existence is validated by the dreaming runtime, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowProfile {
    pub provider: String,
    pub model: String,
    pub enabled: bool,
    pub max_records_per_pass: i64,
}

impl WorkflowProfile {
    pub fn new(provider: impl Into<String>, model: impl Into<String>, enabled: bool) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            enabled,
            max_records_per_pass: 500,
        }
    }
}

impl Default for WorkflowProfile {
    fn default() -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            enabled: true,
            max_records_per_pass: 500,
        }
    }
}

/// `dream.conf`: dreaming scheduling, caps, the general prompt, and the seven
/// workflow assignments. Never contains provider profiles or secrets.
#[derive(Debug, Clone, PartialEq)]
pub struct DreamConfig {
    pub enabled: bool,
    pub schedule_interval_minutes: i64,
    pub min_pending_short_term_memories: i64,
    pub max_pending_short_term_memories: i64,
    pub max_short_term_memories_per_cycle: i64,
    pub not_enough_memories_cycle_threshold: i64,
    pub max_changed_crystals_per_cycle: i64,
    pub max_related_concepts_per_cycle: i64,
    pub max_related_crystals_per_concept: i64,
    pub max_total_affected_crystals: i64,
    pub max_short_term_memories_per_run: i64,
    pub max_long_term_records_affected_per_run: i64,
    pub max_relation_records_per_pass: i64,
    /// Token-level diff ratio at or above which the reconsolidator supersedes
    /// the source crystal instead of reinforcing it in place.
    pub reconsolidation_diff_threshold: f64,
    pub general_prompt: String,
    pub workflows: IndexMap<String, WorkflowProfile>,
}

pub fn default_dream_config() -> DreamConfig {
    let disabled = WorkflowProfile::new("", "", false);
    let mut workflows = IndexMap::new();
    for name in DREAM_WORKFLOW_NAMES {
        workflows.insert(name.to_string(), disabled.clone());
    }
    DreamConfig {
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
        max_long_term_records_affected_per_run: 1000,
        max_relation_records_per_pass: 1000,
        reconsolidation_diff_threshold: 0.20,
        general_prompt:
            "Use English as the primary searchable memory language. Preserve Japanese, \
             Russian, and other languages only as terms, names, renderings, quoted \
             evidence, or metadata. Long-term crystals must be 1-2 sentences. \
             Short-term memories must be 1-6 sentences."
                .to_string(),
        workflows,
    }
}

impl DreamConfig {
    /// Programmatic workflow replacement. Legacy alpha workflow names map to
    /// their current names; the persisted schema never accepts them.
    pub fn with_workflow(&self, name: &str, workflow: WorkflowProfile) -> Self {
        let name = match name {
            "crystallization" => "knowledge_crystals",
            "relation_discovery" => "relations",
            "reinforcement_compaction" => "reinforcement",
            other => other,
        };
        let mut next = self.clone();
        next.workflows.insert(name.to_string(), workflow);
        next
    }
}

pub fn load_dream_config(config: &HieronymusConfig) -> Result<DreamConfig, DreamConfigError> {
    let path = config.dream_config_path();
    if !path.exists() {
        return validate_dream_config(&default_dream_config());
    }

    let text = std::fs::read_to_string(&path)
        .map_err(|error| DreamConfigError::new(format!("dream.conf could not be read: {error}")))?;
    let payload = text
        .parse::<Table>()
        .map_err(|error| DreamConfigError::new(format!("dream.conf is not valid TOML: {error}")))?;
    let (payload, migrated) = migrate_workflow_payload(payload);
    let dream_config = validate_dream_config(&dream_config_from_payload(&payload)?)?;
    if migrated {
        save_dream_config(config, &dream_config)?;
    }
    Ok(dream_config)
}

/// Parse and validate dream.conf without touching any file: unlike
/// [`load_dream_config`] this never persists a legacy-payload migration, so
/// read-only surfaces (doctor) resolve through here.
pub fn resolve_dream_config_readonly(
    config: &HieronymusConfig,
) -> Result<DreamConfig, DreamConfigError> {
    let path = config.dream_config_path();
    if !path.exists() {
        return validate_dream_config(&default_dream_config());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| DreamConfigError::new(format!("dream.conf could not be read: {error}")))?;
    dream_config_from_text(&text)
}

/// Parse and validate dream text without touching any file: the upgrade
/// protocol's parse-back for staged dream content, where the live file must
/// never be rewritten as a side effect of loading.
pub(crate) fn dream_config_from_text(text: &str) -> Result<DreamConfig, DreamConfigError> {
    let payload = text
        .parse::<Table>()
        .map_err(|error| DreamConfigError::new(format!("dream.conf is not valid TOML: {error}")))?;
    let (payload, _) = migrate_workflow_payload(payload);
    validate_dream_config(&dream_config_from_payload(&payload)?)
}

/// Whether the payload uses pre-seven-pass legacy workflow names that the
/// upgrade's staging must migrate into the current format.
pub(crate) fn payload_has_legacy_workflows(payload: &Table) -> bool {
    const LEGACY_SOURCES: &[&str] = &[
        "crystallization",
        "relation_discovery",
        "reinforcement_compaction",
    ];
    payload
        .get("workflows")
        .and_then(|value| value.as_table())
        .map(|workflows| {
            workflows
                .keys()
                .any(|name| LEGACY_SOURCES.contains(&name.as_str()))
        })
        .unwrap_or(false)
}

/// Canonical current-format dream.conf text for a typed config. The upgrade
/// stages this when a legacy-shape file needs a structural rewrite.
pub(crate) fn dream_canonical_text(dream_config: &DreamConfig) -> Result<String, DreamConfigError> {
    toml::to_string(&dream_payload(dream_config))
        .map_err(|error| DreamConfigError::new(format!("dream.conf render failed: {error}")))
}

/// Expand pre-seven-pass workflow configurations before strict validation:
/// `[workflows.crystallization]` seeds the four crystal/candidate passes,
/// `relation_discovery` seeds `relations`, and `reinforcement_compaction`
/// seeds `reinforcement` plus `coverage_audit`.
fn migrate_workflow_payload(mut payload: Table) -> (Table, bool) {
    let legacy_sources: &[(&str, &str)] = &[
        ("concepts", "crystallization"),
        ("terminology_candidates", "crystallization"),
        ("rule_crystals", "crystallization"),
        ("knowledge_crystals", "crystallization"),
        ("relations", "relation_discovery"),
        ("reinforcement", "reinforcement_compaction"),
        ("coverage_audit", "reinforcement_compaction"),
    ];
    let Some(raw_workflows) = payload.get("workflows").and_then(|value| value.as_table()) else {
        return (payload, false);
    };
    let touches_legacy = raw_workflows
        .keys()
        .any(|name| legacy_sources.iter().any(|(_, source)| name == source));
    if !touches_legacy {
        return (payload, false);
    }

    let mut migrated_workflows = Table::new();
    for (name, value) in raw_workflows {
        if !legacy_sources.iter().any(|(_, source)| name == source) {
            migrated_workflows.insert(name.clone(), value.clone());
        }
    }
    for (name, source) in legacy_sources {
        let value = raw_workflows
            .get(*name)
            .or_else(|| raw_workflows.get(*source))
            .cloned()
            .unwrap_or_else(|| Table::new().into());
        migrated_workflows.insert((*name).to_string(), value);
    }
    payload.insert("workflows".into(), migrated_workflows.into());
    (payload, true)
}

pub fn save_dream_config(
    config: &HieronymusConfig,
    dream_config: &DreamConfig,
) -> Result<(), DreamConfigError> {
    let validated = validate_dream_config(dream_config)?;
    let text = toml::to_string(&dream_payload(&validated))
        .map_err(|error| DreamConfigError::new(format!("dream.conf render failed: {error}")))?;
    atomic_write_text(&config.dream_config_path(), &text)
        .map_err(|error| DreamConfigError::new(format!("dream.conf write failed: {error}")))?;
    Ok(())
}

/// DTO projection for daemon surfaces; the dream payload never contains
/// providers or secrets. The REST settings DTO is frozen at its compatibility
/// contract, so the newer `reconsolidation_diff_threshold` field is projected
/// out here: it round-trips through `dream.conf` and stays untouched by
/// settings drafts, which never carry it.
pub fn redacted_dream_config_payload(dream_config: &DreamConfig) -> Table {
    let mut payload = dream_payload(dream_config);
    if let Some(dreaming) = payload
        .get_mut("dreaming")
        .and_then(|value| value.as_table_mut())
    {
        dreaming.remove("reconsolidation_diff_threshold");
    }
    payload
}

/// Full file payload; `save_dream_config` writes every validated field.
fn dream_payload(dream_config: &DreamConfig) -> Table {
    let mut dreaming = Table::new();
    dreaming.insert("enabled".into(), dream_config.enabled.into());
    dreaming.insert(
        "schedule_interval_minutes".into(),
        dream_config.schedule_interval_minutes.into(),
    );
    dreaming.insert(
        "min_pending_short_term_memories".into(),
        dream_config.min_pending_short_term_memories.into(),
    );
    dreaming.insert(
        "max_pending_short_term_memories".into(),
        dream_config.max_pending_short_term_memories.into(),
    );
    dreaming.insert(
        "max_short_term_memories_per_cycle".into(),
        dream_config.max_short_term_memories_per_cycle.into(),
    );
    dreaming.insert(
        "not_enough_memories_cycle_threshold".into(),
        dream_config.not_enough_memories_cycle_threshold.into(),
    );
    dreaming.insert(
        "max_changed_crystals_per_cycle".into(),
        dream_config.max_changed_crystals_per_cycle.into(),
    );
    dreaming.insert(
        "max_related_concepts_per_cycle".into(),
        dream_config.max_related_concepts_per_cycle.into(),
    );
    dreaming.insert(
        "max_related_crystals_per_concept".into(),
        dream_config.max_related_crystals_per_concept.into(),
    );
    dreaming.insert(
        "max_total_affected_crystals".into(),
        dream_config.max_total_affected_crystals.into(),
    );
    dreaming.insert(
        "max_short_term_memories_per_run".into(),
        dream_config.max_short_term_memories_per_run.into(),
    );
    dreaming.insert(
        "max_long_term_records_affected_per_run".into(),
        dream_config.max_long_term_records_affected_per_run.into(),
    );
    dreaming.insert(
        "max_relation_records_per_pass".into(),
        dream_config.max_relation_records_per_pass.into(),
    );
    dreaming.insert(
        "reconsolidation_diff_threshold".into(),
        dream_config.reconsolidation_diff_threshold.into(),
    );
    dreaming.insert(
        "general_prompt".into(),
        dream_config.general_prompt.clone().into(),
    );
    let mut workflows = Table::new();
    for (name, workflow) in &dream_config.workflows {
        let mut payload = Table::new();
        payload.insert("provider".into(), workflow.provider.clone().into());
        payload.insert("model".into(), workflow.model.clone().into());
        payload.insert("enabled".into(), workflow.enabled.into());
        payload.insert(
            "max_records_per_pass".into(),
            workflow.max_records_per_pass.into(),
        );
        workflows.insert(name.clone(), payload.into());
    }
    let mut payload = Table::new();
    payload.insert("dreaming".into(), dreaming.into());
    payload.insert("workflows".into(), workflows.into());
    payload
}

pub fn validate_dream_config(dream_config: &DreamConfig) -> Result<DreamConfig, DreamConfigError> {
    if dream_config.workflows.len() != DREAM_WORKFLOW_NAMES.len()
        || DREAM_WORKFLOW_NAMES
            .iter()
            .any(|name| !dream_config.workflows.contains_key(*name))
    {
        return Err(DreamConfigError::new(
            "workflows must contain exactly the seven Dream passes",
        ));
    }
    require_positive_int(
        "schedule_interval_minutes",
        dream_config.schedule_interval_minutes,
    )?;
    require_positive_int(
        "min_pending_short_term_memories",
        dream_config.min_pending_short_term_memories,
    )?;
    require_positive_int(
        "max_pending_short_term_memories",
        dream_config.max_pending_short_term_memories,
    )?;
    require_positive_int(
        "max_short_term_memories_per_cycle",
        dream_config.max_short_term_memories_per_cycle,
    )?;
    require_positive_int(
        "not_enough_memories_cycle_threshold",
        dream_config.not_enough_memories_cycle_threshold,
    )?;
    require_positive_int(
        "max_changed_crystals_per_cycle",
        dream_config.max_changed_crystals_per_cycle,
    )?;
    require_positive_int(
        "max_related_concepts_per_cycle",
        dream_config.max_related_concepts_per_cycle,
    )?;
    require_positive_int(
        "max_related_crystals_per_concept",
        dream_config.max_related_crystals_per_concept,
    )?;
    require_positive_int(
        "max_total_affected_crystals",
        dream_config.max_total_affected_crystals,
    )?;
    require_positive_int(
        "max_short_term_memories_per_run",
        dream_config.max_short_term_memories_per_run,
    )?;
    require_positive_int(
        "max_long_term_records_affected_per_run",
        dream_config.max_long_term_records_affected_per_run,
    )?;
    require_positive_int(
        "max_relation_records_per_pass",
        dream_config.max_relation_records_per_pass,
    )?;
    if dream_config.max_pending_short_term_memories < dream_config.min_pending_short_term_memories {
        return Err(DreamConfigError::new(
            "max_pending_short_term_memories must be greater than or equal to \
             min_pending_short_term_memories",
        ));
    }
    if dream_config.max_short_term_memories_per_cycle > dream_config.max_pending_short_term_memories
    {
        return Err(DreamConfigError::new(
            "max_short_term_memories_per_cycle must be less than or equal to \
             max_pending_short_term_memories",
        ));
    }
    if !(0.0 < dream_config.reconsolidation_diff_threshold
        && dream_config.reconsolidation_diff_threshold <= 1.0)
    {
        return Err(DreamConfigError::new(
            "reconsolidation_diff_threshold must be greater than 0 and at most 1",
        ));
    }
    for (name, workflow) in &dream_config.workflows {
        let prefix = format!("workflows.{name}");
        require_positive_int(
            &format!("{prefix}.max_records_per_pass"),
            workflow.max_records_per_pass,
        )?;
        if workflow.enabled && workflow.model.is_empty() {
            return Err(DreamConfigError::new(format!(
                "enabled workflow must have a model: {name}"
            )));
        }
    }
    Ok(dream_config.clone())
}

fn dream_config_from_payload(payload: &Table) -> Result<DreamConfig, DreamConfigError> {
    reject_unknown_keys(payload, &["dreaming", "providers", "workflows"], None)?;
    let defaults = default_dream_config();

    let mut dream_config = defaults.clone();
    if let Some(dreaming) = optional_table(payload, "dreaming")? {
        reject_unknown_keys(dreaming, &DREAMING_FIELDS, Some("dreaming"))?;
        for (name, value) in dreaming {
            let prefix = format!("dreaming.{name}");
            match name.as_str() {
                "enabled" => dream_config.enabled = require_exact_bool(&prefix, value)?,
                "general_prompt" => {
                    dream_config.general_prompt = require_exact_str(&prefix, value)?;
                }
                "reconsolidation_diff_threshold" => {
                    dream_config.reconsolidation_diff_threshold =
                        require_exact_float(&prefix, value)?;
                }
                "workflows" => unreachable!("dreaming.workflows is not an allowed field"),
                field_name => {
                    let parsed = require_exact_int(&prefix, value)?;
                    apply_dreaming_field(&mut dream_config, field_name, parsed);
                }
            }
        }
    }
    // Deprecated `[providers.*]` payload: tolerated, never validated, never
    // written back.
    if let Some(providers) = optional_table(payload, "providers")? {
        let _ = providers;
    }

    let workflows_payload = optional_table(payload, "workflows")?;
    if let Some(workflows_payload) = workflows_payload {
        for (name, raw_workflow) in workflows_payload {
            let Some(workflow_default) = dream_config.workflows.get(name) else {
                return Err(DreamConfigError::new(
                    "workflows must contain exactly the seven Dream passes",
                ));
            };
            let prefix = format!("workflows.{name}");
            reject_unknown_keys(
                require_table(raw_workflow, &prefix)?,
                &WORKFLOW_FIELDS,
                Some(&prefix),
            )?;
            let mut workflow = workflow_default.clone();
            for (field_name, value) in require_table(raw_workflow, &prefix)? {
                let prefix = format!("{prefix}.{field_name}");
                match field_name.as_str() {
                    "provider" => workflow.provider = require_exact_str(&prefix, value)?,
                    "model" => workflow.model = require_exact_str(&prefix, value)?,
                    "enabled" => workflow.enabled = require_exact_bool(&prefix, value)?,
                    "max_records_per_pass" => {
                        workflow.max_records_per_pass = require_exact_int(&prefix, value)?;
                    }
                    _ => unreachable!("rejected by unknown-key check"),
                }
            }
            dream_config.workflows.insert(name.clone(), workflow);
        }
    }
    Ok(dream_config)
}

fn apply_dreaming_field(dream_config: &mut DreamConfig, field_name: &str, parsed: i64) {
    match field_name {
        "schedule_interval_minutes" => dream_config.schedule_interval_minutes = parsed,
        "min_pending_short_term_memories" => {
            dream_config.min_pending_short_term_memories = parsed;
        }
        "max_pending_short_term_memories" => {
            dream_config.max_pending_short_term_memories = parsed;
        }
        "max_short_term_memories_per_cycle" => {
            dream_config.max_short_term_memories_per_cycle = parsed;
        }
        "not_enough_memories_cycle_threshold" => {
            dream_config.not_enough_memories_cycle_threshold = parsed;
        }
        "max_changed_crystals_per_cycle" => dream_config.max_changed_crystals_per_cycle = parsed,
        "max_related_concepts_per_cycle" => dream_config.max_related_concepts_per_cycle = parsed,
        "max_related_crystals_per_concept" => {
            dream_config.max_related_crystals_per_concept = parsed;
        }
        "max_total_affected_crystals" => dream_config.max_total_affected_crystals = parsed,
        "max_short_term_memories_per_run" => dream_config.max_short_term_memories_per_run = parsed,
        "max_long_term_records_affected_per_run" => {
            dream_config.max_long_term_records_affected_per_run = parsed;
        }
        "max_relation_records_per_pass" => dream_config.max_relation_records_per_pass = parsed,
        _ => unreachable!("dreaming allowed-fields list and matcher disagree"),
    }
}

fn optional_table<'a>(
    payload: &'a Table,
    name: &str,
) -> Result<Option<&'a Table>, DreamConfigError> {
    match payload.get(name) {
        None => Ok(None),
        Some(value) => value
            .as_table()
            .map(Some)
            .ok_or_else(|| DreamConfigError::new(format!("{name} must be a table"))),
    }
}

fn require_table<'a>(
    value: &'a toml::Value,
    field_name: &str,
) -> Result<&'a Table, DreamConfigError> {
    value
        .as_table()
        .ok_or_else(|| DreamConfigError::new(format!("{field_name} must be a table")))
}

fn reject_unknown_keys(
    payload: &Table,
    allowed: &[&str],
    prefix: Option<&str>,
) -> Result<(), DreamConfigError> {
    for key in payload.keys() {
        if !allowed.contains(&key.as_str()) {
            let setting = match prefix {
                None => key.clone(),
                Some(prefix) => format!("{prefix}.{key}"),
            };
            return Err(DreamConfigError::new(format!(
                "unknown dream config setting: {setting}"
            )));
        }
    }
    Ok(())
}

fn require_exact_int(field_name: &str, value: &toml::Value) -> Result<i64, DreamConfigError> {
    match value {
        toml::Value::Integer(parsed) => Ok(*parsed),
        _ => Err(DreamConfigError::new(format!(
            "{field_name} must be an integer"
        ))),
    }
}

fn require_positive_int(field_name: &str, value: i64) -> Result<(), DreamConfigError> {
    if value < 1 {
        return Err(DreamConfigError::new(format!(
            "{field_name} must be at least 1"
        )));
    }
    Ok(())
}

fn require_exact_bool(field_name: &str, value: &toml::Value) -> Result<bool, DreamConfigError> {
    match value {
        toml::Value::Boolean(parsed) => Ok(*parsed),
        _ => Err(DreamConfigError::new(format!(
            "{field_name} must be a boolean"
        ))),
    }
}

fn require_exact_float(field_name: &str, value: &toml::Value) -> Result<f64, DreamConfigError> {
    match value {
        toml::Value::Float(parsed) => Ok(*parsed),
        _ => Err(DreamConfigError::new(format!(
            "{field_name} must be a float"
        ))),
    }
}

fn require_exact_str(field_name: &str, value: &toml::Value) -> Result<String, DreamConfigError> {
    match value {
        toml::Value::String(text) => Ok(text.clone()),
        _ => Err(DreamConfigError::new(format!(
            "{field_name} must be a string"
        ))),
    }
}
