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
    ApprovalRecord, ApprovalStatus, CheckpointRef, DurableEventEnvelope, DurablePlan,
    DurablePlanListResponse, DurablePlanResponse, EventSourceApprovalListResponse,
    EventSourceApprovalRecord, EventSourceApprovalRecordRequest, EventSourceApprovalRecordResponse,
    EventSourceApprovalStatus, GoalArtifactRecord, GoalEventRecord, GoalId, GoalRecord, GoalStatus,
    GoalStoreApprovalListResponse, GoalStoreArtifactListResponse, GoalStoreArtifactRecordRequest,
    GoalStoreArtifactRecordResponse, GoalStoreCheckpointListResponse, GoalStoreEventAppendRequest,
    GoalStoreEventAppendResponse, GoalStoreEventListResponse, GoalStoreGoalResponse,
    GoalStoreSnapshot, GoalStoreSnapshotUpsertRequest, GoalStoreSnapshotUpsertResponse,
    GoalStoreTaskListResponse, OperatorAction, OperatorActionKind, OperatorActionResolutionKind,
    OperatorEventAppendRequest, OperatorEventAppendResponse, OperatorEventListResponse,
    OperatorTransition, PlanCandidateSelectionRequest, PlanCandidateSelectionResponse,
    PlanCandidateVoteRequest, PlanCandidateVoteResponse, PlanCompileRequest, PlanCompileResult,
    PlanDraftRequest, PlanId, PlanRevisionRequest, PlanStatus, TaskId, TaskRecord, TaskStatus,
    WorkerKind,
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

    fn bad_request(error: impl Into<anyhow::Error>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: error.into(),
        }
    }

    fn not_found(error: impl Into<anyhow::Error>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
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
    operator_events: Vec<DurableEventEnvelope>,
    actions: BTreeMap<String, OperatorAction>,
    approvals: BTreeMap<GoalId, Vec<ApprovalRecord>>,
    event_source_approvals: BTreeMap<Uuid, EventSourceApprovalRecord>,
    snapshots: BTreeMap<GoalId, GoalStoreSnapshot>,
    plans: BTreeMap<PlanId, DurablePlan>,
    drafts: BTreeMap<Uuid, GoalDraftRecord>,
    chat_turns: BTreeMap<String, Vec<ChatTurnRecord>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum JournalEntry {
    Snapshot(GoalStoreSnapshotUpsertRequest),
    Event(GoalStoreEventAppendRequest),
    OperatorEvent(OperatorEventAppendRequest),
    Artifact(GoalStoreArtifactRecordRequest),
    Plan(DurablePlan),
    Draft(GoalDraftRecord),
    EventSourceApproval(EventSourceApprovalRecord),
    ChatTurn(ChatTurnRecord),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct ChatTurnRecord {
    id: Uuid,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    draft_id: Option<Uuid>,
    session_id: String,
    goal_id: Option<GoalId>,
    mode: String,
    role: String,
    content: String,
    created_at: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct ChatTurnAppendRequest {
    id: Option<Uuid>,
    idempotency_key: Option<String>,
    draft_id: Option<Uuid>,
    session_id: String,
    goal_id: Option<GoalId>,
    mode: Option<String>,
    role: String,
    content: String,
    created_at: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    payload_json: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ChatTurnAppendResponse {
    accepted: bool,
    turn: ChatTurnRecord,
}

#[derive(Debug, Serialize)]
struct ChatSessionResponse {
    session_id: String,
    turns: Vec<ChatTurnRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct GoalDraftRecord {
    draft_id: Uuid,
    idempotency_key: Option<String>,
    session_id: Option<String>,
    plan_id: Option<PlanId>,
    goal_id: Option<GoalId>,
    title: String,
    objective: String,
    status: String,
    created_at: Option<String>,
    updated_at: Option<String>,
    payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct GoalDraftRecordRequest {
    draft_id: Option<Uuid>,
    idempotency_key: Option<String>,
    session_id: Option<String>,
    plan_id: Option<PlanId>,
    goal_id: Option<GoalId>,
    title: String,
    objective: String,
    status: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    payload_json: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct GoalDraftRecordResponse {
    accepted: bool,
    draft: GoalDraftRecord,
}

#[derive(Debug, Serialize)]
struct GoalDraftListResponse {
    drafts: Vec<GoalDraftRecord>,
}

#[derive(Debug, Serialize)]
struct OperatorActionListResponse {
    actions: Vec<OperatorAction>,
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
struct OperatorEventFilter {
    goal_id: Option<GoalId>,
    event_type: Option<String>,
    since: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct OperatorActionFilter {
    goal_id: Option<GoalId>,
    kind: Option<Vec<OperatorActionKind>>,
    status: Option<Vec<String>>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct DraftFilter {
    session_id: Option<String>,
    goal_id: Option<GoalId>,
    status: Option<Vec<String>>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ApprovalFilter {
    goal_id: Option<GoalId>,
    status: Option<Vec<ApprovalStatus>>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct EventSourceApprovalFilter {
    source_id: Option<String>,
    approval_ref: Option<String>,
    status: Option<Vec<EventSourceApprovalStatus>>,
    limit: Option<usize>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    coat_observability::init_tracing("coat-goal-store", "coat_goal_store=info,tower_http=info");

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
        .route(
            "/goal-store/operator-events",
            get(list_operator_events).post(append_operator_event),
        )
        .route("/goal-store/actions", get(list_actions))
        .route("/goal-store/artifacts", post(record_artifacts))
        .route("/goal-store/drafts", get(list_drafts).post(record_draft))
        .route("/goal-store/drafts/{draft_id}", get(get_draft))
        .route("/goal-store/plans", get(list_plans).post(create_plan))
        .route("/goal-store/plans/{plan_id}", get(get_plan))
        .route("/goal-store/plans/{plan_id}/revisions", post(revise_plan))
        .route("/goal-store/plans/{plan_id}/compile", post(compile_plan))
        .route(
            "/goal-store/plans/{plan_id}/candidate-votes",
            post(vote_plan_candidate),
        )
        .route(
            "/goal-store/plans/{plan_id}/candidate-selection",
            post(select_plan_candidate),
        )
        .route("/goal-store/goals", get(list_goals))
        .route("/goal-store/tasks", get(list_all_tasks))
        .route("/goal-store/approvals", get(list_all_approvals))
        .route(
            "/goal-store/event-source-approvals",
            get(list_event_source_approvals).post(record_event_source_approval),
        )
        .route("/goal-store/chat/turns", post(append_chat_turn))
        .route(
            "/goal-store/chat/sessions/{session_id}",
            get(get_chat_session),
        )
        .route("/goal-store/goals/{goal_id}", get(get_goal))
        .route("/goal-store/goals/{goal_id}/tasks", get(list_tasks))
        .route("/goal-store/goals/{goal_id}/events", get(list_events))
        .route(
            "/goal-store/goals/{goal_id}/operator-events",
            get(list_goal_operator_events),
        )
        .route("/goal-store/goals/{goal_id}/artifacts", get(list_artifacts))
        .route(
            "/goal-store/goals/{goal_id}/checkpoints",
            get(list_checkpoints),
        )
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
        "control_chat_log": "goal-store chat-turn projection with JSONL or Postgres backend",
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

async fn append_operator_event(
    State(state): State<AppState>,
    Json(request): Json<OperatorEventAppendRequest>,
) -> Result<Json<OperatorEventAppendResponse>, ApiError> {
    if let Some(pool) = &state.postgres {
        append_operator_event_postgres(pool, &request.event)
            .await
            .map_err(ApiError::internal)?;
    } else if let Err(error) =
        append_journal(&state, JournalEntry::OperatorEvent(request.clone())).await
    {
        tracing::warn!(%error, "append operator event journal failed");
    }

    let event_id = request.event.event_id;
    state
        .store
        .write()
        .await
        .apply_operator_event(request.event);
    Ok(Json(OperatorEventAppendResponse {
        accepted: true,
        event_id,
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
    let mut plan = load_plan(&state, plan_id)
        .await?
        .ok_or_else(|| ApiError::not_found(anyhow::anyhow!("plan {plan_id} not found")))?;
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
    let mut plan = load_plan(&state, plan_id)
        .await?
        .ok_or_else(|| ApiError::not_found(anyhow::anyhow!("plan {plan_id} not found")))?;
    request.plan_id = Some(plan_id);
    let result = plan.compile_goal(request);
    upsert_plan_record(&state, &plan)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(result))
}

async fn vote_plan_candidate(
    State(state): State<AppState>,
    Path(plan_id): Path<Uuid>,
    Json(request): Json<PlanCandidateVoteRequest>,
) -> Result<Json<PlanCandidateVoteResponse>, ApiError> {
    let mut plan = load_plan(&state, plan_id)
        .await?
        .ok_or_else(|| ApiError::not_found(anyhow::anyhow!("plan {plan_id} not found")))?;
    let candidate = load_plan(&state, request.candidate_plan_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(anyhow::anyhow!(
                "candidate plan {} not found",
                request.candidate_plan_id
            ))
        })?;
    ensure_candidate_branch(plan_id, &candidate)?;

    let vote = plan.record_candidate_vote(request);
    upsert_plan_record(&state, &plan)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(PlanCandidateVoteResponse {
        accepted: true,
        plan,
        vote,
    }))
}

async fn select_plan_candidate(
    State(state): State<AppState>,
    Path(plan_id): Path<Uuid>,
    Json(request): Json<PlanCandidateSelectionRequest>,
) -> Result<Json<PlanCandidateSelectionResponse>, ApiError> {
    let mut plan = load_plan(&state, plan_id)
        .await?
        .ok_or_else(|| ApiError::not_found(anyhow::anyhow!("plan {plan_id} not found")))?;
    let candidate = load_plan(&state, request.candidate_plan_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(anyhow::anyhow!(
                "candidate plan {} not found",
                request.candidate_plan_id
            ))
        })?;
    ensure_candidate_branch(plan_id, &candidate)?;
    if request.require_compiled_goal && candidate.compiled_goal_id.is_none() {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "candidate plan {} has no compiled_goal_id",
            candidate.id
        )));
    }

    let compiled_goal_id = candidate.compiled_goal_id;
    let selection = plan.select_candidate(request, compiled_goal_id);
    upsert_plan_record(&state, &plan)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(PlanCandidateSelectionResponse {
        accepted: true,
        plan,
        selection,
    }))
}

fn ensure_candidate_branch(
    source_plan_id: PlanId,
    candidate: &DurablePlan,
) -> Result<(), ApiError> {
    if candidate.id == source_plan_id {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "candidate plan cannot be the source plan"
        )));
    }
    if candidate.source_plan_id != Some(source_plan_id) {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "candidate plan {} is not a branch of source plan {}",
            candidate.id,
            source_plan_id
        )));
    }
    Ok(())
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

async fn record_event_source_approval(
    State(state): State<AppState>,
    Json(request): Json<EventSourceApprovalRecordRequest>,
) -> Result<Json<EventSourceApprovalRecordResponse>, ApiError> {
    upsert_event_source_approval_record(&state, &request.record)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(EventSourceApprovalRecordResponse {
        accepted: true,
        record: request.record,
    }))
}

async fn list_event_source_approvals(
    State(state): State<AppState>,
    Query(filter): Query<EventSourceApprovalFilter>,
) -> Result<Json<EventSourceApprovalListResponse>, ApiError> {
    let records = if let Some(pool) = &state.postgres {
        list_event_source_approvals_postgres(pool)
            .await
            .map_err(ApiError::internal)?
    } else {
        state
            .store
            .read()
            .await
            .event_source_approvals
            .values()
            .cloned()
            .collect()
    };
    Ok(Json(EventSourceApprovalListResponse {
        records: filter_event_source_approvals(records, &filter),
    }))
}

async fn append_chat_turn(
    State(state): State<AppState>,
    Json(request): Json<ChatTurnAppendRequest>,
) -> Result<Json<ChatTurnAppendResponse>, ApiError> {
    let turn = chat_turn_from_request(request)?;
    if let Some(pool) = &state.postgres {
        append_chat_turn_postgres(pool, &turn)
            .await
            .map_err(ApiError::internal)?;
    } else if let Err(error) = append_journal(&state, JournalEntry::ChatTurn(turn.clone())).await {
        tracing::warn!(%error, "append chat-turn journal failed");
    }

    state.store.write().await.apply_chat_turn(turn.clone());
    Ok(Json(ChatTurnAppendResponse {
        accepted: true,
        turn,
    }))
}

async fn get_chat_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<ChatSessionResponse>, ApiError> {
    let turns = if let Some(pool) = &state.postgres {
        list_chat_session_postgres(pool, &session_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        state
            .store
            .read()
            .await
            .chat_turns
            .get(&session_id)
            .cloned()
            .unwrap_or_default()
    };
    Ok(Json(ChatSessionResponse { session_id, turns }))
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

async fn list_operator_events(
    State(state): State<AppState>,
    Query(filter): Query<OperatorEventFilter>,
) -> Result<Json<OperatorEventListResponse>, ApiError> {
    let events = if let Some(pool) = &state.postgres {
        list_operator_events_postgres(
            pool,
            filter.goal_id,
            filter.event_type.as_deref(),
            filter.since.as_deref(),
            filter.limit,
        )
        .await
        .map_err(ApiError::internal)?
    } else {
        let store = state.store.read().await;
        filter_operator_events(store.operator_events.clone(), &filter)
    };
    Ok(Json(OperatorEventListResponse { events }))
}

async fn list_goal_operator_events(
    State(state): State<AppState>,
    Path(goal_id): Path<Uuid>,
    Query(mut filter): Query<OperatorEventFilter>,
) -> Result<Json<OperatorEventListResponse>, ApiError> {
    filter.goal_id = Some(goal_id);
    list_operator_events(State(state), Query(filter)).await
}

async fn list_actions(
    State(state): State<AppState>,
    Query(filter): Query<OperatorActionFilter>,
) -> Result<Json<OperatorActionListResponse>, ApiError> {
    let actions = if let Some(pool) = &state.postgres {
        list_actions_postgres(pool)
            .await
            .map_err(ApiError::internal)?
    } else {
        state.store.read().await.actions.values().cloned().collect()
    };
    Ok(Json(OperatorActionListResponse {
        actions: filter_actions(actions, &filter),
    }))
}

async fn record_draft(
    State(state): State<AppState>,
    Json(request): Json<GoalDraftRecordRequest>,
) -> Result<Json<GoalDraftRecordResponse>, ApiError> {
    let draft = draft_record_from_request(request)?;
    upsert_draft_record(&state, &draft)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(GoalDraftRecordResponse {
        accepted: true,
        draft,
    }))
}

async fn list_drafts(
    State(state): State<AppState>,
    Query(filter): Query<DraftFilter>,
) -> Result<Json<GoalDraftListResponse>, ApiError> {
    let drafts = if let Some(pool) = &state.postgres {
        list_drafts_postgres(pool)
            .await
            .map_err(ApiError::internal)?
    } else {
        state.store.read().await.drafts.values().cloned().collect()
    };
    Ok(Json(GoalDraftListResponse {
        drafts: filter_drafts(drafts, &filter),
    }))
}

async fn get_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
) -> Result<Json<GoalDraftRecordResponse>, ApiError> {
    let draft = if let Some(pool) = &state.postgres {
        get_draft_postgres(pool, draft_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        state.store.read().await.drafts.get(&draft_id).cloned()
    }
    .ok_or_else(|| ApiError::not_found(anyhow::anyhow!("draft {draft_id} not found")))?;

    Ok(Json(GoalDraftRecordResponse {
        accepted: true,
        draft,
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

async fn list_checkpoints(
    State(state): State<AppState>,
    Path(goal_id): Path<Uuid>,
) -> Result<Json<GoalStoreCheckpointListResponse>, ApiError> {
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
    Ok(Json(GoalStoreCheckpointListResponse {
        goal_id,
        checkpoints: checkpoints_from_artifacts(&artifacts),
    }))
}

fn checkpoints_from_artifacts(artifacts: &[GoalArtifactRecord]) -> Vec<CheckpointRef> {
    let mut checkpoints: Vec<CheckpointRef> = artifacts
        .iter()
        .filter_map(|artifact| artifact.checkpoint.clone())
        .collect();
    checkpoints.sort_by(|left, right| {
        left.task_id
            .cmp(&right.task_id)
            .then_with(|| left.sequence.cmp(&right.sequence))
            .then_with(|| left.label.cmp(&right.label))
    });
    checkpoints
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

fn filter_event_source_approvals(
    mut records: Vec<EventSourceApprovalRecord>,
    filter: &EventSourceApprovalFilter,
) -> Vec<EventSourceApprovalRecord> {
    records.retain(|record| {
        filter
            .source_id
            .as_ref()
            .is_none_or(|source_id| &record.source_id == source_id)
            && filter
                .approval_ref
                .as_ref()
                .is_none_or(|approval_ref| &record.approval_ref == approval_ref)
            && filter
                .status
                .as_ref()
                .is_none_or(|statuses| statuses.contains(&record.status))
    });
    records.sort_by(|left, right| {
        right
            .recorded_at
            .cmp(&left.recorded_at)
            .then_with(|| left.source_id.cmp(&right.source_id))
            .then_with(|| left.record_id.cmp(&right.record_id))
    });
    if let Some(limit) = filter.limit {
        records.truncate(limit);
    }
    records
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

fn filter_operator_events(
    mut events: Vec<DurableEventEnvelope>,
    filter: &OperatorEventFilter,
) -> Vec<DurableEventEnvelope> {
    events.retain(|event| {
        filter
            .goal_id
            .is_none_or(|goal_id| event.actor.goal_id == Some(goal_id))
            && filter
                .event_type
                .as_ref()
                .is_none_or(|event_type| &event.event_type == event_type)
            && filter
                .since
                .as_ref()
                .is_none_or(|since| &event.created_at > since)
    });
    events.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.event_id.cmp(&left.event_id))
    });
    if let Some(limit) = filter.limit {
        events.truncate(limit);
    }
    events
}

fn filter_actions(
    mut actions: Vec<OperatorAction>,
    filter: &OperatorActionFilter,
) -> Vec<OperatorAction> {
    actions.retain(|action| {
        filter
            .goal_id
            .is_none_or(|goal_id| action.goal_id == goal_id)
            && filter
                .kind
                .as_ref()
                .is_none_or(|kinds| kinds.contains(&action.kind))
            && filter.status.as_ref().is_none_or(|statuses| {
                statuses
                    .iter()
                    .any(|status| status.eq_ignore_ascii_case(&action.status))
            })
    });
    actions.sort_by(|left, right| {
        action_status_rank(&left.status)
            .cmp(&action_status_rank(&right.status))
            .then_with(|| left.goal_id.cmp(&right.goal_id))
            .then_with(|| left.action_id.cmp(&right.action_id))
    });
    if let Some(limit) = filter.limit {
        actions.truncate(limit);
    }
    actions
}

fn action_status_rank(status: &str) -> u8 {
    match status {
        "pending" | "open" => 0,
        "resolved" | "completed" => 1,
        "cancelled" => 2,
        _ => 3,
    }
}

fn filter_drafts(mut drafts: Vec<GoalDraftRecord>, filter: &DraftFilter) -> Vec<GoalDraftRecord> {
    drafts.retain(|draft| {
        filter
            .session_id
            .as_ref()
            .is_none_or(|session_id| draft.session_id.as_ref() == Some(session_id))
            && filter
                .goal_id
                .is_none_or(|goal_id| draft.goal_id == Some(goal_id))
            && filter.status.as_ref().is_none_or(|statuses| {
                statuses
                    .iter()
                    .any(|status| status.eq_ignore_ascii_case(&draft.status))
            })
    });
    drafts.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.draft_id.cmp(&right.draft_id))
    });
    if let Some(limit) = filter.limit {
        drafts.truncate(limit);
    }
    drafts
}

fn chat_turn_from_request(request: ChatTurnAppendRequest) -> Result<ChatTurnRecord, ApiError> {
    let session_id = request.session_id.trim().to_string();
    if session_id.is_empty() {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "chat turn session_id is required"
        )));
    }
    let role = request.role.trim().to_ascii_lowercase();
    if role != "user" && role != "assistant" {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "chat turn role must be user or assistant"
        )));
    }
    let content = request.content.trim().to_string();
    if content.is_empty() {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "chat turn content is required"
        )));
    }

    Ok(ChatTurnRecord {
        id: request.id.unwrap_or_else(Uuid::new_v4),
        idempotency_key: normalize_optional_string(request.idempotency_key),
        draft_id: request.draft_id,
        session_id,
        goal_id: request.goal_id,
        mode: request
            .mode
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "general".to_string()),
        role,
        content,
        created_at: request
            .created_at
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        provider: request
            .provider
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        model: request
            .model
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        payload_json: request
            .payload_json
            .unwrap_or_else(|| serde_json::json!({})),
    })
}

