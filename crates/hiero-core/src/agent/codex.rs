use std::path::PathBuf;

use super::{
    AgentAvailability, AgentError, AgentPlugin, InstallPlan, MCP_HTTP_URL, ProjectAgentContext,
    availability, config_patch::try_patch_toml_config, default_home_path, only_path, plan,
};

pub struct CodexPlugin {
    config_path: PathBuf,
}

impl Default for CodexPlugin {
    fn default() -> Self {
        Self::at(default_home_path(".codex/config.toml"))
    }
}

impl CodexPlugin {
    #[must_use]
    pub fn at(config_path: PathBuf) -> Self {
        Self { config_path }
    }
}

impl AgentPlugin for CodexPlugin {
    fn name(&self) -> &str {
        "codex"
    }
    fn detect(&self) -> AgentAvailability {
        let installed = super::config_patch::load_toml_object(&self.config_path)
            .ok()
            .and_then(|root| {
                root.get("mcp_servers")?
                    .as_table()?
                    .get("hieronymus")?
                    .as_table()?
                    .get("url")?
                    .as_str()
                    .map(str::to_owned)
            })
            .is_some_and(|url| url == MCP_HTTP_URL);
        availability(
            installed,
            &[self
                .config_path
                .parent()
                .unwrap_or(&self.config_path)
                .to_path_buf()],
            &self.config_path,
        )
    }
    fn install_plan(&self, _ctx: &ProjectAgentContext) -> Result<InstallPlan, AgentError> {
        Ok(plan(&self.config_path, self.name()))
    }
    fn apply(&self, plan: &InstallPlan, dry_run: bool) -> Result<(), AgentError> {
        if dry_run {
            return Ok(());
        }
        let path = only_path(plan)?;
        try_patch_toml_config(path, |root| {
            let servers = table_section(root, "mcp_servers", path)?;
            let mut entry = toml::Table::new();
            entry.insert("url".into(), toml::Value::String(MCP_HTTP_URL.to_owned()));
            servers.insert("hieronymus".into(), toml::Value::Table(entry));
            Ok(())
        })
    }
}

fn table_section<'a>(
    root: &'a mut toml::Table,
    section: &'static str,
    path: &std::path::Path,
) -> Result<&'a mut toml::Table, AgentError> {
    let value = root
        .entry(section)
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    value
        .as_table_mut()
        .ok_or_else(|| AgentError::ExpectedObject {
            path: path.to_path_buf(),
            section,
        })
}
