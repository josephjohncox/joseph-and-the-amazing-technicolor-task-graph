//! Queryable goal, task, plan, event, approval, and artifact projection service.
//!
//! Purpose: provide operator and dashboard read models without taking authority
//! away from Restate. The coordinator remains source of truth; this service is a
//! projection backed by JSONL for smoke tests or Postgres for production reads.
//!
//! Architecture references:
//! - `docs/design-docs/070-protobuf-goal-store-protocols.md`
//! - `infra/db/migrations/001_goal_store.sql`
//! - `docs/exec-plans/active/110-protobuf-goal-store.md`

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use anyhow::{Context, bail};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use coat_domain::{
    ApprovalRecord, ApprovalStatus, DurablePlan, DurablePlanListResponse, DurablePlanResponse,
    GoalArtifactRecord, GoalEventRecord, GoalId, GoalRecord, GoalStatus,
    GoalStoreApprovalListResponse, GoalStoreArtifactListResponse, GoalStoreArtifactRecordRequest,
    GoalStoreArtifactRecordResponse, GoalStoreEventAppendRequest, GoalStoreEventAppendResponse,
    GoalStoreEventListResponse, GoalStoreGoalResponse, GoalStoreSnapshot,
    GoalStoreSnapshotUpsertRequest, GoalStoreSnapshotUpsertResponse, GoalStoreTaskListResponse,
    PlanCompileRequest, PlanCompileResult, PlanDraftRequest, PlanId, PlanRevisionRequest,
    PlanStatus, TaskId, TaskRecord, TaskStatus, WorkerKind,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::RwLock};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct AppState {
    store: Arc<RwLock<GoalStore>>,
    journal_path: Option<PathBuf>,
    backend: GoalStoreBackend,
    postgres: Option<PgPool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoalStoreBackend {
    Memory,
    Jsonl,
    Postgres,
}

impl GoalStoreBackend {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "memory" => Ok(Self::Memory),
            "jsonl" => Ok(Self::Jsonl),
            "postgres" | "postgresql" | "postgres_pgvector" => Ok(Self::Postgres),
            other => bail!("unsupported COAT_GOAL_STORE_BACKEND {other:?}"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Jsonl => "jsonl",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    error: anyhow::Error,
}

impl ApiError {
    fn internal(error: impl Into<anyhow::Error>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: error.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({
            "accepted": false,
            "error": self.error.to_string(),
        }));
        (self.status, body).into_response()
    }
}

#[derive(Debug, Default, Clone, Serialize)]
struct GoalStore {
    goals: BTreeMap<GoalId, GoalRecord>,
    tasks: BTreeMap<TaskId, TaskRecord>,
    events: BTreeMap<GoalId, Vec<GoalEventRecord>>,
    artifacts: BTreeMap<GoalId, Vec<GoalArtifactRecord>>,
    approvals: BTreeMap<GoalId, Vec<ApprovalRecord>>,
    snapshots: BTreeMap<GoalId, GoalStoreSnapshot>,
    plans: BTreeMap<PlanId, DurablePlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum JournalEntry {
    Snapshot(GoalStoreSnapshotUpsertRequest),
    Event(GoalStoreEventAppendRequest),
    Artifact(GoalStoreArtifactRecordRequest),
    Plan(DurablePlan),
}

#[derive(Debug, Deserialize)]
struct GoalFilter {
    status: Option<Vec<GoalStatus>>,
    repo: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct PlanFilter {
    status: Option<Vec<PlanStatus>>,
    repo: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TaskFilter {
    goal_id: Option<GoalId>,
    status: Option<Vec<TaskStatus>>,
    role: Option<Vec<WorkerKind>>,
    subgoal_id: Option<String>,
    runnable: Option<bool>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct EventFilter {
    task_id: Option<TaskId>,
    after_sequence: Option<u64>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ApprovalFilter {
    goal_id: Option<GoalId>,
    status: Option<Vec<ApprovalStatus>>,
    limit: Option<usize>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "coat_goal_store=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9088".to_string());
    let journal_path = std::env::var("COAT_GOAL_STORE_JOURNAL_PATH")
        .or_else(|_| std::env::var("GOAL_STORE_JOURNAL_PATH"))
        .ok()
        .map(PathBuf::from);
    let backend_name = std::env::var("COAT_GOAL_STORE_BACKEND").unwrap_or_else(|_| {
        if journal_path.is_some() {
            "jsonl".to_string()
        } else {
            "memory".to_string()
        }
    });
    let backend = GoalStoreBackend::parse(&backend_name)?;

    let postgres = if backend == GoalStoreBackend::Postgres {
        let database_url = std::env::var("COAT_GOAL_STORE_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .context("COAT_GOAL_STORE_DATABASE_URL is required when backend=postgres")?;
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(&database_url)
            .await
            .context("connect to Postgres goal-store database")?;
        verify_postgres_schema(&pool).await?;
        Some(pool)
    } else {
        None
    };

    let store = if backend == GoalStoreBackend::Postgres {
        GoalStore::default()
    } else {
        replay_journal(journal_path.as_ref()).unwrap_or_else(|error| {
            tracing::warn!(%error, "goal-store journal replay failed; starting empty");
            GoalStore::default()
        })
    };
    let state = AppState {
        store: Arc::new(RwLock::new(store)),
        journal_path,
        backend,
        postgres,
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/goal-store/policy", get(policy))
        .route("/goal-store/snapshots", post(upsert_snapshot))
        .route("/goal-store/events", post(append_event))
        .route("/goal-store/artifacts", post(record_artifacts))
        .route("/goal-store/plans", get(list_plans).post(create_plan))
        .route("/goal-store/plans/{plan_id}", get(get_plan))
        .route("/goal-store/plans/{plan_id}/revisions", post(revise_plan))
        .route("/goal-store/plans/{plan_id}/compile", post(compile_plan))
        .route("/goal-store/goals", get(list_goals))
        .route("/goal-store/tasks", get(list_all_tasks))
        .route("/goal-store/approvals", get(list_all_approvals))
        .route("/goal-store/goals/{goal_id}", get(get_goal))
        .route("/goal-store/goals/{goal_id}/tasks", get(list_tasks))
        .route("/goal-store/goals/{goal_id}/events", get(list_events))
        .route("/goal-store/goals/{goal_id}/artifacts", get(list_artifacts))
        .route("/goal-store/goals/{goal_id}/approvals", get(list_approvals))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "goal store listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "authority": "restate_workflow",
        "backend": state.backend.as_str(),
        "postgres_connected": state.postgres.is_some(),
    }))
}

async fn policy(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "state_authority": "restate_workflow",
        "read_model_backend": state.backend.as_str(),
        "production_read_model": "postgres",
        "production_vector_extension": "pgvector",
        "protocol_package": "coat.v1",
        "proto_root": "proto/coat/v1",
    }))
}

async fn upsert_snapshot(
    State(state): State<AppState>,
    Json(request): Json<GoalStoreSnapshotUpsertRequest>,
) -> Result<Json<GoalStoreSnapshotUpsertResponse>, ApiError> {
    if let Some(pool) = &state.postgres {
        upsert_snapshot_postgres(pool, &request)
            .await
            .map_err(ApiError::internal)?;
    } else if let Err(error) = append_journal(&state, JournalEntry::Snapshot(request.clone())).await
    {
        tracing::warn!(%error, "append snapshot journal failed");
    }

    let mut store = state.store.write().await;
    store.apply_snapshot(request.snapshot.clone());
    Ok(Json(GoalStoreSnapshotUpsertResponse {
        accepted: true,
        goal: request.snapshot.goal,
        task_count: request.snapshot.tasks.len() as u32,
    }))
}

async fn append_event(
    State(state): State<AppState>,
    Json(request): Json<GoalStoreEventAppendRequest>,
) -> Result<Json<GoalStoreEventAppendResponse>, ApiError> {
    if let Some(pool) = &state.postgres {
        append_event_postgres(pool, &request.event)
            .await
            .map_err(ApiError::internal)?;
    } else if let Err(error) = append_journal(&state, JournalEntry::Event(request.clone())).await {
        tracing::warn!(%error, "append event journal failed");
    }

    let mut store = state.store.write().await;
    let sequence = request.event.sequence;
    store.apply_event(request.event);
    Ok(Json(GoalStoreEventAppendResponse {
        accepted: true,
        sequence,
    }))
}

async fn record_artifacts(
    State(state): State<AppState>,
    Json(request): Json<GoalStoreArtifactRecordRequest>,
) -> Result<Json<GoalStoreArtifactRecordResponse>, ApiError> {
    let records = request.clone().into_records();
    if let Some(pool) = &state.postgres {
        record_artifacts_postgres(pool, &records)
            .await
            .map_err(ApiError::internal)?;
    } else if let Err(error) = append_journal(&state, JournalEntry::Artifact(request)).await {
        tracing::warn!(%error, "append artifact journal failed");
    }

    let mut store = state.store.write().await;
    store.apply_artifacts(records.clone());
    Ok(Json(GoalStoreArtifactRecordResponse {
        accepted: true,
        artifact_count: records.len() as u32,
    }))
}

async fn get_goal(
    State(state): State<AppState>,
    Path(goal_id): Path<Uuid>,
) -> Result<Json<GoalStoreGoalResponse>, ApiError> {
    let goal = if let Some(pool) = &state.postgres {
        get_goal_postgres(pool, goal_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        state.store.read().await.goals.get(&goal_id).cloned()
    };
    Ok(Json(GoalStoreGoalResponse {
        found: goal.is_some(),
        goal,
    }))
}

async fn create_plan(
    State(state): State<AppState>,
    Json(request): Json<PlanDraftRequest>,
) -> Result<Json<DurablePlanResponse>, ApiError> {
    let plan = DurablePlan::draft(request);
    upsert_plan_record(&state, &plan)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(DurablePlanResponse {
        found: true,
        plan: Some(plan),
    }))
}

async fn list_plans(
    State(state): State<AppState>,
    Query(filter): Query<PlanFilter>,
) -> Result<Json<DurablePlanListResponse>, ApiError> {
    let plans = if let Some(pool) = &state.postgres {
        list_plans_postgres(pool)
            .await
            .map_err(ApiError::internal)?
    } else {
        state.store.read().await.plans.values().cloned().collect()
    };
    Ok(Json(DurablePlanListResponse {
        plans: filter_plans(plans, &filter)
            .into_iter()
            .map(|plan| plan.summary())
            .collect(),
    }))
}

async fn get_plan(
    State(state): State<AppState>,
    Path(plan_id): Path<Uuid>,
) -> Result<Json<DurablePlanResponse>, ApiError> {
    let plan = if let Some(pool) = &state.postgres {
        get_plan_postgres(pool, plan_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        state.store.read().await.plans.get(&plan_id).cloned()
    };
    Ok(Json(DurablePlanResponse {
        found: plan.is_some(),
        plan,
    }))
}

async fn revise_plan(
    State(state): State<AppState>,
    Path(plan_id): Path<Uuid>,
    Json(request): Json<PlanRevisionRequest>,
) -> Result<Json<DurablePlanResponse>, ApiError> {
    let mut plan = load_plan(&state, plan_id).await?.ok_or_else(|| ApiError {
        status: StatusCode::NOT_FOUND,
        error: anyhow::anyhow!("plan {plan_id} not found"),
    })?;
    plan.apply_revision(request);
    upsert_plan_record(&state, &plan)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(DurablePlanResponse {
        found: true,
        plan: Some(plan),
    }))
}

async fn compile_plan(
    State(state): State<AppState>,
    Path(plan_id): Path<Uuid>,
    Json(mut request): Json<PlanCompileRequest>,
) -> Result<Json<PlanCompileResult>, ApiError> {
    let mut plan = load_plan(&state, plan_id).await?.ok_or_else(|| ApiError {
        status: StatusCode::NOT_FOUND,
        error: anyhow::anyhow!("plan {plan_id} not found"),
    })?;
    request.plan_id = Some(plan_id);
    let result = plan.compile_goal(request);
    upsert_plan_record(&state, &plan)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(result))
}

