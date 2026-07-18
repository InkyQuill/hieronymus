use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use rust_embed::RustEmbed;
use tempfile::{Builder, TempDir};

use super::{AgentError, PathFailure};

#[derive(RustEmbed)]
#[folder = "../../assets/skills"]
struct EmbeddedSkills;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillPlan {
    pub installed: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
}

#[must_use]
pub fn skill_assets() -> HashMap<String, String> {
    EmbeddedSkills::iter()
        .filter_map(|path| {
            let contents = EmbeddedSkills::get(path.as_ref())?;
            let text = String::from_utf8(contents.data.into_owned()).ok()?;
            Some((path.into_owned(), text))
        })
        .collect()
}

pub fn install_skills(
    workspace: &Path,
    targets: &[String],
    dry_run: bool,
) -> Result<SkillPlan, AgentError> {
    let roots = target_roots(workspace, targets)?;
    let assets = grouped_assets()?;
    validate_install_destinations(&roots, &assets)?;
    let installed = roots
        .iter()
        .flat_map(|root| {
            assets
                .keys()
                .map(move |directory| root.join(directory).join("SKILL.md"))
        })
        .collect::<Vec<_>>();
    if dry_run {
        return Ok(SkillPlan {
            installed,
            skipped: Vec::new(),
        });
    }

    let staged = stage_skill_directories(&roots, &assets)?;
    replace_staged_directories(&staged.paths)?;
    Ok(SkillPlan {
        installed,
        skipped: Vec::new(),
    })
}

pub fn uninstall_skills(
    workspace: &Path,
    targets: &[String],
    dry_run: bool,
) -> Result<SkillPlan, AgentError> {
    let roots = target_roots(workspace, targets)?;
    let mut installed = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in fs::read_dir(&root).map_err(|source| io_error(&root, source))? {
            let path = entry.map_err(|source| io_error(&root, source))?.path();
            if is_owned_skill_directory(&path) {
                installed.push(path);
            }
        }
    }
    installed.sort();
    if !dry_run {
        for path in &installed {
            fs::remove_dir_all(path).map_err(|source| io_error(path, source))?;
        }
    }
    Ok(SkillPlan {
        installed,
        skipped: Vec::new(),
    })
}

fn target_roots(workspace: &Path, targets: &[String]) -> Result<Vec<PathBuf>, AgentError> {
    if workspace
        .symlink_metadata()
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
        || !workspace.is_dir()
    {
        return Err(AgentError::UnsafePath(workspace.to_path_buf()));
    }
    let requested = targets.iter().map(String::as_str).collect::<HashSet<_>>();
    if let Some(unknown) = requested
        .iter()
        .find(|target| !["agents", "claude"].contains(target))
    {
        return Err(AgentError::UnsupportedSkillTarget((*unknown).to_string()));
    }
    let mut roots = Vec::new();
    for (target, relative) in [("agents", ".agents/skills"), ("claude", ".claude/skills")] {
        if requested.contains(target) {
            let root = workspace.join(relative);
            ensure_not_symlink(&root)?;
            if let Some(parent) = root.parent() {
                ensure_not_symlink(parent)?;
            }
            roots.push(root);
        }
    }
    Ok(roots)
}

fn grouped_assets() -> Result<BTreeMap<String, BTreeMap<PathBuf, String>>, AgentError> {
    let mut grouped = BTreeMap::<String, BTreeMap<PathBuf, String>>::new();
    for (relative, contents) in skill_assets() {
        let path = Path::new(&relative);
        let mut components = path.components();
        let directory = components
            .next()
            .and_then(|part| part.as_os_str().to_str())
            .ok_or_else(|| AgentError::UnsafePath(path.to_path_buf()))?;
        if !directory.starts_with("hieronymus-") {
            return Err(AgentError::UnsafePath(path.to_path_buf()));
        }
        let remainder = components.as_path();
        if remainder != Path::new("SKILL.md") {
            return Err(AgentError::UnsafePath(path.to_path_buf()));
        }
        grouped
            .entry(directory.to_owned())
            .or_default()
            .insert(remainder.to_path_buf(), contents);
    }
    Ok(grouped)
}

