//! Event ingress service for webhooks, generic events, schedules, and triggered goals.
//!
//! Purpose: normalize external events into `ExternalEvent`, apply auth and
//! dedupe policy, then record, route, or hold them for human review. Event
//! sources never invoke workers directly; they create or steer durable goals
//! through the coordinator boundary.
//!
//! Architecture references:
//! - `docs/design-docs/080-events-webhooks-schedules.md`
//! - `docs/api/event-gateway.asyncapi.yaml`
//! - `docs/exec-plans/active/120-events-webhooks-schedules.md`

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use coat_domain::{
    EventRouteMode, EventSource, EventSourceKind, ExternalEvent, GoalSpec, GoalTriggerTemplate,
    SecretProvider, SecretRef, SteeringDirective, TriggeredGoalRequest, TriggeredGoalResponse,
    TriggeredGoalStatus, WebhookAuthKind,
};
use hmac::{Hmac, Mac};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::RwLock};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct AppState {
    state: Arc<RwLock<EventGatewayState>>,
    journal_path: Option<PathBuf>,
    restate_ingress: Option<String>,
    gateway_token: Option<String>,
    require_event_source_approval: bool,
    backend: EventGatewayBackend,
    postgres: Option<PgPool>,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventGatewayBackend {
    Memory,
    Jsonl,
    Postgres,
}

impl EventGatewayBackend {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "memory" => Ok(Self::Memory),
            "jsonl" => Ok(Self::Jsonl),
            "postgres" | "postgresql" => Ok(Self::Postgres),
            other => anyhow::bail!("unsupported COAT_EVENT_GATEWAY_BACKEND {other:?}"),
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

#[derive(Debug, Default, Clone, Serialize)]
struct EventGatewayState {
    sources: BTreeMap<String, EventSource>,
    events: BTreeMap<String, ExternalEvent>,
    dedupe_keys: BTreeSet<String>,
    triggered_goals: Vec<TriggeredGoalResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum JournalEntry {
    Source(EventSource),
    Event(ExternalEvent),
    Trigger(TriggeredGoalResponse),
}

#[derive(Debug, Deserialize)]
struct ListEventsQuery {
    source_id: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct IngestEventQuery {
    route: Option<bool>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "coat_event_gateway=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9089".to_string());
    let journal_path = std::env::var("COAT_EVENT_GATEWAY_JOURNAL_PATH")
        .or_else(|_| std::env::var("EVENT_GATEWAY_JOURNAL_PATH"))
        .ok()
        .map(PathBuf::from);
    let restate_ingress = std::env::var("COAT_RESTATE_INGRESS")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let gateway_token = std::env::var("COAT_EVENT_GATEWAY_TOKEN")
        .or_else(|_| std::env::var("EVENT_GATEWAY_TOKEN"))
        .ok()
        .filter(|value| !value.trim().is_empty());
    let require_event_source_approval = env_bool("COAT_REQUIRE_EVENT_SOURCE_APPROVAL", false)
        || env_bool("EVENT_GATEWAY_REQUIRE_SOURCE_APPROVAL", false);
    let backend_name = std::env::var("COAT_EVENT_GATEWAY_BACKEND").unwrap_or_else(|_| {
        if journal_path.is_some() {
            "jsonl".to_string()
        } else {
            "memory".to_string()
        }
    });
    let backend = EventGatewayBackend::parse(&backend_name)?;
    let postgres = if backend == EventGatewayBackend::Postgres {
        let database_url = std::env::var("COAT_EVENT_GATEWAY_DATABASE_URL")
            .or_else(|_| std::env::var("COAT_GOAL_STORE_DATABASE_URL"))
            .or_else(|_| std::env::var("DATABASE_URL"))
            .context("COAT_EVENT_GATEWAY_DATABASE_URL is required when backend=postgres")?;
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(&database_url)
            .await
            .context("connect to Postgres event-gateway database")?;
        verify_postgres_schema(&pool).await?;
        Some(pool)
    } else {
        None
    };
    let gateway_state = if let Some(pool) = &postgres {
        load_postgres_state(pool).await?
    } else {
        replay_journal(journal_path.as_ref())?
    };
    let state = AppState {
        state: Arc::new(RwLock::new(gateway_state)),
        journal_path,
        restate_ingress,
        gateway_token,
        require_event_source_approval,
        backend,
        postgres,
        client: reqwest::Client::new(),
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/event-sources", get(list_sources).post(register_source))
        .route("/events", get(list_events).post(ingest_event))
        .route("/events/webhook/{source_id}", post(webhook_event))
        .route("/events/generic/{source_id}", post(generic_event))
        .route("/triggers", get(list_triggers).post(trigger_goal))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "event gateway listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "authority": "event_gateway",
        "event_format": "cloudevents_compatible",
        "backend": state.backend.as_str(),
        "postgres_connected": state.postgres.is_some(),
    }))
}

async fn register_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(source): Json<EventSource>,
) -> Result<Json<EventSource>, GatewayError> {
    require_gateway_auth(&state, &headers)?;
    enforce_event_source_activation_policy(&state, &headers, &source)?;
    if let Some(pool) = &state.postgres {
        upsert_source_postgres(pool, &source).await?;
    } else {
        append_journal(&state, JournalEntry::Source(source.clone())).await?;
    }
    state
        .state
        .write()
        .await
        .sources
        .insert(source.id.clone(), source.clone());
    Ok(Json(source))
}

async fn list_sources(State(state): State<AppState>) -> Json<Vec<EventSource>> {
    if let Some(pool) = &state.postgres {
        match list_sources_postgres(pool).await {
            Ok(sources) => return Json(sources),
            Err(error) => {
                tracing::warn!(%error, "postgres source list failed; falling back to memory")
            }
        }
    }
    Json(state.state.read().await.sources.values().cloned().collect())
}

async fn ingest_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<IngestEventQuery>,
    Json(event): Json<ExternalEvent>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    require_gateway_auth(&state, &headers)?;
    let deduped = record_event(&state, event.clone()).await?;
    if query.route.unwrap_or(false) {
        if let Some(response) = route_event_from_source(&state, &event, deduped).await? {
            return Ok(Json(serde_json::to_value(response)?));
        }
    }
    Ok(Json(serde_json::json!({
        "accepted": true,
        "event_id": event.id,
        "deduped": deduped,
    })))
}

async fn webhook_event(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let source = state.state.read().await.sources.get(&source_id).cloned();
    require_webhook_auth(&state, source.as_ref(), &headers, &body)?;
    let source_kind = source
        .as_ref()
        .map(|source| source.kind.clone())
        .unwrap_or(EventSourceKind::Webhook);
    let payload: serde_json::Value = serde_json::from_slice(&body)
        .unwrap_or_else(|_| serde_json::json!({ "raw_utf8": String::from_utf8_lossy(&body) }));
    let event = normalize_webhook_event(source.as_ref(), source_id, source_kind, &headers, payload);
    let deduped = record_event(&state, event.clone()).await?;

    if let Some(response) = route_event_from_source(&state, &event, deduped).await? {
        return Ok(Json(serde_json::to_value(response)?));
    }

    Ok(Json(serde_json::json!({
        "accepted": true,
        "event_id": event.id,
        "deduped": deduped,
    })))
}

