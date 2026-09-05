//! `hiero agent-hook`: the Codex-style session hooks (the Python
//! `agent_hooks` behavior). `session-start` reports the nearest
//! `.hieronymus.json` project context; `session-end` reports completion. With
//! `--json` both embed the local service discovery payload, rendered exactly
//! like the Python `render_json` (`json.dumps(..., ensure_ascii=False,
//! sort_keys=True)`) so the frozen compatibility fixtures byte-match. The
//! hook is read-only: it never starts a daemon and never deletes or writes
//! state.

use std::path::Path;

use hieronymus::agent_context::discover_project_context;
use hieronymus::data_root::HieronymusConfig;

use crate::daemon::discovery::read_discovery;

/// How long the service availability probe waits per candidate address.
const SERVICE_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("failed to read the project context: {0}")]
    Context(#[from] hieronymus::agent_context::AgentContextError),
}

/// What one hook invocation prints: the JSON payload (byte-compatible with
/// the Python hook) and its human one-liner.
pub struct HookOutput {
    pub json: String,
    pub human: String,
}

/// The `session-start` invocation: nearest `.hieronymus.json` context plus
/// the service discovery payload.
pub fn session_start(cwd: &Path, config: &HieronymusConfig) -> Result<HookOutput, HookError> {
    let context = discover_project_context(cwd)?;
    let mut fields: Vec<(&'static str, Field)> = vec![
        ("event", Field::text("session-start")),
        ("handled", Field::flag(context.is_some())),
    ];
    match &context {
        None => fields.push(("reason", Field::text("no .hieronymus.json context found"))),
        Some(context) => {
            fields.push(("series_slug", Field::text(&context.series_slug)));
            fields.push(("source_language", Field::text(&context.source_language)));
            fields.push(("target_language", Field::text(&context.target_language)));
            fields.push(("task_type", Field::text(&context.task_type)));
            fields.push(("volume", Field::text(&context.volume)));
            fields.push(("chapter", Field::text(&context.chapter)));
        }
    }
    fields.push(("service", service_field(config)));
    Ok(HookOutput {
        json: render(&Field::object(fields)),
        human: match context {
            Some(_) => "Hieronymus context loaded".to_string(),
            None => "no .hieronymus.json context found".to_string(),
        },
    })
}

/// The `session-end` invocation.
pub fn session_end(config: &HieronymusConfig) -> HookOutput {
    let fields = vec![
        ("event", Field::text("session-end")),
        ("handled", Field::flag(true)),
        ("service", service_field(config)),
    ];
    HookOutput {
        json: render(&Field::object(fields)),
        human: "Hieronymus session hook complete".to_string(),
    }
}

