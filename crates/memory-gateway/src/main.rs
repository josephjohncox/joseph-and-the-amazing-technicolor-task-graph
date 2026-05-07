//! Durable memory gateway and adapter boundary.
//!
//! Purpose: provide a stable local REST/MCP-shaped interface for memory writes,
//! search, context packs, fork/join consolidation, retraction/editing, repair,
//! and event inspection.
//! Local JSONL durability is the availability boundary; Graphiti/Zep and Qdrant
//! adapters are best-effort mirrors.
//!
//! Architecture references:
//! - `docs/design-docs/030-distributed-memory-knowledgebases.md`
//! - `docs/design-docs/020-memory-research-steering.md`
//! - `docs/exec-plans/active/100-steering-research-memory.md`

use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use coat_domain::{
    GoalId, InformationUsePlan, MemoryAdapterReport, MemoryContextRequest, MemoryContextResponse,
    MemoryEditDiff, MemoryEditPreviewRecord, MemoryEditPreviewRequest, MemoryEditPreviewResponse,
    MemoryEditRequest, MemoryEditResponse, MemoryEpisode, MemoryEpisodeSource,
    MemoryEpisodeSourceType, MemoryEvent, MemoryEventAction, MemoryJoinRequest, MemoryJoinResponse,
    MemoryRepairRequest, MemoryRepairResponse, MemoryRetractRequest, MemoryRetractResponse,
    MemoryScope, MemorySearchHit, MemorySearchRequest, MemorySearchResponse, MemoryStoreKind,
    MemoryStoreRef, MemoryWriteRequest, MemoryWriteResponse,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

type MemoryState = Arc<RwLock<MemoryStore>>;

#[derive(Debug, Default, Serialize, Deserialize)]
struct MemoryStore {
    records: BTreeMap<String, MemoryRecord>,
    events: BTreeMap<GoalId, Vec<MemoryEvent>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryRecord {
    goal_id: GoalId,
    key: String,
    scope: MemoryScope,
    episode: MemoryEpisode,
    event: MemoryEvent,
    promoted: bool,
    invalidated: bool,
}

#[derive(Debug, Clone)]
struct AppConfig {
    bearer_token: Option<String>,
    journal_path: Option<PathBuf>,
    graphiti_mcp_url: Option<String>,
    graphiti_group_id: String,
    graphiti_token: Option<String>,
    qdrant_url: Option<String>,
    qdrant_collection: String,
    qdrant_token: Option<String>,
    embedding_url: Option<String>,
    embedding_model: String,
    embedding_dimensions: usize,
    embedding_token: Option<String>,
    embedding_send_dimensions: bool,
}

#[derive(Debug, Clone)]
struct AppState {
    memory: MemoryState,
    config: AppConfig,
    client: Client,
}

#[derive(Debug, Serialize)]
struct MemoryEventsResponse {
    goal_id: GoalId,
    events: Vec<MemoryEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum MemoryJournalEntry {
    Write {
        goal_id: GoalId,
        record: MemoryRecord,
    },
    Read {
        goal_id: GoalId,
        event: MemoryEvent,
    },
    Join {
        goal_id: GoalId,
        promoted: Vec<MemoryEvent>,
        invalidated: Vec<MemoryEvent>,
    },
    Retract {
        goal_id: GoalId,
        retracted: Vec<MemoryEvent>,
    },
}

impl MemoryStore {
    fn load_journal(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        let mut store = Self::default();
        for (index, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: MemoryJournalEntry = serde_json::from_str(trimmed)
                .map_err(|error| anyhow::anyhow!("parse journal line {}: {error}", index + 1))?;
            store.apply_journal_entry(entry);
        }
        Ok(store)
    }

    fn apply_journal_entry(&mut self, entry: MemoryJournalEntry) {
        match entry {
            MemoryJournalEntry::Write { goal_id, record } => {
                self.records.insert(record.key.clone(), record.clone());
                self.events
                    .entry(goal_id)
                    .or_default()
                    .push(record.event.clone());
            }
            MemoryJournalEntry::Read { goal_id, event } => {
                self.events.entry(goal_id).or_default().push(event);
            }
            MemoryJournalEntry::Join {
                goal_id,
                promoted,
                invalidated,
            } => {
                for event in &promoted {
                    if let Some(record) = self.records.get_mut(&event.key) {
                        record.promoted = true;
                        record.event = event.clone();
                    }
                }
                for event in &invalidated {
                    if let Some(record) = self.records.get_mut(&event.key) {
                        record.invalidated = true;
                    }
                }
                self.events
                    .entry(goal_id)
                    .or_default()
                    .extend(promoted.into_iter().chain(invalidated));
            }
            MemoryJournalEntry::Retract { goal_id, retracted } => {
                for event in &retracted {
                    if let Some(record) = self.records.get_mut(&event.key) {
                        record.invalidated = true;
                        record.event = event.clone();
                    }
                }
                self.events.entry(goal_id).or_default().extend(retracted);
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "coat_memory_gateway=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9087".to_string());
    let journal_path = std::env::var("MEMORY_GATEWAY_JOURNAL_PATH")
        .ok()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let memory = match journal_path.as_deref() {
        Some(path) => MemoryStore::load_journal(path)?,
        None => MemoryStore::default(),
    };
    let graphiti_mcp_url = std::env::var("MEMORY_GATEWAY_GRAPHITI_MCP_URL")
        .ok()
        .filter(|url| !url.is_empty());
    let graphiti_group_id = std::env::var("MEMORY_GATEWAY_GRAPHITI_GROUP_ID")
        .ok()
        .filter(|group| !group.is_empty())
        .unwrap_or_else(|| "jattg".to_string());
    let graphiti_token = std::env::var("MEMORY_GATEWAY_GRAPHITI_TOKEN")
        .ok()
        .filter(|token| !token.is_empty());
    let graphiti_timeout = std::env::var("MEMORY_GATEWAY_GRAPHITI_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(3);
    let qdrant_url = std::env::var("MEMORY_GATEWAY_QDRANT_URL")
        .ok()
        .filter(|url| !url.is_empty());
    let qdrant_collection = std::env::var("MEMORY_GATEWAY_QDRANT_COLLECTION")
        .ok()
        .filter(|collection| !collection.is_empty())
        .unwrap_or_else(|| "jattg_memory".to_string());
    let qdrant_token = std::env::var("MEMORY_GATEWAY_QDRANT_TOKEN")
        .ok()
        .filter(|token| !token.is_empty());
    let embedding_url = std::env::var("MEMORY_GATEWAY_EMBEDDING_URL")
        .ok()
        .filter(|url| !url.is_empty());
    let embedding_model = std::env::var("MEMORY_GATEWAY_EMBEDDING_MODEL")
        .ok()
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| "text-embedding-3-large".to_string());
    let embedding_dimensions = std::env::var("MEMORY_GATEWAY_EMBEDDING_DIMENSIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(3072);
    let embedding_token = std::env::var("MEMORY_GATEWAY_EMBEDDING_TOKEN")
        .ok()
        .filter(|token| !token.is_empty())
        .or_else(|| {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|token| !token.is_empty())
        });
    let embedding_send_dimensions = std::env::var("MEMORY_GATEWAY_EMBEDDING_SEND_DIMENSIONS")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"));
    let state = AppState {
        memory: Arc::new(RwLock::new(memory)),
        config: AppConfig {
            bearer_token: std::env::var("MEMORY_GATEWAY_TOKEN")
                .ok()
                .filter(|token| !token.is_empty()),
            journal_path,
            graphiti_mcp_url,
            graphiti_group_id,
            graphiti_token,
            qdrant_url,
            qdrant_collection,
            qdrant_token,
            embedding_url,
            embedding_model,
            embedding_dimensions,
            embedding_token,
            embedding_send_dimensions,
        },
        client: Client::builder()
            .timeout(Duration::from_secs(graphiti_timeout))
            .build()?,
    };
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/memory/write", post(write_memory))
        .route("/memory/search", post(search_memory))
        .route("/memory/context", post(context_memory))
        .route("/memory/join", post(join_memory))
        .route("/memory/retract", post(retract_memory))
        .route("/memory/edit", post(edit_memory))
        .route("/memory/edit/preview", post(preview_memory_edit))
        .route("/memory/repair", post(repair_memory))
        .route("/memory/events/{goal_id}", get(memory_events))
        .route("/mcp", post(mcp_endpoint))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "memory gateway listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn write_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)?;
    write_memory_with_adapters(&state, request)
        .await
        .map(Json)
        .map_err(server_error)
}

async fn search_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)?;
    search_memory_with_adapters(&state, request)
        .await
        .map(Json)
        .map_err(server_error)
}

async fn context_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryContextRequest>,
) -> Result<Json<MemoryContextResponse>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)?;
    context_memory_with_adapters(&state, request)
        .await
        .map(Json)
        .map_err(server_error)
}

