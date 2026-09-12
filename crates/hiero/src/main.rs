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
use hiero::{lifecycle, project_context, service, uninstall, update};
use hieronymus::data_root::load_config;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "usage: hiero <version|start|stop|restart|status|tray|desktop|admin|config|classify|project-context|doctor|semantic|agent-hook|migrate|recover|service|update|uninstall|daemon|mcp|recall-feedback|tool-call|export|plugins> [--json] [--dry-run] [--data-root <path>] [--port <n>] [--start-daemon]";
const CONSOLE_USAGE: &str = "usage: hiero <admin|config> [--data-root <path>] (opens the authenticated web console in your browser; starts the local daemon if needed)";
const LIFECYCLE_USAGE: &str = "usage: hiero <start|stop|restart|status> [--json] [--data-root <path>] [--unit-dir <dir>] [--binary <path>]";
const RECALL_FEEDBACK_USAGE: &str = "usage: hiero recall-feedback --recall-id <id> --idempotency-key <key> [--useful <activation ids>] [--miss <activation ids>] [--json] [--data-root <path>] (requires the local daemon)";
const TOOL_CALL_USAGE: &str = "usage: hiero tool-call <tool> [--args <json>] [--json] [--data-root <path>] [--start-daemon] (calls the advertised MCP tool through the local daemon's authenticated /mcp route)";
const EXPORT_USAGE: &str = "usage: hiero export --output <path> [--force] [--json] [--data-root <path>] (read-only JSON serialization; never a database file copy. An existing destination is refused unless --force replaces it, and no path this installation owns is ever a legal destination)";
const PLUGINS_USAGE: &str = "usage: hiero plugins generate [--dry-run] [--json] [--data-root <path>] (writes the installation-owned agent plugin bundle)";
const MIGRATE_USAGE: &str = "usage: hiero migrate [--dry-run] [--json] [--data-root <path>]";
const RECOVER_USAGE: &str = "usage: hiero recover [--json] [--data-root <path>]";
const DOCTOR_USAGE: &str =
    "usage: hiero doctor [--json] [--data-root <path>] [--unit-dir <path>] [--skip-registration]";
const SEMANTIC_USAGE: &str = "usage: hiero semantic <status|enable|configure> [--json] [--data-root <path>] (configure: --provider ollama --base-url <origin> --model <installed-model>) (enable: [--url <u>] [--sha256 <hex>] [--bytes <n>] [--runtime <lib>])";
const AGENT_HOOK_USAGE: &str = "usage: hiero agent-hook <session-start|session-end|bind-context|user-prompt-submit|retry-delivery> [--host <claude|codex|zcode>] [--delivery-id <uuid>] [--cwd <dir>] [--json] [--data-root <path>]";
const PROJECT_CONTEXT_USAGE: &str = "usage: hiero project-context [--cwd <path>] [--args '{\"direction_id\":null}'] [--json] [--data-root <path>]";

