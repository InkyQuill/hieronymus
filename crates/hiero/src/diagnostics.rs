//! Readable v0 process diagnostics, protected by the data root's private-file rules.
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
    process::{Command, Stdio},
};

pub(crate) fn open(root: &Path, name: &str) -> io::Result<File> {
    std::fs::create_dir_all(root)?;
    let path = root.join(name);
    match hieronymus::private_file::create_private_new(&path, b"") {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    hieronymus::private_file::open_append(&path)
}

pub(crate) fn redirect(command: &mut Command, root: &Path, name: &str) -> io::Result<()> {
    let mut file = open(root, name)?;
    writeln!(file, "{} Starting process", chrono::Utc::now())?;
    command
        .stdin(Stdio::null())
        .stdout(file.try_clone()?)
        .stderr(file);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_log_handles_append_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let mut first = open(root.path(), "test.log").unwrap();
        let mut second = open(root.path(), "test.log").unwrap();
        first.write_all(b"first\n").unwrap();
        second.write_all(b"second\n").unwrap();
        first.write_all(b"third\n").unwrap();
        assert_eq!(
            std::fs::read(root.path().join("test.log")).unwrap(),
            b"first\nsecond\nthird\n"
        );
    }
    #[cfg(unix)]
    #[test]
    fn aliased_log_is_refused_without_writing_to_its_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("valuable");
        std::fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, root.path().join("test.log")).unwrap();
        assert!(open(root.path(), "test.log").is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "keep");
    }
}
