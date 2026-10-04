//! Generated agent-plugin assets (plan M5, port of the Python
//! `agent_assets`/`agent_plugins` renderers): the installation-owned bundle
//! under the data root's `agent-plugins/` directory — workflow skills, the
//! MCP registration, Codex hooks, and one manifest per supported host.
//!
//! Host configuration is never rewritten here: the generated files are
//! documentation and copy-paste sources for the user's manual host setup.
//! The MCP registration uses the stable `hieronymus-mcp` entry point (the
//! `argv[0]` link to `hiero mcp`): the stdio adapter discovers the local
//! daemon through the data root's discovery record, so the generated config
//! carries no fixed port and no baked-in bearer token.
//!
//! Skills describe autonomous scoped capture and trusted correction dependencies.
//! Hieronymus-owned
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
const TARGETS: [&str; 6] = ["codex", "claude", "gemini", "opencode", "openclaw", "pi"];

const AGREEMENT_BOUNDARY_TEXT: &str = "Before applying or writing memory in a project, follow the current user's free-text project agreement discovered by hieronymus-bootstrap. With no special instruction, project files are primary and Hieronymus is additional memory. Preserve an active Hieronymus rule's actual status and provenance even when the agreement makes it inapplicable to the current work. Technical binding, source_role and credibility labels confer neither trust nor write permission. Unknown identity, order or viewpoint stays tentative. Never turn a quoted correction into explicit-user authority or request routine human approval.";