async fn list_goals(
    State(state): State<AppState>,
    Query(filter): Query<GoalFilter>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let goals = if let Some(pool) = &state.postgres {
        list_goals_postgres(pool)
            .await
            .map_err(ApiError::internal)?
    } else {
        state.store.read().await.goals.values().cloned().collect()
    };
    Ok(Json(serde_json::json!({
        "goals": filter_goals(goals, &filter),
    })))
}

async fn list_all_tasks(
    State(state): State<AppState>,
    Query(filter): Query<TaskFilter>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tasks = if let Some(pool) = &state.postgres {
        list_tasks_postgres_all(pool)
            .await
            .map_err(ApiError::internal)?
    } else {
        state.store.read().await.tasks.values().cloned().collect()
    };
    Ok(Json(serde_json::json!({
        "tasks": filter_tasks(tasks, &filter),
    })))
}

async fn list_all_approvals(
    State(state): State<AppState>,
    Query(filter): Query<ApprovalFilter>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let approvals = if let Some(pool) = &state.postgres {
        list_approvals_postgres_all(pool)
            .await
            .map_err(ApiError::internal)?
    } else {
        state
            .store
            .read()
            .await
            .approvals
            .values()
            .flatten()
            .cloned()
            .collect()
    };
    Ok(Json(serde_json::json!({
        "approvals": filter_approvals(approvals, &filter),
    })))
}

