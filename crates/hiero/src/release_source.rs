//! HTTPS release staging. Downloads never mutate the application or data root.
//! Redirects are refused (including HTTPS-to-HTTP), using the existing bounded
//! rustls model transport and its injectable trust anchors.
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hieronymus::semantic_model::{HttpModelTransport, ModelTransport};
use hieronymus::tls::{TlsRoots, parse_outbound_url};

use crate::app::{LINK_NAMES, TARGET_TRIPLE};

const MAX_ARCHIVE: u64 = 1024 * 1024 * 1024;
const MAX_EXPANDED: u64 = 3 * 1024 * 1024 * 1024;

pub fn validate_base_url(value: &str) -> Result<(), String> {
    let parsed = parse_outbound_url(value)?;
    if !parsed.secure {
        return Err("remote release source must use HTTPS (https://)".into());
    }
    if value.contains('?') {
        return Err("release base URL must not contain a query".into());
    }
    Ok(())
}

pub fn validate_channel(channel: &str) -> Result<(), String> {
    match channel {
        "stable" | "dev" => Ok(()),
        _ => Err("release channel must be stable or dev".into()),
    }
}

/// Typed decoding rejects duplicate security-sensitive metadata fields.
pub fn parse_metadata(bytes: &[u8]) -> Result<serde_json::Value, String> {
    if bytes.len() > 65536 {
        return Err("release metadata exceeds byte bound".into());
    }
    let shape: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if shape.get("format_version").is_some() {
        return serde_json::to_value(crate::release_manifest::ReleaseV2::parse(
            bytes,
            TARGET_TRIPLE,
        )?)
        .map_err(|e| e.to_string());
    }
    #[derive(serde::Deserialize, serde::Serialize)]
    #[serde(deny_unknown_fields)]
    struct Metadata {
        version: String,
        archive: String,
        sha256: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
        signature: Option<serde_json::Value>,
    }
    let metadata: Metadata =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid release metadata: {e}"))?;
    serde_json::to_value(metadata).map_err(|e| e.to_string())
}

/// Validate metadata before constructing a download or filesystem path.
pub fn validate_metadata(payload: &serde_json::Value) -> Result<(), String> {
    if payload.get("format_version").is_some() {
        return crate::release_manifest::ReleaseV2::parse(
            &serde_json::to_vec(payload).map_err(|e| e.to_string())?,
            TARGET_TRIPLE,
        )
        .map(|_| ());
    }
    let field = |name| {
        payload
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("release metadata needs string {name}"))
    };
    let version = field("version")?;
    if version.is_empty()
        || version == "."
        || version == ".."
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
    {
        return Err("invalid release version".into());
    }
    if payload
        .get("target")
        .is_some_and(|target| target.as_str() != Some(TARGET_TRIPLE))
    {
        return Err(format!("release target must be {TARGET_TRIPLE}"));
    }
    if field("archive")? != format!("hieronymus-{version}-{TARGET_TRIPLE}.tar.gz") {
        return Err(format!(
            "release archive must match version and target {TARGET_TRIPLE}"
        ));
    }
    let digest = field("sha256")?;
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("release sha256 must be a 64-digit hex digest".into());
    }
    if payload.get("signature").is_some_and(|s| !s.is_null()) {
        return Err(
            "signature verification is not configured; refusing signed release metadata".into(),
        );
    }
    Ok(())
}

pub fn stage_remote(base_url: &str, channel: &str, destination: &Path) -> Result<PathBuf, String> {
    stage_remote_with_roots(base_url, channel, destination, TlsRoots::default())
}

