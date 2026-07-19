use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use tempfile::Builder;

use crate::domain::ShortMemoryLimits;

use super::IngestError;

type Result<T> = std::result::Result<T, IngestError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LearnLimits {
    pub max_block_chars: usize,
}

impl Default for LearnLimits {
    fn default() -> Self {
        Self {
            max_block_chars: 1_200,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct IngestConfig {
    pub short_memory: ShortMemoryLimits,
    pub learn: LearnLimits,
}

impl IngestConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        validate_existing_ancestors(path)?;
        match path.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(IngestError::UnsafePath {
                    path: path.to_path_buf(),
                    reason: "configuration path is a symbolic link",
                });
            }
            Ok(_) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Self::default().validate();
            }
            Err(source) => {
                return Err(IngestError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
        validate_safe_path(path)?;
        let mut options = OpenOptions::new();
        options.read(true);
        configure_no_follow(&mut options);
        let mut file = options
            .open(path)
            .map_err(|source| map_open_error(path, source))?;
        if !file
            .metadata()
            .map_err(|source| IngestError::Io {
                path: path.to_path_buf(),
                source,
            })?
            .is_file()
        {
            return Err(IngestError::UnsafePath {
                path: path.to_path_buf(),
                reason: "configuration is not a regular file",
            });
        }
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|source| IngestError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        parse_config(&contents)?.validate()
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let config = self.validate()?;
        let path = path.as_ref();
        validate_existing_ancestors(path)?;
        if path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(IngestError::UnsafePath {
                path: path.to_path_buf(),
                reason: "configuration path is a symbolic link",
            });
        }
        let parent = path.parent().ok_or_else(|| IngestError::UnsafePath {
            path: path.to_path_buf(),
            reason: "configuration path has no parent directory",
        })?;
        fs::create_dir_all(parent).map_err(|source| IngestError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        validate_safe_directory(parent)?;
        let contents = toml::to_string_pretty(&config).map_err(IngestError::Serialize)?;
        let mut temporary = Builder::new()
            .prefix(".ingest-")
            .tempfile_in(parent)
            .map_err(|source| IngestError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|source| IngestError::Io {
                    path: temporary.path().to_path_buf(),
                    source,
                })?;
        }
        temporary
            .write_all(contents.as_bytes())
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|source| IngestError::Io {
                path: temporary.path().to_path_buf(),
                source,
            })?;

        // Recheck immediately before replacement so a swapped destination is never followed.
        validate_safe_directory(parent)?;
        if path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(IngestError::UnsafePath {
                path: path.to_path_buf(),
                reason: "configuration path is a symbolic link",
            });
        }
        temporary.persist(path).map_err(|error| IngestError::Io {
            path: path.to_path_buf(),
            source: error.error,
        })?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| IngestError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        Ok(())
    }

    pub fn validate(&self) -> Result<Self> {
        validate_minimum(
            "short_memory.warning_sentence_count",
            self.short_memory.warning_sentence_count,
            1,
        )?;
        validate_minimum(
            "short_memory.rejection_sentence_count",
            self.short_memory.rejection_sentence_count,
            1,
        )?;
        validate_minimum(
            "short_memory.warning_symbol_count",
            self.short_memory.warning_symbol_count,
            0,
        )?;
        validate_minimum(
            "short_memory.rejection_symbol_count",
            self.short_memory.rejection_symbol_count,
            0,
        )?;
        validate_minimum("learn.max_block_chars", self.learn.max_block_chars, 1)?;
        if self.short_memory.rejection_sentence_count < self.short_memory.warning_sentence_count {
            return Err(IngestError::Validation(
                "short_memory.rejection_sentence_count must be greater than or equal to short_memory.warning_sentence_count".into(),
            ));
        }
        if self.short_memory.warning_symbol_count != 0
            && self.short_memory.rejection_symbol_count != 0
            && self.short_memory.rejection_symbol_count < self.short_memory.warning_symbol_count
        {
            return Err(IngestError::Validation(
                "short_memory.rejection_symbol_count must be greater than or equal to short_memory.warning_symbol_count".into(),
            ));
        }
        Ok(*self)
    }
}

