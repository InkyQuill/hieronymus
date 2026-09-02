mod registry;
mod report;

use anyhow::{Result, anyhow, bail};
use registry::{FrozenRegistry, PROTOCOL_REVISION, RegistryProbe, read_json_bounded};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() {
    if run().await.is_err() {
        let _ = writeln!(io::stderr(), "{{\"error\":\"mcp_transport_failed\"}}");
        std::process::exit(2);
    }
}

async fn run() -> Result<()> {
    let mut arguments = env::args_os().skip(1);
    let mode = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| anyhow!("missing mode"))?;
    let options = parse_options(arguments)?;
    match mode.as_str() {
        "stdio" => run_stdio(&options),
        "http" => run_http(&options).await,
        _ => bail!("unsupported mode"),
    }
}

fn parse_options(
    arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<BTreeMap<String, PathBuf>> {
    let arguments: Vec<_> = arguments.collect();
    if arguments.len() % 2 != 0 {
        bail!("option is missing a value");
    }
    let mut options = BTreeMap::new();
    for pair in arguments.chunks_exact(2) {
        let name = pair[0]
            .to_str()
            .filter(|name| name.starts_with("--"))
            .ok_or_else(|| anyhow!("invalid option"))?;
        if options
            .insert(name[2..].to_owned(), PathBuf::from(&pair[1]))
            .is_some()
        {
            bail!("duplicate option");
        }
    }
    Ok(options)
}

fn required_path<'a>(options: &'a BTreeMap<String, PathBuf>, name: &str) -> Result<&'a Path> {
    options
        .get(name)
        .map(PathBuf::as_path)
        .ok_or_else(|| anyhow!("required option is missing"))
}

fn load_oracles(options: &BTreeMap<String, PathBuf>) -> Result<(FrozenRegistry, Value)> {
    let mut registry = FrozenRegistry::load(required_path(options, "registry")?)?;
    let protocol = read_json_bounded(required_path(options, "protocol")?)?;
    let target = protocol
        .get("target")
        .cloned()
        .ok_or_else(|| anyhow!("target protocol is missing"))?;
    registry.load_fixture_result(&target)?;
    Ok((registry, target))
}

fn run_stdio(options: &BTreeMap<String, PathBuf>) -> Result<()> {
    if options.len() != 2 {
        bail!("unexpected stdio option");
    }
    let (registry, _) = load_oracles(options)?;
    let mut input = Vec::new();
    io::stdin()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|_| anyhow!("stdin cannot be read"))?;
    if input.len() > MAX_REQUEST_BYTES {
        bail!("request exceeds size limit");
    }
    let lines: Vec<&[u8]> = input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() != 1 {
        bail!("stdio requires exactly one request");
    }
    let request: Value =
        serde_json::from_slice(lines[0]).map_err(|_| anyhow!("request is not JSON"))?;
    let response = process_request(&registry, &request, false);
    let mut encoded =
        serde_json::to_vec(&response).map_err(|_| anyhow!("response cannot serialize"))?;
    encoded.push(b'\n');
    io::stdout()
        .write_all(&encoded)
        .map_err(|_| anyhow!("stdout cannot be written"))?;
    report::emit(report::Evidence {
        protocol_version: PROTOCOL_REVISION,
        transport: "stdio",
        request: lines[0],
        response: &encoded,
        request_content_type: report::ContentType::Json,
        response_content_type: report::ContentType::Json,
        registry_sha256: registry.digest(),
        stdout_objects: 1,
        stdout_newlines: 1,
        exit_status: 0,
    })
    .map_err(|_| anyhow!("report cannot be written"))?;
    Ok(())
}

struct RouteOracle {
    bearer: String,
}

impl RouteOracle {
    fn load(path: &Path) -> Result<Self> {
        let document = read_json_bounded(path)?;
        let target = document["routes"]
            .as_array()
            .and_then(|routes| {
                routes
                    .iter()
                    .find(|route| route["contract_id"] == "http.route.post.mcp")
            })
            .and_then(|route| route.get("target"))
            .ok_or_else(|| anyhow!("MCP route target is missing"))?;
        let bearer = target["successes"]
            .as_array()
            .and_then(|successes| successes.first())
            .and_then(|success| success.pointer("/request/headers/Authorization"))
            .and_then(Value::as_str)
            .filter(|value| value.starts_with("Bearer "))
            .ok_or_else(|| anyhow!("route bearer fixture is missing"))?;
        Ok(Self {
            bearer: bearer.to_owned(),
        })
    }
}

