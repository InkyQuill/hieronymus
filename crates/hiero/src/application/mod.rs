//! The application dispatcher (plan M1): the daemon-facing boundary over the
//! domain library. It holds the resolved configuration and the recall
//! service; every request opens the existing short-lived store APIs
//! (`Registry`, `WorkspaceStore`, ...) over the data-root database. There is
//! no second daemon and no direct CLI path here.
//!
//! [`Application::call`] takes the tool name, the raw JSON arguments, and the
//! authenticated actor, fans out across the tool families (see
//! [`series_sessions`]), and reports every tool that no family claims as
//! [`AppError::NotImplemented`].

pub mod series_sessions;

use hieronymus::data_root::HieronymusConfig;
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
            .unwrap_or_else(|| Err(AppError::NotImplemented(tool.to_string())))
    }
}
