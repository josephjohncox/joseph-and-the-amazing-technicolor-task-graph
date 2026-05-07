//! `coat` operator CLI.
//!
//! Purpose: give operators a scriptable control surface for initializing the
//! project, authoring plans and goals, submitting/cancelling/steering work,
//! inspecting projections, rendering deployment assets, and exercising local
//! smoke paths.
//!
//! Architecture references:
//! - `README.md` for common commands.
//! - `docs/operations/local-dev.md` for local smoke workflows.
//! - `docs/operations/goal-authoring.md` for structured goal authoring.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use coat_domain::{
    BranchRequest, BranchSelectionRequest, ChildTaskRequest, ControlLoopMode, EventSource,
    ExternalEvent, GoalAuthoringGuidance, GoalPlan, GoalRecord, GoalSpec, GraphColorRef,
    HumanApproval, MemoryContextRequest, MemoryJoinRequest, MemoryRepairRequest,
    MemorySearchRequest, MemoryWriteRequest, NotificationRequest, PlanCompileRequest,
    PlanDraftRequest, PlanQuestion, PlanQuestionStatus, PlanRevisionRequest, PlanningMode,
    RestartRequest, ReviewDoctrine, ReviewDoctrinePreset, RunnerDispatchRequest,
    RunnerRegistration, StandardReviewCheck, SteeringDirective, SteeringDirectiveKind, SubgoalSpec,
    TaskPriority, TaskPurpose, TaskPurposeKind, TaskQuery, TaskStatus, TriggeredGoalRequest,
    WorkerKind,
};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "coat")]
#[command(about = "COAT operator CLI for Joseph and the Amazing Technicolor Task Graph")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Init(InitArgs),
    Plan(PlanCommand),
    Goal(GoalCommand),
    Event(EventCommand),
    Runner(RunnerCommand),
    Memory(MemoryCommand),
    Approve(ApproveArgs),
    Notify(NotifyArgs),
    Store(StoreCommand),
    Sandbox(SandboxCommand),
    Release(ReleaseCommand),
    FollowUps(FollowUpsArgs),
    Compose(ComposeCommand),
    K8s(K8sCommand),
    Restate(RestateCommand),
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

#[derive(Debug, Args)]
struct PlanCommand {
    #[command(subcommand)]
    command: PlanSubcommand,
}

#[derive(Debug, Subcommand)]
enum PlanSubcommand {
    Draft(PlanDraftArgs),
    List(PlanListArgs),
    Show(PlanShowArgs),
    Revise(PlanReviseArgs),
    Compile(PlanCompileArgs),
}

#[derive(Debug, Args)]
struct PlanStoreArgs {
    #[arg(
        long,
        env = "COAT_GOAL_STORE_URL",
        default_value = "http://localhost:9088"
    )]
    goal_store_url: String,
}

#[derive(Debug, Args)]
struct PlanDraftArgs {
    #[command(flatten)]
    store: PlanStoreArgs,
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    objective: Option<String>,
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long, default_value = "interactive")]
    mode: String,
    #[arg(long)]
    author: Option<String>,
    #[arg(long)]
    summary: Option<String>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    emit_only: bool,
    #[arg(long)]
    acceptance_evidence: Vec<String>,
    #[arg(long)]
    constraint: Vec<String>,
    #[arg(long)]
    out_of_scope: Vec<String>,
    #[arg(long)]
    assumption: Vec<String>,
    #[arg(long)]
    open_question: Vec<String>,
    #[arg(long)]
    subgoal: Vec<String>,
    #[arg(long)]
    initial_task: Vec<String>,
}

#[derive(Debug, Args)]
struct PlanListArgs {
    #[command(flatten)]
    store: PlanStoreArgs,
    #[arg(long)]
    status: Vec<String>,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Args)]
struct PlanShowArgs {
    #[command(flatten)]
    store: PlanStoreArgs,
    #[arg(long)]
    plan_id: Uuid,
}

#[derive(Debug, Args)]
struct PlanReviseArgs {
    #[command(flatten)]
    store: PlanStoreArgs,
    #[arg(long)]
    plan_id: Uuid,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct PlanCompileArgs {
    #[command(flatten)]
    store: PlanStoreArgs,
    #[arg(long)]
    plan_id: Uuid,
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    strict_review: bool,
    #[arg(long)]
    human_steered: bool,
    #[arg(long)]
    enable_branching: bool,
}

#[derive(Debug, Args)]
struct GoalCommand {
    #[command(subcommand)]
    command: GoalSubcommand,
}

#[derive(Debug, Args)]
struct RunnerCommand {
    #[command(subcommand)]
    command: RunnerSubcommand,
}

#[derive(Debug, Args)]
struct EventCommand {
    #[command(subcommand)]
    command: EventSubcommand,
}

#[derive(Debug, Args)]
struct SandboxCommand {
    #[command(subcommand)]
    command: SandboxSubcommand,
}

#[derive(Debug, Subcommand)]
enum SandboxSubcommand {
    Plan(SandboxCreateArgs),
    Create(SandboxCreateArgs),
    Snapshot(SandboxWorkspaceArgs),
    Cleanup(SandboxWorkspaceArgs),
}

#[derive(Debug, Args)]
struct SandboxCreateArgs {
    #[arg(
        long,
        env = "COAT_SANDBOX_RUNNER_URL",
        default_value = "http://localhost:9083"
    )]
    sandbox_runner_url: String,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct SandboxWorkspaceArgs {
    #[arg(
        long,
        env = "COAT_SANDBOX_RUNNER_URL",
        default_value = "http://localhost:9083"
    )]
    sandbox_runner_url: String,
    #[arg(long)]
    workspace_id: Uuid,
}

#[derive(Debug, Subcommand)]
enum EventSubcommand {
    Sources(EventGatewayUrlArgs),
    Register(EventFileArgs),
    Ingest(EventIngestArgs),
    Emit(EventEmitArgs),
    Trigger(EventFileArgs),
    List(EventGatewayUrlArgs),
    Triggers(EventGatewayUrlArgs),
}

#[derive(Debug, Args)]
struct EventGatewayUrlArgs {
    #[arg(
        long,
        env = "COAT_EVENT_GATEWAY_URL",
        default_value = "http://localhost:9089"
    )]
    event_gateway_url: String,
    #[arg(long, env = "COAT_EVENT_GATEWAY_TOKEN")]
    token: Option<String>,
}

#[derive(Debug, Args)]
struct EventFileArgs {
    #[arg(
        long,
        env = "COAT_EVENT_GATEWAY_URL",
        default_value = "http://localhost:9089"
    )]
    event_gateway_url: String,
    #[arg(long, env = "COAT_EVENT_GATEWAY_TOKEN")]
    token: Option<String>,
    #[arg(long)]
    file: PathBuf,
    #[arg(long, env = "COAT_EVENT_APPROVAL_ID")]
    approval_id: Option<String>,
}

#[derive(Debug, Args)]
struct EventIngestArgs {
    #[arg(
        long,
        env = "COAT_EVENT_GATEWAY_URL",
        default_value = "http://localhost:9089"
    )]
    event_gateway_url: String,
    #[arg(long, env = "COAT_EVENT_GATEWAY_TOKEN")]
    token: Option<String>,
    #[arg(long)]
    file: PathBuf,
    #[arg(long)]
    route: bool,
}

#[derive(Debug, Args)]
struct EventEmitArgs {
    #[arg(
        long,
        env = "COAT_EVENT_GATEWAY_URL",
        default_value = "http://localhost:9089"
    )]
    event_gateway_url: String,
    #[arg(long, env = "COAT_EVENT_GATEWAY_TOKEN")]
    token: Option<String>,
    #[arg(long)]
    source_id: String,
    #[arg(long)]
    file: PathBuf,
    #[arg(long)]
    no_route: bool,
}

#[derive(Debug, Subcommand)]
enum RunnerSubcommand {
    List(RunnerListArgs),
    Status(RunnerListArgs),
    Register(RunnerRegisterArgs),
    Dispatch(RunnerDispatchArgs),
}

#[derive(Debug, Args)]
struct MemoryCommand {
    #[command(subcommand)]
    command: MemorySubcommand,
}

#[derive(Debug, Subcommand)]
enum MemorySubcommand {
    Write(MemoryWriteArgs),
    Search(MemorySearchArgs),
    Context(MemoryContextArgs),
    Join(MemoryJoinArgs),
    Repair(MemoryRepairArgs),
    Events(MemoryEventsArgs),
}

#[derive(Debug, Args)]
struct MemoryWriteArgs {
    #[arg(
        long,
        env = "COAT_MEMORY_GATEWAY_URL",
        default_value = "http://localhost:9087"
    )]
    memory_gateway_url: String,
    #[arg(long, env = "COAT_MEMORY_GATEWAY_TOKEN")]
    token: Option<String>,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct MemorySearchArgs {
    #[arg(
        long,
        env = "COAT_MEMORY_GATEWAY_URL",
        default_value = "http://localhost:9087"
    )]
    memory_gateway_url: String,
    #[arg(long, env = "COAT_MEMORY_GATEWAY_TOKEN")]
    token: Option<String>,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct MemoryContextArgs {
    #[arg(
        long,
        env = "COAT_MEMORY_GATEWAY_URL",
        default_value = "http://localhost:9087"
    )]
    memory_gateway_url: String,
    #[arg(long, env = "COAT_MEMORY_GATEWAY_TOKEN")]
    token: Option<String>,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct MemoryJoinArgs {
    #[arg(
        long,
        env = "COAT_MEMORY_GATEWAY_URL",
        default_value = "http://localhost:9087"
    )]
    memory_gateway_url: String,
    #[arg(long, env = "COAT_MEMORY_GATEWAY_TOKEN")]
    token: Option<String>,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct MemoryRepairArgs {
    #[arg(
        long,
        env = "COAT_MEMORY_GATEWAY_URL",
        default_value = "http://localhost:9087"
    )]
    memory_gateway_url: String,
    #[arg(long, env = "COAT_MEMORY_GATEWAY_TOKEN")]
    token: Option<String>,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct MemoryEventsArgs {
    #[arg(
        long,
        env = "COAT_MEMORY_GATEWAY_URL",
        default_value = "http://localhost:9087"
    )]
    memory_gateway_url: String,
    #[arg(long, env = "COAT_MEMORY_GATEWAY_TOKEN")]
    token: Option<String>,
    #[arg(long)]
    goal_id: Uuid,
}

#[derive(Debug, Args)]
struct RunnerListArgs {
    #[arg(
        long,
        env = "COAT_RUNNER_REGISTRY",
        default_value = "http://localhost:9085"
    )]
    registry_url: String,
}

#[derive(Debug, Args)]
struct RunnerRegisterArgs {
    #[arg(
        long,
        env = "COAT_RUNNER_REGISTRY",
        default_value = "http://localhost:9085"
    )]
    registry_url: String,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct RunnerDispatchArgs {
    #[arg(
        long,
        env = "COAT_RUNNER_REGISTRY",
        default_value = "http://localhost:9085"
    )]
    registry_url: String,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Subcommand)]
