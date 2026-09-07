//! Plan M5 completeness regression: every tool the frozen registry advertises
//! must have a concrete handler in the application layer, and every handler
//! must actually execute. Registry name equality is supplementary — the
//! behavioral matrix in this file (`compatibility/rust/tool-cases-v1.json`)
//! executes every advertised tool against seeded real domain state over both
//! real transports (HTTP `POST /mcp` and the `hiero mcp` stdio adapter) and
//! asserts persisted mutations through an independent read-only SQLite
//! connection.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use hiero::application::{AppError, Application};
use hiero::daemon::McpRegistry;
use hiero::daemon::semantic_worker::{ArmedPair, SemanticArm, install_test_arm};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::semantic_embeddings::{
    EMBEDDING_DIMENSIONS, EmbeddingProvider, FakeEmbeddingProvider,
};
use hieronymus::semantic_tokenizer::ModelTokenizer;

mod common;

use common::{mcp_headers, send_request, wait_until};

// ------------------------------------------------- the semantic prerequisite

/// A deterministic offline semantic arm for the matrix daemons.
///
/// Task C5 made this a prerequisite rather than a nicety: `hieronymus_rag_search`
/// is semantic RAG search, so it refuses a data root with no semantic service
/// instead of answering with the lexical FTS lane. Every advertised tool must
/// still EXECUTE here, so the matrix daemon arms the same fake-provider seam
/// the semantic integration tests use — no model download, no ONNX runtime,
/// real durable jobs and a real query lane.
struct MatrixArm;

impl SemanticArm for MatrixArm {
    fn identity(&self) -> hieronymus::semantic_embeddings::EmbeddingIdentity {
        FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS)
            .identity()
            .clone()
    }

    fn precheck(&self, _config: &HieronymusConfig) -> Result<(), String> {
        Ok(())
    }

    fn arm(&self, _config: &HieronymusConfig) -> Result<ArmedPair, String> {
        let tokenizer = ModelTokenizer::from_bytes(include_bytes!(
            "../../hieronymus/tests/fixtures/minilm-tokenizer.json"
        ))
        .map_err(|error| error.to_string())?;
        Ok(ArmedPair {
            provider: Box::new(FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS)),
            tokenizer: Box::new(tokenizer),
        })
    }
}

/// Start a matrix daemon whose semantic service can actually reach `ready`,
/// and hand back the probe `run_case` waits on between setup and the case
/// call (the setup of an import-bearing case queues a real rebuild, so
/// readiness is reached, left, and reached again).
fn start_matrix_daemon(root: &Path) -> (hiero::daemon::Daemon, impl Fn() -> bool + use<>) {
    install_test_arm(root, Arc::new(MatrixArm));
    let daemon = common::start_daemon(root);
    let port = daemon.local_addr().port();
    let bearer = daemon.bearer().expose_secret().clone();
    let probe = move || {
        let response = send_request(
            port,
            "GET",
            "/status",
            &[("Authorization".to_string(), format!("Bearer {bearer}"))],
            b"",
        );
        response.status == 200 && response.body()["semantic"]["state"] == json!("ready")
    };
    (daemon, probe)
}

// ------------------------------------------------- the step-1 regression test

#[test]
fn every_advertised_tool_has_a_concrete_handler() {
    let registry = McpRegistry::embedded();
    let mut expected: Vec<_> = registry
        .list_tools()
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    let mut actual = Application::implemented_tools().to_vec();
    expected.sort_unstable();
    actual.sort_unstable();
    assert_eq!(actual, expected);
}

// ------------------------------------------------------- supplementary checks

#[test]
fn implemented_tools_all_dispatch_without_not_implemented() {
    // The handler list is behavioral: every listed name routes to a real
    // handler (an empty-arguments call may legitimately be invalid or a
    // domain rejection, but never `NotImplemented`).
    let root = tempfile::tempdir().unwrap();
    let application =
        Application::open(&hieronymus::data_root::HieronymusConfig::new(root.path())).unwrap();
    for name in Application::implemented_tools() {
        if *name == "hieronymus_status" {
            // Registry-backed frozen contract, served by `McpRegistry::call`
            // rather than the application dispatcher.
            continue;
        }
        if let Err(AppError::NotImplemented(tool)) =
            application.call(name, &json!({}), "local-user")
        {
            panic!("advertised tool {tool} has no concrete handler");
        }
    }
}

