//! `hiero` — the Hieronymus command-line binary. Thin presentation layer:
//! argument parsing, human/JSON output, and exit codes. Domain behavior lives
//! in the `hieronymus` library; the daemon and MCP transports live in the
//! `hiero` runtime library (ADR 0009: one binary, three execution roles).
//!
//! argv[0] routing (distribution spec, installer section): the installed
//! binary is reachable under the historical entry-point names. `hiero` is
//! canonical; `hieronymus` runs the same CLI; `hieronymus-mcp` runs `hiero
//! mcp`; `hieronymus-agent-hook` runs the `agent-hook` subcommand.

use std::process::ExitCode;

use hiero::agent_hook;
use hiero::daemon::{DaemonOptions, run_foreground};
use hiero::doctor;
use hiero::stdio::{StdioOptions, run_stdio_adapter};
use hieronymus::data_root::load_config;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "usage: hiero <version|classify|doctor|semantic|agent-hook|migrate|recover|daemon|mcp|recall-feedback> [--json] [--dry-run] [--data-root <path>] [--port <n>] [--start-daemon]";
const RECALL_FEEDBACK_USAGE: &str = "usage: hiero recall-feedback --recall-id <id> --idempotency-key <key> [--useful <activation ids>] [--miss <activation ids>] [--json] [--data-root <path>]";
const MIGRATE_USAGE: &str = "usage: hiero migrate [--dry-run] [--json] [--data-root <path>]";
const RECOVER_USAGE: &str = "usage: hiero recover [--json] [--data-root <path>]";
const DOCTOR_USAGE: &str = "usage: hiero doctor [--json] [--data-root <path>]";
const SEMANTIC_USAGE: &str = "usage: hiero semantic <status|enable> [--json] [--data-root <path>] (enable: [--url <u>] [--sha256 <hex>] [--bytes <n>] [--runtime <lib>])";
const AGENT_HOOK_USAGE: &str = "usage: hiero agent-hook <session-start|session-end> [--cwd <dir>] [--json] [--data-root <path>]";

/// The command this argv[0] presets, if the binary was invoked under one of
/// the compatibility link names.
fn argv0_command() -> Option<&'static str> {
    let argv0 = std::env::args_os().next()?;
    let name = std::path::Path::new(&argv0).file_name()?.to_str()?;
    match name {
        "hieronymus-mcp" => Some("mcp"),
        "hieronymus-agent-hook" => Some("agent-hook"),
        _ => None,
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(code) => code,
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
    dry_run: bool,
    command: Option<String>,
    subcommand: Option<String>,
    recall_id: Option<String>,
    useful: Option<String>,
    miss: Option<String>,
    idempotency_key: Option<String>,
    cwd: Option<String>,
    url: Option<String>,
    sha256: Option<String>,
    bytes: Option<String>,
    runtime: Option<String>,
}

fn parse_arguments(
    arguments: &[String],
    link_command: Option<&str>,
) -> Result<ParsedArguments, String> {
    let mut parsed = ParsedArguments {
        json: false,
        data_root: None,
        port: None,
        start_daemon: false,
        dry_run: false,
        command: None,
        subcommand: None,
        recall_id: None,
        useful: None,
        miss: None,
        idempotency_key: None,
        cwd: None,
        url: None,
        sha256: None,
        bytes: None,
        runtime: None,
    };
    let mut positionals: Vec<String> = Vec::new();
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
            "--dry-run" => parsed.dry_run = true,
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
            "--cwd" => {
                index += 1;
                parsed.cwd = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--cwd requires a directory argument".to_string())?
                        .clone(),
                );
            }
            "--url" => {
                index += 1;
                parsed.url = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--url requires a url argument".to_string())?
                        .clone(),
                );
            }
            "--sha256" => {
                index += 1;
                parsed.sha256 = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--sha256 requires a hex digest argument".to_string())?
                        .clone(),
                );
            }
            "--bytes" => {
                index += 1;
                parsed.bytes = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--bytes requires a byte count argument".to_string())?
                        .clone(),
                );
            }
            "--runtime" => {
                index += 1;
                parsed.runtime = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--runtime requires a library path argument".to_string())?
                        .clone(),
                );
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}"));
            }
            value => {
                if positionals.len() >= 2 {
                    return Err(format!("unexpected extra argument: {value}"));
                }
                positionals.push(value.to_string());
            }
        }
        index += 1;
    }
    // A compatibility link name presets the command; its positional (when the
    // link needs one, like agent-hook) shifts into the subcommand slot.
    let mut positionals = positionals.into_iter();
    parsed.command = match link_command {
        Some(command) => Some(command.to_string()),
        None => positionals.next(),
    };
    if parsed.command.is_none() {
        return Err(format!("missing command; {USAGE}"));
    }
    parsed.subcommand = positionals.next();
    if let Some(extra) = positionals.next() {
        return Err(format!("unexpected extra argument: {extra}"));
    }
    Ok(parsed)
}

