//! The application dispatcher (plan M1): the daemon-facing boundary over the
//! domain library. It holds the resolved configuration and the recall
//! service; every request opens the existing short-lived store APIs
//! (`Registry`, `WorkspaceStore`, ...) over the data-root database. There is
//! no second daemon and no direct CLI path here.
//!
//! [`Application::call`] takes the tool name, the raw JSON arguments, and the
//! authenticated actor, fans out across the tool families (see
//! [`series_sessions`], [`memory`], [`terms`], [`graph`], and [`dream`]), and
//! reports every tool that no family claims as [`AppError::NotImplemented`].
//! Since plan M5 no advertised tool falls through: [`Application::implemented_tools`]
//! lists the concrete handlers and the `tool_completeness` regression pins it
//! to the frozen registry snapshot.

pub mod admin;
pub mod authority;
pub(crate) mod authority_selection;
mod authority_signal;
pub mod correction_parser;
pub mod dream;
pub mod graph;
pub mod memory;
pub mod series_sessions;
pub mod terms;

use std::sync::{Arc, OnceLock, RwLock, RwLockReadGuard};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::recall::{RecallService, SemanticAvailability, SemanticStatusHook};
use hieronymus::semantic_recall::SemanticLane;

/// Post-commit rebuild notifier installed by the daemon (Task S2): RAG
/// import queues a durable semantic rebuild through it. `None` when no
/// daemon owns this application.
pub type RebuildHook = Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

use crate::daemon::dream_worker::DreamController;

/// Domain-level tool failure modes. `Invalid` covers arguments that do not
/// decode against the frozen input schema (the protocol maps it to JSON-RPC
/// invalid-params); `Domain` covers rejected work (the protocol maps it to a
/// tool error result); `NotImplemented` marks tools no family claims yet.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Authority(#[from] hieronymus::authority_models::DecisionErrorV1),
    #[error(transparent)]
    Coherent(#[from] hieronymus::coherent_reads::CoherentReadError),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Domain(String),
    #[error("tool is not implemented: {0}")]
    NotImplemented(String),
}

