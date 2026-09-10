//! Report value types and deterministic hashing helpers.
//!
//! Reports intentionally carry only classification, counts, digests, ledger
//! outcome counts, an error code, and byte-identity booleans — never row
//! text, hostnames, or absolute home paths.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Lowercase hex encoding, used for stable digests and blob canonicalization.
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// SHA-256 digest of an in-memory byte slice, hex encoded.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

/// SHA-256 digest of a file's bytes, hex encoded.
pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

/// Deterministic digest over every regular file below `dir`, keyed by sorted
/// relative path. Any symlink inside the tree is an error.
pub fn hash_directory(dir: &Path) -> anyhow::Result<String> {
    let mut entries = Vec::new();
    collect_entries(dir, dir, &mut entries)?;
    entries.sort();
    let mut hasher = Sha256::new();
    for (relative, digest) in &entries {
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update(digest.as_bytes());
        hasher.update([b'\n']);
    }
    Ok(hex_encode(&hasher.finalize()))
}

fn collect_entries(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("list {}", dir.display()))? {
        let entry = entry.with_context(|| format!("walk {}", dir.display()))?;
        let path = entry.path();
        let meta =
            fs::symlink_metadata(&path).with_context(|| format!("inspect {}", path.display()))?;
        anyhow::ensure!(
            !meta.is_symlink(),
            "unexpected symlink inside the fixture tree: {}",
            path.display()
        );
        let relative = path
            .strip_prefix(root)
            .expect("entries are below the root")
            .to_string_lossy()
            .into_owned();
        if meta.is_dir() {
            collect_entries(root, &path, out)?;
        } else {
            out.push((relative, sha256_file(&path)?));
        }
    }
    Ok(())
}

/// Classification of one frozen fixture, cross-checked against the verified
/// projection's variant expectations. `name` is the classification label
/// (for example `supported-python`); `source_name` is the fixture basename.
#[derive(Debug, Serialize)]
pub struct Classification {
    pub name: String,
    pub safe_to_convert: bool,
    pub source_name: String,
    pub integrity: String,
    pub foreign_key_violations: u64,
    pub schema_digest: String,
    pub source_sha256: String,
    pub variant_id: String,
}

/// One exact FTS probe: the matched row ids plus the digest of those ids.
#[derive(Debug, Serialize)]
pub struct FtsProbeResult {
    pub ids: Vec<i64>,
    pub digest: String,
}

/// Ledger outcome counts read back from the neutral target.
#[derive(Debug, Serialize)]
pub struct LedgerCounts {
    pub read: u64,
    pub skipped: u64,
    pub blocking: u64,
}

/// Receipt of a successful (or exact) neutral probe import. It carries only
/// counts, digests, and byte-identity booleans.
#[derive(Debug, Serialize)]
pub struct ProbeReceipt {
    pub ok: bool,
    pub source_name: String,
    pub classification: String,
    pub safe_to_convert: bool,
    pub source_bytes_identical: bool,
    pub fixture_directory_identical: bool,
    pub source_sha256: String,
    pub schema_digest: String,
    pub migration_sources_digest: String,
    pub probe_row_count: u64,
    pub ledger: LedgerCounts,
    pub rows_per_table: BTreeMap<String, u64>,
    pub row_counts: BTreeMap<String, i64>,
    pub representative_row_digests: BTreeMap<String, String>,
    pub fts: BTreeMap<String, FtsProbeResult>,
    pub target_sha256: String,
    pub error_code: Option<String>,
}

/// JSON report printed when the probe refuses a non-convertible fixture.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProbeRefusalReport {
    pub ok: bool,
    pub source_name: String,
    pub classification: String,
    pub safe_to_convert: bool,
    pub error_code: String,
}
