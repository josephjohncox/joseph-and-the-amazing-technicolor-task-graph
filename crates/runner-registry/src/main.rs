//! Distributed runner registry and dispatch policy service.
//!
//! Purpose: collect runner registrations and heartbeats, expose runner status,
//! and choose a compatible runner/model for each task using role, capabilities,
//! labels, locality, MCP context, auth policy, and model route.
//!
//! Architecture references:
//! - `docs/design-docs/010-distributed-runners-mcp.md`
//! - `docs/exec-plans/active/090-distributed-runners-mcp-notifications.md`

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use coat_domain::{
    RunnerDispatchDecision, RunnerDispatchRequest, RunnerHeartbeat, RunnerPoolSupply,
    RunnerRegistration, RunnerScalingDecision, RunnerScalingRequest, RunnerStatus,
};
use serde::{Deserialize, Serialize};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::RwLock};
use tower_http::trace::TraceLayer;

#[derive(Debug, Clone)]
struct AppState {
    runners: Arc<RwLock<BTreeMap<String, RunnerRecord>>>,
    journal_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct RunnerRecord {
    registration: RunnerRegistration,
    last_seen: Instant,
    last_seen_unix_seconds: u64,
    running_tasks: u32,
    capacity_remaining: u32,
}

impl RunnerRecord {
    fn new(registration: RunnerRegistration) -> Self {
        let capacity_remaining = registration.max_concurrency;
        Self {
            registration,
            last_seen: Instant::now(),
            last_seen_unix_seconds: unix_now(),
            running_tasks: 0,
            capacity_remaining,
        }
    }