async fn list_tasks(
    State(state): State<AppState>,
    Path(goal_id): Path<Uuid>,
    Query(mut filter): Query<TaskFilter>,
) -> Result<Json<GoalStoreTaskListResponse>, ApiError> {
    filter.goal_id = Some(goal_id);
    let tasks = if let Some(pool) = &state.postgres {
        list_tasks_postgres(pool, goal_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        state
            .store
            .read()
            .await
            .tasks
            .values()
            .filter(|task| task.goal_id == goal_id)
            .cloned()
            .collect()
    };
    Ok(Json(GoalStoreTaskListResponse {
        goal_id,
        tasks: filter_tasks(tasks, &filter),
    }))
}

async fn list_approvals(
    State(state): State<AppState>,
    Path(goal_id): Path<Uuid>,
    Query(mut filter): Query<ApprovalFilter>,
) -> Result<Json<GoalStoreApprovalListResponse>, ApiError> {
    filter.goal_id = Some(goal_id);
    let approvals = if let Some(pool) = &state.postgres {
        list_approvals_postgres(pool, goal_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        state
            .store
            .read()
            .await
            .approvals
            .get(&goal_id)
            .cloned()
            .unwrap_or_default()
    };
    Ok(Json(GoalStoreApprovalListResponse {
        goal_id,
        approvals: filter_approvals(approvals, &filter),
    }))
}

async fn list_events(
    State(state): State<AppState>,
    Path(goal_id): Path<Uuid>,
    Query(filter): Query<EventFilter>,
) -> Result<Json<GoalStoreEventListResponse>, ApiError> {
    let events = if let Some(pool) = &state.postgres {
        list_events_postgres(pool, goal_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        state
            .store
            .read()
            .await
            .events
            .get(&goal_id)
            .cloned()
            .unwrap_or_default()
    };
    Ok(Json(GoalStoreEventListResponse {
        goal_id,
        events: filter_events(events, &filter),
    }))
}

async fn list_artifacts(
    State(state): State<AppState>,
    Path(goal_id): Path<Uuid>,
) -> Result<Json<GoalStoreArtifactListResponse>, ApiError> {
    let artifacts = if let Some(pool) = &state.postgres {
        list_artifacts_postgres(pool, goal_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        state
            .store
            .read()
            .await
            .artifacts
            .get(&goal_id)
            .cloned()
            .unwrap_or_default()
    };
    Ok(Json(GoalStoreArtifactListResponse { goal_id, artifacts }))
}

fn filter_goals(mut goals: Vec<GoalRecord>, filter: &GoalFilter) -> Vec<GoalRecord> {
    goals.retain(|goal| {
        filter
            .status
            .as_ref()
            .is_none_or(|statuses| statuses.contains(&goal.status))
            && filter
                .repo
                .as_ref()
                .is_none_or(|repo| goal.repo.as_ref() == Some(repo))
    });
    goals.sort_by(|left, right| {
        format!("{:?}", left.status)
            .cmp(&format!("{:?}", right.status))
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.goal_id.cmp(&right.goal_id))
    });
    if let Some(limit) = filter.limit {
        goals.truncate(limit);
    }
    goals
}

fn filter_plans(mut plans: Vec<DurablePlan>, filter: &PlanFilter) -> Vec<DurablePlan> {
    plans.retain(|plan| {
        filter
            .status
            .as_ref()
            .is_none_or(|statuses| statuses.contains(&plan.status))
            && filter
                .repo
                .as_ref()
                .is_none_or(|repo| plan.repo.as_ref() == Some(repo))
    });
    plans.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.id.cmp(&right.id))
    });
    if let Some(limit) = filter.limit {
        plans.truncate(limit);
    }
    plans
}

