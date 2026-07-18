use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{Map, Value};

use super::AgentError;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

pub fn load_json_object(path: &Path) -> Result<Map<String, Value>, AgentError> {
    let text = read_optional(path)?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    serde_json::from_str::<Map<String, Value>>(&text).map_err(|source| AgentError::InvalidJson {
        path: path.to_path_buf(),
        source,
    })
}

pub fn patch_json_config(
    path: &Path,
    patch: impl FnOnce(&mut Map<String, Value>),
) -> Result<(), AgentError> {
    try_patch_json_config(path, |object| {
        patch(object);
        Ok(())
    })
}

pub(crate) fn try_patch_json_config(
    path: &Path,
    patch: impl FnOnce(&mut Map<String, Value>) -> Result<(), AgentError>,
) -> Result<(), AgentError> {
    let original = load_json_object(path)?;
    let mut updated = original.clone();
    patch(&mut updated)?;
    let contents = format!(
        "{}\n",
        serde_json::to_string_pretty(&updated).map_err(AgentError::SerializeJson)?
    );
    atomic_write_text(path, &contents)
}

pub fn load_toml_object(path: &Path) -> Result<toml::Table, AgentError> {
    let text = read_optional(path)?;
    if text.trim().is_empty() {
        return Ok(toml::Table::new());
    }
    text.parse::<toml::Table>()
        .map_err(|source| AgentError::InvalidToml {
            path: path.to_path_buf(),
            source,
        })
}

pub fn patch_toml_config(
    path: &Path,
    patch: impl FnOnce(&mut toml::Table),
) -> Result<(), AgentError> {
    try_patch_toml_config(path, |table| {
        patch(table);
        Ok(())
    })
}

pub(crate) fn try_patch_toml_config(
    path: &Path,
    patch: impl FnOnce(&mut toml::Table) -> Result<(), AgentError>,
) -> Result<(), AgentError> {
    let original = load_toml_object(path)?;
    let mut updated = original.clone();
    patch(&mut updated)?;
    let contents = toml::to_string_pretty(&updated).map_err(AgentError::SerializeToml)?;
    atomic_write_text(path, &contents)
}

pub fn atomic_write_text(path: &Path, contents: &str) -> Result<(), AgentError> {
    let parent = path
        .parent()
        .ok_or_else(|| AgentError::MissingParent(path.to_path_buf()))?;
    fs::create_dir_all(parent).map_err(|source| AgentError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let temporary = temporary_path(path);
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|source| AgentError::Io {
            path: temporary.clone(),
            source,
        })?;
        file.write_all(contents.as_bytes())
            .map_err(|source| AgentError::Io {
                path: temporary.clone(),
                source,
            })?;
        file.sync_all().map_err(|source| AgentError::Io {
            path: temporary.clone(),
            source,
        })?;
        fs::rename(&temporary, path).map_err(|source| AgentError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| AgentError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_optional(path: &Path) -> Result<String, AgentError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(AgentError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    path.with_file_name(format!(".{name}.tmp-{}-{id}", std::process::id()))
}
