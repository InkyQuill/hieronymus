//! The series/session tool family: `hieronymus_series_create`,
//! `hieronymus_series_init` (the compatibility wrapper over the same
//! registered operation), `hieronymus_series_list`,
//! `hieronymus_series_set_language_tags`, `hieronymus_session_start`, and
//! `hieronymus_session_complete`.
//!
//! Argument structs decode exactly the frozen `input_schema` rules from
//! `compatibility/snapshots/mcp.json` (required fields, defaults, and null
//! rules); decoding failures are [`AppError::Invalid`]. Store rejections are
//! [`AppError::Domain`]. All database work stays in the `hieronymus` store
//! APIs — no SQL lives here.

use serde::Deserialize;
use serde_json::{Value, json};

use hieronymus::registry::{Registry, Series};
use hieronymus::workspace::WorkspaceStore;

use super::AppError;
use super::Application;
use super::decode;
use super::domain;
use super::translation_context;

/// The family dispatcher: `None` means the tool is not ours.
pub(crate) fn dispatch(
    application: &Application,
    tool: &str,
    arguments: &Value,
    actor: &str,
) -> Option<Result<Value, AppError>> {
    let _ = actor; // No tool in this family is actor-scoped yet.
    match tool {
        "hieronymus_series_create" | "hieronymus_series_init" => {
            Some(create_series(application, arguments))
        }
        "hieronymus_series_list" => Some(list_series(application)),
        "hieronymus_series_set_language_tags" => Some(set_language_tags(application, arguments)),
        "hieronymus_session_start" => Some(session_start(application, arguments)),
        "hieronymus_session_complete" => Some(session_complete(application, arguments)),
        _ => None,
    }
}

#[derive(Deserialize)]
struct CreateSeries {
    slug: String,
    title: String,
    #[serde(default)]
    source_language: String,
    #[serde(default)]
    target_language: String,
    #[serde(default)]
    language_tags: Option<Vec<String>>,
}

/// `hieronymus_series_create` and its compatibility wrapper
/// `hieronymus_series_init`: opening the registry applies the Rust schema to
/// fresh databases (the Python wrapper's project-setup side effect), then the
/// series is upserted by slug.
fn create_series(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<CreateSeries>(arguments)?;
    let registry = Registry::open(application.config()).map_err(domain)?;
    let series = registry
        .create_series(
            &args.slug,
            &args.title,
            &args.source_language,
            &args.target_language,
            args.language_tags.as_deref(),
        )
        .map_err(domain)?;
    Ok(series_payload(&series))
}

fn list_series(application: &Application) -> Result<Value, AppError> {
    let registry = Registry::open(application.config()).map_err(domain)?;
    let series = registry.list_series().map_err(domain)?;
    Ok(Value::Array(series.iter().map(series_payload).collect()))
}

#[derive(Deserialize)]
struct SetLanguageTags {
    series_id: i64,
    language_tags: Vec<String>,
}

fn set_language_tags(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<SetLanguageTags>(arguments)?;
    let registry = Registry::open(application.config()).map_err(domain)?;
    registry
        .set_series_language_tags(args.series_id, &args.language_tags)
        .map_err(domain)?;
    registry
        .list_series()
        .map_err(domain)?
        .into_iter()
        .find(|series| series.id == Some(args.series_id))
        .map(|series| series_payload(&series))
        .ok_or_else(|| {
            domain(hieronymus::registry::RegistryError::UnknownSeriesId(
                args.series_id,
            ))
        })
}

#[derive(Deserialize)]
struct SessionStart {
    series_slug: String,
    #[serde(default)]
    source_language: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default = "default_task_type")]
    task_type: String,
    #[serde(default)]
    volume: String,
    #[serde(default)]
    chapter: String,
}

fn default_task_type() -> String {
    "translation".to_string()
}

/// Start a session for an existing series, mirroring the Python
/// `_translation_context` rules: `None` languages fall back to the series'
/// registry defaults, and explicit overrides must match those defaults.
fn session_start(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<SessionStart>(arguments)?;
    let registry = Registry::open(application.config()).map_err(domain)?;
    let series = registry.get_series(&args.series_slug).map_err(domain)?;
    let context = translation_context(
        &series,
        args.source_language,
        args.target_language,
        &args.task_type,
        &args.volume,
        &args.chapter,
    )?;
    let store = WorkspaceStore::open(application.config()).map_err(domain)?;
    let session = store.start_session(&context).map_err(domain)?;
    Ok(json!({ "session_id": session.id }))
}

#[derive(Deserialize)]
struct SessionComplete {
    session_id: i64,
}

/// Complete a session so it can be dreamed. Like the Python tool, a session
/// that already completed is reported the same as a fresh completion.
fn session_complete(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<SessionComplete>(arguments)?;
    let store = WorkspaceStore::open(application.config()).map_err(domain)?;
    store.complete_session(args.session_id).map_err(domain)?;
    Ok(json!({ "session_id": args.session_id, "completed": true }))
}

/// The series payload, projected field by field (never a serialized store
/// type); `id` appears only for persisted series, matching the Python
/// `_series_payload`.
fn series_payload(series: &Series) -> Value {
    let mut payload = json!({
        "slug": series.slug,
        "title": series.title,
        "source_language": series.source_language,
        "target_language": series.target_language,
        "language_tags": series.language_tags,
    });
    if let Some(id) = series.id {
        payload["id"] = json!(id);
    }
    payload
}
