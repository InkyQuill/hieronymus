use std::path::{Path, PathBuf};

/// Resolved data-root paths. One root owns the database, all configuration
/// files, backups, and generated agent plugins; there is no XDG split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HieronymusConfig {
    data_root: PathBuf,
}

impl HieronymusConfig {
    pub fn new(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_root.join("hieronymus.sqlite")
    }

    pub fn config_root(&self) -> &Path {
        &self.data_root
    }

    pub fn dream_config_path(&self) -> PathBuf {
        self.config_root().join("dream.conf")
    }

    pub fn provider_config_path(&self) -> PathBuf {
        self.config_root().join("provider.conf")
    }

    pub fn ingest_config_path(&self) -> PathBuf {
        self.config_root().join("ingest.conf")
    }

    pub fn release_config_path(&self) -> PathBuf {
        self.config_root().join("release.conf")
    }

    pub fn llm_cache_path(&self) -> PathBuf {
        self.config_root().join("llmcache.tmp")
    }

    pub fn backups_root(&self) -> PathBuf {
        self.config_root().join("backups")
    }

    pub fn agent_plugins_root(&self) -> PathBuf {
        self.config_root().join("agent-plugins")
    }

    /// Non-secret discovery record published by `hiero daemon` (ADR 0009,
    /// ADR 0012 as amended 2026-09-03). Kept separate from the bearer token.
    pub fn daemon_discovery_path(&self) -> PathBuf {
        self.config_root().join("daemon.json")
    }

    /// Per-installation bearer token backing store; user-only permissions.
    pub fn daemon_token_path(&self) -> PathBuf {
        self.config_root().join("daemon.token")
    }
}

/// Resolve the data root: explicit argument wins, then
/// `HIERONYMUS_DATA_ROOT`, then the default `~/.config/hieronymus`. A leading
/// `~` in explicit or environment roots expands to the user's home directory.
pub fn load_config(data_root: Option<&Path>) -> HieronymusConfig {
    if let Some(explicit) = data_root {
        return HieronymusConfig::new(expand_user(explicit));
    }
    if let Some(env_root) = std::env::var_os("HIERONYMUS_DATA_ROOT") {
        return HieronymusConfig::new(expand_user(Path::new(&env_root)));
    }
    let default = home::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("hieronymus");
    HieronymusConfig::new(default)
}

fn expand_user(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    match text.strip_prefix("~/") {
        Some(rest) => match home::home_dir() {
            Some(home) => home.join(rest),
            None => path.to_path_buf(),
        },
        None if text == "~" => home::home_dir().unwrap_or_else(|| path.to_path_buf()),
        None => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_paths_live_under_the_single_root() {
        let root = HieronymusConfig::new("/tmp/data");
        assert_eq!(
            root.daemon_discovery_path(),
            PathBuf::from("/tmp/data/daemon.json")
        );
        assert_eq!(
            root.daemon_token_path(),
            PathBuf::from("/tmp/data/daemon.token")
        );
    }

    #[test]
    fn expand_user_handles_bare_tilde() {
        let home = home::home_dir().unwrap();
        assert_eq!(expand_user(Path::new("~")), home);
    }

    #[test]
    fn expand_user_leaves_other_paths_alone() {
        assert_eq!(expand_user(Path::new("/tmp/x")), PathBuf::from("/tmp/x"));
        assert_eq!(
            expand_user(Path::new("./prefixed~/x")),
            PathBuf::from("./prefixed~/x")
        );
    }
}