fn workflow_skill(name: &str, description: &str, body: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\n---\n\n{body}\n\n{AGREEMENT_BOUNDARY_TEXT}\n"
    )
}
fn bootstrap_skill() -> String {
    workflow_skill(
        "hieronymus-bootstrap",
        "Start ordinary literary work with scoped recall, capture and immediate correction dependencies.",
        r#"Use at the start of work in a writing or translation project.

1. Identify the actual project, series, language direction and current user agreement. For CWS, use the [project context workflow](resources/cws-project.md) and read the reported AGENTS.md. Project files are primary by default; Hieronymus supplies additional memory. A project binding does not itself grant permission to store content.
2. Verify that this agent can use Hieronymus MCP and the workflow skills. During setup, also verify that the intended hooks are actually loaded; installing skills or creating a project binding alone does not connect hooks. For missing or stale integration, use hieronymus-doctor and its [integration check](resources/agent-health.md). Do not claim setup is complete until the actual connection is verified.
3. Reuse a compatible session owned by the leading task. For sessionless research, do not start one unnecessarily. When creating a task session with hieronymus_session_start, retain its returned ID and own its completion. Carry the actual chapter, language direction, chronology and viewpoint; unknown context stays unknown.
4. Recall working memory and source evidence with hieronymus_recall and hieronymus_rag_search before using them. Keep current truth separate from research-only/non_current evidence. Missing semantic retrieval means incomplete recall. Capture only permitted, significant observations during work, and validate translations before returning them.
5. Complete a session this task created with hieronymus_session_complete when the task finishes, including a stopped task that will not continue. Check the acknowledgement before reporting completion. Nested skills leave their caller's session open. For continuing work, hand off the active ID and remaining task explicitly; a failed completion must be reported with its ID. Do not rely on chat shutdown to finish a read/learn task.

If automatic prompt capture is intended, follow the [capture and binding workflow](resources/prompt-capture.md). With configured Jev, the current user message is sent externally before deciding whether to retain it; a local fallback is used when unavailable. Neither filter guarantees perfect relevance, and relevance does not mean a correction was applied. Pausing capture does not complete a session or retract an already started external request. Preserve the session ID before unbinding and complete the ended task explicitly. Never silently enable capture in a read-only project.

For a genuine user correction, follow hieronymus-remember. Use the actual applied decision before dependent reads/validation, and preserve tentative or rejected outcomes. Never fabricate or replay a host event. Report blockers in the conversation, without creating unsolicited project reports."#,
    )
}
fn doctor_skill() -> String {
    workflow_skill(
        "hieronymus-doctor",
        "Diagnose an existing project's Hieronymus MCP, skills, hooks and open memory sessions; repair requested integration.",
        r#"Use when an already bootstrapped project has missing hooks, stale skills, disconnected MCP or read/learn sessions left open. Follow the project-context workflow from hieronymus-bootstrap and preserve the project agreement; do not restart setup blindly or create a diagnostic memory session.

Follow the [project integration and session health check](resources/agent-health.md). Verify the actual host-loaded configuration, generated contents and current cache separately. A project binding is not hook installation. Repair only requested integration, preserve unrelated settings, and use the host's normal command trust mechanism. Report what is verified, missing, stale or still unverified; generated configuration and synthetic CLI tests are not native-host acceptance.

Correlate open memory-session IDs with actual task ownership and host binding. Complete an established ended task explicitly, recover an abandoned bound host only after it has stopped, and leave live or unknown sessions alone. Do not bulk-close sessions by age, silently enable capture, fabricate lifecycle events, read private manuscript text for diagnosis, or edit the database directly."#,
    )
}
fn recall_skill() -> String {
    workflow_skill(
        "hieronymus-recall",
        "Recall remembered knowledge and source evidence before ordinary chapter work.",
        r#"Recall automatically before translation or review and after changing chapters or sessions. First apply the project-context workflow from hieronymus-bootstrap and use its free-text project agreement when deciding which recalled material governs the current work. Use hieronymus_recall for working memory and contracts, and hieronymus_rag_search for source retrieval. Preserve actual languages, timeline, volume/chapter/scene and viewpoint; pass required_decision_id from an applied hook correction before using downstream results. Respect claim disposition, excluded scopes and knowledge gates in every lane. Unknown or future material belongs outside current truth. A useful voice/relationship/rendering should carry across sessions without an author label or special request when the agreement allows it. If project files govern a disagreement, report the Hieronymus rule's actual active status and provenance while applying the project instruction; do not claim the rule was invalidated.

Ordinary work uses mixed hieronymus_recall, which combines crystals, recent memories and RAG; a standalone hieronymus_rag_search is additional source research, not a replacement for memory recall. Read both results and non_current: ordinary recall defaults to source-inspection mode and returns remembered text even when story context is unknown, with its disposition attached. The default limit is 100 per result section; request more when useful. Only use story_query_mode Current when the task specifically needs current-scene truth. Missing chronology is not a reason to skip recall or discard a relevant memory. An active task session lets returned crystals create deduplicated short-term working copies, including research evidence, while retaining original claim lineage. Reuse an owned session or start one for substantive chapter/translation/review work, then complete it when that task ends. Populate supported observations and terminology automatically; do not ask the author to maintain a termbase. Authors correct mistakes. Never turn uncertain evidence into an approved rule merely because it was recalled.

Record relevance feedback using the installed `hiero recall-feedback --recall-id <id> --idempotency-key <key> --miss <activation ids>` (or --useful) with the actual recall_id and returned activation IDs. An unhelpful result is relevance feedback, not a claim invalidation. Reuse the same feedback identity for retries; do not manufacture activations or decrement repeatedly."#,
    )
}
fn learn_skill() -> String {
    workflow_skill(
        "hieronymus-learn",
        "Capture supported observations and activate terminology from independent source evidence.",
        r#"Use the project-context workflow from hieronymus-bootstrap and preserve the project agreement. Store only observations this task is permitted to retain; do not ingest a whole project or protected/hidden material automatically.

Reuse the leading task's compatible session. If this standalone task creates a session, retain its ID and complete it with hieronymus_session_complete after the work finishes, before the final report. Do not complete a caller-owned session. Report failed completion or explicitly continuing work with the active ID.

Capture significant voice, relationships, events and uncertainty through hieronymus_short_term_add or batch. Give each assertion only the scope its evidence supports; distinguish recurring traits from chapter-local events. Unknown identity, chronology or viewpoint stays tentative. See the [scoped observation input](resources/claim-capture.md) when constructing claims. Inspect returned IDs, revisions and warnings before relying on later recall.

For terminology, resolve the actual concept and capture source and aligned-rendering evidence from exact snapshots. See the [complete evidence workflow](resources/evidence-capture.md) for UTF-8 byte selections, whole-file hashes and observed bindings. Two independent aligned paragraph anchors, supported identity/scope and matching revisions can support learned activation; duplicate anchors or stale evidence cannot. A learned replacement needs scoped contradiction evidence. Use hieronymus_termbase_propose and hieronymus_decide, then inspect the actual resulting status and contract. Never call a tentative decision active."#,
    )
}
fn read_skill() -> String {
    workflow_skill(
        "hieronymus-read",
        "Read source files into contextual RAG and capture important conclusions.",
        r#"Before importing a project file, apply the project-context workflow from hieronymus-bootstrap. Identify its structural role, then import only task-required evidence whose free-text project agreement and current user request permit import. Do not import a whole project, protected lifecycle state, supplied originals, secrets, or `<hidden>` content automatically.

Reuse a compatible session owned by the leading workflow; do not create or complete a second session inside a nested skill. For a standalone bounded task, retain any session_id this invocation creates and call hieronymus_session_complete with it after capture/validation finishes, before reporting completion. On an abort with no continuation, complete that owned session as well; on completion failure report its ID and pending error. If work is continuing, hand off its active ID explicitly. Never close a caller-owned or unrelated live session. Do not wait for chat shutdown: SessionEnd only covers its explicit binding. Completion does not mean Dream finished.

Import permitted actual source files with hieronymus_rag_import. Supply new typed claims in a map keyed by zero-based numeric chunk index (the applicability ellipsis is a placeholder for the complete object documented by hieronymus-learn): `{"claims":{"0":[{"text":"Alex speaks briefly.","concept_id":null,"applicability":{...}}]}}`. `text` and `concept_id` belong on ClaimInput, outside applicability. `claim_lineage` is only for binding an already observed claim_id; it does not create a new assertion. Use exact supported story applicability rather than treating missing metadata as timeless truth. Never invent chronology, viewpoint, `All`, or concept identity. RAG retains source text; short-term memory stores concise conclusions. Capture important voice, relationships, events, terminology and uncertainty as independently scoped claims while doing ordinary work. English-first analysis may help cross-language recall, but preserve exact source forms and registered language identifiers. Do not copy a whole book into one memory or discard difficult source-language evidence. Retrieve both working memory and semantic RAG on subsequent chapters, respecting current versus research disposition."#,
    )
}
fn remember_skill() -> String {
    workflow_skill(
        "hieronymus-remember",
        "Distinguish immediate trusted corrections from relevance and ordinary learned observations.",
        r#"Apply the project-context workflow from hieronymus-bootstrap and the current project agreement.

1. Distinguish a correction to selected memory from an ordinary observation or relevance feedback. The user does not need a special Remember command.
2. For a genuine user correction, inspect the returned outcome. Relevant text can still be unsupported or ambiguous and remain tentative. The prompt route currently accepts a single-line English command with multilingual payloads; use the [supported input examples](resources/correction-input.md) when needed. The author console offers structured correction input for unsupported wording.
3. Only Applied/Replayed outcomes supply a usable required_decision_id for dependent recall, contracts and validation. Invalidation changes the selected claim only; qualification keeps its exact scope. A tentative or rejected event is not an applied correction.
4. If context or selection is missing, establish it for a future genuine user event. Never translate, rewrite or replay an already delivered prompt, invent a receipt, or synthesize trusted user ingress. Ordinary permitted observations remain agent evidence.

Use relevance feedback for an unhelpful search result. Report actual correction and background-consolidation states; a pending job is not completed work."#,
    )
}
fn translate_skill() -> String {
    workflow_skill(
        "hieronymus-translate",
        "Translate with current contracts and automatically maintained literary memory.",
        r#"For each chapter, first apply the project-context workflow from hieronymus-bootstrap. Use the free-text project agreement to choose which project files and Hieronymus memories govern the translation and which stores may be updated. With no special instruction, project files are primary and Hieronymus is additional memory; do not require duplicate Markdown storage. Never translate or import secrets or `<hidden>` content automatically.

Establish actual story context, recall prior voice/relationships/renderings, translate using the applicable current deterministic contracts, capture only permitted significant new supported observations, then call hieronymus_termbase_validate with raw_text and translated_text. Pass any applied correction's required_decision_id. Respect outside-scope rules and narrator/character knowledge gates; a later revelation must not leak into an earlier character scene. Validation failure is actionable under the agreement; if a project instruction governs a disagreement, preserve and report the Hieronymus rule's actual active status and provenance rather than silently overriding either record. Continue this loop across sessions without requiring manual curation."#,
    )
}
fn review_skill() -> String {
    workflow_skill(
        "hieronymus-review",
        "Review translation against current terminology, factual validity and knowledge scope.",
        r#"Apply the project-context workflow from hieronymus-bootstrap before review, then recall and validate at the actual story position and viewpoint. Use the free-text project agreement when deciding whether project evidence or Hieronymus memory governs each finding. Inspect applicable strict terminology failures first, then voice and relationships. When the agreement resolves a disagreement, preserve the Hieronymus rule's actual status and provenance in the report; do not describe it as invalidated or superseded unless Hieronymus records that change. Capture only permitted recurring supported observations with scoped claims. Distinguish a factually wrong memory from merely irrelevant retrieval: trusted invalidation/qualification uses the selected claim, while relevance uses the actual recall activation. A reviewer's opinion remains ordinary agent evidence; a label cannot grant explicit-user authority. Do not broaden a chapter correction to all volumes or convert research-only truth into current narration."#,
    )
}
fn orchestrate_skill() -> String {
    workflow_skill(
        "hieronymus-orchestrate",
        "Run the automatic session, recall, capture, validation and feedback loop.",
        r#"Keep the current leading skill in control; never recursively launch another orchestrator. Start with the project-context workflow from hieronymus-bootstrap, apply the current user's free-text project agreement, then run recall→work/permitted capture→validation→correlated relevance feedback for ordinary chapter work. Do not require synchronized writes or a Markdown mirror unless the current instruction does. Capture each permitted supported assertion with the ClaimInput shape documented by hieronymus-learn, then inspect actual returned claim IDs/revisions and conservative context warnings before relying on continuity. Preserve observed authority revision and selected immutable context. After a trusted correction, require its applied decision in dependent calls; after an unresolved signal, observe the new revision before explicitly binding a new event. Complete the memory session owned by this task through hieronymus_session_complete after its work finishes, before the final report; do not rely solely on chat shutdown. Preserve an active session only for explicitly continuing work, with its ID in the handoff. Nested skills must not complete the leading task session. Recover memory in the next session.

Background dreaming and correction consolidation follow daemon policy and budgets. A provider outage leaves immediate corrections effective, with durable retries/parking/recovery. Report actual pending/failed/complete state; never fabricate a completion, recursively enqueue the same correction, or repeat relevance deltas. Native host and semantic retrieval support require actual qualification; generated files alone are not acceptance evidence."#,
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

/// The Codex session hooks: the stable `argv[0]` link names that route to
/// `hiero agent-hook`.
fn prompt_hooks_json(host: &str) -> String {
    let command = if host == "codex" {
        "hieronymus-agent-hook user-prompt-submit --host codex"
    } else {
        "hieronymus-agent-hook user-prompt-submit --host \"${HIERONYMUS_AGENT_HOST:-claude}\""
    };
    let host_argument = if host == "codex" {
        "codex"
    } else {
        "\"${HIERONYMUS_AGENT_HOST:-claude}\""
    };
    pretty_json(&serde_json::json!({"hooks":{
        "UserPromptSubmit":[{"hooks":[{"type":"command","command":command}]}],
        "SessionEnd":[{"hooks":[{"type":"command","command":format!("hieronymus-agent-hook session-end --host {host_argument}")}]}],
        "SessionStart":[{"hooks":[{"type":"command","command":format!("hieronymus-agent-hook session-start --host {host_argument}")}]}]
    }}))
}
fn codex_hooks_json() -> String {
    prompt_hooks_json("codex")
}

fn pi_mcp_json() -> String {
    pretty_json(&serde_json::json!({
        "mcpServers": {
            "hieronymus": {
                "command": "hieronymus-mcp",
                "args": [],
                "env": {},
                "protocolVersion": "2026-07-28"
            }
        }
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
            "skills/hieronymus-bootstrap/resources/cws-project.md".to_string(),
            include_str!("../resources/cws-project.md").to_string(),
        ),
        (
            "skills/hieronymus-learn/resources/evidence-capture.md".to_string(),
            include_str!("../resources/evidence-capture.md").to_string(),
        ),
        (
            "skills/hieronymus-doctor/SKILL.md".to_string(),
            doctor_skill(),
        ),
        (
            "skills/hieronymus-doctor/resources/agent-health.md".to_string(),
            include_str!("../resources/agent-health.md").to_string(),
        ),
        (
            "skills/hieronymus-bootstrap/resources/agent-health.md".to_string(),
            include_str!("../resources/agent-health.md").to_string(),
        ),
        (
            "skills/hieronymus-bootstrap/resources/prompt-capture.md".to_string(),
            include_str!("../resources/prompt-capture.md").to_string(),
        ),
        (
            "skills/hieronymus-remember/resources/correction-input.md".to_string(),
            include_str!("../resources/correction-input.md").to_string(),
        ),
        (
            "skills/hieronymus-learn/resources/claim-capture.md".to_string(),
            include_str!("../resources/claim-capture.md").to_string(),
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
            let mut mcp: serde_json::Value =
                serde_json::from_str(&mcp_config_json()).expect("static MCP configuration");
            mcp["mcpServers"]["hieronymus"]["env"]["CODEX_MCP_PROTOCOL_VERSION"] =
                serde_json::json!("2026-07-28");
            assets.insert(".mcp.json".to_string(), pretty_json(&mcp));
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
            manifest["hooks"] = serde_json::json!("./hooks/hooks.codex.json");
            assets.insert(
                ".codex-plugin/plugin.json".to_string(),
                pretty_json(&manifest),
            );
        }
        "claude" => {
            let mut manifest = plugin_manifest();
            manifest["author"] = author;
            manifest["hooks"] = serde_json::json!("./hooks/hooks.json");
            assets.insert("hooks/hooks.json".into(), prompt_hooks_json("claude"));
            assets.insert(
                ".claude-plugin/plugin.json".to_string(),
                pretty_json(&manifest),
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
        "pi" => {
            assets.remove("mcp/hieronymus.mcp.json");
            assets.remove("hooks/hooks.codex.json");
            assets.insert(
                "package.json".to_string(),
                pretty_json(&serde_json::json!({
                    "name": "hieronymus-pi",
                    "version": env!("CARGO_PKG_VERSION"),
                    "private": true,
                    "type": "module",
                    "pi": {
                        "skills": ["./skills"],
                        "mcp": "./mcp.json"
                    }
                })),
            );
            assets.insert("mcp.json".to_string(), pi_mcp_json());
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
    rendered.push((root.join(".agents/plugins/marketplace.json"), pretty_json(&serde_json::json!({
        "name":"hieronymus-local", "interface":{"displayName":"Installed Hieronymus"},
        "plugins":[{"name":"hieronymus","source":{"source":"local","path":"./codex"},"policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Productivity"}]
    }))));
    rendered.push((root.join(".claude-plugin/marketplace.json"), pretty_json(&serde_json::json!({
        "name": "hieronymus-local",
        "description": "Local Hieronymus translation-memory plugin catalog.",
        "owner": { "name": "Pavel Obruchnikov", "email": "me@inkyquill.net" },
        "plugins": [{
            "name": "hieronymus",
            "description": "Translation memory, termbase, recall, and dreaming workflows for agents.",
            "version": env!("CARGO_PKG_VERSION"),
            "source": "./claude"
        }]
    }))));
    rendered.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(rendered)
}

/// Render and write the bundle atomically. Returns the written paths in the
/// same deterministic order. Existing Hieronymus-owned files are replaced;
/// nothing outside the plugins root is touched (host configuration is never
/// rewritten automatically).
pub fn generate(config: &HieronymusConfig) -> Result<Vec<PathBuf>, String> {
    let rendered = render(config)?;
    // Reject redirected installation-owned subdirectories before any replacement.
    // The configuration root itself may be an explicitly chosen platform alias.
    let retired_pi_extension = config
        .agent_plugins_root()
        .join("pi/extensions/hieronymus.ts");
    for path in rendered
        .iter()
        .map(|(path, _)| path)
        .chain(std::iter::once(&retired_pi_extension))
    {
        for ancestor in path.ancestors().take_while(|p| *p != config.config_root()) {
            match std::fs::symlink_metadata(ancestor) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(format!(
                        "refusing symlinked plugin path: {}",
                        ancestor.display()
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    if retired_pi_extension.exists() {
        std::fs::remove_file(&retired_pi_extension).map_err(|error| {
            format!(
                "cannot remove retired Pi extension {}: {error}",
                retired_pi_extension.display()
            )
        })?;
        let _ = std::fs::remove_dir(
            retired_pi_extension
                .parent()
                .expect("retired extension has a parent"),
        );
    }
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
    fn pi_package_uses_the_installed_adapter_and_pinned_protocol() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let rendered = render(&config).unwrap();
        let asset = |suffix: &str| {
            &rendered
                .iter()
                .find(|(path, _)| path.ends_with(suffix))
                .unwrap_or_else(|| panic!("missing generated Pi asset {suffix}"))
                .1
        };

        let package: serde_json::Value = serde_json::from_str(asset("pi/package.json")).unwrap();
        assert!(package["pi"].get("extensions").is_none());
        assert_eq!(package["pi"]["skills"][0], "./skills");
        assert_eq!(package["pi"]["mcp"], "./mcp.json");
        let mcp: serde_json::Value = serde_json::from_str(asset("pi/mcp.json")).unwrap();
        assert_eq!(mcp["mcpServers"]["hieronymus"]["command"], "hieronymus-mcp");
        assert_eq!(
            mcp["mcpServers"]["hieronymus"]["protocolVersion"],
            "2026-07-28"
        );
        assert_eq!(mcp["mcpServers"].as_object().unwrap().len(), 1);
        assert!(mcp["mcpServers"]["hieronymus"].get("tools").is_none());
        let pi_files = rendered
            .iter()
            .filter(|(path, _)| path.starts_with(config.agent_plugins_root().join("pi")))
            .collect::<Vec<_>>();
        assert_eq!(
            pi_files
                .iter()
                .filter(|(path, _)| path.ends_with("SKILL.md"))
                .count(),
            9
        );
        assert!(pi_files.iter().all(|(path, contents)| {
            !path.to_string_lossy().contains("extensions/")
                && !contents.contains("HIERONYMUS_PI_TRUSTED_LAUNCH")
                && !contents.contains("--no-extensions")
        }));
    }

    #[test]
    fn claude_marketplace_matches_the_generated_plugin() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let rendered = render(&config).unwrap();
        let contents = &rendered
            .iter()
            .find(|(path, _)| path.ends_with(".claude-plugin/marketplace.json"))
            .expect("Claude marketplace catalog")
            .1;
        let catalog: serde_json::Value = serde_json::from_str(contents).unwrap();
        let plugin = &catalog["plugins"][0];
        assert_eq!(catalog["name"], "hieronymus-local");
        assert_eq!(catalog["plugins"].as_array().unwrap().len(), 1);
        assert_eq!(plugin["name"], "hieronymus");
        assert_eq!(plugin["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(plugin["source"], "./claude");
        assert!(plugin.get("command").is_none());
        assert!(plugin.get("url").is_none());
        assert!(plugin.get("authentication").is_none());
    }

    #[test]
    fn generation_removes_the_retired_pi_trusted_extension() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let retired = config
            .agent_plugins_root()
            .join("pi/extensions/hieronymus.ts");
        std::fs::create_dir_all(retired.parent().unwrap()).unwrap();
        std::fs::write(&retired, "stale trusted input handler").unwrap();
        generate(&config).unwrap();
        assert!(!retired.exists());
    }

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
        let hooks = &first
            .iter()
            .find(|(path, _)| path.ends_with("codex/hooks/hooks.codex.json"))
            .unwrap()
            .1;
        assert!(hooks.contains("hieronymus-agent-hook"), "{hooks}");
    }
}
