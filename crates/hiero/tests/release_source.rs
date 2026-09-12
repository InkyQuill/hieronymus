use hiero::release_source::validate_base_url;

#[test]
fn updater_requires_https_for_remote_release_sources() {
    assert!(validate_base_url("http://example.org/releases").is_err());
    assert!(validate_base_url("https://user:secret@example.org/releases").is_err());
    assert!(validate_base_url("https://example.org/releases").is_ok());
}

use hiero::release_source::{inspect_archive, stage_remote_with_roots};
use hieronymus::tls::{TlsRoots, parse_outbound_url};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[test]
fn shared_url_parser_rejects_ambiguous_authorities_and_preserves_http_and_ipv6() {
    for value in [
        "https://@example.org/a",
        "https://host:0/a",
        "https://host:65536/a",
        "https://host:abc/a",
        "https://host name/a",
        "https://host/a\r\nX:y",
        "https://host\\evil/a",
        "https://-host/a",
        "https://host/a#fragment",
        "https:///no-host",
    ] {
        assert!(validate_base_url(value).is_err(), "{value}");
    }
    let ipv6 = parse_outbound_url("https://[::1]:8443/releases").unwrap();
    assert_eq!(ipv6.host, "::1");
    assert_eq!(ipv6.port, 8443);
    assert_eq!(ipv6.http_authority(), "[::1]:8443");
    assert_eq!(
        parse_outbound_url("https://example.org/a")
            .unwrap()
            .http_authority(),
        "example.org:443"
    );
    assert!(
        !parse_outbound_url("http://127.0.0.1:9000/v1")
            .unwrap()
            .secure
    );
}

