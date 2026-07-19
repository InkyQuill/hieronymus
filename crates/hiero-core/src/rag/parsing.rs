use std::{collections::BTreeMap, fs, io::Read, path::Path};

use csv::StringRecord;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

use super::{
    RagError, SourceType,
    chunking::emit_chunk_text,
    models::{ParsedRagChunk, ParsedRagFile},
};

pub const MAX_RAG_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_RAG_CHUNKS: usize = 100_000;
pub const MAX_RAG_METADATA_BYTES: usize = 16 * 1024;

pub fn load_rag_file(path: &Path, requested: SourceType) -> Result<ParsedRagFile, RagError> {
    let file = fs::File::open(path).map_err(|source| RagError::Io {
        path: path.to_owned(),
        source,
    })?;
    let metadata = file.metadata().map_err(|source| RagError::Io {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(RagError::NotAFile(path.to_owned()));
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len().min(MAX_RAG_FILE_BYTES)).unwrap_or_default(),
    );
    file.take(MAX_RAG_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| RagError::Io {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() as u64 > MAX_RAG_FILE_BYTES {
        return Err(RagError::ResourceLimit {
            resource: "source bytes",
            limit: MAX_RAG_FILE_BYTES as usize,
        });
    }
    load_rag_bytes(path, &bytes, requested)
}

pub(crate) fn load_rag_bytes(
    path: &Path,
    bytes: &[u8],
    requested: SourceType,
) -> Result<ParsedRagFile, RagError> {
    if bytes.len() as u64 > MAX_RAG_FILE_BYTES {
        return Err(RagError::ResourceLimit {
            resource: "source bytes",
            limit: MAX_RAG_FILE_BYTES as usize,
        });
    }
    let source_type = resolve(path, requested)?;
    let content_type = extension(path)?;
    let chunks = match source_type {
        SourceType::Text => parse_text(decode(path, bytes)?)?,
        SourceType::Markdown => parse_markdown(decode(path, bytes)?)?,
        SourceType::Csv => parse_delimited(bytes, b',')?,
        SourceType::Tsv => parse_delimited(bytes, b'\t')?,
        SourceType::Json => parse_structured(
            serde_json::from_slice(bytes).map_err(|error| parse_error(path, error))?,
        )?,
        SourceType::Yaml => {
            let value: serde_json::Value =
                serde_yaml::from_slice(bytes).map_err(|error| parse_error(path, error))?;
            parse_structured(value)?
        }
        _ => return Err(RagError::UnsupportedExtension(path.to_owned())),
    };
    if chunks.is_empty() {
        return Err(RagError::NoExtractableText(path.to_owned()));
    }
    Ok(ParsedRagFile {
        path: path.to_owned(),
        source_type,
        content_type,
        checksum: format!("{:x}", Sha256::digest(bytes)),
        chunks,
        metadata: BTreeMap::new(),
    })
}

struct ChunkCollector {
    chunks: Vec<ParsedRagChunk>,
    limit: usize,
}

impl ChunkCollector {
    fn new(limit: usize) -> Self {
        Self {
            chunks: Vec::new(),
            limit,
        }
    }

    fn push(&mut self, chunk: ParsedRagChunk) -> Result<(), RagError> {
        let multiple = chunk.text.chars().count() > super::MAX_RAG_CHUNK_CHARS;
        let mut index = 0;
        emit_chunk_text(&chunk.text, |text| {
            if self.chunks.len() == self.limit {
                return Err(RagError::ResourceLimit {
                    resource: "chunks",
                    limit: self.limit,
                });
            }
            index += 1;
            self.chunks.push(ParsedRagChunk {
                chunk_kind: chunk.chunk_kind.clone(),
                display_text: text.clone(),
                text,
                location: if multiple {
                    format!("{} part {index}", chunk.location)
                } else {
                    chunk.location.clone()
                },
                metadata: chunk.metadata.clone(),
            });
            Ok(())
        })
    }
}

fn extension(path: &Path) -> Result<String, RagError> {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| RagError::UnsupportedExtension(path.to_owned()))
}

fn resolve(path: &Path, requested: SourceType) -> Result<SourceType, RagError> {
    let detected = match extension(path)?.as_str() {
        "txt" => SourceType::Text,
        "md" | "markdown" => SourceType::Markdown,
        "csv" => SourceType::Csv,
        "tsv" => SourceType::Tsv,
        "json" => SourceType::Json,
        "yaml" | "yml" => SourceType::Yaml,
        _ => return Err(RagError::UnsupportedExtension(path.to_owned())),
    };
    match requested {
        SourceType::Auto => Ok(detected),
        SourceType::Text if matches!(detected, SourceType::Text | SourceType::Markdown) => {
            Ok(detected)
        }
        SourceType::Csv | SourceType::Tsv | SourceType::Yaml | SourceType::Json
            if requested == detected =>
        {
            Ok(detected)
        }
        _ => Err(RagError::SourceTypeMismatch {
            requested,
            path: path.to_owned(),
        }),
    }
}

fn decode<'a>(path: &Path, bytes: &'a [u8]) -> Result<&'a str, RagError> {
    std::str::from_utf8(bytes).map_err(|error| parse_error(path, error))
}

