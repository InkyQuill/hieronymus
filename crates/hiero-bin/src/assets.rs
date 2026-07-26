use std::{
    env,
    ffi::OsString,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, bail};
use axum::{
    body::Body,
    extract::{Extension, OriginalUri, Path as AxumPath, State},
    http::{
        HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

use crate::{
    api::error::ApiError,
    daemon::{AppState, RequestId},
};

const NO_CACHE: &str = "no-cache";
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";
const SHORT_CACHE: &str = "public, max-age=3600";

#[derive(RustEmbed)]
#[folder = "../../frontend/dist/"]
#[allow_missing = true]
pub struct Asset;

#[derive(Clone, Debug)]
pub enum AssetSource {
    Embedded,
    Override(Arc<PathBuf>),
}

impl AssetSource {
    #[must_use]
    pub const fn embedded() -> Self {
        Self::Embedded
    }

    pub fn override_root(root: impl AsRef<Path>) -> Result<Self> {
        let Ok(root) = root.as_ref().canonicalize() else {
            bail!("configured frontend asset root is unavailable");
        };
        let Ok(metadata) = root.metadata() else {
            bail!("configured frontend asset root is unavailable");
        };
        if !metadata.is_dir() {
            bail!("configured frontend asset root is unavailable");
        }
        Ok(Self::Override(Arc::new(root)))
    }

    pub fn from_environment() -> Result<Self> {
        Self::from_override_value(env::var_os("HIERONYMUS_ASSETS_DIR"))
    }

    fn from_override_value(value: Option<OsString>) -> Result<Self> {
        match value.filter(|value| !value.is_empty()) {
            Some(root) => Self::override_root(PathBuf::from(root)),
            None => Ok(Self::embedded()),
        }
    }

    async fn load(&self, path: &str) -> Option<Vec<u8>> {
        let path = safe_relative_path(path)?;
        match self {
            Self::Embedded => Asset::get(path.to_str()?).map(|file| file.data.into_owned()),
            Self::Override(root) => load_override(root, &path).await,
        }
    }
}

pub async fn serve_assets(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    serve(&state.assets, &format!("assets/{path}"), false, request_id).await
}

pub(crate) async fn serve_index(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Response {
    serve(&state.assets, "index.html", true, request_id).await
}

pub(crate) async fn serve_client_route(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let path = uri.path();
    if is_client_route(path) {
        serve(&state.assets, "index.html", true, request_id).await
    } else {
        not_found(request_id)
    }
}

#[must_use]
pub(crate) fn is_client_route(path: &str) -> bool {
    let path = path.trim_start_matches('/');
    !path.is_empty()
        && !path.starts_with("api/")
        && !path.starts_with("mcp")
        && !path.starts_with("ws/")
        && !matches!(path, "health" | "status" | "shutdown")
        && path
            .rsplit('/')
            .next()
            .is_some_and(|segment| !segment.contains('.'))
}

async fn serve(source: &AssetSource, path: &str, index: bool, request_id: RequestId) -> Response {
    let Some(bytes) = source.load(path).await else {
        return not_found(request_id);
    };
    let content_type = content_type(path);
    let cache = if index {
        NO_CACHE
    } else if has_content_hash(path) {
        IMMUTABLE_CACHE
    } else {
        SHORT_CACHE
    };
    let mut response = Body::from(bytes).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn safe_relative_path(path: &str) -> Option<PathBuf> {
    if path.is_empty() || path.contains(['\\', '\0']) {
        return None;
    }
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_owned())
}

async fn load_override(root: &Path, path: &Path) -> Option<Vec<u8>> {
    let candidate = tokio::fs::canonicalize(root.join(path)).await.ok()?;
    if !candidate.starts_with(root) {
        return None;
    }
    let metadata = tokio::fs::metadata(&candidate).await.ok()?;
    if !metadata.is_file() {
        return None;
    }
    tokio::fs::read(candidate).await.ok()
}

fn content_type(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|value| value.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn has_content_hash(path: &str) -> bool {
    let Some(stem) = Path::new(path).file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    stem.rsplit_once('-').is_some_and(|(_, suffix)| {
        suffix.len() >= 8
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    })
}

fn not_found(request_id: RequestId) -> Response {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "not_found",
        "asset or client route not found",
        request_id,
    )
    .into_response()
}