enum GoalSubcommand {
    Draft(DraftGoalArgs),
    List(GoalListArgs),
    Submit(SubmitGoalArgs),
    Status(GoalIdArgs),
    Progress(GoalIdArgs),
    Tasks(GoalTasksArgs),
    Lint(GoalLintArgs),
    Steer(SteerGoalArgs),
    SteerStandard(SteerStandardGoalArgs),
    ReviewChecks,
    Restart(RestartGoalArgs),
    Branch(BranchGoalArgs),
    SelectBranch(SelectBranchArgs),
    Cancel(CancelGoalArgs),
}

#[derive(Debug, Args)]
struct DraftGoalArgs {
    #[arg(long)]
    title: String,
    #[arg(long)]
    objective: String,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    strict_review: bool,
    #[arg(long)]
    human_steered: bool,
    #[arg(long)]
    enable_branching: bool,
    #[arg(long)]
    plan_summary: Option<String>,
    #[arg(long)]
    acceptance_evidence: Vec<String>,
    #[arg(long)]
    constraint: Vec<String>,
    #[arg(long)]
    out_of_scope: Vec<String>,
    #[arg(long)]
    assumption: Vec<String>,
    #[arg(long)]
    open_question: Vec<String>,
    #[arg(long)]
    review_preset: Vec<String>,
    #[arg(long)]
    subgoal: Vec<String>,
    #[arg(long)]
    initial_task: Vec<String>,
}

#[derive(Debug, Args)]
struct SubmitGoalArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    objective: Option<String>,
}

#[derive(Debug, Args)]
struct GoalListArgs {
    #[arg(
        long,
        env = "COAT_GOAL_STORE_URL",
        default_value = "http://localhost:9088"
    )]
    goal_store_url: String,
    #[arg(long)]
    status: Vec<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Args)]
struct GoalLintArgs {
    #[arg(long)]
    file: PathBuf,
    #[arg(long)]
    strict: bool,
}

#[derive(Debug, Args)]
struct GoalIdArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
}

#[derive(Debug, Args)]
struct GoalSelectorArgs {
    #[arg(long, env = "COAT_GOAL_ID")]
    goal_id: Option<Uuid>,
    #[arg(long)]
    latest: bool,
    #[arg(
        long,
        env = "COAT_GOAL_STORE_URL",
        default_value = "http://localhost:9088"
    )]
    goal_store_url: String,
}

#[derive(Debug, Args)]
struct GoalTasksArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    subgoal_id: Option<String>,
    #[arg(long)]
    status: Vec<String>,
    #[arg(long)]
    role: Vec<String>,
    #[arg(long)]
    purpose: Vec<String>,
    #[arg(long)]
    color: Vec<String>,
    #[arg(long)]
    tag: Vec<String>,
    #[arg(long)]
    runnable: bool,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Args)]
struct SteerGoalArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct SteerStandardGoalArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
    #[arg(long)]
    task_id: Option<Uuid>,
    #[arg(long)]
    check: String,
    #[arg(long)]
    topic: Option<String>,
    #[arg(long, default_value = "operator requested a standard review check")]
    reason: String,
    #[arg(long)]
    message: Option<String>,
    #[arg(long)]
    operator: Option<String>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    emit_only: bool,
}

#[derive(Debug, Args)]
struct RestartGoalArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct BranchGoalArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct SelectBranchArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct CancelGoalArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
    #[arg(long, default_value = "cancelled by operator")]
    reason: String,
}

#[derive(Debug, Args)]
struct ApproveArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
    #[arg(long)]
    approval_id: Uuid,
    #[arg(long, default_value_t = true)]
    approved: bool,
    #[arg(long)]
    note: Option<String>,
}

#[derive(Debug, Args)]
struct NotifyArgs {
    #[arg(
        long,
        env = "COAT_NOTIFIER_URL",
        default_value = "http://localhost:9086"
    )]
    notifier_url: String,
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    threads: bool,
    #[arg(long)]
    thread_key: Option<String>,
}

#[derive(Debug, Args)]
struct StoreCommand {
    #[command(subcommand)]
    command: StoreSubcommand,
}

#[derive(Debug, Subcommand)]
enum StoreSubcommand {
    Policy(StoreUrlArgs),
    Goals(StoreUrlArgs),
    Plans(StoreUrlArgs),
    AllTasks(StoreUrlArgs),
    Approvals(StoreApprovalsArgs),
    Goal(StoreGoalArgs),
    Tasks(StoreGoalArgs),
    Events(StoreGoalArgs),
    Artifacts(StoreGoalArgs),
    Checkpoints(StoreGoalArgs),
    RecordArtifacts(StoreRecordArtifactsArgs),
    GoalApprovals(StoreGoalArgs),
}

#[derive(Debug, Args)]
struct StoreUrlArgs {
    #[arg(
        long,
        env = "COAT_GOAL_STORE_URL",
        default_value = "http://localhost:9088"
    )]
    goal_store_url: String,
}

#[derive(Debug, Args)]
struct StoreGoalArgs {
    #[arg(
        long,
        env = "COAT_GOAL_STORE_URL",
        default_value = "http://localhost:9088"
    )]
    goal_store_url: String,
    #[arg(long)]
    goal_id: Uuid,
}

#[derive(Debug, Args)]
struct StoreApprovalsArgs {
    #[arg(
        long,
        env = "COAT_GOAL_STORE_URL",
        default_value = "http://localhost:9088"
    )]
    goal_store_url: String,
    #[arg(long)]
    goal_id: Option<Uuid>,
    #[arg(long)]
    status: Vec<String>,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Args)]
struct StoreRecordArtifactsArgs {
    #[arg(
        long,
        env = "COAT_GOAL_STORE_URL",
        default_value = "http://localhost:9088"
    )]
    goal_store_url: String,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct ReleaseCommand {
    #[command(subcommand)]
    command: ReleaseSubcommand,
}

#[derive(Debug, Subcommand)]
enum ReleaseSubcommand {
    Plan(ReleasePlanArgs),
    Bump(ReleaseBumpArgs),
    Cut(ReleaseCutArgs),
}

#[derive(Debug, Args)]
struct ReleasePlanArgs {
    #[arg(long)]
    version: String,
    #[arg(long)]
    chart_version: Option<String>,
    #[arg(long)]
    app_version: Option<String>,
    #[arg(long)]
    tag_suffix: Option<String>,
}

#[derive(Debug, Args)]
struct ReleaseBumpArgs {
    #[arg(long)]
    version: String,
    #[arg(long)]
    chart_version: Option<String>,
    #[arg(long)]
    app_version: Option<String>,
    #[arg(long, default_value = "Cargo.toml")]
    cargo_toml: PathBuf,
    #[arg(long, default_value = "infra/helm/jattg/Chart.yaml")]
    chart_yaml: PathBuf,
    #[arg(long)]
    allow_dirty: bool,
}

#[derive(Debug, Args)]
struct ReleaseCutArgs {
    #[arg(long)]
    version: String,
    #[arg(long)]
    chart_version: Option<String>,
    #[arg(long)]
    app_version: Option<String>,
    #[arg(long)]
    tag_suffix: Option<String>,
    #[arg(long, default_value = "Cargo.toml")]
    cargo_toml: PathBuf,
    #[arg(long, default_value = "Cargo.lock")]
    cargo_lock: PathBuf,
    #[arg(long, default_value = "infra/helm/jattg/Chart.yaml")]
    chart_yaml: PathBuf,
    #[arg(long, default_value = "origin")]
    remote: String,
    #[arg(long)]
    allow_dirty: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    no_verify: bool,
    #[arg(long)]
    push: bool,
}

#[derive(Debug, Args)]
struct FollowUpsArgs {
    #[arg(long, default_value = "docs/exec-plans/active")]
    dir: PathBuf,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    include_empty: bool,
}

#[derive(Debug, Args)]
struct ComposeCommand {
    #[command(subcommand)]
    command: ComposeSubcommand,
}

#[derive(Debug, Subcommand)]
enum ComposeSubcommand {
    Up(ComposeUpArgs),
    Down(ComposeDownArgs),
}

#[derive(Debug, Args)]
struct ComposeUpArgs {
    #[arg(long)]
    restate_cloud: bool,
    #[arg(long, default_value = "infra/compose/restate-cloud.env")]
    env_file: PathBuf,
    #[arg(long)]
    profile: Vec<String>,
}

#[derive(Debug, Args)]
struct ComposeDownArgs {
    #[arg(long)]
    restate_cloud: bool,
    #[arg(long, default_value = "infra/compose/restate-cloud.env")]
    env_file: PathBuf,
}

#[derive(Debug, Args)]
struct K8sCommand {
    #[command(subcommand)]
    command: K8sSubcommand,
}

#[derive(Debug, Subcommand)]
enum K8sSubcommand {
    Render(RenderArgs),
    EphemeralJobs(EphemeralJobsCommand),
}

#[derive(Debug, Args)]
struct EphemeralJobsCommand {
    #[command(subcommand)]
    command: EphemeralJobsSubcommand,
}

#[derive(Debug, Subcommand)]
enum EphemeralJobsSubcommand {
    Render(EphemeralJobsRenderArgs),
}

#[derive(Debug, Args)]
struct RestateCommand {
    #[command(subcommand)]
    command: RestateSubcommand,
}

#[derive(Debug, Subcommand)]
enum RestateSubcommand {
    CloudEnv(RestateCloudEnvArgs),
    TunnelDocker(RestateTunnelDockerArgs),
    RegisterCloud(RestateRegisterCloudArgs),
}

#[derive(Debug, Args)]
struct RenderArgs {
    #[arg(long, default_value = "infra/k8s/rendered.yaml")]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct EphemeralJobsRenderArgs {
    #[arg(
        long,
        default_value = "infra/k8s/examples/ephemeral-agent-runner-jobs.yaml"
    )]
    source: PathBuf,
    #[arg(
        long,
        default_value = "infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml"
    )]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct RestateCloudEnvArgs {
    #[arg(long, env = "RESTATE_TUNNEL_NAME", default_value = "jattg-personal")]
    tunnel_name: String,
    #[arg(long, env = "RESTATE_CLOUD_REGION", default_value = "us")]
    region: String,
    #[arg(long, env = "RESTATE_ENVIRONMENT_ID")]
    environment_id: Option<String>,
    #[arg(long, env = "RESTATE_SIGNING_PUBLIC_KEY")]
    signing_public_key: Option<String>,
    #[arg(long, default_value = "http://localhost:18080")]
    ingress_url: String,
    #[arg(long, default_value = "http://localhost:19070")]
    admin_url: String,
    #[arg(long, default_value = "http://localhost:9080")]
    coordinator_url: String,
}

