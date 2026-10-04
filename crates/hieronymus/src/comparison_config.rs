//! Explicit pair-comparison routing, independent of extraction and prompt filtering.
use crate::{data_root::HieronymusConfig, provider_config::load_provider_catalog};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    /// `jev` uses the existing private TypeSafe credential; other names select catalog profiles.
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ComparisonConfig {
    pub primary: Option<Assignment>,
    pub fallback: Option<Assignment>,
    pub max_pairs_per_run: usize,
    pub timeout_seconds: u64,
}
impl Default for ComparisonConfig {
    fn default() -> Self {
        Self {
            primary: None,
            fallback: None,
            max_pairs_per_run: 8,
            timeout_seconds: 10,
        }
    }
}
impl ComparisonConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=32).contains(&self.max_pairs_per_run) {
            return Err("comparison requires 1–32 pairs");
        }
        if self.timeout_seconds == 0
            || Instant::now()
                .checked_add(Duration::from_secs(self.timeout_seconds))
                .is_none()
        {
            return Err("comparison timeout must be positive and representable");
        }
        if self.primary.is_none() && self.fallback.is_some() {
            return Err("fallback requires a primary assignment");
        }
        for a in self.primary.iter().chain(self.fallback.iter()) {
            if a.provider.trim().is_empty()
                || a.provider.len() > 128
                || a.model.trim().is_empty()
                || a.model.len() > 200
                || a.model.chars().any(char::is_control)
            {
                return Err("invalid comparison provider/model");
            }
        }
        Ok(())
    }
}
pub fn load(config: &HieronymusConfig) -> Result<ComparisonConfig, &'static str> {
    let bytes =
        match crate::private_file::read_private(&config.config_root().join("comparison.conf")) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ComparisonConfig::default());
            }
            Err(_) => return Err("cannot read comparison.conf"),
        };
    let value: ComparisonConfig =
        toml::from_str(std::str::from_utf8(&bytes).map_err(|_| "invalid comparison.conf")?)
            .map_err(|_| "invalid comparison.conf")?;
    value.validate()?;
    Ok(value)
}
pub fn save(config: &HieronymusConfig, value: &ComparisonConfig) -> Result<(), &'static str> {
    value.validate()?;
    crate::private_file::replace_private(
        &config.config_root().join("comparison.conf"),
        toml::to_string(value)
            .map_err(|_| "cannot encode comparison settings")?
            .as_bytes(),
    )
    .map_err(|_| "cannot save comparison settings")
}
/// Readiness means credentials/configuration exist, not that a model was qualified.
pub fn public_payload(config: &HieronymusConfig, value: &ComparisonConfig) -> Value {
    let readiness = |a: &Assignment| {
        if a.provider == "jev" {
            crate::relevance_config::load(config).is_ok_and(|v| !v.key.is_blank())
        } else {
            load_provider_catalog(config).is_ok_and(|c| {
                c.providers
                    .get(&a.provider)
                    .is_some_and(|p| p.provider_type() == "ollama" || !p.key().is_blank())
            })
        }
    };
    json!({"settings":value,"primary_ready":value.primary.as_ref().is_some_and(readiness),"fallback_ready":value.fallback.as_ref().is_some_and(readiness),"qualified":false})
}
