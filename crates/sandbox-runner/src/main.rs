//! Workspace lifecycle, launch-plan, snapshot, cleanup, and command-plan service.
//!
//! Purpose: create per-task workspaces, write sandbox manifests and launch
//! plans, return sandbox attestations, optionally create approved live git
//! worktrees, and plan command execution without running arbitrary shell in the
//! control plane.
//!
//! Architecture references:
//! - `docs/design-docs/100-strong-sandboxing-guardrails.md`
//! - `docs/design-docs/060-result-channels-git-object-storage.md`
//! - `docs/exec-plans/active/070-sandbox-tooling.md`

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::SystemTime,
};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use coat_domain::{
    ArtifactKind, ArtifactRef, GitResultPolicy, GitResultRef, NetworkAccess,
    ObjectStorageArtifactRef, ObjectStoragePolicy, SandboxAttestation, SandboxBackend,
    SandboxLaunchPlan, SandboxNetworkPlan, SandboxProfile, SandboxResourcePlan,
    SandboxSecurityPlan,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct AppState {
    workspace_root: PathBuf,
    supported_backends: Vec<SandboxBackend>,
    enable_live_git_worktrees: bool,
    require_live_git_worktree_approval: bool,
    approved_git_repo_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
struct CreateWorkspaceRequest {
    goal_id: Uuid,
    task_id: Uuid,
    repo: Option<String>,
    sandbox: SandboxProfile,
    #[serde(default)]
    git: GitResultPolicy,
    #[serde(default)]
    object_storage: ObjectStoragePolicy,
    #[serde(default)]
    live_git_worktree: LiveGitWorktreePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
struct LiveGitWorktreePolicy {
    enabled: bool,
    approval_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
struct WorkspaceResponse {
    workspace_id: Uuid,
    path: String,
    artifact: ArtifactRef,
    attestation: SandboxAttestation,
    launch_plan: SandboxLaunchPlan,
    git_result: Option<GitResultRef>,
    object_prefix: Option<ObjectStorageArtifactRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
struct WorkspaceId {
    workspace_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
struct WorkspaceRecord {
    workspace_id: Uuid,
    goal_id: Uuid,
    task_id: Uuid,
    path: String,
    repo: Option<String>,
    sandbox: SandboxProfile,
    attestation: SandboxAttestation,
    launch_plan: SandboxLaunchPlan,
    git_result: Option<GitResultRef>,
    object_prefix: Option<ObjectStorageArtifactRef>,
    created_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
struct CommandPlanRequest {
    workspace_id: Option<Uuid>,
    goal_id: Option<Uuid>,
    task_id: Option<Uuid>,
    #[serde(default)]
    command: serde_json::Value,
    approval_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
struct CommandPlanResponse {
    status: String,
    requires_approval: bool,
    approved: bool,
    workspace_id: Option<Uuid>,
    workspace_path: Option<String>,
    command: serde_json::Value,
    next_service: String,
    artifact_manifest_path: Option<String>,
    diagnostics: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "coat_sandbox_runner=info,tower_http=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9083".to_string());
    let workspace_root = std::env::var("SANDBOX_WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/workspaces"));
    let supported_backends = parse_supported_backends();
    let approved_git_repo_roots = parse_approved_git_repo_roots();
    tokio::fs::create_dir_all(&workspace_root).await?;
    tokio::fs::create_dir_all(registry_dir(&workspace_root)).await?;
    let state = Arc::new(AppState {
        workspace_root,
        supported_backends,
        enable_live_git_worktrees: env_bool("SANDBOX_ENABLE_LIVE_GIT_WORKTREES", false),
        require_live_git_worktree_approval: env_bool(
            "SANDBOX_REQUIRE_LIVE_GIT_WORKTREE_APPROVAL",
            true,
        ),
        approved_git_repo_roots,
    });
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/launch-plan", post(launch_plan))
        .route("/workspaces", post(create_workspace))
        .route("/commands/plan", post(command_plan))
        .route("/snapshot", post(snapshot))
        .route("/cleanup", post(cleanup))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "sandbox runner listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn create_workspace(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateWorkspaceRequest>,
) -> Json<WorkspaceResponse> {
    let response = create_workspace_inner(&state, request)
        .await
        .unwrap_or_else(error_response);
    Json(response)
}

async fn create_workspace_inner(
    state: &AppState,
    request: CreateWorkspaceRequest,
) -> anyhow::Result<WorkspaceResponse> {
    let workspace_id = deterministic_workspace_id(request.goal_id, request.task_id);
    let path_buf = workspace_path(&state.workspace_root, request.goal_id, request.task_id);
    let path = path_buf.display().to_string();
    let mut attestation = sandbox_attestation(&request.sandbox, &state.supported_backends);
    let mut git_result = if request.git.enabled {
        Some(GitResultRef {
            repo: request.repo.clone(),
            remote: request.git.remote.clone(),
            base_ref: request.git.base_ref.clone(),
            branch: request.git.branch_for(request.goal_id, request.task_id),
            worktree_path: Some(path.clone()),
            commit: None,
            pushed: false,
            pull_request_url: None,
            diff_uri: None,
        })
    } else {
        None
    };
    if request.git.enabled && request.live_git_worktree.enabled {
        match create_live_git_worktree(
            state,
            &request,
            &path_buf,
            git_result.as_ref().expect("git result initialized"),
        )
        .await
        {
            Ok(report) => {
                if let Some(git_result_ref) = git_result.as_mut() {
                    git_result_ref.repo = Some(report.repo_root.display().to_string());
                    git_result_ref.worktree_path = Some(report.worktree_path.display().to_string());
                }
                attestation.warnings.extend(report.warnings);
                attestation.evidence.push(ArtifactRef {
                    kind: ArtifactKind::GitWorktree,
                    uri: format!("git+worktree://{}", report.branch),
                    description: format!(
                        "live git worktree {} for approved repo {}",
                        report.worktree_path.display(),
                        report.repo_root.display()
                    ),
                    sha256: None,
                });
            }
            Err(error) => attestation
                .warnings
                .push(format!("live git worktree creation skipped: {error}")),
        }
    }
    let object_prefix = request
        .object_storage
        .store
        .clone()
        .filter(|_| request.object_storage.enabled)
        .map(|store| {
            let prefix = request
                .object_storage
                .key_prefix_template
                .replace("{goal_id}", &request.goal_id.to_string())
                .replace("{task_id}", &request.task_id.to_string())
                .trim_matches('/')
                .to_string();
            let key = format!("{prefix}/artifact-manifest.json");
            ObjectStorageArtifactRef {
                uri: format!("s3://{}/{}", store.bucket, key),
                store,
                key,
                content_type: Some("application/json".to_string()),
                size_bytes: None,
                sha256: None,
                description: "task object artifact manifest prefix".to_string(),
            }
        });
    let launch_plan = sandbox_launch_plan(
        request.goal_id,
        request.task_id,
        workspace_id,
        path.clone(),
        &request.sandbox,
        git_result.clone(),
        object_prefix.clone(),
        attestation.warnings.clone(),
    );
    attestation.evidence.push(ArtifactRef {
        kind: ArtifactKind::Other,
        uri: format!("workspace://{workspace_id}/sandbox-launch-plan"),
        description: "sandbox launch plan generated by sandbox runner".to_string(),
        sha256: None,
    });
    let record = WorkspaceRecord {
        workspace_id,
        goal_id: request.goal_id,
        task_id: request.task_id,
        path: path.clone(),
        repo: request.repo,
        sandbox: request.sandbox.clone(),
        attestation: attestation.clone(),
        launch_plan: launch_plan.clone(),
        git_result: git_result.clone(),
        object_prefix: object_prefix.clone(),
        created_at_unix_seconds: unix_seconds(),
    };
    tokio::fs::create_dir_all(path_buf.join("artifacts")).await?;
    tokio::fs::create_dir_all(path_buf.join("snapshots")).await?;
    tokio::fs::create_dir_all(path_buf.join("checkpoints")).await?;
    write_record(state, &record).await?;
    write_workspace_manifest(&path_buf, &record).await?;
    write_launch_plan(&path_buf, &launch_plan).await?;
    write_checkpoint_manifest(&path_buf, &record).await?;
    Ok(WorkspaceResponse {
        workspace_id,
        path: path.clone(),
        artifact: ArtifactRef {
            kind: if git_result.is_some() {
                ArtifactKind::GitWorktree
            } else {
                ArtifactKind::WorkspaceSnapshot
            },
            uri: format!("workspace://{workspace_id}"),
            description: format!("{} at {path}", attestation.isolation_summary),
            sha256: None,
        },
        attestation,
        launch_plan,
        git_result,
        object_prefix,
    })
}

async fn launch_plan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateWorkspaceRequest>,
) -> Json<SandboxLaunchPlan> {
    let workspace_id = deterministic_workspace_id(request.goal_id, request.task_id);
    let path = workspace_path(&state.workspace_root, request.goal_id, request.task_id)
        .display()
        .to_string();
    let attestation = sandbox_attestation(&request.sandbox, &state.supported_backends);
    let git_result = if request.git.enabled {
        Some(GitResultRef {
            repo: request.repo.clone(),
            remote: request.git.remote.clone(),
            base_ref: request.git.base_ref.clone(),
            branch: request.git.branch_for(request.goal_id, request.task_id),
            worktree_path: Some(path.clone()),
            commit: None,
            pushed: false,
            pull_request_url: None,
            diff_uri: None,
        })
    } else {
        None
    };
    let object_prefix = request
        .object_storage
        .store
        .clone()
        .filter(|_| request.object_storage.enabled)
        .map(|store| {
            let prefix = request
                .object_storage
                .key_prefix_template
                .replace("{goal_id}", &request.goal_id.to_string())
                .replace("{task_id}", &request.task_id.to_string())
                .trim_matches('/')
                .to_string();
            let key = format!("{prefix}/artifact-manifest.json");
            ObjectStorageArtifactRef {
                uri: format!("s3://{}/{}", store.bucket, key),
                store,
                key,
                content_type: Some("application/json".to_string()),
                size_bytes: None,
                sha256: None,
                description: "task object artifact manifest prefix".to_string(),
            }
        });
    Json(sandbox_launch_plan(
        request.goal_id,
        request.task_id,
        workspace_id,
        path,
        &request.sandbox,
        git_result,
        object_prefix,
        attestation.warnings,
    ))
}

async fn command_plan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CommandPlanRequest>,
) -> Json<CommandPlanResponse> {
    Json(command_plan_inner(&state, request).await)
}

async fn command_plan_inner(state: &AppState, request: CommandPlanRequest) -> CommandPlanResponse {
    let workspace_id = request
        .workspace_id
        .or_else(|| match (request.goal_id, request.task_id) {
            (Some(goal_id), Some(task_id)) => Some(deterministic_workspace_id(goal_id, task_id)),
            _ => None,
        });
    let mut diagnostics = Vec::new();
    let record = match workspace_id {
        Some(workspace_id) => match read_record(state, workspace_id).await {
            Ok(record) => record,
            Err(error) => {
                diagnostics.push(format!("workspace lookup failed: {error}"));
                None
            }
        },
        None => {
            diagnostics
                .push("command plan requires workspace_id or both goal_id and task_id".to_string());
            None
        }
    };
    let approved = request
        .approval_id
        .as_deref()
        .map(|approval| !approval.trim().is_empty())
        .unwrap_or(false);
    let status = if record.is_none() {
        "workspace_not_found"
    } else if approved {
        "ready_for_executor"
    } else {
        "waiting_approval"
    };
    if !approved {
        diagnostics.push(
            "test command planning requires approval_id before an executor may run it".to_string(),
        );
    }
    let workspace_path = record.as_ref().map(|record| record.path.clone());
    let artifact_manifest_path = workspace_path.as_ref().map(|path| {
        Path::new(path)
            .join("artifacts/artifact-manifest.json")
            .display()
            .to_string()
    });
    CommandPlanResponse {
        status: status.to_string(),
        requires_approval: true,
        approved,
        workspace_id,
        workspace_path,
        command: request.command,
        next_service: if approved {
            "sandbox-executor".to_string()
        } else {
            "coordinator-approval".to_string()
        },
        artifact_manifest_path,
        diagnostics,
    }
}

async fn snapshot(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkspaceId>,
) -> Json<ArtifactRef> {
    Json(snapshot_inner(&state, request.workspace_id).await)
}

async fn snapshot_inner(state: &AppState, workspace_id: Uuid) -> ArtifactRef {
    match read_record(state, workspace_id).await {
        Ok(Some(record)) => {
            let snapshot_dir = PathBuf::from(&record.path).join("snapshots");
            let latest_path = snapshot_dir.join("latest.json");
            let manifest = serde_json::json!({
                "workspace_id": workspace_id,
                "path": record.path,
                "snapshot_created_at_unix_seconds": unix_seconds(),
                "artifact_uri": format!("workspace://{workspace_id}/snapshot/latest"),
                "object_prefix": record.object_prefix,
            });
            let (description, sha256) = match serde_json::to_vec_pretty(&manifest) {
                Ok(bytes) => {
                    let digest = hex_sha256(&bytes);
                    let addressed_path = snapshot_dir.join(format!("{digest}.json"));
                    let write_result = async {
                        tokio::fs::create_dir_all(&snapshot_dir).await?;
                        tokio::fs::write(&addressed_path, &bytes).await?;
                        tokio::fs::write(&latest_path, &bytes).await?;
                        Ok::<(), std::io::Error>(())
                    }
                    .await;
                    match write_result {
                        Ok(()) => (
                            format!(
                                "content-addressed workspace snapshot at {} and latest alias at {}",
                                addressed_path.display(),
                                latest_path.display()
                            ),
                            Some(digest),
                        ),
                        Err(error) => (
                            format!("workspace snapshot could not be written: {error}"),
                            Some(digest),
                        ),
                    }
                }
                Err(error) => (
                    format!("workspace snapshot manifest could not be encoded: {error}"),
                    None,
                ),
            };
            ArtifactRef {
                kind: ArtifactKind::WorkspaceSnapshot,
                uri: format!("workspace://{workspace_id}/snapshot/latest"),
                description,
                sha256,
            }
        }
        Ok(None) => ArtifactRef {
            kind: ArtifactKind::WorkspaceSnapshot,
            uri: format!("workspace://{workspace_id}/snapshot/latest"),
            description: "workspace record not found; snapshot not created".to_string(),
            sha256: None,
        },
        Err(error) => ArtifactRef {
            kind: ArtifactKind::WorkspaceSnapshot,
            uri: format!("workspace://{workspace_id}/snapshot/latest"),
            description: format!("workspace snapshot lookup failed: {error}"),
            sha256: None,
        },
    }
}

async fn cleanup(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkspaceId>,
) -> Json<serde_json::Value> {
    Json(cleanup_inner(&state, request.workspace_id).await)
}

async fn cleanup_inner(state: &AppState, workspace_id: Uuid) -> serde_json::Value {
    match read_record(state, workspace_id).await {
        Ok(Some(record)) => {
            let path = PathBuf::from(&record.path);
            let mut diagnostics = Vec::new();
            if path.starts_with(&state.workspace_root) {
                match tokio::fs::remove_dir_all(&path).await {
                    Ok(()) => diagnostics.push(format!("removed {}", path.display())),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        diagnostics.push(format!("{} already absent", path.display()));
                    }
                    Err(error) => {
                        diagnostics.push(format!("remove {} failed: {error}", path.display()));
                    }
                }
            } else {
                diagnostics.push(format!(
                    "refused to remove {} outside workspace root {}",
                    path.display(),
                    state.workspace_root.display()
                ));
            }
            match tokio::fs::remove_file(record_path(&state.workspace_root, workspace_id)).await {
                Ok(()) => diagnostics.push("removed workspace registry record".to_string()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    diagnostics.push("workspace registry record already absent".to_string());
                }
                Err(error) => diagnostics.push(format!("remove registry record failed: {error}")),
            }
            serde_json::json!({
                "status": "cleaned",
                "workspace_id": workspace_id,
                "diagnostics": diagnostics
            })
        }
        Ok(None) => serde_json::json!({
            "status": "not_found",
            "workspace_id": workspace_id,
            "diagnostics": ["workspace registry record not found"]
        }),
        Err(error) => serde_json::json!({
            "status": "failed",
            "workspace_id": workspace_id,
            "diagnostics": [format!("workspace cleanup lookup failed: {error}")]
        }),
    }
}

fn error_response(error: anyhow::Error) -> WorkspaceResponse {
    let workspace_id = Uuid::new_v4();
    WorkspaceResponse {
        workspace_id,
        path: String::new(),
        artifact: ArtifactRef {
            kind: ArtifactKind::WorkspaceSnapshot,
            uri: format!("workspace://{workspace_id}"),
            description: format!("workspace creation failed: {error}"),
            sha256: None,
        },
        attestation: SandboxAttestation {
            backend: SandboxBackend::LocalWorkspace,
            runtime_class: None,
            enforceable: false,
            strong_isolation: false,
            isolation_summary: "workspace creation failed before sandbox could be attested"
                .to_string(),
            warnings: vec![error.to_string()],
            evidence: Vec::new(),
        },
        launch_plan: SandboxLaunchPlan {
            goal_id: Uuid::nil(),
            task_id: Uuid::nil(),
            workspace_id,
            backend: SandboxBackend::LocalWorkspace,
            runtime_class: None,
            image: None,
            workspace_path: String::new(),
            artifact_manifest_path: String::new(),
            checkpoint_manifest_path: String::new(),
            command: Vec::new(),
            environment: std::collections::BTreeMap::new(),
            required_capabilities: vec![
                SandboxBackend::LocalWorkspace.required_runner_capability(),
            ],
            resources: SandboxResourcePlan {
                cpu_limit_millis: None,
                memory_limit_mb: None,
                pids_limit: None,
                ephemeral_storage_mb: None,
            },
            security: SandboxSecurityPlan {
                read_only_rootfs: false,
                no_new_privileges: true,
                run_as_non_root: true,
                seccomp_profile: None,
                apparmor_profile: None,
                drop_capabilities: Vec::new(),
            },
            network: SandboxNetworkPlan {
                access: NetworkAccess::Restricted,
                deny_by_default: true,
                egress_policy_ref: None,
                allowed_internal_services: Vec::new(),
            },
            git_result: None,
            object_prefix: None,
            warnings: vec![error.to_string()],
        },
        git_result: None,
        object_prefix: None,
    }
}

fn deterministic_workspace_id(goal_id: Uuid, task_id: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("coat-workspace:{goal_id}:{task_id}").as_bytes(),
    )
}

fn workspace_path(root: &std::path::Path, goal_id: Uuid, task_id: Uuid) -> PathBuf {
    root.join(goal_id.to_string()).join(task_id.to_string())
}

fn registry_dir(root: &std::path::Path) -> PathBuf {
    root.join(".coat-workspaces")
}

fn record_path(root: &std::path::Path, workspace_id: Uuid) -> PathBuf {
    registry_dir(root).join(format!("{workspace_id}.json"))
}

async fn write_record(state: &AppState, record: &WorkspaceRecord) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(registry_dir(&state.workspace_root)).await?;
    let bytes = serde_json::to_vec_pretty(record)?;
    tokio::fs::write(
        record_path(&state.workspace_root, record.workspace_id),
        bytes,
    )
    .await?;
    Ok(())
}

async fn read_record(
    state: &AppState,
    workspace_id: Uuid,
) -> anyhow::Result<Option<WorkspaceRecord>> {
    let path = record_path(&state.workspace_root, workspace_id);
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn write_workspace_manifest(
    path: &std::path::Path,
    record: &WorkspaceRecord,
) -> anyhow::Result<()> {
    let manifest_path = path.join("workspace-manifest.json");
    let bytes = serde_json::to_vec_pretty(record)?;
    tokio::fs::write(manifest_path, bytes).await?;
    Ok(())
}

async fn write_launch_plan(
    path: &std::path::Path,
    launch_plan: &SandboxLaunchPlan,
) -> anyhow::Result<()> {
    let plan_path = path.join("sandbox-launch-plan.json");
    let bytes = serde_json::to_vec_pretty(launch_plan)?;
    tokio::fs::write(plan_path, bytes).await?;
    Ok(())
}

async fn write_checkpoint_manifest(
    path: &std::path::Path,
    record: &WorkspaceRecord,
) -> anyhow::Result<()> {
    let manifest_path = path.join("checkpoints/checkpoint-manifest.json");
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "goal_id": record.goal_id,
        "task_id": record.task_id,
        "workspace_id": record.workspace_id,
        "created_at_unix_seconds": record.created_at_unix_seconds,
        "git_result": record.git_result,
        "object_prefix": record.object_prefix,
        "checkpoints": []
    }))?;
    tokio::fs::write(manifest_path, bytes).await?;
    Ok(())
}

