//! Minimal MCP-facing Rust tool registry.
//!
//! Purpose: expose controlled tool surfaces such as repo status, sandboxed
//! command planning, and artifact manifest lookup. This service must not become
//! an arbitrary shell runner; execution is delegated to sandbox services.
//!
//! Architecture references:
//! - `docs/design-docs/010-distributed-runners-mcp.md`
//! - `docs/design-docs/100-strong-sandboxing-guardrails.md`
//! - `docs/exec-plans/active/070-sandbox-tooling.md`

use std::{
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use coat_domain::{
    AgentRunRequest, AgentRunResult, Budget, DoneCriteria, ReviewDoctrine, RunnerDispatchDecision,
    RunnerDispatchRequest, SandboxProfile, TaskNode, TaskPurpose, TaskStatus, WebSearchRequest,
    WebSearchResponse, WebSearchRoutingPreference, WebSearchStatus,
};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct AppState {
    workspace_root: PathBuf,
    sandbox_workspace_root: Option<PathBuf>,
    sandbox_runner_url: Option<String>,
    runner_registry_url: Option<String>,
    web_search_enabled: bool,
    web_search_route: WebSearchRoutingPreference,
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
    let sandbox_workspace_root = std::env::var("TOOL_REGISTRY_SANDBOX_WORKSPACE_ROOT")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let sandbox_runner_url = std::env::var("TOOL_REGISTRY_SANDBOX_RUNNER_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string());
    let runner_registry_url = env_first(&[
        "TOOL_REGISTRY_RUNNER_REGISTRY_URL",
        "COAT_RUNNER_REGISTRY_URL",
        "RUNNER_REGISTRY_URL",
    ])
    .map(|value| value.trim_end_matches('/').to_string());
    let web_search_enabled = env_truthy(&[
        "TOOL_REGISTRY_WEB_SEARCH_ENABLED",
        "COAT_WEB_SEARCH_ENABLED",
    ]);
    let web_search_route =
        match env_first(&["TOOL_REGISTRY_WEB_SEARCH_ROUTE", "COAT_WEB_SEARCH_ROUTE"])
            .unwrap_or_else(|| "coordinator_task".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "runner_registry" | "runner" | "dispatch" => WebSearchRoutingPreference::RunnerRegistry,
            "plan_only" | "plan" => WebSearchRoutingPreference::PlanOnly,
            _ => WebSearchRoutingPreference::CoordinatorTask,
        };
    let state = Arc::new(AppState {
        workspace_root,
        sandbox_workspace_root,
        sandbox_runner_url,
        runner_registry_url,
        web_search_enabled,
        web_search_route,
        auth_token: env_first(&["COAT_TOOL_REGISTRY_TOKEN", "MCP_TOOL_TOKEN"])
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

fn env_first(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn env_truthy(names: &[&str]) -> bool {
    env_first(names)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn tool_descriptors() -> Vec<ToolDescriptor> {
    vec![
        ToolDescriptor {
            name: "repo_status",
            description: "Return repository and workspace status.",
        },
        ToolDescriptor {
            name: "test_command",
            description: "Route an approved test command through the sandbox runner.",
        },
        ToolDescriptor {
            name: "local_command",
            description: "Plan or run an approved allowlisted local binary through the sandbox runner.",
        },
        ToolDescriptor {
            name: "artifact_manifest",
            description: "List artifacts produced by a task.",
        },
        ToolDescriptor {
            name: "checkpoint_history",
            description: "List checkpoint manifests and git/object refs for a task workspace.",
        },
        ToolDescriptor {
            name: "coat_web_search",
            description: "Route web/reference search through COAT research runners or durable child-task planning.",
        },
        ToolDescriptor {
            name: "subagent_policy",
            description: "Return COAT's durable subagent delegation policy for MCP clients.",
        },
    ]
}

async fn list_tools(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(error) = authorize(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": format!("unauthorized tool list request: {error}")
            })),
        );
    }

    (StatusCode::OK, Json(serde_json::json!(tool_descriptors())))
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
            "description": "Plan test execution through the sandbox runner when TOOL_REGISTRY_SANDBOX_RUNNER_URL is configured.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "workspace_id": {"type": "string"},
                    "goal_id": {"type": "string"},
                    "task_id": {"type": "string"},
                    "local_tools": {"type": "object"},
                    "approval_id": {"type": "string"}
                }
            }
        }),
        serde_json::json!({
            "name": "local_command",
            "description": "Plan or run an approved allowlisted local binary through the sandbox runner. The tool registry never runs the command in-process.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {
                        "oneOf": [
                            {"type": "string"},
                            {"type": "array", "items": {"type": "string"}}
                        ]
                    },
                    "workspace_id": {"type": "string"},
                    "goal_id": {"type": "string"},
                    "task_id": {"type": "string"},
                    "local_tools": {"type": "object"},
                    "approval_id": {"type": "string"},
                    "cwd": {"type": "string"},
                    "timeout_seconds": {"type": "integer", "minimum": 1},
                    "execute": {"type": "boolean", "default": false}
                }
            }
        }),
        serde_json::json!({
            "name": "artifact_manifest",
            "description": "Return artifact and sandbox manifest refs for a task workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "goal_id": {"type": "string"},
                    "task_id": {"type": "string"},
                    "workspace_id": {"type": "string"},
                    "manifest_path": {"type": "string"}
                }
            }
        }),
        serde_json::json!({
            "name": "checkpoint_history",
            "description": "Return checkpoint history refs from a task workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "goal_id": {"type": "string"},
                    "task_id": {"type": "string"},
                    "workspace_id": {"type": "string"},
                    "manifest_path": {"type": "string"}
                }
            }
        }),
        serde_json::json!({
            "name": "coat_web_search",
            "description": "Route web/reference search through configured COAT research agents. This tool does not perform ambient in-process web scraping.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": true,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string"},
                    "goal_id": {"type": "string"},
                    "task_id": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1},
                    "max_search_depth": {"type": "integer", "minimum": 1},
                    "context": {"type": "array", "items": {"type": "string"}},
                    "route": {"type": "string", "enum": ["plan_only", "coordinator_task", "runner_registry"]},
                    "execution": {"type": "object"},
                    "model": {"type": "object"}
                }
            }
        }),
        serde_json::json!({
            "name": "subagent_policy",
            "description": "Explain that subagents are coordinator-owned durable child tasks and native runner subagent spawning is disabled.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {}
            }
        }),
    ]
}

