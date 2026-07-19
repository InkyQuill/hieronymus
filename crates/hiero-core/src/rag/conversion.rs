use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use quick_xml::{Reader, events::Event};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use sha2::{Digest, Sha256};

use super::{NormalizedRagSource, RagError, SourceType, parsing::MAX_RAG_FILE_BYTES};

const MAX_DOCX_XML_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_CONVERTED_TEXT_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct PreparedRagSource {
    pub normalized: NormalizedRagSource,
    pub bytes: Vec<u8>,
}

struct SourceSnapshot {
    path: PathBuf,
    bytes: Vec<u8>,
    checksum: String,
}

impl SourceSnapshot {
    fn capture(path: &Path) -> Result<Self, RagError> {
        let path = fs::canonicalize(path).map_err(|source| RagError::Io {
            path: path.to_owned(),
            source,
        })?;
        let file = fs::File::open(&path).map_err(|source| RagError::Io {
            path: path.clone(),
            source,
        })?;
        let metadata = file.metadata().map_err(|source| RagError::Io {
            path: path.clone(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(RagError::NotAFile(path));
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(metadata.len().min(MAX_RAG_FILE_BYTES)).unwrap_or_default(),
        );
        file.take(MAX_RAG_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| RagError::Io {
                path: path.clone(),
                source,
            })?;
        if bytes.len() as u64 > MAX_RAG_FILE_BYTES {
            return Err(RagError::ResourceLimit {
                resource: "source bytes",
                limit: MAX_RAG_FILE_BYTES as usize,
            });
        }
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        Ok(Self {
            path,
            bytes,
            checksum,
        })
    }
}

pub fn normalize_rag_source(
    path: &Path,
    managed_root: &Path,
) -> Result<NormalizedRagSource, RagError> {
    Ok(prepare_rag_source(path, managed_root)?.normalized)
}

pub(crate) fn prepare_rag_source(
    path: &Path,
    managed_root: &Path,
) -> Result<PreparedRagSource, RagError> {
    normalize_snapshot(SourceSnapshot::capture(path)?, managed_root)
}

fn normalize_snapshot(
    snapshot: SourceSnapshot,
    managed_root: &Path,
) -> Result<PreparedRagSource, RagError> {
    let suffix = snapshot
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let direct = match suffix.as_str() {
        "txt" => Some((SourceType::Text, "text")),
        "md" | "markdown" => Some((SourceType::Markdown, "markdown")),
        "csv" => Some((SourceType::Csv, "glossary")),
        "tsv" => Some((SourceType::Tsv, "glossary")),
        "json" => Some((SourceType::Json, "glossary")),
        "yaml" | "yml" => Some((SourceType::Yaml, "glossary")),
        _ => None,
    };
    if let Some((source_type, format)) = direct {
        return Ok(PreparedRagSource {
            normalized: NormalizedRagSource {
                path: snapshot.path.clone(),
                original_path: snapshot.path,
                source_type,
                format: format.into(),
            },
            bytes: snapshot.bytes,
        });
    }
    if suffix == "epub" {
        return Err(RagError::UnsupportedEpub);
    }
    let markdown = match suffix.as_str() {
        "html" | "htm" => html_to_markdown_bounded(
            std::str::from_utf8(&snapshot.bytes)
                .map_err(|error| parse_error(&snapshot.path, error))?,
            MAX_CONVERTED_TEXT_BYTES,
        )?,
        "docx" => docx_to_markdown(&snapshot.bytes, &snapshot.path)?,
        "pdf" => pdf_to_markdown(&snapshot.bytes, &snapshot.path)?,
        _ => return Err(RagError::UnsupportedExtension(snapshot.path.clone())),
    };
    write_managed(snapshot, managed_root, &markdown)
}

fn pdf_to_markdown(bytes: &[u8], path: &Path) -> Result<String, RagError> {
    let mut document = lopdf::Document::load_mem(bytes).map_err(|error| RagError::Parse {
        path: Some(path.to_owned()),
        message: error.to_string(),
    })?;
    if document.is_encrypted() {
        document.decrypt("").map_err(|error| RagError::Parse {
            path: Some(path.to_owned()),
            message: error.to_string(),
        })?;
    }
    let mut output = BoundedWriter::new(MAX_CONVERTED_TEXT_BYTES);
    let result = {
        let mut device = pdf_extract::PlainTextOutput::new(&mut output as &mut dyn std::io::Write);
        pdf_extract::output_doc(&document, &mut device)
    };
    if output.exceeded {
        return Err(RagError::ResourceLimit {
            resource: "converted text bytes",
            limit: MAX_CONVERTED_TEXT_BYTES,
        });
    }
    result.map_err(|error| RagError::Parse {
        path: Some(path.to_owned()),
        message: error.to_string(),
    })?;
    Ok(output.value)
}

struct BoundedWriter {
    value: String,
    limit: usize,
    exceeded: bool,
}
impl BoundedWriter {
    fn new(limit: usize) -> Self {
        Self {
            value: String::new(),
            limit,
            exceeded: false,
        }
    }
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.value.len().saturating_add(bytes.len()) > self.limit {
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::FileTooLarge,
                "converted text limit",
            ));
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        self.value.push_str(text);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn write_managed(
    snapshot: SourceSnapshot,
    root: &Path,
    markdown: &str,
) -> Result<PreparedRagSource, RagError> {
    let trimmed = markdown.trim();
    if trimmed.len().saturating_add(1) > MAX_CONVERTED_TEXT_BYTES {
        return Err(RagError::ResourceLimit {
            resource: "converted text bytes",
            limit: MAX_CONVERTED_TEXT_BYTES,
        });
    }
    let normalized = format!("{trimmed}\n");
    if normalized.trim().is_empty() {
        return Err(RagError::NoExtractableText(snapshot.path));
    }
    let root = prepare_managed_root(root)?;
    let path = root.join(format!("{}.md", snapshot.checksum));
    reject_symlink(&path)?;
    if path.exists() {
        verify_managed_artifact(&path, normalized.as_bytes())?;
    } else {
        let mut temporary =
            tempfile::NamedTempFile::new_in(&root).map_err(|source| RagError::Io {
                path: root.clone(),
                source,
            })?;
        temporary
            .write_all(normalized.as_bytes())
            .and_then(|()| temporary.flush())
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|source| RagError::Io {
                path: temporary.path().to_owned(),
                source,
            })?;
        match temporary.persist_noclobber(&path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                verify_managed_artifact(&path, normalized.as_bytes())?
            }
            Err(error) => {
                return Err(RagError::Io {
                    path: path.clone(),
                    source: error.error,
                });
            }
        }
    }
    Ok(PreparedRagSource {
        normalized: NormalizedRagSource {
            path,
            original_path: snapshot.path,
            source_type: SourceType::Markdown,
            format: "markdown".into(),
        },
        bytes: normalized.into_bytes(),
    })
}

