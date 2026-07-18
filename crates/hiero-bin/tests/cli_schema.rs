use std::path::PathBuf;

use clap::Parser;
use hiero_bin::cli::{
    AgentHookCommand, Cli, CommandExecution, Commands, ConceptCommand, ConceptFacetCommand,
    RagCommand, SeriesCommand, SessionCommand, SkillsCommand,
};

fn command(arguments: &[&str]) -> Commands {
    Cli::try_parse_from(arguments)
        .expect("command should parse")
        .command
        .expect("test arguments should contain a command")
}

#[test]
fn root_options_and_operational_commands_parse_with_all_arguments() {
    let cli = Cli::try_parse_from([
        "hiero",
        "--data-root",
        "/tmp/hiero-data",
        "--json",
        "start",
        "--host",
        "0.0.0.0",
        "--port",
        "1234",
    ])
    .expect("root options and start should parse");
    assert_eq!(cli.data_root, Some(PathBuf::from("/tmp/hiero-data")));
    assert!(cli.json);
    assert_eq!(
        cli.command,
        Some(Commands::Start {
            host: "0.0.0.0".to_owned(),
            port: 1234,
        })
    );

    let cases = [
        (
            vec!["hiero", "status", "--json"],
            Commands::Status { json: true },
        ),
        (vec!["hiero", "stop"], Commands::Stop),
        (
            vec!["hiero", "config", "--json"],
            Commands::Config { json: true },
        ),
        (vec!["hiero", "doctor"], Commands::Doctor),
        (
            vec![
                "hiero",
                "propose-term",
                "book",
                "person",
                "Alice",
                "Алиса",
                "--tags",
                "lead",
                "--tags",
                "mage",
                "--notes",
                "approved spelling",
            ],
            Commands::ProposeTerm {
                series_slug: "book".to_owned(),
                category: "person".to_owned(),
                source_text: "Alice".to_owned(),
                canonical: "Алиса".to_owned(),
                tags: vec!["lead".to_owned(), "mage".to_owned()],
                notes: Some("approved spelling".to_owned()),
            },
        ),
        (
            vec!["hiero", "approve", "book", "42"],
            Commands::Approve {
                series_slug: "book".to_owned(),
                term_id: 42,
            },
        ),
        (
            vec![
                "hiero",
                "validate",
                "book",
                "translated",
                "--raw-text",
                "raw",
                "--source-text",
                "source",
            ],
            Commands::Validate {
                series_slug: "book".to_owned(),
                translated_text: "translated".to_owned(),
                raw_text: Some("raw".to_owned()),
                source_text: Some("source".to_owned()),
            },
        ),
        (
            vec!["hiero", "search", "book", "query", "--limit", "7"],
            Commands::Search {
                series_slug: "book".to_owned(),
                query: "query".to_owned(),
                limit: 7,
            },
        ),
        (
            vec!["hiero", "remember", "book", "scene", "text"],
            Commands::Remember {
                series_slug: "book".to_owned(),
                kind: "scene".to_owned(),
                text: "text".to_owned(),
            },
        ),
        (
            vec!["hiero", "remember-short", "11", "note", "text"],
            Commands::RememberShort {
                session_id: 11,
                kind: "note".to_owned(),
                text: "text".to_owned(),
            },
        ),
        (
            vec!["hiero", "forget", "12"],
            Commands::Forget { crystal_id: 12 },
        ),
        (
            vec!["hiero", "recall", "11", "book", "query", "--limit", "8"],
            Commands::Recall {
                session_id: 11,
                series_slug: "book".to_owned(),
                query: "query".to_owned(),
                limit: 8,
            },
        ),
        (
            vec!["hiero", "feedback", "11", "correction"],
            Commands::Feedback {
                session_id: 11,
                correction_text: "correction".to_owned(),
            },
        ),
        (
            vec!["hiero", "dream", "--provider", "ollama", "--wait"],
            Commands::Dream {
                provider: Some("ollama".to_owned()),
                wait: true,
            },
        ),
        (
            vec!["hiero", "crystal-validate", "13"],
            Commands::CrystalValidate { crystal_id: 13 },
        ),
        (
            vec!["hiero", "install", "--app", "codex", "--dry-run"],
            Commands::Install {
                app: Some("codex".to_owned()),
                dry_run: true,
            },
        ),
        (
            vec!["hiero", "update", "--check-only"],
            Commands::Update { check_only: true },
        ),
        (vec!["hiero", "mcp"], Commands::Mcp),
    ];

    for (arguments, expected) in cases {
        assert_eq!(command(&arguments), expected, "arguments: {arguments:?}");
    }
}

