use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use coat_domain::{
    NotificationDeliveryReport, NotificationEvent, NotificationRequest, NotificationTarget,
    NotificationTargetKind, SecretProvider, SecretRef,
};
use serde::Serialize;
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
        "delivery_adapters": ["thread", "webhook"]
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
        NotificationTargetKind::Webhook => deliver_webhook(client, request, target).await,
        _ => NotificationDeliveryReport {
            target: Some(target),
            delivered: false,
            external_ref: None,
            error: Some("target kind requires a provider-specific notifier adapter".to_string()),
        },
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
