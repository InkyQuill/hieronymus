//! TLS for the outbound transport seam (rustls): `https://` works end to end
//! for both consumers — the provider client (`provider_http`) and the model
//! download (`semantic_model`) — against a loopback server presenting a locally
//! generated self-signed certificate injected as a custom root. Certificate
//! verification failures (wrong root, name mismatch) fail closed with typed
//! errors. No test ever leaves the machine.

mod common;

use std::time::Duration;

use common::TlsLoopbackServer;
use hieronymus::provider_http::{BlockingHttpTransport, HttpError, ProviderTransport};
use hieronymus::semantic_model::{HttpModelTransport, ModelTransport};
use hieronymus::tls::TlsRoots;
use serde_json::json;

const RESPONSE: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 20\r\nConnection: close\r\n\r\n{\"ok\": true, \"x\": 1}";

fn transport_trusting(certificate_der: &[u8]) -> BlockingHttpTransport {
    BlockingHttpTransport::new(1 << 20)
        .with_tls_roots(TlsRoots::custom(vec![certificate_der.to_vec()]))
}

fn model_transport_trusting(certificate_der: &[u8]) -> HttpModelTransport {
    HttpModelTransport::new(Duration::from_secs(10))
        .with_tls_roots(TlsRoots::custom(vec![certificate_der.to_vec()]))
}

// ---------------------------------------------------------------------------
// Provider client (Task 4 seam)
// ---------------------------------------------------------------------------

#[test]
fn provider_get_and_post_round_trip_over_https() {
    let server = TlsLoopbackServer::start(&["127.0.0.1", "localhost"], RESPONSE);
    let transport = transport_trusting(&server.certificate_der);

    let got = transport
        .get_json(
            &server.https_url("/v1/models"),
            &[],
            Duration::from_secs(10),
        )
        .expect("GET over https");
    assert_eq!(got.status, 200);
    assert_eq!(got.body, "{\"ok\": true, \"x\": 1}");

    let posted = transport
        .post_json(
            &server.https_url("/v1/chat"),
            &[],
            &json!({"prompt": "translate"}),
            Duration::from_secs(10),
        )
        .expect("POST over https");
    assert_eq!(posted.status, 200);

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0].starts_with("GET /v1/models HTTP/1.1\r\n"),
        "{requests:?}"
    );
    assert!(
        requests[1].starts_with("POST /v1/chat HTTP/1.1\r\n"),
        "{requests:?}"
    );
    assert!(
        requests[1].contains("\"prompt\":\"translate\""),
        "{requests:?}"
    );
}

#[test]
fn provider_https_wrong_root_fails_closed_with_typed_error() {
    // The client trusts a different self-signed certificate than the one the
    // server presents: verification must fail closed (unknown issuer).
    let server = TlsLoopbackServer::start(&["127.0.0.1"], RESPONSE);
    let other = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()])
        .expect("unrelated certificate");
    let transport = transport_trusting(other.cert.der());

    let error = transport
        .get_json(
            &server.https_url("/v1/models"),
            &[],
            Duration::from_secs(10),
        )
        .expect_err("wrong root must fail");
    assert!(matches!(error, HttpError::Verification(_)), "{error}");
    assert!(!error.is_retryable(), "{error}");
}

#[test]
fn provider_https_name_mismatch_fails_closed_with_typed_error() {
    // The presented certificate is trusted as a root, but names a different
    // host: the name check must still fail the handshake.
    let server = TlsLoopbackServer::start(&["wrong.example"], RESPONSE);
    let transport = transport_trusting(&server.certificate_der);

    let error = transport
        .get_json(
            &server.https_url("/v1/models"),
            &[],
            Duration::from_secs(10),
        )
        .expect_err("name mismatch must fail");
    assert!(matches!(error, HttpError::Verification(_)), "{error}");
}

#[test]
fn provider_https_with_default_roots_does_not_trust_local_certificates() {
    let server = TlsLoopbackServer::start(&["127.0.0.1"], RESPONSE);
    let transport = BlockingHttpTransport::new(1 << 20);

    let error = transport
        .get_json(
            &server.https_url("/v1/models"),
            &[],
            Duration::from_secs(10),
        )
        .expect_err("webpki roots must not trust a local self-signed certificate");
    assert!(matches!(error, HttpError::Verification(_)), "{error}");
}

#[test]
fn provider_unsupported_schemes_still_fail_closed_without_dialing() {
    let transport = BlockingHttpTransport::new(1 << 20);
    for url in [
        "ftp://example.invalid/x",
        "example.invalid/x",
        "file:///etc",
    ] {
        let error = transport
            .get_json(url, &[], Duration::from_secs(1))
            .expect_err(url);
        assert!(
            matches!(error, HttpError::UnsupportedUrl(_)),
            "{url}: {error}"
        );
    }
}

// ---------------------------------------------------------------------------
// Model download (Task 7 seam)
// ---------------------------------------------------------------------------

const ARTIFACT: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 11\r\nConnection: close\r\n\r\nonnx-bytes!";

#[test]
fn model_download_round_trips_over_https() {
    let server = TlsLoopbackServer::start(&["127.0.0.1", "localhost"], ARTIFACT);
    let transport = model_transport_trusting(&server.certificate_der);
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("model.onnx");

    let written = transport
        .download_to(&server.https_url("/model.onnx"), &destination, 1 << 20)
        .expect("download over https");
    assert_eq!(written, b"onnx-bytes!".len() as u64);
    assert_eq!(std::fs::read(&destination).unwrap(), b"onnx-bytes!");
    assert!(server.requests()[0].starts_with("GET /model.onnx HTTP/1.1\r\n"));
}

#[test]
fn model_download_wrong_root_fails_closed_with_typed_error() {
    let server = TlsLoopbackServer::start(&["127.0.0.1"], ARTIFACT);
    let other = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()])
        .expect("unrelated certificate");
    let transport = model_transport_trusting(other.cert.der());

    let error = transport
        .download_to(
            &server.https_url("/model.onnx"),
            &std::env::temp_dir().join("hieronymus-tls-test-should-not-exist"),
            1 << 20,
        )
        .expect_err("wrong root must fail");
    assert!(
        matches!(
            error,
            hieronymus::semantic_error::SemanticError::Verification(_)
        ),
        "{error}"
    );
    assert!(
        !std::env::temp_dir()
            .join("hieronymus-tls-test-should-not-exist")
            .exists()
    );
}

#[test]
fn model_download_name_mismatch_fails_closed_with_typed_error() {
    let server = TlsLoopbackServer::start(&["wrong.example"], ARTIFACT);
    let transport = model_transport_trusting(&server.certificate_der);

    let error = transport
        .download_to(
            &server.https_url("/model.onnx"),
            &std::env::temp_dir().join("hieronymus-tls-test-should-not-exist"),
            1 << 20,
        )
        .expect_err("name mismatch must fail");
    assert!(
        matches!(
            error,
            hieronymus::semantic_error::SemanticError::Verification(_)
        ),
        "{error}"
    );
}

#[test]
fn model_download_unsupported_schemes_still_fail_closed() {
    let transport = HttpModelTransport::new(Duration::from_secs(1));
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("model.onnx");
    let error = transport
        .download_to("gopher://example.invalid/x", &destination, 1 << 20)
        .expect_err("unsupported scheme");
    assert!(
        matches!(
            error,
            hieronymus::semantic_error::SemanticError::UnsupportedUrl(_)
        ),
        "{error}"
    );
}
