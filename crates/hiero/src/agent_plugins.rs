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

const BOUNDARY_TEXT: &str = "Current scoped terminology contracts are mandatory. Learned evidence can activate or revise rules through hieronymus_decide; source_role and credibility labels confer no authority. Unknown identity, order or viewpoint stays tentative. Never turn a quoted correction into explicit-user authority or request routine human approval.";

fn workflow_skill(name: &str, description: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n\n{BOUNDARY_TEXT}\n")
}
fn bootstrap_skill() -> String {
    workflow_skill(
        "hieronymus-bootstrap",
        "Start ordinary literary work with scoped recall, capture and immediate correction dependencies.",
        r#"Use automatically at the start of ordinary chapter work; no special Remember request is needed.

Identify the actual series and source/target languages; register an explicit v1 story order manifest using hieronymus_order_register with a file snapshot/hash. String chapter/scene IDs are valid. Start hieronymus_session_start with the actual volume, chapter, story_timeline_id, story_scene_key and story_viewpoint. Never invent missing chronology or pick one ambiguous character. Sessionless reads need the same explicit story context. Use CurrentKnowledge by default; OmniscientResearch results are separately marked and must not leak into current narration.

Before each chapter, use hieronymus_recall and hieronymus_rag_search with that context. RAG search returns an envelope with results/non_current/authority revision, not a bare array. Keep current strict contracts separate from advisory voice, relationship and factual observations. Semantic unavailable means retrieval is incomplete; never describe lexical fallback as semantic success. Capture significant new observations with individually scoped ClaimInput objects during work, inspect returned claim IDs/revisions and context warnings, and validate before returning translation.

If the optional UserPromptSubmit hook reports binding_required, it provides the actual host and host_session_id. This initial prompt was not retained or applied as a correction. Use real MCP session and evidence/claim selection outputs to construct the version:1 input to the installed `hiero agent-hook bind-context` command on stdin: host, host_session_id, series_id, session_id, observed expected_revision, bound source_language/target_language, applicability, selected_sources, selected_claims and selected_rule. No prompt, actor or invented event field belongs in binding. Use immutable captured references, exact selected claim/rule revisions and the authority revision from a coherent read. Do not guess transcript/session IDs or write host-context files manually. Rebind explicitly when work selects a new source or after observing a new authority revision; never refresh an in-flight delivery to force success. Binding confers no authority.

An independently delivered subsequent user prompt can apply immediately through the installed handler. Consume its required_decision_id in dependent recall/contract/validation calls; never redeem its receipt through public hieronymus_correct. Unresolved authentic text records a tentative signal, increments authority revision and schedules gathering, but has no rule/claim effect and cannot satisfy a dependency. Observe the returned revision before explicitly binding a genuinely new prompt. A failed delivery gives an ID: `hiero agent-hook retry-delivery --delivery-id <id>` retries its saved context. Re-invoking user-prompt-submit creates a new event, including identical text.

The stable commands hieronymus-mcp, hieronymus-agent-hook and hiero discover the local installation. Optional hook loading is controlled by supported host trust/settings. Normal Pi package loading leaves prompts unchanged and uses the separately installed pi-mcp-adapter for MCP context reads; it does not mint trusted correction provenance. Pi trusted correction delivery requires the documented isolated launch with `HIERONYMUS_PI_TRUSTED_LAUNCH=isolated-v1`, `--no-extensions`, the Hieronymus extension first, the adapter second, and the generated MCP config/skills explicitly selected. The shared Claude/zCode bundle defaults to host claude; a zCode launcher must explicitly set HIERONYMUS_AGENT_HOST=zcode. Codex uses its codex hook. Set HIERONYMUS_DATA_ROOT for a nondefault installation. Generated plugins remain in the application data root, never a book folder. Report a blocking issue in the conversation; do not create unsolicited book-file reports."#,
    )
}
fn recall_skill() -> String {
    workflow_skill(
        "hieronymus-recall",
        "Recall relevant current memory before ordinary chapter work.",
        r#"Recall automatically before translation or review and after changing chapters or sessions. Use hieronymus_recall for working memory and contracts, and hieronymus_rag_search for source retrieval. Preserve actual languages, timeline, volume/chapter/scene and viewpoint; pass required_decision_id from an applied hook correction before using downstream results. Respect claim disposition, excluded scopes and knowledge gates in every lane. Unknown or future material belongs outside current truth. A useful voice/relationship/rendering should carry across sessions without an author label or special request.

Record relevance feedback using the installed `hiero recall-feedback --recall-id <id> --idempotency-key <key> --miss <activation ids>` (or --useful) with the actual recall_id and returned activation IDs. An unhelpful result is relevance feedback, not a claim invalidation. Reuse the same feedback identity for retries; do not manufacture activations or decrement repeatedly."#,
    )
}
fn learn_skill() -> String {
    workflow_skill(
        "hieronymus-learn",
        "Capture supported observations and activate terminology from independent source evidence.",
        r#"During normal reading and translation, store significant observations with hieronymus_short_term_add or batch, using kind/text and individually scoped claims. New-claim template (replace the unquoted `SERIES_ID` token with the actual series ID returned by Hieronymus before sending): `{"text":"Alex speaks in clipped phrases.","concept_id":null,"applicability":{"series_id":SERIES_ID,"timeline_id":null,"volume_key":null,"chapter_key":null,"scope_predicates":[],"valid_from":null,"valid_until":null,"metadata_state":"Unspecified","knowledge_gates":[]}}`. `text` and `concept_id` are siblings of `applicability`; use `concept_id:null` when identity is unknown; never invent an ID. This template records unknown context. Populate applicability only from observed, supported context; when typed context is unavailable, omit the optional claims array and let ordinary capture preserve the session context. Use actual resolved positions and viewpoints. Never invent chronology, a character, or `All`; unresolved context or a missing knowledge gate remains conservatively outside current truth. Separate reusable voice/relationship assertions from chapter-local events and give each only the scope supported by the source. Inspect returned claim IDs, revisions and warnings before relying on later recall. Preserve uncertainty and source evidence. No author label, approval queue or special Learn request is needed.

For terminology, create/reuse the exact concept, capture immutable source_passage and aligned_rendering evidence via hieronymus_evidence_capture from actual file snapshots and hashes. Offsets index the whole UTF-8 file; alignments link aligned_source_id. Two independent aligned paragraph anchors, a resolved identity and scope, and matching revisions can support learned activation through hieronymus_termbase_propose plus hieronymus_decide Activate. A learned replacement additionally needs scoped contradiction evidence. Reused bytes/duplicate anchors and stale revisions do not satisfy policy. Read returned status and resulting contract; never claim a tentative decision activated a rule. Two people named Alex need separate concepts and explicit anchors; ambiguity remains an observation, never a global replacement."#,
    )
}
fn read_skill() -> String {
    workflow_skill(
        "hieronymus-read",
        "Read source files into contextual RAG and capture important conclusions.",
        r#"Import actual source files with hieronymus_rag_import. Supply new typed claims in a map keyed by zero-based numeric chunk index (the applicability ellipsis is a placeholder for the complete object documented by hieronymus-learn): `{"claims":{"0":[{"text":"Alex speaks briefly.","concept_id":null,"applicability":{...}}]}}`. `text` and `concept_id` belong on ClaimInput, outside applicability. `claim_lineage` is only for binding an already observed claim_id; it does not create a new assertion. Use exact supported story applicability rather than treating missing metadata as timeless truth. Never invent chronology, viewpoint, `All`, or concept identity. RAG retains source text; short-term memory stores concise conclusions. Capture important voice, relationships, events, terminology and uncertainty as independently scoped claims while doing ordinary work. English-first analysis may help cross-language recall, but preserve exact source forms and registered language identifiers. Do not copy a whole book into one memory or discard difficult source-language evidence. Retrieve both working memory and semantic RAG on subsequent chapters, respecting current versus research disposition."#,
    )
}
fn remember_skill() -> String {
    workflow_skill(
        "hieronymus-remember",
        "Distinguish immediate trusted corrections from relevance and ordinary learned observations.",
        r#"Corrections require no Remember command. The independently supplied host UserPromptSubmit handler or dedicated authenticated console applies clear selected rendering, invalidation or qualification immediately. If the hook has no binding, say the correction was not applied; establish an explicit selection for a subsequent genuine event. Do not replay quoted user text through shell/model arguments to fabricate a host event, invent receipt_ref, or set source_role=user/user_rule as authority.

Consume Applied/Replayed required_decision_id before dependent reads/validation. An invalidation marks only the selected claim incorrect and invents no replacement. Qualification preserves its exact scope. Unhelpful recall goes to relevance feedback instead. Ambiguous selection stays tentative with visible reasons. Provider outage cannot delay an already applied correction; consolidation is durable background work with retries, not evidence that a provider run succeeded. Never claim completion from a pending/parked job."#,
    )
}
fn translate_skill() -> String {
    workflow_skill(
        "hieronymus-translate",
        "Translate with current contracts and automatically maintained literary memory.",
        r#"For each chapter: establish actual story context, recall prior voice/relationships/renderings, translate using current deterministic contracts, capture significant new supported observations, then call hieronymus_termbase_validate with raw_text and translated_text. Pass any applied correction's required_decision_id. Respect outside-scope rules and narrator/character knowledge gates; a later revelation must not leak into an earlier character scene. Validation failure is actionable; do not silently override the corrected rendering with Dream or an older active projection. Continue this loop across sessions without requiring manual curation."#,
    )
}
fn review_skill() -> String {
    workflow_skill(
        "hieronymus-review",
        "Review translation against current terminology, factual validity and knowledge scope.",
        r#"Recall and validate at the actual story position and viewpoint. Inspect strict terminology failures first, then voice and relationships. Capture recurring supported observations with scoped claims. Distinguish a factually wrong memory from merely irrelevant retrieval: trusted invalidation/qualification uses the selected claim, while relevance uses the actual recall activation. A reviewer's opinion remains ordinary agent evidence; a label cannot grant explicit-user authority. Do not broaden a chapter correction to all volumes or convert research-only truth into current narration."#,
    )
}
fn orchestrate_skill() -> String {
    workflow_skill(
        "hieronymus-orchestrate",
        "Run the automatic session, recall, capture, validation and feedback loop.",
        r#"Run bootstrap→recall→work/capture→validation→correlated relevance feedback for ordinary chapter work. Capture each supported assertion with the ClaimInput shape documented by hieronymus-learn, then inspect actual returned claim IDs/revisions and conservative context warnings before relying on continuity. Preserve observed authority revision and selected immutable context. After a trusted correction, require its applied decision in dependent calls; after an unresolved signal, observe the new revision before explicitly binding a new event. Complete sessions normally and recover memory in the next session.

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
    pretty_json(
        &serde_json::json!({"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":command}]}]}}),
    )
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

