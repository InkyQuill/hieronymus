//! The admin websocket (`GET /ws/admin`, ADR 0012 as amended 2026-09-03):
//! the session cookie is checked at upgrade, with `Host`/`Origin` validation
//! and the CSRF layer waived. Guards follow the frozen fixture order:
//! invalid-host → session → Origin → upgrade headers.
//!
//! The server side is the minimal hand-rolled RFC 6455 subset the Python
//! reference used (keeps the std-threaded server model, no async runtime):
//! unmasked JSON text frames out, a bounded drain of masked client frames in.
//! The client's first text frame may be `{"resume_from_event_id": N}` — the
//! logical resume request the frozen fixture records as the upgrade body;
//! see [`super::events`] for the replay and snapshot-refresh rules. Any other
//! client frame is drained and discarded, and a close frame (or EOF) ends the
//! session and unsubscribes it from the event hub.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use serde_json::Value;
use sha1::{Digest, Sha1};

use super::DaemonRuntime;
use super::events::{AdminEvent, Subscriber};
use super::http::{Request, header};
use super::rest;
use super::server::Dispatch;

/// The fixed RFC 6455 handshake GUID.
const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// How often the frame loop wakes (also the shutdown-poll granularity,
/// mirroring the Python reference's 1s socket timeout).
const FRAME_POLL: Duration = Duration::from_secs(1);

const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Client frames larger than this are a protocol violation; the only
/// legitimate client frame is the tiny resume request.
const MAX_CLIENT_FRAME_BYTES: usize = super::http::MAX_REQUEST_BYTES;

/// A completed upgrade: guards passed, accept key computed.
pub(crate) struct Session {
    accept: String,
}

/// Route-table entry point: the guard chain plus the upgrade-header check.
/// Guard failures are plain JSON responses; success is a connection the
/// caller hands to [`Session::serve`].
pub(crate) fn handle(request: &Request, runtime: &DaemonRuntime) -> Dispatch {
    if !rest::host_is_valid(request, runtime) {
        return Dispatch::Respond(rest::invalid_host());
    }
    let authorized = rest::presented_session(request)
        .is_some_and(|session| runtime.sessions.session_is_valid(&session));
    if !authorized {
        return Dispatch::Respond(rest::unauthorized());
    }
    if !rest::origin_is_valid(request, runtime) {
        return Dispatch::Respond(rest::forbidden_origin());
    }
    let is_upgrade = header(&request.headers, "upgrade")
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    let key = header(&request.headers, "sec-websocket-key")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if !is_upgrade || key.is_none() {
        // Python reference behavior for a browser context without a
        // websocket handshake.
        return Dispatch::Respond(rest::websocket_upgrade_required());
    }
    Dispatch::Upgrade(Session {
        accept: accept_key(key.unwrap()),
    })
}

/// `Sec-WebSocket-Accept` = base64(SHA-1(key + the RFC 6455 GUID)).
fn accept_key(client_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(WEBSOCKET_GUID.as_bytes());
    let digest = hasher.finalize();
    base64::engine::general_purpose::STANDARD.encode(digest)
}

impl Session {
    /// Run the websocket session on the upgraded connection until the client
    /// disconnects (or the daemon stops), then unsubscribe from the hub.
    pub(crate) fn serve(self, stream: TcpStream, runtime: &DaemonRuntime) {
        let _ = stream.set_read_timeout(Some(FRAME_POLL));
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let mut writer = match stream.try_clone() {
            Ok(writer) => writer,
            Err(_) => return,
        };
        if write_upgrade_head(&mut writer, &self.accept).is_err() {
            return;
        }
        let sender: Subscriber = {
            let writer = Mutex::new(writer);
            Arc::new(move |event: &AdminEvent| {
                let body = serde_json::to_vec(&event.to_wire()).unwrap_or_default();
                let mut writer = writer.lock().unwrap_or_else(PoisonError::into_inner);
                // A write failure reports the subscriber as dead; the hub
                // drops it (the Python remove-on-exception port).
                write_text_frame(&mut *writer, &body).is_ok()
            })
        };
        let token = runtime.events.subscribe(Arc::clone(&sender));
        let mut frames = FrameReader::new(stream);
        let mut resume_window_open = true;
        loop {
            if runtime.stop.load(Ordering::Acquire) {
                break;
            }
            match frames.read_frame() {
                ReadOutcome::Frame(ClientFrame::Text(payload)) => {
                    if resume_window_open {
                        resume_window_open = false;
                        if let Some(resume_from) = parse_resume_request(&payload) {
                            runtime.events.replay_after(resume_from, &sender);
                        }
                    }
                }
                ReadOutcome::Frame(ClientFrame::Drain) => {}
                ReadOutcome::Frame(ClientFrame::Close)
                | ReadOutcome::PeerClosed
                | ReadOutcome::Invalid => break,
                ReadOutcome::TimedOut => continue,
            }
        }
        runtime.events.unsubscribe(token);
    }
}

