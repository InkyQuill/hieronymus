//! Embedding provider surface for the semantic RAG lane: one identity type,
//! one provider trait whose document and query requests share that identity, a
//! deterministic fake provider for tests, and the real ONNX provider ported
//! from the qualified harness (`qualification/harnesses/semantic-native`).
//!
//! The provider consumes token sequences, exactly like the qualified harness:
//! tokenization is a separate concern owned by the recall integration.

use sha2::{Digest, Sha256};
use std::path::Path;

use crate::semantic_error::SemanticError;
use crate::semantic_model::{MODEL_NAME, MODEL_REVISION, MODEL_SHA256};

/// Embedding width of the pinned qualification model
/// (`sentence-transformers/all-MiniLM-L6-v2`, ONNX export).
pub const EMBEDDING_DIMENSIONS: usize = 384;
/// WordPiece vocabulary size of the pinned model; token streams are folded
/// into this range so the ONNX graph always gathers in-bounds rows.
const MODEL_VOCABULARY_SIZE: u32 = 30_522;
/// Maximum token positions accepted per sequence by the exported graph.
const MODEL_MAX_SEQUENCE: usize = 512;
/// Batches of sequences the provider accepts per request.
const MODEL_MAX_BATCH_INPUTS: usize = 32;
/// Preferred output of the sentence-transformers export; mean pooling runs on
/// the token-level hidden states.
const PREFERRED_OUTPUT: &str = "last_hidden_state";

/// Immutable identity of one embedding model.
///
/// Document and query embeddings must resolve to the same identity: every
/// generation records the identity it was built with and every later request
/// is rejected unless the identities are equal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddingIdentity {
    provider: String,
    model: String,
    revision: String,
    dimensions: usize,
    normalization: String,
    max_input_tokens: usize,
    max_batch_inputs: usize,
}

impl EmbeddingIdentity {
    /// Creates a validated identity. Every field participates in equality, so
    /// a changed model, revision, width, or limit is a different identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: impl Into<String>,
        model: impl Into<String>,
        revision: impl Into<String>,
        dimensions: usize,
        normalization: impl Into<String>,
        max_input_tokens: usize,
        max_batch_inputs: usize,
    ) -> Result<Self, SemanticError> {
        let provider = provider.into();
        let model = model.into();
        let revision = revision.into();
        let normalization = normalization.into();
        if provider.is_empty() {
            return Err(SemanticError::InvalidEmbedding(
                "provider name must not be empty".to_string(),
            ));
        }
        if model.is_empty() {
            return Err(SemanticError::InvalidEmbedding(
                "model name must not be empty".to_string(),
            ));
        }
        if revision.is_empty() {
            return Err(SemanticError::InvalidEmbedding(
                "model revision must not be empty".to_string(),
            ));
        }
        if dimensions == 0 {
            return Err(SemanticError::InvalidEmbedding(
                "embedding dimensions must be positive".to_string(),
            ));
        }
        if normalization.is_empty() {
            return Err(SemanticError::InvalidEmbedding(
                "normalization must be named".to_string(),
            ));
        }
        if max_input_tokens == 0 || max_batch_inputs == 0 {
            return Err(SemanticError::InvalidEmbedding(
                "provider limits must be positive".to_string(),
            ));
        }
        Ok(Self {
            provider,
            model,
            revision,
            dimensions,
            normalization,
            max_input_tokens,
            max_batch_inputs,
        })
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    pub fn normalization(&self) -> &str {
        &self.normalization
    }

    pub fn max_input_tokens(&self) -> usize {
        self.max_input_tokens
    }

    pub fn max_batch_inputs(&self) -> usize {
        self.max_batch_inputs
    }

    /// Checks one token sequence against the input limits before any
    /// inference happens. Testable without a loaded runtime or model.
    pub fn check_tokens(&self, length: usize) -> Result<(), SemanticError> {
        if length == 0 {
            return Err(SemanticError::InvalidEmbedding(
                "cannot embed an empty token sequence".to_string(),
            ));
        }
        if length > self.max_input_tokens {
            return Err(SemanticError::InvalidEmbedding(format!(
                "token sequence of {length} exceeds the model limit of {}",
                self.max_input_tokens
            )));
        }
        Ok(())
    }
}

/// Embedding provider over token sequences. Document and query requests share
/// one identity; implementations must never report different identities for
/// the two request kinds.
pub trait EmbeddingProvider: Send {
    fn identity(&self) -> &EmbeddingIdentity;

