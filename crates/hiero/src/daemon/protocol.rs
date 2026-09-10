//! The stateless MCP JSON-RPC layer (revision `2026-07-28`), ported from the
//! qualified transport reference `qualification/harnesses/mcp-transport`:
//! request validation, mirrored-header enforcement (`-32020`), unsupported
//! protocol version (`-32022`), and the tools/list + tools/call dispatch.
//! There is no session, no initialize, and no server-initiated state.

use base64::Engine;
use serde_json::{Map, Value, json};

use crate::application::Application;

pub use super::registry::PROTOCOL_REVISION;
use super::registry::{CallError, McpRegistry};

pub(crate) enum ValidatedRequest<'a> {
    Discover,
    List,
    Call {
        name: &'a str,
        arguments: Option<&'a Value>,
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

/// Typed request validation errors shared by HTTP and stdio.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RequestError {
    InvalidRequest,
    InvalidParams,
    InvalidCursor,
    UnsupportedContinuation,
    UnknownMethod,
    UnsupportedVersion,
}
impl RequestError {
    pub(crate) fn response(self, request: &Value) -> Value {
        let id = request_id(request);
        match self {
            Self::InvalidRequest => error_response(id, -32600, "Invalid Request", None),
            Self::InvalidParams => invalid_params(id),
            Self::InvalidCursor => error_response(id, -32602, "Invalid or non-issued cursor", None),
            Self::UnsupportedContinuation => {
                error_response(id, -32602, "Unsupported continuation state", None)
            }
            Self::UnknownMethod => {
                error_response(id, -32601, "Method not found (MCP 2026-07-28)", None)
            }
            Self::UnsupportedVersion => unsupported_version_response(
                id,
                request
                    .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            ),
        }
    }
}
pub(crate) fn valid_id(id: &Value) -> bool {
    id.is_string() || id.is_i64() || id.is_u64()
}
pub(crate) fn request_id(request: &Value) -> Value {
    request
        .get("id")
        .filter(|id| valid_id(id))
        .cloned()
        .unwrap_or(Value::Null)
}
pub(crate) fn validate_envelope(request: &Value) -> Result<(), RequestError> {
    if request.get("jsonrpc") != Some(&json!("2.0"))
        || !request.get("id").is_some_and(valid_id)
        || !request.get("method").is_some_and(Value::is_string)
        || request.get("result").is_some()
        || request.get("error").is_some()
    {
        Err(RequestError::InvalidRequest)
    } else {
        Ok(())
    }
}
pub(crate) fn validate_request(request: &Value) -> Result<ValidatedRequest<'_>, RequestError> {
    use RequestError::*;
    validate_envelope(request)?;
    let method = request["method"].as_str().ok_or(InvalidRequest)?;
    let params = request
        .get("params")
        .and_then(Value::as_object)
        .ok_or(InvalidParams)?;
    for legacy in ["protocolVersion", "clientCapabilities", "clientInfo"] {
        if params.contains_key(legacy) {
            return Err(InvalidParams);
        }
    }
    let metadata = params
        .get("_meta")
        .and_then(Value::as_object)
        .ok_or(InvalidParams)?;
    if metadata.keys().any(|key| !valid_meta_key(key, false)) {
        return Err(InvalidParams);
    }
    let version = metadata
        .get("io.modelcontextprotocol/protocolVersion")
        .and_then(Value::as_str)
        .ok_or(InvalidParams)?;
    if !protocol_version_label_safe(version) {
        return Err(InvalidParams);
    }
    if !metadata
        .get("io.modelcontextprotocol/clientCapabilities")
        .is_some_and(valid_capabilities)
    {
        return Err(InvalidParams);
    }
    if let Some(info) = metadata.get("io.modelcontextprotocol/clientInfo") {
        let info = info.as_object().ok_or(InvalidParams)?;
        for key in ["name", "version"] {
            if !info.get(key).is_some_and(Value::is_string) {
                return Err(InvalidParams);
            }
        }
        if info.get("icons").is_some_and(|v| !valid_icons(v)) {
            return Err(InvalidParams);
        }
        for key in ["title", "description", "websiteUrl"] {
            if info.get(key).is_some_and(|v| !v.is_string()) {
                return Err(InvalidParams);
            }
        }
    }
    if metadata
        .get("progressToken")
        .is_some_and(|v| !(v.is_string() || v.is_number()))
    {
        return Err(InvalidParams);
    }
    if metadata
        .get("io.modelcontextprotocol/logLevel")
        .is_some_and(|v| {
            !matches!(
                v.as_str(),
                Some(
                    "debug"
                        | "info"
                        | "notice"
                        | "warning"
                        | "error"
                        | "critical"
                        | "alert"
                        | "emergency"
                )
            )
        })
    {
        return Err(InvalidParams);
    }
    if version != PROTOCOL_REVISION {
        return Err(UnsupportedVersion);
    }
    match method {
        "server/discover" if params.keys().all(|k| k == "_meta") => Ok(ValidatedRequest::Discover),
        "tools/list" if params.keys().all(|k| k == "_meta") => Ok(ValidatedRequest::List),
        "tools/list" if params.contains_key("cursor") => Err(InvalidCursor),
        "server/discover" | "tools/list" => Err(InvalidParams),
        "tools/call" => {
            if params.contains_key("inputResponses") || params.contains_key("requestState") {
                return Err(UnsupportedContinuation);
            }
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .ok_or(InvalidParams)?;
            let arguments = params.get("arguments");
            if arguments.is_some_and(|v| !v.is_object())
                || params
                    .keys()
                    .any(|k| !matches!(k.as_str(), "_meta" | "name" | "arguments"))
            {
                return Err(InvalidParams);
            }
            Ok(ValidatedRequest::Call { name, arguments })
        }
        _ => Err(UnknownMethod),
    }
}

