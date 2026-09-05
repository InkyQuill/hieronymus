//! Release embedded-console tests (`console-embed` feature): the real
//! production bundle (`frontend/dist`) served through the same static-route
//! contracts the frozen suites pin — index fallback, MIME types, hashed
//! assets — plus the distribution spec's source-map secrecy rule. The bundle
//! only exists after the explicit frontend build step, so these tests skip
//! cleanly without it: ordinary frontend-free developer targets never invoke
//! Bun (spec §Build Ownership).

#![cfg(feature = "console-embed")]

mod common;

use common::send_request;
use hiero::daemon::{Assets, Daemon, DaemonOptions};
use std::path::{Path, PathBuf};

/// The production console bundle this crate embeds (repo-root
/// `frontend/dist`, produced by the release orchestration step).
fn dist_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/dist")
}

/// Skip guard: without a built bundle (no Bun on the machine) the embedded
/// backend is the empty set and the release-asset checks cannot run.
fn bundle_is_built() -> bool {
    dist_dir().join("index.html").is_file()
}

fn skip_without_bundle() -> bool {
    if bundle_is_built() {
        return false;
    }
    eprintln!("skipping: frontend/dist is not built (frontend-free developer target)");
    true
}

/// A daemon serving the release-embedded console set.
fn start_daemon_with_embedded_assets() -> (tempfile::TempDir, Daemon) {
    let root = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: Assets::release(),
    })
    .unwrap();
    (root, daemon)
}

/// The `assets/...` bundle references inside an index body (vite emits
/// hashed names like `assets/index-<hash>.js`).
fn asset_references(index_body: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(index_body);
    let mut references = Vec::new();
    let mut rest = text.as_ref();
    while let Some(position) = rest.find("assets/") {
        let candidate = &rest[position..];
        let end = candidate
            .find(|character: char| {
                matches!(character, '"' | '\'' | '(' | ')' | '>') || character.is_whitespace()
            })
            .unwrap_or(candidate.len());
        references.push(candidate[..end].to_string());
        rest = &candidate[end..];
    }
    references.sort();
    references.dedup();
    references
}

/// The exact response types the asset table serves for bundle files.
fn expected_mime(asset_path: &str) -> &'static str {
    if asset_path.ends_with(".js") {
        "application/javascript"
    } else if asset_path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

/// Every file below `dist`, as asset-set paths (`assets/index-<hash>.js`).
/// The release target is POSIX, so no separator translation is needed.
fn collect_dist_asset_paths(dir: &Path, out: &mut Vec<String>) {
    let entries = std::fs::read_dir(dir).unwrap();
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_dist_asset_paths(&path, out);
        } else {
            out.push(
                path.strip_prefix(dist_dir())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
}

#[test]
fn embedded_index_serves_the_production_console() {
    if skip_without_bundle() {
        return;
    }
    let assets = Assets::release();
    let index = assets
        .lookup("index.html")
        .expect("the bundle embeds index.html");
    assert_eq!(index.content_type, "text/html; charset=utf-8");
    assert!(!index.body.is_empty());

    let (_root, daemon) = start_daemon_with_embedded_assets();
    let port = daemon.local_addr().port();
    let response = send_request(port, "GET", "/", &[], b"");
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/html; charset=utf-8");
    assert_eq!(response.raw_body, index.body);

    // Client-side routes fall back to the same embedded index (the frozen
    // static-route contract, now against the real bundle).
    for path in ["/admin", "/config/general"] {
        let response = send_request(port, "GET", path, &[], b"");
        assert_eq!(response.status, 200, "{path} status");
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        assert_eq!(response.raw_body, index.body, "{path} body");
    }
    // The console references its hashed bundle assets.
    assert!(
        asset_references(&index.body)
            .iter()
            .any(|reference| reference.ends_with(".js")),
        "the index references its javascript bundle"
    );
}

#[test]
fn embedded_hashed_assets_resolve_with_mime_types() {
    if skip_without_bundle() {
        return;
    }
    let index = Assets::release()
        .lookup("index.html")
        .expect("the bundle embeds index.html");
    let references = asset_references(&index.body);
    assert!(
        references.iter().any(|r| r.ends_with(".js"))
            && references.iter().any(|r| r.ends_with(".css")),
        "vite emits hashed js/css references, got {references:?}"
    );

    let (_root, daemon) = start_daemon_with_embedded_assets();
    let port = daemon.local_addr().port();
    for reference in &references {
        let response = send_request(port, "GET", &format!("/{reference}"), &[], b"");
        assert_eq!(response.status, 200, "{reference} must resolve");
        assert_eq!(
            response.content_type,
            expected_mime(reference),
            "{reference}"
        );
    }
}

#[test]
fn embedded_console_ships_no_source_map_secrets() {
    if skip_without_bundle() {
        return;
    }
    let assets = Assets::release();
    let mut files = Vec::new();
    collect_dist_asset_paths(&dist_dir(), &mut files);
    assert!(
        files.iter().any(|path| path == "index.html"),
        "the bundle manifest includes index.html"
    );

    // `.map` content is never embedded, even if the build emits it.
    let maps: Vec<String> = files
        .iter()
        .filter(|path| path.ends_with(".map"))
        .cloned()
        .collect();
    for map in &maps {
        assert!(assets.lookup(map).is_none(), "{map} must not be embedded");
    }

    // Served javascript/css never references a source map.
    for path in &files {
        if path.ends_with(".map") {
            continue;
        }
        let asset = assets
            .lookup(path)
            .unwrap_or_else(|| panic!("{path} must be embedded"));
        if path.ends_with(".js") || path.ends_with(".css") {
            let body = String::from_utf8_lossy(&asset.body);
            assert!(
                !body.contains("sourceMappingURL"),
                "{path} leaks a source-map reference"
            );
        }
    }

    // Over HTTP a map request is a plain 404, never content.
    let (_root, daemon) = start_daemon_with_embedded_assets();
    let port = daemon.local_addr().port();
    let probe = maps
        .first()
        .map(String::as_str)
        .unwrap_or("assets/none.map");
    let response = send_request(port, "GET", &format!("/assets/{probe}"), &[], b"");
    assert_eq!(response.status, 404);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
}

#[test]
fn default_assets_stay_empty_under_the_embed_feature() {
    // The feature must not change the developer default: the frozen
    // `web_console_not_built` suite keeps its empty set, and only the
    // shipping path (`Assets::release`) gains the bundle.
    assert!(Assets::default().lookup("index.html").is_none());
    if bundle_is_built() {
        assert!(Assets::release().lookup("index.html").is_some());
    } else {
        assert!(Assets::release().lookup("index.html").is_none());
    }
}