async fn run_http(options: &BTreeMap<String, PathBuf>) -> Result<()> {
    if options.len() != 5 {
        bail!("unexpected HTTP option");
    }
    let (registry, _) = load_oracles(options)?;
    let routes = RouteOracle::load(required_path(options, "route-cases")?)?;
    let bind: SocketAddr = required_path(options, "bind")?
        .to_str()
        .ok_or_else(|| anyhow!("bind is not UTF-8"))?
        .parse()
        .map_err(|_| anyhow!("bind is invalid"))?;
    if !is_loopback(bind.ip()) {
        bail!("bind must use loopback");
    }
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|_| anyhow!("bind failed"))?;
    let address = listener
        .local_addr()
        .map_err(|_| anyhow!("bound address unavailable"))?;
    let ready = json!({"address": address.to_string()});
    fs::write(
        required_path(options, "ready-file")?,
        serde_json::to_vec(&ready).map_err(|_| anyhow!("ready value cannot serialize"))?,
    )
    .map_err(|_| anyhow!("ready file cannot be written"))?;

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|_| anyhow!("accept failed"))?;
        if let Err(_error) = serve_connection(stream, address, &registry, &routes).await {
            continue;
        }
    }
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

struct HttpResponse {
    status: u16,
    content_type: report::ContentType,
    body: Vec<u8>,
}

async fn serve_connection(
    mut stream: TcpStream,
    bound_address: SocketAddr,
    registry: &FrozenRegistry,
    routes: &RouteOracle,
) -> Result<()> {
    let request = read_http_request(&mut stream).await?;
    let request_digest_input = request.body.clone();
    let response = handle_http(&request, bound_address, registry, routes)?;
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type.as_str(),
        response.body.len()
    );
    timeout(IO_TIMEOUT, stream.write_all(head.as_bytes()))
        .await
        .map_err(|_| anyhow!("response write timed out"))?
        .map_err(|_| anyhow!("response head write failed"))?;
    timeout(IO_TIMEOUT, stream.write_all(&response.body))
        .await
        .map_err(|_| anyhow!("response write timed out"))?
        .map_err(|_| anyhow!("response body write failed"))?;
    let _ = stream.shutdown().await;
    report::emit(report::Evidence {
        protocol_version: PROTOCOL_REVISION,
        transport: "http",
        request: &request_digest_input,
        response: &response.body,
        request_content_type: request_content_type(&request.headers),
        response_content_type: response.content_type,
        registry_sha256: registry.digest(),
        stdout_objects: 0,
        stdout_newlines: 0,
        exit_status: 0,
    })
    .map_err(|_| anyhow!("report cannot be written"))?;
    Ok(())
}

async fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut bytes = Vec::new();
    let mut temporary = [0_u8; 8192];
    let header_end = loop {
        if bytes.len() > MAX_REQUEST_BYTES {
            bail!("HTTP request exceeds size limit");
        }
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let count = timeout(IO_TIMEOUT, stream.read(&mut temporary))
            .await
            .map_err(|_| anyhow!("HTTP read timed out"))?
            .map_err(|_| anyhow!("HTTP read failed"))?;
        if count == 0 {
            bail!("HTTP headers are incomplete");
        }
        bytes.extend_from_slice(&temporary[..count]);
    };
    let header_text = std::str::from_utf8(&bytes[..header_end - 4])
        .map_err(|_| anyhow!("HTTP headers are not UTF-8"))?;
    let mut lines = header_text.split("\r\n");
    let mut request_line = lines
        .next()
        .ok_or_else(|| anyhow!("HTTP request line is missing"))?
        .split_whitespace();
    let method = request_line.next().unwrap_or_default().to_owned();
    let path = request_line.next().unwrap_or_default().to_owned();
    if request_line.next() != Some("HTTP/1.1") || request_line.next().is_some() {
        bail!("HTTP request line is invalid");
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| anyhow!("HTTP header is invalid"))?;
        let normalized = name.trim().to_ascii_lowercase();
        if normalized.is_empty()
            || headers
                .insert(normalized, value.trim().to_owned())
                .is_some()
        {
            bail!("HTTP header is duplicated or empty");
        }
    }
    let content_length: usize = header(&headers, "content-length")
        .ok_or_else(|| anyhow!("content length is missing"))?
        .parse()
        .map_err(|_| anyhow!("content length is invalid"))?;
    if content_length > MAX_REQUEST_BYTES || header_end + content_length > MAX_REQUEST_BYTES {
        bail!("HTTP body exceeds size limit");
    }
    while bytes.len() < header_end + content_length {
        let count = timeout(IO_TIMEOUT, stream.read(&mut temporary))
            .await
            .map_err(|_| anyhow!("HTTP body read timed out"))?
            .map_err(|_| anyhow!("HTTP body read failed"))?;
        if count == 0 {
            bail!("HTTP body is incomplete");
        }
        bytes.extend_from_slice(&temporary[..count]);
    }
    if bytes.len() != header_end + content_length {
        bail!("HTTP request has trailing bytes");
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: bytes[header_end..].to_vec(),
    })
}

