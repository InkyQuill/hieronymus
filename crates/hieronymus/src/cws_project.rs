//! Read-only CWS structural discovery. No CWS runtime or memory authority is implied.

use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::Serialize;

mod direction;
pub(crate) use direction::valid_identity;
pub use direction::{CwsEdition, CwsSourceUnit, SelectedDirection, select_direction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentRole {
    Instructions,
    Manifest,
    AcceptedProse,
    Draft,
    Work,
    Knowledge,
    Derived,
    PrivateState,
    Opaque,
    SourceEdition,
    SourceUnit,
    TranslationDirection,
    DirectionSettings,
    TranslationMemory,
    Alignment,
    Entity,
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CwsProject {
    pub root: PathBuf,
    pub schema_version: u64,
    pub language: String,
    pub project_kind: String,
    pub work_kind: String,
    pub translation_enabled: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CwsError {
    #[error("unsafe CWS project path")]
    UnsafePath,
    #[error("invalid CWS project manifest")]
    InvalidManifest,
    #[error("unsupported CWS project schema {0}")]
    UnsupportedSchema(u64),
    #[error("invalid CWS binding")]
    InvalidBinding,
    #[error("unsupported CWS binding version {0}")]
    UnsupportedBinding(u64),
    #[error("unsupported CWS project contract version {0}")]
    UnsupportedContractVersion(u64),
    #[error("selected path conflicts with the explicit CWS direction")]
    ConflictingDirection,
    #[error("CWS direction has different effective sources across volumes")]
    AmbiguousVolume,
    #[error("CWS edition does not cover the selected volume")]
    UncoveredEdition,
    #[error("multiple CWS translation directions require explicit selection")]
    AmbiguousDirection,
    #[error("unknown CWS translation direction {0}")]
    UnknownDirection(String),
    #[error("CWS project I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Find the nearest regular, non-linked manifest. Invalid manifests stop discovery.
/// Only the requested path and its ancestors are inspected; content is never walked.
pub fn discover(start: &Path) -> Result<Option<CwsProject>, CwsError> {
    if start.components().any(|part| part == Component::ParentDir) {
        return Err(CwsError::UnsafePath);
    }
    let absolute = std::path::absolute(start)?;
    let directory = match metadata(&absolute)? {
        Some(info) if !info.is_dir() => absolute.parent().ok_or(CwsError::UnsafePath)?,
        _ => absolute.as_path(),
    };
    for root in directory.ancestors() {
        let path = root.join("project.md");
        if !is_manifest(&path)? {
            continue;
        }
        if metadata(root)?.is_some_and(|info| info.file_type().is_symlink()) {
            return Err(CwsError::UnsafePath);
        }
        let relative = absolute
            .strip_prefix(root)
            .map_err(|_| CwsError::UnsafePath)?;
        let canonical_root = root.canonicalize()?;
        let fields = parse_frontmatter(&fs::read(canonical_root.join("project.md"))?)?;
        let project = project_from_metadata(canonical_root, &fields)?;
        safe_project_path(&project, relative)?;
        return Ok(Some(project));
    }
    Ok(None)
}

fn project_from_metadata(
    root: PathBuf,
    fields: &BTreeMap<String, MetadataValue>,
) -> Result<CwsProject, CwsError> {
    let schema_version = match fields.get("schema-version") {
        Some(MetadataValue::Integer(value)) => {
            serde_yaml_ng::from_str::<u64>(value).map_err(|_| CwsError::InvalidManifest)?
        }
        _ => return Err(CwsError::InvalidManifest),
    };
    if !matches!(schema_version, 1 | 2) {
        return Err(CwsError::UnsupportedSchema(schema_version));
    }
    required_string(fields, "title")?;
    let language = required_string(fields, "language")?.to_owned();
    if !matches!(
        required_string(fields, "status")?,
        "planning" | "drafting" | "revising" | "complete" | "archived"
    ) {
        return Err(CwsError::InvalidManifest);
    }
    let (project_kind, work_kind, translation_enabled) = if schema_version == 2 {
        let kind = required_string(fields, "project-kind")?;
        let work = required_string(fields, "work-kind")?;
        if !matches!(kind, "authoring" | "translation")
            || !matches!(work, "book" | "series")
            || fields.get("translation-enabled") != Some(&MetadataValue::Bool(true))
        {
            return Err(CwsError::InvalidManifest);
        }
        (kind, work, true)
    } else {
        ("authoring", "book", false)
    };
    Ok(CwsProject {
        root,
        schema_version,
        language,
        project_kind: project_kind.to_owned(),
        work_kind: work_kind.to_owned(),
        translation_enabled,
    })
}

fn required_string<'a>(
    fields: &'a BTreeMap<String, MetadataValue>,
    key: &str,
) -> Result<&'a str, CwsError> {
    match fields.get(key) {
        Some(MetadataValue::String(value)) if !value.trim_matches(cws_whitespace).is_empty() => {
            Ok(value)
        }
        _ => Err(CwsError::InvalidManifest),
    }
}

// Retain decimal spelling independently of machine integer range: CWS integers
// are arbitrary precision, and numeric list items retain their original spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetadataValue {
    String(String),
    Integer(String),
    Bool(bool),
    List(Vec<String>),
}

/// The deliberately flat cwcli.documents subset, never general YAML or body YAML.
pub(crate) fn parse_frontmatter(bytes: &[u8]) -> Result<BTreeMap<String, MetadataValue>, CwsError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CwsError::InvalidManifest)?;
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    // Python splitlines also recognizes these Unicode/control line separators.
    const SEPARATORS: [char; 9] = [
        '\n', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
    ];
    let mut lines = text
        .split_inclusive(SEPARATORS)
        .map(|line| line.trim_end_matches(SEPARATORS));
    if lines.next() != Some("---") {
        return Err(CwsError::InvalidManifest);
    }
    let mut fields = BTreeMap::new();
    let mut list_key: Option<String> = None;
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(item) = line.strip_prefix("  -")
            && (item.is_empty() || item.starts_with([' ', '\t']))
        {
            let key = list_key.as_ref().ok_or(CwsError::InvalidManifest)?;
            let value = parse_scalar(item)?;
            let item = match value {
                MetadataValue::String(value) => value,
                _ => item.trim_matches(cws_whitespace).to_owned(),
            };
            let Some(MetadataValue::List(items)) = fields.get_mut(key) else {
                return Err(CwsError::InvalidManifest);
            };
            items.push(item);
            continue;
        }
        if line.starts_with(cws_whitespace) || line.starts_with('-') {
            return Err(CwsError::InvalidManifest);
        }
        let (key, value) = line.split_once(':').ok_or(CwsError::InvalidManifest)?;
        if key.is_empty() || fields.contains_key(key) {
            return Err(CwsError::InvalidManifest);
        }
        let parsed = parse_scalar(value)?;
        if value.trim_matches(cws_whitespace).is_empty() {
            fields.insert(key.to_owned(), MetadataValue::List(Vec::new()));
            list_key = Some(key.to_owned());
        } else {
            fields.insert(key.to_owned(), parsed);
            list_key = None;
        }
    }
    if !closed {
        return Err(CwsError::InvalidManifest);
    }
    for value in fields.values_mut() {
        if matches!(value, MetadataValue::List(items) if items.is_empty()) {
            *value = MetadataValue::String(String::new());
        }
    }
    Ok(fields)
}