fn prepare_managed_root(root: &Path) -> Result<PathBuf, RagError> {
    reject_symlink_components(root)?;
    fs::create_dir_all(root).map_err(|source| RagError::Io {
        path: root.to_owned(),
        source,
    })?;
    reject_symlink_components(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(|source| {
            RagError::Io {
                path: root.to_owned(),
                source,
            }
        })?;
    }
    fs::canonicalize(root).map_err(|source| RagError::Io {
        path: root.to_owned(),
        source,
    })
}

fn reject_symlink_components(path: &Path) -> Result<(), RagError> {
    for component in path.ancestors() {
        if component.exists()
            && fs::symlink_metadata(component)
                .map_err(|source| RagError::Io {
                    path: component.to_owned(),
                    source,
                })?
                .file_type()
                .is_symlink()
        {
            return Err(RagError::UnsafeManagedPath(component.to_owned()));
        }
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), RagError> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_symlink()
                || !metadata.is_file() && path.extension().is_some() =>
        {
            Err(RagError::UnsafeManagedPath(path.to_owned()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(RagError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn verify_managed_artifact(path: &Path, expected: &[u8]) -> Result<(), RagError> {
    reject_symlink(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).map_err(|source| RagError::Io {
        path: path.to_owned(),
        source,
    })?;
    let metadata = file.metadata().map_err(|source| RagError::Io {
        path: path.to_owned(),
        source,
    })?;
    #[cfg(windows)]
    let is_reparse_point = {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    };
    #[cfg(not(windows))]
    let is_reparse_point = false;
    if !metadata.is_file() || is_reparse_point {
        return Err(RagError::UnsafeManagedPath(path.to_owned()));
    }
    let mut actual = Vec::with_capacity(expected.len().saturating_add(1));
    file.take(expected.len() as u64 + 1)
        .read_to_end(&mut actual)
        .map_err(|source| RagError::Io {
            path: path.to_owned(),
            source,
        })?;
    if actual != expected {
        return Err(RagError::ManagedArtifactMismatch(path.to_owned()));
    }
    Ok(())
}

fn html_to_markdown_bounded(input: &str, limit: usize) -> Result<String, RagError> {
    let document = Html::parse_document(input);
    let mut output = String::new();
    for node in document.tree.root().descendants() {
        let Some(element) = ElementRef::wrap(node) else {
            continue;
        };
        let name = element.value().name();
        if !matches!(
            name,
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "li" | "pre" | "table"
        ) {
            continue;
        }
        if element
            .ancestors()
            .skip(1)
            .filter_map(ElementRef::wrap)
            .any(|ancestor| matches!(ancestor.value().name(), "li" | "table"))
        {
            continue;
        }
        if name == "table" {
            table_markdown_bounded(&element, &mut output, limit)?;
            continue;
        }
        let remaining = limit.saturating_sub(output.len());
        let text = inline_markdown_bounded(&element.inner_html(), remaining)
            .map_err(|error| normalize_converted_limit(error, limit))?;
        if text.is_empty() {
            continue;
        }
        if !output.is_empty() {
            push_bounded_with_limit(&mut output, "\n\n", limit)?;
        }
        let (prefix, suffix) = match name {
            "h1" => ("# ", ""),
            "h2" => ("## ", ""),
            "h3" => ("### ", ""),
            "h4" => ("#### ", ""),
            "h5" => ("##### ", ""),
            "h6" => ("###### ", ""),
            "li" => ("- ", ""),
            "pre" => ("```\n", "\n```"),
            _ => ("", ""),
        };
        push_bounded_with_limit(&mut output, prefix, limit)?;
        push_bounded_with_limit(&mut output, &text, limit)?;
        push_bounded_with_limit(&mut output, suffix, limit)?;
    }
    Ok(output)
}

fn inline_markdown_bounded(html: &str, limit: usize) -> Result<String, RagError> {
    static LINK: OnceLock<Regex> = OnceLock::new();
    static STRONG: OnceLock<Regex> = OnceLock::new();
    static EMPHASIS: OnceLock<Regex> = OnceLock::new();
    static BREAK: OnceLock<Regex> = OnceLock::new();
    let mut value = LINK
        .get_or_init(|| {
            Regex::new(r#"(?is)<a[^>]*href=['\"]([^'\"]+)['\"][^>]*>(.*?)</a>"#)
                .expect("constant regex")
        })
        .replace_all(html, "[$2]($1)")
        .into_owned();
    ensure_converted_limit(value.len(), limit)?;
    value = STRONG
        .get_or_init(|| Regex::new(r"(?is)</?(?:strong|b)[^>]*>").expect("constant regex"))
        .replace_all(&value, "**")
        .into_owned();
    ensure_converted_limit(value.len(), limit)?;
    value = EMPHASIS
        .get_or_init(|| Regex::new(r"(?is)</?(?:em|i)[^>]*>").expect("constant regex"))
        .replace_all(&value, "*")
        .into_owned();
    ensure_converted_limit(value.len(), limit)?;
    value = BREAK
        .get_or_init(|| Regex::new(r"(?is)<br\s*/?>").expect("constant regex"))
        .replace_all(&value, "\n")
        .into_owned();
    ensure_converted_limit(value.len(), limit)?;
    let fragment = Html::parse_fragment(&value);
    let mut output = String::new();
    for text in fragment.root_element().text() {
        for word in text.split_whitespace() {
            if !output.is_empty() {
                push_bounded_with_limit(&mut output, " ", limit)?;
            }
            push_bounded_with_limit(&mut output, word, limit)?;
        }
    }
    Ok(output)
}

fn table_markdown_bounded(
    table: &ElementRef<'_>,
    output: &mut String,
    limit: usize,
) -> Result<(), RagError> {
    let row_selector = Selector::parse("tr").expect("constant selector");
    let cell_selector = Selector::parse("th, td").expect("constant selector");
    let mut wrote_row = false;
    for row in table.select(&row_selector) {
        let mut cells = row.select(&cell_selector).peekable();
        if cells.peek().is_none() {
            continue;
        }
        if !wrote_row {
            if !output.is_empty() {
                push_bounded_with_limit(output, "\n\n", limit)?;
            }
        } else {
            push_bounded_with_limit(output, "\n", limit)?;
        }
        push_bounded_with_limit(output, "| ", limit)?;
        let mut cell_count = 0;
        for cell in cells {
            if cell_count > 0 {
                push_bounded_with_limit(output, " | ", limit)?;
            }
            let remaining = limit.saturating_sub(output.len());
            let text = inline_markdown_bounded(&cell.inner_html(), remaining)
                .map_err(|error| normalize_converted_limit(error, limit))?;
            push_bounded_with_limit(output, &text, limit)?;
            cell_count += 1;
        }
        push_bounded_with_limit(output, " |", limit)?;
        if !wrote_row {
            push_bounded_with_limit(output, "\n| ", limit)?;
            for index in 0..cell_count {
                if index > 0 {
                    push_bounded_with_limit(output, " | ", limit)?;
                }
                push_bounded_with_limit(output, "---", limit)?;
            }
            push_bounded_with_limit(output, " |", limit)?;
        }
        wrote_row = true;
    }
    Ok(())
}

fn ensure_converted_limit(length: usize, limit: usize) -> Result<(), RagError> {
    if length > limit {
        return Err(RagError::ResourceLimit {
            resource: "converted text bytes",
            limit,
        });
    }
    Ok(())
}

fn normalize_converted_limit(error: RagError, limit: usize) -> RagError {
    match error {
        RagError::ResourceLimit {
            resource: "converted text bytes",
            ..
        } => RagError::ResourceLimit {
            resource: "converted text bytes",
            limit,
        },
        error => error,
    }
}

fn docx_to_markdown(bytes: &[u8], path: &Path) -> Result<String, RagError> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| RagError::Parse {
            path: Some(path.to_owned()),
            message: error.to_string(),
        })?;
    let relationships = {
        let xml = if let Ok(file) = archive.by_name("word/_rels/document.xml.rels") {
            let size = file.size();
            read_docx_xml_bounded(
                file,
                size,
                MAX_DOCX_XML_BYTES as usize,
                "DOCX relationships XML bytes",
                path,
            )?
        } else {
            String::new()
        };
        parse_docx_relationships(&xml, path)?
    };
    let document = archive
        .by_name("word/document.xml")
        .map_err(|error| RagError::Parse {
            path: Some(path.to_owned()),
            message: error.to_string(),
        })?;
    let size = document.size();
    let xml = read_docx_xml_bounded(
        document,
        size,
        MAX_DOCX_XML_BYTES as usize,
        "DOCX document XML bytes",
        path,
    )?;
    docx_document_to_markdown(&xml, &relationships, path)
}

fn read_docx_xml_bounded(
    reader: impl Read,
    declared_size: u64,
    limit: usize,
    resource: &'static str,
    path: &Path,
) -> Result<String, RagError> {
    if declared_size > limit as u64 {
        return Err(RagError::ResourceLimit { resource, limit });
    }
    let mut xml = String::with_capacity(usize::try_from(declared_size).unwrap_or(limit));
    reader
        .take(limit as u64 + 1)
        .read_to_string(&mut xml)
        .map_err(|source| RagError::Io {
            path: path.to_owned(),
            source,
        })?;
    if xml.len() > limit {
        return Err(RagError::ResourceLimit { resource, limit });
    }
    Ok(xml)
}

fn docx_document_to_markdown(
    xml: &str,
    relationships: &std::collections::HashMap<String, String>,
    path: &Path,
) -> Result<String, RagError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut output = String::new();
    let mut style = String::new();
    let mut paragraph_started = false;
    let mut bold = false;
    let mut italic = false;
    let mut hyperlink = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) if start.name().as_ref() == b"w:p" => {
                style.clear();
                paragraph_started = false;
            }
            Ok(Event::Start(start)) if start.name().as_ref() == b"w:hyperlink" => {
                hyperlink = attribute(&start, b"r:id", &reader)
                    .and_then(|id| relationships.get(&id).cloned());
                if hyperlink.is_some() {
                    push_bounded(&mut output, "[", path)?;
                }
            }
            Ok(Event::Empty(empty)) if empty.name().as_ref() == b"w:pStyle" => {
                style = attribute(&empty, b"w:val", &reader).unwrap_or_default();
            }
            Ok(Event::Empty(empty)) if empty.name().as_ref() == b"w:b" => bold = true,
            Ok(Event::Empty(empty)) if empty.name().as_ref() == b"w:i" => italic = true,
            Ok(Event::Empty(empty)) if empty.name().as_ref() == b"w:tab" => {
                push_bounded(&mut output, "\t", path)?
            }
            Ok(Event::Empty(empty)) if empty.name().as_ref() == b"w:br" => {
                push_bounded(&mut output, "\n", path)?
            }
            Ok(Event::Start(start)) if start.name().as_ref() == b"w:tr" => {
                push_bounded(&mut output, "| ", path)?
            }
            Ok(Event::Text(text)) => {
                let text = text.decode().map_err(|error| RagError::Parse {
                    path: Some(path.to_owned()),
                    message: error.to_string(),
                })?;
                if !paragraph_started {
                    if let Some(level) = style
                        .strip_prefix("Heading")
                        .and_then(|value| value.parse::<usize>().ok())
                        .filter(|level| (1..=6).contains(level))
                    {
                        push_bounded(&mut output, &format!("{} ", "#".repeat(level)), path)?;
                    }
                    paragraph_started = true;
                }
                if bold {
                    push_bounded(&mut output, "**", path)?;
                }
                if italic {
                    push_bounded(&mut output, "*", path)?;
                }
                push_bounded(&mut output, &text, path)?;
                if italic {
                    push_bounded(&mut output, "*", path)?;
                }
                if bold {
                    push_bounded(&mut output, "**", path)?;
                }
            }
            Ok(Event::End(end)) if end.name().as_ref() == b"w:r" => {
                bold = false;
                italic = false;
            }
            Ok(Event::End(end)) if end.name().as_ref() == b"w:hyperlink" => {
                if let Some(target) = hyperlink.take() {
                    push_bounded(&mut output, &format!("]({target})"), path)?;
                }
            }
            Ok(Event::End(end)) if end.name().as_ref() == b"w:tc" => {
                push_bounded(&mut output, " | ", path)?
            }
            Ok(Event::End(end)) if end.name().as_ref() == b"w:tr" => {
                push_bounded(&mut output, "\n", path)?
            }
            Ok(Event::End(end)) if end.name().as_ref() == b"w:p" => {
                push_bounded(&mut output, "\n\n", path)?
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(RagError::Parse {
                    path: Some(path.to_owned()),
                    message: error.to_string(),
                });
            }
            _ => {}
        }
    }
    Ok(output)
}