async fn generic_event(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    Query(query): Query<IngestEventQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let source = state.state.read().await.sources.get(&source_id).cloned();
    require_generic_auth(&state, source.as_ref(), &headers, &body)?;
    let event = normalize_generic_event(source.as_ref(), source_id, &headers, &body)?;
    let deduped = record_event(&state, event.clone()).await?;
    if query.route.unwrap_or(true) {
        if let Some(response) = route_event_from_source(&state, &event, deduped).await? {
            return Ok(Json(serde_json::to_value(response)?));
        }
    }
    Ok(Json(serde_json::json!({
        "accepted": true,
        "event_id": event.id,
        "deduped": deduped,
    })))
}

async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<ListEventsQuery>,
) -> Json<Vec<ExternalEvent>> {
    let mut events: Vec<_> = if let Some(pool) = &state.postgres {
        match list_events_postgres(pool, query.source_id.as_deref()).await {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!(%error, "postgres event list failed; falling back to memory");
                state
                    .state
                    .read()
                    .await
                    .events
                    .values()
                    .filter(|event| {
                        query
                            .source_id
                            .as_ref()
                            .is_none_or(|source_id| &event.source_id == source_id)
                    })
                    .cloned()
                    .collect()
            }
        }
    } else {
        state
            .state
            .read()
            .await
            .events
            .values()
            .filter(|event| {
                query
                    .source_id
                    .as_ref()
                    .is_none_or(|source_id| &event.source_id == source_id)
            })
            .cloned()
            .collect()
    };
    events.sort_by(|left, right| left.id.cmp(&right.id));
    if let Some(limit) = query.limit {
        events.truncate(limit);
    }
    Json(events)
}

async fn trigger_goal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TriggeredGoalRequest>,
) -> Result<Json<TriggeredGoalResponse>, GatewayError> {
    require_gateway_auth(&state, &headers)?;
    let deduped = record_event(&state, request.event.clone()).await?;
    Ok(Json(trigger_goal_inner(&state, request, deduped).await?))
}

async fn list_triggers(State(state): State<AppState>) -> Json<Vec<TriggeredGoalResponse>> {
    if let Some(pool) = &state.postgres {
        match list_triggers_postgres(pool).await {
            Ok(triggers) => return Json(triggers),
            Err(error) => {
                tracing::warn!(%error, "postgres trigger list failed; falling back to memory")
            }
        }
    }
    Json(state.state.read().await.triggered_goals.clone())
}

async fn record_event(state: &AppState, event: ExternalEvent) -> Result<bool, GatewayError> {
    if let Some(pool) = &state.postgres {
        let deduped = insert_event_postgres(pool, &event).await?;
        if !deduped {
            let mut store = state.state.write().await;
            store.dedupe_keys.insert(event.dedupe_key.clone());
            store.events.insert(event.id.clone(), event);
        }
        return Ok(deduped);
    }
    let mut store = state.state.write().await;
    let deduped = !store.dedupe_keys.insert(event.dedupe_key.clone());
    if !deduped {
        append_journal(state, JournalEntry::Event(event.clone())).await?;
        store.events.insert(event.id.clone(), event);
    }
    Ok(deduped)
}

async fn route_event_from_source(
    state: &AppState,
    event: &ExternalEvent,
    deduped: bool,
) -> Result<Option<TriggeredGoalResponse>, GatewayError> {
    let source = state
        .state
        .read()
        .await
        .sources
        .get(&event.source_id)
        .cloned();
    let Some(source) = source.filter(|source| source.enabled) else {
        return Ok(None);
    };
    if source.route.mode == EventRouteMode::RecordOnly && !source.route.require_approval {
        return Ok(None);
    }
    let request = TriggeredGoalRequest {
        event: event.clone(),
        route: source.route.clone(),
        goal: None,
        idempotency_key: event.dedupe_key.clone(),
    };
    trigger_goal_inner(state, request, deduped).await.map(Some)
}

async fn trigger_goal_inner(
    state: &AppState,
    request: TriggeredGoalRequest,
    deduped: bool,
) -> Result<TriggeredGoalResponse, GatewayError> {
    if deduped {
        let response = TriggeredGoalResponse {
            accepted: true,
            status: TriggeredGoalStatus::Deduped,
            event_id: request.event.id,
            goal_id: None,
            deduped: true,
            diagnostics: vec!["event dedupe key was already processed".to_string()],
        };
        append_trigger(state, response.clone()).await?;
        return Ok(response);
    }

    if request.route.require_approval || request.route.mode == EventRouteMode::HumanReview {
        let response = TriggeredGoalResponse {
            accepted: true,
            status: TriggeredGoalStatus::AwaitingHumanReview,
            event_id: request.event.id,
            goal_id: request.goal.as_ref().map(|goal| goal.id),
            deduped: false,
            diagnostics: vec!["event route requires human review before goal submit".to_string()],
        };
        append_trigger(state, response.clone()).await?;
        return Ok(response);
    }

    if request.route.mode == EventRouteMode::RecordOnly {
        let response = TriggeredGoalResponse {
            accepted: true,
            status: TriggeredGoalStatus::Recorded,
            event_id: request.event.id,
            goal_id: None,
            deduped: false,
            diagnostics: vec!["event route is record_only".to_string()],
        };
        append_trigger(state, response.clone()).await?;
        return Ok(response);
    }

    if request.route.mode == EventRouteMode::SteerGoal {
        let mut diagnostics = Vec::new();
        let Some(goal_id) = request.route.target_goal_id else {
            let response = TriggeredGoalResponse {
                accepted: false,
                status: TriggeredGoalStatus::Failed,
                event_id: request.event.id,
                goal_id: None,
                deduped: false,
                diagnostics: vec!["steer_goal route requires target_goal_id".to_string()],
            };
            append_trigger(state, response.clone()).await?;
            return Ok(response);
        };
        let Some(directive) = request.route.steering_directive.clone() else {
            let response = TriggeredGoalResponse {
                accepted: false,
                status: TriggeredGoalStatus::Failed,
                event_id: request.event.id,
                goal_id: Some(goal_id),
                deduped: false,
                diagnostics: vec!["steer_goal route requires steering_directive".to_string()],
            };
            append_trigger(state, response.clone()).await?;
            return Ok(response);
        };
        let status = if let Some(restate_ingress) = &state.restate_ingress {
            match steer_goal(&state.client, restate_ingress, goal_id, &directive).await {
                Ok(()) => TriggeredGoalStatus::Submitted,
                Err(error) => {
                    diagnostics.push(format!("steer through Restate failed: {error}"));
                    TriggeredGoalStatus::Failed
                }
            }
        } else {
            diagnostics.push(
                "COAT_RESTATE_INGRESS is not configured; steer request recorded only".to_string(),
            );
            TriggeredGoalStatus::Recorded
        };
        let response = TriggeredGoalResponse {
            accepted: status != TriggeredGoalStatus::Failed,
            status,
            event_id: request.event.id,
            goal_id: Some(goal_id),
            deduped: false,
            diagnostics,
        };
        append_trigger(state, response.clone()).await?;
        return Ok(response);
    }

    let mut diagnostics = Vec::new();
    let goal = match request.goal {
        Some(goal) => Some(goal),
        None => request
            .route
            .goal_template
            .as_ref()
            .map(|template| goal_from_template(template, &request.event)),
    };

    let Some(goal) = goal else {
        let response = TriggeredGoalResponse {
            accepted: true,
            status: TriggeredGoalStatus::Recorded,
            event_id: request.event.id,
            goal_id: None,
            deduped: false,
            diagnostics: vec!["event recorded without a goal template".to_string()],
        };
        append_trigger(state, response.clone()).await?;
        return Ok(response);
    };

    let status = if let Some(restate_ingress) = &state.restate_ingress {
        match submit_goal(&state.client, restate_ingress, &goal).await {
            Ok(()) => TriggeredGoalStatus::Submitted,
            Err(error) => {
                diagnostics.push(format!("submit to Restate failed: {error}"));
                TriggeredGoalStatus::Failed
            }
        }
    } else {
        diagnostics.push("COAT_RESTATE_INGRESS is not configured; goal recorded only".to_string());
        TriggeredGoalStatus::Recorded
    };

    let response = TriggeredGoalResponse {
        accepted: status != TriggeredGoalStatus::Failed,
        status,
        event_id: request.event.id,
        goal_id: Some(goal.id),
        deduped: false,
        diagnostics,
    };
    append_trigger(state, response.clone()).await?;
    Ok(response)
}

