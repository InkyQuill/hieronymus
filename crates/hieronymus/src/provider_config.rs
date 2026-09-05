use std::collections::BTreeMap;

use toml::Table;

use crate::atomic::atomic_write_text;
use crate::data_root::HieronymusConfig;
use crate::secret::Secret;

pub const SUPPORTED_PROVIDER_TYPES: [&str; 4] = ["anthropic", "google", "ollama", "openai"];

const PROFILE_FIELDS: [&str; 5] = ["name", "type", "url", "key", "timeout_seconds"];
const DEFAULTS_FIELDS: [&str; 2] = ["provider", "model"];
const LEGACY_DREAM_PROFILE_FIELDS: [&str; 7] = [
    "name",
    "type",
    "endpoint",
    "url",
    "api_key",
    "key",
    "timeout_seconds",
];

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ProviderCatalogError {
    message: String,
}

impl ProviderCatalogError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// One provider endpoint profile. The API key is a [`Secret`]; the plaintext
/// value is written only into `provider.conf` (ADR 0001) and only by
/// [`save_provider_catalog`].
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderProfile {
    name: String,
    provider_type: String,
    url: String,
    key: Secret<String>,
    timeout_seconds: f64,
}

impl ProviderProfile {
    pub fn new(
        name: impl Into<String>,
        provider_type: impl Into<String>,
        url: impl Into<String>,
        key: impl Into<String>,
        timeout_seconds: f64,
    ) -> Self {
        Self {
            name: name.into(),
            provider_type: provider_type.into(),
            url: url.into(),
            key: Secret::new(key.into()),
            timeout_seconds,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn provider_type(&self) -> &str {
        &self.provider_type
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn key(&self) -> &Secret<String> {
        &self.key
    }

    pub fn timeout_seconds(&self) -> f64 {
        self.timeout_seconds
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderDefaults {
    pub provider: String,
    pub model: String,
}

impl ProviderDefaults {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

/// `provider.conf`: the global local catalog of provider profiles plus the
/// default provider/model assignment (ADR 0007).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderCatalog {
    pub providers: BTreeMap<String, ProviderProfile>,
    pub defaults: ProviderDefaults,
}

impl ProviderCatalog {
    pub fn with_provider(mut self, name: impl Into<String>, provider: ProviderProfile) -> Self {
        self.providers.insert(name.into(), provider);
        self
    }
}

pub fn default_provider_catalog() -> ProviderCatalog {
    ProviderCatalog::default()
}

/// Load `provider.conf`, migrating legacy `dream.conf.providers` blocks and
/// canonicalizing the removed `gemini` type to `google` (rewriting the file
/// when that legacy type is present).
pub fn load_provider_catalog(
    config: &HieronymusConfig,
) -> Result<ProviderCatalog, ProviderCatalogError> {
    let catalog = load_provider_catalog_file(config)?;
    let catalog = migrate_legacy_dream_providers(config, catalog)?;
    if provider_config_uses_legacy_gemini_type(config) {
        save_provider_catalog(config, &catalog)?;
    }
    Ok(catalog)
}

/// Rewrite dream.conf without its deprecated `[providers.*]` payload after a
/// provider migration; a broken dream config does not fail the migration.
pub fn load_and_resave_dream_config(
    config: &HieronymusConfig,
) -> Result<(), crate::dream_config::DreamConfigError> {
    let dream_config = crate::dream_config::load_dream_config(config)?;
    crate::dream_config::save_dream_config(config, &dream_config)
}

fn load_provider_catalog_file(
    config: &HieronymusConfig,
) -> Result<ProviderCatalog, ProviderCatalogError> {
    let path = config.provider_config_path();
    if !path.exists() {
        return validate_provider_catalog(&default_provider_catalog());
    }
    let text = std::fs::read_to_string(&path).map_err(|error| {
        ProviderCatalogError::new(format!("provider.conf could not be read: {error}"))
    })?;
    let payload = text.parse::<Table>().map_err(|error| {
        ProviderCatalogError::new(format!("provider.conf is not valid TOML: {error}"))
    })?;
    validate_provider_catalog(&provider_catalog_from_payload(&payload)?)
}

/// Parse and validate provider.conf text without touching any file: the
/// upgrade protocol's parse-back for staged provider content.
pub(crate) fn provider_catalog_from_text(
    text: &str,
) -> Result<ProviderCatalog, ProviderCatalogError> {
    let payload = text.parse::<Table>().map_err(|error| {
        ProviderCatalogError::new(format!("provider.conf is not valid TOML: {error}"))
    })?;
    validate_provider_catalog(&provider_catalog_from_payload(&payload)?)
}

fn migrate_legacy_dream_providers(
    config: &HieronymusConfig,
    catalog: ProviderCatalog,
) -> Result<ProviderCatalog, ProviderCatalogError> {
    let Some(providers_payload) = legacy_dream_provider_payload(config)? else {
        return Ok(catalog);
    };
    let migrated = migrate_dream_provider_payload(&providers_payload, &catalog)?;
    save_provider_catalog(config, &migrated)?;
    // Remove the migrated block from dream.conf; a broken dream config must
    // not fail the provider migration.
    let _ = load_and_resave_dream_config(config);
    Ok(migrated)
}

/// Returns `None` when dream.conf is absent, has no `providers` key, or is
/// not valid TOML (a broken dream file is ignored here; readable-but-foreign
/// state errors instead).
fn legacy_dream_provider_payload(
    config: &HieronymusConfig,
) -> Result<Option<Table>, ProviderCatalogError> {
    let path = config.dream_config_path();
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|error| {
        ProviderCatalogError::new(format!("dream.conf could not be read: {error}"))
    })?;
    let Ok(payload) = text.parse::<Table>() else {
        return Ok(None);
    };
    match payload.get("providers") {
        None => Ok(None),
        Some(value) => value
            .as_table()
            .cloned()
            .map(Some)
            .ok_or_else(|| ProviderCatalogError::new("providers must be a table")),
    }
}

pub fn save_provider_catalog(
    config: &HieronymusConfig,
    catalog: &ProviderCatalog,
) -> Result<(), ProviderCatalogError> {
    let validated = validate_provider_catalog(catalog)?;
    let payload = catalog_payload(&validated, false);
    let text = toml::to_string(&payload).map_err(|error| {
        ProviderCatalogError::new(format!("provider.conf render failed: {error}"))
    })?;
    atomic_write_text(&config.provider_config_path(), &text).map_err(|error| {
        ProviderCatalogError::new(format!("provider.conf write failed: {error}"))
    })?;
    Ok(())
}

/// DTO projection: keys are `***` when set, empty stays empty.
pub fn redacted_provider_catalog_payload(catalog: &ProviderCatalog) -> Table {
    catalog_payload(catalog, true)
}

fn catalog_payload(catalog: &ProviderCatalog, redact: bool) -> Table {
    let mut payload = Table::new();
    for (name, provider) in &catalog.providers {
        payload.insert(name.clone(), profile_payload(provider, redact).into());
    }
    let mut defaults = Table::new();
    defaults.insert("provider".into(), catalog.defaults.provider.clone().into());
    defaults.insert("model".into(), catalog.defaults.model.clone().into());
    payload.insert("defaults".into(), defaults.into());
    payload
}

fn profile_payload(provider: &ProviderProfile, redact: bool) -> Table {
    let key_value = match redact && !provider.key.expose_secret().is_empty() {
        true => "***".to_string(),
        false => provider.key.expose_secret().clone(),
    };
    let mut payload = Table::new();
    payload.insert("name".into(), provider.name.clone().into());
    payload.insert("type".into(), provider.provider_type.clone().into());
    payload.insert("url".into(), provider.url.clone().into());
    payload.insert("key".into(), key_value.into());
    payload.insert("timeout_seconds".into(), provider.timeout_seconds.into());
    payload
}

/// Convert a legacy `dream.conf.providers` payload into catalog profiles.
/// Field precedence inside each legacy table: written value, then the
/// existing profile's value, then defaults; `endpoint` maps to `url` and
/// `api_key` maps to `key`. An existing profile that would change is a
/// collision error instead of a silent overwrite.
pub fn migrate_dream_provider_payload(
    providers_payload: &Table,
    existing: &ProviderCatalog,
) -> Result<ProviderCatalog, ProviderCatalogError> {
    let mut next_catalog = existing.clone();
    for (profile_id, raw_profile) in providers_payload {
        validate_provider_id(profile_id)?;
        let table = require_table(Some(raw_profile), &format!("providers.{profile_id}"))?;
        reject_unknown_keys(
            table,
            &LEGACY_DREAM_PROFILE_FIELDS,
            &format!("providers.{profile_id}"),
        )?;
        let existing_profile = next_catalog.providers.get(profile_id);

        let mut name = match table.get("name") {
            Some(value) => Some(require_exact_str(
                &format!("providers.{profile_id}.name"),
                value,
            )?),
            None => None,
        };
        let mut provider_type = match table.get("type") {
            Some(value) => Some(require_exact_str(
                &format!("providers.{profile_id}.type"),
                value,
            )?),
            None => None,
        };
        let mut timeout = match table.get("timeout_seconds") {
            Some(value) => Some(coerce_positive_float(
                &format!("providers.{profile_id}.timeout_seconds"),
                value,
            )?),
            None => None,
        };
        let mut key = match table.get("api_key").or_else(|| table.get("key")) {
            Some(value) => Some(require_exact_str(
                &format!("providers.{profile_id}.api_key"),
                value,
            )?),
            None => None,
        };
        let mut url = match table.get("endpoint").or_else(|| table.get("url")) {
            Some(value) => Some(require_exact_str(
                &format!("providers.{profile_id}.endpoint"),
                value,
            )?),
            None => None,
        };

        if let Some(existing_profile) = existing_profile {
            name.get_or_insert_with(|| existing_profile.name.clone());
            provider_type.get_or_insert_with(|| existing_profile.provider_type.clone());
            url.get_or_insert_with(|| existing_profile.url.clone());
            key.get_or_insert_with(|| existing_profile.key.expose_secret().clone());
            timeout.get_or_insert(existing_profile.timeout_seconds);
        }
        if provider_type.is_none() {
            return Err(ProviderCatalogError::new(format!(
                "providers.{profile_id}.type is required"
            )));
        }
        if url.is_none() {
            return Err(ProviderCatalogError::new(format!(
                "providers.{profile_id}.url is required"
            )));
        }
        let profile = ProviderProfile::new(
            name.unwrap_or_else(|| title_case(profile_id)),
            provider_type.unwrap_or_default(),
            url.unwrap_or_default(),
            key.unwrap_or_default(),
            timeout.unwrap_or(30.0),
        );
        if let Some(existing_profile) = existing_profile
            && *existing_profile != profile
        {
            return Err(ProviderCatalogError::new(format!(
                "dream.conf migration would overwrite provider profile: {profile_id}"
            )));
        }
        next_catalog = next_catalog.with_provider(profile_id.clone(), profile);
    }
    validate_provider_catalog(&next_catalog)
}

pub fn validate_provider_catalog(
    catalog: &ProviderCatalog,
) -> Result<ProviderCatalog, ProviderCatalogError> {
    for (name, provider) in &catalog.providers {
        validate_provider_id(name)?;
        validate_provider_profile(name, provider)?;
    }
    validate_provider_defaults(&catalog.defaults, &catalog.providers)?;
    Ok(catalog.clone())
}

fn provider_catalog_from_payload(payload: &Table) -> Result<ProviderCatalog, ProviderCatalogError> {
    let defaults_payload = require_table(payload.get("defaults"), "defaults")?;
    reject_unknown_keys(defaults_payload, &DEFAULTS_FIELDS, "defaults")?;
    let defaults = ProviderDefaults::new(
        defaults_payload
            .get("provider")
            .map(|value| require_exact_str("defaults.provider", value))
            .transpose()?
            .unwrap_or_default(),
        defaults_payload
            .get("model")
            .map(|value| require_exact_str("defaults.model", value))
            .transpose()?
            .unwrap_or_default(),
    );

    let mut providers = BTreeMap::new();
    for (name, raw_provider) in payload {
        if name == "defaults" {
            continue;
        }
        validate_provider_id(name)?;
        let provider_payload = require_table(Some(raw_provider), name)?;
        reject_unknown_keys(provider_payload, &PROFILE_FIELDS, name)?;

        let mut name_value: Option<String> = None;
        let mut type_value: Option<String> = None;
        let mut url_value: Option<String> = None;
        let mut key_value: Option<String> = None;
        let mut timeout_value: Option<f64> = None;
        for (field_name, value) in provider_payload {
            match field_name.as_str() {
                "name" => {
                    name_value = Some(require_exact_str(&format!("providers.{name}.name"), value)?)
                }
                "type" => {
                    type_value = Some(require_exact_str(&format!("providers.{name}.type"), value)?)
                }
                "url" => {
                    url_value = Some(require_exact_str(&format!("providers.{name}.url"), value)?)
                }
                "key" => {
                    key_value = Some(require_exact_str(&format!("providers.{name}.key"), value)?)
                }
                "timeout_seconds" => {
                    timeout_value = Some(coerce_positive_float(
                        &format!("providers.{name}.timeout_seconds"),
                        value,
                    )?);
                }
                _ => unreachable!("rejected by unknown-key check"),
            }
        }
        let type_string = type_value.ok_or_else(|| {
            ProviderCatalogError::new(format!("providers.{name}.type is required"))
        })?;
        let url_string = url_value.ok_or_else(|| {
            ProviderCatalogError::new(format!("providers.{name}.url is required"))
        })?;
        let mut profile = ProviderProfile::new(
            name_value.unwrap_or_else(|| name.clone()),
            type_string,
            url_string,
            key_value.unwrap_or_default(),
            timeout_value.unwrap_or(30.0),
        );
        if profile.provider_type() == "gemini" {
            profile = ProviderProfile::new(
                profile.name().to_string(),
                "google",
                profile.url().to_string(),
                profile.key().expose_secret().clone(),
                profile.timeout_seconds(),
            );
        }
        providers.insert(name.clone(), profile);
    }

    Ok(ProviderCatalog {
        providers,
        defaults,
    })
}

fn provider_config_uses_legacy_gemini_type(config: &HieronymusConfig) -> bool {
    let path = config.provider_config_path();
    if !path.exists() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(payload) = text.parse::<Table>() else {
        return false;
    };
    payload.iter().any(|(name, profile)| {
        name != "defaults"
            && profile
                .as_table()
                .and_then(|table| table.get("type"))
                .and_then(|value| value.as_str())
                == Some("gemini")
    })
}

fn validate_provider_profile(
    name: &str,
    provider: &ProviderProfile,
) -> Result<(), ProviderCatalogError> {
    let prefix = format!("providers.{name}");
    if provider.name.is_empty() {
        return Err(ProviderCatalogError::new(format!(
            "{prefix}.name is required"
        )));
    }
    if !SUPPORTED_PROVIDER_TYPES.contains(&provider.provider_type.as_str()) {
        return Err(ProviderCatalogError::new(format!(
            "unsupported provider type for {name}: {}",
            provider.provider_type
        )));
    }
    if provider.url.is_empty() {
        return Err(ProviderCatalogError::new(format!(
            "{prefix}.url is required"
        )));
    }
    if !provider.timeout_seconds.is_finite() {
        return Err(ProviderCatalogError::new(format!(
            "{prefix}.timeout_seconds must be finite and greater than 0"
        )));
    }
    if provider.timeout_seconds <= 0.0 {
        return Err(ProviderCatalogError::new(format!(
            "{prefix}.timeout_seconds must be greater than 0"
        )));
    }
    Ok(())
}

fn validate_provider_defaults(
    defaults: &ProviderDefaults,
    providers: &BTreeMap<String, ProviderProfile>,
) -> Result<(), ProviderCatalogError> {
    if !defaults.provider.is_empty() && !providers.contains_key(&defaults.provider) {
        return Err(ProviderCatalogError::new(format!(
            "default provider is missing: {}",
            defaults.provider
        )));
    }
    Ok(())
}

fn validate_provider_id(name: &str) -> Result<(), ProviderCatalogError> {
    let valid = !name.is_empty()
        && name != "defaults"
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        });
    if valid {
        Ok(())
    } else {
        Err(ProviderCatalogError::new(format!(
            "invalid provider id: {name}"
        )))
    }
}

fn require_table<'a>(
    value: Option<&'a toml::Value>,
    field_name: &str,
) -> Result<&'a Table, ProviderCatalogError> {
    match value {
        None => {
            static EMPTY: std::sync::OnceLock<Table> = std::sync::OnceLock::new();
            Ok(EMPTY.get_or_init(Table::new))
        }
        Some(value) => value
            .as_table()
            .ok_or_else(|| ProviderCatalogError::new(format!("{field_name} must be a table"))),
    }
}