    fn heartbeat(&mut self, heartbeat: RunnerHeartbeat) {
        self.last_seen = Instant::now();
        self.last_seen_unix_seconds = unix_now();
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum RunnerJournalEntry {
    Registered {
        registration: RunnerRegistration,
        recorded_at_unix_seconds: u64,
    },
    Heartbeat {
        heartbeat: RunnerHeartbeat,
        recorded_at_unix_seconds: u64,
    },
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
    let journal_path = std::env::var("COAT_RUNNER_REGISTRY_JOURNAL_PATH")
        .or_else(|_| std::env::var("RUNNER_REGISTRY_JOURNAL_PATH"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let runners = replay_journal(journal_path.as_deref())?;
    let state = AppState {
        runners: Arc::new(RwLock::new(runners)),
        journal_path,
    };
    let app = registry_app(state);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "runner registry listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn registry_app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/runners", get(list_runners).post(register_runner))
        .route("/runners/status", get(list_runner_status))
        .route("/runners/heartbeat", post(heartbeat))
        .route("/dispatch", post(dispatch))
        .route("/capacity/plan", post(capacity_plan))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn register_runner(
    State(state): State<AppState>,
    Json(registration): Json<RunnerRegistration>,
) -> Json<RunnerRegistration> {
    state.runners.write().await.insert(
        registration.runner_id.clone(),
        RunnerRecord::new(registration.clone()),
    );
    append_journal(
        &state,
        RunnerJournalEntry::Registered {
            registration: registration.clone(),
            recorded_at_unix_seconds: unix_now(),
        },
    )
    .await;
    Json(registration)
}

async fn list_runners(State(state): State<AppState>) -> Json<Vec<RunnerRegistration>> {
    Json(
        state
            .runners
            .read()
            .await
            .values()
            .map(|record| record.registration.clone())
            .collect(),
    )
}

async fn list_runner_status(State(state): State<AppState>) -> Json<Vec<RunnerStatus>> {
    let now = Instant::now();
    Json(
        state
            .runners
            .read()
            .await
            .values()
            .map(|record| record.status(now))
            .collect(),
    )
}

async fn heartbeat(
    State(state): State<AppState>,
    Json(heartbeat): Json<RunnerHeartbeat>,
) -> Json<serde_json::Value> {
    let mut runners = state.runners.write().await;
    let known = runners.contains_key(&heartbeat.runner_id);
    if let Some(record) = runners.get_mut(&heartbeat.runner_id) {
        record.heartbeat(heartbeat.clone());
    }
    drop(runners);
    if known {
        append_journal(
            &state,
            RunnerJournalEntry::Heartbeat {
                heartbeat: heartbeat.clone(),
                recorded_at_unix_seconds: unix_now(),
            },
        )
        .await;
    }
    Json(serde_json::json!({
        "known": known,
        "runner_id": heartbeat.runner_id,
        "running_tasks": heartbeat.running_tasks,
        "capacity_remaining": heartbeat.capacity_remaining
    }))
}

async fn dispatch(
    State(state): State<AppState>,
    Json(mut request): Json<RunnerDispatchRequest>,
) -> Json<RunnerDispatchDecision> {
    if request.registered_runners.is_empty() {
        let now = Instant::now();
        request.registered_runners = state
            .runners
            .read()
            .await
            .values()
            .filter(|record| record.is_dispatchable(now))
            .map(|record| record.registration.clone())
            .collect();
    }
    Json(RunnerDispatchDecision::choose(request))
}

async fn capacity_plan(
    State(state): State<AppState>,
    Json(mut request): Json<RunnerScalingRequest>,
) -> Json<RunnerScalingDecision> {
    if request.supplies.is_empty() {
        request.supplies = runner_pool_supplies(&state).await;
    }
    Json(RunnerScalingDecision::recommend(request))
}

async fn runner_pool_supplies(state: &AppState) -> Vec<RunnerPoolSupply> {
    let now = Instant::now();
    let mut pools = BTreeMap::<String, RunnerPoolSupply>::new();
    for record in state.runners.read().await.values() {
        let pool_key = runner_pool_key(&record.registration);
        let entry = pools.entry(pool_key.clone()).or_insert(RunnerPoolSupply {
            pool_key,
            registered_runners: 0,
            dispatchable_runners: 0,
            running_tasks: 0,
            capacity_remaining: 0,
            max_concurrency: 0,
            pending_provisions: 0,
            stale_runners: 0,
        });
        let status = record.status(now);
        entry.registered_runners = entry.registered_runners.saturating_add(1);
        if status.dispatchable {
            entry.dispatchable_runners = entry.dispatchable_runners.saturating_add(1);
        }
        if status.stale {
            entry.stale_runners = entry.stale_runners.saturating_add(1);
        }
        entry.running_tasks = entry.running_tasks.saturating_add(status.running_tasks);
        entry.capacity_remaining = entry
            .capacity_remaining
            .saturating_add(status.capacity_remaining);
        entry.max_concurrency = entry
            .max_concurrency
            .saturating_add(record.registration.max_concurrency);
    }
    pools.into_values().collect()
}

fn runner_pool_key(registration: &RunnerRegistration) -> String {
    registration
        .labels
        .get("pool")
        .or_else(|| registration.labels.get("lane"))
        .cloned()
        .or_else(|| {
            registration
                .roles
                .first()
                .map(|role| role.as_str().to_string())
        })
        .unwrap_or_else(|| "default".to_string())
}

async fn append_journal(state: &AppState, entry: RunnerJournalEntry) {
    let Some(path) = &state.journal_path else {
        return;
    };
    let result = async {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        let line = format!("{}\n", serde_json::to_string(&entry)?);
        file.write_all(line.as_bytes()).await?;
        anyhow::Ok(())
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, path = %path.display(), "runner registry journal append failed");
    }
}

fn replay_journal(path: Option<&Path>) -> anyhow::Result<BTreeMap<String, RunnerRecord>> {
    let mut runners = BTreeMap::new();
    let Some(path) = path else {
        return Ok(runners);
    };
    if !path.exists() {
        return Ok(runners);
    }
    let contents = std::fs::read_to_string(path)?;
    let mut skipped_lines = 0usize;
    let mut first_skipped_line = None::<usize>;
    let mut first_error = None::<String>;
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<RunnerJournalEntry>(line) {
            Ok(entry) => apply_journal_entry(&mut runners, entry),
            Err(error) => {
                skipped_lines += 1;
                first_skipped_line.get_or_insert(index + 1);
                first_error.get_or_insert_with(|| error.to_string());
            }
        }
    }
    if skipped_lines > 0 {
        tracing::warn!(
            path = %path.display(),
            skipped_lines,
            first_skipped_line,
            first_error,
            "skipping invalid runner registry journal entries"
        );
    }
    Ok(runners)
}

fn apply_journal_entry(runners: &mut BTreeMap<String, RunnerRecord>, entry: RunnerJournalEntry) {
    match entry {
        RunnerJournalEntry::Registered {
            registration,
            recorded_at_unix_seconds,
        } => {
            let mut record = RunnerRecord::new(registration);
            record.last_seen_unix_seconds = recorded_at_unix_seconds;
            record.last_seen = instant_from_wall_clock(recorded_at_unix_seconds);
            runners.insert(record.registration.runner_id.clone(), record);
        }
        RunnerJournalEntry::Heartbeat {
            heartbeat,
            recorded_at_unix_seconds,
        } => {
            if let Some(record) = runners.get_mut(&heartbeat.runner_id) {
                record.running_tasks = heartbeat.running_tasks;
                record.capacity_remaining = heartbeat.capacity_remaining;
                record.last_seen_unix_seconds = recorded_at_unix_seconds;
                record.last_seen = instant_from_wall_clock(recorded_at_unix_seconds);
            }
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn instant_from_wall_clock(recorded_at_unix_seconds: u64) -> Instant {
    let elapsed = unix_now().saturating_sub(recorded_at_unix_seconds);
    Instant::now()
        .checked_sub(Duration::from_secs(elapsed))
        .unwrap_or_else(Instant::now)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::Arc,
        time::{Duration, Instant},
    };

    use axum::{
        Json, Router,
        body::{Body, to_bytes},
        extract::State,
        http::{Method, Request, StatusCode},
    };
    use coat_domain::{
        CapacityScalingMode, CapacityScalingPolicy, GoalSpec, GoalState, RunnerCapability,
        RunnerDispatchDecision, RunnerDispatchRequest, RunnerDispatchStatus, RunnerHeartbeat,
        RunnerLocality, RunnerPoolDemand, RunnerRegistration, RunnerScalingDecision,
        RunnerScalingRequest, RunnerScalingStatus, RunnerStatus, WorkerKind,
    };
    use serde::{Serialize, de::DeserializeOwned};
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    use super::{
        AppState, RunnerJournalEntry, RunnerRecord, apply_journal_entry, dispatch, registry_app,
        replay_journal, unix_now,
    };

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

    async fn post_json<T, R>(app: Router, uri: &str, body: &T) -> R
    where
        T: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(body).expect("request json")))
            .expect("request");
        let response = app.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("response json")
    }

    async fn get_json<R>(app: Router, uri: &str) -> R
    where
        R: DeserializeOwned,
    {
        let request = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        let response = app.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("response json")
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

    #[test]
    fn runner_journal_replays_registration_and_heartbeat() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("runner-registry.jsonl");
        let registration = registration();
        let heartbeat = coat_domain::RunnerHeartbeat {
            runner_id: registration.runner_id.clone(),
            node_id: registration.node_id.clone(),
            running_tasks: 1,
            capacity_remaining: 1,
        };
        let lines = [
            serde_json::to_string(&RunnerJournalEntry::Registered {
                registration: registration.clone(),
                recorded_at_unix_seconds: unix_now(),
            })
            .expect("registration json"),
            serde_json::to_string(&RunnerJournalEntry::Heartbeat {
                heartbeat,
                recorded_at_unix_seconds: unix_now(),
            })
            .expect("heartbeat json"),
        ];
        std::fs::write(&path, format!("{}\n{}\n", lines[0], lines[1])).expect("write journal");

        let runners = replay_journal(Some(&path)).expect("replay journal");
        let record = runners.get("runner-a").expect("runner replayed");
        assert_eq!(record.registration.endpoint, registration.endpoint);
        assert_eq!(record.running_tasks, 1);
        assert_eq!(record.capacity_remaining, 1);
    }

    #[test]
    fn runner_journal_skips_corrupt_lines_and_replays_valid_entries() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("runner-registry-corrupt.jsonl");
        let registration = registration();
        let line = serde_json::to_string(&RunnerJournalEntry::Registered {
            registration: registration.clone(),
            recorded_at_unix_seconds: unix_now(),
        })
        .expect("registration json");
        std::fs::write(&path, format!("not-json\n{line}\n{{}}\n")).expect("write journal");

        let runners = replay_journal(Some(&path)).expect("corrupt lines should not block replay");
        let record = runners.get("runner-a").expect("valid runner replayed");
        assert_eq!(record.registration.endpoint, registration.endpoint);
    }