fn filter_tasks(mut tasks: Vec<TaskRecord>, filter: &TaskFilter) -> Vec<TaskRecord> {
    tasks.retain(|task| {
        filter.goal_id.is_none_or(|goal_id| task.goal_id == goal_id)
            && filter
                .status
                .as_ref()
                .is_none_or(|statuses| statuses.contains(&task.status))
            && filter
                .role
                .as_ref()
                .is_none_or(|roles| roles.contains(&task.role))
            && filter
                .subgoal_id
                .as_ref()
                .is_none_or(|subgoal_id| task.subgoal_id.as_ref() == Some(subgoal_id))
            && filter
                .runnable
                .is_none_or(|runnable| task.runnable == runnable)
    });
    tasks.sort_by(|left, right| {
        right
            .priority_rank
            .cmp(&left.priority_rank)
            .then_with(|| left.depth.cmp(&right.depth))
            .then_with(|| left.task_id.cmp(&right.task_id))
    });
    if let Some(limit) = filter.limit {
        tasks.truncate(limit);
    }
    tasks
}

fn filter_approvals(
    mut approvals: Vec<ApprovalRecord>,
    filter: &ApprovalFilter,
) -> Vec<ApprovalRecord> {
    approvals.retain(|approval| {
        filter
            .goal_id
            .is_none_or(|goal_id| approval.goal_id == goal_id)
            && filter
                .status
                .as_ref()
                .is_none_or(|statuses| statuses.contains(&approval.status))
    });
    approvals.sort_by(|left, right| {
        left.goal_id
            .cmp(&right.goal_id)
            .then_with(|| left.task_id.cmp(&right.task_id))
            .then_with(|| left.approval_id.cmp(&right.approval_id))
    });
    if let Some(limit) = filter.limit {
        approvals.truncate(limit);
    }
    approvals
}

fn filter_events(mut events: Vec<GoalEventRecord>, filter: &EventFilter) -> Vec<GoalEventRecord> {
    events.retain(|event| {
        filter
            .task_id
            .is_none_or(|task_id| event.task_id == Some(task_id))
            && filter
                .after_sequence
                .is_none_or(|after| event.sequence > after)
    });
    events.sort_by_key(|event| event.sequence);
    if let Some(limit) = filter.limit {
        events.truncate(limit);
    }
    events
}

async fn verify_postgres_schema(pool: &PgPool) -> anyhow::Result<()> {
    let table: Option<String> = sqlx::query_scalar("SELECT to_regclass('coat.goals')::text")
        .fetch_one(pool)
        .await
        .context("check coat.goals table")?;
    if table.is_none() {
        bail!("coat.goals table missing; run infra/db/migrations before starting goal-store");
    }
    Ok(())
}

