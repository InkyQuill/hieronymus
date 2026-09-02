use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const PROTOCOL_REVISION: &str = "2026-07-28";

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("harness lives three levels below the repository root")
        .to_path_buf()
}

fn fixture(path: &str) -> Value {
    let bytes = fs::read(repository_root().join(path)).expect("fixture is readable");
    serde_json::from_slice(&bytes).expect("fixture is JSON")
}

fn target_protocol() -> Value {
    fixture("compatibility/fixtures/mcp/protocol.json")
        .get("target")
        .expect("target protocol exists")
        .clone()
}

fn route_target() -> Value {
    fixture("compatibility/fixtures/http/route-cases.json")["routes"]
        .as_array()
        .expect("routes is an array")
        .iter()
        .find(|route| route["contract_id"] == "http.route.post.mcp")
        .expect("MCP route exists")["target"]
        .clone()
}

fn registry_path() -> PathBuf {
    repository_root().join("compatibility/snapshots/mcp.json")
}

fn protocol_path() -> PathBuf {
    repository_root().join("compatibility/fixtures/mcp/protocol.json")
}

fn route_cases_path() -> PathBuf {
    repository_root().join("compatibility/fixtures/http/route-cases.json")
}

fn stdio_exchange(request: &Value) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mcp-transport"))
        .arg("stdio")
        .arg("--registry")
        .arg(registry_path())
        .arg("--protocol")
        .arg(protocol_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("stdio harness starts");
    let mut input = serde_json::to_vec(request).expect("request serializes");
    input.push(b'\n');
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(&input)
        .expect("request is written");
    child.wait_with_output().expect("stdio harness exits")
}

fn one_stdout_response(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout.iter().filter(|byte| **byte == b'\n').count(),
        1
    );
    assert!(output.stdout.ends_with(b"\n"));
    serde_json::from_slice(&output.stdout).expect("stdout is one JSON object")
}

fn assert_safe_report(stderr: &[u8], transport: &str) {
    let report: Value = serde_json::from_slice(stderr).expect("stderr is one bounded JSON report");
    assert_eq!(report["protocolVersion"], PROTOCOL_REVISION);
    assert_eq!(report["transport"], transport);
    for digest in ["requestSha256", "responseSha256", "registrySha256"] {
        assert_eq!(
            report[digest].as_str().expect("digest is a string").len(),
            64
        );
    }
    assert!(report["exitStatus"].is_i64());
    let rendered = String::from_utf8_lossy(stderr);
    for forbidden in [
        "compat-secret-do-not-log",
        "Authorization",
        "Bearer",
        "headers",
        "params",
        "/home/",
        "/Users/",
    ] {
        assert!(!rendered.contains(forbidden), "report leaked {forbidden}");
    }
}

fn canonical_requests() -> (Value, Value) {
    let protocol = target_protocol();
    (
        protocol["tools_list"]["request"].clone(),
        protocol["tools_call"]["request"].clone(),
    )
}

fn assert_complete(response: &Value) {
    assert_eq!(response["result"]["resultType"], "complete");
    assert!(response["result"].get("_meta").is_none());
    let encoded = serde_json::to_string(response).expect("response serializes");
    assert!(!encoded.contains("serverInfo"));
    assert!(!encoded.contains(env!("CARGO_PKG_VERSION")));
}