async fn join_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryJoinRequest>,
) -> Result<Json<MemoryJoinResponse>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)?;
    join_memory_with_adapters(&state, request)
        .await
        .map(Json)
        .map_err(server_error)
}

async fn retract_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryRetractRequest>,
) -> Result<Json<MemoryRetractResponse>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)?;
    retract_memory_with_adapters(&state, request)
        .await
        .map(Json)
        .map_err(server_error)
}

async fn edit_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryEditRequest>,
) -> Result<Json<MemoryEditResponse>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)?;
    edit_memory_with_adapters(&state, request)
        .await
        .map(Json)
        .map_err(server_error)
}

async fn preview_memory_edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryEditPreviewRequest>,
) -> Result<Json<MemoryEditPreviewResponse>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)?;
    preview_memory_edit_inner(&state.memory, request)
        .await
        .map(Json)
        .map_err(server_error)
}

async fn repair_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryRepairRequest>,
) -> Result<Json<MemoryRepairResponse>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)?;
    repair_memory_with_adapters(&state, request)
        .await
        .map(Json)
        .map_err(server_error)
}

async fn memory_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(goal_id): AxumPath<GoalId>,
) -> Result<Json<MemoryEventsResponse>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)?;
    let events = state
        .memory
        .read()
        .await
        .events
        .get(&goal_id)
        .cloned()
        .unwrap_or_default();
    Ok(Json(MemoryEventsResponse { goal_id, events }))
}

async fn write_memory_with_adapters(
    state: &AppState,
    request: MemoryWriteRequest,
) -> anyhow::Result<MemoryWriteResponse> {
    let mut response = write_memory_inner(
        &state.memory,
        state.config.journal_path.as_deref(),
        request.clone(),
    )
    .await?;
    if should_forward_to_graphiti(state, request.store.as_ref()) {
        let report = graphiti_add_episode(
            state,
            "memory_write",
            &request.store,
            &request.episode.title,
            &request.episode.content,
            &graphiti_source_description(&request.scope, &request.episode),
            Some(&response.key),
        )
        .await;
        if report.success {
            response.external_ref = report.external_ref.clone();
        }
        response.adapter_reports.push(report);
    }
    if should_forward_to_qdrant(state) {
        let report = qdrant_upsert_memory(state, &request, &response.key).await;
        if report.success && response.external_ref.is_none() {
            response.external_ref = report.external_ref.clone();
        }
        response.adapter_reports.push(report);
    }
    Ok(response)
}

async fn search_memory_with_adapters(
    state: &AppState,
    request: MemorySearchRequest,
) -> anyhow::Result<MemorySearchResponse> {
    let mut response = search_memory_inner(
        &state.memory,
        state.config.journal_path.as_deref(),
        request.clone(),
    )
    .await?;
    if should_forward_to_graphiti(state, request.store.as_ref()) {
        response.adapter_reports.extend(
            graphiti_search(state, &request.store, &request.query, response.hits.len()).await,
        );
    }
    if should_forward_to_qdrant(state) {
        let limit = request.limit.unwrap_or(10).max(1);
        let (vector_hits, report) = qdrant_search(state, &request, limit).await;
        merge_hits(&mut response.hits, vector_hits, limit);
        response.adapter_reports.push(report);
    }
    Ok(response)
}

async fn context_memory_with_adapters(
    state: &AppState,
    request: MemoryContextRequest,
) -> anyhow::Result<MemoryContextResponse> {
    let scopes = if request.scopes.is_empty() {
        vec![
            MemoryScope::Goal,
            MemoryScope::Task,
            MemoryScope::Repo,
            MemoryScope::Persona,
        ]
    } else {
        request.scopes.clone()
    };
    let search = MemorySearchRequest {
        goal_id: request.goal_id,
        task_id: request.task_id,
        query: request.objective.clone(),
        scopes,
        limit: request.limit.or(Some(8)),
        store: request.store.clone(),
    };
    let search_response = search_memory_with_adapters(state, search).await?;
    let use_plan = build_context_use_plan(&request, &search_response);
    Ok(MemoryContextResponse {
        goal_id: request.goal_id,
        task_id: request.task_id,
        query: request.objective,
        hits: search_response.hits,
        use_plan,
        adapter_reports: search_response.adapter_reports,
    })
}

async fn join_memory_with_adapters(
    state: &AppState,
    request: MemoryJoinRequest,
) -> anyhow::Result<MemoryJoinResponse> {
    let mut response = join_memory_inner(
        &state.memory,
        state.config.journal_path.as_deref(),
        request.clone(),
    )
    .await?;
    if should_forward_to_graphiti(state, request.store.as_ref()) {
        let body = serde_json::json!({
            "goal_id": request.goal_id,
            "parent_task_id": request.parent_task_id,
            "branch_task_ids": &request.branch_task_ids,
            "unifier_task_id": request.unifier_task_id,
            "promote_keys": &request.promote_keys,
            "invalidate_keys": &request.invalidate_keys,
            "decision": &request.decision,
            "reason": &request.reason,
        });
        response.adapter_reports.push(
            graphiti_add_episode(
                state,
                "memory_join",
                &request.store,
                "COAT fork/join memory decision",
                &serde_json::to_string_pretty(&body)?,
                "coat memory join",
                None,
            )
            .await,
        );
    }
    if should_forward_to_qdrant(state) {
        let body = serde_json::json!({
            "goal_id": request.goal_id,
            "parent_task_id": request.parent_task_id,
            "branch_task_ids": &request.branch_task_ids,
            "unifier_task_id": request.unifier_task_id,
            "promote_keys": &request.promote_keys,
            "invalidate_keys": &request.invalidate_keys,
            "decision": &request.decision,
            "reason": &request.reason,
        });
        let join_key = format!(
            "{}:join:{}",
            request.goal_id,
            request
                .unifier_task_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "unassigned".to_string())
        );
        response.adapter_reports.push(
            qdrant_upsert_text(
                state,
                "memory_join",
                &join_key,
                "COAT fork/join memory decision",
                &serde_json::to_string_pretty(&body)?,
                serde_json::json!({
                    "goal_id": request.goal_id.to_string(),
                    "parent_task_id": request.parent_task_id.map(|id| id.to_string()),
                    "task_id": request.unifier_task_id.map(|id| id.to_string()),
                    "key": "fork_join",
                    "scope": "goal",
                    "title": "COAT fork/join memory decision",
                    "summary": request.reason,
                    "content": body,
                    "tags": ["fork_join", "unification"],
                }),
            )
            .await,
        );
    }
    Ok(response)
}

async fn retract_memory_with_adapters(
    state: &AppState,
    request: MemoryRetractRequest,
) -> anyhow::Result<MemoryRetractResponse> {
    let mut response = retract_memory_inner(
        &state.memory,
        state.config.journal_path.as_deref(),
        request.clone(),
    )
    .await?;
    if should_forward_to_graphiti(state, request.store.as_ref()) {
        let body = serde_json::json!({
            "goal_id": request.goal_id,
            "task_id": request.task_id,
            "keys": &request.keys,
            "reason": &request.reason,
            "missing_keys": &response.missing_keys,
        });
        response.adapter_reports.push(
            graphiti_add_episode(
                state,
                "memory_retract",
                &request.store,
                "COAT memory retraction",
                &serde_json::to_string_pretty(&body)?,
                "coat memory retract",
                None,
            )
            .await,
        );
    }
    if should_forward_to_qdrant(state) {
        let body = serde_json::json!({
            "goal_id": request.goal_id,
            "task_id": request.task_id,
            "keys": &request.keys,
            "reason": &request.reason,
            "missing_keys": &response.missing_keys,
        });
        let retract_key = format!(
            "{}:retract:{}",
            request.goal_id,
            request
                .task_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string())
        );
        response.adapter_reports.push(
            qdrant_upsert_text(
                state,
                "memory_retract",
                &retract_key,
                "COAT memory retraction",
                &serde_json::to_string_pretty(&body)?,
                serde_json::json!({
                    "goal_id": request.goal_id.to_string(),
                    "task_id": request.task_id.map(|id| id.to_string()),
                    "key": "memory_retract",
                    "scope": "goal",
                    "title": "COAT memory retraction",
                    "summary": request.reason,
                    "content": body,
                    "tags": ["memory", "retraction"],
                }),
            )
            .await,
        );
    }
    Ok(response)
}