async fn call_tool(state: &AppState, params: ToolCallParams) -> anyhow::Result<serde_json::Value> {
    match params.name.as_str() {
        "repo_status" => repo_status(state, &params.arguments),
        "test_command" => test_command(state, &params.arguments).await,
        "local_command" => local_command(state, &params.arguments).await,
        "artifact_manifest" => artifact_manifest(state, &params.arguments).await,
        "checkpoint_history" => checkpoint_history(state, &params.arguments).await,
        "coat_web_search" => coat_web_search(state, &params.arguments).await,
        "subagent_policy" => Ok(subagent_policy()),
        other => anyhow::bail!("unknown tool: {other}"),
    }
}

fn subagent_policy() -> serde_json::Value {
    serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": "COAT subagents are durable child tasks created by the coordinator; MCP clients and runners must not spawn native in-process subagents."
            }
        ],
        "structuredContent": {
            "mode": "coordinator_durable_tasks",
            "native_subagent_spawn": "disabled",
            "child_request_channel": "AgentRunResult.child_requests",
            "durable_queue": "coat coordinator task tree",
            "runner_context_requirements": [
                "initialize Codex, Claude Code, SDK, or local-model contexts with this rule",
                "return proposed child work as ChildTaskRequest objects",
                "let the coordinator apply budget, approval, runner routing, memory, and sandbox policy"
            ]
        },
        "isError": false
    })
}

