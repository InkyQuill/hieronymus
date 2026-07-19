use std::{collections::BTreeMap, fs, path::Path};

use csv::StringRecord;
use sha2::{Digest, Sha256};

use super::{
    RagError, SourceType,
    models::{ParsedRagChunk, ParsedRagFile},
    split_chunk_text,
};

pub const MAX_RAG_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_RAG_CHUNKS: usize = 100_000;

pub fn load_rag_file(path: &Path, requested: SourceType) -> Result<ParsedRagFile, RagError> {
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
    let bytes = fs::read(path).map_err(|source| RagError::Io {
        path: path.to_owned(),
        source,
    })?;
    let source_type = resolve(path, requested)?;
    let content_type = extension(path)?;
    let chunks = match source_type {
        SourceType::Text => parse_text(decode(path, &bytes)?),
        SourceType::Markdown => parse_markdown(decode(path, &bytes)?),
        SourceType::Csv => parse_delimited(&bytes, b',')?,
        SourceType::Tsv => parse_delimited(&bytes, b'\t')?,
        SourceType::Json => parse_structured(
            serde_json::from_slice(&bytes).map_err(|error| parse_error(path, error))?,
        )?,
        SourceType::Yaml => {
            let value: serde_json::Value =
                serde_yaml::from_slice(&bytes).map_err(|error| parse_error(path, error))?;
            parse_structured(value)?
        }
        _ => return Err(RagError::UnsupportedExtension(path.to_owned())),
    };
    let chunks = enforce_chunk_boundaries(chunks);
    if chunks.is_empty() {
        return Err(RagError::NoExtractableText(path.to_owned()));
    }
    if chunks.len() > MAX_RAG_CHUNKS {
        return Err(RagError::ResourceLimit {
            resource: "chunks",
            limit: MAX_RAG_CHUNKS,
        });
    }
    Ok(ParsedRagFile {
        path: path.to_owned(),
        source_type,
        content_type,
        checksum: format!("{:x}", Sha256::digest(&bytes)),
        chunks,
        metadata: BTreeMap::new(),
    })
}

fn enforce_chunk_boundaries(chunks: Vec<ParsedRagChunk>) -> Vec<ParsedRagChunk> {
    chunks
        .into_iter()
        .flat_map(|chunk| {
            let parts = split_chunk_text(&chunk.text);
            let multiple = parts.len() > 1;
            parts.into_iter().enumerate().map(move |(index, text)| {
                let location = if multiple {
                    format!("{} part {}", chunk.location, index + 1)
                } else {
                    chunk.location.clone()
                };
                ParsedRagChunk {
                    chunk_kind: chunk.chunk_kind.clone(),
                    display_text: text.clone(),
                    text,
                    location,
                    metadata: chunk.metadata.clone(),
                }
            })
        })
        .collect()
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

fn parse_text(text: &str) -> Vec<ParsedRagChunk> {
    paragraphs(text)
        .into_iter()
        .enumerate()
        .flat_map(|(index, text)| located("text", &format!("paragraph {}", index + 1), &text))
        .collect()
}

fn parse_markdown(text: &str) -> Vec<ParsedRagChunk> {
    let mut headings: Vec<(usize, String)> = Vec::new();
    let mut paragraphs = Vec::new();
    let mut lines = Vec::new();
    let mut count = 0;
    let mut fence = None;
    let flush = |lines: &mut Vec<String>,
                 headings: &[(usize, String)],
                 count: &mut usize,
                 output: &mut Vec<ParsedRagChunk>| {
        if lines.is_empty() {
            return;
        }
        let text = lines
            .drain(..)
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            return;
        }
        *count += 1;
        let prefix = headings
            .iter()
            .map(|(_, h)| h.as_str())
            .collect::<Vec<_>>()
            .join(" > ");
        let location = if prefix.is_empty() {
            format!("paragraph {count}")
        } else {
            format!("{prefix} paragraph {count}")
        };
        output.extend(located("markdown_section", &location, &text));
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(marker) = fence {
            lines.push(line.to_owned());
            if trimmed.chars().all(|ch| ch == marker) && trimmed.chars().count() >= 3 {
                fence = None;
            }
            continue;
        }
        let leading = line.len().saturating_sub(line.trim_start().len());
        if leading <= 3 && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
            fence = trimmed.chars().next();
            lines.push(line.to_owned());
            continue;
        }
        let level = trimmed.chars().take_while(|ch| *ch == '#').count();
        if (1..=6).contains(&level) && trimmed.chars().nth(level) == Some(' ') {
            flush(&mut lines, &headings, &mut count, &mut paragraphs);
            headings.retain(|(old, _)| *old < level);
            headings.push((
                level,
                trimmed[level + 1..].trim_end_matches('#').trim().to_owned(),
            ));
        } else if trimmed.is_empty() {
            flush(&mut lines, &headings, &mut count, &mut paragraphs);
        } else {
            lines.push(line.to_owned());
        }
    }
    flush(&mut lines, &headings, &mut count, &mut paragraphs);
    paragraphs
}

fn paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(|block| {
            block
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|block| !block.is_empty())
        .collect()
}

fn located(kind: &str, location: &str, text: &str) -> Vec<ParsedRagChunk> {
    let chunks = split_chunk_text(text);
    let multiple = chunks.len() > 1;
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, text)| ParsedRagChunk {
            chunk_kind: kind.to_owned(),
            display_text: text.clone(),
            text,
            location: if multiple {
                format!("{location} part {}", index + 1)
            } else {
                location.to_owned()
            },
            metadata: BTreeMap::new(),
        })
        .collect()
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
    reader
        .records()
        .enumerate()
        .map(|(index, row)| match row {
            Ok(row) => row_to_chunk(&headers, &row, format!("row {}", index + 2)),
            Err(error) => Err(RagError::Parse {
                path: None,
                message: error.to_string(),
            }),
        })
        .collect()
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
    let entries: Vec<BTreeMap<String, serde_json::Value>> = match value {
        serde_json::Value::Null => vec![],
        serde_json::Value::Array(values) => values.into_iter().map(value_metadata).collect(),
        serde_json::Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| {
                let mut metadata = value_metadata(value);
                metadata.insert("key".into(), key.into());
                metadata
            })
            .collect(),
        _ => {
            return Err(RagError::Parse {
                path: None,
                message: "glossary data must be a list or mapping".into(),
            });
        }
    };
    entries
        .into_iter()
        .enumerate()
        .map(|(i, metadata)| glossary_chunk(metadata, format!("entry {}", i + 1)))
        .collect()
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

fn parse_error(path: &Path, error: impl std::fmt::Display) -> RagError {
    RagError::Parse {
        path: Some(path.to_owned()),
        message: error.to_string(),
    }
}
