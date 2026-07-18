use std::{env, ffi::OsString, fs, path::PathBuf};

const APPLICATION_DIR: &str = "hieronymus";

trait Environment {
    fn var_os(&self, key: &str) -> Option<OsString>;
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
    #[error("failed to create configuration directory `{path}`: {source}")]
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

impl HieronymusConfig {
    pub fn load(data_root: Option<PathBuf>) -> Result<Self, ConfigError> {
        Self::load_with_environment(data_root, &ProcessEnvironment)
    }

    fn load_with_environment(
        data_root: Option<PathBuf>,
        environment: &impl Environment,
    ) -> Result<Self, ConfigError> {
        let home = || environment.var_os("HOME").map(PathBuf::from);
        let data_root = data_root
            .or_else(|| {
                environment
                    .var_os("HIERONYMUS_DATA_ROOT")
                    .map(PathBuf::from)
            })
            .or_else(|| {
                environment
                    .var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .map(|root| root.join(APPLICATION_DIR))
            })
            .or_else(|| home().map(|root| root.join(".local/share").join(APPLICATION_DIR)))
            .ok_or(ConfigError::MissingHome)?;
        let config_root = environment
            .var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .map(|root| root.join(APPLICATION_DIR))
            .or_else(|| home().map(|root| root.join(".config").join(APPLICATION_DIR)))
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
        os::unix::ffi::OsStringExt,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

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
    fn port_follows_cli_environment_default_precedence() {
        let cases = [
            (Some(8080), Some(OsString::from("9090")), 8080),
            (None, Some(OsString::from("9090")), 9090),
            (None, None, 9768),
            (None, Some(OsString::from("invalid")), 9768),
            (None, Some(OsString::from_vec(vec![0xff])), 9768),
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