async fn coat_web_search(
    state: &AppState,
    arguments: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let mut request: WebSearchRequest = serde_json::from_value(arguments.clone())?;
    let requested_route = arguments.get("route").is_some();
    if !requested_route {
        request.route = state.web_search_route.clone();
    }
    let child_task = request.child_task_request();
    let goal_id = request.goal_id.unwrap_or_else(Uuid::new_v4);

    if !state.web_search_enabled {
        return Ok(web_search_tool_response(WebSearchResponse {
            status: WebSearchStatus::Blocked,
            request,
            child_task: Some(child_task),
            dispatch: None,
            result: None,
            research: None,
            diagnostics: vec![
                "COAT_WEB_SEARCH_ENABLED/TOOL_REGISTRY_WEB_SEARCH_ENABLED is not enabled"
                    .to_string(),
                "return the child_task to the coordinator or enable runner-routed search in setup/config"
                    .to_string(),
            ],
        }));
    }

    if matches!(
        request.route,
        WebSearchRoutingPreference::PlanOnly | WebSearchRoutingPreference::CoordinatorTask
    ) {
        return Ok(web_search_tool_response(WebSearchResponse {
            status: WebSearchStatus::Planned,
            request,
            child_task: Some(child_task),
            dispatch: None,
            result: None,
            research: None,
            diagnostics: vec![
                "coat_web_search compiled to a coordinator-owned durable research child task"
                    .to_string(),
            ],
        }));
    }

    let Some(runner_registry_url) = state.runner_registry_url.as_ref() else {
        return Ok(web_search_tool_response(WebSearchResponse {
            status: WebSearchStatus::Blocked,
            request,
            child_task: Some(child_task),
            dispatch: None,
            result: None,
            research: None,
            diagnostics: vec![
                "runner-registry routing was requested but TOOL_REGISTRY_RUNNER_REGISTRY_URL/COAT_RUNNER_REGISTRY_URL is not configured"
                    .to_string(),
            ],
        }));
    };

    let task = task_from_web_search_child(goal_id, child_task.clone());
    let client = reqwest::Client::new();
    let dispatch = client
        .post(format!("{runner_registry_url}/dispatch"))
        .json(&RunnerDispatchRequest {
            goal_id,
            task: task.clone(),
            coordinator_node_id: None,
            registered_runners: Vec::new(),
        })
        .send()
        .await?
        .error_for_status()?
        .json::<RunnerDispatchDecision>()
        .await?;

    let Some(endpoint) = dispatch.runner_endpoint.as_ref() else {
        return Ok(web_search_tool_response(WebSearchResponse {
            status: WebSearchStatus::Blocked,
            request,
            child_task: Some(child_task),
            dispatch: Some(dispatch),
            result: None,
            research: None,
            diagnostics: vec![
                "runner registry found no research runner with web_search capability".to_string(),
            ],
        }));
    };

    let result = client
        .post(runner_run_task_url(endpoint))
        .json(&AgentRunRequest {
            goal_id,
            task,
            context_artifacts: Vec::new(),
            coordinator_trace_id: Some("coat_web_search".to_string()),
            timeout_seconds: None,
        })
        .send()
        .await?
        .error_for_status()?
        .json::<AgentRunResult>()
        .await?;

    Ok(web_search_tool_response(WebSearchResponse {
        status: WebSearchStatus::Routed,
        request,
        child_task: Some(child_task),
        dispatch: Some(dispatch),
        research: result.research.clone(),
        result: Some(result),
        diagnostics: vec![
            "web search delegated through runner registry and AgentRunRequest".to_string(),
        ],
    }))
}

fn task_from_web_search_child(goal_id: Uuid, child: coat_domain::ChildTaskRequest) -> TaskNode {
    let task_id = Uuid::new_v4();
    let role = child.role.clone();
    let purpose = child.purpose.unwrap_or_else(|| TaskPurpose::Research {
        question: child.prompt.clone(),
    });
    let execution = child.execution.unwrap_or_default().with_role(role.clone());
    TaskNode {
        id: task_id,
        parent_id: None,
        goal_id,
        depth: 0,
        status: TaskStatus::Runnable,
        role,
        purpose,
        title: child
            .title
            .unwrap_or_else(|| "Routed web/reference search".to_string()),
        subgoal_id: child.subgoal_id,
        execution,
        prompt: child.prompt,
        dependencies: child.dependencies,
        children: Vec::new(),
        budget: child
            .budget
            .unwrap_or_else(|| Budget::default_goal().child_budget()),
        sandbox: child.sandbox.unwrap_or_else(SandboxProfile::default),
        done_criteria: child.done_criteria.unwrap_or_else(DoneCriteria::default),
        review_doctrine: child
            .review_doctrine
            .unwrap_or_else(ReviewDoctrine::default),
        priority: child.priority,
        tags: child.tags,
        color: child.color,
        result: None,
        attempts: 0,
    }
}

fn runner_run_task_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if trimmed.ends_with("/run-task") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/run-task")
    }
}

