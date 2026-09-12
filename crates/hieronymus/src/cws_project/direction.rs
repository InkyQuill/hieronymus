//! Actionable CWS direction/edition selection over the structural adapter.
use super::{
    CwsError, CwsProject, DocumentRole, MetadataValue, classify, is_manifest, metadata,
    parse_frontmatter, required_string, safe_project_path,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

/// The edition metadata selected as task evidence, without interpreting trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CwsEdition {
    pub edition_id: String,
    pub language: String,
    pub edition_role: String,
    pub revision_label: String,
    pub coverage: Vec<String>,
}

/// Source identity is edition-qualified. Hashes retain the exact selected bytes
/// and the producer's original-file receipt; neither implies chapter alignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CwsSourceUnit {
    pub reference: String,
    pub volume_id: Option<String>,
    pub path: PathBuf,
    pub sha256: String,
    pub original_path: Option<PathBuf>,
    pub original_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectedDirection {
    pub direction_id: String,
    pub work_kind: String,
    pub volume_id: Option<String>,
    pub source_language: String,
    pub target_language: String,
    pub primary_edition: CwsEdition,
    pub auxiliary_editions: Vec<CwsEdition>,
    pub source_units: Vec<CwsSourceUnit>,
}

pub(crate) fn valid_identity(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        })
}

fn identity<'a>(
    fields: &'a BTreeMap<String, MetadataValue>,
    key: &str,
) -> Result<&'a str, CwsError> {
    let value = required_string(fields, key)?;
    if !valid_identity(value) {
        return Err(CwsError::InvalidManifest);
    }
    Ok(value)
}

fn strings(fields: &BTreeMap<String, MetadataValue>, key: &str) -> Result<Vec<String>, CwsError> {
    let values = match fields.get(key) {
        None => return Ok(Vec::new()),
        Some(MetadataValue::String(value)) if value.is_empty() => return Ok(Vec::new()),
        Some(MetadataValue::List(values)) => values,
        _ => return Err(CwsError::InvalidManifest),
    };
    let mut seen = std::collections::BTreeSet::new();
    if values.iter().any(|v| v.is_empty() || !seen.insert(v)) {
        return Err(CwsError::InvalidManifest);
    }
    Ok(values.clone())
}

fn read_metadata(
    project: &CwsProject,
    path: &Path,
) -> Result<BTreeMap<String, MetadataValue>, CwsError> {
    let path = safe_project_path(project, path)?;
    if !is_manifest(&path)? {
        return Err(CwsError::InvalidManifest);
    }
    parse_frontmatter(&fs::read(path)?)
}

