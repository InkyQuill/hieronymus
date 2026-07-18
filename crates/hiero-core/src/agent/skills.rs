use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use rust_embed::RustEmbed;

use super::AgentError;

#[derive(RustEmbed)]
#[folder = "../../assets/skills"]
struct EmbeddedSkills;

static STAGE_ID: AtomicU64 = AtomicU64::new(0);

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
    replace_staged_directories(&staged)?;
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
) -> Result<Vec<(PathBuf, PathBuf)>, AgentError> {
    let mut staged = Vec::new();
    let result = (|| {
        for root in roots {
            fs::create_dir_all(root).map_err(|source| io_error(root, source))?;
            for (directory, files) in assets {
                let id = STAGE_ID.fetch_add(1, Ordering::Relaxed);
                let stage = root.join(format!(".{directory}.stage-{}-{id}", std::process::id()));
                staged.push((root.join(directory), stage.clone()));
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
        remove_stages(&staged);
        return Err(error);
    }
    Ok(staged)
}

fn replace_staged_directories(staged: &[(PathBuf, PathBuf)]) -> Result<(), AgentError> {
    let mut replaced = Vec::<(PathBuf, PathBuf, Option<PathBuf>)>::new();
    for (destination, stage) in staged {
        let backup = if destination.exists() {
            let id = STAGE_ID.fetch_add(1, Ordering::Relaxed);
            let name = destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("skill");
            let backup =
                destination.with_file_name(format!(".{name}.backup-{}-{id}", std::process::id()));
            if let Err(source) = fs::rename(destination, &backup) {
                rollback_replacements(&replaced);
                remove_stages(staged);
                return Err(io_error(destination, source));
            }
            Some(backup)
        } else {
            None
        };
        replaced.push((destination.clone(), stage.clone(), backup));
        if let Err(source) = fs::rename(stage, destination) {
            rollback_replacements(&replaced);
            remove_stages(staged);
            return Err(io_error(destination, source));
        }
    }
    for (_, _, backup) in replaced {
        if let Some(backup) = backup {
            fs::remove_dir_all(&backup).map_err(|source| io_error(&backup, source))?;
        }
    }
    Ok(())
}

fn rollback_replacements(replaced: &[(PathBuf, PathBuf, Option<PathBuf>)]) {
    for (destination, stage, backup) in replaced.iter().rev() {
        if destination.exists() {
            let _ = fs::rename(destination, stage);
        }
        if let Some(backup) = backup {
            let _ = fs::rename(backup, destination);
        }
    }
}

fn remove_stages(staged: &[(PathBuf, PathBuf)]) {
    for (_, stage) in staged {
        if stage.exists() {
            let _ = fs::remove_dir_all(stage);
        }
    }
}

fn is_owned_skill_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("hieronymus-"))
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
