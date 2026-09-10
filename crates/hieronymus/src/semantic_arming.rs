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

#[derive(Debug, thiserror::Error)]
pub enum SemanticConfigError {
    #[error("cannot read semantic configuration: {0}")]
    Read(#[from] std::io::Error),
    #[error("invalid semantic configuration: {0}")]
    Invalid(String),
}

/// Application-owned embedding selection. Omitted provider preserves old ONNX settings.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticConfiguration {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_library: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub configuration_revision: u64,
}
fn default_provider() -> String {
    "onnx".into()
}
impl SemanticConfiguration {
    pub fn ollama(url: &str, model: &str) -> Self {
        Self {
            provider: "ollama".into(),
            runtime_library: None,
            base_url: Some(url.into()),
            model: Some(model.into()),
            configuration_revision: 0,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        match self.provider.as_str() {
            "onnx" if self.base_url.is_none() && self.model.is_none() => {
                if !self
                    .runtime_library
                    .as_ref()
                    .is_some_and(|p| p.is_absolute())
                {
                    return Err("runtime_library must be an absolute path".into());
                }
                Ok(())
            }
            "ollama" if self.runtime_library.is_none() => {
                crate::ollama_embeddings::validate_endpoint(
                    self.base_url
                        .as_deref()
                        .ok_or("Ollama base_url is required")?,
                    self.model.as_deref().ok_or("Ollama model is required")?,
                )
            }
            _ => Err(
                "provider must be onnx (runtime_library only) or ollama (base_url and model only)"
                    .into(),
            ),
        }
    }
}

pub fn load_configuration(
    config: &HieronymusConfig,
) -> Result<Option<SemanticConfiguration>, SemanticConfigError> {
    let text = match std::fs::read_to_string(semantic_config_path(config)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let settings: SemanticConfiguration =
        toml::from_str(&text).map_err(|error| SemanticConfigError::Invalid(error.to_string()))?;
    settings.validate().map_err(SemanticConfigError::Invalid)?;
    Ok(Some(settings))
}

/// Persist under the existing single writer/revision authority, without model requests.
pub fn save_configuration(
    config: &HieronymusConfig,
    settings: &SemanticConfiguration,
) -> Result<(), String> {
    settings.validate()?;
    let mut settings = settings.clone();
    if let Some(runtime) = &settings.runtime_library {
        if !runtime.is_file() {
            return Err("runtime_library must name an existing absolute file".into());
        }
        settings.runtime_library = Some(runtime.canonicalize().map_err(|e| e.to_string())?);
    }
    settings.configuration_revision = configuration_revision(config)
        .map_err(|e| e.to_string())?
        .checked_add(1)
        .ok_or("semantic configuration revision exhausted")?;
    std::fs::create_dir_all(config.config_root()).map_err(|e| e.to_string())?;
    let text = toml::to_string(&settings).map_err(|e| e.to_string())?;
    crate::atomic::atomic_write_text(&semantic_config_path(config), &text)
        .map_err(|e| format!("semantic.conf write failed: {e}"))
}

pub fn save_runtime_library(config: &HieronymusConfig, runtime: &Path) -> Result<(), String> {
    save_configuration(
        config,
        &SemanticConfiguration {
            provider: "onnx".into(),
            runtime_library: Some(runtime.into()),
            base_url: None,
            model: None,
            configuration_revision: 0,
        },
    )
}

pub fn configuration_revision(config: &HieronymusConfig) -> Result<u64, SemanticConfigError> {
    Ok(load_configuration(config)?.map_or(0, |settings| settings.configuration_revision))
}

/// Qualified native runtime identity for the current release line.
pub const RUNTIME_SHA256: &str = "1461ef7cc3d9e49982591721683cc3e3a55580aeca9a5254e7aac47b75ee4bab";
pub const RUNTIME_VERSION: &str = "1.28.0";

/// Resolve from the canonical executable, never a stable link or cwd. A
/// managed version with missing assets remains a broken bundle, not a cue to
/// reuse another version's assets from the data root.
pub fn bundled_asset_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let directory = exe.parent()?;
    (directory.join("assets.json").exists() || directory.parent()?.file_name()? == "versions")
        .then(|| directory.to_path_buf())
}

pub fn verify_runtime_library(runtime: &Path) -> Result<(), String> {
    let digest = crate::semantic_model::sha256_file(runtime).map_err(|e| e.to_string())?;
    if digest != RUNTIME_SHA256 {
        return Err(format!(
            "ONNX runtime {} checksum mismatch; expected qualified runtime {RUNTIME_VERSION} ({RUNTIME_SHA256}), got {digest}",
            runtime.display()
        ));
    }
    Ok(())
}

/// Missing configuration is distinct from unreadable or malformed settings.
pub fn load_runtime_library(
    config: &HieronymusConfig,
) -> Result<Option<PathBuf>, SemanticConfigError> {
    match load_configuration(config)? {
        Some(settings) => Ok(settings.runtime_library),
        None => Ok(bundled_asset_root().map(|root| root.join("lib/libonnxruntime.so"))),
    }
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
    let provider: Box<dyn EmbeddingProvider> =
        match load_configuration(config).map_err(|e| e.to_string())? {
            Some(settings) if settings.provider == "ollama" => Box::new(
                crate::ollama_embeddings::OllamaEmbeddingProvider::load(
                    settings.base_url.as_deref().unwrap(),
                    settings.model.as_deref().unwrap(),
                )
                .map_err(|e| e.to_string())?,
            ),
            _ => Box::new(
                store
                    .load_embedding_provider(runtime_library)
                    .map_err(|e| e.to_string())?,
            ),
        };
    let tokenizer = store
        .load_model_tokenizer()
        .map_err(|error| error.to_string())?;
    Ok(SemanticLane::new(provider, Box::new(tokenizer)))
}

/// Report-only semantic health: what doctor and `hiero semantic status`
/// render. Reads the model pre-check and the manifest state through a
/// read-only connection; never downloads, never loads the runtime, never
/// creates the database, and never opens the vector store.
pub struct SemanticStatus {
    pub configuration: Option<SemanticConfiguration>,
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
        configuration: load_configuration(config)
            .map_err(|e| crate::semantic_error::SemanticError::Store(e.to_string()))?,
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
