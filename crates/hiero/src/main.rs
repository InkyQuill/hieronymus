//! `hiero` — the Hieronymus command-line binary. Thin presentation layer:
//! argument parsing, human/JSON output, and exit codes. Domain behavior lives
//! in the `hieronymus` library; the daemon and MCP transports live in the
//! `hiero` runtime library (ADR 0009: one binary, three execution roles).

use std::process::ExitCode;

use hiero::daemon::{DaemonOptions, run_foreground};
use hiero::stdio::{StdioOptions, run_stdio_adapter};
use hieronymus::data_root::load_config;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "usage: hiero <version|classify|daemon|mcp|recall-feedback> [--json] [--data-root <path>] [--port <n>] [--start-daemon]";
const RECALL_FEEDBACK_USAGE: &str = "usage: hiero recall-feedback --recall-id <id> --idempotency-key <key> [--useful <activation ids>] [--miss <activation ids>] [--json] [--data-root <path>]";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hiero: {error}");
            ExitCode::from(2)
        }
    }
}

struct ParsedArguments {
    json: bool,
    data_root: Option<String>,
    port: Option<u16>,
    start_daemon: bool,
    command: Option<String>,
    recall_id: Option<String>,
    useful: Option<String>,
    miss: Option<String>,
    idempotency_key: Option<String>,
}

fn parse_arguments(arguments: &[String]) -> Result<ParsedArguments, String> {
    let mut parsed = ParsedArguments {
        json: false,
        data_root: None,
        port: None,
        start_daemon: false,
        command: None,
        recall_id: None,
        useful: None,
        miss: None,
        idempotency_key: None,
    };
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        match argument.as_str() {
            "--json" => parsed.json = true,
            "--data-root" => {
                index += 1;
                parsed.data_root = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--data-root requires a path argument".to_string())?
                        .clone(),
                );
            }
            "--port" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or_else(|| "--port requires a number argument".to_string())?;
                parsed.port =
                    Some(value.parse().map_err(|_| {
                        format!("--port requires a valid port number, got {value}")
                    })?);
            }
            "--start-daemon" => parsed.start_daemon = true,
            "--recall-id" => {
                index += 1;
                parsed.recall_id = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--recall-id requires a recall id argument".to_string())?
                        .clone(),
                );
            }
            "--useful" => {
                index += 1;
                parsed.useful = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--useful requires activation id arguments".to_string())?
                        .clone(),
                );
            }
            "--miss" => {
                index += 1;
                parsed.miss = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--miss requires activation id arguments".to_string())?
                        .clone(),
                );
            }
            "--idempotency-key" => {
                index += 1;
                parsed.idempotency_key = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--idempotency-key requires a key argument".to_string())?
                        .clone(),
                );
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}"));
            }
            value => {
                if parsed.command.is_some() {
                    return Err(format!("unexpected extra argument: {value}"));
                }
                parsed.command = Some(value.to_string());
            }
        }
        index += 1;
    }
    Ok(parsed)
}