const SERVICE_USAGE: &str = "usage: hiero service <install|uninstall|status|start|stop> [--json] [--data-root <path>] [--unit-dir <dir>] [--binary <path>] (install: [--no-activate]; status exits 0 when the unit is installed and consistent, 1 otherwise)";
const UPDATE_USAGE: &str = "usage: hiero update (--release-dir <dir> | --release-url <https-base>) [--channel stable|dev] [--app-dir <dir>] [--data-root <path>] [--unit-dir <dir>] [--json]";
const UNINSTALL_USAGE: &str = "usage: hiero uninstall [--yes] [--delete-data] [--app-dir <dir>] [--data-root <path>] [--unit-dir <dir>] [--json]";

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
    hook_host: Option<String>,
    delivery_id: Option<String>,
    url: Option<String>,
    sha256: Option<String>,
    bytes: Option<String>,
    runtime: Option<String>,
    embedding_provider: Option<String>,
    embedding_url: Option<String>,
    embedding_model: Option<String>,
    unit_dir: Option<String>,
    binary: Option<String>,
    no_activate: bool,
    skip_registration: bool,
    release_dir: Option<String>,
    release_url: Option<String>,
    channel: Option<String>,
    app_dir: Option<String>,
    yes: bool,
    delete_data: bool,
    args_json: Option<String>,
    output: Option<String>,
    /// `hiero export --force`: replace an existing destination instead of
    /// refusing it. Never relaxes the destination guard itself.
    force: bool,
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
        hook_host: None,
        delivery_id: None,
        url: None,
        sha256: None,
        bytes: None,
        runtime: None,
        embedding_provider: None,
        embedding_url: None,
        embedding_model: None,
        unit_dir: None,
        binary: None,
        no_activate: false,
        skip_registration: false,
        release_dir: None,
        release_url: None,
        channel: None,
        app_dir: None,
        yes: false,
        delete_data: false,
        args_json: None,
        output: None,
        force: false,
    };
    let mut positionals: Vec<String> = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        match argument.as_str() {
            "--json" => parsed.json = true,
            "--skip-registration" => parsed.skip_registration = true,
            "--host" | "--delivery-id" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or_else(|| format!("{argument} requires a value"))?
                    .clone();
                if argument == "--host" {
                    parsed.hook_host = Some(value)
                } else {
                    parsed.delivery_id = Some(value)
                }
            }
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
            "--provider" | "--base-url" | "--model" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or_else(|| format!("{argument} requires a value"))?
                    .clone();
                match argument.as_str() {
                    "--provider" => parsed.embedding_provider = Some(value),
                    "--base-url" => parsed.embedding_url = Some(value),
                    _ => parsed.embedding_model = Some(value),
                }
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
            "--unit-dir" => {
                index += 1;
                parsed.unit_dir = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--unit-dir requires a directory argument".to_string())?
                        .clone(),
                );
            }
            "--binary" => {
                index += 1;
                parsed.binary = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--binary requires a path argument".to_string())?
                        .clone(),
                );
            }
            "--release-url" | "--channel" => {
                let flag = arguments[index].clone();
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or_else(|| format!("{flag} requires a value"))?
                    .clone();
                if flag == "--channel" {
                    parsed.channel = Some(value);
                } else {
                    parsed.release_url = Some(value);
                }
            }
            "--release-dir" => {
                index += 1;
                parsed.release_dir = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--release-dir requires a directory argument".to_string())?
                        .clone(),
                );
            }
            "--app-dir" => {
                index += 1;
                parsed.app_dir = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--app-dir requires a directory argument".to_string())?
                        .clone(),
                );
            }
            "--no-activate" => parsed.no_activate = true,
            "--yes" => parsed.yes = true,
            "--delete-data" => parsed.delete_data = true,
            "--args" => {
                index += 1;
                parsed.args_json = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--args requires a JSON object argument".to_string())?
                        .clone(),
                );
            }
            "--output" => {
                index += 1;
                parsed.output = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--output requires a path argument".to_string())?
                        .clone(),
                );
            }
            "--force" => parsed.force = true,
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
    if parsed.skip_registration && parsed.command.as_deref() != Some("doctor") {
        return Err("--skip-registration is only supported by doctor".into());
    }
    if parsed.command.is_none() {
        return Err(format!("missing command; {USAGE}"));
    }
    parsed.subcommand = positionals.next();
    if let Some(extra) = positionals.next() {
        return Err(format!("unexpected extra argument: {extra}"));
    }
    if parsed.command.as_deref() != Some("agent-hook")
        && (parsed.hook_host.is_some() || parsed.delivery_id.is_some())
    {
        return Err("--host and --delivery-id belong to agent-hook".into());
    }
    if (parsed.embedding_provider.is_some()
        || parsed.embedding_url.is_some()
        || parsed.embedding_model.is_some())
        && (parsed.command.as_deref() != Some("semantic")
            || parsed.subcommand.as_deref() != Some("configure"))
    {
        return Err("--provider, --base-url and --model require semantic configure".into());
    }
    Ok(parsed)
}

