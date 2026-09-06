//! `hiero` runtime library: the local daemon (HTTP MCP transport, discovery,
//! credentials) and the `hiero mcp` stdio adapter support code. The `hiero`
//! binary is a thin CLI shell over these modules; domain storage primitives
//! live in the `hieronymus` library.

pub mod agent_hook;
pub mod agent_plugins;
pub mod app;
pub mod application;
pub mod client;
pub mod console;
pub mod daemon;
pub mod daemon_client;
pub mod doctor;
pub mod export;
pub mod lifecycle;
pub mod service;
pub mod stdio;
pub mod uninstall;
pub mod update;