fn goal_from_template(template: &GoalTriggerTemplate, event: &ExternalEvent) -> GoalSpec {
    let title = render_template(&template.title_template, event);
    let objective = render_template(&template.objective_template, event);
    let mut goal = GoalSpec::new(title, objective);
    goal.repo = template.repo.clone();
    goal.root_budget = template.budget.clone();
    goal.done_criteria = template.done_criteria.clone();
    goal.default_execution = template
        .execution
        .clone()
        .with_role(template.worker_role.clone());
    goal.authoring.intake_summary = format!(
        "Generated from event {} from source {}",
        event.id, event.source_id
    );
    goal.authoring.acceptance_evidence = vec![
        "event was acknowledged".to_string(),
        "resulting task evidence satisfies done criteria".to_string(),
    ];
    goal
}

fn render_template(template: &str, event: &ExternalEvent) -> String {
    template
        .replace("{{event_id}}", &event.id)
        .replace("{{source_id}}", &event.source_id)
        .replace("{{event_type}}", &event.event_type)
        .replace(
            "{{subject}}",
            event.subject.as_deref().unwrap_or("no subject"),
        )
}

fn normalize_webhook_event(
    source: Option<&EventSource>,
    source_id: String,
    source_kind: EventSourceKind,
    headers: &HeaderMap,
    payload: serde_json::Value,
) -> ExternalEvent {
    let base = webhook_base_event(source, &source_id, &source_kind, headers, &payload);
    let mut event = match source_kind {
        EventSourceKind::GitHubWebhook => normalize_github_webhook(base, headers, &payload),
        EventSourceKind::GitLabWebhook => normalize_gitlab_webhook(base, headers, &payload),
        EventSourceKind::SlackEvent => normalize_slack_event(base, &payload),
        EventSourceKind::StripeWebhook => normalize_stripe_webhook(base, &payload),
        EventSourceKind::JiraWebhook => normalize_jira_webhook(base, &payload),
        EventSourceKind::LinearWebhook => normalize_linear_webhook(base, &payload),
        _ => base,
    };
    if let Some(dedupe_header) = source
        .and_then(|source| source.webhook.as_ref())
        .and_then(|webhook| webhook.dedupe_header.as_ref())
    {
        if let Some(dedupe_key) = header(headers, dedupe_header) {
            event.dedupe_key = dedupe_key;
        }
    }
    event
}

fn webhook_base_event(
    source: Option<&EventSource>,
    source_id: &str,
    source_kind: &EventSourceKind,
    headers: &HeaderMap,
    payload: &serde_json::Value,
) -> ExternalEvent {
    let event_type = header(headers, "ce-type")
        .or_else(|| json_path_string(payload, &["type"]))
        .unwrap_or_else(|| "webhook.received".to_string());
    let event_id = header(headers, "ce-id")
        .or_else(|| json_path_string(payload, &["id"]))
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let subject = header(headers, "ce-subject").or_else(|| json_path_string(payload, &["subject"]));
    let dedupe_key = source
        .and_then(|source| source.webhook.as_ref())
        .and_then(|webhook| webhook.dedupe_header.as_ref())
        .and_then(|name| header(headers, name))
        .unwrap_or_else(|| format!("{source_id}:{event_id}:{event_type}"));
    ExternalEvent {
        id: event_id,
        source_id: source_id.to_string(),
        source_kind: source_kind.clone(),
        event_type,
        subject,
        dedupe_key,
        occurred_at: header(headers, "ce-time"),
        received_at: None,
        headers: header_map(headers),
        payload: payload.clone(),
    }
}

fn normalize_github_webhook(
    mut event: ExternalEvent,
    headers: &HeaderMap,
    payload: &serde_json::Value,
) -> ExternalEvent {
    let github_event = header(headers, "x-github-event")
        .or_else(|| json_path_string(payload, &["event"]))
        .unwrap_or_else(|| "event".to_string());
    let action = json_path_string(payload, &["action"]);
    event.id = header(headers, "x-github-delivery").unwrap_or(event.id);
    event.event_type = provider_event_type("github", &github_event, action.as_deref());
    event.subject = json_path_string(payload, &["repository", "full_name"])
        .map(|repo| {
            json_path_string(payload, &["pull_request", "number"])
                .or_else(|| json_path_string(payload, &["issue", "number"]))
                .map(|number| format!("{repo}#{number}"))
                .unwrap_or(repo)
        })
        .or(event.subject);
    event.dedupe_key = format!("{}:{}", event.source_id, event.id);
    event
}

fn normalize_gitlab_webhook(
    mut event: ExternalEvent,
    headers: &HeaderMap,
    payload: &serde_json::Value,
) -> ExternalEvent {
    let gitlab_event = header(headers, "x-gitlab-event")
        .or_else(|| json_path_string(payload, &["object_kind"]))
        .or_else(|| json_path_string(payload, &["event_name"]))
        .unwrap_or_else(|| "event".to_string());
    event.id = header(headers, "x-gitlab-event-uuid")
        .or_else(|| json_path_string(payload, &["object_attributes", "id"]))
        .unwrap_or(event.id);
    event.event_type = provider_event_type("gitlab", &gitlab_event, None);
    event.subject = json_path_string(payload, &["project", "path_with_namespace"])
        .or_else(|| json_path_string(payload, &["project", "web_url"]))
        .or(event.subject);
    event.dedupe_key = format!("{}:{}", event.source_id, event.id);
    event
}

fn normalize_slack_event(mut event: ExternalEvent, payload: &serde_json::Value) -> ExternalEvent {
    event.id = json_path_string(payload, &["event_id"]).unwrap_or(event.id);
    let inner_type = json_path_string(payload, &["event", "type"]);
    let outer_type = json_path_string(payload, &["type"]).unwrap_or_else(|| "event".to_string());
    event.event_type = provider_event_type("slack", &inner_type.unwrap_or(outer_type), None);
    event.subject = json_path_string(payload, &["event", "channel"])
        .or_else(|| json_path_string(payload, &["event", "user"]))
        .or(event.subject);
    event.dedupe_key = format!("{}:{}", event.source_id, event.id);
    event
}