async fn upsert_snapshot_postgres(
    pool: &PgPool,
    request: &GoalStoreSnapshotUpsertRequest,
) -> anyhow::Result<()> {
    let snapshot = &request.snapshot;
    let goal = &snapshot.goal;
    let mut tx = pool
        .begin()
        .await
        .context("begin goal snapshot transaction")?;

    sqlx::query(
        r#"
        INSERT INTO coat.goals (
            id, title, objective, repo, status, total_tasks, open_tasks, blocked_tasks,
            failed_tasks, percent_done, root_task_id, satisfied, satisfaction_score,
            updated_at_text, payload_json, full_state_json, record_json, protocol_json,
            projection_reason, projected_at
        )
        VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
            $11, $12, $13, $14, $15, $16, $17, $18, $19, now()
        )
        ON CONFLICT (id) DO UPDATE SET
            title = EXCLUDED.title,
            objective = EXCLUDED.objective,
            repo = EXCLUDED.repo,
            status = EXCLUDED.status,
            total_tasks = EXCLUDED.total_tasks,
            open_tasks = EXCLUDED.open_tasks,
            blocked_tasks = EXCLUDED.blocked_tasks,
            failed_tasks = EXCLUDED.failed_tasks,
            percent_done = EXCLUDED.percent_done,
            root_task_id = EXCLUDED.root_task_id,
            satisfied = EXCLUDED.satisfied,
            satisfaction_score = EXCLUDED.satisfaction_score,
            updated_at_text = EXCLUDED.updated_at_text,
            payload_json = EXCLUDED.payload_json,
            full_state_json = EXCLUDED.full_state_json,
            record_json = EXCLUDED.record_json,
            protocol_json = EXCLUDED.protocol_json,
            projection_reason = EXCLUDED.projection_reason,
            projected_at = now(),
            version = coat.goals.version + 1
        "#,
    )
    .bind(goal.goal_id)
    .bind(&goal.title)
    .bind(&goal.objective)
    .bind(&goal.repo)
    .bind(json_string(&goal.status)?)
    .bind(as_i32(goal.total_tasks, "goal.total_tasks")?)
    .bind(as_i32(goal.open_tasks, "goal.open_tasks")?)
    .bind(as_i32(goal.blocked_tasks, "goal.blocked_tasks")?)
    .bind(as_i32(goal.failed_tasks, "goal.failed_tasks")?)
    .bind(goal.percent_done)
    .bind(goal.root_task_id)
    .bind(goal.satisfied)
    .bind(goal.satisfaction_score)
    .bind(&goal.updated_at)
    .bind(goal.payload_json.clone())
    .bind(snapshot.full_state_json.clone())
    .bind(serde_json::to_value(goal)?)
    .bind(serde_json::to_value(&request.metadata)?)
    .bind(&request.projection_reason)
    .execute(&mut *tx)
    .await
    .context("upsert goal record")?;

    sqlx::query("DELETE FROM coat.goal_events WHERE goal_id = $1")
        .bind(goal.goal_id)
        .execute(&mut *tx)
        .await
        .context("delete goal events before snapshot projection")?;
    sqlx::query("DELETE FROM coat.artifacts WHERE goal_id = $1")
        .bind(goal.goal_id)
        .execute(&mut *tx)
        .await
        .context("delete artifacts before snapshot projection")?;
    sqlx::query("DELETE FROM coat.approvals WHERE goal_id = $1")
        .bind(goal.goal_id)
        .execute(&mut *tx)
        .await
        .context("delete approvals before snapshot projection")?;
    sqlx::query("DELETE FROM coat.tasks WHERE goal_id = $1")
        .bind(goal.goal_id)
        .execute(&mut *tx)
        .await
        .context("delete tasks before snapshot projection")?;

    for task in &snapshot.tasks {
        sqlx::query(
            r#"
            INSERT INTO coat.tasks (
                id, goal_id, parent_task_id, subgoal_id, title, role, status, purpose_kind,
                depth, priority, priority_rank, attempts, runnable, tags, result_uri, payload_json,
                record_json
            )
            VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8,
                $9, $10, $11, $12, $13, $14, $15, $16, $17
            )
            "#,
        )
        .bind(task.task_id)
        .bind(task.goal_id)
        .bind(task.parent_task_id)
        .bind(&task.subgoal_id)
        .bind(&task.title)
        .bind(json_string(&task.role)?)
        .bind(json_string(&task.status)?)
        .bind(json_string(&task.purpose_kind)?)
        .bind(as_i32(task.depth, "task.depth")?)
        .bind(json_string(&task.priority)?)
        .bind(i16::from(task.priority_rank))
        .bind(as_i32(task.attempts, "task.attempts")?)
        .bind(task.runnable)
        .bind(&task.tags)
        .bind(&task.result_uri)
        .bind(task.payload_json.clone())
        .bind(serde_json::to_value(task)?)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("insert task {}", task.task_id))?;
    }

    for approval in &snapshot.approvals {
        sqlx::query(
            r#"
            INSERT INTO coat.approvals (
                id, goal_id, task_id, status, risk, reason, requested_action,
                updated_at_text, payload_json, record_json
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(approval.approval_id)
        .bind(approval.goal_id)
        .bind(approval.task_id)
        .bind(json_string(&approval.status)?)
        .bind(json_string(&approval.risk)?)
        .bind(&approval.reason)
        .bind(&approval.requested_action)
        .bind(&approval.updated_at)
        .bind(approval.payload_json.clone())
        .bind(serde_json::to_value(approval)?)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("insert approval {}", approval.approval_id))?;
    }

    for artifact in &snapshot.artifacts {
        sqlx::query(
            r#"
            INSERT INTO coat.artifacts (
                id, goal_id, task_id, artifact_type, uri, description, git_remote, git_ref,
                git_commit_sha, object_bucket, object_key, sha256, created_at_text, payload_json,
                record_json
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            "#,
        )
        .bind(artifact_record_id(artifact))
        .bind(artifact.goal_id)
        .bind(artifact.task_id)
        .bind(json_string(&artifact.artifact.kind)?)
        .bind(&artifact.artifact.uri)
        .bind(&artifact.artifact.description)
        .bind(
            artifact
                .git_result
                .as_ref()
                .and_then(|git| git.remote.clone()),
        )
        .bind(artifact.git_result.as_ref().map(|git| git.branch.clone()))
        .bind(
            artifact
                .git_result
                .as_ref()
                .and_then(|git| git.commit.clone()),
        )
        .bind(
            artifact
                .object_artifact
                .as_ref()
                .map(|object| object.store.bucket.clone()),
        )
        .bind(
            artifact
                .object_artifact
                .as_ref()
                .map(|object| object.key.clone()),
        )
        .bind(&artifact.artifact.sha256)
        .bind(&artifact.created_at)
        .bind(artifact.payload_json.clone())
        .bind(serde_json::to_value(artifact)?)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("insert artifact {}", artifact.artifact.uri))?;
    }

    for event in &snapshot.events {
        insert_event_postgres(&mut tx, event).await?;
    }

    tx.commit()
        .await
        .context("commit goal snapshot transaction")?;
    Ok(())
}

