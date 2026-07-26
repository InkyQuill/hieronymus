use std::{env, ffi::OsString, fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::provider::{ProviderError, secure_read_bounded, secure_write};

const APPLICATION_DIR: &str = "hieronymus";
const MAX_RELEASE_CONFIG_BYTES: usize = 64 * 1024;

trait Environment {
    fn var_os(&self, key: &str) -> Option<OsString>;
}

fn environment_path(environment: &impl Environment, key: &str) -> Option<PathBuf> {
    environment
        .var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn absolute_environment_path(environment: &impl Environment, key: &str) -> Option<PathBuf> {
    environment_path(environment, key).filter(|path| path.is_absolute())
}

struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn var_os(&self, key: &str) -> Option<OsString> {
        env::var_os(key)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("HOME is required when an XDG root is not configured")]
    MissingHome,
    #[error("failed to create required directory `{path}`: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HieronymusConfig {
    pub data_root: PathBuf,
    config_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseConfig {
    pub update_channel: String,
}

impl Default for ReleaseConfig {
    fn default() -> Self {
        Self {
            update_channel: "stable".into(),
        }
    }
}

impl ReleaseConfig {
    pub fn load(config: &HieronymusConfig) -> Result<Self, ProviderError> {
        let Some(contents) =
            secure_read_bounded(&config.release_config_path(), MAX_RELEASE_CONFIG_BYTES)?
        else {
            return Ok(Self::default());
        };
        let persisted: PersistedRelease =
            toml::from_str(&contents).map_err(|error| ProviderError::Config(error.to_string()))?;
        Self {
            update_channel: persisted.updates.channel,
        }
        .validate()
    }

    pub fn save(&self, config: &HieronymusConfig) -> Result<(), ProviderError> {
        let release = self.clone().validate()?;
        let contents = toml::to_string_pretty(&PersistedRelease {
            updates: PersistedUpdates {
                channel: release.update_channel,
            },
        })
        .map_err(|error| ProviderError::Config(error.to_string()))?;
        secure_write(&config.release_config_path(), contents.as_bytes())
    }

    fn validate(self) -> Result<Self, ProviderError> {
        if matches!(self.update_channel.as_str(), "stable" | "dev") {
            Ok(self)
        } else {
            Err(ProviderError::Config(
                "release update channel must be stable or dev".into(),
            ))
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedRelease {
    updates: PersistedUpdates,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedUpdates {
    channel: String,
}

impl HieronymusConfig {
    #[must_use]
    pub fn with_roots(data_root: PathBuf, config_root: PathBuf) -> Self {
        Self {
            data_root,
            config_root,
        }
    }

    pub fn load(data_root: Option<PathBuf>) -> Result<Self, ConfigError> {
        Self::load_with_environment(data_root, &ProcessEnvironment)
    }

    fn load_with_environment(
        data_root: Option<PathBuf>,
        environment: &impl Environment,
    ) -> Result<Self, ConfigError> {
        let data_home = || absolute_environment_path(environment, "HOME");
        let data_root = data_root
            .filter(|path| !path.as_os_str().is_empty())
            .or_else(|| environment_path(environment, "HIERONYMUS_DATA_ROOT"))
            .or_else(|| {
                absolute_environment_path(environment, "XDG_DATA_HOME")
                    .map(|root| root.join(APPLICATION_DIR))
            })
            .or_else(|| data_home().map(|root| root.join(".local/share").join(APPLICATION_DIR)))
            .ok_or(ConfigError::MissingHome)?;
        let config_root = absolute_environment_path(environment, "XDG_CONFIG_HOME")
            .map(|root| root.join(APPLICATION_DIR))
            .or_else(|| {
                absolute_environment_path(environment, "HOME")
                    .map(|root| root.join(".config").join(APPLICATION_DIR))
            })
            .ok_or(ConfigError::MissingHome)?;

        Ok(Self {
            data_root,
            config_root,
        })
    }

    #[must_use]
    pub fn config_root(&self) -> PathBuf {
        self.config_root.clone()
    }

    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.data_path("hieronymus.db")
    }

    #[must_use]
    pub fn provider_config_path(&self) -> PathBuf {
        self.config_path("provider.conf")
    }

    #[must_use]
    pub fn dream_config_path(&self) -> PathBuf {
        self.config_path("dream.conf")
    }

    #[must_use]
    pub fn ingest_config_path(&self) -> PathBuf {
        self.config_path("ingest.conf")
    }

    #[must_use]
    pub fn semantic_config_path(&self) -> PathBuf {
        self.config_path("semantic.conf")
    }

    #[must_use]
    pub fn release_config_path(&self) -> PathBuf {
        self.config_path("release.conf")
    }

    #[must_use]
    pub fn llm_cache_path(&self) -> PathBuf {
        self.config_path("llm-cache.json")
    }

    #[must_use]
    pub fn auth_token_path(&self) -> PathBuf {
        self.config_path("auth-token")
    }

    #[must_use]
    pub fn lancedb_dir(&self) -> PathBuf {
        self.data_path("lancedb")
    }

    #[must_use]
    pub fn models_dir(&self) -> PathBuf {
        self.data_path("models")
    }

    #[must_use]
    pub fn daemon_log_path(&self) -> PathBuf {
        self.data_path("daemon.log")
    }

    #[must_use]
    pub fn agent_plugins_dir(&self) -> PathBuf {
        self.data_path("agent-plugins")
    }

    #[must_use]
    pub fn dream_cycle_lock_path(&self) -> PathBuf {
        self.data_path("dream-cycle.lock")
    }

    #[must_use]
    pub fn dream_cycle_state_path(&self) -> PathBuf {
        self.data_path("dream-cycle.json")
    }

    pub fn ensure_directories(&self) -> Result<(), ConfigError> {
        for path in [
            self.data_root.clone(),
            self.config_root(),
            self.lancedb_dir(),
            self.models_dir(),
            self.agent_plugins_dir(),
        ] {
            fs::create_dir_all(&path)
                .map_err(|source| ConfigError::CreateDirectory { path, source })?;
        }
        Ok(())
    }

    #[must_use]
    pub fn resolve_port(&self, cli_arg: Option<u16>) -> u16 {
        Self::resolve_port_with_environment(cli_arg, &ProcessEnvironment)
    }

    fn resolve_port_with_environment(cli_arg: Option<u16>, environment: &impl Environment) -> u16 {
        cli_arg
            .or_else(|| {
                environment
                    .var_os("HIERONYMUS_PORT")
                    .and_then(|value| value.into_string().ok())
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or(9768)
    }

    fn data_path(&self, filename: &str) -> PathBuf {
        self.data_root.join(filename)
    }

    fn config_path(&self, filename: &str) -> PathBuf {
        self.config_root.join(filename)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        ffi::OsString,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    use super::{Environment, HieronymusConfig};

    #[derive(Default)]
    struct TestEnvironment {
        values: HashMap<&'static str, OsString>,
    }

    impl TestEnvironment {
        fn with(mut self, key: &'static str, value: impl Into<OsString>) -> Self {
            self.values.insert(key, value.into());
            self
        }
    }

    impl Environment for TestEnvironment {
        fn var_os(&self, key: &str) -> Option<OsString> {
            self.values.get(key).cloned()
        }
    }

    #[test]
    fn data_root_follows_documented_precedence() {
        let cases = [
            (
                Some(PathBuf::from("/explicit")),
                TestEnvironment::default()
                    .with("HIERONYMUS_DATA_ROOT", "/env")
                    .with("XDG_DATA_HOME", "/xdg")
                    .with("HOME", "/home/test"),
                PathBuf::from("/explicit"),
            ),
            (
                None,
                TestEnvironment::default()
                    .with("HIERONYMUS_DATA_ROOT", "/env")
                    .with("XDG_DATA_HOME", "/xdg")
                    .with("HOME", "/home/test"),
                PathBuf::from("/env"),
            ),
            (
                None,
                TestEnvironment::default()
                    .with("XDG_DATA_HOME", "/xdg")
                    .with("HOME", "/home/test"),
                PathBuf::from("/xdg/hieronymus"),
            ),
            (
                None,
                TestEnvironment::default().with("HOME", "/home/test"),
                PathBuf::from("/home/test/.local/share/hieronymus"),
            ),
        ];

        for (explicit, environment, expected) in cases {
            let config = HieronymusConfig::load_with_environment(explicit, &environment)
                .expect("configuration should resolve");
            assert_eq!(config.data_root, expected);
        }
    }

    #[test]
    fn data_root_ignores_invalid_xdg_and_home_values() {
        let cases = [
            (
                TestEnvironment::default()
                    .with("XDG_DATA_HOME", "/xdg")
                    .with("HOME", "/home/test"),
                Ok(PathBuf::from("/xdg/hieronymus")),
            ),
            (
                TestEnvironment::default()
                    .with("HIERONYMUS_DATA_ROOT", "")
                    .with("XDG_DATA_HOME", "")
                    .with("HOME", "/home/test"),
                Ok(PathBuf::from("/home/test/.local/share/hieronymus")),
            ),
            (
                TestEnvironment::default()
                    .with("XDG_DATA_HOME", "relative/xdg")
                    .with("HOME", "/home/test"),
                Ok(PathBuf::from("/home/test/.local/share/hieronymus")),
            ),
            (
                TestEnvironment::default().with("HOME", "/home/test"),
                Ok(PathBuf::from("/home/test/.local/share/hieronymus")),
            ),
            (
                TestEnvironment::default()
                    .with("XDG_DATA_HOME", "")
                    .with("HOME", "relative/home")
                    .with("XDG_CONFIG_HOME", "/config"),
                Err(()),
            ),
            (
                TestEnvironment::default().with("XDG_CONFIG_HOME", "/config"),
                Err(()),
            ),
        ];

        for (environment, expected) in cases {
            let actual = HieronymusConfig::load_with_environment(None, &environment);
            match expected {
                Ok(expected_root) => {
                    assert_eq!(
                        actual.expect("configuration should resolve").data_root,
                        expected_root
                    )
                }
                Err(()) => assert!(matches!(actual, Err(super::ConfigError::MissingHome))),
            }
        }
    }

    #[test]
    fn data_root_treats_an_empty_explicit_path_as_unset() {
        let environment = TestEnvironment::default()
            .with("XDG_DATA_HOME", "/xdg")
            .with("XDG_CONFIG_HOME", "/config");

        let config = HieronymusConfig::load_with_environment(Some(PathBuf::new()), &environment)
            .expect("configuration should fall back from an empty explicit path");

        assert_eq!(config.data_root, PathBuf::from("/xdg/hieronymus"));
    }

    #[test]
    fn config_root_uses_xdg_config_home_separately_from_data_root() {
        let environment = TestEnvironment::default()
            .with("HIERONYMUS_DATA_ROOT", "/data")
            .with("XDG_CONFIG_HOME", "/config")
            .with("HOME", "/home/test");

        let config = HieronymusConfig::load_with_environment(None, &environment)
            .expect("configuration should resolve");

        assert_eq!(config.config_root(), PathBuf::from("/config/hieronymus"));
    }

    #[test]
    fn config_root_ignores_invalid_xdg_and_home_values() {
        let cases = [
            (
                TestEnvironment::default()
                    .with("XDG_CONFIG_HOME", "/config")
                    .with("HOME", "/home/test"),
                Ok(PathBuf::from("/config/hieronymus")),
            ),
            (
                TestEnvironment::default()
                    .with("XDG_CONFIG_HOME", "")
                    .with("HOME", "/home/test"),
                Ok(PathBuf::from("/home/test/.config/hieronymus")),
            ),
            (
                TestEnvironment::default()
                    .with("XDG_CONFIG_HOME", "relative/config")
                    .with("HOME", "/home/test"),
                Ok(PathBuf::from("/home/test/.config/hieronymus")),
            ),
            (
                TestEnvironment::default().with("HOME", "/home/test"),
                Ok(PathBuf::from("/home/test/.config/hieronymus")),
            ),
            (
                TestEnvironment::default()
                    .with("XDG_CONFIG_HOME", "")
                    .with("HOME", "relative/home"),
                Err(()),
            ),
            (TestEnvironment::default(), Err(())),
        ];

        for (environment, expected) in cases {
            let actual =
                HieronymusConfig::load_with_environment(Some(PathBuf::from("/data")), &environment);
            match expected {
                Ok(expected_root) => assert_eq!(
                    actual.expect("configuration should resolve").config_root(),
                    expected_root
                ),
                Err(()) => assert!(matches!(actual, Err(super::ConfigError::MissingHome))),
            }
        }
    }

    #[test]
    fn derived_paths_use_their_canonical_roots_and_filenames() {
        let environment = TestEnvironment::default()
            .with("HIERONYMUS_DATA_ROOT", "/data")
            .with("XDG_CONFIG_HOME", "/config")
            .with("HOME", "/home/test");
        let config = HieronymusConfig::load_with_environment(None, &environment)
            .expect("configuration should resolve");

        let paths = [
            (config.database_path(), "/data/hieronymus.db"),
            (
                config.provider_config_path(),
                "/config/hieronymus/provider.conf",
            ),
            (config.dream_config_path(), "/config/hieronymus/dream.conf"),
            (
                config.ingest_config_path(),
                "/config/hieronymus/ingest.conf",
            ),
            (
                config.semantic_config_path(),
                "/config/hieronymus/semantic.conf",
            ),
            (config.llm_cache_path(), "/config/hieronymus/llm-cache.json"),
            (config.auth_token_path(), "/config/hieronymus/auth-token"),
            (config.lancedb_dir(), "/data/lancedb"),
            (config.models_dir(), "/data/models"),
            (config.daemon_log_path(), "/data/daemon.log"),
            (config.agent_plugins_dir(), "/data/agent-plugins"),
            (config.dream_cycle_lock_path(), "/data/dream-cycle.lock"),
            (config.dream_cycle_state_path(), "/data/dream-cycle.json"),
        ];

        for (actual, expected) in paths {
            assert_eq!(actual, PathBuf::from(expected));
        }
    }

    #[test]
    fn ensure_directories_creates_roots_and_data_subdirectories() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should follow the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hiero-config-{unique}"));
        let data_root = root.join("data");
        let config_root = root.join("hieronymus");
        let environment = TestEnvironment::default()
            .with("HIERONYMUS_DATA_ROOT", data_root.as_os_str())
            .with("XDG_CONFIG_HOME", root.as_os_str())
            .with("HOME", "/home/test");
        let config = HieronymusConfig::load_with_environment(None, &environment)
            .expect("configuration should resolve");
        assert!(!data_root.exists());
        assert!(!config_root.exists());

        config
            .ensure_directories()
            .expect("directories should be created");

        for path in [
            data_root,
            config_root,
            config.lancedb_dir(),
            config.models_dir(),
            config.agent_plugins_dir(),
        ] {
            assert!(path.is_dir(), "{} should be a directory", path.display());
        }
        fs::remove_dir_all(root).expect("test directories should be removable");
    }

    #[test]
    fn ensure_directories_preserves_the_failing_path_and_io_source() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should follow the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hiero-config-error-{unique}"));
        fs::create_dir(&root).expect("test root should be created");
        let blocking_file = root.join("data");
        fs::write(&blocking_file, b"not a directory").expect("blocking file should be created");
        let environment = TestEnvironment::default()
            .with("XDG_CONFIG_HOME", root.as_os_str())
            .with("HOME", "/home/test");
        let config =
            HieronymusConfig::load_with_environment(Some(blocking_file.clone()), &environment)
                .expect("configuration should resolve without touching the filesystem");

        let error = config
            .ensure_directories()
            .expect_err("a regular file cannot be used as the data directory");

        assert!(error.to_string().starts_with(&format!(
            "failed to create required directory `{}`:",
            blocking_file.display()
        )));
        assert!(std::error::Error::source(&error).is_some());
        match &error {
            super::ConfigError::CreateDirectory { path, source } => {
                assert_eq!(path, &blocking_file);
                assert_eq!(source.kind(), std::io::ErrorKind::AlreadyExists);
            }
            super::ConfigError::MissingHome => panic!("expected directory creation error"),
        }
        fs::remove_dir_all(root).expect("test directories should be removable");
    }

    #[test]
    fn port_follows_cli_environment_default_precedence() {
        let cases = [
            (Some(8080), Some(OsString::from("9090")), 8080),
            (None, Some(OsString::from("9090")), 9090),
            (None, None, 9768),
            (None, Some(OsString::from("invalid")), 9768),
        ];

        for (cli_arg, env_port, expected) in cases {
            let mut environment = TestEnvironment::default();
            if let Some(env_port) = env_port {
                environment = environment.with("HIERONYMUS_PORT", env_port);
            }
            assert_eq!(
                HieronymusConfig::resolve_port_with_environment(cli_arg, &environment),
                expected
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn port_falls_back_for_non_unicode_environment_values() {
        let environment =
            TestEnvironment::default().with("HIERONYMUS_PORT", OsString::from_vec(vec![0xff]));

        assert_eq!(
            HieronymusConfig::resolve_port_with_environment(None, &environment),
            9768
        );
    }

    #[test]
    fn resolve_port_accepts_an_explicit_cli_value() {
        let environment = TestEnvironment::default()
            .with("HIERONYMUS_DATA_ROOT", "/data")
            .with("XDG_CONFIG_HOME", "/config")
            .with("HOME", "/home/test");
        let config = HieronymusConfig::load_with_environment(None, &environment)
            .expect("configuration should resolve");

        assert_eq!(config.resolve_port(Some(8123)), 8123);
    }
}