    #[test]
    fn heartbeat_journal_entry_without_registration_is_ignored() {
        let mut runners = BTreeMap::new();
        apply_journal_entry(
            &mut runners,
            RunnerJournalEntry::Heartbeat {
                heartbeat: coat_domain::RunnerHeartbeat {
                    runner_id: "missing".to_string(),
                    node_id: "node-a".to_string(),
                    running_tasks: 1,
                    capacity_remaining: 0,
                },
                recorded_at_unix_seconds: unix_now(),
            },
        );
        assert!(runners.is_empty());
    }

    #[test]
    fn replayed_stale_runner_is_not_dispatchable() {
        let mut registration = registration();
        registration.lease_ttl_seconds = 300;
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("runner-registry-stale.jsonl");
        let stale_timestamp = unix_now().saturating_sub(301);
        let line = serde_json::to_string(&RunnerJournalEntry::Registered {
            registration,
            recorded_at_unix_seconds: stale_timestamp,
        })
        .expect("registration json");
        std::fs::write(&path, format!("{line}\n")).expect("write journal");

        let runners = replay_journal(Some(&path)).expect("replay journal");
        let record = runners.get("runner-a").expect("runner replayed");
        assert!(!record.is_dispatchable(Instant::now()));
        assert!(record.status(Instant::now()).stale);
    }

