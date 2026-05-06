use axum::{Json, Router, routing::get, routing::post};
use jattg_domain::{ArtifactKind, ArtifactRef, SandboxProfile};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
struct CreateWorkspaceRequest {
    goal_id: Uuid,
    task_id: Uuid,
    repo: Option<String>,
    sandbox: SandboxProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
struct WorkspaceResponse {
    workspace_id: Uuid,
    path: String,
    artifact: ArtifactRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
struct WorkspaceId {
    workspace_id: Uuid,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "jattg_sandbox_runner=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9083".to_string());
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/workspaces", post(create_workspace))
        .route("/snapshot", post(snapshot))
        .route("/cleanup", post(cleanup))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "sandbox runner listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn create_workspace(Json(request): Json<CreateWorkspaceRequest>) -> Json<WorkspaceResponse> {
    let workspace_id = Uuid::new_v4();
    let path = format!("/workspaces/{}/{}", request.goal_id, request.task_id);
    Json(WorkspaceResponse {
        workspace_id,
        path: path.clone(),
        artifact: ArtifactRef {
            kind: ArtifactKind::WorkspaceSnapshot,
            uri: format!("workspace://{workspace_id}"),
            description: format!("sandbox profile {:?} at {path}", request.sandbox),
            sha256: None,
        },
    })
}

async fn snapshot(Json(request): Json<WorkspaceId>) -> Json<ArtifactRef> {
    Json(ArtifactRef {
        kind: ArtifactKind::WorkspaceSnapshot,
        uri: format!("workspace://{}/snapshot/latest", request.workspace_id),
        description: "workspace snapshot placeholder".to_string(),
        sha256: None,
    })
}

async fn cleanup(Json(_request): Json<WorkspaceId>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "cleanup_requested" }))
}