fn cws_whitespace(ch: char) -> bool {
    // Python str.isspace additionally includes the ASCII information separators.
    ch.is_whitespace() || matches!(ch, '\u{1c}'..='\u{1f}')
}

fn parse_scalar(raw: &str) -> Result<MetadataValue, CwsError> {
    let value = raw.trim_matches(cws_whitespace);
    if let Some(rest) = value.strip_prefix(['>', '|']) {
        let rest = rest.strip_prefix(['+', '-']).unwrap_or(rest);
        let rest = rest.trim_start_matches(|ch: char| ch.is_ascii_digit());
        if rest.is_empty() || rest.starts_with(cws_whitespace) {
            return Err(CwsError::InvalidManifest);
        }
    }
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value)
            .map(MetadataValue::String)
            .map_err(|_| CwsError::InvalidManifest);
    }
    if let Some(rest) = value.strip_prefix('\'') {
        return rest
            .strip_suffix('\'')
            .map(|text| MetadataValue::String(text.replace("''", "'")))
            .ok_or(CwsError::InvalidManifest);
    }
    if value.starts_with(['&', '*', '!', '{', '[']) {
        return Err(CwsError::InvalidManifest);
    }
    if matches!(value, "true" | "false") {
        // Invoke the existing YAML scalar parser only after lexical restriction;
        // null, dates, floats, and YAML-only spellings must remain CWS strings.
        return serde_yaml_ng::from_str::<bool>(value)
            .map(MetadataValue::Bool)
            .map_err(|_| CwsError::InvalidManifest);
    }
    let digits = value.strip_prefix('-').unwrap_or(value);
    if !digits.is_empty()
        && digits.bytes().all(|ch| ch.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
    {
        return Ok(MetadataValue::Integer(value.to_owned()));
    }
    Ok(MetadataValue::String(value.to_owned()))
}

