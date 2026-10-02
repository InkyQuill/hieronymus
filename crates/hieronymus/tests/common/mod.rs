//! Shared loopback TLS test helper: a locally generated self-signed
//! certificate (rcgen) plus a one-shot HTTPS server (rustls) bound to
//! 127.0.0.1. Tests never egress: the client must trust the injected
//! certificate explicitly through the transport seam's root configuration.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One loopback HTTPS endpoint serving a canned response to every request.
/// The presented certificate is returned so tests can inject it as a trusted
/// root (success case), inject a different root (wrong-root case), or leave
/// the client on the default webpki roots (untrusted case).
pub struct TlsLoopbackServer {
    pub port: u16,
    /// DER bytes of the presented certificate (rcgen-generated, self-signed).
    pub certificate_der: Vec<u8>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    accept_thread: Option<std::thread::JoinHandle<()>>,
}

impl TlsLoopbackServer {
    /// Starts the server. `names` are the certificate's subject alternative
    /// names (IP addresses allowed); requests whose names do not match fail
    /// client-side verification, which is exactly the negative case.
    pub fn start(names: &[&str], response: &'static str) -> Self {
        let certified = rcgen::generate_simple_self_signed(
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
        )
        .expect("self-signed certificate generation");
        let certificate_der = certified.cert.der().to_vec();
        let certified_key = rustls::sign::CertifiedKey::new(
            vec![certified.cert.der().clone()],
            rustls::crypto::ring::sign::any_supported_type(
                &rustls::pki_types::PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into()),
            )
            .expect("ring supports the generated key type"),
        );
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(FixedResolver(certified_key)));

        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let accept_thread = std::thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("nonblocking listener");
            loop {
                if thread_stop.load(Ordering::Acquire) {
                    break;
                }
                let Ok((socket, _peer)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                // Darwin propagates O_NONBLOCK from the listener to accepted
                // sockets. The per-connection TLS exchange is deliberately
                // blocking and bounded by its read/write timeouts.
                socket
                    .set_nonblocking(false)
                    .expect("blocking accepted socket");
                let _ = serve_once(socket, &config, response, &thread_requests);
            }
        });
        Self {
            port,
            certificate_der,
            requests,
            stop,
            accept_thread: Some(accept_thread),
        }
    }

    pub fn https_url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{port}{path}", port = self.port)
    }

    /// The complete raw requests the server received (after TLS decryption), so
    /// tests can assert the client actually spoke TLS.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for TlsLoopbackServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

/// Presents the same certificate to every handshake, whatever the client's
/// SNI: the tests address the server by IP, where SNI is meaningless.
#[derive(Debug)]
struct FixedResolver(rustls::sign::CertifiedKey);

impl rustls::server::ResolvesServerCert for FixedResolver {
    fn resolve(
        &self,
        _client_hello: rustls::server::ClientHello<'_>,
    ) -> Option<std::sync::Arc<rustls::sign::CertifiedKey>> {
        Some(std::sync::Arc::new(self.0.clone()))
    }
}

/// Handles exactly one request on `socket`: completes the TLS handshake,
/// reads the head and declared body, records them, answers with the canned
/// response, and closes the TLS connection properly (close_notify) so the
/// client's read-to-EOF loop terminates cleanly.
fn serve_once(
    socket: TcpStream,
    config: &rustls::ServerConfig,
    response: &str,
    requests: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    socket.set_nonblocking(false)?;
    let _ = socket.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = socket.set_write_timeout(Some(Duration::from_secs(10)));
    let connection = rustls::ServerConnection::new(Arc::new(config.clone()))
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut stream = rustls::StreamOwned::new(connection, socket);
    while stream.conn.is_handshaking() {
        stream
            .conn
            .complete_io(&mut stream.sock)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
    }

    let received = read_request(&mut stream)?;
    requests
        .lock()
        .unwrap()
        .push(String::from_utf8_lossy(&received).into_owned());

    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    stream.conn.send_close_notify();
    stream
        .conn
        .complete_io(&mut stream.sock)
        .map(|_| ())
        .map_err(|error| std::io::Error::other(error.to_string()))
}

/// Bounded HTTP/1 fixture reader; TLS reads may split the head and body.
pub(crate) fn read_request(reader: &mut impl Read) -> std::io::Result<Vec<u8>> {
    const MAX_HEADER: usize = 64 * 1024;
    const MAX_BODY: usize = 1024 * 1024;
    let invalid = |reason| std::io::Error::new(std::io::ErrorKind::InvalidData, reason);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut received = Vec::new();
    let mut expected = None;
    let mut buffer = [0_u8; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "request deadline",
            ));
        }
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "incomplete request",
            ));
        }
        received.extend_from_slice(&buffer[..count]);
        if received.len() > MAX_HEADER + MAX_BODY {
            return Err(invalid("request too large"));
        }
        if expected.is_none() {
            if let Some(end) = received
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|n| n + 4)
            {
                if end > MAX_HEADER {
                    return Err(invalid("header too large"));
                }
                let head =
                    std::str::from_utf8(&received[..end]).map_err(|_| invalid("invalid header"))?;
                let mut length = None;
                for line in head.split("\r\n").skip(1).filter(|s| !s.is_empty()) {
                    let (name, value) = line
                        .split_once(':')
                        .ok_or_else(|| invalid("invalid header field"))?;
                    if name.eq_ignore_ascii_case("transfer-encoding") {
                        return Err(invalid("transfer encoding unsupported by fixture"));
                    }
                    if name.eq_ignore_ascii_case("content-length") {
                        if length.is_some() {
                            return Err(invalid("duplicate content length"));
                        }
                        let size = value
                            .trim()
                            .parse::<usize>()
                            .map_err(|_| invalid("invalid content length"))?;
                        if size > MAX_BODY {
                            return Err(invalid("body too large"));
                        }
                        length = Some(size);
                    }
                }
                expected = Some(end + length.unwrap_or(0));
            } else if received.len() > MAX_HEADER {
                return Err(invalid("header too large"));
            }
        }
        if let Some(total) = expected.filter(|total| received.len() >= *total) {
            received.truncate(total);
            return Ok(received);
        }
    }
}
