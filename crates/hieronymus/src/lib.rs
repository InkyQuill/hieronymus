//! Hieronymus domain library.
//!
//! Owns typed configuration, secret handling, data-root resolution, and
//! storage primitives shared by the `hiero` binary, the daemon, and the MCP
//! transports. Presentation lives in the binary crate; this library must not
//! print or log. Its only outbound network access is the configured provider
//! client (`dream_providers` over `provider_http`), which stays behind a
//! transport seam so tests run on in-process loopback servers.

pub mod agent_context;
pub mod atomic;
pub mod concept_models;
pub mod concepts;
pub mod crystals;
pub mod data_root;
pub mod db;
pub mod dream_audit;
pub mod dream_config;
pub mod dream_link_progress;
pub mod dream_locks;
pub mod dream_output;
pub mod dream_providers;
pub mod dream_workflows;
pub mod dreaming;
pub mod feedback;
pub mod ingest_config;
pub mod memory_models;
pub mod migrate;
pub mod ownership;
pub mod provider_config;
pub mod provider_http;
pub mod rag;
pub mod rag_models;
pub mod recall;
pub mod registry;
pub mod release_config;
pub mod schema_upgrade;
pub mod secret;
pub mod semantic_arming;
pub mod semantic_embeddings;
pub mod semantic_error;
pub mod semantic_index;
pub mod semantic_jobs;
pub mod semantic_model;
pub mod semantic_recall;
pub mod semantic_store;
pub mod semantic_tokenizer;
pub mod short_memory;
pub mod state_classifier;
pub mod story_applicability;
pub mod terminology;
pub mod tls;
pub mod upgrade;
pub mod workspace;

pub mod authority;
mod authority_applicability;
mod authority_evidence;
pub mod authority_models;

pub mod claim_capture;
pub mod claim_reads;
pub mod coherent_reads;
pub mod corrections;
