use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use hiero_bin::{
    cli::{Cli, Commands},
    daemon, mcp,
    output::{render_doctor_human, render_doctor_json},
};
use hiero_core::{config::HieronymusConfig, doctor::run_doctor};
use tracing::error;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    if let Err(error) = run(cli).await.context("hiero command failed") {
        error!(error = ?error, "hiero exited with an error");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

async fn run(cli: Cli) -> Result<()> {
    let config = HieronymusConfig::load(cli.data_root.clone())
        .context("failed to resolve Hieronymus configuration")?;

    match cli.command {
        Some(Commands::Doctor) => {
            let report = run_doctor(&config).await;
            if cli.json {
                println!("{}", render_doctor_json(&report)?);
            } else {
                print!("{}", render_doctor_human(&report));
            }
            Ok(())
        }
        Some(Commands::Start { host, port }) => start_daemon(config, &host, port, cli.json).await,
        Some(Commands::Status { json }) => {
            let port = config.resolve_port(None);
            let status = daemon::daemon_status(&config, port)
                .await?
                .context("Hieronymus daemon is not running")?;
            print_status(&status, cli.json || json)?;
            Ok(())
        }
        Some(Commands::Stop) => {
            let result = daemon::request_shutdown(&config, config.resolve_port(None)).await?;
            println!("{}", serde_json::to_string(&result)?);
            Ok(())
        }
        Some(Commands::Config { json }) => {
            let port = config.resolve_port(None);
            daemon::daemon_status(&config, port)
                .await?
                .context("Hieronymus daemon is not running")?;
            if cli.json || json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "url": format!("http://127.0.0.1:{port}/config")
                    }))?
                );
            } else {
                println!("http://127.0.0.1:{port}/config");
            }
            Ok(())
        }
        Some(Commands::Mcp) => mcp::stdio::run(&config).await,
        None => start_daemon(config, "127.0.0.1", 0, cli.json).await,
        command => {
            let name = command.as_ref().map_or("start", command_name);
            anyhow::bail!("{name} command is unavailable until its Rust service is implemented")
        }
    }
}

async fn start_daemon(
    config: HieronymusConfig,
    host: &str,
    explicit_port: u16,
    json: bool,
) -> Result<()> {
    anyhow::ensure!(
        host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost"),
        "the daemon may only bind to 127.0.0.1"
    );
    let port = if explicit_port == 0 {
        config.resolve_port(None)
    } else {
        explicit_port
    };
    if let Some(status) = daemon::daemon_status(&config, port).await? {
        print_status(&status, json)?;
        return Ok(());
    }
    daemon::serve(config, port, daemon::shutdown_signal()).await
}

fn print_status(status: &serde_json::Value, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(status)?);
    } else {
        println!("Hieronymus daemon is running");
    }
    Ok(())
}

const fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Start { .. } => "start",
        Commands::Status { .. } => "status",
        Commands::Stop => "stop",
        Commands::Config { .. } => "config",
        Commands::Doctor => "doctor",
        Commands::Series { .. } => "series",
        Commands::Session { .. } => "session",
        Commands::Concept { .. } => "concept",
        Commands::Rag { .. } => "rag",
        Commands::Skills { .. } => "skills",
        Commands::AgentHook { .. } => "agent-hook",
        Commands::ProposeTerm { .. } => "propose-term",
        Commands::Approve { .. } => "approve",
        Commands::Validate { .. } => "validate",
        Commands::Search { .. } => "search",
        Commands::Remember { .. } => "remember",
        Commands::RememberShort { .. } => "remember-short",
        Commands::Forget { .. } => "forget",
        Commands::Recall { .. } => "recall",
        Commands::Feedback { .. } => "feedback",
        Commands::Dream { .. } => "dream",
        Commands::CrystalValidate { .. } => "crystal-validate",
        Commands::Install { .. } => "install",
        Commands::Update { .. } => "update",
        Commands::Mcp => "mcp",
    }
}
