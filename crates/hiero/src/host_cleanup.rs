//! Remove only registrations for the installed Hieronymus plugin, preserving host settings.
use std::path::{Path, PathBuf};
use toml_edit::DocumentMut;

const PLUGIN: &str = "hieronymus@hieronymus-local";

pub(crate) struct Cleanup {
    files: Vec<(PathBuf, String, String)>,
    cache: Option<PathBuf>,
}

fn regular_text(path: &Path) -> Result<Option<String>, String> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
        Ok(metadata) if !metadata.is_file() || metadata.len() > 4 * 1024 * 1024 => Err(format!(
            "refusing nonregular or oversized host config: {}",
            path.display()
        )),
        Ok(_) => std::fs::read_to_string(path)
            .map(Some)
            .map_err(|e| e.to_string()),
    }
}

fn remove_legacy(document: &mut DocumentMut, data_root: &Path) {
    let owned_marketplace = document
        .get("marketplaces")
        .and_then(|i| i.get("hieronymus-local"))
        .and_then(|i| i.get("source"))
        .and_then(|i| i.as_str())
        .is_some_and(|source| Path::new(source) == data_root.join("agent-plugins"));
    if let Some(plugins) = document.get_mut("plugins").and_then(|i| i.as_table_mut()) {
        let generated = plugins
            .get("hieronymus")
            .and_then(|i| i.get("path"))
            .and_then(|i| i.as_str())
            .is_some_and(|p| {
                Path::new(p) == data_root.join("agent-plugins/codex")
                    || (owned_marketplace
                        && p.replace('\\', "/").ends_with("/agent-plugins/codex")
                        && !Path::new(p).exists())
            });
        if generated {
            plugins.remove("hieronymus");
        }
    }
}

