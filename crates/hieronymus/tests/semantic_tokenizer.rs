//! P2: the pinned multilingual Unigram tokenizer runs on
//! both the document and query paths. The fixture is the tokenizer.json of the
//! pinned model revision, hash-verified at acquisition time.

use hieronymus::semantic_embeddings::{
    EMBEDDING_DIMENSIONS, EmbeddingProvider, FakeEmbeddingProvider,
};
use hieronymus::semantic_error::SemanticError;
use hieronymus::semantic_jobs::{AuthoritativeChunk, ChunkTokenizer};
use hieronymus::semantic_model::{
    DEFAULT_TOKENIZER_URL, ModelTransport, TOKENIZER_BYTES, TOKENIZER_SHA256,
};
use hieronymus::semantic_store::SemanticStore;
use hieronymus::semantic_tokenizer::{MAX_TOKENS, MINILM_TOKENIZER_ID, ModelTokenizer};
use std::path::Path;

/// <s> and </s> ids in the multilingual MiniLM Unigram vocabulary.
const CLS: u32 = 0;
const SEP: u32 = 2;
/// [PAD]: never emitted once padding is disabled.
const PAD: u32 = 1;

const FIXTURE: &[u8] = include_bytes!("fixtures/multilingual-minilm-tokenizer.json");

fn tokenizer() -> ModelTokenizer {
    ModelTokenizer::from_bytes(FIXTURE).unwrap()
}

#[test]
fn multilingual_minilm_uses_pinned_ids_and_special_tokens() {
    let tokenizer = ModelTokenizer::from_bytes(FIXTURE).unwrap();
    assert_eq!(
        tokenizer.encode("Hello world").unwrap(),
        vec![0, 35378, 8999, 2]
    );
}

// ---------------------------------------------------------------------------
// Asset pinning
// ---------------------------------------------------------------------------

#[test]
fn the_fixture_is_the_pinned_asset() {
    use sha2::Digest;
    assert_eq!(FIXTURE.len() as u64, TOKENIZER_BYTES);
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(FIXTURE)),
        TOKENIZER_SHA256
    );
    // The identity embeds the pinned digest, so a different asset can never
    // masquerade as the same tokenizer.
    assert!(MINILM_TOKENIZER_ID.contains(TOKENIZER_SHA256));
}

#[test]
fn an_invalid_asset_is_rejected() {
    assert!(matches!(
        ModelTokenizer::from_bytes(b"not a tokenizer"),
        Err(SemanticError::InvalidEmbedding(_))
    ));
    assert!(matches!(
        ModelTokenizer::from_bytes(b"{\"truncated\": true,"),
        Err(SemanticError::InvalidEmbedding(_))
    ));
}

// ---------------------------------------------------------------------------
// Normalization: casing, accents, punctuation, scripts, empty input
// ---------------------------------------------------------------------------

#[test]
fn casing_is_preserved_the_way_the_model_was_trained() {
    // The multilingual asset preserves case.
    assert_ne!(
        tokenizer().encode("Hello World").unwrap(),
        tokenizer().encode("hello world").unwrap()
    );
    // But different words still map to different ids.
    assert_ne!(
        tokenizer().encode("hello").unwrap(),
        tokenizer().encode("world").unwrap()
    );
}

#[test]
fn accents_and_punctuation_preserve_asset_tokenization() {
    let stripped = tokenizer().encode("cafe").unwrap();
    let accented = tokenizer().encode("café").unwrap();
    // The precompiled normalizer and Unigram vocabulary preserve the
    // asset policy; both forms retain their special-token boundaries.
    assert_eq!(stripped.first(), Some(&CLS));
    assert_eq!(accented.first(), Some(&CLS));
    assert_eq!(stripped.last(), Some(&SEP));
    assert_eq!(accented.last(), Some(&SEP));
    // Punctuation changes the encoded sequence.
    let with_punct = tokenizer().encode("wizard!").unwrap();
    let without = tokenizer().encode("wizard").unwrap();
    assert_ne!(with_punct, without);
    assert!(with_punct.len() > without.len());
}

#[test]
fn cjk_and_cyrillic_text_encode_without_unk_explosions() {
    let cjk = tokenizer().encode("魔法の森を歩く").unwrap();
    let cyrillic = tokenizer().encode("Лес полон тишины").unwrap();
    for stream in [cjk, cyrillic] {
        assert_eq!(stream.first(), Some(&CLS));
        assert_eq!(stream.last(), Some(&SEP));
        assert!(stream.len() >= 3);
        assert!(
            !stream.contains(&3),
            "supported literary scripts must not collapse to <unk>"
        );
    }
    // This two-character sample has distinct vocabulary pieces.
    let cjk = tokenizer().encode("魔法").unwrap();
    assert!(cjk.len() >= 4, "two CJK chars plus specials: {:?}", cjk);
}

