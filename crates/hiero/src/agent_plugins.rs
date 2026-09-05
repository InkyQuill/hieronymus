//! Generated agent-plugin assets (plan M5, port of the Python
//! `agent_assets`/`agent_plugins` renderers): the installation-owned bundle
//! under the data root's `agent-plugins/` directory — workflow skills, the
//! MCP registration, Codex hooks, and one manifest per supported host.
//!
//! Host configuration is never rewritten here: the generated files are
//! documentation and copy-paste sources for the user's manual host setup.
//! The MCP registration uses the stable `hieronymus-mcp` entry point (the
//! argv[0] link to `hiero mcp`): the stdio adapter discovers the local
//! daemon through the data root's discovery record, so the generated config
//! carries no fixed port and no baked-in bearer token.
//!
//! Skills keep the Python guidance: English-first memory writes, strict
//! concept contracts are mandatory while crystals stay advisory,
//! candidate-only ingestion (agents never approve terminology themselves,
//! dreaming crystallizes later), and stable CLI paths. Hieronymus-owned
//! integration files stay in the Hieronymus config root — no source code or
//! plugin files go into book workspaces.
//!
//! [`render`] is pure; [`generate`] writes it with atomic replaces, so
//! repeated runs are byte-identical (deterministic documented outputs).

use std::collections::BTreeMap;
use std::path::PathBuf;

use hieronymus::data_root::HieronymusConfig;

/// The supported host targets, each rendered as its own plugin directory
/// (mirrors the Python `render_agent_plugin_assets` targets).
const TARGETS: [&str; 5] = ["codex", "claude", "gemini", "opencode", "openclaw"];

/// The strict-boundary guidance every skill carries (Python `BOUNDARY_TEXT`).
const BOUNDARY_TEXT: &str = "Strict concept contracts are mandatory. Crystals and lessons are advisory.\
\nDo not approve terminology proposals yourself; record proposals for human approval instead.";

/// The `source_role` provenance note (Python `SOURCE_ROLE_TEXT`).
const SOURCE_ROLE_TEXT: &str = "`source_role` is a freeform provenance label. It does not determine a\
\ncrystal type, confidence, or priority: dreaming chooses those from the evidence, text,\
\n`source_credibility`, and `rule_intent`. Omit it for an ordinary agent note (it defaults to\
\n`agent`), or use a useful label such as `user`, `mentor`, `reviewer`, `source-text`, or `system`.";

fn bootstrap_skill() -> String {
    format!(
        r#"---
name: hieronymus-bootstrap
description: Use at the start of a project session with Hieronymus skills or MCP tools.
---

# Hieronymus Bootstrap

Use this skill first. It explains the installed workflow and prevents malformed memory writes.

## Skill map

| Need | Skill |
| --- | --- |
| Retrieve relevant memory | `hieronymus-recall` |
| Read sources into RAG and conclusions into memory | `hieronymus-read` |
| Study or import material | `hieronymus-learn` |
| Preserve a direct correction | `hieronymus-remember` |
| Translate with terminology boundaries | `hieronymus-translate` |
| Review with expert observations | `hieronymus-review` |
| Coordinate session, recall, feedback, and dreaming | `hieronymus-orchestrate` |

## MCP memory contract

Start a session before writing short-term memory. Use `hieronymus_short_term_add` for one block
or `hieronymus_short_term_add_batch` for up to 500 independently valid blocks. Every item needs
`kind` and `text`; `source_role` is optional.

{SOURCE_ROLE_TEXT}

For example, use `source_role="agent"` for an ordinary note and `source_role="user"` for a
direct correction. Any non-empty provenance label is accepted, including `source_role="system"`.

## Stable entry points

The MCP server runs through the installed `hieronymus-mcp` command (stdio). Headless inspection
uses the same binary: `hiero tool-call <tool> --args '{{...}}'`, `hiero export --output <path>`,
and `hiero recall-feedback` apply through the local daemon. Never copy Hieronymus source code or
plugin files into a book workspace; integration files live in the Hieronymus config root.

## Report observed problems

Whenever you notice a Hieronymus bug, missing capability, bad recall, rejected valid input,
ambiguous skill instruction, or confusing MCP response, append it to `./hiero_report.md` in the
current project. Record the date, workflow or tool, concise reproduction/context, observed result,
expected result, and relevant IDs or non-secret evidence. Keep working unless the problem blocks
the user. Never put API keys, tokens, or private source text in the report.

{BOUNDARY_TEXT}
"#
    )
}