fn draft_record_from_request(request: GoalDraftRecordRequest) -> Result<GoalDraftRecord, ApiError> {
    let title = request.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "draft title is required"
        )));
    }
    let objective = request.objective.trim().to_string();
    if objective.is_empty() {
        return Err(ApiError::bad_request(anyhow::anyhow!(
            "draft objective is required"
        )));
    }

    Ok(GoalDraftRecord {
        draft_id: request.draft_id.unwrap_or_else(Uuid::new_v4),
        idempotency_key: normalize_optional_string(request.idempotency_key),
        session_id: normalize_optional_string(request.session_id),
        plan_id: request.plan_id,
        goal_id: request.goal_id,
        title,
        objective,
        status: request
            .status
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "draft".to_string()),
        created_at: normalize_optional_string(request.created_at),
        updated_at: normalize_optional_string(request.updated_at),
        payload_json: request
            .payload_json
            .unwrap_or_else(|| serde_json::json!({})),
    })
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn verify_postgres_schema(pool: &PgPool) -> anyhow::Result<()> {
    let table: Option<String> = sqlx::query_scalar("SELECT to_regclass('coat.goals')::text")
        .fetch_one(pool)
        .await
        .context("check coat.goals table")?;
    if table.is_none() {
        bail!("coat.goals table missing; run infra/db/migrations before starting goal-store");
    }
    let event_source_approvals: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('coat.event_source_approvals')::text")
            .fetch_one(pool)
            .await
            .context("check coat.event_source_approvals table")?;
    if event_source_approvals.is_none() {
        bail!(
            "coat.event_source_approvals table missing; run infra/db/migrations before starting goal-store"
        );
    }
    let chat_turns: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('coat.control_chat_turns')::text")
            .fetch_one(pool)
            .await
            .context("check coat.control_chat_turns table")?;
    if chat_turns.is_none() {
        bail!(
            "coat.control_chat_turns table missing; run infra/db/migrations before starting goal-store"
        );
    }
    let operator_events: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('coat.operator_events')::text")
            .fetch_one(pool)
            .await
            .context("check coat.operator_events table")?;
    if operator_events.is_none() {
        bail!(
            "coat.operator_events table missing; run infra/db/migrations before starting goal-store"
        );
    }
    let action_queue: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('coat.operator_action_queue')::text")
            .fetch_one(pool)
            .await
            .context("check coat.operator_action_queue table")?;
    if action_queue.is_none() {
        bail!(
            "coat.operator_action_queue table missing; run infra/db/migrations before starting goal-store"
        );
    }
    let drafts: Option<String> = sqlx::query_scalar("SELECT to_regclass('coat.goal_drafts')::text")
        .fetch_one(pool)
        .await
        .context("check coat.goal_drafts table")?;
    if drafts.is_none() {
        bail!("coat.goal_drafts table missing; run infra/db/migrations before starting goal-store");
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
    sqlx::query("DELETE FROM coat.operator_action_queue WHERE goal_id = $1")
        .bind(goal.goal_id)
        .execute(&mut *tx)
        .await
        .context("delete actions before snapshot projection")?;
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
                git_commit_sha, checkpoint_id, checkpoint_kind, checkpoint_label, object_bucket,
                object_key, sha256, created_at_text, payload_json, record_json
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
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
        .bind(artifact.checkpoint.as_ref().map(|checkpoint| checkpoint.id))
        .bind(
            artifact
                .checkpoint
                .as_ref()
                .map(|checkpoint| json_string(&checkpoint.kind))
                .transpose()?,
        )
        .bind(
            artifact
                .checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.label.clone()),
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

    for action in actions_from_snapshot(snapshot) {
        insert_action_postgres(&mut tx, &action).await?;
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

async fn append_operator_event_postgres(
    pool: &PgPool,
    event: &DurableEventEnvelope,
) -> anyhow::Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("begin operator event transaction")?;
    sqlx::query(
        r#"
        INSERT INTO coat.operator_events (
            id, event_type, actor_kind, actor_id, goal_id, task_id, transition,
            idempotency_key, causation_id, correlation_id, restate_invocation_id,
            payload_json, record_json, created_at_text
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(event.event_id)
    .bind(&event.event_type)
    .bind(json_string(&event.actor.kind)?)
    .bind(&event.actor.id)
    .bind(event.actor.goal_id)
    .bind(event.actor.task_id)
    .bind(json_string(&event.transition)?)
    .bind(&event.idempotency_key)
    .bind(&event.causation_id)
    .bind(&event.correlation_id)
    .bind(&event.restate_invocation_id)
    .bind(event.payload_json.clone())
    .bind(serde_json::to_value(event)?)
    .bind(&event.created_at)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("append operator event {}", event.event_id))?;
    update_actions_for_operator_event_postgres(&mut tx, event).await?;
    tx.commit()
        .await
        .context("commit operator event transaction")?;
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
            git_commit_sha, checkpoint_id, checkpoint_kind, checkpoint_label, object_bucket,
            object_key, sha256, created_at_text, payload_json, record_json
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
        ON CONFLICT (id) DO UPDATE SET
            artifact_type = EXCLUDED.artifact_type,
            uri = EXCLUDED.uri,
            description = EXCLUDED.description,
            git_remote = EXCLUDED.git_remote,
            git_ref = EXCLUDED.git_ref,
            git_commit_sha = EXCLUDED.git_commit_sha,
            checkpoint_id = EXCLUDED.checkpoint_id,
            checkpoint_kind = EXCLUDED.checkpoint_kind,
            checkpoint_label = EXCLUDED.checkpoint_label,
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
    .bind(artifact.checkpoint.as_ref().map(|checkpoint| checkpoint.id))
    .bind(
        artifact
            .checkpoint
            .as_ref()
            .map(|checkpoint| json_string(&checkpoint.kind))
            .transpose()?,
    )
    .bind(
        artifact
            .checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.label.clone()),
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
    if let Some(draft) = draft_from_plan(plan)? {
        upsert_draft_record(state, &draft).await?;
    }
    Ok(())
}

async fn upsert_draft_record(state: &AppState, draft: &GoalDraftRecord) -> anyhow::Result<()> {
    if let Some(pool) = &state.postgres {
        upsert_draft_postgres(pool, draft).await?;
    } else if let Err(error) = append_journal(state, JournalEntry::Draft(draft.clone())).await {
        tracing::warn!(%error, "append draft journal failed");
    }
    state.store.write().await.apply_draft(draft.clone());
    Ok(())
}

async fn upsert_event_source_approval_record(
    state: &AppState,
    record: &EventSourceApprovalRecord,
) -> anyhow::Result<()> {
    if let Some(pool) = &state.postgres {
        upsert_event_source_approval_postgres(pool, record).await?;
    } else if let Err(error) =
        append_journal(state, JournalEntry::EventSourceApproval(record.clone())).await
    {
        tracing::warn!(%error, "append event source approval journal failed");
    }
    state
        .store
        .write()
        .await
        .apply_event_source_approval(record.clone());
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
        ON CONFLICT DO NOTHING
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

async fn insert_action_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    action: &OperatorAction,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO coat.operator_action_queue (
            action_id, kind, goal_id, task_id, title, question, status,
            allowed_resolutions, payload_json, record_json, projected_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now())
        ON CONFLICT (action_id) DO UPDATE SET
            kind = EXCLUDED.kind,
            goal_id = EXCLUDED.goal_id,
            task_id = EXCLUDED.task_id,
            title = EXCLUDED.title,
            question = EXCLUDED.question,
            status = EXCLUDED.status,
            allowed_resolutions = EXCLUDED.allowed_resolutions,
            payload_json = EXCLUDED.payload_json,
            record_json = EXCLUDED.record_json,
            projected_at = now()
        "#,
    )
    .bind(&action.action_id)
    .bind(json_string(&action.kind)?)
    .bind(action.goal_id)
    .bind(action.task_id)
    .bind(&action.title)
    .bind(&action.question)
    .bind(&action.status)
    .bind(
        action
            .allowed_resolutions
            .iter()
            .map(json_string)
            .collect::<anyhow::Result<Vec<_>>>()?,
    )
    .bind(action.payload_json.clone())
    .bind(serde_json::to_value(action)?)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("upsert action {}", action.action_id))?;
    Ok(())
}

async fn update_actions_for_operator_event_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &DurableEventEnvelope,
) -> anyhow::Result<()> {
    let Some(goal_id) = event.actor.goal_id else {
        return Ok(());
    };
    let Some(status) = status_for_operator_transition(&event.transition) else {
        return Ok(());
    };

    let kind = action_kind_for_operator_transition(&event.transition)
        .map(|kind| json_string(&kind))
        .transpose()?;
    sqlx::query(
        r#"
        UPDATE coat.operator_action_queue
        SET status = $1,
            payload_json = jsonb_set(
                payload_json,
                '{resolution_event_id}',
                to_jsonb($2::text),
                true
            ),
            record_json = jsonb_set(
                jsonb_set(
                    record_json,
                    '{status}',
                    to_jsonb($1::text),
                    true
                ),
                '{payload_json,resolution_event_id}',
                to_jsonb($2::text),
                true
            ),
            projected_at = now()
        WHERE goal_id = $3
          AND ($4::uuid IS NULL OR task_id = $4)
          AND ($5::text IS NULL OR kind = $5)
          AND status IN ('pending', 'open')
        "#,
    )
    .bind(status)
    .bind(event.event_id.to_string())
    .bind(goal_id)
    .bind(event.actor.task_id)
    .bind(kind)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("update actions for operator event {}", event.event_id))?;
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
            id, source_plan_id, title, objective, repo, status, mode, version,
            compiled_goal_id, updated_at_text, record_json
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (id) DO UPDATE SET
            source_plan_id = EXCLUDED.source_plan_id,
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
    .bind(plan.source_plan_id)
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