fn reject_unknown_keys(
    payload: &Table,
    allowed: &[&str],
    prefix: &str,
) -> Result<(), ProviderCatalogError> {
    for key in payload.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ProviderCatalogError::new(format!(
                "unknown provider config setting: {prefix}.{key}"
            )));
        }
    }
    Ok(())
}

fn require_exact_str(
    field_name: &str,
    value: &toml::Value,
) -> Result<String, ProviderCatalogError> {
    match value {
        toml::Value::String(text) => Ok(text.clone()),
        _ => Err(ProviderCatalogError::new(format!(
            "{field_name} must be a string"
        ))),
    }
}

fn coerce_positive_float(
    field_name: &str,
    value: &toml::Value,
) -> Result<f64, ProviderCatalogError> {
    let number = match value {
        toml::Value::Integer(parsed) => *parsed as f64,
        toml::Value::Float(parsed) => *parsed,
        _ => {
            return Err(ProviderCatalogError::new(format!(
                "{field_name} must be a number"
            )));
        }
    };
    if !number.is_finite() {
        return Err(ProviderCatalogError::new(format!(
            "{field_name} must be finite and greater than 0"
        )));
    }
    if number <= 0.0 {
        return Err(ProviderCatalogError::new(format!(
            "{field_name} must be greater than 0"
        )));
    }
    Ok(number)
}

/// Python `str.title()` over the underscore-to-space form: every word's first
/// character is uppercased, the rest lowercased ("openai" -> "Openai").
fn title_case(profile_id: &str) -> String {
    profile_id
        .replace('_', " ")
        .split(' ')
        .map(|word| {
            let mut characters = word.chars();
            match characters.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &characters.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