#[derive(Debug, Args)]
struct RestateTunnelDockerArgs {
    #[arg(long, env = "RESTATE_TUNNEL_NAME", default_value = "jattg-personal")]
    tunnel_name: String,
    #[arg(long, env = "RESTATE_CLOUD_REGION", default_value = "us")]
    region: String,
    #[arg(long, env = "RESTATE_ENVIRONMENT_ID", default_value = "env_...")]
    environment_id: String,
    #[arg(
        long,
        env = "RESTATE_SIGNING_PUBLIC_KEY",
        default_value = "publickeyv1_..."
    )]
    signing_public_key: String,
    #[arg(
        long,
        default_value = "ghcr.io/restatedev/restate-cloud-tunnel-client:latest"
    )]
    image: String,
    #[arg(long, default_value_t = 18080)]
    ingress_port: u16,
    #[arg(long, default_value_t = 19070)]
    admin_port: u16,
    #[arg(long, default_value_t = 19090)]
    health_port: u16,
}

#[derive(Debug, Args)]
struct RestateRegisterCloudArgs {
    #[arg(long, env = "RESTATE_TUNNEL_NAME", default_value = "jattg-personal")]
    tunnel_name: String,
    #[arg(long, default_value = "http://localhost:9080")]
    service_url: String,
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Commands::Init(args) => init(args),
        Commands::Plan(args) => plan(args).await,
        Commands::Goal(args) => goal(args).await,
        Commands::Event(args) => event(args).await,
        Commands::Runner(args) => runner(args).await,
        Commands::Memory(args) => memory(args).await,
        Commands::Approve(args) => approve(args).await,
        Commands::Notify(args) => notify(args).await,
        Commands::Store(args) => store(args).await,
        Commands::Sandbox(args) => sandbox(args).await,
        Commands::Release(args) => release(args),
        Commands::FollowUps(args) => follow_ups(args),
        Commands::Compose(args) => compose(args),
        Commands::K8s(args) => k8s(args),
        Commands::Restate(args) => restate(args),
    }
}

fn follow_ups(args: FollowUpsArgs) -> anyhow::Result<()> {
    let report = follow_up_report(&args.dir, args.include_empty)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let plans = report["plans"]
        .as_array()
        .context("follow-up report plans should be an array")?;
    if plans.is_empty() {
        println!("No active plan follow-ups found.");
        return Ok(());
    }

    for plan in plans {
        let path = plan["path"].as_str().unwrap_or("<unknown>");
        let title = plan["title"].as_str().unwrap_or("<untitled>");
        println!("{path} - {title}");
        for follow_up in plan["follow_ups"].as_array().into_iter().flatten() {
            if let Some(text) = follow_up.as_str() {
                println!("  - {text}");
            }
        }
    }
    Ok(())
}

fn follow_up_report(plan_dir: &Path, include_empty: bool) -> anyhow::Result<serde_json::Value> {
    let mut plan_paths = Vec::new();
    for entry in
        fs::read_dir(plan_dir).with_context(|| format!("reading {}", plan_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            plan_paths.push(path);
        }
    }
    plan_paths.sort();

    let mut plans = Vec::new();
    let mut follow_up_count = 0usize;
    for path in plan_paths {
        let contents =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let title = contents
            .lines()
            .find_map(|line| line.strip_prefix("# ").map(ToOwned::to_owned))
            .unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
        let follow_ups = extract_follow_ups(&contents);
        follow_up_count += follow_ups.len();
        if include_empty || !follow_ups.is_empty() {
            plans.push(serde_json::json!({
                "path": path.display().to_string(),
                "title": title,
                "follow_ups": follow_ups
            }));
        }
    }

    Ok(serde_json::json!({
        "plan_dir": plan_dir.display().to_string(),
        "plan_count": plans.len(),
        "follow_up_count": follow_up_count,
        "plans": plans
    }))
}

fn extract_follow_ups(contents: &str) -> Vec<String> {
    let mut in_follow_ups = false;
    let mut follow_ups = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed == "## Follow-Ups" {
            in_follow_ups = true;
            continue;
        }
        if in_follow_ups && trimmed.starts_with("## ") {
            break;
        }
        if in_follow_ups {
            if let Some(item) = trimmed.strip_prefix("- ") {
                follow_ups.push(item.to_string());
            }
        }
    }
    follow_ups
}

fn release(args: ReleaseCommand) -> anyhow::Result<()> {
    match args.command {
        ReleaseSubcommand::Plan(args) => {
            let plan = release_plan_json(
                &args.version,
                args.chart_version.as_deref(),
                args.app_version.as_deref(),
                args.tag_suffix.as_deref(),
            )?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
            Ok(())
        }
        ReleaseSubcommand::Bump(args) => {
            let plan = release_plan_json(
                &args.version,
                args.chart_version.as_deref(),
                args.app_version.as_deref(),
                None,
            )?;
            if !args.allow_dirty {
                ensure_clean_git_worktree()?;
            }
            bump_release_versions(
                &args.cargo_toml,
                &args.chart_yaml,
                plan["app_version"].as_str().expect("app version"),
                plan["chart_version"].as_str().expect("chart version"),
            )?;
            refresh_cargo_lock(&args.cargo_toml)?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
            Ok(())
        }
        ReleaseSubcommand::Cut(args) => release_cut(args),
    }
}

fn release_cut(args: ReleaseCutArgs) -> anyhow::Result<()> {
    let plan = release_plan_json(
        &args.version,
        args.chart_version.as_deref(),
        args.app_version.as_deref(),
        args.tag_suffix.as_deref(),
    )?;
    let app_version = required_plan_str(&plan, "app_version")?;
    let chart_version = required_plan_str(&plan, "chart_version")?;
    let binary_tag = required_plan_str(&plan, "binary_tag")?;
    let chart_tag = required_plan_str(&plan, "chart_tag")?;
    let release_commit = format!("chore(release): {binary_tag}");
    let tag_suffix = plan
        .get("tag_suffix")
        .and_then(serde_json::Value::as_str)
        .filter(|suffix| !suffix.is_empty());

    if args.dry_run {
        let mut dry_run_plan = plan.clone();
        if let Some(object) = dry_run_plan.as_object_mut() {
            object.insert("release_commit".to_string(), release_commit.into());
            object.insert(
                "release_commit_required".to_string(),
                tag_suffix.is_none().into(),
            );
            object.insert("remote".to_string(), args.remote.into());
            object.insert("push".to_string(), args.push.into());
            object.insert("dry_run".to_string(), true.into());
        }
        println!("{}", serde_json::to_string_pretty(&dry_run_plan)?);
        return Ok(());
    }

    if !args.allow_dirty {
        ensure_clean_git_worktree()?;
    }
    ensure_git_tag_absent(binary_tag)?;
    ensure_git_tag_absent(chart_tag)?;

    bump_release_versions(
        &args.cargo_toml,
        &args.chart_yaml,
        app_version,
        chart_version,
    )?;
    refresh_cargo_lock(&args.cargo_toml)?;
    git_add_paths(&[
        args.cargo_toml.as_path(),
        args.cargo_lock.as_path(),
        args.chart_yaml.as_path(),
    ])?;
    let release_paths = [
        args.cargo_toml.as_path(),
        args.cargo_lock.as_path(),
        args.chart_yaml.as_path(),
    ];
    let release_changes = staged_release_changes_exist(&release_paths)?;
    if release_changes {
        git_commit(&release_commit, args.no_verify)?;
    } else if tag_suffix.is_none() {
        bail!("release bump produced no staged changes");
    }
    git_tag(binary_tag, &format!("COAT {app_version} binaries"))?;
    git_tag(chart_tag, &format!("JATTG Helm chart {chart_version}"))?;

    if args.push {
        git_push(&args.remote, &["HEAD"])?;
        git_push(&args.remote, &[binary_tag, chart_tag])?;
    }

    let mut cut_result = plan.clone();
    if let Some(object) = cut_result.as_object_mut() {
        object.insert(
            "release_commit".to_string(),
            if release_changes {
                release_commit.into()
            } else {
                serde_json::Value::Null
            },
        );
        object.insert("release_commit_created".to_string(), release_changes.into());
        object.insert("remote".to_string(), args.remote.into());
        object.insert("pushed".to_string(), args.push.into());
    }
    println!("{}", serde_json::to_string_pretty(&cut_result)?);
    Ok(())
}

fn release_plan_json(
    version: &str,
    chart_version: Option<&str>,
    app_version: Option<&str>,
    tag_suffix: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let app_version = app_version.unwrap_or(version).trim_start_matches('v');
    let chart_version = chart_version.unwrap_or(app_version).trim_start_matches('v');
    let version = version.trim_start_matches('v');
    let tag_suffix = normalize_tag_suffix(tag_suffix)?;
    validate_semver(version).context("invalid release version")?;
    validate_semver(app_version).context("invalid app version")?;
    validate_semver(chart_version).context("invalid chart version")?;
    let mut cut_command = format!("coat release cut --version {version}");
    if app_version != version {
        cut_command.push_str(&format!(" --app-version {app_version}"));
    }
    if chart_version != version {
        cut_command.push_str(&format!(" --chart-version {chart_version}"));
    }
    if let Some(suffix) = tag_suffix.as_deref() {
        cut_command.push_str(&format!(" --tag-suffix {suffix}"));
    }
    let binary_tag = release_tag("v", version, tag_suffix.as_deref());
    let chart_tag = release_tag("chart-v", chart_version, tag_suffix.as_deref());
    let image_version_tag = version_with_tag_suffix(version, tag_suffix.as_deref());
    let tag_suffix_value = tag_suffix.clone().unwrap_or_default();
    let container_registry = "ghcr.io/josephjohncox/joseph-and-the-amazing-technicolor-task-graph";

    Ok(serde_json::json!({
        "version": version,
        "app_version": app_version,
        "chart_version": chart_version,
        "tag_suffix": tag_suffix_value,
        "binary_tag": binary_tag,
        "chart_tag": chart_tag,
        "bump_files": [
            "Cargo.toml",
            "Cargo.lock",
            "infra/helm/jattg/Chart.yaml"
        ],
        "binary_workflow": ".github/workflows/release-binaries.yml",
        "helm_workflow": ".github/workflows/release-helm.yml",
        "binary_assets": [
            format!("jattg-binaries-{image_version_tag}-x86_64-unknown-linux-gnu.tar.gz"),
            format!("jattg-binaries-{image_version_tag}-aarch64-unknown-linux-gnu.tar.gz"),
            format!("jattg-binaries-{image_version_tag}-aarch64-apple-darwin.tar.gz")
        ],
        "container_registry": container_registry,
        "container_images": [
            format!("{container_registry}/jattg-coordinator:v{image_version_tag}"),
            format!("{container_registry}/jattg-event-gateway:v{image_version_tag}"),
            format!("{container_registry}/jattg-goal-store:v{image_version_tag}"),
            format!("{container_registry}/jattg-memory-gateway:v{image_version_tag}"),
            format!("{container_registry}/jattg-notifier:v{image_version_tag}"),
            format!("{container_registry}/jattg-runner-registry:v{image_version_tag}"),
            format!("{container_registry}/jattg-sandbox-runner:v{image_version_tag}"),
            format!("{container_registry}/jattg-tool-registry:v{image_version_tag}"),
            format!("{container_registry}/jattg-validator:v{image_version_tag}"),
            format!("{container_registry}/jattg-agent-toolbox:v{image_version_tag}"),
            format!("{container_registry}/jattg-control-web:v{image_version_tag}"),
            format!("{container_registry}/jattg-codex-runner:v{image_version_tag}"),
            format!("{container_registry}/jattg-claude-code-runner:v{image_version_tag}"),
            format!("{container_registry}/jattg-model-provider-runner:v{image_version_tag}"),
            format!("{container_registry}/jattg-staff-engineer-runner:v{image_version_tag}")
        ],
        "container_image_tags": [
            format!("v{image_version_tag}"),
            image_version_tag,
            "latest"
        ],
        "helm_assets": [
            format!("jattg-{}.tgz", version_with_tag_suffix(chart_version, tag_suffix.as_deref())),
            "index.yaml"
        ],
        "publish_steps": [
            cut_command,
            format!("git tag {binary_tag}"),
            format!("git tag {chart_tag}"),
            format!("git push origin {binary_tag} {chart_tag}")
        ]
    }))
}