fn valid_meta_key(key: &str, prefix_required: bool) -> bool {
    let name = if let Some((prefix, name)) = key.split_once('/') {
        if prefix.split('.').any(|label| {
            let bytes = label.as_bytes();
            bytes.first().is_none_or(|b| !b.is_ascii_alphabetic())
                || bytes.last().is_none_or(|b| !b.is_ascii_alphanumeric())
                || !bytes
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        }) {
            return false;
        }
        name
    } else {
        if prefix_required {
            return false;
        }
        key
    };
    let bytes = name.as_bytes();
    bytes.is_empty()
        || (bytes[0].is_ascii_alphanumeric()
            && bytes[bytes.len() - 1].is_ascii_alphanumeric()
            && bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')))
}
fn valid_capabilities(value: &Value) -> bool {
    let Some(capabilities) = value.as_object() else {
        return false;
    };
    for key in [
        "roots",
        "sampling",
        "elicitation",
        "experimental",
        "extensions",
    ] {
        if capabilities.get(key).is_some_and(|v| !v.is_object()) {
            return false;
        }
    }
    for (key, members) in [
        ("sampling", &["context", "tools"][..]),
        ("elicitation", &["form", "url"][..]),
    ] {
        if let Some(value) = capabilities.get(key)
            && members
                .iter()
                .any(|member| value.get(member).is_some_and(|v| !v.is_object()))
        {
            return false;
        }
    }
    for key in ["experimental", "extensions"] {
        if let Some(values) = capabilities.get(key).and_then(Value::as_object)
            && (values.values().any(|v| !v.is_object())
                || key == "extensions" && values.keys().any(|name| !valid_meta_key(name, true)))
        {
            return false;
        }
    }
    true
}
fn valid_icons(value: &Value) -> bool {
    value.as_array().is_some_and(|icons| {
        icons.iter().all(|icon| {
            icon.is_object()
                && icon.get("src").is_some_and(Value::is_string)
                && icon.get("mimeType").is_none_or(Value::is_string)
                && icon.get("sizes").is_none_or(|sizes| {
                    sizes
                        .as_array()
                        .is_some_and(|sizes| sizes.iter().all(Value::is_string))
                })
                && icon
                    .get("theme")
                    .is_none_or(|v| matches!(v.as_str(), Some("light" | "dark")))
        })
    })
}

pub(crate) fn encode_header(value: &str) -> String {
    if value.trim() != value
        || !value
            .bytes()
            .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b))
        || (value.starts_with("=?base64?") && value.ends_with("?="))
    {
        format!(
            "=?base64?{}?=",
            base64::engine::general_purpose::STANDARD.encode(value)
        )
    } else {
        value.to_owned()
    }
}
fn decode_header(value: String) -> Option<String> {
    if let Some(encoded) = value.strip_prefix("=?base64?") {
        let encoded = encoded.strip_suffix("?=")?;
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?,
        )
        .ok()
    } else if value.trim() == value
        && value
            .bytes()
            .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b))
    {
        Some(value)
    } else {
        None
    }
}

