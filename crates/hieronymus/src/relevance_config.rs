//! Optional remote classification for prompt capture. Public projections never expose the key.
use crate::{data_root::HieronymusConfig, secret::Secret};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct RelevanceConfig {
    pub key: Secret<String>,
    pub model: String,
    pub minimum_relevance: f64,
    pub maximum_technical: f64,
    pub timeout_seconds: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Stored {
    api_key: String,
    model: String,
    minimum_relevance: f64,
    maximum_technical: f64,
    timeout_seconds: u64,
}
impl Default for Stored {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "jev-1.13.0".into(),
            minimum_relevance: 0.85,
            maximum_technical: 0.15,
            timeout_seconds: 5,
        }
    }
}
impl From<Stored> for RelevanceConfig {
    fn from(v: Stored) -> Self {
        Self {
            key: Secret::new(v.api_key),
            model: v.model,
            minimum_relevance: v.minimum_relevance,
            maximum_technical: v.maximum_technical,
            timeout_seconds: v.timeout_seconds,
        }
    }
}
impl Default for RelevanceConfig {
    fn default() -> Self {
        Stored::default().into()
    }
}
fn validate(v: RelevanceConfig) -> Result<RelevanceConfig, &'static str> {
    if v.model.is_empty()
        || v.model.len() > 128
        || !v
            .model
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-._".contains(&c))
    {
        return Err("invalid Jev model ID");
    }
    if !v.minimum_relevance.is_finite()
        || !(0.5..=1.0).contains(&v.minimum_relevance)
        || !v.maximum_technical.is_finite()
        || !(0.0..=0.5).contains(&v.maximum_technical)
    {
        return Err("relevance threshold must be 0.5–1; technical threshold must be 0–0.5");
    }
    if !(1..=10).contains(&v.timeout_seconds) {
        return Err("Jev timeout must be 1–10 seconds");
    }
    if v.key.expose_secret().len() > 4096 || v.key.expose_secret().chars().any(char::is_control) {
        return Err("invalid Jev API key");
    }
    Ok(v)
}
/// Missing configuration keeps capture local. Errors never include file contents or credentials.
pub fn load(config: &HieronymusConfig) -> Result<RelevanceConfig, &'static str> {
    let text = match crate::private_file::read_private(&config.config_root().join("relevance.conf"))
    {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RelevanceConfig::default()),
        Err(_) => return Err("cannot read relevance.conf"),
    };
    let text = std::str::from_utf8(&text).map_err(|_| "invalid relevance.conf")?;
    let stored: Stored = toml::from_str(text).map_err(|_| "invalid relevance.conf")?;
    validate(stored.into())
}
/// Blank key drafts preserve the saved key; explicit clear_key removes it.
pub fn apply_draft(base: RelevanceConfig, draft: &Value) -> Result<RelevanceConfig, &'static str> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Draft {
        api_key: String,
        clear_key: bool,
        model: String,
        minimum_relevance: f64,
        maximum_technical: f64,
        timeout_seconds: u64,
    }
    let draft: Draft =
        serde_json::from_value(draft.clone()).map_err(|_| "invalid relevance settings")?;
    let key = if draft.clear_key {
        Secret::new(String::new())
    } else if draft.api_key.trim().is_empty() {
        base.key
    } else {
        Secret::new(draft.api_key.trim().into())
    };
    validate(RelevanceConfig {
        key,
        model: draft.model,
        minimum_relevance: draft.minimum_relevance,
        maximum_technical: draft.maximum_technical,
        timeout_seconds: draft.timeout_seconds,
    })
}
/// Save credentials in a private atomic file, never through public serialization.
pub fn save(config: &HieronymusConfig, value: &RelevanceConfig) -> Result<(), &'static str> {
    let value = validate(value.clone())?;
    let stored = Stored {
        api_key: value.key.expose_secret().clone(),
        model: value.model,
        minimum_relevance: value.minimum_relevance,
        maximum_technical: value.maximum_technical,
        timeout_seconds: value.timeout_seconds,
    };
    let text = toml::to_string(&stored).map_err(|_| "cannot encode relevance.conf")?;
    crate::private_file::replace_private(
        &config.config_root().join("relevance.conf"),
        text.as_bytes(),
    )
    .map_err(|_| "cannot save relevance.conf")
}
/// Browser settings contain presence only; plaintext secrets cannot round-trip back to the browser.
pub fn public_payload(value: &RelevanceConfig) -> Value {
    json!({"key_configured": !value.key.is_blank(), "model": value.model, "minimum_relevance": value.minimum_relevance, "maximum_technical": value.maximum_technical, "timeout_seconds": value.timeout_seconds})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_preserve_clear_and_redact_credentials() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let mut draft = json!({"api_key":"secret-test-key", "clear_key":false,"model":"jev-1.13.0", "minimum_relevance":0.85,"maximum_technical":0.15,"timeout_seconds":5});
        let value = apply_draft(load(&config).unwrap(), &draft).unwrap();
        save(&config, &value).unwrap();
        assert!(
            !public_payload(&load(&config).unwrap())
                .to_string()
                .contains("secret-test-key")
        );
        assert!(!format!("{value:?}").contains("secret-test-key"));
        draft["api_key"] = json!("");
        assert!(
            !apply_draft(load(&config).unwrap(), &draft)
                .unwrap()
                .key
                .is_blank()
        );
        draft["clear_key"] = json!(true);
        assert!(apply_draft(value, &draft).unwrap().key.is_blank());
    }
    #[cfg(unix)]
    #[test]
    fn secret_config_rejects_permissive_files_and_aliases() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        save(&config, &RelevanceConfig::default()).unwrap();
        let path = root.path().join("relevance.conf");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(load(&config).unwrap_err(), "cannot read relevance.conf");
        std::fs::rename(&path, root.path().join("other.conf")).unwrap();
        symlink(root.path().join("other.conf"), &path).unwrap();
        assert_eq!(load(&config).unwrap_err(), "cannot read relevance.conf");
    }

    #[test]
    fn invalid_files_and_drafts_never_echo_secrets() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        crate::private_file::replace_private(
            &root.path().join("relevance.conf"),
            b"api_key = secret-test-key",
        )
        .unwrap();
        assert_eq!(load(&config).unwrap_err(), "invalid relevance.conf");
        assert!(
            apply_draft(
                RelevanceConfig::default(),
                &json!({"api_key":"secret-test-key"})
            )
            .is_err()
        );
    }
}