fn recall_skill() -> String {
    format!(
        r#"---
name: hieronymus-recall
description: Recall Hieronymus memory before translation, review, terminology, or docs work.
---

# Hieronymus Recall

Use this skill before memory-sensitive work. Start or identify a task session, call
`hieronymus_recall`, and keep strict concept contracts separate from advisory crystals.

{BOUNDARY_TEXT}
Cite influential crystals when they shape a translation or review decision.
"#
    )
}

fn learn_skill() -> String {
    format!(
        r#"---
name: hieronymus-learn
description: Commit material into short-term memory for later dreaming and crystallization.
---

# Hieronymus Learn

Use when the user says to absorb, remember, study, ingest, import, or learn from a source.
The agent does the judgment: split material into small observed facts, attach source credibility,
language tags, story scopes, and semantic tags, then call `hieronymus_short_term_add`. source_role
is optional provenance metadata; it does not classify the resulting crystal.

MCP tools are storage and retrieval primitives, not judgment engines. There is no supported Learn
judgment MCP tool; use this skill workflow plus `hieronymus_short_term_add`. Do not promote strict
terminology directly — ingestion stays candidate-only; dreaming can produce crystals, lessons,
erudition, and proposals later, and a human approves terminology.

{BOUNDARY_TEXT}
"#
    )
}

fn read_skill() -> String {
    format!(
        r#"---
name: hieronymus-read
description: >
  Read source material into RAG and preserve concise agent conclusions in short-term memory.
---

# Hieronymus Read

Use for reading files, lookup, summaries, or temporary understanding. First import each source file
into project RAG with `hieronymus_rag_import`: RAG retains the source material itself for later
retrieval.

Do not copy file text or long extracts into short-term memory. Instead, after reading, record the
agent's own conclusions with `hieronymus_short_term_add_batch`: learned terminology, concepts,
important facts, implications, uncertainties, and connections to the current work. Each
short-term memory block must contain 1–6 sentences. Create as many separate blocks as necessary
to cover every important term, concept, and detail. The size limit applies to each block,
never to the total set.
Accumulate up to 500 validated blocks per request. Continue making batches until every important
detail is covered; a book commonly needs hundreds of conclusion blocks.

MCP tools are storage and retrieval primitives, not judgment engines. There is no supported Read
judgment MCP tool; use this skill workflow plus `hieronymus_short_term_add_batch`.

source_role is optional provenance metadata and does not classify the resulting crystal.

RAG stores the direct source; short-term memory stores the agent's indirect understanding of it.

{BOUNDARY_TEXT}
"#
    )
}

fn remember_skill() -> String {
    format!(
        r#"---
name: hieronymus-remember
description: Record user corrections as high-credibility short-term memory.
---

# Hieronymus Remember

Use when the user corrects terminology, style, facts, or workflow rules. The agent does the
judgment: turn the correction into a short short-term memory, preserve scope and tags, and call
`hieronymus_short_term_add`.

For high-credibility user rules, phrase the memory as `User told me to ...`, use source_role `user`,
kind `correction`, source_credibility `user_rule`, and a specific rule_intent when known.

MCP tools are storage and retrieval primitives, not judgment engines. Do not create or promote rule
crystals manually; dreaming handles crystallization later.

{BOUNDARY_TEXT}
"#
    )
}

fn translate_skill() -> String {
    format!(
        r#"---
name: hieronymus-translate
description: Translate with Hieronymus strict terminology and advisory crystals.
---

# Hieronymus Translate

{BOUNDARY_TEXT}
Apply approved concept contracts first. Use crystals and lessons only as context, and record
uncertainty or discoveries as short-term memories.
"#
    )
}

fn review_skill() -> String {
    format!(
        r#"---
name: hieronymus-review
description: Review translation output using strict terminology and mentor-grade observations.
---

# Hieronymus Review

{BOUNDARY_TEXT}
Check strict validation findings first. Identify whether crystals helped or misled. Record recurring
issues, contradictions, and correction patterns as short-term memories. Use an optional
source_role such as `reviewer` when the provenance will help later audit.
"#
    )
}

fn orchestrate_skill() -> String {
    format!(
        r#"---
name: hieronymus-orchestrate
description: Coordinate Hieronymus task sessions, recall, validation, feedback, and dreaming.
---

# Hieronymus Orchestrate

{BOUNDARY_TEXT}
Create a task session, recall before work, collect short-term memories, record feedback events, and
trigger or defer dreaming based on configuration or user instruction. source_role is optional
provenance metadata; it never decides how dreaming categorizes the memory.
"#
    )
}