async fn edit_memory_with_adapters(
    state: &AppState,
    request: MemoryEditRequest,
) -> anyhow::Result<MemoryEditResponse> {
    let retract = retract_memory_with_adapters(
        state,
        MemoryRetractRequest {
            goal_id: request.goal_id,
            task_id: request.task_id,
            keys: request.replace_keys.clone(),
            reason: format!(
                "{} Replacement key: {:?}",
                request.reason, request.replacement_key
            ),
            store: request.store.clone(),
        },
    )
    .await?;
    let write = write_memory_with_adapters(
        state,
        MemoryWriteRequest {
            goal_id: request.goal_id,
            task_id: request.task_id,
            scope: request.scope,
            key: request.replacement_key,
            episode: request.replacement_episode,
            store: request.store,
        },
    )
    .await?;
    let mut adapter_reports = retract.adapter_reports.clone();
    adapter_reports.extend(write.adapter_reports.clone());
    Ok(MemoryEditResponse {
        retracted: retract.retracted,
        missing_keys: retract.missing_keys,
        written: write,
        adapter_reports,
    })
}

async fn preview_memory_edit_inner(
    memory: &MemoryState,
    request: MemoryEditPreviewRequest,
) -> anyhow::Result<MemoryEditPreviewResponse> {
    let memory = memory.read().await;
    let mut existing = Vec::new();
    let mut missing_keys = Vec::new();
    let mut diffs = Vec::new();

    for key in &request.replace_keys {
        if let Some(record) = memory.records.get(key) {
            if record.goal_id != request.goal_id {
                missing_keys.push(key.clone());
                continue;
            }
            existing.push(MemoryEditPreviewRecord {
                key: key.clone(),
                scope: record.scope.clone(),
                title: record.episode.title.clone(),
                content: record.episode.content.clone(),
                source: record.episode.source.clone(),
                tags: record.episode.tags.clone(),
                promoted: record.promoted,
                invalidated: record.invalidated,
            });
            diffs.push(MemoryEditDiff {
                key: key.clone(),
                before_title: record.episode.title.clone(),
                before_excerpt: summarize(&record.episode.content),
                after_title: request.replacement_episode.title.clone(),
                after_excerpt: summarize(&request.replacement_episode.content),
                guidance: "Review the source, tags, and replacement content before promoting this edit to shared memory.".to_string(),
            });
        } else {
            missing_keys.push(key.clone());
        }
    }

    Ok(MemoryEditPreviewResponse {
        goal_id: request.goal_id,
        existing,
        missing_keys: missing_keys.clone(),
        replacement_key: request.replacement_key,
        replacement_title: request.replacement_episode.title,
        replacement_content: request.replacement_episode.content,
        replacement_tags: request.replacement_episode.tags,
        reason: request.reason,
        ready_to_edit: !request.replace_keys.is_empty() && missing_keys.is_empty(),
        diffs,
    })
}

async fn repair_memory_with_adapters(
    state: &AppState,
    request: MemoryRepairRequest,
) -> anyhow::Result<MemoryRepairResponse> {
    let records = state
        .memory
        .read()
        .await
        .records
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let scanned = records.len();
    let selected_records = records
        .into_iter()
        .filter(|record| {
            request
                .goal_id
                .is_none_or(|goal_id| record.goal_id == goal_id)
        })
        .filter(|record| request.keys.is_empty() || request.keys.contains(&record.key))
        .filter(|record| request.include_invalidated || !record.invalidated)
        .collect::<Vec<_>>();
    let selected = selected_records.len();
    let store_kinds = repair_store_kinds(state, &request.store_kinds);
    let mut response = MemoryRepairResponse {
        scanned,
        selected,
        repaired: 0,
        skipped: 0,
        adapter_reports: Vec::new(),
    };

    if request.dry_run {
        response.skipped = selected.saturating_mul(store_kinds.len());
        response.adapter_reports.push(MemoryAdapterReport {
            store_kind: store_kinds
                .first()
                .cloned()
                .unwrap_or(MemoryStoreKind::Other),
            operation: "memory_repair_dry_run".to_string(),
            attempted: false,
            success: true,
            external_ref: None,
            error: Some(format!(
                "dry run selected {selected} records for {} adapter operations",
                response.skipped
            )),
        });
        return Ok(response);
    }

    for record in selected_records {
        for store_kind in &store_kinds {
            let report = match store_kind {
                MemoryStoreKind::ZepGraphiti => repair_graphiti_record(state, &record).await,
                MemoryStoreKind::Qdrant => repair_qdrant_record(state, &record).await,
                other => unsupported_repair_report(other.clone()),
            };
            if report.success {
                response.repaired += 1;
            } else {
                response.skipped += 1;
            }
            response.adapter_reports.push(report);
        }
    }

    Ok(response)
}

async fn write_memory_inner(
    memory: &MemoryState,
    journal_path: Option<&Path>,
    request: MemoryWriteRequest,
) -> anyhow::Result<MemoryWriteResponse> {
    let key = request
        .key
        .clone()
        .unwrap_or_else(|| format!("{}:{}", request.goal_id, Uuid::new_v4()));
    let summary = summarize(&request.episode.content);
    let event = MemoryEvent {
        task_id: request.task_id,
        scope: request.scope.clone(),
        action: MemoryEventAction::Write,
        store_kind: request
            .store
            .as_ref()
            .map(|store| store.kind.clone())
            .unwrap_or(MemoryStoreKind::ZepGraphiti),
        key: key.clone(),
        summary,
    };
    let record = MemoryRecord {
        goal_id: request.goal_id,
        key: key.clone(),
        scope: request.scope,
        episode: request.episode,
        event: event.clone(),
        promoted: false,
        invalidated: false,
    };

    let mut memory = memory.write().await;
    memory.records.insert(key.clone(), record);
    memory
        .events
        .entry(request.goal_id)
        .or_default()
        .push(event.clone());
    if let Some(journal_path) = journal_path {
        append_journal(
            journal_path,
            &MemoryJournalEntry::Write {
                goal_id: request.goal_id,
                record: memory
                    .records
                    .get(&key)
                    .expect("record inserted before journal")
                    .clone(),
            },
        )?;
    }

    Ok(MemoryWriteResponse {
        event,
        key,
        stored_locally: true,
        external_ref: None,
        adapter_reports: Vec::new(),
    })
}

async fn search_memory_inner(
    memory: &MemoryState,
    journal_path: Option<&Path>,
    request: MemorySearchRequest,
) -> anyhow::Result<MemorySearchResponse> {
    let query_terms = tokenize(&request.query);
    let limit = request.limit.unwrap_or(10).max(1);
    let mut hits: Vec<MemorySearchHit> = memory
        .read()
        .await
        .records
        .values()
        .filter(|record| record.goal_id == request.goal_id)
        .filter(|record| !record.invalidated)
        .filter(|record| request.scopes.is_empty() || request.scopes.contains(&record.scope))
        .filter_map(|record| {
            let haystack = format!(
                "{} {} {}",
                record.episode.title, record.episode.content, record.event.summary
            );
            let score = lexical_score(&query_terms, &haystack);
            (score > 0.0).then(|| MemorySearchHit {
                key: record.key.clone(),
                scope: record.scope.clone(),
                score,
                summary: record.event.summary.clone(),
                source: record.episode.source.clone(),
                tags: record.episode.tags.clone(),
            })
        })
        .collect();

    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.key.cmp(&right.key))
    });
    hits.truncate(limit);

    let read_event = MemoryEvent {
        task_id: request.task_id,
        scope: request.scopes.first().cloned().unwrap_or(MemoryScope::Goal),
        action: MemoryEventAction::Read,
        store_kind: request
            .store
            .as_ref()
            .map(|store| store.kind.clone())
            .unwrap_or(MemoryStoreKind::ZepGraphiti),
        key: format!("search:{}", Uuid::new_v4()),
        summary: format!(
            "memory search '{}' returned {} hits",
            request.query,
            hits.len()
        ),
    };

    memory
        .write()
        .await
        .events
        .entry(request.goal_id)
        .or_default()
        .push(read_event.clone());
    if let Some(journal_path) = journal_path {
        append_journal(
            journal_path,
            &MemoryJournalEntry::Read {
                goal_id: request.goal_id,
                event: read_event.clone(),
            },
        )?;
    }

    Ok(MemorySearchResponse {
        hits,
        events: vec![read_event],
        adapter_reports: Vec::new(),
    })
}

