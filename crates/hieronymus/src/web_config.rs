//! Optional browser authentication for the local desktop web interface.
use crate::data_root::HieronymusConfig;
use serde::Deserialize;

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebConfig {
    /// Require the existing local launch-grant/session flow when explicitly enabled.
    pub authentication_required: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum WebConfigError {
    #[error("could not read web.conf: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid web.conf: {0}")]
    Invalid(#[from] toml::de::Error),
}

/// Read once at server startup. Only a missing file uses the desktop default.
pub fn load(config: &HieronymusConfig) -> Result<WebConfig, WebConfigError> {
    match std::fs::read_to_string(config.config_root().join("web.conf")) {
        Ok(text) => Ok(toml::from_str(&text)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(WebConfig::default()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_open_and_explicit_authentication_is_strictly_typed() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        assert!(!load(&config).unwrap().authentication_required);
        std::fs::write(
            root.path().join("web.conf"),
            "authentication_required = true\n",
        )
        .unwrap();
        assert!(load(&config).unwrap().authentication_required);
        for value in [
            "authentication_required = 'true'",
            "authentication_required = true\nunknown = false",
            "not toml",
        ] {
            std::fs::write(root.path().join("web.conf"), value).unwrap();
            assert!(load(&config).is_err());
        }
    }
}
