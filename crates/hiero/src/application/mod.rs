//! The application dispatcher (plan M1): the daemon-facing boundary over the
//! domain library. It holds the resolved configuration and the recall
//! service; every request opens the existing short-lived store APIs
//! (`Registry`, `WorkspaceStore`, ...) over the data-root database. There is
//! no second daemon and no direct CLI path here.
//!
//! [`Application::call`] takes the tool name, the raw JSON arguments, and the
//! authenticated actor, fans out across the tool families (see
//! [`series_sessions`], [`memory`], and [`terms`]), and reports every tool
//! that no family claims as [`AppError::NotImplemented`].

pub mod memory;
pub mod series_sessions;
pub mod terms;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::recall::RecallService;

/// Domain-level tool failure modes. `Invalid` covers arguments that do not
/// decode against the frozen input schema (the protocol maps it to JSON-RPC
/// invalid-params); `Domain` covers rejected work (the protocol maps it to a
/// tool error result); `NotImplemented` marks tools no family claims yet.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Domain(String),
    #[error("tool is not implemented: {0}")]
    NotImplemented(String),
}

/// The application: selected config plus long-lived services. Stores are
/// opened per request (short-lived connections by design), so the application
/// itself holds no database connection.
pub struct Application {
    config: HieronymusConfig,
    recall: RecallService,
}

impl std::fmt::Debug for Application {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // RecallService is not `Debug` (it may carry an armed semantic lane
        // behind a mutex); the config identifies the application.
        formatter
            .debug_struct("Application")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Application {
    /// Open the application over a data root: validates the database
    /// (fail closed on unsupported states) exactly like the store APIs will
    /// per request.
    pub fn open(config: &HieronymusConfig) -> Result<Self, AppError> {
        let recall =
            RecallService::open(config).map_err(|error| AppError::Domain(error.to_string()))?;
        Ok(Self {
            config: config.clone(),
            recall,
        })
    }

    /// The resolved data-root configuration.
    pub fn config(&self) -> &HieronymusConfig {
        &self.config
    }

    /// The recall service backing the recall tool family.
    pub fn recall(&self) -> &RecallService {
        &self.recall
    }

    /// Dispatch one tool call. The final parameter is the authenticated
    /// actor (the credential holder the transport authenticated).
    pub fn call(
        &self,
        tool: &str,
        arguments: &serde_json::Value,
        actor: &str,
    ) -> Result<serde_json::Value, AppError> {
        series_sessions::dispatch(self, tool, arguments, actor)
            .or_else(|| memory::dispatch(self, tool, arguments, actor))
            .or_else(|| terms::dispatch(self, tool, arguments, actor))
            .unwrap_or_else(|| Err(AppError::NotImplemented(tool.to_string())))
    }
}

/// Decode one tool's arguments against its frozen schema shape.
pub(crate) fn decode<T: serde::de::DeserializeOwned>(
    arguments: &serde_json::Value,
) -> Result<T, AppError> {
    T::deserialize(arguments).map_err(|error| AppError::Invalid(error.to_string()))
}

/// Map any store rejection to a domain failure; the protocol layer decides
/// the envelope.
pub(crate) fn domain<E: std::fmt::Display>(error: E) -> AppError {
    AppError::Domain(error.to_string())
}

/// The Python `_translation_context` rule, shared by every tool that accepts
/// optional language overrides over a registered series: `None` languages
/// fall back to the series' registry defaults, and explicit overrides must
/// match those defaults (mismatches are domain rejections, mirroring the
/// Python `ValueError`s).
pub(crate) fn translation_context(
    series: &hieronymus::registry::Series,
    source_language: Option<String>,
    target_language: Option<String>,
    task_type: &str,
    volume: &str,
    chapter: &str,
) -> Result<TranslationContext, AppError> {
    let source = source_language.unwrap_or_else(|| series.source_language.clone());
    let target = target_language.unwrap_or_else(|| series.target_language.clone());
    if source != series.source_language {
        return Err(AppError::Domain(format!(
            "source_language {source:?} does not match registry default {:?} for series {:?}",
            series.source_language, series.slug
        )));
    }
    if target != series.target_language {
        return Err(AppError::Domain(format!(
            "target_language {target:?} does not match registry default {:?} for series {:?}",
            series.target_language, series.slug
        )));
    }
    Ok(
        TranslationContext::new(series.slug.clone(), source, target, task_type)
            .volume(volume)
            .chapter(chapter),
    )
}
