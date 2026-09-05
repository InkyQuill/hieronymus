//! Build-time gate for the `console-embed` release backend (distribution
//! spec §Build Ownership): the frontend bundle is produced by an explicit
//! release orchestration step *before* compiling the binary, and release
//! builds fail here, at compile time, when the bundle is missing or empty.
//! Developer targets stay frontend-free: a debug-profile build with the
//! feature but no bundle compiles without the embedded backend (the daemon
//! then reports `web_console_not_built`), and without the feature this
//! script does nothing at all — ordinary tests never invoke Bun.

use std::path::PathBuf;

fn main() {
    // The cfg this script emits is conditional, so declare it up front to
    // keep `unexpected_cfgs` quiet in every feature combination.
    println!("cargo::rustc-check-cfg=cfg(hiero_dist_embeddable)");
    println!("cargo::rerun-if-changed=build.rs");

    if std::env::var_os("CARGO_FEATURE_CONSOLE_EMBED").is_none() {
        return;
    }

    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .expect("cargo sets CARGO_MANIFEST_DIR for build scripts");
    let frontend = manifest_dir.join("../../frontend");
    let dist = frontend.join("dist");

    // Track the bundle so building (or wiping) `frontend/dist` re-runs this
    // script and re-embeds, even when no Rust file changed. The `frontend`
    // directory always exists and its mtime moves when `dist` is created or
    // removed; cargo ignores rerun paths that were absent on the previous
    // run, so it is the reliable transition signal. The bundle directory
    // itself is tracked when present so a rebuild re-embeds.
    let tracked_frontend = std::fs::canonicalize(&frontend).unwrap_or_else(|_| frontend.clone());
    println!("cargo::rerun-if-changed={}", tracked_frontend.display());
    if dist.is_dir() {
        let tracked_dist = std::fs::canonicalize(&dist).unwrap_or_else(|_| dist.clone());
        println!("cargo::rerun-if-changed={}", tracked_dist.display());
    }

    let embeddable = dist.join("index.html").is_file();

    if embeddable {
        println!("cargo::rustc-cfg=hiero_dist_embeddable");
        return;
    }

    if std::env::var("PROFILE").as_deref() == Ok("release") {
        panic!(
            "console-embed release build requires the production console bundle at \
             frontend/dist (missing or empty); build it first with \
             `scripts/release-build.sh --assets-only`"
        );
    }
}
