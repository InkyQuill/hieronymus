use std::{
    env,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

#[cfg(unix)]
use std::{io::Read, sync::Arc};

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
    #[cfg(unix)]
    Override(Arc<OverrideRoot>),
}

#[cfg(unix)]
#[derive(Debug)]
pub struct OverrideRoot {
    descriptor: std::os::fd::OwnedFd,
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
        #[cfg(unix)]
        {
            let descriptor = rustix::fs::open(
                &root,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|_| anyhow::anyhow!("configured frontend asset root is unavailable"))?;
            Ok(Self::Override(Arc::new(OverrideRoot { descriptor })))
        }
        #[cfg(not(unix))]
        {
            let _ = root;
            bail!("filesystem frontend asset overrides are unsupported on this platform")
        }
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
            #[cfg(unix)]
            Self::Override(root) => {
                let root = root.clone();
                tokio::task::spawn_blocking(move || {
                    load_override_unix_with_hook(&root, &path, || {})
                })
                .await
                .ok()
                .flatten()
            }
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
        && !is_reserved_namespace(path)
        && path
            .rsplit('/')
            .next()
            .is_some_and(|segment| !segment.contains('.'))
}

fn is_reserved_namespace(path: &str) -> bool {
    ["api", "mcp", "ws", "health", "status", "shutdown"]
        .into_iter()
        .any(|namespace| {
            path.strip_prefix(namespace)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
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

#[cfg(unix)]
fn load_override_unix_with_hook(
    root: &OverrideRoot,
    path: &Path,
    before_file_open: impl FnOnce(),
) -> Option<Vec<u8>> {
    let mut components = path.components();
    let Component::Normal(file_name) = components.next_back()? else {
        return None;
    };
    let mut current = rustix::io::dup(&root.descriptor).ok()?;
    for component in components {
        let Component::Normal(component) = component else {
            return None;
        };
        current = rustix::fs::openat(
            &current,
            component,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .ok()?;
    }
    before_file_open();
    let descriptor = rustix::fs::openat(
        &current,
        file_name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let metadata = rustix::fs::fstat(&descriptor).ok()?;
    if !rustix::fs::FileType::from_raw_mode(metadata.st_mode).is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::from(descriptor)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
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
    stem.match_indices('-').any(|(separator, _)| {
        let suffix = &stem[separator + 1..];
        suffix.len() == 8
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{fs, path::PathBuf, sync::mpsc, time::Duration};

    use tempfile::TempDir;

    use super::{AssetSource, load_override_unix_with_hook};

    #[tokio::test]
    async fn override_read_is_anchored_when_ancestor_and_file_are_swapped_concurrently() {
        let temp = TempDir::new().expect("temporary directory should be created");
        let root = temp.path().join("dist");
        let assets = root.join("assets");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&assets).expect("asset directory should be created");
        fs::create_dir_all(&outside).expect("outside directory should be created");
        fs::write(assets.join("app.js"), b"safe").expect("safe fixture should be written");
        let fifo = outside.join("app.js");
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .expect("outside FIFO should be created");
        let source = AssetSource::override_root(&root).expect("override root should open");
        let AssetSource::Override(root_handle) = source else {
            panic!("fixture should use an override root");
        };

        let parked = root.join("parked-assets");
        let (opened_tx, opened_rx) = mpsc::sync_channel(0);
        let (swapped_tx, swapped_rx) = mpsc::sync_channel(0);
        let swapper = std::thread::spawn(move || {
            opened_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("reader should reach the final component");
            fs::rename(&assets, &parked).expect("opened ancestor should be renamed");
            std::os::unix::fs::symlink(&outside, &assets)
                .expect("root entry should be replaced by an outside symlink");
            fs::remove_file(parked.join("app.js")).expect("safe file should be removed");
            std::os::unix::fs::symlink(&fifo, parked.join("app.js"))
                .expect("opened ancestor file should be replaced by an outside FIFO symlink");
            swapped_tx.send(()).expect("reader should be released");
        });

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            tokio::task::spawn_blocking(move || {
                load_override_unix_with_hook(&root_handle, &PathBuf::from("assets/app.js"), || {
                    opened_tx.send(()).expect("swapper should be ready");
                    swapped_rx.recv().expect("swap should finish");
                })
            }),
        )
        .await
        .expect("anchored read must not hang on the outside FIFO")
        .expect("blocking reader should not panic");
        swapper.join().expect("swapper should not panic");

        assert_eq!(result, None);
    }
}
