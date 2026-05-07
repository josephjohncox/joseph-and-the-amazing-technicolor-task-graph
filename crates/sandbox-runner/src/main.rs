use std::{path::PathBuf, sync::Arc, time::SystemTime};

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
    tokio::fs::create_dir_all(&workspace_root).await?;
    tokio::fs::create_dir_all(registry_dir(&workspace_root)).await?;
    let state = Arc::new(AppState {
        workspace_root,
        supported_backends,
    });
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/launch-plan", post(launch_plan))
        .route("/workspaces", post(create_workspace))
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
    tokio::fs::create_dir_all(path_buf.join("artifacts")).await?;
    tokio::fs::create_dir_all(path_buf.join("snapshots")).await?;
    let path = path_buf.display().to_string();
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
    let mut attestation = attestation;
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
    write_record(state, &record).await?;
    write_workspace_manifest(&path_buf, &record).await?;
    write_launch_plan(&path_buf, &launch_plan).await?;
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
    let mut environment = std::collections::BTreeMap::new();
    environment.insert("COAT_GOAL_ID".to_string(), goal_id.to_string());
    environment.insert("COAT_TASK_ID".to_string(), task_id.to_string());
    environment.insert("COAT_WORKSPACE_ID".to_string(), workspace_id.to_string());
    environment.insert(
        "COAT_ARTIFACT_MANIFEST".to_string(),
        artifact_manifest_path.clone(),
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
}
