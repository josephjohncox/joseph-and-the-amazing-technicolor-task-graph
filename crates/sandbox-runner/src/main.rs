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
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command as StdCommand,
    sync::Arc,
    time::SystemTime,
};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use coat_domain::{
    ArtifactKind, ArtifactRef, GitResultPolicy, GitResultRef,
    KubernetesExecutorJobProvisionRequest, KubernetesExecutorJobProvisionResponse,
    KubernetesObjectRef, KubernetesProvisionMode, KubernetesProvisionStatus, LocalToolPolicy,
    NetworkAccess, ObjectStorageArtifactRef, ObjectStoragePolicy, SandboxAttestation,
    SandboxBackend, SandboxLaunchPlan, SandboxNetworkPlan, SandboxProfile, SandboxResourcePlan,
    SandboxSecurityPlan,
};
use k8s_openapi::api::{batch::v1::Job, core::v1::ConfigMap};
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    process::Command as TokioCommand,
    time::{Duration, timeout},
};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct AppState {
    workspace_root: PathBuf,
    supported_backends: Vec<SandboxBackend>,
    enable_live_git_worktrees: bool,
    require_live_git_worktree_approval: bool,
    approved_git_repo_roots: Vec<PathBuf>,
    enable_local_command_execution: bool,
    require_command_approval: bool,
    allowed_local_binaries: Vec<String>,
    command_timeout_seconds: u64,
    command_max_output_bytes: usize,
    enable_kubernetes_provisioner: bool,
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
    #[serde(default)]
    local_tools: LocalToolPolicy,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
struct CommandRunRequest {
    workspace_id: Option<Uuid>,
    goal_id: Option<Uuid>,
    task_id: Option<Uuid>,
    #[serde(default)]
    command: serde_json::Value,
    #[serde(default)]
    local_tools: LocalToolPolicy,
    approval_id: Option<String>,
    cwd: Option<String>,
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
struct CommandRunResponse {
    status: String,
    success: bool,
    workspace_id: Option<Uuid>,
    workspace_path: Option<String>,
    command: Vec<String>,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
    artifact: Option<ArtifactRef>,
    diagnostics: Vec<String>,
    started_at_unix_seconds: u64,
    finished_at_unix_seconds: u64,
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
        enable_local_command_execution: env_bool("SANDBOX_ENABLE_LOCAL_COMMAND_EXECUTION", false),
        require_command_approval: env_bool("SANDBOX_REQUIRE_COMMAND_APPROVAL", true),
        allowed_local_binaries: parse_allowed_local_binaries(),
        command_timeout_seconds: parse_u64_env("SANDBOX_COMMAND_TIMEOUT_SECONDS", 600),
        command_max_output_bytes: parse_usize_env("SANDBOX_COMMAND_MAX_OUTPUT_BYTES", 65_536),
        enable_kubernetes_provisioner: env_bool("SANDBOX_ENABLE_KUBERNETES_PROVISIONER", false),
    });
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/launch-plan", post(launch_plan))
        .route(
            "/kubernetes/executor-jobs/provision",
            post(kubernetes_executor_job_provision),
        )
        .route("/workspaces", post(create_workspace))
        .route("/commands/plan", post(command_plan))
        .route("/commands/run", post(command_run))
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

async fn kubernetes_executor_job_provision(
    State(state): State<Arc<AppState>>,
    Json(request): Json<KubernetesExecutorJobProvisionRequest>,
) -> Json<KubernetesExecutorJobProvisionResponse> {
    let response = match kubernetes_executor_job_provision_inner(&state, request).await {
        Ok(response) => response,
        Err(error) => KubernetesExecutorJobProvisionResponse {
            status: KubernetesProvisionStatus::Failed,
            namespace: "unknown".to_string(),
            job_name: "unknown".to_string(),
            config_map_name: "unknown".to_string(),
            objects: Vec::new(),
            manifest: serde_json::json!({"kind": "List", "items": []}),
            diagnostics: vec![error.to_string()],
        },
    };
    Json(response)
}

async fn kubernetes_executor_job_provision_inner(
    state: &AppState,
    request: KubernetesExecutorJobProvisionRequest,
) -> anyhow::Result<KubernetesExecutorJobProvisionResponse> {
    let manifest = kubernetes_executor_job_manifest(&request)?;
    let (job_name, config_map_name) = provisioned_resource_names(&request);
    let mut diagnostics = vec![
        "Kubernetes executor Jobs are provisioned from SandboxLaunchPlan through the sandbox-runner backend, not from worker-authored manifest snippets.".to_string(),
    ];
    let planned_objects =
        planned_kubernetes_objects(&request.namespace, &job_name, &config_map_name);

    match request.mode {
        KubernetesProvisionMode::PlanOnly => Ok(KubernetesExecutorJobProvisionResponse {
            status: KubernetesProvisionStatus::Planned,
            namespace: request.namespace,
            job_name,
            config_map_name,
            objects: planned_objects,
            manifest,
            diagnostics,
        }),
        KubernetesProvisionMode::ServerDryRun | KubernetesProvisionMode::Apply => {
            if !state.enable_kubernetes_provisioner {
                diagnostics.push(
                    "SANDBOX_ENABLE_KUBERNETES_PROVISIONER is false; returning the generated manifest without contacting the Kubernetes API"
                        .to_string(),
                );
                return Ok(KubernetesExecutorJobProvisionResponse {
                    status: KubernetesProvisionStatus::Planned,
                    namespace: request.namespace,
                    job_name,
                    config_map_name,
                    objects: planned_objects,
                    manifest,
                    diagnostics,
                });
            }
            let server_dry_run = request.mode == KubernetesProvisionMode::ServerDryRun;
            let objects =
                apply_kubernetes_executor_job(&request, &manifest, server_dry_run).await?;
            Ok(KubernetesExecutorJobProvisionResponse {
                status: if server_dry_run {
                    KubernetesProvisionStatus::ServerDryRunAccepted
                } else {
                    KubernetesProvisionStatus::Applied
                },
                namespace: request.namespace,
                job_name,
                config_map_name,
                objects,
                manifest,
                diagnostics,
            })
        }
    }
}

async fn apply_kubernetes_executor_job(
    request: &KubernetesExecutorJobProvisionRequest,
    manifest: &serde_json::Value,
    server_dry_run: bool,
) -> anyhow::Result<Vec<KubernetesObjectRef>> {
    let items = manifest
        .get("items")
        .and_then(serde_json::Value::as_array)
        .context("kubernetes provision manifest must be a List with items")?;
    let config_map_value = items
        .iter()
        .find(|item| item.get("kind").and_then(serde_json::Value::as_str) == Some("ConfigMap"))
        .context("kubernetes provision manifest missing ConfigMap")?;
    let job_value = items
        .iter()
        .find(|item| item.get("kind").and_then(serde_json::Value::as_str) == Some("Job"))
        .context("kubernetes provision manifest missing Job")?;

    let client = Client::try_default()
        .await
        .context("create Kubernetes client from environment")?;
    let config_maps: Api<ConfigMap> = Api::namespaced(client.clone(), &request.namespace);
    let jobs: Api<Job> = Api::namespaced(client, &request.namespace);
    let mut params = PatchParams::apply(&request.field_manager).force();
    if server_dry_run {
        params.dry_run = true;
    }
    let (_, config_map_name) = provisioned_resource_names(request);
    let config_map = config_maps
        .patch(&config_map_name, &params, &Patch::Apply(config_map_value))
        .await
        .with_context(|| format!("server-side apply ConfigMap {config_map_name}"))?;
    let (job_name, _) = provisioned_resource_names(request);
    let job = jobs
        .patch(&job_name, &params, &Patch::Apply(job_value))
        .await
        .with_context(|| format!("server-side apply Job {job_name}"))?;
    Ok(vec![
        object_ref_from_config_map(&request.namespace, &config_map),
        object_ref_from_job(&request.namespace, &job),
    ])
}

fn kubernetes_executor_job_manifest(
    request: &KubernetesExecutorJobProvisionRequest,
) -> anyhow::Result<serde_json::Value> {
    if request.launch_plan.backend != SandboxBackend::KubernetesJob {
        anyhow::bail!(
            "launch_plan.backend must be kubernetes_job for Kubernetes executor provisioning"
        );
    }
    let (job_name, config_map_name) = provisioned_resource_names(request);
    let mut labels = BTreeMap::from([
        ("app.kubernetes.io/name".to_string(), job_name.clone()),
        ("app.kubernetes.io/part-of".to_string(), "jattg".to_string()),
        (
            "app.kubernetes.io/component".to_string(),
            "sandbox-executor".to_string(),
        ),
        (
            "jattg.dev/goal-id".to_string(),
            request.launch_plan.goal_id.to_string(),
        ),
        (
            "jattg.dev/task-id".to_string(),
            request.launch_plan.task_id.to_string(),
        ),
        (
            "jattg.dev/network-access".to_string(),
            network_access_label(&request.launch_plan.network.access).to_string(),
        ),
    ]);
    for (key, value) in &request.launch_plan.network.network_policy_labels {
        labels.insert(key.clone(), value.clone());
    }
    for (key, value) in &request.labels {
        labels.insert(key.clone(), value.clone());
    }
    let mut annotations = request.annotations.clone();
    annotations.insert(
        "jattg.dev/workspace-id".to_string(),
        request.launch_plan.workspace_id.to_string(),
    );
    annotations.insert(
        "jattg.dev/artifact-manifest-path".to_string(),
        request.launch_plan.artifact_manifest_path.clone(),
    );
    annotations.insert(
        "jattg.dev/checkpoint-manifest-path".to_string(),
        request.launch_plan.checkpoint_manifest_path.clone(),
    );
    if let Some(apparmor) = &request.launch_plan.security.apparmor_profile {
        annotations.insert(
            "container.apparmor.security.beta.kubernetes.io/executor".to_string(),
            apparmor.clone(),
        );
    }

    let launch_plan_json = serde_json::to_string_pretty(&request.launch_plan)?;
    let config_map = serde_json::json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {
            "name": config_map_name,
            "namespace": request.namespace,
            "labels": labels,
        },
        "data": {
            "sandbox-launch-plan.json": launch_plan_json,
        },
    });

    let image = request
        .image
        .clone()
        .or_else(|| request.launch_plan.image.clone())
        .unwrap_or_else(|| {
            "ghcr.io/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/jattg-agent-toolbox:latest"
                .to_string()
        });
    let command = if request.launch_plan.command.is_empty() {
        vec!["/usr/local/bin/jattg-ephemeral-entrypoint".to_string()]
    } else {
        request.launch_plan.command.clone()
    };
    let mut env = BTreeMap::from([
        ("COAT_EPHEMERAL_KIND".to_string(), "command".to_string()),
        (
            "COAT_SANDBOX_LAUNCH_PLAN".to_string(),
            "/coat/sandbox-launch-plan.json".to_string(),
        ),
        (
            "COAT_LAUNCH_PLAN_PATH".to_string(),
            "/coat/sandbox-launch-plan.json".to_string(),
        ),
        (
            "COAT_ARTIFACT_MANIFEST_PATH".to_string(),
            request.launch_plan.artifact_manifest_path.clone(),
        ),
        (
            "COAT_CHECKPOINT_MANIFEST".to_string(),
            request.launch_plan.checkpoint_manifest_path.clone(),
        ),
        ("HOME".to_string(), request.workspace_mount_path.clone()),
    ]);
    for (key, value) in &request.launch_plan.environment {
        env.insert(key.clone(), value.clone());
    }

    let job = serde_json::json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {
            "name": job_name,
            "namespace": request.namespace,
            "labels": labels,
            "annotations": annotations,
        },
        "spec": {
            "activeDeadlineSeconds": request.active_deadline_seconds,
            "ttlSecondsAfterFinished": request.ttl_seconds_after_finished,
            "backoffLimit": request.backoff_limit,
            "template": {
                "metadata": {
                    "labels": labels,
                    "annotations": annotations,
                },
                "spec": {
                    "restartPolicy": "Never",
                    "serviceAccountName": request.service_account.clone().unwrap_or_else(|| "jattg-sandbox-task".to_string()),
                    "runtimeClassName": request.runtime_class.clone().or_else(|| request.launch_plan.runtime_class.clone()),
                    "securityContext": pod_security_context(&request.launch_plan.security),
                    "containers": [{
                        "name": "executor",
                        "image": image,
                        "command": command,
                        "env": env.into_iter().map(|(name, value)| serde_json::json!({"name": name, "value": value})).collect::<Vec<_>>(),
                        "resources": container_resources(&request.launch_plan.resources),
                        "securityContext": container_security_context(&request.launch_plan.security),
                        "volumeMounts": [
                            {
                                "name": "launch-plan",
                                "mountPath": "/coat/sandbox-launch-plan.json",
                                "subPath": "sandbox-launch-plan.json",
                                "readOnly": true
                            },
                            {
                                "name": "workspace",
                                "mountPath": request.workspace_mount_path
                            }
                        ]
                    }],
                    "volumes": [
                        {
                            "name": "launch-plan",
                            "configMap": {
                                "name": config_map_name
                            }
                        },
                        workspace_volume(request)
                    ]
                }
            }
        }
    });

    Ok(serde_json::json!({
        "apiVersion": "v1",
        "kind": "List",
        "items": [config_map, job],
    }))
}