struct Server {
    url: String,
    roots: TlsRoots,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(responses: Vec<(String, Vec<u8>)>) -> Self {
        Self::on_address(responses, "127.0.0.1:0", "127.0.0.1")
    }
    fn on_address(
        responses: Vec<(String, Vec<u8>)>,
        address: &str,
        certificate_host: &str,
    ) -> Self {
        let certified = rcgen::generate_simple_self_signed(vec![certificate_host.into()]).unwrap();
        let roots = TlsRoots::custom(vec![certified.cert.der().to_vec()]);
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certified.cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into()),
        )
        .unwrap();
        let listener = std::net::TcpListener::bind(address).unwrap();
        let url = format!("https://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let seen = requests.clone();
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let Ok((socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let connection = rustls::ServerConnection::new(Arc::new(config.clone())).unwrap();
                let mut stream = rustls::StreamOwned::new(connection, socket);
                let mut request = Vec::new();
                let mut byte = [0u8];
                while stream.read_exact(&mut byte).is_ok() {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                if listener.local_addr().unwrap().is_ipv6() && !request.contains("\r\nHost: [::1]:")
                {
                    let _ = stream.write_all(b"HTTP/1.1 400 Bad Host\r\nContent-Length: 0\r\n\r\n");
                    let _ = stream.flush();
                    continue;
                }
                let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                seen.lock().unwrap().push(path.clone());
                if let Some((_, response)) = responses.iter().find(|(p, _)| p == &path) {
                    let _ = stream.write_all(response);
                    let _ = stream.flush();
                    stream.conn.send_close_notify();
                    let _ = stream.flush();
                } else {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    let _ = stream.flush();
                    stream.conn.send_close_notify();
                    let _ = stream.flush();
                }
            }
        });
        Self {
            url,
            roots,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn response(bytes: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )
    .into_bytes();
    response.extend_from_slice(bytes);
    response
}
fn archive(extra: Option<(&str, tar::EntryType, &str)>) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    let mut add = |name: &str, kind: tar::EntryType, link: &str| {
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o755);
        header.set_entry_type(kind);
        header.set_size(0);
        // Raw header spelling also exercises traversal that Builder::append_data refuses.
        header.as_mut_bytes()[..100].fill(0);
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        if !link.is_empty() {
            header.set_link_name(link).unwrap();
        }
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
    };
    add("hiero", tar::EntryType::Regular, "");
    for name in ["hieronymus", "hieronymus-agent-hook", "hieronymus-mcp"] {
        add(name, tar::EntryType::Symlink, "hiero");
    }
    if let Some((name, kind, link)) = extra {
        add(name, kind, link);
    }
    builder.into_inner().unwrap().finish().unwrap()
}
fn metadata(channel: &str, bytes: &[u8]) -> serde_json::Value {
    use sha2::Digest;
    let target = hiero::app::TARGET_TRIPLE;
    serde_json::json!({"version":"0.7.0", "channel":channel, "target":hiero::app::TARGET_TRIPLE,
        "archive":format!("hieronymus-0.7.0-{target}.tar.gz"), "sha256":format!("{:x}",sha2::Sha256::digest(bytes)), "signature": null})
}
fn server_for(channel: &str, payload: &serde_json::Value, archive_response: Vec<u8>) -> Server {
    let target = hiero::app::TARGET_TRIPLE;
    Server::new(vec![
        (
            format!("/{channel}/release.json"),
            response(&serde_json::to_vec(payload).unwrap()),
        ),
        (
            format!("/{channel}/hieronymus-0.7.0-{target}.tar.gz"),
            archive_response,
        ),
    ])
}
#[test]
fn stable_and_dev_stage_exact_requested_channels_over_verified_tls() {
    for channel in ["stable", "dev"] {
        let bytes = archive(None);
        let payload = metadata(channel, &bytes);
        let server = server_for(channel, &payload, response(&bytes));
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("verified");
        assert_eq!(
            stage_remote_with_roots(&server.url, channel, &destination, server.roots.clone())
                .unwrap(),
            destination
        );
        assert_eq!(
            hiero::release_source::verify_directory(&destination)
                .unwrap()
                .version,
            "0.7.0"
        );
        assert!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|p| p.starts_with(&format!("/{channel}/")))
        );
    }
}
#[test]
fn failed_release_downloads_leave_no_staged_release_or_activation() {
    for case in [
        "wrong-target",
        "wrong-channel",
        "signature",
        "checksum",
        "interrupted",
        "traversal",
        "symlink",
        "duplicate",
        "hardlink",
    ] {
        let extra = match case {
            "traversal" => Some(("../escaped", tar::EntryType::Regular, "")),
            "symlink" => Some(("lib", tar::EntryType::Symlink, "/tmp")),
            "duplicate" => Some(("hiero", tar::EntryType::Regular, "")),
            "hardlink" => Some(("assets.json", tar::EntryType::Link, "hiero")),
            _ => None,
        };
        let bytes = archive(extra);
        let mut payload = metadata("stable", &bytes);
        match case {
            "wrong-target" => payload["target"] = "aarch64-unknown-linux-gnu".into(),
            "wrong-channel" => payload["channel"] = "dev".into(),
            "signature" => payload["signature"] = "unverified".into(),
            "checksum" => payload["sha256"] = "ab".repeat(32).into(),
            _ => {}
        }
        let body = if case == "interrupted" {
            b"HTTP/1.1 200 OK\r\nContent-Length: 9000\r\nConnection: close\r\n\r\nshort".to_vec()
        } else {
            response(&bytes)
        };
        let server = server_for("stable", &payload, body);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("verified");
        let app = temp.path().join("app");
        std::fs::create_dir(&app).unwrap();
        std::fs::write(app.join("current"), "prior version").unwrap();
        let error =
            stage_remote_with_roots(&server.url, "stable", &destination, server.roots.clone())
                .unwrap_err();
        assert!(!error.is_empty(), "{case}");
        assert!(!destination.exists(), "{case}");
        assert_eq!(
            std::fs::read_to_string(app.join("current")).unwrap(),
            "prior version"
        );
        assert!(!temp.path().join("escaped").exists());
        assert_eq!(
            std::fs::read_dir(temp.path()).unwrap().count(),
            1,
            "{case}: temporary stage leaked"
        );
    }
}
#[test]
fn redirects_and_untrusted_tls_are_refused() {
    for roots_valid in [true, false] {
        let server = Server::new(vec![("/stable/release.json".into(), b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/release.json\r\nContent-Length: 0\r\n\r\n".to_vec())]);
        let temp = tempfile::tempdir().unwrap();
        let roots = if roots_valid {
            server.roots.clone()
        } else {
            TlsRoots::default()
        };
        assert!(
            stage_remote_with_roots(&server.url, "stable", &temp.path().join("verified"), roots)
                .is_err()
        );
        assert!(!temp.path().join("verified").exists());
    }
}
#[test]
fn local_archive_verification_rejects_escape_before_extraction() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bad.tar.gz");
    std::fs::write(
        &path,
        archive(Some(("/absolute", tar::EntryType::Regular, ""))),
    )
    .unwrap();
    assert!(inspect_archive(Path::new(&path)).is_err());
}

#[test]
fn duplicate_metadata_and_path_versions_fail_closed() {
    assert!(
        hiero::release_source::parse_metadata(
            br#"{"version":"0.7.0","version":"0.8.0","archive":"a","sha256":"b"}"#
        )
        .is_err()
    );
    let bytes = archive(None);
    for version in ["..", ".", "../escape", "/tmp", "a/b"] {
        let mut payload = metadata("stable", &bytes);
        payload["version"] = version.into();
        assert!(hiero::release_source::validate_metadata(&payload).is_err());
    }
}

#[test]
fn doctor_refuses_corrupt_explicit_runtime_and_model_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = temp.path().join("bad-runtime.so");
    std::fs::write(&runtime, "unqualified runtime").unwrap();
    std::fs::write(
        temp.path().join("semantic.conf"),
        format!(
            "runtime_library = {:?}\nconfiguration_revision = 1\n",
            runtime.to_str().unwrap()
        ),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["doctor", "--json", "--data-root"])
        .arg(temp.path())
        .env(
            "HIERO_SEMANTIC_MODEL_DIR",
            temp.path().join("missing-model"),
        )
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("checksum mismatch"), "{text}");
    assert!(text.contains("bad-runtime.so"), "{text}");
    assert!(text.contains("missing-model/model.onnx"), "{text}");
}

