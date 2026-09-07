//! The embedded console asset abstraction (spec §HTTP And Frontend
//! Contracts): static SPA assets are served from a lookup over a fixed asset
//! set, never a live filesystem `ServeDir`. Three backends exist:
//!
//! - [`Assets::Memory`] — an injected in-memory set (tests and the fixture
//!   world);
//! - [`Assets::Dist`] — an explicit `frontend/dist` filesystem override
//!   (development);
//! - the release backend (module `embedded`, feature `console-embed`) —
//!   `frontend/dist` embedded at compile time via `rust-embed`, served in
//!   the same fixed-set shape by [`Assets::release`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One static asset: its bytes and the response `Content-Type`.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

/// A fixed console asset set.
#[derive(Debug, Clone)]
pub enum Assets {
    /// The injected in-memory set; the release rust-embed backend will take
    /// this shape at compile time in the console-build slice.
    Memory(BTreeMap<String, Asset>),
    /// Development override: serve from a `frontend/dist` directory.
    Dist(PathBuf),
}

impl Default for Assets {
    fn default() -> Self {
        // The empty embedded set: without a console build every web route
        // reports `web_console_not_built` (the Python no-build outcome).
        Assets::Memory(BTreeMap::new())
    }
}

impl Assets {
    /// The asset set a shipping binary serves: the compile-time embedded
    /// console bundle when built with the `console-embed` feature, otherwise
    /// the empty set (`web_console_not_built`). A frontend-free developer
    /// build with the feature enabled (debug profile, no bundle) also falls
    /// back to the empty set; release-profile builds fail at compile time in
    /// `build.rs` instead.
    pub fn release() -> Assets {
        #[cfg(feature = "console-embed")]
        if let Some(assets) = embedded::embedded_assets() {
            return assets;
        }
        Assets::default()
    }

    /// Build an in-memory set from `(path, body)` entries; content types
    /// derive from the path extensions.
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Assets {
        Assets::Memory(
            entries
                .into_iter()
                .map(|(path, body)| {
                    (
                        path.to_string(),
                        Asset {
                            content_type: content_type_for(path),
                            body: body.to_vec(),
                        },
                    )
                })
                .collect(),
        )
    }

    /// Look up one asset by its path inside the set (`index.html`,
    /// `assets/app.js`, ...). Paths that escape the set (`..`, absolute) never
    /// resolve.
    pub fn lookup(&self, path: &str) -> Option<Asset> {
        match self {
            Assets::Memory(entries) => entries.get(path).cloned(),
            Assets::Dist(root) => {
                let relative = Path::new(path);
                if path.is_empty()
                    || relative.is_absolute()
                    || relative
                        .components()
                        .any(|component| component == std::path::Component::ParentDir)
                {
                    return None;
                }
                let body = std::fs::read(root.join(relative)).ok()?;
                Some(Asset {
                    content_type: content_type_for(path),
                    body,
                })
            }
        }
    }
}

/// The response `Content-Type` for an asset path (the frozen fixture types
/// are exact: `text/html; charset=utf-8`, `application/javascript`).
pub(crate) fn content_type_for(path: &str) -> &'static str {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    match extension {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" => "application/javascript",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// The release backend: the production console bundle embedded at compile
/// time via `rust-embed` (security spec §HTTP And Frontend Contracts:
/// lookup/iteration over the embedded set, never a filesystem `ServeDir`).
/// It exists only when the `console-embed` feature is on AND `build.rs`
/// verified the bundle (its `hiero_dist_embeddable` cfg): a release-profile
/// build without `frontend/dist` already failed there, and a frontend-free
/// developer build simply has no embedded backend.
#[cfg(feature = "console-embed")]
mod embedded {
    use super::Assets;

    #[cfg(hiero_dist_embeddable)]
    #[derive(rust_embed::RustEmbed)]
    #[folder = "$CARGO_MANIFEST_DIR/../../frontend/dist"]
    struct ConsoleDist;

    /// Materialize the embedded bundle into the fixed in-memory set the
    /// daemon serves. Source maps never ship: `.map` entries are dropped
    /// from the set even if a build configuration emits them.
    #[cfg(hiero_dist_embeddable)]
    pub(super) fn embedded_assets() -> Option<Assets> {
        use super::Asset;
        use std::collections::BTreeMap;

        let entries: BTreeMap<String, Asset> = ConsoleDist::iter()
            .filter_map(|path| {
                if path.ends_with(".map") {
                    return None;
                }
                let file = ConsoleDist::get(path.as_ref())?;
                let content_type = super::content_type_for(&path);
                Some((
                    path.into_owned(),
                    Asset {
                        content_type,
                        body: file.data.into_owned(),
                    },
                ))
            })
            .collect();
        Some(Assets::Memory(entries))
    }

    #[cfg(not(hiero_dist_embeddable))]
    pub(super) fn embedded_assets() -> Option<Assets> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types_match_the_frozen_fixture_types() {
        assert_eq!(content_type_for("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type_for("assets/app.min.js"),
            "application/javascript"
        );
        assert_eq!(content_type_for("assets/logo.svg"), "image/svg+xml");
        assert_eq!(
            content_type_for("assets/font.bin"),
            "application/octet-stream"
        );
    }

    #[test]
    fn memory_lookup_resolves_injected_entries_only() {
        let assets = Assets::from_entries([
            ("index.html", b"<!doctype html><title>x</title>" as &[u8]),
            ("assets/fixture", b"console.log('x');"),
        ]);
        let index = assets.lookup("index.html").unwrap();
        assert_eq!(index.content_type, "text/html; charset=utf-8");
        assert_eq!(index.body, b"<!doctype html><title>x</title>");
        assert!(assets.lookup("assets/missing.js").is_none());
        assert!(assets.lookup("index.html/../assets/fixture").is_none());
    }

    #[test]
    fn dist_lookup_serves_the_override_directory_and_never_escapes_it() {
        let root = tempfile::tempdir().unwrap();
        let dist = root.path().join("dist");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::write(dist.join("index.html"), b"<html></html>").unwrap();
        std::fs::write(root.path().join("secret.txt"), b"outside").unwrap();

        let assets = Assets::Dist(dist);
        let index = assets.lookup("index.html").unwrap();
        assert_eq!(index.body, b"<html></html>");
        assert!(assets.lookup("secret.txt").is_none());
        assert!(assets.lookup("../secret.txt").is_none());
        assert!(assets.lookup("/secret.txt").is_none());
        assert!(assets.lookup("").is_none());
    }
}
