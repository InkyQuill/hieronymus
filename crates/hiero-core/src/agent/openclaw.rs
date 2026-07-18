use std::path::PathBuf;

use super::{
    AgentAvailability, AgentError, AgentPlugin, InstallPlan, ProjectAgentContext, availability,
    command_entry, default_home_path, json_mcp_matches, only_path, patch_json_mcp, plan,
};

pub struct OpenClawPlugin {
    config_path: PathBuf,
}
impl Default for OpenClawPlugin {
    fn default() -> Self {
        Self::at(default_home_path(".openclaw/openclaw.json"))
    }
}
impl OpenClawPlugin {
    #[must_use]
    pub fn at(config_path: PathBuf) -> Self {
        Self { config_path }
    }
}
impl AgentPlugin for OpenClawPlugin {
    fn name(&self) -> &str {
        "openclaw"
    }
    fn detect(&self) -> AgentAvailability {
        availability(
            json_mcp_matches(&self.config_path, &command_entry()),
            &[default_home_path(".openclaw")],
            &self.config_path,
        )
    }
    fn install_plan(&self, _ctx: &ProjectAgentContext) -> Result<InstallPlan, AgentError> {
        Ok(plan(&self.config_path, self.name()))
    }
    fn apply(&self, plan: &InstallPlan, dry_run: bool) -> Result<(), AgentError> {
        if dry_run {
            Ok(())
        } else {
            patch_json_mcp(only_path(plan)?, command_entry())
        }
    }
}
