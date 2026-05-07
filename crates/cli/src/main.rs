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
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use coat_domain::{
    BranchRequest, BranchSelectionRequest, ChildTaskRequest, ControlLoopMode, EventSource,
    ExternalEvent, GoalAuthoringGuidance, GoalPlan, GoalRecord, GoalSpec, GraphColorRef,
    HumanApproval, MemoryContextRequest, MemoryEditPreviewRequest, MemoryEditRequest,
    MemoryJoinRequest, MemoryRepairRequest, MemoryRetractRequest, MemorySearchRequest,
    MemoryWriteRequest, NotificationRequest, PlanCompileRequest, PlanDraftRequest, PlanQuestion,
    PlanQuestionStatus, PlanRevisionRequest, PlanningMode, RestartRequest, ReviewDoctrine,
    ReviewDoctrinePreset, RunnerDispatchRequest, RunnerRegistration, StandardReviewCheck,
    SteeringDirective, SteeringDirectiveKind, SubgoalSpec, TaskPriority, TaskPurpose,
    TaskPurposeKind, TaskQuery, TaskStatus, TriggeredGoalRequest, WorkerKind,
};
use dialoguer::{Confirm, Input, MultiSelect, Select, theme::ColorfulTheme};
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
    Setup(SetupCommand),
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
    source_plan_id: Option<Uuid>,
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
    Webhook(EventWebhookArgs),
    PollSqs(EventSqsPollArgs),
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

#[derive(Debug, Args)]
struct EventWebhookArgs {
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
}

#[derive(Debug, Args)]
struct EventSqsPollArgs {
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
    max_messages: Option<i32>,
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
    Retract(MemoryRetractArgs),
    Edit(MemoryEditArgs),
    PreviewEdit(MemoryEditPreviewArgs),
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
struct MemoryRetractArgs {
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
struct MemoryEditArgs {
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
struct MemoryEditPreviewArgs {
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
    queue: bool,
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
    EventSourceApprovals(StoreEventSourceApprovalsArgs),
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
struct StoreEventSourceApprovalsArgs {
    #[arg(
        long,
        env = "COAT_GOAL_STORE_URL",
        default_value = "http://localhost:9088"
    )]
    goal_store_url: String,
    #[arg(long)]
    source_id: Option<String>,
    #[arg(long)]
    approval_ref: Option<String>,
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
struct SetupCommand {
    #[command(subcommand)]
    command: SetupSubcommand,
}

#[derive(Debug, Subcommand)]
enum SetupSubcommand {
    LocalAuth(LocalAuthArgs),
    ChatClient(ChatClientArgs),
}

#[derive(Debug, Args)]
struct LocalAuthArgs {
    #[arg(long, default_value = "infra/compose/local-providers.env")]
    output: PathBuf,
    #[arg(long)]
    write_env: bool,
    #[arg(long)]
    check: bool,
    #[arg(long)]
    print_commands: bool,
}

#[derive(Debug, Args, Clone)]
struct ChatClientArgs {
    #[arg(
        long,
        env = "COAT_CONTROL_MCP_URL",
        default_value = "http://localhost:9090/mcp"
    )]
    mcp_url: String,
    #[arg(long, default_value = "coat-control")]
    server_name: String,
    #[arg(long, default_value = "COAT_CONTROL_MCP_TOKEN")]
    token_env: String,
    #[arg(long)]
    no_token: bool,
    #[arg(long, default_value = "user")]
    claude_scope: String,
    #[arg(long)]
    install_codex_mcp: bool,
    #[arg(long)]
    install_claude_mcp: bool,
    #[arg(long)]
    write_claude_project_config: bool,
    #[arg(long, default_value = ".mcp.json")]
    claude_project_config: PathBuf,
    #[arg(long)]
    write_skill: bool,
    #[arg(long, default_value = ".claude/skills/coat-control-plane")]
    skill_dir: PathBuf,
    #[arg(long)]
    install_codex_skill: bool,
    #[arg(long)]
    install_claude_skill: bool,
    #[arg(long)]
    print_commands: bool,
}

#[derive(Debug, Args)]
struct ComposeCommand {
    #[command(subcommand)]
    command: ComposeSubcommand,
}

#[derive(Debug, Subcommand)]
enum ComposeSubcommand {
    Up(ComposeUpArgs),
    Config(ComposeConfigArgs),
    Down(ComposeDownArgs),
}

#[derive(Debug, Args, Clone)]
struct ComposeUpArgs {
    #[arg(long)]
    restate_cloud: bool,
    #[arg(
        long = "restate-cloud-env-file",
        default_value = "infra/compose/restate-cloud.env"
    )]
    restate_cloud_env_file: PathBuf,
    #[arg(long = "env-file")]
    env_file: Vec<PathBuf>,
    #[arg(long)]
    profile: Vec<String>,
    #[arg(long)]
    detach: bool,
    #[arg(long)]
    register_cloud: bool,
    #[arg(long)]
    init_env: bool,
    #[arg(long, env = "RESTATE_TUNNEL_NAME", default_value = "jattg-personal")]
    tunnel_name: String,
    #[arg(long, default_value = "http://coordinator:9080")]
    service_url: String,
    #[arg(value_name = "SERVICE")]
    services: Vec<String>,
}

