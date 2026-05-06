use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use jattg_domain::{
    RunnerDispatchDecision, RunnerDispatchRequest, RunnerHeartbeat, RunnerRegistration,
};
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;

type RegistryState = Arc<RwLock<BTreeMap<String, RunnerRegistration>>>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "jattg_runner_registry=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9085".to_string());
    let state = Arc::new(RwLock::new(BTreeMap::new()));
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/runners", get(list_runners).post(register_runner))
        .route("/runners/heartbeat", post(heartbeat))
        .route("/dispatch", post(dispatch))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "runner registry listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn register_runner(
    State(state): State<RegistryState>,
    Json(registration): Json<RunnerRegistration>,
) -> Json<RunnerRegistration> {
    state
        .write()
        .await
        .insert(registration.runner_id.clone(), registration.clone());
    Json(registration)
}

async fn list_runners(State(state): State<RegistryState>) -> Json<Vec<RunnerRegistration>> {
    Json(state.read().await.values().cloned().collect())
}

async fn heartbeat(
    State(state): State<RegistryState>,
    Json(heartbeat): Json<RunnerHeartbeat>,
) -> Json<serde_json::Value> {
    let known = state.read().await.contains_key(&heartbeat.runner_id);
    Json(serde_json::json!({
        "known": known,
        "runner_id": heartbeat.runner_id,
        "capacity_remaining": heartbeat.capacity_remaining
    }))
}

async fn dispatch(
    State(state): State<RegistryState>,
    Json(mut request): Json<RunnerDispatchRequest>,
) -> Json<RunnerDispatchDecision> {
    if request.registered_runners.is_empty() {
        request.registered_runners = state.read().await.values().cloned().collect();
    }
    Json(RunnerDispatchDecision::choose(request))
}
