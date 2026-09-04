//! The embedded console asset abstraction (spec §HTTP And Frontend
//! Contracts): static SPA assets are served from a lookup over a fixed asset
//! set, never a live filesystem `ServeDir`. Per the controller ruling, the
//! release-time `rust-embed` compile-time backend lands with the
//! console-build slice (when `frontend/dist` is produced by the build
//! pipeline); until then two backends exist:
//!
//! - [`Assets::Memory`] — an injected in-memory set (tests and the fixture
//!   world);
//! - [`Assets::Dist`] — an explicit `frontend/dist` filesystem override
//!   (development).

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