#[test]
fn defaults_and_no_subcommand_are_stable() {
    let cli = Cli::try_parse_from(["hiero"]).expect("no command starts the daemon");
    assert_eq!(cli.data_root, None);
    assert!(!cli.json);
    assert_eq!(cli.command, None);

    assert_eq!(
        command(&["hiero", "start"]),
        Commands::Start {
            host: "127.0.0.1".to_owned(),
            port: 9768,
        }
    );
    assert_eq!(
        command(&["hiero", "search", "book", "query"]),
        Commands::Search {
            series_slug: "book".to_owned(),
            query: "query".to_owned(),
            limit: 5,
        }
    );
    assert_eq!(
        command(&["hiero", "recall", "11", "book", "query"]),
        Commands::Recall {
            session_id: 11,
            series_slug: "book".to_owned(),
            query: "query".to_owned(),
            limit: 10,
        }
    );
    assert_eq!(
        command(&["hiero", "rag", "search", "book", "query"]),
        Commands::Rag {
            command: RagCommand::Search {
                series_slug: "book".to_owned(),
                query: "query".to_owned(),
                limit: 10,
            },
        }
    );

    let cases = [
        (vec!["hiero", "status"], Commands::Status { json: false }),
        (vec!["hiero", "config"], Commands::Config { json: false }),
        (
            vec!["hiero", "series", "list"],
            Commands::Series {
                command: SeriesCommand::List { json: false },
            },
        ),
        (
            vec!["hiero", "propose-term", "book", "person", "Alice", "Алиса"],
            Commands::ProposeTerm {
                series_slug: "book".to_owned(),
                category: "person".to_owned(),
                source_text: "Alice".to_owned(),
                canonical: "Алиса".to_owned(),
                tags: Vec::new(),
                notes: None,
            },
        ),
        (
            vec!["hiero", "validate", "book", "translated"],
            Commands::Validate {
                series_slug: "book".to_owned(),
                translated_text: "translated".to_owned(),
                raw_text: None,
                source_text: None,
            },
        ),
        (
            vec!["hiero", "dream"],
            Commands::Dream {
                provider: None,
                wait: false,
            },
        ),
        (
            vec!["hiero", "install"],
            Commands::Install {
                app: None,
                dry_run: false,
            },
        ),
        (
            vec!["hiero", "update"],
            Commands::Update { check_only: false },
        ),
        (
            vec!["hiero", "concept", "update", "21"],
            Commands::Concept {
                command: ConceptCommand::Update {
                    concept_id: 21,
                    name: None,
                },
            },
        ),
        (
            vec!["hiero", "concept", "semantic-tags-set", "21"],
            Commands::Concept {
                command: ConceptCommand::SemanticTagsSet {
                    concept_id: 21,
                    tags: Vec::new(),
                },
            },
        ),
        (
            vec!["hiero", "rag", "import", "book", "/tmp/corpus"],
            Commands::Rag {
                command: RagCommand::Import {
                    series_slug: "book".to_owned(),
                    path: PathBuf::from("/tmp/corpus"),
                    source_type: None,
                },
            },
        ),
        (
            vec!["hiero", "skills", "install"],
            Commands::Skills {
                command: SkillsCommand::Install {
                    targets: Vec::new(),
                    dry_run: false,
                },
            },
        ),
        (
            vec!["hiero", "skills", "uninstall"],
            Commands::Skills {
                command: SkillsCommand::Uninstall {
                    targets: Vec::new(),
                    dry_run: false,
                },
            },
        ),
    ];

    for (arguments, expected) in cases {
        assert_eq!(command(&arguments), expected, "arguments: {arguments:?}");
    }
}

