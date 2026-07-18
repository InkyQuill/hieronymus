use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use hiero_bin::cli::Cli;
use tracing::error;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    if let Err(error) = run(cli).await.context("hiero command failed") {
        error!(error = ?error, "hiero exited with an error");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

async fn run(cli: Cli) -> Result<()> {
    anyhow::bail!(
        "{:?} command execution is not available in the initial CLI schema",
        cli.execution()
    )
}