fn web_search_tool_response(response: WebSearchResponse) -> serde_json::Value {
    let status = response.status.clone();
    serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": match status {
                    WebSearchStatus::Routed => "coat_web_search routed through a configured research runner",
                    WebSearchStatus::Planned => "coat_web_search compiled to a coordinator-owned research task",
                    WebSearchStatus::Blocked => "coat_web_search is blocked until search routing is configured",
                    WebSearchStatus::Failed => "coat_web_search failed",
                }
            }
        ],
        "structuredContent": serde_json::to_value(response).unwrap_or(serde_json::Value::Null),
        "isError": matches!(status, WebSearchStatus::Failed)
    })
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

async fn test_command(
    state: &AppState,
    arguments: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let requested_command = arguments
        .get("command")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if let Some(sandbox_runner_url) = state.sandbox_runner_url.as_ref() {
        let mut payload = serde_json::json!({
            "workspace_id": arguments.get("workspace_id"),
            "goal_id": arguments.get("goal_id"),
            "task_id": arguments.get("task_id"),
            "command": requested_command,
            "approval_id": arguments.get("approval_id")
        });
        if let Some(local_tools) = arguments.get("local_tools") {
            payload["local_tools"] = local_tools.clone();
        }
        let response = reqwest::Client::new()
            .post(format!("{sandbox_runner_url}/commands/plan"))
            .json(&payload)
            .send()
            .await?;
        let status = response.status();
        let value = response.json::<serde_json::Value>().await?;
        return Ok(serde_json::json!({
            "content": [
                {
                    "type": "text",
                    "text": "test execution was routed to sandbox-runner for approval-aware command planning"
                }
            ],
            "structuredContent": {
                "sandbox_runner_url": sandbox_runner_url,
                "http_status": status.as_u16(),
                "plan": value
            },
            "isError": !status.is_success()
        }));
    }

    Ok(serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": "test execution is delegated to sandbox-runner; this MCP tool does not execute commands directly"
            }
        ],
        "structuredContent": {
            "requested_command": requested_command,
            "status": "blocked",
            "next_service": "sandbox-runner",
            "diagnostics": ["TOOL_REGISTRY_SANDBOX_RUNNER_URL is not configured"]
        },
        "isError": false
    }))
}

async fn local_command(
    state: &AppState,
    arguments: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let requested_command = arguments
        .get("command")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let execute = arguments
        .get("execute")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let Some(sandbox_runner_url) = state.sandbox_runner_url.as_ref() else {
        return Ok(serde_json::json!({
            "content": [
                {
                    "type": "text",
                    "text": "local command execution is delegated to sandbox-runner; this MCP tool does not execute commands directly"
                }
            ],
            "structuredContent": {
                "requested_command": requested_command,
                "status": "blocked",
                "next_service": "sandbox-runner",
                "diagnostics": ["TOOL_REGISTRY_SANDBOX_RUNNER_URL is not configured"]
            },
            "isError": false
        }));
    };

    let endpoint = if execute {
        "commands/run"
    } else {
        "commands/plan"
    };
    let mut payload = serde_json::json!({
            "workspace_id": arguments.get("workspace_id"),
            "goal_id": arguments.get("goal_id"),
            "task_id": arguments.get("task_id"),
            "command": requested_command,
            "approval_id": arguments.get("approval_id"),
            "cwd": arguments.get("cwd"),
            "timeout_seconds": arguments.get("timeout_seconds")
    });
    if let Some(local_tools) = arguments.get("local_tools") {
        payload["local_tools"] = local_tools.clone();
    }
    let response = reqwest::Client::new()
        .post(format!("{sandbox_runner_url}/{endpoint}"))
        .json(&payload)
        .send()
        .await?;
    let status = response.status();
    let value = response.json::<serde_json::Value>().await?;
    Ok(serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": if execute {
                    "local command was routed to sandbox-runner for approval-aware execution"
                } else {
                    "local command was routed to sandbox-runner for approval-aware planning"
                }
            }
        ],
        "structuredContent": {
            "sandbox_runner_url": sandbox_runner_url,
            "http_status": status.as_u16(),
            "execute": execute,
            "result": value
        },
        "isError": !status.is_success()
    }))
}

