//! Embedding model acquisition: the pinned qualification model's constants, a
//! streaming download transport seam (loopback-testable, `http`/`https`), and
//! the temp-file + checksum + atomic-promotion acquisition flow.
//!
//! Acquisition runs only when a caller invokes it explicitly (the store's
//! `acquire_model` API); config load, store open, and doctor-style checks
//! never touch the network and never download anything. An incomplete or
//! incompatible model leaves FTS retrieval fully functional. `https://`
//! downloads run over rustls (see the `tls` module); other schemes fail
//! closed.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::semantic_error::SemanticError;
use crate::tls::{self, TlsError, TlsRoots};

/// Multilingual model pinned after the P2 public-MCP corpus comparison
/// (`docs/semantic-validation.md`); historical native records retain their old identity.
pub const MODEL_NAME: &str = "paraphrase-multilingual-MiniLM-L12-v2";
/// Immutable upstream revision of the pinned model file.
pub const MODEL_REVISION: &str = "e8f8c211226b894fcb81acc59f3b34ba3efd5f42";
/// SHA-256 of the pinned ONNX model file (hex).
pub const MODEL_SHA256: &str = "10f7a088420252b26caf819236ca2c9d2987afd0fc06fec7553b542a5655a05a";
/// Size of the pinned ONNX model file in bytes.
pub const MODEL_BYTES: u64 = 470_301_610;
/// Canonical download source of the pinned model file.
pub const DEFAULT_MODEL_URL: &str = "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/resolve/e8f8c211226b894fcb81acc59f3b34ba3efd5f42/onnx/model.onnx";

/// File name of the pinned tokenizer asset inside the model directory.
pub const TOKENIZER_FILE_NAME: &str = "tokenizer.json";
/// SHA-256 of the pinned tokenizer.json (hex). Same model revision as the
/// ONNX file above; the tokenizer identity in `semantic_tokenizer` embeds
/// this digest.
pub const TOKENIZER_SHA256: &str =
    "2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8";
/// Size of the pinned tokenizer.json in bytes.
pub const TOKENIZER_BYTES: u64 = 9_081_518;
/// Canonical download source of the pinned tokenizer asset.
pub const DEFAULT_TOKENIZER_URL: &str = "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/resolve/e8f8c211226b894fcb81acc59f3b34ba3efd5f42/tokenizer.json";

/// Read window used for checksum computation and network streaming.
const STREAM_BUFFER_BYTES: usize = 1024 * 1024;

/// Cheap availability verdict over the locally present model file. The full
/// SHA-256 verification runs only when the provider is loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelStatus {
    /// The model file is present with the expected size.
    Available,
    /// The model file exists but does not match the pinned artifact.
    Invalid(String),
    /// No model file has been acquired.
    Missing,
}

/// Outcome of a successful acquisition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAcquisition {
    pub path: PathBuf,
    pub bytes: u64,
    pub checksum: String,
}

/// The download transport seam. Implementations stream `url` into
/// `destination` (a fresh file) and return the number of bytes written; they
/// must never buffer more than `max_bytes` and must treat a short body as an
/// error. Loopback tests implement this in-process over `http://127.0.0.1`.
pub trait ModelTransport: Send + Sync {
    fn download_to(
        &self,
        url: &str,
        destination: &Path,
        max_bytes: u64,
    ) -> Result<u64, SemanticError>;
}

/// Production transport: one blocking streaming HTTP/1.1 GET per call
/// (`http://` plain, `https://` over rustls with the configured roots),
/// read-bounded by `max_bytes`.
pub struct HttpModelTransport {
    timeout: Duration,
    tls_roots: TlsRoots,
}

impl HttpModelTransport {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            tls_roots: TlsRoots::default(),
        }
    }

    /// Overrides the trust anchors (loopback tests inject the locally
    /// generated certificate; nothing else is trusted then).
    pub fn with_tls_roots(mut self, roots: TlsRoots) -> Self {
        self.tls_roots = roots;
        self
    }
}

impl ModelTransport for HttpModelTransport {
    fn download_to(
        &self,
        url: &str,
        destination: &Path,
        max_bytes: u64,
    ) -> Result<u64, SemanticError> {
        let parsed = tls::parse_outbound_url(url).map_err(SemanticError::UnsupportedUrl)?;
        let mut stream: Box<dyn ReadWrite> = if parsed.secure {
            Box::new(
                tls::https_stream(&parsed.host, parsed.port, &self.tls_roots, self.timeout)
                    .map_err(tls_error)?,
            )
        } else {
            Box::new(self.connect_plain(&parsed)?)
        };

        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
            path = parsed.path,
            host = parsed.host,
            port = parsed.port,
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|error| SemanticError::Download(format!("request write failed: {error}")))?;