fn parse_text(text: &str) -> Result<Vec<ParsedRagChunk>, RagError> {
    static BLANK: OnceLock<Regex> = OnceLock::new();
    let mut collector = ChunkCollector::new(MAX_RAG_CHUNKS);
    let mut paragraph = 0;
    for block in BLANK
        .get_or_init(|| Regex::new(r"\n\s*\n").expect("constant regex"))
        .split(text)
    {
        let text = block
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            continue;
        }
        paragraph += 1;
        collector.push(ParsedRagChunk {
            chunk_kind: "text".into(),
            display_text: text.clone(),
            text,
            location: format!("paragraph {paragraph}"),
            metadata: BTreeMap::new(),
        })?;
    }
    Ok(collector.chunks)
}

fn parse_markdown(text: &str) -> Result<Vec<ParsedRagChunk>, RagError> {
    let mut headings: Vec<(usize, String)> = Vec::new();
    let mut collector = ChunkCollector::new(MAX_RAG_CHUNKS);
    let mut lines = Vec::new();
    let mut count = 0;
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some((marker, length)) = fence {
            lines.push(line.to_owned());
            if line.len().saturating_sub(line.trim_start().len()) <= 3
                && trimmed.chars().all(|ch| ch == marker)
                && trimmed.chars().count() >= length
            {
                fence = None;
            }
            continue;
        }
        let leading = line.len().saturating_sub(line.trim_start().len());
        if leading <= 3 && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
            let marker = trimmed.chars().next().expect("checked marker");
            fence = Some((
                marker,
                trimmed.chars().take_while(|ch| *ch == marker).count(),
            ));
            lines.push(line.to_owned());
            continue;
        }
        if line.starts_with("    ") || line.starts_with('\t') {
            lines.push(line.to_owned());
            continue;
        }
        let level = trimmed.chars().take_while(|ch| *ch == '#').count();
        if (1..=6).contains(&level) && trimmed.chars().nth(level) == Some(' ') {
            flush_markdown(&mut lines, &headings, &mut count, &mut collector)?;
            headings.retain(|(old, _)| *old < level);
            headings.push((
                level,
                trimmed[level + 1..].trim_end_matches('#').trim().to_owned(),
            ));
        } else if trimmed.is_empty() {
            flush_markdown(&mut lines, &headings, &mut count, &mut collector)?;
        } else {
            lines.push(line.to_owned());
        }
    }
    flush_markdown(&mut lines, &headings, &mut count, &mut collector)?;
    Ok(collector.chunks)
}

fn flush_markdown(
    lines: &mut Vec<String>,
    headings: &[(usize, String)],
    count: &mut usize,
    collector: &mut ChunkCollector,
) -> Result<(), RagError> {
    if lines.is_empty() {
        return Ok(());
    }
    let text = lines
        .drain(..)
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return Ok(());
    }
    *count += 1;
    let prefix = headings
        .iter()
        .map(|(_, heading)| heading.as_str())
        .collect::<Vec<_>>()
        .join(" > ");
    let location = if prefix.is_empty() {
        format!("paragraph {count}")
    } else {
        format!("{prefix} paragraph {count}")
    };
    collector.push(ParsedRagChunk {
        chunk_kind: "markdown_section".into(),
        display_text: text.clone(),
        text,
        location,
        metadata: BTreeMap::new(),
    })
}

fn parse_delimited(bytes: &[u8], delimiter: u8) -> Result<Vec<ParsedRagChunk>, RagError> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|error| RagError::Parse {
            path: None,
            message: error.to_string(),
        })?
        .clone();
    validate_headers(&headers)?;
    let mut collector = ChunkCollector::new(MAX_RAG_CHUNKS);
    for (index, row) in reader.records().enumerate() {
        let row = row.map_err(|error| RagError::Parse {
            path: None,
            message: error.to_string(),
        })?;
        if row.iter().all(|value| value.trim().is_empty()) {
            continue;
        }
        collector.push(row_to_chunk(&headers, &row, format!("row {}", index + 2))?)?;
    }
    Ok(collector.chunks)
}

