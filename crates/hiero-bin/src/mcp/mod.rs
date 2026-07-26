mod backend;
pub mod http;
mod tools;

use std::{collections::HashMap, sync::Arc};

use rmcp::{
    ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, ContentBlock, ErrorCode, ErrorData, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
};
use serde_json::Value;

pub use backend::{DreamRunner, McpBackend, StoreDreamRunner, StoreMcpBackend};
pub use tools::{ToolContract, tool_catalog};

const SANITIZED_TOOL_ERROR: &str = "The tool could not complete the request.";

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct SafeMcpError(pub(crate) String);

#[derive(Clone)]
pub struct McpServer {
    backend: Arc<dyn McpBackend>,
    tools: Arc<Vec<Tool>>,
    by_name: Arc<HashMap<String, Tool>>,
}

impl McpServer {
    #[must_use]
    pub fn new(backend: Arc<dyn McpBackend>) -> Self {
        let tools = register_tools();
        let by_name = tools
            .iter()
            .map(|tool| (tool.name.to_string(), tool.clone()))
            .collect();
        Self {
            backend,
            tools: Arc::new(tools),
            by_name: Arc::new(by_name),
        }
    }
}

#[must_use]
pub fn register_tools() -> Vec<Tool> {
    tool_catalog()
        .into_iter()
        .map(|contract| {
            let schema = contract
                .input_schema
                .as_object()
                .cloned()
                .expect("tool input schema is always an object");
            Tool::new(contract.name, contract.description, schema)
        })
        .collect()
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("hieronymus", env!("CARGO_PKG_VERSION")))
            .with_instructions("Local-first literary translation memory")
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.by_name.get(name).cloned()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.by_name.contains_key(request.name.as_ref()) {
            return Err(ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown MCP tool: {}", request.name),
                None,
            ));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        match self.backend.call(request.name.as_ref(), arguments).await {
            Ok(value) => Ok(CallToolResult::structured(value)),
            Err(error) => {
                tracing::error!(
                    tool = request.name.as_ref(),
                    error = ?error,
                    "MCP tool backend failed"
                );
                let message = error
                    .downcast_ref::<SafeMcpError>()
                    .map_or(SANITIZED_TOOL_ERROR, |safe| safe.0.as_str());
                Ok(CallToolResult::error(vec![ContentBlock::text(message)]))
            }
        }
    }
}
