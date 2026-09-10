//! Shared helpers for the `hiero` daemon contract tests: raw loopback HTTP,
//! frozen fixture loading, and placeholder substitution. Deliberately
//! independent of the production HTTP client so contract checks stay
//! end-to-end.

#![allow(dead_code)]

pub mod installed;

use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const PROTOCOL_REVISION: &str = "2026-07-28";
pub const INVALID_BEARER: &str = "hieronymus-invalid-bearer";

/// The `Sec-WebSocket-Key` the frozen WS fixture carries.
pub const WS_FIXTURE_KEY: &str = "Zml4dHVyZS13ZWJzb2NrZXQta2V5";
/// The `Sec-WebSocket-Accept` the frozen key must produce
/// (base64 of SHA-1 over key + the RFC 6455 GUID).
pub const WS_FIXTURE_ACCEPT: &str = "zRmV7JySKs1VpyXat69eoJAJafs=";

/// The frozen index body (`frontend.route.get.root` target).
pub const FIXTURE_INDEX_HTML: &str = "<!doctype html><title>Hieronymus Web Console</title>";
/// The frozen JS asset body (`frontend.route.get.assets.path` target).
pub const FIXTURE_ASSET_JS: &str = "console.log('Hieronymus fixture');";

/// The fixture-world asset set: the frozen index body
/// (`frontend.route.get.root` target) and the frozen JS asset
/// (`frontend.route.get.assets.path` target, exact Content-Type). The
/// injected set carries its content types explicitly because the fixture
/// asset name has no extension to infer from.
pub fn fixture_assets() -> hiero::daemon::Assets {
    let mut entries = std::collections::BTreeMap::new();
    entries.insert(
        "index.html".to_string(),
        hiero::daemon::assets::Asset {
            content_type: "text/html; charset=utf-8",
            body: FIXTURE_INDEX_HTML.as_bytes().to_vec(),
        },
    );
    entries.insert(
        "assets/fixture".to_string(),
        hiero::daemon::assets::Asset {
            content_type: "application/javascript",
            body: FIXTURE_ASSET_JS.as_bytes().to_vec(),
        },
    );
    hiero::daemon::Assets::Memory(entries)
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn fixture(relative: &str) -> Value {
    let bytes = std::fs::read(repo_root().join(relative)).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

pub fn mcp_protocol() -> Value {
    fixture("compatibility/fixtures/mcp/protocol.json")
}

pub fn route_target(route_id: &str) -> Value {
    route_cases()["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|route| route["contract_id"] == route_id)
        .unwrap_or_else(|| panic!("route {route_id} is missing from route-cases.json"))["target"]
        .clone()
}

pub fn route_cases() -> Value {
    fixture("compatibility/fixtures/http/route-cases.json")
}

/// Replace fixture placeholders: `<PORT>` with the daemon's bound port and
/// `<INVALID_BEARER_TOKEN>` with a fixed wrong credential.
pub fn substitute_placeholders(value: &Value, port: u16) -> Value {
    match value {
        Value::String(text) => Value::String(
            text.replace("<PORT>", &port.to_string())
                .replace("<INVALID_BEARER_TOKEN>", INVALID_BEARER),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute_placeholders(item, port))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), substitute_placeholders(value, port)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Live values the REST route fixtures are normalized against: the browser
/// session and launch grant are obtained through the mint/exchange flow, the
/// rest identify the running daemon and its data root.
pub struct RouteFixture {
    pub port: u16,
    pub session: String,
    pub grant: String,
    pub pid: u32,
    pub started_at: String,
    pub data_root: std::path::PathBuf,
    /// The daemon's per-installation bearer; the oracle headers use a fixed
    /// placeholder credential that the harness swaps in.
    pub bearer: String,
}

/// Replace every fixture placeholder, including the REST-only ones. The
/// synthetic root in the Python oracle was `<root>/data`; the Rust tests root
/// the daemon directly at the temp directory.
pub fn substitute_route_placeholders(value: &Value, fixture: &RouteFixture) -> Value {
    let port = fixture.port.to_string();
    match value {
        Value::String(text) => {
            let text = text
                .replace("<PORT>", &port)
                .replace("<INVALID_BEARER_TOKEN>", INVALID_BEARER)
                .replace("<SESSION>", &fixture.session)
                .replace("<SINGLE_USE_LAUNCH_GRANT>", &fixture.grant)
                .replace("<PID>", &fixture.pid.to_string())
                .replace("<TIMESTAMP>", &fixture.started_at)
                .replace(
                    "<SYNTHETIC_ROOT>/data",
                    &fixture.data_root.to_string_lossy(),
                );
            Value::String(text)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute_route_placeholders(item, fixture))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), substitute_route_placeholders(value, fixture)))
                .collect(),
        ),
        other => other.clone(),
    }
}