/// The MCP registration: the stable stdio entry point with stdio discovery.
/// No fixed port, no bearer token, no environment secrets.
fn mcp_config_json() -> String {
    pretty_json(&serde_json::json!({
        "mcpServers": {
            "hieronymus": {
                "command": "hieronymus-mcp",
                "args": [],
                "env": {},
            }
        }
    }))
}

/// The Codex session hooks: the stable argv[0] link names that route to
/// `hiero agent-hook`.
fn codex_hooks_json() -> String {
    pretty_json(&serde_json::json!({
        "hooks": [
            {
                "event": "SessionStart",
                "command": "hieronymus-agent-hook",
                "args": ["session-start"],
            },
            {
                "event": "Stop",
                "command": "hieronymus-agent-hook",
                "args": ["session-end"],
            },
        ]
    }))
}

fn pretty_json(value: &serde_json::Value) -> String {
    let mut text =
        serde_json::to_string_pretty(value).expect("plugin JSON payloads always serialize");
    text.push('\n');
    text
}

fn common_assets() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "skills/hieronymus-bootstrap/SKILL.md".to_string(),
            bootstrap_skill(),
        ),
        (
            "skills/hieronymus-recall/SKILL.md".to_string(),
            recall_skill(),
        ),
        (
            "skills/hieronymus-learn/SKILL.md".to_string(),
            learn_skill(),
        ),
        ("skills/hieronymus-read/SKILL.md".to_string(), read_skill()),
        (
            "skills/hieronymus-remember/SKILL.md".to_string(),
            remember_skill(),
        ),
        (
            "skills/hieronymus-translate/SKILL.md".to_string(),
            translate_skill(),
        ),
        (
            "skills/hieronymus-review/SKILL.md".to_string(),
            review_skill(),
        ),
        (
            "skills/hieronymus-orchestrate/SKILL.md".to_string(),
            orchestrate_skill(),
        ),
        ("mcp/hieronymus.mcp.json".to_string(), mcp_config_json()),
        ("hooks/hooks.codex.json".to_string(), codex_hooks_json()),
    ])
}

fn plugin_manifest() -> serde_json::Value {
    serde_json::json!({
        "name": "hieronymus",
        "version": env!("CARGO_PKG_VERSION"),
        "description":
            "Translation memory, termbase, recall, and dreaming workflows for agents.",
        "skills": "./skills/",
        "mcpServers": "./mcp/hieronymus.mcp.json",
    })
}

/// The per-target manifest entries layered over the common bundle. Mirrors
/// the Python `render_agent_plugin_assets` target manifests with the stable
/// Rust entry points.
fn target_assets(target: &str) -> Result<BTreeMap<String, String>, String> {
    let mut assets = common_assets();
    let author = serde_json::json!({
        "email": "me@inkyquill.net",
        "name": "Pavel Obruchnikov",
    });
    match target {
        "codex" => {
            assets.insert(".mcp.json".to_string(), mcp_config_json());
            let mut manifest = plugin_manifest();
            manifest["author"] = author;
            manifest["interface"] = serde_json::json!({
                "capabilities": [
                    "translation memory recall",
                    "terminology boundary guidance",
                    "literary translation workflow skills",
                ],
                "category": "productivity",
                "defaultPrompt":
                    "Use Hieronymus skills and MCP tools when translation memory, \
                     terminology boundaries, or literary workflow context matter.",
                "developerName": "Pavel Obruchnikov",
                "displayName": "Hieronymus",
                "longDescription":
                    "Hieronymus packages local-first translation memory workflows, \
                     strict terminology boundary guidance, and MCP recall for literary \
                     translation agents.",
                "shortDescription":
                    "Local-first translation memory and terminology workflows for agents.",
            });
            manifest["mcpServers"] = serde_json::json!("./.mcp.json");
            assets.insert(
                ".codex-plugin/plugin.json".to_string(),
                pretty_json(&manifest),
            );
        }
        "claude" => {
            assets.insert(
                ".claude-plugin/plugin.json".to_string(),
                pretty_json(&plugin_manifest()),
            );
        }
        "gemini" => {
            assets.insert(
                "gemini-extension.json".to_string(),
                pretty_json(&serde_json::json!({
                    "name": "hieronymus",
                    "version": env!("CARGO_PKG_VERSION"),
                    "contextFileName": "AGENTS.md",
                    "mcpServers": {
                        "hieronymus": {
                            "command": "hieronymus-mcp",
                            "args": [],
                            "env": {},
                        }
                    },
                })),
            );
        }
        "opencode" => {
            assets.insert(
                "opencode/plugin.json".to_string(),
                pretty_json(&serde_json::json!({
                    "name": "hieronymus",
                    "version": env!("CARGO_PKG_VERSION"),
                    "mcp": "./mcp/hieronymus.mcp.json",
                })),
            );
        }
        "openclaw" => {
            assets.insert(
                "openclaw/plugin.json".to_string(),
                pretty_json(&plugin_manifest()),
            );
        }
        other => return Err(format!("unsupported agent plugin target: {other}")),
    }
    Ok(assets)
}

