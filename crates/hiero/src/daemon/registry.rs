//! The MCP tool registry. `tools/list` serves the frozen registry snapshot
//! (the machine-readable port of the Python `mcp_server.py` registry, owned
//! by `compatibility/snapshots/mcp.json`); `tools/call` wraps results in MCP
//! content/structuredContent envelopes centrally: the frozen
//! `hieronymus_status` contract stays registry-backed, ported tools run
//! through the [`Application`] dispatcher, and everything else reports a
//! clean JSON-RPC error.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::application::{AppError, Application};

/// The exact MCP protocol revision served by this registry (ADR 0015).
pub const PROTOCOL_REVISION: &str = "2026-07-28";

const EMBEDDED_SNAPSHOT: &str = include_str!("../../../../compatibility/snapshots/mcp.json");

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("embedded MCP registry snapshot is invalid: {0}")]
    Invalid(&'static str),
}

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("invalid tool arguments: {0}")]
    InvalidParams(String),
    #[error("tool is not implemented by the Rust daemon yet: {0}")]
    NotPorted(String),
}

#[derive(Debug, Clone)]
pub struct McpRegistry {
    tools: Vec<ToolDefinition>,
}

#[derive(Deserialize)]
struct Snapshot {
    #[serde(rename = "protocol_revision")]
    protocol_revision: String,
    #[serde(rename = "derived_tool_count")]
    derived_tool_count: usize,
    tools: Vec<SnapshotTool>,
}

#[derive(Deserialize)]
struct SnapshotTool {
    name: String,
    description: String,
    #[serde(rename = "input_schema")]
    input_schema: Value,
}

impl McpRegistry {
    /// Load the registry from embedded snapshot text.
    pub fn from_snapshot_str(snapshot: &str) -> Result<Self, RegistryError> {
        let snapshot: Snapshot = serde_json::from_str(snapshot)
            .map_err(|_| RegistryError::Invalid("unparsable JSON"))?;
        if snapshot.protocol_revision != PROTOCOL_REVISION {
            return Err(RegistryError::Invalid("unexpected protocol revision"));
        }
        if snapshot.tools.is_empty() {
            return Err(RegistryError::Invalid("snapshot contains no tools"));
        }
        if snapshot.derived_tool_count != snapshot.tools.len() {
            return Err(RegistryError::Invalid("derived tool count mismatch"));
        }
        let mut names = std::collections::BTreeSet::new();
        let mut tools = Vec::with_capacity(snapshot.tools.len());
        for tool in snapshot.tools {
            if tool.name.is_empty()
                || tool.description.is_empty()
                || !tool.input_schema.is_object()
                || !names.insert(tool.name.clone())
            {
                return Err(RegistryError::Invalid("invalid tool definition"));
            }
            tools.push(ToolDefinition {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
            });
        }
        Ok(Self { tools })
    }

    /// The registry compiled into the binary.
    pub fn embedded() -> Self {
        // The snapshot is validated by workspace tests; a panic here means the
        // frozen contract and the binary have diverged, which must never ship.
        Self::from_snapshot_str(EMBEDDED_SNAPSHOT).expect("embedded MCP registry snapshot is valid")
    }

    pub fn list_tools(&self) -> &[ToolDefinition] {
        &self.tools
    }

    pub fn contains_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|tool| tool.name == name)
    }

    /// Dispatch a `tools/call` through the live application: the frozen
    /// status contract stays registry-backed, ported tools run real domain
    /// work, and both are wrapped in MCP envelopes here. Argument-decoding
    /// failures report [`CallError::InvalidParams`] (JSON-RPC invalid-params);
    /// domain failures are tool error results (`isError: true`); tools no
    /// family claims report [`CallError::NotPorted`].
    pub fn call(
        &self,
        application: &Application,
        name: &str,
        arguments: &Value,
        actor: &str,
    ) -> Result<Value, CallError> {
        match name {
            "hieronymus_status" => Ok(status_result()),
            _ => match application.call(name, arguments, actor) {
                Ok(structured) => Ok(success_envelope(structured)),
                Err(AppError::Invalid(message)) => Err(CallError::InvalidParams(message)),
                Err(AppError::Domain(message)) => Ok(error_result_envelope(&message)),
                Err(AppError::NotImplemented(name)) => Err(CallError::NotPorted(name)),
            },
        }
    }

    /// Dispatch a `tools/call` without a live application: only the
    /// registry-backed status contract is served; everything else reports
    /// [`CallError::NotPorted`]. This is the replay surface for the frozen
    /// stdio wire contract; the daemon's HTTP route uses [`McpRegistry::call`]
    /// with the live application.
    pub fn call_skeleton(&self, name: &str) -> Result<Value, CallError> {
        match name {
            "hieronymus_status" => Ok(status_result()),
            _ => Err(CallError::NotPorted(name.to_string())),
        }
    }
}