pub struct RawResponse {
    pub status: u16,
    pub content_type: String,
    pub headers: BTreeMap<String, String>,
    pub raw_body: Vec<u8>,
}

impl RawResponse {
    pub fn body(&self) -> Value {
        serde_json::from_slice(&self.raw_body).unwrap_or_else(|error| {
            panic!("response body is not JSON: {error}: {:?}", self.raw_body)
        })
    }
}

pub fn send_request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> RawResponse {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        request.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    stream.write_all(request.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> RawResponse {
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("response has no header separator: {:?}", raw));
    let head = std::str::from_utf8(&raw[..separator]).unwrap();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_type = headers.get("content-type").cloned().unwrap_or_default();
    let header_map = headers;
    let full_body = &raw[separator + 4..];
    let body = match header_map
        .get("content-length")
        .and_then(|v| v.parse().ok())
    {
        Some(length) => &full_body[..length],
        None => full_body,
    };
    RawResponse {
        status,
        content_type,
        headers: header_map,
        raw_body: body.to_vec(),
    }
}

pub fn start_daemon_on_ephemeral_port() -> (tempfile::TempDir, hiero::daemon::Daemon) {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_daemon(root.path());
    (root, daemon)
}

pub fn start_daemon(root: &Path) -> hiero::daemon::Daemon {
    hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap()
}

/// Daemon with the fixture-world console assets injected (static SPA tests).
pub fn start_daemon_with_fixture_assets() -> (tempfile::TempDir, hiero::daemon::Daemon) {
    let root = tempfile::tempdir().unwrap();
    let daemon = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: fixture_assets(),
    })
    .unwrap();
    (root, daemon)
}

/// Headers every authorized POST /mcp request carries, mirroring the frozen
/// route-case success requests.
pub fn mcp_headers(
    daemon: &hiero::daemon::Daemon,
    extra: &[(&str, &str)],
) -> Vec<(String, String)> {
    let mut headers = vec![
        (
            "Authorization".to_string(),
            format!("Bearer {}", daemon.bearer().expose_secret()),
        ),
        ("Content-Type".to_string(), "application/json".to_string()),
        (
            "Accept".to_string(),
            "application/json, text/event-stream".to_string(),
        ),
        (
            "MCP-Protocol-Version".to_string(),
            PROTOCOL_REVISION.to_string(),
        ),
    ];
    for (name, value) in extra {
        headers.push(((*name).to_string(), (*value).to_string()));
    }
    headers
}

/// A browser session obtained through the real flow: bearer-authenticated
/// grant mint, then the one-time exchange for the session cookie value.
pub fn console_credential(daemon: &hiero::daemon::Daemon) -> hieronymus::secret::Secret<String> {
    let config = hieronymus::data_root::load_config(Some(daemon.data_root()));
    hiero::daemon::discovery::read_local_credential(
        &config,
        hiero::daemon::discovery::LocalCredential::Console,
    )
    .unwrap()
}

