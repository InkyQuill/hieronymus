//! Minimal blocking HTTP/1.1 client for configured provider endpoints
//! (spec §Provider Policy of the dreaming design): configured timeouts,
//! bounded response reads, and `Connection: close` semantics, behind a
//! transport seam that loopback tests implement in-process. TLS is out of
//! scope for this slice; only `http://` endpoints are supported and anything
//! else fails closed.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Provider responses are size-limited before any parse (spec §Provider
/// Policy). The transport refuses to buffer more than this.
pub const MAX_PROVIDER_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// One provider HTTP response. Bodies are always valid UTF-8 with replacement
/// characters (the Python transport decoded with `errors="replace"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("network error: {0}")]
    Network(String),
    #[error("request timed out after {millis} ms")]
    Timeout { millis: u64 },
    #[error("provider response exceeded the {limit} byte limit")]
    TooLarge { limit: usize },
    #[error("{0}")]
    UnsupportedUrl(String),
}

impl HttpError {
    /// Retryable classification (spec: bounded retry for retryable failures):
    /// transport failures and timeouts retry; size violations and unusable
    /// URLs never will.
    pub fn is_retryable(&self) -> bool {
        matches!(self, HttpError::Network(_) | HttpError::Timeout { .. })
    }
}

/// The outbound transport seam. Implementations must honor `timeout` as a
/// whole-request deadline and must never buffer more than a bounded body.
pub trait ProviderTransport: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        payload: &Value,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError>;

    fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError>;
}

/// The production transport: one blocking TCP request per call, read-bounded
/// by `max_response_bytes`.
pub struct BlockingHttpTransport {
    max_response_bytes: usize,
}

impl BlockingHttpTransport {
    pub fn new(max_response_bytes: usize) -> Self {
        Self { max_response_bytes }
    }
}

impl Default for BlockingHttpTransport {
    fn default() -> Self {
        Self::new(MAX_PROVIDER_RESPONSE_BYTES)
    }
}

impl ProviderTransport for BlockingHttpTransport {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        payload: &Value,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        let body =
            serde_json::to_vec(payload).map_err(|error| HttpError::Network(error.to_string()))?;
        self.request("POST", url, headers, Some(&body), timeout)
    }

    fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        self.request("GET", url, headers, None, timeout)
    }
}

impl BlockingHttpTransport {
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        let deadline = Instant::now() + timeout;
        let (host, port, path) = parse_url(url)?;
        let address = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|error| HttpError::Network(format!("could not resolve {host}: {error}")))?
            .next()
            .ok_or_else(|| HttpError::Network(format!("host resolved to no addresses: {host}")))?;
        let mut stream = TcpStream::connect_timeout(&address, remaining(deadline, timeout)?)
            .map_err(|error| connect_error(error, timeout))?;
        let _ = stream.set_write_timeout(Some(timeout));
        let _ = stream.set_nodelay(true);

        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: application/json\r\nConnection: close\r\n"
        );
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        match body {
            Some(body) => {
                request.push_str(&format!(
                    "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                ));
                stream
                    .write_all(request.as_bytes())
                    .and_then(|()| stream.write_all(body))
            }
            None => {
                request.push_str("\r\n");
                stream.write_all(request.as_bytes())
            }
        }
        .map_err(|error| HttpError::Network(format!("request write failed: {error}")))?;
        let _ = stream.flush();

        let raw = self.read_bounded(&mut stream, deadline, timeout)?;
        parse_response(&raw)
    }

    /// Read until EOF or the deadline, buffering at most
    /// `max_response_bytes + 1` bytes so an oversized response is rejected
    /// before it can be parsed.
    fn read_bounded(
        &self,
        stream: &mut TcpStream,
        deadline: Instant,
        timeout: Duration,
    ) -> Result<Vec<u8>, HttpError> {
        let limit = self.max_response_bytes;
        let mut raw: Vec<u8> = Vec::with_capacity(8192);
        let mut buffer = [0_u8; 8192];
        loop {
            let remaining = remaining(deadline, timeout)?;
            let _ = stream.set_read_timeout(Some(remaining.min(Duration::from_millis(500))));
            match stream.read(&mut buffer) {
                Ok(0) => return Ok(raw),
                Ok(count) => {
                    raw.extend_from_slice(&buffer[..count]);
                    if raw.len() > limit {
                        return Err(HttpError::TooLarge { limit });
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => return Err(HttpError::Network(format!("read failed: {error}"))),
            }
        }
    }
}

fn remaining(deadline: Instant, timeout: Duration) -> Result<Duration, HttpError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(HttpError::Timeout {
            millis: timeout.as_millis() as u64,
        });
    }
    Ok(remaining)
}

fn connect_error(error: std::io::Error, timeout: Duration) -> HttpError {
    if error.kind() == std::io::ErrorKind::TimedOut {
        HttpError::Timeout {
            millis: timeout.as_millis() as u64,
        }
    } else {
        HttpError::Network(format!("connect failed: {error}"))
    }
}

/// Parse `scheme://host:port/path`. Only `http://` is supported; TLS is out
/// of scope for this slice and anything else fails closed without dialing.
fn parse_url(url: &str) -> Result<(String, u16, String), HttpError> {
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(HttpError::UnsupportedUrl(format!(
            "provider url has no scheme: {url}"
        )));
    };
    if !scheme.eq_ignore_ascii_case("http") {
        return Err(HttpError::UnsupportedUrl(format!(
            "unsupported provider url scheme: {scheme}:// (only http:// endpoints are supported)"
        )));
    }
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host.to_string(),
            port.parse::<u16>().map_err(|_| {
                HttpError::UnsupportedUrl(format!("provider url has an invalid port: {url}"))
            })?,
        ),
        None => (authority.to_string(), 80),
    };
    if host.is_empty() {
        return Err(HttpError::UnsupportedUrl(format!(
            "provider url has no host: {url}"
        )));
    }
    Ok((host, port, path))
}

fn parse_response(raw: &[u8]) -> Result<HttpResponse, HttpError> {
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| HttpError::Network("response has no header separator".to_string()))?;
    let head = std::str::from_utf8(&raw[..separator])
        .map_err(|_| HttpError::Network("response head is not UTF-8".to_string()))?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| HttpError::Network("response has no status line".to_string()))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|status| status.parse().ok())
        .ok_or_else(|| {
            HttpError::Network(format!(
                "response has an invalid status line: {status_line}"
            ))
        })?;
    let mut chunked = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }
    let body_bytes = &raw[separator + 4..];
    let body = if chunked {
        decode_chunked(body_bytes)?
    } else {
        body_bytes.to_vec()
    };
    Ok(HttpResponse {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn decode_chunked(mut bytes: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut decoded = Vec::with_capacity(bytes.len());
    loop {
        let line_end = find_line_end(bytes).ok_or_else(|| {
            HttpError::Network("chunked response ended mid-size-line".to_string())
        })?;
        let size_text = std::str::from_utf8(&bytes[..line_end])
            .map_err(|_| HttpError::Network("chunk size is not UTF-8".to_string()))?;
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| HttpError::Network(format!("invalid chunk size: {size_text}")))?;
        bytes = &bytes[line_end + 2..];
        if size == 0 {
            return Ok(decoded);
        }
        if bytes.len() < size + 2 {
            return Err(HttpError::Network(
                "chunked response ended mid-chunk".to_string(),
            ));
        }
        decoded.extend_from_slice(&bytes[..size]);
        bytes = &bytes[size + 2..];
    }
}

fn find_line_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|window| window == b"\r\n")
}