fn validate_headers(headers: &StringRecord) -> Result<(), RagError> {
    let clean: Vec<_> = headers.iter().map(str::trim).collect();
    if clean.is_empty()
        || clean.iter().any(|header| header.is_empty())
        || clean
            .iter()
            .enumerate()
            .any(|(i, header)| clean[..i].contains(header))
    {
        return Err(RagError::Parse {
            path: None,
            message: "delimited glossary requires unique non-empty headers".into(),
        });
    }
    Ok(())
}

fn row_to_chunk(
    headers: &StringRecord,
    row: &StringRecord,
    location: String,
) -> Result<ParsedRagChunk, RagError> {
    if headers.len() != row.len() {
        return Err(RagError::Parse {
            path: None,
            message: format!("malformed delimited {location}"),
        });
    }
    let metadata = headers
        .iter()
        .zip(row)
        .filter_map(|(key, value)| {
            let value = value.trim();
            (!value.is_empty()).then(|| {
                (
                    key.trim().to_owned(),
                    serde_json::Value::String(value.to_owned()),
                )
            })
        })
        .collect();
    glossary_chunk(metadata, location)
}

fn parse_structured(value: serde_json::Value) -> Result<Vec<ParsedRagChunk>, RagError> {
    let mut collector = ChunkCollector::new(MAX_RAG_CHUNKS);
    match value {
        serde_json::Value::Null => {}
        serde_json::Value::Array(values) => {
            for (index, value) in values.into_iter().enumerate() {
                collector.push(glossary_chunk(
                    value_metadata(value),
                    format!("entry {}", index + 1),
                )?)?;
            }
        }
        serde_json::Value::Object(values) => {
            for (index, (key, value)) in values.into_iter().enumerate() {
                let mut metadata = value_metadata(value);
                metadata.insert("key".into(), key.into());
                collector.push(glossary_chunk(metadata, format!("entry {}", index + 1))?)?;
            }
        }
        _ => {
            return Err(RagError::Parse {
                path: None,
                message: "glossary data must be a list or mapping".into(),
            });
        }
    }
    Ok(collector.chunks)
}

fn value_metadata(value: serde_json::Value) -> BTreeMap<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(values) => values
            .into_iter()
            .filter(|(_, value)| !value.is_null())
            .collect(),
        value => BTreeMap::from([("value".into(), value)]),
    }
}

fn glossary_chunk(
    metadata: BTreeMap<String, serde_json::Value>,
    location: String,
) -> Result<ParsedRagChunk, RagError> {
    if metadata.is_empty() {
        return Err(RagError::Parse {
            path: None,
            message: "empty glossary entry".into(),
        });
    }
    if retained_metadata_bytes(&metadata) > MAX_RAG_METADATA_BYTES {
        return Err(RagError::ResourceLimit {
            resource: "chunk metadata bytes",
            limit: MAX_RAG_METADATA_BYTES,
        });
    }
    let text = metadata
        .iter()
        .map(|(key, value)| {
            format!(
                "{key}: {}",
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_owned)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ParsedRagChunk {
        chunk_kind: "glossary_entry".into(),
        display_text: text.clone(),
        text,
        location,
        metadata,
    })
}

fn retained_metadata_bytes(metadata: &BTreeMap<String, serde_json::Value>) -> usize {
    let mut total: usize = 0;
    for (key, value) in metadata {
        total = total.saturating_add(key.len());
        total = total.saturating_add(retained_value_bytes(value));
        if total > MAX_RAG_METADATA_BYTES {
            break;
        }
    }
    total
}

fn retained_value_bytes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => 0,
        serde_json::Value::String(value) => value.len(),
        serde_json::Value::Array(values) => values.iter().fold(0usize, |total, value| {
            total.saturating_add(retained_value_bytes(value))
        }),
        serde_json::Value::Object(values) => values.iter().fold(0usize, |total, (key, value)| {
            total
                .saturating_add(key.len())
                .saturating_add(retained_value_bytes(value))
        }),
    }
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

    #[test]
    fn collector_enforces_limit_during_boundary_expansion() {
        let mut collector = ChunkCollector::new(2);
        let chunk = ParsedRagChunk {
            chunk_kind: "text".into(),
            text: "x".repeat(super::super::MAX_RAG_CHUNK_CHARS * 3),
            display_text: String::new(),
            location: "paragraph 1".into(),
            metadata: BTreeMap::new(),
        };
        assert!(matches!(
            collector.push(chunk),
            Err(RagError::ResourceLimit {
                resource: "chunks",
                limit: 2,
            })
        ));
        assert_eq!(collector.chunks.len(), 2);
    }
}