fn assert_list_result(response: &Value) {
    assert_complete(response);
    assert_eq!(response["result"]["cacheScope"], "private");
    assert_eq!(response["result"]["ttlMs"], 0);
    let tools = response["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 39);
    assert!(tools.iter().all(|tool| tool.get("inputSchema").is_some()));
    assert!(tools.iter().all(|tool| tool.get("input_schema").is_none()));

    let snapshot = fixture("compatibility/snapshots/mcp.json");
    let mapped: Vec<Value> = snapshot["tools"]
        .as_array()
        .expect("snapshot tools")
        .iter()
        .map(|tool| {
            json!({
                "name": tool["name"],
                "description": tool["description"],
                "inputSchema": tool["input_schema"],
            })
        })
        .collect();
    assert_eq!(tools, &mapped);
}

#[test]
fn stdio_is_stateless_and_preserves_official_wire_shapes() {
    let (list, call) = canonical_requests();
    for (request, is_list) in [(&list, true), (&call, false)] {
        let output = stdio_exchange(request);
        let response = one_stdout_response(&output);
        assert_eq!(response["id"], request["id"]);
        if is_list {
            assert_list_result(&response);
        } else {
            assert_complete(&response);
            assert_eq!(response, target_protocol()["tools_call"]["response"]);
        }
        assert_safe_report(&output.stderr, "stdio");
    }

    let mut without_client_info = list.clone();
    without_client_info["params"]["_meta"]
        .as_object_mut()
        .expect("metadata object")
        .remove("io.modelcontextprotocol/clientInfo");
    assert_list_result(&one_stdout_response(&stdio_exchange(&without_client_info)));
}

#[test]
fn stdio_rejects_legacy_missing_wrong_and_handshake_metadata() {
    let (list, _) = canonical_requests();
    let mut cases = Vec::new();

    for direct in ["protocolVersion", "clientCapabilities", "clientInfo"] {
        let mut request = list.clone();
        request["params"][direct] = json!("legacy");
        cases.push(request);
    }
    for required in [
        "io.modelcontextprotocol/protocolVersion",
        "io.modelcontextprotocol/clientCapabilities",
    ] {
        let mut missing = list.clone();
        missing["params"]["_meta"]
            .as_object_mut()
            .expect("metadata object")
            .remove(required);
        cases.push(missing);
    }
    let mut wrong_version = list.clone();
    wrong_version["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!(false);
    cases.push(wrong_version);
    let mut wrong_capabilities = list.clone();
    wrong_capabilities["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"] = json!([]);
    cases.push(wrong_capabilities);
    let mut wrong_client_info = list.clone();
    wrong_client_info["params"]["_meta"]["io.modelcontextprotocol/clientInfo"] = json!({});
    cases.push(wrong_client_info);
    let mut initialize = list.clone();
    initialize["method"] = json!("initialize");
    cases.push(initialize);

    for request in cases {
        let output = stdio_exchange(&request);
        let response = one_stdout_response(&output);
        assert_eq!(
            response["error"]["code"], -32602,
            "request was accepted: {request}"
        );
        assert!(response.get("result").is_none());
        assert_safe_report(&output.stderr, "stdio");
    }
}

struct HttpServer {
    child: Child,
    address: String,
    reports: Receiver<Vec<u8>>,
    stderr_thread: Option<JoinHandle<()>>,
    _temporary: tempfile::TempDir,
}

impl HttpServer {
    fn start() -> Self {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let ready = temporary.path().join("ready.json");
        let mut child = Command::new(env!("CARGO_BIN_EXE_mcp-transport"))
            .arg("http")
            .arg("--registry")
            .arg(registry_path())
            .arg("--protocol")
            .arg(protocol_path())
            .arg("--route-cases")
            .arg(route_cases_path())
            .arg("--bind")
            .arg("127.0.0.1:0")
            .arg("--ready-file")
            .arg(&ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("HTTP harness starts");
        let stderr = child.stderr.take().expect("HTTP stderr is piped");
        let (reports_tx, reports) = mpsc::sync_channel(128);
        let stderr_thread = thread::spawn(move || {
            for line in BufReader::new(stderr).split(b'\n') {
                let Ok(line) = line else { break };
                if !line.is_empty() && reports_tx.send(line).is_err() {
                    break;
                }
            }
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "HTTP harness did not become ready"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let ready_value: Value =
            serde_json::from_slice(&fs::read(&ready).expect("ready file is readable"))
                .expect("ready file is JSON");
        let address = ready_value["address"]
            .as_str()
            .expect("ready address")
            .to_owned();
        Self {
            child,
            address,
            reports,
            stderr_thread: Some(stderr_thread),
            _temporary: temporary,
        }
    }

    fn exchange(&self, request: &Value, accept_override: Option<&str>) -> HttpResponse {
        let body = serde_json::to_vec(&request["body"]).expect("body serializes");
        let headers = accept_override
            .map(|accept| vec![("Accept", accept)])
            .unwrap_or_default();
        self.exchange_bytes(request, &body, &headers)
    }

    fn exchange_bytes(
        &self,
        request: &Value,
        body: &[u8],
        header_overrides: &[(&str, &str)],
    ) -> HttpResponse {
        let mut headers = request["headers"]
            .as_object()
            .expect("headers object")
            .clone();
        if headers.get("Host").and_then(Value::as_str) == Some("127.0.0.1:<PORT>") {
            headers.insert("Host".into(), json!(self.address));
        }
        for (name, value) in header_overrides {
            headers.insert((*name).into(), json!(*value));
        }
        let mut wire = format!(
            "{} {} HTTP/1.1\r\n",
            request["method"].as_str().expect("HTTP method"),
            request["path"].as_str().expect("HTTP path")
        );
        for (name, value) in headers {
            wire.push_str(&format!(
                "{name}: {}\r\n",
                value.as_str().expect("header is a string")
            ));
        }
        wire.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ));
        let mut stream = TcpStream::connect(&self.address).expect("connect to HTTP harness");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("read timeout");
        stream.write_all(wire.as_bytes()).expect("write headers");
        stream.write_all(body).expect("write body");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).expect("read response");
        HttpResponse::parse(&response)
    }

    fn next_report(&self) -> Vec<u8> {
        self.reports
            .recv_timeout(Duration::from_secs(10))
            .expect("HTTP evidence report arrives")
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(stderr_thread) = self.stderr_thread.take() {
            stderr_thread.join().expect("stderr reader exits");
        }
    }
}

struct HttpResponse {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

impl HttpResponse {
    fn parse(wire: &[u8]) -> Self {
        let separator = wire
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("HTTP header terminator");
        let header = std::str::from_utf8(&wire[..separator]).expect("ASCII headers");
        let mut lines = header.lines();
        let status = lines
            .next()
            .expect("status line")
            .split_whitespace()
            .nth(1)
            .expect("status code")
            .parse()
            .expect("numeric status");
        let content_type = lines
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-type")
                    .then(|| value.trim().to_owned())
            })
            .expect("content type");
        Self {
            status,
            content_type,
            body: wire[separator + 4..].to_vec(),
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("JSON response")
    }

    fn sse_json(&self) -> Value {
        let body = std::str::from_utf8(&self.body).expect("UTF-8 SSE");
        assert!(body.starts_with("event: message\n"));
        assert!(body.ends_with("\n\n"));
        let data = body
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .expect("SSE data line");
        serde_json::from_str(data).expect("SSE data is JSON")
    }
}

#[test]
fn http_replays_json_and_request_scoped_sse_without_sessions() {
    let route = route_target();
    let server = HttpServer::start();
    for success in route["successes"].as_array().expect("successes") {
        let json_response = server.exchange(&success["request"], Some("application/json"));
        assert_eq!(json_response.status, 200);
        assert_eq!(
            json_response.content_type,
            "application/json; charset=utf-8"
        );
        let json_body = json_response.json();
        if success["id"] == "tools-list" {
            assert_list_result(&json_body);
            assert!(success["request"]["headers"].get("Mcp-Name").is_none());
        } else {
            assert_complete(&json_body);
            assert_eq!(
                success["request"]["headers"]["Mcp-Name"],
                "hieronymus_status"
            );
        }

        let sse_response = server.exchange(&success["request"], Some("text/event-stream"));
        assert_eq!(sse_response.status, 200);
        assert_eq!(sse_response.content_type, "text/event-stream");
        let sse_body = sse_response.sse_json();
        assert_eq!(sse_body, json_body);
        assert_complete(&sse_body);
    }
}

#[test]
fn http_replays_all_frozen_failures_with_exact_classification() {
    let route = route_target();
    let server = HttpServer::start();
    let failures = route["failures"].as_array().expect("failures");
    assert_eq!(failures.len(), 11);
    for failure in failures {
        let response = server.exchange(&failure["request"], None);
        assert_eq!(response.status, failure["response"]["status"]);
        assert_eq!(
            response.content_type,
            failure["response"]["headers"]["Content-Type"]
        );
        assert_eq!(
            response.json(),
            failure["response"]["body"],
            "case {}",
            failure["id"]
        );
    }

    let header_ids: BTreeSet<&str> = [
        "missing-version",
        "protocol-version-header-mismatch",
        "missing-mcp-method",
        "wrong-mcp-method",
        "unexpected-mcp-name-tools-list",
        "missing-mcp-name-tools-call",
        "wrong-mcp-name-tools-call",
    ]
    .into_iter()
    .collect();
    let actual_header_ids: BTreeSet<&str> = failures
        .iter()
        .filter(|case| case["response"]["body"]["error"]["code"] == -32020)
        .map(|case| case["id"].as_str().expect("case id"))
        .collect();
    assert_eq!(actual_header_ids, header_ids);

    let mismatch = failures
        .iter()
        .find(|case| case["id"] == "protocol-version-header-mismatch")
        .expect("mismatch case");
    assert_eq!(
        mismatch["request"]["headers"]["MCP-Protocol-Version"],
        "2025-06-18"
    );
    assert_eq!(
        mismatch["request"]["body"]["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        PROTOCOL_REVISION
    );
    let unsupported = failures
        .iter()
        .find(|case| case["id"] == "unsupported-version")
        .expect("unsupported case");
    assert_eq!(unsupported["response"]["body"]["error"]["code"], -32022);
    assert_eq!(
        unsupported["response"]["body"]["error"]["data"],
        json!({"requested":"2025-06-18","supported":[PROTOCOL_REVISION]})
    );
}

fn leaf_paths(value: &Value, prefix: &str, output: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                leaf_paths(child, &format!("{prefix}/{escaped}"), output);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                leaf_paths(child, &format!("{prefix}/{index}"), output);
            }
        }
        _ => {
            output.insert(prefix.to_owned());
        }
    }
}