fn pi_extension() -> String {
    r#"import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

type HookPayload = Record<string, unknown>;
type HookRunner = (subcommand: "session-start" | "user-prompt-submit", payload: HookPayload, cwd: string) => Promise<unknown>;

function verifiedIsolatedLaunch(): boolean {
  if (process.env.HIERONYMUS_PI_TRUSTED_LAUNCH !== "isolated-v1" || !process.argv.includes("--no-extensions")) return false;
  const short = process.argv.indexOf("-e");
  const long = process.argv.indexOf("--extension");
  const index = short >= 0 && long >= 0 ? Math.min(short, long) : Math.max(short, long);
  const firstExtension = index >= 0 ? process.argv[index + 1] : undefined;
  return Boolean(firstExtension) && resolve(firstExtension) === fileURLToPath(import.meta.url);
}

function installedHook(subcommand: "session-start" | "user-prompt-submit", payload: HookPayload, cwd: string): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const child = spawn("hieronymus-agent-hook", [subcommand, "--host", "pi"], {
      cwd,
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8").on("data", (chunk) => { stdout += chunk; });
    child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code !== 0) {
        reject(new Error(`Hieronymus ${subcommand} failed (${code}): ${stderr.slice(0, 512)}`));
        return;
      }
      try { resolve(JSON.parse(stdout)); }
      catch { reject(new Error(`Hieronymus ${subcommand} returned invalid JSON`)); }
    });
    child.stdin.end(JSON.stringify(payload));
  });
}

