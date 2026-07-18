use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use hiero_core::agent::{
    AgentPlugin, ProjectAgentContext, agent_plugins,
    claude::ClaudePlugin,
    codex::CodexPlugin,
    config_patch::{load_json_object, load_toml_object, patch_json_config, patch_toml_config},
    context::discover_context,
    gemini::GeminiPlugin,
    install_skills, resolve_plugin, skill_assets, uninstall_skills,
};
use serde_json::json;

static ID: AtomicU64 = AtomicU64::new(0);

fn workspace(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "hiero-agent-{label}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn context_discovery_walks_up_and_applies_defaults() {
    let root = workspace("context");
    let chapter = root.join("volume/chapter");
    fs::create_dir_all(&chapter).unwrap();
    fs::write(
        root.join(".hieronymus.json"),
        r#"{"series_slug":"oso","volume":"1"}"#,
    )
    .unwrap();

    let context = discover_context(&chapter).expect("context should be discovered");

    assert_eq!(context.series_slug, "oso");
    assert_eq!(context.source_language, "ja");
    assert_eq!(context.target_language, "en");
    assert_eq!(context.task_type, "translation");
    assert_eq!(context.volume, "1");
    assert_eq!(context.chapter, "");
    assert_eq!(context.workspace, root);
}

#[test]
fn config_patchers_preserve_unrelated_keys_and_reject_malformed_input_without_mutation() {
    let root = workspace("patch");
    let json_path = root.join("settings.json");
    fs::write(&json_path, "{\n  \"theme\": \"dark\"\n}\n").unwrap();
    patch_json_config(&json_path, |object| {
        object.insert(
            "mcpServers".into(),
            json!({"hieronymus": {"url": "http://127.0.0.1:9768/mcp"}}),
        );
    })
    .unwrap();
    let json = load_json_object(&json_path).unwrap();
    assert_eq!(json["theme"], "dark");
    assert_eq!(
        json["mcpServers"]["hieronymus"]["url"],
        "http://127.0.0.1:9768/mcp"
    );

    let toml_path = root.join("config.toml");
    fs::write(&toml_path, "model = \"gpt-5\"\n").unwrap();
    patch_toml_config(&toml_path, |table| {
        table.insert("mcp_servers".into(), toml::Value::Table(toml::Table::new()));
    })
    .unwrap();
    assert_eq!(
        load_toml_object(&toml_path).unwrap()["model"].as_str(),
        Some("gpt-5")
    );

    fs::write(&toml_path, "[broken\n").unwrap();
    let original_toml = fs::read(&toml_path).unwrap();
    assert!(patch_toml_config(&toml_path, |_| {}).is_err());
    assert_eq!(fs::read(&toml_path).unwrap(), original_toml);

    fs::write(&json_path, "{ broken").unwrap();
    let original = fs::read(&json_path).unwrap();
    assert!(patch_json_config(&json_path, |_| {}).is_err());
    assert_eq!(fs::read(&json_path).unwrap(), original);
}

#[test]
fn plugin_discovery_is_complete_and_resolves_case_insensitively() {
    assert_eq!(
        agent_plugins()
            .iter()
            .map(|plugin| plugin.name())
            .collect::<Vec<_>>(),
        ["claude", "codex", "gemini", "opencode", "openclaw"]
    );
    assert_eq!(resolve_plugin("CoDeX").unwrap().name(), "codex");
    assert!(resolve_plugin("unknown").is_none());
}