    fn embed_document(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError>;

    fn embed_query(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError>;
}

/// Deterministic stand-in provider for tests and loopback runs: token streams
/// map to fixed, L2-normalized vectors with the same identity reporting as the
/// real provider. Identical token streams always produce identical vectors.
pub struct FakeEmbeddingProvider {
    identity: EmbeddingIdentity,
}

impl FakeEmbeddingProvider {
    pub fn new(dimensions: usize) -> Self {
        Self::with_model(dimensions, "fake-model")
    }

    /// A fake provider under a different model name forms a different
    /// identity, which the store must reject at build and activation time.
    pub fn with_model(dimensions: usize, model: &str) -> Self {
        Self {
            identity: EmbeddingIdentity::new(
                "fake",
                model,
                "fake-revision",
                dimensions,
                "l2",
                MODEL_MAX_SEQUENCE,
                MODEL_MAX_BATCH_INPUTS,
            )
            .expect("fake provider identity is valid"),
        }
    }
}

impl EmbeddingProvider for FakeEmbeddingProvider {
    fn identity(&self) -> &EmbeddingIdentity {
        &self.identity
    }

    fn embed_document(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.identity.check_tokens(token_ids.len())?;
        Ok(fake_vector(token_ids, self.identity.dimensions()))
    }

    fn embed_query(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.embed_document(token_ids)
    }
}

/// xorshift64* expansion of the token-stream digest: pure integer math plus
/// one f32 division per component, so vectors are stable across platforms.
fn fake_vector(token_ids: &[u32], dimensions: usize) -> Vec<f32> {
    let mut hasher = Sha256::new();
    for token in token_ids {
        hasher.update(token.to_le_bytes());
    }
    let seed = hasher.finalize();
    let mut state = u64::from_be_bytes(seed[..8].try_into().expect("eight digest bytes")) | 1;
    let mut vector = Vec::with_capacity(dimensions);
    for _ in 0..dimensions {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let unit = ((state >> 11) as f32) / ((1_u32 << 21) as f32);
        vector.push(unit - 1.0);
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in &mut vector {
        *value /= norm;
    }
    vector
}

/// Verified handle over the acquired ONNX runtime and model, ported from the
/// qualified harness's `OnnxEmbeddingProvider`.
///
/// `load` authenticates the model by SHA-256 against the pinned qualification
/// checksum before anything is executed and loads the runtime through a
/// checked dynamic link. Mean pooling over token-level hidden states followed
/// by L2 normalization mirrors the qualified embedding path.
pub struct OnnxEmbeddingProvider {
    session: ort::session::Session,
    output_name: String,
    identity: EmbeddingIdentity,
}

impl std::fmt::Debug for OnnxEmbeddingProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OnnxEmbeddingProvider")
            .field("output_name", &self.output_name)
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl OnnxEmbeddingProvider {
    /// The identity of the pinned qualification model. Constructible without
    /// a runtime, model file, or network access.
    pub fn static_identity() -> EmbeddingIdentity {
        EmbeddingIdentity::new(
            "onnx",
            MODEL_NAME,
            MODEL_REVISION,
            EMBEDDING_DIMENSIONS,
            "l2",
            MODEL_MAX_SEQUENCE,
            MODEL_MAX_BATCH_INPUTS,
        )
        .expect("the qualified model identity is valid")
    }

    /// Loads the shared runtime and the model after verifying the model
    /// checksum against the pinned qualification digest.
    pub fn load(runtime: &Path, model: &Path) -> Result<Self, SemanticError> {
        if !runtime.is_file() {
            return Err(SemanticError::ModelUnavailable(format!(
                "onnx runtime shared library {} does not exist",
                runtime.display()
            )));
        }

        let model_checksum = sha256_hex(model)?;
        if model_checksum != MODEL_SHA256 {
            return Err(SemanticError::ChecksumMismatch {
                expected: MODEL_SHA256.to_string(),
                actual: model_checksum,
            });
        }

        // The dynamic library is loaded before any other ort API is touched,
        // as required by `ort::init_from`; a binary that fails to export a
        // compatible ONNX Runtime API is rejected here.
        ort::init_from(runtime)
            .map_err(|error| {
                SemanticError::ModelUnavailable(format!(
                    "could not load the verified onnx runtime from {}: {error}",
                    runtime.display()
                ))
            })?
            .commit();

        let session = ort::session::Session::builder()
            .map_err(|error| SemanticError::Store(format!("session builder: {error}")))?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level1)
            .map_err(|error| SemanticError::Store(format!("optimization level: {error}")))?
            .with_intra_threads(2)
            .map_err(|error| SemanticError::Store(format!("intra-op threads: {error}")))?
            .commit_from_file(model)
            .map_err(|error| {
                SemanticError::ModelUnavailable(format!(
                    "could not load the model at {}: {error}",
                    model.display()
                ))
            })?;

        let output_name = if session
            .outputs()
            .iter()
            .any(|outlet| outlet.name() == PREFERRED_OUTPUT)
        {
            PREFERRED_OUTPUT.to_string()
        } else {
            session
                .outputs()
                .first()
                .ok_or_else(|| SemanticError::Store("model exposes no outputs".to_string()))?
                .name()
                .to_string()
        };

        Ok(Self {
            session,
            output_name,
            identity: Self::static_identity(),
        })
    }

    /// Embeds one token sequence: mean-pool over positions, then L2-normalize.
    fn embed(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.identity.check_tokens(token_ids.len())?;

        let sequence = token_ids.len() as i64;
        let shape = vec![1_i64, sequence];
        let input_ids: Vec<i64> = token_ids
            .iter()
            .map(|token| u64::from(*token % MODEL_VOCABULARY_SIZE) as i64)
            .collect();
        let attention_mask = vec![1_i64; token_ids.len()];
        let token_type_ids = vec![0_i64; token_ids.len()];

        let outputs = self
            .session
            .run(ort::inputs![
                "input_ids" => ort::value::Tensor::from_array((shape.clone(), input_ids))
                    .map_err(|error| SemanticError::Store(format!("input_ids tensor: {error}")))?,
                "attention_mask" => ort::value::Tensor::from_array((shape.clone(), attention_mask))
                    .map_err(|error| SemanticError::Store(format!("attention tensor: {error}")))?,
                "token_type_ids" => ort::value::Tensor::from_array((shape, token_type_ids))
                    .map_err(|error| SemanticError::Store(format!("type tensor: {error}")))?,
            ])
            .map_err(|error| SemanticError::Store(format!("inference failed: {error}")))?;

        let output = outputs.get(self.output_name.as_str()).ok_or_else(|| {
            SemanticError::Store(format!("model output {} is missing", self.output_name))
        })?;
        let (dims, hidden_states) = output
            .try_extract_tensor::<f32>()
            .map_err(|error| SemanticError::Store(format!("hidden states: {error}")))?;
        let dims: &[i64] = dims;
        if dims.len() != 3 || dims[0] != 1 {
            return Err(SemanticError::Store(format!(
                "unexpected hidden state shape {dims:?}"
            )));
        }
        if dims[2] as usize != EMBEDDING_DIMENSIONS {
            return Err(SemanticError::Store(format!(
                "unexpected hidden state width {}: expected {EMBEDDING_DIMENSIONS}",
                dims[2]
            )));
        }
        let positions = dims[1] as usize;
        if hidden_states.len() != positions * EMBEDDING_DIMENSIONS {
            return Err(SemanticError::Store(
                "hidden state payload is truncated".to_string(),
            ));
        }

        // Attention mask is all ones for a single unpadded sequence, so the
        // mean pool divides by the position count.
        let mut pooled = vec![0.0_f32; EMBEDDING_DIMENSIONS];
        for position in 0..positions {
            let row = &hidden_states[position * EMBEDDING_DIMENSIONS..][..EMBEDDING_DIMENSIONS];
            for (sum, value) in pooled.iter_mut().zip(row) {
                *sum += value;
            }
        }
        for value in &mut pooled {
            *value /= positions as f32;
        }

        let norm = pooled.iter().map(|value| value * value).sum::<f32>().sqrt();
        if norm <= f32::EPSILON {
            return Err(SemanticError::Store(
                "cannot normalize a zero embedding".to_string(),
            ));
        }
        for value in &mut pooled {
            *value /= norm;
        }
        Ok(pooled)
    }
}

impl EmbeddingProvider for OnnxEmbeddingProvider {
    fn identity(&self) -> &EmbeddingIdentity {
        &self.identity
    }

    fn embed_document(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.embed(token_ids)
    }

    fn embed_query(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.embed(token_ids)
    }
}

fn sha256_hex(path: &Path) -> Result<String, SemanticError> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    // The model is ~90 MB; a 1 MiB window keeps the read bounded and cheap.
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_vectors_are_deterministic_across_instances() {
        let mut first = FakeEmbeddingProvider::new(8);
        let mut second = FakeEmbeddingProvider::new(8);
        assert_eq!(
            first.embed_document(&[7, 8, 9]).unwrap(),
            second.embed_document(&[7, 8, 9]).unwrap()
        );
        // The expansion must actually consume the token stream.
        assert_ne!(
            first.embed_document(&[7, 8, 9]).unwrap(),
            first.embed_document(&[7, 8, 10]).unwrap()
        );
    }

    #[test]
    fn identity_rejects_over_limit_batch_requests() {
        let identity = EmbeddingIdentity::new("fake", "m", "r", 4, "l2", 4, 2).unwrap();
        assert_eq!(identity.max_batch_inputs(), 2);
        assert!(identity.check_tokens(4).is_ok());
        assert!(matches!(
            identity.check_tokens(5),
            Err(SemanticError::InvalidEmbedding(_))
        ));
    }
}
