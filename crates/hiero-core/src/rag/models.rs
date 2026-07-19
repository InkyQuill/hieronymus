use std::{collections::BTreeMap, fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{db::RagChunkRecord as RawRagChunkRecord, recall::MemorySource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Auto,
    Markdown,
    Text,
    Csv,
    Tsv,
    Yaml,
    Json,
    Docx,
    Html,
    Pdf,
}

impl SourceType {
    pub(crate) fn persisted(self) -> &'static str {
        match self {
            Self::Markdown | Self::Docx | Self::Html | Self::Pdf => "markdown",
            Self::Text => "text",
            Self::Csv | Self::Tsv | Self::Yaml | Self::Json => "glossary",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedRagChunk {
    pub chunk_kind: String,
    pub text: String,
    pub display_text: String,
    pub location: String,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedRagFile {
    pub path: PathBuf,
    pub source_type: SourceType,
    pub content_type: String,
    pub checksum: String,
    pub chunks: Vec<ParsedRagChunk>,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedRagSource {
    pub path: PathBuf,
    pub original_path: PathBuf,
    pub source_type: SourceType,
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ImportOptions {
    pub source_ref: Option<String>,
    pub source_type: Option<SourceType>,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub managed_root: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagSourceRecord {
    pub id: i64,
    pub series_slug: String,
    pub source_ref: String,
    pub source_type: String,
    pub content_type: String,
    pub checksum: String,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagChunkRecord {
    #[serde(flatten)]
    pub record: RawRagChunkRecord,
    pub source_ref: String,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
}

impl std::ops::Deref for RagChunkRecord {
    type Target = RawRagChunkRecord;
    fn deref(&self) -> &Self::Target {
        &self.record
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagImportResult {
    pub source: RagSourceRecord,
    pub source_id: i64,
    pub chunks_created: usize,
    pub skipped: bool,
    pub normalized_path: String,
    pub normalized_format: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode {
    #[default]
    Lexical,
    Hybrid,
}

impl fmt::Display for RetrievalMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Lexical => "lexical",
            Self::Hybrid => "hybrid",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOptions {
    pub limit: usize,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    /// Task 6 supports lexical retrieval only. Task 7 may consume `Hybrid` and
    /// fuse this stable lexical lane with a rebuildable vector lane.
    pub retrieval_mode: RetrievalMode,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 10,
            language_tags: vec![],
            story_scopes: vec![],
            semantic_tags: vec![],
            retrieval_mode: RetrievalMode::Lexical,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagSearchResult {
    pub chunk: RagChunkRecord,
    pub score: f64,
    pub source: MemorySource,
    pub reason: String,
}