fn normalize_tag_suffix(tag_suffix: Option<&str>) -> anyhow::Result<Option<String>> {
    let Some(raw) = tag_suffix else {
        return Ok(None);
    };
    let suffix = raw
        .trim()
        .trim_start_matches(|c| c == '-' || c == '_' || c == '.');
    if suffix.is_empty() {
        bail!("tag suffix must not be empty");
    }
    if !suffix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        bail!("tag suffix may only contain ASCII letters, digits, '.', '-', or '_'");
    }
    Ok(Some(suffix.to_string()))
}

fn version_with_tag_suffix(version: &str, tag_suffix: Option<&str>) -> String {
    match tag_suffix {
        Some(suffix) => format!("{version}-{suffix}"),
        None => version.to_string(),
    }
}

fn release_tag(prefix: &str, version: &str, tag_suffix: Option<&str>) -> String {
    format!("{prefix}{}", version_with_tag_suffix(version, tag_suffix))
}

fn required_plan_str<'a>(plan: &'a serde_json::Value, key: &str) -> anyhow::Result<&'a str> {
    plan.get(key)
        .and_then(serde_json::Value::as_str)
        .with_context(|| format!("release plan missing string field {key}"))
}

fn validate_semver(version: &str) -> anyhow::Result<()> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()))
    {
        bail!("expected MAJOR.MINOR.PATCH, got {version}");
    }
    Ok(())
}

fn ensure_clean_git_worktree() -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .context("checking git status")?;
    if !output.status.success() {
        bail!("git status failed");
    }
    if !output.stdout.is_empty() {
        bail!("worktree is dirty; pass --allow-dirty to bump release files anyway");
    }
    Ok(())
}

fn ensure_git_tag_absent(tag: &str) -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(["rev-parse", "--quiet", "--verify"])
        .arg(format!("refs/tags/{tag}"))
        .output()
        .with_context(|| format!("checking whether tag {tag} already exists"))?;
    if output.status.success() {
        bail!("release tag already exists: {tag}");
    }
    Ok(())
}

fn git_add_paths(paths: &[&Path]) -> anyhow::Result<()> {
    let mut command = Command::new("git");
    command.arg("add").arg("--");
    for path in paths {
        command.arg(path);
    }
    run_command(command, "stage release version files")
}

fn staged_release_changes_exist(paths: &[&Path]) -> anyhow::Result<bool> {
    let mut command = Command::new("git");
    command.arg("diff").arg("--cached").arg("--quiet").arg("--");
    for path in paths {
        command.arg(path);
    }
    let status = command
        .status()
        .context("checking staged release version changes")?;
    Ok(!status.success())
}

fn git_commit(message: &str, no_verify: bool) -> anyhow::Result<()> {
    let mut command = Command::new("git");
    command.arg("commit");
    if no_verify {
        command.arg("--no-verify");
    }
    command.arg("-m").arg(message);
    run_command(command, "commit release version bump")
}

fn git_tag(tag: &str, message: &str) -> anyhow::Result<()> {
    let mut command = Command::new("git");
    command.arg("tag").arg("-a").arg(tag).arg("-m").arg(message);
    run_command(command, &format!("create release tag {tag}"))
}

fn git_push(remote: &str, refs: &[&str]) -> anyhow::Result<()> {
    let mut command = Command::new("git");
    command.arg("push").arg(remote);
    for git_ref in refs {
        command.arg(git_ref);
    }
    run_command(command, &format!("push release refs to {remote}"))
}