/// Inspect one level of a known structural directory, never originals/private
/// trees. Callers validate recognized paths before opening them.
fn child_paths(project: &CwsProject, relative: &Path) -> Result<Vec<PathBuf>, CwsError> {
    let directory = safe_project_path(project, relative)?;
    let Some(info) = metadata(&directory)? else {
        return Ok(Vec::new());
    };
    if !info.is_dir() {
        return Err(CwsError::InvalidManifest);
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = relative.join(entry.file_name());
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

fn edition(project: &CwsProject, id: &str, volume: Option<&str>) -> Result<CwsEdition, CwsError> {
    if !valid_identity(id) {
        return Err(CwsError::InvalidManifest);
    }
    let fields = read_metadata(project, Path::new(&format!("sources/{id}/edition.md")))?;
    if identity(&fields, "edition-id")? != id {
        return Err(CwsError::InvalidManifest);
    }
    let role = required_string(&fields, "edition-role")?;
    if !matches!(role, "original" | "translation") {
        return Err(CwsError::InvalidManifest);
    }
    let coverage = strings(&fields, "coverage")?;
    if coverage.iter().any(|id| !valid_identity(id)) {
        return Err(CwsError::InvalidManifest);
    }
    if volume.is_some_and(|v| !coverage.iter().any(|c| c == v)) {
        return Err(CwsError::UncoveredEdition);
    }
    Ok(CwsEdition {
        edition_id: id.into(),
        language: required_string(&fields, "language")?.trim().to_lowercase(),
        edition_role: required_string(&fields, "edition-role")?.into(),
        revision_label: required_string(&fields, "revision-label")?.into(),
        coverage,
    })
}

fn effective_direction(
    project: &CwsProject,
    fields: &BTreeMap<String, MetadataValue>,
    volume: Option<&str>,
) -> Result<SelectedDirection, CwsError> {
    let id = identity(fields, "direction-id")?;
    let mut effective = fields.clone();
    if let Some(volume) = volume {
        if !valid_identity(volume) || !strings(fields, "coverage")?.iter().any(|v| v == volume) {
            return Err(CwsError::InvalidManifest);
        }
        let path = PathBuf::from(format!("translations/{id}/volumes/{volume}/settings.md"));
        let absolute = safe_project_path(project, &path)?;
        if metadata(&absolute)?.is_some() {
            let overrides = read_metadata(project, &path)?;
            if identity(&overrides, "direction-id")? != id
                || identity(&overrides, "volume-id")? != volume
                || overrides.keys().any(|k| {
                    !matches!(
                        k.as_str(),
                        "direction-id"
                            | "volume-id"
                            | "primary-edition"
                            | "auxiliary-editions"
                            | "inheritance"
                    )
                })
            {
                return Err(CwsError::InvalidManifest);
            }
            for key in ["primary-edition", "auxiliary-editions", "inheritance"] {
                if let Some(value) = overrides.get(key) {
                    effective.insert(key.into(), value.clone());
                }
            }
        }
    }
    strings(&effective, "inheritance")?;
    let primary_id = identity(&effective, "primary-edition")?;
    let auxiliary_ids = strings(&effective, "auxiliary-editions")?;
    if auxiliary_ids.iter().any(|id| id == primary_id) {
        return Err(CwsError::InvalidManifest);
    }
    let primary_edition = edition(project, primary_id, volume)?;
    let auxiliary_editions = auxiliary_ids
        .iter()
        .map(|id| edition(project, id, volume))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SelectedDirection {
        direction_id: id.into(),
        work_kind: project.work_kind.clone(),
        volume_id: volume.map(str::to_owned),
        source_language: primary_edition.language.clone(),
        target_language: required_string(fields, "language")?.trim().to_lowercase(),
        primary_edition,
        auxiliary_editions,
        source_units: Vec::new(),
    })
}

fn direction_at_volume(
    project: &CwsProject,
    fields: &BTreeMap<String, MetadataValue>,
    volume: Option<&str>,
) -> Result<SelectedDirection, CwsError> {
    let coverage = strings(fields, "coverage")?;
    if coverage.iter().any(|id| !valid_identity(id)) {
        return Err(CwsError::InvalidManifest);
    }
    if project.work_kind == "book" {
        return effective_direction(project, fields, None);
    }
    if let Some(volume) = volume {
        return effective_direction(project, fields, Some(volume));
    }
    let first = coverage.first().ok_or(CwsError::InvalidManifest)?;
    let mut selected = effective_direction(project, fields, Some(first))?;
    for volume in coverage.iter().skip(1) {
        let next = effective_direction(project, fields, Some(volume))?;
        if selected.primary_edition != next.primary_edition
            || selected.auxiliary_editions != next.auxiliary_editions
            || selected.target_language != next.target_language
        {
            return Err(CwsError::AmbiguousVolume);
        }
    }
    if coverage.len() > 1 {
        selected.volume_id = None;
    }
    Ok(selected)
}

fn uses_edition(
    project: &CwsProject,
    fields: &BTreeMap<String, MetadataValue>,
    edition: &str,
    volume: Option<&str>,
) -> Result<bool, CwsError> {
    let coverage = strings(fields, "coverage")?;
    let volumes = if project.work_kind == "book" {
        vec![None]
    } else if let Some(volume) = volume {
        if !coverage.iter().any(|v| v == volume) {
            return Ok(false);
        }
        vec![Some(volume)]
    } else {
        coverage.iter().map(|v| Some(v.as_str())).collect()
    };
    for volume in volumes {
        let selected = effective_direction(project, fields, volume)?;
        if selected.primary_edition.edition_id == edition
            || selected
                .auxiliary_editions
                .iter()
                .any(|e| e.edition_id == edition)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn selected_unit(project: &CwsProject, relative: &Path) -> Result<CwsSourceUnit, CwsError> {
    use sha2::{Digest, Sha256};
    if classify(project, relative)? != DocumentRole::SourceUnit {
        return Err(CwsError::InvalidManifest);
    }
    let parts: Vec<_> = relative
        .components()
        .filter_map(|part| match part {
            Component::Normal(value) => Some(value.to_str().ok_or(CwsError::UnsafePath)),
            _ => None,
        })
        .collect::<Result<_, _>>()?;
    let edition_id = parts[1];
    if !valid_identity(edition_id) {
        return Err(CwsError::InvalidManifest);
    }
    let path = safe_project_path(project, relative)?;
    let bytes = fs::read(&path)?;
    let fields = parse_frontmatter(&bytes)?;
    let unit_id = identity(&fields, "unit-id")?;
    let volume_id = if project.work_kind == "series" {
        let volume = identity(&fields, "volume-id")?;
        if volume != parts[3] {
            return Err(CwsError::InvalidManifest);
        }
        Some(volume.to_owned())
    } else {
        if fields.contains_key("volume-id") {
            return Err(CwsError::InvalidManifest);
        }
        None
    };
    let original_path = match fields.get("original-path") {
        None => None,
        Some(_) => Some(safe_project_path(
            project,
            Path::new(required_string(&fields, "original-path")?),
        )?),
    };
    let original_sha256 = match fields.get("original-sha256") {
        None => None,
        Some(_) => {
            let hash = required_string(&fields, "original-sha256")?;
            if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err(CwsError::InvalidManifest);
            }
            Some(hash.into())
        }
    };
    Ok(CwsSourceUnit {
        reference: format!("{edition_id}:{unit_id}"),
        volume_id,
        path,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        original_path,
        original_sha256,
    })
}

fn resolve_source_unit(project: &CwsProject, reference: &str) -> Result<CwsSourceUnit, CwsError> {
    let (edition_id, unit_id) = reference.split_once(':').ok_or(CwsError::InvalidManifest)?;
    if !valid_identity(edition_id) || !valid_identity(unit_id) {
        return Err(CwsError::InvalidManifest);
    }
    let edition = edition(project, edition_id, None)?;
    let roots = if project.work_kind == "book" {
        vec![PathBuf::from(format!("sources/{edition_id}/text"))]
    } else {
        edition
            .coverage
            .iter()
            .map(|v| PathBuf::from(format!("sources/{edition_id}/volumes/{v}/text")))
            .collect()
    };
    let mut selected = None;
    for root in roots {
        for path in child_paths(project, &root)? {
            if path.extension().is_none_or(|ext| ext != "md")
                || path.file_name().is_some_and(|name| name == "_index.md")
            {
                continue;
            }
            let fields = read_metadata(project, &path)?;
            if identity(&fields, "unit-id")? == unit_id {
                if selected.is_some() {
                    return Err(CwsError::InvalidManifest);
                }
                selected = Some(selected_unit(project, &path)?);
            }
        }
    }
    selected.ok_or(CwsError::InvalidManifest)
}

/// Resolve explicit metadata or a uniquely identifying selected path. Binding
/// maps never choose a direction, and no chapter-number alignment is inferred.
pub fn select_direction(
    project: &CwsProject,
    selected_path: &Path,
    requested: Option<&str>,
) -> Result<Option<SelectedDirection>, CwsError> {
    let relative = if selected_path.is_absolute() {
        selected_path
            .strip_prefix(&project.root)
            .map_err(|_| CwsError::UnsafePath)?
    } else {
        selected_path
    };
    safe_project_path(project, relative)?;
    let parts: Vec<_> = relative
        .components()
        .filter_map(|p| match p {
            Component::Normal(v) => v.to_str(),
            _ => None,
        })
        .collect();
    let path_direction = (parts.first() == Some(&"translations"))
        .then(|| parts.get(1).copied().filter(|id| valid_identity(id)))
        .flatten();
    let source_edition = (parts.first() == Some(&"sources"))
        .then(|| parts.get(1).copied().filter(|id| valid_identity(id)))
        .flatten();
    if requested.is_none()
        && path_direction.is_none()
        && source_edition.is_none()
        && project.project_kind == "authoring"
    {
        return Ok(None);
    }
    let requested = requested.or(path_direction);
    if project.schema_version != 2 {
        return Err(CwsError::UnknownDirection(requested.unwrap_or("").into()));
    }
    if let Some(id) = requested {
        if !valid_identity(id) {
            return Err(CwsError::InvalidManifest);
        }
        if path_direction.is_some_and(|path| path != id) {
            return Err(CwsError::ConflictingDirection);
        }
    }
    let volume = if matches!(parts.first(), Some(&"translations" | &"sources"))
        && parts.get(2) == Some(&"volumes")
    {
        parts.get(3).copied()
    } else {
        None
    };
    if volume.is_some() && project.work_kind != "series" {
        return Err(CwsError::InvalidManifest);
    }
    let mut candidates = Vec::new();
    let directories = match requested {
        Some(id) => vec![PathBuf::from(format!("translations/{id}"))],
        None => child_paths(project, Path::new("translations"))?,
    };
    for directory in directories {
        let Some(id) = directory
            .file_name()
            .and_then(|v| v.to_str())
            .filter(|v| valid_identity(v))
        else {
            continue;
        };
        if requested.is_some_and(|r| r != id) {
            continue;
        }
        let path = directory.join("translation.md");
        if !is_manifest(&safe_project_path(project, &path)?)? {
            continue;
        }
        let fields = read_metadata(project, &path)?;
        if identity(&fields, "direction-id")? != id {
            return Err(CwsError::InvalidManifest);
        }
        if let Some(edition) = source_edition {
            // Only directions using the selected edition at the selected volume
            // are candidates. A shared source never implies one rendering.
            if !uses_edition(project, &fields, edition, volume)? {
                continue;
            }
        }
        candidates.push(fields);
    }
    if candidates.len() > 1 {
        return Err(CwsError::AmbiguousDirection);
    }
    let fields = candidates
        .pop()
        .ok_or_else(|| CwsError::UnknownDirection(requested.unwrap_or("").into()))?;
    let mut selected = direction_at_volume(project, &fields, volume)?;
    if metadata(&safe_project_path(project, relative)?)?.is_some_and(|m| m.is_file()) {
        match classify(project, relative)? {
            DocumentRole::SourceUnit => selected
                .source_units
                .push(selected_unit(project, relative)?),
            DocumentRole::Draft | DocumentRole::Review | DocumentRole::AcceptedProse
                if path_direction.is_some() =>
            {
                let fields = read_metadata(project, relative)?;
                if project.work_kind == "series"
                    && fields.contains_key("volume-id")
                    && Some(identity(&fields, "volume-id")?) != selected.volume_id.as_deref()
                {
                    return Err(CwsError::InvalidManifest);
                }
                if identity(&fields, "direction-id")? != selected.direction_id {
                    return Err(CwsError::ConflictingDirection);
                }
                for reference in strings(&fields, "source-units")? {
                    let unit = resolve_source_unit(project, &reference)?;
                    let (id, _) = unit
                        .reference
                        .split_once(':')
                        .ok_or(CwsError::InvalidManifest)?;
                    if (selected.primary_edition.edition_id != id
                        && !selected
                            .auxiliary_editions
                            .iter()
                            .any(|e| e.edition_id == id))
                        || unit.volume_id != selected.volume_id
                    {
                        return Err(CwsError::InvalidManifest);
                    }
                    selected.source_units.push(unit);
                }
            }
            _ => {}
        }
    }
    Ok(Some(selected))
}