/// The application: selected config plus long-lived services. Stores are
/// opened per request (short-lived connections by design), so the application
/// itself holds no database connection. The recall service sits behind a
/// read-write lock so the semantic controller (Task S2) can install/refresh
/// the query-time semantic lane after verified generation activations without
/// rebuilding the application.
pub struct Application {
    config: HieronymusConfig,
    recall: RwLock<RecallService>,
    rebuild_hook: RwLock<Option<RebuildHook>>,
    /// The daemon's semantic-readiness probe (task C5), installed by
    /// [`crate::daemon::Daemon::start`] over its semantic controller. It is
    /// remembered here as well as handed to the recall service, because a
    /// later lane installation rebuilds that service and must not drop it.
    /// When absent — a bare `Application::open` that is not serving a daemon
    /// — there is no semantic service at all, and strict semantic search
    /// fails closed rather than answering lexically.
    semantic_status: RwLock<Option<SemanticStatusHook>>,
    /// The daemon's dream controller (task D5), installed by
    /// [`crate::daemon::Daemon::start`] exactly once. When absent — a bare
    /// `Application::open` that is not serving a daemon — the dream
    /// dispatch fails closed instead of constructing a provider itself.
    dream: OnceLock<DreamController>,
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
            recall: RwLock::new(recall),
            rebuild_hook: RwLock::new(None),
            semantic_status: RwLock::new(None),
            dream: OnceLock::new(),
        })
    }

    /// The resolved data-root configuration.
    pub fn config(&self) -> &HieronymusConfig {
        &self.config
    }

    /// The recall service backing the recall tool family. The read guard
    /// derefs to the service; the semantic controller takes the write side
    /// (rare, on verified activation) to swap in a freshly armed lane.
    pub fn recall(&self) -> RwLockReadGuard<'_, RecallService> {
        self.recall
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Installs (or refreshes) the query-time semantic lane. Called by the
    /// semantic controller on first arming and on every verified generation
    /// activation, so queries always run on a coherent identity and
    /// generation.
    ///
    /// A failed reopen keeps the previous service in place and is reported to
    /// the caller: whether a query lane is installed is the load-bearing fact
    /// behind `RequiredSemanticState::Ready` (Task C3), so swallowing this
    /// error would let the controller advertise a lane that never arrived.
    pub fn install_semantic_lane(&self, lane: SemanticLane) -> Result<(), String> {
        let mut guard = self
            .recall
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let service = RecallService::open(&self.config).map_err(|error| {
            format!(
                "could not reopen the recall service over {}: {error}",
                self.config.database_path().display()
            )
        })?;
        let mut service = service.with_semantic_lane(lane);
        // The reopen builds a fresh service, so the installed availability
        // probe has to be carried over or a lane refresh would silently make
        // recall stop consuming the shared semantic status (task C5).
        if let Some(hook) = self
            .semantic_status
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            service.set_semantic_status(hook);
        }
        *guard = service;
        Ok(())
    }

    /// Installs the shared semantic-readiness probe (task C5, review finding
    /// A5): the daemon's analogue of [`Self::set_rebuild_hook`], reading its
    /// semantic controller's `RequiredSemanticState` through
    /// `require_semantic_ready`.
    ///
    /// This is the single seam by which the application learns whether
    /// required semantics can actually serve a query. `install_semantic_lane`
    /// only ever said that a lane was armed, which is not the same claim: a
    /// lane can be armed over a generation that does not cover the current
    /// corpus, and a cold or failed runtime arms nothing at all while lexical
    /// search keeps answering.
    ///
    /// Installing is order-independent with respect to lane installation:
    /// whichever lands second re-applies the other.
    ///
    /// The two locks are taken one at a time on purpose. `install_semantic_lane`
    /// holds the recall write guard and then reads this field, so holding this
    /// field's guard while waiting for the recall lock here would be a lock
    /// order inversion between two paths the semantic worker can drive
    /// concurrently.
    pub fn install_semantic_status(&self, hook: SemanticStatusHook) {
        {
            let mut remembered = self
                .semantic_status
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *remembered = Some(Arc::clone(&hook));
        }
        self.recall
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_semantic_status(hook);
    }

    /// The shared semantic service's verdict, or `None` when no semantic
    /// service is attached to this application.
    pub(crate) fn semantic_availability(&self) -> Option<SemanticAvailability> {
        let guard = self
            .semantic_status
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.as_ref().map(|hook| hook())
    }

    /// The strict gate for tools whose contract IS semantic retrieval: the
    /// service must be attached and ready, or the call fails with the
    /// service's own actionable reason.
    ///
    /// A missing hook is a failure, not a pass. The owner constraint is that
    /// both working memory and semantic RAG are mandatory — FTS-only is not a
    /// release alternative — so "no semantic service is attached" cannot be
    /// answered with lexical rows that look exactly like complete ones.
    pub(crate) fn require_semantic_service(&self) -> Result<(), AppError> {
        match self.semantic_availability() {
            Some(SemanticAvailability::Ready) => Ok(()),
            Some(SemanticAvailability::Unavailable(reason)) => Err(AppError::Domain(reason)),
            None => Err(AppError::Domain(
                "semantic retrieval unavailable: no semantic service is attached to this data \
                 root; semantic RAG search requires a running daemon with semantics enabled \
                 (`hiero semantic enable --runtime <libonnxruntime.so>`)"
                    .to_string(),
            )),
        }
    }

    /// Installs the daemon's post-commit rebuild notifier (Task S2).
    pub fn set_rebuild_hook(&self, hook: RebuildHook) {
        *self
            .rebuild_hook
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(hook);
    }

    /// Queues a durable semantic rebuild through the installed hook (used by
    /// RAG import after its authoritative commit). `None` when no daemon
    /// owns this application; hook failures surface to the caller without
    /// failing the import.
    ///
    /// Both non-queued outcomes are safe to report rather than retry here
    /// (task C4): the import wrote a durable `semantic_work_intent` inside its
    /// own transaction, so the owed indexing is recorded in SQLite whether or
    /// not this best-effort notification lands, and startup/periodic
    /// reconciliation queues it from that record. What callers must NOT be
    /// told is that indexing was queued when it was not — see
    /// `memory::rag_import`.
    pub(crate) fn request_rebuild(&self, series_slug: &str) -> Option<Result<String, String>> {
        let guard = self
            .rebuild_hook
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.as_ref().map(|hook| hook(series_slug))
    }

    /// Install the daemon's dream controller. Called once by
    /// [`crate::daemon::Daemon::start`]; a second install is refused (the
    /// first controller keeps serving).
    pub fn install_dream_controller(&self, controller: DreamController) {
        let _ = self.dream.set(controller);
    }

    /// The installed dream controller, if any.
    pub(crate) fn dream_controller(&self) -> Option<&DreamController> {
        self.dream.get()
    }

    /// Dispatch one tool call. The final parameter is the authenticated
    /// actor (the credential holder the transport authenticated).
    pub fn call(
        &self,
        tool: &str,
        arguments: &serde_json::Value,
        actor: &str,
    ) -> Result<serde_json::Value, AppError> {
        authority::dispatch(self, tool, arguments)
            .or_else(|| series_sessions::dispatch(self, tool, arguments, actor))
            .or_else(|| memory::dispatch(self, tool, arguments, actor))
            .or_else(|| terms::dispatch(self, tool, arguments, actor))
            .or_else(|| graph::dispatch(self, tool, arguments, actor))
            .or_else(|| dream::dispatch(self, tool, arguments, actor))
            .unwrap_or_else(|| Err(AppError::NotImplemented(tool.to_string())))
    }

    /// Every advertised tool this application serves with a concrete handler,
    /// listed by name. This is the completeness surface (plan M5): the
    /// `tool_completeness` regression compares it against the frozen registry
    /// snapshot, so a newly advertised tool without a handler fails the gate
    /// instead of answering `NotImplemented` at runtime. `hieronymus_status`
    /// is included: its handler is the registry-backed frozen contract rather
    /// than a [`Self::call`] family member, but it is concrete daemon code.
    pub fn implemented_tools() -> &'static [&'static str] {
        &[
            "hieronymus_decide",
            "hieronymus_correct",
            "hieronymus_order_register",
            "hieronymus_evidence_capture",
            // series/sessions family (M1).
            "hieronymus_series_create",
            "hieronymus_series_init",
            "hieronymus_series_list",
            "hieronymus_series_set_language_tags",
            "hieronymus_session_start",
            "hieronymus_session_complete",
            // memory family (M2).
            "hieronymus_memory_add",
            "hieronymus_memory_search",
            "hieronymus_short_term_add",
            "hieronymus_short_term_add_batch",
            "hieronymus_feedback",
            "hieronymus_recall",
            "hieronymus_rag_import",
            "hieronymus_rag_search",
            // terms/rule-lifecycle family (M3).
            "hieronymus_termbase_propose",
            "hieronymus_termbase_approve",
            "hieronymus_termbase_contract",
            "hieronymus_termbase_validate",
            "hieronymus_rule_crystal_archive",
            "hieronymus_rule_crystal_validate",
            "hieronymus_rule_crystals_list",
            // graph family (M4).
            "hieronymus_concept_create",
            "hieronymus_concept_get",
            "hieronymus_concept_list",
            "hieronymus_concept_update",
            "hieronymus_concept_archive",
            "hieronymus_concept_merge",
            "hieronymus_concept_rename",
            "hieronymus_concept_semantic_tags_set",
            "hieronymus_concept_facet_add",
            "hieronymus_concept_facet_update",
            "hieronymus_concept_facet_list",
            "hieronymus_concept_facet_set_canonical",
            "hieronymus_crystal_link_concept",
            "hieronymus_crystal_story_scopes_set",
            "hieronymus_crystal_semantic_tags_set",
            "hieronymus_concept_proposals_list",
            // dream family (M5).
            "hieronymus_dream",
            // registry-backed frozen status contract.
            "hieronymus_status",
        ]
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

/// Registry languages are defaults; an explicit task direction may use another
/// normalized, nonempty language pair in the same common-work series.
pub(crate) fn translation_context(
    series: &hieronymus::registry::Series,
    source_language: Option<String>,
    target_language: Option<String>,
    task_type: &str,
    volume: &str,
    chapter: &str,
) -> Result<TranslationContext, AppError> {
    let source = context_language(source_language, &series.source_language, "source_language")?;
    let target = context_language(target_language, &series.target_language, "target_language")?;
    Ok(
        TranslationContext::new(series.slug.clone(), source, target, task_type)
            .volume(volume)
            .chapter(chapter),
    )
}

fn context_language(value: Option<String>, default: &str, field: &str) -> Result<String, AppError> {
    let Some(value) = value else {
        return Ok(default.to_owned());
    };
    let value = value.trim().to_lowercase();
    if value.is_empty() {
        return Err(AppError::Domain(format!("{field} must not be empty")));
    }
    Ok(value)
}

/// Narrative overrides are explicit for this request; research never becomes
/// a stored default. None preserves the durable session's supplied viewpoint.
#[derive(Default, serde::Deserialize)]
pub(crate) struct StoryReadArgs {
    #[serde(default)]
    pub story_scopes: Option<Vec<String>>,
    #[serde(default)]
    pub story_timeline_id: Option<i64>,
    #[serde(default)]
    pub story_scene_key: Option<String>,
    #[serde(default)]
    pub story_viewpoint: Option<hieronymus::story_applicability::Viewpoint>,
    #[serde(default)]
    pub story_query_mode: hieronymus::story_applicability::QueryMode,
    #[serde(default)]
    pub required_decision_id: Option<String>,
}
impl StoryReadArgs {
    pub fn apply(&self, context: &mut TranslationContext) -> Result<(), AppError> {
        if let Some(scopes) = &self.story_scopes {
            for scope in scopes {
                if scope.is_empty() || scope.trim() != scope || scope.chars().any(char::is_control)
                {
                    return Err(AppError::Domain(
                        "story_scopes require nonempty, unpadded predicates".into(),
                    ));
                }
                for prefix in ["cws:direction:", "cws:edition:"] {
                    if let Some(id) = scope.strip_prefix(prefix)
                        && (id.is_empty()
                            || !id.split('-').all(|part| {
                                !part.is_empty()
                                    && part
                                        .bytes()
                                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
                            }))
                    {
                        return Err(AppError::Domain(
                            "story_scopes contain an invalid CWS identity".into(),
                        ));
                    }
                }
            }
            let mut direction = None;
            for scope in context.story_scopes.iter().chain(scopes) {
                if let Some(id) = scope.strip_prefix("cws:direction:") {
                    if direction.is_some_and(|previous| previous != id) {
                        return Err(AppError::Domain(
                            "story_scopes contain conflicting CWS directions".into(),
                        ));
                    }
                    direction = Some(id);
                }
            }
            for scope in scopes {
                if !context.story_scopes.contains(scope) {
                    context.story_scopes.push(scope.clone());
                }
            }
        }
        if let Some(id) = self.story_timeline_id {
            context.story_timeline_id = Some(id);
        }
        if let Some(scene) = &self.story_scene_key {
            context.story_scene_key = Some(scene.clone());
        }
        if let Some(viewpoint) = self.story_viewpoint {
            context.story_viewpoint = viewpoint;
        }
        context.story_query_mode = self.story_query_mode;
        Ok(())
    }
}
pub(crate) fn recall_error(error: hieronymus::recall::RecallError) -> AppError {
    match error {
        hieronymus::recall::RecallError::Coherent(e) => AppError::Coherent(e),
        other => domain(other),
    }
}
pub(crate) fn term_read_error(error: hieronymus::terminology::TermbaseError) -> AppError {
    match error {
        hieronymus::terminology::TermbaseError::Coherent(e) => AppError::Coherent(e),
        other => domain(other),
    }
}