// ------------------------------------------------------------- the case file

const TOOL_CASES: &str = include_str!("../../../compatibility/rust/tool-cases-v1.json");

#[derive(Deserialize)]
struct CaseFile {
    cases: Vec<ToolCase>,
}

#[derive(Deserialize)]
struct ToolCase {
    name: String,
    #[serde(default)]
    setup: Vec<SetupOp>,
    arguments: Value,
    expected_subset: Value,
    #[serde(default)]
    persist: Option<PersistSpec>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SetupOp {
    /// A real `tools/call` through the transport under test; `save` captures
    /// a `structuredContent` path (dotted) for later `$name` substitution.
    Call {
        tool: String,
        #[serde(default)]
        arguments: Value,
        #[serde(default)]
        save: Option<String>,
    },
    /// A deterministic SQL seed against the data-root database for state no
    /// tool surface exists for (e.g. the audited `term_rules.rule_crystal_id`
    /// link that only the dreaming authority creates).
    Sql {
        statement: String,
        #[serde(default)]
        arguments: Vec<Value>,
    },
    /// A fixture file for import tools.
    WriteFile { path: String, contents: String },
}

#[derive(Deserialize)]
struct PersistSpec {
    table: String,
    #[serde(default, rename = "where")]
    condition: BTreeMap<String, Value>,
    #[serde(default)]
    expect_at_least: Option<usize>,
}

fn cases() -> Vec<ToolCase> {
    let file: CaseFile = serde_json::from_str(TOOL_CASES).expect("tool-cases-v1.json is valid");
    let advertised: Vec<String> = McpRegistry::embedded()
        .list_tools()
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    let case_names: Vec<String> = file.cases.iter().map(|case| case.name.clone()).collect();
    assert_eq!(
        case_names.len(),
        advertised.len(),
        "the matrix must carry one case per advertised tool"
    );
    for name in &advertised {
        assert!(
            case_names.contains(name),
            "tool {name} has no behavioral case"
        );
    }
    file.cases
}

// ------------------------------------------------------------ subset matcher

/// `contains(actual, expected)`: objects match key-by-key, arrays require
/// every expected element to be contained in some actual element, the string
/// `"*"` matches any present value, everything else compares equal.
fn contains(actual: &Value, expected: &Value) -> bool {
    match expected {
        Value::Object(entries) => entries
            .iter()
            .all(|(key, value)| actual.get(key).is_some_and(|inner| contains(inner, value))),
        Value::Array(entries) => {
            actual.is_array()
                && entries.iter().all(|entry| {
                    actual
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|inner| contains(inner, entry))
                })
        }
        Value::String(text) if text == "*" => true,
        other => actual == other,
    }
}

fn assert_subset(actual: &Value, expected: &Value, label: &str) {
    assert!(
        contains(actual, expected),
        "{label}: payload does not satisfy the expected subset\n  actual:   {actual}\n  expected: {expected}"
    );
}

// ------------------------------------------------------------------ bindings

/// Resolve placeholders in a JSON value: `<ROOT>` becomes the data root, and
/// a string that is exactly `$name` becomes a saved binding (type-preserving).
fn substitute(value: &Value, root: &Path, bindings: &BTreeMap<String, Value>) -> Value {
    match value {
        Value::String(text) => {
            let replaced = text.replace("<ROOT>", &root.display().to_string());
            if let Some(name) = replaced.strip_prefix('$')
                && let Some(bound) = bindings.get(name)
            {
                return bound.clone();
            }
            Value::String(replaced)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute(item, root, bindings))
                .collect(),
        ),
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), substitute(value, root, bindings)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn lookup_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = match current {
            Value::Object(entries) => entries.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

// ---------------------------------------------------------------- transports

/// One real transport the matrix drives end to end.
trait Transport {
    fn call_tool(&mut self, name: &str, arguments: &Value) -> Value;

    fn label(&self) -> &'static str;
}

struct HttpTransport {
    daemon: hiero::daemon::Daemon,
    next_id: i64,
}

impl Transport for HttpTransport {
    fn call_tool(&mut self, name: &str, arguments: &Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let request = tools_call(id, name, arguments);
        let response = send_request(
            self.daemon.local_addr().port(),
            "POST",
            "/mcp",
            &mcp_headers(
                &self.daemon,
                &[("Mcp-Method", "tools/call"), ("Mcp-Name", name)],
            ),
            serde_json::to_vec(&request).unwrap().as_slice(),
        );
        assert_eq!(
            response.status,
            200,
            "[http] {name}: daemon rejected the request: {:?}",
            response.body()
        );
        let body = response.body();
        assert!(
            body.get("error").is_none(),
            "[http] {name}: unexpected protocol error: {body}"
        );
        body["result"].clone()
    }

    fn label(&self) -> &'static str {
        "http"
    }
}

