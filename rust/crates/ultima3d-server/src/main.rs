//! Ultima3D MCP server (Rust host). Mirrors ultima3d/server.py:
//! one server, four backends — procedural worker, live Blender GUI,
//! Godot editor bridge, Photoshop CS3 tools.
//!
//! The Blender geometry worker itself stays Python: it runs inside
//! Blender's embedded interpreter, which exposes no Rust API.

mod worker;

use std::env;
use std::path::PathBuf;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::CallToolResult,
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};

use worker::Worker;

fn env_path(name: &str, fallback: String) -> PathBuf {
    PathBuf::from(env::var(name).unwrap_or(fallback))
}

fn out_dir(sub: &str) -> PathBuf {
    let base = env_path(
        "ULTIMA3D_OUT",
        std::env::current_dir()
            .unwrap_or(PathBuf::from("."))
            .join("output")
            .to_string_lossy()
            .into_owned(),
    );
    let p = if sub.is_empty() { base } else { base.join(sub) };
    let _ = std::fs::create_dir_all(&p);
    p
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Create3dArgs {
    recipe: Map<String, Value>,
    #[serde(default = "default_name")]
    name: String,
    #[serde(default)]
    blueprint: Option<Map<String, Value>>,
    #[serde(default = "default_true")]
    join: bool,
}

fn default_name() -> String {
    "asset".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Clone)]
struct Ultima3d {
    tool_router: ToolRouter<Self>,
    worker: Worker,
}

#[tool_router]
impl Ultima3d {
    fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or(PathBuf::from("."));
        let blender = env_path(
            "ULTIMA3D_BLENDER",
            r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe".to_string(),
        );
        let worker_py = env::var("ULTIMA3D_WORKER")
            .map(PathBuf::from)
            .unwrap_or_else(|_| cwd.join("ultima3d").join("blender_worker.py"));
        let timeout: u64 = env::var("ULTIMA3D_CALL_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600);
        let worker = Worker::new(blender, worker_py, timeout);
        Self { tool_router: Self::tool_router(), worker }
    }

    fn err_text(e: anyhow::Error) -> Result<CallToolResult, McpError> {
        // Tool ran and failed: tool-level error content, not a protocol error,
        // so the model receives the message (mirrors ToolError in server.py).
        Ok(CallToolResult::error(vec![rmcp::model::ContentBlock::text(
            e.to_string(),
        )]))
    }

    #[tool(description = "Build a 3D asset in Blender from a parametric recipe and (optionally) a design blueprint.")]
    async fn create_3d(&self, Parameters(args): Parameters<Create3dArgs>) -> Result<CallToolResult, McpError> {
        let params = serde_json::json!({
            "recipe": args.recipe,
            "blueprint": args.blueprint,
            "name": args.name,
            "join": args.join,
        });
        match self.worker.call("build", params).await {
            Ok(r) => Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                r.to_string(),
            )])),
            Err(e) => Self::err_text(e),
        }
    }

    #[tool(description = "Return machine-readable facts about the current asset: triangles, dimensions, materials, UVs, recipe.")]
    async fn inspect_asset(&self) -> Result<CallToolResult, McpError> {
        match self.worker.call("inspect", Value::Object(Default::default())).await {
            Ok(r) => Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                r.to_string(),
            )])),
            Err(e) => Self::err_text(e),
        }
    }

    #[tool(description = "Check the Blender worker: version, liveness, current scene state.")]
    async fn ultima3d_status(&self) -> Result<CallToolResult, McpError> {
        match self.worker.call("ping", Value::Object(Default::default())).await {
            Ok(ping) => {
                let body = serde_json::json!({"ok": true, "blender": ping.get("blender"), "worker": "running"});
                Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                    body.to_string(),
                )]))
            }
            Err(e) => Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::json!({"ok": false, "error": e.to_string()}).to_string(),
            )])),
        }
    }

    #[tool(description = "Clear the Blender scene and forget the current recipe/blueprint.")]
    async fn reset_scene(&self) -> Result<CallToolResult, McpError> {
        match self.worker.call("clear", Value::Object(Default::default())).await {
            Ok(r) => Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                r.to_string(),
            )])),
            Err(e) => Self::err_text(e),
        }
    }
}

#[tool_handler]
impl ServerHandler for Ultima3d {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = Ultima3d::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