fn run(arguments: &[String]) -> Result<ExitCode, String> {
    let parsed = parse_arguments(arguments, argv0_command())?;
    let data_root = parsed.data_root.as_deref().map(std::path::Path::new);
    match parsed.command.as_deref() {
        Some("version") => {
            if parsed.json {
                println!("{{\"version\": \"{VERSION}\"}}");
            } else {
                println!("hiero v{VERSION}\u{03b1}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("classify") => {
            if parsed.port.is_some() || parsed.start_daemon {
                return Err("classify does not accept --port or --start-daemon".to_string());
            }
            reject_subcommand(&parsed, "classify")?;
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
            Ok(ExitCode::SUCCESS)
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
            run_foreground(options).map_err(|error| error.to_string())?;
            Ok(ExitCode::SUCCESS)
        }
        Some("mcp") => {
            if parsed.json || parsed.port.is_some() {
                return Err("mcp does not accept --json or --port".to_string());
            }
            reject_subcommand(&parsed, "mcp")?;
            reject_feedback_flags(&parsed, "mcp")?;
            let options = StdioOptions {
                data_root: parsed.data_root.clone().map(std::path::PathBuf::from),
                start_daemon: parsed.start_daemon,
            };
            run_stdio_adapter(&options).map_err(|error| error.to_string())?;
            Ok(ExitCode::SUCCESS)
        }
        Some("doctor") => run_doctor(&parsed, data_root),
        Some("semantic") => run_semantic(&parsed, data_root),
        Some("agent-hook") => run_agent_hook(&parsed, data_root),
        Some("recall-feedback") => run_recall_feedback(&parsed, data_root),
        Some("migrate") => run_migrate(&parsed, data_root),
        Some("recover") => run_recover(&parsed, data_root),
        Some(other) => Err(format!("unknown command: {other}; {USAGE}")),
        None => Err(format!("missing command; {USAGE}")),
    }
}

fn reject_subcommand(parsed: &ParsedArguments, command: &str) -> Result<(), String> {
    if let Some(subcommand) = &parsed.subcommand {
        return Err(format!(
            "unexpected extra argument: {subcommand} ({command} takes no subcommand)"
        ));
    }
    Ok(())
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

/// Whether a live local daemon is reachable per its discovery record. Read
/// only: preflight reports the fact; locking the daemon is the write-side
/// upgrade protocol's job.
fn daemon_is_active(config: &hieronymus::data_root::HieronymusConfig) -> bool {
    use std::net::ToSocketAddrs;
    let Ok(record) = hiero::daemon::discovery::read_discovery(config) else {
        return false;
    };
    let Ok(addresses) = (record.host.as_str(), record.port).to_socket_addrs() else {
        return false;
    };
    for address in addresses {
        if std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(250))
            .is_ok()
        {
            return true;
        }
    }
    false
}

/// The `doctor` subcommand: non-mutating checks, human or JSON output, and
/// exit codes 0 (healthy) / 1 (degraded) / 2 (unhealthy).
fn run_doctor(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon {
        return Err(format!(
            "doctor does not accept --port or --start-daemon; {DOCTOR_USAGE}"
        ));
    }
    reject_subcommand(parsed, "doctor")?;
    reject_feedback_flags(parsed, "doctor")?;
    let config = load_config(data_root);
    let report = doctor::run(&config);
    if parsed.json {
        let text =
            serde_json::to_string_pretty(&report.to_json()).map_err(|error| error.to_string())?;
        println!("{text}");
    } else {
        print!("{}", report.render_human());
    }
    Ok(ExitCode::from(report.exit_code()))
}

/// The `semantic` subcommand: `status` is report-only; `enable` is the
/// explicit model-acquisition operation (download through the TLS-capable
/// transport) with an optional `--runtime` arming verification.
fn run_semantic(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon {
        return Err(format!(
            "semantic does not accept --port or --start-daemon; {SEMANTIC_USAGE}"
        ));
    }
    reject_feedback_flags(parsed, "semantic")?;
    let config = load_config(data_root);
    match parsed.subcommand.as_deref() {
        Some("status") => {
            let status = hieronymus::semantic_arming::semantic_status(&config)
                .map_err(|error| error.to_string())?;
            let generation = &status.active_generation;
            if parsed.json {
                let payload = serde_json::json!({
                    "model": model_status_name(&status.model_status),
                    "model_detail": model_status_detail(&status.model_status),
                    "generation": generation.as_ref().map(|manifest| serde_json::json!({
                        "generation_id": manifest.generation_id,
                        "status": manifest.status,
                        "model": manifest.identity.model(),
                        "revision": manifest.identity.revision(),
                        "dimensions": manifest.identity.dimensions(),
                        "tokenizer": manifest.identity.tokenizer(),
                    })),
                    "intact": status.generation_intact,
                    "tokenizer": status.tokenizer,
                });
                println!("{payload}");
            } else {
                println!(
                    "semantic model: {}",
                    model_status_line(&status.model_status)
                );
                match generation {
                    Some(manifest) => println!(
                        "active generation: {} ({}, {} dims, tokenizer {})",
                        manifest.generation_id,
                        manifest.identity.model(),
                        manifest.identity.dimensions(),
                        manifest.identity.tokenizer(),
                    ),
                    None => println!("active generation: none"),
                }
                println!(
                    "index intact: {}",
                    if status.generation_intact {
                        "yes"
                    } else {
                        "no"
                    }
                );
                println!(
                    "recall mode: {}",
                    if status.model_status == hieronymus::semantic_model::ModelStatus::Available {
                        "semantic + fts"
                    } else {
                        "fts-only"
                    }
                );
                println!("tokenizer: {}", status.tokenizer);
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("enable") => run_semantic_enable(parsed, &config),
        Some(other) => Err(format!(
            "unknown semantic subcommand: {other}; {SEMANTIC_USAGE}"
        )),
        None => Err(format!("semantic requires a subcommand; {SEMANTIC_USAGE}")),
    }
}

fn run_semantic_enable(
    parsed: &ParsedArguments,
    config: &hieronymus::data_root::HieronymusConfig,
) -> Result<ExitCode, String> {
    use hieronymus::semantic_model::{
        DEFAULT_MODEL_URL, HttpModelTransport, MODEL_BYTES, MODEL_SHA256, ModelStatus,
    };

    let store = hieronymus::semantic_store::SemanticStore::open(config)
        .map_err(|error| error.to_string())?;
    let url = parsed.url.as_deref().unwrap_or(DEFAULT_MODEL_URL);
    let expected_sha = parsed.sha256.as_deref().unwrap_or(MODEL_SHA256);
    let expected_bytes = match &parsed.bytes {
        None => MODEL_BYTES,
        Some(text) => text
            .parse::<u64>()
            .map_err(|_| format!("--bytes requires a byte count, got {text}"))?,
    };
    // With an explicit `--bytes` override, availability is judged solely
    // against that expectation and the user's checksum: a file that merely
    // matches the pinned size says nothing about the requested artifact, so
    // skipping the download would report an unverified `--sha256` as
    // available. The pinned default keeps Task 7's cheap size pre-check
    // (cryptographic verification happens at provider load).
    let local_status = || match &parsed.bytes {
        Some(_) => match std::fs::metadata(store.model_path()) {
            Ok(metadata) if metadata.len() == expected_bytes => {
                match hieronymus::semantic_model::sha256_file(&store.model_path()) {
                    Ok(digest) if digest == expected_sha.trim().to_ascii_lowercase() => {
                        ModelStatus::Available
                    }
                    Ok(digest) => ModelStatus::Invalid(format!(
                        "model file checksum mismatch: expected {expected_sha}, got {digest}"
                    )),
                    Err(error) => {
                        ModelStatus::Invalid(format!("model file could not be hashed: {error}"))
                    }
                }
            }
            Ok(metadata) => ModelStatus::Invalid(format!(
                "model file is {} bytes, expected {expected_bytes}",
                metadata.len()
            )),
            Err(_) => ModelStatus::Missing,
        },
        None => store.model_status(),
    };

    // Explicit acquisition: download only when the local file is missing or
    // fails its size pre-check; a healthy model is never re-fetched.
    let mut downloaded = false;
    if local_status() != ModelStatus::Available {
        let transport = HttpModelTransport::new(std::time::Duration::from_secs(600));
        store
            .acquire_model_verifying(&transport, url, expected_sha, expected_bytes)
            .map_err(|error| error.to_string())?;
        downloaded = true;
    }
    let final_status = local_status();

    // Arming verification needs the ONNX runtime library; without it the
    // model is acquired but the lane verdict stays honestly disarmed.
    let verdict = match &parsed.runtime {
        Some(runtime) => {
            hieronymus::semantic_arming::arming_verdict(config, std::path::Path::new(runtime))
        }
        None => hieronymus::semantic_arming::LaneState::Disarmed {
            reason: "onnx runtime library not provided; pass --runtime <lib> to verify arming"
                .to_string(),
        },
    };
    let runtime_verified = parsed.runtime.is_some();

    if parsed.json {
        let payload = serde_json::json!({
            "model_status": model_status_name(&final_status),
            "model_detail": model_status_detail(&final_status),
            "downloaded": downloaded,
            "model_path": store.model_path(),
            "runtime_verified": runtime_verified,
            "lane": verdict.as_str(),
            "reason": match &verdict {
                hieronymus::semantic_arming::LaneState::Armed => serde_json::Value::Null,
                hieronymus::semantic_arming::LaneState::Disarmed { reason } => {
                    serde_json::Value::String(reason.clone())
                }
            },
        });
        println!("{payload}");
    } else if downloaded {
        println!(
            "semantic model acquired: {} (checksum verified)",
            store.model_path().display()
        );
        print_lane_verdict(&verdict);
    } else {
        match &final_status {
            ModelStatus::Available => {
                println!(
                    "semantic model already acquired: {}",
                    store.model_path().display()
                );
            }
            ModelStatus::Invalid(reason) => {
                println!("semantic model could not be verified: {reason}");
            }
            ModelStatus::Missing => {
                println!("semantic model missing: {}", store.model_path().display());
            }
        }
        print_lane_verdict(&verdict);
    }
    Ok(ExitCode::SUCCESS)
}

fn print_lane_verdict(verdict: &hieronymus::semantic_arming::LaneState) {
    match verdict {
        hieronymus::semantic_arming::LaneState::Armed => {
            println!("lane armed: the semantic lane will fuse into recall");
        }
        hieronymus::semantic_arming::LaneState::Disarmed { reason } => {
            println!("lane disarmed: {reason}");
        }
    }
}

fn model_status_name(status: &hieronymus::semantic_model::ModelStatus) -> &'static str {
    match status {
        hieronymus::semantic_model::ModelStatus::Available => "available",
        hieronymus::semantic_model::ModelStatus::Invalid(_) => "invalid",
        hieronymus::semantic_model::ModelStatus::Missing => "missing",
    }
}

fn model_status_detail(status: &hieronymus::semantic_model::ModelStatus) -> serde_json::Value {
    match status {
        hieronymus::semantic_model::ModelStatus::Available
        | hieronymus::semantic_model::ModelStatus::Missing => serde_json::Value::Null,
        hieronymus::semantic_model::ModelStatus::Invalid(reason) => {
            serde_json::Value::String(reason.clone())
        }
    }
}

fn model_status_line(status: &hieronymus::semantic_model::ModelStatus) -> String {
    match status {
        hieronymus::semantic_model::ModelStatus::Available => "available".to_string(),
        hieronymus::semantic_model::ModelStatus::Missing => "missing".to_string(),
        hieronymus::semantic_model::ModelStatus::Invalid(reason) => {
            format!("invalid ({reason})")
        }
    }
}

/// The `agent-hook` subcommand (also reached through the
/// `hieronymus-agent-hook` link name): session-start/session-end outputs.
fn run_agent_hook(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon || parsed.dry_run {
        return Err(format!(
            "agent-hook does not accept --port, --start-daemon, or --dry-run; {AGENT_HOOK_USAGE}"
        ));
    }
    reject_feedback_flags(parsed, "agent-hook")?;
    let config = load_config(data_root);
    let output = match parsed.subcommand.as_deref() {
        Some("session-start") => {
            let cwd = match &parsed.cwd {
                Some(cwd) => std::path::PathBuf::from(cwd),
                None => std::env::current_dir().map_err(|error| error.to_string())?,
            };
            if !cwd.is_dir() {
                return Err(format!(
                    "--cwd must be an existing directory: {}",
                    cwd.display()
                ));
            }
            agent_hook::session_start(&cwd, &config).map_err(|error| error.to_string())?
        }
        Some("session-end") => agent_hook::session_end(&config),
        Some(other) => {
            return Err(format!(
                "unknown agent-hook subcommand: {other}; {AGENT_HOOK_USAGE}"
            ));
        }
        None => {
            return Err(format!(
                "agent-hook requires a subcommand (session-start or session-end); {AGENT_HOOK_USAGE}"
            ));
        }
    };
    if parsed.json {
        println!("{}", output.json);
    } else {
        println!("{}", output.human);
    }
    Ok(ExitCode::SUCCESS)
}

/// The `migrate` subcommand. Without `--dry-run` this is the write-side
/// upgrade protocol: fresh roots run the full cutover, interrupted roots are
/// resumed through the cutover journal (a `config_promotion_required` state
/// promotes the staged configs without rerunning the committed converters),
/// and a `complete` journal is a no-op.
fn run_migrate(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon {
        return Err(format!(
            "migrate does not accept --port or --start-daemon; {MIGRATE_USAGE}"
        ));
    }
    reject_subcommand(parsed, "migrate")?;
    reject_feedback_flags(parsed, "migrate")?;
    let config = load_config(data_root);
    if parsed.dry_run {
        let daemon_active = daemon_is_active(&config);
        let report = hieronymus::migrate::run_dry_run(&config, daemon_active)
            .map_err(|error| error.to_string())?;
        print_migrate_report(&report, parsed.json)?;
        if let Some(code) = &report.refused {
            return Err(format!("dry-run refused: {code}"));
        }
        return Ok(ExitCode::SUCCESS);
    }
    let daemon_active = daemon_is_active(&config);
    let report = hieronymus::upgrade::run_upgrade(
        &config,
        daemon_active,
        &hieronymus::upgrade::UpgradeOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    if parsed.json {
        let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
        println!("{text}");
    } else {
        print!("{}", report.render_human());
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
    }
    Ok(ExitCode::SUCCESS)
}

fn print_migrate_report(
    report: &hieronymus::migrate::DryRunReport,
    json: bool,
) -> Result<(), String> {
    if json {
        let text = serde_json::to_string_pretty(report).map_err(|error| error.to_string())?;
        println!("{text}");
    } else {
        print!("{}", report.render_human());
    }
    Ok(())
}

/// The `recover` subcommand: rebuild the live database from the last verified
/// pre-upgrade backup through the current Rust converter, then promote it
/// atomically. Never touches the immutable backup and never launches Python.
fn run_recover(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon || parsed.dry_run {
        return Err(format!(
            "recover does not accept --port, --start-daemon, or --dry-run; {RECOVER_USAGE}"
        ));
    }
    reject_subcommand(parsed, "recover")?;
    reject_feedback_flags(parsed, "recover")?;
    let config = load_config(data_root);
    let daemon_active = daemon_is_active(&config);
    let report = hieronymus::upgrade::run_recovery(&config, daemon_active)
        .map_err(|error| error.to_string())?;
    if parsed.json {
        let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
        println!("{text}");
    } else {
        println!(
            "recovery complete: rebuilt from {} ({} terms converted); \
             the replaced database was moved to {}",
            report.recovered_from.display(),
            report.converted_terms,
            report.replacement_backup_dir.display()
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// The `recall-feedback` subcommand: apply one feedback request through the
/// store and print the outcome (human or JSON).
fn run_recall_feedback(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon {
        return Err("recall-feedback does not accept --port or --start-daemon".to_string());
    }
    reject_subcommand(parsed, "recall-feedback")?;
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
    Ok(ExitCode::SUCCESS)
}