fn changed_leaf_paths(left: &Value, right: &Value) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    leaf_paths(left, "", &mut paths);
    leaf_paths(right, "", &mut paths);
    paths
        .into_iter()
        .filter(|path| left.pointer(path) != right.pointer(path))
        .collect()
}

#[test]
fn frozen_failure_mutations_have_one_semantic_change() {
    let route = route_target();
    let successes = route["successes"].as_array().expect("successes");
    let list = &successes
        .iter()
        .find(|case| case["id"] == "tools-list")
        .unwrap()["request"];
    let call = &successes
        .iter()
        .find(|case| case["id"] == "tools-call")
        .unwrap()["request"];
    for failure in route["failures"].as_array().expect("failures") {
        let baseline = if failure["request"]["body"]["method"] == "tools/call" {
            call
        } else {
            list
        };
        let changed = changed_leaf_paths(baseline, &failure["request"]);
        if failure["id"] == "unsupported-version" {
            assert_eq!(
                changed,
                BTreeSet::from([
                    "/body/params/_meta/io.modelcontextprotocol~1protocolVersion".to_owned(),
                    "/headers/MCP-Protocol-Version".to_owned(),
                ])
            );
        } else {
            assert_eq!(
                changed.len(),
                1,
                "case {} changed {changed:?}",
                failure["id"]
            );
        }
    }
}