pub(crate) fn metadata(path: &Path) -> Result<Option<fs::Metadata>, CwsError> {
    match fs::symlink_metadata(path) {
        Ok(info) => Ok(Some(info)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn is_manifest(path: &Path) -> Result<bool, CwsError> {
    Ok(metadata(path)?.is_some_and(|info| info.is_file() && !info.file_type().is_symlink()))
}

fn ensure_no_links(path: &Path) -> Result<(), CwsError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if metadata(&current)?.is_some_and(|info| info.file_type().is_symlink()) {
            return Err(CwsError::UnsafePath);
        }
    }
    Ok(())
}

// Resolve only the spelling of the root; validating the unchanged suffix keeps
// links inside the project forbidden, including when the selected leaf is absent.
fn project_relative_path(project: &CwsProject, path: &Path) -> Result<PathBuf, CwsError> {
    if path.components().any(|part| part == Component::ParentDir) {
        return Err(CwsError::UnsafePath);
    }
    let mut relative = None;
    for ancestor in path.ancestors() {
        match ancestor.canonicalize() {
            Ok(root) if root == project.root => {
                if metadata(ancestor)?.is_some_and(|info| info.file_type().is_symlink()) {
                    continue;
                }
                let suffix = path
                    .strip_prefix(ancestor)
                    .map_err(|_| CwsError::UnsafePath)?;
                relative = Some(suffix.to_owned());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let relative = relative.ok_or(CwsError::UnsafePath)?;
    safe_project_path(project, &relative)?;
    Ok(relative)
}

/// Validate a single relative path without enumerating private or opaque trees.
pub(crate) fn safe_project_path(
    project: &CwsProject,
    relative: &Path,
) -> Result<PathBuf, CwsError> {
    let text = relative.to_str().ok_or(CwsError::UnsafePath)?;
    if text.contains('\\')
        || text.contains(':')
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(CwsError::UnsafePath);
    }
    ensure_no_links(&project.root)?;
    let mut current = project.root.clone();
    for component in relative.components() {
        if component == Component::CurDir {
            continue;
        }
        current.push(component);
        if let Some(info) = metadata(&current)? {
            if info.file_type().is_symlink() {
                return Err(CwsError::UnsafePath);
            }
            if info.is_dir() && is_manifest(&current.join("project.md"))? {
                return Err(CwsError::UnsafePath);
            }
        }
    }
    Ok(current)
}

/// Classify only exact contract paths; unknown files stay opaque.
pub fn classify(project: &CwsProject, relative: &Path) -> Result<DocumentRole, CwsError> {
    if !matches!(project.schema_version, 1 | 2) {
        return Err(CwsError::UnsupportedSchema(project.schema_version));
    }
    let target = safe_project_path(project, relative)?;
    if metadata(&target)?.is_some_and(|info| !info.is_file()) {
        return Ok(DocumentRole::Opaque);
    }
    let parts: Vec<_> = relative
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    if parts.first() == Some(&".creative-writing") {
        return Ok(DocumentRole::PrivateState);
    }
    match parts.as_slice() {
        ["AGENTS.md"] => return Ok(DocumentRole::Instructions),
        ["project.md"] => return Ok(DocumentRole::Manifest),
        _ => {}
    }
    if project.schema_version == 2
        && let Some(role) = translation_role(project, &parts)
    {
        return Ok(role);
    }
    let Some(name) = parts.last() else {
        return Ok(DocumentRole::Opaque);
    };
    let markdown = name.len() > 3 && name.ends_with(".md") && *name != "_index.md";
    let directory = parts[..parts.len() - 1].join("/");
    let prose = matches!(directory.as_str(), "story/chapters" | "story/side-stories");
    let work = matches!(
        directory.as_str(),
        "work/archive" | "work/brainstorm" | "work/drafts" | "work/plans" | "work/reviews"
    );
    let knowledge = matches!(
        directory.as_str(),
        "kb/continuity/scenes"
            | "kb/canon"
            | "kb/characters"
            | "kb/issues"
            | "kb/samples"
            | "kb/styles"
            | "kb/world"
    );
    if *name == "_index.md"
        && (prose
            || work
            || knowledge
            || matches!(
                directory.as_str(),
                "kb" | "kb/continuity" | "story" | "work"
            ))
    {
        return Ok(DocumentRole::Derived);
    }
    if markdown {
        if prose {
            return Ok(DocumentRole::AcceptedProse);
        }
        if work {
            return Ok(DocumentRole::Work);
        }
        if knowledge
            || matches!(
                parts.as_slice(),
                ["kb", "vocab.md"]
                    | [
                        "kb",
                        "continuity",
                        "promises.md" | "questions.md" | "state.md" | "timeline.md"
                    ]
            )
        {
            return Ok(DocumentRole::Knowledge);
        }
    }
    Ok(DocumentRole::Opaque)
}

fn translation_role(project: &CwsProject, parts: &[&str]) -> Option<DocumentRole> {
    if parts.last()?.len() <= 3 || !parts.last()?.ends_with(".md") || parts.contains(&"originals") {
        return None;
    }
    match parts {
        ["sources" | "translations", "_index.md"]
        | ["kb", "entities" | "source-comparisons", "_index.md"]
        | ["translations", _, "_index.md"] => return Some(DocumentRole::Derived),
        ["kb", "entities", _] => return Some(DocumentRole::Entity),
        ["kb", "source-comparisons", _] => return Some(DocumentRole::Alignment),
        ["sources", _, "edition.md"] => return Some(DocumentRole::SourceEdition),
        ["sources", _, "text", _] if project.work_kind == "book" => {
            return Some(DocumentRole::SourceUnit);
        }
        ["sources", _, "volumes", _, "text", _] if project.work_kind == "series" => {
            return Some(DocumentRole::SourceUnit);
        }
        ["translations", _, "translation.md"] => return Some(DocumentRole::TranslationDirection),
        ["translations", _, "memory", "style.md"]
        | [
            "translations",
            _,
            "memory",
            "terms" | "voices" | "decisions",
            _,
        ] => return Some(DocumentRole::TranslationMemory),
        ["translations", _, "volumes", _, "settings.md"] if project.work_kind == "series" => {
            return Some(DocumentRole::DirectionSettings);
        }
        _ => {}
    }
    let kind = match parts {
        ["translations", _, kind, _] if project.work_kind == "book" => *kind,
        ["translations", _, "volumes", _, kind, _] if project.work_kind == "series" => *kind,
        _ => return None,
    };
    match kind {
        "drafts" => Some(DocumentRole::Draft),
        "reviews" => Some(DocumentRole::Review),
        "accepted" => Some(DocumentRole::AcceptedProse),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cws_frontmatter_values_preserve_scalar_and_list_semantics() {
        let fields = parse_frontmatter(b"---\nempty:\nquoted-empty: \"\"\nnumber: -0\nhuge: 123456789012345678901234567890\nboolean: false\nnull: null\nfloat: 1.5\nleading: 01\nlist:\n  - true\n  - -0\n  - \"42\"\n  - user: wording\n---\n[body: ignored").unwrap();
        assert_eq!(fields["empty"], MetadataValue::String(String::new()));
        assert_eq!(fields["quoted-empty"], fields["empty"]);
        assert_eq!(fields["number"], MetadataValue::Integer("-0".into()));
        assert_eq!(
            fields["huge"],
            MetadataValue::Integer("123456789012345678901234567890".into())
        );
        let unicode_lines = parse_frontmatter(
            "---\nfirst: 日本語\u{2028}second: value\u{1c}third: end\n---\n".as_bytes(),
        )
        .unwrap();
        assert_eq!(
            unicode_lines["first"],
            MetadataValue::String("日本語".into())
        );
        assert_eq!(
            unicode_lines["second"],
            MetadataValue::String("value".into())
        );
        assert_eq!(unicode_lines["third"], MetadataValue::String("end".into()));
        assert_eq!(fields["boolean"], MetadataValue::Bool(false));
        for (key, value) in [("null", "null"), ("float", "1.5"), ("leading", "01")] {
            assert_eq!(fields[key], MetadataValue::String(value.into()));
        }
        assert_eq!(
            fields["list"],
            MetadataValue::List(vec![
                "true".into(),
                "-0".into(),
                "42".into(),
                "user: wording".into()
            ])
        );
    }
}
