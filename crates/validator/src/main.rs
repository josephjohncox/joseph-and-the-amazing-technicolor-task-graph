use axum::{Json, Router, routing::get, routing::post};
use jattg_domain::{ValidationReport, ValidationRequest};
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "jattg_validator=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9082".to_string());
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/validate", post(validate))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "validator listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn validate(Json(request): Json<ValidationRequest>) -> Json<ValidationReport> {
    Json(ValidationReport::from_result(request))
}