pub(crate) fn migrate_legacy(home: &Path, data_root: &Path) -> Result<(), String> {
    let path = home.join(".codex/config.toml");
    let Some(before) = regular_text(&path)? else {
        return Ok(());
    };
    let mut document = before.parse::<DocumentMut>().map_err(|e| e.to_string())?;
    remove_legacy(&mut document, data_root);
    let after = document.to_string();
    if after != before {
        hieronymus::atomic::atomic_write_text(&path, &after).map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl Cleanup {
    pub(crate) fn prepare(home: &Path, data_root: &Path) -> Result<Self, String> {
        let mut files = Vec::new();
        let path = home.join(".codex/config.toml");
        let mut owned_marketplace = false;
        if let Some(before) = regular_text(&path)? {
            let mut document = before.parse::<DocumentMut>().map_err(|e| e.to_string())?;
            remove_legacy(&mut document, data_root);
            owned_marketplace = document
                .get("marketplaces")
                .and_then(|i| i.get("hieronymus-local"))
                .and_then(|i| i.get("source"))
                .and_then(|i| i.as_str())
                .is_some_and(|source| Path::new(source) == data_root.join("agent-plugins"));
            if owned_marketplace {
                if let Some(table) = document
                    .get_mut("marketplaces")
                    .and_then(|i| i.as_table_mut())
                {
                    table.remove("hieronymus-local");
                }
                if let Some(table) = document.get_mut("plugins").and_then(|i| i.as_table_mut()) {
                    table.remove(PLUGIN);
                }
                if let Some(table) = document
                    .get_mut("hooks")
                    .and_then(|i| i.get_mut("state"))
                    .and_then(|i| i.as_table_mut())
                {
                    let keys: Vec<String> = table
                        .iter()
                        .filter(|(key, _)| key.starts_with(&format!("{PLUGIN}:")))
                        .map(|(key, _)| key.to_owned())
                        .collect();
                    for key in keys {
                        table.remove(&key);
                    }
                }
            }
            if let Some(table) = document
                .get_mut("mcp_servers")
                .and_then(|i| i.as_table_mut())
            {
                for name in ["hieronymus", "hiero"] {
                    let owned = table.get(name).is_some_and(|entry| {
                        let command = entry.get("command").and_then(|i| i.as_str()).unwrap_or("");
                        let args: Vec<&str> = entry
                            .get("args")
                            .and_then(|i| i.as_array())
                            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                            .unwrap_or_default();
                        let root = args
                            .windows(2)
                            .rfind(|pair| pair[0] == "--data-root")
                            .map(|pair| PathBuf::from(pair[1]))
                            .or_else(|| {
                                entry
                                    .get("env")
                                    .and_then(|i| i.get("HIERONYMUS_DATA_ROOT"))
                                    .and_then(|i| i.as_str())
                                    .map(PathBuf::from)
                            });
                        // A rootless/relative entry may be launched with another host environment
                        // or working directory. Preserve it unless its recorded root proves ownership.
                        let owned_root = root.is_some_and(|root| {
                            let root = hieronymus::data_root::load_config(Some(&root))
                                .data_root()
                                .to_owned();
                            root.is_absolute() && root == data_root
                        });
                        owned_root
                            && (command == "hieronymus-mcp"
                                || command == "hiero"
                                    && args.first().is_some_and(|a| matches!(*a, "mcp" | "stdio")))
                    });
                    if owned {
                        table.remove(name);
                    }
                }
            }
            let after = document.to_string();
            if after != before {
                files.push((path, before, after));
            }
        }
        let cache = owned_marketplace
            .then(|| home.join(".codex/plugins/cache/hieronymus-local/hieronymus"));
        if let Some(cache) = &cache {
            match cache.symlink_metadata() {
                Ok(metadata) if !metadata.is_dir() => {
                    return Err("refusing non-directory Hieronymus plugin cache".into());
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(Self { files, cache })
    }
    pub(crate) fn apply(self) -> Result<Vec<String>, String> {
        let mut removed = Vec::new();
        for (path, before, after) in self.files {
            if regular_text(&path)?.as_deref() != Some(&before) {
                return Err("host configuration changed during uninstall; retry".into());
            }
            hieronymus::atomic::atomic_write_text(&path, &after).map_err(|e| e.to_string())?;
            removed.push(format!("owned host registrations: {}", path.display()));
        }
        if let Some(cache) = self.cache {
            match std::fs::remove_dir_all(&cache) {
                Ok(()) => removed.push(format!("owned plugin cache: {}", cache.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_preserves_foreign_settings_and_is_repeatable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let path = temp.path().join(".codex/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!(
                r#"model = "keep"
[marketplaces.hieronymus-local]
source = "{}/agent-plugins"
[plugins."hieronymus@hieronymus-local"]
enabled = true
[plugins.hieronymus]
path = "/old/hieronymus/agent-plugins/codex"
[hooks.state."hieronymus@hieronymus-local:hooks/hooks.codex.json:x"]
trusted_hash = "old"
[hooks.state.foreign]
trusted_hash = "keep"
[mcp_servers.hieronymus]
command = "hieronymus-mcp"
args = ["--data-root", "{}/data"]
[mcp_servers.foreign]
command = "keep"
"#,
                root.display(),
                temp.path().display()
            ),
        )
        .unwrap();
        let cache = temp
            .path()
            .join(".codex/plugins/cache/hieronymus-local/hieronymus");
        std::fs::create_dir_all(&cache).unwrap();
        Cleanup::prepare(temp.path(), &root)
            .unwrap()
            .apply()
            .unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(!after.contains("hieronymus"));
        assert!(after.contains("foreign") && after.contains("model = \"keep\""));
        assert!(!cache.exists());
        Cleanup::prepare(temp.path(), &root)
            .unwrap()
            .apply()
            .unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), after);
    }
    #[test]
    fn rootless_mcp_registration_is_preserved_for_an_explicit_uninstall_root() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".codex/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let contents = "[mcp_servers.hieronymus]\ncommand = \"hiero\"\nargs = [\"stdio\"]\n";
        std::fs::write(&path, contents).unwrap();
        // This matches the process environment's root but is not recorded in the entry.
        let root = hieronymus::data_root::load_config(None);
        Cleanup::prepare(home.path(), root.data_root())
            .unwrap()
            .apply()
            .unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), contents);
    }
    #[test]
    fn another_installations_mcp_and_legacy_registration_are_preserved() {
        let home = tempfile::tempdir().unwrap();
        let foreign = home.path().join("other/agent-plugins/codex");
        std::fs::create_dir_all(&foreign).unwrap();
        let path = home.path().join(".codex/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let contents = format!(
            r#"[plugins.hieronymus]
path = "{}"
[mcp_servers.hieronymus]
command = "hiero"
args = ["stdio", "--data-root", "{}", "--data-root", "/other-install"]
"#,
            foreign.display(),
            home.path().join("ours").display()
        );
        std::fs::write(&path, &contents).unwrap();
        Cleanup::prepare(home.path(), &home.path().join("ours"))
            .unwrap()
            .apply()
            .unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), contents);
    }
}