fn provisioned_resource_names(request: &KubernetesExecutorJobProvisionRequest) -> (String, String) {
    let job_name = k8s_name(request.name.clone().unwrap_or_else(|| {
        format!(
            "jattg-executor-{}-{}",
            short_uuid(request.launch_plan.goal_id),
            short_uuid(request.launch_plan.task_id)
        )
    }));
    let config_map_name = k8s_name(format!("{job_name}-plan"));
    (job_name, config_map_name)
}

fn planned_kubernetes_objects(
    namespace: &str,
    job_name: &str,
    config_map_name: &str,
) -> Vec<KubernetesObjectRef> {
    vec![
        KubernetesObjectRef {
            api_version: "v1".to_string(),
            kind: "ConfigMap".to_string(),
            namespace: namespace.to_string(),
            name: config_map_name.to_string(),
            uid: None,
            resource_version: None,
        },
        KubernetesObjectRef {
            api_version: "batch/v1".to_string(),
            kind: "Job".to_string(),
            namespace: namespace.to_string(),
            name: job_name.to_string(),
            uid: None,
            resource_version: None,
        },
    ]
}

fn object_ref_from_config_map(namespace: &str, config_map: &ConfigMap) -> KubernetesObjectRef {
    KubernetesObjectRef {
        api_version: "v1".to_string(),
        kind: "ConfigMap".to_string(),
        namespace: namespace.to_string(),
        name: config_map.metadata.name.clone().unwrap_or_default(),
        uid: config_map.metadata.uid.clone(),
        resource_version: config_map.metadata.resource_version.clone(),
    }
}

