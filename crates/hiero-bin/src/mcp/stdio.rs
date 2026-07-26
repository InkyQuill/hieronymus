use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hiero_core::config::HieronymusConfig;
use reqwest::Client;
use rmcp::ServiceExt;
use serde_json::{Value, json};

use crate::daemon::{AppState, loopback_http_client, read_auth_token};

use super::{
    McpBackend, McpServer, SANITIZED_TOOL_ERROR, SafeMcpError, StoreDreamRunner, StoreMcpBackend,
};

const TOKEN_HEADER: &str = "X-Hieronymus-Token";

#[derive(Clone)]
struct DaemonMcpBackend {
    client: Client,
    endpoint: String,
    token: Arc<str>,
}

impl DaemonMcpBackend {
    fn new(config: &HieronymusConfig) -> Result<Self> {
        let token = read_auth_token(&config.auth_token_path())
            .context("failed to read the daemon authentication token")?;
        Ok(Self {
            client: loopback_http_client()?,
            endpoint: format!("http://127.0.0.1:{}", config.resolve_port(None)),
            token,
        })
    }
}

#[async_trait]
impl McpBackend for DaemonMcpBackend {
    async fn call(&self, name: &str, arguments: Value) -> Result<Value> {
        let operation = name.strip_prefix("hieronymus_").unwrap_or(name);
        let response = self
            .client
            .post(format!("{}/api/mcp/{operation}", self.endpoint))
            .header(TOKEN_HEADER, self.token.as_ref())
            .json(&arguments)
            .send()
            .await
            .map_err(|error| {
                let message = if error.is_timeout() {
                    format!("Hieronymus daemon request timed out: {error}")
                } else {
                    format!("Hieronymus daemon is unavailable: {error}")
                };
                anyhow!(SafeMcpError(message))
            })?;
        let status = response.status();
        let payload = response.json::<Value>().await.map_err(|error| {
            anyhow!(SafeMcpError(format!(
                "Hieronymus daemon returned a malformed response: {error}"
            )))
        })?;
        if !status.is_success() {
            let detail = payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            return Err(anyhow!(SafeMcpError(format!(
                "Hieronymus daemon rejected the MCP request: {detail}"
            ))));
        }
        payload.get("result").cloned().ok_or_else(|| {
            anyhow!(SafeMcpError(
                "Hieronymus daemon returned a malformed response: missing result".into()
            ))
        })
    }
}

pub async fn run(config: &HieronymusConfig) -> Result<()> {
    let server = McpServer::new(Arc::new(DaemonMcpBackend::new(config)?))
        .serve(rmcp::transport::stdio())
        .await
        .context("failed to start stdio MCP transport")?;
    server
        .waiting()
        .await
        .map(|_| ())
        .context("stdio MCP transport failed")
}

pub async fn proxy_operation(
    State(state): State<AppState>,
    Path(operation): Path<String>,
    Json(arguments): Json<Value>,
) -> Response {
    let name = format!("hieronymus_{operation}");
    if !super::register_tools()
        .iter()
        .any(|tool| tool.name.as_ref() == name)
    {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown_mcp_operation"})),
        )
            .into_response();
    }
    let runner = Arc::new(
        StoreDreamRunner::new(state.pool.clone(), state.config.clone())
            .with_notifier(state.events.clone()),
    );
    let backend = StoreMcpBackend::new(state.pool, state.config)
        .with_dream_runner(runner)
        .with_notifier(state.events);
    match backend.call(&name, arguments).await {
        Ok(result) => Json(json!({"result": result})).into_response(),
        Err(error) => {
            let (status, message) = if let Some(safe) = error.downcast_ref::<SafeMcpError>() {
                (StatusCode::BAD_REQUEST, safe.0.clone())
            } else {
                tracing::error!(operation, error = ?error, "MCP compatibility operation failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    SANITIZED_TOOL_ERROR.to_owned(),
                )
            };
            (status, Json(json!({"error": message}))).into_response()
        }
    }
}