/// The success envelope: the structured result plus its pretty-text rendering.
fn success_envelope(structured: Value) -> Value {
    let text = serde_json::to_string_pretty(&structured).unwrap_or_default();
    serde_json::json!({
        "content": [{ "text": text, "type": "text" }],
        "isError": false,
        "resultType": "complete",
        "structuredContent": structured
    })
}

/// The tool error result envelope for domain failures: the diagnostic text is
/// the content, and `isError` marks the failure without corrupting the
/// JSON-RPC layer.
fn error_result_envelope(message: &str) -> Value {
    serde_json::json!({
        "content": [{ "text": message, "type": "text" }],
        "isError": true,
        "resultType": "complete"
    })
}

/// The frozen `hieronymus_status` contract: a truthfully minimal daemon state
/// (the daemon answers, so the local HTTP service is available).
fn status_result() -> Value {
    success_envelope(serde_json::json!({
        "service": {
            "available": true,
            "mode": "local-http"
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hieronymus::data_root::HieronymusConfig;

    fn test_application() -> (tempfile::TempDir, Application) {
        let root = tempfile::tempdir().unwrap();
        let application = Application::open(&HieronymusConfig::new(root.path())).unwrap();
        (root, application)
    }

    #[test]
    fn embedded_snapshot_loads_with_the_frozen_tool_count() {
        let registry = McpRegistry::embedded();
        let snapshot: serde_json::Value = serde_json::from_str(EMBEDDED_SNAPSHOT).unwrap();
        assert_eq!(
            registry.list_tools().len() as u64,
            snapshot["derived_tool_count"].as_u64().unwrap()
        );
        assert_eq!(registry.list_tools()[0].name, "hieronymus_concept_archive");
        assert!(registry.contains_tool("hieronymus_status"));
        assert!(!registry.contains_tool("hieronymus_nonexistent"));
    }

    #[test]
    fn status_call_matches_the_frozen_result() {
        let registry = McpRegistry::embedded();
        let (_root, application) = test_application();
        let result = registry
            .call(
                &application,
                "hieronymus_status",
                &serde_json::json!({}),
                "local-user",
            )
            .unwrap();
        let protocol: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../compatibility/fixtures/mcp/protocol.json"
        ))
        .unwrap();
        assert_eq!(
            result,
            protocol["target"]["tools_call"]["response"]["result"]
        );
    }

    #[test]
    fn unported_tools_report_not_ported() {
        let registry = McpRegistry::embedded();
        let (_root, application) = test_application();
        // Every advertised tool dispatches since M5 (hieronymus_dream runs
        // the DreamService seam); a name no family claims is the honest
        // leftover that still reports NotPorted.
        let error = registry
            .call(
                &application,
                "hieronymus_nonexistent",
                &serde_json::json!({}),
                "local-user",
            )
            .unwrap_err();
        assert!(error.to_string().contains("hieronymus_nonexistent"));

        // The dream dispatch runs for real: on a fresh root the workflow
        // gate passes and the run completes with nothing pending.
        let result = registry
            .call(
                &application,
                "hieronymus_dream",
                &serde_json::json!({}),
                "local-user",
            )
            .unwrap();
        assert_eq!(result["isError"], serde_json::json!(false));
        assert_eq!(
            result["structuredContent"]["provider"],
            serde_json::json!("deterministic")
        );
    }

    #[test]
    fn skeleton_calls_serve_only_the_frozen_status_contract() {
        let registry = McpRegistry::embedded();
        let result = registry.call_skeleton("hieronymus_status").unwrap();
        let protocol: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../compatibility/fixtures/mcp/protocol.json"
        ))
        .unwrap();
        assert_eq!(
            result,
            protocol["target"]["tools_call"]["response"]["result"]
        );
        let error = registry.call_skeleton("hieronymus_dream").unwrap_err();
        assert!(error.to_string().contains("hieronymus_dream"));
    }

    #[test]
    fn snapshot_with_wrong_revision_is_rejected() {
        let error = McpRegistry::from_snapshot_str(
            r#"{"protocol_revision":"2025-06-18","derived_tool_count":0,"tools":[]}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("protocol revision"));
    }
}