fn sandbox_launch_plan(
    goal_id: Uuid,
    task_id: Uuid,
    workspace_id: Uuid,
    workspace_path: String,
    sandbox: &SandboxProfile,
    git_result: Option<GitResultRef>,
    object_prefix: Option<ObjectStorageArtifactRef>,
    warnings: Vec<String>,
) -> SandboxLaunchPlan {
    let artifact_manifest_path = std::path::Path::new(&workspace_path)
        .join("artifacts/artifact-manifest.json")
        .display()
        .to_string();
    let checkpoint_manifest_path = std::path::Path::new(&workspace_path)
        .join("checkpoints/checkpoint-manifest.json")
        .display()
        .to_string();
    let mut environment = std::collections::BTreeMap::new();
    environment.insert("COAT_GOAL_ID".to_string(), goal_id.to_string());
    environment.insert("COAT_TASK_ID".to_string(), task_id.to_string());
    environment.insert("COAT_WORKSPACE_ID".to_string(), workspace_id.to_string());
    environment.insert(
        "COAT_ARTIFACT_MANIFEST".to_string(),
        artifact_manifest_path.clone(),
    );
    environment.insert(
        "COAT_CHECKPOINT_MANIFEST".to_string(),
        checkpoint_manifest_path.clone(),
    );

    SandboxLaunchPlan {
        goal_id,
        task_id,
        workspace_id,
        backend: sandbox.isolation.backend,
        runtime_class: sandbox.isolation.runtime_class.clone(),
        image: sandbox.isolation.image.clone(),
        workspace_path,
        artifact_manifest_path,
        checkpoint_manifest_path,
        command: Vec::new(),
        environment,
        required_capabilities: sandbox.required_runner_capabilities(),
        resources: SandboxResourcePlan {
            cpu_limit_millis: sandbox.isolation.cpu_limit_millis,
            memory_limit_mb: sandbox.isolation.memory_limit_mb,
            pids_limit: sandbox.isolation.pids_limit,
            ephemeral_storage_mb: None,
        },
        security: SandboxSecurityPlan {
            read_only_rootfs: sandbox.isolation.read_only_rootfs,
            no_new_privileges: sandbox.isolation.no_new_privileges,
            run_as_non_root: true,
            seccomp_profile: sandbox.isolation.seccomp_profile.clone(),
            apparmor_profile: sandbox.isolation.apparmor_profile.clone(),
            drop_capabilities: sandbox.isolation.drop_capabilities.clone(),
        },
        network: SandboxNetworkPlan {
            access: sandbox.network.clone(),
            deny_by_default: sandbox.network != NetworkAccess::Open,
            egress_policy_ref: sandbox.isolation.egress_policy_ref.clone(),
            allowed_internal_services: if sandbox.network == NetworkAccess::Disabled {
                Vec::new()
            } else {
                vec![
                    "coordinator".to_string(),
                    "runner-registry".to_string(),
                    "tool-registry".to_string(),
                    "memory-gateway".to_string(),
                    "object-store".to_string(),
                ]
            },
        },
        git_result,
        object_prefix,
        warnings,
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_supported_backends() -> Vec<SandboxBackend> {
    std::env::var("SANDBOX_SUPPORTED_BACKENDS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| parse_sandbox_backend(item.trim()))
                .collect::<Vec<_>>()
        })
        .filter(|backends| !backends.is_empty())
        .unwrap_or_else(|| vec![SandboxBackend::LocalWorkspace])
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn parse_approved_git_repo_roots() -> Vec<PathBuf> {
    std::env::var("SANDBOX_APPROVED_GIT_REPO_ROOTS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| {
                    let item = item.trim();
                    if item.is_empty() {
                        None
                    } else {
                        std::fs::canonicalize(item).ok()
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn parse_sandbox_backend(value: &str) -> Option<SandboxBackend> {
    match value {
        "local_workspace" => Some(SandboxBackend::LocalWorkspace),
        "container" => Some(SandboxBackend::Container),
        "gvisor" => Some(SandboxBackend::Gvisor),
        "firecracker" => Some(SandboxBackend::Firecracker),
        "kata" => Some(SandboxBackend::Kata),
        "kubernetes_job" => Some(SandboxBackend::KubernetesJob),
        "namespace_jail" => Some(SandboxBackend::NamespaceJail),
        "provider_sandbox" => Some(SandboxBackend::ProviderSandbox),
        _ => None,
    }
}

#[derive(Debug)]
struct GitWorktreeReport {
    repo_root: PathBuf,
    worktree_path: PathBuf,
    branch: String,
    warnings: Vec<String>,
}

async fn create_live_git_worktree(
    state: &AppState,
    request: &CreateWorkspaceRequest,
    worktree_path: &Path,
    git_result: &GitResultRef,
) -> anyhow::Result<GitWorktreeReport> {
    if !state.enable_live_git_worktrees {
        anyhow::bail!("SANDBOX_ENABLE_LIVE_GIT_WORKTREES is not enabled");
    }
    if state.require_live_git_worktree_approval
        && request
            .live_git_worktree
            .approval_id
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
    {
        anyhow::bail!("live worktree creation requires live_git_worktree.approval_id");
    }

    let repo = request
        .repo
        .as_deref()
        .filter(|repo| !repo.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("live worktree creation requires repo as a local path"))?;
    let repo_path = std::fs::canonicalize(repo)?;
    let repo_root = git_repo_root(&repo_path)?;
    if !state
        .approved_git_repo_roots
        .iter()
        .any(|root| repo_root.starts_with(root))
    {
        anyhow::bail!(
            "repo {} is not under SANDBOX_APPROVED_GIT_REPO_ROOTS",
            repo_root.display()
        );
    }
    if !worktree_path.starts_with(&state.workspace_root) {
        anyhow::bail!(
            "worktree path {} is outside workspace root {}",
            worktree_path.display(),
            state.workspace_root.display()
        );
    }

    let branch = git_result.branch.clone();
    let base_ref = git_result.base_ref.as_deref().unwrap_or("HEAD");
    let mut warnings = Vec::new();
    if is_git_worktree(worktree_path) {
        warnings.push(format!(
            "reused existing live git worktree at {}",
            worktree_path.display()
        ));
        return Ok(GitWorktreeReport {
            repo_root,
            worktree_path: worktree_path.to_path_buf(),
            branch,
            warnings,
        });
    }

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let branch_exists = git_branch_exists(&repo_root, &branch)?;
    let mut command = Command::new("git");
    command.arg("-C").arg(&repo_root).arg("worktree").arg("add");
    if !branch_exists {
        command.arg("-b").arg(&branch);
    }
    command.arg(worktree_path);
    if branch_exists {
        command.arg(&branch);
    } else {
        command.arg(base_ref);
    }
    let output = command.output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git worktree add failed: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    warnings.push(format!(
        "created live git worktree for branch {branch} at {}",
        worktree_path.display()
    ));
    Ok(GitWorktreeReport {
        repo_root,
        worktree_path: worktree_path.to_path_buf(),
        branch,
        warnings,
    })
}

fn git_repo_root(repo_path: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "{} is not a git repository: {}",
            repo_path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(std::fs::canonicalize(root)?)
}

fn git_branch_exists(repo_root: &Path, branch: &str) -> anyhow::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("show-ref")
        .arg("--verify")
        .arg(format!("refs/heads/{branch}"))
        .output()?;
    Ok(output.status.success())
}

fn is_git_worktree(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn sandbox_attestation(
    profile: &SandboxProfile,
    supported_backends: &[SandboxBackend],
) -> SandboxAttestation {
    let backend = profile.isolation.backend;
    let backend_supported = supported_backends.contains(&backend);
    let enforceable = profile.isolation.enforce && backend_supported;
    let strong_isolation = enforceable && profile.strongly_isolated();
    let mut warnings = Vec::new();
    if !backend_supported {
        warnings.push(format!(
            "requested sandbox backend {} is not supported by this runner",
            backend.as_str()
        ));
    }
    if profile.isolation.enforce && !backend_supported {
        warnings.push(
            "sandbox enforcement requested but this runner can only record metadata".to_string(),
        );
    }
    if !strong_isolation {
        warnings.push("workspace is not attested as a strong isolation boundary".to_string());
    }
    SandboxAttestation {
        backend,
        runtime_class: profile.isolation.runtime_class.clone(),
        enforceable,
        strong_isolation,
        isolation_summary: format!(
            "{} sandbox requested; enforceable={enforceable}; strong_isolation={strong_isolation}",
            backend.as_str()
        ),
        warnings,
        evidence: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn workspace_create_snapshot_and_cleanup_are_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = AppState {
            workspace_root: temp.path().to_path_buf(),
            supported_backends: vec![SandboxBackend::LocalWorkspace],
            enable_live_git_worktrees: false,
            require_live_git_worktree_approval: true,
            approved_git_repo_roots: Vec::new(),
        };
        let goal_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let request = CreateWorkspaceRequest {
            goal_id,
            task_id,
            repo: Some("example/repo".to_string()),
            sandbox: SandboxProfile::default(),
            git: GitResultPolicy {
                enabled: true,
                ..GitResultPolicy::default()
            },
            object_storage: ObjectStoragePolicy::default(),
            live_git_worktree: LiveGitWorktreePolicy::default(),
        };

        let first = create_workspace_inner(&state, request.clone())
            .await
            .expect("create workspace");
        let second = create_workspace_inner(&state, request)
            .await
            .expect("create workspace again");

        assert_eq!(first.workspace_id, second.workspace_id);
        assert!(
            PathBuf::from(&first.path)
                .join("workspace-manifest.json")
                .exists()
        );
        assert!(
            PathBuf::from(&first.path)
                .join("sandbox-launch-plan.json")
                .exists()
        );
        assert_eq!(
            first.git_result.as_ref().expect("git result").worktree_path,
            Some(first.path.clone())
        );
        assert_eq!(first.attestation.backend, SandboxBackend::LocalWorkspace);
        assert!(!first.attestation.strong_isolation);
        assert_eq!(first.launch_plan.workspace_id, first.workspace_id);

        let snapshot = snapshot_inner(&state, first.workspace_id).await;
        assert_eq!(snapshot.kind, ArtifactKind::WorkspaceSnapshot);
        let snapshot_hash = snapshot.sha256.expect("content-addressed snapshot hash");
        assert!(
            PathBuf::from(&first.path)
                .join("snapshots/latest.json")
                .exists()
        );
        assert!(
            PathBuf::from(&first.path)
                .join(format!("snapshots/{snapshot_hash}.json"))
                .exists()
        );

        let cleanup = cleanup_inner(&state, first.workspace_id).await;
        assert_eq!(cleanup["status"], "cleaned");
        assert!(!PathBuf::from(&first.path).exists());

        let cleanup_again = cleanup_inner(&state, first.workspace_id).await;
        assert_eq!(cleanup_again["status"], "not_found");
    }

    #[tokio::test]
    async fn live_git_worktree_requires_enablement_and_approval() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = AppState {
            workspace_root: temp.path().join("workspaces"),
            supported_backends: vec![SandboxBackend::LocalWorkspace],
            enable_live_git_worktrees: false,
            require_live_git_worktree_approval: true,
            approved_git_repo_roots: Vec::new(),
        };
        let request = CreateWorkspaceRequest {
            goal_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            repo: Some(temp.path().display().to_string()),
            sandbox: SandboxProfile::default(),
            git: GitResultPolicy {
                enabled: true,
                ..GitResultPolicy::default()
            },
            object_storage: ObjectStoragePolicy::default(),
            live_git_worktree: LiveGitWorktreePolicy {
                enabled: true,
                approval_id: None,
            },
        };

        let response = create_workspace_inner(&state, request)
            .await
            .expect("metadata workspace still succeeds");

        assert!(response.git_result.is_some());
        assert!(
            response
                .attestation
                .warnings
                .iter()
                .any(|warning| warning.contains("SANDBOX_ENABLE_LIVE_GIT_WORKTREES"))
        );
    }

    #[tokio::test]
    async fn live_git_worktree_creates_approved_branch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        run_git(&repo, &["init"]);
        run_git(&repo, &["config", "user.email", "coat@example.test"]);
        run_git(&repo, &["config", "user.name", "COAT Test"]);
        run_git(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("README.md"), "hello\n").expect("readme");
        run_git(&repo, &["add", "README.md"]);
        run_git(&repo, &["commit", "-m", "init"]);

        let workspace_root = temp.path().join("workspaces");
        let state = AppState {
            workspace_root: workspace_root.clone(),
            supported_backends: vec![SandboxBackend::LocalWorkspace],
            enable_live_git_worktrees: true,
            require_live_git_worktree_approval: true,
            approved_git_repo_roots: vec![temp.path().canonicalize().expect("canonical temp")],
        };
        let goal_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let request = CreateWorkspaceRequest {
            goal_id,
            task_id,
            repo: Some(repo.display().to_string()),
            sandbox: SandboxProfile::default(),
            git: GitResultPolicy {
                enabled: true,
                ..GitResultPolicy::default()
            },
            object_storage: ObjectStoragePolicy::default(),
            live_git_worktree: LiveGitWorktreePolicy {
                enabled: true,
                approval_id: Some("approval-123".to_string()),
            },
        };

        let response = create_workspace_inner(&state, request)
            .await
            .expect("workspace with live git worktree");
        let git_result = response.git_result.expect("git result");
        let worktree = PathBuf::from(git_result.worktree_path.expect("worktree path"));
        assert!(worktree.starts_with(&workspace_root));
        assert!(worktree.join("README.md").exists());
        assert_eq!(
            current_branch(&worktree),
            git_result.branch,
            "worktree is checked out on the task branch"
        );
        assert!(
            response
                .attestation
                .evidence
                .iter()
                .any(|artifact| artifact.kind == ArtifactKind::GitWorktree)
        );
    }

    #[tokio::test]
    async fn command_plan_waits_for_approval_then_targets_executor() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = AppState {
            workspace_root: temp.path().to_path_buf(),
            supported_backends: vec![SandboxBackend::LocalWorkspace],
            enable_live_git_worktrees: false,
            require_live_git_worktree_approval: true,
            approved_git_repo_roots: Vec::new(),
        };
        let goal_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let request = CreateWorkspaceRequest {
            goal_id,
            task_id,
            repo: None,
            sandbox: SandboxProfile::default(),
            git: GitResultPolicy::default(),
            object_storage: ObjectStoragePolicy::default(),
            live_git_worktree: LiveGitWorktreePolicy::default(),
        };
        let workspace = create_workspace_inner(&state, request)
            .await
            .expect("workspace");

        let waiting = command_plan_inner(
            &state,
            CommandPlanRequest {
                workspace_id: Some(workspace.workspace_id),
                command: serde_json::json!("cargo test -p parser"),
                approval_id: None,
                ..CommandPlanRequest::default()
            },
        )
        .await;
        assert_eq!(waiting.status, "waiting_approval");
        assert_eq!(waiting.next_service, "coordinator-approval");

        let approved = command_plan_inner(
            &state,
            CommandPlanRequest {
                workspace_id: Some(workspace.workspace_id),
                command: serde_json::json!("cargo test -p parser"),
                approval_id: Some("approval-123".to_string()),
                ..CommandPlanRequest::default()
            },
        )
        .await;
        assert_eq!(approved.status, "ready_for_executor");
        assert_eq!(approved.next_service, "sandbox-executor");
        assert_eq!(approved.workspace_path, Some(workspace.path));
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }

    fn current_branch(repo: &Path) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .arg("branch")
            .arg("--show-current")
            .output()
            .expect("branch");
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
}