/// Write the 101 handshake head: the frozen header set plus the accept key.
fn write_upgrade_head(writer: &mut TcpStream, accept: &str) -> std::io::Result<()> {
    writer.write_all(
        format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Connection: Upgrade\r\n\
             Upgrade: websocket\r\n\
             Sec-WebSocket-Accept: {accept}\r\n\
             \r\n"
        )
        .as_bytes(),
    )?;
    writer.flush()
}

/// One unmasked server text frame: FIN + opcode 0x1, then the payload length
/// (7 bits, 16-bit, or 64-bit form), then the payload.
fn write_text_frame<W: Write>(writer: &mut W, payload: &[u8]) -> std::io::Result<()> {
    let mut head = [0_u8; 10];
    head[0] = 0x81;
    let head_length = if payload.len() < 126 {
        head[1] = payload.len() as u8;
        2
    } else if payload.len() <= 0xFFFF {
        head[1] = 126;
        head[2..4].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        4
    } else {
        head[1] = 127;
        head[2..10].copy_from_slice(&(payload.len() as u64).to_be_bytes());
        10
    };
    writer.write_all(&head[..head_length])?;
    writer.write_all(payload)?;
    writer.flush()
}

/// The resume point in `{"resume_from_event_id": N}`; `None` for any other
/// first frame (which is then simply drained).
fn parse_resume_request(payload: &[u8]) -> Option<u64> {
    serde_json::from_slice::<Value>(payload)
        .ok()?
        .get("resume_from_event_id")?
        .as_u64()
}

#[derive(Debug)]
enum ClientFrame {
    /// A text frame payload (the resume frame is parsed, the rest drained).
    Text(Vec<u8>),
    /// Any other frame: drained and discarded (Python reference behavior).
    Drain,
    Close,
}

#[derive(Debug)]
enum ReadOutcome {
    Frame(ClientFrame),
    PeerClosed,
    /// The read timeout elapsed; the loop retries.
    TimedOut,
    /// Protocol violation or IO error: end the session.
    Invalid,
}

/// A buffered bounded reader for client frames; timeouts never lose
/// partially read bytes, and frames are capped instead of growing reads
/// without bound.
struct FrameReader {
    stream: TcpStream,
    buffer: Vec<u8>,
}

impl FrameReader {
    fn new(stream: TcpStream) -> FrameReader {
        FrameReader {
            stream,
            buffer: Vec::new(),
        }
    }

    fn read_frame(&mut self) -> ReadOutcome {
        match self.read_header_payload() {
            Ok(Some(frame)) => ReadOutcome::Frame(frame),
            Ok(None) => ReadOutcome::PeerClosed,
            Err(ReadError::TimedOut) => ReadOutcome::TimedOut,
            Err(ReadError::Fatal) => ReadOutcome::Invalid,
        }
    }

    /// `Ok(None)` when the peer closes before a frame is complete.
    fn read_header_payload(&mut self) -> Result<Option<ClientFrame>, ReadError> {
        if matches!(self.fill(2)?, Filled::Eof) {
            return Ok(None);
        }
        let opcode = self.buffer[0] & 0x0F;
        let masked = self.buffer[1] & 0x80 != 0;
        // Clients always mask (RFC 6455); anything else ends the session.
        if !masked {
            return Err(ReadError::Fatal);
        }
        let (payload_length, header_length) = match (self.buffer[1] & 0x7F) as usize {
            length @ 0..=125 => (length, 2),
            126 => {
                self.fill(4)?;
                (
                    u16::from_be_bytes([self.buffer[2], self.buffer[3]]) as usize,
                    4,
                )
            }
            _ => {
                self.fill(10)?;
                let mut bytes = [0_u8; 8];
                bytes.copy_from_slice(&self.buffer[2..10]);
                (u64::from_be_bytes(bytes) as usize, 10)
            }
        };
        if payload_length > MAX_CLIENT_FRAME_BYTES {
            return Err(ReadError::Fatal);
        }
        self.fill(header_length + 4 + payload_length)?;
        let mask = [
            self.buffer[header_length],
            self.buffer[header_length + 1],
            self.buffer[header_length + 2],
            self.buffer[header_length + 3],
        ];
        let start = header_length + 4;
        let payload: Vec<u8> = self.buffer[start..start + payload_length]
            .iter()
            .zip(mask.iter().cycle())
            .map(|(byte, key)| byte ^ key)
            .collect();
        self.buffer.drain(..start + payload_length);
        match opcode {
            0x8 => Ok(Some(ClientFrame::Close)),
            0x1 => Ok(Some(ClientFrame::Text(payload))),
            // Binary, ping, pong, continuation: drained and discarded.
            _ => Ok(Some(ClientFrame::Drain)),
        }
    }

