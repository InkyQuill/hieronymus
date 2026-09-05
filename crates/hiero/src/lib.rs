//! `hiero` runtime library: the local daemon (HTTP MCP transport, discovery,
//! credentials) and the `hiero mcp` stdio adapter support code. The `hiero`
//! binary is a thin CLI shell over these modules; domain storage primitives
//! live in the `hieronymus` library.

pub mod agent_hook;
pub mod app;
pub mod client;
pub mod daemon;
pub mod doctor;
pub mod service;
pub mod stdio;
pub mod uninstall;
pub mod update;