    #[tokio::test]
    async fn dispatch_ignores_replayed_stale_and_full_runners_before_locality_matching() {
        let mut goal = GoalSpec::new(
            "multi-node dispatch",
            "route a task only to available runners on compatible nodes",
        );
        goal.default_execution.runner.locality = RunnerLocality::RemoteOnly;
        goal.default_execution
            .runner
            .required_capabilities
            .push(RunnerCapability::Code);
        goal.default_execution
            .runner
            .required_labels
            .insert("region".to_string(), "west".to_string());
        let task = GoalState::new(goal).runnable_tasks().remove(0);

        let mut local_full = registration();
        local_full.runner_id = "local-full".to_string();
        local_full.node_id = "control-node".to_string();
        local_full.endpoint = "http://local-full:9091".to_string();
        local_full.capabilities = vec![RunnerCapability::Code];
        local_full.roles = vec![WorkerKind::Planner];
        local_full
            .labels
            .insert("region".to_string(), "west".to_string());
        local_full.models = task.execution.model.candidates.clone();

        let mut stale_remote = local_full.clone();
        stale_remote.runner_id = "stale-remote".to_string();
        stale_remote.node_id = "worker-stale".to_string();
        stale_remote.endpoint = "http://stale-remote:9091".to_string();
        stale_remote.lease_ttl_seconds = 300;

        let mut active_remote = local_full.clone();
        active_remote.runner_id = "active-remote".to_string();
        active_remote.node_id = "worker-active".to_string();
        active_remote.endpoint = "http://active-remote:9091".to_string();

        let mut runners = BTreeMap::new();
        apply_journal_entry(
            &mut runners,
            RunnerJournalEntry::Registered {
                registration: local_full.clone(),
                recorded_at_unix_seconds: unix_now(),
            },
        );
        apply_journal_entry(
            &mut runners,
            RunnerJournalEntry::Heartbeat {
                heartbeat: coat_domain::RunnerHeartbeat {
                    runner_id: local_full.runner_id.clone(),
                    node_id: local_full.node_id.clone(),
                    running_tasks: 2,
                    capacity_remaining: 0,
                },
                recorded_at_unix_seconds: unix_now(),
            },
        );
        apply_journal_entry(
            &mut runners,
            RunnerJournalEntry::Registered {
                registration: stale_remote.clone(),
                recorded_at_unix_seconds: unix_now().saturating_sub(301),
            },
        );
        apply_journal_entry(
            &mut runners,
            RunnerJournalEntry::Registered {
                registration: active_remote,
                recorded_at_unix_seconds: unix_now(),
            },
        );

        let statuses: BTreeMap<String, _> = runners
            .iter()
            .map(|(runner_id, record)| (runner_id.clone(), record.status(Instant::now())))
            .collect();
        assert!(statuses["local-full"].full);
        assert!(statuses["stale-remote"].stale);
        assert!(statuses["active-remote"].dispatchable);

        let state = AppState {
            runners: Arc::new(RwLock::new(runners)),
            journal_path: None,
        };
        let Json(decision) = dispatch(
            State(state),
            Json(RunnerDispatchRequest {
                goal_id: task.goal_id,
                task,
                coordinator_node_id: Some("control-node".to_string()),
                registered_runners: Vec::new(),
            }),
        )
        .await;

        assert_eq!(decision.status, RunnerDispatchStatus::Matched);
        assert_eq!(decision.runner_id.as_deref(), Some("active-remote"));
        assert_eq!(decision.candidates.len(), 1);
        assert!(
            decision
                .reasons
                .iter()
                .any(|reason| reason.contains("selected runner active-remote"))
        );
    }