    /// Grow the buffer to `needed` bytes; `Eof` means the peer closed before
    /// the buffer was full (buffered bytes survive timeouts).
    fn fill(&mut self, needed: usize) -> Result<Filled, ReadError> {
        let mut temporary = [0_u8; 4096];
        while self.buffer.len() < needed {
            match self.stream.read(&mut temporary) {
                Ok(0) => return Ok(Filled::Eof),
                Ok(count) => self.buffer.extend_from_slice(&temporary[..count]),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Err(ReadError::TimedOut);
                }
                // Like every connection-layer error in this daemon, a failed
                // read simply ends the session.
                Err(_) => return Err(ReadError::Fatal),
            }
        }
        Ok(Filled::Complete)
    }
}

enum Filled {
    Complete,
    Eof,
}

/// A failed frame read: either retry later (timeout) or end the session.
enum ReadError {
    TimedOut,
    Fatal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_key_matches_the_rfc_6455_example() {
        // RFC 6455 §1.3: key "dGhlIHNhbXBsZSBub25jZQ==" must produce exactly
        // "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=".
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).unwrap();
        let (server, _) = listener.accept().unwrap();
        // A stuck test must time out, not hang forever.
        server
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        (client, server)
    }

    /// Encode a masked client frame like a real browser would.
    fn masked_client_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [0xAA_u8, 0xBB, 0xCC, 0xDD];
        let mut frame = vec![0x80 | opcode];
        if payload.len() < 126 {
            frame.push(0x80 | payload.len() as u8);
        } else {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .zip(mask.iter().cycle())
                .map(|(byte, key)| byte ^ key),
        );
        frame
    }

    #[test]
    fn server_text_frames_encode_lengths_per_rfc_6455() {
        let mut small = Vec::new();
        write_text_frame(&mut small, b"hello").unwrap();
        assert_eq!(small, [0x81, 0x05, b'h', b'e', b'l', b'l', b'o']);

        let medium_payload = vec![b'x'; 300];
        let mut medium = Vec::new();
        write_text_frame(&mut medium, &medium_payload).unwrap();
        assert_eq!(&medium[..4], &[0x81, 126, 0x01, 0x2C]);

        let large_payload = vec![b'y'; 70_000];
        let mut large = Vec::new();
        write_text_frame(&mut large, &large_payload).unwrap();
        assert_eq!(&large[..2], &[0x81, 127]);
        assert_eq!(&large[2..10], &70_000_u64.to_be_bytes());
    }

    #[test]
    fn client_frames_are_unmasked_and_resume_frames_parse() {
        let (mut client, server) = socket_pair();
        client
            .write_all(&masked_client_frame(
                0x1,
                br#"{"resume_from_event_id": 41}"#,
            ))
            .unwrap();
        client
            .write_all(&masked_client_frame(0x2, b"binary is drained"))
            .unwrap();
        client.write_all(&masked_client_frame(0x8, b"")).unwrap();
        client.flush().unwrap();

        let mut reader = FrameReader::new(server);
        match reader.read_frame() {
            ReadOutcome::Frame(ClientFrame::Text(payload)) => {
                assert_eq!(payload, br#"{"resume_from_event_id": 41}"#);
                assert_eq!(parse_resume_request(&payload), Some(41));
            }
            other => panic!("expected the text frame, got {other:?}"),
        }
        assert!(matches!(
            reader.read_frame(),
            ReadOutcome::Frame(ClientFrame::Drain)
        ));
        assert!(matches!(
            reader.read_frame(),
            ReadOutcome::Frame(ClientFrame::Close)
        ));
    }

    #[test]
    fn unmasked_client_frames_are_a_protocol_violation() {
        let (mut client, server) = socket_pair();
        client.write_all(&[0x81, 0x02, b'h', b'i']).unwrap();
        client.flush().unwrap();
        let mut reader = FrameReader::new(server);
        assert!(matches!(reader.read_frame(), ReadOutcome::Invalid));
    }

    #[test]
    fn oversized_client_frames_are_a_protocol_violation() {
        let (mut client, server) = socket_pair();
        // A 64-bit length just above the frame cap: rejected before any
        // payload is read.
        let mut header = vec![0x81, 0x80 | 127];
        header.extend_from_slice(&(MAX_CLIENT_FRAME_BYTES as u64 + 1).to_be_bytes());
        client.write_all(&header).unwrap();
        client.flush().unwrap();
        let mut reader = FrameReader::new(server);
        assert!(matches!(reader.read_frame(), ReadOutcome::Invalid));
    }

    #[test]
    fn resume_frames_without_a_numeric_id_are_drained() {
        assert_eq!(
            parse_resume_request(br#"{"resume_from_event_id": 41}"#),
            Some(41)
        );
        assert_eq!(parse_resume_request(br#"{}"#), None);
        assert_eq!(
            parse_resume_request(br#"{"resume_from_event_id": "41"}"#),
            None
        );
        assert_eq!(parse_resume_request(b"not json"), None);
    }
}
