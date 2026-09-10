//! Data records for the RAG store, ported from Python `rag_models.py`.

use serde_json::{Map, Value};

/// One imported RAG source (a row in `rag_sources`).
#[derive(Debug, Clone, PartialEq)]
pub struct RagSourceRecord {
    pub id: i64,
    pub series_slug: String,
    pub source_ref: String,
    pub source_type: String,
    pub content_type: String,
    pub checksum: String,
    pub metadata: Map<String, Value>,
}

/// One searchable RAG chunk (a row in `rag_chunks` plus its typed metadata
/// side tables and the owning source's `source_ref`).
#[derive(Debug, Clone, PartialEq)]
pub struct RagChunkRecord {
    pub claim_annotation: crate::claim_reads::ClaimReadAnnotation,
    pub id: i64,
    pub source_id: i64,
    pub series_slug: String,
    pub source_ref: String,
    pub chunk_kind: String,
    pub text: String,
    pub display_text: String,
    pub location: String,
    pub metadata: Map<String, Value>,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
}

impl RagChunkRecord {
    /// Display title: the source reference plus the intra-source location.
    pub fn title(&self) -> String {
        if self.location.is_empty() {
            return self.source_ref.clone();
        }
        format!("{} {}", self.source_ref, self.location)
    }

    pub fn kind(&self) -> &str {
        &self.chunk_kind
    }
}

/// One ranked search hit from the RAG store.
#[derive(Debug, Clone, PartialEq)]
pub struct RagSearchHit {
    pub chunk: RagChunkRecord,
    pub score: f64,
    pub reason: String,
}

/// Outcome of [`crate::rag::RagStore::import_file`], mirroring the Python
/// `RagImportResult` dataclass.
#[derive(Debug, Clone, PartialEq)]
pub struct RagImportResult {
    pub source: RagSourceRecord,
    pub chunk_count: usize,
    pub skipped: bool,
    pub normalized_path: String,
    pub normalized_format: String,
}