#[test]
fn update_cli_honors_configured_source_and_rejects_conflicting_sources() {
    let temp = tempfile::tempdir().unwrap();
    let base = || {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_hiero"));
        command
            .args(["update", "--app-dir"])
            .arg(temp.path().join("app"))
            .env_remove("HIERONYMUS_RELEASE_DIR")
            .env_remove("HIERONYMUS_RELEASE_URL");
        command
    };
    let output = base()
        .env("HIERONYMUS_RELEASE_URL", "http://127.0.0.1:1/releases")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("HTTPS"));
    let output = base()
        .args([
            "--release-dir",
            "/unused",
            "--release-url",
            "https://127.0.0.1:1",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("mutually exclusive"));
    assert!(!temp.path().join("app").exists());
}

#[test]
#[ignore = "requires IPv6 loopback connectivity; qualification host times out in TCP connect before TLS"]
fn ipv6_release_staging_sends_bracketed_authority_over_real_tls() {
    let bytes = archive(None);
    let payload = metadata("stable", &bytes);
    let server = Server::on_address(
        vec![
            (
                "/stable/release.json".into(),
                response(&serde_json::to_vec(&payload).unwrap()),
            ),
            (
                "/stable/hieronymus-0.7.0-x86_64-unknown-linux-gnu.tar.gz".into(),
                response(&bytes),
            ),
        ],
        "[::1]:0",
        "::1",
    );
    let temp = tempfile::tempdir().unwrap();
    stage_remote_with_roots(
        &server.url,
        "stable",
        &temp.path().join("verified"),
        server.roots.clone(),
    )
    .unwrap();
}

#[test]
fn split_manifest_is_strict_and_target_specific() {
    use hiero::release_manifest::*;
    let target = hiero::app::TARGET_TRIPLE;
    let value = serde_json::json!({"format_version":2,"version":"0.8.0","target":target,"channel":"stable","platform":{"archive":platform_name("0.8.0",target),"sha256":"a".repeat(64)},"model":{"name":hieronymus::semantic_model::MODEL_NAME,"revision":hieronymus::semantic_model::MODEL_REVISION,"archive":model_name(),"sha256":"b".repeat(64),"members":model_members()},"signature":null});
    let text = value.to_string();
    assert!(ReleaseV2::parse(text.as_bytes(), target).is_ok());
    for broken in [
        text.replace(
            "\"format_version\":2",
            "\"format_version\":2,\"format_version\":2",
        ),
        text.replace("\"platform\":", "\"unknown\":true,\"platform\":"),
        text.replace(
            "\"models/minilm/LICENSE\":",
            "\"models/minilm/LICENSE\":\"bad\",\"models/minilm/LICENSE\":",
        ),
        text.replace(".tar.gz", ".zip"),
        text.replace("\"signature\":null", "\"signature\":\"fake\""),
    ] {
        assert!(
            ReleaseV2::parse(broken.as_bytes(), target).is_err(),
            "accepted {broken}"
        );
    }
    assert!(hiero::release_source::parse_metadata(text.as_bytes()).is_ok());
    let disguised = text.replace("\"format_version\":2", "\"format_version\":1");
    assert!(hiero::release_source::parse_metadata(disguised.as_bytes()).is_err());
}