/// Trust-anchor injection for real loopback TLS tests; production uses WebPKI.
pub fn stage_remote_with_roots(
    base_url: &str,
    channel: &str,
    destination: &Path,
    roots: TlsRoots,
) -> Result<PathBuf, String> {
    validate_base_url(base_url)?;
    validate_channel(channel)?;
    if destination.exists() {
        return Err("release staging destination already exists".into());
    }
    let parent = destination
        .parent()
        .ok_or("staging destination has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = tempfile::Builder::new()
        .prefix(".release-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    let transport = HttpModelTransport::new(Duration::from_secs(60)).with_tls_roots(roots);
    let base = format!("{}/{channel}", base_url.trim_end_matches('/'));
    let target_name = crate::release_manifest::metadata_name(TARGET_TRIPLE);
    let target_metadata = temporary.path().join(&target_name);
    match transport.download_to(
        &format!("{base}/{target_name}"),
        &target_metadata,
        64 * 1024,
    ) {
        Ok(_) => {
            let manifest = crate::release_manifest::ReleaseV2::parse(
                &std::fs::read(&target_metadata).map_err(|e| e.to_string())?,
                TARGET_TRIPLE,
            )?;
            if manifest.channel != channel {
                return Err("release metadata channel mismatch".into());
            }
            for name in [&manifest.platform.archive, &manifest.model.archive] {
                transport
                    .download_to(
                        &format!("{base}/{name}"),
                        &temporary.path().join(name),
                        MAX_ARCHIVE,
                    )
                    .map_err(|e| e.to_string())?;
            }
            crate::release_archive::verify_split_directory(temporary.path(), TARGET_TRIPLE)?;
            std::fs::rename(temporary.path(), destination).map_err(|e| e.to_string())?;
            return Ok(destination.to_path_buf());
        }
        Err(error)
            if error
                .to_string()
                .ends_with("server answered with status 404") =>
        {
            if target_metadata.exists() {
                std::fs::remove_file(target_metadata).map_err(|e| e.to_string())?;
            }
        }
        Err(error) => return Err(error.to_string()),
    }
    let metadata = temporary.path().join("release.json");
    transport
        .download_to(&format!("{base}/release.json"), &metadata, 64 * 1024)
        .map_err(|e| e.to_string())?;
    let payload = parse_metadata(&std::fs::read(&metadata).map_err(|e| e.to_string())?)?;
    if payload.get("format_version").is_some() {
        return Err("split metadata must use its exact-target filename".into());
    }
    validate_metadata(&payload)?;
    if payload.get("channel").and_then(serde_json::Value::as_str) != Some(channel) {
        return Err(format!(
            "release metadata does not declare requested channel {channel}"
        ));
    }
    let archive_name = payload
        .get("archive")
        .and_then(serde_json::Value::as_str)
        .ok_or("legacy metadata needs an archive")?;
    let archive = temporary.path().join(archive_name);
    transport
        .download_to(&format!("{base}/{archive_name}"), &archive, MAX_ARCHIVE)
        .map_err(|e| e.to_string())?;
    verify_directory(temporary.path())?;
    std::fs::rename(temporary.path(), destination).map_err(|e| e.to_string())?;
    Ok(destination.to_path_buf())
}

/// Shared local/remote release checks, before any activation or native load.
pub fn verify_directory(directory: &Path) -> Result<crate::update::ResolvedRelease, String> {
    let release = crate::update::resolve_release(directory).map_err(|e| e.to_string())?;
    if crate::update::sha256_file(&release.archive).map_err(|e| e.to_string())? != release.sha256 {
        return Err("release archive checksum mismatch".into());
    }
    inspect_archive(&release.archive)?;
    Ok(release)
}

type ArchiveReader = tar::Archive<std::io::Take<flate2::read::GzDecoder<std::fs::File>>>;

fn archive_reader(path: &Path) -> Result<ArchiveReader, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_ARCHIVE {
        return Err("release archive exceeds size limit".into());
    }
    Ok(tar::Archive::new(
        flate2::read::GzDecoder::new(file).take(MAX_EXPANDED + 1),
    ))
}

/// The fixed release layout permits only the three command symlinks. All
/// other entries must be regular files or known directories, with unique
/// literal paths; hardlinks, devices, extensions and traversal fail closed.
pub fn inspect_archive(path: &Path) -> Result<(), String> {
    let mut archive = archive_reader(path)?;
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let name = name.trim_end_matches('/');
        if !seen.insert(name.to_string()) {
            return Err(format!("duplicate archive entry: {name}"));
        }
        let kind = entry.header().entry_type();
        let directory = [
            "lib",
            "models",
            "models/minilm",
            "licenses",
            "licenses/runtime",
        ];
        let regular = [
            "hiero",
            "assets.json",
            "lib/libonnxruntime.so",
            "models/minilm/model.onnx",
            "models/minilm/tokenizer.json",
            "models/minilm/LICENSE",
            "models/minilm/README.md",
            "licenses/runtime/LICENSE",
            "licenses/runtime/ThirdPartyNotices.txt",
            "licenses/runtime/VERSION_NUMBER",
        ];
        if LINK_NAMES[1..].contains(&name) {
            if !kind.is_symlink()
                || entry.link_name().map_err(|e| e.to_string())?.as_deref()
                    != Some(Path::new("hiero"))
            {
                return Err(format!("invalid command symlink: {name}"));
            }
        } else if !(kind.is_dir() && directory.contains(&name)
            || kind.is_file() && regular.contains(&name))
        {
            return Err(format!("unsafe archive entry: {name}"));
        }
        total = total
            .checked_add(entry.size())
            .ok_or("archive size overflow")?;
        if total > MAX_EXPANDED {
            return Err("expanded archive exceeds size limit".into());
        }
        std::io::copy(&mut entry, &mut std::io::sink()).map_err(|e| e.to_string())?;
    }
    let mut reader = archive.into_inner();
    std::io::copy(&mut reader, &mut std::io::sink()).map_err(|e| e.to_string())?;
    if reader.limit() == 0 {
        return Err("expanded archive exceeds size limit".into());
    }
    if !LINK_NAMES.iter().all(|name| seen.contains(*name)) {
        return Err("archive is missing executable or command links".into());
    }
    Ok(())
}

pub fn extract_archive(path: &Path, destination: &Path) -> Result<(), String> {
    inspect_archive(path)?;
    let mut archive = archive_reader(path)?;
    archive.set_preserve_permissions(false);
    archive.set_preserve_mtime(false);
    archive.unpack(destination).map_err(|e| e.to_string())
}