async fn append_event_postgres(pool: &PgPool, event: &GoalEventRecord) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("begin event transaction")?;
    insert_event_postgres(&mut tx, event).await?;
    tx.commit().await.context("commit event transaction")?;
    Ok(())
}

async fn record_artifacts_postgres(
    pool: &PgPool,
    records: &[GoalArtifactRecord],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("begin artifact transaction")?;
    for artifact in records {
        insert_artifact_postgres(&mut tx, artifact).await?;
    }
    tx.commit().await.context("commit artifact transaction")?;
    Ok(())
}

async fn insert_artifact_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    artifact: &GoalArtifactRecord,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO coat.artifacts (
            id, goal_id, task_id, artifact_type, uri, description, git_remote, git_ref,
            git_commit_sha, object_bucket, object_key, sha256, created_at_text, payload_json,
            record_json
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        ON CONFLICT (id) DO UPDATE SET
            artifact_type = EXCLUDED.artifact_type,
            uri = EXCLUDED.uri,
            description = EXCLUDED.description,
            git_remote = EXCLUDED.git_remote,
            git_ref = EXCLUDED.git_ref,
            git_commit_sha = EXCLUDED.git_commit_sha,
            object_bucket = EXCLUDED.object_bucket,
            object_key = EXCLUDED.object_key,
            sha256 = EXCLUDED.sha256,
            created_at_text = EXCLUDED.created_at_text,
            payload_json = EXCLUDED.payload_json,
            record_json = EXCLUDED.record_json
        "#,
    )
    .bind(artifact_record_id(artifact))
    .bind(artifact.goal_id)
    .bind(artifact.task_id)
    .bind(json_string(&artifact.artifact.kind)?)
    .bind(&artifact.artifact.uri)
    .bind(&artifact.artifact.description)
    .bind(
        artifact
            .git_result
            .as_ref()
            .and_then(|git| git.remote.clone()),
    )
    .bind(artifact.git_result.as_ref().map(|git| git.branch.clone()))
    .bind(
        artifact
            .git_result
            .as_ref()
            .and_then(|git| git.commit.clone()),
    )
    .bind(
        artifact
            .object_artifact
            .as_ref()
            .map(|object| object.store.bucket.clone()),
    )
    .bind(
        artifact
            .object_artifact
            .as_ref()
            .map(|object| object.key.clone()),
    )
    .bind(&artifact.artifact.sha256)
    .bind(&artifact.created_at)
    .bind(artifact.payload_json.clone())
    .bind(serde_json::to_value(artifact)?)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("insert artifact {}", artifact.artifact.uri))?;
    Ok(())
}

async fn load_plan(state: &AppState, plan_id: PlanId) -> Result<Option<DurablePlan>, ApiError> {
    if let Some(pool) = &state.postgres {
        get_plan_postgres(pool, plan_id)
            .await
            .map_err(ApiError::internal)
    } else {
        Ok(state.store.read().await.plans.get(&plan_id).cloned())
    }
}

async fn upsert_plan_record(state: &AppState, plan: &DurablePlan) -> anyhow::Result<()> {
    if let Some(pool) = &state.postgres {
        upsert_plan_postgres(pool, plan).await?;
    } else if let Err(error) = append_journal(state, JournalEntry::Plan(plan.clone())).await {
        tracing::warn!(%error, "append plan journal failed");
    }
    state.store.write().await.apply_plan(plan.clone());
    Ok(())
}

async fn insert_event_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &GoalEventRecord,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO coat.goal_events (
            id, goal_id, task_id, sequence, kind, message, actor, idempotency_key,
            created_at_text, payload_json, record_json
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (goal_id, sequence) DO UPDATE SET
            id = EXCLUDED.id,
            task_id = EXCLUDED.task_id,
            kind = EXCLUDED.kind,
            message = EXCLUDED.message,
            actor = EXCLUDED.actor,
            idempotency_key = EXCLUDED.idempotency_key,
            created_at_text = EXCLUDED.created_at_text,
            payload_json = EXCLUDED.payload_json,
            record_json = EXCLUDED.record_json
        "#,
    )
    .bind(event.event_id)
    .bind(event.goal_id)
    .bind(event.task_id)
    .bind(as_i64(event.sequence, "event.sequence")?)
    .bind(json_string(&event.kind)?)
    .bind(&event.message)
    .bind(&event.actor)
    .bind(&event.idempotency_key)
    .bind(&event.created_at)
    .bind(event.payload_json.clone())
    .bind(serde_json::to_value(event)?)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("insert event {}", event.event_id))?;
    Ok(())
}

async fn get_goal_postgres(pool: &PgPool, goal_id: GoalId) -> anyhow::Result<Option<GoalRecord>> {
    let row = sqlx::query("SELECT record_json FROM coat.goals WHERE id = $1")
        .bind(goal_id)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("query goal {goal_id}"))?;
    row.map(|row| decode_record(row.try_get("record_json")?, "goal"))
        .transpose()
}

