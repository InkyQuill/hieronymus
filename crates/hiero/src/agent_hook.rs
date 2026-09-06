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

use crate::lifecycle::{self, DiscoveryHealth};

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

/// The local service availability payload (the Python
/// `discover_local_service` shape): a missing discovery record means
/// "direct-local, unavailable" with the frozen reason; a record whose endpoint
/// passes the authenticated probe means "local-http, available"; anything else
/// is "health check failed" with the verdict.
///
/// The liveness judgement is ADR 0009's, not Python's: the record counts as
/// live only when the endpoint answers the authenticated `GET /status` **and**
/// reports the same process instance the record claims. The old PID check is
/// gone — ADR 0009 forbids deciding this by "PID existence alone", and a
/// recycled PID or a foreign listener on the port would both pass it.
///
/// The hook stays strictly read-only: unlike Python's `cleanup_stale_state` it
/// never deletes a stale record, and it never starts anything.
fn service_field(config: &HieronymusConfig) -> Field {
    match lifecycle::probe(config) {
        DiscoveryHealth::Live { record, .. } => Field::object(vec![
            ("available", Field::flag(true)),
            (
                "base_url",
                Field::text(format!("http://{}:{}", record.host, record.port)),
            ),
            ("mode", Field::text("local-http")),
            ("pid", Field::Count(i64::from(record.pid))),
        ]),
        // No record at all: the frozen "nothing is running" payload.
        DiscoveryHealth::NoRecord { .. } | DiscoveryHealth::Unreadable { .. } => {
            unavailable_service()
        }
        health => Field::object(vec![
            ("available", Field::flag(false)),
            ("mode", Field::text("direct-local")),
            (
                "reason",
                Field::text(format!(
                    "local service state exists but health check failed: {}",
                    health.detail()
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

    fn seeded_record(port: u16) -> crate::daemon::discovery::DiscoveryRecord {
        crate::daemon::discovery::DiscoveryRecord {
            discovery_version: crate::daemon::discovery::DISCOVERY_VERSION,
            protocol_version: crate::daemon::registry::PROTOCOL_REVISION.to_string(),
            host: "127.0.0.1".to_string(),
            port,
            // A live pid on purpose: ADR 0009 forbids deciding liveness by pid
            // existence, so the hook's verdict must not depend on this value.
            pid: std::process::id(),
            instance_id: "ab".repeat(16),
            started_at: "2026-09-04T00:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn service_payload_without_a_record_is_no_running_service() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
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
        crate::daemon::discovery::write_token(
            &config,
            &crate::daemon::discovery::generate_bearer_token().unwrap(),
        )
        .unwrap();
        // Port 1 refuses connections: the record exists but nothing answers.
        crate::daemon::discovery::write_discovery(&config, &seeded_record(1)).unwrap();
        let rendered = render(&service_field(&config));
        assert!(
            rendered.starts_with(
                "{\"available\": false, \"mode\": \"direct-local\", \
                 \"reason\": \"local service state exists but health check failed: "
            ),
            "{rendered}"
        );
    }

    #[test]
    fn a_foreign_listener_on_the_recorded_port_is_never_reported_available() {
        // The stale-port-reuse case ADR 0009 names: an unrelated process now
        // owns the port the record advertises. A bare TCP connect (and the
        // old pid check) would both call this "available"; the authenticated
        // probe must not.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                // Accept and say nothing: not an HTTP daemon.
                std::thread::sleep(std::time::Duration::from_millis(200));
                drop(stream);
            }
        });
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        crate::daemon::discovery::write_token(
            &config,
            &crate::daemon::discovery::generate_bearer_token().unwrap(),
        )
        .unwrap();
        crate::daemon::discovery::write_discovery(&config, &seeded_record(port)).unwrap();
        let rendered = render(&service_field(&config));
        assert!(rendered.contains("\"available\": false"), "{rendered}");
        assert!(!rendered.contains("local-http"), "{rendered}");
        // Read-only: the hook never repairs the record it just disproved.
        assert!(config.daemon_discovery_path().exists());
    }
}
