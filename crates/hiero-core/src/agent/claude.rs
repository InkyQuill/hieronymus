use std::path::PathBuf;

use serde_json::json;

use super::{
    AgentAvailability, AgentError, AgentPlugin, InstallPlan, MCP_HTTP_URL, ProjectAgentContext,
    availability, default_home_path, json_mcp_matches, only_path, patch_json_mcp, plan,
};

pub struct ClaudePlugin {
    config_path: PathBuf,
}

impl Default for ClaudePlugin {
    fn default() -> Self {
        Self::at(default_home_path(".claude.json"))
    }
}

impl ClaudePlugin {
    #[must_use]
    pub fn at(config_path: PathBuf) -> Self {
        Self { config_path }
    }
}

impl AgentPlugin for ClaudePlugin {
    fn name(&self) -> &str {
        "claude"
    }
    fn detect(&self) -> AgentAvailability {
        let entry = json!({"type": "http", "url": MCP_HTTP_URL});
        availability(
            json_mcp_matches(&self.config_path, &entry),
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
        patch_json_mcp(
            only_path(plan)?,
            json!({"type": "http", "url": MCP_HTTP_URL}),
        )
    }
}