struct StdioTransport {
    adapter: Child,
    next_id: i64,
}

impl StdioTransport {
    fn spawn(root: &Path) -> Self {
        Self::spawn_binary(Path::new(env!("CARGO_BIN_EXE_hiero")), root)
    }

    fn spawn_binary(binary: &Path, root: &Path) -> Self {
        let adapter = common::installed::command(binary, root)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the hiero mcp adapter must spawn");
        Self {
            adapter,
            next_id: 0,
        }
    }

    /// The stdio contract: JSON-RPC answers live on stdout only; stderr is a
    /// diagnostics channel that stays empty during normal operation.
    fn finish(&mut self, case_label: &str) {
        let _ = self.adapter.kill();
        let outcome = self.adapter.wait().unwrap();
        let mut stderr = String::new();
        if let Some(handle) = self.adapter.stderr.as_mut() {
            let _ = BufReader::new(handle).read_to_string(&mut stderr);
        }
        assert!(
            stderr.is_empty(),
            "{case_label}: the stdio adapter wrote diagnostics during normal operation: {stderr}"
        );
        let _ = outcome;
    }
}

impl Transport for StdioTransport {
    fn call_tool(&mut self, name: &str, arguments: &Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let line = serde_json::to_string(&tools_call(id, name, arguments)).unwrap();
        let stdin = self.adapter.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut response_line = String::new();
        BufReader::new(self.adapter.stdout.as_mut().unwrap())
            .read_line(&mut response_line)
            .expect("the adapter must answer every request line");
        let body: Value = serde_json::from_str(response_line.trim_end()).unwrap_or_else(|error| {
            panic!(
                "[stdio] {name}: response line is not one JSON-RPC object ({error}): {response_line}"
            )
        });
        assert!(
            body.get("error").is_none(),
            "[stdio] {name}: unexpected protocol error: {body}"
        );
        body["result"].clone()
    }

    fn label(&self) -> &'static str {
        "stdio"
    }
}

/// An in-process loopback LLM standing in for the configured dream lane
/// (task D5: production dreaming runs configured providers, so the matrix
/// wires `provider.conf` at this URL through the case's `WriteFile` setup).
/// The `knowledge_crystals` answer replicates the deterministic provider's
/// crystal derivation (rule versus concept from credibility and rule
/// intent), so cases that need a dream-created crystal keep their shape.
struct DreamLoopback {
    url: String,
    stop: Arc<AtomicBool>,
    accept_thread: Option<std::thread::JoinHandle<()>>,
}

impl DreamLoopback {
    fn start() -> Self {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let accept_thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        std::thread::spawn(move || serve_dream_request(stream));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            stop,
            accept_thread: Some(accept_thread),
        }
    }
}

impl Drop for DreamLoopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            handle.join().unwrap();
        }
    }
}

fn serve_dream_request(mut stream: std::net::TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let body = read_dream_request(&mut stream);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
    let _ = std::io::Write::flush(&mut stream);
}

