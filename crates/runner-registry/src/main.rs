use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use coat_domain::{
    RunnerDispatchDecision, RunnerDispatchRequest, RunnerHeartbeat, RunnerRegistration,
    RunnerStatus,
};
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;

type RegistryState = Arc<RwLock<BTreeMap<String, RunnerRecord>>>;

#[derive(Debug, Clone)]
struct RunnerRecord {
    registration: RunnerRegistration,
    last_seen: Instant,
    running_tasks: u32,
    capacity_remaining: u32,
}

impl RunnerRecord {
    fn new(registration: RunnerRegistration) -> Self {
        let capacity_remaining = registration.max_concurrency;
        Self {
            registration,
            last_seen: Instant::now(),
            running_tasks: 0,
            capacity_remaining,
        }
    }

    fn heartbeat(&mut self, heartbeat: RunnerHeartbeat) {
        self.last_seen = Instant::now();
        self.running_tasks = heartbeat.running_tasks;
        self.capacity_remaining = heartbeat.capacity_remaining;
    }

    fn is_dispatchable(&self, now: Instant) -> bool {
        let ttl = Duration::from_secs(self.registration.lease_ttl_seconds);
        now.duration_since(self.last_seen) <= ttl && self.capacity_remaining > 0
    }

    fn status(&self, now: Instant) -> RunnerStatus {
        let last_seen_age = now.duration_since(self.last_seen);
        let stale = last_seen_age > Duration::from_secs(self.registration.lease_ttl_seconds);
        let full = self.capacity_remaining == 0;
        RunnerStatus {
            registration: self.registration.clone(),
            running_tasks: self.running_tasks,
            capacity_remaining: self.capacity_remaining,
            last_seen_age_seconds: last_seen_age.as_secs(),
            dispatchable: !stale && !full,
            stale,
            full,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "coat_runner_registry=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9085".to_string());
    let state = Arc::new(RwLock::new(BTreeMap::new()));
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/runners", get(list_runners).post(register_runner))
        .route("/runners/status", get(list_runner_status))
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
    state.write().await.insert(
        registration.runner_id.clone(),
        RunnerRecord::new(registration.clone()),
    );
    Json(registration)
}

async fn list_runners(State(state): State<RegistryState>) -> Json<Vec<RunnerRegistration>> {
    Json(
        state
            .read()
            .await
            .values()
            .map(|record| record.registration.clone())
            .collect(),
    )
}

async fn list_runner_status(State(state): State<RegistryState>) -> Json<Vec<RunnerStatus>> {
    let now = Instant::now();
    Json(
        state
            .read()
            .await
            .values()
            .map(|record| record.status(now))
            .collect(),
    )
}

async fn heartbeat(
    State(state): State<RegistryState>,
    Json(heartbeat): Json<RunnerHeartbeat>,
) -> Json<serde_json::Value> {
    let mut runners = state.write().await;
    let known = runners.contains_key(&heartbeat.runner_id);
    if let Some(record) = runners.get_mut(&heartbeat.runner_id) {
        record.heartbeat(heartbeat.clone());
    }
    Json(serde_json::json!({
        "known": known,
        "runner_id": heartbeat.runner_id,
        "running_tasks": heartbeat.running_tasks,
        "capacity_remaining": heartbeat.capacity_remaining
    }))
}

async fn dispatch(
    State(state): State<RegistryState>,
    Json(mut request): Json<RunnerDispatchRequest>,
) -> Json<RunnerDispatchDecision> {
    if request.registered_runners.is_empty() {
        let now = Instant::now();
        request.registered_runners = state
            .read()
            .await
            .values()
            .filter(|record| record.is_dispatchable(now))
            .map(|record| record.registration.clone())
            .collect();
    }
    Json(RunnerDispatchDecision::choose(request))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        time::{Duration, Instant},
    };

    use coat_domain::RunnerRegistration;

    use super::RunnerRecord;

    fn registration() -> RunnerRegistration {
        RunnerRegistration {
            runner_id: "runner-a".to_string(),
            node_id: "node-a".to_string(),
            endpoint: "http://runner-a:9091".to_string(),
            roles: Vec::new(),
            capabilities: Vec::new(),
            models: Vec::new(),
            labels: BTreeMap::new(),
            mcp_servers: Vec::new(),
            max_concurrency: 2,
            lease_ttl_seconds: 300,
        }
    }

    #[test]
    fn runner_record_filters_stale_or_full_runners() {
        let now = Instant::now();
        let mut record = RunnerRecord::new(registration());
        assert!(record.is_dispatchable(now));
        assert!(record.status(now).dispatchable);

        record.capacity_remaining = 0;
        assert!(!record.is_dispatchable(now));
        let full = record.status(now);
        assert!(full.full);
        assert!(!full.dispatchable);

        record.capacity_remaining = 1;
        record.last_seen = now - Duration::from_secs(301);
        assert!(!record.is_dispatchable(now));
        let stale = record.status(now);
        assert!(stale.stale);
        assert!(!stale.dispatchable);
    }
}