fn normalize_stripe_webhook(
    mut event: ExternalEvent,
    payload: &serde_json::Value,
) -> ExternalEvent {
    event.id = json_path_string(payload, &["id"]).unwrap_or(event.id);
    event.event_type = json_path_string(payload, &["type"])
        .map(|event_type| format!("stripe.{event_type}"))
        .unwrap_or_else(|| "stripe.event".to_string());
    event.subject = json_path_string(payload, &["data", "object", "id"]).or(event.subject);
    event.dedupe_key = format!("{}:{}", event.source_id, event.id);
    event.occurred_at = json_path_string(payload, &["created"]).or(event.occurred_at);
    event
}

fn normalize_jira_webhook(mut event: ExternalEvent, payload: &serde_json::Value) -> ExternalEvent {
    let webhook_event =
        json_path_string(payload, &["webhookEvent"]).unwrap_or_else(|| "event".to_string());
    event.id = json_path_string(payload, &["webhookEvent"])
        .zip(json_path_string(payload, &["issue", "id"]))
        .map(|(event_type, issue_id)| format!("{event_type}:{issue_id}"))
        .unwrap_or(event.id);
    event.event_type = provider_event_type("jira", &webhook_event, None);
    event.subject = json_path_string(payload, &["issue", "key"]).or(event.subject);
    event.dedupe_key = format!("{}:{}", event.source_id, event.id);
    event
}

fn normalize_linear_webhook(
    mut event: ExternalEvent,
    payload: &serde_json::Value,
) -> ExternalEvent {
    let event_type = json_path_string(payload, &["type"]).unwrap_or_else(|| "event".to_string());
    let action = json_path_string(payload, &["action"]);
    event.id = json_path_string(payload, &["id"])
        .or_else(|| json_path_string(payload, &["data", "id"]))
        .unwrap_or(event.id);
    event.event_type = provider_event_type("linear", &event_type, action.as_deref());
    event.subject = json_path_string(payload, &["data", "identifier"])
        .or_else(|| json_path_string(payload, &["data", "title"]))
        .or(event.subject);
    event.dedupe_key = format!("{}:{}", event.source_id, event.id);
    event
}

fn provider_event_type(provider: &str, event: &str, action: Option<&str>) -> String {
    let base = event
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', ':'], "_");
    action
        .filter(|action| !action.trim().is_empty())
        .map(|action| {
            format!(
                "{provider}.{base}.{}",
                action
                    .trim()
                    .to_ascii_lowercase()
                    .replace([' ', '-', ':'], "_")
            )
        })
        .unwrap_or_else(|| format!("{provider}.{base}"))
}

