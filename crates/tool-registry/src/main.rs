use axum::{Json, Router, routing::get, routing::post};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct ToolDescriptor {
    name: &'static str,
    description: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct McpRequest {
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "jattg_tool_registry=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9084".to_string());
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/tools/list", get(list_tools))
        .route("/mcp", post(mcp))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "tool registry listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn list_tools() -> Json<Vec<ToolDescriptor>> {
    Json(vec![
        ToolDescriptor {
            name: "repo_status",
            description: "Return repository and workspace status.",
        },
        ToolDescriptor {
            name: "test_command",
            description: "Run an approved test command in an isolated sandbox.",
        },
        ToolDescriptor {
            name: "artifact_manifest",
            description: "List artifacts produced by a task.",
        },
    ])
}

async fn mcp(Json(request): Json<McpRequest>) -> Json<serde_json::Value> {
    let result = match request.method.as_str() {
        "tools/list" => serde_json::json!({
            "tools": [
                {"name": "repo_status", "description": "Return repository and workspace status."},
                {"name": "test_command", "description": "Run an approved test command in an isolated sandbox."},
                {"name": "artifact_manifest", "description": "List artifacts produced by a task."}
            ]
        }),
        _ => serde_json::json!({
            "error": {
                "code": -32601,
                "message": format!("method not implemented: {}", request.method),
                "params": request.params
            }
        }),
    };

    Json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": request.id,
        "result": result
    }))
}