pub fn browser_session(daemon: &hiero::daemon::Daemon) -> (String, String) {
    let port = daemon.local_addr().port();
    let mint = send_request(
        port,
        "POST",
        "/auth/launch-grant",
        &[(
            "Authorization".to_string(),
            format!("Bearer {}", console_credential(daemon).expose_secret()),
        )],
        b"",
    );
    assert_eq!(
        mint.status, 200,
        "grant mint must succeed: {:?}",
        mint.raw_body
    );
    let grant = mint.body()["launch_grant"]
        .as_str()
        .unwrap_or_else(|| panic!("grant mint must return launch_grant: {:?}", mint.raw_body))
        .to_string();

    let exchange = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), format!("http://127.0.0.1:{port}")),
        ],
        format!(r#"{{"launch_grant": "{grant}"}}"#).as_bytes(),
    );
    assert_eq!(
        exchange.status, 200,
        "grant exchange must succeed: {:?}",
        exchange.raw_body
    );
    let cookie = exchange
        .headers
        .get("set-cookie")
        .unwrap_or_else(|| panic!("exchange must set the session cookie"));
    let session = cookie
        .strip_prefix("hieronymus_session=")
        .and_then(|rest| rest.split(';').next())
        .unwrap_or_else(|| panic!("unexpected Set-Cookie: {cookie}"))
        .to_string();
    (grant, session)
}

/// Daemon plus a live browser session, ready for the REST route cases.
pub fn start_daemon_with_browser_session()
-> (RouteFixture, tempfile::TempDir, hiero::daemon::Daemon) {
    let (root, daemon) = start_daemon_on_ephemeral_port();
    let (grant, session) = browser_session(&daemon);
    let fixture = RouteFixture {
        port: daemon.local_addr().port(),
        session,
        grant,
        pid: std::process::id(),
        started_at: daemon.discovery_record().started_at,
        data_root: root.path().to_path_buf(),
        bearer: daemon.bearer().expose_secret().clone(),
    };
    (fixture, root, daemon)
}

pub fn browser_headers(fixture: &RouteFixture, extra: &[(&str, String)]) -> Vec<(String, String)> {
    let mut headers = vec![(
        "Cookie".to_string(),
        format!("hieronymus_session={}", fixture.session),
    )];
    for (name, value) in extra {
        headers.push(((*name).to_string(), value.clone()));
    }
    headers
}

/// The same-origin Origin header for the running daemon.
pub fn same_origin(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Seed the admin domain data exactly like the frozen fixture oracle did:
/// one synthetic series with one active rule crystal (strength 0.8 /
/// confidence 0.9).
pub fn seed_synthetic_admin_data(root: &Path) -> i64 {
    let config = hieronymus::data_root::HieronymusConfig::new(root);
    let registry = hieronymus::registry::Registry::open(&config).unwrap();
    let series = registry
        .create_series("synthetic-series", "Synthetic Series", "ja", "en", None)
        .unwrap();
    let context = hieronymus::memory_models::TranslationContext::new(
        series.slug.clone(),
        series.source_language.clone(),
        series.target_language.clone(),
        "translation",
    );
    let crystal = hieronymus::crystals::NewCrystal {
        title: "Synthetic Rule".to_string(),
        ..hieronymus::crystals::NewCrystal::new("rule", "Use Sense, not Feeling.")
            .strength(0.8)
            .confidence(0.9)
    };
    let store = hieronymus::crystals::CrystalStore::open(&config).unwrap();
    store.add_crystal(&context, "rule", &crystal).unwrap()
}

/// Headers of an authorized same-origin websocket upgrade on `/ws/admin`,
/// mirroring the frozen fixture success request.
pub fn ws_upgrade_headers(fixture: &RouteFixture) -> Vec<(String, String)> {
    vec![
        (
            "Cookie".to_string(),
            format!("hieronymus_session={}", fixture.session),
        ),
        ("Origin".to_string(), same_origin(fixture.port)),
        ("Upgrade".to_string(), "websocket".to_string()),
        ("Sec-WebSocket-Key".to_string(), WS_FIXTURE_KEY.to_string()),
        ("Sec-WebSocket-Version".to_string(), "13".to_string()),
    ]
}

/// The parsed response head of a websocket handshake (a 101 has no body).
pub struct WsHandshake {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
}

/// A raw byte-level websocket test client: masked client frames (clients
/// always mask), deadline-bounded server-frame reads. Deliberately
/// independent of any websocket library so the tests assert exact framing.
pub struct WsClient {
    stream: TcpStream,
    buffer: Vec<u8>,
}

/// Perform the `GET /ws/admin` upgrade handshake on a fresh connection and
/// return the raw response head plus a client for frame I/O.
pub fn ws_connect(port: u16, headers: &[(String, String)]) -> (WsHandshake, WsClient) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut request = String::from("GET /ws/admin HTTP/1.1\r\n");
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        request.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !raw.windows(4).any(|window| window == b"\r\n\r\n") {
        assert!(Instant::now() < deadline, "handshake head never completed");
        let mut temporary = [0_u8; 4096];
        match stream.read(&mut temporary) {
            Ok(0) => panic!("connection closed during handshake"),
            Ok(count) => raw.extend_from_slice(&temporary[..count]),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => panic!("handshake read failed: {error}"),
        }
    }
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let head = std::str::from_utf8(&raw[..separator]).unwrap();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut response_headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            response_headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    (
        WsHandshake {
            status,
            headers: response_headers,
        },
        WsClient {
            stream,
            buffer: Vec::new(),
        },
    )
}