fn read_dream_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 8192];
    let separator = loop {
        if let Some(position) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
        match stream.read(&mut buffer) {
            Ok(0) => return String::new(),
            Ok(count) => raw.extend_from_slice(&buffer[..count]),
            Err(_) => return String::new(),
        }
    };
    let head = String::from_utf8_lossy(&raw[..separator]).to_string();
    let content_length = head
        .split("\r\n")
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = raw[separator + 4..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => body.extend_from_slice(&buffer[..count]),
            Err(_) => break,
        }
    }
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let instruction: Value = serde_json::from_str(
        request["messages"][0]["content"]
            .as_str()
            .unwrap_or_default(),
    )
    .unwrap_or(Value::Null);
    let pass = instruction["instruction"]
        .as_str()
        .unwrap_or_default()
        .split("Dream pass: ")
        .nth(1)
        .map(|rest| rest.split('.').next().unwrap_or_default())
        .unwrap_or_default()
        .to_string();
    let memory_ids: Vec<Value> = instruction["memories"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .map(|memory| memory["id"].clone())
        .collect();
    let content = if pass == "coverage_audit" {
        json!({ "covered_memory_ids": memory_ids }).to_string()
    } else if pass == "knowledge_crystals" {
        // The deterministic provider's derivation, over the wire schema.
        let crystals: Vec<Value> = instruction["memories"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|memory| {
                let rule = memory["source_credibility"] == json!("user_rule")
                    || !memory["rule_intent"].as_str().unwrap_or_default().trim().is_empty();
                json!({
                    "crystal_type": if rule { "rule" } else { "concept" },
                    "title": hieronymus::dreaming::title_from_kind(memory["kind"].as_str().unwrap_or_default()),
                    "text": hieronymus::dreaming::normalize_candidate_text(memory["text"].as_str().unwrap_or_default()),
                    "strength": 0.6,
                    "confidence": hieronymus::dreaming::source_credibility_confidence(
                        memory["source_credibility"].as_str().unwrap_or_default()),
                    "source_memory_ids": [memory["id"].clone()],
                    "source_credibility": memory["source_credibility"].clone(),
                    "rule_intent": memory["rule_intent"].clone(),
                })
            })
            .collect();
        json!({ "crystals": crystals }).to_string()
    } else {
        "{}".to_string()
    };
    json!({ "choices": [{ "message": { "content": content } }] }).to_string()
}

/// A stateless `tools/call` request carrying the exact `_meta` the protocol
/// layer mirrors.
fn tools_call(id: i64, name: &str, arguments: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": common::PROTOCOL_REVISION,
                "io.modelcontextprotocol/clientCapabilities": {},
            },
        },
    })
}

// -------------------------------------------------------------- case runner

fn run_case(
    transport: &mut dyn Transport,
    root: &Path,
    case: &ToolCase,
    dream_url: &str,
    semantic_ready: &dyn Fn() -> bool,
) {
    let label = format!("[{}] {}", transport.label(), case.name);
    let mut bindings = BTreeMap::new();

    for operation in &case.setup {
        match operation {
            SetupOp::Call {
                tool,
                arguments,
                save,
            } => {
                let resolved = substitute(arguments, root, &bindings);
                let result = transport.call_tool(tool, &resolved);
                assert!(
                    result["isError"] == json!(false),
                    "{label}: setup call {tool} failed: {}",
                    result["content"][0]["text"]
                );
                if let Some(spec) = save {
                    // `save` is `binding=path` (capture `structuredContent` at
                    // the dotted `path`, bind it as `$binding`); a bare dotted
                    // path binds under its own name.
                    let (binding, path) = match spec.split_once('=') {
                        Some((binding, path)) => (binding, path),
                        None => (spec.as_str(), spec.as_str()),
                    };
                    let value = lookup_path(&result["structuredContent"], path)
                        .unwrap_or_else(|| {
                            panic!("{label}: setup call {tool} did not produce {path}")
                        })
                        .clone();
                    bindings.insert(binding.to_string(), value);
                }
            }
            SetupOp::Sql {
                statement,
                arguments,
            } => {
                let params: Vec<rusqlite::types::Value> = arguments.iter().map(sql_value).collect();
                let connection = open_connection(root);
                let changed = connection.execute(statement, rusqlite::params_from_iter(params));
                if let Err(error) = changed {
                    panic!("{label}: SQL seed {statement:?} failed: {error}");
                }
            }
            SetupOp::WriteFile { path, contents } => {
                let contents = contents.replace("<DREAM_PROVIDER_URL>", dream_url);
                let destination = root.join(path.strip_prefix("<ROOT>/").unwrap_or(path));
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&destination, contents).unwrap();
            }
        }
    }

    // Required semantics must be serving before the case runs: a setup
    // import queues a real rebuild, and a tool whose contract is semantic
    // retrieval refuses a service that is not ready (task C5). Waiting here
    // keeps the matrix a statement about handlers rather than about timing —
    // and fails loudly if readiness never arrives.
    assert!(
        wait_until(semantic_ready, Duration::from_secs(30)),
        "{label}: the semantic service never reached ready"
    );

    // The case call itself, over the transport under test.
    let arguments = substitute(&case.arguments, root, &bindings);
    let result = transport.call_tool(&case.name, &arguments);
    assert!(
        result["isError"] == json!(false),
        "{label}: tool failed: {}",
        result["content"][0]["text"]
    );
    assert_eq!(
        result["resultType"],
        json!("complete"),
        "{label}: unexpected result type: {result}"
    );
    let rendered: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        rendered, result["structuredContent"],
        "{label}: the rendered content must be the structured content"
    );
    assert_subset(
        &result["structuredContent"],
        &substitute(&case.expected_subset, root, &bindings),
        &label,
    );

    // Persisted-mutation assertion through an independent read-only SQLite
    // connection: nothing the transports say is trusted on its own.
    if let Some(persist) = &case.persist {
        assert_persisted(root, persist, &bindings, &label);
    }
}