async fn upsert_event_source_approval_postgres(
    pool: &PgPool,
    record: &EventSourceApprovalRecord,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO coat.event_source_approvals (
            id, approval_ref, source_id, source_kind, status, risky, reason,
            operator, recorded_at_text, payload_json, record_json
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (approval_ref, source_id) DO UPDATE SET
            id = EXCLUDED.id,
            source_kind = EXCLUDED.source_kind,
            status = EXCLUDED.status,
            risky = EXCLUDED.risky,
            reason = EXCLUDED.reason,
            operator = EXCLUDED.operator,
            recorded_at_text = EXCLUDED.recorded_at_text,
            payload_json = EXCLUDED.payload_json,
            record_json = EXCLUDED.record_json,
            recorded_at = now()
        "#,
    )
    .bind(record.record_id)
    .bind(&record.approval_ref)
    .bind(&record.source_id)
    .bind(json_string(&record.source_kind)?)
    .bind(json_string(&record.status)?)
    .bind(record.risky)
    .bind(&record.reason)
    .bind(&record.operator)
    .bind(&record.recorded_at)
    .bind(record.payload_json.clone())
    .bind(serde_json::to_value(record)?)
    .execute(pool)
    .await
    .with_context(|| {
        format!(
            "upsert event source approval {} for source {}",
            record.approval_ref, record.source_id
        )
    })?;
    Ok(())
}