fn json_path_string(payload: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = payload;
    for key in path {
        current = current.get(*key)?;
    }
    match current {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn normalize_generic_event(
    source: Option<&EventSource>,
    source_id: String,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<ExternalEvent, GatewayError> {
    let generic = source.and_then(|source| source.generic.as_ref());
    if let Some(generic) = generic {
        if generic.max_payload_bytes > 0 && body.len() as u64 > generic.max_payload_bytes {
            return Err(GatewayError::BadRequest(format!(
                "generic event payload exceeds max_payload_bytes {}",
                generic.max_payload_bytes
            )));
        }
    }
    let payload: serde_json::Value = serde_json::from_slice(body)
        .unwrap_or_else(|_| serde_json::json!({ "raw_utf8": String::from_utf8_lossy(body) }));
    let event_type = header(headers, "ce-type")
        .or_else(|| {
            generic
                .and_then(|generic| generic.type_json_pointer.as_deref())
                .and_then(|pointer| json_pointer_string(&payload, pointer))
        })
        .or_else(|| {
            payload
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "generic.received".to_string());
    if let Some(generic) = generic {
        if !generic.allowed_event_types.is_empty()
            && !generic
                .allowed_event_types
                .iter()
                .any(|allowed| allowed == &event_type)
        {
            return Err(GatewayError::BadRequest(format!(
                "event type {event_type} is not allowed for source {source_id}"
            )));
        }
    }
    let event_id = header(headers, "ce-id")
        .or_else(|| {
            generic
                .and_then(|generic| generic.id_json_pointer.as_deref())
                .and_then(|pointer| json_pointer_string(&payload, pointer))
        })
        .or_else(|| {
            payload
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let subject = header(headers, "ce-subject").or_else(|| {
        generic
            .and_then(|generic| generic.subject_json_pointer.as_deref())
            .and_then(|pointer| json_pointer_string(&payload, pointer))
            .or_else(|| {
                payload
                    .get("subject")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
    });
    let dedupe_key = generic
        .and_then(|generic| generic.dedupe_header.as_deref())
        .and_then(|name| header(headers, name))
        .or_else(|| {
            generic
                .and_then(|generic| generic.dedupe_json_pointer.as_deref())
                .and_then(|pointer| json_pointer_string(&payload, pointer))
        })
        .unwrap_or_else(|| format!("{source_id}:{event_id}:{event_type}"));
    let occurred_at = header(headers, "ce-time").or_else(|| {
        payload
            .get("time")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    });
    Ok(ExternalEvent {
        id: event_id,
        source_id,
        source_kind: source
            .map(|source| source.kind.clone())
            .unwrap_or(EventSourceKind::Generic),
        event_type,
        subject,
        dedupe_key,
        occurred_at,
        received_at: None,
        headers: header_map(headers),
        payload,
    })
}

fn json_pointer_string(payload: &serde_json::Value, pointer: &str) -> Option<String> {
    payload.pointer(pointer).and_then(|value| match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    })
}

async fn submit_goal(
    client: &reqwest::Client,
    restate_ingress: &str,
    goal: &GoalSpec,
) -> Result<(), String> {
    let url = format!(
        "{}/GoalWorkflow/{}/run",
        restate_ingress.trim_end_matches('/'),
        goal.id
    );
    let response = client
        .post(url)
        .json(goal)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!(
            "status {status}: {}",
            response.text().await.unwrap_or_default()
        ))
    }
}

async fn steer_goal(
    client: &reqwest::Client,
    restate_ingress: &str,
    goal_id: Uuid,
    directive: &SteeringDirective,
) -> Result<(), String> {
    let url = format!(
        "{}/GoalWorkflow/{}/steer",
        restate_ingress.trim_end_matches('/'),
        goal_id
    );
    let response = client
        .post(url)
        .json(directive)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!(
            "status {status}: {}",
            response.text().await.unwrap_or_default()
        ))
    }
}

async fn append_trigger(
    state: &AppState,
    response: TriggeredGoalResponse,
) -> Result<(), GatewayError> {
    if let Some(pool) = &state.postgres {
        insert_trigger_postgres(pool, &response).await?;
    } else {
        append_journal(state, JournalEntry::Trigger(response.clone())).await?;
    }
    state.state.write().await.triggered_goals.push(response);
    Ok(())
}

fn require_gateway_auth(state: &AppState, headers: &HeaderMap) -> Result<(), GatewayError> {
    let Some(expected) = &state.gateway_token else {
        return Ok(());
    };
    let authorized = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {expected}"));
    if authorized {
        Ok(())
    } else {
        Err(GatewayError::Unauthorized)
    }
}

fn enforce_event_source_activation_policy(
    state: &AppState,
    headers: &HeaderMap,
    source: &EventSource,
) -> Result<(), GatewayError> {
    if !state.require_event_source_approval || !source.enabled || !event_source_is_risky(source) {
        return Ok(());
    }
    let approval_id = header(headers, "x-coat-approval-id")
        .or_else(|| header(headers, "x-approval-id"))
        .unwrap_or_default();
    if approval_id.trim().is_empty() {
        return Err(GatewayError::BadRequest(format!(
            "event source {} requires an approval reference before activation; register it disabled or provide x-coat-approval-id",
            source.id
        )));
    }
    Ok(())
}

fn event_source_is_risky(source: &EventSource) -> bool {
    source.webhook.is_some()
        || source.generic.is_some()
        || source.schedule.is_some()
        || source.calendar.is_some()
        || source.route.require_approval
        || !matches!(source.route.mode, EventRouteMode::RecordOnly)
        || !matches!(
            source.kind,
            EventSourceKind::Manual | EventSourceKind::Other
        )
}

fn require_webhook_auth(
    state: &AppState,
    source: Option<&EventSource>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), GatewayError> {
    let Some(source) = source else {
        return require_gateway_auth(state, headers);
    };
    let Some(webhook) = &source.webhook else {
        return require_gateway_auth(state, headers);
    };
    let defaults = provider_webhook_defaults(&source.kind);

    require_event_auth(
        state,
        &webhook.auth,
        headers,
        body,
        defaults.secret_header,
        defaults.signature_header,
        defaults.signature_style,
    )
}

fn require_generic_auth(
    state: &AppState,
    source: Option<&EventSource>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), GatewayError> {
    let Some(source) = source else {
        return require_gateway_auth(state, headers);
    };
    let Some(generic) = &source.generic else {
        return require_gateway_auth(state, headers);
    };
    require_event_auth(
        state,
        &generic.auth,
        headers,
        body,
        "x-coat-event-secret",
        "x-coat-signature-256",
        WebhookSignatureStyle::GenericSha256,
    )
}

fn require_event_auth(
    state: &AppState,
    auth: &coat_domain::WebhookAuthPolicy,
    headers: &HeaderMap,
    body: &[u8],
    default_secret_header: &str,
    default_signature_header: &str,
    signature_style: WebhookSignatureStyle,
) -> Result<(), GatewayError> {
    match auth.kind {
        WebhookAuthKind::None => require_gateway_auth(state, headers),
        WebhookAuthKind::SharedSecretHeader => {
            let header_name = auth.header_name.as_deref().unwrap_or(default_secret_header);
            let provided = header(headers, header_name).ok_or(GatewayError::Unauthorized)?;
            let expected = resolve_secret(auth.secret_ref.as_ref())?;
            if constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
                Ok(())
            } else {
                Err(GatewayError::Unauthorized)
            }
        }
        WebhookAuthKind::BearerToken => {
            let expected = resolve_secret(auth.secret_ref.as_ref())?;
            let authorized = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == format!("Bearer {expected}"));
            if authorized {
                Ok(())
            } else {
                Err(GatewayError::Unauthorized)
            }
        }
        WebhookAuthKind::HmacSha256 => {
            let header_name = auth
                .header_name
                .as_deref()
                .unwrap_or(default_signature_header);
            let provided = header(headers, header_name).ok_or(GatewayError::Unauthorized)?;
            let secret = resolve_secret(auth.secret_ref.as_ref())?;
            verify_provider_hmac_sha256(&secret, headers, body, &provided, signature_style)
        }
        WebhookAuthKind::Basic | WebhookAuthKind::Mtls | WebhookAuthKind::OidcJwt => {
            Err(GatewayError::BadRequest(format!(
                "webhook auth kind {:?} is declared but not implemented in the local gateway",
                auth.kind
            )))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebhookSignatureStyle {
    GenericSha256,
    SlackV0,
    StripeV1,
}

#[derive(Debug, Clone, Copy)]
struct WebhookAuthDefaults {
    secret_header: &'static str,
    signature_header: &'static str,
    signature_style: WebhookSignatureStyle,
}

fn provider_webhook_defaults(kind: &EventSourceKind) -> WebhookAuthDefaults {
    match kind {
        EventSourceKind::GitHubWebhook => WebhookAuthDefaults {
            secret_header: "x-coat-webhook-secret",
            signature_header: "x-hub-signature-256",
            signature_style: WebhookSignatureStyle::GenericSha256,
        },
        EventSourceKind::GitLabWebhook => WebhookAuthDefaults {
            secret_header: "x-gitlab-token",
            signature_header: "x-gitlab-token",
            signature_style: WebhookSignatureStyle::GenericSha256,
        },
        EventSourceKind::SlackEvent => WebhookAuthDefaults {
            secret_header: "x-coat-webhook-secret",
            signature_header: "x-slack-signature",
            signature_style: WebhookSignatureStyle::SlackV0,
        },
        EventSourceKind::StripeWebhook => WebhookAuthDefaults {
            secret_header: "x-coat-webhook-secret",
            signature_header: "stripe-signature",
            signature_style: WebhookSignatureStyle::StripeV1,
        },
        _ => WebhookAuthDefaults {
            secret_header: "x-coat-webhook-secret",
            signature_header: "x-coat-signature-256",
            signature_style: WebhookSignatureStyle::GenericSha256,
        },
    }
}

fn resolve_secret(secret_ref: Option<&SecretRef>) -> Result<String, GatewayError> {
    let Some(secret_ref) = secret_ref else {
        return Err(GatewayError::BadRequest(
            "webhook auth requires secret_ref".to_string(),
        ));
    };
    match secret_ref.provider {
        SecretProvider::Env => std::env::var(
            secret_ref
                .key
                .as_deref()
                .unwrap_or(secret_ref.name.as_str()),
        )
        .map_err(|_| GatewayError::Unauthorized),
        SecretProvider::LocalFile => std::fs::read_to_string(&secret_ref.name)
            .map(|value| value.trim().to_string())
            .map_err(|error| GatewayError::BadRequest(format!("read local secret file: {error}"))),
        _ => Err(GatewayError::BadRequest(format!(
            "secret provider {:?} must be resolved by production secret middleware",
            secret_ref.provider
        ))),
    }
}

fn verify_hmac_sha256(secret: &str, body: &[u8], provided: &str) -> Result<(), GatewayError> {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|error| GatewayError::BadRequest(format!("invalid hmac key: {error}")))?;
    mac.update(body);
    let expected = mac.finalize().into_bytes();
    let provided = provided
        .strip_prefix("sha256=")
        .or_else(|| provided.strip_prefix("v0="))
        .unwrap_or(provided);
    let provided = decode_hex(provided).ok_or(GatewayError::Unauthorized)?;
    if constant_time_eq(expected.as_ref(), &provided) {
        Ok(())
    } else {
        Err(GatewayError::Unauthorized)
    }
}

fn verify_provider_hmac_sha256(
    secret: &str,
    headers: &HeaderMap,
    body: &[u8],
    provided: &str,
    style: WebhookSignatureStyle,
) -> Result<(), GatewayError> {
    match style {
        WebhookSignatureStyle::GenericSha256 => verify_hmac_sha256(secret, body, provided),
        WebhookSignatureStyle::SlackV0 => {
            verify_slack_v0_signature(secret, headers, body, provided)
        }
        WebhookSignatureStyle::StripeV1 => verify_stripe_v1_signature(secret, body, provided),
    }
}

fn verify_slack_v0_signature(
    secret: &str,
    headers: &HeaderMap,
    body: &[u8],
    provided: &str,
) -> Result<(), GatewayError> {
    let timestamp =
        header(headers, "x-slack-request-timestamp").ok_or(GatewayError::Unauthorized)?;
    ensure_recent_unix_timestamp(&timestamp, 300)?;
    let mut base = Vec::new();
    base.extend_from_slice(b"v0:");
    base.extend_from_slice(timestamp.as_bytes());
    base.extend_from_slice(b":");
    base.extend_from_slice(body);
    verify_hmac_sha256(secret, &base, provided)
}

fn verify_stripe_v1_signature(
    secret: &str,
    body: &[u8],
    provided: &str,
) -> Result<(), GatewayError> {
    let parts = parse_comma_kv_header(provided);
    let timestamp = parts.get("t").ok_or(GatewayError::Unauthorized)?;
    ensure_recent_unix_timestamp(timestamp, 300)?;
    let signature = parts.get("v1").ok_or(GatewayError::Unauthorized)?;
    let mut base = Vec::new();
    base.extend_from_slice(timestamp.as_bytes());
    base.extend_from_slice(b".");
    base.extend_from_slice(body);
    verify_hmac_sha256(secret, &base, signature)
}

fn parse_comma_kv_header(value: &str) -> BTreeMap<String, String> {
    value
        .split(',')
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn ensure_recent_unix_timestamp(
    timestamp: &str,
    tolerance_seconds: u64,
) -> Result<(), GatewayError> {
    let timestamp = timestamp
        .parse::<u64>()
        .map_err(|_| GatewayError::Unauthorized)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| GatewayError::Unauthorized)?
        .as_secs();
    let delta = now.abs_diff(timestamp);
    if delta <= tolerance_seconds {
        Ok(())
    } else {
        Err(GatewayError::Unauthorized)
    }
}

fn decode_hex(input: &str) -> Option<Vec<u8>> {
    let input = input.trim();
    if !input.len().is_multiple_of(2) {
        return None;
    }
    (0..input.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&input[index..index + 2], 16).ok())
        .collect()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn header_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect()
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(default)
}

fn replay_journal(path: Option<&PathBuf>) -> anyhow::Result<EventGatewayState> {
    let Some(path) = path else {
        return Ok(EventGatewayState::default());
    };
    if !path.exists() {
        return Ok(EventGatewayState::default());
    }
    let mut state = EventGatewayState::default();
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: JournalEntry = serde_json::from_str(line)
            .with_context(|| format!("parse {} line {}", path.display(), index + 1))?;
        match entry {
            JournalEntry::Source(source) => {
                state.sources.insert(source.id.clone(), source);
            }
            JournalEntry::Event(event) => {
                state.dedupe_keys.insert(event.dedupe_key.clone());
                state.events.insert(event.id.clone(), event);
            }
            JournalEntry::Trigger(trigger) => {
                state.triggered_goals.push(trigger);
            }
        }
    }
    Ok(state)
}

async fn verify_postgres_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query("SELECT 1 FROM coat.event_sources LIMIT 1")
        .execute(pool)
        .await
        .context(
            "verify coat.event_sources exists; run infra/db/migrations/002_event_gateway.sql",
        )?;
    Ok(())
}

