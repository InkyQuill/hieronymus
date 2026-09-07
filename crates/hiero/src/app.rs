//! Application-directory layout shared by the update and uninstall flows
//! (distribution spec §Installer): a versioned application directory per
//! release, plus stable command links that are switched by renaming, never by
//! in-place overwrites.
//!
//! ```text
//! <app>/versions/<version>/hiero            the release binary
//! <app>/versions/<version>/hieronymus       relative link -> hiero (argv[0])
//! <app>/versions/<version>/hieronymus-mcp   relative link -> hiero
//! <app>/versions/<version>/hieronymus-agent-hook
//! <app>/bin/<name>                          stable link -> ../versions/<version>/<name>
//! ```
//!
//! The default root matches the historical managed install
//! (`~/.local/share/hieronymus/app`). The bootstrap installer
//! (`scripts/install.sh`) owns the same layout, so the shell and Rust flows
//! interoperate.

use std::path::{Path, PathBuf};

/// The only supported release target (distribution spec §Support Matrix).
pub const TARGET_TRIPLE: &str = "x86_64-unknown-linux-gnu";

/// The command link names this project owns. `hiero` is canonical; the others
/// route through the binary's argv[0] handling. Removing any of them requires
/// a later ADR.
pub const LINK_NAMES: [&str; 4] = [
    "hiero",
    "hieronymus",
    "hieronymus-agent-hook",
    "hieronymus-mcp",
];

/// Default application root, matching the historical managed install.
pub fn default_app_dir() -> PathBuf {
    home::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("share")
        .join("hieronymus")
        .join("app")
}

/// The versioned-application-directory layout rooted at one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppLayout {
    root: PathBuf,
}

impl AppLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn version_dir(&self, version: &str) -> PathBuf {
        self.versions_dir().join(version)
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }

    pub fn stable_link(&self, name: &str) -> PathBuf {
        self.bin_dir().join(name)
    }

    /// The version the stable `hiero` link points at right now, derived from
    /// the link target (`../versions/<version>/hiero`). `None` when the link
    /// is missing or does not point into this layout.
    pub fn current_version(&self) -> Option<String> {
        let target = std::fs::read_link(self.stable_link("hiero")).ok()?;
        // Relative target such as `../versions/0.7.0/hiero`.
        let text = target.to_str()?;
        let rest = text.strip_prefix("../versions/")?;
        let version = rest.strip_suffix("/hiero")?;
        if version.is_empty() || version.contains('/') {
            return None;
        }
        Some(version.to_string())
    }

    /// (Re)point every stable command link at `version`. Each link is created
    /// under a temporary name and renamed into place, so a reader never sees a
    /// half-switched set. Idempotent: rerunning produces the same links.
    pub fn switch_stable_links(&self, version: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(self.bin_dir())?;
        for name in LINK_NAMES {
            let link = self.stable_link(name);
            let temporary = self.bin_dir().join(format!(".{name}.switch"));
            let _ = std::fs::remove_file(&temporary);
            std::os::unix::fs::symlink(format!("../versions/{version}/{name}"), &temporary)?;
            std::fs::rename(&temporary, &link)?;
        }
        Ok(())
    }

    /// Resolve the layout root from the currently running binary: a managed
    /// binary lives at `<app>/versions/<version>/hiero`, so the layout root is
    /// two levels up. Fails with an actionable message for developer builds.
    pub fn detect_from_exe() -> Result<PathBuf, String> {
        let exe = std::env::current_exe()
            .map_err(|error| format!("could not locate the running binary: {error}"))?
            .canonicalize()
            .map_err(|error| format!("could not resolve the running binary: {error}"))?;
        let version_dir = exe.parent().ok_or_else(detect_failure_message)?;
        let versions_dir = version_dir.parent().ok_or_else(detect_failure_message)?;
        if versions_dir.file_name() != Some(std::ffi::OsStr::new("versions")) {
            return Err(detect_failure_message());
        }
        versions_dir
            .parent()
            .map(|root| root.to_path_buf())
            .ok_or_else(detect_failure_message)
    }
}

fn detect_failure_message() -> String {
    "could not derive the application directory from the running binary; \
     pass --app-dir <path> (managed installs live at <app>/versions/<version>/hiero)"
        .to_string()
}

