use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

use serde_json::{Map, Value};
use tempfile::{Builder, NamedTempFile};

use super::AgentError;

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
    atomic_write_text_with(path, contents, &SystemAtomicFileOperations)
}

struct PersistFailure {
    temporary: NamedTempFile,
    source: std::io::Error,
}

trait AtomicFileOperations {
    fn persist(&self, temporary: NamedTempFile, path: &Path) -> Result<File, PersistFailure>;
    fn sync_parent(&self, parent: &Path) -> std::io::Result<()>;
}

struct SystemAtomicFileOperations;

impl AtomicFileOperations for SystemAtomicFileOperations {
    fn persist(&self, temporary: NamedTempFile, path: &Path) -> Result<File, PersistFailure> {
        temporary.persist(path).map_err(|error| PersistFailure {
            temporary: error.file,
            source: error.error,
        })
    }

    fn sync_parent(&self, parent: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            File::open(parent)?.sync_all()
        }
        #[cfg(not(unix))]
        {
            let _ = parent;
            Ok(())
        }
    }
}

fn atomic_write_text_with(
    path: &Path,
    contents: &str,
    operations: &impl AtomicFileOperations,
) -> Result<(), AgentError> {
    let parent = path
        .parent()
        .ok_or_else(|| AgentError::MissingParent(path.to_path_buf()))?;
    fs::create_dir_all(parent).map_err(|source| AgentError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temporary = Builder::new()
        .prefix(".hiero-config-")
        .tempfile_in(parent)
        .map_err(|source| AgentError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|source| AgentError::Io {
                path: temporary.path().to_path_buf(),
                source,
            })?;
    }
    temporary
        .write_all(contents.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| AgentError::Io {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    let _persisted = operations.persist(temporary, path).map_err(|failure| {
        let temporary_path = failure.temporary.path().to_path_buf();
        drop(failure.temporary);
        AgentError::Io {
            path: temporary_path,
            source: failure.source,
        }
    })?;
    operations
        .sync_parent(parent)
        .map_err(|source| AgentError::CommittedDurability {
            path: path.to_path_buf(),
            source,
        })
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

#[cfg(test)]
mod tests {
    use std::{fs::File, io, path::Path};

    use tempfile::NamedTempFile;

    use super::{AtomicFileOperations, PersistFailure, atomic_write_text_with};
    use crate::agent::AgentError;

    struct FailingOperations {
        fail_persist: bool,
        fail_sync: bool,
    }

    impl AtomicFileOperations for FailingOperations {
        fn persist(&self, temporary: NamedTempFile, path: &Path) -> Result<File, PersistFailure> {
            if self.fail_persist {
                return Err(PersistFailure {
                    temporary,
                    source: io::Error::other("injected persist failure"),
                });
            }
            temporary.persist(path).map_err(|error| PersistFailure {
                temporary: error.file,
                source: error.error,
            })
        }

        fn sync_parent(&self, _parent: &Path) -> io::Result<()> {
            if self.fail_sync {
                Err(io::Error::other("injected directory sync failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn pre_commit_replace_failure_preserves_the_original() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        std::fs::write(&path, "original").unwrap();

        let error = atomic_write_text_with(
            &path,
            "replacement",
            &FailingOperations {
                fail_persist: true,
                fail_sync: false,
            },
        )
        .unwrap_err();

        assert!(matches!(error, AgentError::Io { .. }));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
    }

    #[test]
    fn post_commit_sync_failure_reports_that_replacement_committed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        std::fs::write(&path, "original").unwrap();

        let error = atomic_write_text_with(
            &path,
            "replacement",
            &FailingOperations {
                fail_persist: false,
                fail_sync: true,
            },
        )
        .unwrap_err();

        assert!(
            matches!(error, AgentError::CommittedDurability { path: ref actual, .. } if actual == &path)
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "replacement");
    }
}