async fn join_memory_inner(
    memory: &MemoryState,
    journal_path: Option<&Path>,
    request: MemoryJoinRequest,
) -> anyhow::Result<MemoryJoinResponse> {
    let mut memory = memory.write().await;
    let mut promoted = Vec::new();
    let mut invalidated = Vec::new();

    for key in &request.promote_keys {
        if let Some(record) = memory.records.get_mut(key) {
            record.promoted = true;
            let event = MemoryEvent {
                task_id: request.unifier_task_id,
                scope: record.scope.clone(),
                action: MemoryEventAction::Join,
                store_kind: request
                    .store
                    .as_ref()
                    .map(|store| store.kind.clone())
                    .unwrap_or(MemoryStoreKind::ZepGraphiti),
                key: key.clone(),
                summary: format!("promoted memory after fork/join: {}", request.reason),
            };
            record.event = event.clone();
            promoted.push(event);
        }
    }

    for key in &request.invalidate_keys {
        if let Some(record) = memory.records.get_mut(key) {
            record.invalidated = true;
            let event = MemoryEvent {
                task_id: request.unifier_task_id,
                scope: record.scope.clone(),
                action: MemoryEventAction::Invalidate,
                store_kind: request
                    .store
                    .as_ref()
                    .map(|store| store.kind.clone())
                    .unwrap_or(MemoryStoreKind::ZepGraphiti),
                key: key.clone(),
                summary: format!("invalidated memory after fork/join: {}", request.reason),
            };
            invalidated.push(event);
        }
    }

    memory
        .events
        .entry(request.goal_id)
        .or_default()
        .extend(promoted.iter().chain(invalidated.iter()).cloned());
    if let Some(journal_path) = journal_path {
        append_journal(
            journal_path,
            &MemoryJournalEntry::Join {
                goal_id: request.goal_id,
                promoted: promoted.clone(),
                invalidated: invalidated.clone(),
            },
        )?;
    }

    Ok(MemoryJoinResponse {
        promoted,
        invalidated,
        adapter_reports: Vec::new(),
    })
}

async fn retract_memory_inner(
    memory: &MemoryState,
    journal_path: Option<&Path>,
    request: MemoryRetractRequest,
) -> anyhow::Result<MemoryRetractResponse> {
    let mut memory = memory.write().await;
    let mut retracted = Vec::new();
    let mut missing_keys = Vec::new();

    for key in &request.keys {
        if let Some(record) = memory.records.get_mut(key) {
            if record.goal_id != request.goal_id {
                missing_keys.push(key.clone());
                continue;
            }
            record.invalidated = true;
            let event = MemoryEvent {
                task_id: request.task_id,
                scope: record.scope.clone(),
                action: MemoryEventAction::Retract,
                store_kind: request
                    .store
                    .as_ref()
                    .map(|store| store.kind.clone())
                    .unwrap_or(MemoryStoreKind::ZepGraphiti),
                key: key.clone(),
                summary: format!("retracted memory: {}", request.reason),
            };
            record.event = event.clone();
            retracted.push(event);
        } else {
            missing_keys.push(key.clone());
        }
    }

    memory
        .events
        .entry(request.goal_id)
        .or_default()
        .extend(retracted.iter().cloned());
    if let Some(journal_path) = journal_path {
        append_journal(
            journal_path,
            &MemoryJournalEntry::Retract {
                goal_id: request.goal_id,
                retracted: retracted.clone(),
            },
        )?;
    }

    Ok(MemoryRetractResponse {
        retracted,
        missing_keys,
        adapter_reports: Vec::new(),
    })
}

async fn mcp_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers)?;
    let id = request
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let method = request
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "tools/list" => serde_json::json!({
            "tools": [
                {"name": "memory_write", "description": "Write a reviewed memory episode"},
                {"name": "memory_search", "description": "Search local gateway memory records"},
                {"name": "memory_context", "description": "Build a bounded task context pack from memory retrieval"},
                {"name": "memory_join", "description": "Promote or invalidate branch memories"},
                {"name": "memory_retract", "description": "Retract selected memory records after operator or unifier review"},
                {"name": "memory_edit", "description": "Retract old keys and write a linked replacement memory record"},
                {"name": "memory_edit_preview", "description": "Preview old memory keys versus a replacement before committing an edit"},
                {"name": "memory_repair", "description": "Replay local memory records into configured external adapters"},
                {"name": "memory_events", "description": "List memory events for a goal"}
            ]
        }),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_default();
            let name = params
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let arguments = params.get("arguments").cloned().unwrap_or_default();
            match name {
                "memory_write" => {
                    let parsed = serde_json::from_value::<MemoryWriteRequest>(arguments)
                        .map_err(bad_request)?;
                    serde_json::to_value(
                        write_memory_with_adapters(&state, parsed)
                            .await
                            .map_err(server_error)?,
                    )
                    .map_err(bad_request)?
                }
                "memory_search" => {
                    let parsed = serde_json::from_value::<MemorySearchRequest>(arguments)
                        .map_err(bad_request)?;
                    serde_json::to_value(
                        search_memory_with_adapters(&state, parsed)
                            .await
                            .map_err(server_error)?,
                    )
                    .map_err(bad_request)?
                }
                "memory_context" => {
                    let parsed = serde_json::from_value::<MemoryContextRequest>(arguments)
                        .map_err(bad_request)?;
                    serde_json::to_value(
                        context_memory_with_adapters(&state, parsed)
                            .await
                            .map_err(server_error)?,
                    )
                    .map_err(bad_request)?
                }
                "memory_join" => {
                    let parsed = serde_json::from_value::<MemoryJoinRequest>(arguments)
                        .map_err(bad_request)?;
                    serde_json::to_value(
                        join_memory_with_adapters(&state, parsed)
                            .await
                            .map_err(server_error)?,
                    )
                    .map_err(bad_request)?
                }
                "memory_retract" => {
                    let parsed = serde_json::from_value::<MemoryRetractRequest>(arguments)
                        .map_err(bad_request)?;
                    serde_json::to_value(
                        retract_memory_with_adapters(&state, parsed)
                            .await
                            .map_err(server_error)?,
                    )
                    .map_err(bad_request)?
                }
                "memory_edit" => {
                    let parsed = serde_json::from_value::<MemoryEditRequest>(arguments)
                        .map_err(bad_request)?;
                    serde_json::to_value(
                        edit_memory_with_adapters(&state, parsed)
                            .await
                            .map_err(server_error)?,
                    )
                    .map_err(bad_request)?
                }
                "memory_edit_preview" => {
                    let parsed = serde_json::from_value::<MemoryEditPreviewRequest>(arguments)
                        .map_err(bad_request)?;
                    serde_json::to_value(
                        preview_memory_edit_inner(&state.memory, parsed)
                            .await
                            .map_err(server_error)?,
                    )
                    .map_err(bad_request)?
                }
                "memory_repair" => {
                    let parsed = serde_json::from_value::<MemoryRepairRequest>(arguments)
                        .map_err(bad_request)?;
                    serde_json::to_value(
                        repair_memory_with_adapters(&state, parsed)
                            .await
                            .map_err(server_error)?,
                    )
                    .map_err(bad_request)?
                }
                "memory_events" => {
                    let goal_id = arguments
                        .get("goal_id")
                        .cloned()
                        .ok_or_else(|| bad_request("goal_id is required"))?;
                    let goal_id = serde_json::from_value::<GoalId>(goal_id).map_err(bad_request)?;
                    let events = state
                        .memory
                        .read()
                        .await
                        .events
                        .get(&goal_id)
                        .cloned()
                        .unwrap_or_default();
                    serde_json::json!({ "goal_id": goal_id, "events": events })
                }
                _ => return Err(bad_request(format!("unknown tool {name}"))),
            }
        }
        _ => return Err(bad_request(format!("unknown MCP method {method}"))),
    };

    Ok(Json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })))
}

fn should_forward_to_graphiti(state: &AppState, store: Option<&MemoryStoreRef>) -> bool {
    state.config.graphiti_mcp_url.is_some()
        && store.is_none_or(|store| store.kind == MemoryStoreKind::ZepGraphiti)
}

fn should_forward_to_qdrant(state: &AppState) -> bool {
    state.config.qdrant_url.is_some()
}

fn build_context_use_plan(
    request: &MemoryContextRequest,
    search_response: &MemorySearchResponse,
) -> InformationUsePlan {
    let facts_to_use = search_response
        .hits
        .iter()
        .map(|hit| {
            format!(
                "{} [{} score={:.3}]: {}",
                hit.key,
                json_label(&hit.scope),
                hit.score,
                hit.summary
            )
        })
        .collect::<Vec<_>>();
    let mut facts_to_avoid = vec![
        "Do not treat retrieved memories as more authoritative than current source code, live service responses, or explicit human instructions.".to_string(),
        "Do not expose or copy secrets from memory, MCP context, environment variables, or adapter errors into prompts or artifacts.".to_string(),
    ];
    if search_response
        .adapter_reports
        .iter()
        .any(|report| report.attempted && !report.success)
    {
        facts_to_avoid.push(
            "External memory adapters returned errors; treat this context pack as incomplete and ask for research or repair when missing context matters.".to_string(),
        );
    }
    if search_response.hits.is_empty() {
        facts_to_avoid.push(format!(
            "No memory hits matched '{}'; do not infer prior decisions from an empty result.",
            request.objective
        ));
    }
    InformationUsePlan {
        facts_to_use,
        facts_to_avoid,
        proposed_task_updates: Vec::new(),
        validation_checks: vec![
            "Check retrieved memory against the current task objective and done criteria before acting.".to_string(),
            "Preserve memory keys, scopes, and provenance when citing retrieved context in worker output.".to_string(),
            "Write new durable facts only after evidence, review, or unifier approval according to MemoryPolicy.write_policy.".to_string(),
        ],
        ..InformationUsePlan::default()
    }
}

