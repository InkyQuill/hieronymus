//! Narrow System One protocol derived from zchee's decision-model-sdk.
//! Transport, credentials, retries and decision authority belong to the caller.
mod json;
mod request;
mod response;
pub use request::Request;

/// Maximum Jev response size, checked before decoding.
pub const MAX_RESPONSE_BYTES: usize = 65_536;

/// Protocol diagnostics never contain keys or provider/request text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Unsupported named-question envelope.
    InvalidRequest,
    /// Oversized, malformed or ambiguous JSON response.
    InvalidResponse,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "invalid decision request",
            Self::InvalidResponse => "invalid decision response",
        })
    }
}
impl std::error::Error for Error {}