fn object_ref_from_job(namespace: &str, job: &Job) -> KubernetesObjectRef {
    KubernetesObjectRef {
        api_version: "batch/v1".to_string(),
        kind: "Job".to_string(),
        namespace: namespace.to_string(),
        name: job.metadata.name.clone().unwrap_or_default(),
        uid: job.metadata.uid.clone(),
        resource_version: job.metadata.resource_version.clone(),
    }
}

fn pod_security_context(security: &SandboxSecurityPlan) -> serde_json::Value {
    let mut value = serde_json::json!({
        "runAsNonRoot": security.run_as_non_root,
    });
    if let Some(profile) = &security.seccomp_profile {
        value["seccompProfile"] = serde_json::json!({
            "type": if profile == "RuntimeDefault" || profile == "Localhost" {
                profile
            } else {
                "Localhost"
            },
            "localhostProfile": if profile == "RuntimeDefault" || profile == "Localhost" {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(profile.clone())
            }
        });
    }
    value
}

fn container_security_context(security: &SandboxSecurityPlan) -> serde_json::Value {
    serde_json::json!({
        "allowPrivilegeEscalation": !security.no_new_privileges,
        "readOnlyRootFilesystem": security.read_only_rootfs,
        "runAsNonRoot": security.run_as_non_root,
        "capabilities": {
            "drop": if security.drop_capabilities.is_empty() {
                vec!["ALL".to_string()]
            } else {
                security.drop_capabilities.clone()
            }
        }
    })
}

fn container_resources(resources: &SandboxResourcePlan) -> serde_json::Value {
    let mut limits = serde_json::Map::new();
    if let Some(cpu_millis) = resources.cpu_limit_millis {
        limits.insert(
            "cpu".to_string(),
            serde_json::json!(format!("{cpu_millis}m")),
        );
    }
    if let Some(memory_mb) = resources.memory_limit_mb {
        limits.insert(
            "memory".to_string(),
            serde_json::json!(format!("{memory_mb}Mi")),
        );
    }
    if let Some(ephemeral_storage_mb) = resources.ephemeral_storage_mb {
        limits.insert(
            "ephemeral-storage".to_string(),
            serde_json::json!(format!("{ephemeral_storage_mb}Mi")),
        );
    }
    serde_json::json!({
        "requests": limits.clone(),
        "limits": limits,
    })
}