async fn load_postgres_state(pool: &PgPool) -> anyhow::Result<EventGatewayState> {
    let sources = list_sources_postgres(pool).await?;
    let events = list_events_postgres(pool, None).await?;
    let triggers = list_triggers_postgres(pool).await?;
    let mut state = EventGatewayState::default();
    for source in sources {
        state.sources.insert(source.id.clone(), source);
    }
    for event in events {
        state.dedupe_keys.insert(event.dedupe_key.clone());
        state.events.insert(event.id.clone(), event);
    }
    state.triggered_goals = triggers;
    Ok(state)
}

async fn upsert_source_postgres(pool: &PgPool, source: &EventSource) -> anyhow::Result<()> {
    let auth_policy = source
        .webhook
        .as_ref()
        .map(|webhook| serde_json::to_value(&webhook.auth))
        .or_else(|| {
            source
                .generic
                .as_ref()
                .map(|generic| serde_json::to_value(&generic.auth))
        })
        .transpose()?
        .unwrap_or_else(|| serde_json::json!({}));
    let status = if source.enabled { "active" } else { "disabled" };
    sqlx::query(
        r#"
        INSERT INTO coat.event_sources (
            id, source_key, kind, display_name, status, auth_policy, route_policy,
            schedule, cursor_state, record_json
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '{}'::jsonb, $9)
        ON CONFLICT (id) DO UPDATE SET
            source_key = EXCLUDED.source_key,
            kind = EXCLUDED.kind,
            display_name = EXCLUDED.display_name,
            status = EXCLUDED.status,
            auth_policy = EXCLUDED.auth_policy,
            route_policy = EXCLUDED.route_policy,
            schedule = EXCLUDED.schedule,
            record_json = EXCLUDED.record_json,
            updated_at = now(),
            disabled_at = CASE WHEN EXCLUDED.status = 'disabled' THEN now() ELSE NULL END
        "#,
    )
    .bind(&source.id)
    .bind(&source.id)
    .bind(json_string(&source.kind)?)
    .bind(&source.description)
    .bind(status)
    .bind(auth_policy)
    .bind(serde_json::to_value(&source.route)?)
    .bind(serde_json::to_value(&source.schedule)?)
    .bind(serde_json::to_value(source)?)
    .execute(pool)
    .await
    .with_context(|| format!("upsert event source {}", source.id))?;
    Ok(())
}

async fn list_sources_postgres(pool: &PgPool) -> anyhow::Result<Vec<EventSource>> {
    let rows = sqlx::query("SELECT record_json FROM coat.event_sources ORDER BY source_key")
        .fetch_all(pool)
        .await
        .context("query event sources")?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "event source"))
        .collect()
}

