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
}

/// POST a JSON body to `http://address/path` and return the status code and
/// response body.
pub fn post_json(
    address: SocketAddr,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<(u16, Vec<u8>), ClientError> {
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
    let mut stream =
        TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(ClientError::Connect)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;

    let mut request = format!("POST {path} HTTP/1.1\r\nHost: {}\r\n", address);
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
    let status_line = lines.next().ok_or(ClientError::Protocol("empty response"))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or(ClientError::Protocol("status line is invalid"))?;
    let mut content_length = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().ok();
            }
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
}
