use std::io::Write;
use std::path::Path;

/// Write `contents` to `path` atomically: create parent directories, write a
/// sibling temporary file, fsync it, then rename it over the destination.
/// A crash never leaves a half-written authoritative file.
pub fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let mut temporary = tempfile::Builder::new()
        .prefix(&format!(".{name}."))
        .suffix(".tmp")
        .rand_bytes(8)
        .tempfile_in(path.parent().unwrap_or(Path::new(".")))?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Convenience wrapper for UTF-8 text payloads.
pub fn atomic_write_text(path: &Path, text: &str) -> std::io::Result<()> {
    atomic_write(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_existing_file_and_cleans_temporaries() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("release.conf");
        std::fs::write(&path, "old").unwrap();

        atomic_write_text(&path, "[updates]\nchannel = \"dev\"\n").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[updates]\nchannel = \"dev\"\n"
        );
        let leftovers: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["release.conf".to_string()]);
    }

    #[test]
    fn atomic_write_creates_missing_parents() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested").join("dir").join("file.conf");

        atomic_write_text(&path, "value = 1\n").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "value = 1\n");
    }
}