fn repair_store_kinds(state: &AppState, requested: &[MemoryStoreKind]) -> Vec<MemoryStoreKind> {
    if !requested.is_empty() {
        return requested.to_vec();
    }
    let mut kinds = Vec::new();
    if state.config.graphiti_mcp_url.is_some() {
        kinds.push(MemoryStoreKind::ZepGraphiti);
    }
    if state.config.qdrant_url.is_some() {
        kinds.push(MemoryStoreKind::Qdrant);
    }
    kinds
}

async fn repair_graphiti_record(state: &AppState, record: &MemoryRecord) -> MemoryAdapterReport {
    if state.config.graphiti_mcp_url.is_none() {
        return MemoryAdapterReport {
            store_kind: MemoryStoreKind::ZepGraphiti,
            operation: "memory_repair_graphiti".to_string(),
            attempted: false,
            success: false,
            external_ref: None,
            error: Some("Graphiti MCP adapter is not configured".to_string()),
        };
    }
    graphiti_add_episode(
        state,
        "memory_repair_graphiti",
        &None,
        &record.episode.title,
        &record.episode.content,
        &graphiti_source_description(&record.scope, &record.episode),
        Some(&record.key),
    )
    .await
}

async fn repair_qdrant_record(state: &AppState, record: &MemoryRecord) -> MemoryAdapterReport {
    if state.config.qdrant_url.is_none() {
        return qdrant_config_report(
            "memory_repair_vector",
            false,
            "Qdrant URL is not configured",
        );
    }
    qdrant_upsert_memory_record(state, record, "memory_repair_vector").await
}

fn unsupported_repair_report(store_kind: MemoryStoreKind) -> MemoryAdapterReport {
    MemoryAdapterReport {
        store_kind,
        operation: "memory_repair".to_string(),
        attempted: false,
        success: false,
        external_ref: None,
        error: Some("memory repair adapter is not implemented for this store kind".to_string()),
    }
}

async fn graphiti_add_episode(
    state: &AppState,
    operation: &str,
    store: &Option<MemoryStoreRef>,
    title: &str,
    body: &str,
    source_description: &str,
    key: Option<&str>,
) -> MemoryAdapterReport {
    let group_id = graphiti_group_id(&state.config, store.as_ref());
    let mut arguments = serde_json::json!({
        "name": title,
        "episode_body": body,
        "source": "text",
        "source_description": source_description,
        "group_id": group_id,
    });
    if let Some(key) = key {
        arguments["uuid"] = serde_json::Value::String(stable_graphiti_uuid(key));
    }

    let external_ref = key
        .map(|key| format!("graphiti-mcp://{group_id}/episode/{key}"))
        .unwrap_or_else(|| format!("graphiti-mcp://{group_id}/episode/join"));
    adapter_report_from_result(
        operation,
        Some(external_ref),
        call_graphiti_tool(state, "add_episode", arguments).await,
    )
}

async fn graphiti_search(
    state: &AppState,
    store: &Option<MemoryStoreRef>,
    query: &str,
    local_hit_count: usize,
) -> Vec<MemoryAdapterReport> {
    let group_id = graphiti_group_id(&state.config, store.as_ref());
    let limit = local_hit_count.max(10);
    let node_report = adapter_report_from_result(
        "memory_search_nodes",
        None,
        call_graphiti_tool(
            state,
            "search_nodes",
            serde_json::json!({
                "query": query,
                "group_ids": [group_id.clone()],
                "max_nodes": limit,
            }),
        )
        .await,
    );
    let fact_report = adapter_report_from_result(
        "memory_search_facts",
        None,
        call_graphiti_tool(
            state,
            "search_facts",
            serde_json::json!({
                "query": query,
                "group_ids": [group_id],
                "max_facts": limit,
            }),
        )
        .await,
    );
    vec![node_report, fact_report]
}

async fn qdrant_upsert_memory(
    state: &AppState,
    request: &MemoryWriteRequest,
    key: &str,
) -> MemoryAdapterReport {
    let record = MemoryRecord {
        goal_id: request.goal_id,
        key: key.to_string(),
        scope: request.scope.clone(),
        episode: request.episode.clone(),
        event: MemoryEvent {
            task_id: request.task_id,
            scope: request.scope.clone(),
            action: MemoryEventAction::Write,
            store_kind: MemoryStoreKind::Qdrant,
            key: key.to_string(),
            summary: summarize(&request.episode.content),
        },
        promoted: false,
        invalidated: false,
    };
    qdrant_upsert_memory_record(state, &record, "memory_write_vector").await
}

async fn qdrant_upsert_memory_record(
    state: &AppState,
    record: &MemoryRecord,
    operation: &str,
) -> MemoryAdapterReport {
    let body = format!(
        "{}\n\n{}\n\nTags: {}",
        record.episode.title,
        record.episode.content,
        record.episode.tags.join(", ")
    );
    let payload = serde_json::json!({
        "goal_id": record.goal_id.to_string(),
        "task_id": record.event.task_id.map(|id| id.to_string()),
        "key": record.key,
        "scope": json_label(&record.scope),
        "title": record.episode.title,
        "summary": summarize(&record.episode.content),
        "content": record.episode.content,
        "source": record.episode.source,
        "tags": record.episode.tags,
        "promoted": record.promoted,
        "invalidated": record.invalidated,
    });
    qdrant_upsert_text(
        state,
        operation,
        &record.key,
        &record.episode.title,
        &body,
        payload,
    )
    .await
}

async fn qdrant_upsert_text(
    state: &AppState,
    operation: &str,
    key: &str,
    title: &str,
    body: &str,
    payload: serde_json::Value,
) -> MemoryAdapterReport {
    let Some(qdrant_url) = state.config.qdrant_url.as_ref() else {
        return qdrant_config_report(operation, false, "Qdrant URL is not configured");
    };
    let external_ref = Some(format!(
        "qdrant://{}/points/{}",
        state.config.qdrant_collection, key
    ));
    let result = async {
        let embedding = embed_text(state, body).await?;
        ensure_qdrant_collection(state, qdrant_url, embedding.len()).await?;
        let url = qdrant_url_for(
            qdrant_url,
            &format!(
                "/collections/{}/points?wait=true",
                state.config.qdrant_collection
            ),
        );
        let point = serde_json::json!({
            "id": stable_qdrant_uuid(key),
            "vector": embedding,
            "payload": payload,
        });
        let response = qdrant_request(state, state.client.put(url))
            .json(&serde_json::json!({ "points": [point] }))
            .send()
            .await
            .map_err(|error| format!("upsert {title}: {error}"))?;
        qdrant_expect_success("upsert point", response).await
    }
    .await;
    qdrant_report_from_result(operation, external_ref, result)
}

