//! Embedded asset resolution behavior for the frontend qualification harness.
//!
//! Ownership boundary: these tests prove embedded path resolution, MIME
//! typing, body digests, SPA fallback, and filesystem independence only.
//! They make no claims about Host validation, bearer/session auth, CSRF, or
//! full HTTP routing.

#![allow(clippy::expect_used)]

#[path = "../src/assets.rs"]
mod assets;

use assets::{AssetResponse, manifest, resolve_asset};

/// SHA-256 of the empty byte string, the body digest of every 400/404
/// metadata-only response.
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[test]
fn embedded_index_and_spa_paths_resolve() {
    assert_eq!(resolve_asset("/").status, 200);
    assert!(resolve_asset("/admin/fixture").fallback);
    assert!(resolve_asset("/config/fixture").fallback);
}

#[test]
fn missing_real_asset_is_not_an_index_fallback() {
    let response = resolve_asset("/assets/qualification-missing.js");
    assert_eq!(response.status, 404);
    assert!(!response.fallback);
}

#[test]
fn metadata_only_responses_carry_empty_bodies() {
    for response in [
        resolve_asset("/foo%2fbar"),
        resolve_asset("/assets/qualification-missing.js"),
    ] {
        assert_eq!(response.body_sha256, EMPTY_SHA256);
        assert_eq!(response.len, 0);
    }
}

#[test]
fn embedded_hits_normalize_the_leading_slash() {
    let with_slash = resolve_asset("/index.html");
    let without_slash = resolve_asset("index.html");
    assert_eq!(with_slash.status, 200);
    assert!(!with_slash.fallback);
    assert_eq!(with_slash.content_type, "text/html; charset=utf-8");
    assert_eq!(with_slash.body_sha256, without_slash.body_sha256);
    assert_eq!(with_slash.len, without_slash.len);
}

#[test]
fn encoded_separators_and_nul_are_rejected() {
    for path in [
        "/foo%2fbar",
        "/foo%2Fbar",
        "/foo%5cbar",
        "/foo%5Cbar",
        "/foo%00bar",
        "/%00",
        "/assets%2f..%2f..%2fsecret",
    ] {
        let response = resolve_asset(path);
        assert_eq!(response.status, 400, "path {path}");
        assert!(!response.fallback, "path {path}");
        assert_eq!(response.body_sha256, EMPTY_SHA256, "path {path}");
    }
}

#[test]
fn backslashes_nul_and_dotdot_segments_are_rejected() {
    for path in [
        "/foo\\bar",
        "/foo\u{0}bar",
        "/assets/%2e%2e/secret",
        "/a/../b",
        "/a/b/../../../etc/passwd",
        "/..",
    ] {
        let response = resolve_asset(path);
        assert_eq!(response.status, 400, "path {path}");
        assert!(!response.fallback, "path {path}");
        assert_eq!(response.body_sha256, EMPTY_SHA256, "path {path}");
    }
}

#[test]
fn embedded_assets_are_served_with_extension_mime_types() {
    let asset_keys: Vec<String> = manifest()
        .into_iter()
        .map(|entry| entry.key)
        .filter(|key| key.starts_with("assets/"))
        .collect();
    assert!(!asset_keys.is_empty(), "bundle must contain assets/ files");
    for key in asset_keys {
        let response = resolve_asset(&format!("/{key}"));
        assert_eq!(response.status, 200, "key {key}");
        assert!(!response.fallback, "key {key}");
        let expected_mime = if key.ends_with(".js") {
            "text/javascript"
        } else if key.ends_with(".css") {
            "text/css"
        } else if key.ends_with(".woff2") {
            "font/woff2"
        } else {
            panic!("unhandled test extension for key {key}");
        };
        assert_eq!(response.content_type, expected_mime, "key {key}");
    }
}

#[test]
fn manifest_lists_every_embedded_file_with_matching_digests() {
    let manifest = manifest();
    let keys: Vec<&str> = manifest.iter().map(|entry| entry.key.as_str()).collect();
    assert!(keys.contains(&"index.html"), "keys {keys:?}");
    assert!(keys.iter().any(|key| key.starts_with("assets/")));
    assert!(
        keys.windows(2).all(|pair| pair[0] < pair[1]),
        "manifest must be sorted by key"
    );
    for entry in &manifest {
        let resolved = resolve_asset(&format!("/{}", entry.key));
        assert_eq!(resolved.status, 200, "key {}", entry.key);
        assert_eq!(resolved.body_sha256, entry.sha256, "key {}", entry.key);
        assert_eq!(resolved.len, entry.len, "key {}", entry.key);
    }
}

#[test]
fn cli_get_prints_metadata_only_json() {
    let exe = env!("CARGO_BIN_EXE_frontend-embedding");
    let output = std::process::Command::new(exe)
        .args(["get", "--path", "/"])
        .output()
        .expect("spawn frontend-embedding");
    assert!(output.status.success());
    assert!(output.stderr.is_empty(), "stderr must stay empty");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    let mut fields: Vec<&str> = value
        .as_object()
        .expect("JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        vec!["body_sha256", "content_type", "fallback", "len", "status"]
    );
    assert_eq!(value["status"], 200);
    assert_eq!(value["fallback"], false);
    assert_eq!(value["content_type"], "text/html; charset=utf-8");
}

#[test]
fn cli_manifest_prints_embedded_file_metadata() {
    let exe = env!("CARGO_BIN_EXE_frontend-embedding");
    let output = std::process::Command::new(exe)
        .arg("manifest")
        .output()
        .expect("spawn frontend-embedding");
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    let array = value.as_array().expect("JSON array");
    assert!(
        array
            .iter()
            .any(|entry| entry["key"] == serde_json::json!("index.html"))
    );
    for entry in array {
        let mut fields: Vec<&str> = entry
            .as_object()
            .expect("JSON object")
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(fields, vec!["key", "len", "sha256"]);
    }
}

#[test]
fn cli_rejects_missing_path_and_unknown_subcommands() {
    let exe = env!("CARGO_BIN_EXE_frontend-embedding");
    for args in [
        vec!["get"],
        vec!["get", "--path"],
        vec!["get", "--bogus", "/"],
        vec!["manifest", "extra"],
        vec!["serve"],
        vec![],
    ] {
        let output = std::process::Command::new(exe)
            .args(&args)
            .output()
            .unwrap_or_else(|error| panic!("spawn frontend-embedding: {error}"));
        assert!(!output.status.success(), "args {args:?}");
        assert!(
            output.stdout.is_empty(),
            "args {:?} printed to stdout",
            args
        );
        assert!(
            !output.stderr.is_empty(),
            "args {:?} printed no usage error",
            args
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_lower = stderr.to_ascii_lowercase();
        assert!(
            !stderr_lower.contains("<!doctype") && !stderr_lower.contains("<script"),
            "stderr must not echo bodies: {stderr}"
        );
    }
}

#[test]
fn resolver_response_type_is_debug_and_serializable() {
    let response: AssetResponse = resolve_asset("/admin/fixture");
    let debug = format!("{response:?}");
    assert!(debug.contains("fallback: true"));
    let json = serde_json::to_string(&response).expect("serialize AssetResponse");
    assert!(json.contains("\"fallback\":true"));
}