/// Standard named-method field mirrored by both HTTP server and stdio adapter.
pub(crate) fn request_name(body: &Value) -> Option<&str> {
    let pointer = if body.get("method").and_then(Value::as_str) == Some("resources/read") {
        "/params/uri"
    } else {
        "/params/name"
    };
    body.pointer(pointer).and_then(Value::as_str)
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
    let name_header = match headers("mcp-name") {
        Some(value) => match decode_header(value) {
            Some(value) => Some(value),
            None => return Some(generic_mirror_mismatch()),
        },
        None => None,
    };
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
        let name = match request_name(body) {
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
        "server/discover" | "tools/list" | "tools/call" | "resources/read" | "prompts/get"
    )
}

/// Serve one stateless request from the registry skeleton: `tools/list` and
/// the registry-backed `tools/call` contract without a live application.
/// This is the replay surface for the frozen stdio wire contract; the daemon's
/// HTTP route serves ported tools through
/// [`process_request_with_application`].
pub fn process_request(registry: &McpRegistry, request: &Value) -> Value {
    serve_request(registry, None, "", request)
}

/// Serve one stateless request with the live application: ported `tools/call`
/// names run real domain work and are wrapped centrally by the registry.
/// `actor` is the authenticated credential holder reported to the domain.
pub fn process_request_with_application(
    registry: &McpRegistry,
    application: &Application,
    actor: &str,
    request: &Value,
) -> Value {
    serve_request(registry, Some(application), actor, request)
}

fn serve_request(
    registry: &McpRegistry,
    application: Option<&Application>,
    actor: &str,
    request: &Value,
) -> Value {
    let id = request_id(request);
    match validate_request(request) {
        Ok(ValidatedRequest::Discover) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"resultType":"complete","supportedVersions":[PROTOCOL_REVISION],"capabilities":{"tools":{}},"_meta":server_metadata(),"cacheScope":"private","ttlMs":0}})
        }
        Ok(ValidatedRequest::List) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "cacheScope": "private",
                "resultType": "complete",
                "tools": registry.list_tools(),
                "_meta": server_metadata(),
                "ttlMs": 0
            }
        }),
        Ok(ValidatedRequest::Call { name, arguments }) => {
            if !registry.contains_tool(name) {
                return error_response(id, -32602, &format!("Unknown tool: {name}"), None);
            }
            let empty = json!({});
            let arguments = arguments.unwrap_or(&empty);
            let outcome = match application {
                Some(application) => registry.call(application, name, arguments, actor),
                None => registry.call_skeleton(name),
            };
            match outcome {
                Ok(mut result) => {
                    result["_meta"] = server_metadata();
                    json!({"jsonrpc": "2.0", "id": id, "result": result})
                }
                // Malformed arguments are invalid params; everything else is
                // an internal error surfaced with its diagnostic.
                Err(CallError::InvalidParams(message)) => {
                    error_response(id, -32602, &message, None)
                }
                Err(error) => error_response(id, -32603, &error.to_string(), None),
            }
        }
        Err(error) => error.response(request),
    }
}

