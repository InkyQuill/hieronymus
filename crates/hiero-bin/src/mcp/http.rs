use std::sync::Arc;

use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};

use super::{McpBackend, McpServer};

pub type HieronymusHttpService = StreamableHttpService<McpServer, LocalSessionManager>;

#[must_use]
pub fn service(backend: Arc<dyn McpBackend>) -> HieronymusHttpService {
    let server = McpServer::new(backend);
    StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_stateful_mode(true)
            .with_json_response(true)
            .with_sse_keep_alive(None),
    )
}