fn run(arguments: &[String]) -> Result<ExitCode, String> {
    #[cfg(target_os = "macos")]
    if arguments == ["__macos-native-broker"] {
        return hiero::platform::macos_broker::run().map(|_| ExitCode::SUCCESS);
    }
    #[cfg(windows)]
    if arguments == ["__windows-native-broker"] {
        return hiero::platform::windows_broker::run().map(|_| ExitCode::SUCCESS);
    }
    if argv0_command().is_none() && arguments.first().map(String::as_str) == Some("desktop") {
        #[cfg(windows)]
        return hiero::desktop::windows_registration::run(&arguments[1..])
            .map(|_| ExitCode::SUCCESS);
        #[cfg(target_os = "macos")]
        return hiero::desktop::macos_registration::run(&arguments[1..]).map(|_| ExitCode::SUCCESS);
        #[cfg(not(any(windows, target_os = "macos")))]
        return hiero::desktop::linux_cli::run(&arguments[1..]).map(|_| ExitCode::SUCCESS);
    }
    let parsed = parse_arguments(arguments, argv0_command())?;
    if parsed.command.as_deref() == Some("project-context") {
        return run_project_context(&parsed);
    }
    let data_root = parsed.data_root.as_deref().map(std::path::Path::new);
    match parsed.command.as_deref() {
        Some("version") => {
            if parsed.json {
                // The update flow probes exactly this payload to learn what
                // a candidate binary supports before touching anything.
                println!(
                    "{{\"version\": \"{VERSION}\", \"protocol_revision\": \"{}\", \
                     \"supported_schema_version\": {}}}",
                    hiero::daemon::registry::PROTOCOL_REVISION,
                    hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION,
                );
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
            reject_headless_flags(&parsed, "classify")?;
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
                assets: hiero::daemon::Assets::release(),
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
            reject_headless_flags(&parsed, "mcp")?;
            let options = StdioOptions {
                data_root: parsed.data_root.clone().map(std::path::PathBuf::from),
                start_daemon: parsed.start_daemon,
            };
            run_stdio_adapter(&options).map_err(|error| error.to_string())?;
            Ok(ExitCode::SUCCESS)
        }
        // The top-level lifecycle commands (ADR 0009). They operate through
        // the per-user service integration and the authenticated discovery
        // record; `hiero service <...>` remains the lower-level unit surface.
        Some(command @ ("start" | "stop" | "restart" | "status")) => {
            run_lifecycle(command, &parsed, data_root)
        }
        Some("tray") => {
            reject_subcommand(&parsed, "tray")?;
            reject_feedback_flags(&parsed, "tray")?;
            reject_headless_flags(&parsed, "tray")?;
            if parsed.port.is_some() || parsed.start_daemon || parsed.json || parsed.dry_run {
                return Err("usage: hiero tray [--data-root <path>]".into());
            }
            #[cfg(any(windows, target_os = "macos"))]
            hiero::desktop::launch::launch_with_options(
                &load_config(data_root),
                &service_options(&parsed, data_root)?,
            )?;
            #[cfg(not(any(windows, target_os = "macos")))]
            hiero::desktop::launch::launch(&load_config(data_root))?;
            Ok(ExitCode::SUCCESS)
        }
        // The authenticated web console launchers mint a one-time launch grant.
        // Never print the grant, the bearer, or the URL.
        Some(page @ ("admin" | "config")) => run_console(page, &parsed, data_root),
        Some("doctor") => run_doctor(&parsed, data_root),
        Some("semantic") => run_semantic(&parsed, data_root),
        Some("agent-hook") => run_agent_hook(&parsed, data_root),
        Some("recall-feedback") => run_recall_feedback(&parsed, data_root),
        Some("tool-call") => run_tool_call(&parsed, data_root),
        Some("export") => run_export(&parsed, data_root),
        Some("plugins") => run_plugins(&parsed, data_root),
        Some("migrate") => run_migrate(&parsed, data_root),
        Some("recover") => run_recover(&parsed, data_root),
        Some("service") => run_service(&parsed, data_root),
        Some("release-ready") => {
            update::require_release_ready(&load_config(data_root))?;
            println!("release ready: authenticated version and semantic lane verified");
            Ok(ExitCode::SUCCESS)
        }
        Some("release-assets") => {
            let directory = parsed
                .output
                .as_deref()
                .map(absolute_path)
                .unwrap_or_else(|| {
                    std::env::current_exe()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .to_path_buf()
                });
            let manifest = hiero::app::verify_semantic_assets(&directory)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?
            );
            Ok(ExitCode::SUCCESS)
        }
        Some("release-verify") => {
            let directory = absolute_path(
                parsed
                    .release_dir
                    .as_deref()
                    .ok_or("release-verify requires --release-dir")?,
            );
            if directory
                .join(hiero::release_manifest::metadata_name(
                    hiero::app::TARGET_TRIPLE,
                ))
                .try_exists()
                .map_err(|e| e.to_string())?
            {
                let release = if let Some(output) = &parsed.output {
                    hiero::release_archive::extract_split_directory(
                        &directory,
                        hiero::app::TARGET_TRIPLE,
                        &absolute_path(output),
                    )?
                } else {
                    hiero::release_archive::verify_split_directory(
                        &directory,
                        hiero::app::TARGET_TRIPLE,
                    )?
                };
                println!(
                    "verified split release {} ({})",
                    release.manifest.version,
                    hiero::app::TARGET_TRIPLE
                );
                return Ok(ExitCode::SUCCESS);
            }
            let release = hiero::release_source::verify_directory(&directory)?;
            if let Some(output) = &parsed.output {
                hiero::release_source::extract_archive(&release.archive, &absolute_path(output))?;
            }
            println!(
                "verified release {} ({})",
                release.version,
                hiero::app::TARGET_TRIPLE
            );
            Ok(ExitCode::SUCCESS)
        }
        Some("desktop-bootstrap") => {
            let options = update::UpdateOptions {
                release_dir: parsed
                    .release_dir
                    .as_deref()
                    .map(absolute_path)
                    .ok_or("desktop-bootstrap requires --release-dir")?,
                app_dir: Some(
                    parsed
                        .app_dir
                        .as_deref()
                        .map(absolute_path)
                        .unwrap_or_else(hiero::app::default_app_dir),
                ),
                data_root: parsed.data_root.as_deref().map(absolute_path),
                unit_dir: parsed.unit_dir.as_deref().map(absolute_path),
            };
            let report = update::run_desktop_install(&options, parsed.no_activate)
                .map_err(|e| e.to_string())?;
            println!("{}", report.render_human());
            Ok(ExitCode::SUCCESS)
        }
        Some("update") => run_update_command(&parsed),
        Some("uninstall") => run_uninstall_command(&parsed),
        Some(other) => Err(format!("unknown command: {other}; {USAGE}")),
        None => Err(format!("missing command; {USAGE}")),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectContextArguments {
    #[serde(default)]
    direction_id: Option<String>,
}

fn run_project_context(parsed: &ParsedArguments) -> Result<ExitCode, String> {
    reject_subcommand(parsed, "project-context")?;
    reject_feedback_flags(parsed, "project-context")?;
    reject_output_flag(parsed, "project-context")?;
    if parsed.port.is_some() || parsed.start_daemon || parsed.dry_run {
        return Err(format!(
            "project-context does not accept --port, --start-daemon, or --dry-run; {PROJECT_CONTEXT_USAGE}"
        ));
    }
    if parsed.force
        || parsed.no_activate
        || parsed.unit_dir.is_some()
        || parsed.binary.is_some()
        || parsed.url.is_some()
        || parsed.sha256.is_some()
        || parsed.bytes.is_some()
        || parsed.runtime.is_some()
        || parsed.release_dir.is_some()
        || parsed.release_url.is_some()
        || parsed.channel.is_some()
        || parsed.app_dir.is_some()
        || parsed.yes
        || parsed.delete_data
    {
        return Err(format!(
            "project-context received an unrelated option; {PROJECT_CONTEXT_USAGE}"
        ));
    }
    let arguments: ProjectContextArguments = match parsed.args_json.as_deref() {
        Some(raw) => serde_json::from_str(raw).map_err(|error| {
            format!("--args must contain only direction_id:string|null: {error}")
        })?,
        None => ProjectContextArguments { direction_id: None },
    };
    let cwd = match &parsed.cwd {
        Some(cwd) => std::path::PathBuf::from(cwd),
        None => std::path::PathBuf::from("."),
    };
    let report = project_context::inspect(&cwd, arguments.direction_id.as_deref());
    if parsed.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
        );
    } else {
        print!("{}", project_context::render_human(&report));
    }
    Ok(ExitCode::from(project_context::exit_code(&report)))
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

/// `--args` and `--output` belong to the headless adapters (`tool-call`,
/// `export`); other commands reject them instead of silently ignoring a flag
/// the user may believe took effect.
fn reject_headless_flags(parsed: &ParsedArguments, command: &str) -> Result<(), String> {
    if command != "agent-hook" && (parsed.hook_host.is_some() || parsed.delivery_id.is_some()) {
        return Err("--host and --delivery-id belong to agent-hook".into());
    }
    reject_args_flag(parsed, command)?;
    reject_output_flag(parsed, command)
}

/// Each headless command also rejects the OTHER adapters' flags: `tool-call`
/// takes `--args` but not `--output`, `export` takes `--output` but not
/// `--args`, and `plugins` takes neither — no headless flag is ever silently
/// ignored.
fn reject_args_flag(parsed: &ParsedArguments, command: &str) -> Result<(), String> {
    if parsed.args_json.is_some() {
        return Err(format!("{command} does not accept --args (see tool-call)"));
    }
    Ok(())
}

fn reject_output_flag(parsed: &ParsedArguments, command: &str) -> Result<(), String> {
    if parsed.output.is_some() {
        return Err(format!("{command} does not accept --output (see export)"));
    }
    Ok(())
}

/// Whether a live local daemon is reachable per its discovery record. Read
/// only: preflight reports the fact; locking the daemon is the write-side
/// upgrade protocol's and the update flow's job.
fn daemon_is_active(config: &hieronymus::data_root::HieronymusConfig) -> bool {
    hiero::lifecycle::probe(config).is_live()
}

/// Make a user-supplied path absolute against the current working directory.
/// Service units and stable links carry absolute paths by design, so relative
/// `--unit-dir`/`--app-dir`/`--data-root`/`--binary` values are resolved at
/// the CLI boundary instead of failing deep inside the flows.
fn absolute_path(path: impl AsRef<std::path::Path>) -> std::path::PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    }
}