fn header<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers.get(name).map(String::as_str)
}

fn request_content_type(headers: &BTreeMap<String, String>) -> report::ContentType {
    if header(headers, "content-type")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        report::ContentType::Json
    } else {
        report::ContentType::Invalid
    }
}

fn handle_http(
    request: &HttpRequest,
    bound_address: SocketAddr,
    registry: &FrozenRegistry,
    routes: &RouteOracle,
) -> Result<HttpResponse> {
    if request.method != "POST" || request.path != "/mcp" {
        return json_response(404, &json!({"error":"not_found"}), false);
    }
    if header(&request.headers, "host") != Some(bound_address.to_string().as_str()) {
        return json_response(400, &json!({"error":"invalid_host"}), false);
    }
    if header(&request.headers, "authorization") != Some(routes.bearer.as_str()) {
        return json_response(401, &json!({"error":"unauthorized"}), false);
    }
    if request_content_type(&request.headers) == report::ContentType::Invalid {
        return protocol_error(400, Value::Null, -32602, "Invalid request metadata", None);
    }
    let body: Value = match serde_json::from_slice(&request.body) {
        Ok(body) => body,
        Err(_) => return protocol_error(400, Value::Null, -32700, "Parse error", None),
    };
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    if header(&request.headers, "mcp-session-id").is_some()
        || header(&request.headers, "last-event-id").is_some()
    {
        return protocol_error(400, id, -32602, "Invalid request metadata", None);
    }
    if validated_request(&body, true).is_err() {
        return protocol_error(400, id, -32602, "Invalid request metadata", None);
    }
    if let Some(error) = mirrored_header_error(request, &body, registry) {
        return protocol_error(400, id, -32020, &error, None);
    }
    let requested_version = body
        .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("protocol version metadata is invalid"))?;
    if requested_version != PROTOCOL_REVISION {
        return protocol_error(
            400,
            id,
            -32022,
            &format!("Unsupported protocol version: {requested_version}"),
            Some(json!({"requested":requested_version,"supported":[PROTOCOL_REVISION]})),
        );
    }
    let response = process_request(registry, &body, true);
    let is_error = response.get("error").is_some();
    let status = if is_error { 400 } else { 200 };
    let wants_sse = header(&request.headers, "accept")
        .is_some_and(|accept| accept.trim().eq_ignore_ascii_case("text/event-stream"));
    json_response(status, &response, wants_sse && !is_error)
}

fn mirrored_header_error(
    request: &HttpRequest,
    body: &Value,
    registry: &FrozenRegistry,
) -> Option<String> {
    let body_version = body
        .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str)?;
    let protocol_header = match header(&request.headers, "mcp-protocol-version") {
        Some(value) => value,
        None => {
            return Some(
                "Header mismatch: required MCP-Protocol-Version header is missing".to_owned(),
            );
        }
    };
    if protocol_header != body_version {
        return Some(if protocol_version_label_safe(protocol_header) {
            format!(
                "Header mismatch: MCP-Protocol-Version header value '{protocol_header}' does not match body value '{body_version}'"
            )
        } else {
            "Header mismatch: MCP-Protocol-Version header does not match body value".to_owned()
        });
    }
    let method = body.get("method").and_then(Value::as_str)?;
    let method_header = match header(&request.headers, "mcp-method") {
        Some(value) => value,
        None => {
            return Some("Header mismatch: required Mcp-Method header is missing".to_owned());
        }
    };
    if method_header != method {
        return Some(if mcp_method_label_safe(method_header) {
            format!(
                "Header mismatch: Mcp-Method header value '{method_header}' does not match body value '{method}'"
            )
        } else {
            "Header mismatch: Mcp-Method header does not match body value".to_owned()
        });
    }
    let name_header = header(&request.headers, "mcp-name");
    let requires_name = matches!(method, "tools/call" | "resources/read" | "prompts/get");
    if requires_name {
        let name = body.pointer("/params/name").and_then(Value::as_str)?;
        let actual = match name_header {
            Some(value) if !value.is_empty() => value,
            _ => {
                return Some(format!(
                    "Header mismatch: required Mcp-Name header is missing for {method}"
                ));
            }
        };
        if actual != name {
            return Some(
                if registry.contains_tool(actual) && registry.contains_tool(name) {
                    format!(
                        "Header mismatch: Mcp-Name header value '{actual}' does not match body value '{name}'"
                    )
                } else {
                    "Header mismatch: Mcp-Name header does not match body value".to_owned()
                },
            );
        }
    } else if name_header.is_some() {
        return Some(format!(
            "Header mismatch: Mcp-Name header must be omitted for {method}"
        ));
    }
    None
}

