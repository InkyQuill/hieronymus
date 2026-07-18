use std::{
    env,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};

pub mod claude;
pub mod codex;
pub mod config_patch;
pub mod context;
pub mod gemini;
pub mod openclaw;
pub mod opencode;
mod skills;

pub use context::ProjectAgentContext;
pub use skills::{SkillPlan, install_skills, skill_assets, uninstall_skills};

pub const MCP_HTTP_URL: &str = "http://127.0.0.1:9768/mcp";

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("failed to access `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON object in `{path}`: {source}")]
    InvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid TOML object in `{path}`: {source}")]
    InvalidToml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to serialize JSON configuration: {0}")]
    SerializeJson(serde_json::Error),
    #[error("failed to serialize TOML configuration: {0}")]
    SerializeToml(toml::ser::Error),
    #[error("expected object at `{section}` in `{path}`")]
    ExpectedObject {
        path: PathBuf,
        section: &'static str,
    },
    #[error("path has no parent: `{0}`")]
    MissingParent(PathBuf),
    #[error("unsupported skill target: `{0}`")]
    UnsupportedSkillTarget(String),
    #[error("unsafe filesystem path: `{0}`")]
    UnsafePath(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAvailability {
    pub installed: bool,
    pub detect_paths: Vec<PathBuf>,
    pub config_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallStep {
    pub description: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub steps: Vec<InstallStep>,
}

pub trait AgentPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn detect(&self) -> AgentAvailability;
    fn install_plan(&self, ctx: &ProjectAgentContext) -> Result<InstallPlan, AgentError>;
    fn apply(&self, plan: &InstallPlan, dry_run: bool) -> Result<(), AgentError>;
}

pub fn agent_plugins() -> Vec<Box<dyn AgentPlugin>> {
    vec![
        Box::new(claude::ClaudePlugin::default()),
        Box::new(codex::CodexPlugin::default()),
        Box::new(gemini::GeminiPlugin::default()),
        Box::new(opencode::OpenCodePlugin::default()),
        Box::new(openclaw::OpenClawPlugin::default()),
    ]
}

pub fn resolve_plugin(name: &str) -> Option<Box<dyn AgentPlugin>> {
    agent_plugins()
        .into_iter()
        .find(|plugin| plugin.name().eq_ignore_ascii_case(name))
}

pub(crate) fn default_home_path(relative: &str) -> PathBuf {
    env::var_os("HOME").map_or_else(
        || PathBuf::from(relative),
        |home| PathBuf::from(home).join(relative),
    )
}

pub(crate) fn availability(
    installed: bool,
    detect_paths: &[PathBuf],
    config_path: &Path,
) -> AgentAvailability {
    AgentAvailability {
        installed,
        detect_paths: detect_paths.to_vec(),
        config_paths: vec![config_path.to_path_buf()],
    }
}

pub(crate) fn plan(path: &Path, name: &str) -> InstallPlan {
    InstallPlan {
        steps: vec![InstallStep {
            description: format!("Configure the Hieronymus MCP server for {name}"),
            path: path.to_path_buf(),
        }],
    }
}

pub(crate) fn only_path(plan: &InstallPlan) -> Result<&Path, AgentError> {
    plan.steps
        .first()
        .map(|step| step.path.as_path())
        .ok_or_else(|| AgentError::MissingParent(PathBuf::from("empty install plan")))
}

pub(crate) fn patch_json_mcp(path: &Path, entry: Value) -> Result<(), AgentError> {
    config_patch::try_patch_json_config(path, |root| {
        let servers = object_section(root, "mcpServers", path)?;
        servers.insert("hieronymus".into(), entry);
        Ok(())
    })
}

pub(crate) fn command_entry() -> Value {
    json!({"command": "hiero", "args": ["mcp"]})
}

pub(crate) fn json_mcp_matches(path: &Path, expected: &Value) -> bool {
    config_patch::load_json_object(path)
        .ok()
        .and_then(|root| root.get("mcpServers").and_then(Value::as_object).cloned())
        .and_then(|servers| servers.get("hieronymus").cloned())
        .is_some_and(|entry| entry == *expected)
}

fn object_section<'a>(
    root: &'a mut Map<String, Value>,
    section: &'static str,
    path: &Path,
) -> Result<&'a mut Map<String, Value>, AgentError> {
    let value = root
        .entry(section)
        .or_insert_with(|| Value::Object(Map::new()));
    value
        .as_object_mut()
        .ok_or_else(|| AgentError::ExpectedObject {
            path: path.to_path_buf(),
            section,
        })
}
