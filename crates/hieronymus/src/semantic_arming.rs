//! Semantic lane arming (Task 9 review ruling): the real, non-test path from
//! model identity to an armed [`RecallService`].
//!
//! Arming means: the acquired ONNX model is loaded through the verified
//! provider and the recall service is built with [`SemanticLane::new`] over
//! the pinned WordPiece tokenizer (the acquired, SHA-verified tokenizer.json
//! of the model revision). Discipline:
//!
//! - Arming is always an explicit operation. Nothing here is called at config
//!   load, store open, or daemon start; the `hiero semantic enable|status`
//!   subcommands are the production entry points (the daemon's recall tool
//!   will reuse `arm_recall_service` when it lands).
//! - A missing, invalid, or unloadable model never fails: the lane reports
//!   [`LaneState::Disarmed`] with the reason and the service stays the
//!   supported FTS-only degraded mode.
//! - Identity is checked per recall by the lane itself; an armed lane whose
//!   identity no longer matches the active generation (model, revision,
//!   dimensions, limits, or tokenizer) degrades with the structured
//!   `semantic_lane_unavailable` warning, which is what forces a rebuild.

use std::path::{Path, PathBuf};

use crate::data_root::HieronymusConfig;
use crate::recall::{RecallError, RecallService};
use crate::semantic_embeddings::{EmbeddingProvider, OnnxEmbeddingProvider};
use crate::semantic_jobs::ChunkTokenizer;
use crate::semantic_model::ModelStatus;
use crate::semantic_recall::SemanticLane;
use crate::semantic_store::{GenerationManifest, SemanticStore};

/// The semantic settings file under the config root (`semantic.conf`), the
/// same ownership pattern as `dream.conf`/`provider.conf`: an explicit
/// `hiero semantic enable --runtime <lib>` write, validated at daemon
/// startup, retained across restarts.
pub fn semantic_config_path(config: &HieronymusConfig) -> PathBuf {
    config.config_root().join("semantic.conf")
}

/// Persists the configured ONNX runtime location for this data root.
pub fn save_runtime_library(config: &HieronymusConfig, runtime: &Path) -> Result<(), String> {
    std::fs::create_dir_all(config.config_root())
        .map_err(|error| format!("semantic.conf directory: {error}"))?;
    let text = format!(
        "runtime_library = {}\n",
        toml::Value::String(runtime.display().to_string())
    );
    crate::atomic::atomic_write_text(&semantic_config_path(config), &text)
        .map_err(|error| format!("semantic.conf write failed: {error}"))
}

/// The persisted ONNX runtime location, when one was configured.
pub fn load_runtime_library(config: &HieronymusConfig) -> Option<PathBuf> {
    let text = std::fs::read_to_string(semantic_config_path(config)).ok()?;
    let table = text.parse::<toml::Table>().ok()?;
    table
        .get("runtime_library")?
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

/// The outcome of arming: a recall service plus whether the semantic lane is
/// attached to it.
pub struct ArmedRecall {
    pub service: RecallService,
    pub lane: LaneState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaneState {
    /// The ONNX provider loaded; the lane is attached to the service.
    Armed,
    /// The lane could not be attached. The service stays fully functional
    /// FTS-only; `reason` names the missing or invalid piece.
    Disarmed { reason: String },
}

impl LaneState {
    /// The JSON form the CLI and doctor render (`armed` / `disarmed`).
    pub fn as_str(&self) -> &'static str {
        match self {
            LaneState::Armed => "armed",
            LaneState::Disarmed { .. } => "disarmed",
        }
    }
}

/// Arms a recall service from the data root's acquired model. The model must
/// already have been acquired (an explicit operation); when it is absent,
/// invalid, or the ONNX runtime cannot be loaded, the service comes back
/// FTS-only with [`LaneState::Disarmed`] — this function never fails on
/// semantic state, only on database-open errors.
pub fn arm_recall_service(
    config: &HieronymusConfig,
    runtime_library: &Path,
) -> Result<ArmedRecall, RecallError> {
    let service = RecallService::open(config)?;
    match load_lane(config, runtime_library) {
        Ok(lane) => Ok(ArmedRecall {
            service: service.with_semantic_lane(lane),
            lane: LaneState::Armed,
        }),
        Err(reason) => Ok(ArmedRecall {
            service,
            lane: LaneState::Disarmed { reason },
        }),
    }
}

/// Arms a recall service around an explicit provider and tokenizer (the fake
/// provider keeps unit tests offline; production passes the loaded ONNX
/// provider and the pinned model tokenizer). The lane's per-recall identity
/// check still guards against mixed identities.
pub fn arm_with_provider(
    config: &HieronymusConfig,
    provider: Box<dyn EmbeddingProvider>,
    tokenizer: Box<dyn ChunkTokenizer>,
) -> Result<ArmedRecall, RecallError> {
    let service = RecallService::open(config)?;
    let lane = SemanticLane::new(provider, tokenizer);
    Ok(ArmedRecall {
        service: service.with_semantic_lane(lane),
        lane: LaneState::Armed,
    })
}

/// The lane verdict only, without building a service: what `hiero semantic
/// enable` reports after acquisition. Reads the model pre-check and attempts
/// the verified provider load; never downloads and never fails.
pub fn arming_verdict(config: &HieronymusConfig, runtime_library: &Path) -> LaneState {
    match load_lane(config, runtime_library) {
        Ok(_) => LaneState::Armed,
        Err(reason) => LaneState::Disarmed { reason },
    }
}

fn load_lane(config: &HieronymusConfig, runtime_library: &Path) -> Result<SemanticLane, String> {
    let store = SemanticStore::open(config).map_err(|error| error.to_string())?;
    let provider = store
        .load_embedding_provider(runtime_library)
        .map_err(|error| error.to_string())?;
    let tokenizer = store
        .load_model_tokenizer()
        .map_err(|error| error.to_string())?;
    Ok(SemanticLane::new(Box::new(provider), Box::new(tokenizer)))
}

/// Report-only semantic health: what doctor and `hiero semantic status`
/// render. Reads the model pre-check and the manifest state through a
/// read-only connection; never downloads, never loads the runtime, never
/// creates the database, and never opens the vector store.
pub struct SemanticStatus {
    pub model_status: ModelStatus,
    pub active_generation: Option<GenerationManifest>,
    /// Whether the active generation's index survived on disk (`true` when no
    /// generation is active; losing it means a rebuild is required).
    pub generation_intact: bool,
    pub tokenizer: &'static str,
}

pub fn semantic_status(
    config: &HieronymusConfig,
) -> Result<SemanticStatus, crate::semantic_error::SemanticError> {
    let (active_generation, generation_intact) = SemanticStore::probe_active_generation(config)?;
    Ok(SemanticStatus {
        model_status: SemanticStore::model_status_for(config),
        active_generation,
        generation_intact,
        tokenizer: crate::semantic_tokenizer::MINILM_TOKENIZER_ID,
    })
}

/// The pinned model identity for reporting: what `enable` compares the active
/// generation against.
pub fn pinned_identity() -> crate::semantic_embeddings::EmbeddingIdentity {
    OnnxEmbeddingProvider::static_identity()
}
