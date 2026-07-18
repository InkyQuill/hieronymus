use std::path::PathBuf;

use clap::Subcommand;

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Commands {
    /// Start the daemon (no-op if already running — the binary IS the daemon)
    #[command(name = "start")]
    Start {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 9768)]
        port: u16,
    },
    /// Show daemon status
    #[command(name = "status")]
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Stop the daemon (HTTP POST /shutdown)
    #[command(name = "stop")]
    Stop,
    /// Open the web console
    #[command(name = "config")]
    Config {
        #[arg(long)]
        json: bool,
    },
    /// Run system diagnostics
    #[command(name = "doctor")]
    Doctor,
    #[command(name = "series")]
    Series {
        #[command(subcommand)]
        command: SeriesCommand,
    },
    #[command(name = "session")]
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    #[command(name = "concept")]
    Concept {
        #[command(subcommand)]
        command: ConceptCommand,
    },
    #[command(name = "rag")]
    Rag {
        #[command(subcommand)]
        command: RagCommand,
    },
    #[command(name = "skills")]
    Skills {
        #[command(subcommand)]
        command: SkillsCommand,
    },
    #[command(name = "agent-hook")]
    AgentHook {
        #[command(subcommand)]
        command: AgentHookCommand,
    },
    #[command(name = "propose-term")]
    ProposeTerm {
        series_slug: String,
        category: String,
        source_text: String,
        canonical: String,
        #[arg(long)]
        tags: Vec<String>,
        #[arg(long)]
        notes: Option<String>,
    },
    #[command(name = "approve")]
    Approve { series_slug: String, term_id: i64 },
    #[command(name = "validate")]
    Validate {
        series_slug: String,
        translated_text: String,
        #[arg(long)]
        raw_text: Option<String>,
        #[arg(long)]
        source_text: Option<String>,
    },
    #[command(name = "search")]
    Search {
        series_slug: String,
        query: String,
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    #[command(name = "remember")]
    Remember {
        series_slug: String,
        kind: String,
        text: String,
    },
    #[command(name = "remember-short")]
    RememberShort {
        session_id: i64,
        kind: String,
        text: String,
    },
    #[command(name = "forget")]
    Forget { crystal_id: i64 },
    #[command(name = "recall")]
    Recall {
        session_id: i64,
        series_slug: String,
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    #[command(name = "feedback")]
    Feedback {
        session_id: i64,
        correction_text: String,
    },
    #[command(name = "dream")]
    Dream {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        wait: bool,
    },
    #[command(name = "crystal-validate")]
    CrystalValidate { crystal_id: i64 },
    #[command(name = "install")]
    Install {
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    #[command(name = "update")]
    Update {
        #[arg(long)]
        check_only: bool,
    },
    /// Stdio MCP compatibility shim
    #[command(name = "mcp")]
    Mcp,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum SeriesCommand {
    #[command(name = "create")]
    Create {
        slug: String,
        title: String,
        source_language: String,
        target_language: String,
    },
    #[command(name = "list")]
    List {
        #[arg(long)]
        json: bool,
    },
    #[command(name = "init")]
    Init { slug: String },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum SessionCommand {
    #[command(name = "start")]
    Start { series_slug: String },
    #[command(name = "complete")]
    Complete { session_id: i64 },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ConceptCommand {
    #[command(name = "list")]
    List { series_slug: String },
    #[command(name = "create")]
    Create { series_slug: String, name: String },
    #[command(name = "update")]
    Update {
        concept_id: i64,
        #[arg(long)]
        name: Option<String>,
    },
    #[command(name = "archive")]
    Archive { concept_id: i64 },
    #[command(name = "merge")]
    Merge {
        source_id: i64,
        target_id: i64,
        reason: String,
    },
    #[command(name = "rename")]
    Rename { concept_id: i64, new_name: String },
    #[command(name = "facet")]
    Facet {
        #[command(subcommand)]
        command: ConceptFacetCommand,
    },
    #[command(name = "semantic-tags-set")]
    SemanticTagsSet { concept_id: i64, tags: Vec<String> },
    #[command(name = "proposals-list")]
    ProposalsList { series_slug: String },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ConceptFacetCommand {
    #[command(name = "add")]
    Add {
        concept_id: i64,
        kind: String,
        text: String,
    },
    #[command(name = "update")]
    Update { facet_id: i64, text: String },
    #[command(name = "list")]
    List { concept_id: i64 },
    #[command(name = "set-canonical")]
    SetCanonical { facet_id: i64 },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum RagCommand {
    #[command(name = "import")]
    Import {
        series_slug: String,
        path: PathBuf,
        #[arg(long)]
        source_type: Option<String>,
    },
    #[command(name = "search")]
    Search {
        series_slug: String,
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    #[command(name = "index-status")]
    IndexStatus,
    #[command(name = "index-rebuild")]
    IndexRebuild,
    #[command(name = "index-cancel")]
    IndexCancel,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum SkillsCommand {
    #[command(name = "install")]
    Install {
        targets: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    #[command(name = "uninstall")]
    Uninstall {
        targets: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum AgentHookCommand {
    #[command(name = "session-start")]
    SessionStart,
    #[command(name = "session-end")]
    SessionEnd,
}
