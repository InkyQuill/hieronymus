use std::time::Duration;

use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    routing::{delete, get, post},
};
use chrono::Utc;
use hiero_core::provider::{
    CredentialSource, ModelCache, ProviderCatalog, ProviderProfile, ProviderRegistry,
    ReqwestTransportOptions,
};
use serde_json::json;

use crate::daemon::{AppState, RequestId};

use super::{
    contracts::{
        ModelsResponse, ProviderCheck, ProviderCheckResponse, ProviderContract, ProviderResponse,
        ProvidersResponse, SaveProviderRequest,
    },
    error::ApiError,
};

const CACHE_ENTRIES: usize = 128;
const CACHE_BYTES: usize = 1_000_000;
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(save))
        .route("/{id}", delete(remove))
        .route("/{id}/models", get(models))
        .route("/{id}/check", post(check))
}

async fn list(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<ProvidersResponse>, ApiError> {
    let catalog = load_catalog(&state, request_id).await?;
    let providers = catalog
        .iter()
        .map(|(_, profile)| ProviderContract::from_profile(profile, &catalog))
        .collect();
    Ok(Json(ProvidersResponse { providers }))
}

async fn save(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(request): Json<SaveProviderRequest>,
) -> Result<Json<ProviderResponse>, ApiError> {
    let mut catalog = load_catalog(&state, request_id.clone()).await?;
    let draft = request.provider;
    let timeout = draft
        .timeout_seconds
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| ApiError::bad_request(request_id.clone(), "invalid provider timeout"))?;
    let existing_credential = catalog
        .get(&draft.id)
        .map(|profile| profile.credential().clone())
        .unwrap_or(CredentialSource::None);
    let mut profile = ProviderProfile::new(draft.id, draft.name, draft.provider_type, draft.url)
        .with_timeout(
            Duration::try_from_secs_f64(timeout)
                .map_err(|error| ApiError::internal(request_id.clone(), &error))?,
        );
    profile = if draft.key.is_empty() {
        profile.with_credential_source(existing_credential)
    } else {
        profile.with_inline_credential(draft.key)
    };
    catalog
        .upsert(profile.clone())
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
    let provider = ProviderContract::from_profile(&profile, &catalog);
    save_catalog(catalog, state.config.provider_config_path(), request_id).await?;
    Ok(Json(ProviderResponse { provider }))
}

async fn remove(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut catalog = load_catalog(&state, request_id.clone()).await?;
    if !catalog.delete(&id) {
        return Err(ApiError::not_found(request_id, "provider not found"));
    }
    save_catalog(catalog, state.config.provider_config_path(), request_id).await?;
    Ok(Json(json!({})))
}

async fn models(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<Json<ModelsResponse>, ApiError> {
    let catalog = load_catalog(&state, request_id.clone()).await?;
    let profile = catalog
        .get(&id)
        .ok_or_else(|| ApiError::not_found(request_id.clone(), "provider not found"))?;
    let (registry, cache) = registry(&state, request_id.clone()).await?;
    let models = registry
        .suggest_models(profile, Utc::now())
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
    save_cache(cache, state.config.llm_cache_path(), request_id).await?;
    Ok(Json(ModelsResponse { models }))
}

async fn check(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(id): Path<String>,
    Json(_request): Json<serde_json::Value>,
) -> Result<Json<ProviderCheckResponse>, ApiError> {
    let catalog = load_catalog(&state, request_id.clone()).await?;
    let profile = catalog
        .get(&id)
        .ok_or_else(|| ApiError::not_found(request_id.clone(), "provider not found"))?;
    let (registry, cache) = registry(&state, request_id.clone()).await?;
    let result = registry.suggest_models(profile, Utc::now()).await;
    let check = match result {
        Ok(models) => {
            save_cache(cache, state.config.llm_cache_path(), request_id).await?;
            ProviderCheck {
                ok: true,
                models,
                source: "live".into(),
                error: String::new(),
            }
        }
        Err(error) => ProviderCheck {
            ok: false,
            models: Vec::new(),
            source: "live".into(),
            error: error.to_string(),
        },
    };
    Ok(Json(ProviderCheckResponse { check }))
}

async fn load_catalog(
    state: &AppState,
    request_id: RequestId,
) -> Result<ProviderCatalog, ApiError> {
    let path = state.config.provider_config_path();
    tokio::task::spawn_blocking(move || ProviderCatalog::load(path))
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?
        .map_err(|error| ApiError::internal(request_id, &error))
}

async fn save_catalog(
    catalog: ProviderCatalog,
    path: std::path::PathBuf,
    request_id: RequestId,
) -> Result<(), ApiError> {
    tokio::task::spawn_blocking(move || catalog.save(path))
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?
        .map_err(|error| ApiError::internal(request_id, &error))
}

async fn registry(
    state: &AppState,
    request_id: RequestId,
) -> Result<(ProviderRegistry, ModelCache), ApiError> {
    let path = state.config.llm_cache_path();
    let cache = tokio::task::spawn_blocking(move || {
        ModelCache::load(path, CACHE_ENTRIES, CACHE_BYTES, CACHE_TTL, Utc::now())
    })
    .await
    .map_err(|error| ApiError::internal(request_id.clone(), &error))?
    .map_err(|error| ApiError::internal(request_id, &error))?;
    let registry = ProviderRegistry::production(cache.clone(), ReqwestTransportOptions::default());
    Ok((registry, cache))
}

async fn save_cache(
    cache: ModelCache,
    path: std::path::PathBuf,
    request_id: RequestId,
) -> Result<(), ApiError> {
    tokio::task::spawn_blocking(move || cache.save(path))
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?
        .map_err(|error| ApiError::internal(request_id, &error))
}
