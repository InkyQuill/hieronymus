//! `hiero` — the Hieronymus command-line binary. Thin presentation layer:
//! argument parsing, human/JSON output, and exit codes. Domain behavior lives
//! in the `hieronymus` library.

use std::process::ExitCode;

use hieronymus::data_root::load_config;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hiero: {error}");
            eprintln!("usage: hiero <version|classify> [--json] [--data-root <path>]");
            ExitCode::from(2)
        }
    }
}

fn run(arguments: &[String]) -> Result<(), String> {
    let mut json = false;
    let mut data_root: Option<String> = None;
    let mut positional: Option<String> = None;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        match argument.as_str() {
            "--json" => json = true,
            "--data-root" => {
                index += 1;
                data_root = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--data-root requires a path argument".to_string())?
                        .clone(),
                );
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}"));
            }
            value => {
                if positional.is_some() {
                    return Err(format!("unexpected extra argument: {value}"));
                }
                positional = Some(value.to_string());
            }
        }
        index += 1;
    }

    match positional.as_deref() {
        Some("version") => {
            if json {
                println!("{{\"version\": \"{VERSION}\"}}");
            } else {
                println!("hiero v{VERSION}\u{03b1}");
            }
            Ok(())
        }
        Some("classify") => {
            let config = load_config(data_root.as_deref().map(std::path::Path::new));
            let state = hieronymus::db::classify_database(&config.database_path());
            if json {
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
        Some(other) => Err(format!("unknown command: {other}")),
        None => Err("missing command".to_string()),
    }
}