fn run_command(mut command: Command, description: &str) -> anyhow::Result<()> {
    let output = command.output().with_context(|| description.to_string())?;
    if !output.status.success() {
        bail!(
            "{description} failed: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    Ok(())
}

fn refresh_cargo_lock(cargo_toml_path: &Path) -> anyhow::Result<()> {
    let mut command = Command::new("cargo");
    command
        .arg("update")
        .arg("--workspace")
        .arg("--manifest-path")
        .arg(cargo_toml_path);
    run_command(command, "update Cargo.lock after release version bump")
}

fn bump_release_versions(
    cargo_toml_path: &PathBuf,
    chart_yaml_path: &PathBuf,
    app_version: &str,
    chart_version: &str,
) -> anyhow::Result<()> {
    let cargo_toml = fs::read_to_string(cargo_toml_path)
        .with_context(|| format!("reading {}", cargo_toml_path.display()))?;
    let cargo_toml = replace_toml_section_value(
        &cargo_toml,
        "workspace.package",
        "version",
        &format!("\"{app_version}\""),
    )?;
    fs::write(cargo_toml_path, cargo_toml)
        .with_context(|| format!("writing {}", cargo_toml_path.display()))?;

    let chart_yaml = fs::read_to_string(chart_yaml_path)
        .with_context(|| format!("reading {}", chart_yaml_path.display()))?;
    let chart_yaml = replace_yaml_root_value(&chart_yaml, "version", chart_version)?;
    let chart_yaml = replace_yaml_root_value(&chart_yaml, "appVersion", app_version)?;
    fs::write(chart_yaml_path, chart_yaml)
        .with_context(|| format!("writing {}", chart_yaml_path.display()))?;
    Ok(())
}

fn replace_toml_section_value(
    contents: &str,
    section: &str,
    key: &str,
    value: &str,
) -> anyhow::Result<String> {
    let section_header = format!("[{section}]");
    let mut in_section = false;
    let mut replaced = false;
    let mut lines = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed == section_header;
        }
        if in_section && trimmed.starts_with(&format!("{key} =")) {
            lines.push(format!("{key} = {value}"));
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !replaced {
        bail!("could not find {key} in TOML section {section}");
    }
    Ok(format!("{}\n", lines.join("\n")))
}

fn replace_yaml_root_value(contents: &str, key: &str, value: &str) -> anyhow::Result<String> {
    let mut replaced = false;
    let mut lines = Vec::new();
    for line in contents.lines() {
        if !line.starts_with(' ') && line.trim_start().starts_with(&format!("{key}:")) {
            lines.push(format!("{key}: {value}"));
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !replaced {
        bail!("could not find root YAML key {key}");
    }
    Ok(format!("{}\n", lines.join("\n")))
}

async fn store(args: StoreCommand) -> anyhow::Result<()> {
    match args.command {
        StoreSubcommand::Policy(args) => {
            get_url(
                &format!(
                    "{}/goal-store/policy",
                    args.goal_store_url.trim_end_matches('/')
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Goals(args) => {
            get_url(
                &format!(
                    "{}/goal-store/goals",
                    args.goal_store_url.trim_end_matches('/')
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Plans(args) => {
            get_url(
                &format!(
                    "{}/goal-store/plans",
                    args.goal_store_url.trim_end_matches('/')
                ),
                None,
            )
            .await
        }
        StoreSubcommand::AllTasks(args) => {
            get_url(
                &format!(
                    "{}/goal-store/tasks",
                    args.goal_store_url.trim_end_matches('/')
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Approvals(args) => {
            let mut params = Vec::new();
            if let Some(goal_id) = args.goal_id {
                params.push(format!("goal_id={goal_id}"));
            }
            for status in args.status {
                params.push(format!("status={status}"));
            }
            if let Some(limit) = args.limit {
                params.push(format!("limit={limit}"));
            }
            let query = if params.is_empty() {
                String::new()
            } else {
                format!("?{}", params.join("&"))
            };
            get_url(
                &format!(
                    "{}/goal-store/approvals{}",
                    args.goal_store_url.trim_end_matches('/'),
                    query
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Goal(args) => {
            get_url(
                &format!(
                    "{}/goal-store/goals/{}",
                    args.goal_store_url.trim_end_matches('/'),
                    args.goal_id
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Tasks(args) => {
            get_url(
                &format!(
                    "{}/goal-store/goals/{}/tasks",
                    args.goal_store_url.trim_end_matches('/'),
                    args.goal_id
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Events(args) => {
            get_url(
                &format!(
                    "{}/goal-store/goals/{}/events",
                    args.goal_store_url.trim_end_matches('/'),
                    args.goal_id
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Artifacts(args) => {
            get_url(
                &format!(
                    "{}/goal-store/goals/{}/artifacts",
                    args.goal_store_url.trim_end_matches('/'),
                    args.goal_id
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Checkpoints(args) => {
            get_url(
                &format!(
                    "{}/goal-store/goals/{}/checkpoints",
                    args.goal_store_url.trim_end_matches('/'),
                    args.goal_id
                ),
                None,
            )
            .await
        }
        StoreSubcommand::RecordArtifacts(args) => {
            let request: serde_json::Value = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/goal-store/artifacts",
                    args.goal_store_url.trim_end_matches('/')
                ),
                &request,
                None,
                None,
            )
            .await
        }
        StoreSubcommand::GoalApprovals(args) => {
            get_url(
                &format!(
                    "{}/goal-store/goals/{}/approvals",
                    args.goal_store_url.trim_end_matches('/'),
                    args.goal_id
                ),
                None,
            )
            .await
        }
    }
}

async fn plan(args: PlanCommand) -> anyhow::Result<()> {
    match args.command {
        PlanSubcommand::Draft(args) => {
            let request = plan_draft_request_from_args(&args)?;
            if args.emit_only || args.out.is_some() {
                return write_json_or_stdout(&request, args.out.as_ref());
            }
            post_json_to_url(
                &format!(
                    "{}/goal-store/plans",
                    args.store.goal_store_url.trim_end_matches('/')
                ),
                &request,
                None,
                None,
            )
            .await
        }
        PlanSubcommand::List(args) => {
            let mut params = Vec::new();
            for status in args.status {
                params.push(format!("status={status}"));
            }
            if let Some(limit) = args.limit {
                params.push(format!("limit={limit}"));
            }
            let query = if params.is_empty() {
                String::new()
            } else {
                format!("?{}", params.join("&"))
            };
            get_url(
                &format!(
                    "{}/goal-store/plans{}",
                    args.store.goal_store_url.trim_end_matches('/'),
                    query
                ),
                None,
            )
            .await
        }
        PlanSubcommand::Show(args) => {
            get_url(
                &format!(
                    "{}/goal-store/plans/{}",
                    args.store.goal_store_url.trim_end_matches('/'),
                    args.plan_id
                ),
                None,
            )
            .await
        }
        PlanSubcommand::Revise(args) => {
            let request: PlanRevisionRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/goal-store/plans/{}/revisions",
                    args.store.goal_store_url.trim_end_matches('/'),
                    args.plan_id
                ),
                &request,
                None,
                None,
            )
            .await
        }
        PlanSubcommand::Compile(args) => {
            let mut request = if let Some(file) = &args.file {
                read_json_file::<PlanCompileRequest>(file)?
            } else {
                PlanCompileRequest {
                    plan_id: Some(args.plan_id),
                    goal_id: None,
                    title_override: None,
                    objective_override: None,
                    strict_review: args.strict_review,
                    human_steered: args.human_steered,
                    enable_branching: args.enable_branching,
                }
            };
            request.plan_id = Some(args.plan_id);
            if args.strict_review {
                request.strict_review = true;
            }
            if args.human_steered {
                request.human_steered = true;
            }
            if args.enable_branching {
                request.enable_branching = true;
            }
            if let Some(out) = &args.out {
                let value = post_json_value_to_url(
                    &format!(
                        "{}/goal-store/plans/{}/compile",
                        args.store.goal_store_url.trim_end_matches('/'),
                        args.plan_id
                    ),
                    &request,
                    None,
                    None,
                )
                .await?;
                if let Some(goal) = value.get("goal") {
                    return write_json_or_stdout(goal, Some(out));
                }
                return write_json_or_stdout(&value, Some(out));
            }
            post_json_to_url(
                &format!(
                    "{}/goal-store/plans/{}/compile",
                    args.store.goal_store_url.trim_end_matches('/'),
                    args.plan_id
                ),
                &request,
                None,
                None,
            )
            .await
        }
    }
}

async fn sandbox(args: SandboxCommand) -> anyhow::Result<()> {
    match args.command {
        SandboxSubcommand::Plan(args) => {
            let request: serde_json::Value = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/launch-plan",
                    args.sandbox_runner_url.trim_end_matches('/')
                ),
                &request,
                None,
                None,
            )
            .await
        }
        SandboxSubcommand::Create(args) => {
            let request: serde_json::Value = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/workspaces",
                    args.sandbox_runner_url.trim_end_matches('/')
                ),
                &request,
                None,
                None,
            )
            .await
        }
        SandboxSubcommand::Snapshot(args) => {
            let request = serde_json::json!({ "workspace_id": args.workspace_id });
            post_json_to_url(
                &format!("{}/snapshot", args.sandbox_runner_url.trim_end_matches('/')),
                &request,
                None,
                None,
            )
            .await
        }
        SandboxSubcommand::Cleanup(args) => {
            let request = serde_json::json!({ "workspace_id": args.workspace_id });
            post_json_to_url(
                &format!("{}/cleanup", args.sandbox_runner_url.trim_end_matches('/')),
                &request,
                None,
                None,
            )
            .await
        }
    }
}

async fn event(args: EventCommand) -> anyhow::Result<()> {
    match args.command {
        EventSubcommand::Sources(args) => {
            get_url(
                &format!(
                    "{}/event-sources",
                    args.event_gateway_url.trim_end_matches('/')
                ),
                args.token.as_deref(),
            )
            .await
        }
        EventSubcommand::Register(args) => {
            let request: EventSource = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/event-sources",
                    args.event_gateway_url.trim_end_matches('/')
                ),
                &request,
                args.token.as_deref(),
                args.approval_id.as_deref(),
            )
            .await
        }
        EventSubcommand::Ingest(args) => {
            let request: ExternalEvent = read_json_file(&args.file)?;
            let route = if args.route { "?route=true" } else { "" };
            post_json_to_url(
                &format!(
                    "{}/events{}",
                    args.event_gateway_url.trim_end_matches('/'),
                    route
                ),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        EventSubcommand::Emit(args) => {
            let request: serde_json::Value = read_json_file(&args.file)?;
            let route = if args.no_route {
                "?route=false"
            } else {
                "?route=true"
            };
            post_json_to_url(
                &format!(
                    "{}/events/generic/{}{}",
                    args.event_gateway_url.trim_end_matches('/'),
                    args.source_id,
                    route
                ),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        EventSubcommand::Trigger(args) => {
            let request: TriggeredGoalRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!("{}/triggers", args.event_gateway_url.trim_end_matches('/')),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        EventSubcommand::List(args) => {
            get_url(
                &format!("{}/events", args.event_gateway_url.trim_end_matches('/')),
                args.token.as_deref(),
            )
            .await
        }
        EventSubcommand::Triggers(args) => {
            get_url(
                &format!("{}/triggers", args.event_gateway_url.trim_end_matches('/')),
                args.token.as_deref(),
            )
            .await
        }
    }
}

async fn memory(args: MemoryCommand) -> anyhow::Result<()> {
    match args.command {
        MemorySubcommand::Write(args) => {
            let request: MemoryWriteRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/memory/write",
                    args.memory_gateway_url.trim_end_matches('/')
                ),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        MemorySubcommand::Search(args) => {
            let request: MemorySearchRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/memory/search",
                    args.memory_gateway_url.trim_end_matches('/')
                ),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        MemorySubcommand::Context(args) => {
            let request: MemoryContextRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/memory/context",
                    args.memory_gateway_url.trim_end_matches('/')
                ),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        MemorySubcommand::Join(args) => {
            let request: MemoryJoinRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/memory/join",
                    args.memory_gateway_url.trim_end_matches('/')
                ),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        MemorySubcommand::Repair(args) => {
            let request: MemoryRepairRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/memory/repair",
                    args.memory_gateway_url.trim_end_matches('/')
                ),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        MemorySubcommand::Events(args) => {
            get_url(
                &format!(
                    "{}/memory/events/{}",
                    args.memory_gateway_url.trim_end_matches('/'),
                    args.goal_id
                ),
                args.token.as_deref(),
            )
            .await
        }
    }
}

async fn runner(args: RunnerCommand) -> anyhow::Result<()> {
    match args.command {
        RunnerSubcommand::List(args) => {
            get_url(
                &format!("{}/runners", args.registry_url.trim_end_matches('/')),
                None,
            )
            .await
        }
        RunnerSubcommand::Status(args) => {
            get_url(
                &format!("{}/runners/status", args.registry_url.trim_end_matches('/')),
                None,
            )
            .await
        }
        RunnerSubcommand::Register(args) => {
            let registration: RunnerRegistration = read_json_file(&args.file)?;
            post_json_to_url(
                &format!("{}/runners", args.registry_url.trim_end_matches('/')),
                &registration,
                None,
                None,
            )
            .await
        }
        RunnerSubcommand::Dispatch(args) => {
            let request: RunnerDispatchRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!("{}/dispatch", args.registry_url.trim_end_matches('/')),
                &request,
                None,
                None,
            )
            .await
        }
    }
}

async fn notify(args: NotifyArgs) -> anyhow::Result<()> {
    if args.threads {
        return get_url(
            &format!("{}/threads", args.notifier_url.trim_end_matches('/')),
            None,
        )
        .await;
    }
    if let Some(thread_key) = args.thread_key {
        return get_url(
            &format!(
                "{}/threads/{}",
                args.notifier_url.trim_end_matches('/'),
                thread_key
            ),
            None,
        )
        .await;
    }
    let file = args
        .file
        .context("--file is required unless --threads or --thread-key is provided")?;
    let request: NotificationRequest = read_json_file(&file)?;
    post_json_to_url(
        &format!("{}/notify", args.notifier_url.trim_end_matches('/')),
        &request,
        None,
        None,
    )
    .await
}

fn init(args: InitArgs) -> anyhow::Result<()> {
    fs::create_dir_all(args.path.join("docs/exec-plans/active"))?;
    fs::create_dir_all(args.path.join("docs/exec-plans/completed"))?;
    fs::create_dir_all(args.path.join("schemas"))?;
    println!("initialized COAT directories under {}", args.path.display());
    Ok(())
}

async fn goal(args: GoalCommand) -> anyhow::Result<()> {
    match args.command {
        GoalSubcommand::Draft(args) => draft_goal(args),
        GoalSubcommand::List(args) => list_goals(args).await,
        GoalSubcommand::Submit(args) => submit_goal(args).await,
        GoalSubcommand::Status(args) => {
            let goal_id = resolve_goal_id(&args.selector).await?;
            restate_post_without_body(&args.restate_ingress, goal_id, "status").await
        }
        GoalSubcommand::Progress(args) => {
            let goal_id = resolve_goal_id(&args.selector).await?;
            restate_post_without_body(&args.restate_ingress, goal_id, "progress").await
        }
        GoalSubcommand::Tasks(args) => {
            let goal_id = resolve_goal_id(&args.selector).await?;
            let query = task_query_from_args(&args)?;
            restate_post_json(&args.restate_ingress, goal_id, "tasks", &query).await
        }
        GoalSubcommand::Lint(args) => lint_goal(args),
        GoalSubcommand::Steer(args) => {
            let goal_id = resolve_goal_id(&args.selector).await?;
            let directive: SteeringDirective =
                read_goal_scoped_json_file(&args.file, goal_id, "SteeringDirective")?;
            restate_post_json(&args.restate_ingress, goal_id, "steer", &directive).await
        }
        GoalSubcommand::SteerStandard(args) => steer_standard_goal(args).await,
        GoalSubcommand::ReviewChecks => review_checks(),
        GoalSubcommand::Restart(args) => {
            let goal_id = resolve_goal_id(&args.selector).await?;
            let request: RestartRequest =
                read_goal_scoped_json_file(&args.file, goal_id, "RestartRequest")?;
            restate_post_json(&args.restate_ingress, goal_id, "restart", &request).await
        }
        GoalSubcommand::Branch(args) => {
            let goal_id = resolve_goal_id(&args.selector).await?;
            let request: BranchRequest =
                read_goal_scoped_json_file(&args.file, goal_id, "BranchRequest")?;
            restate_post_json(&args.restate_ingress, goal_id, "branch", &request).await
        }
        GoalSubcommand::SelectBranch(args) => {
            let goal_id = resolve_goal_id(&args.selector).await?;
            let request: BranchSelectionRequest =
                read_goal_scoped_json_file(&args.file, goal_id, "BranchSelectionRequest")?;
            restate_post_json(&args.restate_ingress, goal_id, "select_branch", &request).await
        }
        GoalSubcommand::Cancel(args) => {
            let goal_id = resolve_goal_id(&args.selector).await?;
            restate_post_json(&args.restate_ingress, goal_id, "cancel", &args.reason).await
        }
    }
}

fn draft_goal(args: DraftGoalArgs) -> anyhow::Result<()> {
    let mut goal = GoalSpec::new(args.title, args.objective);
    goal.repo = args.repo;
    goal.authoring.intake_summary =
        "Drafted by coat goal draft; review and refine before submit.".to_string();
    goal.authoring.acceptance_evidence = args.acceptance_evidence;
    goal.authoring.constraints = args.constraint;
    goal.authoring.out_of_scope = args.out_of_scope;
    goal.authoring.assumptions = args.assumption;
    goal.authoring.open_questions = args.open_question;
    goal.plan.summary = args
        .plan_summary
        .unwrap_or_else(|| "Structured goal draft; subgoals should be refined by the authoring critic before submit.".to_string());
    goal.plan.subgoals = args
        .subgoal
        .iter()
        .map(|raw| parse_subgoal_spec(raw))
        .collect::<anyhow::Result<Vec<_>>>()?;
    goal.initial_tasks = args
        .initial_task
        .iter()
        .map(|raw| parse_initial_task_spec(raw))
        .collect::<anyhow::Result<Vec<_>>>()?;

    if args.human_steered {
        goal.control_policy.mode = ControlLoopMode::HumanSteeredContinuous;
    }
    if args.enable_branching {
        goal.branching_policy.enabled = true;
    }
    if args.strict_review || !args.review_preset.is_empty() {
        let mut doctrine = if args.strict_review {
            ReviewDoctrine::strict_engineering()
        } else {
            ReviewDoctrine::default()
        };
        doctrine.enabled = true;
        if !args.review_preset.is_empty() {
            doctrine.presets = args
                .review_preset
                .iter()
                .map(|preset| {
                    parse_json_enum::<ReviewDoctrinePreset>(preset, "ReviewDoctrinePreset")
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
        }
        doctrine.coverage.require_objective_results = true;
        doctrine.coverage.require_gate_results = true;
        doctrine.coverage.require_required_evidence = true;
        doctrine.coverage.min_objective_score = Some(0.85);
        goal.review_policy.doctrine = doctrine;
        goal.review_policy.min_reviews = goal.review_policy.min_reviews.max(1);
        goal.review_policy.reviewer_roles = vec![
            WorkerKind::Reviewer,
            WorkerKind::Tester,
            WorkerKind::FormalMethods,
        ];
    }

    write_json_or_stdout(&goal, args.out.as_ref())
}

fn plan_draft_request_from_args(args: &PlanDraftArgs) -> anyhow::Result<PlanDraftRequest> {
    if let Some(file) = &args.file {
        return read_json_file(file);
    }
    let title = args
        .title
        .clone()
        .context("--title is required when --file is not provided")?;
    let objective = args
        .objective
        .clone()
        .context("--objective is required when --file is not provided")?;
    let mode = parse_json_enum::<PlanningMode>(&args.mode, "PlanningMode")?;
    let mut authoring = GoalAuthoringGuidance {
        intake_summary: args.prompt.clone().unwrap_or_else(|| objective.clone()),
        acceptance_evidence: args.acceptance_evidence.clone(),
        constraints: args.constraint.clone(),
        out_of_scope: args.out_of_scope.clone(),
        assumptions: args.assumption.clone(),
        open_questions: args.open_question.clone(),
    };
    if authoring.intake_summary.trim().is_empty() {
        authoring.intake_summary =
            "Drafted through coat plan draft; revise before compiling to a GoalSpec.".to_string();
    }
    let plan = GoalPlan {
        summary: args.summary.clone().unwrap_or_else(|| {
            "Durable planning draft; use plan revise until questions and subgoals are ready."
                .to_string()
        }),
        subgoals: args
            .subgoal
            .iter()
            .map(|raw| parse_subgoal_spec(raw))
            .collect::<anyhow::Result<Vec<_>>>()?,
        distribution_notes: vec![
            "Plan revisions are durable; compile to GoalSpec only after questions and evidence are clear."
                .to_string(),
        ],
    };
    let questions = args
        .open_question
        .iter()
        .enumerate()
        .map(|(index, question)| PlanQuestion {
            id: format!("q{}", index + 1),
            question: question.clone(),
            required: true,
            status: PlanQuestionStatus::Open,
            answer: None,
        })
        .collect();
    Ok(PlanDraftRequest {
        plan_id: None,
        title,
        objective: objective.clone(),
        repo: args.repo.clone(),
        prompt: args.prompt.clone().unwrap_or(objective),
        mode,
        status: None,
        author: args.author.clone(),
        summary: args.summary.clone(),
        authoring,
        plan,
        initial_tasks: args
            .initial_task
            .iter()
            .map(|raw| parse_initial_task_spec(raw))
            .collect::<anyhow::Result<Vec<_>>>()?,
        questions,
        decisions: Vec::new(),
    })
}

async fn steer_standard_goal(args: SteerStandardGoalArgs) -> anyhow::Result<()> {
    let goal_id = resolve_goal_id(&args.selector).await?;
    let check: StandardReviewCheck = parse_json_enum(&args.check, "StandardReviewCheck")?;
    let message = args
        .message
        .unwrap_or_else(|| format!("Request {}", check.title()));
    let directive = SteeringDirective {
        id: Uuid::new_v4(),
        goal_id,
        task_id: args.task_id,
        operator: args.operator,
        message,
        kind: SteeringDirectiveKind::RequestStandardReview {
            check,
            topic: args.topic,
            reason: args.reason,
        },
    };
    if args.emit_only || args.out.is_some() {
        return write_json_or_stdout(&directive, args.out.as_ref());
    }
    restate_post_json(&args.restate_ingress, goal_id, "steer", &directive).await
}

fn review_checks() -> anyhow::Result<()> {
    let checks: Vec<_> = StandardReviewCheck::all()
        .iter()
        .map(|check| {
            serde_json::json!({
                "check": check.as_str(),
                "title": check.title(),
                "worker_role": check.worker_role(),
                "research_like": check.is_research_like(),
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&checks)?);
    Ok(())
}

async fn submit_goal(args: SubmitGoalArgs) -> anyhow::Result<()> {
    let goal = if let Some(file) = args.file {
        let raw = fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
        serde_json::from_str::<GoalSpec>(&raw)
            .with_context(|| format!("parse {}", file.display()))?
    } else {
        let title = args
            .title
            .context("--title is required when --file is not provided")?;
        let objective = args
            .objective
            .context("--objective is required when --file is not provided")?;
        GoalSpec::new(title, objective)
    };

    let url = workflow_url(&args.restate_ingress, goal.id, "run");
    let response = reqwest::Client::new().post(&url).json(&goal).send().await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("submit failed with {status}: {body}");
    }
    let response = serde_json::from_str::<serde_json::Value>(&body)
        .unwrap_or_else(|_| serde_json::Value::String(body));
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "submitted": true,
            "goal_id": goal.id,
            "workflow_url": url,
            "set_env": format!("export COAT_GOAL_ID={}", goal.id),
            "response": response,
        }))?
    );
    Ok(())
}

async fn list_goals(args: GoalListArgs) -> anyhow::Result<()> {
    let mut params = Vec::new();
    for status in args.status {
        params.push(format!("status={status}"));
    }
    if let Some(repo) = args.repo {
        params.push(format!("repo={repo}"));
    }
    if let Some(limit) = args.limit {
        params.push(format!("limit={limit}"));
    }
    let query = if params.is_empty() {
        String::new()
    } else {
        format!("?{}", params.join("&"))
    };
    get_url(
        &format!(
            "{}/goal-store/goals{}",
            args.goal_store_url.trim_end_matches('/'),
            query
        ),
        None,
    )
    .await
}

fn lint_goal(args: GoalLintArgs) -> anyhow::Result<()> {
    let goal: GoalSpec = read_json_file(&args.file)?;
    let report = goal.quality_report();
    println!("{}", serde_json::to_string_pretty(&report)?);
    if args.strict && !report.ready {
        bail!("goal quality report is not ready");
    }
    Ok(())
}

fn task_query_from_args(args: &GoalTasksArgs) -> anyhow::Result<TaskQuery> {
    let mut query = if let Some(file) = &args.file {
        read_json_file::<TaskQuery>(file)?
    } else {
        TaskQuery::default()
    };
    if args.subgoal_id.is_some() {
        query.subgoal_id = args.subgoal_id.clone();
    }
    for status in &args.status {
        query
            .statuses
            .push(parse_json_enum::<TaskStatus>(status, "TaskStatus")?);
    }
    for role in &args.role {
        query
            .roles
            .push(parse_json_enum::<WorkerKind>(role, "WorkerKind")?);
    }
    for purpose in &args.purpose {
        query.purpose_kinds.push(parse_json_enum::<TaskPurposeKind>(
            purpose,
            "TaskPurposeKind",
        )?);
    }
    query.color_keys.extend(args.color.clone());
    query.tags.extend(args.tag.clone());
    if args.runnable {
        query.runnable_only = true;
    }
    if args.limit.is_some() {
        query.limit = args.limit;
    }
    Ok(query)
}

fn parse_subgoal_spec(raw: &str) -> anyhow::Result<SubgoalSpec> {
    let kv = parse_kv_spec(raw)?;
    let role = match kv.get("role") {
        Some(role) => parse_json_enum::<WorkerKind>(role, "WorkerKind")?,
        None => WorkerKind::Codex,
    };
    let priority = match kv.get("priority") {
        Some(priority) => parse_json_enum::<TaskPriority>(priority, "TaskPriority")?,
        None => TaskPriority::Normal,
    };
    Ok(SubgoalSpec {
        id: required_kv(&kv, "id")?,
        title: required_kv(&kv, "title")?,
        objective: required_kv(&kv, "objective")?,
        owner_role: role,
        color: parse_graph_color(&kv),
        priority,
        dependencies: split_list(kv.get("dependencies")),
        tags: split_list(kv.get("tags")),
        acceptance_evidence: split_list(kv.get("acceptance_evidence")),
    })
}

fn parse_initial_task_spec(raw: &str) -> anyhow::Result<ChildTaskRequest> {
    let kv = parse_kv_spec(raw)?;
    let role = match kv.get("role") {
        Some(role) => parse_json_enum::<WorkerKind>(role, "WorkerKind")?,
        None => WorkerKind::Codex,
    };
    let priority = match kv.get("priority") {
        Some(priority) => parse_json_enum::<TaskPriority>(priority, "TaskPriority")?,
        None => TaskPriority::Normal,
    };
    let prompt = required_kv(&kv, "prompt")?;
    let purpose = match kv.get("purpose").map(String::as_str) {
        Some("research") => Some(TaskPurpose::Research {
            question: kv
                .get("question")
                .cloned()
                .unwrap_or_else(|| prompt.clone()),
        }),
        Some("work") | None => {
            if role == WorkerKind::Research {
                Some(TaskPurpose::Research {
                    question: kv
                        .get("question")
                        .cloned()
                        .unwrap_or_else(|| prompt.clone()),
                })
            } else {
                Some(TaskPurpose::Work)
            }
        }
        Some(other) => bail!(
            "unsupported initial task purpose '{other}'; use work or research for draft seeding"
        ),
    };
    Ok(ChildTaskRequest {
        role,
        purpose,
        title: kv.get("title").cloned(),
        subgoal_id: kv.get("subgoal_id").or_else(|| kv.get("subgoal")).cloned(),
        color: parse_graph_color(&kv),
        prompt,
        reason: kv
            .get("reason")
            .cloned()
            .unwrap_or_else(|| "seeded by coat goal draft".to_string()),
        dependencies: Vec::new(),
        budget: None,
        sandbox: None,
        done_criteria: None,
        review_doctrine: None,
        execution: None,
        priority,
        tags: split_list(kv.get("tags")),
    })
}

fn parse_graph_color(kv: &BTreeMap<String, String>) -> Option<GraphColorRef> {
    let key = kv.get("color").or_else(|| kv.get("color_key"))?.clone();
    let label = kv
        .get("color_label")
        .cloned()
        .unwrap_or_else(|| key.clone());
    let hex = kv
        .get("color_hex")
        .cloned()
        .unwrap_or_else(|| "#9aa6ad".to_string());
    let meaning = kv
        .get("color_meaning")
        .cloned()
        .unwrap_or_else(|| format!("custom graph color {key}"));
    Some(GraphColorRef::new(key, label, hex, meaning))
}

fn parse_kv_spec(raw: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let mut kv = BTreeMap::new();
    for part in raw.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (key, value) = trimmed
            .split_once('=')
            .with_context(|| format!("expected key=value in '{trimmed}'"))?;
        let key = key.trim();
        if key.is_empty() {
            bail!("empty key in '{trimmed}'");
        }
        kv.insert(key.to_string(), value.trim().to_string());
    }
    Ok(kv)
}

fn required_kv(kv: &BTreeMap<String, String>, key: &str) -> anyhow::Result<String> {
    kv.get(key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .with_context(|| format!("missing required key '{key}'"))
}

fn split_list(value: Option<&String>) -> Vec<String> {
    value
        .map(|value| {
            value
                .split('|')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_json_enum<T: serde::de::DeserializeOwned>(
    value: &str,
    type_name: &str,
) -> anyhow::Result<T> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .with_context(|| format!("parse {type_name} value '{value}'"))
}

fn write_json_or_stdout<T: serde::Serialize>(
    value: &T,
    out: Option<&PathBuf>,
) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    if let Some(out) = out {
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(out, format!("{json}\n"))?;
        println!("wrote {}", out.display());
    } else {
        println!("{json}");
    }
    Ok(())
}

async fn approve(args: ApproveArgs) -> anyhow::Result<()> {
    let goal_id = resolve_goal_id(&args.selector).await?;
    let approval = HumanApproval {
        approval_id: args.approval_id,
        approved: args.approved,
        note: args.note,
    };
    restate_post_json(&args.restate_ingress, goal_id, "approve", &approval).await
}

async fn restate_post_without_body(
    ingress: &str,
    goal_id: Uuid,
    handler: &str,
) -> anyhow::Result<()> {
    let url = workflow_url(ingress, goal_id, handler);
    let response = reqwest::Client::new().post(&url).send().await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{handler} failed with {status}: {body}");
    }
    println!("{body}");
    Ok(())
}

async fn restate_post_json<T: serde::Serialize + ?Sized>(
    ingress: &str,
    goal_id: Uuid,
    handler: &str,
    body: &T,
) -> anyhow::Result<()> {
    let url = workflow_url(ingress, goal_id, handler);
    let response = reqwest::Client::new().post(&url).json(body).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{handler} failed with {status}: {text}");
    }
    println!("{text}");
    Ok(())
}

async fn post_json_to_url<T: serde::Serialize + ?Sized>(
    url: &str,
    body: &T,
    bearer_token: Option<&str>,
    approval_id: Option<&str>,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let mut request = client.post(url).json(body);
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    if let Some(approval_id) = approval_id {
        request = request.header("x-coat-approval-id", approval_id);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("POST {url} failed with {status}: {text}");
    }
    println!("{text}");
    Ok(())
}

async fn post_json_value_to_url<T: serde::Serialize + ?Sized>(
    url: &str,
    body: &T,
    bearer_token: Option<&str>,
    approval_id: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let client = reqwest::Client::new();
    let mut request = client.post(url).json(body);
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    if let Some(approval_id) = approval_id {
        request = request.header("x-coat-approval-id", approval_id);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("POST {url} failed with {status}: {text}");
    }
    serde_json::from_str(&text).with_context(|| format!("parse response from {url}"))
}

async fn get_json_value_from_url(
    url: &str,
    bearer_token: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let client = reqwest::Client::new();
    let mut request = client.get(url);
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("GET {url} failed with {status}: {text}");
    }
    serde_json::from_str(&text).with_context(|| format!("parse response from {url}"))
}

async fn get_url(url: &str, bearer_token: Option<&str>) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let mut request = client.get(url);
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("GET {url} failed with {status}: {text}");
    }
    println!("{text}");
    Ok(())
}

fn read_json_file<T: serde::de::DeserializeOwned>(path: &PathBuf) -> anyhow::Result<T> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

fn read_goal_scoped_json_file<T: serde::de::DeserializeOwned>(
    path: &PathBuf,
    goal_id: Uuid,
    type_name: &str,
) -> anyhow::Result<T> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut value: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    ensure_json_goal_id(&mut value, goal_id)
        .with_context(|| format!("prepare {type_name} from {}", path.display()))?;
    serde_json::from_value(value)
        .with_context(|| format!("parse {type_name} from {}", path.display()))
}

fn ensure_json_goal_id(value: &mut serde_json::Value, goal_id: Uuid) -> anyhow::Result<()> {
    let object = value
        .as_object_mut()
        .context("goal-scoped JSON must be an object")?;
    match object.get("goal_id") {
        Some(existing) if !existing.is_null() => {
            let existing_goal_id: Uuid =
                serde_json::from_value(existing.clone()).context("parse existing goal_id")?;
            if existing_goal_id != goal_id {
                bail!("JSON goal_id {existing_goal_id} does not match selected goal_id {goal_id}");
            }
        }
        _ => {
            object.insert(
                "goal_id".to_string(),
                serde_json::Value::String(goal_id.to_string()),
            );
        }
    }
    Ok(())
}

async fn resolve_goal_id(selector: &GoalSelectorArgs) -> anyhow::Result<Uuid> {
    match (selector.goal_id, selector.latest) {
        (Some(_), true) => bail!("use either --goal-id/COAT_GOAL_ID or --latest, not both"),
        (Some(goal_id), false) => Ok(goal_id),
        (None, true) => latest_goal_id(&selector.goal_store_url).await,
        (None, false) => bail!(
            "select a goal with --goal-id, set COAT_GOAL_ID, or use --latest with a reachable goal store"
        ),
    }
}

async fn latest_goal_id(goal_store_url: &str) -> anyhow::Result<Uuid> {
    let url = format!("{}/goal-store/goals", goal_store_url.trim_end_matches('/'));
    let value = get_json_value_from_url(&url, None).await?;
    latest_goal_id_from_value(&value)
}

fn latest_goal_id_from_value(value: &serde_json::Value) -> anyhow::Result<Uuid> {
    let goals: Vec<GoalRecord> = serde_json::from_value(
        value
            .get("goals")
            .cloned()
            .context("goal-store response is missing goals")?,
    )
    .context("parse goal-store goals")?;
    goals
        .into_iter()
        .max_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.title.cmp(&right.title))
                .then_with(|| left.goal_id.cmp(&right.goal_id))
        })
        .map(|goal| goal.goal_id)
        .context("goal store returned no goals")
}

fn workflow_url(ingress: &str, goal_id: Uuid, handler: &str) -> String {
    format!(
        "{}/GoalWorkflow/{}/{}",
        ingress.trim_end_matches('/'),
        goal_id,
        handler
    )
}

fn compose(args: ComposeCommand) -> anyhow::Result<()> {
    let mut command = Command::new("docker");
    match args.command {
        ComposeSubcommand::Up(args) => {
            command.arg("compose");
            if args.restate_cloud {
                command.arg("--env-file").arg(args.env_file);
            }
            command.arg("-f").arg("infra/compose/docker-compose.yml");
            if args.restate_cloud {
                command
                    .arg("-f")
                    .arg("infra/compose/docker-compose.restate-cloud.yml")
                    .arg("--profile")
                    .arg("restate-cloud");
            }
            for profile in args.profile {
                command.arg("--profile").arg(profile);
            }
            command.arg("up").arg("--build");
        }
        ComposeSubcommand::Down(args) => {
            command.arg("compose");
            if args.restate_cloud {
                command.arg("--env-file").arg(args.env_file);
            }
            command.arg("-f").arg("infra/compose/docker-compose.yml");
            if args.restate_cloud {
                command
                    .arg("-f")
                    .arg("infra/compose/docker-compose.restate-cloud.yml");
            }
            command.arg("down");
        }
    }
    let status = command.status().context("run docker compose")?;
    if !status.success() {
        bail!("docker compose exited with {status}");
    }
    Ok(())
}

fn k8s(args: K8sCommand) -> anyhow::Result<()> {
    match args.command {
        K8sSubcommand::Render(args) => {
            let manifest = fs::read_to_string("infra/k8s/base/all.yaml")
                .context("read infra/k8s/base/all.yaml")?;
            if let Some(parent) = args.output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&args.output, manifest)?;
            println!("rendered {}", args.output.display());
            Ok(())
        }
        K8sSubcommand::EphemeralJobs(args) => match args.command {
            EphemeralJobsSubcommand::Render(args) => {
                let manifest = fs::read_to_string(&args.source)
                    .with_context(|| format!("read {}", args.source.display()))?;
                if let Some(parent) = args.output.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&args.output, manifest)?;
                println!(
                    "rendered ephemeral runner jobs {} from {}",
                    args.output.display(),
                    args.source.display()
                );
                Ok(())
            }
        },
    }
}

fn restate(args: RestateCommand) -> anyhow::Result<()> {
    match args.command {
        RestateSubcommand::CloudEnv(args) => restate_cloud_env(args),
        RestateSubcommand::TunnelDocker(args) => restate_tunnel_docker(args),
        RestateSubcommand::RegisterCloud(args) => restate_register_cloud(args),
    }
}

fn restate_cloud_env(args: RestateCloudEnvArgs) -> anyhow::Result<()> {
    println!("# Restate Cloud operator environment for COAT");
    println!(
        "export RESTATE_TUNNEL_NAME={}",
        shell_quote(&args.tunnel_name)
    );
    println!("export RESTATE_CLOUD_REGION={}", shell_quote(&args.region));
    if let Some(environment_id) = args.environment_id.as_deref() {
        println!(
            "export RESTATE_ENVIRONMENT_ID={}",
            shell_quote(environment_id)
        );
    }
    if let Some(signing_public_key) = args.signing_public_key.as_deref() {
        println!(
            "export RESTATE_SIGNING_PUBLIC_KEY={}",
            shell_quote(signing_public_key)
        );
        println!(
            "export RESTATE_IDENTITY_KEYS={}",
            shell_quote(signing_public_key)
        );
    }
    println!(
        "export COAT_RESTATE_INGRESS={}",
        shell_quote(&args.ingress_url)
    );
    println!("export COAT_RESTATE_ADMIN={}", shell_quote(&args.admin_url));
    println!(
        "export COAT_COORDINATOR_RESTATE_ENDPOINT={}",
        shell_quote(&args.coordinator_url)
    );
    println!();
    println!("# Common next steps:");
    println!(
        "#   restate cloud env tunnel --tunnel-name {}",
        shell_quote(&args.tunnel_name)
    );
    println!(
        "#   coat restate register-cloud --tunnel-name {} --service-url {}",
        shell_quote(&args.tunnel_name),
        shell_quote(&args.coordinator_url)
    );
    Ok(())
}

fn restate_tunnel_docker(args: RestateTunnelDockerArgs) -> anyhow::Result<()> {
    println!("docker run \\");
    println!("  -e RESTATE_ENVIRONMENT_ID \\");
    println!("  -e RESTATE_BEARER_TOKEN \\");
    println!("  -e RESTATE_TUNNEL_NAME \\");
    println!("  -e RESTATE_SIGNING_PUBLIC_KEY \\");
    println!("  -e RESTATE_CLOUD_REGION \\");
    println!("  -p {}:8080 \\", args.ingress_port);
    println!("  -p {}:9090 \\", args.health_port);
    println!("  -p {}:9070 \\", args.admin_port);
    println!("  -it {}", shell_quote(&args.image));
    println!();
    println!("# Suggested environment:");
    println!(
        "export RESTATE_ENVIRONMENT_ID={}",
        shell_quote(&args.environment_id)
    );
    println!("export RESTATE_BEARER_TOKEN=replace-me");
    println!(
        "export RESTATE_TUNNEL_NAME={}",
        shell_quote(&args.tunnel_name)
    );
    println!(
        "export RESTATE_SIGNING_PUBLIC_KEY={}",
        shell_quote(&args.signing_public_key)
    );
    println!("export RESTATE_CLOUD_REGION={}", shell_quote(&args.region));
    Ok(())
}

fn restate_register_cloud(args: RestateRegisterCloudArgs) -> anyhow::Result<()> {
    let command_args = [
        "deployments",
        "register",
        "--tunnel-name",
        args.tunnel_name.as_str(),
        args.service_url.as_str(),
    ];
    if args.dry_run {
        println!("restate {}", command_args.join(" "));
        return Ok(());
    }
    let status = Command::new("restate")
        .args(command_args)
        .status()
        .context("run restate deployments register")?;
    if !status.success() {
        bail!("restate deployments register exited with {status}");
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '@'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bump_release_versions, ensure_json_goal_id, extract_follow_ups, latest_goal_id_from_value,
        release_plan_json, replace_toml_section_value, replace_yaml_root_value,
    };
    use uuid::Uuid;

    #[test]
    fn extracts_follow_ups_until_next_section() {
        let markdown = r#"# Example Plan

## Implementation

- Already done.

## Follow-Ups

- Add live adapter tests.
- Document production rollout.

## Acceptance

- Tests pass.
"#;

        assert_eq!(
            extract_follow_ups(markdown),
            vec![
                "Add live adapter tests.".to_string(),
                "Document production rollout.".to_string()
            ]
        );
    }

    #[test]
    fn release_plan_includes_cut_command_and_tags() {
        let plan = release_plan_json("v1.2.3", Some("v1.2.4"), Some("v1.2.3"), None)
            .expect("release plan");

        assert_eq!(plan["version"], "1.2.3");
        assert_eq!(plan["app_version"], "1.2.3");
        assert_eq!(plan["chart_version"], "1.2.4");
        assert_eq!(plan["binary_tag"], "v1.2.3");
        assert_eq!(plan["chart_tag"], "chart-v1.2.4");
        assert_eq!(
            plan["binary_assets"],
            serde_json::json!([
                "jattg-binaries-1.2.3-x86_64-unknown-linux-gnu.tar.gz",
                "jattg-binaries-1.2.3-aarch64-unknown-linux-gnu.tar.gz",
                "jattg-binaries-1.2.3-aarch64-apple-darwin.tar.gz"
            ])
        );
        assert_eq!(
            plan["container_registry"],
            "ghcr.io/josephjohncox/joseph-and-the-amazing-technicolor-task-graph"
        );
        assert_eq!(
            plan["container_image_tags"],
            serde_json::json!(["v1.2.3", "1.2.3", "latest"])
        );
        assert!(
            plan["bump_files"]
                .as_array()
                .expect("bump files")
                .iter()
                .any(|file| file == "Cargo.lock")
        );
        assert!(
            plan["container_images"]
                .as_array()
                .expect("container images")
                .iter()
                .any(|image| image
                    == "ghcr.io/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/jattg-runner-registry:v1.2.3")
        );
        assert!(
            plan["publish_steps"]
                .as_array()
                .expect("publish steps")
                .iter()
                .any(|step| step == "coat release cut --version 1.2.3 --chart-version 1.2.4")
        );
    }

    #[test]
    fn release_plan_supports_tag_suffix_for_retry_cuts() {
        let plan = release_plan_json("v1.2.3", None, None, Some("ghcr.1")).expect("release plan");

        assert_eq!(plan["version"], "1.2.3");
        assert_eq!(plan["tag_suffix"], "ghcr.1");
        assert_eq!(plan["binary_tag"], "v1.2.3-ghcr.1");
        assert_eq!(plan["chart_tag"], "chart-v1.2.3-ghcr.1");
        assert_eq!(
            plan["container_image_tags"],
            serde_json::json!(["v1.2.3-ghcr.1", "1.2.3-ghcr.1", "latest"])
        );
        assert!(
            plan["publish_steps"]
                .as_array()
                .expect("publish steps")
                .iter()
                .any(|step| step == "coat release cut --version 1.2.3 --tag-suffix ghcr.1")
        );
    }

    #[test]
    fn release_version_replacements_update_expected_roots() {
        let cargo_toml = r#"[workspace]
members = []

[workspace.package]
version = "0.1.0"
edition = "2024"
"#;
        let chart_yaml = r#"apiVersion: v2
name: jattg
version: 0.1.0
appVersion: 0.1.0
"#;

        let cargo_toml =
            replace_toml_section_value(cargo_toml, "workspace.package", "version", "\"0.2.0\"")
                .expect("replace cargo version");
        let chart_yaml =
            replace_yaml_root_value(chart_yaml, "version", "0.2.1").expect("replace chart version");
        let chart_yaml = replace_yaml_root_value(&chart_yaml, "appVersion", "0.2.0")
            .expect("replace chart app version");

        assert!(cargo_toml.contains("version = \"0.2.0\""));
        assert!(chart_yaml.contains("version: 0.2.1"));
        assert!(chart_yaml.contains("appVersion: 0.2.0"));
    }

    #[test]
    fn bump_release_versions_updates_cargo_and_chart_files() {
        let temp = std::env::temp_dir().join(format!(
            "coat-cli-release-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp).expect("tempdir");
        let cargo_toml = temp.join("Cargo.toml");
        let chart_yaml = temp.join("Chart.yaml");
        std::fs::write(
            &cargo_toml,
            r#"[workspace.package]
version = "0.1.0"
edition = "2024"
"#,
        )
        .expect("cargo toml");
        std::fs::write(
            &chart_yaml,
            r#"apiVersion: v2
name: jattg
version: 0.1.0
appVersion: 0.1.0
"#,
        )
        .expect("chart yaml");

        bump_release_versions(&cargo_toml, &chart_yaml, "0.2.0", "0.2.1").expect("bump release");

        assert!(
            std::fs::read_to_string(&cargo_toml)
                .expect("read cargo")
                .contains("version = \"0.2.0\"")
        );
        let chart = std::fs::read_to_string(&chart_yaml).expect("read chart");
        assert!(chart.contains("version: 0.2.1"));
        assert!(chart.contains("appVersion: 0.2.0"));
        std::fs::remove_dir_all(&temp).expect("cleanup tempdir");
    }

    #[test]
    fn goal_scoped_json_injects_missing_goal_id_and_rejects_mismatch() {
        let goal_id = Uuid::parse_str("018f8f2f-1fd8-7688-bb12-8bfb6b756602").expect("goal id");
        let mut value = serde_json::json!({
            "id": "018f8f2f-1fd8-7688-bb12-8bfb6b756603",
            "message": "operator steering"
        });
        ensure_json_goal_id(&mut value, goal_id).expect("inject goal id");
        assert_eq!(value["goal_id"], goal_id.to_string());

        let mut null_goal = serde_json::json!({
            "goal_id": null,
            "message": "operator steering"
        });
        ensure_json_goal_id(&mut null_goal, goal_id).expect("replace null goal id");
        assert_eq!(null_goal["goal_id"], goal_id.to_string());

        let mut mismatched = serde_json::json!({
            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756604"
        });
        assert!(ensure_json_goal_id(&mut mismatched, goal_id).is_err());
    }

    #[test]
    fn latest_goal_id_prefers_newest_updated_at() {
        let expected = Uuid::parse_str("018f8f2f-1fd8-7688-bb12-8bfb6b756602").expect("goal id");
        let value = serde_json::json!({
            "goals": [
                {
                    "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                    "title": "Older",
                    "objective": "old objective",
                    "repo": null,
                    "status": "running",
                    "total_tasks": 1,
                    "open_tasks": 1,
                    "blocked_tasks": 0,
                    "failed_tasks": 0,
                    "percent_done": 0.0,
                    "root_task_id": null,
                    "satisfied": false,
                    "satisfaction_score": null,
                    "updated_at": "2026-05-06T10:00:00Z",
                    "payload_json": {}
                },
                {
                    "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
                    "title": "Newer",
                    "objective": "new objective",
                    "repo": null,
                    "status": "running",
                    "total_tasks": 1,
                    "open_tasks": 1,
                    "blocked_tasks": 0,
                    "failed_tasks": 0,
                    "percent_done": 0.0,
                    "root_task_id": null,
                    "satisfied": false,
                    "satisfaction_score": null,
                    "updated_at": "2026-05-06T11:00:00Z",
                    "payload_json": {}
                }
            ]
        });
        assert_eq!(latest_goal_id_from_value(&value).expect("latest"), expected);
    }
}