fn protocol_version_label_safe(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn mcp_method_label_safe(value: &str) -> bool {
    matches!(
        value,
        "tools/list" | "tools/call" | "resources/read" | "prompts/get"
    )
}

fn process_request(registry: &FrozenRegistry, request: &Value, allow_unsupported: bool) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match validated_request(request, allow_unsupported) {
        Ok(ValidatedRequest::List) => json!({
            "jsonrpc":"2.0",
            "id":id,
            "result":{
                "cacheScope":"private",
                "resultType":"complete",
                "tools":registry.list_tools(),
                "ttlMs":0
            }
        }),
        Ok(ValidatedRequest::Call { name, arguments }) => {
            match registry.fixture_result(name, arguments) {
                Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                Err(_) => invalid_params(id),
            }
        }
        Err(_) => invalid_params(id),
    }
}

enum ValidatedRequest<'a> {
    List,
    Call { name: &'a str, arguments: &'a Value },
}

fn validated_request(request: &Value, allow_unsupported: bool) -> Result<ValidatedRequest<'_>> {
    if request.get("jsonrpc") != Some(&json!("2.0")) || request.get("id").is_none() {
        bail!("invalid JSON-RPC envelope");
    }
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("method is invalid"))?;
    let params = request
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("params are invalid"))?;
    for legacy in ["protocolVersion", "clientCapabilities", "clientInfo"] {
        if params.contains_key(legacy) {
            bail!("legacy metadata is forbidden");
        }
    }
    let metadata = params
        .get("_meta")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("metadata is missing"))?;
    let version = metadata
        .get("io.modelcontextprotocol/protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("protocol version is invalid"))?;
    if !protocol_version_label_safe(version) {
        bail!("protocol version is invalid");
    }
    if !allow_unsupported && version != PROTOCOL_REVISION {
        bail!("unsupported protocol version");
    }
    if !metadata
        .get("io.modelcontextprotocol/clientCapabilities")
        .is_some_and(Value::is_object)
    {
        bail!("client capabilities are invalid");
    }
    if let Some(client_info) = metadata.get("io.modelcontextprotocol/clientInfo") {
        let client_info = client_info
            .as_object()
            .ok_or_else(|| anyhow!("client info is invalid"))?;
        for key in ["name", "version"] {
            if client_info
                .get(key)
                .and_then(Value::as_str)
                .is_none_or(|value| value.is_empty())
            {
                bail!("client info is invalid");
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
                .ok_or_else(|| anyhow!("tool name is invalid"))?;
            let arguments = params
                .get("arguments")
                .filter(|arguments| arguments.is_object())
                .ok_or_else(|| anyhow!("tool arguments are invalid"))?;
            if params.len() != 3 {
                bail!("unexpected call parameter");
            }
            Ok(ValidatedRequest::Call { name, arguments })
        }
        _ => bail!("unsupported method"),
    }
}

fn invalid_params(id: Value) -> Value {
    json!({
        "jsonrpc":"2.0",
        "id":id,
        "error":{"code":-32602,"message":"Invalid request metadata"}
    })
}

fn protocol_error(
    status: u16,
    id: Value,
    code: i64,
    message: &str,
    data: Option<Value>,
) -> Result<HttpResponse> {
    let mut error = Map::new();
    error.insert("code".to_owned(), json!(code));
    if let Some(data) = data {
        error.insert("data".to_owned(), data);
    }
    error.insert("message".to_owned(), json!(message));
    json_response(
        status,
        &json!({"jsonrpc":"2.0","id":id,"error":error}),
        false,
    )
}

fn json_response(status: u16, body: &Value, sse: bool) -> Result<HttpResponse> {
    if sse {
        let data = serde_json::to_string(body).map_err(|_| anyhow!("SSE cannot serialize"))?;
        Ok(HttpResponse {
            status,
            content_type: report::ContentType::Sse,
            body: format!("event: message\ndata: {data}\n\n").into_bytes(),
        })
    } else {
        Ok(HttpResponse {
            status,
            content_type: report::ContentType::JsonUtf8,
            body: serde_json::to_vec(body).map_err(|_| anyhow!("JSON cannot serialize"))?,
        })
    }
}