#[test]
fn http_plugins_patch_url_configs_idempotently_and_honor_dry_run() {
    let root = workspace("http-plugin");
    let context = ProjectAgentContext::for_workspace(&root, "oso");
    let claude_path = root.join(".claude.json");
    fs::write(&claude_path, r#"{"theme":"dark"}"#).unwrap();
    let claude = ClaudePlugin::at(claude_path.clone());
    let plan = claude.install_plan(&context).unwrap();

    assert!(!claude.detect().installed);
    claude.apply(&plan, true).unwrap();
    assert_eq!(
        fs::read_to_string(&claude_path).unwrap(),
        r#"{"theme":"dark"}"#
    );
    claude.apply(&plan, false).unwrap();
    claude.apply(&plan, false).unwrap();
    assert!(claude.detect().installed);

    let payload = load_json_object(&claude_path).unwrap();
    assert_eq!(payload["theme"], "dark");
    assert_eq!(
        payload["mcpServers"]["hieronymus"],
        json!({"type":"http", "url":"http://127.0.0.1:9768/mcp"})
    );

    let codex_path = root.join("config.toml");
    fs::write(&codex_path, "model = \"gpt-5\"\n").unwrap();
    let codex = CodexPlugin::at(codex_path.clone());
    let plan = codex.install_plan(&context).unwrap();
    codex.apply(&plan, false).unwrap();
    let payload = load_toml_object(&codex_path).unwrap();
    assert_eq!(payload["model"].as_str(), Some("gpt-5"));
    assert_eq!(
        payload["mcp_servers"]["hieronymus"]["url"].as_str(),
        Some("http://127.0.0.1:9768/mcp")
    );
}

#[test]
fn command_plugin_rejects_malformed_section_without_mutation() {
    let root = workspace("command-plugin");
    let context = ProjectAgentContext::for_workspace(&root, "oso");
    let path = root.join("settings.json");
    fs::write(&path, r#"{"mcpServers":[],"theme":"dark"}"#).unwrap();
    let original = fs::read(&path).unwrap();
    let plugin = GeminiPlugin::at(path.clone());
    let plan = plugin.install_plan(&context).unwrap();

    assert!(plugin.apply(&plan, false).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn command_plugin_writes_the_main_binary_mcp_shim() {
    let root = workspace("command-plugin-valid");
    let context = ProjectAgentContext::for_workspace(&root, "oso");
    let path = root.join("settings.json");
    fs::write(&path, r#"{"theme":"dark"}"#).unwrap();
    let plugin = GeminiPlugin::at(path.clone());

    assert!(!plugin.detect().installed);
    plugin
        .apply(&plugin.install_plan(&context).unwrap(), false)
        .unwrap();
    assert!(plugin.detect().installed);

    let payload = load_json_object(&path).unwrap();
    assert_eq!(
        payload["mcpServers"]["hieronymus"],
        json!({"command":"hiero", "args":["mcp"]})
    );
    assert_eq!(payload["theme"], "dark");
}

#[test]
fn skill_install_is_idempotent_and_uninstall_removes_only_owned_directories() {
    let root = workspace("skills");
    assert_eq!(skill_assets().len(), 8);

    let dry_run = install_skills(&root, &["agents".into(), "claude".into()], true).unwrap();
    assert!(!dry_run.installed.is_empty());
    assert!(!root.join(".agents").exists());

    install_skills(&root, &["agents".into()], false).unwrap();
    let owned = root.join(".agents/skills/hieronymus-read");
    fs::write(owned.join("stale.txt"), "stale").unwrap();
    let custom = root.join(".agents/skills/custom/SKILL.md");
    fs::create_dir_all(custom.parent().unwrap()).unwrap();
    fs::write(&custom, "custom").unwrap();
    install_skills(&root, &["agents".into()], false).unwrap();
    assert!(!owned.join("stale.txt").exists());

    let removed = uninstall_skills(&root, &["agents".into()], false).unwrap();
    assert!(
        removed
            .installed
            .iter()
            .any(|path| path.ends_with("hieronymus-read"))
    );
    assert_eq!(fs::read_to_string(custom).unwrap(), "custom");
}

#[test]
fn skill_install_rejects_unknown_targets_before_writing() {
    let root = workspace("skill-target");
    assert!(install_skills(&root, &["codex".into()], false).is_err());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn skill_install_rejects_symlinked_target_roots() {
    use std::os::unix::fs::symlink;

    let root = workspace("skill-symlink");
    let outside = workspace("skill-symlink-outside");
    fs::create_dir_all(root.join(".agents")).unwrap();
    symlink(&outside, root.join(".agents/skills")).unwrap();

    assert!(install_skills(&root, &["agents".into()], false).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn atomic_config_writes_are_private() {
    use std::os::unix::fs::PermissionsExt;

    let root = workspace("permissions");
    let path = root.join("settings.json");
    patch_json_config(&path, |_| {}).unwrap();
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