    #[tokio::test]
    async fn http_registry_registers_heartbeats_and_dispatches_active_runner() {
        let state = AppState {
            runners: Arc::new(RwLock::new(BTreeMap::new())),
            journal_path: None,
        };
        let app = registry_app(state);

        let mut goal = GoalSpec::new(
            "http dispatch",
            "dispatch through the registry HTTP surface to an active remote runner",
        );
        goal.default_execution.runner.locality = RunnerLocality::RemoteOnly;
        goal.default_execution
            .runner
            .required_capabilities
            .push(RunnerCapability::Code);
        goal.default_execution
            .runner
            .required_labels
            .insert("pool".to_string(), "default".to_string());
        let task = GoalState::new(goal).runnable_tasks().remove(0);

        let mut local_full = registration();
        local_full.runner_id = "local-full-http".to_string();
        local_full.node_id = "control-node".to_string();
        local_full.endpoint = "http://local-full-http:9091".to_string();
        local_full.capabilities = vec![RunnerCapability::Code];
        local_full.roles = vec![WorkerKind::Planner];
        local_full
            .labels
            .insert("pool".to_string(), "default".to_string());
        local_full.models = task.execution.model.candidates.clone();
        local_full.lease_ttl_seconds = 300;

        let mut stale_remote = local_full.clone();
        stale_remote.runner_id = "stale-remote-http".to_string();
        stale_remote.node_id = "worker-stale".to_string();
        stale_remote.endpoint = "http://stale-remote-http:9091".to_string();
        stale_remote.lease_ttl_seconds = 1;

        for registration in [&local_full, &stale_remote] {
            let response: RunnerRegistration =
                post_json(app.clone(), "/runners", registration).await;
            assert_eq!(response.runner_id, registration.runner_id);
        }

        let heartbeat_response: serde_json::Value = post_json(
            app.clone(),
            "/runners/heartbeat",
            &RunnerHeartbeat {
                runner_id: local_full.runner_id.clone(),
                node_id: local_full.node_id.clone(),
                running_tasks: 2,
                capacity_remaining: 0,
            },
        )
        .await;
        assert_eq!(heartbeat_response["known"], true);

        tokio::time::sleep(Duration::from_millis(1200)).await;

        let mut active_remote = local_full.clone();
        active_remote.runner_id = "active-remote-http".to_string();
        active_remote.node_id = "worker-active".to_string();
        active_remote.endpoint = "http://active-remote-http:9091".to_string();
        let response: RunnerRegistration = post_json(app.clone(), "/runners", &active_remote).await;
        assert_eq!(response.runner_id, "active-remote-http");

        let statuses: Vec<RunnerStatus> = get_json(app.clone(), "/runners/status").await;
        let status_by_runner: BTreeMap<String, RunnerStatus> = statuses
            .into_iter()
            .map(|status| (status.registration.runner_id.clone(), status))
            .collect();
        assert!(status_by_runner["local-full-http"].full);
        assert!(status_by_runner["stale-remote-http"].stale);
        assert!(status_by_runner["active-remote-http"].dispatchable);

        let decision: RunnerDispatchDecision = post_json(
            app,
            "/dispatch",
            &RunnerDispatchRequest {
                goal_id: task.goal_id,
                task,
                coordinator_node_id: Some("control-node".to_string()),
                registered_runners: Vec::new(),
            },
        )
        .await;

        assert_eq!(decision.status, RunnerDispatchStatus::Matched);
        assert_eq!(decision.runner_id.as_deref(), Some("active-remote-http"));
        assert_eq!(decision.candidates.len(), 1);
    }

