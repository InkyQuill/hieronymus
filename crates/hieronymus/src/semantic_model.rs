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
pub type DownloadProgress = std::sync::Arc<dyn Fn(&str, u64, Option<u64>) + Send + Sync>;

pub struct HttpModelTransport {
    timeout: Duration,
    tls_roots: TlsRoots,
    https_redirects: usize,
    progress: Option<DownloadProgress>,
}

impl HttpModelTransport {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            tls_roots: TlsRoots::default(),
            https_redirects: 0,
            progress: None,
        }
    }

    /// Opt into bounded HTTPS redirects for release assets. Model acquisition
    /// keeps its existing no-redirect policy unless explicitly enabled.
    pub fn with_https_redirects(mut self, maximum: usize) -> Self {
        self.https_redirects = maximum;
        self
    }

    pub fn with_progress(mut self, progress: Option<DownloadProgress>) -> Self {
        self.progress = progress;
        self
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
        let progress = |bytes, total| {
            if let Some(report) = &self.progress {
                report(url, bytes, total);
            }
        };
        progress(0, None);
        let mut current = url.to_owned();
        for hop in 0..=self.https_redirects {
            let parsed =
                tls::parse_outbound_url(&current).map_err(SemanticError::UnsupportedUrl)?;
            let mut stream: Box<dyn ReadWrite> = if parsed.secure {
                Box::new(
                    tls::https_stream(&parsed.host, parsed.port, &self.tls_roots, self.timeout)
                        .map_err(tls_error)?,
                )
            } else {
                Box::new(self.connect_plain(&parsed)?)
            };

            let request = format!(
                "GET {path} HTTP/1.1\r\nHost: {authority}\r\nAccept: */*\r\nUser-Agent: Hieronymus\r\nConnection: close\r\n\r\n",
                path = parsed.path,
                authority = parsed.http_authority(),
            );
            stream.write_all(request.as_bytes()).map_err(|error| {
                SemanticError::Download(format!("request write failed: {error}"))
            })?;

            let head = read_response_head(&mut stream)?;
            if matches!(head.status, 301 | 302 | 303 | 307 | 308) && hop < self.https_redirects {
                let location = head
                    .location
                    .ok_or_else(|| SemanticError::Download("redirect has no Location".into()))?;
                current = if location.starts_with('/') && !location.starts_with("//") {
                    format!("https://{}{}", parsed.http_authority(), location)
                } else {
                    location
                };
                let next =
                    tls::parse_outbound_url(&current).map_err(SemanticError::UnsupportedUrl)?;
                if !parsed.secure || !next.secure {
                    return Err(SemanticError::Download(
                        "release redirect must remain HTTPS".into(),
                    ));
                }
                continue;
            }
            if head.status != 200 {
                return Err(SemanticError::Download(format!(
                    "server answered with status {}",
                    head.status
                )));
            }
            if head.chunked {
                return stream_chunked_body_with_progress(
                    &mut stream,
                    destination,
                    &head.buffered,
                    max_bytes,
                    &progress,
                );
            }
            return stream_body(
                &mut stream,
                destination,
                &head.buffered,
                head.content_length,
                max_bytes,
                &progress,
            );
        }
        unreachable!("bounded redirect loop always returns")
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
struct DownloadHead {
    status: u16,
    location: Option<String>,
    content_length: Option<u64>,
    chunked: bool,
    buffered: Vec<u8>,
}
fn read_response_head(stream: &mut impl Read) -> Result<DownloadHead, SemanticError> {
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
    let mut content_length = None;
    let mut location = None;
    let mut chunked = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse::<u64>().ok();
        }
    }
    for line in head.split("\r\n").skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("location") {
                location = Some(value.trim().to_owned());
            }
            if name.eq_ignore_ascii_case("transfer-encoding") {
                if !value.trim().eq_ignore_ascii_case("chunked") {
                    return Err(SemanticError::Download(
                        "unsupported transfer encoding".into(),
                    ));
                }
                chunked = true;
            }
        }
    }
    if chunked && content_length.is_some() {
        return Err(SemanticError::Download(
            "ambiguous response body framing".into(),
        ));
    }
    Ok(DownloadHead {
        status,
        location,
        content_length,
        chunked,
        buffered: raw[separator + 4..].to_vec(),
    })
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
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<u64, SemanticError> {
    progress(0, content_length);
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
        progress(written, content_length);
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

#[cfg(test)]
fn stream_chunked_body(
    stream: &mut impl Read,
    destination: &Path,
    buffered: &[u8],
    max_bytes: u64,
) -> Result<u64, SemanticError> {
    stream_chunked_body_with_progress(stream, destination, buffered, max_bytes, &|_, _| {})
}

fn stream_chunked_body_with_progress(
    stream: &mut impl Read,
    destination: &Path,
    buffered: &[u8],
    max_bytes: u64,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<u64, SemanticError> {
    use std::io::BufRead;
    let mut input = std::io::BufReader::new(std::io::Cursor::new(buffered).chain(stream));
    let mut output = std::io::BufWriter::new(std::fs::File::create(destination)?);
    let mut total = 0u64;
    loop {
        let mut line = Vec::new();
        input.by_ref().take(129).read_until(b'\n', &mut line)?;
        if line.len() > 128 || !line.ends_with(b"\r\n") {
            return Err(SemanticError::Download("invalid chunk size line".into()));
        }
        let text = std::str::from_utf8(&line)
            .map_err(|_| SemanticError::Download("invalid chunk size".into()))?;
        let size = u64::from_str_radix(text.trim().split(';').next().unwrap_or(""), 16)
            .map_err(|_| SemanticError::Download("invalid chunk size".into()))?;
        if size == 0 {
            let mut trailer_bytes = 0usize;
            loop {
                let mut trailer = Vec::new();
                input.by_ref().take(8193).read_until(b'\n', &mut trailer)?;
                trailer_bytes += trailer.len();
                if trailer_bytes > 8192 || !trailer.ends_with(b"\r\n") {
                    return Err(SemanticError::Download(
                        "invalid or truncated chunk trailers".into(),
                    ));
                }
                if trailer == b"\r\n" {
                    break;
                }
                if !trailer.contains(&b':') {
                    return Err(SemanticError::Download("invalid chunk trailer".into()));
                }
            }
            break;
        }
        total = total
            .checked_add(size)
            .filter(|value| *value <= max_bytes)
            .ok_or_else(|| {
                SemanticError::Download(format!("download exceeded the {max_bytes} byte limit"))
            })?;
        let copied = std::io::copy(&mut input.by_ref().take(size), &mut output)?;
        let mut separator = [0; 2];
        input.read_exact(&mut separator)?;
        progress(total, None);
        if copied != size || separator != *b"\r\n" {
            return Err(SemanticError::Download("truncated chunked download".into()));
        }
    }
    output.flush()?;
    Ok(total)
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

#[cfg(test)]
mod chunk_tests {
    use super::*;
    #[test]
    fn download_progress_tracks_the_bytes_written_and_total() {
        let dir = tempfile::tempdir().unwrap();
        let observations = std::cell::RefCell::new(Vec::new());
        let body = vec![42; STREAM_BUFFER_BYTES * 2 + 7];
        let bytes = stream_body(
            &mut std::io::Cursor::new(&body),
            &dir.path().join("body"),
            &[],
            Some(body.len() as u64),
            body.len() as u64,
            &|n, total| observations.borrow_mut().push((n, total)),
        )
        .unwrap();
        assert_eq!(bytes, body.len() as u64);
        assert_eq!(observations.borrow().first(), Some(&(0, Some(bytes))));
        assert_eq!(observations.borrow().last(), Some(&(bytes, Some(bytes))));
        assert!(observations.borrow().windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(std::fs::read(dir.path().join("body")).unwrap(), body);
    }
    #[test]
    fn chunked_response_requires_complete_trailers() {
        let dir = tempfile::tempdir().unwrap();
        for truncated in [b"1\r\nx\r\n0\r\n".as_slice(), b"0\r\nX-Test: value\r\n"] {
            assert!(
                stream_chunked_body(
                    &mut std::io::Cursor::new(truncated),
                    &dir.path().join("body"),
                    &[],
                    10
                )
                .is_err()
            );
        }
        assert_eq!(
            stream_chunked_body(
                &mut std::io::Cursor::new(b"1\r\nx\r\n0\r\nX-Test: value\r\n\r\n"),
                &dir.path().join("body"),
                &[],
                10
            )
            .unwrap(),
            1
        );
    }
}
