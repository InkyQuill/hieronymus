use std::{
    fs,
    io::{Read, Write},
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

pub fn normalize_rag_source(
    path: &Path,
    managed_root: &Path,
) -> Result<NormalizedRagSource, RagError> {
    let path = fs::canonicalize(path).map_err(|source| RagError::Io {
        path: path.to_owned(),
        source,
    })?;
    let path = path.as_path();
    let suffix = path
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
        validate_file(path)?;
        return Ok(NormalizedRagSource {
            path: path.to_owned(),
            original_path: path.to_owned(),
            source_type,
            format: format.into(),
        });
    }
    if suffix == "epub" {
        return Err(RagError::UnsupportedEpub);
    }
    let markdown = match suffix.as_str() {
        "html" | "htm" => html_to_markdown(&read_bounded(path)?),
        "docx" => docx_to_markdown(path)?,
        "pdf" => {
            validate_file(path)?;
            pdf_to_markdown(path)?
        }
        _ => return Err(RagError::UnsupportedExtension(path.to_owned())),
    };
    write_managed(path, managed_root, &markdown)
}

fn pdf_to_markdown(path: &Path) -> Result<String, RagError> {
    let mut document = lopdf::Document::load(path).map_err(|error| RagError::Parse {
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

fn validate_file(path: &Path) -> Result<(), RagError> {
    let metadata = fs::metadata(path).map_err(|source| RagError::Io {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(RagError::NotAFile(path.to_owned()));
    }
    if metadata.len() > MAX_RAG_FILE_BYTES {
        return Err(RagError::ResourceLimit {
            resource: "source bytes",
            limit: MAX_RAG_FILE_BYTES as usize,
        });
    }
    Ok(())
}

fn read_bounded(path: &Path) -> Result<String, RagError> {
    validate_file(path)?;
    fs::read_to_string(path).map_err(|source| RagError::Io {
        path: path.to_owned(),
        source,
    })
}

fn write_managed(
    source: &Path,
    root: &Path,
    markdown: &str,
) -> Result<NormalizedRagSource, RagError> {
    let normalized = format!("{}\n", markdown.trim());
    if normalized.trim().is_empty() {
        return Err(RagError::NoExtractableText(source.to_owned()));
    }
    let bytes = fs::read(source).map_err(|error| RagError::Io {
        path: source.to_owned(),
        source: error,
    })?;
    let root = prepare_managed_root(root)?;
    let path = root.join(format!("{:x}.md", Sha256::digest(bytes)));
    reject_symlink(&path)?;
    if !path.exists() {
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
                reject_symlink(&path)?
            }
            Err(error) => {
                return Err(RagError::Io {
                    path: path.clone(),
                    source: error.error,
                });
            }
        }
    }
    Ok(NormalizedRagSource {
        path,
        original_path: source.to_owned(),
        source_type: SourceType::Markdown,
        format: "markdown".into(),
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

fn html_to_markdown(input: &str) -> String {
    let document = Html::parse_document(input);
    let mut blocks = Vec::new();
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
        let text = if name == "table" {
            table_markdown(&element)
        } else {
            inline_markdown(&element.inner_html())
        };
        if text.is_empty() {
            continue;
        }
        let block = match name {
            "h1" => format!("# {text}"),
            "h2" => format!("## {text}"),
            "h3" => format!("### {text}"),
            "h4" => format!("#### {text}"),
            "h5" => format!("##### {text}"),
            "h6" => format!("###### {text}"),
            "li" => format!("- {text}"),
            "pre" => format!("```\n{text}\n```"),
            _ => text,
        };
        blocks.push(block);
    }
    blocks.join("\n\n")
}

fn inline_markdown(html: &str) -> String {
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
    value = STRONG
        .get_or_init(|| Regex::new(r"(?is)</?(?:strong|b)[^>]*>").expect("constant regex"))
        .replace_all(&value, "**")
        .into_owned();
    value = EMPHASIS
        .get_or_init(|| Regex::new(r"(?is)</?(?:em|i)[^>]*>").expect("constant regex"))
        .replace_all(&value, "*")
        .into_owned();
    value = BREAK
        .get_or_init(|| Regex::new(r"(?is)<br\s*/?>").expect("constant regex"))
        .replace_all(&value, "\n")
        .into_owned();
    Html::parse_fragment(&value)
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn table_markdown(table: &ElementRef<'_>) -> String {
    let row_selector = Selector::parse("tr").expect("constant selector");
    let cell_selector = Selector::parse("th, td").expect("constant selector");
    let rows = table
        .select(&row_selector)
        .map(|row| {
            row.select(&cell_selector)
                .map(|cell| inline_markdown(&cell.inner_html()))
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return String::new();
    }
    let mut output = Vec::new();
    output.push(format!("| {} |", rows[0].join(" | ")));
    output.push(format!(
        "| {} |",
        rows[0]
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join(" | ")
    ));
    output.extend(
        rows.iter()
            .skip(1)
            .map(|row| format!("| {} |", row.join(" | "))),
    );
    output.join("\n")
}

fn docx_to_markdown(path: &Path) -> Result<String, RagError> {
    validate_file(path)?;
    let file = fs::File::open(path).map_err(|source| RagError::Io {
        path: path.to_owned(),
        source,
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| RagError::Parse {
        path: Some(path.to_owned()),
        message: error.to_string(),
    })?;
    let relationships = {
        let mut xml = String::new();
        if let Ok(file) = archive.by_name("word/_rels/document.xml.rels") {
            file.take(MAX_DOCX_XML_BYTES + 1)
                .read_to_string(&mut xml)
                .map_err(|source| RagError::Io {
                    path: path.to_owned(),
                    source,
                })?;
        }
        parse_docx_relationships(&xml, path)?
    };
    let document = archive
        .by_name("word/document.xml")
        .map_err(|error| RagError::Parse {
            path: Some(path.to_owned()),
            message: error.to_string(),
        })?;
    if document.size() > MAX_DOCX_XML_BYTES {
        return Err(RagError::ResourceLimit {
            resource: "DOCX document XML bytes",
            limit: MAX_DOCX_XML_BYTES as usize,
        });
    }
    let mut xml = String::new();
    document
        .take(MAX_DOCX_XML_BYTES + 1)
        .read_to_string(&mut xml)
        .map_err(|source| RagError::Io {
            path: path.to_owned(),
            source,
        })?;
    if xml.len() as u64 > MAX_DOCX_XML_BYTES {
        return Err(RagError::ResourceLimit {
            resource: "DOCX document XML bytes",
            limit: MAX_DOCX_XML_BYTES as usize,
        });
    }
    let mut reader = Reader::from_str(&xml);
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
    if output.len().saturating_add(text.len()) > MAX_CONVERTED_TEXT_BYTES {
        return Err(RagError::ResourceLimit {
            resource: "converted text bytes",
            limit: MAX_CONVERTED_TEXT_BYTES,
        });
    }
    output.push_str(text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_writer_rejects_before_allocating_beyond_limit() {
        let mut writer = BoundedWriter::new(8);
        writer.write_all(b"12345678").unwrap();
        assert!(writer.write_all(b"9").is_err());
        assert_eq!(writer.value.len(), 8);
        assert!(writer.exceeded);
    }
}
