//! Installation-owned bundle refresh and explicit native host-cache synchronization.
use hieronymus::data_root::HieronymusConfig;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, Serialize)]
pub struct BundleStatus {
    pub state: &'static str,
    pub version: Option<String>,
    pub changed_files: usize,
    pub hook_commands_changed: bool,
}

fn read_small(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
        Ok(m) if !m.is_file() || m.len() > 1024 * 1024 => Err(format!(
            "nonregular or oversized plugin file: {}",
            path.display()
        )),
        Ok(_) => std::fs::read(path).map(Some).map_err(|e| e.to_string()),
    }
}

/// Compare contents, not only versions: development builds may share a version.
pub fn bundle_status(
    config: &HieronymusConfig,
    target: &str,
    root: &Path,
) -> Result<BundleStatus, String> {
    let expected_root = config.agent_plugins_root().join(target);
    let rendered = crate::agent_plugins::render(config)?;
    let mut changed = 0;
    let mut hooks_changed = false;
    for (path, expected) in rendered {
        let Ok(relative) = path.strip_prefix(&expected_root) else {
            continue;
        };
        let current = read_small(&root.join(relative))?;
        if current.as_deref() != Some(expected.as_bytes()) {
            changed += 1;
            hooks_changed |= relative.starts_with("hooks");
        }
    }
    let manifest = match target {
        "codex" => ".codex-plugin/plugin.json",
        "claude" => ".claude-plugin/plugin.json",
        "pi" => "package.json",
        "gemini" => "gemini-extension.json",
        "opencode" => "opencode/plugin.json",
        "openclaw" => "openclaw/plugin.json",
        _ => return Err("unsupported cache target".into()),
    };
    let version = read_small(&root.join(manifest))?
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["version"].as_str().map(str::to_owned));
    Ok(BundleStatus {
        state: if !root.exists() {
            "missing"
        } else if changed == 0 {
            "current"
        } else {
            "stale"
        },
        version,
        changed_files: changed,
        hook_commands_changed: hooks_changed,
    })
}

/// Cheap lazy refresh, using the generator's atomic per-file replacements.
/// No native host process or configuration is touched on daemon startup.
pub fn ensure_current(config: &HieronymusConfig) -> Result<bool, String> {
    let files = crate::agent_plugins::render(config)?;
    for (path, expected) in files {
        if read_small(&path)?.as_deref() != Some(expected.as_bytes()) {
            crate::agent_plugins::generate(config)?;
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home::home_dir().map(|p| p.join(".codex")))
}

/// Cached versions are disk inventory, never proof of a running chat's catalog.
pub fn inspect(config: &HieronymusConfig, host_home: Option<&Path>) -> Value {
    let generated: Vec<_> = ["codex", "claude", "gemini", "opencode", "openclaw", "pi"]
        .into_iter()
        .map(
            |host| match bundle_status(config, host, &config.agent_plugins_root().join(host)) {
                Ok(status) => json!({"host":host,"bundle":status}),
                Err(error) => json!({"host":host,"error":error}),
            },
        )
        .collect();
    let caches = host_home
        .map(|p| cache_inventory(config, p))
        .unwrap_or_else(|| json!({"state":"unavailable"}));
    json!({"executable_version":env!("CARGO_PKG_VERSION"),"generated":generated,"codex_cache":caches,
        "other_hosts":"Claude/zCode: use the native plugin update/reload flow. Pi: reload the installed local package. See docs/agent-workflows.md.",
        "reload":"Reopen existing conversations after refresh; disk state does not prove the running skill catalog changed."})
}

fn cache_inventory(config: &HieronymusConfig, host_home: &Path) -> Value {
    let result = (|| -> Result<Value, String> {
        let Some(bytes) = read_small(&host_home.join("config.toml"))? else {
            return Ok(json!({"state":"not_registered"}));
        };
        let doc = std::str::from_utf8(&bytes)
            .map_err(|_| "invalid Codex config")?
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| "invalid Codex config")?;
        let source = doc
            .get("marketplaces")
            .and_then(|v| v.get("hieronymus-local"))
            .and_then(|v| v.get("source"))
            .and_then(|v| v.as_str());
        if source.map(Path::new) != Some(config.agent_plugins_root().as_path()) {
            return Ok(
                json!({"state":"unverified_registration","detail":"No matching local marketplace in this Codex config; native list during sync is authoritative."}),
            );
        }
        let cache = host_home.join("plugins/cache/hieronymus-local/hieronymus");
        let entries = match std::fs::read_dir(&cache) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(json!({"state":"missing","versions":[]}));
            }
            Err(e) => return Err(e.to_string()),
        };
        let mut versions = Vec::new();
        for entry in entries.take(65) {
            if versions.len() == 64 {
                return Err("too many cached plugin versions".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                return Err("non-directory cache entry".into());
            }
            let mut version = json!({"directory_version":entry.file_name().to_string_lossy()});
            match bundle_status(config, "codex", &entry.path()) {
                Ok(bundle) => version["bundle"] = json!(bundle),
                Err(error) => version["error"] = json!(error),
            }
            versions.push(version);
        }
        versions.sort_by_key(|v| v["directory_version"].as_str().unwrap_or("").to_owned());
        Ok(json!({"state":"inventory","versions":versions,"active_version":"not_observed"}))
    })();
    result.unwrap_or_else(|error| json!({"state":"error","error":error}))
}

