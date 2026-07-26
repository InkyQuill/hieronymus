use std::{collections::BTreeMap, time::Duration};

use axum::{Extension, Json, Router, extract::State, routing::get};
use chrono::Utc;
use hiero_core::{
    config::ReleaseConfig,
    dreaming::DreamConfig,
    ingest::IngestConfig,
    provider::{ModelCache, ProviderCatalog, ProviderRegistry, ReqwestTransportOptions},
};

use crate::daemon::{AppState, RequestId};

use super::{
    contracts::{
        CachedModels, DreamRequest, DreamResponse, DreamSettings, IngestRequest,
        ModelCacheContract, ProviderContract, ReleaseRequest, ReleaseSettings,
    },
    error::ApiError,
};

const CACHE_ENTRIES: usize = 128;
const CACHE_BYTES: usize = 1_000_000;
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dream", get(load_dream).post(save_dream))
        .route("/ingest", get(load_ingest).post(save_ingest))
        .route("/release", get(load_release).post(save_release))
}

async fn load_dream(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<DreamResponse>, ApiError> {
    let config_root = state.config.clone();
    let config = tokio::task::spawn_blocking(move || DreamConfig::load(&config_root))
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
    let provider_path = state.config.provider_config_path();
    let cache_path = state.config.llm_cache_path();
    let (catalog, cache) = tokio::task::spawn_blocking(move || {
        let catalog = ProviderCatalog::load(provider_path)?;
        let cache = ModelCache::load(
            cache_path,
            CACHE_ENTRIES,
            CACHE_BYTES,
            CACHE_TTL,
            Utc::now(),
        )?;
        Ok::<_, hiero_core::provider::ProviderError>((catalog, cache))
    })
    .await
    .map_err(|error| ApiError::internal(request_id.clone(), &error))?
    .map_err(|error| ApiError::internal(request_id, &error))?;
    let providers = catalog
        .iter()
        .map(|(_, profile)| ProviderContract::from_profile(profile, &catalog))
        .collect();
    let registry = ProviderRegistry::production(cache, ReqwestTransportOptions::default());
    let mut cached_providers = BTreeMap::new();
    for (id, profile) in catalog.iter() {
        if let Ok(Some(models)) = registry.cached_models(profile, Utc::now()).await {
            cached_providers.insert(id.to_owned(), CachedModels { models });
        }
    }
    Ok(Json(DreamResponse {
        dream: DreamSettings::from(&config),
        providers,
        model_cache: ModelCacheContract {
            providers: cached_providers,
        },
    }))
}

async fn save_dream(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(request): Json<DreamRequest>,
) -> Result<Json<DreamRequest>, ApiError> {
    let contract = request.dream;
    let config = DreamConfig::try_from(contract.clone())
        .map_err(|_| ApiError::bad_request(request_id.clone(), "invalid dream settings"))?;
    let config_root = state.config.clone();
    tokio::task::spawn_blocking(move || config.save(&config_root))
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?
        .map_err(|error| ApiError::internal(request_id, &error))?;
    state.events.notify_refresh();
    Ok(Json(DreamRequest { dream: contract }))
}

async fn load_ingest(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<IngestRequest<IngestConfig>>, ApiError> {
    let path = state.config.ingest_config_path();
    let ingest = tokio::task::spawn_blocking(move || IngestConfig::load(path))
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?
        .map_err(|error| ApiError::internal(request_id, &error))?;
    Ok(Json(IngestRequest { ingest }))
}

async fn save_ingest(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(request): Json<IngestRequest<IngestConfig>>,
) -> Result<Json<IngestRequest<IngestConfig>>, ApiError> {
    let ingest = request.ingest;
    let path = state.config.ingest_config_path();
    tokio::task::spawn_blocking(move || ingest.save(path))
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?
        .map_err(|error| ApiError::internal(request_id, &error))?;
    state.events.notify_refresh();
    Ok(Json(request))
}

async fn load_release(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<ReleaseRequest>, ApiError> {
    let config = state.config.clone();
    let release = tokio::task::spawn_blocking(move || ReleaseConfig::load(&config))
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?
        .map_err(|error| ApiError::internal(request_id, &error))?;
    Ok(Json(ReleaseRequest {
        release: ReleaseSettings {
            update_channel: release.update_channel,
        },
    }))
}

async fn save_release(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(request): Json<ReleaseRequest>,
) -> Result<Json<ReleaseRequest>, ApiError> {
    if !matches!(request.release.update_channel.as_str(), "stable" | "dev") {
        return Err(ApiError::bad_request(
            request_id,
            "update channel must be stable or dev",
        ));
    }
    let release = ReleaseConfig {
        update_channel: request.release.update_channel.clone(),
    };
    let config = state.config.clone();
    tokio::task::spawn_blocking(move || release.save(&config))
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?
        .map_err(|error| ApiError::internal(request_id, &error))?;
    state.events.notify_refresh();
    Ok(Json(request))
}
