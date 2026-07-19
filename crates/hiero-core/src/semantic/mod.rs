mod embedding;
mod index;
mod jobs;
mod lancedb;
mod rrf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDiagnostic {
    pub ok: bool,
    pub message: Option<String>,
}

impl SemanticDiagnostic {
    #[must_use]
    pub const fn healthy() -> Self {
        Self {
            ok: true,
            message: None,
        }
    }
    #[must_use]
    pub fn degraded(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: Some(message.into()),
        }
    }
}

pub use embedding::{EmbeddingProvider, FakeEmbeddingProvider, OrtEmbeddingProvider};
pub use index::{
    FakeSemanticIndex, GenerationId, GenerationInfo, IndexHealth, SearchFilter, SearchHit,
    SemanticError, SemanticIndex, VectorRecord,
};
pub use jobs::SemanticJobQueue;
pub use lancedb::LanceDbIndex;
pub use rrf::reciprocal_rank_fusion;

pub type Result<T> = std::result::Result<T, SemanticError>;