async fn artifact_manifest(
    state: &AppState,
    arguments: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let Some(sandbox_root) = state.sandbox_workspace_root.as_ref() else {
        return Ok(serde_json::json!({
            "content": [
                {
                    "type": "text",
                    "text": "artifact lookup requires TOOL_REGISTRY_SANDBOX_WORKSPACE_ROOT"
                }
            ],
            "structuredContent": {
                "configured": false,
                "artifacts": [],
                "diagnostics": ["TOOL_REGISTRY_SANDBOX_WORKSPACE_ROOT is not configured"]
            },
            "isError": false
        }));
    };

    let lookup = resolve_artifact_lookup(sandbox_root, arguments)?;
    let mut diagnostics = Vec::new();
    let mut artifacts = Vec::new();
    let mut workspace_manifest = None;
    let mut sandbox_launch_plan = None;
    let mut snapshot_manifest = None;
    let mut artifact_manifest = None;

    if let Some(path) = lookup.workspace_manifest.as_ref() {
        workspace_manifest = read_json_file(path, "workspace manifest", &mut diagnostics).await;
    }
    if let Some(path) = lookup.sandbox_launch_plan.as_ref() {
        sandbox_launch_plan = read_json_file(path, "sandbox launch plan", &mut diagnostics).await;
    }
    if let Some(path) = lookup.snapshot_manifest.as_ref() {
        snapshot_manifest = read_json_file(path, "snapshot manifest", &mut diagnostics).await;
    }
    if let Some(path) = lookup.artifact_manifest.as_ref() {
        artifact_manifest = read_json_file(path, "artifact manifest", &mut diagnostics).await;
        if let Some(manifest) = artifact_manifest.as_ref() {
            artifacts = extract_artifacts(manifest);
        }
    }

    let found = workspace_manifest.is_some()
        || sandbox_launch_plan.is_some()
        || snapshot_manifest.is_some()
        || artifact_manifest.is_some();
    let text = if found {
        "artifact workspace manifests found"
    } else {
        "no artifact manifests found for lookup"
    };

    Ok(serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ],
        "structuredContent": {
            "configured": true,
            "found": found,
            "goal_id": lookup.goal_id,
            "task_id": lookup.task_id,
            "workspace_id": lookup.workspace_id,
            "paths": {
                "workspace": lookup.workspace_path,
                "workspace_manifest": lookup.workspace_manifest,
                "sandbox_launch_plan": lookup.sandbox_launch_plan,
                "snapshot_manifest": lookup.snapshot_manifest,
                "checkpoint_manifest": lookup.checkpoint_manifest,
                "artifact_manifest": lookup.artifact_manifest
            },
            "workspace_manifest": workspace_manifest,
            "sandbox_launch_plan": sandbox_launch_plan,
            "snapshot_manifest": snapshot_manifest,
            "artifact_manifest": artifact_manifest,
            "artifacts": artifacts,
            "diagnostics": diagnostics
        },
        "isError": false
    }))
}

async fn checkpoint_history(
    state: &AppState,
    arguments: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let Some(sandbox_root) = state.sandbox_workspace_root.as_ref() else {
        return Ok(serde_json::json!({
            "content": [
                {
                    "type": "text",
                    "text": "checkpoint lookup requires TOOL_REGISTRY_SANDBOX_WORKSPACE_ROOT"
                }
            ],
            "structuredContent": {
                "configured": false,
                "checkpoints": [],
                "diagnostics": ["TOOL_REGISTRY_SANDBOX_WORKSPACE_ROOT is not configured"]
            },
            "isError": false
        }));
    };

    let lookup = resolve_artifact_lookup(sandbox_root, arguments)?;
    let mut diagnostics = Vec::new();
    let checkpoint_manifest = match lookup.checkpoint_manifest.as_ref() {
        Some(path) => read_json_file(path, "checkpoint manifest", &mut diagnostics).await,
        None => None,
    };
    let checkpoints = checkpoint_manifest
        .as_ref()
        .and_then(|manifest| manifest.get("checkpoints"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": if checkpoint_manifest.is_some() { "checkpoint manifest found" } else { "no checkpoint manifest found for lookup" }
            }
        ],
        "structuredContent": {
            "configured": true,
            "found": checkpoint_manifest.is_some(),
            "goal_id": lookup.goal_id,
            "task_id": lookup.task_id,
            "workspace_id": lookup.workspace_id,
            "paths": {
                "workspace": lookup.workspace_path,
                "checkpoint_manifest": lookup.checkpoint_manifest
            },
            "checkpoint_manifest": checkpoint_manifest,
            "checkpoints": checkpoints,
            "diagnostics": diagnostics
        },
        "isError": false
    }))
}