fn parse_docx_relationships(
    xml: &str,
    path: &Path,
) -> Result<std::collections::HashMap<String, String>, RagError> {
    let mut relationships = std::collections::HashMap::new();
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Empty(element)) | Ok(Event::Start(element))
                if element.name().as_ref() == b"Relationship" =>
            {
                if let (Some(id), Some(target)) = (
                    attribute(&element, b"Id", &reader),
                    attribute(&element, b"Target", &reader),
                ) {
                    relationships.insert(id, target);
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(RagError::Parse {
                    path: Some(path.to_owned()),
                    message: error.to_string(),
                });
            }
            _ => {}
        }
    }
    Ok(relationships)
}

fn attribute(
    element: &quick_xml::events::BytesStart<'_>,
    key: &[u8],
    reader: &Reader<&[u8]>,
) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|attribute| attribute.key.as_ref() == key)
        .and_then(|attribute| {
            attribute
                .decode_and_unescape_value(reader.decoder())
                .ok()
                .map(|value| value.into_owned())
        })
}

fn push_bounded(output: &mut String, text: &str, _path: &Path) -> Result<(), RagError> {
    push_bounded_with_limit(output, text, MAX_CONVERTED_TEXT_BYTES)
}

fn push_bounded_with_limit(output: &mut String, text: &str, limit: usize) -> Result<(), RagError> {
    if output.len().saturating_add(text.len()) > limit {
        return Err(RagError::ResourceLimit {
            resource: "converted text bytes",
            limit,
        });
    }
    output.push_str(text);
    Ok(())
}