/// Render every installation-owned plugin file: absolute destination paths
/// under the config root's `agent-plugins/` directory plus the exact file
/// contents. Deterministic: the same config always renders the same bytes in
/// the same order.
pub fn render(config: &HieronymusConfig) -> Result<Vec<(PathBuf, String)>, String> {
    let root = config.agent_plugins_root();
    let mut rendered = Vec::new();
    for target in TARGETS {
        for (relative, contents) in target_assets(target)? {
            rendered.push((root.join(target).join(relative), contents));
        }
    }
    rendered.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(rendered)
}

/// Render and write the bundle atomically. Returns the written paths in the
/// same deterministic order. Existing Hieronymus-owned files are replaced;
/// nothing outside the plugins root is touched (host configuration is never
/// rewritten automatically).
pub fn generate(config: &HieronymusConfig) -> Result<Vec<PathBuf>, String> {
    let rendered = render(config)?;
    let mut written = Vec::with_capacity(rendered.len());
    for (path, contents) in &rendered {
        hieronymus::atomic::atomic_write_text(path, contents)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        written.push(path.clone());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_is_deterministic_and_never_bakes_endpoints_or_secrets() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let first = render(&config).unwrap();
        let second = render(&config).unwrap();
        assert_eq!(first, second, "rendering must be deterministic");
        assert!(first.len() > 40, "every target carries the full bundle");

        // The stdio discovery contract: only the MCP-config files carry the
        // stable entry point, and NO generated file ever bakes a fixed port,
        // a loopback address, or a bearer credential. The file filter is a
        // STRING suffix on the rendered path — a `Path::ends_with(".json")`
        // compares whole path components and would match nothing, silently
        // scanning an empty set.
        let mut json_files = 0_usize;
        let mut mcp_configs = 0_usize;
        for (path, contents) in &first {
            assert!(
                path.starts_with(config.agent_plugins_root()),
                "{path:?} escapes the plugins root"
            );
            let file_name = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default();
            let is_mcp_config = matches!(
                file_name,
                "hieronymus.mcp.json" | ".mcp.json" | "gemini-extension.json"
            );
            if path.to_string_lossy().ends_with(".json") {
                json_files += 1;
            }
            if is_mcp_config {
                mcp_configs += 1;
                assert!(
                    contents.contains("hieronymus-mcp"),
                    "{path:?} must register the stable stdio entry point: {contents}"
                );
            }
            assert!(
                !contents.contains("\"port\""),
                "{path:?} must not bake a fixed port: {contents}"
            );
            assert!(
                !contents.to_ascii_lowercase().contains("bearer"),
                "{path:?} must not bake a bearer credential"
            );
            assert!(
                !contents.contains("127.0.0.1"),
                "{path:?} must not bake a loopback endpoint: {contents}"
            );
        }
        assert!(
            json_files > 0,
            "the endpoint guard must scan the generated JSON files"
        );
        assert!(
            mcp_configs > 0,
            "the entry-point guard must scan the MCP-config files"
        );
        // English-first guidance and the strict boundaries survive the port.
        let learn = &first
            .iter()
            .find(|(path, _)| path.ends_with("codex/skills/hieronymus-learn/SKILL.md"))
            .unwrap()
            .1;
        assert!(learn.contains("candidate-only"), "{learn}");
        let bootstrap = &first
            .iter()
            .find(|(path, _)| path.ends_with("codex/skills/hieronymus-bootstrap/SKILL.md"))
            .unwrap()
            .1;
        assert!(
            bootstrap.contains("Do not approve terminology proposals yourself"),
            "{bootstrap}"
        );
        let hooks = &first
            .iter()
            .find(|(path, _)| path.ends_with("codex/hooks/hooks.codex.json"))
            .unwrap()
            .1;
        assert!(hooks.contains("hieronymus-agent-hook"), "{hooks}");
    }
}