async fn qdrant_search(
    state: &AppState,
    request: &MemorySearchRequest,
    limit: usize,
) -> (Vec<MemorySearchHit>, MemoryAdapterReport) {
    let Some(qdrant_url) = state.config.qdrant_url.as_ref() else {
        return (
            Vec::new(),
            qdrant_config_report(
                "memory_search_vector",
                false,
                "Qdrant URL is not configured",
            ),
        );
    };
    let result = async {
        let embedding = embed_text(state, &request.query).await?;
        ensure_qdrant_collection(state, qdrant_url, embedding.len()).await?;
        let filter = qdrant_filter(request);
        let query_body = serde_json::json!({
            "query": embedding,
            "limit": limit,
            "with_payload": true,
            "filter": filter,
        });
        let query_url = qdrant_url_for(
            qdrant_url,
            &format!(
                "/collections/{}/points/query",
                state.config.qdrant_collection
            ),
        );
        let response = qdrant_request(state, state.client.post(query_url))
            .json(&query_body)
            .send()
            .await
            .map_err(|error| format!("query points: {error}"))?;
        if response.status().is_success() {
            return parse_qdrant_hits(response.json().await.map_err(|error| {
                format!("query points: parse JSON response: {error}")
            })?);
        }

        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable response body>".to_string());
        let search_url = qdrant_url_for(
            qdrant_url,
            &format!(
                "/collections/{}/points/search",
                state.config.qdrant_collection
            ),
        );
        let legacy_body = serde_json::json!({
            "vector": query_body["query"].clone(),
            "limit": limit,
            "with_payload": true,
            "filter": filter,
        });
        let fallback = qdrant_request(state, state.client.post(search_url))
            .json(&legacy_body)
            .send()
            .await
            .map_err(|error| format!("query points failed with HTTP {status}: {body}; fallback search failed: {error}"))?;
        if !fallback.status().is_success() {
            return Err(qdrant_error(
                "legacy search points",
                fallback,
                Some(format!("query points failed with HTTP {status}: {body}")),
            )
            .await);
        }
        parse_qdrant_hits(fallback.json().await.map_err(|error| {
            format!("legacy search points: parse JSON response: {error}")
        })?)
    }
    .await;

    match result {
        Ok(hits) => (
            hits,
            MemoryAdapterReport {
                store_kind: MemoryStoreKind::Qdrant,
                operation: "memory_search_vector".to_string(),
                attempted: true,
                success: true,
                external_ref: Some(format!(
                    "qdrant://{}/search",
                    state.config.qdrant_collection
                )),
                error: None,
            },
        ),
        Err(error) => (
            Vec::new(),
            MemoryAdapterReport {
                store_kind: MemoryStoreKind::Qdrant,
                operation: "memory_search_vector".to_string(),
                attempted: true,
                success: false,
                external_ref: None,
                error: Some(error),
            },
        ),
    }
}

async fn embed_text(state: &AppState, input: &str) -> Result<Vec<f32>, String> {
    let Some(url) = state.config.embedding_url.as_ref() else {
        return Err("embedding endpoint is not configured".to_string());
    };
    let mut payload = serde_json::json!({
        "model": state.config.embedding_model,
        "input": input,
    });
    if state.config.embedding_send_dimensions {
        payload["dimensions"] = serde_json::json!(state.config.embedding_dimensions);
    }
    let mut request = state.client.post(url).json(&payload);
    if let Some(token) = &state.config.embedding_token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("embedding request: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable response body>".to_string());
        return Err(format!("embedding request: HTTP {status}: {body}"));
    }
    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("embedding request: parse JSON response: {error}"))?;
    let embedding_value = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .and_then(|data| data.first())
        .and_then(|entry| entry.get("embedding"))
        .cloned()
        .or_else(|| value.as_array().and_then(|rows| rows.first()).cloned())
        .ok_or_else(|| "embedding response did not contain data[0].embedding".to_string())?;
    let embedding: Vec<f32> = serde_json::from_value(embedding_value)
        .map_err(|error| format!("embedding response vector parse: {error}"))?;
    if embedding.is_empty() {
        Err("embedding response vector was empty".to_string())
    } else {
        Ok(embedding)
    }
}

async fn ensure_qdrant_collection(
    state: &AppState,
    qdrant_url: &str,
    vector_size: usize,
) -> Result<(), String> {
    let url = qdrant_url_for(
        qdrant_url,
        &format!("/collections/{}", state.config.qdrant_collection),
    );
    let response = qdrant_request(state, state.client.put(url))
        .json(&serde_json::json!({
            "vectors": {
                "size": vector_size,
                "distance": "Cosine",
            }
        }))
        .send()
        .await
        .map_err(|error| format!("ensure collection: {error}"))?;
    if response.status().is_success() || response.status() == StatusCode::CONFLICT {
        return Ok(());
    }
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<unreadable response body>".to_string());
    if body.contains("already exists") {
        Ok(())
    } else {
        Err(format!("ensure collection: HTTP {status}: {body}"))
    }
}

fn qdrant_filter(request: &MemorySearchRequest) -> serde_json::Value {
    let mut must = vec![serde_json::json!({
        "key": "goal_id",
        "match": {
            "value": request.goal_id.to_string(),
        }
    })];
    if !request.scopes.is_empty() {
        let scopes = request.scopes.iter().map(json_label).collect::<Vec<_>>();
        must.push(serde_json::json!({
            "key": "scope",
            "match": {
                "any": scopes,
            }
        }));
    }
    serde_json::json!({ "must": must })
}

fn parse_qdrant_hits(value: serde_json::Value) -> Result<Vec<MemorySearchHit>, String> {
    let points = value
        .get("result")
        .and_then(|result| {
            result
                .get("points")
                .and_then(serde_json::Value::as_array)
                .or_else(|| result.as_array())
        })
        .ok_or_else(|| "Qdrant response did not contain result points".to_string())?;
    points.iter().map(parse_qdrant_hit).collect()
}

fn parse_qdrant_hit(value: &serde_json::Value) -> Result<MemorySearchHit, String> {
    let payload = value
        .get("payload")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "Qdrant hit did not contain an object payload".to_string())?;
    let key = payload
        .get("key")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let scope = payload
        .get("scope")
        .cloned()
        .and_then(|scope| serde_json::from_value(scope).ok())
        .unwrap_or(MemoryScope::Goal);
    let source = payload
        .get("source")
        .cloned()
        .and_then(|source| serde_json::from_value(source).ok())
        .unwrap_or(MemoryEpisodeSource {
            source_type: MemoryEpisodeSourceType::Tool,
            uri: None,
            actor: Some("qdrant".to_string()),
        });
    let tags = payload
        .get("tags")
        .cloned()
        .and_then(|tags| serde_json::from_value(tags).ok())
        .unwrap_or_default();
    Ok(MemorySearchHit {
        key,
        scope,
        score: value
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default() as f32,
        summary: payload
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .or_else(|| payload.get("title").and_then(serde_json::Value::as_str))
            .unwrap_or("Qdrant memory hit")
            .to_string(),
        source,
        tags,
    })
}

fn merge_hits(hits: &mut Vec<MemorySearchHit>, vector_hits: Vec<MemorySearchHit>, limit: usize) {
    for hit in vector_hits {
        if !hits.iter().any(|existing| existing.key == hit.key) {
            hits.push(hit);
        }
    }
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.key.cmp(&right.key))
    });
    hits.truncate(limit);
}

fn qdrant_request(state: &AppState, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if let Some(token) = &state.config.qdrant_token {
        request.header("api-key", token)
    } else {
        request
    }
}

async fn qdrant_expect_success(operation: &str, response: reqwest::Response) -> Result<(), String> {
    if response.status().is_success() {
        Ok(())
    } else {
        Err(qdrant_error(operation, response, None).await)
    }
}

async fn qdrant_error(
    operation: &str,
    response: reqwest::Response,
    prefix: Option<String>,
) -> String {
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<unreadable response body>".to_string());
    match prefix {
        Some(prefix) => format!("{prefix}; {operation}: HTTP {status}: {body}"),
        None => format!("{operation}: HTTP {status}: {body}"),
    }
}

fn qdrant_report_from_result(
    operation: &str,
    external_ref: Option<String>,
    result: Result<(), String>,
) -> MemoryAdapterReport {
    match result {
        Ok(()) => MemoryAdapterReport {
            store_kind: MemoryStoreKind::Qdrant,
            operation: operation.to_string(),
            attempted: true,
            success: true,
            external_ref,
            error: None,
        },
        Err(error) => MemoryAdapterReport {
            store_kind: MemoryStoreKind::Qdrant,
            operation: operation.to_string(),
            attempted: true,
            success: false,
            external_ref: None,
            error: Some(error),
        },
    }
}

fn qdrant_config_report(operation: &str, attempted: bool, error: &str) -> MemoryAdapterReport {
    MemoryAdapterReport {
        store_kind: MemoryStoreKind::Qdrant,
        operation: operation.to_string(),
        attempted,
        success: false,
        external_ref: None,
        error: Some(error.to_string()),
    }
}

fn qdrant_url_for(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

fn stable_qdrant_uuid(key: &str) -> String {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("coat-memory-vector:{key}").as_bytes(),
    )
    .to_string()
}

async fn call_graphiti_tool(
    state: &AppState,
    name: &str,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let url = state
        .config
        .graphiti_mcp_url
        .as_ref()
        .ok_or_else(|| "Graphiti MCP adapter is not configured".to_string())?;
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": Uuid::new_v4().to_string(),
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
        },
    });
    let mut request = state
        .client
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .json(&payload);
    if let Some(token) = &state.config.graphiti_token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("call {name}: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable response body>".to_string());
        return Err(format!("call {name}: HTTP {status}: {body}"));
    }
    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("call {name}: parse JSON response: {error}"))?;
    if let Some(error) = value.get("error") {
        return Err(format!("call {name}: MCP error {error}"));
    }
    let result = value.get("result").cloned().unwrap_or(value);
    if let Some(error) = result.get("error") {
        return Err(format!("call {name}: tool error {error}"));
    }
    Ok(result)
}