    #[tokio::test]
    async fn http_capacity_plan_uses_runner_heartbeats_and_policy_limits() {
        let state = AppState {
            runners: Arc::new(RwLock::new(BTreeMap::new())),
            journal_path: None,
        };
        let app = registry_app(state);

        let mut registration = registration();
        registration.runner_id = "research-runner".to_string();
        registration.roles = vec![WorkerKind::Research];
        registration
            .labels
            .insert("pool".to_string(), "research".to_string());
        registration.max_concurrency = 2;
        let response: RunnerRegistration = post_json(app.clone(), "/runners", &registration).await;
        assert_eq!(response.runner_id, "research-runner");

        let _: serde_json::Value = post_json(
            app.clone(),
            "/runners/heartbeat",
            &RunnerHeartbeat {
                runner_id: registration.runner_id.clone(),
                node_id: registration.node_id.clone(),
                running_tasks: 2,
                capacity_remaining: 0,
            },
        )
        .await;

        let decision: RunnerScalingDecision = post_json(
            app,
            "/capacity/plan",
            &RunnerScalingRequest {
                generated_at_unix_seconds: 1,
                policy: CapacityScalingPolicy {
                    enabled: true,
                    mode: CapacityScalingMode::ProvisionEphemeral,
                    max_runners: 4,
                    max_scale_up_step: 1,
                    slots_per_runner: 2,
                    target_backlog_per_runner: 2,
                    ..CapacityScalingPolicy::default()
                },
                demands: vec![RunnerPoolDemand {
                    pool_key: "research".to_string(),
                    worker: Some(WorkerKind::Research),
                    required_capabilities: Vec::new(),
                    required_labels: BTreeMap::new(),
                    queued_tasks: 5,
                    running_tasks: 2,
                    blocked_tasks: 0,
                    unmatched_tasks: 0,
                    event_backlog: 1,
                    priority_boost: 0,
                }],
                supplies: Vec::new(),
            },
        )
        .await;

        assert_eq!(decision.status, RunnerScalingStatus::ProvisionRecommended);
        let pool = decision.pool_decisions.first().expect("pool decision");
        assert_eq!(pool.pool_key, "research");
        assert_eq!(pool.current_runners, 0);
        assert_eq!(pool.provision_runners, 1);
    }
}
