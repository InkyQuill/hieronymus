use toml::Table;

use crate::atomic::atomic_write_text;
use crate::data_root::HieronymusConfig;

const UPDATE_CHANNELS: [&str; 2] = ["stable", "dev"];

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ReleaseConfigError {
    message: String,
}

impl ReleaseConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// `release.conf`: authoritative `updates.channel` selection
/// (`"stable"` or `"dev"`, ADR 0006). Dev channel tracks `main`, stable tracks
/// the latest signed release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseConfig {
    update_channel: String,
}

impl ReleaseConfig {
    pub fn new(update_channel: impl Into<String>) -> Self {
        Self {
            update_channel: update_channel.into(),
        }
    }

    pub fn update_channel(&self) -> &str {
        &self.update_channel
    }

    pub fn update_target(&self) -> &'static str {
        if self.update_channel == "dev" {
            "main"
        } else {
            "latest"
        }
    }

    pub fn allows_dev_updates(&self) -> bool {
        self.update_channel == "dev"
    }

    pub fn with_update_channel(&self, update_channel: impl Into<String>) -> Self {
        Self {
            update_channel: update_channel.into(),
        }
    }
}

pub fn default_release_config() -> ReleaseConfig {
    ReleaseConfig::new("stable")
}

pub fn load_release_config(config: &HieronymusConfig) -> Result<ReleaseConfig, ReleaseConfigError> {
    let path = config.release_config_path();
    if !path.exists() {
        return validate_release_config(&default_release_config());
    }

    let text = std::fs::read_to_string(&path).map_err(|error| {
        ReleaseConfigError::new(format!("release.conf could not be read: {error}"))
    })?;
    let payload = parse_toml(&text)?;
    validate_release_config(&release_config_from_payload(&payload)?)
}

pub fn save_release_config(
    config: &HieronymusConfig,
    release_config: &ReleaseConfig,
) -> Result<(), ReleaseConfigError> {
    let validated = validate_release_config(release_config)?;
    let payload = to_payload(&validated);
    let text = toml::to_string(&payload)
        .map_err(|error| ReleaseConfigError::new(format!("release.conf render failed: {error}")))?;
    atomic_write_text(&config.release_config_path(), &text)
        .map_err(|error| ReleaseConfigError::new(format!("release.conf write failed: {error}")))?;
    Ok(())
}

pub fn validate_release_config(
    release_config: &ReleaseConfig,
) -> Result<ReleaseConfig, ReleaseConfigError> {
    if !UPDATE_CHANNELS.contains(&release_config.update_channel.as_str()) {
        let mut allowed = UPDATE_CHANNELS;
        allowed.sort_unstable();
        return Err(ReleaseConfigError::new(format!(
            "updates.channel must be one of: {}",
            allowed.join(", ")
        )));
    }
    Ok(release_config.clone())
}

fn parse_toml(text: &str) -> Result<Table, ReleaseConfigError> {
    text.parse().map_err(|error| {
        ReleaseConfigError::new(format!("release.conf is not valid TOML: {error}"))
    })
}

fn to_payload(release_config: &ReleaseConfig) -> Table {
    let mut channel = Table::new();
    channel.insert(
        "channel".into(),
        release_config.update_channel.clone().into(),
    );
    let mut payload = Table::new();
    payload.insert("updates".into(), channel.into());
    payload
}

fn release_config_from_payload(payload: &Table) -> Result<ReleaseConfig, ReleaseConfigError> {
    for key in payload.keys() {
        if key != "updates" {
            return Err(ReleaseConfigError::new(format!(
                "unknown release config setting: {key}"
            )));
        }
    }
    let updates = match payload.get("updates") {
        None => return Ok(default_release_config()),
        Some(value) => value
            .as_table()
            .ok_or_else(|| ReleaseConfigError::new("updates must be a table"))?,
    };
    for key in updates.keys() {
        if key != "channel" {
            return Err(ReleaseConfigError::new(format!(
                "unknown release config setting: updates.{key}"
            )));
        }
    }
    match updates.get("channel") {
        None => Ok(default_release_config()),
        Some(value) => {
            let channel = value
                .as_str()
                .ok_or_else(|| ReleaseConfigError::new("updates.channel must be a string"))?;
            Ok(ReleaseConfig::new(channel))
        }
    }
}
