use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProjectAgentContext {
    #[serde(skip)]
    pub workspace: PathBuf,
    pub series_slug: String,
    #[serde(default = "default_source_language")]
    pub source_language: String,
    #[serde(default = "default_target_language")]
    pub target_language: String,
    #[serde(default = "default_task_type")]
    pub task_type: String,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub chapter: String,
}

impl ProjectAgentContext {
    #[must_use]
    pub fn for_workspace(workspace: impl Into<PathBuf>, series_slug: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            series_slug: series_slug.into(),
            source_language: default_source_language(),
            target_language: default_target_language(),
            task_type: default_task_type(),
            volume: String::new(),
            chapter: String::new(),
        }
    }
}

#[must_use]
pub fn discover_context(cwd: &Path) -> Option<ProjectAgentContext> {
    let cwd = cwd.canonicalize().ok()?;
    for directory in cwd.ancestors() {
        let path = directory.join(".hieronymus.json");
        if !path.is_file() {
            continue;
        }
        let mut context: ProjectAgentContext =
            serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        context.workspace = directory.to_path_buf();
        return Some(context);
    }
    None
}

fn default_source_language() -> String {
    "ja".to_owned()
}
fn default_target_language() -> String {
    "en".to_owned()
}
fn default_task_type() -> String {
    "translation".to_owned()
}
