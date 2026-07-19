mod config;
mod service;

pub use config::{IngestConfig, LearnLimits};
pub use service::{
    IngestError, IngestionService, LearnInput, LearnResult, LearningBlock, ReadInput, ReadResult,
    extract_terms, split_blocks,
};
