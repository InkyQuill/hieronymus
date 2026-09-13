//! File mutations performed while the caller retains installation ownership.
use std::{io, path::Path};

/// A stopped Windows process can release its coordination locks just before its
/// executable mappings disappear. Retry only the individual filesystem mutation;
/// never repeat the installation transaction or change permissions.
fn mutate<T>(operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    #[cfg(windows)]
    {
        retry_windows(operation, std::time::Duration::from_secs(3))
    }
    #[cfg(not(windows))]
    {
        let mut operation = operation;
        operation()
    }
}

#[cfg(any(windows, test))]
fn retry_windows<T>(
    mut operation: impl FnMut() -> io::Result<T>,
    timeout: std::time::Duration,
) -> io::Result<T> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + timeout;
    loop {
        match operation() {
            Err(error)
                if matches!(error.raw_os_error(), Some(5 | 32 | 33))
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(
                    Duration::from_millis(25)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            result => return result,
        }
    }
}

pub(crate) fn promote(source: &Path, destination: &Path) -> io::Result<()> {
    mutate(|| std::fs::rename(source, destination)).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "promoting staged application to {}: {error}",
                destination.display()
            ),
        )
    })
}

pub(crate) fn remove(root: &Path) -> io::Result<()> {
    mutate(|| std::fs::remove_dir_all(root)).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "removing installed application at {}: {error}",
                root.display()
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[cfg(windows)]
    #[test]
    fn native_windows_sharing_handles_release_before_owned_mutation_finishes() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        let installed = temp.path().join("installed");
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("payload.exe"), b"fixture").unwrap();
        let hold = |path: &Path| {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
                .open(path)
                .unwrap();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                drop(file);
            })
        };
        let reader = hold(&staging.join("payload.exe"));
        promote(&staging, &installed).unwrap();
        reader.join().unwrap();
        let reader = hold(&installed.join("payload.exe"));
        remove(&installed).unwrap();
        reader.join().unwrap();
        assert!(!installed.exists());
    }

    #[test]
    fn transient_windows_handles_can_settle_without_replaying_the_caller() {
        for code in [5, 32, 33] {
            let mut attempts = 0;
            let value = retry_windows(
                || {
                    attempts += 1;
                    if attempts < 2 {
                        Err(io::Error::from_raw_os_error(code))
                    } else {
                        Ok(42)
                    }
                },
                Duration::from_millis(100),
            )
            .unwrap();
            assert_eq!((value, attempts), (42, 2));
        }
    }

    #[test]
    fn permanent_and_expired_errors_remain_failures() {
        for (code, timeout) in [(2, Duration::from_secs(1)), (5, Duration::ZERO)] {
            let mut attempts = 0;
            let result: io::Result<()> = retry_windows(
                || {
                    attempts += 1;
                    Err(io::Error::from_raw_os_error(code))
                },
                timeout,
            );
            assert_eq!(result.unwrap_err().raw_os_error(), Some(code));
            assert_eq!(attempts, 1);
        }
        let started = std::time::Instant::now();
        let result: io::Result<()> = retry_windows(
            || Err(io::Error::from_raw_os_error(32)),
            Duration::from_millis(10),
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(32));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn promotion_and_removal_preserve_contents_and_report_the_failed_path() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        let installed = temp.path().join("installed");
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("payload"), b"verified").unwrap();
        promote(&staging, &installed).unwrap();
        assert_eq!(
            std::fs::read(installed.join("payload")).unwrap(),
            b"verified"
        );
        remove(&installed).unwrap();
        let error = remove(&installed).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains(&installed.display().to_string()));
    }
}