fn sql_value(value: &Value) -> rusqlite::types::Value {
    match value {
        Value::Bool(flag) => rusqlite::types::Value::Integer(i64::from(*flag)),
        Value::Number(number) if number.is_i64() => {
            rusqlite::types::Value::Integer(number.as_i64().unwrap())
        }
        Value::Number(number) => rusqlite::types::Value::Real(number.as_f64().unwrap()),
        Value::String(text) => rusqlite::types::Value::Text(text.clone()),
        Value::Null => rusqlite::types::Value::Null,
        other => panic!("unsupported SQL seed parameter: {other}"),
    }
}

fn open_connection(root: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(
        hieronymus::data_root::HieronymusConfig::new(root).database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    )
    .expect("the data-root database must exist once the daemon started")
}

fn assert_persisted(
    root: &Path,
    persist: &PersistSpec,
    bindings: &BTreeMap<String, Value>,
    label: &str,
) {
    let connection = rusqlite::Connection::open_with_flags(
        hieronymus::data_root::HieronymusConfig::new(root).database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("an independent read-only SQLite connection must open");
    let mut clauses = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    for (column, raw) in &persist.condition {
        let resolved = substitute(raw, root, bindings);
        clauses.push(format!("{column} = ?"));
        params.push(sql_value(&resolved));
    }
    let condition = if clauses.is_empty() {
        String::new()
    } else {
        format!(" where {}", clauses.join(" and "))
    };
    let count: i64 = connection
        .query_row(
            &format!("select count(*) from {}{}", persist.table, condition),
            rusqlite::params_from_iter(params),
            |row| row.get(0),
        )
        .unwrap_or_else(|error| {
            panic!(
                "{label}: persist check on {} failed: {error}",
                persist.table
            )
        });
    let minimum = persist.expect_at_least.unwrap_or(1);
    assert!(
        count >= minimum as i64,
        "{label}: table {} has {count} matching rows, expected at least {minimum}",
        persist.table
    );
}

// ---------------------------------------------------------------- the matrix

#[test]
fn every_case_executes_over_real_http_with_persisted_mutations() {
    for case in cases() {
        let root = tempfile::tempdir().unwrap();
        let dream_provider = DreamLoopback::start();
        let (daemon, semantic_ready) = start_matrix_daemon(root.path());
        let mut transport = HttpTransport { daemon, next_id: 0 };
        run_case(
            &mut transport,
            root.path(),
            &case,
            &dream_provider.url,
            &semantic_ready,
        );
        transport.daemon.shutdown().unwrap();
    }
}

#[test]
fn every_case_executes_over_stdio_with_persisted_mutations() {
    for case in cases() {
        let root = tempfile::tempdir().unwrap();
        // The in-process daemon publishes the discovery record the adapter
        // uses for stdio discovery (no fixed port, no baked-in credential).
        let (daemon, semantic_ready) = start_matrix_daemon(root.path());
        let dream_provider = DreamLoopback::start();
        let mut transport = StdioTransport::spawn(root.path());
        run_case(
            &mut transport,
            root.path(),
            &case,
            &dream_provider.url,
            &semantic_ready,
        );
        transport.finish(&format!("[stdio] {}", case.name));
        daemon.shutdown().unwrap();
    }
}

#[test]
fn the_tool_call_cli_drives_the_daemon_boundary() {
    // The headless adapter: a real `hiero tool-call` process reaches the
    // daemon's authenticated /mcp route with no database access of its own.
    let root = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon(root.path());
    let data_root = root.path().to_str().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "tool-call",
            "hieronymus_series_create",
            "--args",
            r#"{"slug":"book","title":"Book","source_language":"ja","target_language":"en"}"#,
            "--data-root",
            data_root,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(payload["slug"], json!("book"));

    // The mutation persisted where only the daemon could have written it.
    assert_persisted(
        root.path(),
        &PersistSpec {
            table: "series".to_string(),
            condition: BTreeMap::from([("slug".to_string(), json!("book"))]),
            expect_at_least: None,
        },
        &BTreeMap::new(),
        "[cli] tool-call",
    );

    // A domain rejection is exit 1 with the diagnostic text; a protocol
    // rejection (unknown tool) is exit 2.
    let refused = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "tool-call",
            "hieronymus_session_start",
            "--args",
            r#"{"series_slug":"ghost"}"#,
            "--data-root",
            data_root,
        ])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&refused.stdout).contains("unknown series"),
        "{}",
        String::from_utf8_lossy(&refused.stdout)
    );

    let unknown = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "tool-call",
            "hieronymus_nonexistent",
            "--data-root",
            data_root,
        ])
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("hieronymus_nonexistent"),
        "{}",
        String::from_utf8_lossy(&unknown.stderr)
    );

    // Another adapter's flag is rejected, never silently ignored.
    let wrong_flag = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "tool-call",
            "hieronymus_series_list",
            "--output",
            "somewhere.json",
            "--data-root",
            data_root,
        ])
        .output()
        .unwrap();
    assert_eq!(wrong_flag.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&wrong_flag.stderr).contains("--output"),
        "{}",
        String::from_utf8_lossy(&wrong_flag.stderr)
    );

    daemon.shutdown().unwrap();

    // Without a running daemon the adapter is honest about it.
    let offline = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "tool-call",
            "hieronymus_series_list",
            "--data-root",
            data_root,
        ])
        .output()
        .unwrap();
    assert_eq!(offline.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&offline.stderr);
    assert!(
        stderr.contains("no running local daemon was discovered"),
        "{stderr}"
    );
    assert!(stderr.contains("hiero daemon"), "{stderr}");
}

