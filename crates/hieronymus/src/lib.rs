//! Hieronymus domain library.
//!
//! Owns typed configuration, secret handling, data-root resolution, and
//! storage primitives shared by the `hiero` binary, the daemon, and the MCP
//! transports. Presentation lives in the binary crate; this library must not
//! print, log, or touch the network.

pub mod atomic;
pub mod concept_models;
pub mod concepts;
pub mod data_root;
pub mod db;
pub mod dream_config;
pub mod ingest_config;
pub mod memory_models;
pub mod provider_config;
pub mod registry;
pub mod release_config;
pub mod secret;
pub mod short_memory;
pub mod workspace;