fn adapter_report_from_result(
    operation: &str,
    external_ref: Option<String>,
    result: Result<serde_json::Value, String>,
) -> MemoryAdapterReport {
    match result {
        Ok(_) => MemoryAdapterReport {
            store_kind: MemoryStoreKind::ZepGraphiti,
            operation: operation.to_string(),
            attempted: true,
            success: true,
            external_ref,
            error: None,
        },
        Err(error) => MemoryAdapterReport {
            store_kind: MemoryStoreKind::ZepGraphiti,
            operation: operation.to_string(),
            attempted: true,
            success: false,
            external_ref: None,
            error: Some(error),
        },
    }
}

fn graphiti_group_id(config: &AppConfig, store: Option<&MemoryStoreRef>) -> String {
    store
        .and_then(|store| store.namespace.clone())
        .unwrap_or_else(|| config.graphiti_group_id.clone())
}

fn graphiti_source_description(scope: &MemoryScope, episode: &MemoryEpisode) -> String {
    format!(
        "coat memory episode; scope={}; source_type={}; actor={}",
        json_label(scope),
        json_label(&episode.source.source_type),
        source_actor(&episode.source)
    )
}

fn json_label<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_string())
}

fn stable_graphiti_uuid(key: &str) -> String {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("coat-memory:{key}").as_bytes(),
    )
    .to_string()
}

fn authorize(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let Some(expected) = &state.config.bearer_token else {
        return Ok(());
    };
    let actual = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if actual == Some(expected.as_str()) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        ))
    }
}

fn bad_request(error: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": error.to_string()})),
    )
}

fn server_error(error: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": error.to_string()})),
    )
}

fn append_journal(path: &Path, entry: &MemoryJournalEntry) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(entry)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    file.sync_data()?;
    Ok(())
}

fn summarize(content: &str) -> String {
    const MAX_CHARS: usize = 240;
    let trimmed = content.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_CHARS).collect::<String>()
}

fn tokenize(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| term.len() > 2)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn lexical_score(query_terms: &[String], haystack: &str) -> f32 {
    if query_terms.is_empty() {
        return 0.0;
    }
    let haystack = haystack.to_ascii_lowercase();
    let matches = query_terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count();
    matches as f32 / query_terms.len() as f32
}