async fn list_event_source_approvals_postgres(
    pool: &PgPool,
) -> anyhow::Result<Vec<EventSourceApprovalRecord>> {
    let rows = sqlx::query(
        "SELECT record_json FROM coat.event_source_approvals ORDER BY recorded_at DESC, source_id ASC",
    )
    .fetch_all(pool)
    .await
    .context("query event source approvals")?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "event source approval"))
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

async fn list_operator_events_postgres(
    pool: &PgPool,
    goal_id: Option<GoalId>,
    event_type: Option<&str>,
    since: Option<&str>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<DurableEventEnvelope>> {
    let limit = limit.unwrap_or(100).min(500);
    let rows = sqlx::query(
        r#"
        SELECT record_json
        FROM coat.operator_events
        WHERE ($1::uuid IS NULL OR goal_id = $1)
          AND ($2::text IS NULL OR event_type = $2)
          AND ($3::text IS NULL OR created_at_text > $3)
        ORDER BY recorded_at DESC, id DESC
        LIMIT $4
        "#,
    )
    .bind(goal_id)
    .bind(event_type)
    .bind(since)
    .bind(as_i64(limit as u64, "operator_event.limit")?)
    .fetch_all(pool)
    .await
    .context("query operator events")?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "operator event"))
        .collect()
}

