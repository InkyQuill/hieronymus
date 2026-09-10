//! Minimal loopback HTTP/1.1 client used by the `hiero mcp` stdio adapter.
//! The daemon always answers with `Content-Length` and `Connection: close`,
//! which keeps this client small: no chunking, no TLS, no keep-alive.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("daemon is unreachable: {0}")]
    Connect(#[source] std::io::Error),
    #[error("daemon response is malformed: {0}")]
    Protocol(&'static str),
    #[error("daemon request failed: {0}")]
    Io(#[from] std::io::Error),
    /// No usable discovery record. Carries the record path or the reason,
    /// never a credential.
    #[error("no running local daemon was discovered: {0}")]
    NotDiscovered(String),
    /// The stored credential is missing or unusable. Carries the token path
    /// and the reason only — never the token itself.
    #[error("the local daemon credential is unavailable: {0}")]
    Credential(String),
    /// The daemon answered with a non-2xx status.
    #[error("daemon rejected the request: HTTP {status} {detail}")]
    Status { status: u16, detail: String },
}

/// One request's cancellation handle. It never represents a protocol session.
#[derive(Default)]
pub struct Cancellation {
    cancelled: std::sync::atomic::AtomicBool,
    stream: std::sync::Mutex<Option<TcpStream>>,
}
impl Cancellation {
    pub fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(stream) = self.stream.lock().unwrap().as_ref() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// POST a JSON body to `http://address/path`: the historical entry point,
/// kept as a thin wrapper over [`request_json`].
pub fn post_json(
    address: SocketAddr,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<(u16, Vec<u8>), ClientError> {
    request_json("POST", address, path, headers, body)
}

/// Send one request to `http://address/path` and return the status code and
/// response body. `method` is the HTTP verb (`GET`, `POST`); a body is sent
/// whenever one is supplied.
pub fn request_json(
    method: &str,
    address: SocketAddr,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<(u16, Vec<u8>), ClientError> {
    request_json_within(method, address, path, headers, body, IO_TIMEOUT)
}

/// [`request_json`] with an explicit socket deadline. Health probing uses a
/// short one: a listener that accepts and then says nothing must not stall an
/// interactive command for the MCP-sized default.
pub fn request_json_within(
    method: &str,
    address: SocketAddr,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
    timeout: Duration,
) -> Result<(u16, Vec<u8>), ClientError> {
    request_cancellable(method, address, path, headers, body, timeout, None)
}
pub(crate) fn request_cancellable(
    method: &str,
    address: SocketAddr,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
    timeout: Duration,
    cancellation: Option<&Cancellation>,
) -> Result<(u16, Vec<u8>), ClientError> {
    // The method goes into the request line: refuse anything that is not a
    // plain token before any I/O.
    if method.is_empty()
        || !method
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte == b'-')
    {
        return Err(ClientError::Protocol("request method is invalid"));
    }
    // Validate headers before any I/O so injection is refused regardless of
    // the daemon's state.
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        // Header values here are protocol identifiers and the bearer token;
        // they must not contain injection bytes.
        if value.contains('\r') || value.contains('\n') {
            return Err(ClientError::Protocol("header value contains CR/LF"));
        }
    }
    let connect_timeout = CONNECT_TIMEOUT.min(timeout);
    let mut stream =
        TcpStream::connect_timeout(&address, connect_timeout).map_err(ClientError::Connect)?;
    if let Some(cancellation) = cancellation {
        let mut socket = cancellation.stream.lock().unwrap();
        if cancellation.is_cancelled() {
            return Err(ClientError::Protocol("request cancelled"));
        }
        *socket = Some(stream.try_clone()?);
    }
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {}\r\n", address);
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    stream.write_all(request.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;

    let mut raw = Vec::new();
    let mut temporary = [0_u8; 8192];
    let header_end = loop {
        if raw.len() > MAX_RESPONSE_BYTES {
            return Err(ClientError::Protocol("response exceeds size limit"));
        }
        if let Some(position) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let count = stream.read(&mut temporary)?;
        if count == 0 {
            return Err(ClientError::Protocol("response headers are incomplete"));
        }
        raw.extend_from_slice(&temporary[..count]);
    };
    let head = std::str::from_utf8(&raw[..header_end - 4])
        .map_err(|_| ClientError::Protocol("response head is not UTF-8"))?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or(ClientError::Protocol("empty response"))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or(ClientError::Protocol("status line is invalid"))?;
    let mut content_length = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().ok();
        }
    }
    let body_start = raw[header_end..].to_vec();
    let body = match content_length {
        Some(length) => {
            if header_end + length > MAX_RESPONSE_BYTES {
                return Err(ClientError::Protocol("response body exceeds size limit"));
            }
            let mut body = body_start;
            while body.len() < length {
                let count = stream.read(&mut temporary)?;
                if count == 0 {
                    return Err(ClientError::Protocol("response body is incomplete"));
                }
                body.extend_from_slice(&temporary[..count]);
            }
            body.truncate(length);
            body
        }
        None => body_start,
    };
    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_closes_the_inflight_socket() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_rx_tx, accepted_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            accepted_rx_tx.send(()).unwrap();
            let mut data = [0; 1024];
            loop {
                match stream.read(&mut data) {
                    Ok(0) => return true,
                    Ok(_) => {}
                    Err(_) => return false,
                }
            }
        });
        let cancellation = std::sync::Arc::new(Cancellation::default());
        let worker_cancel = cancellation.clone();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            result_tx
                .send(request_cancellable(
                    "POST",
                    address,
                    "/mcp",
                    &[],
                    b"{}",
                    Duration::from_secs(2),
                    Some(&worker_cancel),
                ))
                .unwrap()
        });
        accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        cancellation.cancel();
        assert!(
            result_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_err()
        );
        worker.join().unwrap();
        assert!(server.join().unwrap());
    }

    #[test]
    fn header_injection_is_refused() {
        let address: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let error = post_json(
            address,
            "/mcp",
            &[("Mcp-Name".to_string(), "evil\r\nX: y".to_string())],
            b"{}",
        )
        .unwrap_err();
        assert!(error.to_string().contains("malformed"), "{error}");
    }

    #[test]
    fn method_injection_is_refused() {
        let address: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let error = request_json("GET /x HTTP/1.1\r\nX", address, "/status", &[], b"").unwrap_err();
        assert!(error.to_string().contains("malformed"), "{error}");
    }
}