export function createHieronymusHandlers(
  runHook: HookRunner = installedHook,
  options: {
    trustedLaunch?: boolean;
    notify?: (message: string) => void;
  } = {},
) {
  let activeSession: string | undefined;
  let generation = 0;
  let idleSlashInFlight: object | undefined;
  let pendingIdleSlash: { generation: number; sessionId: string; context: string } | undefined;
  const trustedLaunch = options.trustedLaunch ?? verifiedIsolatedLaunch();
  const notify = (ctx: any, message: string) => {
    if (options.notify) options.notify(message);
    else ctx.ui.notify(message, "error");
  };

  return {
    async sessionStart(_event: unknown, ctx: any) {
      const invocation = ++generation;
      activeSession = undefined;
      idleSlashInFlight = undefined;
      pendingIdleSlash = undefined;
      if (!trustedLaunch) return;
      const sessionId = ctx.sessionManager.getSessionId();
      await runHook("session-start", {
        hook_event_name: "SessionStart",
        session_id: sessionId,
        cwd: ctx.cwd,
      }, ctx.cwd);
      if (invocation === generation) activeSession = sessionId;
    },
    async input(event: any, ctx: any) {
      if (event.source !== "interactive") return { action: "continue" as const };
      if (!trustedLaunch) {
        return { action: "continue" as const };
      }
      const sessionId = ctx.sessionManager.getSessionId();
      if (!activeSession || sessionId !== activeSession) {
        notify(ctx, "Hieronymus blocked this prompt because the Pi session identity was not validated.");
        return { action: "handled" as const };
      }
      if (event.text.startsWith("/") && event.streamingBehavior) {
        notify(ctx, "Hieronymus blocked this slash skill/template while Pi is streaming. Retry when Pi is idle so native expansion and trusted receipt context stay in one turn.");
        return { action: "handled" as const };
      }
      if (idleSlashInFlight || pendingIdleSlash) {
        notify(ctx, "Hieronymus blocked overlapping input. Retry after the current slash prompt starts.");
        return { action: "handled" as const };
      }
      if (event.text.startsWith("/") && !ctx.isIdle()) {
        notify(ctx, "Hieronymus blocked this slash skill/template because Pi is not idle. Retry when Pi is idle.");
        return { action: "handled" as const };
      }
      const invocation = generation;
      const slashToken = event.text.startsWith("/") ? {} : undefined;
      if (slashToken) idleSlashInFlight = slashToken;
      try {
        const response: any = await runHook("user-prompt-submit", {
          hook_event_name: "UserPromptSubmit",
          session_id: sessionId,
          prompt: event.text,
          images_present: Boolean(event.images?.length),
        }, ctx.cwd);
        if (invocation !== generation || activeSession !== sessionId) {
          notify(ctx, "Hieronymus blocked this prompt because its session changed during delivery.");
          return { action: "handled" as const };
        }
        const context = response?.hookSpecificOutput?.additionalContext;
        if (typeof context !== "string" || !context) {
          throw new Error("Hieronymus prompt delivery returned no trusted turn context");
        }
        const imageNotice = event.images?.length
          ? "\n\nAttached image content was not included in the trusted correction; only the exact interactive text was delivered. Do not claim image content as trusted or infer it through OCR."
          : "";
        const deliveryContext = context + imageNotice;
        if (slashToken) {
          if (!ctx.isIdle()) {
            notify(ctx, "Hieronymus blocked this slash skill/template because Pi stopped being idle during trusted delivery.");
            return { action: "handled" as const };
          }
          pendingIdleSlash = { generation: invocation, sessionId, context: deliveryContext };
          return { action: "continue" as const };
        }
        return {
          action: "transform" as const,
          text: `${event.text}\n\n<HieronymusTrustedDelivery>\n${deliveryContext}\n</HieronymusTrustedDelivery>`,
          images: event.images,
        };
      } catch (error) {
        const detail = error instanceof Error ? error.message : String(error);
        notify(ctx, `Hieronymus blocked this prompt because trusted delivery failed: ${detail}`);
        return { action: "handled" as const };
      } finally {
        if (idleSlashInFlight === slashToken) idleSlashInFlight = undefined;
      }
    },
    async beforeAgentStart(_event: unknown, ctx: any) {
      const pending = pendingIdleSlash;
      pendingIdleSlash = undefined;
      if (!pending || pending.generation !== generation || pending.sessionId !== ctx.sessionManager.getSessionId()) return;
      return {
        message: {
          customType: "hieronymus-delivery",
          content: pending.context,
          display: true,
        },
      };
    },
    async shutdown() {
      generation += 1;
      activeSession = undefined;
      idleSlashInFlight = undefined;
      pendingIdleSlash = undefined;
    },
  };
}