#[test]
fn the_empty_string_encodes_to_the_special_token_pair() {
    assert_eq!(tokenizer().encode("").unwrap(), vec![CLS, SEP]);
    // Never empty: the provider rejects empty token sequences by identity.
    assert!(!tokenizer().encode("").unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Truncation and padding
// ---------------------------------------------------------------------------

#[test]
fn long_input_truncates_to_128_with_special_tokens_retained() {
    let long: String = "token ".repeat(5_000);
    let stream = tokenizer().encode(&long).unwrap();
    assert_eq!(stream.len(), MAX_TOKENS);
    assert_eq!(stream.first(), Some(&CLS), "[CLS] must survive truncation");
    assert_eq!(stream.last(), Some(&SEP), "[SEP] must survive truncation");
    // A short input is never padded to the cap.
    assert!(tokenizer().encode("short").unwrap().len() < MAX_TOKENS);
}

#[test]
fn padding_is_disabled_so_streams_never_contain_pad_ids() {
    // Padding disabled: every stream ends at its own length, and the pad id
    // never appears inside a stream (the caller builds attention masks).
    for text in ["a", "a longer sentence about masks", ""] {
        let stream = tokenizer().encode(text).unwrap();
        assert!(!stream.contains(&PAD), "pad id leaked into {text:?}");
    }
}

// ---------------------------------------------------------------------------
// Document/query path integration
// ---------------------------------------------------------------------------

#[test]
fn the_chunk_tokenizer_impl_encodes_chunk_text() {
    let mut tokenizer = tokenizer();
    let chunk = AuthoritativeChunk {
        chunk_id: 7,
        series_slug: "demo".to_string(),
        text: "The Cooking Talent appears here.".to_string(),
    };
    assert_eq!(
        tokenizer.tokenize(&chunk).unwrap(),
        ModelTokenizer::from_bytes(FIXTURE)
            .unwrap()
            .encode("The Cooking Talent appears here.")
            .unwrap()
    );
}

#[test]
fn tokenized_streams_embed_to_finite_normalized_384_dim_vectors() {
    // The provider surface contract over a real tokenized stream: finite,
    // 384-dimensional, L2-normalized (mean pooling happens on the ONNX side;
    // the fake provider mirrors the contract deterministically).
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    assert_eq!(provider.identity().tokenizer(), MINILM_TOKENIZER_ID);
    for text in ["Hello world", "魔法の森を歩く", ""] {
        let ids = tokenizer().encode(text).unwrap();
        let vector = provider.embed_query(&ids).unwrap();
        assert_eq!(vector.len(), EMBEDDING_DIMENSIONS);
        assert!(vector.iter().all(|value| value.is_finite()));
        let norm: f32 = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm {norm} for {text:?}");
    }
}

// ---------------------------------------------------------------------------
// Acquisition discipline (same verified flow as the model, offline)
// ---------------------------------------------------------------------------

struct MemoryTransport {
    body: Vec<u8>,
}

impl ModelTransport for MemoryTransport {
    fn download_to(
        &self,
        _url: &str,
        destination: &Path,
        _max_bytes: u64,
    ) -> Result<u64, SemanticError> {
        std::fs::write(destination, &self.body)?;
        Ok(self.body.len() as u64)
    }
}

#[test]
fn the_store_acquires_verifies_and_loads_the_tokenizer() {
    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path().join("hieronymus"));
    let store = SemanticStore::open(&config).unwrap();

    // Missing: no load, and the status says so.
    assert_eq!(
        store.tokenizer_status(),
        hieronymus::semantic_model::ModelStatus::Missing
    );
    assert!(matches!(
        store.load_model_tokenizer(),
        Err(SemanticError::ModelUnavailable(_))
    ));

    // Acquisition verifies the pinned digest and promotes atomically.
    let acquisition = store
        .acquire_tokenizer(
            &MemoryTransport {
                body: FIXTURE.to_vec(),
            },
            DEFAULT_TOKENIZER_URL,
        )
        .unwrap();
    assert_eq!(acquisition.bytes, TOKENIZER_BYTES);
    assert_eq!(acquisition.checksum, TOKENIZER_SHA256);
    assert!(store.tokenizer_path().is_file());

    // The loaded tokenizer is the pinned one.
    let loaded = store.load_model_tokenizer().unwrap();
    assert_eq!(
        loaded.encode("Hello world").unwrap(),
        vec![0, 35378, 8999, 2]
    );

    // A same-size tampered asset passes the size pre-check and fails the
    // load-time SHA-256 check.
    let mut tampered = FIXTURE.to_vec();
    let middle = tampered.len() / 2;
    tampered[middle] = tampered[middle].wrapping_add(1);
    std::fs::write(store.tokenizer_path(), tampered).unwrap();
    assert!(matches!(
        store.load_model_tokenizer(),
        Err(SemanticError::ChecksumMismatch { .. })
    ));
}

#[test]
fn acquisition_rejects_an_unpinned_asset_without_promoting() {
    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path().join("hieronymus"));
    let store = SemanticStore::open(&config).unwrap();
    let mut wrong = FIXTURE.to_vec();
    let last = wrong.len() - 1;
    wrong[last] = wrong[last].wrapping_add(1);
    let tampered = MemoryTransport { body: wrong };
    assert!(matches!(
        store.acquire_tokenizer(&tampered, DEFAULT_TOKENIZER_URL),
        Err(SemanticError::ChecksumMismatch { .. })
    ));
    assert!(
        !store.tokenizer_path().exists(),
        "a failed acquisition must not promote anything"
    );
}