fn validate_install_destinations(
    roots: &[PathBuf],
    assets: &BTreeMap<String, BTreeMap<PathBuf, String>>,
) -> Result<(), AgentError> {
    for root in roots {
        for directory in assets.keys() {
            let destination = root.join(directory);
            ensure_not_symlink(&destination)?;
            ensure_not_symlink(&destination.join("SKILL.md"))?;
            if destination.exists() && !is_owned_skill_directory(&destination) {
                return Err(AgentError::UnsafePath(destination));
            }
        }
    }
    Ok(())
}

fn stage_skill_directories(
    roots: &[PathBuf],
    assets: &BTreeMap<String, BTreeMap<PathBuf, String>>,
) -> Result<StagedBatch, AgentError> {
    let mut paths = Vec::new();
    let mut owners = Vec::new();
    let result = (|| {
        for root in roots {
            fs::create_dir_all(root).map_err(|source| io_error(root, source))?;
            for (directory, files) in assets {
                let owner = Builder::new()
                    .prefix(&format!(".{directory}.stage-"))
                    .tempdir_in(root)
                    .map_err(|source| io_error(root, source))?;
                let stage = owner.path().to_path_buf();
                paths.push((root.join(directory), stage.clone()));
                owners.push(owner);
                for (relative, contents) in files {
                    let path = stage.join(relative);
                    fs::create_dir_all(
                        path.parent()
                            .ok_or_else(|| AgentError::MissingParent(path.clone()))?,
                    )
                    .map_err(|source| io_error(&path, source))?;
                    fs::write(&path, contents).map_err(|source| io_error(&path, source))?;
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let failures = remove_stages_with(&paths, &SystemDirectoryOperations);
        return if failures.is_empty() {
            Err(error)
        } else {
            Err(AgentError::SkillRollbackFailed {
                cause: Box::new(error),
                failures,
            })
        };
    }
    Ok(StagedBatch {
        paths,
        _owners: owners,
    })
}

struct StagedBatch {
    paths: Vec<(PathBuf, PathBuf)>,
    _owners: Vec<TempDir>,
}

fn replace_staged_directories(staged: &[(PathBuf, PathBuf)]) -> Result<(), AgentError> {
    replace_staged_directories_with(staged, &SystemDirectoryOperations)
}

trait DirectoryOperations {
    fn exists(&self, path: &Path) -> bool;
    fn rename(&self, source: &Path, destination: &Path) -> std::io::Result<()>;
    fn remove_dir_all(&self, path: &Path) -> std::io::Result<()>;
    fn create_backup(&self, parent: &Path) -> std::io::Result<OwnedBackup>;
}

struct OwnedBackup {
    path: PathBuf,
    _owner: Option<TempDir>,
}

struct SystemDirectoryOperations;

impl DirectoryOperations for SystemDirectoryOperations {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn rename(&self, source: &Path, destination: &Path) -> std::io::Result<()> {
        fs::rename(source, destination)
    }

    fn remove_dir_all(&self, path: &Path) -> std::io::Result<()> {
        fs::remove_dir_all(path)
    }

    fn create_backup(&self, parent: &Path) -> std::io::Result<OwnedBackup> {
        let owner = Builder::new()
            .prefix(".hiero-skill-backup-")
            .tempdir_in(parent)?;
        let path = owner.path().join("original");
        Ok(OwnedBackup {
            path,
            _owner: Some(owner),
        })
    }
}

fn replace_staged_directories_with(
    staged: &[(PathBuf, PathBuf)],
    operations: &impl DirectoryOperations,
) -> Result<(), AgentError> {
    let mut replaced = Vec::<(PathBuf, PathBuf, Option<OwnedBackup>)>::new();
    for (destination, stage) in staged {
        let backup = if operations.exists(destination) {
            let parent = destination
                .parent()
                .ok_or_else(|| AgentError::MissingParent(destination.clone()))?;
            let backup = match operations.create_backup(parent) {
                Ok(backup) => backup,
                Err(source) => {
                    return rollback_or_error(
                        io_error(parent, source),
                        &replaced,
                        staged,
                        operations,
                    );
                }
            };
            if let Err(source) = operations.rename(destination, &backup.path) {
                return rollback_or_error(
                    io_error(destination, source),
                    &replaced,
                    staged,
                    operations,
                );
            }
            Some(backup)
        } else {
            None
        };
        replaced.push((destination.clone(), stage.clone(), backup));
        if let Err(source) = operations.rename(stage, destination) {
            return rollback_or_error(io_error(destination, source), &replaced, staged, operations);
        }
    }
    let mut cleanup_failures = Vec::new();
    for (_, _, backup) in replaced {
        if let Some(backup) = backup
            && let Err(source) = operations.remove_dir_all(&backup.path)
        {
            cleanup_failures.push(path_failure(
                "remove committed backup",
                &backup.path,
                source,
            ));
        }
    }
    if cleanup_failures.is_empty() {
        Ok(())
    } else {
        Err(AgentError::CommittedSkillCleanup {
            failures: cleanup_failures,
        })
    }
}

fn rollback_or_error(
    cause: AgentError,
    replaced: &[(PathBuf, PathBuf, Option<OwnedBackup>)],
    staged: &[(PathBuf, PathBuf)],
    operations: &impl DirectoryOperations,
) -> Result<(), AgentError> {
    let mut failures = rollback_replacements(replaced, operations);
    failures.extend(remove_stages_with(staged, operations));
    if failures.is_empty() {
        Err(cause)
    } else {
        Err(AgentError::SkillRollbackFailed {
            cause: Box::new(cause),
            failures,
        })
    }
}

fn rollback_replacements(
    replaced: &[(PathBuf, PathBuf, Option<OwnedBackup>)],
    operations: &impl DirectoryOperations,
) -> Vec<PathFailure> {
    let mut failures = Vec::new();
    for (destination, stage, backup) in replaced.iter().rev() {
        if operations.exists(destination)
            && let Err(source) = operations.rename(destination, stage)
        {
            failures.push(path_failure(
                "move replacement back to stage",
                destination,
                source,
            ));
        }
        if let Some(backup) = backup
            && let Err(source) = operations.rename(&backup.path, destination)
        {
            failures.push(path_failure("restore original skill", &backup.path, source));
        }
    }
    failures
}

fn remove_stages_with(
    staged: &[(PathBuf, PathBuf)],
    operations: &impl DirectoryOperations,
) -> Vec<PathFailure> {
    let mut failures = Vec::new();
    for (_, stage) in staged {
        if operations.exists(stage)
            && let Err(source) = operations.remove_dir_all(stage)
        {
            failures.push(path_failure("remove staged skill", stage, source));
        }
    }
    failures
}

fn path_failure(operation: &'static str, path: &Path, source: std::io::Error) -> PathFailure {
    PathFailure {
        operation,
        path: path.to_path_buf(),
        error: source.to_string(),
    }
}

fn is_owned_skill_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_embedded_skill_name)
        && path.is_dir()
        && !path
            .symlink_metadata()
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(true)
        && path.join("SKILL.md").is_file()
        && !path
            .join("SKILL.md")
            .symlink_metadata()
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(true)
}

fn is_embedded_skill_name(name: &str) -> bool {
    EmbeddedSkills::iter().any(|path| {
        Path::new(path.as_ref())
            .components()
            .next()
            .and_then(|part| part.as_os_str().to_str())
            == Some(name)
    })
}

fn ensure_not_symlink(path: &Path) -> Result<(), AgentError> {
    if path
        .symlink_metadata()
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        Err(AgentError::UnsafePath(path.to_path_buf()))
    } else {
        Ok(())
    }
}

fn io_error(path: &Path, source: std::io::Error) -> AgentError {
    AgentError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::HashSet,
        io,
        path::{Path, PathBuf},
    };

    use super::{
        DirectoryOperations, OwnedBackup, install_skills, replace_staged_directories,
        replace_staged_directories_with,
    };
    use crate::agent::AgentError;

    struct FakeOperations {
        paths: RefCell<HashSet<PathBuf>>,
        rename_calls: Cell<usize>,
        fail_renames: HashSet<usize>,
        fail_cleanup: bool,
    }

    impl FakeOperations {
        fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
            Self {
                paths: RefCell::new(paths.into_iter().collect()),
                rename_calls: Cell::new(0),
                fail_renames: HashSet::new(),
                fail_cleanup: false,
            }
        }

        fn failing_renames(mut self, calls: &[usize]) -> Self {
            self.fail_renames.extend(calls.iter().copied());
            self
        }
    }

    impl DirectoryOperations for FakeOperations {
        fn exists(&self, path: &Path) -> bool {
            self.paths.borrow().contains(path)
        }

        fn rename(&self, source: &Path, destination: &Path) -> io::Result<()> {
            let call = self.rename_calls.get() + 1;
            self.rename_calls.set(call);
            if self.fail_renames.contains(&call) {
                return Err(io::Error::other(format!("injected rename failure {call}")));
            }
            let mut paths = self.paths.borrow_mut();
            if !paths.remove(source) || paths.contains(destination) {
                return Err(io::Error::other("invalid virtual rename"));
            }
            paths.insert(destination.to_path_buf());
            Ok(())
        }

        fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
            if self.fail_cleanup {
                return Err(io::Error::other("injected cleanup failure"));
            }
            self.paths.borrow_mut().remove(path);
            Ok(())
        }

        fn create_backup(&self, parent: &Path) -> io::Result<OwnedBackup> {
            Ok(OwnedBackup {
                path: parent.join(format!(".fake-backup-{}/original", self.rename_calls.get())),
                _owner: None,
            })
        }
    }

    #[test]
    fn pre_commit_failure_rolls_back_every_completed_replacement() {
        let destination_a = PathBuf::from("/skills/a");
        let destination_b = PathBuf::from("/skills/b");
        let stage_a = PathBuf::from("/skills/.a-stage");
        let stage_b = PathBuf::from("/skills/.b-stage");
        let operations = FakeOperations::new([
            destination_a.clone(),
            destination_b.clone(),
            stage_a.clone(),
            stage_b.clone(),
        ])
        .failing_renames(&[3]);

        let error = replace_staged_directories_with(
            &[
                (destination_a.clone(), stage_a),
                (destination_b.clone(), stage_b),
            ],
            &operations,
        )
        .unwrap_err();

        assert!(matches!(error, AgentError::Io { .. }));
        assert!(operations.exists(&destination_a));
        assert!(operations.exists(&destination_b));
    }

    #[test]
    fn rollback_failure_reports_each_failed_restore() {
        let destination_a = PathBuf::from("/skills/a");
        let destination_b = PathBuf::from("/skills/b");
        let stage_a = PathBuf::from("/skills/.a-stage");
        let stage_b = PathBuf::from("/skills/.b-stage");
        let operations = FakeOperations::new([
            destination_a.clone(),
            destination_b.clone(),
            stage_a.clone(),
            stage_b.clone(),
        ])
        .failing_renames(&[3, 4]);

        let error = replace_staged_directories_with(
            &[(destination_a, stage_a), (destination_b, stage_b)],
            &operations,
        )
        .unwrap_err();

        assert!(
            matches!(error, AgentError::SkillRollbackFailed { ref failures, .. } if failures.len() == 2)
        );
    }

    #[test]
    fn cleanup_failure_reports_commit_and_retained_backup() {
        let destination = PathBuf::from("/skills/a");
        let stage = PathBuf::from("/skills/.a-stage");
        let mut operations = FakeOperations::new([destination.clone(), stage.clone()]);
        operations.fail_cleanup = true;

        let error = replace_staged_directories_with(&[(destination.clone(), stage)], &operations)
            .unwrap_err();

        assert!(
            matches!(error, AgentError::CommittedSkillCleanup { ref failures } if failures.len() == 1)
        );
        assert!(operations.exists(&destination));
    }

    #[cfg(unix)]
    #[test]
    fn staging_never_follows_a_preexisting_predictable_symlink() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = workspace.path().join(".agents/skills");
        std::fs::create_dir_all(&root).unwrap();
        let legacy_candidate = root.join(format!(
            ".hieronymus-bootstrap.stage-{}-0",
            std::process::id()
        ));
        symlink(outside.path(), &legacy_candidate).unwrap();

        install_skills(workspace.path(), &["agents".into()], false).unwrap();

        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        assert!(legacy_candidate.is_symlink());
    }

    #[test]
    fn backup_creation_ignores_a_preexisting_legacy_candidate() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("a");
        let stage = root.path().join("stage");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("value"), "old").unwrap();
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("value"), "new").unwrap();
        let legacy_candidate = root
            .path()
            .join(format!(".a.backup-{}-0", std::process::id()));
        std::fs::create_dir(&legacy_candidate).unwrap();

        replace_staged_directories(&[(destination.clone(), stage)]).unwrap();

        assert_eq!(
            std::fs::read_to_string(destination.join("value")).unwrap(),
            "new"
        );
        assert!(legacy_candidate.is_dir());
    }
}