async fn upsert_plan_postgres(pool: &PgPool, plan: &DurablePlan) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO coat.plans (
            id, title, objective, repo, status, mode, version, compiled_goal_id,
            updated_at_text, record_json
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (id) DO UPDATE SET
            title = EXCLUDED.title,
            objective = EXCLUDED.objective,
            repo = EXCLUDED.repo,
            status = EXCLUDED.status,
            mode = EXCLUDED.mode,
            version = EXCLUDED.version,
            compiled_goal_id = EXCLUDED.compiled_goal_id,
            updated_at_text = EXCLUDED.updated_at_text,
            record_json = EXCLUDED.record_json,
            projected_at = now()
        "#,
    )
    .bind(plan.id)
    .bind(&plan.title)
    .bind(&plan.objective)
    .bind(&plan.repo)
    .bind(json_string(&plan.status)?)
    .bind(json_string(&plan.mode)?)
    .bind(as_i32(plan.version, "plan.version")?)
    .bind(plan.compiled_goal_id)
    .bind(&plan.updated_at)
    .bind(serde_json::to_value(plan)?)
    .execute(pool)
    .await
    .with_context(|| format!("upsert plan {}", plan.id))?;
    Ok(())
}

async fn get_plan_postgres(pool: &PgPool, plan_id: PlanId) -> anyhow::Result<Option<DurablePlan>> {
    let row = sqlx::query("SELECT record_json FROM coat.plans WHERE id = $1")
        .bind(plan_id)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("query plan {plan_id}"))?;
    row.map(|row| decode_record(row.try_get("record_json")?, "plan"))
        .transpose()
}

async fn list_plans_postgres(pool: &PgPool) -> anyhow::Result<Vec<DurablePlan>> {
    let rows =
        sqlx::query("SELECT record_json FROM coat.plans ORDER BY projected_at DESC, title ASC")
            .fetch_all(pool)
            .await
            .context("query plans")?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "plan"))
        .collect()
}

async fn list_goals_postgres(pool: &PgPool) -> anyhow::Result<Vec<GoalRecord>> {
    let rows =
        sqlx::query("SELECT record_json FROM coat.goals ORDER BY projected_at DESC, title ASC")
            .fetch_all(pool)
            .await
            .context("query goals")?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "goal"))
        .collect()
}

async fn list_tasks_postgres(pool: &PgPool, goal_id: GoalId) -> anyhow::Result<Vec<TaskRecord>> {
    let rows = sqlx::query(
        "SELECT record_json FROM coat.tasks WHERE goal_id = $1 ORDER BY priority_rank DESC, depth ASC, id ASC",
    )
    .bind(goal_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("query tasks for goal {goal_id}"))?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "task"))
        .collect()
}

async fn list_tasks_postgres_all(pool: &PgPool) -> anyhow::Result<Vec<TaskRecord>> {
    let rows = sqlx::query(
        "SELECT record_json FROM coat.tasks ORDER BY goal_id ASC, priority_rank DESC, depth ASC, id ASC",
    )
    .fetch_all(pool)
    .await
    .context("query all tasks")?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "task"))
        .collect()
}

async fn list_approvals_postgres(
    pool: &PgPool,
    goal_id: GoalId,
) -> anyhow::Result<Vec<ApprovalRecord>> {
    let rows = sqlx::query(
        "SELECT record_json FROM coat.approvals WHERE goal_id = $1 ORDER BY created_at DESC, id ASC",
    )
    .bind(goal_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("query approvals for goal {goal_id}"))?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "approval"))
        .collect()
}

async fn list_approvals_postgres_all(pool: &PgPool) -> anyhow::Result<Vec<ApprovalRecord>> {
    let rows =
        sqlx::query("SELECT record_json FROM coat.approvals ORDER BY created_at DESC, id ASC")
            .fetch_all(pool)
            .await
            .context("query approvals")?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "approval"))
        .collect()
}

async fn list_events_postgres(
    pool: &PgPool,
    goal_id: GoalId,
) -> anyhow::Result<Vec<GoalEventRecord>> {
    let rows = sqlx::query(
        "SELECT record_json FROM coat.goal_events WHERE goal_id = $1 ORDER BY sequence",
    )
    .bind(goal_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("query events for goal {goal_id}"))?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "event"))
        .collect()
}

async fn list_artifacts_postgres(
    pool: &PgPool,
    goal_id: GoalId,
) -> anyhow::Result<Vec<GoalArtifactRecord>> {
    let rows = sqlx::query(
        "SELECT record_json FROM coat.artifacts WHERE goal_id = $1 ORDER BY recorded_at",
    )
    .bind(goal_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("query artifacts for goal {goal_id}"))?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "artifact"))
        .collect()
}

fn decode_record<T: DeserializeOwned>(value: serde_json::Value, kind: &str) -> anyhow::Result<T> {
    serde_json::from_value(value).with_context(|| format!("decode {kind} record_json"))
}

fn json_string(value: &impl Serialize) -> anyhow::Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(value) => Ok(value),
        other => bail!("expected enum to serialize as string, got {other}"),
    }
}