// ------------------------------------------------- the transport contracts

#[test]
fn the_legacy_api_mcp_bridge_is_gone() {
    // ADR 0015: the private /api/mcp operation bridge is removed; the MCP
    // surface is the authenticated POST /mcp route only.
    let root = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon(root.path());
    for path in ["/api/mcp", "/api/mcp/tools"] {
        let response = send_request(
            daemon.local_addr().port(),
            "POST",
            path,
            &mcp_headers(&daemon, &[]),
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert_eq!(response.status, 404, "{path}");
        assert_eq!(response.body(), json!({"error": "not_found"}), "{path}");
    }
    daemon.shutdown().unwrap();
}

#[test]
fn stdio_stdout_is_the_only_jsonrpc_channel() {
    // One framed JSON-RPC object per request line on stdout; nothing on
    // stderr during normal operation.
    let root = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon(root.path());
    let mut transport = StdioTransport::spawn(root.path());

    transport.next_id += 1;
    let request = json!({
        "jsonrpc": "2.0", "id": transport.next_id, "method": "tools/list",
        "params": {"_meta": {
            "io.modelcontextprotocol/protocolVersion": common::PROTOCOL_REVISION,
            "io.modelcontextprotocol/clientCapabilities": {},
        }}
    });
    let stdin = transport.adapter.stdin.as_mut().unwrap();
    stdin
        .write_all(serde_json::to_string(&request).unwrap().as_bytes())
        .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    BufReader::new(transport.adapter.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    let listed: Value = serde_json::from_str(line.trim_end()).expect("one JSON-RPC object");
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 39);

    // A tool call over the same session.
    let result = transport.call_tool("hieronymus_status", &json!({}));
    assert_eq!(
        result["structuredContent"]["service"]["mode"],
        json!("local-http")
    );

    transport.finish("stdio channel contract");
    daemon.shutdown().unwrap();
}

#[test]
fn default_null_and_error_contracts_hold_over_http() {
    let root = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon(root.path());
    let mut transport = HttpTransport { daemon, next_id: 0 };

    // Schema defaults apply when optional fields are omitted…
    let result = transport.call_tool(
        "hieronymus_series_create",
        &json!({"slug": "book", "title": "Book"}),
    );
    assert_eq!(result["structuredContent"]["source_language"], json!(""));
    assert_eq!(result["structuredContent"]["language_tags"], json!([]));

    // …and explicit JSON nulls are accepted where the schema allows them.
    let result = transport.call_tool(
        "hieronymus_series_create",
        &json!({"slug": "nulls", "title": "Nulls", "language_tags": null}),
    );
    assert_eq!(result["isError"], json!(false));

    // Malformed arguments are a JSON-RPC invalid-params protocol error.
    let request = tools_call(90, "hieronymus_series_create", &json!({"title": "No slug"}));
    let response = send_request(
        transport.daemon.local_addr().port(),
        "POST",
        "/mcp",
        &mcp_headers(
            &transport.daemon,
            &[
                ("Mcp-Method", "tools/call"),
                ("Mcp-Name", "hieronymus_series_create"),
            ],
        ),
        serde_json::to_vec(&request).unwrap().as_slice(),
    );
    assert_eq!(response.status, 400);
    assert_eq!(response.body()["error"]["code"], json!(-32602));

    // Domain rejections are tool error results, not protocol errors.
    let result = transport.call_tool("hieronymus_session_start", &json!({"series_slug": "ghost"}));
    assert_eq!(result["isError"], json!(true));
    assert_eq!(result["resultType"], json!("complete"));
    assert!(result.get("structuredContent").is_none());
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown series")
    );

    transport.daemon.shutdown().unwrap();
}