/// Synchronize only an already installed, enabled Codex plugin belonging to this root.
/// Missing/unsupported hosts are warnings, never a reason to fail an application update.
pub fn sync(config: &HieronymusConfig) -> Result<Value, String> {
    crate::agent_plugins::generate(config)?;
    let mut codex = sync_codex(config, |args| {
        native_json(config, "codex", args, Duration::from_secs(30))
    });
    let agents = inspect(config, codex_home().as_deref());
    verify_cache_refresh(&mut codex, &agents);
    Ok(json!({"generated":true,"codex":codex,"agents":agents,
        "notice":"Reopen conversations. Changed hook commands require native trust review; unchanged trust hashes are not modified."}))
}
fn verify_cache_refresh(codex: &mut Value, agents: &Value) {
    if codex["state"] != "refreshed" {
        return;
    }
    let current = cache_is_current(agents);
    if !current {
        codex["state"] = json!("warning");
        codex["detail"] = json!(
            "Native metadata refreshed, but current cache contents could not be verified; inspect plugins status and the native host."
        );
    }
}

/// Historical cache versions are inventory, not a reason to repeat refresh forever.
pub(crate) fn cache_is_current(agents: &Value) -> bool {
    agents["codex_cache"]["versions"]
        .as_array()
        .is_some_and(|versions| {
            versions.iter().any(|v| {
                v["directory_version"] == env!("CARGO_PKG_VERSION")
                    && v["bundle"]["state"] == "current"
            })
        })
}

fn sync_codex(
    config: &HieronymusConfig,
    mut run: impl FnMut(&[&str]) -> Result<Value, String>,
) -> Value {
    let result = (|| -> Result<Value, String> {
        let listed = run(&[
            "plugin",
            "list",
            "--marketplace",
            "hieronymus-local",
            "--json",
        ])?;
        let plugins = listed["installed"]
            .as_array()
            .ok_or("unsupported native plugin-list response")?;
        let Some(plugin) = plugins
            .iter()
            .find(|p| p["pluginId"] == "hieronymus@hieronymus-local")
        else {
            return Ok(
                json!({"state":"not_installed","action":"Install through the native host when desired; sync does not enroll new hosts."}),
            );
        };
        if plugin["enabled"] != true {
            return Ok(
                json!({"state":"disabled","action":"Enable explicitly in the host if desired."}),
            );
        }
        let source = plugin["source"]["path"].as_str().map(Path::new);
        if plugin["source"]["source"] != "local"
            || source != Some(config.agent_plugins_root().join("codex").as_path())
        {
            return Err(
                "Codex plugin belongs to a different source; registration left unchanged".into(),
            );
        }
        run(&["plugin", "add", "hieronymus@hieronymus-local", "--json"])?;
        let after = run(&[
            "plugin",
            "list",
            "--marketplace",
            "hieronymus-local",
            "--json",
        ])?;
        let refreshed = after["installed"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|p| p["pluginId"] == "hieronymus@hieronymus-local")
            })
            .ok_or("plugin missing after native refresh")?;
        if refreshed["version"] != env!("CARGO_PKG_VERSION")
            || refreshed["enabled"] != true
            || refreshed["source"] != plugin["source"]
        {
            return Err("native refresh did not confirm the current enabled plugin".into());
        }
        Ok(
            json!({"state":"refreshed","previous_version":plugin["version"],"version":refreshed["version"],"reload_required":true}),
        )
    })();
    result.unwrap_or_else(|error|json!({"state":"warning","detail":error,"action":"Check the native CLI and private logs/agent-sync.log; application remains usable."}))
}

/// Candidate bundle refresh shares bounded capture and redacted private diagnostics.
pub(crate) fn sync_candidate(config: &HieronymusConfig, binary: &Path) -> Result<Value, String> {
    native_json(
        config,
        binary.to_str().ok_or("candidate path is not UTF-8")?,
        &[
            "plugins",
            "sync",
            "--json",
            "--data-root",
            config
                .data_root()
                .to_str()
                .ok_or("data root is not UTF-8")?,
        ],
        Duration::from_secs(90),
    )
}