fn workspace_volume(request: &KubernetesExecutorJobProvisionRequest) -> serde_json::Value {
    if let Some(claim_name) = &request.workspace_pvc {
        serde_json::json!({
            "name": "workspace",
            "persistentVolumeClaim": {
                "claimName": claim_name
            }
        })
    } else {
        serde_json::json!({
            "name": "workspace",
            "emptyDir": {}
        })
    }
}

fn network_access_label(access: &NetworkAccess) -> &'static str {
    match access {
        NetworkAccess::Disabled => "disabled",
        NetworkAccess::Restricted => "restricted",
        NetworkAccess::Open => "open",
    }
}

fn short_uuid(id: Uuid) -> String {
    id.simple().to_string()[..12].to_string()
}

fn k8s_name(input: impl Into<String>) -> String {
    let mut name = input
        .into()
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while name.contains("--") {
        name = name.replace("--", "-");
    }
    name = name.trim_matches('-').to_string();
    if name.is_empty() {
        name = "jattg-executor".to_string();
    }
    if name.len() > 63 {
        name.truncate(63);
        name = name.trim_matches('-').to_string();
    }
    name
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
    let approval_satisfied = !state.require_command_approval || approved;
    let policy_status = match parse_command_value(&request.command) {
        Ok(command) => validate_command_policy(record.as_ref(), &request.local_tools, &command)
            .err()
            .map(|error| ("tool_policy_denied", error)),
        Err(error) => Some(("invalid_command", error)),
    };
    let status = if record.is_none() {
        "workspace_not_found"
    } else if let Some((status, _)) = policy_status.as_ref() {
        *status
    } else if approval_satisfied {
        "ready_for_executor"
    } else {
        "waiting_approval"
    };
    if state.enable_local_command_execution {
        diagnostics.push(format!(
            "local command execution is enabled for allowlisted binaries: {}",
            state.allowed_local_binaries.join(", ")
        ));
    } else {
        diagnostics.push(
            "local command execution is disabled; an external sandbox executor must consume this plan"
                .to_string(),
        );
    }
    if state.require_command_approval && !approved {
        diagnostics.push(
            "test command planning requires approval_id before an executor may run it".to_string(),
        );
    }
    if let Some((_, error)) = policy_status {
        diagnostics.push(error);
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
        requires_approval: state.require_command_approval,
        approved: approval_satisfied,
        workspace_id,
        workspace_path,
        command: request.command,
        next_service: if status == "workspace_not_found"
            || status == "invalid_command"
            || status == "tool_policy_denied"
        {
            "operator-fix".to_string()
        } else if approval_satisfied {
            if state.enable_local_command_execution {
                "sandbox-runner:/commands/run".to_string()
            } else {
                "sandbox-executor".to_string()
            }
        } else {
            "coordinator-approval".to_string()
        },
        artifact_manifest_path,
        diagnostics,
    }
}

async fn command_run(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CommandRunRequest>,
) -> Json<CommandRunResponse> {
    Json(command_run_inner(&state, request).await)
}