fn run(arguments: &[String]) -> Result<(), String> {
    let parsed = parse_arguments(arguments)?;
    let data_root = parsed.data_root.as_deref().map(std::path::Path::new);
    match parsed.command.as_deref() {
        Some("version") => {
            if parsed.json {
                println!("{{\"version\": \"{VERSION}\"}}");
            } else {
                println!("hiero v{VERSION}\u{03b1}");
            }
            Ok(())
        }
        Some("classify") => {
            if parsed.port.is_some() || parsed.start_daemon {
                return Err("classify does not accept --port or --start-daemon".to_string());
            }
            reject_feedback_flags(&parsed, "classify")?;
            let config = load_config(data_root);
            let state = hieronymus::db::classify_database(&config.database_path());
            if parsed.json {
                let version = state
                    .schema_version()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "null".to_string());
                println!(
                    "{{\"state\": \"{}\", \"schema_version\": {}, \"database_path\": {:?}}}",
                    state.as_str(),
                    version,
                    config.database_path()
                );
            } else {
                let suffix = match state.schema_version() {
                    Some(version) => format!(" (schema version {version})"),
                    None => String::new(),
                };
                println!("database state: {}{}", state.as_str(), suffix);
                println!("database path: {}", config.database_path().display());
            }
            Ok(())
        }
        Some("daemon") => {
            if parsed.json {
                return Err("daemon does not accept --json".to_string());
            }
            let options = DaemonOptions {
                data_root: parsed.data_root.clone().map(std::path::PathBuf::from),
                port: parsed.port.unwrap_or(hiero::daemon::DEFAULT_PORT),
                assets: hiero::daemon::Assets::default(),
            };
            run_foreground(options).map_err(|error| error.to_string())
        }
        Some("mcp") => {
            if parsed.json || parsed.port.is_some() {
                return Err("mcp does not accept --json or --port".to_string());
            }
            reject_feedback_flags(&parsed, "mcp")?;
            let options = StdioOptions {
                data_root: parsed.data_root.clone().map(std::path::PathBuf::from),
                start_daemon: parsed.start_daemon,
            };
            run_stdio_adapter(&options).map_err(|error| error.to_string())
        }
        Some("recall-feedback") => run_recall_feedback(&parsed, data_root),
        Some(other) => Err(format!("unknown command: {other}; {USAGE}")),
        None => Err(format!("missing command; {USAGE}")),
    }
}

fn reject_feedback_flags(parsed: &ParsedArguments, command: &str) -> Result<(), String> {
    let feedback_flag_used = parsed.recall_id.is_some()
        || parsed.useful.is_some()
        || parsed.miss.is_some()
        || parsed.idempotency_key.is_some();
    if feedback_flag_used {
        return Err(format!("{command} does not accept recall-feedback options"));
    }
    Ok(())
}

/// The `recall-feedback` subcommand: apply one feedback request through the
/// store and print the outcome (human or JSON).
fn run_recall_feedback(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<(), String> {
    if parsed.port.is_some() || parsed.start_daemon {
        return Err("recall-feedback does not accept --port or --start-daemon".to_string());
    }
    let recall_id = parsed
        .recall_id
        .clone()
        .ok_or_else(|| format!("recall-feedback requires --recall-id; {RECALL_FEEDBACK_USAGE}"))?;
    let idempotency_key = parsed.idempotency_key.clone().ok_or_else(|| {
        format!("recall-feedback requires --idempotency-key; {RECALL_FEEDBACK_USAGE}")
    })?;
    let parse_ids = |flag: &str, raw: &Option<String>| -> Result<Vec<i64>, String> {
        let Some(raw) = raw else {
            return Ok(Vec::new());
        };
        raw.split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .parse::<i64>()
                    .map_err(|_| format!("--{flag} expects integer activation ids, got {value}"))
            })
            .collect()
    };
    let useful = parse_ids("useful", &parsed.useful)?;
    let miss = parse_ids("miss", &parsed.miss)?;

    let config = load_config(data_root);
    let store =
        hieronymus::feedback::FeedbackStore::open(&config).map_err(|error| error.to_string())?;
    let outcome = store
        .record_recall_outcome(&hieronymus::feedback::RecallFeedback {
            recall_id: recall_id.clone(),
            useful_activation_ids: useful,
            missed_activation_ids: miss,
            idempotency_key,
        })
        .map_err(|error| error.to_string())?;

    if parsed.json {
        println!(
            "{{\"recall_id\": {:?}, \"applied\": {}, \"useful\": {}, \"miss\": {}}}",
            recall_id, outcome.applied, outcome.useful_count, outcome.miss_count
        );
    } else if outcome.applied {
        println!(
            "recall feedback applied: {} useful, {} miss (recall {recall_id})",
            outcome.useful_count, outcome.miss_count
        );
    } else {
        println!("recall feedback already applied: {recall_id}");
    }
    Ok(())
}