/// One JSON value in a hook payload. Only the shapes the hook emits are
/// modeled, which is what lets the renderer byte-match the frozen fixtures.
enum Field {
    Text(String),
    Flag(bool),
    Count(i64),
    Object(Vec<(&'static str, Field)>),
}

impl Field {
    fn text(value: impl Into<String>) -> Self {
        Field::Text(value.into())
    }

    fn flag(value: bool) -> Self {
        Field::Flag(value)
    }

    fn object(mut entries: Vec<(&'static str, Field)>) -> Self {
        entries.sort_by(|left, right| left.0.cmp(right.0));
        Field::Object(entries)
    }
}

/// Renders a payload exactly like the Python hook's `render_json`
/// (`json.dumps(..., ensure_ascii=False, sort_keys=True)`): `", "`/`": "`
/// separators, alphabetically sorted keys, quote/backslash/control-character
/// escapes, and raw non-ASCII text.
fn render(field: &Field) -> String {
    let mut out = String::new();
    render_into(&mut out, field);
    out
}

fn render_into(out: &mut String, field: &Field) {
    match field {
        Field::Text(text) => write_escaped(out, text),
        Field::Flag(value) => out.push_str(if *value { "true" } else { "false" }),
        Field::Count(value) => out.push_str(&value.to_string()),
        Field::Object(entries) => {
            out.push('{');
            for (index, (key, value)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_escaped(out, key);
                out.push_str(": ");
                render_into(out, value);
            }
            out.push('}');
        }
    }
}

fn write_escaped(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// The local service availability payload (mirrors the Python
/// `discover_local_service`, including its stale-state semantics): a missing
/// or stale discovery record means "direct-local, unavailable" with the
/// frozen reason; a record whose port answers means "local-http, available".
/// This is a reachability probe only — the hook never authenticates and,
/// unlike Python's `cleanup_stale_state`, never deletes a stale record.
fn service_field(config: &HieronymusConfig) -> Field {
    let record = match read_discovery(config) {
        Ok(record) => record,
        Err(_) => return unavailable_service(),
    };
    // Python cleans a stale record up and then reports "no running local
    // service discovered"; the hook stays read-only and just reports the
    // same verdict for a record whose pid is gone.
    if !pid_alive(record.pid) {
        return unavailable_service();
    }
    match probe(&record.host, record.port) {
        Ok(()) => Field::object(vec![
            ("available", Field::flag(true)),
            (
                "base_url",
                Field::text(format!("http://{}:{}", record.host, record.port)),
            ),
            ("mode", Field::text("local-http")),
            ("pid", Field::Count(i64::from(record.pid))),
        ]),
        Err(detail) => Field::object(vec![
            ("available", Field::flag(false)),
            ("mode", Field::text("direct-local")),
            (
                "reason",
                Field::text(format!(
                    "local service state exists but health check failed: {detail}"
                )),
            ),
        ]),
    }
}

fn unavailable_service() -> Field {
    Field::object(vec![
        ("available", Field::flag(false)),
        ("mode", Field::text("direct-local")),
        ("reason", Field::text("no running local service discovered")),
    ])
}

/// Whether the recorded daemon pid is still running (linux-only target per
/// ADR 0013; our own pid counts as alive).
fn pid_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

fn probe(host: &str, port: u16) -> Result<(), String> {
    use std::net::ToSocketAddrs;
    let addresses = match (host, port).to_socket_addrs() {
        Ok(addresses) => addresses,
        Err(error) => return Err(error.to_string()),
    };
    let mut last_error = String::from("no addresses resolved");
    for address in addresses {
        match std::net::TcpStream::connect_timeout(&address, SERVICE_PROBE_TIMEOUT) {
            Ok(_) => return Ok(()),
            Err(error) => last_error = error.to_string(),
        }
    }
    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_renders_python_style_with_sorted_keys_and_escapes() {
        let payload = Field::object(vec![
            ("event", Field::text("session-start")),
            ("series_slug", Field::text("démo \"x\"\n")),
            ("handled", Field::flag(true)),
        ]);
        assert_eq!(
            render(&payload),
            "{\"event\": \"session-start\", \"handled\": true, \
             \"series_slug\": \"démo \\\"x\\\"\\n\"}"
        );
    }

    #[test]
    fn service_payload_rejects_a_stale_record_as_no_running_service() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let record = crate::daemon::discovery::DiscoveryRecord {
            discovery_version: crate::daemon::discovery::DISCOVERY_VERSION,
            protocol_version: crate::daemon::registry::PROTOCOL_REVISION.to_string(),
            host: "127.0.0.1".to_string(),
            port: 1,
            // A pid no process can have any more: Python's cleanup_stale_state
            // would have deleted this record, so the verdict is "no running
            // local service discovered" — not "health check failed".
            pid: 4_000_000_000,
            instance_id: "ab".repeat(16),
            started_at: "2026-09-04T00:00:00+00:00".to_string(),
        };
        crate::daemon::discovery::write_discovery(&config, &record).unwrap();
        let rendered = render(&service_field(&config));
        assert!(
            rendered.contains("no running local service discovered"),
            "{rendered}"
        );
        assert!(!rendered.contains("health check failed"), "{rendered}");
    }

    #[test]
    fn service_payload_failure_reason_carries_the_probe_error() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        // The test process itself is alive, so the record is not stale; port 1
        // refuses connections, which is the Python `{exc}` branch.
        let record = crate::daemon::discovery::DiscoveryRecord {
            discovery_version: crate::daemon::discovery::DISCOVERY_VERSION,
            protocol_version: crate::daemon::registry::PROTOCOL_REVISION.to_string(),
            host: "127.0.0.1".to_string(),
            port: 1,
            pid: std::process::id(),
            instance_id: "ab".repeat(16),
            started_at: "2026-09-04T00:00:00+00:00".to_string(),
        };
        crate::daemon::discovery::write_discovery(&config, &record).unwrap();
        let rendered = render(&service_field(&config));
        assert!(
            rendered.starts_with(
                "{\"available\": false, \"mode\": \"direct-local\", \
                 \"reason\": \"local service state exists but health check failed: "
            ),
            "{rendered}"
        );
    }
}