fn split_metadata(platform: &[u8], model: &[u8]) -> serde_json::Value {
    use hiero::release_manifest::*;
    use sha2::{Digest, Sha256};
    let target = hiero::app::TARGET_TRIPLE;
    serde_json::json!({"format_version":2,"version":"0.8.0","target":target,"channel":"stable","platform":{"archive":platform_name("0.8.0",target),"sha256":format!("{:x}",Sha256::digest(platform))},"model":{"name":hieronymus::semantic_model::MODEL_NAME,"revision":hieronymus::semantic_model::MODEL_REVISION,"archive":model_name(),"sha256":format!("{:x}",Sha256::digest(model)),"members":model_members()},"signature":null})
}
#[test]
fn split_second_download_failure_never_promotes_or_falls_back() {
    let platform = b"platform fixture";
    let value = split_metadata(platform, b"missing model");
    let target = hiero::app::TARGET_TRIPLE;
    let metadata_path = format!("/stable/{}", hiero::release_manifest::metadata_name(target));
    let platform_path = format!("/stable/{}", value["platform"]["archive"].as_str().unwrap());
    let model_path = format!("/stable/{}", value["model"]["archive"].as_str().unwrap());
    let server = Server::new(vec![
        (
            metadata_path.clone(),
            response(value.to_string().as_bytes()),
        ),
        (platform_path.clone(), response(platform)),
    ]);
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("staged");
    let _error = stage_remote_with_roots(&server.url, "stable", &destination, server.roots.clone())
        .unwrap_err();
    assert!(!destination.exists());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    assert_eq!(
        *server.requests.lock().unwrap(),
        vec![metadata_path, platform_path, model_path]
    );
}
#[test]
fn split_missing_or_corrupt_model_cleans_fresh_assembly() {
    let root = tempfile::tempdir().unwrap();
    let release = root.path().join("release");
    std::fs::create_dir(&release).unwrap();
    let value = split_metadata(b"platform", b"model");
    std::fs::write(
        release.join(hiero::release_manifest::metadata_name(
            hiero::app::TARGET_TRIPLE,
        )),
        value.to_string(),
    )
    .unwrap();
    std::fs::write(
        release.join(value["platform"]["archive"].as_str().unwrap()),
        b"platform",
    )
    .unwrap();
    for exists in [false, true] {
        if exists {
            std::fs::write(
                release.join(value["model"]["archive"].as_str().unwrap()),
                b"tampered",
            )
            .unwrap();
        }
        let out = root.path().join("assembled");
        assert!(
            hiero::release_archive::extract_split_directory(
                &release,
                hiero::app::TARGET_TRIPLE,
                &out
            )
            .is_err()
        );
        assert!(!out.exists());
    }
}
#[test]
#[ignore = "requires HIERO_SPLIT_RELEASE_DIR containing actual pinned platform+model artifacts"]
fn real_split_archive_pair_verifies_assembles_and_rejects_tampering() {
    let source = std::path::PathBuf::from(
        std::env::var_os("HIERO_SPLIT_RELEASE_DIR")
            .expect("HIERO_SPLIT_RELEASE_DIR must name the disposable qualified pair"),
    );
    let target = hiero::app::TARGET_TRIPLE;
    let verified = hiero::release_archive::verify_split_directory(&source, target).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("assembled");
    let assembled =
        hiero::release_archive::extract_split_directory(&source, target, &output).unwrap();
    assert_eq!(verified.assets_sha256, assembled.assets_sha256);
    assert_eq!(
        hiero::update::sha256_file(&output.join("models/minilm/model.onnx")).unwrap(),
        hieronymus::semantic_model::MODEL_SHA256
    );
    let candidate = hiero::app::verify_semantic_assets(&output).unwrap();
    let declared: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("assets.json")).unwrap()).unwrap();
    assert_eq!(candidate, declared);
}

#[test]
fn split_metadata_cannot_masquerade_as_legacy_release_json() {
    let payload = split_metadata(b"platform", b"model");
    let server = Server::new(vec![(
        "/stable/release.json".into(),
        response(payload.to_string().as_bytes()),
    )]);
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("staged");
    let error = stage_remote_with_roots(&server.url, "stable", &destination, server.roots.clone())
        .unwrap_err();
    assert!(
        error.contains("exact-target filename"),
        "{error}; requests={:?}",
        server.requests.lock().unwrap()
    );
    assert!(!destination.exists());
}

#[test]
fn authenticated_snapshot_inspection_failure_cleans_assembly_and_preserves_sources() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("release");
    std::fs::create_dir(&directory).unwrap();
    let platform = archive(Some(("undeclared", tar::EntryType::Regular, "")));
    let model = b"model transport fixture";
    let payload = split_metadata(&platform, model);
    let metadata_path = directory.join(hiero::release_manifest::metadata_name(
        hiero::app::TARGET_TRIPLE,
    ));
    let platform_path = directory.join(payload["platform"]["archive"].as_str().unwrap());
    let model_path = directory.join(payload["model"]["archive"].as_str().unwrap());
    std::fs::write(&metadata_path, payload.to_string()).unwrap();
    std::fs::write(&platform_path, &platform).unwrap();
    std::fs::write(&model_path, model).unwrap();
    let output = root.path().join("assembled");
    let error = hiero::release_archive::extract_split_directory(
        &directory,
        hiero::app::TARGET_TRIPLE,
        &output,
    )
    .unwrap_err();
    assert!(error.contains("unsafe"), "{error}");
    assert!(!output.exists());
    assert_eq!(std::fs::read(&platform_path).unwrap(), platform);
    assert_eq!(std::fs::read(&model_path).unwrap(), model);
    assert_eq!(
        std::fs::read_to_string(&metadata_path).unwrap(),
        payload.to_string()
    );
    assert_eq!(std::fs::read_dir(directory).unwrap().count(), 3);
}
