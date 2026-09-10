//! Build-time guard for the embedded frontend asset root.
//!
//! The qualification runner, not this script, builds the Svelte bundle into
//! `qualification/.artifacts/frontend-dist/current` before invoking Cargo.
//! This script only validates that folder and wires it into Cargo's change
//! tracking. The asset root is resolved strictly from `CARGO_MANIFEST_DIR`
//! with a fixed relative path; no environment variable may redirect it.

use std::fs;
use std::path::{Path, PathBuf};

/// Manifest-relative location of the replayed Svelte bundle. Must stay in
/// sync with the RustEmbed folder path used by the binary (Task 14).
const ASSET_ROOT_RELATIVE: &str = "../../.artifacts/frontend-dist/current";

fn main() {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .expect("cargo sets CARGO_MANIFEST_DIR when running build scripts"),
    );
    let unresolved_root = manifest_dir.join(ASSET_ROOT_RELATIVE);
    let asset_root = unresolved_root.canonicalize().unwrap_or_else(|error| {
        panic!(
            "embedded frontend bundle root {} does not resolve ({error}); \
             build the Svelte bundle into it before running cargo",
            unresolved_root.display()
        )
    });

    if !asset_root.is_dir() {
        panic!(
            "embedded frontend bundle root {} is not a directory",
            asset_root.display()
        );
    }

    require_index_html(&asset_root);
    require_assets_files(&asset_root);
    reject_escaping_symlinks(&asset_root);

    // Exact paths only: cargo watches this directory recursively, and this is
    // the single canonical location of the embedded bundle.
    println!("cargo:rerun-if-changed={}", asset_root.display());
}

fn require_index_html(asset_root: &Path) {
    let index = asset_root.join("index.html");
    let index = index
        .canonicalize()
        .unwrap_or_else(|error| panic!("bundle index.html is missing ({error})"));
    if !index.is_file() {
        panic!("bundle entry {} is not a regular file", index.display());
    }
}

fn require_assets_files(asset_root: &Path) {
    let assets = asset_root.join("assets");
    if !assets.is_dir() {
        panic!("bundle assets directory {} is missing", assets.display());
    }
    if walk_files(&assets).next().is_none() {
        panic!(
            "bundle assets directory {} contains no files",
            assets.display()
        );
    }
}

/// Panics if any symlink inside `asset_root` resolves outside of it.
fn reject_escaping_symlinks(asset_root: &Path) {
    for entry in walk_entries(asset_root) {
        let file_type = entry.file_type().unwrap_or_else(|error| {
            panic!(
                "cannot stat bundle entry {}: {error}",
                entry.path().display()
            )
        });
        if !file_type.is_symlink() {
            continue;
        }
        let target = entry.path().canonicalize().unwrap_or_else(|error| {
            panic!(
                "bundle symlink {} does not resolve ({error})",
                entry.path().display()
            )
        });
        if !target.starts_with(asset_root) {
            panic!(
                "bundle symlink {} escapes the asset root to {}",
                entry.path().display(),
                target.display()
            );
        }
    }
}

fn walk_files(root: &Path) -> impl Iterator<Item = PathBuf> {
    walk_entries(root)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .map(|entry| entry.path())
}

fn walk_entries(root: &Path) -> WalkEntries {
    let read_dir = fs::read_dir(root)
        .unwrap_or_else(|error| panic!("cannot read bundle directory {root:?}: {error}"));
    WalkEntries {
        stack: vec![read_dir],
    }
}

/// Depth-first enumeration of every entry below `root`. Symlinked directories
/// are yielded but never descended into; symlink targets are validated by the
/// caller.
struct WalkEntries {
    stack: Vec<fs::ReadDir>,
}

impl Iterator for WalkEntries {
    type Item = fs::DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(top) = self.stack.last_mut() {
            match top.next() {
                Some(Ok(entry)) => {
                    // `DirEntry::file_type` does not follow symlinks, so only
                    // real directories are pushed onto the stack.
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        let read_dir = fs::read_dir(entry.path()).unwrap_or_else(|error| {
                            panic!(
                                "cannot read bundle directory {}: {error}",
                                entry.path().display()
                            )
                        });
                        self.stack.push(read_dir);
                    }
                    return Some(entry);
                }
                Some(Err(error)) => {
                    panic!("cannot enumerate bundle directory: {error}");
                }
                None => {
                    self.stack.pop();
                }
            }
        }
        None
    }
}
