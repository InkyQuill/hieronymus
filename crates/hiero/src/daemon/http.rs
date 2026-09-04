//! Minimal HTTP/1.1 plumbing for the loopback daemon: bounded request reads
//! and `Connection: close` responses. Behavior is ported from the qualified
//! transport reference (`qualification/harnesses/mcp-transport`): strict
//! request-line/header parsing, duplicate headers rejected, 1 MiB body cap.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;

pub(crate) const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;

pub(crate) const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
pub(crate) const SSE_CONTENT_TYPE: &str = "text/event-stream";

pub(crate) struct Request {
    pub method: String,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub(crate) struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(status: u16, body: &serde_json::Value) -> Response {
        Response {
            status,
            content_type: JSON_CONTENT_TYPE,
            body: serde_json::to_vec(body).unwrap_or_default(),
        }
    }

    pub fn sse(body: &serde_json::Value) -> Response {
        let data = serde_json::to_string(body).unwrap_or_default();
        Response {
            status: 200,
            content_type: SSE_CONTENT_TYPE,
            body: format!("event: message\ndata: {data}\n\n").into_bytes(),
        }
    }
}

pub(crate) fn header<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers.get(name).map(String::as_str)
}

/// Read one HTTP/1.1 request. Missing `Content-Length` means an empty body
/// (GET requests carry none); anything malformed is an error and the
/// connection is dropped without a response.
pub(crate) fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    let mut bytes = Vec::new();
    let mut temporary = [0_u8; 8192];
    let header_end = loop {
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP request exceeds size limit",
            ));
        }
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let count = stream.read(&mut temporary)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "HTTP headers are incomplete",
            ));
        }
        bytes.extend_from_slice(&temporary[..count]);
    };
    if header_end > MAX_HEADER_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "HTTP headers exceed size limit",
        ));
    }
    let header_text = std::str::from_utf8(&bytes[..header_end - 4])
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "headers not UTF-8"))?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "empty request"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_owned();
    let target = request_parts.next().unwrap_or_default().to_owned();
    if request_parts.next() != Some("HTTP/1.1") || request_parts.next().is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "HTTP request line is invalid",
        ));
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad header"))?;
        let normalized = name.trim().to_ascii_lowercase();
        if normalized.is_empty()
            || headers
                .insert(normalized, value.trim().to_owned())
                .is_some()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP header is duplicated or empty",
            ));
        }
    }
    let content_length: usize = match header(&headers, "content-length") {
        Some(value) => value
            .parse()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad length"))?,
        None => 0,
    };
    if content_length > MAX_REQUEST_BYTES || header_end + content_length > MAX_REQUEST_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "HTTP body exceeds size limit",
        ));
    }
    while bytes.len() < header_end + content_length {
        let count = stream.read(&mut temporary)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "HTTP body is incomplete",
            ));
        }
        bytes.extend_from_slice(&temporary[..count]);
    }
    if bytes.len() != header_end + content_length {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "trailing bytes",
        ));
    }
    Ok(Request {
        method,
        target,
        headers,
        body: bytes[header_end..].to_vec(),
    })
}

/// Write a response head and body, then leave the connection closing to the
/// caller (`Connection: close` is announced).
pub(crate) fn write_response(stream: &mut TcpStream, response: &Response) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_body_for_sse_has_request_scoped_framing() {
        let response = Response::sse(&serde_json::json!({"ok": true}));
        assert_eq!(response.content_type, SSE_CONTENT_TYPE);
        assert_eq!(response.body, b"event: message\ndata: {\"ok\":true}\n\n");
    }
}