// ---------------------------------------------- generated bundle contracts

#[test]
fn generated_plugin_bundle_is_deterministic_bytes() {
    use hiero::agent_plugins;

    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());

    // `hiero plugins generate` is idempotent: the second run rewrites the
    // exact same bytes.
    let first = agent_plugins::generate(&config).unwrap();
    let mut bytes = BTreeMap::new();
    for path in &first {
        bytes.insert(path.clone(), std::fs::read(path).unwrap());
    }
    let second = agent_plugins::generate(&config).unwrap();
    assert_eq!(first, second, "generate must be deterministic");
    for path in &second {
        assert_eq!(
            bytes.get(path).unwrap(),
            &std::fs::read(path).unwrap(),
            "{} changed between runs",
            path.display()
        );
    }

    // The CLI surface renders the same bundle the library writes.
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "plugins",
            "generate",
            "--data-root",
            root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (path, contents) in &bytes {
        assert_eq!(
            &std::fs::read(path).unwrap(),
            contents,
            "the CLI run changed {}",
            path.display()
        );
    }

    // Another adapter's flag is rejected, never silently ignored.
    let wrong_flag = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "plugins",
            "generate",
            "--args",
            "{}",
            "--data-root",
            root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(wrong_flag.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&wrong_flag.stderr);
    assert!(stderr.contains("--args"), "{stderr}");
}

