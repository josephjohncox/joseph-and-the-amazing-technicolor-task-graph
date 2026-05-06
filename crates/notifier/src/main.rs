use axum::{Json, Router, routing::get, routing::post};
use jattg_domain::{NotificationDeliveryReport, NotificationRequest};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "jattg_notifier=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9086".to_string());
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/notify", post(notify))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "notifier listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn notify(Json(request): Json<NotificationRequest>) -> Json<Vec<NotificationDeliveryReport>> {
    if request.policy.targets.is_empty() {
        tracing::info!(
            goal_id = %request.goal_id,
            task_id = ?request.task_id,
            event = ?request.event,
            message = %request.message,
            "notification logged without external target"
        );
        return Json(vec![NotificationDeliveryReport {
            target: None,
            delivered: true,
            external_ref: Some(format!("log://{}", Uuid::new_v4())),
            error: None,
        }]);
    }

    Json(
        request
            .policy
            .targets
            .into_iter()
            .map(|target| NotificationDeliveryReport {
                target: Some(target),
                delivered: true,
                external_ref: Some(format!("stub://{}", Uuid::new_v4())),
                error: None,
            })
            .collect(),
    )
}