/// Three-way version comparison for dotted release versions: numeric
/// components compare numerically, missing components sort lower, and
/// non-numeric components fall back to a lexical compare.
pub fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut left_parts = left.split('.');
    let mut right_parts = right.split('.');
    loop {
        match (left_parts.next(), right_parts.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_part), Some(right_part)) => {
                let left_number = left_part.parse::<u64>().ok();
                let right_number = right_part.parse::<u64>().ok();
                let ordering = match (left_number, right_number) {
                    (Some(left_number), Some(right_number)) => left_number.cmp(&right_number),
                    _ => left_part.cmp(right_part),
                };
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_app_dir_matches_the_historical_managed_install() {
        let root = default_app_dir();
        assert!(root.ends_with(".local/share/hieronymus/app"), "{root:?}");
    }

    #[test]
    fn stable_links_point_at_the_versioned_binary_and_reruns_are_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let layout = AppLayout::new(temp.path());
        // The versioned payload: the binary plus its relative argv[0] links.
        std::fs::create_dir_all(layout.version_dir("1.2.3")).unwrap();
        std::fs::write(layout.version_dir("1.2.3").join("hiero"), b"binary").unwrap();

        layout.switch_stable_links("1.2.3").unwrap();
        layout.switch_stable_links("1.2.3").unwrap();

        for name in LINK_NAMES {
            let link = layout.stable_link(name);
            let target = std::fs::read_link(&link).unwrap();
            assert_eq!(
                target,
                std::path::Path::new("../versions/1.2.3").join(name),
                "{link:?}"
            );
            let leftovers: Vec<_> = std::fs::read_dir(layout.bin_dir())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(leftovers.len(), LINK_NAMES.len(), "no switch leftovers");
        }
        assert_eq!(layout.current_version().as_deref(), Some("1.2.3"));
    }

    #[test]
    fn switching_versions_moves_every_link() {
        let temp = tempfile::tempdir().unwrap();
        let layout = AppLayout::new(temp.path());
        for version in ["1.0.0", "2.0.0"] {
            std::fs::create_dir_all(layout.version_dir(version)).unwrap();
        }
        layout.switch_stable_links("1.0.0").unwrap();
        layout.switch_stable_links("2.0.0").unwrap();
        assert_eq!(layout.current_version().as_deref(), Some("2.0.0"));
        assert_eq!(
            std::fs::read_link(layout.stable_link("hieronymus-mcp")).unwrap(),
            std::path::Path::new("../versions/2.0.0/hieronymus-mcp")
        );
    }

    #[test]
    fn current_version_is_none_without_a_stable_link() {
        let temp = tempfile::tempdir().unwrap();
        let layout = AppLayout::new(temp.path());
        assert_eq!(layout.current_version(), None);
    }

    #[test]
    fn version_compare_is_numeric_per_component() {
        use std::cmp::Ordering;
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("2.0.0", "10.0.0"), Ordering::Less);
    }
}

/// Verify the qualified payload and execute real native document/query
/// inference. Used by the release builder and bootstrap before activation.
pub fn verify_semantic_assets(root: &Path) -> Result<serde_json::Value, String> {
    use hieronymus::semantic_arming::{RUNTIME_SHA256, RUNTIME_VERSION, verify_runtime_library};
    use hieronymus::semantic_embeddings::{EmbeddingProvider, OnnxEmbeddingProvider};
    use hieronymus::semantic_model::{MODEL_NAME, MODEL_REVISION, MODEL_SHA256, TOKENIZER_SHA256};
    let runtime = root.join("lib/libonnxruntime.so");
    verify_runtime_library(&runtime)?;
    let mut hashes = serde_json::Map::new();
    for (file, expected) in [
        ("lib/libonnxruntime.so", RUNTIME_SHA256),
        ("models/minilm/model.onnx", MODEL_SHA256),
        ("models/minilm/tokenizer.json", TOKENIZER_SHA256),
        (
            "models/minilm/LICENSE",
            "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
        ),
        (
            "models/minilm/README.md",
            "1e98ea05b0de579fcaad3d625b62ea55647142ed674d5f5ebf1440e4bbbb6f23",
        ),
    ] {
        let path = root.join(file);
        if !std::fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .is_file()
        {
            return Err(format!("asset must be a regular file: {file}"));
        }
        let digest = crate::update::sha256_file(&path).map_err(|e| e.to_string())?;
        if digest != expected {
            return Err(format!("asset checksum mismatch: {file}"));
        }
        hashes.insert(file.into(), serde_json::json!(digest));
    }
    for file in ["LICENSE", "ThirdPartyNotices.txt", "VERSION_NUMBER"] {
        let name = format!("licenses/runtime/{file}");
        let path = root.join(&name);
        if !std::fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .is_file()
            || std::fs::metadata(&path).map_err(|e| e.to_string())?.len() == 0
        {
            return Err(format!("required runtime notice is missing: {name}"));
        }
        hashes.insert(
            name,
            serde_json::json!(crate::update::sha256_file(&path).map_err(|e| e.to_string())?),
        );
    }
    if std::fs::read_to_string(root.join("licenses/runtime/VERSION_NUMBER"))
        .map_err(|e| e.to_string())?
        .trim()
        != RUNTIME_VERSION
    {
        return Err("runtime VERSION_NUMBER does not match qualified runtime".into());
    }
    let bytes =
        std::fs::read(root.join("models/minilm/tokenizer.json")).map_err(|e| e.to_string())?;
    let tokenizer = hieronymus::semantic_tokenizer::ModelTokenizer::from_bytes(&bytes)
        .map_err(|e| e.to_string())?;
    let mut provider =
        OnnxEmbeddingProvider::load(&runtime, &root.join("models/minilm/model.onnx"))
            .map_err(|e| e.to_string())?;
    let tokens = tokenizer
        .encode("Картограф描いた地図")
        .map_err(|e| e.to_string())?;
    for vector in [
        provider.embed_document(&tokens),
        provider.embed_query(&tokens),
    ] {
        let vector = vector.map_err(|e| e.to_string())?;
        if vector.len() != 384 || vector.iter().any(|v| !v.is_finite()) {
            return Err("native inference returned an invalid vector".into());
        }
    }
    Ok(
        serde_json::json!({"model": MODEL_NAME, "revision": MODEL_REVISION,
        "runtime_version": RUNTIME_VERSION, "target": TARGET_TRIPLE, "sha256": hashes}),
    )
}
