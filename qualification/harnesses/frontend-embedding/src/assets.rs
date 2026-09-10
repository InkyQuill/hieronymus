//! Embedded frontend asset resolution for the qualification harness.
//!
//! The Svelte bundle from `qualification/.artifacts/frontend-dist/current`
//! is compiled into the binary by `rust-embed`; resolution below reads only
//! from that embedded image. Ownership boundary: this module proves embedded
//! path resolution, MIME typing, body digests, SPA fallback, and filesystem
//! independence only. It does not implement or claim Host validation,
//! bearer/session auth, CSRF, or full HTTP routing.

use serde::Serialize;
use sha2::Digest;

#[derive(rust_embed::RustEmbed)]
#[folder = "../../.artifacts/frontend-dist/current/"]
struct FrontendAssets;

#[derive(Debug, Serialize)]
pub struct AssetResponse {
    pub status: u16,
    pub content_type: String,
    pub body_sha256: String,
    pub len: usize,
    pub fallback: bool,
}

/// One entry of the embedded manifest: the asset key, its byte length, and
/// its body digest. Keys are sorted so CLI output is byte-stable.
#[derive(Debug, Serialize)]
pub struct ManifestEntry {
    pub key: String,
    pub len: usize,
    pub sha256: String,
}

/// Metadata-only listing of every embedded file, sorted by key.
pub fn manifest() -> Vec<ManifestEntry> {
    let mut entries: Vec<ManifestEntry> = FrontendAssets::iter()
        .map(|key| {
            let file = FrontendAssets::get(key.as_ref()).expect("iter key is always present");
            ManifestEntry {
                sha256: format!("{:x}", sha2::Sha256::digest(file.data.as_ref())),
                key: key.into_owned(),
                len: file.data.len(),
            }
        })
        .collect();
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    entries
}

fn response(status: u16, key: &str, bytes: &[u8], fallback: bool) -> AssetResponse {
    AssetResponse {
        status,
        content_type: if key.ends_with(".html") {
            "text/html; charset=utf-8".to_owned()
        } else {
            mime_guess::from_path(key)
                .first_or_octet_stream()
                .to_string()
        },
        body_sha256: format!("{:x}", sha2::Sha256::digest(bytes)),
        len: bytes.len(),
        fallback,
    }
}

fn empty_response(status: u16) -> AssetResponse {
    response(status, "response.json", b"", false)
}

pub fn resolve_asset(request_path: &str) -> AssetResponse {
    let lowercase = request_path.to_ascii_lowercase();
    if lowercase.contains("%2f") || lowercase.contains("%5c") || lowercase.contains("%00") {
        return empty_response(400);
    }
    let decoded = match percent_encoding::percent_decode_str(request_path).decode_utf8() {
        Ok(path) => path,
        Err(_) => return empty_response(400),
    };
    if decoded.contains('\\')
        || decoded.contains('\0')
        || decoded.split('/').any(|part| part == "..")
    {
        return empty_response(400);
    }
    let key = decoded.trim_start_matches('/');
    let key = if key.is_empty() { "index.html" } else { key };
    if let Some(asset) = FrontendAssets::get(key) {
        return response(200, key, asset.data.as_ref(), false);
    }
    if key.starts_with("assets/") {
        return empty_response(404);
    }
    let index = FrontendAssets::get("index.html").expect("build.rs verified index.html");
    response(200, "index.html", index.data.as_ref(), true)
}
