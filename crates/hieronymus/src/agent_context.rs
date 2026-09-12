//! Project agent context: the `.hieronymus.json` marker file that binds an
//! editor/agent working directory to a Hieronymus series (the Python
//! `agent_context` behavior, ported). The nearest marker from `cwd` upward
//! wins outside CWS; missing fields fall back to the Python defaults. CWS
//! projects use `hiero project-context` and never receive these defaults.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The project context an agent hook injects at session start. Field defaults
/// mirror the Python dataclass (`source_language` "ja", `target_language`
/// "en", `task_type` "translation", empty volume/chapter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectAgentContext {
    pub series_slug: String,
    pub source_language: String,
    pub target_language: String,
    pub task_type: String,
    pub volume: String,
    pub chapter: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentContextError {
    #[error("agent context at {path} is unreadable: {message}")]
    Unreadable {
        path: std::path::PathBuf,
        message: String,
    },
}

#[derive(Deserialize)]
struct RawContext {
    series_slug: serde_json::Value,
    #[serde(default = "default_source_language")]
    source_language: serde_json::Value,
    #[serde(default = "default_target_language")]
    target_language: serde_json::Value,
    #[serde(default = "default_task_type")]
    task_type: serde_json::Value,
    #[serde(default)]
    volume: serde_json::Value,
    #[serde(default)]
    chapter: serde_json::Value,
}

fn default_source_language() -> serde_json::Value {
    serde_json::Value::String("ja".to_string())
}

fn default_target_language() -> serde_json::Value {
    serde_json::Value::String("en".to_string())
}

fn default_task_type() -> serde_json::Value {
    serde_json::Value::String("translation".to_string())
}

impl RawContext {
    fn into_context(self) -> ProjectAgentContext {
        let string_or_default = |value: serde_json::Value, default: &str| match value {
            serde_json::Value::Null => default.to_string(),
            other => match other.as_str() {
                Some(text) => text.to_string(),
                None => other.to_string(),
            },
        };
        ProjectAgentContext {
            series_slug: string_or_default(self.series_slug, ""),
            source_language: string_or_default(self.source_language, "ja"),
            target_language: string_or_default(self.target_language, "en"),
            task_type: string_or_default(self.task_type, "translation"),
            volume: string_or_default(self.volume, ""),
            chapter: string_or_default(self.chapter, ""),
        }
    }
}

/// Walks from `cwd` to the filesystem root and returns the context of the
/// nearest directory containing `.hieronymus.json`; `None` when no marker
/// exists anywhere up the tree (the hook's unhandled case, not an error).
/// CWS boundaries and markers return `None`; callers must use `project-context`
/// for validated CWS metadata and diagnostics instead of legacy language defaults.
pub fn discover_project_context(
    cwd: &Path,
) -> Result<Option<ProjectAgentContext>, AgentContextError> {
    let cwd = if cwd.is_file() {
        cwd.parent().unwrap_or(cwd)
    } else {
        cwd
    };
    // Determine CWS enclosure first so a descendant legacy marker cannot
    // reintroduce legacy defaults inside a CWS project.
    for ancestor in cwd.ancestors() {
        let manifest = ancestor.join("project.md");
        if crate::cws_project::is_manifest(&manifest).map_err(|error| {
            AgentContextError::Unreadable {
                path: manifest,
                message: error.to_string(),
            }
        })? {
            return Ok(None);
        }
    }
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join(".hieronymus.json");
        if !candidate.is_file() {
            continue;
        }
        let text =
            std::fs::read_to_string(&candidate).map_err(|error| AgentContextError::Unreadable {
                path: candidate.clone(),
                message: error.to_string(),
            })?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| AgentContextError::Unreadable {
                path: candidate.clone(),
                message: error.to_string(),
            })?;
        if value
            .as_object()
            .is_some_and(|object| object.contains_key("cws"))
        {
            return Ok(None);
        }
        let raw: RawContext =
            serde_json::from_value(value).map_err(|error| AgentContextError::Unreadable {
                path: candidate,
                message: error.to_string(),
            })?;
        return Ok(Some(raw.into_context()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cws_boundary_does_not_inherit_a_legacy_outer_marker() {
        let root = tempfile::tempdir().unwrap();
        let inner = root.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(
            root.path().join(".hieronymus.json"),
            r#"{"series_slug":"outer"}"#,
        )
        .unwrap();
        std::fs::write(inner.join("project.md"), "---\nschema-version: 1\n---\n").unwrap();
        assert!(discover_project_context(&inner).unwrap().is_none());
        std::fs::write(inner.join(".hieronymus.json"), r#"{"series_slug":"inner"}"#).unwrap();
        assert!(discover_project_context(&inner).unwrap().is_none());
        let below = inner.join("below");
        std::fs::create_dir(&below).unwrap();
        std::fs::write(below.join(".hieronymus.json"), r#"{"series_slug":"below"}"#).unwrap();
        assert!(discover_project_context(&below).unwrap().is_none());
    }

    #[test]
    fn cws_marker_never_uses_legacy_language_defaults() {
        let root = tempfile::tempdir().unwrap();
        for cws in [
            r#"{"binding_version":1,"project_contract_version":1,"directions":{}}"#,
            "null",
            "false",
        ] {
            std::fs::write(
                root.path().join(".hieronymus.json"),
                format!(r#"{{"series_slug":"work","cws":{cws}}}"#),
            )
            .unwrap();
            assert!(discover_project_context(root.path()).unwrap().is_none());
        }
    }

    #[test]
    fn nearest_marker_wins_and_defaults_apply() {
        let root = tempfile::tempdir().unwrap();
        let outer = root.path().join("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(
            outer.join(".hieronymus.json"),
            r#"{"series_slug": "outer-series"}"#,
        )
        .unwrap();
        std::fs::write(
            inner.join(".hieronymus.json"),
            r#"{"series_slug": "inner-series", "chapter": "ch-3", "source_language": "fr"}"#,
        )
        .unwrap();

        let context = discover_project_context(&inner).unwrap().unwrap();
        assert_eq!(context.series_slug, "inner-series");
        assert_eq!(context.chapter, "ch-3");
        assert_eq!(context.source_language, "fr");
        assert_eq!(context.target_language, "en");
        assert_eq!(context.task_type, "translation");
        assert_eq!(context.volume, "");

        // One level up, the outer marker applies.
        let context = discover_project_context(&outer).unwrap().unwrap();
        assert_eq!(context.series_slug, "outer-series");

        // A sibling tree with no marker is unhandled.
        let other = root.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        assert!(discover_project_context(&other).unwrap().is_none());
    }

    #[test]
    fn broken_markers_are_typed_errors() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".hieronymus.json"), "{not json").unwrap();
        let error = discover_project_context(root.path()).unwrap_err();
        assert!(error.to_string().contains("unreadable"), "{error}");
    }
}
