//! Package snapshots require settled native registration, independent of readback.
use std::path::PathBuf;

/// Caller retains the native continuation gates while checking and capturing.
/// Even absent native definitions may have a durable recovery journal. Refuse
/// any entry at a recognized journal path; passive readback must not recover it.
pub(crate) fn require_settled(journals: impl IntoIterator<Item = PathBuf>) -> Result<(), String> {
    for path in journals {
        match std::fs::symlink_metadata(&path) {
            Ok(_) => return Err(
                "Native registration recovery is pending; complete it before package replacement"
                    .into(),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not inspect native registration recovery state".into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_journal_blocks_capture_for_before_after_and_absent_native_state() {
        for actual in [Some("before"), Some("after"), None] {
            for tray in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let paths = [
                    "hieronymus-task.pending.json",
                    "hieronymus-tray-task.pending.json",
                ]
                .map(|name| temp.path().join(name));
                let native = temp.path().join("native-state");
                if let Some(actual) = actual {
                    std::fs::write(&native, actual).unwrap();
                }
                let journal = &paths[usize::from(tray)];
                std::fs::write(journal, r#"{"before":"before","after":null}"#).unwrap();
                let before = std::fs::read(journal).unwrap();
                let package = temp.path().join("package-pending.json");
                let result = require_settled(paths.clone())
                    .and_then(|()| std::fs::write(&package, "snapshot").map_err(|e| e.to_string()));
                assert!(result.unwrap_err().contains("recovery is pending"));
                assert!(!package.exists());
                assert_eq!(std::fs::read(journal).unwrap(), before);
                assert_eq!(std::fs::read_to_string(&native).ok().as_deref(), actual);
                // Only the normal authenticated recovery path clears the journal.
                std::fs::remove_file(journal).unwrap();
                require_settled(paths).unwrap();
            }
        }
    }
}
