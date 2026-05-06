use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
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
    client: reqwest::Client,
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
    let state = AppState {
        state: Arc::new(RwLock::new(replay_journal(journal_path.as_ref())?)),
        journal_path,
        restate_ingress,
        gateway_token,
        require_event_source_approval,
        client: reqwest::Client::new(),
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/event-sources", get(list_sources).post(register_source))
        .route("/events", get(list_events).post(ingest_event))
        .route("/events/webhook/{source_id}", post(webhook_event))
        .route("/triggers", get(list_triggers).post(trigger_goal))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "event gateway listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "authority": "event_gateway",
        "event_format": "cloudevents_compatible",
    }))
}

async fn register_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(source): Json<EventSource>,
) -> Result<Json<EventSource>, GatewayError> {
    require_gateway_auth(&state, &headers)?;
    enforce_event_source_activation_policy(&state, &headers, &source)?;
    append_journal(&state, JournalEntry::Source(source.clone())).await?;
    state
        .state
        .write()
        .await
        .sources
        .insert(source.id.clone(), source.clone());
    Ok(Json(source))
}

async fn list_sources(State(state): State<AppState>) -> Json<Vec<EventSource>> {
    Json(state.state.read().await.sources.values().cloned().collect())
}

async fn ingest_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(event): Json<ExternalEvent>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    require_gateway_auth(&state, &headers)?;
    let deduped = record_event(&state, event.clone()).await?;
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
    let event_type = header(&headers, "ce-type")
        .or_else(|| {
            payload
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "webhook.received".to_string());
    let event_id = header(&headers, "ce-id")
        .or_else(|| {
            payload
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let subject = header(&headers, "ce-subject").or_else(|| {
        payload
            .get("subject")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    });
    let dedupe_key = source
        .as_ref()
        .and_then(|source| source.webhook.as_ref())
        .and_then(|webhook| webhook.dedupe_header.as_ref())
        .and_then(|name| header(&headers, name))
        .unwrap_or_else(|| format!("{source_id}:{event_id}:{event_type}"));
    let event = ExternalEvent {
        id: event_id,
        source_id,
        source_kind,
        event_type,
        subject,
        dedupe_key,
        occurred_at: header(&headers, "ce-time"),
        received_at: None,
        headers: header_map(&headers),
        payload,
    };
    let deduped = record_event(&state, event.clone()).await?;

    if let Some(source) = source {
        if matches!(
            source.route.mode,
            EventRouteMode::CreateGoal
                | EventRouteMode::CreateResearchGoal
                | EventRouteMode::SteerGoal
                | EventRouteMode::HumanReview
        ) {
            let request = TriggeredGoalRequest {
                event: event.clone(),
                route: source.route.clone(),
                goal: None,
                idempotency_key: event.dedupe_key.clone(),
            };
            let response = trigger_goal_inner(&state, request, deduped).await?;
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
    let mut events: Vec<_> = state
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
        .collect();
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
    Json(state.state.read().await.triggered_goals.clone())
}

async fn record_event(state: &AppState, event: ExternalEvent) -> Result<bool, GatewayError> {
    let mut store = state.state.write().await;
    let deduped = !store.dedupe_keys.insert(event.dedupe_key.clone());
    if !deduped {
        append_journal(state, JournalEntry::Event(event.clone())).await?;
        store.events.insert(event.id.clone(), event);
    }
    Ok(deduped)
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
    append_journal(state, JournalEntry::Trigger(response.clone())).await?;
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

    match webhook.auth.kind {
        WebhookAuthKind::None => require_gateway_auth(state, headers),
        WebhookAuthKind::SharedSecretHeader => {
            let header_name = webhook
                .auth
                .header_name
                .as_deref()
                .unwrap_or("x-coat-webhook-secret");
            let provided = header(headers, header_name).ok_or(GatewayError::Unauthorized)?;
            let expected = resolve_secret(webhook.auth.secret_ref.as_ref())?;
            if constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
                Ok(())
            } else {
                Err(GatewayError::Unauthorized)
            }
        }
        WebhookAuthKind::BearerToken => {
            let expected = resolve_secret(webhook.auth.secret_ref.as_ref())?;
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
            let header_name = webhook
                .auth
                .header_name
                .as_deref()
                .unwrap_or("x-coat-signature-256");
            let provided = header(headers, header_name).ok_or(GatewayError::Unauthorized)?;
            let secret = resolve_secret(webhook.auth.secret_ref.as_ref())?;
            verify_hmac_sha256(&secret, body, &provided)
        }
        WebhookAuthKind::Basic | WebhookAuthKind::Mtls | WebhookAuthKind::OidcJwt => {
            Err(GatewayError::BadRequest(format!(
                "webhook auth kind {:?} is declared but not implemented in the local gateway",
                webhook.auth.kind
            )))
        }
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
    use coat_domain::{EventGoalRoute, EventSourceKind, ExternalEvent, GoalTriggerTemplate};

    use super::{
        AppState, enforce_event_source_activation_policy, event_source_is_risky,
        goal_from_template, render_template, verify_hmac_sha256,
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
    fn approval_policy_blocks_risky_source_activation_without_reference() {
        let source = coat_domain::EventSource {
            id: "calendar-daily-brief".to_string(),
            kind: EventSourceKind::CalendarPoll,
            enabled: true,
            description: "calendar poller".to_string(),
            namespace: None,
            webhook: None,
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
}