fn as_i32(value: u32, field: &str) -> anyhow::Result<i32> {
    i32::try_from(value).with_context(|| format!("{field} does not fit into i32"))
}

fn as_i64(value: u64, field: &str) -> anyhow::Result<i64> {
    i64::try_from(value).with_context(|| format!("{field} does not fit into i64"))
}

fn artifact_record_id(record: &GoalArtifactRecord) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "coat://goal/{}/artifact/{:?}/{}",
            record.goal_id, record.task_id, record.artifact.uri
        )
        .as_bytes(),
    )
}

impl GoalStore {
    fn apply_snapshot(&mut self, snapshot: GoalStoreSnapshot) {
        let goal_id = snapshot.goal.goal_id;
        self.goals.insert(goal_id, snapshot.goal.clone());
        for task in &snapshot.tasks {
            self.tasks.insert(task.task_id, task.clone());
        }
        self.artifacts.insert(goal_id, snapshot.artifacts.clone());
        self.approvals.insert(goal_id, snapshot.approvals.clone());
        self.events.insert(goal_id, snapshot.events.clone());
        self.snapshots.insert(goal_id, snapshot);
    }

    fn apply_event(&mut self, event: GoalEventRecord) {
        self.events.entry(event.goal_id).or_default().push(event);
    }

    fn apply_artifacts(&mut self, records: Vec<GoalArtifactRecord>) {
        for record in records {
            self.artifacts
                .entry(record.goal_id)
                .or_default()
                .push(record);
        }
    }

    fn apply_plan(&mut self, plan: DurablePlan) {
        self.plans.insert(plan.id, plan);
    }
}

fn replay_journal(path: Option<&PathBuf>) -> anyhow::Result<GoalStore> {
    let Some(path) = path else {
        return Ok(GoalStore::default());
    };
    if !path.exists() {
        return Ok(GoalStore::default());
    }
    let mut store = GoalStore::default();
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: JournalEntry = serde_json::from_str(line)
            .with_context(|| format!("parse {} line {}", path.display(), index + 1))?;
        match entry {
            JournalEntry::Snapshot(request) => store.apply_snapshot(request.snapshot),
            JournalEntry::Event(request) => store.apply_event(request.event),
            JournalEntry::Artifact(request) => store.apply_artifacts(request.into_records()),
            JournalEntry::Plan(plan) => store.apply_plan(plan),
        }
    }
    Ok(store)
}

async fn append_journal(state: &AppState, entry: JournalEntry) -> anyhow::Result<()> {
    let Some(path) = &state.journal_path else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    let line = serde_json::to_string(&entry)?;
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use coat_domain::{
        ArtifactKind, ArtifactRef, GoalSpec, GoalState, GoalStoreArtifactRecordRequest,
        GoalStoreSnapshotUpsertRequest, PlanDraftRequest, ProtocolMetadata,
    };

    use super::GoalStore;

    #[test]
    fn snapshot_indexes_goal_tasks_and_events() {
        let state = GoalState::new(GoalSpec::new(
            "Index goal",
            "Index this durable goal into the local read model",
        ));
        let request = GoalStoreSnapshotUpsertRequest::from_state(&state, "test_projection");
        let goal_id = request.snapshot.goal.goal_id;
        let task_id = request.snapshot.tasks[0].task_id;
        let mut store = GoalStore::default();

        store.apply_snapshot(request.snapshot);

        assert!(store.goals.contains_key(&goal_id));
        assert!(store.tasks.contains_key(&task_id));
        assert!(!store.events[&goal_id].is_empty());
    }

    #[test]
    fn plan_projection_indexes_durable_plan() {
        let request = PlanDraftRequest {
            plan_id: None,
            title: "Plan first".to_string(),
            objective: "Create a durable plan before compiling a goal.".to_string(),
            repo: None,
            prompt: "Work in planning mode.".to_string(),
            mode: Default::default(),
            status: None,
            author: None,
            summary: None,
            authoring: Default::default(),
            plan: Default::default(),
            initial_tasks: Vec::new(),
            questions: Vec::new(),
            decisions: Vec::new(),
        };
        let plan = coat_domain::DurablePlan::draft(request);
        let plan_id = plan.id;
        let mut store = GoalStore::default();

        store.apply_plan(plan);

        assert!(store.plans.contains_key(&plan_id));
    }

    #[test]
    fn artifact_record_request_indexes_artifacts() {
        let goal_id = uuid::Uuid::new_v4();
        let task_id = uuid::Uuid::new_v4();
        let request = GoalStoreArtifactRecordRequest {
            metadata: ProtocolMetadata::new(format!("goal:{goal_id}:artifact:test")),
            goal_id,
            task_id: Some(task_id),
            artifacts: vec![ArtifactRef {
                kind: ArtifactKind::Report,
                uri: "workspace://artifact/report.json".to_string(),
                description: "test report".to_string(),
                sha256: Some("abc123".to_string()),
            }],
            git_results: Vec::new(),
            object_artifacts: Vec::new(),
        };
        let records = request.into_records();
        let mut store = GoalStore::default();

        store.apply_artifacts(records);

        assert_eq!(store.artifacts[&goal_id].len(), 1);
        assert_eq!(store.artifacts[&goal_id][0].task_id, Some(task_id));
        assert_eq!(
            store.artifacts[&goal_id][0].artifact.uri,
            "workspace://artifact/report.json"
        );
    }
}