async fn command_run_inner(state: &AppState, request: CommandRunRequest) -> CommandRunResponse {
    let started_at = unix_seconds();
    let workspace_id = request
        .workspace_id
        .or_else(|| match (request.goal_id, request.task_id) {
            (Some(goal_id), Some(task_id)) => Some(deterministic_workspace_id(goal_id, task_id)),
            _ => None,
        });
    let mut diagnostics = Vec::new();
    let mut command = Vec::new();
    let mut workspace_path = None;

    let Some(workspace_id_value) = workspace_id else {
        diagnostics
            .push("command run requires workspace_id or both goal_id and task_id".to_string());
        return command_run_blocked(
            "workspace_not_found",
            workspace_id,
            workspace_path,
            command,
            diagnostics,
            started_at,
        );
    };

    if !state.enable_local_command_execution {
        diagnostics.push(
            "SANDBOX_ENABLE_LOCAL_COMMAND_EXECUTION is false; use /commands/plan and an external sandbox executor"
                .to_string(),
        );
        return command_run_blocked(
            "execution_disabled",
            workspace_id,
            workspace_path,
            command,
            diagnostics,
            started_at,
        );
    }

    let approved = request
        .approval_id
        .as_deref()
        .map(|approval| !approval.trim().is_empty())
        .unwrap_or(false);
    if state.require_command_approval && !approved {
        diagnostics.push("approval_id is required before local command execution".to_string());
        return command_run_blocked(
            "waiting_approval",
            workspace_id,
            workspace_path,
            command,
            diagnostics,
            started_at,
        );
    }

    let record = match read_record(state, workspace_id_value).await {
        Ok(Some(record)) => record,
        Ok(None) => {
            diagnostics.push(format!("workspace {workspace_id_value} was not found"));
            return command_run_blocked(
                "workspace_not_found",
                workspace_id,
                workspace_path,
                command,
                diagnostics,
                started_at,
            );
        }
        Err(error) => {
            diagnostics.push(format!("workspace lookup failed: {error}"));
            return command_run_blocked(
                "workspace_not_found",
                workspace_id,
                workspace_path,
                command,
                diagnostics,
                started_at,
            );
        }
    };
    workspace_path = Some(record.path.clone());

    command = match parse_command_value(&request.command) {
        Ok(command) => command,
        Err(error) => {
            diagnostics.push(error);
            return command_run_blocked(
                "invalid_command",
                workspace_id,
                workspace_path,
                command,
                diagnostics,
                started_at,
            );
        }
    };

    let policy_timeout_seconds =
        match validate_command_policy(Some(&record), &request.local_tools, &command) {
            Ok(timeout_seconds) => timeout_seconds,
            Err(error) => {
                diagnostics.push(error);
                return command_run_blocked(
                    "tool_policy_denied",
                    workspace_id,
                    workspace_path,
                    command,
                    diagnostics,
                    started_at,
                );
            }
        };

    if let Err(error) = validate_allowed_binary(state, &command) {
        diagnostics.push(error);
        return command_run_blocked(
            "binary_not_allowed",
            workspace_id,
            workspace_path,
            command,
            diagnostics,
            started_at,
        );
    }

    let cwd = match resolve_command_cwd(&record, request.cwd.as_deref()) {
        Ok(cwd) => cwd,
        Err(error) => {
            diagnostics.push(error.to_string());
            return command_run_blocked(
                "invalid_cwd",
                workspace_id,
                workspace_path,
                command,
                diagnostics,
                started_at,
            );
        }
    };
    let timeout_seconds = request
        .timeout_seconds
        .or(policy_timeout_seconds)
        .unwrap_or(state.command_timeout_seconds)
        .clamp(1, state.command_timeout_seconds.max(1));
    let max_output_bytes = command_max_output_bytes(state, &request.local_tools);

    let mut child = TokioCommand::new(&command[0]);
    child.args(&command[1..]).current_dir(&cwd);
    let output = match timeout(Duration::from_secs(timeout_seconds), child.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            diagnostics.push(format!("failed to execute local command: {error}"));
            return command_run_blocked(
                "execution_failed",
                workspace_id,
                workspace_path,
                command,
                diagnostics,
                started_at,
            );
        }
        Err(_) => {
            diagnostics.push(format!("command timed out after {timeout_seconds}s"));
            return command_run_blocked(
                "timed_out",
                workspace_id,
                workspace_path,
                command,
                diagnostics,
                started_at,
            );
        }
    };

    let (stdout, stdout_truncated) = truncate_output(&output.stdout, max_output_bytes);
    let (stderr, stderr_truncated) = truncate_output(&output.stderr, max_output_bytes);
    let exit_code = output.status.code();
    let success = output.status.success();
    let artifact = write_command_evidence(
        workspace_id_value,
        &record,
        &command,
        &cwd,
        exit_code,
        success,
        &stdout,
        &stderr,
        stdout_truncated,
        stderr_truncated,
        started_at,
    )
    .await;
    if let Err(error) = artifact.as_ref() {
        diagnostics.push(format!(
            "failed to write command evidence artifact: {error}"
        ));
    }

    CommandRunResponse {
        status: if success { "completed" } else { "failed" }.to_string(),
        success,
        workspace_id,
        workspace_path,
        command,
        exit_code,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        artifact: artifact.ok(),
        diagnostics,
        started_at_unix_seconds: started_at,
        finished_at_unix_seconds: unix_seconds(),
    }
}

fn command_run_blocked(
    status: &str,
    workspace_id: Option<Uuid>,
    workspace_path: Option<String>,
    command: Vec<String>,
    diagnostics: Vec<String>,
    started_at: u64,
) -> CommandRunResponse {
    CommandRunResponse {
        status: status.to_string(),
        success: false,
        workspace_id,
        workspace_path,
        command,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        artifact: None,
        diagnostics,
        started_at_unix_seconds: started_at,
        finished_at_unix_seconds: unix_seconds(),
    }
}

fn parse_command_value(value: &serde_json::Value) -> Result<Vec<String>, String> {
    match value {
        serde_json::Value::Array(items) => {
            let command = items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "command array entries must be strings".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if command.is_empty() {
                Err("command array must not be empty".to_string())
            } else {
                Ok(command)
            }
        }
        serde_json::Value::String(command) => {
            let parts = command
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>();
            if parts.is_empty() {
                Err("command string must not be empty".to_string())
            } else {
                Ok(parts)
            }
        }
        _ => Err("command must be a string or an argv array".to_string()),
    }
}

fn validate_allowed_binary(state: &AppState, command: &[String]) -> Result<(), String> {
    let binary = command
        .first()
        .ok_or_else(|| "command must include a binary".to_string())?;
    if binary.contains('/') || binary.contains('\\') {
        return Err(
            "command binary must be a bare executable name; absolute or relative paths are not allowed"
                .to_string(),
        );
    }
    if state
        .allowed_local_binaries
        .iter()
        .any(|allowed| allowed == binary)
    {
        Ok(())
    } else {
        Err(format!(
            "binary {binary} is not in SANDBOX_ALLOWED_LOCAL_BINARIES"
        ))
    }
}