async fn list_actions_postgres(pool: &PgPool) -> anyhow::Result<Vec<OperatorAction>> {
    let rows = sqlx::query(
        r#"
        SELECT record_json
        FROM coat.operator_action_queue
        ORDER BY projected_at DESC, action_id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("query operator action queue")?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "operator action"))
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

async fn upsert_draft_postgres(pool: &PgPool, draft: &GoalDraftRecord) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("begin draft transaction")?;
    sqlx::query(
        r#"
        INSERT INTO coat.goal_drafts (
            id, idempotency_key, session_id, plan_id, goal_id, title, objective,
            status, created_at_text, updated_at_text, payload_json, record_json, projected_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, now())
        ON CONFLICT (id) DO UPDATE SET
            idempotency_key = EXCLUDED.idempotency_key,
            session_id = EXCLUDED.session_id,
            plan_id = EXCLUDED.plan_id,
            goal_id = EXCLUDED.goal_id,
            title = EXCLUDED.title,
            objective = EXCLUDED.objective,
            status = EXCLUDED.status,
            created_at_text = EXCLUDED.created_at_text,
            updated_at_text = EXCLUDED.updated_at_text,
            payload_json = EXCLUDED.payload_json,
            record_json = EXCLUDED.record_json,
            projected_at = now()
        "#,
    )
    .bind(draft.draft_id)
    .bind(&draft.idempotency_key)
    .bind(&draft.session_id)
    .bind(draft.plan_id)
    .bind(draft.goal_id)
    .bind(&draft.title)
    .bind(&draft.objective)
    .bind(&draft.status)
    .bind(&draft.created_at)
    .bind(&draft.updated_at)
    .bind(draft.payload_json.clone())
    .bind(serde_json::to_value(draft)?)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("upsert draft {}", draft.draft_id))?;
    if let Some(action) = draft_action(draft) {
        insert_action_postgres(&mut tx, &action).await?;
    }
    tx.commit().await.context("commit draft transaction")?;
    Ok(())
}

async fn list_drafts_postgres(pool: &PgPool) -> anyhow::Result<Vec<GoalDraftRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT record_json
        FROM coat.goal_drafts
        ORDER BY projected_at DESC, id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("query goal drafts")?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "draft"))
        .collect()
}

async fn get_draft_postgres(
    pool: &PgPool,
    draft_id: Uuid,
) -> anyhow::Result<Option<GoalDraftRecord>> {
    let row = sqlx::query("SELECT record_json FROM coat.goal_drafts WHERE id = $1")
        .bind(draft_id)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("query draft {draft_id}"))?;
    row.map(|row| decode_record(row.try_get("record_json")?, "draft"))
        .transpose()
}

async fn append_chat_turn_postgres(pool: &PgPool, turn: &ChatTurnRecord) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO coat.control_chat_turns (
            id, idempotency_key, draft_id, session_id, goal_id, mode, role, content, provider, model,
            created_at_text, payload_json, record_json
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(turn.id)
    .bind(&turn.idempotency_key)
    .bind(turn.draft_id)
    .bind(&turn.session_id)
    .bind(turn.goal_id)
    .bind(&turn.mode)
    .bind(&turn.role)
    .bind(&turn.content)
    .bind(&turn.provider)
    .bind(&turn.model)
    .bind(&turn.created_at)
    .bind(turn.payload_json.clone())
    .bind(serde_json::to_value(turn)?)
    .execute(pool)
    .await
    .with_context(|| {
        format!(
            "append chat turn {} for session {}",
            turn.id, turn.session_id
        )
    })?;
    Ok(())
}

async fn list_chat_session_postgres(
    pool: &PgPool,
    session_id: &str,
) -> anyhow::Result<Vec<ChatTurnRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT record_json
        FROM coat.control_chat_turns
        WHERE session_id = $1
        ORDER BY recorded_at ASC, id ASC
        "#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("query chat session {session_id}"))?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "chat turn"))
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

fn actions_from_snapshot(snapshot: &GoalStoreSnapshot) -> Vec<OperatorAction> {
    let mut actions = Vec::new();
    for approval in &snapshot.approvals {
        if approval.status == ApprovalStatus::Pending {
            actions.push(action_from_approval(approval));
        }
    }
    for task in &snapshot.tasks {
        if action_needed_task_status(&task.status) {
            let has_pending_approval = snapshot.approvals.iter().any(|approval| {
                approval.task_id == Some(task.task_id) && approval.status == ApprovalStatus::Pending
            });
            if !has_pending_approval {
                actions.push(action_from_task(task));
            }
        }
    }
    actions
}

fn action_from_approval(approval: &ApprovalRecord) -> OperatorAction {
    OperatorAction {
        action_id: format!("approval:{}", approval.approval_id),
        kind: OperatorActionKind::ResolveApproval,
        goal_id: approval.goal_id,
        task_id: approval.task_id,
        title: "Resolve approval".to_string(),
        question: format!("Approve or reject: {}", approval.requested_action),
        status: "pending".to_string(),
        allowed_resolutions: vec![
            OperatorActionResolutionKind::Approve,
            OperatorActionResolutionKind::Reject,
            OperatorActionResolutionKind::CancelGoal,
        ],
        approval: Some(approval.clone()),
        thunk: None,
        payload_json: serde_json::json!({
            "reason": approval.reason,
            "risk": json_string(&approval.risk).unwrap_or_else(|_| "unknown".to_string()),
        }),
    }
}

fn action_from_task(task: &TaskRecord) -> OperatorAction {
    let (kind, title, question, allowed_resolutions) = match task.status {
        TaskStatus::WaitingInput => (
            OperatorActionKind::ResumeThunk,
            "Continue waiting task",
            "Provide the missing input or continue this task.",
            vec![
                OperatorActionResolutionKind::Continue,
                OperatorActionResolutionKind::Answer,
                OperatorActionResolutionKind::AddContext,
                OperatorActionResolutionKind::Replan,
                OperatorActionResolutionKind::CancelGoal,
            ],
        ),
        TaskStatus::WaitingApproval => (
            OperatorActionKind::ResolveApproval,
            "Resolve approval",
            "Resolve the approval needed for this task.",
            vec![
                OperatorActionResolutionKind::Approve,
                OperatorActionResolutionKind::Reject,
                OperatorActionResolutionKind::CancelGoal,
            ],
        ),
        TaskStatus::Failed => (
            OperatorActionKind::RestartTask,
            "Recover failed task",
            "Retry, replan, or cancel this failed task.",
            vec![
                OperatorActionResolutionKind::Retry,
                OperatorActionResolutionKind::Replan,
                OperatorActionResolutionKind::CancelGoal,
            ],
        ),
        _ => (
            OperatorActionKind::ReplanTask,
            "Recover blocked task",
            "Retry, replan, or cancel this blocked task.",
            vec![
                OperatorActionResolutionKind::Retry,
                OperatorActionResolutionKind::Replan,
                OperatorActionResolutionKind::CancelGoal,
            ],
        ),
    };

    OperatorAction {
        action_id: format!(
            "task:{}:{}",
            task.task_id,
            json_string(&task.status).unwrap_or_default()
        ),
        kind,
        goal_id: task.goal_id,
        task_id: Some(task.task_id),
        title: title.to_string(),
        question: format!("{question} Task: {}", task.title),
        status: "pending".to_string(),
        allowed_resolutions,
        approval: None,
        thunk: None,
        payload_json: serde_json::json!({
            "task_status": json_string(&task.status).unwrap_or_else(|_| "unknown".to_string()),
            "task_title": task.title.clone(),
            "subgoal_id": task.subgoal_id.clone(),
            "recovery": "coordinator_action_required",
        }),
    }
}

fn action_needed_task_status(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Blocked
            | TaskStatus::Failed
            | TaskStatus::WaitingApproval
            | TaskStatus::WaitingInput
    )
}

