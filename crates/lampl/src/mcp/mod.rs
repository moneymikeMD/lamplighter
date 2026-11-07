// SPDX-License-Identifier: Apache-2.0

//! `lampl mcp`: the memory tools over MCP stdio. Schemas are built from the `lampl-core` enums and every tool body calls the same code as the CLI, so a failure is an MCP tool error, never an empty result.

mod tools;

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, ServiceExt, transport};
use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Memory;

impl ServerHandler for Memory {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("lampl", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: tools::definitions(),
            ..ListToolsResult::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.to_string();
        let args = Value::Object(request.arguments.unwrap_or_default());
        let outcome = tokio::task::spawn_blocking(move || tools::call(&name, args))
            .await
            .map_err(|e| ErrorData::internal_error(format!("tool task failed: {e}"), None))?;
        Ok(match outcome {
            Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
            Err(msg) => CallToolResult::error(vec![ContentBlock::text(msg)]),
        }
        .into())
    }
}

pub(crate) fn run() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("starting runtime: {e}"))?;
    runtime.block_on(async {
        let service = Memory
            .serve(transport::stdio())
            .await
            .map_err(|e| format!("mcp handshake failed: {e}"))?;
        service
            .waiting()
            .await
            .map_err(|e| format!("mcp session ended abnormally: {e}"))?;
        Ok(())
    })
}

pub(crate) fn schema_arc(v: Value) -> Arc<serde_json::Map<String, Value>> {
    match v {
        Value::Object(m) => Arc::new(m),
        other => unreachable!("tool schemas are JSON objects, got {other}"),
    }
}
