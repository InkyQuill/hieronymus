//! The stateless MCP JSON-RPC layer (revision `2026-07-28`), ported from the
//! qualified transport reference `qualification/harnesses/mcp-transport`:
//! request validation, mirrored-header enforcement (`-32020`), unsupported
//! protocol version (`-32022`), and the tools/list + tools/call dispatch.
//! There is no session, no initialize, and no server-initiated state.

use serde_json::{json, Map, Value};

pub use super::registry::PROTOCOL_REVISION;
use super::registry::McpRegistry;

pub(crate) enum ValidatedRequest<'a> {
    List,
    Call {
        name: &'a str,
        arguments: &'a Value,
    },
}

/// Build a JSON-RPC error envelope. `data` is included only when present,
/// matching the frozen error fixtures.
pub fn error_response(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = Map::new();
    error.insert("code".to_owned(), json!(code));
    if let Some(data) = data {
        error.insert("data".to_owned(), data);
    }
    error.insert("message".to_owned(), json!(message));
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

/// The uniform `-32602` envelope for metadata validation failures.
pub fn invalid_params(id: Value) -> Value {
    error_response(id, -32602, "Invalid request metadata", None)
}

/// Validate a JSON-RPC request envelope against the stateless contract.
/// `allow_unsupported` defers the protocol-version check to the caller (the
/// HTTP surface reports `-32022` for a mismatching version instead of the
/// generic `-32602`).
pub(crate) fn validate_request(
    request: &Value,
    allow_unsupported: bool,
) -> Result<ValidatedRequest<'_>, ()> {
    if request.get("jsonrpc") != Some(&json!("2.0")) || request.get("id").is_none() {
        return Err(());
    }
    let method = request.get("method").and_then(Value::as_str).ok_or(())?;
    let params = request
        .get("params")
        .and_then(Value::as_object)
        .ok_or(())?;
    for legacy in ["protocolVersion", "clientCapabilities", "clientInfo"] {
        if params.contains_key(legacy) {
            return Err(());
        }
    }
    let metadata = params.get("_meta").and_then(Value::as_object).ok_or(())?;
    let version = metadata
        .get("io.modelcontextprotocol/protocolVersion")
        .and_then(Value::as_str)
        .ok_or(())?;
    if !protocol_version_label_safe(version) {
        return Err(());
    }
    if !allow_unsupported && version != PROTOCOL_REVISION {
        return Err(());
    }
    if !metadata
        .get("io.modelcontextprotocol/clientCapabilities")
        .is_some_and(Value::is_object)
    {
        return Err(());
    }
    if let Some(client_info) = metadata.get("io.modelcontextprotocol/clientInfo") {
        let client_info = client_info.as_object().ok_or(())?;
        for key in ["name", "version"] {
            if client_info
                .get(key)
                .and_then(Value::as_str)
                .is_none_or(|value| value.is_empty())
            {
                return Err(());
            }
        }
    }
    match method {
        "tools/list" if params.len() == 1 => Ok(ValidatedRequest::List),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or(())?;
            let arguments = params
                .get("arguments")
                .filter(|arguments| arguments.is_object())
                .ok_or(())?;
            if params.len() != 3 {
                return Err(());
            }
            Ok(ValidatedRequest::Call { name, arguments })
        }
        _ => Err(()),
    }
}

