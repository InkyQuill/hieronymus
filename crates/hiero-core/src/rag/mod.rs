mod chunking;
mod conversion;
mod models;
mod parsing;
mod store;

use std::{path::PathBuf, sync::OnceLock};

pub use chunking::{MAX_RAG_CHUNK_CHARS, split_chunk_text};
pub use conversion::{MAX_CONVERTED_TEXT_BYTES, normalize_rag_source};
pub use models::*;
pub use parsing::{
    MAX_RAG_CHUNKS, MAX_RAG_CSV_COLUMNS, MAX_RAG_FILE_BYTES, MAX_RAG_METADATA_BYTES, load_rag_file,
};
pub use store::RagStore;

pub(crate) fn default_managed_root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::temp_dir().join(format!("hieronymus-rag-{}", uuid::Uuid::new_v4()))
    })
    .clone()
}

#[derive(Debug, thiserror::Error)]
pub enum RagError {
    #[error("RAG source is not a file: {0}")]
    NotAFile(PathBuf),
    #[error("unsupported RAG source extension: {0}")]
    UnsupportedExtension(PathBuf),
    #[error("EPUB is unsupported; split EPUB into chapters before RAG import")]
    UnsupportedEpub,
    #[error("source type {requested:?} is incompatible with {path}")]
    SourceTypeMismatch {
        requested: SourceType,
        path: PathBuf,
    },
    #[error("source produced no extractable text: {0}")]
    NoExtractableText(PathBuf),
    #[error("RAG resource limit exceeded for {resource}; limit is {limit}")]
    ResourceLimit {
        resource: &'static str,
        limit: usize,
    },
    #[error("invalid RAG source{path}: {message}", path = path.as_ref().map(|p| format!(" {}", p.display())).unwrap_or_default())]
    Parse {
        path: Option<PathBuf>,
        message: String,
    },
    #[error("RAG I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("RAG parsing task failed: {0}")]
    Join(#[source] tokio::task::JoinError),
    #[error("RAG database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("RAG metadata is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("limit must be at least 1")]
    InvalidLimit,
    #[error("semantic retrieval is introduced by migration Phase 003 Task 7; use lexical mode")]
    SemanticUnavailable,
    #[error("unsafe managed RAG path: {0}")]
    UnsafeManagedPath(PathBuf),
    #[error("managed RAG artifact does not match expected content: {0}")]
    ManagedArtifactMismatch(PathBuf),
}