        let (content_length, buffered) = read_response_head(&mut stream)?;
        stream_body(
            &mut stream,
            destination,
            &buffered,
            content_length,
            max_bytes,
        )
    }
}

impl HttpModelTransport {
    fn connect_plain(
        &self,
        parsed: &tls::OutboundUrl,
    ) -> Result<std::net::TcpStream, SemanticError> {
        use std::net::{TcpStream, ToSocketAddrs};
        let address = (parsed.host.as_str(), parsed.port)
            .to_socket_addrs()
            .map_err(|error| {
                SemanticError::Download(format!("could not resolve {}: {error}", parsed.host))
            })?
            .next()
            .ok_or_else(|| {
                SemanticError::Download(format!("host resolved to no addresses: {}", parsed.host))
            })?;
        let stream = TcpStream::connect_timeout(&address, self.timeout)
            .map_err(|error| SemanticError::Download(format!("connect failed: {error}")))?;
        let _ = stream.set_write_timeout(Some(self.timeout));
        let _ = stream.set_read_timeout(Some(self.timeout));
        Ok(stream)
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

/// TLS handshake/configuration failures are download failures; certificate
/// verification failures get their own typed variant (never retry, never
/// fall back to plain text).
fn tls_error(error: TlsError) -> SemanticError {
    match error {
        TlsError::Verification(reason) => SemanticError::Verification(reason),
        other => SemanticError::Download(other.to_string()),
    }
}

/// Reads the status line and headers, rejecting non-200 responses, and returns
/// the advertised `Content-Length` when present plus the body bytes already
/// buffered past the header separator (head and body can arrive in one TCP
/// segment, and discarding them would truncate the artifact).
fn read_response_head(stream: &mut impl Read) -> Result<(Option<u64>, Vec<u8>), SemanticError> {
    let mut raw: Vec<u8> = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    let separator = loop {
        if let Some(position) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
        let count = match stream.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if tls::is_peer_closed_without_close_notify(&error) => 0,
            Err(error) => return Err(SemanticError::Download(format!("read failed: {error}"))),
        };
        if count == 0 {
            return Err(SemanticError::Download(
                "response ended before the header separator".to_string(),
            ));
        }
        raw.extend_from_slice(&buffer[..count]);
        if raw.len() > 64 * 1024 {
            return Err(SemanticError::Download(
                "response head exceeded 64 KiB".to_string(),
            ));
        }
    };
    let head = String::from_utf8_lossy(&raw[..separator]);
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| SemanticError::Download("response has no status line".to_string()))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|status| status.parse().ok())
        .ok_or_else(|| {
            SemanticError::Download(format!(
                "response has an invalid status line: {status_line}"
            ))
        })?;
    if status != 200 {
        return Err(SemanticError::Download(format!(
            "server answered with status {status}"
        )));
    }
    let mut content_length = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse::<u64>().ok();
        }
    }
    Ok((content_length, raw[separator + 4..].to_vec()))
}

/// Streams the body to `destination`, starting from the bytes buffered with
/// the response head, refusing to buffer more than `max_bytes`, and treating
/// a short body as a truncated download.
fn stream_body(
    stream: &mut impl Read,
    destination: &Path,
    buffered: &[u8],
    content_length: Option<u64>,
    max_bytes: u64,
) -> Result<u64, SemanticError> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(destination)?);
    let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
    let mut written: u64 = 0;
    let mut buffered = buffered;
    loop {
        let count = if buffered.is_empty() {
            match stream.read(&mut buffer) {
                Ok(count) => count,
                // A TLS peer that closes TCP without close_notify after a
                // complete body is an EOF, not a read failure.
                Err(error) if tls::is_peer_closed_without_close_notify(&error) => 0,
                Err(error) => {
                    return Err(SemanticError::Download(format!(
                        "body read failed: {error}"
                    )));
                }
            }
        } else {
            let take = buffered.len().min(buffer.len());
            buffer[..take].copy_from_slice(&buffered[..take]);
            buffered = &buffered[take..];
            take
        };
        if count == 0 {
            break;
        }
        written += count as u64;
        if written > max_bytes {
            return Err(SemanticError::Download(format!(
                "download exceeded the {max_bytes} byte limit"
            )));
        }
        file.write_all(&buffer[..count])?;
    }
    file.flush()?;
    if let Some(content_length) = content_length
        && written != content_length
    {
        return Err(SemanticError::Download(format!(
            "truncated download: server advertised {content_length} bytes but sent {written}"
        )));
    }
    Ok(written)
}

/// Streams the SHA-256 of the file at `path` with a bounded read window.
pub fn sha256_file(path: &Path) -> Result<String, SemanticError> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