/// `hiero admin` / `hiero config`: open the authenticated web console. The
/// grant, the bearer, and the launch URL fragment never touch stdout/stderr;
/// only the page name and, on opener failure, the origin and page path (never
/// the fragment/grant) are printed.
fn run_console(
    page: &str,
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.json {
        return Err(format!("{page} does not accept --json; {CONSOLE_USAGE}"));
    }
    if parsed.port.is_some() || parsed.start_daemon || parsed.dry_run {
        return Err(format!(
            "{page} does not accept --port, --start-daemon, or --dry-run; {CONSOLE_USAGE}"
        ));
    }
    reject_subcommand(parsed, page)?;
    reject_feedback_flags(parsed, page)?;
    reject_headless_flags(parsed, page)?;
    let config = load_config(data_root);
    hiero::console::launch(&config, page)?;
    println!("opening the {page} console in your browser");
    Ok(ExitCode::SUCCESS)
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
    reject_headless_flags(parsed, "doctor")?;
    let config = load_config(data_root);
    let unit_dir = parsed.unit_dir.as_deref().map(absolute_path);
    let report = if parsed.skip_registration {
        doctor::run_without_registration(&config)
    } else {
        doctor::run_with_service(&config, unit_dir.as_deref())
    };
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
    reject_headless_flags(parsed, "semantic")?;
    let config = load_config(data_root);
    match parsed.subcommand.as_deref() {
        Some("status") => {
            let status = hieronymus::semantic_arming::semantic_status(&config)
                .map_err(|error| error.to_string())?;
            let generation = &status.active_generation;
            let ollama = status
                .configuration
                .as_ref()
                .is_some_and(|c| c.provider == "ollama");
            let live = hiero::lifecycle::connect(&config, false)
                .ok()
                .and_then(|client| client.get("/status").ok())
                .map(|value| value["semantic"].clone());
            if parsed.json {
                let payload = serde_json::json!({
                    "configuration": status.configuration,
                    "daemon": live,
                    "model": if ollama { "external" } else { model_status_name(&status.model_status) },
                    "model_detail": model_status_detail(&status.model_status),
                    "generation": generation.as_ref().map(|manifest| serde_json::json!({
                        "provider": manifest.identity.provider(),
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
                if let Some(settings) = &status.configuration {
                    println!(
                        "semantic provider: {} (configuration revision {})",
                        settings.provider, settings.configuration_revision
                    );
                    if ollama {
                        println!(
                            "Ollama model: {} at {}",
                            settings.model.as_deref().unwrap(),
                            settings.base_url.as_deref().unwrap()
                        );
                    }
                }
                if !ollama {
                    println!(
                        "semantic model: {}",
                        model_status_line(&status.model_status)
                    );
                }
                println!(
                    "daemon semantic status: {}",
                    live.as_ref()
                        .map_or_else(|| "unavailable".into(), ToString::to_string)
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
                println!("tokenizer: {}", status.tokenizer);
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("enable") => run_semantic_enable(parsed, &config),
        Some("configure") => {
            if parsed.embedding_provider.as_deref() != Some("ollama")
                || parsed.runtime.is_some()
                || parsed.url.is_some()
                || parsed.sha256.is_some()
                || parsed.bytes.is_some()
            {
                return Err(format!(
                    "configure requires --provider ollama and accepts --base-url and --model; {SEMANTIC_USAGE}"
                ));
            }
            let settings = hieronymus::semantic_arming::SemanticConfiguration::ollama(
                parsed
                    .embedding_url
                    .as_deref()
                    .ok_or("--base-url is required")?,
                parsed
                    .embedding_model
                    .as_deref()
                    .ok_or("--model is required")?,
            );
            settings.validate()?;
            let client = hiero::lifecycle::connect(&config, false).map_err(|e| e.to_string())?;
            // Explicit acquisition of the pinned segmentation asset only. No ONNX model/runtime or Ollama pull.
            client
                .post_with_timeout(
                    "/semantic/acquire",
                    &serde_json::json!({"tokenizer_only":true}),
                    std::time::Duration::from_secs(610),
                )
                .map_err(|e| e.to_string())?;
            let result = client
                .post(
                    "/semantic/configure",
                    &serde_json::to_value(settings).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            println!("{result}");
            Ok(ExitCode::SUCCESS)
        }
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
    let client = hiero::lifecycle::connect(config, false).map_err(|error| error.to_string())?;
    // Relative CLI paths have an explicit base: the invoking process cwd.
    // Persist only a canonical absolute path, never a daemon-dependent one.
    let runtime = parsed
        .runtime
        .as_ref()
        .map(|path| {
            std::fs::canonicalize(path).map_err(|error| format!("runtime_library: {error}"))
        })
        .transpose()?;
    let mut result = client
        .post_with_timeout(
            "/semantic/acquire",
            &serde_json::json!({
                "url": parsed.url, "sha256": parsed.sha256, "bytes": parsed.bytes,
            }),
            std::time::Duration::from_secs(1210),
        )
        .map_err(|error| error.to_string())?;
    let configured = match runtime {
        Some(runtime) => {
            let mut configured = client
                .post(
                    "/semantic/configure",
                    &serde_json::json!({"runtime_library": runtime}),
                )
                .map_err(|error| error.to_string())?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while configured["state"] == "acquiring" && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let status = client.get("/status").map_err(|error| error.to_string())?;
                if configured["configuration_revision"]
                    != status["semantic"]["configuration_revision"]
                {
                    return Err(
                        "semantic configuration changed concurrently; inspect daemon status".into(),
                    );
                }
                configured["state"] = status["semantic"]["state"].clone();
                configured["detail"] = status["semantic"]["detail"].clone();
            }
            configured
        }
        None => {
            serde_json::json!({"state": "failed", "detail": "runtime not provided; pass --runtime <lib> to configure the daemon"})
        }
    };
    let ready = configured["state"] == "ready";
    result["runtime_saved"] = serde_json::json!(parsed.runtime.is_some());
    result["runtime_verified"] = serde_json::json!(ready);
    result["lane"] = serde_json::json!(if ready { "armed" } else { "disarmed" });
    result["reason"] = configured["detail"].clone();
    result["state"] = configured["state"].clone();
    result["configuration_revision"] = configured["configuration_revision"].clone();
    if parsed.json {
        println!("{result}");
    } else {
        println!(
            "semantic model {}: {}",
            if result["downloaded"] == true {
                "acquired"
            } else {
                "already acquired"
            },
            result["model_path"]
        );
        println!(
            "daemon semantic state: {} ({})",
            result["state"], result["reason"]
        );
    }
    Ok(ExitCode::SUCCESS)
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
    reject_headless_flags(parsed, "agent-hook")?;
    let config = load_config(data_root);
    if matches!(
        parsed.subcommand.as_deref(),
        Some("bind-context" | "user-prompt-submit" | "retry-delivery")
    ) {
        use hiero::agent_prompt_delivery as delivery;
        if parsed.cwd.is_some() {
            return Err("trusted prompt commands read host context from stdin, not --cwd".into());
        }
        let result = match parsed.subcommand.as_deref() {
            Some("bind-context") if parsed.hook_host.is_none() && parsed.delivery_id.is_none() => {
                let input =
                    delivery::read_json(std::io::stdin().lock()).map_err(|e| e.to_string())?;
                delivery::bind_context(&config, &input)
            }
            Some("user-prompt-submit") if parsed.delivery_id.is_none() => {
                let host = parsed
                    .hook_host
                    .as_deref()
                    .ok_or("user-prompt-submit requires --host")?;
                let input =
                    delivery::read_json(std::io::stdin().lock()).map_err(|e| e.to_string())?;
                delivery::handle_prompt(&config, host, &input)
            }
            Some("retry-delivery") if parsed.hook_host.is_none() => {
                let id = parsed
                    .delivery_id
                    .as_deref()
                    .ok_or("retry-delivery requires --delivery-id")?;
                delivery::retry_delivery(&config, id).map(|v| delivery::hook_output(&v))
            }
            _ => return Err("invalid trusted hook flags".into()),
        }
        .map_err(|e| e.to_string())?;
        println!("{result}");
        return Ok(ExitCode::SUCCESS);
    }
    if parsed.hook_host.is_some() || parsed.delivery_id.is_some() {
        return Err("host/delivery flags require a prompt command".into());
    }
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
    reject_headless_flags(parsed, "migrate")?;
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
    reject_headless_flags(parsed, "recover")?;
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
/// local daemon's `POST /recall/feedback` route (the audited write path) and
/// print the outcome (human or JSON). This path contains no
/// `FeedbackStore::open` and no direct database connection — when the daemon
/// is not running the command reports that honestly instead of writing
/// around it.
fn run_recall_feedback(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon {
        return Err("recall-feedback does not accept --port or --start-daemon".to_string());
    }
    reject_subcommand(parsed, "recall-feedback")?;
    reject_headless_flags(parsed, "recall-feedback")?;
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
    // The daemon boundary (plan M5; the runtime plan's `lifecycle::DaemonClient`
    // swaps in behind the same seam): the same at-most-once contract the REST
    // route enforces, so the CLI and daemon share one audit ledger.
    let client =
        hiero::daemon_client::DaemonClient::connect(&config).map_err(|error| error.to_string())?;
    let payload = serde_json::json!({
        "recall_id": recall_id,
        "useful": useful,
        "miss": miss,
        "idempotency_key": idempotency_key,
    });
    let reply = client
        .post("/recall/feedback", &payload)
        .map_err(|error| error.to_string())?;
    let applied = reply
        .get("applied")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let useful_count = reply
        .get("useful")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let miss_count = reply
        .get("miss")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);

    if parsed.json {
        println!(
            "{{\"recall_id\": {:?}, \"applied\": {}, \"useful\": {}, \"miss\": {}}}",
            recall_id, applied, useful_count, miss_count
        );
    } else if applied {
        println!(
            "recall feedback applied: {} useful, {} miss (recall {recall_id})",
            useful_count, miss_count
        );
    } else {
        println!("recall feedback already applied: {recall_id}");
    }
    Ok(ExitCode::SUCCESS)
}

/// The `tool-call` subcommand: the headless adapter for every advertised MCP
/// tool (series/session, recall, dream, RAG import/search, termbase
/// validation, graph) over the daemon's authenticated `/mcp` route. No
/// database access happens in this process.
fn run_tool_call(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.dry_run {
        return Err(format!(
            "tool-call does not accept --dry-run; {TOOL_CALL_USAGE}"
        ));
    }
    reject_output_flag(parsed, "tool-call")?;
    let tool = parsed
        .subcommand
        .clone()
        .ok_or_else(|| format!("tool-call requires a tool name; {TOOL_CALL_USAGE}"))?;
    let arguments: serde_json::Value = match &parsed.args_json {
        Some(text) => serde_json::from_str(text)
            .map_err(|error| format!("--args must be a JSON object: {error}"))?,
        None => serde_json::json!({}),
    };
    if !arguments.is_object() {
        return Err(format!("--args must be a JSON object; {TOOL_CALL_USAGE}"));
    }
    let config = load_config(data_root);
    let client = hiero::daemon_client::DaemonClient::connect_opt_in(&config, parsed.start_daemon)
        .map_err(|error| error.to_string())?;
    let reply = client
        .call_tool(&tool, &arguments)
        .map_err(|error| error.to_string())?;

    let Some(result) = reply.get("result") else {
        // A JSON-RPC protocol error (unknown tool, invalid params): surface
        // the daemon's message and fail.
        let message = reply
            .pointer("/error/message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("the daemon returned no result");
        return Err(message.to_string());
    };
    let is_error = result
        .get("isError")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if parsed.json {
        let text = serde_json::to_string_pretty(&reply).map_err(|error| error.to_string())?;
        println!("{text}");
    } else {
        // Human output prints the payload: the structured content when the
        // tool completed, the diagnostic text on a tool error.
        let payload = match result.get("structuredContent") {
            Some(structured) => {
                serde_json::to_string_pretty(structured).map_err(|error| error.to_string())?
            }
            None => result
                .pointer("/content/0/text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("no content")
                .to_string(),
        };
        println!("{payload}");
    }
    if is_error {
        // Domain rejection: the request executed, the work was refused.
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// The `export` subcommand: serialize the memory content tables to JSON at an
/// explicit destination. Read-only by design — never a live database copy.
/// A destination this installation owns is always refused; an existing file is
/// refused unless `--force` asks for it to be replaced.
fn run_export(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.dry_run || parsed.start_daemon || parsed.port.is_some() {
        return Err(format!(
            "export does not accept --dry-run, --start-daemon, or --port; {EXPORT_USAGE}"
        ));
    }
    reject_args_flag(parsed, "export")?;
    let output = parsed
        .output
        .as_deref()
        .ok_or_else(|| format!("export requires --output <path>; {EXPORT_USAGE}"))?;
    let destination = absolute_path(output);
    let config = load_config(data_root);
    let report = if parsed.force {
        hiero::export::run_overwriting(&config, &destination)
    } else {
        hiero::export::run(&config, &destination)
    }
    .map_err(|error| error.to_string())?;
    if parsed.json {
        let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
        println!("{text}");
    } else {
        let rows: usize = report.tables.iter().map(|(_, count)| count).sum();
        println!(
            "exported {} rows across {} tables to {}",
            rows,
            report.tables.len(),
            report.output.display()
        );
        for (table, count) in &report.tables {
            println!("  {table}: {count}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The `plugins generate` subcommand: write the installation-owned agent
/// plugin bundle under the config root. Generated host configuration uses the
/// stable stdio entry point (stdio discovery — no fixed port, no baked-in
/// bearer) and is a copy/paste source for the user's manual host setup; user
/// host configuration is never rewritten automatically.
fn run_plugins(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon {
        return Err(format!(
            "plugins does not accept --port or --start-daemon; {PLUGINS_USAGE}"
        ));
    }
    if parsed.subcommand.as_deref() != Some("generate") {
        return Err(format!(
            "plugins requires the 'generate' subcommand; {PLUGINS_USAGE}"
        ));
    }
    reject_args_flag(parsed, "plugins")?;
    reject_output_flag(parsed, "plugins")?;
    let config = load_config(data_root);
    if parsed.dry_run {
        let rendered = hiero::agent_plugins::render(&config)?;
        if parsed.json {
            let paths: Vec<String> = rendered
                .iter()
                .map(|(path, _)| path.display().to_string())
                .collect();
            println!("{}", serde_json::json!({ "files": paths, "dry_run": true }));
        } else {
            println!(
                "would write {} files under {}:",
                rendered.len(),
                config.agent_plugins_root().display()
            );
            for (path, _) in &rendered {
                println!("  {}", path.display());
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    let written = hiero::agent_plugins::generate(&config)?;
    if parsed.json {
        let paths: Vec<String> = written
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        println!(
            "{}",
            serde_json::json!({ "files": paths, "written": paths.len() })
        );
    } else {
        println!(
            "wrote {} files under {}:",
            written.len(),
            config.agent_plugins_root().display()
        );
        for path in &written {
            println!("  {}", path.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The per-user service definition these arguments describe. Shared by the
/// top-level lifecycle commands and the `service` subcommands so both address
/// exactly the same unit.
fn service_options(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<service::ServiceOptions, String> {
    let binary = match &parsed.binary {
        Some(path) => absolute_path(path),
        None => std::env::current_exe()
            .map_err(|error| format!("could not locate the running binary: {error}"))?,
    };
    #[cfg(any(windows, target_os = "macos"))]
    let binary = if parsed.binary.is_none() {
        hiero::desktop::launch::stable_cli(&binary)?
    } else {
        binary
    };
    Ok(service::ServiceOptions {
        data_root: absolute_path(load_config(data_root).data_root()),
        unit_dir: parsed
            .unit_dir
            .as_deref()
            .map(absolute_path)
            .unwrap_or_else(service::default_unit_dir),
        binary,
        use_manager: !parsed.no_activate,
    })
}

/// `hiero start | stop | restart | status` (ADR 0009 §Decision).
///
/// `start` installs/starts the per-user service; `stop` requests an
/// authenticated graceful shutdown through the discovered endpoint and only
/// falls back to the service manager when no daemon answers that probe;
/// `restart` is the two in order; `status` reports the authenticated status of
/// the discovered daemon. None of these ever print the bearer token.
fn run_lifecycle(
    command: &str,
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon || parsed.dry_run {
        return Err(format!(
            "{command} does not accept --port, --start-daemon, or --dry-run; {LIFECYCLE_USAGE}"
        ));
    }
    reject_subcommand(parsed, command)?;
    reject_feedback_flags(parsed, command)?;
    let config = load_config(data_root);

    if command == "status" {
        let report = lifecycle::status(&config);
        if parsed.json {
            let text = serde_json::to_string_pretty(&report.to_json())
                .map_err(|error| error.to_string())?;
            println!("{text}");
        } else {
            print!("{}", report.render_human());
        }
        // A stopped daemon is a reportable fact, not a CLI failure.
        return Ok(ExitCode::SUCCESS);
    }

    if parsed.json {
        return Err(format!(
            "{command} does not accept --json; {LIFECYCLE_USAGE}"
        ));
    }
    let options = service_options(parsed, data_root)?;
    let lines = match command {
        "start" => lifecycle::start(&options),
        "stop" => lifecycle::stop(&config, &options),
        "restart" => lifecycle::restart(&config, &options),
        other => return Err(format!("unknown lifecycle command: {other}")),
    }
    .map_err(|error| error.to_string())?;
    for line in lines {
        println!("{line}");
    }
    Ok(ExitCode::SUCCESS)
}

/// The `service` subcommand: install (idempotent unit render + optional
/// manager enable), uninstall (graceful stop and unit removal), status, start, stop. The
/// systemd user manager is only contacted for the default unit location;
/// `--unit-dir` overrides are render/remove only, which keeps tests and
/// custom setups away from the real manager.
fn run_service(
    parsed: &ParsedArguments,
    data_root: Option<&std::path::Path>,
) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon || parsed.dry_run {
        return Err(format!(
            "service does not accept --port, --start-daemon, or --dry-run; {SERVICE_USAGE}"
        ));
    }
    let subcommand = parsed
        .subcommand
        .as_deref()
        .ok_or_else(|| format!("service requires a subcommand (install, uninstall, status, start, or stop); {SERVICE_USAGE}"))?;
    let options = service_options(parsed, data_root)?;
    match subcommand {
        "install" => {
            for line in service::install(&options).map_err(|error| error.to_string())? {
                println!("{line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        "uninstall" => {
            for line in service::uninstall(&options).map_err(|error| error.to_string())? {
                println!("{line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        "status" => {
            let status = service::status(&options).map_err(|error| error.to_string())?;
            if parsed.json {
                let text = serde_json::to_string_pretty(&status.to_json())
                    .map_err(|error| error.to_string())?;
                println!("{text}");
            } else {
                println!("{}", status.render_human());
            }
            Ok(ExitCode::from(if status.consistent() { 0 } else { 1 }))
        }
        "start" | "stop" => {
            let lines = if subcommand == "start" {
                service::start(&options)
            } else {
                service::stop(&options)
            }
            .map_err(|error| error.to_string())?;
            for line in lines {
                println!("{line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!(
            "unknown service subcommand: {other}; {SERVICE_USAGE}"
        )),
    }
}

/// The `update` subcommand: the one-way cutover flow over a release feed
/// directory. Refusals exit 2 (nothing changed), applied-but-rolled-back
/// failures exit 1, success (including migration-pending) exits 0.
fn run_update_command(parsed: &ParsedArguments) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon {
        return Err(format!(
            "update does not accept --port or --start-daemon; {UPDATE_USAGE}"
        ));
    }
    let local = parsed
        .release_dir
        .clone()
        .or_else(|| std::env::var("HIERONYMUS_RELEASE_DIR").ok());
    let remote = parsed
        .release_url
        .clone()
        .or_else(|| std::env::var("HIERONYMUS_RELEASE_URL").ok());
    if local.is_some() && remote.is_some() {
        return Err("--release-dir and --release-url are mutually exclusive".into());
    }
    let config = load_config(parsed.data_root.as_deref().map(std::path::Path::new));
    let operation = lifecycle::operation::LifecycleOperation::acquire(&config)
        .map_err(|error| error.to_string())?;
    let staging = tempfile::tempdir().map_err(|e| e.to_string())?;
    let channel = parsed
        .channel
        .clone()
        .or_else(|| std::env::var("HIERONYMUS_RELEASE_CHANNEL").ok())
        .unwrap_or_else(|| "stable".into());
    hiero::release_source::validate_channel(&channel)?;
    let release_dir = match (local, remote) {
        (Some(directory), None) => absolute_path(&directory),
        (None, Some(base)) => hiero::release_source::stage_remote_cached_with_roots(
            &base,
            &channel,
            &staging.path().join("verified"),
            &parsed
                .app_dir
                .as_deref()
                .map(absolute_path)
                .unwrap_or_else(hiero::app::default_app_dir)
                .join("cache/models"),
            hieronymus::tls::TlsRoots::default(),
        )?,
        _ => {
            return Err(format!(
                "configure HIERONYMUS_RELEASE_URL or pass --release-url / --release-dir; {UPDATE_USAGE}"
            ));
        }
    };
    let options = update::UpdateOptions {
        release_dir,
        app_dir: parsed.app_dir.as_deref().map(absolute_path),
        data_root: parsed.data_root.as_deref().map(absolute_path),
        unit_dir: parsed.unit_dir.as_deref().map(absolute_path),
    };
    match update::run_update_guarded(&options, &operation) {
        Ok(report) => {
            if parsed.json {
                let text = serde_json::to_string_pretty(&report.to_json())
                    .map_err(|error| error.to_string())?;
                println!("{text}");
            } else {
                print!("{}", report.render_human());
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            for step in error.steps() {
                eprintln!("  {step}");
            }
            eprintln!("hiero update: {error}");
            Ok(ExitCode::from(error.exit_code()))
        }
    }
}

/// The `uninstall` subcommand: confirmation-gated removal of the service
/// unit, application directory, owned PATH links, and generated agent-plugin
/// entries. Data deletion happens only through the explicit `--delete-data`.
fn run_uninstall_command(parsed: &ParsedArguments) -> Result<ExitCode, String> {
    if parsed.port.is_some() || parsed.start_daemon {
        return Err(format!(
            "uninstall does not accept --port or --start-daemon; {UNINSTALL_USAGE}"
        ));
    }
    let mut confirmed = parsed.yes;
    if !confirmed {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            print!(
                "Uninstall the Hieronymus binaries, command links, and service unit? \
                 Databases, configuration, models, and backups are preserved unless \
                 --delete-data is passed. [y/N] "
            );
            use std::io::Write as _;
            let _ = std::io::stdout().flush();
            let mut answer = String::new();
            let _ = std::io::stdin().read_line(&mut answer);
            confirmed = matches!(answer.trim(), "y" | "Y" | "yes" | "YES");
        }
    }
    let options = uninstall::UninstallOptions {
        app_dir: parsed.app_dir.as_deref().map(absolute_path),
        data_root: parsed.data_root.as_deref().map(absolute_path),
        unit_dir: parsed.unit_dir.as_deref().map(absolute_path),
        confirmed,
        delete_data: parsed.delete_data,
    };
    match uninstall::run_uninstall(&options) {
        Ok(report) => {
            if parsed.json {
                let text = serde_json::to_string_pretty(&report.to_json())
                    .map_err(|error| error.to_string())?;
                println!("{text}");
            } else {
                print!("{}", report.render_human());
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod agent_hook_usage_tests {
    use super::*;

    #[test]
    fn agent_hook_usage_excludes_passive_pi() {
        assert!(AGENT_HOOK_USAGE.contains("claude|codex|zcode"));
        assert!(!AGENT_HOOK_USAGE.contains("|pi|"));
    }
}