#[test]
fn export_cli_writes_deterministic_readonly_json() {
    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    let application = Application::open(&config).unwrap();
    application
        .call(
            "hieronymus_series_create",
            &json!({"slug": "book", "title": "Book"}),
            "local-user",
        )
        .unwrap();

    let database_bytes = std::fs::read(config.database_path()).unwrap();
    let export_to = |destination: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_hiero"))
            .args([
                "export",
                "--output",
                destination.to_str().unwrap(),
                "--data-root",
                root.path().to_str().unwrap(),
                "--json",
            ])
            .output()
            .unwrap()
    };
    let run_export = |destination: &std::path::Path| {
        let output = export_to(destination);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::read_to_string(destination).unwrap()
    };
    // Determinism is compared across two fresh destinations: an export never
    // overwrites, so re-running onto the same path is a refusal, not a rewrite
    // (finding A1 — the destination guard).
    let output_path = root.path().join("export").join("memory.json");
    let first = run_export(&output_path);
    let second = run_export(&root.path().join("export").join("memory-again.json"));
    assert_eq!(first, second, "export must be byte-deterministic");

    // Re-exporting onto an existing file is refused, and leaves it intact.
    let existing = export_to(&output_path);
    assert!(!existing.status.success());
    assert!(
        String::from_utf8_lossy(&existing.stderr).contains("already exists"),
        "{}",
        String::from_utf8_lossy(&existing.stderr)
    );
    assert_eq!(std::fs::read_to_string(&output_path).unwrap(), first);

    // `--force` is the deliberate overwrite affordance the refusal points at.
    let forced = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "export",
            "--output",
            output_path.to_str().unwrap(),
            "--data-root",
            root.path().to_str().unwrap(),
            "--force",
        ])
        .output()
        .unwrap();
    assert!(
        forced.status.success(),
        "{}",
        String::from_utf8_lossy(&forced.stderr)
    );
    assert_eq!(std::fs::read_to_string(&output_path).unwrap(), first);

    // The authoritative database is never a legal destination — with or
    // without `--force`.
    let onto_database = export_to(&config.database_path());
    assert!(!onto_database.status.success());
    let forced_onto_database = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "export",
            "--output",
            config.database_path().to_str().unwrap(),
            "--data-root",
            root.path().to_str().unwrap(),
            "--force",
        ])
        .output()
        .unwrap();
    assert!(!forced_onto_database.status.success());
    assert_eq!(
        database_bytes,
        std::fs::read(config.database_path()).unwrap(),
        "a refused export must leave the database byte-identical"
    );
    let document: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(document["format"], json!(hiero::export::EXPORT_FORMAT));
    assert_eq!(document["tables"]["series"][0]["slug"], json!("book"));

    // Export is read-only: the database file it serialized is untouched.
    assert_eq!(
        database_bytes,
        std::fs::read(config.database_path()).unwrap(),
        "export must never modify the database"
    );

    // An explicit destination is required.
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["export", "--data-root", root.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--output"), "{stderr}");

    // Another adapter's flag is rejected, never silently ignored.
    let wrong_flag = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "export",
            "--output",
            output_path.to_str().unwrap(),
            "--args",
            "{}",
            "--data-root",
            root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(wrong_flag.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&wrong_flag.stderr);
    assert!(stderr.contains("--args"), "{stderr}");
}

#[test]
fn stale_discovery_is_rejected_by_the_tool_client() {
    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    let daemon = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.path().into()),
        port: 0,
        ..Default::default()
    })
    .unwrap();
    let saved = std::fs::read(config.daemon_discovery_path()).unwrap();
    daemon.shutdown().unwrap();
    std::fs::write(config.daemon_discovery_path(), saved).unwrap();
    assert!(hiero::daemon_client::DaemonClient::connect(&config).is_err());
}

/// Same registry cases and persistence assertions, executed by the installed
/// daemon with its real bundled ONNX model. Dream replies remain a controlled
/// loopback provider fixture; this does not qualify a commercial provider/host.
#[test]
#[ignore = "requires disposable installed release"]
fn installed_registry_executes_both_transports() {
    struct InstalledHttp<'a>(&'a hiero::daemon_client::DaemonClient);
    impl Transport for InstalledHttp<'_> {
        fn call_tool(&mut self, name: &str, arguments: &Value) -> Value {
            self.0
                .call_tool(name, arguments)
                .expect("installed HTTP tools/call")["result"]
                .clone()
        }
        fn label(&self) -> &'static str {
            "installed-http"
        }
    }
    let binary = common::installed::binary();
    for stdio in [false, true] {
        for case in cases() {
            let root = tempfile::tempdir().unwrap();
            let mut daemon = common::installed::InstalledDaemon::start(&binary, root.path());
            let provider = DreamLoopback::start();
            if stdio {
                let mut transport = StdioTransport::spawn_binary(&binary, root.path());
                run_case(&mut transport, root.path(), &case, &provider.url, &|| {
                    daemon.ready()
                });
                transport.finish(&case.name);
            } else {
                run_case(
                    &mut InstalledHttp(&daemon.client),
                    root.path(),
                    &case,
                    &provider.url,
                    &|| daemon.ready(),
                );
            }
            daemon.client.post("/shutdown", &json!({})).unwrap();
            daemon.wait_stopped();
            eprintln!(
                "installed {} {} PASS (fixture Dream provider)",
                if stdio { "stdio" } else { "HTTP" },
                case.name
            );
        }
    }
}
