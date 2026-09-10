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
    serde_json::json!({"version":"0.7.0", "channel":channel, "target":hiero::app::TARGET_TRIPLE,
        "archive":"hieronymus-0.7.0-x86_64-unknown-linux-gnu.tar.gz", "sha256":format!("{:x}",sha2::Sha256::digest(bytes)), "signature": null})
}
fn server_for(channel: &str, payload: &serde_json::Value, archive_response: Vec<u8>) -> Server {
    Server::new(vec![
        (
            format!("/{channel}/release.json"),
            response(&serde_json::to_vec(payload).unwrap()),
        ),
        (
            format!("/{channel}/hieronymus-0.7.0-x86_64-unknown-linux-gnu.tar.gz"),
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