fn draft_action(draft: &GoalDraftRecord) -> Option<OperatorAction> {
    let goal_id = draft.goal_id?;
    if matches!(draft.status.as_str(), "accepted" | "discarded" | "compiled") {
        return None;
    }
    Some(OperatorAction {
        action_id: format!("draft:{}:accept", draft.draft_id),
        kind: OperatorActionKind::AcceptDraft,
        goal_id,
        task_id: None,
        title: "Accept draft".to_string(),
        question: format!("Submit draft goal: {}", draft.title),
        status: "pending".to_string(),
        allowed_resolutions: vec![
            OperatorActionResolutionKind::AcceptDraft,
            OperatorActionResolutionKind::DiscardDraft,
        ],
        approval: None,
        thunk: None,
        payload_json: serde_json::json!({
            "draft_id": draft.draft_id,
            "plan_id": draft.plan_id,
            "objective": draft.objective,
        }),
    })
}

fn draft_from_plan(plan: &DurablePlan) -> anyhow::Result<Option<GoalDraftRecord>> {
    if matches!(plan.status, PlanStatus::Archived | PlanStatus::Superseded) {
        return Ok(None);
    }
    Ok(Some(GoalDraftRecord {
        draft_id: plan.id,
        idempotency_key: Some(format!("plan:{}:draft", plan.id)),
        session_id: None,
        plan_id: Some(plan.id),
        goal_id: plan.compiled_goal_id,
        title: plan.title.clone(),
        objective: plan.objective.clone(),
        status: json_string(&plan.status)?,
        created_at: plan.created_at.clone(),
        updated_at: plan.updated_at.clone(),
        payload_json: serde_json::to_value(plan)?,
    }))
}

fn status_for_operator_transition(transition: &OperatorTransition) -> Option<&'static str> {
    match transition {
        OperatorTransition::DraftAccepted
        | OperatorTransition::ThunkResumed
        | OperatorTransition::ApprovalResolved
        | OperatorTransition::GoalSatisfied => Some("resolved"),
        OperatorTransition::GoalCancelled => Some("cancelled"),
        _ => None,
    }
}