#[allow(dead_code)]
fn source_actor(source: &MemoryEpisodeSource) -> String {
    source
        .actor
        .clone()
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_score_counts_query_term_hits() {
        let terms = tokenize("graphiti semantic memory");
        let score = lexical_score(&terms, "Graphiti is the semantic memory layer");
        assert!(score > 0.6);
    }

    #[test]
    fn qdrant_filter_scopes_goal_and_memory_scope() {
        let goal_id = Uuid::new_v4();
        let filter = qdrant_filter(&MemorySearchRequest {
            goal_id,
            task_id: None,
            query: "durable memory".to_string(),
            scopes: vec![MemoryScope::Goal, MemoryScope::Repo],
            limit: Some(5),
            store: None,
        });

        assert_eq!(
            filter["must"][0]["match"]["value"],
            serde_json::json!(goal_id.to_string())
        );
        assert_eq!(
            filter["must"][1]["match"]["any"],
            serde_json::json!(["goal", "repo"])
        );
    }

    #[test]
    fn parses_qdrant_query_points_response() {
        let response = serde_json::json!({
            "result": {
                "points": [
                    {
                        "id": "8b2c5f21-10d2-5f6b-9974-d0a51e908f58",
                        "score": 0.91,
                        "payload": {
                            "key": "memory-key",
                            "scope": "goal",
                            "summary": "Use Qdrant for vector memory.",
                            "source": {
                                "source_type": "unifier",
                                "uri": "artifact://review",
                                "actor": "unifier"
                            },
                            "tags": ["memory", "qdrant"]
                        }
                    }
                ]
            }
        });

        let hits = parse_qdrant_hits(response).expect("parse qdrant hits");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].key, "memory-key");
        assert_eq!(hits[0].scope, MemoryScope::Goal);
        assert_eq!(hits[0].tags, vec!["memory", "qdrant"]);
        assert_eq!(
            hits[0].source.source_type,
            coat_domain::MemoryEpisodeSourceType::Unifier
        );
    }

    #[tokio::test]
    async fn write_then_search_memory() {
        let memory = Arc::new(RwLock::new(MemoryStore::default()));
        let goal_id = Uuid::new_v4();
        write_memory_inner(
            &memory,
            None,
            MemoryWriteRequest {
                goal_id,
                task_id: None,
                scope: MemoryScope::Goal,
                key: Some("graphiti".to_string()),
                episode: MemoryEpisode {
                    title: "Graphiti decision".to_string(),
                    content: "Use Graphiti for temporal agent memory".to_string(),
                    source: MemoryEpisodeSource {
                        source_type: coat_domain::MemoryEpisodeSourceType::Unifier,
                        uri: None,
                        actor: Some("tester".to_string()),
                    },
                    artifacts: Vec::new(),
                    tags: vec!["memory".to_string()],
                },
                store: None,
            },
        )
        .await
        .expect("write memory");

        let response = search_memory_inner(
            &memory,
            None,
            MemorySearchRequest {
                goal_id,
                task_id: None,
                query: "temporal graphiti memory".to_string(),
                scopes: vec![MemoryScope::Goal],
                limit: Some(5),
                store: None,
            },
        )
        .await
        .expect("search memory");

        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].key, "graphiti");
    }

    #[tokio::test]
    async fn repair_dry_run_counts_selected_adapter_operations() {
        let memory = Arc::new(RwLock::new(MemoryStore::default()));
        let goal_id = Uuid::new_v4();
        write_memory_inner(
            &memory,
            None,
            MemoryWriteRequest {
                goal_id,
                task_id: None,
                scope: MemoryScope::Goal,
                key: Some("repairable".to_string()),
                episode: MemoryEpisode {
                    title: "Repairable memory".to_string(),
                    content: "Replay this memory into external adapters later".to_string(),
                    source: MemoryEpisodeSource {
                        source_type: coat_domain::MemoryEpisodeSourceType::Unifier,
                        uri: None,
                        actor: Some("tester".to_string()),
                    },
                    artifacts: Vec::new(),
                    tags: vec!["repair".to_string()],
                },
                store: None,
            },
        )
        .await
        .expect("write memory");
        let state = AppState {
            memory,
            config: AppConfig {
                bearer_token: None,
                journal_path: None,
                graphiti_mcp_url: Some("http://graphiti:8000/mcp".to_string()),
                graphiti_group_id: "jattg".to_string(),
                graphiti_token: None,
                qdrant_url: Some("http://qdrant:6333".to_string()),
                qdrant_collection: "jattg_memory".to_string(),
                qdrant_token: None,
                embedding_url: None,
                embedding_model: "text-embedding-3-large".to_string(),
                embedding_dimensions: 3072,
                embedding_token: None,
                embedding_send_dimensions: false,
            },
            client: Client::new(),
        };

        let response = repair_memory_with_adapters(
            &state,
            MemoryRepairRequest {
                goal_id: Some(goal_id),
                keys: Vec::new(),
                store_kinds: Vec::new(),
                include_invalidated: false,
                dry_run: true,
            },
        )
        .await
        .expect("repair dry run");

        assert_eq!(response.scanned, 1);
        assert_eq!(response.selected, 1);
        assert_eq!(response.repaired, 0);
        assert_eq!(response.skipped, 2);
    }

    #[tokio::test]
    async fn context_pack_returns_hits_and_use_plan() {
        let memory = Arc::new(RwLock::new(MemoryStore::default()));
        let goal_id = Uuid::new_v4();
        write_memory_inner(
            &memory,
            None,
            MemoryWriteRequest {
                goal_id,
                task_id: None,
                scope: MemoryScope::Repo,
                key: Some("qdrant-policy".to_string()),
                episode: MemoryEpisode {
                    title: "Vector memory policy".to_string(),
                    content:
                        "Use Qdrant for vector RAG memory and keep Graphiti for temporal facts."
                            .to_string(),
                    source: MemoryEpisodeSource {
                        source_type: coat_domain::MemoryEpisodeSourceType::Unifier,
                        uri: None,
                        actor: Some("tester".to_string()),
                    },
                    artifacts: Vec::new(),
                    tags: vec!["qdrant".to_string(), "rag".to_string()],
                },
                store: None,
            },
        )
        .await
        .expect("write memory");
        let state = AppState {
            memory,
            config: AppConfig {
                bearer_token: None,
                journal_path: None,
                graphiti_mcp_url: None,
                graphiti_group_id: "jattg".to_string(),
                graphiti_token: None,
                qdrant_url: None,
                qdrant_collection: "jattg_memory".to_string(),
                qdrant_token: None,
                embedding_url: None,
                embedding_model: "text-embedding-3-large".to_string(),
                embedding_dimensions: 3072,
                embedding_token: None,
                embedding_send_dimensions: false,
            },
            client: Client::new(),
        };

        let response = context_memory_with_adapters(
            &state,
            MemoryContextRequest {
                goal_id,
                task_id: None,
                objective: "qdrant vector rag memory".to_string(),
                scopes: vec![MemoryScope::Repo],
                limit: Some(4),
                store: None,
            },
        )
        .await
        .expect("context pack");

        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].key, "qdrant-policy");
        assert!(
            response
                .use_plan
                .facts_to_use
                .iter()
                .any(|fact| fact.contains("qdrant-policy"))
        );
        assert!(!response.use_plan.validation_checks.is_empty());
    }

    #[tokio::test]
    async fn retract_invalidates_memory_and_replays_from_journal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = dir.path().join("memory.jsonl");
        let memory = Arc::new(RwLock::new(MemoryStore::default()));
        let goal_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        write_memory_inner(
            &memory,
            Some(&journal),
            MemoryWriteRequest {
                goal_id,
                task_id: Some(task_id),
                scope: MemoryScope::Goal,
                key: Some("stale-fact".to_string()),
                episode: MemoryEpisode {
                    title: "Stale fact".to_string(),
                    content: "This memory should no longer be used by workers.".to_string(),
                    source: MemoryEpisodeSource {
                        source_type: coat_domain::MemoryEpisodeSourceType::Human,
                        uri: None,
                        actor: Some("tester".to_string()),
                    },
                    artifacts: Vec::new(),
                    tags: vec!["stale".to_string()],
                },
                store: None,
            },
        )
        .await
        .expect("write memory");

        let response = retract_memory_inner(
            &memory,
            Some(&journal),
            MemoryRetractRequest {
                goal_id,
                task_id: Some(task_id),
                keys: vec!["stale-fact".to_string()],
                reason: "newer evidence superseded this fact".to_string(),
                store: None,
            },
        )
        .await
        .expect("retract memory");

        assert_eq!(response.retracted.len(), 1);
        assert!(response.missing_keys.is_empty());
        assert_eq!(response.retracted[0].action, MemoryEventAction::Retract);

        let search = search_memory_inner(
            &memory,
            None,
            MemorySearchRequest {
                goal_id,
                task_id: None,
                query: "workers".to_string(),
                scopes: vec![MemoryScope::Goal],
                limit: Some(5),
                store: None,
            },
        )
        .await
        .expect("search after retract");
        assert!(search.hits.is_empty());

        let replayed = MemoryStore::load_journal(&journal).expect("replay journal");
        let record = replayed.records.get("stale-fact").expect("record replayed");
        assert!(record.invalidated);
        assert_eq!(replayed.events.get(&goal_id).expect("events").len(), 2);
    }

    #[tokio::test]
    async fn edit_retracts_old_key_and_writes_replacement() {
        let memory = Arc::new(RwLock::new(MemoryStore::default()));
        let goal_id = Uuid::new_v4();
        let state = AppState {
            memory: memory.clone(),
            config: AppConfig {
                bearer_token: None,
                journal_path: None,
                graphiti_mcp_url: None,
                graphiti_group_id: "jattg".to_string(),
                graphiti_token: None,
                qdrant_url: None,
                qdrant_collection: "jattg_memory".to_string(),
                qdrant_token: None,
                embedding_url: None,
                embedding_model: "text-embedding-3-large".to_string(),
                embedding_dimensions: 3072,
                embedding_token: None,
                embedding_send_dimensions: false,
            },
            client: Client::new(),
        };

        write_memory_inner(
            &memory,
            None,
            MemoryWriteRequest {
                goal_id,
                task_id: None,
                scope: MemoryScope::Goal,
                key: Some("candidate".to_string()),
                episode: MemoryEpisode {
                    title: "Candidate".to_string(),
                    content: "Use the preliminary memory substrate decision.".to_string(),
                    source: MemoryEpisodeSource {
                        source_type: coat_domain::MemoryEpisodeSourceType::Research,
                        uri: None,
                        actor: Some("researcher".to_string()),
                    },
                    artifacts: Vec::new(),
                    tags: vec!["candidate".to_string()],
                },
                store: None,
            },
        )
        .await
        .expect("write candidate");

        let preview = preview_memory_edit_inner(
            &memory,
            MemoryEditPreviewRequest {
                goal_id,
                replace_keys: vec!["candidate".to_string()],
                replacement_key: Some("reviewed".to_string()),
                replacement_episode: MemoryEpisode {
                    title: "Reviewed".to_string(),
                    content: "Use the reviewed Graphiti and Qdrant memory decision.".to_string(),
                    source: MemoryEpisodeSource {
                        source_type: coat_domain::MemoryEpisodeSourceType::Human,
                        uri: None,
                        actor: Some("operator".to_string()),
                    },
                    artifacts: Vec::new(),
                    tags: vec!["reviewed".to_string()],
                },
                reason: "critic accepted the replacement".to_string(),
            },
        )
        .await
        .expect("preview edit");
        assert!(preview.ready_to_edit);
        assert_eq!(preview.existing.len(), 1);
        assert_eq!(preview.diffs[0].key, "candidate");
        assert!(preview.diffs[0].after_excerpt.contains("Graphiti"));

        let response = edit_memory_with_adapters(
            &state,
            MemoryEditRequest {
                goal_id,
                task_id: None,
                scope: MemoryScope::Goal,
                replace_keys: vec!["candidate".to_string()],
                replacement_key: Some("reviewed".to_string()),
                replacement_episode: MemoryEpisode {
                    title: "Reviewed".to_string(),
                    content: "Use the reviewed Graphiti and Qdrant memory decision.".to_string(),
                    source: MemoryEpisodeSource {
                        source_type: coat_domain::MemoryEpisodeSourceType::Human,
                        uri: None,
                        actor: Some("operator".to_string()),
                    },
                    artifacts: Vec::new(),
                    tags: vec!["reviewed".to_string()],
                },
                reason: "critic accepted the replacement".to_string(),
                store: None,
            },
        )
        .await
        .expect("edit memory");

        assert_eq!(response.retracted.len(), 1);
        assert_eq!(response.written.key, "reviewed");

        let search = search_memory_inner(
            &memory,
            None,
            MemorySearchRequest {
                goal_id,
                task_id: None,
                query: "reviewed graphiti qdrant".to_string(),
                scopes: vec![MemoryScope::Goal],
                limit: Some(5),
                store: None,
            },
        )
        .await
        .expect("search replacement");
        assert_eq!(search.hits.len(), 1);
        assert_eq!(search.hits[0].key, "reviewed");
    }

    #[tokio::test]
    async fn journal_replays_write_search_and_join() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = dir.path().join("memory.jsonl");
        let memory = Arc::new(RwLock::new(MemoryStore::default()));
        let goal_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        write_memory_inner(
            &memory,
            Some(&journal),
            MemoryWriteRequest {
                goal_id,
                task_id: Some(task_id),
                scope: MemoryScope::Goal,
                key: Some("durable".to_string()),
                episode: MemoryEpisode {
                    title: "Durable memory".to_string(),
                    content: "Replay this durable Graphiti memory decision".to_string(),
                    source: MemoryEpisodeSource {
                        source_type: coat_domain::MemoryEpisodeSourceType::Unifier,
                        uri: None,
                        actor: Some("tester".to_string()),
                    },
                    artifacts: Vec::new(),
                    tags: vec!["durable".to_string()],
                },
                store: None,
            },
        )
        .await
        .expect("journal write");
        search_memory_inner(
            &memory,
            Some(&journal),
            MemorySearchRequest {
                goal_id,
                task_id: None,
                query: "durable graphiti".to_string(),
                scopes: vec![MemoryScope::Goal],
                limit: Some(5),
                store: None,
            },
        )
        .await
        .expect("journal search");
        join_memory_inner(
            &memory,
            Some(&journal),
            MemoryJoinRequest {
                goal_id,
                parent_task_id: None,
                branch_task_ids: vec![task_id],
                unifier_task_id: Some(Uuid::new_v4()),
                promote_keys: vec!["durable".to_string()],
                invalidate_keys: Vec::new(),
                decision: Some(coat_domain::ReviewDecision::Accept),
                reason: "accepted".to_string(),
                store: None,
            },
        )
        .await
        .expect("journal join");

        let replayed = MemoryStore::load_journal(&journal).expect("replay journal");
        let record = replayed.records.get("durable").expect("record replayed");
        assert!(record.promoted);
        assert_eq!(replayed.events.get(&goal_id).expect("events").len(), 3);
    }
}
