//! Human notification and feedback delivery service.
//!
//! Purpose: record local human-feedback threads and deliver generic webhook,
//! stable queue, paging, tracker, and outbox notifications for approval,
//! blocked, failed, completed, and feedback events.
//! Durable approval and feedback decisions still flow through coordinator
//! workflow handlers; this service is a delivery and visibility surface.
//!
//! Architecture references:
//! - `docs/design-docs/010-distributed-runners-mcp.md`
//! - `docs/operations/local-dev.md`

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use aws_config::{BehaviorVersion, meta::region::RegionProviderChain};
use aws_sdk_sqs::{Client as SqsClient, config::Region};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use coat_domain::{
    NotificationDeliveryReport, NotificationEvent, NotificationRequest, NotificationTarget,
    NotificationTargetKind, SecretProvider, SecretRef,
};
use serde::{Deserialize, Serialize};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::RwLock};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

type NotificationStoreRef = Arc<RwLock<NotificationStore>>;

#[derive(Clone)]
struct AppState {
    store: NotificationStoreRef,
    client: reqwest::Client,
    journal_path: Option<PathBuf>,
    max_attempts: u32,
    retry_backoff_seconds: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct NotificationStore {
    threads: BTreeMap<String, Vec<NotificationThreadEntry>>,
    outbox: BTreeMap<Uuid, NotificationOutboxEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NotificationThreadEntry {
    id: Uuid,
    thread_key: String,
    request: NotificationRequest,
    reports: Vec<NotificationDeliveryReport>,
}

#[derive(Debug, Clone, Serialize)]
struct NotificationThreadSummary {
    thread_key: String,
    count: usize,
    latest_event: Option<NotificationEvent>,
}

#[derive(Debug, Clone, Serialize)]
struct NotificationQueueEntry {
    id: Uuid,
    thread_key: String,
    event: NotificationEvent,
    goal_id: Uuid,
    task_id: Option<Uuid>,
    message: String,
    require_ack: bool,
    targets: Vec<NotificationTarget>,
    status: NotificationOutboxStatus,
    attempts: u32,
    max_attempts: u32,
    next_attempt_after_unix_seconds: Option<u64>,
    external_ref: Option<String>,
    last_error: Option<String>,
    delivered_at_unix_seconds: Option<u64>,
    acknowledged_at_unix_seconds: Option<u64>,
    acknowledged_by: Option<String>,
    ack_note: Option<String>,
    dead_lettered_at_unix_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NotificationOutboxEntry {
    id: Uuid,
    thread_key: String,
    request: NotificationRequest,
    target: NotificationTarget,
    status: NotificationOutboxStatus,
    attempts: u32,
    max_attempts: u32,
    last_attempt_at_unix_seconds: Option<u64>,
    next_attempt_after_unix_seconds: Option<u64>,
    external_ref: Option<String>,
    last_error: Option<String>,
    delivered_at_unix_seconds: Option<u64>,
    acknowledged_at_unix_seconds: Option<u64>,
    acknowledged_by: Option<String>,
    ack_note: Option<String>,
    dead_lettered_at_unix_seconds: Option<u64>,
    reports: Vec<NotificationDeliveryReport>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum NotificationOutboxStatus {
    Pending,
    Delivered,
    AwaitingAck,
    Acknowledged,
    RetryScheduled,
    DeadLettered,
}

#[derive(Debug, Deserialize)]
struct ListOutboxQuery {
    status: Option<NotificationOutboxStatus>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AckOutboxRequest {
    acknowledged_by: Option<String>,
    note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum NotifierJournalEntry {
    Thread(NotificationThreadEntry),
    Outbox(NotificationOutboxEntry),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SlackWebhookPayload {
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EmailOutboxMessage {
    to: String,
    subject: String,
    text: String,
    request: NotificationRequest,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    coat_observability::init_tracing("coat-notifier", "coat_notifier=info,tower_http=info");

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9086".to_string());
    let journal_path = std::env::var("COAT_NOTIFIER_JOURNAL_PATH")
        .or_else(|_| std::env::var("NOTIFIER_JOURNAL_PATH"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let store = replay_journal(journal_path.as_ref()).context("replay notifier outbox journal")?;
    let state = AppState {
        store: Arc::new(RwLock::new(store)),
        client: reqwest::Client::new(),
        journal_path,
        max_attempts: env_u32("COAT_NOTIFIER_MAX_ATTEMPTS", 3).max(1),
        retry_backoff_seconds: env_u64("COAT_NOTIFIER_RETRY_BACKOFF_SECONDS", 30),
    };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/notify", post(notify))
        .route("/queue", get(list_queue))
        .route("/outbox", get(list_outbox))
        .route("/outbox/retry-due", post(retry_due_outbox))
        .route("/outbox/{id}", get(get_outbox_entry))
        .route("/outbox/{id}/ack", post(ack_outbox))
        .route("/outbox/{id}/retry", post(retry_outbox))
        .route("/dlq", get(list_dlq))
        .route("/threads", get(list_threads))
        .route("/threads/{thread_key}", get(get_thread))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "notifier listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "authority": "notifier",
        "local_threads": true,
        "outbox_journal_enabled": state.journal_path.is_some(),
        "outbox_max_attempts": state.max_attempts,
        "outbox_retry_backoff_seconds": state.retry_backoff_seconds,
        "delivery_adapters": ["thread", "dashboard", "webhook", "slack_incoming_webhook", "email_outbox", "sqs", "pagerduty_events_v2", "tracker_webhook"]
    }))
}

async fn notify(
    State(state): State<AppState>,
    Json(request): Json<NotificationRequest>,
) -> Result<Json<Vec<NotificationDeliveryReport>>, NotifierError> {
    let thread_key = feedback_thread_key(&request);
    let reports = if request.policy.targets.is_empty() {
        tracing::info!(
            goal_id = %request.goal_id,
            task_id = ?request.task_id,
            event = ?request.event,
            message = %request.message,
            "notification logged without external target"
        );
        vec![NotificationDeliveryReport {
            target: None,
            delivered: true,
            external_ref: Some(format!("thread://{thread_key}/{}", Uuid::new_v4())),
            error: None,
        }]
    } else {
        let mut reports = Vec::new();
        let mut outbox_entries = Vec::new();
        for target in request.policy.targets.clone() {
            let mut entry = NotificationOutboxEntry::new(
                thread_key.clone(),
                request.clone(),
                target,
                state.max_attempts,
            );
            let report =
                attempt_outbox_delivery(&state.client, &mut entry, state.retry_backoff_seconds)
                    .await;
            reports.push(report);
            outbox_entries.push(entry);
        }
        for entry in outbox_entries {
            append_journal(&state, NotifierJournalEntry::Outbox(entry.clone())).await?;
            state.store.write().await.outbox.insert(entry.id, entry);
        }
        reports
    };

    let thread_entry = NotificationThreadEntry {
        id: Uuid::new_v4(),
        thread_key: thread_key.clone(),
        request,
        reports: reports.clone(),
    };
    append_journal(&state, NotifierJournalEntry::Thread(thread_entry.clone())).await?;
    state
        .store
        .write()
        .await
        .threads
        .entry(thread_key)
        .or_default()
        .push(thread_entry);

    Ok(Json(reports))
}

async fn list_threads(State(state): State<AppState>) -> Json<Vec<NotificationThreadSummary>> {
    Json(
        state
            .store
            .read()
            .await
            .threads
            .iter()
            .map(|(thread_key, entries)| NotificationThreadSummary {
                thread_key: thread_key.clone(),
                count: entries.len(),
                latest_event: entries.last().map(|entry| entry.request.event.clone()),
            })
            .collect(),
    )
}

async fn get_thread(
    State(state): State<AppState>,
    Path(thread_key): Path<String>,
) -> Json<Vec<NotificationThreadEntry>> {
    Json(
        state
            .store
            .read()
            .await
            .threads
            .get(&thread_key)
            .cloned()
            .unwrap_or_default(),
    )
}

async fn list_queue(State(state): State<AppState>) -> Json<Vec<NotificationQueueEntry>> {
    let store = state.store.read().await;
    let entries = store
        .outbox
        .values()
        .filter(|entry| entry.visible_in_operator_queue())
        .map(NotificationQueueEntry::from)
        .collect();
    Json(entries)
}

async fn list_outbox(
    State(state): State<AppState>,
    Query(query): Query<ListOutboxQuery>,
) -> Json<Vec<NotificationOutboxEntry>> {
    let store = state.store.read().await;
    Json(
        store
            .outbox
            .values()
            .filter(|entry| query.status.is_none_or(|status| entry.status == status))
            .cloned()
            .collect(),
    )
}

async fn list_dlq(State(state): State<AppState>) -> Json<Vec<NotificationOutboxEntry>> {
    let store = state.store.read().await;
    Json(
        store
            .outbox
            .values()
            .filter(|entry| entry.status == NotificationOutboxStatus::DeadLettered)
            .cloned()
            .collect(),
    )
}

async fn get_outbox_entry(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<NotificationOutboxEntry>, NotifierError> {
    let store = state.store.read().await;
    let entry = store
        .outbox
        .get(&id)
        .cloned()
        .ok_or_else(|| NotifierError::NotFound(format!("notification outbox entry {id}")))?;
    Ok(Json(entry))
}

async fn ack_outbox(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<AckOutboxRequest>,
) -> Result<Json<NotificationOutboxEntry>, NotifierError> {
    let mut entry = read_outbox_entry(&state, id).await?;
    acknowledge_outbox_entry(&mut entry, request, unix_seconds())?;
    append_journal(&state, NotifierJournalEntry::Outbox(entry.clone())).await?;
    state.store.write().await.outbox.insert(id, entry.clone());
    Ok(Json(entry))
}

async fn retry_outbox(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<NotificationOutboxEntry>, NotifierError> {
    let entry = retry_outbox_entry(&state, id).await?;
    Ok(Json(entry))
}

async fn retry_due_outbox(
    State(state): State<AppState>,
) -> Result<Json<Vec<NotificationOutboxEntry>>, NotifierError> {
    let now = unix_seconds();
    let due_ids: Vec<Uuid> = {
        let store = state.store.read().await;
        store
            .outbox
            .values()
            .filter(|entry| {
                entry.status == NotificationOutboxStatus::RetryScheduled
                    && entry
                        .next_attempt_after_unix_seconds
                        .is_some_and(|next_attempt| next_attempt <= now)
            })
            .map(|entry| entry.id)
            .collect()
    };
    let mut retried = Vec::new();
    for id in due_ids {
        retried.push(retry_outbox_entry(&state, id).await?);
    }
    Ok(Json(retried))
}

fn feedback_thread_key(request: &NotificationRequest) -> String {
    request
        .policy
        .feedback_thread_key
        .clone()
        .or_else(|| {
            request
                .policy
                .targets
                .iter()
                .find(|target| target.kind == NotificationTargetKind::Thread)
                .map(|target| target.address.clone())
        })
        .unwrap_or_else(|| request.goal_id.to_string())
}

impl NotificationOutboxEntry {
    fn new(
        thread_key: String,
        request: NotificationRequest,
        target: NotificationTarget,
        max_attempts: u32,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            thread_key,
            request,
            target,
            status: NotificationOutboxStatus::Pending,
            attempts: 0,
            max_attempts: max_attempts.max(1),
            last_attempt_at_unix_seconds: None,
            next_attempt_after_unix_seconds: None,
            external_ref: None,
            last_error: None,
            delivered_at_unix_seconds: None,
            acknowledged_at_unix_seconds: None,
            acknowledged_by: None,
            ack_note: None,
            dead_lettered_at_unix_seconds: None,
            reports: Vec::new(),
        }
    }

    fn visible_in_operator_queue(&self) -> bool {
        if self.status == NotificationOutboxStatus::Acknowledged {
            return false;
        }
        matches!(
            self.status,
            NotificationOutboxStatus::RetryScheduled | NotificationOutboxStatus::DeadLettered
        ) || self.target.require_ack
            || matches!(
                self.target.kind,
                NotificationTargetKind::Thread
                    | NotificationTargetKind::Dashboard
                    | NotificationTargetKind::Email
            )
    }
}

impl From<&NotificationOutboxEntry> for NotificationQueueEntry {
    fn from(entry: &NotificationOutboxEntry) -> Self {
        Self {
            id: entry.id,
            thread_key: entry.thread_key.clone(),
            event: entry.request.event.clone(),
            goal_id: entry.request.goal_id,
            task_id: entry.request.task_id,
            message: entry.request.message.clone(),
            require_ack: entry.target.require_ack,
            targets: vec![entry.target.clone()],
            status: entry.status,
            attempts: entry.attempts,
            max_attempts: entry.max_attempts,
            next_attempt_after_unix_seconds: entry.next_attempt_after_unix_seconds,
            external_ref: entry.external_ref.clone(),
            last_error: entry.last_error.clone(),
            delivered_at_unix_seconds: entry.delivered_at_unix_seconds,
            acknowledged_at_unix_seconds: entry.acknowledged_at_unix_seconds,
            acknowledged_by: entry.acknowledged_by.clone(),
            ack_note: entry.ack_note.clone(),
            dead_lettered_at_unix_seconds: entry.dead_lettered_at_unix_seconds,
        }
    }
}

async fn read_outbox_entry(
    state: &AppState,
    id: Uuid,
) -> Result<NotificationOutboxEntry, NotifierError> {
    state
        .store
        .read()
        .await
        .outbox
        .get(&id)
        .cloned()
        .ok_or_else(|| NotifierError::NotFound(format!("notification outbox entry {id}")))
}

async fn retry_outbox_entry(
    state: &AppState,
    id: Uuid,
) -> Result<NotificationOutboxEntry, NotifierError> {
    let mut entry = read_outbox_entry(state, id).await?;
    if !entry.status.retryable() {
        return Err(NotifierError::BadRequest(format!(
            "notification outbox entry {id} is {:?}, not retryable",
            entry.status
        )));
    }
    attempt_outbox_delivery(&state.client, &mut entry, state.retry_backoff_seconds).await;
    append_journal(state, NotifierJournalEntry::Outbox(entry.clone())).await?;
    state.store.write().await.outbox.insert(id, entry.clone());
    Ok(entry)
}

async fn attempt_outbox_delivery(
    client: &reqwest::Client,
    entry: &mut NotificationOutboxEntry,
    retry_backoff_seconds: u64,
) -> NotificationDeliveryReport {
    entry.attempts = entry.attempts.saturating_add(1);
    entry.last_attempt_at_unix_seconds = Some(unix_seconds());
    let report = deliver_target(
        client,
        &entry.request,
        &entry.thread_key,
        entry.target.clone(),
    )
    .await;
    apply_delivery_report(entry, report.clone(), retry_backoff_seconds, unix_seconds());
    report
}

fn apply_delivery_report(
    entry: &mut NotificationOutboxEntry,
    report: NotificationDeliveryReport,
    retry_backoff_seconds: u64,
    now: u64,
) {
    if report.delivered {
        entry.status = if entry.target.require_ack {
            NotificationOutboxStatus::AwaitingAck
        } else {
            NotificationOutboxStatus::Delivered
        };
        entry.external_ref = report.external_ref.clone();
        entry.last_error = None;
        entry.next_attempt_after_unix_seconds = None;
        entry.delivered_at_unix_seconds = Some(now);
        entry.dead_lettered_at_unix_seconds = None;
    } else {
        entry.external_ref = report.external_ref.clone();
        entry.last_error = report
            .error
            .clone()
            .or_else(|| Some("notification delivery failed".to_string()));
        entry.delivered_at_unix_seconds = None;
        if entry.attempts >= entry.max_attempts {
            entry.status = NotificationOutboxStatus::DeadLettered;
            entry.next_attempt_after_unix_seconds = None;
            entry.dead_lettered_at_unix_seconds = Some(now);
        } else {
            entry.status = NotificationOutboxStatus::RetryScheduled;
            entry.next_attempt_after_unix_seconds = Some(now.saturating_add(retry_backoff_seconds));
            entry.dead_lettered_at_unix_seconds = None;
        }
    }
    entry.reports.push(report);
}

fn acknowledge_outbox_entry(
    entry: &mut NotificationOutboxEntry,
    request: AckOutboxRequest,
    now: u64,
) -> Result<(), NotifierError> {
    if entry.status == NotificationOutboxStatus::DeadLettered {
        return Err(NotifierError::BadRequest(format!(
            "notification outbox entry {} is dead-lettered and cannot be acknowledged",
            entry.id
        )));
    }
    if matches!(
        entry.status,
        NotificationOutboxStatus::Pending | NotificationOutboxStatus::RetryScheduled
    ) {
        return Err(NotifierError::BadRequest(format!(
            "notification outbox entry {} has not been delivered and cannot be acknowledged",
            entry.id
        )));
    }
    entry.status = NotificationOutboxStatus::Acknowledged;
    entry.acknowledged_at_unix_seconds = Some(now);
    entry.acknowledged_by = request.acknowledged_by;
    entry.ack_note = request.note;
    entry.next_attempt_after_unix_seconds = None;
    entry.external_ref = Some(format!("ack://notification/{}", entry.id));
    entry.last_error = None;
    entry.reports.push(NotificationDeliveryReport {
        target: Some(entry.target.clone()),
        delivered: true,
        external_ref: entry.external_ref.clone(),
        error: None,
    });
    Ok(())
}

impl NotificationOutboxStatus {
    fn retryable(self) -> bool {
        matches!(
            self,
            NotificationOutboxStatus::Pending
                | NotificationOutboxStatus::RetryScheduled
                | NotificationOutboxStatus::DeadLettered
        )
    }
}

async fn deliver_target(
    client: &reqwest::Client,
    request: &NotificationRequest,
    thread_key: &str,
    target: NotificationTarget,
) -> NotificationDeliveryReport {
    match target.kind {
        NotificationTargetKind::Thread => NotificationDeliveryReport {
            target: Some(target),
            delivered: true,
            external_ref: Some(format!("thread://{thread_key}/{}", Uuid::new_v4())),
            error: None,
        },
        NotificationTargetKind::Dashboard => NotificationDeliveryReport {
            target: Some(target),
            delivered: true,
            external_ref: Some(format!("dashboard://queue/{thread_key}/{}", Uuid::new_v4())),
            error: None,
        },
        NotificationTargetKind::Webhook => deliver_webhook(client, request, target).await,
        NotificationTargetKind::Slack => deliver_slack(client, request, target).await,
        NotificationTargetKind::Email => deliver_email(request, target),
        NotificationTargetKind::Sqs => deliver_sqs(request, target).await,
        NotificationTargetKind::PagerDuty => deliver_pagerduty(client, request, target).await,
        NotificationTargetKind::GitHub
        | NotificationTargetKind::Linear
        | NotificationTargetKind::Jira => deliver_tracker(client, request, target).await,
        _ => NotificationDeliveryReport {
            target: Some(target),
            delivered: false,
            external_ref: None,
            error: Some("target kind requires a provider-specific notifier adapter".to_string()),
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PagerDutyEventPayload {
    routing_key: String,
    event_action: String,
    dedup_key: String,
    payload: PagerDutyPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PagerDutyPayload {
    summary: String,
    source: String,
    severity: String,
    custom_details: NotificationRequest,
}

async fn deliver_pagerduty(
    client: &reqwest::Client,
    request: &NotificationRequest,
    target: NotificationTarget,
) -> NotificationDeliveryReport {
    let Some(secret_ref) = target.secret_ref.as_ref() else {
        return NotificationDeliveryReport {
            target: Some(target),
            delivered: false,
            external_ref: None,
            error: Some("pagerduty target requires a SecretRef routing key".to_string()),
        };
    };
    let routing_key = match resolve_secret(secret_ref) {
        Ok(secret) => secret,
        Err(error) => {
            return NotificationDeliveryReport {
                target: Some(target),
                delivered: false,
                external_ref: None,
                error: Some(error),
            };
        }
    };
    let payload = PagerDutyEventPayload {
        routing_key,
        event_action: "trigger".to_string(),
        dedup_key: format!(
            "coat-{}-{}-{:?}",
            request.goal_id,
            request
                .task_id
                .map(|task_id| task_id.to_string())
                .unwrap_or_else(|| "goal".to_string()),
            request.event
        ),
        payload: PagerDutyPayload {
            summary: request.message.clone(),
            source: if target.address.trim().is_empty() {
                "coat-notifier".to_string()
            } else {
                target.address.clone()
            },
            severity: pagerduty_severity(&request.event).to_string(),
            custom_details: request.clone(),
        },
    };
    match client
        .post("https://events.pagerduty.com/v2/enqueue")
        .json(&payload)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            NotificationDeliveryReport {
                target: Some(target),
                delivered: status.is_success(),
                external_ref: Some(format!("pagerduty://events-v2/status/{status}")),
                error: if status.is_success() {
                    None
                } else {
                    Some(format!("pagerduty returned status {status}"))
                },
            }
        }
        Err(error) => NotificationDeliveryReport {
            target: Some(target),
            delivered: false,
            external_ref: None,
            error: Some(error.to_string()),
        },
    }
}

fn pagerduty_severity(event: &NotificationEvent) -> &'static str {
    match event {
        NotificationEvent::TaskFailed | NotificationEvent::RunnerLost => "error",
        NotificationEvent::TaskBlocked | NotificationEvent::BudgetWarning => "warning",
        _ => "info",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueueNotificationEnvelope {
    provider: NotificationTargetKind,
    id: Uuid,
    goal_id: Uuid,
    task_id: Option<Uuid>,
    event: NotificationEvent,
    message: String,
    require_ack: bool,
    target_address: String,
    created_at_unix_seconds: u64,
    request: NotificationRequest,
}

impl QueueNotificationEnvelope {
    fn new(request: &NotificationRequest, target: &NotificationTarget) -> Self {
        Self {
            provider: target.kind.clone(),
            id: Uuid::new_v4(),
            goal_id: request.goal_id,
            task_id: request.task_id,
            event: request.event.clone(),
            message: request.message.clone(),
            require_ack: target.require_ack,
            target_address: target.address.clone(),
            created_at_unix_seconds: unix_seconds(),
            request: request.clone(),
        }
    }
}

async fn deliver_sqs(
    request: &NotificationRequest,
    target: NotificationTarget,
) -> NotificationDeliveryReport {
    let queue_url = match sqs_queue_url(&target) {
        Ok(queue_url) => queue_url,
        Err(error) => {
            return NotificationDeliveryReport {
                target: Some(target),
                delivered: false,
                external_ref: None,
                error: Some(error),
            };
        }
    };
    let envelope = QueueNotificationEnvelope::new(request, &target);
    let body = match serde_json::to_string(&envelope) {
        Ok(body) => body,
        Err(error) => {
            return NotificationDeliveryReport {
                target: Some(target),
                delivered: false,
                external_ref: None,
                error: Some(format!("serialize SQS notification envelope: {error}")),
            };
        }
    };

    let client = sqs_client().await;
    let mut send = client
        .send_message()
        .queue_url(&queue_url)
        .message_body(body);
    if let Some(group_id) = sqs_message_group_id(&queue_url) {
        send = send
            .message_group_id(group_id)
            .message_deduplication_id(envelope.id.to_string());
    }

    match send.send().await {
        Ok(output) => {
            let message_id = output.message_id().unwrap_or("unknown");
            NotificationDeliveryReport {
                target: Some(target),
                delivered: true,
                external_ref: Some(format!("sqs://message/{message_id}")),
                error: None,
            }
        }
        Err(error) => NotificationDeliveryReport {
            target: Some(target),
            delivered: false,
            external_ref: None,
            error: Some(format!("send SQS notification: {error}")),
        },
    }
}

async fn sqs_client() -> SqsClient {
    let fallback_region = std::env::var("COAT_SQS_REGION")
        .or_else(|_| std::env::var("AWS_REGION"))
        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|_| "us-east-1".to_string());
    let region_provider =
        RegionProviderChain::default_provider().or_else(Region::new(fallback_region));
    let mut loader = aws_config::defaults(BehaviorVersion::latest()).region(region_provider);
    if let Ok(endpoint_url) = std::env::var("COAT_SQS_ENDPOINT_URL") {
        if !endpoint_url.trim().is_empty() {
            loader = loader.endpoint_url(endpoint_url);
        }
    }
    SqsClient::new(&loader.load().await)
}

fn sqs_queue_url(target: &NotificationTarget) -> Result<String, String> {
    if !target.address.trim().is_empty() {
        return Ok(target.address.clone());
    }
    if let Some(secret_ref) = target.secret_ref.as_ref() {
        return resolve_secret(secret_ref);
    }
    Err("sqs target requires queue URL in address or secret_ref".to_string())
}

fn sqs_message_group_id(queue_url: &str) -> Option<String> {
    if !queue_url.ends_with(".fifo") {
        return None;
    }
    std::env::var("COAT_SQS_MESSAGE_GROUP_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| Some("coat-notifications".to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrackerNotificationPayload {
    provider: NotificationTargetKind,
    goal_id: Uuid,
    task_id: Option<Uuid>,
    event: NotificationEvent,
    message: String,
    require_ack: bool,
}

async fn deliver_tracker(
    client: &reqwest::Client,
    request: &NotificationRequest,
    target: NotificationTarget,
) -> NotificationDeliveryReport {
    if target.address.trim().is_empty() {
        return NotificationDeliveryReport {
            target: Some(target),
            delivered: false,
            external_ref: None,
            error: Some("tracker target requires an HTTP endpoint address".to_string()),
        };
    }
    let mut builder = client
        .post(&target.address)
        .json(&TrackerNotificationPayload {
            provider: target.kind.clone(),
            goal_id: request.goal_id,
            task_id: request.task_id,
            event: request.event.clone(),
            message: request.message.clone(),
            require_ack: target.require_ack,
        });
    if let Some(secret_ref) = target.secret_ref.as_ref() {
        match resolve_secret(secret_ref) {
            Ok(secret) => {
                builder = builder.bearer_auth(secret);
            }
            Err(error) => {
                return NotificationDeliveryReport {
                    target: Some(target),
                    delivered: false,
                    external_ref: None,
                    error: Some(error),
                };
            }
        }
    }
    match builder.send().await {
        Ok(response) => {
            let status = response.status();
            NotificationDeliveryReport {
                target: Some(target),
                delivered: status.is_success(),
                external_ref: Some(format!("tracker://status/{status}")),
                error: if status.is_success() {
                    None
                } else {
                    Some(format!("tracker endpoint returned status {status}"))
                },
            }
        }
        Err(error) => NotificationDeliveryReport {
            target: Some(target),
            delivered: false,
            external_ref: None,
            error: Some(error.to_string()),
        },
    }
}

async fn deliver_slack(
    client: &reqwest::Client,
    request: &NotificationRequest,
    target: NotificationTarget,
) -> NotificationDeliveryReport {
    let webhook_url =
        if target.address.starts_with("http://") || target.address.starts_with("https://") {
            Ok(target.address.clone())
        } else if let Some(secret_ref) = target.secret_ref.as_ref() {
            resolve_secret(secret_ref)
        } else {
            Err(
                "slack target requires an incoming webhook URL or a secret ref containing one"
                    .to_string(),
            )
        };

    let webhook_url = match webhook_url {
        Ok(webhook_url) => webhook_url,
        Err(error) => {
            return NotificationDeliveryReport {
                target: Some(target),
                delivered: false,
                external_ref: None,
                error: Some(error),
            };
        }
    };

    let payload = SlackWebhookPayload {
        text: format!(
            "[COAT {:?}] goal={} task={:?}: {}",
            request.event, request.goal_id, request.task_id, request.message
        ),
    };
    match client.post(webhook_url).json(&payload).send().await {
        Ok(response) => {
            let status = response.status();
            NotificationDeliveryReport {
                target: Some(target),
                delivered: status.is_success(),
                external_ref: Some(format!("slack://incoming-webhook/status/{status}")),
                error: if status.is_success() {
                    None
                } else {
                    Some(format!("slack webhook returned status {status}"))
                },
            }
        }
        Err(error) => NotificationDeliveryReport {
            target: Some(target),
            delivered: false,
            external_ref: None,
            error: Some(error.to_string()),
        },
    }
}

fn deliver_email(
    request: &NotificationRequest,
    target: NotificationTarget,
) -> NotificationDeliveryReport {
    let message = EmailOutboxMessage {
        to: target.address.clone(),
        subject: format!("COAT {:?}: {}", request.event, request.goal_id),
        text: request.message.clone(),
        request: request.clone(),
    };
    let outbox_id = Uuid::new_v4();
    if let Ok(outbox_dir) = std::env::var("COAT_EMAIL_OUTBOX_DIR") {
        let outbox_dir = std::path::PathBuf::from(outbox_dir);
        if let Err(error) = std::fs::create_dir_all(&outbox_dir) {
            return NotificationDeliveryReport {
                target: Some(target),
                delivered: false,
                external_ref: None,
                error: Some(format!("create email outbox dir: {error}")),
            };
        }
        let path = outbox_dir.join(format!("{outbox_id}.json"));
        match serde_json::to_vec_pretty(&message)
            .map_err(anyhow::Error::from)
            .and_then(|payload| std::fs::write(&path, payload).map_err(anyhow::Error::from))
        {
            Ok(()) => NotificationDeliveryReport {
                target: Some(target),
                delivered: true,
                external_ref: Some(format!("email-outbox://{}", path.display())),
                error: None,
            },
            Err(error) => NotificationDeliveryReport {
                target: Some(target),
                delivered: false,
                external_ref: None,
                error: Some(format!("write email outbox message: {error}")),
            },
        }
    } else {
        NotificationDeliveryReport {
            target: Some(target),
            delivered: true,
            external_ref: Some(format!("email-outbox://memory/{outbox_id}")),
            error: None,
        }
    }
}

async fn deliver_webhook(
    client: &reqwest::Client,
    request: &NotificationRequest,
    target: NotificationTarget,
) -> NotificationDeliveryReport {
    let mut builder = client.post(&target.address).json(request);
    if let Some(secret_ref) = target.secret_ref.as_ref() {
        match resolve_secret(secret_ref) {
            Ok(secret) => {
                builder = builder.bearer_auth(secret);
            }
            Err(error) => {
                return NotificationDeliveryReport {
                    target: Some(target),
                    delivered: false,
                    external_ref: None,
                    error: Some(error),
                };
            }
        }
    }

    match builder.send().await {
        Ok(response) => {
            let status = response.status();
            NotificationDeliveryReport {
                target: Some(target),
                delivered: status.is_success(),
                external_ref: Some(format!("webhook://status/{status}")),
                error: if status.is_success() {
                    None
                } else {
                    Some(format!("webhook returned status {status}"))
                },
            }
        }
        Err(error) => NotificationDeliveryReport {
            target: Some(target),
            delivered: false,
            external_ref: None,
            error: Some(error.to_string()),
        },
    }
}

fn resolve_secret(secret_ref: &SecretRef) -> Result<String, String> {
    match secret_ref.provider {
        SecretProvider::Env => std::env::var(
            secret_ref
                .key
                .as_deref()
                .unwrap_or(secret_ref.name.as_str()),
        )
        .map_err(|_| "notification secret env var is not available".to_string()),
        SecretProvider::LocalFile => std::fs::read_to_string(&secret_ref.name)
            .map(|value| value.trim().to_string())
            .map_err(|error| format!("read notification secret file: {error}")),
        _ => Err(format!(
            "secret provider {:?} must be resolved by production secret middleware",
            secret_ref.provider
        )),
    }
}

fn replay_journal(path: Option<&PathBuf>) -> anyhow::Result<NotificationStore> {
    let Some(path) = path else {
        return Ok(NotificationStore::default());
    };
    if !path.exists() {
        return Ok(NotificationStore::default());
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("read notifier journal {}", path.display()))?;
    let mut store = NotificationStore::default();
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: NotifierJournalEntry = serde_json::from_str(line).with_context(|| {
            format!(
                "decode notifier journal {} line {}",
                path.display(),
                index + 1
            )
        })?;
        apply_journal_entry(&mut store, entry);
    }
    Ok(store)
}

fn apply_journal_entry(store: &mut NotificationStore, entry: NotifierJournalEntry) {
    match entry {
        NotifierJournalEntry::Thread(entry) => {
            store
                .threads
                .entry(entry.thread_key.clone())
                .or_default()
                .push(entry);
        }
        NotifierJournalEntry::Outbox(entry) => {
            store.outbox.insert(entry.id, entry);
        }
    }
}

async fn append_journal(
    state: &AppState,
    entry: NotifierJournalEntry,
) -> Result<(), NotifierError> {
    let Some(path) = &state.journal_path else {
        return Ok(());
    };
    append_journal_path(path, &entry)
        .await
        .map_err(|error| NotifierError::Internal(error.to_string()))
}

async fn append_journal_path(path: &PathBuf, entry: &NotifierJournalEntry) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    let line = serde_json::to_string(entry)?;
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await?;
    Ok(())
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug)]
enum NotifierError {
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for NotifierError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::NotFound(message) => (StatusCode::NOT_FOUND, message),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use coat_domain::{
        NotificationEvent, NotificationPolicy, NotificationRequest, NotificationTarget,
        NotificationTargetKind,
    };
    use uuid::Uuid;

    use super::{
        AckOutboxRequest, NotificationOutboxEntry, NotificationOutboxStatus,
        NotificationThreadEntry, NotifierJournalEntry, QueueNotificationEnvelope,
        acknowledge_outbox_entry, append_journal_path, apply_delivery_report, replay_journal,
        sqs_message_group_id, sqs_queue_url,
    };

    #[test]
    fn sqs_queue_url_uses_target_address() {
        let target = NotificationTarget {
            kind: NotificationTargetKind::Sqs,
            address: "https://sqs.us-east-1.amazonaws.com/123456789012/coat-notifications"
                .to_string(),
            secret_ref: None,
            require_ack: false,
        };

        assert_eq!(
            sqs_queue_url(&target).expect("queue url"),
            "https://sqs.us-east-1.amazonaws.com/123456789012/coat-notifications"
        );
    }

    #[test]
    fn sqs_fifo_queue_gets_default_message_group() {
        assert_eq!(
            sqs_message_group_id(
                "https://sqs.us-east-1.amazonaws.com/123456789012/coat-notifications.fifo",
            )
            .as_deref(),
            Some("coat-notifications")
        );
        assert!(
            sqs_message_group_id(
                "https://sqs.us-east-1.amazonaws.com/123456789012/coat-notifications",
            )
            .is_none()
        );
    }

    #[test]
    fn sqs_envelope_preserves_notification_request() {
        let request = NotificationRequest {
            goal_id: Uuid::new_v4(),
            task_id: Some(Uuid::new_v4()),
            event: NotificationEvent::TaskBlocked,
            message: "waiting for human steering".to_string(),
            policy: NotificationPolicy::default(),
        };
        let target = NotificationTarget {
            kind: NotificationTargetKind::Sqs,
            address: "https://sqs.us-east-1.amazonaws.com/123456789012/coat-notifications"
                .to_string(),
            secret_ref: None,
            require_ack: true,
        };

        let envelope = QueueNotificationEnvelope::new(&request, &target);
        assert_eq!(envelope.provider, NotificationTargetKind::Sqs);
        assert_eq!(envelope.goal_id, request.goal_id);
        assert_eq!(envelope.task_id, request.task_id);
        assert_eq!(envelope.event, request.event);
        assert_eq!(envelope.require_ack, target.require_ack);
        assert_eq!(envelope.request.message, request.message);
    }

    #[test]
    fn delivered_required_ack_entry_stays_queued_until_acknowledged() {
        let request = notification_request();
        let target = NotificationTarget {
            kind: NotificationTargetKind::Dashboard,
            address: "human-queue".to_string(),
            secret_ref: None,
            require_ack: true,
        };
        let mut entry = NotificationOutboxEntry::new("ops".to_string(), request, target.clone(), 3);
        entry.attempts = 1;

        apply_delivery_report(
            &mut entry,
            delivery_report(target, true, Some("dashboard://queue/ops/1"), None),
            30,
            100,
        );

        assert_eq!(entry.status, NotificationOutboxStatus::AwaitingAck);
        assert!(entry.visible_in_operator_queue());
        assert_eq!(entry.delivered_at_unix_seconds, Some(100));
        assert_eq!(entry.next_attempt_after_unix_seconds, None);

        acknowledge_outbox_entry(
            &mut entry,
            AckOutboxRequest {
                acknowledged_by: Some("operator".to_string()),
                note: Some("reviewed".to_string()),
            },
            120,
        )
        .expect("ack delivered entry");

        assert_eq!(entry.status, NotificationOutboxStatus::Acknowledged);
        assert!(!entry.visible_in_operator_queue());
        assert_eq!(entry.acknowledged_at_unix_seconds, Some(120));
        assert_eq!(entry.acknowledged_by.as_deref(), Some("operator"));
        assert_eq!(entry.ack_note.as_deref(), Some("reviewed"));
    }

    #[test]
    fn failed_delivery_schedules_retry_then_dead_letters() {
        let request = notification_request();
        let target = NotificationTarget {
            kind: NotificationTargetKind::Sqs,
            address: String::new(),
            secret_ref: None,
            require_ack: true,
        };
        let mut entry = NotificationOutboxEntry::new("ops".to_string(), request, target.clone(), 2);

        entry.attempts = 1;
        apply_delivery_report(
            &mut entry,
            delivery_report(
                target.clone(),
                false,
                None,
                Some("sqs target requires queue URL in address or secret_ref"),
            ),
            15,
            100,
        );

        assert_eq!(entry.status, NotificationOutboxStatus::RetryScheduled);
        assert_eq!(entry.next_attempt_after_unix_seconds, Some(115));
        assert_eq!(entry.dead_lettered_at_unix_seconds, None);
        assert!(entry.visible_in_operator_queue());

        entry.attempts = 2;
        apply_delivery_report(
            &mut entry,
            delivery_report(
                target,
                false,
                None,
                Some("sqs target requires queue URL in address or secret_ref"),
            ),
            15,
            115,
        );

        assert_eq!(entry.status, NotificationOutboxStatus::DeadLettered);
        assert_eq!(entry.next_attempt_after_unix_seconds, None);
        assert_eq!(entry.dead_lettered_at_unix_seconds, Some(115));
        assert!(entry.visible_in_operator_queue());
    }

    #[tokio::test]
    async fn notifier_journal_replays_latest_outbox_state() {
        let journal_path =
            std::env::temp_dir().join(format!("coat-notifier-{}.jsonl", Uuid::new_v4()));
        let request = notification_request();
        let target = NotificationTarget {
            kind: NotificationTargetKind::Dashboard,
            address: "human-queue".to_string(),
            secret_ref: None,
            require_ack: true,
        };
        let mut entry =
            NotificationOutboxEntry::new("ops".to_string(), request.clone(), target.clone(), 3);
        entry.attempts = 1;
        apply_delivery_report(
            &mut entry,
            delivery_report(target, true, Some("dashboard://queue/ops/1"), None),
            30,
            100,
        );
        let thread = NotificationThreadEntry {
            id: Uuid::new_v4(),
            thread_key: "ops".to_string(),
            request,
            reports: entry.reports.clone(),
        };

        append_journal_path(&journal_path, &NotifierJournalEntry::Thread(thread))
            .await
            .expect("append thread");
        append_journal_path(&journal_path, &NotifierJournalEntry::Outbox(entry.clone()))
            .await
            .expect("append delivered outbox");
        acknowledge_outbox_entry(
            &mut entry,
            AckOutboxRequest {
                acknowledged_by: Some("operator".to_string()),
                note: None,
            },
            130,
        )
        .expect("ack delivered entry");
        append_journal_path(&journal_path, &NotifierJournalEntry::Outbox(entry.clone()))
            .await
            .expect("append acked outbox");

        let store = replay_journal(Some(&journal_path)).expect("replay notifier journal");
        let replayed = store.outbox.get(&entry.id).expect("outbox entry");
        assert_eq!(replayed.status, NotificationOutboxStatus::Acknowledged);
        assert_eq!(replayed.acknowledged_by.as_deref(), Some("operator"));
        assert_eq!(store.threads.get("ops").expect("thread").len(), 1);
        let _ = std::fs::remove_file(journal_path);
    }

    fn notification_request() -> NotificationRequest {
        NotificationRequest {
            goal_id: Uuid::new_v4(),
            task_id: Some(Uuid::new_v4()),
            event: NotificationEvent::HumanFeedbackRequested,
            message: "waiting for human steering".to_string(),
            policy: NotificationPolicy::default(),
        }
    }

    fn delivery_report(
        target: NotificationTarget,
        delivered: bool,
        external_ref: Option<&str>,
        error: Option<&str>,
    ) -> coat_domain::NotificationDeliveryReport {
        coat_domain::NotificationDeliveryReport {
            target: Some(target),
            delivered,
            external_ref: external_ref.map(str::to_string),
            error: error.map(str::to_string),
        }
    }
}