fn parse_error(path: &Path, error: impl std::fmt::Display) -> RagError {
    RagError::Parse {
        path: Some(path.to_owned()),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn bounded_writer_rejects_before_allocating_beyond_limit() {
        let mut writer = BoundedWriter::new(8);
        writer.write_all(b"12345678").unwrap();
        assert!(writer.write_all(b"9").is_err());
        assert_eq!(writer.value.len(), 8);
        assert!(writer.exceeded);
    }

    #[test]
    fn captured_snapshot_remains_self_consistent_after_source_replacement() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.html");
        fs::write(&source, "<p>first snapshot</p>").unwrap();
        let snapshot = SourceSnapshot::capture(&source).unwrap();
        fs::write(&source, "<p>replacement</p>").unwrap();

        let prepared = normalize_snapshot(snapshot, &dir.path().join("managed")).unwrap();
        assert_eq!(
            prepared
                .normalized
                .path
                .file_stem()
                .unwrap()
                .to_string_lossy(),
            format!("{:x}", Sha256::digest(b"<p>first snapshot</p>"))
        );
        assert_eq!(prepared.bytes, b"first snapshot\n");
        assert_eq!(fs::read(&prepared.normalized.path).unwrap(), prepared.bytes);
        let parsed = crate::rag::parsing::load_rag_bytes(
            &prepared.normalized.path,
            &prepared.bytes,
            SourceType::Auto,
        )
        .unwrap();
        assert_eq!(
            parsed.checksum,
            format!("{:x}", Sha256::digest(&prepared.bytes))
        );
    }

    #[test]
    fn html_builder_stops_at_limit_before_returning_oversized_output() {
        assert!(matches!(
            html_to_markdown_bounded("<h1>one</h1><h1>two</h1>", 8),
            Err(RagError::ResourceLimit {
                resource: "converted text bytes",
                limit: 8,
            })
        ));
    }

    #[test]
    fn table_builder_stops_while_collecting_an_oversized_cell() {
        let document =
            Html::parse_fragment("<table><tr><td>abcdefghijklmnopqrstuvwxyz</td></tr></table>");
        let selector = Selector::parse("table").unwrap();
        let table = document.select(&selector).next().unwrap();
        let mut output = String::new();
        assert!(matches!(
            table_markdown_bounded(&table, &mut output, 16),
            Err(RagError::ResourceLimit {
                resource: "converted text bytes",
                limit: 16,
            })
        ));
        assert!(output.len() <= 16);
    }

    #[test]
    fn docx_relationships_member_checks_declared_and_actual_uncompressed_size() {
        let path = Path::new("fixture.docx");
        assert!(matches!(
            read_docx_xml_bounded(
                Cursor::new(b"tiny"),
                9,
                8,
                "DOCX relationships XML bytes",
                path
            ),
            Err(RagError::ResourceLimit {
                resource: "DOCX relationships XML bytes",
                limit: 8,
            })
        ));
        assert!(matches!(
            read_docx_xml_bounded(
                Cursor::new(b"123456789"),
                8,
                8,
                "DOCX relationships XML bytes",
                path
            ),
            Err(RagError::ResourceLimit {
                resource: "DOCX relationships XML bytes",
                limit: 8,
            })
        ));
    }
}
