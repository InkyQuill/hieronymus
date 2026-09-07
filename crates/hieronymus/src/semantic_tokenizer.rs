//! The pinned Unigram tokenizer for the semantic RAG lane (P2).
//!
//! This replaces the English-model preprocessing on both the document
//! (rebuild jobs) and the query (recall lane) paths. The tokenizer asset is
//! the `tokenizer.json` of the pinned model revision (see
//! [`crate::semantic_model::TOKENIZER_SHA256`]); it is acquired through the
//! same verified download/promotion flow as the ONNX model and re-verified by
//! SHA-256 before every load, so both paths always tokenize under one
//! identity.
//!
//! Identity discipline: [`MINILM_TOKENIZER_ID`] encodes the tokenizer kind,
//! the pinned asset hash, the 128-token longest-first truncation (special
//! tokens included), the asset normalizer, and the XLM-R special-token postprocessor.
//! It is part of every [`crate::semantic_embeddings::EmbeddingIdentity`], so
//! generations persisted under earlier model/tokenizer identities are rejected and
//! rebuilt — never relabeled.

use crate::semantic_error::SemanticError;
use crate::semantic_jobs::{AuthoritativeChunk, ChunkTokenizer};

/// Maximum token positions per sequence after truncation (special tokens
/// included). The exported graph accepts 512; production tokenization is
/// capped at the sentence-transformers configured maximum of 128.
pub const MAX_TOKENS: usize = 128;

/// The stable identity of the pinned tokenization: tokenizer kind, the pinned
/// asset digest, truncation policy, and normalization. Any change to any of
/// those is a different identity and forces a full generation rebuild.
pub const MINILM_TOKENIZER_ID: &str = concat!(
    "unigram-multilingual-minilm-l12-v2",
    "@sha256:2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8",
    ":max-128:longest-first",
    ":precompiled-normalize:metaspace:xlmr-specials"
);

/// The pinned Unigram tokenizer over the MiniLM vocabulary.
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

    /// Encodes one text into Unigram token ids with the special tokens
    /// (<s> ... </s>) added and truncation applied. Never empty: even the
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