/// The mirrored-header rules from the qualified reference: the
/// `MCP-Protocol-Version`, `Mcp-Method`, and (for named methods) `Mcp-Name`
/// headers must mirror the body, and must be present. Returns the exact
/// diagnostic for `-32020` when they do not.
pub(crate) fn mirrored_header_error(
    headers: &dyn Fn(&str) -> Option<String>,
    body: &Value,
    registry: &McpRegistry,
) -> Option<String> {
    let protocol_header = match headers("mcp-protocol-version") {
        Some(value) => value,
        None => {
            return Some(
                "Header mismatch: required MCP-Protocol-Version header is missing".to_owned(),
            );
        }
    };
    let body_version = match body
        .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str)
    {
        Some(value) => value,
        None => return Some(generic_mirror_mismatch()),
    };
    if protocol_header != body_version {
        return Some(
            if protocol_version_label_safe(&protocol_header)
                && protocol_version_label_safe(body_version)
            {
                format!(
                    "Header mismatch: MCP-Protocol-Version header value '{protocol_header}' does not match body value '{body_version}'"
                )
            } else {
                generic_mirror_mismatch()
            },
        );
    }
    let method_header = match headers("mcp-method") {
        Some(value) => value,
        None => return Some("Header mismatch: required Mcp-Method header is missing".to_owned()),
    };
    let method = match body.get("method").and_then(Value::as_str) {
        Some(value) => value,
        None => return Some(generic_mirror_mismatch()),
    };
    if method_header != method {
        return Some(
            if mcp_method_label_safe(&method_header) && mcp_method_label_safe(method) {
                format!(
                    "Header mismatch: Mcp-Method header value '{method_header}' does not match body value '{method}'"
                )
            } else {
                generic_mirror_mismatch()
            },
        );
    }
    let name_header = headers("mcp-name");
    let requires_name = matches!(method, "tools/call" | "resources/read" | "prompts/get");
    if requires_name {
        let actual = match name_header {
            Some(value) if !value.is_empty() => value,
            _ => {
                return Some(format!(
                    "Header mismatch: required Mcp-Name header is missing for {method}"
                ));
            }
        };
        let name = match body.pointer("/params/name").and_then(Value::as_str) {
            Some(value) => value,
            None => return Some(generic_mirror_mismatch()),
        };
        if actual != name {
            return Some(
                if registry.contains_tool(&actual) && registry.contains_tool(name) {
                    format!(
                        "Header mismatch: Mcp-Name header value '{actual}' does not match body value '{name}'"
                    )
                } else {
                    generic_mirror_mismatch()
                },
            );
        }
    } else if name_header.is_some() {
        return Some(if mcp_method_label_safe(method) {
            format!("Header mismatch: Mcp-Name header must be omitted for {method}")
        } else {
            generic_mirror_mismatch()
        });
    }
    None
}

fn generic_mirror_mismatch() -> String {
    "Header mismatch: mirrored request metadata does not match".to_owned()
}

/// A protocol-version label is exactly `YYYY-MM-DD` digits/dashes; anything
/// else is never echoed back into diagnostics.
pub(crate) fn protocol_version_label_safe(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

pub(crate) fn mcp_method_label_safe(value: &str) -> bool {
    matches!(
        value,
        "tools/list" | "tools/call" | "resources/read" | "prompts/get"
    )
}

/// Serve one stateless request from the registry: `tools/list` or
/// `tools/call`. Used by both transports (stdio frames and HTTP bodies).
pub fn process_request(registry: &McpRegistry, request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match validate_request(request, false) {
        Ok(ValidatedRequest::List) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "cacheScope": "private",
                "resultType": "complete",
                "tools": registry.list_tools(),
                "ttlMs": 0
            }
        }),
        Ok(ValidatedRequest::Call { name, arguments }) => match registry.call(name, arguments) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => error_response(id, -32603, &error.to_string(), None),
        },
        Err(()) => invalid_params(id),
    }
}

/// The `-32022` envelope with the exact requested/supported data.
pub fn unsupported_version_response(id: Value, requested: &str) -> Value {
    error_response(
        id,
        -32022,
        &format!("Unsupported protocol version: {requested}"),
        Some(json!({"requested": requested, "supported": [PROTOCOL_REVISION]})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_labels_are_screened_before_echo() {
        assert!(protocol_version_label_safe("2026-07-28"));
        assert!(protocol_version_label_safe("2025-06-18"));
        assert!(!protocol_version_label_safe("2026-7-28"));
        assert!(!protocol_version_label_safe("<script>x</scr>"));
        assert!(!protocol_version_label_safe("2026-07-2\n"));
    }

    #[test]
    fn unsupported_version_carries_exact_data() {
        let response = unsupported_version_response(json!(1), "2025-06-18");
        assert_eq!(response["error"]["code"], json!(-32022));
        assert_eq!(
            response["error"]["data"],
            json!({"requested": "2025-06-18", "supported": ["2026-07-28"]})
        );
        assert_eq!(
            response["error"]["message"],
            json!("Unsupported protocol version: 2025-06-18")
        );
    }

    #[test]
    fn missing_protocol_version_header_is_reported_verbatim() {
        let registry = McpRegistry::embedded();
        let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{
            "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28",
                     "io.modelcontextprotocol/clientCapabilities":{}}}});
        let error = mirrored_header_error(&|_| None, &body, &registry).unwrap();
        assert_eq!(
            error,
            "Header mismatch: required MCP-Protocol-Version header is missing"
        );
    }
}