#[test]
fn http_rejects_session_headers_and_invalid_request_metadata() {
    let route = route_target();
    let canonical = route["successes"][0]["request"].clone();
    let server = HttpServer::start();

    for header in ["Mcp-Session-Id", "Last-Event-ID"] {
        let mut request = canonical.clone();
        request["headers"][header] = json!("forbidden-session");
        let response = server.exchange(&request, None);
        assert_eq!(response.status, 400);
        assert_eq!(response.json()["error"]["code"], -32602);
    }

    let mut invalid_requests = Vec::new();
    for direct_key in ["protocolVersion", "clientCapabilities", "clientInfo"] {
        let mut direct = canonical.clone();
        direct["body"]["params"][direct_key] = json!("legacy");
        invalid_requests.push(direct);
    }
    for required_key in [
        "io.modelcontextprotocol/protocolVersion",
        "io.modelcontextprotocol/clientCapabilities",
    ] {
        let mut missing = canonical.clone();
        missing["body"]["params"]["_meta"]
            .as_object_mut()
            .expect("metadata object")
            .remove(required_key);
        invalid_requests.push(missing);
    }
    let mut wrong_version = canonical.clone();
    wrong_version["body"]["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] =
        json!(false);
    invalid_requests.push(wrong_version);
    let mut wrong_capabilities = canonical.clone();
    wrong_capabilities["body"]["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"] =
        json!([]);
    invalid_requests.push(wrong_capabilities);
    let mut wrong_client_info = canonical.clone();
    wrong_client_info["body"]["params"]["_meta"]["io.modelcontextprotocol/clientInfo"] = json!({});
    invalid_requests.push(wrong_client_info);

    for invalid in invalid_requests {
        let response = server.exchange(&invalid, None);
        assert_eq!(response.status, 400, "request was accepted: {invalid}");
        assert_eq!(response.json()["error"]["code"], -32602);
    }

    let mut no_client_info = canonical;
    no_client_info["body"]["params"]["_meta"]
        .as_object_mut()
        .expect("metadata object")
        .remove("io.modelcontextprotocol/clientInfo");
    assert_list_result(&server.exchange(&no_client_info, None).json());
}

#[test]
fn http_mismatch_errors_never_reflect_arbitrary_header_values() {
    let route = route_target();
    let list = route["successes"][0]["request"].clone();
    let call = route["successes"][1]["request"].clone();
    let server = HttpServer::start();
    for (mut request, header_name) in [
        (list.clone(), "MCP-Protocol-Version"),
        (list, "Mcp-Method"),
        (call, "Mcp-Name"),
    ] {
        request["headers"][header_name] = json!("compat-secret-do-not-log");
        let response = server.exchange(&request, None);
        assert_eq!(response.status, 400);
        assert_eq!(response.json()["error"]["code"], -32020);
        assert!(!String::from_utf8_lossy(&response.body).contains("compat-secret-do-not-log"));
    }
}

#[test]
fn http_reports_classify_content_type_without_reflecting_request_headers() {
    let route = route_target();
    let canonical = route["successes"][0]["request"].clone();
    let canonical_bearer = canonical["headers"]["Authorization"]
        .as_str()
        .expect("bearer is a string");
    let body = serde_json::to_vec(&canonical["body"]).expect("body serializes");
    let server = HttpServer::start();

    let response = server.exchange_bytes(
        &canonical,
        &body,
        &[
            ("Content-Type", "compat-secret-do-not-log"),
            ("X-Compat-Secret", "custom-header-secret"),
        ],
    );
    assert_eq!(response.status, 400);
    assert_eq!(
        response.json(),
        json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {"code": -32602, "message": "Invalid request metadata"}
        })
    );

    let report = server.next_report();
    assert_safe_report(&report, "http");
    let report_json: Value = serde_json::from_slice(&report).expect("report is JSON");
    assert_eq!(report_json["requestContentType"], "invalid");
    let rendered = String::from_utf8_lossy(&report);
    for secret in [canonical_bearer, "custom-header-secret"] {
        assert!(!rendered.contains(secret), "report leaked request header");
    }
}

#[test]
fn http_malformed_json_returns_bounded_parse_error_and_evidence() {
    let route = route_target();
    let canonical = route["successes"][0]["request"].clone();
    let server = HttpServer::start();

    let response = server.exchange_bytes(&canonical, br#"{"broken"#, &[]);
    assert_eq!(response.status, 400);
    assert_eq!(
        response.json(),
        json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {"code": -32700, "message": "Parse error"}
        })
    );
    assert_safe_report(&server.next_report(), "http");
}

#[test]
fn http_bind_is_restricted_to_loopback() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let output = Command::new(env!("CARGO_BIN_EXE_mcp-transport"))
        .arg("http")
        .arg("--registry")
        .arg(registry_path())
        .arg("--protocol")
        .arg(protocol_path())
        .arg("--route-cases")
        .arg(route_cases_path())
        .arg("--bind")
        .arg("0.0.0.0:0")
        .arg("--ready-file")
        .arg(temporary.path().join("ready.json"))
        .output()
        .expect("harness executes");
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("/home/"));
}
