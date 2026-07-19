use std::{fs, io::Read, path::Path};

use quick_xml::{Reader, events::Event};
use scraper::{ElementRef, Html};
use sha2::{Digest, Sha256};

use super::{NormalizedRagSource, RagError, SourceType, parsing::MAX_RAG_FILE_BYTES};

const MAX_DOCX_XML_BYTES: u64 = 64 * 1024 * 1024;

pub fn normalize_rag_source(
    path: &Path,
    managed_root: &Path,
) -> Result<NormalizedRagSource, RagError> {
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
            pdf_extract::extract_text(path).map_err(|error| RagError::Parse {
                path: Some(path.to_owned()),
                message: error.to_string(),
            })?
        }
        _ => return Err(RagError::UnsupportedExtension(path.to_owned())),
    };
    write_managed(path, managed_root, &markdown)
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
    fs::create_dir_all(root).map_err(|error| RagError::Io {
        path: root.to_owned(),
        source: error,
    })?;
    let path = root.join(format!("{:x}.md", Sha256::digest(bytes)));
    fs::write(&path, normalized).map_err(|error| RagError::Io {
        path: path.clone(),
        source: error,
    })?;
    Ok(NormalizedRagSource {
        path,
        original_path: source.to_owned(),
        source_type: SourceType::Markdown,
        format: "markdown".into(),
    })
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
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "li" | "pre"
        ) {
            continue;
        }
        let text = element
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
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
    loop {
        match reader.read_event() {
            Ok(Event::Text(text)) => {
                output.push_str(&text.decode().map_err(|error| RagError::Parse {
                    path: Some(path.to_owned()),
                    message: error.to_string(),
                })?)
            }
            Ok(Event::End(end)) if end.name().as_ref() == b"w:p" => output.push_str("\n\n"),
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