fn validate_command_policy(
    record: Option<&WorkspaceRecord>,
    policy: &LocalToolPolicy,
    command: &[String],
) -> Result<Option<u64>, String> {
    let binary = command
        .first()
        .ok_or_else(|| "command must include a binary".to_string())?;
    if !policy.is_active() {
        return Ok(None);
    }
    if policy.denied_binaries.iter().any(|denied| denied == binary) {
        return Err(format!(
            "binary {binary} is denied by task local_tools.denied_binaries"
        ));
    }
    let permission = policy
        .allowed_tools
        .iter()
        .find(|tool| tool.binary == *binary)
        .ok_or_else(|| {
            format!("binary {binary} is not declared in task local_tools.allowed_tools")
        })?;

    if !permission.allowed_subcommands.is_empty() {
        let Some(subcommand) = command.get(1) else {
            return Err(format!(
                "binary {binary} requires one of the allowed subcommands: {}",
                permission.allowed_subcommands.join(", ")
            ));
        };
        if !permission
            .allowed_subcommands
            .iter()
            .any(|allowed| allowed == subcommand)
        {
            return Err(format!(
                "subcommand {subcommand} for binary {binary} is not allowed by task local_tools; allowed: {}",
                permission.allowed_subcommands.join(", ")
            ));
        }
    }

    for arg in command.iter().skip(1) {
        if let Some(denied) = permission
            .denied_args
            .iter()
            .find(|denied| command_arg_matches_denied(arg, denied))
        {
            return Err(format!(
                "argument {arg} for binary {binary} is denied by task local_tools.denied_args entry {denied}"
            ));
        }
    }

    if permission.requires_network
        && record
            .map(|record| record.sandbox.network == NetworkAccess::Disabled)
            .unwrap_or(false)
    {
        return Err(format!(
            "binary {binary} requires network access, but the workspace sandbox profile has network disabled"
        ));
    }
    Ok(permission
        .timeout_seconds
        .or_else(|| (policy.default_timeout_seconds > 0).then_some(policy.default_timeout_seconds)))
}

fn command_arg_matches_denied(arg: &str, denied: &str) -> bool {
    arg == denied || (denied.starts_with("--") && arg.starts_with(&format!("{denied}=")))
}

fn command_max_output_bytes(state: &AppState, policy: &LocalToolPolicy) -> usize {
    if policy.is_active() && policy.max_output_bytes > 0 {
        state
            .command_max_output_bytes
            .min(policy.max_output_bytes.try_into().unwrap_or(usize::MAX))
    } else {
        state.command_max_output_bytes
    }
}

fn resolve_command_cwd(
    record: &WorkspaceRecord,
    requested: Option<&str>,
) -> anyhow::Result<PathBuf> {
    let workspace = PathBuf::from(&record.path).canonicalize()?;
    let candidate = requested
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.clone());
    let path = if candidate.is_absolute() {
        candidate
    } else {
        workspace.join(candidate)
    };
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(&workspace) {
        anyhow::bail!(
            "command cwd {} is outside workspace {}",
            canonical.display(),
            workspace.display()
        );
    }
    Ok(canonical)
}

fn truncate_output(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    let truncated = bytes.len() > max_bytes;
    let slice = if truncated {
        &bytes[..max_bytes]
    } else {
        bytes
    };
    (String::from_utf8_lossy(slice).to_string(), truncated)
}

