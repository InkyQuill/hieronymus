//! Verify two bounded archives through authenticated anonymous snapshots before assembly.
use crate::release_manifest::{ReleaseV2, metadata_name, model_members};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const MAX_ARCHIVE: u64 = 1024 * 1024 * 1024;
pub const MAX_EXPANDED: u64 = 3 * 1024 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct VerifiedSplitRelease {
    pub manifest: ReleaseV2,
    pub manifest_sha256: String,
    pub platform_archive: PathBuf,
    pub model_archive: PathBuf,
    pub assets_sha256: String,
}
/// Open without following symlinks/reparse points and verify the same handle used later.
fn regular_file(path: &Path, limit: u64) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(
            (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|e| format!("cannot open release member {}: {e}", path.display()))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file()
        || meta.len() > limit
        || std::fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
    {
        return Err("release file type or size mismatch".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err("release file must not be hardlinked".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err("release reparse point refused".into());
        }
    }
    Ok(file)
}
/// An owned anonymous spool containing only the authenticated byte stream.
/// Inspection receives Read+Seek, never a pathname or writable file capability.
struct AuthenticatedArchive(File);
struct SnapshotReader<'a>(&'a mut File);
impl Read for SnapshotReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(bytes)
    }
}
impl Seek for SnapshotReader<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(position)
    }
}
impl AuthenticatedArchive {
    fn reader(&mut self) -> SnapshotReader<'_> {
        SnapshotReader(&mut self.0)
    }
}
fn verified_file(path: &Path, expected: &str) -> Result<AuthenticatedArchive, String> {
    let mut source = regular_file(path, MAX_ARCHIVE)?;
    // tempfile 3.27: Unix O_TMPFILE or unlink-before-return; Windows create_new,
    // share_mode(0), DELETE_ON_CLOSE. No source-directory files are created.
    let mut snapshot =
        tempfile::tempfile().map_err(|e| format!("cannot create release snapshot: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // Fail closed if the crate's named fallback could not unlink its empty file.
        if snapshot.metadata().map_err(|e| e.to_string())?.nlink() != 0 {
            return Err("release snapshot must be anonymous".into());
        }
        snapshot
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let size = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 {
            break;
        }
        count = count
            .checked_add(size as u64)
            .ok_or("release snapshot size overflow")?;
        if count > MAX_ARCHIVE {
            return Err("release archive exceeds snapshot size bound".into());
        }
        // The digest authenticates exactly the bytes successfully copied into our spool.
        snapshot
            .write_all(&buffer[..size])
            .map_err(|e| format!("release snapshot write failed: {e}"))?;
        hash.update(&buffer[..size]);
    }
    if format!("{:x}", hash.finalize()) != expected {
        return Err(format!(
            "release archive checksum mismatch: {}",
            path.display()
        ));
    }
    snapshot
        .sync_all()
        .map_err(|e| format!("release snapshot sync failed: {e}"))?;
    snapshot.rewind().map_err(|e| e.to_string())?;
    Ok(AuthenticatedArchive(snapshot))
}
#[derive(Default)]
struct Inspection {
    paths: BTreeSet<String>,
    hashes: BTreeMap<String, String>,
    bytes: u64,
    assets: Option<Vec<u8>>,
}
#[derive(Clone, Copy)]
enum Policy {
    Platform,
    Model,
}
fn runtime_pins(target: &str) -> Result<BTreeMap<String, String>, String> {
    hieronymus::semantic_arming::runtime_member_pins(target)
}
fn executable_names(target: &str) -> Vec<&'static str> {
    if target.contains("windows") {
        vec!["hiero.exe", "hiero-launcher.exe", "hiero-desktop.exe"]
    } else {
        vec!["hiero", "hiero-desktop"]
    }
}
fn permitted(name: &str, directory: bool, policy: Policy, target: &str) -> Result<bool, String> {
    if name.is_empty()
        || name.contains('\\')
        || name.split('/').any(|s| {
            s.is_empty() || s == "." || s == ".." || s.contains(':') || s.ends_with(['.', ' '])
        })
    {
        return Ok(false);
    }
    Ok(match policy {
        Policy::Model => {
            if directory {
                ["models", "models/minilm"].contains(&name)
            } else {
                model_members().contains_key(name)
            }
        }
        Policy::Platform => {
            if directory {
                ["lib", "licenses", "licenses/runtime"].contains(&name)
            } else {
                name == "assets.json"
                    || executable_names(target).contains(&name)
                    || runtime_pins(target)?.contains_key(name)
            }
        }
    })
}
struct Member<'a> {
    name: &'a str,
    directory: bool,
    size: u64,
}
fn record(
    member: Member<'_>,
    reader: &mut dyn Read,
    policy: Policy,
    target: &str,
    state: &mut Inspection,
    destination: Option<&Path>,
) -> Result<(), String> {
    let Member {
        name,
        directory,
        size,
    } = member;
    if !state.paths.insert(name.into()) || !permitted(name, directory, policy, target)? {
        return Err(format!("unsafe or duplicate archive member: {name}"));
    }
    state.bytes = state
        .bytes
        .checked_add(size)
        .ok_or("archive size overflow")?;
    if state.bytes > MAX_EXPANDED || (directory && size != 0) {
        return Err("expanded archive exceeds bound".into());
    }
    if directory {
        return Ok(());
    }
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buf = [0; 65536];
    let mut asset = Vec::new();
    let mut output = if let Some(root) = destination {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().ok_or("missing parent")?)
            .map_err(|e| e.to_string())?;
        Some(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    loop {
        let count = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > size {
            return Err("archive member size mismatch".into());
        }
        hash.update(&buf[..count]);
        if name == "assets.json" {
            if bytes > 65536 {
                return Err("assets manifest exceeds bound".into());
            }
            asset.extend_from_slice(&buf[..count]);
        }
        if let Some(file) = &mut output {
            file.write_all(&buf[..count]).map_err(|e| e.to_string())?;
        }
    }
    if bytes != size {
        return Err("truncated archive member".into());
    }
    state
        .hashes
        .insert(name.into(), format!("{:x}", hash.finalize()));
    if name == "assets.json" {
        state.assets = Some(asset);
    }
    Ok(())
}
fn inspect<R: Read + Seek>(
    file: &mut R,
    zip: bool,
    policy: Policy,
    target: &str,
    destination: Option<&Path>,
) -> Result<Inspection, String> {
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut state = Inspection::default();
    if zip {
        crate::release_zip::validate(file)?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        if archive.len() > 10000 {
            return Err("ZIP member count exceeds bound".into());
        }
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
            let directory = entry.is_dir();
            let name = entry.name().trim_end_matches('/').to_owned();
            if entry.encrypted()
                || entry.is_symlink()
                || entry
                    .unix_mode()
                    .is_some_and(|m| ![0, 0o100000, 0o040000].contains(&(m & 0o170000)))
            {
                return Err("unsupported ZIP member".into());
            }
            let size = entry.size();
            record(
                Member {
                    name: &name,
                    directory,
                    size,
                },
                &mut entry,
                policy,
                target,
                &mut state,
                destination,
            )?;
        }
    } else {
        let mut archive =
            tar::Archive::new(flate2::read::MultiGzDecoder::new(file).take(MAX_EXPANDED + 1));
        for entry in archive.entries().map_err(|e| e.to_string())?.raw(true) {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path_bytes();
            let name = std::str::from_utf8(&path)
                .map_err(|e| e.to_string())?
                .trim_end_matches('/')
                .to_owned();
            let kind = entry.header().entry_type();
            if kind.is_symlink()
                && matches!(policy, Policy::Platform)
                && !target.contains("windows")
                && ["hieronymus", "hieronymus-mcp", "hieronymus-agent-hook"]
                    .contains(&name.as_str())
            {
                if entry.link_name().map_err(|e| e.to_string())?.as_deref()
                    != Some(Path::new("hiero"))
                    || entry.size() != 0
                    || !state.paths.insert(name.clone())
                {
                    return Err("invalid or duplicate command symlink".into());
                }
                #[cfg(unix)]
                if let Some(root) = destination {
                    std::os::unix::fs::symlink("hiero", root.join(name))
                        .map_err(|e| e.to_string())?;
                }
                continue;
            }
            if !kind.is_file() && !kind.is_dir() {
                return Err("unsupported tar member".into());
            }
            let size = entry.size();
            record(
                Member {
                    name: &name,
                    directory: kind.is_dir(),
                    size,
                },
                &mut entry,
                policy,
                target,
                &mut state,
                destination,
            )?;
        }
        let mut reader = archive.into_inner();
        let mut tail = [0u8; 65536];
        loop {
            let count = reader.read(&mut tail).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            if tail[..count].iter().any(|byte| *byte != 0) {
                return Err("unaccounted tar bytes after terminator".into());
            }
        }
        if reader.limit() == 0 {
            return Err("expanded archive exceeds bound".into());
        }
    }
    for (name, expected) in match policy {
        Policy::Model => model_members(),
        Policy::Platform => runtime_pins(target)?,
    } {
        if state.hashes.get(&name) != Some(&expected) {
            return Err(format!(
                "missing or mismatched pinned archive member: {name}"
            ));
        }
    }
    if matches!(policy, Policy::Platform) {
        let required = if target.contains("windows") {
            vec!["hiero.exe", "hiero-launcher.exe", "assets.json"]
        } else {
            vec!["hiero", "assets.json"]
        };
        if required
            .iter()
            .any(|name| !state.hashes.contains_key(*name))
        {
            return Err("platform archive is missing executable/launcher/assets".into());
        }
    }
    Ok(state)
}
fn verify_pair(
    directory: &Path,
    target: &str,
    destination: Option<&Path>,
) -> Result<VerifiedSplitRelease, String> {
    let mut metadata = regular_file(&directory.join(metadata_name(target)), 65536)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut metadata)
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let manifest = ReleaseV2::parse(&bytes, target)?;
    let platform_archive = directory.join(&manifest.platform.archive);
    let model_archive = directory.join(&manifest.model.archive);
    let mut platform = verified_file(&platform_archive, &manifest.platform.sha256)?;
    let mut model = verified_file(&model_archive, &manifest.model.sha256)?;
    let p = inspect(
        &mut platform.reader(),
        target.contains("windows"),
        Policy::Platform,
        target,
        None,
    )?;
    let m = inspect(&mut model.reader(), false, Policy::Model, target, None)?;
    if p.bytes
        .checked_add(m.bytes)
        .ok_or("combined archive overflow")?
        > MAX_EXPANDED
        || !p.paths.is_disjoint(&m.paths)
    {
        return Err("cross-archive collision or combined extraction overflow".into());
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Assets {
        model: String,
        revision: String,
        runtime_version: String,
        target: String,
        #[serde(deserialize_with = "crate::release_manifest::unique_members")]
        sha256: BTreeMap<String, String>,
    }
    let assets: Assets =
        serde_json::from_slice(p.assets.as_ref().ok_or("missing assets manifest")?)
            .map_err(|e| e.to_string())?;
    let mut expected = p.hashes.clone();
    let assets_sha256 = expected
        .remove("assets.json")
        .ok_or("missing assets digest")?;
    expected.extend(m.hashes);
    if assets.model != manifest.model.name
        || assets.revision != manifest.model.revision
        || assets.runtime_version != "1.28.0"
        || assets.target != target
        || assets.sha256 != expected
    {
        return Err("assembled assets manifest does not bind every member".into());
    }
    if let Some(root) = destination {
        if !root.is_dir()
            || std::fs::read_dir(root)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            return Err("assembly destination must be an empty directory".into());
        }
        // Every pass reads the same immutable authenticated snapshots; source files are never reread.
        let written_platform = inspect(
            &mut platform.reader(),
            target.contains("windows"),
            Policy::Platform,
            target,
            Some(root),
        )?;
        let written_model = inspect(
            &mut model.reader(),
            false,
            Policy::Model,
            target,
            Some(root),
        )?;
        let mut written = written_platform.hashes;
        written.remove("assets.json");
        written.extend(written_model.hashes);
        if written != expected || written_platform.assets != p.assets {
            return Err("archive changed during assembly".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in executable_names(target) {
                let path = root.join(name);
                if path.exists() {
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                        .map_err(|e| e.to_string())?;
                }
            }
        }
    }
    Ok(VerifiedSplitRelease {
        manifest,
        manifest_sha256: format!("{:x}", Sha256::digest(bytes)),
        platform_archive,
        model_archive,
        assets_sha256,
    })
}
pub fn verify_split_directory(
    directory: &Path,
    target: &str,
) -> Result<VerifiedSplitRelease, String> {
    verify_pair(directory, target, None)
}
/// Destination must be absent; failure cleans only this fresh assembly directory.
pub fn extract_split_directory(
    directory: &Path,
    target: &str,
    destination: &Path,
) -> Result<VerifiedSplitRelease, String> {
    std::fs::create_dir(destination).map_err(|e| e.to_string())?;
    let result = verify_pair(directory, target, Some(destination));
    match result {
        Ok(verified) => Ok(verified),
        Err(error) => {
            std::fs::remove_dir_all(destination).map_err(|cleanup| {
                format!("{error}; failed to remove incomplete release assembly: {cleanup}")
            })?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tar(names: &[(&str, u8, u64)]) -> File {
        let mut bytes = Vec::new();
        for (name, kind, size) in names {
            let mut header = tar::Header::new_ustar();
            header.set_size(*size);
            header.set_mode(0o600);
            header.set_entry_type(tar::EntryType::new(*kind));
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            bytes.extend_from_slice(header.as_bytes());
            if *size < 1024 {
                bytes.extend(std::iter::repeat_n(0, (*size).div_ceil(512) as usize * 512));
            }
        }
        bytes.extend([0; 1024]);
        let file = tempfile::tempfile().unwrap();
        let mut gzip = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        gzip.write_all(&bytes).unwrap();
        gzip.finish().unwrap()
    }
    #[test]
    fn rejects_duplicate_model_members_and_cross_archive_smuggling() {
        let mut duplicate = tar(&[
            ("models/minilm/model.onnx", b'0', 1),
            ("models/minilm/model.onnx", b'0', 1),
        ]);
        assert!(
            inspect(
                &mut duplicate,
                false,
                Policy::Model,
                "x86_64-unknown-linux-gnu",
                None
            )
            .err()
            .unwrap()
            .contains("duplicate")
        );
        let mut smuggled = tar(&[("models/minilm/model.onnx", b'0', 1)]);
        assert!(
            inspect(
                &mut smuggled,
                false,
                Policy::Platform,
                "x86_64-unknown-linux-gnu",
                None
            )
            .err()
            .unwrap()
            .contains("unsafe")
        );
    }
    #[test]
    fn rejects_traversal_extensions_and_declared_expansion_overflow() {
        for (name, kind, size) in [
            ("models/../escape", b'0', 1),
            ("models/minilm/model.onnx", b'x', 1),
            ("models/minilm/model.onnx", b'0', MAX_EXPANDED + 1),
        ] {
            assert!(
                inspect(
                    &mut tar(&[(name, kind, size)]),
                    false,
                    Policy::Model,
                    "x86_64-unknown-linux-gnu",
                    None
                )
                .is_err()
            );
        }
    }
    #[test]
    fn refuses_hidden_concatenated_tar_streams() {
        let mut empty = tar(&[]);
        let mut second = tar(&[("models/minilm/model.onnx", b'0', 1)]);
        second.rewind().unwrap();
        empty.seek(SeekFrom::End(0)).unwrap();
        std::io::copy(&mut second, &mut empty).unwrap();
        assert!(
            inspect(
                &mut empty,
                false,
                Policy::Model,
                "x86_64-unknown-linux-gnu",
                None
            )
            .err()
            .unwrap()
            .contains("unaccounted")
        );
    }
    fn executable_tar(value: &[u8]) -> Vec<u8> {
        let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut archive = tar::Builder::new(gzip);
        let mut header = tar::Header::new_ustar();
        header.set_size(value.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append_data(&mut header, "hiero", value).unwrap();
        archive.into_inner().unwrap().finish().unwrap()
    }
    #[test]
    fn inspection_uses_authenticated_bytes_after_source_is_rewritten() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("platform.tar.gz");
        let original = executable_tar(b"authenticated executable");
        std::fs::write(&path, &original).unwrap();
        let mut snapshot =
            verified_file(&path, &format!("{:x}", Sha256::digest(&original))).unwrap();
        // Deterministic replacement in place, after authentication and before first inspection.
        std::fs::write(&path, executable_tar(b"replacement executable")).unwrap();
        let mut archive = tar::Archive::new(flate2::read::MultiGzDecoder::new(snapshot.reader()));
        let mut inspected = Vec::new();
        archive
            .entries()
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .read_to_end(&mut inspected)
            .unwrap();
        assert_eq!(inspected, b"authenticated executable");
    }
    #[cfg(unix)]
    #[test]
    fn authenticated_snapshot_is_anonymous_and_private() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::write(&source, b"original").unwrap();
        let snapshot =
            verified_file(&source, &format!("{:x}", Sha256::digest(b"original"))).unwrap();
        let metadata = snapshot.0.metadata().unwrap();
        assert_eq!(metadata.nlink(), 0);
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        drop(snapshot);
        assert_eq!(std::fs::read(&source).unwrap(), b"original");
    }
    #[cfg(windows)]
    #[test]
    fn snapshot_denies_reopening_and_is_deleted_on_close() {
        use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::write(&source, b"original").unwrap();
        let snapshot =
            verified_file(&source, &format!("{:x}", Sha256::digest(b"original"))).unwrap();
        let handle = snapshot.0.as_raw_handle();
        // Test-only inspection of our live handle; production never exposes this pathname.
        let size = unsafe { GetFinalPathNameByHandleW(handle, std::ptr::null_mut(), 0, 0) };
        assert_ne!(size, 0, "{}", std::io::Error::last_os_error());
        let mut wide = vec![0u16; size as usize + 1];
        let written =
            unsafe { GetFinalPathNameByHandleW(handle, wide.as_mut_ptr(), wide.len() as u32, 0) };
        assert!(written > 0 && written < wide.len() as u32);
        let path = PathBuf::from(std::ffi::OsString::from_wide(&wide[..written as usize]));
        let error = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap_err();
        assert_eq!(
            error.raw_os_error(),
            Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
        );
        drop(snapshot);
        assert!(!path.try_exists().unwrap());
        assert_eq!(std::fs::read(source).unwrap(), b"original");
    }
    #[test]
    fn rejects_compressed_overflow_before_hashing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sparse");
        File::create(&path)
            .unwrap()
            .set_len(MAX_ARCHIVE + 1)
            .unwrap();
        assert!(
            verified_file(&path, &"a".repeat(64))
                .err()
                .unwrap()
                .contains("size")
        );
    }
}