export default function hieronymus(pi: any) {
  const handlers = createHieronymusHandlers();
  pi.on("session_start", handlers.sessionStart);
  pi.on("input", handlers.input);
  pi.on("before_agent_start", handlers.beforeAgentStart);
  pi.on("session_shutdown", handlers.shutdown);
}
"#.to_string()
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
                        "extensions": ["./extensions/hieronymus.ts"],
                        "skills": ["./skills"],
                        "mcp": "./mcp.json"
                    }
                })),
            );
            assets.insert("mcp.json".to_string(), pi_mcp_json());
            assets.insert("extensions/hieronymus.ts".to_string(), pi_extension());
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
        assert_eq!(package["pi"]["extensions"][0], "./extensions/hieronymus.ts");
        assert_eq!(package["pi"]["mcp"], "./mcp.json");
        let mcp: serde_json::Value = serde_json::from_str(asset("pi/mcp.json")).unwrap();
        assert_eq!(mcp["mcpServers"]["hieronymus"]["command"], "hieronymus-mcp");
        assert_eq!(
            mcp["mcpServers"]["hieronymus"]["protocolVersion"],
            "2026-07-28"
        );
        assert_eq!(mcp["mcpServers"].as_object().unwrap().len(), 1);
        assert!(mcp["mcpServers"]["hieronymus"].get("tools").is_none());
    }

    #[test]
    fn pi_extension_runtime_enforces_trusted_single_turn_delivery() {
        run_pi_extension_harness(false);
    }

    #[test]
    #[ignore = "requires installed Pi 0.85.1 dispatcher"]
    fn installed_pi_dispatcher_enforces_fail_closed_ordering() {
        run_pi_extension_harness(true);
    }

    fn run_pi_extension_harness(use_installed_runner: bool) {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let rendered = render(&config).unwrap();
        let extension = rendered
            .iter()
            .find(|(path, _)| path.ends_with("pi/extensions/hieronymus.ts"))
            .unwrap();
        let extension_path = root.path().join("hieronymus.ts");
        std::fs::write(&extension_path, &extension.1).unwrap();
        let harness = root.path().join("harness.mjs");
        std::fs::write(&harness, PI_EXTENSION_TEST_HARNESS).unwrap();
        let mut command = std::process::Command::new("node");
        command
            .arg("--experimental-strip-types")
            .arg(&harness)
            .arg(&extension_path);
        if use_installed_runner {
            let pi = std::process::Command::new("which")
                .arg("pi")
                .output()
                .unwrap();
            assert!(pi.status.success(), "installed Pi executable is required");
            let mut runner_path =
                std::fs::canonicalize(String::from_utf8(pi.stdout).unwrap().trim()).unwrap();
            runner_path.pop();
            runner_path.pop();
            runner_path.push("core/extensions/runner.js");
            command.arg(&runner_path);
            let mut session_path = runner_path;
            session_path.pop();
            session_path.pop();
            session_path.push("agent-session.js");
            command.arg(&session_path);
            session_path.pop();
            session_path.push("prompt-templates.js");
            command.arg(session_path);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "generated Pi extension behavior failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    const PI_EXTENSION_TEST_HARNESS: &str = r#"
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const { createHieronymusHandlers } = await import(pathToFileURL(process.argv[2]));
const ExtensionRunner = process.argv[3]
  ? (await import(pathToFileURL(process.argv[3]))).ExtensionRunner
  : undefined;
const AgentSession = process.argv[4]
  ? (await import(pathToFileURL(process.argv[4]))).AgentSession
  : undefined;
const expandPromptTemplate = process.argv[5]
  ? (await import(pathToFileURL(process.argv[5]))).expandPromptTemplate
  : undefined;
const calls = [];
let sessionId = '11111111-1111-4111-8111-111111111111';
let idle = true;
const notices = [];
const runHook = async (subcommand, payload) => {
  calls.push({ subcommand, payload });
  if (payload.prompt === 'fail') throw new Error('transport failed');
  return { hookSpecificOutput: { hookEventName: 'UserPromptSubmit', additionalContext: `receipt:${payload.prompt}` } };
};
const handlers = createHieronymusHandlers(runHook, {
  trustedLaunch: true,
  notify: (message) => notices.push(message),
});
const ctx = () => ({ cwd: '/fixture', isIdle: () => idle, sessionManager: { getSessionId: () => sessionId } });
await handlers.sessionStart({}, ctx());
assert.deepEqual(calls[0], { subcommand: 'session-start', payload: { hook_event_name: 'SessionStart', session_id: sessionId, cwd: '/fixture' } });
assert.deepEqual(await handlers.input({ source: 'rpc', text: 'rpc' }, ctx()), { action: 'continue' });
assert.deepEqual(await handlers.input({ source: 'extension', text: 'extension' }, ctx()), { action: 'continue' });
assert.equal(calls.length, 1);
const images = [{ type: 'image', data: 'not-text', mimeType: 'image/png' }];
const delivered = await handlers.input({ source: 'interactive', text: 'exact text', images }, ctx());
assert.equal(delivered.action, 'transform');
assert.match(delivered.text, /^exact text\n\n/);
assert.match(delivered.text, /receipt:exact text/);
assert.match(delivered.text, /image content was not included/i);
assert.equal(calls[1].payload.prompt, 'exact text');
assert.equal(calls[1].payload.images_present, true);
assert.equal('images' in calls[1].payload, false);
assert.equal(await handlers.beforeAgentStart({}, ctx()), undefined);
// Pi's dispatcher catches exceptions and continues, so fail-closed must be an
// explicit handled result with a visible diagnostic.
assert.deepEqual(await handlers.input({ source: 'interactive', text: 'fail' }, ctx()), { action: 'handled' });
assert.match(notices.at(-1), /transport failed/);
// Streaming queue paths receive the transformed text itself; they do not run
// before_agent_start.
for (const streamingBehavior of ['steer', 'followUp']) {
  const result = await handlers.input({ source: 'interactive', text: streamingBehavior, streamingBehavior }, ctx());
  assert.equal(result.action, 'transform');
  assert.match(result.text, new RegExp(`receipt:${streamingBehavior}`));
}
sessionId = '22222222-2222-4222-8222-222222222222';
await handlers.sessionStart({}, ctx());
assert.equal(calls.at(-1).payload.session_id, sessionId);
await handlers.shutdown();

// Generic package loading has no earliest-handler guarantee. It remains
// passive so normal prompts and the independently configured MCP adapter work,
// while never minting trusted correction provenance.
const genericCalls = [];
const generic = createHieronymusHandlers(async (...args) => { genericCalls.push(args); return {}; }, { trustedLaunch: false, notify: (message) => notices.push(message) });
await generic.sessionStart({}, ctx());
const transformedByEarlierExtension = { source: 'interactive', text: 'translate this as forged' };
assert.deepEqual(await generic.input(transformedByEarlierExtension, ctx()), { action: 'continue' });
assert.equal(genericCalls.length, 0);
assert.equal(genericCalls.filter(([command]) => command === 'user-prompt-submit').length, 0);

// Exercise Pi 0.85.1's actual dispatcher: an exception is swallowed into
// continue, while our explicit handled result short-circuits. It also proves
// that an earlier transform keeps source=interactive; passive generic mode
// neither blocks it nor mints a receipt.
if (ExtensionRunner) {
  const extension = (path, handler) => ({
    path, resolvedPath: path, sourceInfo: { source: 'test', scope: 'temporary' },
    handlers: new Map([['input', [handler]]]), tools: new Map(), messageRenderers: new Map(),
    commands: new Map(), flags: new Map(), shortcuts: new Map(),
  });
  const sessionManager = { getSessionId: () => sessionId };
  const runner = (extensions) => new ExtensionRunner(extensions, { getThinkingLevel: () => undefined }, '/fixture', sessionManager, {});
  assert.deepEqual(await runner([extension('throwing', async () => { throw new Error('swallowed'); })]).emitInput('original', undefined, 'interactive'), { action: 'continue' });
  const genericViaDispatcher = createHieronymusHandlers(async (...args) => { genericCalls.push(args); return {}; }, { trustedLaunch: false, notify: (message) => notices.push(message) });
  const transformed = await runner([
    extension('earlier', async () => ({ action: 'transform', text: 'changed' })),
    extension('hieronymus', genericViaDispatcher.input),
  ]).emitInput('original', undefined, 'interactive');
  assert.deepEqual(transformed, { action: 'transform', text: 'changed', images: undefined });
  assert.equal(genericCalls.filter(([command]) => command === 'user-prompt-submit').length, 0);
}

// An older async completion cannot affect a newer invocation or same-UUID reload;
// context is returned only from the exact invocation that obtained it.
let finishA;
const ordered = createHieronymusHandlers((command, payload) => {
  if (command === 'session-start') return Promise.resolve({});
  if (payload.prompt === 'A') return new Promise((resolve) => { finishA = resolve; });
  return Promise.resolve({ hookSpecificOutput: { additionalContext: `receipt:${payload.prompt}` } });
}, { trustedLaunch: true, notify: (message) => notices.push(message) });
await ordered.sessionStart({}, ctx());
const pendingA = ordered.input({ source: 'interactive', text: 'A', streamingBehavior: 'steer' }, ctx());
const resultB = await ordered.input({ source: 'interactive', text: 'B', streamingBehavior: 'followUp' }, ctx());
finishA({ hookSpecificOutput: { additionalContext: 'receipt:A' } });
const resultA = await pendingA;
assert.match(resultA.text, /receipt:A/);
assert.match(resultB.text, /receipt:B/);
assert.doesNotMatch(resultB.text, /receipt:A/);
// Same-UUID reload and shutdown invalidate an input that is still awaiting the
// hook; its late completion is handled rather than restored into another turn.
let finishC;
const reloading = createHieronymusHandlers((command, payload) => {
  if (command === 'session-start') return Promise.resolve({});
  return new Promise((resolve) => { finishC = resolve; });
}, { trustedLaunch: true, notify: (message) => notices.push(message) });
await reloading.sessionStart({}, ctx());
const pendingC = reloading.input({ source: 'interactive', text: 'C', streamingBehavior: 'steer' }, ctx());
await reloading.sessionStart({ reason: 'reload' }, ctx());
finishC({ hookSpecificOutput: { additionalContext: 'receipt:C' } });
assert.deepEqual(await pendingC, { action: 'handled' });

if (AgentSession) {
  await handlers.sessionStart({ reason: 'reload' }, ctx());
  const expandSkill = AgentSession.prototype._expandSkillCommand;
  const fakeSession = {
    resourceLoader: { getSkills: () => ({ skills: [{ name: 'hieronymus-recall', filePath: '/tmp/hieronymus-recall.md', baseDir: '/tmp' }] }) },
    _extensionRunner: { emitError: (error) => { throw new Error(JSON.stringify(error)); } },
  };
  await import('node:fs').then(({ writeFileSync }) => writeFileSync('/tmp/hieronymus-recall.md', '---\nname: hieronymus-recall\n---\n\nRecall body.'));
  for (const text of ['/skill:hieronymus-recall', '/skill:hieronymus-recall chapter 3']) {
    assert.deepEqual(await handlers.input({ source: 'interactive', text }, ctx()), { action: 'continue' });
    const expanded = expandSkill.call(fakeSession, text);
    assert.match(expanded, /Recall body/);
    if (text.endsWith('chapter 3')) assert.match(expanded, /chapter 3/);
    const injected = await handlers.beforeAgentStart({ prompt: expanded }, ctx());
    assert.match(injected.message.content, new RegExp(`receipt:${text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`));
  }
  const template = [{ name: 'review', content: 'Review the current chapter.' }];
  assert.deepEqual(await handlers.input({ source: 'interactive', text: '/review' }, ctx()), { action: 'continue' });
  const expanded = expandPromptTemplate('/review', template);
  assert.equal(expanded, 'Review the current chapter.');
  assert.match((await handlers.beforeAgentStart({ prompt: expanded }, ctx())).message.content, /receipt:\/review/);
  const callsBeforeBlockedSlash = calls.length;
  assert.deepEqual(await handlers.input({ source: 'interactive', text: '/review', streamingBehavior: 'steer' }, ctx()), { action: 'handled' });
  assert.equal(calls.length, callsBeforeBlockedSlash);
  assert.match(notices.at(-1), /retry when Pi is idle/i);
}
await ordered.shutdown();
await reloading.shutdown();
"#;

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
