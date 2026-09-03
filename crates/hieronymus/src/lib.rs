//! Hieronymus domain library.
//!
//! Owns typed configuration, secret handling, data-root resolution, and
//! storage primitives shared by the `hiero` binary, the daemon, and the MCP
//! transports. Presentation lives in the binary crate; this library must not
//! print, log, or touch the network.

pub mod atomic;
pub mod data_root;
pub mod dream_config;
pub mod ingest_config;
pub mod provider_config;
pub mod release_config;
pub mod secret;
