//! Stateful host lifecycle at the boundary of the stateless daemon protocol.

use serde_json::{Value, json};

use crate::daemon::protocol::{PROTOCOL_REVISION, error_response, request_id, validate_envelope};

const SUPPORTED_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Default)]
pub(super) struct HostSession {
    client_info: Option<Value>,
    initialized: bool,
}

pub(super) enum Action {
    Forward(Value),
    Reply(Value),
}

impl HostSession {
    pub(super) fn initialized(&mut self) {
        if self.client_info.is_some() {
            self.initialized = true;
        }
    }

    /// Called on the reader thread, before dispatch to concurrent request workers.
    /// This keeps pipelined initialization and notification ordering deterministic.
    pub(super) fn prepare(&mut self, mut body: Value) -> Action {
        let id = request_id(&body);
        if let Err(error) = validate_envelope(&body) {
            return Action::Reply(error.response(&body));
        }
        if body["method"] == "initialize" {
            return Action::Reply(self.initialize(&body));
        }
        if body["method"] == "ping" {
            return Action::Reply(if body.get("params").is_none_or(Value::is_object) {
                json!({"jsonrpc":"2.0","id":id,"result":{}})
            } else {
                error_response(id, -32602, "Invalid ping parameters", None)
            });
        }
        let Some(client_info) = &self.client_info else {
            // Existing 2026-07-28 stateless hosts use the daemon contract directly.
            return Action::Forward(body);
        };
        if !self.initialized {
            return Action::Reply(error_response(
                id,
                -32600,
                "Send notifications/initialized before invoking tools",
                None,
            ));
        }
        if !matches!(body["method"].as_str(), Some("tools/list" | "tools/call")) {
            return Action::Reply(error_response(id, -32601, "Method not found", None));
        }
        let params = body
            .as_object_mut()
            .expect("validated request object")
            .entry("params")
            .or_insert_with(|| json!({}));
        let Some(params) = params.as_object_mut() else {
            return Action::Reply(error_response(
                id,
                -32602,
                "Invalid request parameters",
                None,
            ));
        };
        let meta = params.entry("_meta").or_insert_with(|| json!({}));
        let Some(meta) = meta.as_object_mut() else {
            return Action::Reply(error_response(id, -32602, "Invalid request metadata", None));
        };
        meta.insert(
            "io.modelcontextprotocol/protocolVersion".into(),
            json!(PROTOCOL_REVISION),
        );
        // The bridge cannot perform callbacks, even if the host offers them.
        meta.insert(
            "io.modelcontextprotocol/clientCapabilities".into(),
            json!({}),
        );
        meta.insert(
            "io.modelcontextprotocol/clientInfo".into(),
            client_info.clone(),
        );
        Action::Forward(body)
    }

    fn initialize(&mut self, body: &Value) -> Value {
        let id = request_id(body);
        if self.client_info.is_some() {
            return error_response(id, -32600, "Session is already initialized", None);
        }
        let params = &body["params"];
        let Some(version) = params["protocolVersion"].as_str() else {
            return error_response(id, -32602, "Invalid initialization parameters", None);
        };
        if !crate::daemon::protocol::protocol_version_label_safe(version)
            || !params["capabilities"].is_object()
            || !params["clientInfo"]["name"].is_string()
            || !params["clientInfo"]["version"].is_string()
        {
            return error_response(id, -32602, "Invalid initialization parameters", None);
        }
        // MCP version negotiation returns a supported alternative for unknown versions.
        let version = if SUPPORTED_VERSIONS.contains(&version) {
            version
        } else {
            SUPPORTED_VERSIONS[0]
        };
        self.client_info = Some(json!({
            "name":params["clientInfo"]["name"], "version":params["clientInfo"]["version"]
        }));
        json!({"jsonrpc":"2.0","id":id,"result":{
            "protocolVersion":version,"capabilities":{"tools":{}},
            "serverInfo":{"name":"hieronymus","version":env!("CARGO_PKG_VERSION")}
        }})
    }
}