async fn insert_event_postgres(pool: &PgPool, event: &ExternalEvent) -> anyhow::Result<bool> {
    let cloud_event_id = event.headers.get("ce-id").cloned();
    let cloud_event_source = event.headers.get("ce-source").cloned();
    let result = sqlx::query(
        r#"
        INSERT INTO coat.external_events (
            id, source_id, source_key, event_type, subject, dedupe_key, cloud_event_id,
            cloud_event_source, occurred_at, payload, headers, record_json
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(&event.id)
    .bind(&event.source_id)
    .bind(&event.source_id)
    .bind(&event.event_type)
    .bind(&event.subject)
    .bind(&event.dedupe_key)
    .bind(cloud_event_id)
    .bind(cloud_event_source)
    .bind(&event.occurred_at)
    .bind(event.payload.clone())
    .bind(serde_json::to_value(&event.headers)?)
    .bind(serde_json::to_value(event)?)
    .execute(pool)
    .await
    .with_context(|| format!("insert external event {}", event.id))?;
    Ok(result.rows_affected() == 0)
}

async fn list_events_postgres(
    pool: &PgPool,
    source_id: Option<&str>,
) -> anyhow::Result<Vec<ExternalEvent>> {
    let rows = if let Some(source_id) = source_id {
        sqlx::query(
            "SELECT record_json FROM coat.external_events WHERE source_id = $1 ORDER BY observed_at DESC",
        )
        .bind(source_id)
        .fetch_all(pool)
        .await
        .with_context(|| format!("query external events for source {source_id}"))?
    } else {
        sqlx::query("SELECT record_json FROM coat.external_events ORDER BY observed_at DESC")
            .fetch_all(pool)
            .await
            .context("query external events")?
    };
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "external event"))
        .collect()
}

async fn insert_trigger_postgres(
    pool: &PgPool,
    response: &TriggeredGoalResponse,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO coat.triggered_goals (
            id, external_event_id, route_mode, status, goal_id, target_goal_id,
            template, result, record_json
        )
        VALUES ($1, $2, $3, $4, $5, $6, '{}'::jsonb, $7, $8)
        ON CONFLICT (id) DO UPDATE SET
            status = EXCLUDED.status,
            goal_id = EXCLUDED.goal_id,
            target_goal_id = EXCLUDED.target_goal_id,
            result = EXCLUDED.result,
            record_json = EXCLUDED.record_json,
            updated_at = now(),
            completed_at = CASE
                WHEN EXCLUDED.status IN ('submitted', 'recorded', 'deduped', 'failed') THEN now()
                ELSE coat.triggered_goals.completed_at
            END
        "#,
    )
    .bind(trigger_record_id(response))
    .bind(&response.event_id)
    .bind(route_mode_for_trigger(response))
    .bind(json_string(&response.status)?)
    .bind(response.goal_id)
    .bind(response.goal_id)
    .bind(serde_json::to_value(response)?)
    .bind(serde_json::to_value(response)?)
    .execute(pool)
    .await
    .with_context(|| format!("insert triggered goal for event {}", response.event_id))?;
    Ok(())
}

async fn list_triggers_postgres(pool: &PgPool) -> anyhow::Result<Vec<TriggeredGoalResponse>> {
    let rows = sqlx::query("SELECT record_json FROM coat.triggered_goals ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
        .context("query triggered goals")?;
    rows.into_iter()
        .map(|row| decode_record(row.try_get("record_json")?, "triggered goal"))
        .collect()
}

fn trigger_record_id(response: &TriggeredGoalResponse) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "coat://event/{}/trigger/{:?}/{:?}",
            response.event_id, response.status, response.goal_id
        )
        .as_bytes(),
    )
}

fn route_mode_for_trigger(response: &TriggeredGoalResponse) -> &'static str {
    match response.status {
        TriggeredGoalStatus::AwaitingHumanReview => "human_review",
        TriggeredGoalStatus::Recorded | TriggeredGoalStatus::Deduped => "record_only",
        TriggeredGoalStatus::Submitted => "create_goal",
        TriggeredGoalStatus::Failed => "record_only",
    }
}

fn decode_record<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
    kind: &str,
) -> anyhow::Result<T> {
    serde_json::from_value(value).with_context(|| format!("decode {kind} record_json"))
}

fn json_string(value: &impl Serialize) -> anyhow::Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(value) => Ok(value),
        other => anyhow::bail!("expected enum to serialize as string, got {other}"),
    }
}

async fn append_journal(state: &AppState, entry: JournalEntry) -> Result<(), GatewayError> {
    let Some(path) = &state.journal_path else {
        return Ok(());
    };
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
    Ok(())
}

#[derive(Debug)]
enum GatewayError {
    Unauthorized,
    BadRequest(String),
    Internal(String),
}

impl From<anyhow::Error> for GatewayError {
    fn from(value: anyhow::Error) -> Self {
        Self::Internal(value.to_string())
    }
}

impl From<std::io::Error> for GatewayError {
    fn from(value: std::io::Error) -> Self {
        Self::Internal(value.to_string())
    }
}