#[test]
fn grouped_commands_parse_with_all_arguments() {
    let cases = [
        (
            vec!["hiero", "series", "create", "book", "Book", "en", "ru"],
            Commands::Series {
                command: SeriesCommand::Create {
                    slug: "book".to_owned(),
                    title: "Book".to_owned(),
                    source_language: "en".to_owned(),
                    target_language: "ru".to_owned(),
                },
            },
        ),
        (
            vec!["hiero", "series", "list", "--json"],
            Commands::Series {
                command: SeriesCommand::List { json: true },
            },
        ),
        (
            vec!["hiero", "series", "init", "book"],
            Commands::Series {
                command: SeriesCommand::Init {
                    slug: "book".to_owned(),
                },
            },
        ),
        (
            vec!["hiero", "session", "start", "book"],
            Commands::Session {
                command: SessionCommand::Start {
                    series_slug: "book".to_owned(),
                },
            },
        ),
        (
            vec!["hiero", "session", "complete", "11"],
            Commands::Session {
                command: SessionCommand::Complete { session_id: 11 },
            },
        ),
        (
            vec![
                "hiero",
                "rag",
                "import",
                "book",
                "/tmp/corpus",
                "--source-type",
                "glossary",
            ],
            Commands::Rag {
                command: RagCommand::Import {
                    series_slug: "book".to_owned(),
                    path: PathBuf::from("/tmp/corpus"),
                    source_type: Some("glossary".to_owned()),
                },
            },
        ),
        (
            vec!["hiero", "rag", "search", "book", "query", "--limit", "12"],
            Commands::Rag {
                command: RagCommand::Search {
                    series_slug: "book".to_owned(),
                    query: "query".to_owned(),
                    limit: 12,
                },
            },
        ),
        (
            vec!["hiero", "rag", "index-status"],
            Commands::Rag {
                command: RagCommand::IndexStatus,
            },
        ),
        (
            vec!["hiero", "rag", "index-rebuild"],
            Commands::Rag {
                command: RagCommand::IndexRebuild,
            },
        ),
        (
            vec!["hiero", "rag", "index-cancel"],
            Commands::Rag {
                command: RagCommand::IndexCancel,
            },
        ),
        (
            vec!["hiero", "skills", "install", "codex", "claude", "--dry-run"],
            Commands::Skills {
                command: SkillsCommand::Install {
                    targets: vec!["codex".to_owned(), "claude".to_owned()],
                    dry_run: true,
                },
            },
        ),
        (
            vec!["hiero", "skills", "uninstall", "codex", "--dry-run"],
            Commands::Skills {
                command: SkillsCommand::Uninstall {
                    targets: vec!["codex".to_owned()],
                    dry_run: true,
                },
            },
        ),
        (
            vec!["hiero", "agent-hook", "session-start"],
            Commands::AgentHook {
                command: AgentHookCommand::SessionStart,
            },
        ),
        (
            vec!["hiero", "agent-hook", "session-end"],
            Commands::AgentHook {
                command: AgentHookCommand::SessionEnd,
            },
        ),
    ];

    for (arguments, expected) in cases {
        assert_eq!(command(&arguments), expected, "arguments: {arguments:?}");
    }
}

#[test]
fn concept_commands_parse_with_all_arguments() {
    let cases = [
        (
            vec!["hiero", "concept", "list", "book"],
            ConceptCommand::List {
                series_slug: "book".to_owned(),
            },
        ),
        (
            vec!["hiero", "concept", "create", "book", "Alice"],
            ConceptCommand::Create {
                series_slug: "book".to_owned(),
                name: "Alice".to_owned(),
            },
        ),
        (
            vec!["hiero", "concept", "update", "21", "--name", "Alicia"],
            ConceptCommand::Update {
                concept_id: 21,
                name: Some("Alicia".to_owned()),
            },
        ),
        (
            vec!["hiero", "concept", "archive", "21"],
            ConceptCommand::Archive { concept_id: 21 },
        ),
        (
            vec!["hiero", "concept", "merge", "21", "22", "duplicate"],
            ConceptCommand::Merge {
                source_id: 21,
                target_id: 22,
                reason: "duplicate".to_owned(),
            },
        ),
        (
            vec!["hiero", "concept", "rename", "21", "Alicia"],
            ConceptCommand::Rename {
                concept_id: 21,
                new_name: "Alicia".to_owned(),
            },
        ),
        (
            vec![
                "hiero",
                "concept",
                "semantic-tags-set",
                "21",
                "lead",
                "mage",
            ],
            ConceptCommand::SemanticTagsSet {
                concept_id: 21,
                tags: vec!["lead".to_owned(), "mage".to_owned()],
            },
        ),
        (
            vec!["hiero", "concept", "proposals-list", "book"],
            ConceptCommand::ProposalsList {
                series_slug: "book".to_owned(),
            },
        ),
    ];

    for (arguments, expected) in cases {
        assert_eq!(
            command(&arguments),
            Commands::Concept { command: expected },
            "arguments: {arguments:?}"
        );
    }

    let facet_cases = [
        (
            vec!["hiero", "concept", "facet", "add", "21", "name", "Alice"],
            ConceptFacetCommand::Add {
                concept_id: 21,
                kind: "name".to_owned(),
                text: "Alice".to_owned(),
            },
        ),
        (
            vec!["hiero", "concept", "facet", "update", "31", "Alicia"],
            ConceptFacetCommand::Update {
                facet_id: 31,
                text: "Alicia".to_owned(),
            },
        ),
        (
            vec!["hiero", "concept", "facet", "list", "21"],
            ConceptFacetCommand::List { concept_id: 21 },
        ),
        (
            vec!["hiero", "concept", "facet", "set-canonical", "31"],
            ConceptFacetCommand::SetCanonical { facet_id: 31 },
        ),
    ];

    for (arguments, expected) in facet_cases {
        assert_eq!(
            command(&arguments),
            Commands::Concept {
                command: ConceptCommand::Facet { command: expected },
            },
            "arguments: {arguments:?}"
        );
    }
}