#[allow(clippy::too_many_arguments)]
async fn write_command_evidence(
    workspace_id: Uuid,
    record: &WorkspaceRecord,
    command: &[String],
    cwd: &Path,
    exit_code: Option<i32>,
    success: bool,
    stdout: &str,
    stderr: &str,
    stdout_truncated: bool,
    stderr_truncated: bool,
    started_at: u64,
) -> anyhow::Result<ArtifactRef> {
    let finished_at = unix_seconds();
    let evidence = serde_json::json!({
        "workspace_id": workspace_id,
        "goal_id": record.goal_id,
        "task_id": record.task_id,
        "command": command,
        "cwd": cwd.display().to_string(),
        "exit_code": exit_code,
        "success": success,
        "stdout": stdout,
        "stderr": stderr,
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated,
        "started_at_unix_seconds": started_at,
        "finished_at_unix_seconds": finished_at
    });
    let bytes = serde_json::to_vec_pretty(&evidence)?;
    let digest = hex_sha256(&bytes);
    let artifact_dir = PathBuf::from(&record.path).join("artifacts");
    tokio::fs::create_dir_all(&artifact_dir).await?;
    let path = artifact_dir.join(format!("command-{digest}.json"));
    tokio::fs::write(&path, bytes).await?;
    Ok(ArtifactRef {
        kind: ArtifactKind::TestResult,
        uri: format!("workspace://{workspace_id}/artifacts/command-{digest}.json"),
        description: format!("local command evidence for {}", command.join(" ")),
        sha256: Some(digest),
    })
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
                ingress_policy_ref: None,
                network_policy_labels: std::collections::BTreeMap::new(),
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
            ingress_policy_ref: sandbox.isolation.ingress_policy_ref.clone(),
            network_policy_labels: sandbox.isolation.network_policy_labels.clone(),
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

fn parse_u64_env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn parse_usize_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn parse_allowed_local_binaries() -> Vec<String> {
    std::env::var("SANDBOX_ALLOWED_LOCAL_BINARIES")
        .unwrap_or_else(|_| {
            "git,make,cargo,npm,pnpm,yarn,node,python3,python,pytest,go,buf,docker,helm,kubectl"
                .to_string()
        })
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
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
    let mut command = StdCommand::new("git");
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
    let output = StdCommand::new("git")
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
    let output = StdCommand::new("git")
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
    StdCommand::new("git")
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
    use coat_domain::{
        KubernetesExecutorJobProvisionRequest, KubernetesProvisionMode, LocalToolCategory,
        LocalToolPermission, LocalToolRisk, RunnerCapability,
    };

    #[tokio::test]
    async fn workspace_create_snapshot_and_cleanup_are_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = AppState {
            workspace_root: temp.path().to_path_buf(),
            supported_backends: vec![SandboxBackend::LocalWorkspace],
            enable_live_git_worktrees: false,
            require_live_git_worktree_approval: true,
            approved_git_repo_roots: Vec::new(),
            enable_local_command_execution: false,
            require_command_approval: true,
            allowed_local_binaries: parse_allowed_local_binaries(),
            command_timeout_seconds: 600,
            command_max_output_bytes: 65_536,
            enable_kubernetes_provisioner: false,
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
            enable_local_command_execution: false,
            require_command_approval: true,
            allowed_local_binaries: parse_allowed_local_binaries(),
            command_timeout_seconds: 600,
            command_max_output_bytes: 65_536,
            enable_kubernetes_provisioner: false,
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
            enable_local_command_execution: false,
            require_command_approval: true,
            allowed_local_binaries: parse_allowed_local_binaries(),
            command_timeout_seconds: 600,
            command_max_output_bytes: 65_536,
            enable_kubernetes_provisioner: false,
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
            enable_local_command_execution: false,
            require_command_approval: true,
            allowed_local_binaries: parse_allowed_local_binaries(),
            command_timeout_seconds: 600,
            command_max_output_bytes: 65_536,
            enable_kubernetes_provisioner: false,
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

    #[tokio::test]
    async fn command_run_executes_allowlisted_binary_with_evidence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = AppState {
            workspace_root: temp.path().to_path_buf(),
            supported_backends: vec![SandboxBackend::LocalWorkspace],
            enable_live_git_worktrees: false,
            require_live_git_worktree_approval: true,
            approved_git_repo_roots: Vec::new(),
            enable_local_command_execution: true,
            require_command_approval: true,
            allowed_local_binaries: vec!["git".to_string()],
            command_timeout_seconds: 30,
            command_max_output_bytes: 65_536,
            enable_kubernetes_provisioner: false,
        };
        let request = CreateWorkspaceRequest {
            goal_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            repo: None,
            sandbox: SandboxProfile::default(),
            git: GitResultPolicy::default(),
            object_storage: ObjectStoragePolicy::default(),
            live_git_worktree: LiveGitWorktreePolicy::default(),
        };
        let workspace = create_workspace_inner(&state, request)
            .await
            .expect("workspace");

        let blocked = command_run_inner(
            &state,
            CommandRunRequest {
                workspace_id: Some(workspace.workspace_id),
                command: serde_json::json!(["git", "--version"]),
                approval_id: None,
                ..CommandRunRequest::default()
            },
        )
        .await;
        assert_eq!(blocked.status, "waiting_approval");

        let ran = command_run_inner(
            &state,
            CommandRunRequest {
                workspace_id: Some(workspace.workspace_id),
                command: serde_json::json!(["git", "--version"]),
                approval_id: Some("approval-123".to_string()),
                ..CommandRunRequest::default()
            },
        )
        .await;
        assert!(ran.success, "{:?}", ran.diagnostics);
        assert!(ran.stdout.contains("git version"));
        assert!(ran.artifact.is_some());
        assert!(
            PathBuf::from(workspace.path)
                .join("artifacts")
                .read_dir()
                .expect("artifact dir")
                .next()
                .is_some()
        );
    }

    #[tokio::test]
    async fn command_run_enforces_task_local_tool_policy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = AppState {
            workspace_root: temp.path().to_path_buf(),
            supported_backends: vec![SandboxBackend::LocalWorkspace],
            enable_live_git_worktrees: false,
            require_live_git_worktree_approval: true,
            approved_git_repo_roots: Vec::new(),
            enable_local_command_execution: true,
            require_command_approval: false,
            allowed_local_binaries: vec!["git".to_string(), "docker".to_string()],
            command_timeout_seconds: 30,
            command_max_output_bytes: 65_536,
            enable_kubernetes_provisioner: false,
        };
        let request = CreateWorkspaceRequest {
            goal_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            repo: None,
            sandbox: SandboxProfile::default(),
            git: GitResultPolicy::default(),
            object_storage: ObjectStoragePolicy::default(),
            live_git_worktree: LiveGitWorktreePolicy::default(),
        };
        let workspace = create_workspace_inner(&state, request)
            .await
            .expect("workspace");
        let local_tools = LocalToolPolicy {
            enabled: true,
            allowed_tools: vec![LocalToolPermission {
                binary: "git".to_string(),
                category: LocalToolCategory::VersionControl,
                risk: LocalToolRisk::Low,
                allowed_subcommands: vec!["status".to_string()],
                denied_args: vec!["--porcelain".to_string()],
                requires_network: false,
                requires_docker_socket: false,
                requires_cluster_access: false,
                required_capabilities: Vec::new(),
                required_labels: Default::default(),
                timeout_seconds: Some(5),
            }],
            denied_binaries: vec!["docker".to_string()],
            default_timeout_seconds: 5,
            max_output_bytes: 1_024,
            ..LocalToolPolicy::default()
        };

        let denied_plan = command_plan_inner(
            &state,
            CommandPlanRequest {
                workspace_id: Some(workspace.workspace_id),
                command: serde_json::json!(["git", "clone", "https://example.invalid/repo.git"]),
                local_tools: local_tools.clone(),
                approval_id: Some("approval-123".to_string()),
                ..CommandPlanRequest::default()
            },
        )
        .await;
        assert_eq!(denied_plan.status, "tool_policy_denied");
        assert_eq!(denied_plan.next_service, "operator-fix");

        let denied_binary = command_run_inner(
            &state,
            CommandRunRequest {
                workspace_id: Some(workspace.workspace_id),
                command: serde_json::json!(["docker", "ps"]),
                local_tools: local_tools.clone(),
                ..CommandRunRequest::default()
            },
        )
        .await;
        assert_eq!(denied_binary.status, "tool_policy_denied");
        assert!(
            denied_binary
                .diagnostics
                .iter()
                .any(|line| line.contains("denied_binaries"))
        );

        let denied_subcommand = command_run_inner(
            &state,
            CommandRunRequest {
                workspace_id: Some(workspace.workspace_id),
                command: serde_json::json!(["git", "clone", "https://example.invalid/repo.git"]),
                local_tools: local_tools.clone(),
                ..CommandRunRequest::default()
            },
        )
        .await;
        assert_eq!(denied_subcommand.status, "tool_policy_denied");
        assert!(
            denied_subcommand
                .diagnostics
                .iter()
                .any(|line| line.contains("subcommand clone"))
        );

        let denied_arg = command_run_inner(
            &state,
            CommandRunRequest {
                workspace_id: Some(workspace.workspace_id),
                command: serde_json::json!(["git", "status", "--porcelain"]),
                local_tools: local_tools.clone(),
                ..CommandRunRequest::default()
            },
        )
        .await;
        assert_eq!(denied_arg.status, "tool_policy_denied");
        assert!(
            denied_arg
                .diagnostics
                .iter()
                .any(|line| line.contains("denied_args"))
        );

        let allowed = command_run_inner(
            &state,
            CommandRunRequest {
                workspace_id: Some(workspace.workspace_id),
                command: serde_json::json!(["git", "status"]),
                local_tools,
                ..CommandRunRequest::default()
            },
        )
        .await;
        assert_ne!(allowed.status, "tool_policy_denied");
        assert_eq!(
            allowed.command,
            vec!["git".to_string(), "status".to_string()]
        );
        assert!(allowed.artifact.is_some());
    }

    #[tokio::test]
    async fn kubernetes_executor_job_plan_projects_launch_plan_to_backend_capacity_request() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = AppState {
            workspace_root: temp.path().to_path_buf(),
            supported_backends: vec![SandboxBackend::KubernetesJob],
            enable_live_git_worktrees: false,
            require_live_git_worktree_approval: true,
            approved_git_repo_roots: Vec::new(),
            enable_local_command_execution: false,
            require_command_approval: true,
            allowed_local_binaries: parse_allowed_local_binaries(),
            command_timeout_seconds: 600,
            command_max_output_bytes: 65_536,
            enable_kubernetes_provisioner: false,
        };
        let goal_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let launch_plan = SandboxLaunchPlan {
            goal_id,
            task_id,
            workspace_id: Uuid::new_v4(),
            backend: SandboxBackend::KubernetesJob,
            runtime_class: Some("gvisor".to_string()),
            image: Some("ghcr.io/example/jattg-agent-toolbox:test".to_string()),
            workspace_path: "/workspace".to_string(),
            artifact_manifest_path: "/workspace/artifacts/artifact-manifest.json".to_string(),
            checkpoint_manifest_path: "/workspace/checkpoints/checkpoint-manifest.json".to_string(),
            command: vec!["coat-executor".to_string(), "run".to_string()],
            environment: BTreeMap::from([(
                "RUNNER_REGISTRY_URL".to_string(),
                "http://runner-registry:9085".to_string(),
            )]),
            required_capabilities: vec![RunnerCapability::KubernetesJobSandbox],
            resources: SandboxResourcePlan {
                cpu_limit_millis: Some(1000),
                memory_limit_mb: Some(2048),
                pids_limit: Some(256),
                ephemeral_storage_mb: Some(4096),
            },
            security: SandboxSecurityPlan {
                read_only_rootfs: true,
                no_new_privileges: true,
                run_as_non_root: true,
                seccomp_profile: Some("RuntimeDefault".to_string()),
                apparmor_profile: None,
                drop_capabilities: vec!["ALL".to_string()],
            },
            network: SandboxNetworkPlan {
                access: NetworkAccess::Restricted,
                deny_by_default: true,
                egress_policy_ref: Some("allow-control-plane-egress".to_string()),
                ingress_policy_ref: None,
                network_policy_labels: BTreeMap::from([(
                    "jattg.dev/network-profile".to_string(),
                    "control-plane".to_string(),
                )]),
                allowed_internal_services: vec!["runner-registry".to_string()],
            },
            git_result: None,
            object_prefix: None,
            warnings: Vec::new(),
        };
        let request = KubernetesExecutorJobProvisionRequest {
            launch_plan,
            mode: KubernetesProvisionMode::PlanOnly,
            namespace: "jattg-sandboxes".to_string(),
            name: Some("Executor_Example".to_string()),
            image: None,
            service_account: Some("jattg-sandbox-task".to_string()),
            runtime_class: None,
            workspace_pvc: Some("sandbox-workspaces".to_string()),
            workspace_mount_path: "/workspace".to_string(),
            field_manager: "coat-sandbox-runner".to_string(),
            active_deadline_seconds: Some(900),
            ttl_seconds_after_finished: Some(300),
            backoff_limit: 0,
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
        };

        let response = kubernetes_executor_job_provision_inner(&state, request)
            .await
            .expect("plan-only provisioning response");

        assert_eq!(response.status, KubernetesProvisionStatus::Planned);
        assert_eq!(response.namespace, "jattg-sandboxes");
        assert_eq!(response.objects.len(), 2);
        let items = response.manifest["items"]
            .as_array()
            .expect("manifest items");
        assert_eq!(items[0]["kind"], "ConfigMap");
        assert_eq!(items[1]["kind"], "Job");
        assert_eq!(items[1]["metadata"]["name"], "executor-example");
        assert_eq!(
            items[1]["spec"]["template"]["spec"]["runtimeClassName"],
            "gvisor"
        );
        assert_eq!(
            items[1]["spec"]["template"]["spec"]["volumes"][1]["persistentVolumeClaim"]["claimName"],
            "sandbox-workspaces"
        );
        assert_eq!(
            items[1]["metadata"]["labels"]["jattg.dev/network-profile"],
            "control-plane"
        );
        assert!(
            response
                .diagnostics
                .iter()
                .any(|line| line.contains("not from worker-authored manifest snippets"))
        );
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let output = StdCommand::new("git")
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
        let output = StdCommand::new("git")
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
