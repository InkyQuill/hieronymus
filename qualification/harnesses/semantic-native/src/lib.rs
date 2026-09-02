mod corpus;
pub mod fts;
#[cfg(feature = "semantic-native")]
mod index;
#[cfg(feature = "semantic-native")]
mod model;
#[cfg(feature = "semantic-native")]
pub mod scenario;

pub use corpus::{Chunk, CorpusSpec, Query, corpus_digest, generate_corpus, load_default_spec};
#[cfg(feature = "semantic-native")]
pub use index::{
    ANN_INDEX_NAME, ANN_NUM_PARTITIONS, ANN_NUM_PROBES, ANN_PQ_NUM_BITS, ANN_REFINE_FACTOR,
    EMBEDDING_DIMENSIONS, GenerationIndex, IndexEvidence, IndexRow, ModelIdentity, PrefilterPlan,
    RowFingerprint, SearchHit, VECTOR_COLUMN, index_input_digest,
};
#[cfg(feature = "semantic-native")]
pub use model::OnnxEmbeddingProvider;
