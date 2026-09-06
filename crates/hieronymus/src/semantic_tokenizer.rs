//! The pinned WordPiece tokenizer for the semantic RAG lane (Task S1).
//!
//! This replaces the synthetic byte-fold preprocessing on both the document
//! (rebuild jobs) and the query (recall lane) paths. The tokenizer asset is
//! the `tokenizer.json` of the pinned model revision (see
//! [`crate::semantic_model::TOKENIZER_SHA256`]); it is acquired through the
//! same verified download/promotion flow as the ONNX model and re-verified by
//! SHA-256 before every load, so both paths always tokenize under one
//! identity.
//!
//! Identity discipline: [`MINILM_TOKENIZER_ID`] encodes the tokenizer kind,
//! the pinned asset hash, the 256-token longest-first truncation (special
//! tokens included), the BertNormalizer, and the [CLS]/[SEP] postprocessor.
//! It is part of every [`crate::semantic_embeddings::EmbeddingIdentity`], so
//! generations persisted under the old byte-fold identity are rejected and
//! rebuilt — never relabeled.

use crate::semantic_error::SemanticError;
use crate::semantic_jobs::{AuthoritativeChunk, ChunkTokenizer};

/// Maximum token positions per sequence after truncation (special tokens
/// included). The exported graph accepts 512; production tokenization is
/// capped at the sentence-transformers sweet spot of 256.
pub const MAX_TOKENS: usize = 256;

/// The stable identity of the pinned tokenization: tokenizer kind, the pinned
/// asset digest, truncation policy, and normalization. Any change to any of
/// those is a different identity and forces a full generation rebuild.
pub const MINILM_TOKENIZER_ID: &str = concat!(
    "wordpiece-minilm-l6-v2",
    "@sha256:be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037",
    ":max-256:longest-first",
    ":bert-normalize:cls-sep"
);

/// The pinned WordPiece tokenizer over the MiniLM vocabulary.
///
/// Built from verified asset bytes only; truncation is pinned to
/// longest-first at [`MAX_TOKENS`] tokens including the special tokens, and
/// padding is disabled so callers build their own attention masks.
pub struct ModelTokenizer(tokenizers::Tokenizer);

impl ModelTokenizer {
    /// Builds the tokenizer from the pinned asset bytes (downloaded and
    /// hash-verified by the caller, or the committed test fixture). Any
    /// malformed asset is a [`SemanticError::InvalidEmbedding`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SemanticError> {
        let mut tokenizer = tokenizers::Tokenizer::from_bytes(bytes)
            .map_err(|error| SemanticError::InvalidEmbedding(error.to_string()))?;
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: MAX_TOKENS,
                ..Default::default()
            }))
            .map_err(|error| SemanticError::InvalidEmbedding(error.to_string()))?;
        tokenizer.with_padding(None);
        Ok(Self(tokenizer))
    }

    /// Encodes one text into WordPiece token ids with the special tokens
    /// ([CLS] ... [SEP]) added and truncation applied. Never empty: even the
    /// empty string encodes to the special-token pair.
    pub fn encode(&self, text: &str) -> Result<Vec<u32>, SemanticError> {
        self.0
            .encode(text, true)
            .map(|encoding| encoding.get_ids().to_vec())
            .map_err(|error| SemanticError::InvalidEmbedding(error.to_string()))
    }
}

impl ChunkTokenizer for ModelTokenizer {
    fn tokenize(&mut self, chunk: &AuthoritativeChunk) -> Result<Vec<u32>, SemanticError> {
        self.encode(&chunk.text)
    }
}
