//! Keep session ownership alive until explicit worker/control cleanup returns.
// This helper also keeps the owner alive when cleanup reports an error.
pub fn shutdown_owned<T, C, E>(
    mut owner: Option<T>,
    take_controller: impl FnOnce(&mut T) -> Option<C>,
    shutdown: impl FnOnce(C) -> Result<(), E>,
) -> Result<(), E> {
    let controller = owner.as_mut().and_then(take_controller);
    let result = controller.map_or(Ok(()), shutdown);
    drop(owner);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::{File, OpenOptions},
        sync::mpsc,
        time::Duration,
    };

    fn claim(path: &std::path::Path) -> File {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap();
        file.try_lock().unwrap();
        file
    }
    #[test]
    fn shutdown_owner_covers_blocked_join_and_cleanup_success_and_failure() {
        for fail in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join(format!(".tray-{}.lock", "a".repeat(64)));
            let record = path.with_extension("json");
            std::fs::write(&record, "control record").unwrap();
            let owner = claim(&path);
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let cleanup_record = record.clone();
            let worker = std::thread::spawn(move || {
                shutdown_owned(
                    Some((owner, Some(cleanup_record))),
                    |runtime| runtime.1.take(),
                    move |cleanup_record| {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        if fail {
                            Err("cleanup failed")
                        } else {
                            std::fs::remove_file(cleanup_record).unwrap();
                            Ok(())
                        }
                    },
                )
            });
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let competing = File::options().read(true).write(true).open(&path).unwrap();
            let excluded = matches!(competing.try_lock(), Err(std::fs::TryLockError::WouldBlock));
            assert!(record.exists());
            release_tx.send(()).unwrap();
            assert_eq!(worker.join().unwrap().is_err(), fail);
            assert!(
                excluded,
                "singleton must cover the blocked shutdown attempt"
            );
            assert_eq!(record.exists(), fail);
            competing.try_lock().unwrap();
        }
    }
}