fn action_kind_for_operator_transition(
    transition: &OperatorTransition,
) -> Option<OperatorActionKind> {
    match transition {
        OperatorTransition::DraftAccepted => Some(OperatorActionKind::AcceptDraft),
        OperatorTransition::ThunkResumed => Some(OperatorActionKind::ResumeThunk),
        OperatorTransition::ApprovalResolved => Some(OperatorActionKind::ResolveApproval),
        OperatorTransition::GoalCancelled | OperatorTransition::GoalSatisfied => None,
        _ => None,
    }
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
        for event in &snapshot.events {
            self.apply_event(event.clone());
        }
        self.snapshots.insert(goal_id, snapshot);
        self.rebuild_goal_actions(goal_id);
    }

    fn apply_event(&mut self, event: GoalEventRecord) {
        let events = self.events.entry(event.goal_id).or_default();
        if events.iter().any(|existing| {
            existing.sequence == event.sequence || existing.idempotency_key == event.idempotency_key
        }) {
            return;
        }
        events.push(event);
    }

    fn apply_operator_event(&mut self, event: DurableEventEnvelope) {
        if self.operator_events.iter().any(|existing| {
            existing.event_id == event.event_id || existing.idempotency_key == event.idempotency_key
        }) {
            return;
        }
        self.resolve_actions_for_event(&event);
        self.operator_events.push(event);
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

    fn apply_draft(&mut self, draft: GoalDraftRecord) {
        if self.drafts.values().any(|existing| {
            existing.draft_id != draft.draft_id
                && existing.idempotency_key.is_some()
                && existing.idempotency_key == draft.idempotency_key
        }) {
            return;
        }
        if let Some(previous) = self.drafts.get(&draft.draft_id) {
            if let Some(previous_action) = draft_action(previous) {
                self.actions.remove(&previous_action.action_id);
            }
        }
        if let Some(action) = draft_action(&draft) {
            self.actions.insert(action.action_id.clone(), action);
        }
        self.drafts.insert(draft.draft_id, draft);
    }

    fn apply_event_source_approval(&mut self, record: EventSourceApprovalRecord) {
        self.event_source_approvals.insert(record.record_id, record);
    }

    fn apply_chat_turn(&mut self, turn: ChatTurnRecord) {
        let turns = self.chat_turns.entry(turn.session_id.clone()).or_default();
        if turns.iter().any(|existing| {
            existing.id == turn.id
                || existing.idempotency_key.is_some()
                    && existing.idempotency_key == turn.idempotency_key
        }) {
            return;
        }
        turns.push(turn);
    }

    fn rebuild_goal_actions(&mut self, goal_id: GoalId) {
        self.actions.retain(|_, action| action.goal_id != goal_id);
        if let Some(snapshot) = self.snapshots.get(&goal_id) {
            for action in actions_from_snapshot(snapshot) {
                self.actions.insert(action.action_id.clone(), action);
            }
        }
        for draft in self.drafts.values() {
            if draft.goal_id == Some(goal_id) {
                if let Some(action) = draft_action(draft) {
                    self.actions.insert(action.action_id.clone(), action);
                }
            }
        }
    }

    fn rebuild_action_queue(&mut self) {
        self.actions.clear();
        let goal_ids: Vec<_> = self.snapshots.keys().copied().collect();
        for goal_id in goal_ids {
            self.rebuild_goal_actions(goal_id);
        }
        for draft in self.drafts.values() {
            if let Some(action) = draft_action(draft) {
                self.actions
                    .entry(action.action_id.clone())
                    .or_insert(action);
            }
        }
    }

    fn resolve_actions_for_event(&mut self, event: &DurableEventEnvelope) {
        let Some(goal_id) = event.actor.goal_id else {
            return;
        };
        let Some(status) = status_for_operator_transition(&event.transition) else {
            return;
        };
        let kind = action_kind_for_operator_transition(&event.transition);
        for action in self.actions.values_mut() {
            if action.goal_id == goal_id
                && event
                    .actor
                    .task_id
                    .is_none_or(|task_id| action.task_id == Some(task_id))
                && kind.as_ref().is_none_or(|kind| &action.kind == kind)
                && matches!(action.status.as_str(), "pending" | "open")
            {
                action.status = status.to_string();
                action.payload_json["resolution_event_id"] =
                    serde_json::Value::String(event.event_id.to_string());
            }
        }
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
            JournalEntry::OperatorEvent(request) => store.apply_operator_event(request.event),
            JournalEntry::Artifact(request) => store.apply_artifacts(request.into_records()),
            JournalEntry::Plan(plan) => store.apply_plan(plan),
            JournalEntry::Draft(draft) => store.apply_draft(draft),
            JournalEntry::EventSourceApproval(record) => {
                store.apply_event_source_approval(record);
            }
            JournalEntry::ChatTurn(turn) => store.apply_chat_turn(turn),
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
    use std::sync::Arc;

    use axum::{
        Json,
        extract::{Path, State},
    };
    use coat_domain::{
        ApprovalRecord, ApprovalRisk, ArtifactKind, ArtifactRef, BranchSelector, CheckpointRef,
        DurableEventEnvelope, EventSourceApprovalRecord, EventSourceApprovalStatus,
        EventSourceKind, GoalEventKind, GoalEventRecord, GoalSpec, GoalState,
        GoalStoreArtifactRecordRequest, GoalStoreSnapshotUpsertRequest, OperatorActionKind,
        OperatorActorRef, OperatorEventAppendRequest, OperatorTransition,
        PlanCandidateSelectionRequest, PlanCandidateVoteRequest, PlanCompileRequest,
        PlanDraftRequest, ProtocolMetadata, TaskStatus,
    };
    use tokio::sync::RwLock;

    use super::{
        AppState, ChatTurnAppendRequest, GoalStore, GoalStoreBackend, OperatorActionFilter,
        OperatorEventFilter, append_chat_turn, append_operator_event, checkpoints_from_artifacts,
        filter_actions, filter_operator_events, get_chat_session, select_plan_candidate,
        upsert_plan_record, vote_plan_candidate,
    };

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
    fn snapshot_rebuilds_action_queue_for_approvals_and_blocked_tasks() {
        let state = GoalState::new(GoalSpec::new(
            "Actionable goal",
            "Project blocked work into the operator action queue",
        ));
        let mut request = GoalStoreSnapshotUpsertRequest::from_state(&state, "action_projection");
        let goal_id = request.snapshot.goal.goal_id;
        let task_id = request.snapshot.tasks[0].task_id;
        request.snapshot.tasks[0].status = TaskStatus::Blocked;
        request.snapshot.approvals.push(ApprovalRecord {
            approval_id: uuid::Uuid::new_v4(),
            goal_id,
            task_id: Some(task_id),
            status: coat_domain::ApprovalStatus::Pending,
            risk: ApprovalRisk::Medium,
            reason: "dangerous operation needs confirmation".to_string(),
            requested_action: "continue sandbox command".to_string(),
            updated_at: Some("2026-05-14T12:00:00Z".to_string()),
            payload_json: serde_json::json!({"source": "test"}),
        });
        let mut store = GoalStore::default();

        store.apply_snapshot(request.snapshot.clone());
        store.apply_snapshot(request.snapshot);
        store.rebuild_action_queue();

        let actions = filter_actions(
            store.actions.values().cloned().collect(),
            &OperatorActionFilter {
                goal_id: Some(goal_id),
                kind: None,
                status: Some(vec!["pending".to_string()]),
                limit: None,
            },
        );

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, OperatorActionKind::ResolveApproval);
        assert_eq!(actions[0].task_id, Some(task_id));
        assert!(actions[0].question.contains("continue sandbox command"));
    }

    #[test]
    fn plan_projection_indexes_durable_plan() {
        let request = PlanDraftRequest {
            plan_id: None,
            source_plan_id: None,
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

    #[tokio::test]
    async fn plan_projection_persists_draft_hook() {
        let state = AppState {
            store: Arc::new(RwLock::new(GoalStore::default())),
            journal_path: None,
            backend: GoalStoreBackend::Memory,
            postgres: None,
        };
        let plan = coat_domain::DurablePlan::draft(PlanDraftRequest {
            plan_id: None,
            source_plan_id: None,
            title: "Draft hook".to_string(),
            objective: "Keep the chat-created draft queryable.".to_string(),
            repo: None,
            prompt: "Draft this goal.".to_string(),
            mode: Default::default(),
            status: None,
            author: None,
            summary: None,
            authoring: Default::default(),
            plan: Default::default(),
            initial_tasks: Vec::new(),
            questions: Vec::new(),
            decisions: Vec::new(),
        });
        let plan_id = plan.id;

        upsert_plan_record(&state, &plan).await.unwrap();

        let store = state.store.read().await;
        assert!(store.plans.contains_key(&plan_id));
        assert!(store.drafts.contains_key(&plan_id));
        assert_eq!(store.drafts[&plan_id].status, "draft");
    }

    #[test]
    fn chat_turn_projection_is_idempotent_by_key() {
        let mut store = GoalStore::default();
        let mut turn = super::ChatTurnRecord {
            id: uuid::Uuid::new_v4(),
            idempotency_key: Some("chat:dedupe".to_string()),
            draft_id: Some(uuid::Uuid::new_v4()),
            session_id: "operator:dedupe".to_string(),
            goal_id: None,
            mode: "draft_plan".to_string(),
            role: "user".to_string(),
            content: "first".to_string(),
            created_at: Some("2026-05-14T12:00:00Z".to_string()),
            provider: None,
            model: None,
            payload_json: serde_json::json!({}),
        };
        store.apply_chat_turn(turn.clone());
        turn.id = uuid::Uuid::new_v4();
        turn.content = "duplicate".to_string();
        store.apply_chat_turn(turn);

        let turns = &store.chat_turns["operator:dedupe"];
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "first");
    }

    #[tokio::test]
    async fn chat_turn_handlers_persist_session_history() {
        let state = AppState {
            store: Arc::new(RwLock::new(GoalStore::default())),
            journal_path: None,
            backend: GoalStoreBackend::Memory,
            postgres: None,
        };
        let session_id = "operator:smoke".to_string();

        let user_turn = append_chat_turn(
            State(state.clone()),
            Json(ChatTurnAppendRequest {
                id: None,
                idempotency_key: Some("chat:operator:smoke:1".to_string()),
                draft_id: None,
                session_id: session_id.clone(),
                goal_id: None,
                mode: Some("draft_plan".to_string()),
                role: "user".to_string(),
                content: "Draft a durable plan.".to_string(),
                created_at: Some("2026-05-08T00:00:00Z".to_string()),
                provider: None,
                model: None,
                payload_json: Some(serde_json::json!({"source": "test"})),
            }),
        )
        .await
        .expect("user turn accepted");
        assert!(user_turn.0.accepted);

        let assistant_turn = append_chat_turn(
            State(state.clone()),
            Json(ChatTurnAppendRequest {
                id: None,
                idempotency_key: Some("chat:operator:smoke:2".to_string()),
                draft_id: None,
                session_id: session_id.clone(),
                goal_id: None,
                mode: Some("draft_plan".to_string()),
                role: "assistant".to_string(),
                content: "Use the plan editor and compile to a GoalSpec.".to_string(),
                created_at: Some("2026-05-08T00:00:01Z".to_string()),
                provider: Some("stub".to_string()),
                model: None,
                payload_json: None,
            }),
        )
        .await
        .expect("assistant turn accepted");
        assert!(assistant_turn.0.accepted);

        let session = get_chat_session(State(state), Path(session_id.clone()))
            .await
            .expect("session returned")
            .0;

        assert_eq!(session.session_id, session_id);
        assert_eq!(session.turns.len(), 2);
        assert_eq!(session.turns[0].role, "user");
        assert_eq!(session.turns[0].content, "Draft a durable plan.");
        assert_eq!(session.turns[1].role, "assistant");
        assert_eq!(session.turns[1].provider.as_deref(), Some("stub"));
    }

    #[tokio::test]
    async fn plan_candidate_handlers_enforce_branch_and_compilation() {
        let state = AppState {
            store: Arc::new(RwLock::new(GoalStore::default())),
            journal_path: None,
            backend: GoalStoreBackend::Memory,
            postgres: None,
        };
        let source = coat_domain::DurablePlan::draft(PlanDraftRequest {
            plan_id: None,
            source_plan_id: None,
            title: "Source plan".to_string(),
            objective: "Pick a branch candidate.".to_string(),
            repo: None,
            prompt: "Plan first.".to_string(),
            mode: Default::default(),
            status: None,
            author: None,
            summary: None,
            authoring: Default::default(),
            plan: Default::default(),
            initial_tasks: Vec::new(),
            questions: Vec::new(),
            decisions: Vec::new(),
        });
        let mut candidate = coat_domain::DurablePlan::draft(PlanDraftRequest {
            plan_id: None,
            source_plan_id: Some(source.id),
            title: "Candidate plan".to_string(),
            objective: "Candidate branch.".to_string(),
            repo: None,
            prompt: "Branch candidate.".to_string(),
            mode: Default::default(),
            status: None,
            author: None,
            summary: None,
            authoring: Default::default(),
            plan: Default::default(),
            initial_tasks: Vec::new(),
            questions: Vec::new(),
            decisions: Vec::new(),
        });
        upsert_plan_record(&state, &source).await.unwrap();
        upsert_plan_record(&state, &candidate).await.unwrap();

        let uncompiled_selection = select_plan_candidate(
            State(state.clone()),
            Path(source.id),
            Json(PlanCandidateSelectionRequest {
                candidate_plan_id: candidate.id,
                selector: BranchSelector::HighestScore,
                reason: "compiled winner required".to_string(),
                operator: Some("operator".to_string()),
                require_compiled_goal: true,
            }),
        )
        .await;
        assert!(uncompiled_selection.is_err());

        candidate.compile_goal(PlanCompileRequest {
            plan_id: Some(candidate.id),
            goal_id: None,
            title_override: None,
            objective_override: None,
            strict_review: false,
            human_steered: false,
            enable_branching: true,
        });
        upsert_plan_record(&state, &candidate).await.unwrap();

        let vote = vote_plan_candidate(
            State(state.clone()),
            Path(source.id),
            Json(PlanCandidateVoteRequest {
                candidate_plan_id: candidate.id,
                ranked_plan_ids: vec![candidate.id],
                voter: Some("reviewer".to_string()),
                score: 0.9,
                confidence: 0.8,
                rationale: "branch evidence is strongest".to_string(),
                evidence: vec!["compiled goal exists".to_string()],
            }),
        )
        .await
        .expect("vote accepted")
        .0;
        assert_eq!(vote.vote.candidate_plan_id, candidate.id);

        let selected = select_plan_candidate(
            State(state.clone()),
            Path(source.id),
            Json(PlanCandidateSelectionRequest {
                candidate_plan_id: candidate.id,
                selector: BranchSelector::HighestScore,
                reason: "highest score with compiled goal".to_string(),
                operator: Some("operator".to_string()),
                require_compiled_goal: true,
            }),
        )
        .await
        .expect("selection accepted")
        .0;
        assert_eq!(selected.selection.candidate_plan_id, candidate.id);
        assert!(selected.selection.selected_compiled_goal_id.is_some());
        assert_eq!(selected.plan.status, coat_domain::PlanStatus::Superseded);
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
            checkpoints: Vec::new(),
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

    #[test]
    fn event_projection_is_idempotent_by_sequence_or_key() {
        let goal_id = uuid::Uuid::new_v4();
        let mut store = GoalStore::default();
        let event = GoalEventRecord {
            event_id: uuid::Uuid::new_v4(),
            goal_id,
            task_id: None,
            sequence: 7,
            kind: GoalEventKind::StateProjected,
            message: "first projection".to_string(),
            actor: Some("coordinator".to_string()),
            idempotency_key: "goal:event:7".to_string(),
            created_at: Some("1715000000".to_string()),
            payload_json: serde_json::json!({"attempt": 1}),
        };
        let mut replayed = event.clone();
        replayed.event_id = uuid::Uuid::new_v4();
        replayed.message = "replayed projection".to_string();
        replayed.payload_json = serde_json::json!({"attempt": 2});

        store.apply_event(event);
        store.apply_event(replayed);

        assert_eq!(store.events[&goal_id].len(), 1);
        assert_eq!(store.events[&goal_id][0].message, "first projection");
        assert_eq!(store.events[&goal_id][0].payload_json["attempt"], 1);
    }

    #[test]
    fn operator_event_projection_is_append_only_and_idempotent() {
        let goal_id = uuid::Uuid::new_v4();
        let mut store = GoalStore::default();
        let event = DurableEventEnvelope {
            event_id: uuid::Uuid::new_v4(),
            event_type: "goal.updated".to_string(),
            actor: OperatorActorRef::goal(goal_id),
            transition: OperatorTransition::GoalSteered,
            idempotency_key: "operator:goal:steer:1".to_string(),
            causation_id: None,
            correlation_id: None,
            restate_invocation_id: None,
            created_at: "2026-05-14T12:00:00Z".to_string(),
            payload_json: serde_json::json!({"message": "first"}),
        };
        let mut replayed = event.clone();
        replayed.event_id = uuid::Uuid::new_v4();
        replayed.payload_json = serde_json::json!({"message": "replayed"});

        store.apply_operator_event(event);
        store.apply_operator_event(replayed);

        assert_eq!(store.operator_events.len(), 1);
        assert_eq!(store.operator_events[0].payload_json["message"], "first");
    }

    #[test]
    fn operator_events_resolve_projected_actions() {
        let state = GoalState::new(GoalSpec::new(
            "Resolve action",
            "Close action queue entries when durable events arrive",
        ));
        let mut request = GoalStoreSnapshotUpsertRequest::from_state(&state, "blocked_projection");
        let goal_id = request.snapshot.goal.goal_id;
        request.snapshot.tasks[0].status = TaskStatus::Blocked;
        let mut store = GoalStore::default();
        store.apply_snapshot(request.snapshot);
        assert_eq!(store.actions.len(), 1);

        store.apply_operator_event(DurableEventEnvelope {
            event_id: uuid::Uuid::new_v4(),
            event_type: "goal.cancelled".to_string(),
            actor: OperatorActorRef::goal(goal_id),
            transition: OperatorTransition::GoalCancelled,
            idempotency_key: "operator:goal:cancel:resolve-actions".to_string(),
            causation_id: None,
            correlation_id: None,
            restate_invocation_id: None,
            created_at: "2026-05-14T12:00:00Z".to_string(),
            payload_json: serde_json::json!({}),
        });

        let action = store.actions.values().next().expect("projected action");
        assert_eq!(action.status, "cancelled");
        assert!(action.payload_json["resolution_event_id"].is_string());
    }

    #[test]
    fn operator_event_filter_scopes_goal_type_and_since() {
        let goal_id = uuid::Uuid::new_v4();
        let other_goal_id = uuid::Uuid::new_v4();
        let event = DurableEventEnvelope {
            event_id: uuid::Uuid::new_v4(),
            event_type: "goal.updated".to_string(),
            actor: OperatorActorRef::goal(goal_id),
            transition: OperatorTransition::GoalSteered,
            idempotency_key: "operator:goal:steer:filter".to_string(),
            causation_id: None,
            correlation_id: None,
            restate_invocation_id: None,
            created_at: "2026-05-14T12:00:00Z".to_string(),
            payload_json: serde_json::json!({}),
        };
        let other = DurableEventEnvelope {
            event_id: uuid::Uuid::new_v4(),
            event_type: "goal.cancelled".to_string(),
            actor: OperatorActorRef::goal(other_goal_id),
            transition: OperatorTransition::GoalCancelled,
            idempotency_key: "operator:goal:cancel:filter".to_string(),
            causation_id: None,
            correlation_id: None,
            restate_invocation_id: None,
            created_at: "2026-05-14T12:01:00Z".to_string(),
            payload_json: serde_json::json!({}),
        };

        let filtered = filter_operator_events(
            vec![event.clone(), other],
            &OperatorEventFilter {
                goal_id: Some(goal_id),
                event_type: Some("goal.updated".to_string()),
                since: Some("2026-05-14T11:59:00Z".to_string()),
                limit: None,
            },
        );

        assert_eq!(filtered, vec![event]);
    }

    #[tokio::test]
    async fn operator_event_handler_persists_memory_event() {
        let state = AppState {
            store: Arc::new(RwLock::new(GoalStore::default())),
            journal_path: None,
            backend: GoalStoreBackend::Memory,
            postgres: None,
        };
        let goal_id = uuid::Uuid::new_v4();
        let event = DurableEventEnvelope {
            event_id: uuid::Uuid::new_v4(),
            event_type: "goal.updated".to_string(),
            actor: OperatorActorRef::goal(goal_id),
            transition: OperatorTransition::SubmitGoal,
            idempotency_key: "operator:goal:submit:handler".to_string(),
            causation_id: None,
            correlation_id: None,
            restate_invocation_id: None,
            created_at: "2026-05-14T12:00:00Z".to_string(),
            payload_json: serde_json::json!({"title": "handler"}),
        };

        let response = append_operator_event(
            State(state.clone()),
            Json(OperatorEventAppendRequest {
                event: event.clone(),
            }),
        )
        .await
        .expect("operator event accepted")
        .0;

        assert!(response.accepted);
        assert_eq!(response.event_id, event.event_id);
        assert_eq!(state.store.read().await.operator_events[0], event);
    }

    #[test]
    fn checkpoint_records_are_queryable_history() {
        let state = GoalState::new(GoalSpec::new("checkpoint", "record checkpoint history"));
        let task = state.tasks.values().next().expect("root task");
        let checkpoint =
            CheckpointRef::metadata_for_task(task, "before-review", "checkpoint before review");
        let request = GoalStoreArtifactRecordRequest {
            metadata: ProtocolMetadata::new(format!("goal:{}:checkpoint:test", task.goal_id)),
            goal_id: task.goal_id,
            task_id: Some(task.id),
            artifacts: Vec::new(),
            git_results: Vec::new(),
            object_artifacts: Vec::new(),
            checkpoints: vec![checkpoint.clone()],
        };
        let records = request.into_records();
        let mut store = GoalStore::default();

        store.apply_artifacts(records);

        let checkpoints = checkpoints_from_artifacts(&store.artifacts[&task.goal_id]);
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].id, checkpoint.id);
        assert_eq!(checkpoints[0].label, "before-review");
    }

    #[test]
    fn event_source_approval_records_are_queryable() {
        let record = EventSourceApprovalRecord {
            record_id: uuid::Uuid::new_v4(),
            approval_ref: "approval-123".to_string(),
            source_id: "ci-events".to_string(),
            source_kind: EventSourceKind::Ci,
            status: EventSourceApprovalStatus::Provided,
            risky: true,
            reason: "enabled source can create or steer work".to_string(),
            operator: Some("local-operator".to_string()),
            recorded_at: Some("1715000000".to_string()),
            payload_json: serde_json::json!({"source_enabled": true}),
        };
        let mut store = GoalStore::default();

        store.apply_event_source_approval(record.clone());

        assert_eq!(store.event_source_approvals.len(), 1);
        assert_eq!(
            store.event_source_approvals[&record.record_id].approval_ref,
            "approval-123"
        );
    }
}