fn parse_config(contents: &str) -> Result<IngestConfig> {
    let value = contents
        .parse::<toml::Table>()
        .map_err(IngestError::InvalidToml)?;
    reject_unknown(&value, &["short_memory", "learn"], None)?;
    let defaults = IngestConfig::default();
    let short = optional_table(&value, "short_memory")?;
    reject_unknown(
        short,
        &[
            "warning_sentence_count",
            "rejection_sentence_count",
            "warning_symbol_count",
            "rejection_symbol_count",
        ],
        Some("short_memory"),
    )?;
    let learn = optional_table(&value, "learn")?;
    reject_unknown(learn, &["max_block_chars"], Some("learn"))?;
    Ok(IngestConfig {
        short_memory: ShortMemoryLimits {
            warning_sentence_count: integer_or(
                short,
                "warning_sentence_count",
                defaults.short_memory.warning_sentence_count,
                "short_memory.warning_sentence_count",
                1,
            )?,
            rejection_sentence_count: integer_or(
                short,
                "rejection_sentence_count",
                defaults.short_memory.rejection_sentence_count,
                "short_memory.rejection_sentence_count",
                1,
            )?,
            warning_symbol_count: integer_or(
                short,
                "warning_symbol_count",
                defaults.short_memory.warning_symbol_count,
                "short_memory.warning_symbol_count",
                0,
            )?,
            rejection_symbol_count: integer_or(
                short,
                "rejection_symbol_count",
                defaults.short_memory.rejection_symbol_count,
                "short_memory.rejection_symbol_count",
                0,
            )?,
        },
        learn: LearnLimits {
            max_block_chars: integer_or(
                learn,
                "max_block_chars",
                defaults.learn.max_block_chars,
                "learn.max_block_chars",
                1,
            )?,
        },
    })
}

fn optional_table<'a>(root: &'a toml::Table, key: &str) -> Result<&'a toml::Table> {
    match root.get(key) {
        None => Ok(empty_table()),
        Some(toml::Value::Table(table)) => Ok(table),
        Some(_) => Err(IngestError::Validation(format!("{key} must be a table"))),
    }
}

fn empty_table() -> &'static toml::Table {
    static EMPTY: std::sync::OnceLock<toml::Table> = std::sync::OnceLock::new();
    EMPTY.get_or_init(toml::Table::new)
}

fn integer_or(
    table: &toml::Table,
    key: &str,
    default: usize,
    field: &str,
    minimum: usize,
) -> Result<usize> {
    let Some(value) = table.get(key) else {
        return Ok(default);
    };
    let Some(value) = value.as_integer() else {
        return Err(IngestError::Validation(format!(
            "{field} must be an integer"
        )));
    };
    usize::try_from(value)
        .map_err(|_| IngestError::Validation(format!("{field} must be at least {minimum}")))
}

fn reject_unknown(table: &toml::Table, allowed: &[&str], prefix: Option<&str>) -> Result<()> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            let setting = prefix.map_or_else(|| key.clone(), |prefix| format!("{prefix}.{key}"));
            return Err(IngestError::Validation(format!(
                "unknown ingest config setting: {setting}"
            )));
        }
    }
    Ok(())
}

fn validate_minimum(field: &str, value: usize, minimum: usize) -> Result<()> {
    if value < minimum {
        Err(IngestError::Validation(format!(
            "{field} must be at least {minimum}"
        )))
    } else {
        Ok(())
    }
}

fn validate_safe_path(path: &Path) -> Result<()> {
    validate_existing_ancestors(path)?;
    let metadata = path.symlink_metadata().map_err(|source| IngestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        Err(IngestError::UnsafePath {
            path: path.to_path_buf(),
            reason: "configuration path is a symbolic link",
        })
    } else {
        Ok(())
    }
}

fn validate_existing_ancestors(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().skip(1) {
        match ancestor.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(IngestError::UnsafePath {
                    path: ancestor.to_path_buf(),
                    reason: "configuration ancestor is a symbolic link",
                });
            }
            Ok(_) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(IngestError::Io {
                    path: ancestor.to_path_buf(),
                    source,
                });
            }
        }
    }
    Ok(())
}

fn validate_safe_directory(path: &Path) -> Result<()> {
    validate_existing_ancestors(path)?;
    let metadata = path.symlink_metadata().map_err(|source| IngestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        Err(IngestError::UnsafePath {
            path: path.to_path_buf(),
            reason: "configuration parent is not a real directory",
        })
    } else {
        Ok(())
    }
}

fn map_open_error(path: &Path, source: std::io::Error) -> IngestError {
    if source.raw_os_error() == Some(libc::ELOOP) {
        IngestError::UnsafePath {
            path: path.to_path_buf(),
            reason: "configuration path is a symbolic link",
        }
    } else {
        IngestError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

#[cfg(unix)]
fn configure_no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
}

#[cfg(windows)]
fn configure_no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn configure_no_follow(_options: &mut OpenOptions) {}
