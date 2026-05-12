//! Standalone validation service.
//!
//! Purpose: convert `AgentRunResult` evidence into `ValidationReport` decisions
//! using shared domain rules. The coordinator applies reports to durable state;
//! the validator does not own the task tree.
//!
//! Architecture references:
//! - `docs/exec-plans/active/060-test-review-validator.md`
//! - `docs/design-docs/090-review-doctrine-stdlib.md`

use axum::{Json, Router, routing::get, routing::post};
use coat_domain::{ValidationReport, ValidationRequest};
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    coat_observability::init_tracing("coat-validator", "coat_validator=info,tower_http=info");

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
