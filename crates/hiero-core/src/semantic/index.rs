use std::{collections::HashMap, sync::RwLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GenerationId(pub uuid::Uuid);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationInfo {
    pub id: GenerationId,
    pub created_at: DateTime<Utc>,
    pub vector_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexHealth {
    pub ok: bool,
    pub vector_count: usize,
    pub active_generation: Option<GenerationId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorRecord {
    pub chunk_id: i64,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchFilter {
    pub series_slug: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub chunk_id: i64,
    pub distance: f32,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SemanticError {
    #[error("semantic model is unavailable: {path}")]
    MissingModel { path: String },
    #[error("semantic index has no active generation")]
    NoActiveGeneration,
    #[error("semantic generation does not exist: {0:?}")]
    MissingGeneration(GenerationId),
    #[error("semantic index is corrupt: {reason}")]
    CorruptIndex { reason: String },
    #[error("invalid semantic vector: {reason}")]
    InvalidVector { reason: String },
    #[error("semantic operation was cancelled")]
    Cancelled,
    #[error("semantic filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("semantic database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("LanceDB operation failed: {0}")]
    Lance(#[from] lancedb::Error),
    #[error("Arrow operation failed: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),
    #[error("semantic worker failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("ONNX Runtime operation failed: {0}")]
    Ort(String),
    #[error("semantic model download failed: {0}")]
    Download(String),
}

#[async_trait]
pub trait SemanticIndex: Send + Sync {
    async fn health(&self) -> Result<IndexHealth, SemanticError>;
    async fn active_generation(&self) -> Result<Option<GenerationInfo>, SemanticError>;
    async fn begin_rebuild(&self) -> Result<GenerationId, SemanticError>;
    async fn upsert_vectors(
        &self,
        generation: GenerationId,
        vectors: Vec<VectorRecord>,
    ) -> Result<usize, SemanticError>;
    async fn search(
        &self,
        query: &[f32],
        filter: &SearchFilter,
        limit: usize,
    ) -> Result<Vec<SearchHit>, SemanticError>;
    async fn delete_vectors(&self, chunk_ids: &[i64]) -> Result<usize, SemanticError>;
    async fn activate_generation(&self, generation: GenerationId) -> Result<(), SemanticError>;
    async fn cancel_generation(&self, generation: GenerationId) -> Result<(), SemanticError>;
    async fn close(&self) -> Result<(), SemanticError>;
}

#[derive(Default)]
struct FakeState {
    active: Option<GenerationId>,
    generations: HashMap<GenerationId, FakeGeneration>,
}

struct FakeGeneration {
    created_at: DateTime<Utc>,
    dimensions: Option<usize>,
    vectors: Vec<VectorRecord>,
}

/// Deterministic in-memory contract implementation for callers' tests.
#[derive(Default)]
pub struct FakeSemanticIndex {
    state: RwLock<FakeState>,
}

impl FakeSemanticIndex {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SemanticIndex for FakeSemanticIndex {
    async fn health(&self) -> Result<IndexHealth, SemanticError> {
        let state = self.state.read().map_err(|_| SemanticError::CorruptIndex {
            reason: "fake index lock poisoned".into(),
        })?;
        let count = state
            .active
            .and_then(|id| state.generations.get(&id))
            .map_or(0, |generation| generation.vectors.len());
        Ok(IndexHealth {
            ok: true,
            vector_count: count,
            active_generation: state.active,
        })
    }

    async fn active_generation(&self) -> Result<Option<GenerationInfo>, SemanticError> {
        let state = self.state.read().map_err(|_| SemanticError::CorruptIndex {
            reason: "fake index lock poisoned".into(),
        })?;
        Ok(state.active.and_then(|id| {
            state.generations.get(&id).map(|generation| GenerationInfo {
                id,
                created_at: generation.created_at,
                vector_count: generation.vectors.len(),
            })
        }))
    }

    async fn begin_rebuild(&self) -> Result<GenerationId, SemanticError> {
        let id = GenerationId(uuid::Uuid::new_v4());
        self.state
            .write()
            .map_err(|_| SemanticError::CorruptIndex {
                reason: "fake index lock poisoned".into(),
            })?
            .generations
            .insert(
                id,
                FakeGeneration {
                    created_at: Utc::now(),
                    dimensions: None,
                    vectors: vec![],
                },
            );
        Ok(id)
    }

    async fn upsert_vectors(
        &self,
        generation: GenerationId,
        vectors: Vec<VectorRecord>,
    ) -> Result<usize, SemanticError> {
        validate_vectors(&vectors)?;
        let mut state = self
            .state
            .write()
            .map_err(|_| SemanticError::CorruptIndex {
                reason: "fake index lock poisoned".into(),
            })?;
        let generation = state
            .generations
            .get_mut(&generation)
            .ok_or(SemanticError::MissingGeneration(generation))?;
        let dimensions = vectors[0].embedding.len();
        if generation
            .dimensions
            .is_some_and(|expected| expected != dimensions)
        {
            return Err(SemanticError::InvalidVector {
                reason: "vector dimensions differ from the generation schema".into(),
            });
        }
        generation.dimensions = Some(dimensions);
        let input_count = vectors.len();
        for vector in vectors {
            generation
                .vectors
                .retain(|existing| existing.chunk_id != vector.chunk_id);
            generation.vectors.push(vector);
        }
        Ok(input_count)
    }

    async fn search(
        &self,
        query: &[f32],
        _filter: &SearchFilter,
        limit: usize,
    ) -> Result<Vec<SearchHit>, SemanticError> {
        validate_query(query)?;
        let state = self.state.read().map_err(|_| SemanticError::CorruptIndex {
            reason: "fake index lock poisoned".into(),
        })?;
        let generation = state.active.ok_or(SemanticError::NoActiveGeneration)?;
        let generation = state
            .generations
            .get(&generation)
            .ok_or(SemanticError::MissingGeneration(generation))?;
        rank_vectors(query, &generation.vectors, limit)
    }

    async fn delete_vectors(&self, chunk_ids: &[i64]) -> Result<usize, SemanticError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| SemanticError::CorruptIndex {
                reason: "fake index lock poisoned".into(),
            })?;
        let generation = state.active.ok_or(SemanticError::NoActiveGeneration)?;
        let generation = state
            .generations
            .get_mut(&generation)
            .ok_or(SemanticError::MissingGeneration(generation))?;
        let before = generation.vectors.len();
        generation
            .vectors
            .retain(|record| !chunk_ids.contains(&record.chunk_id));
        Ok(before - generation.vectors.len())
    }

    async fn activate_generation(&self, generation: GenerationId) -> Result<(), SemanticError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| SemanticError::CorruptIndex {
                reason: "fake index lock poisoned".into(),
            })?;
        if !state.generations.contains_key(&generation) {
            return Err(SemanticError::MissingGeneration(generation));
        }
        state.active = Some(generation);
        Ok(())
    }

    async fn cancel_generation(&self, generation: GenerationId) -> Result<(), SemanticError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| SemanticError::CorruptIndex {
                reason: "fake index lock poisoned".into(),
            })?;
        if state.active == Some(generation) {
            return Err(SemanticError::Cancelled);
        }
        state
            .generations
            .remove(&generation)
            .ok_or(SemanticError::MissingGeneration(generation))?;
        Ok(())
    }

    async fn close(&self) -> Result<(), SemanticError> {
        Ok(())
    }
}

pub(crate) fn validate_query(query: &[f32]) -> Result<(), SemanticError> {
    if query.is_empty() || query.iter().any(|value| !value.is_finite()) {
        return Err(SemanticError::InvalidVector {
            reason: "query must be non-empty and finite".into(),
        });
    }
    Ok(())
}

pub(crate) fn validate_vectors(vectors: &[VectorRecord]) -> Result<(), SemanticError> {
    let mut ids = std::collections::HashSet::with_capacity(vectors.len());
    let dimensions = vectors.first().map_or(0, |vector| vector.embedding.len());
    if dimensions == 0
        || vectors.iter().any(|vector| {
            vector.embedding.len() != dimensions
                || vector.embedding.iter().any(|value| !value.is_finite())
        })
    {
        return Err(SemanticError::InvalidVector {
            reason: "vectors must have one non-zero dimension and finite values".into(),
        });
    }
    if vectors.iter().any(|vector| !ids.insert(vector.chunk_id)) {
        return Err(SemanticError::InvalidVector {
            reason: "one upsert batch must not contain duplicate chunk IDs".into(),
        });
    }
    Ok(())
}

pub(crate) fn rank_vectors(
    query: &[f32],
    vectors: &[VectorRecord],
    limit: usize,
) -> Result<Vec<SearchHit>, SemanticError> {
    let mut hits = vectors
        .iter()
        .map(|record| {
            if record.embedding.len() != query.len() {
                return Err(SemanticError::InvalidVector {
                    reason: "query and index dimensions differ".into(),
                });
            }
            let dot: f32 = query
                .iter()
                .zip(&record.embedding)
                .map(|(a, b)| a * b)
                .sum();
            let qn: f32 = query.iter().map(|value| value * value).sum::<f32>().sqrt();
            let vn: f32 = record
                .embedding
                .iter()
                .map(|value| value * value)
                .sum::<f32>()
                .sqrt();
            let distance = if qn == 0.0 || vn == 0.0 {
                1.0
            } else {
                1.0 - dot / (qn * vn)
            };
            Ok(SearchHit {
                chunk_id: record.chunk_id,
                distance,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    hits.sort_by(|left, right| {
        left.distance
            .total_cmp(&right.distance)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    hits.truncate(limit);
    Ok(hits)
}
