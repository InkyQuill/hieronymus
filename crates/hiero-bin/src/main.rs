use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use hiero_bin::{
    cli::{Cli, Commands},
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
        command => {
            let name = command.as_ref().map_or("start", command_name);
            anyhow::bail!("{name} command is unavailable until its Rust service is implemented")
        }
    }
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
