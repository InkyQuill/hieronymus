//! Error type for the semantic RAG data plane. Semantic failures are
//! recoverable by design: callers fall back to the FTS lane and the
//! authoritative SQLite rows are never touched by them.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SemanticError {
    #[error("invalid generation or series slug: {0}")]
    InvalidSlug(String),
    #[error("invalid embedding input: {0}")]
    InvalidEmbedding(String),
    #[error("invalid semantic state transition: {0}")]
    InvalidState(String),
    #[error("semantic validation failed: {0}")]
    ValidationFailed(String),
    #[error("embedding identity mismatch: expected {expected}, got {actual}")]
    IdentityMismatch { expected: String, actual: String },
    #[error("embedding model unavailable: {0}")]
    ModelUnavailable(String),
    #[error("model checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("model download failed: {0}")]
    Download(String),
    #[error("unsupported model url: {0}")]
    UnsupportedUrl(String),
    #[error("semantic artifact not found: {0}")]
    NotFound(String),
    #[error("could not promote {from} to {to}: {message}")]
    Promotion {
        from: PathBuf,
        to: PathBuf,
        message: String,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("vector store error: {0}")]
    Store(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_name_the_failing_concern() {
        assert_eq!(
            SemanticError::InvalidSlug("bad slug!".to_string()).to_string(),
            "invalid generation or series slug: bad slug!"
        );
        assert_eq!(
            SemanticError::ChecksumMismatch {
                expected: "aa".to_string(),
                actual: "bb".to_string(),
            }
            .to_string(),
            "model checksum mismatch: expected aa, got bb"
        );
    }
}
