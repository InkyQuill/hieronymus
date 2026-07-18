mod commands;

use std::path::PathBuf;

use clap::Parser;

pub use commands::{
    AgentHookCommand, Commands, ConceptCommand, ConceptFacetCommand, RagCommand, SeriesCommand,
    SessionCommand, SkillsCommand,
};

#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "hiero")]
#[command(version)]
#[command(about = "Hieronymus local-first translation memory manager")]
pub struct Cli {
    #[arg(long)]
    pub data_root: Option<PathBuf>,

    #[arg(short, long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandExecution {
    DirectStore,
    DaemonHttp,
    StartDaemon,
    StdioMcp,
}

impl Cli {
    #[must_use]
    pub const fn execution(&self) -> CommandExecution {
        match self.command {
            None | Some(Commands::Start { .. }) => CommandExecution::StartDaemon,
            Some(Commands::Status { .. } | Commands::Stop | Commands::Config { .. }) => {
                CommandExecution::DaemonHttp
            }
            Some(Commands::Mcp) => CommandExecution::StdioMcp,
            Some(_) => CommandExecution::DirectStore,
        }
    }
}