#[derive(Debug, Args, Clone)]
struct ComposeConfigArgs {
    #[arg(long)]
    restate_cloud: bool,
    #[arg(
        long = "restate-cloud-env-file",
        default_value = "infra/compose/restate-cloud.env"
    )]
    restate_cloud_env_file: PathBuf,
    #[arg(long = "env-file")]
    env_file: Vec<PathBuf>,
    #[arg(long)]
    profile: Vec<String>,
}

#[derive(Debug, Args)]
struct ComposeDownArgs {
    #[arg(long)]
    restate_cloud: bool,
    #[arg(
        long = "restate-cloud-env-file",
        default_value = "infra/compose/restate-cloud.env"
    )]
    restate_cloud_env_file: PathBuf,
    #[arg(long = "env-file")]
    env_file: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct K8sCommand {
    #[command(subcommand)]
    command: K8sSubcommand,
}

#[derive(Debug, Subcommand)]
enum K8sSubcommand {
    Render(RenderArgs),
    Apply(K8sApplyArgs),
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
    Apply(EphemeralJobsApplyArgs),
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
struct K8sApplyArgs {
    #[arg(long, default_value = "infra/k8s/base/all.yaml")]
    file: PathBuf,
    #[arg(long, default_value = "kubectl")]
    kubectl: String,
    #[arg(long)]
    context: Option<String>,
    #[arg(long)]
    kubeconfig: Option<PathBuf>,
    #[arg(long)]
    namespace: Option<String>,
    #[arg(long, value_name = "client|server")]
    dry_run: Option<String>,
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

#[derive(Debug, Args, Clone)]
struct EphemeralJobsApplyArgs {
    #[arg(
        long,
        default_value = "infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml"
    )]
    file: PathBuf,
    #[arg(long, default_value = "kubectl")]
    kubectl: String,
    #[arg(long)]
    context: Option<String>,
    #[arg(long)]
    kubeconfig: Option<PathBuf>,
    #[arg(long)]
    namespace: Option<String>,
    #[arg(long, value_name = "client|server")]
    dry_run: Option<String>,
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
        Commands::Setup(args) => setup(args),
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
        StoreSubcommand::EventSourceApprovals(args) => {
            let mut params = Vec::new();
            if let Some(source_id) = args.source_id {
                params.push(format!("source_id={source_id}"));
            }
            if let Some(approval_ref) = args.approval_ref {
                params.push(format!("approval_ref={approval_ref}"));
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
                    "{}/goal-store/event-source-approvals{}",
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
        EventSubcommand::Webhook(args) => {
            let request: serde_json::Value = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/events/webhook/{}",
                    args.event_gateway_url.trim_end_matches('/'),
                    args.source_id
                ),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        EventSubcommand::PollSqs(args) => {
            let mut query = Vec::new();
            query.push(format!("route={}", (!args.no_route)));
            if let Some(max_messages) = args.max_messages {
                query.push(format!("max_messages={max_messages}"));
            }
            let request = serde_json::json!({});
            post_json_to_url(
                &format!(
                    "{}/events/sqs/{}/poll?{}",
                    args.event_gateway_url.trim_end_matches('/'),
                    args.source_id,
                    query.join("&")
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
        MemorySubcommand::Retract(args) => {
            let request: MemoryRetractRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/memory/retract",
                    args.memory_gateway_url.trim_end_matches('/')
                ),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        MemorySubcommand::Edit(args) => {
            let request: MemoryEditRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/memory/edit",
                    args.memory_gateway_url.trim_end_matches('/')
                ),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        MemorySubcommand::PreviewEdit(args) => {
            let request: MemoryEditPreviewRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/memory/edit/preview",
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
    if args.queue {
        return get_url(
            &format!("{}/queue", args.notifier_url.trim_end_matches('/')),
            None,
        )
        .await;
    }
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
        .context("--file is required unless --queue, --threads, or --thread-key is provided")?;
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
        source_plan_id: args.source_plan_id,
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

fn setup(args: SetupCommand) -> anyhow::Result<()> {
    match args.command {
        SetupSubcommand::LocalAuth(args) => local_auth_setup(args),
        SetupSubcommand::ChatClient(args) => chat_client_setup(args),
    }
}

fn local_auth_setup(args: LocalAuthArgs) -> anyhow::Result<()> {
    let default_action = !args.write_env && !args.check && !args.print_commands;
    if default_action {
        return interactive_local_auth_setup(args);
    }
    if args.write_env {
        write_local_provider_env(&args.output, local_provider_env_template())?;
    }
    if args.check {
        print_local_auth_checks();
    }
    if args.print_commands {
        print_local_auth_commands();
    }
    Ok(())
}

fn interactive_local_auth_setup(args: LocalAuthArgs) -> anyhow::Result<()> {
    let theme = ColorfulTheme::default();
    println!("COAT local provider setup");
    if Confirm::with_theme(&theme)
        .with_prompt("Check installed provider CLIs and relevant environment variables?")
        .default(true)
        .interact()?
    {
        print_local_auth_checks();
    }

    let profiles = [
        "OpenAI hosted models/embeddings",
        "Anthropic or Claude Code",
        "AWS Bedrock",
        "Host-local Ollama",
        "Host-local vLLM/OpenAI-compatible server",
        "Hugging Face tooling",
        "Control gateway Chat tab",
    ];
    let selections = MultiSelect::with_theme(&theme)
        .with_prompt("Select provider surfaces to prepare")
        .items(&profiles)
        .defaults(&[true, true, false, true, false, false, true])
        .interact()?;

    let mut env_text = local_provider_env_template().to_string();
    let populate_from_env = Confirm::with_theme(&theme)
        .with_prompt("Copy currently exported secret env values into the local env file?")
        .default(false)
        .interact()?;
    if populate_from_env {
        env_text = populate_secret_env_values(env_text);
    }

    if selections.contains(&3) || selections.contains(&4) {
        let local_kind_default = if selections.contains(&4) {
            "vllm"
        } else {
            "ollama"
        };
        let local_kind: String = Input::with_theme(&theme)
            .with_prompt("Local model provider kind")
            .default(local_kind_default.to_string())
            .interact_text()?;
        let local_model: String = Input::with_theme(&theme)
            .with_prompt("Local model name")
            .default(if local_kind == "vllm" {
                "local-vllm".to_string()
            } else {
                "llama3.1".to_string()
            })
            .interact_text()?;
        let local_endpoint: String = Input::with_theme(&theme)
            .with_prompt("Local OpenAI-compatible endpoint from Compose containers")
            .default(if local_kind == "vllm" {
                "http://host.docker.internal:8000/v1".to_string()
            } else {
                "http://host.docker.internal:11434/v1".to_string()
            })
            .interact_text()?;
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_KIND", &local_kind);
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_MODEL", &local_model);
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_ENDPOINT", &local_endpoint);
    }

    if selections.contains(&6) {
        let chat_choices = [
            "Use local model endpoint",
            "Use OpenAI hosted chat completions",
            "Leave chat stubbed",
        ];
        let choice = Select::with_theme(&theme)
            .with_prompt("Control gateway Chat tab backend")
            .items(&chat_choices)
            .default(2)
            .interact()?;
        match choice {
            0 => {
                let url: String = Input::with_theme(&theme)
                    .with_prompt("Chat completions URL")
                    .default("http://host.docker.internal:8000/v1/chat/completions".to_string())
                    .interact_text()?;
                let model: String = Input::with_theme(&theme)
                    .with_prompt("Chat model")
                    .default("local-chat-model".to_string())
                    .interact_text()?;
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_COMPLETIONS_URL", &url);
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_MODEL", &model);
            }
            1 => {
                let model: String = Input::with_theme(&theme)
                    .with_prompt("OpenAI chat model")
                    .default("gpt-5.4".to_string())
                    .interact_text()?;
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_MODEL", &model);
            }
            _ => {}
        }
    }

    let write_env = Confirm::with_theme(&theme)
        .with_prompt("Write local provider env file?")
        .default(true)
        .interact()?;
    if write_env {
        let output: String = Input::with_theme(&theme)
            .with_prompt("Env file path")
            .default(args.output.display().to_string())
            .interact_text()?;
        write_local_provider_env(&PathBuf::from(output), &env_text)?;
    }

    if Confirm::with_theme(&theme)
        .with_prompt("Print provider login and startup commands?")
        .default(true)
        .interact()?
    {
        print_local_auth_commands();
    }
    Ok(())
}

fn write_local_provider_env(path: &Path, env_text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, env_text)?;
    println!("wrote {}", path.display());
    println!(
        "use with: docker compose --env-file {} -f infra/compose/docker-compose.yml up --build",
        path.display()
    );
    Ok(())
}

fn populate_secret_env_values(mut env_text: String) -> String {
    for name in [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "MEMORY_GATEWAY_EMBEDDING_TOKEN",
    ] {
        if let Ok(value) = env::var(name) {
            if !value.trim().is_empty() {
                env_text = replace_env_line(env_text, name, &value);
            }
        }
    }
    env_text
}

fn replace_env_line(env_text: String, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    env_text
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                format!("{prefix}{value}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn print_local_auth_checks() {
    println!("local provider auth check");
    println!("tools:");
    for command in [
        "coat", "docker", "node", "npm", "codex", "claude", "aws", "ollama", "vllm", "hf",
    ] {
        let (available, detail) = probe_command(command);
        println!(
            "  {:<8} {} {}",
            command,
            if available { "ok" } else { "missing" },
            detail
        );
    }
    println!("environment:");
    for name in [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "AWS_PROFILE",
        "AWS_REGION",
        "AWS_DEFAULT_REGION",
        "MODEL_PROVIDER_ENDPOINT",
        "LOCAL_MODEL_PROVIDER_ENDPOINT",
        "COAT_CONTROL_CHAT_MODEL",
        "MEMORY_GATEWAY_EMBEDDING_TOKEN",
    ] {
        println!(
            "  {:<34} {}",
            name,
            if env_var_present(name) {
                "set"
            } else {
                "unset"
            }
        );
    }
    println!("secret values are intentionally not printed");
}

fn print_local_auth_commands() {
    println!("suggested local auth/setup commands:");
    println!("  codex login");
    println!("  claude login");
    println!("  aws sso login --profile <profile>");
    println!("  ollama pull llama3.1");
    println!("  vllm serve <model> --host 0.0.0.0 --port 8000");
    println!("  hf auth login");
    println!("after auth, write an env file with:");
    println!("  coat setup local-auth --write-env --output infra/compose/local-providers.env");
    println!("then start Compose with that env file:");
    println!("  coat compose up --env-file infra/compose/local-providers.env");
}

fn probe_command(command: &str) -> (bool, String) {
    match Command::new(command).arg("--version").output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if text.is_empty() {
                text = String::from_utf8_lossy(&output.stderr).trim().to_string();
            }
            let first_line = text.lines().next().unwrap_or("").trim();
            if first_line.is_empty() {
                (true, String::new())
            } else {
                (true, format!("- {first_line}"))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, String::new()),
        Err(error) => (false, format!("- {error}")),
    }
}

fn env_var_present(name: &str) -> bool {
    env::var(name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn local_provider_env_template() -> &'static str {
    include_str!("../../../infra/compose/local-providers.env.example")
}

fn chat_client_setup(args: ChatClientArgs) -> anyhow::Result<()> {
    if chat_client_default_action(&args) {
        return interactive_chat_client_setup(args);
    }
    run_chat_client_setup_actions(&args)
}

fn chat_client_default_action(args: &ChatClientArgs) -> bool {
    !args.install_codex_mcp
        && !args.install_claude_mcp
        && !args.write_claude_project_config
        && !args.write_skill
        && !args.install_codex_skill
        && !args.install_claude_skill
        && !args.print_commands
}

fn interactive_chat_client_setup(mut args: ChatClientArgs) -> anyhow::Result<()> {
    let theme = ColorfulTheme::default();
    println!("COAT chat-client setup");

    args.mcp_url = Input::with_theme(&theme)
        .with_prompt("Control gateway MCP URL")
        .default(args.mcp_url)
        .interact_text()?;
    args.server_name = Input::with_theme(&theme)
        .with_prompt("MCP server name in the chat client")
        .default(args.server_name)
        .interact_text()?;

    let auth_modes = [
        "Use bearer-token environment variable",
        "No MCP token for trusted local development",
    ];
    let auth_mode = Select::with_theme(&theme)
        .with_prompt("MCP authentication mode")
        .items(&auth_modes)
        .default(if args.no_token { 1 } else { 0 })
        .interact()?;
    args.no_token = auth_mode == 1;
    if !args.no_token {
        args.token_env = Input::with_theme(&theme)
            .with_prompt("Bearer token environment variable name")
            .default(args.token_env)
            .interact_text()?;
    }

    let scopes = ["user", "project", "local"];
    let default_scope = scopes
        .iter()
        .position(|scope| *scope == args.claude_scope)
        .unwrap_or(0);
    let scope = Select::with_theme(&theme)
        .with_prompt("Claude Code MCP scope")
        .items(&scopes)
        .default(default_scope)
        .interact()?;
    args.claude_scope = scopes[scope].to_string();

    let actions = [
        "Print install and verification commands",
        "Run Codex MCP registration now",
        "Run Claude Code MCP registration now",
        "Write Claude project .mcp.json",
        "Install Codex personal skill",
        "Install Claude Code personal skill",
        "Write skill to a custom directory",
    ];
    let selections = MultiSelect::with_theme(&theme)
        .with_prompt("Select setup actions")
        .items(&actions)
        .defaults(&[true, false, false, true, false, false, false])
        .interact()?;

    args.print_commands = selections.contains(&0);
    args.install_codex_mcp = selections.contains(&1);
    args.install_claude_mcp = selections.contains(&2);
    args.write_claude_project_config = selections.contains(&3);
    args.install_codex_skill = selections.contains(&4);
    args.install_claude_skill = selections.contains(&5);
    args.write_skill = selections.contains(&6);

    if args.write_claude_project_config {
        let path: String = Input::with_theme(&theme)
            .with_prompt("Claude project MCP config path")
            .default(args.claude_project_config.display().to_string())
            .interact_text()?;
        args.claude_project_config = PathBuf::from(path);
    }
    if args.write_skill {
        let path: String = Input::with_theme(&theme)
            .with_prompt("Custom skill directory")
            .default(args.skill_dir.display().to_string())
            .interact_text()?;
        args.skill_dir = PathBuf::from(path);
    }

    run_chat_client_setup_actions(&args)
}

fn run_chat_client_setup_actions(args: &ChatClientArgs) -> anyhow::Result<()> {
    if args.print_commands {
        print_chat_client_commands(args)?;
    }
    if args.write_skill {
        write_skill_dir(&args.skill_dir)?;
        println!("wrote skill {}", args.skill_dir.display());
    }
    if args.install_codex_skill {
        let dir = default_codex_skill_dir();
        write_skill_dir(&dir)?;
        println!("installed Codex skill {}", dir.display());
    }
    if args.install_claude_skill {
        let dir = default_claude_skill_dir();
        write_skill_dir(&dir)?;
        println!("installed Claude Code skill {}", dir.display());
    }
    if args.write_claude_project_config {
        write_claude_project_mcp_config(args)?;
        println!("wrote {}", args.claude_project_config.display());
    }
    if args.install_codex_mcp {
        run_program_args("codex", &codex_mcp_add_args(args)?)?;
    }
    if args.install_claude_mcp {
        run_program_args("claude", &claude_mcp_add_args(args)?)?;
    }
    Ok(())
}

fn print_chat_client_commands(args: &ChatClientArgs) -> anyhow::Result<()> {
    println!("COAT chat-client setup");
    println!("1. Configure provider credentials and local model endpoints:");
    println!("  coat setup local-auth --write-env --output infra/compose/local-providers.env");
    println!("2. Start a local or remote control gateway. Local example:");
    println!(
        "  docker compose --env-file infra/compose/local-providers.env -f infra/compose/docker-compose.yml up --build"
    );
    println!("3. Export the MCP token in the shell that launches the chat client:");
    if args.no_token {
        println!("  # no token requested for this install");
    } else {
        println!("  export {}=<redacted-token>", args.token_env);
    }
    println!("4. Install the remote HTTP MCP server:");
    println!("  {}", shell_command("codex", &codex_mcp_add_args(args)?));
    println!("  {}", shell_command("claude", &claude_mcp_add_args(args)?));
    println!("5. Install the COAT skill:");
    println!("  coat setup chat-client --install-codex-skill");
    println!("  coat setup chat-client --install-claude-skill");
    println!("6. Verify from the chat client:");
    println!("  codex mcp get {}", shell_quote(&args.server_name));
    println!("  claude mcp get {}", shell_quote(&args.server_name));
    println!("  # In Claude Code, run /mcp to inspect connection status.");
    Ok(())
}

fn codex_mcp_add_args(args: &ChatClientArgs) -> anyhow::Result<Vec<String>> {
    ensure_chat_client_args(args)?;
    let mut command_args = vec![
        "mcp".to_string(),
        "add".to_string(),
        "--url".to_string(),
        args.mcp_url.clone(),
    ];
    if !args.no_token {
        command_args.push("--bearer-token-env-var".to_string());
        command_args.push(args.token_env.clone());
    }
    command_args.push(args.server_name.clone());
    Ok(command_args)
}

fn claude_mcp_add_args(args: &ChatClientArgs) -> anyhow::Result<Vec<String>> {
    ensure_chat_client_args(args)?;
    if args.no_token {
        return Ok(vec![
            "mcp".to_string(),
            "add".to_string(),
            "--transport".to_string(),
            "http".to_string(),
            "--scope".to_string(),
            args.claude_scope.clone(),
            args.server_name.clone(),
            args.mcp_url.clone(),
        ]);
    }
    Ok(vec![
        "mcp".to_string(),
        "add-json".to_string(),
        "--scope".to_string(),
        args.claude_scope.clone(),
        args.server_name.clone(),
        claude_mcp_json(args)?,
    ])
}

fn ensure_chat_client_args(args: &ChatClientArgs) -> anyhow::Result<()> {
    if args.server_name.trim().is_empty() {
        bail!("--server-name cannot be empty");
    }
    if args.mcp_url.trim().is_empty() {
        bail!("--mcp-url cannot be empty");
    }
    if !args.no_token && args.token_env.trim().is_empty() {
        bail!("--token-env cannot be empty unless --no-token is set");
    }
    Ok(())
}

fn claude_mcp_json(args: &ChatClientArgs) -> anyhow::Result<String> {
    let mut server = serde_json::json!({
        "type": "http",
        "url": args.mcp_url,
    });
    if !args.no_token {
        server["headers"] = serde_json::json!({
            "Authorization": format!("Bearer ${{{}}}", args.token_env),
        });
    }
    serde_json::to_string(&server).context("serialize Claude MCP config")
}

fn write_claude_project_mcp_config(args: &ChatClientArgs) -> anyhow::Result<()> {
    ensure_chat_client_args(args)?;
    let path = expand_home_path(&args.claude_project_config)?;
    let mut root = if path.exists() {
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str::<serde_json::Value>(&raw)
            .with_context(|| format!("parse {}", path.display()))?
    } else {
        serde_json::json!({})
    };
    if !root.is_object() {
        bail!("{} must contain a JSON object", path.display());
    }
    if root.get("mcpServers").is_none() {
        root["mcpServers"] = serde_json::json!({});
    }
    if !root["mcpServers"].is_object() {
        bail!("{}.mcpServers must be a JSON object", path.display());
    }
    root["mcpServers"][args.server_name.as_str()] =
        serde_json::from_str(&claude_mcp_json(args)?).context("parse Claude MCP server JSON")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&root)?))?;
    Ok(())
}

fn write_skill_dir(path: &Path) -> anyhow::Result<()> {
    let path = expand_home_path(path)?;
    fs::create_dir_all(&path)?;
    fs::write(path.join("SKILL.md"), coat_control_plane_skill())?;
    Ok(())
}

fn default_codex_skill_dir() -> PathBuf {
    env::var("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|_| env::var("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .unwrap_or_else(|_| PathBuf::from(".codex"))
        .join("skills")
        .join("coat-control-plane")
}

fn default_claude_skill_dir() -> PathBuf {
    env::var("HOME")
        .map(|home| PathBuf::from(home).join(".claude"))
        .unwrap_or_else(|_| PathBuf::from(".claude"))
        .join("skills")
        .join("coat-control-plane")
}

fn expand_home_path(path: &Path) -> anyhow::Result<PathBuf> {
    let value = path.display().to_string();
    if value == "~" {
        return env::var("HOME")
            .map(PathBuf::from)
            .context("HOME is not set for ~ expansion");
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return env::var("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .context("HOME is not set for ~ expansion");
    }
    Ok(path.to_path_buf())
}

fn run_program_args(program: &str, args: &[String]) -> anyhow::Result<()> {
    println!("{}", shell_command(program, args));
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("run {program}"))?;
    if !status.success() {
        bail!("{program} exited with {status}");
    }
    Ok(())
}

fn shell_command(program: &str, args: &[String]) -> String {
    std::iter::once(shell_quote(program))
        .chain(args.iter().map(|arg| shell_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn coat_control_plane_skill() -> &'static str {
    include_str!("../../../skills/coat-control-plane/SKILL.md")
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
    match args.command {
        ComposeSubcommand::Up(args) => {
            if args.restate_cloud {
                ensure_restate_cloud_env_file(&args.restate_cloud_env_file, args.init_env)?;
                if args.init_env {
                    return Ok(());
                }
            }
            let register_cloud = args.register_cloud;
            let tunnel_name = args.tunnel_name.clone();
            let service_url = args.service_url.clone();
            run_docker_compose(compose_up_command_args(&args), "run docker compose up")?;
            if register_cloud {
                restate_register_cloud(RestateRegisterCloudArgs {
                    tunnel_name,
                    service_url,
                    dry_run: false,
                })?;
            }
            Ok(())
        }
        ComposeSubcommand::Config(args) => {
            if args.restate_cloud {
                ensure_restate_cloud_env_file(&args.restate_cloud_env_file, false)?;
            }
            run_docker_compose(
                compose_config_command_args(&args),
                "run docker compose config",
            )
        }
        ComposeSubcommand::Down(args) => {
            run_docker_compose(compose_down_command_args(&args), "run docker compose down")
        }
    }
}

fn run_docker_compose(args: Vec<String>, description: &str) -> anyhow::Result<()> {
    let status = Command::new("docker")
        .args(&args)
        .status()
        .with_context(|| description.to_string())?;
    if !status.success() {
        bail!("docker compose exited with {status}");
    }
    Ok(())
}

fn compose_base_args(
    restate_cloud: bool,
    restate_cloud_env_file: &Path,
    env_files: &[PathBuf],
    profiles: &[String],
) -> Vec<String> {
    let mut args = vec!["compose".to_string()];
    if restate_cloud {
        args.push("--env-file".to_string());
        args.push(restate_cloud_env_file.display().to_string());
    }
    for env_file in env_files {
        args.push("--env-file".to_string());
        args.push(env_file.display().to_string());
    }
    args.push("-f".to_string());
    args.push("infra/compose/docker-compose.yml".to_string());
    if restate_cloud {
        args.push("-f".to_string());
        args.push("infra/compose/docker-compose.restate-cloud.yml".to_string());
        args.push("--profile".to_string());
        args.push("restate-cloud".to_string());
    }
    for profile in profiles {
        args.push("--profile".to_string());
        args.push(profile.clone());
    }
    args
}

fn compose_up_command_args(args: &ComposeUpArgs) -> Vec<String> {
    let mut command_args = compose_base_args(
        args.restate_cloud,
        &args.restate_cloud_env_file,
        &args.env_file,
        &args.profile,
    );
    command_args.push("up".to_string());
    command_args.push("--build".to_string());
    if args.detach || args.register_cloud {
        command_args.push("--detach".to_string());
    }
    command_args.extend(args.services.iter().cloned());
    command_args
}

fn compose_config_command_args(args: &ComposeConfigArgs) -> Vec<String> {
    let mut command_args = compose_base_args(
        args.restate_cloud,
        &args.restate_cloud_env_file,
        &args.env_file,
        &args.profile,
    );
    command_args.push("config".to_string());
    command_args
}

fn compose_down_command_args(args: &ComposeDownArgs) -> Vec<String> {
    let mut command_args = compose_base_args(
        args.restate_cloud,
        &args.restate_cloud_env_file,
        &args.env_file,
        &[],
    );
    command_args.push("down".to_string());
    command_args
}

fn ensure_restate_cloud_env_file(env_file: &Path, init_only: bool) -> anyhow::Result<()> {
    let example = Path::new("infra/compose/restate-cloud.env.example");
    if !env_file.exists() {
        if let Some(parent) = env_file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(example, env_file)
            .with_context(|| format!("copy {} to {}", example.display(), env_file.display()))?;
        println!(
            "created {} from {}; fill in Restate Cloud values and rerun",
            env_file.display(),
            example.display()
        );
        if !init_only {
            bail!(
                "{} contains placeholders; edit it with RESTATE_ENVIRONMENT_ID, RESTATE_BEARER_TOKEN, RESTATE_CLOUD_REGION, and RESTATE_SIGNING_PUBLIC_KEY, then rerun `coat compose up --restate-cloud`",
                env_file.display()
            );
        }
        return Ok(());
    }

    if init_only {
        println!("{} already exists", env_file.display());
        return Ok(());
    }

    let content =
        fs::read_to_string(env_file).with_context(|| format!("read {}", env_file.display()))?;
    let placeholders = restate_cloud_env_placeholders(&content);
    if !placeholders.is_empty() {
        bail!(
            "{} still has placeholder Restate Cloud values: {}",
            env_file.display(),
            placeholders.join(", ")
        );
    }
    Ok(())
}

fn restate_cloud_env_placeholders(content: &str) -> Vec<&'static str> {
    let mut placeholders = Vec::new();
    if content.contains("RESTATE_ENVIRONMENT_ID=env_...") {
        placeholders.push("RESTATE_ENVIRONMENT_ID");
    }
    if content.contains("RESTATE_BEARER_TOKEN=replace-me") {
        placeholders.push("RESTATE_BEARER_TOKEN");
    }
    if content.contains("RESTATE_SIGNING_PUBLIC_KEY=publickeyv1_...") {
        placeholders.push("RESTATE_SIGNING_PUBLIC_KEY");
    }
    if content.contains("RESTATE_IDENTITY_KEYS=publickeyv1_...") {
        placeholders.push("RESTATE_IDENTITY_KEYS");
    }
    placeholders
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
        K8sSubcommand::Apply(args) => apply_k8s_manifest(args),
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
            EphemeralJobsSubcommand::Apply(args) => apply_ephemeral_jobs(args),
        },
    }
}

fn apply_k8s_manifest(args: K8sApplyArgs) -> anyhow::Result<()> {
    if !args.file.exists() {
        bail!("{} does not exist; pass --file or run `coat k8s render --output {}` first", args.file.display(), args.file.display());
    }
    let command_args = kubectl_apply_args(KubectlApplySpec {
        file: args.file,
        context: args.context,
        kubeconfig: args.kubeconfig,
        namespace: args.namespace,
        dry_run: args.dry_run,
    })?;
    println!("{} {}", shell_quote(&args.kubectl), command_args.join(" "));
    let status = Command::new(&args.kubectl)
        .args(&command_args)
        .status()
        .context("run kubectl apply for COAT Kubernetes manifests")?;
    if !status.success() {
        bail!("kubectl apply exited with {status}");
    }
    Ok(())
}

fn apply_ephemeral_jobs(args: EphemeralJobsApplyArgs) -> anyhow::Result<()> {
    if !args.file.exists() {
        bail!(
            "{} does not exist; run `coat k8s ephemeral-jobs render --output {}` first or pass --file",
            args.file.display(),
            args.file.display()
        );
    }
    let command_args = kubectl_ephemeral_jobs_apply_args(&args)?;
    println!("{} {}", shell_quote(&args.kubectl), command_args.join(" "));
    let status = Command::new(&args.kubectl)
        .args(&command_args)
        .status()
        .context("run kubectl apply for ephemeral runner jobs")?;
    if !status.success() {
        bail!("kubectl apply exited with {status}");
    }
    Ok(())
}

#[derive(Debug)]
struct KubectlApplySpec {
    file: PathBuf,
    context: Option<String>,
    kubeconfig: Option<PathBuf>,
    namespace: Option<String>,
    dry_run: Option<String>,
}

fn kubectl_apply_args(spec: KubectlApplySpec) -> anyhow::Result<Vec<String>> {
    let mut command_args = Vec::new();
    if let Some(context) = spec.context.as_deref() {
        command_args.push("--context".to_string());
        command_args.push(context.to_string());
    }
    if let Some(kubeconfig) = &spec.kubeconfig {
        command_args.push("--kubeconfig".to_string());
        command_args.push(kubeconfig.display().to_string());
    }
    if let Some(namespace) = spec.namespace.as_deref() {
        command_args.push("--namespace".to_string());
        command_args.push(namespace.to_string());
    }
    command_args.push("apply".to_string());
    command_args.push("-f".to_string());
    command_args.push(spec.file.display().to_string());
    if let Some(dry_run) = spec.dry_run.as_deref() {
        match dry_run {
            "client" | "server" => command_args.push(format!("--dry-run={dry_run}")),
            other => bail!("--dry-run must be client or server, got {other:?}"),
        }
    }
    Ok(command_args)
}

fn kubectl_ephemeral_jobs_apply_args(args: &EphemeralJobsApplyArgs) -> anyhow::Result<Vec<String>> {
    kubectl_apply_args(KubectlApplySpec {
        file: args.file.clone(),
        context: args.context.clone(),
        kubeconfig: args.kubeconfig.clone(),
        namespace: args.namespace.clone(),
        dry_run: args.dry_run.clone(),
    })
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
        ChatClientArgs, ComposeConfigArgs, ComposeUpArgs, EphemeralJobsApplyArgs,
        KubectlApplySpec, bump_release_versions, chat_client_default_action, claude_mcp_json, codex_mcp_add_args,
        compose_config_command_args, compose_up_command_args, ensure_json_goal_id,
        extract_follow_ups, kubectl_apply_args, kubectl_ephemeral_jobs_apply_args, latest_goal_id_from_value,
        release_plan_json, replace_env_line, replace_toml_section_value, replace_yaml_root_value,
        restate_cloud_env_placeholders,
    };
    use std::path::PathBuf;
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
    fn kubectl_ephemeral_jobs_apply_args_are_explicit_and_dry_run_safe() {
        let args = EphemeralJobsApplyArgs {
            file: PathBuf::from("infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml"),
            kubectl: "kubectl".to_string(),
            context: Some("dev-cluster".to_string()),
            kubeconfig: Some(PathBuf::from("/tmp/kubeconfig")),
            namespace: Some("jattg-ephemeral".to_string()),
            dry_run: Some("client".to_string()),
        };

        assert_eq!(
            kubectl_ephemeral_jobs_apply_args(&args).expect("apply args"),
            vec![
                "--context",
                "dev-cluster",
                "--kubeconfig",
                "/tmp/kubeconfig",
                "--namespace",
                "jattg-ephemeral",
                "apply",
                "-f",
                "infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml",
                "--dry-run=client",
            ]
        );
    }

    #[test]
    fn kubectl_base_apply_args_support_context_namespace_and_dry_run() {
        let args = KubectlApplySpec {
            file: PathBuf::from("infra/k8s/base/all.yaml"),
            context: Some("prod".to_string()),
            kubeconfig: Some(PathBuf::from("/tmp/kubeconfig")),
            namespace: Some("jattg".to_string()),
            dry_run: Some("server".to_string()),
        };

        assert_eq!(
            kubectl_apply_args(args).expect("apply args"),
            vec![
                "--context",
                "prod",
                "--kubeconfig",
                "/tmp/kubeconfig",
                "--namespace",
                "jattg",
                "apply",
                "-f",
                "infra/k8s/base/all.yaml",
                "--dry-run=server",
            ]
        );
    }

    #[test]
    fn compose_restate_cloud_register_uses_detached_tunnel_profile() {
        let args = ComposeUpArgs {
            restate_cloud: true,
            restate_cloud_env_file: PathBuf::from("infra/compose/restate-cloud.env"),
            env_file: vec![PathBuf::from("infra/compose/local-providers.env")],
            profile: vec!["db".to_string()],
            detach: false,
            register_cloud: true,
            init_env: false,
            tunnel_name: "jattg-personal".to_string(),
            service_url: "http://coordinator:9080".to_string(),
            services: vec!["coordinator".to_string()],
        };

        assert_eq!(
            compose_up_command_args(&args),
            vec![
                "compose",
                "--env-file",
                "infra/compose/restate-cloud.env",
                "--env-file",
                "infra/compose/local-providers.env",
                "-f",
                "infra/compose/docker-compose.yml",
                "-f",
                "infra/compose/docker-compose.restate-cloud.yml",
                "--profile",
                "restate-cloud",
                "--profile",
                "db",
                "up",
                "--build",
                "--detach",
                "coordinator",
            ]
        );
    }

    #[test]
    fn compose_config_wraps_restate_cloud_override() {
        let args = ComposeConfigArgs {
            restate_cloud: true,
            restate_cloud_env_file: PathBuf::from("infra/compose/restate-cloud.env"),
            env_file: Vec::new(),
            profile: Vec::new(),
        };

        assert_eq!(
            compose_config_command_args(&args),
            vec![
                "compose",
                "--env-file",
                "infra/compose/restate-cloud.env",
                "-f",
                "infra/compose/docker-compose.yml",
                "-f",
                "infra/compose/docker-compose.restate-cloud.yml",
                "--profile",
                "restate-cloud",
                "config",
            ]
        );
    }

    #[test]
    fn restate_cloud_env_placeholder_detection_blocks_unsafe_up() {
        let placeholders = restate_cloud_env_placeholders(
            "RESTATE_ENVIRONMENT_ID=env_...\nRESTATE_BEARER_TOKEN=replace-me\nRESTATE_SIGNING_PUBLIC_KEY=publickeyv1_...\nRESTATE_IDENTITY_KEYS=publickeyv1_...\n",
        );

        assert_eq!(
            placeholders,
            vec![
                "RESTATE_ENVIRONMENT_ID",
                "RESTATE_BEARER_TOKEN",
                "RESTATE_SIGNING_PUBLIC_KEY",
                "RESTATE_IDENTITY_KEYS",
            ]
        );
        assert!(restate_cloud_env_placeholders(
            "RESTATE_ENVIRONMENT_ID=env_123\nRESTATE_BEARER_TOKEN=secret\nRESTATE_SIGNING_PUBLIC_KEY=publickeyv1_real\nRESTATE_IDENTITY_KEYS=publickeyv1_real\n",
        )
        .is_empty());
    }

    #[test]
    fn replace_env_line_updates_existing_key_without_touching_others() {
        let env_text = "OPENAI_API_KEY=\nLOCAL_MODEL_PROVIDER_KIND=ollama\nOTHER=value\n";
        let updated = replace_env_line(env_text.to_string(), "LOCAL_MODEL_PROVIDER_KIND", "vllm");

        assert_eq!(
            updated,
            "OPENAI_API_KEY=\nLOCAL_MODEL_PROVIDER_KIND=vllm\nOTHER=value\n"
        );
    }

    #[test]
    fn chat_client_setup_args_generate_codex_and_claude_mcp_configs() {
        let args = ChatClientArgs {
            mcp_url: "http://localhost:9090/mcp".to_string(),
            server_name: "coat-control".to_string(),
            token_env: "COAT_CONTROL_MCP_TOKEN".to_string(),
            no_token: false,
            claude_scope: "user".to_string(),
            install_codex_mcp: false,
            install_claude_mcp: false,
            write_claude_project_config: false,
            claude_project_config: PathBuf::from(".mcp.json"),
            write_skill: false,
            skill_dir: PathBuf::from(".claude/skills/coat-control-plane"),
            install_codex_skill: false,
            install_claude_skill: false,
            print_commands: false,
        };

        assert!(chat_client_default_action(&args));

        let codex_args = codex_mcp_add_args(&args).expect("codex mcp args");
        assert_eq!(
            codex_args,
            [
                "mcp",
                "add",
                "--url",
                "http://localhost:9090/mcp",
                "--bearer-token-env-var",
                "COAT_CONTROL_MCP_TOKEN",
                "coat-control"
            ]
            .map(String::from)
            .to_vec()
        );

        let claude_json: serde_json::Value =
            serde_json::from_str(&claude_mcp_json(&args).expect("claude mcp json"))
                .expect("parse claude json");
        assert_eq!(claude_json["type"], "http");
        assert_eq!(claude_json["url"], "http://localhost:9090/mcp");
        assert_eq!(
            claude_json["headers"]["Authorization"],
            "Bearer ${COAT_CONTROL_MCP_TOKEN}"
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
