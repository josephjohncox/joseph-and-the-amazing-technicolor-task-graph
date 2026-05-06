use std::{path::PathBuf, process::Command, sync::Arc};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;

#[derive(Debug, Clone)]
struct AppState {
    workspace_root: PathBuf,
    auth_token: Option<String>,
}

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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "coat_tool_registry=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9084".to_string());
    let workspace_root = std::env::var("TOOL_REGISTRY_WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?)
        .canonicalize()?;
    let state = Arc::new(AppState {
        workspace_root,
        auth_token: std::env::var("MCP_TOOL_TOKEN")
            .ok()
            .filter(|token| !token.is_empty() && token != "replace-me"),
    });
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/tools/list", get(list_tools))
        .route("/mcp", post(mcp))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "tool registry listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn tool_descriptors() -> Vec<ToolDescriptor> {
    vec![
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
    ]
}

async fn list_tools() -> Json<Vec<ToolDescriptor>> {
    Json(tool_descriptors())
}

async fn mcp(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<McpRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(error) = authorize(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(mcp_error(
                request.id,
                -32001,
                format!("unauthorized MCP request: {error}"),
            )),
        );
    }

    match request.method.as_str() {
        "tools/list" => (
            StatusCode::OK,
            Json(mcp_success(
                request.id,
                serde_json::json!({ "tools": mcp_tools() }),
            )),
        ),
        "tools/call" => {
            let params = serde_json::from_value::<ToolCallParams>(request.params.clone());
            match params {
                Ok(params) => match call_tool(&state, params).await {
                    Ok(result) => (StatusCode::OK, Json(mcp_success(request.id, result))),
                    Err(error) => (
                        StatusCode::OK,
                        Json(mcp_success(
                            request.id,
                            tool_error(format!("tool call failed: {error}")),
                        )),
                    ),
                },
                Err(error) => (
                    StatusCode::OK,
                    Json(mcp_error(
                        request.id,
                        -32602,
                        format!("invalid tools/call params: {error}"),
                    )),
                ),
            }
        }
        _ => (
            StatusCode::OK,
            Json(mcp_error(
                request.id,
                -32601,
                format!("method not implemented: {}", request.method),
            )),
        ),
    }
}

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), String> {
    let Some(expected) = &state.auth_token else {
        return Ok(());
    };

    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "missing authorization header".to_string())?;
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .ok_or_else(|| "authorization must use bearer token".to_string())?;

    if token == expected {
        Ok(())
    } else {
        Err("bearer token mismatch".to_string())
    }
}

fn mcp_tools() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "repo_status",
            "description": "Return repository status for a path under TOOL_REGISTRY_WORKSPACE_ROOT.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo_path": {"type": "string"}
                }
            }
        }),
        serde_json::json!({
            "name": "test_command",
            "description": "Report how test execution should be routed through the sandbox runner.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                }
            }
        }),
        serde_json::json!({
            "name": "artifact_manifest",
            "description": "Return the known artifact manifest placeholder for a task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"}
                }
            }
        }),
    ]
}

async fn call_tool(state: &AppState, params: ToolCallParams) -> anyhow::Result<serde_json::Value> {
    match params.name.as_str() {
        "repo_status" => repo_status(state, &params.arguments),
        "test_command" => Ok(test_command(&params.arguments)),
        "artifact_manifest" => Ok(artifact_manifest(&params.arguments)),
        other => anyhow::bail!("unknown tool: {other}"),
    }
}

fn repo_status(
    state: &AppState,
    arguments: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let repo = resolve_repo_path(state, arguments)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .arg("status")
        .arg("--short")
        .output()?;
    Ok(serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": String::from_utf8_lossy(&output.stdout).to_string()
            }
        ],
        "structuredContent": {
            "repo_path": repo,
            "success": output.status.success(),
            "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
            "stderr": String::from_utf8_lossy(&output.stderr).to_string()
        },
        "isError": !output.status.success()
    }))
}

fn resolve_repo_path(state: &AppState, arguments: &serde_json::Value) -> anyhow::Result<PathBuf> {
    let requested = arguments
        .get("repo_path")
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| state.workspace_root.clone());
    let candidate = if requested.is_absolute() {
        requested
    } else {
        state.workspace_root.join(requested)
    };
    let canonical = candidate.canonicalize()?;
    if !canonical.starts_with(&state.workspace_root) {
        anyhow::bail!(
            "repo_path {} is outside workspace root {}",
            canonical.display(),
            state.workspace_root.display()
        );
    }
    Ok(canonical)
}

fn test_command(arguments: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": "test execution is delegated to sandbox-runner; this MCP tool does not execute commands directly"
            }
        ],
        "structuredContent": {
            "requested_command": arguments.get("command").and_then(|value| value.as_str()),
            "status": "blocked",
            "next_service": "sandbox-runner"
        },
        "isError": false
    })
}

fn artifact_manifest(arguments: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": "artifact manifest placeholder"
            }
        ],
        "structuredContent": {
            "task_id": arguments.get("task_id").and_then(|value| value.as_str()),
            "artifacts": []
        },
        "isError": false
    })
}

fn tool_error(message: String) -> serde_json::Value {
    serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": message
            }
        ],
        "structuredContent": {},
        "isError": true
    })
}

fn mcp_success(id: Option<serde_json::Value>, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn mcp_error(id: Option<serde_json::Value>, code: i64, message: String) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};

    use super::{AppState, authorize, resolve_repo_path, test_command};

    #[test]
    fn bearer_auth_is_required_when_token_is_configured() {
        let state = AppState {
            workspace_root: std::env::current_dir().expect("cwd"),
            auth_token: Some("secret".to_string()),
        };
        let headers = HeaderMap::new();
        assert!(authorize(&state, &headers).is_err());

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer secret"));
        assert!(authorize(&state, &headers).is_ok());
    }

    #[test]
    fn repo_path_is_confined_to_workspace_root() {
        let state = AppState {
            workspace_root: std::env::current_dir()
                .expect("cwd")
                .canonicalize()
                .expect("canonical cwd"),
            auth_token: None,
        };
        let inside = resolve_repo_path(&state, &serde_json::json!({})).expect("inside path");
        assert!(inside.starts_with(&state.workspace_root));

        let outside = resolve_repo_path(&state, &serde_json::json!({ "repo_path": "/" }));
        assert!(outside.is_err());
    }

    #[test]
    fn test_command_reports_sandbox_delegation() {
        let result = test_command(&serde_json::json!({ "command": "cargo test" }));
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["status"], "blocked");
        assert_eq!(
            result["structuredContent"]["next_service"],
            "sandbox-runner"
        );
    }
}