impl From<serde_json::Error> for GatewayError {
    fn from(value: serde_json::Error) -> Self {
        Self::Internal(value.to_string())
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "missing or invalid event gateway token",
            )
                .into_response(),
            Self::BadRequest(error) => (
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "error": error }).to_string(),
            )
                .into_response(),
            Self::Internal(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": error }).to_string(),
            )
                .into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};
    use coat_domain::{
        EventGoalRoute, EventSourceKind, ExternalEvent, GenericEventSource, GoalTriggerTemplate,
        WebhookAuthKind, WebhookAuthPolicy,
    };
    use hmac::Mac;

    use super::{
        AppState, WebhookSignatureStyle, enforce_event_source_activation_policy,
        event_source_is_risky, goal_from_template, normalize_generic_event,
        normalize_webhook_event, provider_webhook_defaults, render_template, verify_hmac_sha256,
        verify_provider_hmac_sha256,
    };

    fn event() -> ExternalEvent {
        ExternalEvent {
            id: "event-1".to_string(),
            source_id: "calendar".to_string(),
            source_kind: EventSourceKind::CalendarPoll,
            event_type: "calendar.window_due".to_string(),
            subject: Some("Focus block".to_string()),
            dedupe_key: "calendar:event-1".to_string(),
            occurred_at: None,
            received_at: None,
            headers: Default::default(),
            payload: serde_json::json!({ "summary": "Focus block" }),
        }
    }

    #[test]
    fn template_expands_event_fields() {
        assert_eq!(
            render_template("{{source_id}} {{event_type}} {{subject}}", &event()),
            "calendar calendar.window_due Focus block"
        );
    }

    #[test]
    fn template_builds_goal_spec() {
        let goal = goal_from_template(&GoalTriggerTemplate::default(), &event());
        assert!(goal.title.contains("calendar.window_due"));
        assert!(goal.objective.contains("event-1"));
    }

    #[test]
    fn verifies_sha256_hmac_signature() {
        assert!(
            verify_hmac_sha256(
                "secret",
                b"payload",
                "sha256=b82fcb791acec57859b989b430a826488ce2e479fdf92326bd0a2e8375a42ba4",
            )
            .is_ok()
        );
        assert!(verify_hmac_sha256("secret", b"payload", "sha256=deadbeef").is_err());
    }

    #[test]
    fn provider_defaults_match_common_webhook_headers() {
        let github = provider_webhook_defaults(&EventSourceKind::GitHubWebhook);
        assert_eq!(github.signature_header, "x-hub-signature-256");
        assert_eq!(github.signature_style, WebhookSignatureStyle::GenericSha256);

        let slack = provider_webhook_defaults(&EventSourceKind::SlackEvent);
        assert_eq!(slack.signature_header, "x-slack-signature");
        assert_eq!(slack.signature_style, WebhookSignatureStyle::SlackV0);

        let stripe = provider_webhook_defaults(&EventSourceKind::StripeWebhook);
        assert_eq!(stripe.signature_header, "stripe-signature");
        assert_eq!(stripe.signature_style, WebhookSignatureStyle::StripeV1);
    }

    #[test]
    fn normalizes_github_webhook_subject_and_dedupe() {
        let mut headers = HeaderMap::new();
        headers.insert("x-github-event", HeaderValue::from_static("pull_request"));
        headers.insert(
            "x-github-delivery",
            HeaderValue::from_static("delivery-123"),
        );
        let event = normalize_webhook_event(
            None,
            "github-source".to_string(),
            EventSourceKind::GitHubWebhook,
            &headers,
            serde_json::json!({
                "action": "opened",
                "repository": {"full_name": "example/repo"},
                "pull_request": {"number": 42}
            }),
        );

        assert_eq!(event.id, "delivery-123");
        assert_eq!(event.event_type, "github.pull_request.opened");
        assert_eq!(event.subject.as_deref(), Some("example/repo#42"));
        assert_eq!(event.dedupe_key, "github-source:delivery-123");
    }

    #[test]
    fn normalizes_slack_and_stripe_webhook_shapes() {
        let slack = normalize_webhook_event(
            None,
            "slack-source".to_string(),
            EventSourceKind::SlackEvent,
            &HeaderMap::new(),
            serde_json::json!({
                "type": "event_callback",
                "event_id": "Ev123",
                "event": {"type": "app_mention", "channel": "C123"}
            }),
        );
        assert_eq!(slack.id, "Ev123");
        assert_eq!(slack.event_type, "slack.app_mention");
        assert_eq!(slack.subject.as_deref(), Some("C123"));

        let stripe = normalize_webhook_event(
            None,
            "stripe-source".to_string(),
            EventSourceKind::StripeWebhook,
            &HeaderMap::new(),
            serde_json::json!({
                "id": "evt_123",
                "type": "checkout.session.completed",
                "created": 1770000000,
                "data": {"object": {"id": "cs_123"}}
            }),
        );
        assert_eq!(stripe.id, "evt_123");
        assert_eq!(stripe.event_type, "stripe.checkout.session.completed");
        assert_eq!(stripe.subject.as_deref(), Some("cs_123"));
        assert_eq!(stripe.occurred_at.as_deref(), Some("1770000000"));
    }

    #[test]
    fn verifies_slack_v0_signature_base_string() {
        let timestamp = unix_now_string();
        let body = br#"{"type":"event_callback"}"#;
        let mut base = Vec::new();
        base.extend_from_slice(b"v0:");
        base.extend_from_slice(timestamp.as_bytes());
        base.extend_from_slice(b":");
        base.extend_from_slice(body);
        let signature = format!("v0={}", hmac_sha256_hex("secret", &base));

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-slack-request-timestamp",
            HeaderValue::from_str(&timestamp).expect("timestamp header"),
        );

        assert!(
            verify_provider_hmac_sha256(
                "secret",
                &headers,
                body,
                &signature,
                WebhookSignatureStyle::SlackV0,
            )
            .is_ok()
        );
    }

    #[test]
    fn verifies_stripe_v1_signature_base_string() {
        let timestamp = unix_now_string();
        let body = br#"{"id":"evt_123"}"#;
        let mut base = Vec::new();
        base.extend_from_slice(timestamp.as_bytes());
        base.extend_from_slice(b".");
        base.extend_from_slice(body);
        let signature = format!("t={timestamp},v1={}", hmac_sha256_hex("secret", &base));

        assert!(
            verify_provider_hmac_sha256(
                "secret",
                &HeaderMap::new(),
                body,
                &signature,
                WebhookSignatureStyle::StripeV1,
            )
            .is_ok()
        );
    }

    #[test]
    fn normalizes_generic_event_with_json_pointers() {
        let source = coat_domain::EventSource {
            id: "ci-events".to_string(),
            kind: EventSourceKind::Ci,
            enabled: true,
            description: "CI events".to_string(),
            namespace: None,
            webhook: None,
            generic: Some(GenericEventSource {
                auth: WebhookAuthPolicy {
                    kind: WebhookAuthKind::None,
                    secret_ref: None,
                    header_name: None,
                },
                accepts_cloudevents: true,
                max_payload_bytes: 1024,
                allowed_event_types: vec!["ci.workflow.failed".to_string()],
                id_json_pointer: Some("/id".to_string()),
                type_json_pointer: Some("/type".to_string()),
                subject_json_pointer: Some("/subject".to_string()),
                dedupe_json_pointer: Some("/delivery_id".to_string()),
                dedupe_header: None,
                payload_schema: None,
                mcp_context: None,
            }),
            schedule: None,
            calendar: None,
            route: EventGoalRoute {
                mode: coat_domain::EventRouteMode::HumanReview,
                goal_template: None,
                target_goal_id: None,
                steering_directive: None,
                require_approval: true,
                dedupe_window_seconds: 3600,
            },
        };
        let event = normalize_generic_event(
            Some(&source),
            "ci-events".to_string(),
            &HeaderMap::new(),
            br#"{
                "id": "run-1",
                "type": "ci.workflow.failed",
                "subject": "tests failed",
                "delivery_id": "delivery-1"
            }"#,
        )
        .expect("generic event normalizes");
        assert_eq!(event.id, "run-1");
        assert_eq!(event.event_type, "ci.workflow.failed");
        assert_eq!(event.subject.as_deref(), Some("tests failed"));
        assert_eq!(event.dedupe_key, "delivery-1");
        assert_eq!(event.source_kind, EventSourceKind::Ci);
    }

    #[test]
    fn approval_policy_blocks_risky_source_activation_without_reference() {
        let source = coat_domain::EventSource {
            id: "calendar-daily-brief".to_string(),
            kind: EventSourceKind::CalendarPoll,
            enabled: true,
            description: "calendar poller".to_string(),
            namespace: None,
            webhook: None,
            generic: None,
            schedule: None,
            calendar: None,
            route: EventGoalRoute {
                mode: coat_domain::EventRouteMode::HumanReview,
                goal_template: None,
                target_goal_id: None,
                steering_directive: None,
                require_approval: true,
                dedupe_window_seconds: 3600,
            },
        };
        assert!(event_source_is_risky(&source));

        let state = AppState {
            state: Default::default(),
            journal_path: None,
            restate_ingress: None,
            gateway_token: None,
            require_event_source_approval: true,
            backend: super::EventGatewayBackend::Memory,
            postgres: None,
            client: reqwest::Client::new(),
        };
        let headers = HeaderMap::new();
        assert!(enforce_event_source_activation_policy(&state, &headers, &source).is_err());

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-coat-approval-id",
            HeaderValue::from_static("approval-123"),
        );
        assert!(enforce_event_source_activation_policy(&state, &headers, &source).is_ok());

        let mut disabled_source = source;
        disabled_source.enabled = false;
        assert!(
            enforce_event_source_activation_policy(&state, &HeaderMap::new(), &disabled_source)
                .is_ok()
        );
    }

    fn unix_now_string() -> String {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_secs()
            .to_string()
    }

    fn hmac_sha256_hex(secret: &str, body: &[u8]) -> String {
        type HmacSha256 = hmac::Hmac<sha2::Sha256>;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
        mac.update(body);
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
