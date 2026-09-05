//! Outbound TLS for the transport seam (rustls, no dynamic OpenSSL): root
//! configuration, `https://` URL parsing, and one blocking client stream used
//! by both consumers — the provider client (`provider_http`) and the model
//! download (`semantic_model`).
//!
//! Root choice: the default trust store is the static Mozilla set bundled via
//! `webpki-roots` — no dependence on the host's CA bundle layout (several
//! minimal Linux installs ship none), identical behavior across machines, and
//! no reload ceremonies. Tests and air-gapped installs inject explicit custom
//! roots through [`TlsRoots::custom`]. Verification failures fail closed with
//! [`TlsError::Verification`]; a local daemon surface stays loopback-plain
//! (ADR 0012: TLS is an outbound-only concern).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::CertificateDer;

/// Where the client takes its trust anchors from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TlsRoots {
    /// The bundled Mozilla root store (webpki-roots): the default for real
    /// provider endpoints and the pinned model URL.
    #[default]
    WebPki,
    /// Explicitly trusted DER certificates (loopback tests, pinning). Nothing
    /// else is trusted when this variant is selected — no fallback to system
    /// or bundled roots.
    Custom(Vec<Vec<u8>>),
}

impl TlsRoots {
    /// Trusts exactly the given DER certificates.
    pub fn custom(certificates: Vec<Vec<u8>>) -> Self {
        Self::Custom(certificates)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("tls certificate verification failed: {0}")]
    Verification(String),
    #[error("tls handshake failed: {0}")]
    Handshake(String),
    #[error("tls configuration failed: {0}")]
    Config(String),
}

impl TlsError {
    /// Verification failures never retry: retrying cannot make an untrusted
    /// or mismatched certificate trustworthy.
    pub fn is_verification(&self) -> bool {
        matches!(self, TlsError::Verification(_))
    }
}

/// A blocking TLS client stream over one TCP connection.
pub struct TlsStream(rustls::StreamOwned<rustls::ClientConnection, TcpStream>);

impl TlsStream {
    /// Bounds the underlying socket's blocking reads and writes (`None`
    /// restores blocking-forever semantics).
    pub fn set_socket_timeouts(&self, read: Option<Duration>, write: Option<Duration>) {
        let _ = self.0.sock.set_read_timeout(read);
        let _ = self.0.sock.set_write_timeout(write);
    }
}

