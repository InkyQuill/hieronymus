//! CLI for the embedded frontend asset qualification harness.
//!
//! Subcommands:
//! - `manifest`: print metadata for every embedded bundle file.
//! - `get --path <request-path>`: resolve one request path against the
//!   embedded bundle.
//!
//! Output discipline: the CLI prints only status, MIME type, body digest,
//! byte length, and the fallback flag. It never prints response bodies and
//! makes no claims about Host validation, auth, CSRF, or HTTP routing.

use std::process::ExitCode;

mod assets;

const USAGE: &str =
    "usage: frontend-embedding manifest | frontend-embedding get --path <request-path>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("frontend-embedding: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> anyhow::Result<()> {
    let Some(command) = args.first() else {
        anyhow::bail!("{USAGE}");
    };
    match command.as_str() {
        "manifest" => {
            let rest = &args[1..];
            if !rest.is_empty() {
                anyhow::bail!("unexpected argument {:?} for manifest; {USAGE}", rest[0]);
            }
            println!("{}", serde_json::to_string_pretty(&assets::manifest())?);
            Ok(())
        }
        "get" => {
            let request_path = parse_get_path(&args[1..])?;
            let response = assets::resolve_asset(&request_path);
            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
        other => anyhow::bail!("unknown subcommand {other:?}; {USAGE}"),
    }
}

fn parse_get_path(args: &[String]) -> anyhow::Result<String> {
    let mut request_path: Option<&String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--path" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("--path requires a request path; {USAGE}"))?;
                if request_path.replace(value).is_some() {
                    anyhow::bail!("--path given more than once; {USAGE}");
                }
                index += 2;
            }
            other => anyhow::bail!("unexpected argument {other:?} for get; {USAGE}"),
        }
    }
    request_path
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("get requires --path <request-path>; {USAGE}"))
}