impl WsClient {
    /// Send a masked text frame (the client-side mask key is fixed but
    /// nonzero, exercising the unmask path).
    pub fn send_text(&mut self, text: &str) {
        let frame = encode_client_frame(0x1, text.as_bytes());
        self.stream.write_all(&frame).unwrap();
        self.stream.flush().unwrap();
    }

    /// Send a masked close frame and drop the connection.
    pub fn close(mut self) {
        let frame = encode_client_frame(0x8, b"");
        let _ = self.stream.write_all(&frame);
        let _ = self.stream.flush();
    }

    /// Read the next server frame before `deadline`: `(opcode, payload)`.
    /// `None` on EOF, clean close, or deadline.
    pub fn read_frame(&mut self, deadline: Instant) -> Option<(u8, Vec<u8>)> {
        loop {
            if let Some(frame) = parse_server_frame(&mut self.buffer) {
                return Some(frame);
            }
            if Instant::now() >= deadline {
                return None;
            }
            let mut temporary = [0_u8; 4096];
            match self.stream.read(&mut temporary) {
                Ok(0) => return None,
                Ok(count) => self.buffer.extend_from_slice(&temporary[..count]),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => return None,
            }
        }
    }
}

/// Encode a masked client frame (fixed nonzero mask key).
fn encode_client_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mask = [0x37_u8, 0x13, 0x90, 0x77];
    let mut frame = vec![0x80 | opcode];
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= 0xFFFF {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .zip(mask.iter().cycle())
            .map(|(byte, key)| byte ^ key),
    );
    frame
}

/// Parse one complete unmasked server frame from the front of `buffer`.
fn parse_server_frame(buffer: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
    if buffer.len() < 2 {
        return None;
    }
    let opcode = buffer[0] & 0x0F;
    let masked = buffer[1] & 0x80 != 0;
    let (length, header_length): (usize, usize) = match (buffer[1] & 0x7F) as usize {
        length @ 0..=125 => (length, 2),
        126 => {
            if buffer.len() < 4 {
                return None;
            }
            (u16::from_be_bytes([buffer[2], buffer[3]]) as usize, 4)
        }
        _ => {
            if buffer.len() < 10 {
                return None;
            }
            let mut bytes = [0_u8; 8];
            bytes.copy_from_slice(&buffer[2..10]);
            (u64::from_be_bytes(bytes) as usize, 10)
        }
    };
    let mask_bytes = if masked { 4 } else { 0 };
    let total = header_length.checked_add(mask_bytes)?.checked_add(length)?;
    if buffer.len() < total {
        return None;
    }
    let payload = buffer[header_length + mask_bytes..total].to_vec();
    buffer.drain(..total);
    Some((opcode, payload))
}

/// Poll `condition` until it holds or the deadline passes.
pub fn wait_until(condition: impl Fn() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    condition()
}

pub mod authority;
