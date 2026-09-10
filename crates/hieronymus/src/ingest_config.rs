use toml::Table;

use crate::atomic::atomic_write_text;
use crate::data_root::HieronymusConfig;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct IngestConfigError {
    message: String,
}

impl IngestConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Short-memory ingestion thresholds. Sentence counts are always active;
/// a zero symbol threshold means "disabled", which is why symbol-order
/// validation only compares active limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortMemoryLimits {
    pub warning_sentence_count: i64,
    pub rejection_sentence_count: i64,
    pub warning_symbol_count: i64,
    pub rejection_symbol_count: i64,
}

impl ShortMemoryLimits {
    pub const fn new(
        warning_sentence_count: i64,
        rejection_sentence_count: i64,
        warning_symbol_count: i64,
        rejection_symbol_count: i64,
    ) -> Self {
        Self {
            warning_sentence_count,
            rejection_sentence_count,
            warning_symbol_count,
            rejection_symbol_count,
        }
    }
}

impl Default for ShortMemoryLimits {
    fn default() -> Self {
        Self::new(6, 30, 0, 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnLimits {
    pub max_block_chars: i64,
}

impl LearnLimits {
    pub const fn new(max_block_chars: i64) -> Self {
        Self { max_block_chars }
    }
}

impl Default for LearnLimits {
    fn default() -> Self {
        Self::new(1200)
    }
}

/// `ingest.conf`: global ingestion policy — short-memory thresholds and
/// Learn-style block splitting limits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IngestConfig {
    pub short_memory: ShortMemoryLimits,
    pub learn: LearnLimits,
}

impl IngestConfig {
    pub fn new(short_memory: ShortMemoryLimits, learn: LearnLimits) -> Self {
        Self {
            short_memory,
            learn,
        }
    }
}

pub fn default_ingest_config() -> IngestConfig {
    IngestConfig::default()
}

pub fn load_ingest_config(config: &HieronymusConfig) -> Result<IngestConfig, IngestConfigError> {
    let path = config.ingest_config_path();
    if !path.exists() {
        return validate_ingest_config(&default_ingest_config());
    }

    let text = std::fs::read_to_string(&path).map_err(|error| {
        IngestConfigError::new(format!("ingest.conf could not be read: {error}"))
    })?;
    let payload = text.parse::<Table>().map_err(|error| {
        IngestConfigError::new(format!("ingest.conf is not valid TOML: {error}"))
    })?;
    validate_ingest_config(&ingest_config_from_payload(&payload)?)
}

pub fn save_ingest_config(
    config: &HieronymusConfig,
    ingest_config: &IngestConfig,
) -> Result<(), IngestConfigError> {
    let validated = validate_ingest_config(ingest_config)?;
    let payload = to_payload(&validated);
    let text = toml::to_string(&payload)
        .map_err(|error| IngestConfigError::new(format!("ingest.conf render failed: {error}")))?;
    atomic_write_text(&config.ingest_config_path(), &text)
        .map_err(|error| IngestConfigError::new(format!("ingest.conf write failed: {error}")))?;
    Ok(())
}

/// Parse and validate ingest.conf text without touching any file: the
/// upgrade protocol's typed round-trip for staged ingest content.
pub(crate) fn ingest_config_from_text(text: &str) -> Result<IngestConfig, IngestConfigError> {
    let payload = text.parse::<Table>().map_err(|error| {
        IngestConfigError::new(format!("ingest.conf is not valid TOML: {error}"))
    })?;
    validate_ingest_config(&ingest_config_from_payload(&payload)?)
}

/// Canonical current-format ingest.conf text for a typed config.
pub(crate) fn ingest_canonical_text(
    ingest_config: &IngestConfig,
) -> Result<String, IngestConfigError> {
    toml::to_string(&to_payload(ingest_config))
        .map_err(|error| IngestConfigError::new(format!("ingest.conf render failed: {error}")))
}

pub fn validate_ingest_config(
    ingest_config: &IngestConfig,
) -> Result<IngestConfig, IngestConfigError> {
    require_minimum(
        "short_memory.warning_sentence_count",
        ingest_config.short_memory.warning_sentence_count,
        1,
    )?;
    require_minimum(
        "short_memory.rejection_sentence_count",
        ingest_config.short_memory.rejection_sentence_count,
        1,
    )?;
    require_minimum(
        "short_memory.warning_symbol_count",
        ingest_config.short_memory.warning_symbol_count,
        0,
    )?;
    require_minimum(
        "short_memory.rejection_symbol_count",
        ingest_config.short_memory.rejection_symbol_count,
        0,
    )?;
    require_minimum(
        "learn.max_block_chars",
        ingest_config.learn.max_block_chars,
        1,
    )?;
    if ingest_config.short_memory.rejection_sentence_count
        < ingest_config.short_memory.warning_sentence_count
    {
        return Err(IngestConfigError::new(
            "short_memory.rejection_sentence_count must be greater than or equal to \
             short_memory.warning_sentence_count",
        ));
    }
    if ingest_config.short_memory.warning_symbol_count != 0
        && ingest_config.short_memory.rejection_symbol_count != 0
        && ingest_config.short_memory.rejection_symbol_count
            < ingest_config.short_memory.warning_symbol_count
    {
        // A zero symbol threshold is treated as disabled, so compare only active limits.
        return Err(IngestConfigError::new(
            "short_memory.rejection_symbol_count must be greater than or equal to \
             short_memory.warning_symbol_count",
        ));
    }
    Ok(ingest_config.clone())
}

fn ingest_config_from_payload(payload: &Table) -> Result<IngestConfig, IngestConfigError> {
    reject_unknown_keys(payload, &["short_memory", "learn"], None)?;
    let defaults = default_ingest_config();

    let mut ingest_config = defaults;
    if let Some(section) = optional_table(payload, "short_memory")? {
        reject_unknown_keys(
            section,
            &[
                "warning_sentence_count",
                "rejection_sentence_count",
                "warning_symbol_count",
                "rejection_symbol_count",
            ],
            Some("short_memory"),
        )?;
        for (name, value) in section {
            let parsed = require_exact_int(&format!("short_memory.{name}"), value)?;
            match name.as_str() {
                "warning_sentence_count" => {
                    ingest_config.short_memory.warning_sentence_count = parsed;
                }
                "rejection_sentence_count" => {
                    ingest_config.short_memory.rejection_sentence_count = parsed;
                }
                "warning_symbol_count" => {
                    ingest_config.short_memory.warning_symbol_count = parsed;
                }
                "rejection_symbol_count" => {
                    ingest_config.short_memory.rejection_symbol_count = parsed;
                }
                _ => unreachable!("rejected by unknown-key check"),
            }
        }
    }
    if let Some(section) = optional_table(payload, "learn")? {
        reject_unknown_keys(section, &["max_block_chars"], Some("learn"))?;
        for (name, value) in section {
            let parsed = require_exact_int(&format!("learn.{name}"), value)?;
            match name.as_str() {
                "max_block_chars" => ingest_config.learn.max_block_chars = parsed,
                _ => unreachable!("rejected by unknown-key check"),
            }
        }
    }
    Ok(ingest_config)
}

fn to_payload(ingest_config: &IngestConfig) -> Table {
    let mut short_memory = Table::new();
    short_memory.insert(
        "warning_sentence_count".into(),
        ingest_config.short_memory.warning_sentence_count.into(),
    );
    short_memory.insert(
        "rejection_sentence_count".into(),
        ingest_config.short_memory.rejection_sentence_count.into(),
    );
    short_memory.insert(
        "warning_symbol_count".into(),
        ingest_config.short_memory.warning_symbol_count.into(),
    );
    short_memory.insert(
        "rejection_symbol_count".into(),
        ingest_config.short_memory.rejection_symbol_count.into(),
    );
    let mut learn = Table::new();
    learn.insert(
        "max_block_chars".into(),
        ingest_config.learn.max_block_chars.into(),
    );
    let mut payload = Table::new();
    payload.insert("short_memory".into(), short_memory.into());
    payload.insert("learn".into(), learn.into());
    payload
}

/// Borrow the named section as a table; `None` means "absent, use defaults".
/// A present non-table value is the Python `type(v) is not dict` error
/// (booleans and other scalars included).
fn optional_table<'a>(
    payload: &'a Table,
    name: &str,
) -> Result<Option<&'a Table>, IngestConfigError> {
    match payload.get(name) {
        None => Ok(None),
        Some(value) => value
            .as_table()
            .map(Some)
            .ok_or_else(|| IngestConfigError::new(format!("{name} must be a table"))),
    }
}

fn reject_unknown_keys(
    payload: &Table,
    allowed: &[&str],
    prefix: Option<&str>,
) -> Result<(), IngestConfigError> {
    for key in payload.keys() {
        if !allowed.contains(&key.as_str()) {
            let setting = match prefix {
                None => key.clone(),
                Some(prefix) => format!("{prefix}.{key}"),
            };
            return Err(IngestConfigError::new(format!(
                "unknown ingest config setting: {setting}"
            )));
        }
    }
    Ok(())
}

fn require_exact_int(field_name: &str, value: &toml::Value) -> Result<i64, IngestConfigError> {
    // Booleans are not integers (the Python reference uses `type(v) is int`,
    // which rejects `True`).
    match value {
        toml::Value::Integer(parsed) => Ok(*parsed),
        _ => Err(IngestConfigError::new(format!(
            "{field_name} must be an integer"
        ))),
    }
}

fn require_minimum(field_name: &str, value: i64, minimum: i64) -> Result<(), IngestConfigError> {
    if value < minimum {
        return Err(IngestConfigError::new(format!(
            "{field_name} must be at least {minimum}"
        )));
    }
    Ok(())
}