#[test]
fn missing_unknown_and_invalid_arguments_are_rejected() {
    let invalid = [
        vec!["hiero", "unknown"],
        vec!["hiero", "start", "--port", "not-a-port"],
        vec!["hiero", "start", "--port", "70000"],
        vec!["hiero", "approve", "book", "not-an-id"],
        vec!["hiero", "search", "book"],
        vec!["hiero", "series"],
        vec!["hiero", "series", "create", "book", "Book", "en"],
        vec!["hiero", "session", "complete"],
        vec!["hiero", "concept"],
        vec!["hiero", "concept", "facet"],
        vec!["hiero", "concept", "facet", "add", "21", "name"],
        vec!["hiero", "rag", "import", "book"],
        vec!["hiero", "skills"],
        vec!["hiero", "agent-hook"],
        vec!["hiero", "agent-hook", "start"],
    ];

    for arguments in invalid {
        assert!(
            Cli::try_parse_from(&arguments).is_err(),
            "arguments should be rejected: {arguments:?}"
        );
    }
}

#[test]
fn execution_classifies_every_boundary() {
    assert_eq!(
        Cli::try_parse_from(["hiero"])
            .expect("no command should parse")
            .execution(),
        CommandExecution::StartDaemon
    );
    assert_eq!(
        Cli::try_parse_from(["hiero", "start"])
            .expect("start should parse")
            .execution(),
        CommandExecution::StartDaemon
    );

    for arguments in [
        ["hiero", "status"].as_slice(),
        ["hiero", "stop"].as_slice(),
        ["hiero", "config"].as_slice(),
    ] {
        assert_eq!(
            Cli::try_parse_from(arguments)
                .expect("daemon HTTP command should parse")
                .execution(),
            CommandExecution::DaemonHttp,
            "arguments: {arguments:?}"
        );
    }

    assert_eq!(
        Cli::try_parse_from(["hiero", "mcp"])
            .expect("mcp should parse")
            .execution(),
        CommandExecution::StdioMcp
    );

    let direct_store_commands = [
        vec!["hiero", "doctor"],
        vec!["hiero", "series", "list"],
        vec!["hiero", "session", "start", "book"],
        vec!["hiero", "concept", "list", "book"],
        vec!["hiero", "rag", "index-status"],
        vec!["hiero", "skills", "install"],
        vec!["hiero", "agent-hook", "session-start"],
        vec!["hiero", "propose-term", "book", "person", "Alice", "Алиса"],
        vec!["hiero", "approve", "book", "1"],
        vec!["hiero", "validate", "book", "text"],
        vec!["hiero", "search", "book", "query"],
        vec!["hiero", "remember", "book", "kind", "text"],
        vec!["hiero", "remember-short", "1", "kind", "text"],
        vec!["hiero", "forget", "1"],
        vec!["hiero", "recall", "1", "book", "query"],
        vec!["hiero", "feedback", "1", "correction"],
        vec!["hiero", "dream"],
        vec!["hiero", "crystal-validate", "1"],
        vec!["hiero", "install"],
        vec!["hiero", "update"],
    ];

    for arguments in direct_store_commands {
        assert_eq!(
            Cli::try_parse_from(&arguments)
                .expect("direct-store command should parse")
                .execution(),
            CommandExecution::DirectStore,
            "arguments: {arguments:?}"
        );
    }
}

#[test]
fn help_surfaces_match_snapshots() {
    trycmd::TestCases::new().case("tests/cmd/*.trycmd");
}