fn native_json(
    config: &HieronymusConfig,
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<Value, String> {
    let mut stdout = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut stderr = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().map_err(|e| e.to_string())?)
        .stderr(stderr.try_clone().map_err(|e| e.to_string())?);
    let mut child = command
        .spawn()
        .map_err(|e| format!("native host command unavailable: {e}"))?;
    let deadline = Instant::now() + timeout;
    let outcome = loop {
        let bounded_output = stdout.metadata().and_then(|out| {
            stderr
                .metadata()
                .map(|err| out.len() <= 1024 * 1024 && err.len() <= 1024 * 1024)
        });
        if !matches!(bounded_output, Ok(true)) {
            let _ = child.kill();
            let _ = child.wait();
            break Err("native host output exceeded limit".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!(
                    "native host command timed out or failed: {other:?}"
                ));
            }
        }
    };
    stdout.rewind().map_err(|e| e.to_string())?;
    stderr.rewind().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    stdout
        .take(1024 * 1024 + 1)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    stderr
        .take(64 * 1024)
        .read_to_end(&mut err)
        .map_err(|e| e.to_string())?;
    let out = String::from_utf8_lossy(&out);
    let err = String::from_utf8_lossy(&err);
    write_diagnostics(config, args, &out, &err)?;
    let status = outcome?;
    if !status.success() {
        return Err(format!("native host command exited with {status}"));
    }
    if out.len() > 1024 * 1024 {
        return Err("native host output exceeded limit".into());
    }
    serde_json::from_str(&out).map_err(|_| "native host returned invalid JSON".into())
}
fn write_diagnostics(
    config: &HieronymusConfig,
    args: &[&str],
    out: &str,
    err: &str,
) -> Result<(), String> {
    let secrets: Vec<String> = std::env::vars()
        .filter(|(k, _)| {
            let k = k.to_ascii_uppercase();
            ["TOKEN", "SECRET", "PASSWORD", "KEY"]
                .iter()
                .any(|s| k.contains(s))
        })
        .map(|(_, v)| v)
        .collect();
    let values: Vec<&str> = secrets.iter().map(String::as_str).collect();
    let redact = |text: &str| {
        hieronymus::secret::redact_values(text, &values)
            .lines()
            .map(|line| {
                let lower = line.to_ascii_lowercase();
                if [
                    "token",
                    "secret",
                    "password",
                    "authorization",
                    "grant=",
                    "api_key",
                    "api-key",
                ]
                .iter()
                .any(|key| lower.contains(key))
                {
                    "[redacted credential diagnostic]"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut log = crate::diagnostics::open(&config.config_root().join("logs"), "agent-sync.log")
        .map_err(|e| e.to_string())?;
    writeln!(
        log,
        "{} codex {}\nstdout:\n{}\nstderr:\n{}",
        chrono::Utc::now(),
        redact(&args.join(" ")),
        redact(out),
        redact(err)
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lazy_refresh_repairs_same_version_drift_and_then_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(dir.path());
        assert!(ensure_current(&config).unwrap());
        assert!(!ensure_current(&config).unwrap());
        let hook = config
            .agent_plugins_root()
            .join("codex/hooks/hooks.codex.json");
        std::fs::write(&hook, "{}").unwrap();
        let status =
            bundle_status(&config, "codex", &config.agent_plugins_root().join("codex")).unwrap();
        assert_eq!(status.state, "stale");
        assert!(status.hook_commands_changed);
        assert!(ensure_current(&config).unwrap());
        assert!(!ensure_current(&config).unwrap());
    }
    #[test]
    fn native_refresh_is_scoped_and_verified() {
        let dir = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(dir.path());
        let mut calls = 0;
        let result = sync_codex(&config, |args| {
            calls += 1;
            if calls == 2 {
                assert_eq!(
                    args,
                    ["plugin", "add", "hieronymus@hieronymus-local", "--json"]
                );
                return Ok(json!({}));
            }
            assert_eq!(
                args,
                [
                    "plugin",
                    "list",
                    "--marketplace",
                    "hieronymus-local",
                    "--json"
                ]
            );
            Ok(
                json!({"installed":[{"pluginId":"hieronymus@hieronymus-local","enabled":true,
                "version":if calls == 1 {"old"} else {env!("CARGO_PKG_VERSION")},
                "source":{"source":"local","path":config.agent_plugins_root().join("codex")}}]}),
            )
        });
        assert_eq!(calls, 3);
        assert_eq!(result["state"], "refreshed");
    }
    #[test]
    fn unavailable_and_foreign_hosts_are_warnings_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(dir.path());
        assert_eq!(
            sync_codex(&config, |_| Err("unavailable".into()))["state"],
            "warning"
        );
        let mut calls = 0;
        let result = sync_codex(&config, |_| {
            calls += 1;
            Ok(
                json!({"installed":[{"pluginId":"hieronymus@hieronymus-local","enabled":true,
                "source":{"source":"local","path":"/foreign"}}]}),
            )
        });
        assert_eq!(calls, 1);
        assert_eq!(result["state"], "warning");
    }
    #[test]
    fn current_native_metadata_does_not_hide_stale_cache() {
        let mut result = json!({"state":"refreshed"});
        verify_cache_refresh(
            &mut result,
            &json!({"codex_cache":{"versions":[{
                "directory_version":env!("CARGO_PKG_VERSION"),"bundle":{"state":"stale"}
            }]}}),
        );
        assert_eq!(result["state"], "warning");
    }

    #[test]
    fn registered_cache_reports_content_drift_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(dir.path().join("data"));
        ensure_current(&config).unwrap();
        let host = dir.path().join("host");
        std::fs::create_dir_all(&host).unwrap();
        let mut doc = toml_edit::DocumentMut::new();
        doc["marketplaces"]["hieronymus-local"]["source"] =
            toml_edit::value(config.agent_plugins_root().to_str().unwrap());
        std::fs::write(host.join("config.toml"), doc.to_string()).unwrap();
        let cache = host
            .join("plugins/cache/hieronymus-local/hieronymus")
            .join(env!("CARGO_PKG_VERSION"));
        for (path, content) in crate::agent_plugins::render(&config).unwrap() {
            if let Ok(relative) = path.strip_prefix(config.agent_plugins_root().join("codex")) {
                let dest = cache.join(relative);
                std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
                std::fs::write(dest, content).unwrap();
            }
        }
        assert_eq!(
            inspect(&config, Some(&host))["codex_cache"]["versions"][0]["bundle"]["state"],
            "current"
        );
        let broken = cache.parent().unwrap().join("broken");
        std::fs::create_dir_all(broken.join(".codex-plugin/plugin.json")).unwrap();
        let inventory = inspect(&config, Some(&host));
        assert_eq!(inventory["codex_cache"]["state"], "inventory");
        assert!(cache_is_current(&inventory));
        let versions = inventory["codex_cache"]["versions"].as_array().unwrap();
        assert_eq!(versions.len(), 2);
        assert!(
            versions
                .iter()
                .any(|v| v["directory_version"] == "broken" && v["error"].is_string())
        );
        let hook = cache.join("hooks/hooks.codex.json");
        std::fs::write(&hook, "{}").unwrap();
        assert_eq!(
            inspect(&config, Some(&host))["codex_cache"]["versions"][0]["bundle"]["state"],
            "stale"
        );
        assert_eq!(std::fs::read_to_string(hook).unwrap(), "{}");
    }

    #[cfg(unix)]
    #[test]
    fn lazy_refresh_rejects_redirected_bundle_directory() {
        let dir = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(dir.path().join("data"));
        std::fs::create_dir_all(config.agent_plugins_root()).unwrap();
        let foreign = dir.path().join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        std::os::unix::fs::symlink(&foreign, config.agent_plugins_root().join("codex")).unwrap();
        assert!(ensure_current(&config).is_err());
        assert_eq!(std::fs::read_dir(foreign).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn native_process_is_bounded_and_diagnostics_are_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(dir.path());
        let response = native_json(
            &config,
            "sh",
            &["-c", "printf '{}'; printf 'token=fixture-secret\n' >&2"],
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(response, json!({}));
        let log =
            std::fs::read_to_string(config.config_root().join("logs/agent-sync.log")).unwrap();
        assert!(log.contains("[redacted credential diagnostic]"));
        assert!(!log.contains("fixture-secret"));
        assert!(
            native_json(
                &config,
                "sh",
                &["-c", "while :; do :; done"],
                Duration::from_millis(10)
            )
            .is_err()
        );
    }

    #[test]
    fn inspection_is_read_only_on_fresh_root() {
        let dir = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(dir.path().join("absent"));
        let status = inspect(&config, Some(&dir.path().join("host")));
        assert_eq!(status["generated"][0]["bundle"]["state"], "missing");
        assert_eq!(status["codex_cache"]["state"], "not_registered");
        assert!(!config.config_root().exists());
    }
}
