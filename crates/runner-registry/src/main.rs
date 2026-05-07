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
    RunnerDispatchDecision, RunnerDispatchRequest, RunnerHeartbeat, RunnerRegistration,
    RunnerStatus,
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
        let line = serde_json::to_string(&entry)?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
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
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: RunnerJournalEntry = serde_json::from_str(line).map_err(|error| {
            anyhow::anyhow!("invalid runner journal line {}: {error}", index + 1)
        })?;
        apply_journal_entry(&mut runners, entry);
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
        time::{Duration, Instant},
    };

    use coat_domain::RunnerRegistration;

    use super::{RunnerJournalEntry, RunnerRecord, apply_journal_entry, replay_journal, unix_now};

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
}