fn server_metadata() -> Value {
    json!({"io.modelcontextprotocol/serverInfo":{"name":"hieronymus","version":env!("CARGO_PKG_VERSION")}})
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

    fn modern(method: &str) -> Value {
        json!({"jsonrpc":"2.0","id":0,"method":method,"params":{"_meta":{
            "io.modelcontextprotocol/protocolVersion":"2026-07-28",
            "io.modelcontextprotocol/clientInfo":{"name":"codex-mcp-client","title":"Codex","version":"0.147.0"},
            "io.modelcontextprotocol/clientCapabilities":{"elicitation":{"form":{},"url":{}}}}}})
    }
    #[test]
    fn modern_discovery_and_optional_arguments() {
        let registry = McpRegistry::embedded();
        let result = process_request(&registry, &modern("server/discover"));
        assert_eq!(
            result["result"]["supportedVersions"],
            json!([PROTOCOL_REVISION])
        );
        assert_eq!(result["result"]["capabilities"], json!({"tools":{}}));
        assert_eq!(
            result["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
            env!("CARGO_PKG_VERSION")
        );
        let mut call = modern("tools/call");
        call["params"]["name"] = json!("hieronymus_status");
        assert!(validate_request(&call).is_ok());
        for args in [Value::Null, json!([])] {
            call["params"]["arguments"] = args;
            assert_eq!(process_request(&registry, &call)["error"]["code"], -32602);
        }
    }
    #[test]
    fn malformed_ids_and_unknown_methods_are_distinct() {
        let registry = McpRegistry::embedded();
        for id in [Value::Null, json!(true), json!({}), json!([]), json!(1.5)] {
            let mut req = modern("tools/list");
            req["id"] = id;
            let result = process_request(&registry, &req);
            assert_eq!(result["error"]["code"], -32600);
            assert!(result["id"].is_null());
        }
        assert_eq!(
            process_request(&registry, &modern("unknown/method"))["error"]["code"],
            -32601
        );
        let mut req = modern("server/discover");
        req["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2025-11-25");
        assert_eq!(process_request(&registry, &req)["error"]["code"], -32022);
    }
    #[test]
    fn encoded_name_and_reserved_metadata() {
        let registry = McpRegistry::embedded();
        let mut req = modern("tools/call");
        req["params"]["name"] = json!("hieronymus_status");
        let headers = |name: &str| {
            Some(
                match name {
                    "mcp-protocol-version" => PROTOCOL_REVISION,
                    "mcp-method" => "tools/call",
                    "mcp-name" => "=?base64?aGllcm9ueW11c19zdGF0dXM=?=",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        assert!(mirrored_header_error(&headers, &req, &registry).is_none());
        req["params"]["_meta"]["progressToken"] = json!([]);
        assert!(validate_request(&req).is_err());
    }

    #[test]
    fn exact_reserved_shapes_and_optional_metadata() {
        let registry = McpRegistry::embedded();
        let mut req = modern("tools/list");
        req["params"]["_meta"]
            .as_object_mut()
            .unwrap()
            .remove("io.modelcontextprotocol/clientInfo");
        req["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"] =
            json!({"unknownFutureCapability":true,"extensions":{"org.example/feature":{}}});
        assert!(validate_request(&req).is_ok());
        for caps in [
            json!({"elicitation":false}),
            json!({"sampling":{"tools":[]}}),
            json!({"extensions":{"org.example/feature":null}}),
        ] {
            req["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"] = caps;
            assert_eq!(process_request(&registry, &req)["error"]["code"], -32602);
        }
        req = modern("tools/list");
        req["params"]["_meta"]["io.modelcontextprotocol/clientInfo"]["icons"] = json!([{"src":7}]);
        assert!(validate_request(&req).is_err());
        req = modern("tools/list");
        req["params"]["cursor"] = json!("unissued");
        assert!(
            process_request(&registry, &req)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("cursor")
        );
        req = modern("tools/call");
        req["params"]["name"] = json!("hieronymus_status");
        req["params"]["requestState"] = json!({});
        assert!(
            process_request(&registry, &req)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("continuation")
        );
    }
    #[test]
    fn metadata_keys_obey_final_schema_without_extension_whitelist() {
        for key in ["bad/key/extra", "-invalid", "com.9bad/name"] {
            let mut req = modern("tools/list");
            req["params"]["_meta"][key] = json!(true);
            assert!(validate_request(&req).is_err(), "{key}");
        }
        let mut req = modern("tools/list");
        req["params"]["_meta"]["com.example/future-v1"] = json!([1]);
        assert!(validate_request(&req).is_ok());
        req["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]["extensions"] =
            json!({"unprefixed":{}});
        assert!(validate_request(&req).is_err());
    }
    #[test]
    fn unsupported_resource_uri_still_obeys_mirrored_header_contract() {
        let registry = McpRegistry::embedded();
        let mut req = modern("resources/read");
        req["params"]["uri"] = json!("file:///book.txt");
        let headers = |name: &str| {
            Some(
                match name {
                    "mcp-protocol-version" => PROTOCOL_REVISION,
                    "mcp-method" => "resources/read",
                    "mcp-name" => "file:///book.txt",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        assert!(mirrored_header_error(&headers, &req, &registry).is_none());
        assert_eq!(process_request(&registry, &req)["error"]["code"], -32601);
    }
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
