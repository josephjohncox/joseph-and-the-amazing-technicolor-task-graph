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
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use aws_config::{BehaviorVersion, meta::region::RegionProviderChain};
use aws_sdk_sqs::{Client as SqsClient, config::Region};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use coat_domain::{
    NotificationDeliveryReport, NotificationEvent, NotificationRequest, NotificationTarget,
    NotificationTargetKind, SecretProvider, SecretRef,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

type NotificationThreads = Arc<RwLock<BTreeMap<String, Vec<NotificationThreadEntry>>>>;

#[derive(Clone)]
struct AppState {
    threads: NotificationThreads,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize)]
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
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "coat_notifier=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9086".to_string());
    let state = AppState {
        threads: Arc::new(RwLock::new(BTreeMap::new())),
        client: reqwest::Client::new(),
    };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/notify", post(notify))
        .route("/queue", get(list_queue))
        .route("/threads", get(list_threads))
        .route("/threads/{thread_key}", get(get_thread))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "notifier listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "authority": "notifier",
        "local_threads": true,
        "delivery_adapters": ["thread", "dashboard", "webhook", "slack_incoming_webhook", "email_outbox", "sqs", "pagerduty_events_v2", "tracker_webhook"]
    }))
}

async fn notify(
    State(state): State<AppState>,
    Json(request): Json<NotificationRequest>,
) -> Json<Vec<NotificationDeliveryReport>> {
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
        for target in request.policy.targets.clone() {
            reports.push(deliver_target(&state.client, &request, &thread_key, target).await);
        }
        reports
    };

    state
        .threads
        .write()
        .await
        .entry(thread_key.clone())
        .or_default()
        .push(NotificationThreadEntry {
            id: Uuid::new_v4(),
            thread_key,
            request,
            reports: reports.clone(),
        });

    Json(reports)
}

async fn list_threads(State(state): State<AppState>) -> Json<Vec<NotificationThreadSummary>> {
    Json(
        state
            .threads
            .read()
            .await
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
            .threads
            .read()
            .await
            .get(&thread_key)
            .cloned()
            .unwrap_or_default(),
    )
}

async fn list_queue(State(state): State<AppState>) -> Json<Vec<NotificationQueueEntry>> {
    let threads = state.threads.read().await;
    let mut entries = Vec::new();
    for (thread_key, thread_entries) in threads.iter() {
        for entry in thread_entries {
            let targets: Vec<NotificationTarget> = entry
                .request
                .policy
                .targets
                .iter()
                .filter(|target| {
                    matches!(
                        target.kind,
                        NotificationTargetKind::Thread
                            | NotificationTargetKind::Dashboard
                            | NotificationTargetKind::Email
                    ) || target.require_ack
                })
                .cloned()
                .collect();
            if targets.is_empty() {
                continue;
            }
            entries.push(NotificationQueueEntry {
                id: entry.id,
                thread_key: thread_key.clone(),
                event: entry.request.event.clone(),
                goal_id: entry.request.goal_id,
                task_id: entry.request.task_id,
                message: entry.request.message.clone(),
                require_ack: targets.iter().any(|target| target.require_ack),
                targets,
            });
        }
    }
    Json(entries)
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

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use coat_domain::{
        NotificationEvent, NotificationPolicy, NotificationRequest, NotificationTarget,
        NotificationTargetKind,
    };
    use uuid::Uuid;

    use super::{QueueNotificationEnvelope, sqs_message_group_id, sqs_queue_url};

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
}