impl Read for TlsStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for TlsStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// Opens an authenticated, verified TLS connection to `host:port`. The socket
/// carries the caller's timeouts; the handshake is driven to completion here
/// so verification failures surface as typed errors before any request is
/// written.
pub fn https_stream(
    host: &str,
    port: u16,
    roots: &TlsRoots,
    timeout: Duration,
) -> Result<TlsStream, TlsError> {
    let config = client_config(roots)?;
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|error| TlsError::Config(format!("invalid server name {host}: {error}")))?;
    let connection = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|error| TlsError::Config(error.to_string()))?;

    let socket = connect_socket(host, port, timeout)?;
    let mut stream = rustls::StreamOwned::new(connection, socket);
    let deadline = Instant::now() + timeout;
    while stream.conn.is_handshaking() {
        match stream.conn.complete_io(&mut stream.sock) {
            Ok(_) => {}
            Err(error) if is_would_block(&error) => {
                if Instant::now() >= deadline {
                    return Err(TlsError::Handshake(format!(
                        "handshake timed out after {} ms",
                        timeout.as_millis()
                    )));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(classify_io_error(&error)),
        }
    }
    Ok(TlsStream(stream))
}

fn connect_socket(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, TlsError> {
    let address = (host, port);
    let mut last_error: Option<std::io::Error> = None;
    for candidate in address
        .to_socket_addrs()
        .map_err(|error| TlsError::Handshake(format!("could not resolve {host}: {error}")))?
    {
        match TcpStream::connect_timeout(&candidate, timeout) {
            Ok(socket) => {
                let _ = socket.set_read_timeout(Some(timeout));
                let _ = socket.set_write_timeout(Some(timeout));
                return Ok(socket);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(TlsError::Handshake(format!(
        "connect to {host}:{port} failed: {}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no addresses".to_string())
    )))
}

fn is_would_block(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock || error.kind() == std::io::ErrorKind::TimedOut
}

/// Maps a rustls-layer I/O failure onto the typed error surface: certificate
/// problems are verification failures (fail closed, never retry); everything
/// else is a plain handshake failure.
fn classify_io_error(error: &std::io::Error) -> TlsError {
    let rustls_error = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>());
    match rustls_error {
        Some(rustls::Error::InvalidCertificate(reason)) => {
            TlsError::Verification(format!("{reason:?}"))
        }
        Some(other) => TlsError::Handshake(other.to_string()),
        None => TlsError::Handshake(error.to_string()),
    }
}

/// Whether a read over the TLS stream hit the peer's TCP EOF without a
/// close_notify alert. rustls surfaces that as `UnexpectedEof`; servers that
/// close abruptly (still common for `Connection: close` HTTP/1.1) must not be
/// reported as read failures when the body is already complete.
pub fn is_peer_closed_without_close_notify(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::UnexpectedEof && error.to_string().contains("close_notify")
}

fn client_config(roots: &TlsRoots) -> Result<rustls::ClientConfig, TlsError> {
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| TlsError::Config(error.to_string()))?;
    let mut root_store = rustls::RootCertStore::empty();
    match roots {
        TlsRoots::WebPki => {
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        TlsRoots::Custom(certificates) => {
            for certificate in certificates {
                let certificate = CertificateDer::from(certificate.clone());
                root_store
                    .add(certificate)
                    .map_err(|error| TlsError::Config(error.to_string()))?;
            }
        }
    }
    Ok(builder
        .with_root_certificates(root_store)
        .with_no_client_auth())
}

/// One parsed outbound URL: scheme is `http` (plain) or `https` (TLS); every
/// other scheme fails closed. `Err` carries the human-readable rejection the
/// transports wrap into their own `UnsupportedUrl` errors.
pub struct OutboundUrl {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn parse_outbound_url(url: &str) -> Result<OutboundUrl, String> {
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(format!("outbound url has no scheme: {url}"));
    };
    let secure = match scheme.to_ascii_lowercase().as_str() {
        "http" => false,
        "https" => true,
        other => {
            return Err(format!(
                "unsupported outbound url scheme: {other}:// (only http:// and https:// are supported)"
            ));
        }
    };
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("outbound url has an invalid port: {url}"))?;
            (host.to_string(), port)
        }
        None => (authority.to_string(), if secure { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(format!("outbound url has no host: {url}"));
    }
    Ok(OutboundUrl {
        secure,
        host,
        port,
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parsing_accepts_http_and_https_only() {
        let parsed = parse_outbound_url("http://127.0.0.1:9/x").unwrap();
        assert!(!parsed.secure);
        assert_eq!(
            (parsed.host.as_str(), parsed.port, parsed.path.as_str()),
            ("127.0.0.1", 9, "/x")
        );

        let parsed = parse_outbound_url("https://huggingface.co/a/b").unwrap();
        assert!(parsed.secure);
        assert_eq!(parsed.port, 443);
        assert_eq!(parsed.path, "/a/b");

        assert!(parse_outbound_url("ftp://example.invalid/x").is_err());
        assert!(parse_outbound_url("example.invalid/x").is_err());
        assert!(parse_outbound_url("http://host:notaport/x").is_err());
        assert!(parse_outbound_url("http:///path-only").is_err());
    }

    #[test]
    fn verification_errors_are_typed_and_non_retryable() {
        let error = TlsError::Verification("UnknownIssuer".to_string());
        assert!(error.is_verification());
        assert!(!TlsError::Handshake("eof".to_string()).is_verification());
        assert!(error.to_string().contains("verification failed"));
    }
}