#[derive(Debug)]
struct ArtifactLookup {
    goal_id: Option<String>,
    task_id: Option<String>,
    workspace_id: Option<String>,
    workspace_path: Option<PathBuf>,
    workspace_manifest: Option<PathBuf>,
    sandbox_launch_plan: Option<PathBuf>,
    snapshot_manifest: Option<PathBuf>,
    checkpoint_manifest: Option<PathBuf>,
    artifact_manifest: Option<PathBuf>,
}

fn resolve_artifact_lookup(
    sandbox_root: &Path,
    arguments: &serde_json::Value,
) -> anyhow::Result<ArtifactLookup> {
    let goal_id = arguments
        .get("goal_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let task_id = arguments
        .get("task_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let workspace_id = arguments
        .get("workspace_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let manifest_path = arguments
        .get("manifest_path")
        .and_then(|value| value.as_str())
        .map(PathBuf::from);

    let workspace_path = match (&goal_id, &task_id, &workspace_id) {
        (Some(goal), Some(task), _) => Some(confined_join(sandbox_root, &[goal, task])?),
        (_, _, Some(workspace)) => workspace_path_from_registry(sandbox_root, workspace)?,
        _ => None,
    };

    let artifact_manifest = match manifest_path {
        Some(path) => Some(confined_path(sandbox_root, &path)?),
        None => workspace_path
            .as_ref()
            .map(|path| path.join("artifacts/artifact-manifest.json")),
    };

    Ok(ArtifactLookup {
        goal_id,
        task_id,
        workspace_id,
        workspace_manifest: workspace_path
            .as_ref()
            .map(|path| path.join("workspace-manifest.json")),
        sandbox_launch_plan: workspace_path
            .as_ref()
            .map(|path| path.join("sandbox-launch-plan.json")),
        snapshot_manifest: workspace_path
            .as_ref()
            .map(|path| path.join("snapshots/latest.json")),
        checkpoint_manifest: workspace_path
            .as_ref()
            .map(|path| path.join("checkpoints/checkpoint-manifest.json")),
        workspace_path,
        artifact_manifest,
    })
}

fn workspace_path_from_registry(
    sandbox_root: &Path,
    workspace_id: &str,
) -> anyhow::Result<Option<PathBuf>> {
    let record_path = confined_join(
        sandbox_root,
        &[".coat-workspaces", &format!("{workspace_id}.json")],
    )?;
    if !record_path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&record_path)?;
    let record: serde_json::Value = serde_json::from_slice(&bytes)?;
    let Some(path) = record.get("path").and_then(|value| value.as_str()) else {
        return Ok(None);
    };
    Ok(Some(confined_path(sandbox_root, Path::new(path))?))
}

fn confined_join(root: &Path, parts: &[&str]) -> anyhow::Result<PathBuf> {
    let mut path = PathBuf::from(root);
    for part in parts {
        if part.is_empty() || part.contains('/') || part.contains('\\') || part == &".." {
            anyhow::bail!("unsafe path component: {part}");
        }
        path.push(part);
    }
    Ok(path)
}

fn confined_path(root: &Path, requested: &Path) -> anyhow::Result<PathBuf> {
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        anyhow::bail!("path may not contain parent directory components");
    }
    let path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    if !path.starts_with(root) {
        anyhow::bail!(
            "path {} is outside sandbox workspace root {}",
            path.display(),
            root.display()
        );
    }
    Ok(path)
}

async fn read_json_file(
    path: &Path,
    label: &str,
    diagnostics: &mut Vec<String>,
) -> Option<serde_json::Value> {
    match tokio::fs::read(path).await {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(value) => Some(value),
            Err(error) => {
                diagnostics.push(format!(
                    "{label} at {} is invalid JSON: {error}",
                    path.display()
                ));
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            diagnostics.push(format!("{label} not found at {}", path.display()));
            None
        }
        Err(error) => {
            diagnostics.push(format!(
                "{label} could not be read at {}: {error}",
                path.display()
            ));
            None
        }
    }
}

fn extract_artifacts(manifest: &serde_json::Value) -> Vec<serde_json::Value> {
    manifest
        .get("artifacts")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
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
    use std::sync::Arc;

    use axum::{
        extract::State,
        http::{HeaderMap, HeaderValue, StatusCode, header::AUTHORIZATION},
    };

    use super::{
        AppState, artifact_manifest, authorize, checkpoint_history, coat_web_search, list_tools,
        local_command, resolve_repo_path, runner_run_task_url, subagent_policy, test_command,
    };
    use coat_domain::{ExecutionProfile, WebSearchRoutingPreference, WorkerKind};

    fn test_state() -> AppState {
        AppState {
            workspace_root: std::env::current_dir().expect("cwd"),
            sandbox_workspace_root: None,
            sandbox_runner_url: None,
            runner_registry_url: None,
            web_search_enabled: false,
            web_search_route: WebSearchRoutingPreference::CoordinatorTask,
            auth_token: None,
        }
    }

    #[test]
    fn bearer_auth_is_required_when_token_is_configured() {
        let state = AppState {
            auth_token: Some("secret".to_string()),
            ..test_state()
        };
        let headers = HeaderMap::new();
        assert!(authorize(&state, &headers).is_err());

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer secret"));
        assert!(authorize(&state, &headers).is_ok());
    }

    #[tokio::test]
    async fn tools_list_uses_same_bearer_auth_policy_as_mcp() {
        let state = Arc::new(AppState {
            auth_token: Some("secret".to_string()),
            ..test_state()
        });

        let (status, body) = list_tools(State(Arc::clone(&state)), HeaderMap::new()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            body.0
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .contains("unauthorized tool list request"),
            "unauthorized body should explain the failure: {body:?}"
        );

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer secret"));
        let (status, body) = list_tools(State(state), headers).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.0
                .as_array()
                .map(|tools| tools
                    .iter()
                    .any(|tool| tool.get("name") == Some(&serde_json::json!("coat_web_search"))))
                .unwrap_or(false),
            "authorized list should expose coat_web_search: {body:?}"
        );
    }

    #[test]
    fn repo_path_is_confined_to_workspace_root() {
        let state = AppState {
            workspace_root: std::env::current_dir()
                .expect("cwd")
                .canonicalize()
                .expect("canonical cwd"),
            ..test_state()
        };
        let inside = resolve_repo_path(&state, &serde_json::json!({})).expect("inside path");
        assert!(inside.starts_with(&state.workspace_root));

        let outside = resolve_repo_path(&state, &serde_json::json!({ "repo_path": "/" }));
        assert!(outside.is_err());
    }

    #[tokio::test]
    async fn test_command_reports_sandbox_delegation() {
        let state = test_state();
        let result = test_command(&state, &serde_json::json!({ "command": "cargo test" }))
            .await
            .expect("test command response");
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["status"], "blocked");
        assert_eq!(
            result["structuredContent"]["next_service"],
            "sandbox-runner"
        );
    }

    #[tokio::test]
    async fn local_command_reports_sandbox_delegation() {
        let state = test_state();
        let result = local_command(
            &state,
            &serde_json::json!({ "command": ["git", "status"], "execute": true }),
        )
        .await
        .expect("local command response");
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["status"], "blocked");
        assert_eq!(
            result["structuredContent"]["next_service"],
            "sandbox-runner"
        );
    }

    #[test]
    fn subagent_policy_reports_durable_child_task_channel() {
        let result = subagent_policy();
        assert_eq!(result["isError"], false);
        assert_eq!(
            result["structuredContent"]["mode"],
            "coordinator_durable_tasks"
        );
        assert_eq!(
            result["structuredContent"]["native_subagent_spawn"],
            "disabled"
        );
        assert_eq!(
            result["structuredContent"]["child_request_channel"],
            "AgentRunResult.child_requests"
        );
    }

    #[tokio::test]
    async fn coat_web_search_returns_child_task_when_not_enabled() {
        let state = test_state();
        let result = coat_web_search(
            &state,
            &serde_json::json!({
                "query": "current web search contract",
                "route": "runner_registry"
            }),
        )
        .await
        .expect("web search response");

        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["status"], "blocked");
        assert_eq!(
            result["structuredContent"]["child_task"]["role"],
            "research"
        );
        assert_eq!(
            result["structuredContent"]["child_task"]["purpose"]["kind"],
            "research"
        );
        assert_eq!(
            result["structuredContent"]["child_task"]["execution"]["runner"]["required_capabilities"]
                [1],
            "web_search"
        );
    }

    #[tokio::test]
    async fn coat_web_search_preserves_configured_codex_runner_route() {
        let state = test_state();
        let execution = ExecutionProfile::default().with_role(WorkerKind::Codex);
        let result = coat_web_search(
            &state,
            &serde_json::json!({
                "query": "current Codex-native search contract",
                "route": "runner_registry",
                "execution": execution
            }),
        )
        .await
        .expect("web search response");

        assert_eq!(result["structuredContent"]["child_task"]["role"], "codex");
        assert_eq!(
            result["structuredContent"]["child_task"]["execution"]["runner"]["worker"],
            "codex"
        );
    }

    #[test]
    fn runner_run_task_url_does_not_append_twice() {
        assert_eq!(
            runner_run_task_url("http://runner:9091"),
            "http://runner:9091/run-task"
        );
        assert_eq!(
            runner_run_task_url("http://runner:9091/run-task"),
            "http://runner:9091/run-task"
        );
    }

    #[tokio::test]
    async fn artifact_manifest_reads_task_workspace_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let goal_id = "018f8f2f-1fd8-7688-bb12-8bfb6b756611";
        let task_id = "018f8f2f-1fd8-7688-bb12-8bfb6b756612";
        let task_root = temp.path().join(goal_id).join(task_id);
        std::fs::create_dir_all(task_root.join("artifacts")).expect("artifact dir");
        std::fs::create_dir_all(task_root.join("snapshots")).expect("snapshot dir");
        std::fs::create_dir_all(task_root.join("checkpoints")).expect("checkpoint dir");
        std::fs::write(
            task_root.join("workspace-manifest.json"),
            r#"{"workspace_id":"workspace-1"}"#,
        )
        .expect("workspace manifest");
        std::fs::write(
            task_root.join("sandbox-launch-plan.json"),
            r#"{"backend":"local_workspace"}"#,
        )
        .expect("launch plan");
        std::fs::write(
            task_root.join("snapshots/latest.json"),
            r#"{"artifact_uri":"workspace://workspace-1/snapshot/latest"}"#,
        )
        .expect("snapshot");
        std::fs::write(
            task_root.join("artifacts/artifact-manifest.json"),
            r#"{"artifacts":[{"uri":"workspace://workspace-1/report.json"}]}"#,
        )
        .expect("artifact manifest");
        std::fs::write(
            task_root.join("checkpoints/checkpoint-manifest.json"),
            r#"{"checkpoints":[{"label":"before-review","uri":"git+checkpoint://branch"}]}"#,
        )
        .expect("checkpoint manifest");

        let state = AppState {
            sandbox_workspace_root: Some(temp.path().to_path_buf()),
            ..test_state()
        };
        let result = artifact_manifest(
            &state,
            &serde_json::json!({
                "goal_id": goal_id,
                "task_id": task_id
            }),
        )
        .await
        .expect("artifact lookup");

        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["found"], true);
        assert_eq!(
            result["structuredContent"]["artifacts"][0]["uri"],
            "workspace://workspace-1/report.json"
        );
        assert_eq!(
            result["structuredContent"]["paths"]["checkpoint_manifest"],
            task_root
                .join("checkpoints/checkpoint-manifest.json")
                .to_string_lossy()
                .as_ref()
        );
    }

    #[tokio::test]
    async fn checkpoint_history_reads_checkpoint_manifest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let goal_id = "018f8f2f-1fd8-7688-bb12-8bfb6b756611";
        let task_id = "018f8f2f-1fd8-7688-bb12-8bfb6b756612";
        let task_root = temp.path().join(goal_id).join(task_id);
        std::fs::create_dir_all(task_root.join("checkpoints")).expect("checkpoint dir");
        std::fs::write(
            task_root.join("checkpoints/checkpoint-manifest.json"),
            r#"{"checkpoints":[{"label":"checkpoint-1","uri":"git+checkpoint://branch"}]}"#,
        )
        .expect("checkpoint manifest");
        let state = AppState {
            sandbox_workspace_root: Some(temp.path().to_path_buf()),
            ..test_state()
        };
        let result = checkpoint_history(
            &state,
            &serde_json::json!({
                "goal_id": goal_id,
                "task_id": task_id
            }),
        )
        .await
        .expect("checkpoint lookup");

        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["found"], true);
        assert_eq!(
            result["structuredContent"]["checkpoints"][0]["label"],
            "checkpoint-1"
        );
    }
}
