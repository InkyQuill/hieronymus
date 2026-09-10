use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{self, Write};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FramingCounts {
    objects: u32,
    newlines: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EvidenceReport<'a> {
    protocol_version: &'a str,
    transport: &'a str,
    request_sha256: String,
    response_sha256: String,
    request_content_type: &'a str,
    response_content_type: &'a str,
    registry_sha256: &'a str,
    stdout_framing: FramingCounts,
    exit_status: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentType {
    Json,
    JsonUtf8,
    Sse,
    Invalid,
}

impl ContentType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::JsonUtf8 => "application/json; charset=utf-8",
            Self::Sse => "text/event-stream",
            Self::Invalid => "invalid",
        }
    }
}

pub struct Evidence<'a> {
    pub protocol_version: &'a str,
    pub transport: &'a str,
    pub request: &'a [u8],
    pub response: &'a [u8],
    pub request_content_type: ContentType,
    pub response_content_type: ContentType,
    pub registry_sha256: &'a str,
    pub stdout_objects: u32,
    pub stdout_newlines: u32,
    pub exit_status: i32,
}

pub fn emit(evidence: Evidence<'_>) -> io::Result<()> {
    let report = EvidenceReport {
        protocol_version: evidence.protocol_version,
        transport: evidence.transport,
        request_sha256: digest(evidence.request),
        response_sha256: digest(evidence.response),
        request_content_type: evidence.request_content_type.as_str(),
        response_content_type: evidence.response_content_type.as_str(),
        registry_sha256: evidence.registry_sha256,
        stdout_framing: FramingCounts {
            objects: evidence.stdout_objects,
            newlines: evidence.stdout_newlines,
        },
        exit_status: evidence.exit_status,
    };
    let stderr = io::stderr();
    let mut lock = stderr.lock();
    serde_json::to_writer(&mut lock, &report)?;
    lock.write_all(b"\n")
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
