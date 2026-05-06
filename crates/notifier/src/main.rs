use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use coat_domain::{
    NotificationDeliveryReport, NotificationEvent, NotificationRequest, NotificationTargetKind,
};
use serde::Serialize;
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

type NotifierState = Arc<RwLock<BTreeMap<String, Vec<NotificationThreadEntry>>>>;

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
    let state = Arc::new(RwLock::new(BTreeMap::new()));
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
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

async fn notify(
    State(state): State<NotifierState>,
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
        request
            .clone()
            .policy
            .targets
            .into_iter()
            .map(|target| NotificationDeliveryReport {
                target: Some(target),
                delivered: true,
                external_ref: Some(format!("thread://{thread_key}/{}", Uuid::new_v4())),
                error: None,
            })
            .collect()
    };

    state
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

async fn list_threads(State(state): State<NotifierState>) -> Json<Vec<NotificationThreadSummary>> {
    Json(
        state
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
    State(state): State<NotifierState>,
    Path(thread_key): Path<String>,
) -> Json<Vec<NotificationThreadEntry>> {
    Json(
        state
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
