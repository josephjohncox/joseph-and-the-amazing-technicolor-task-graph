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
    sync::OnceLock,
};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use coat_domain::{
    BranchRequest, BranchSelectionRequest, ChildTaskRequest, CoatCliConfig, CoatCloudConfig,
    CoatConfig, CoatConfigPaths, CoatKubernetesConfig, CoatLocalDeployConfig, CoatOperatorDefaults,
    CoatProfileConfig, CoatProjectConfig, CoatRestateCloudConfig, CoatServiceEndpoints,
    CoatUserConfig, ControlLoopMode, EventSource, ExternalEvent, GoalAuthoringGuidance, GoalPlan,
    GoalRecord, GoalSpec, GraphColorRef, HumanApproval, MemoryContextRequest,
    MemoryEditPreviewRequest, MemoryEditRequest, MemoryJoinRequest, MemoryRepairRequest,
    MemoryRetractRequest, MemorySearchRequest, MemoryWriteRequest, NetworkAccess,
    NotificationRequest, PlanCandidateSelectionRequest, PlanCandidateVoteRequest,
    PlanCompileRequest, PlanDraftRequest, PlanQuestion, PlanQuestionStatus, PlanRevisionRequest,
    PlanningMode, RestartRequest, ReviewDoctrine, ReviewDoctrinePreset, RunnerDispatchRequest,
    RunnerRegistration, SandboxLaunchPlan, SandboxResourcePlan, SandboxSecurityPlan,
    StandardReviewCheck, SteeringDirective, SteeringDirectiveKind, SubgoalSpec, TaskPriority,
    TaskPurpose, TaskPurposeKind, TaskQuery, TaskStatus, TriggeredGoalRequest, WorkerKind,
};
use dialoguer::{Confirm, Input, MultiSelect, Select, theme::ColorfulTheme};
use uuid::Uuid;

const COAT_PROJECT_MARKER: &str = ".coat/project.json";
const DEFAULT_USER_CONFIG: &str = "~/.coat/config.json";
const DEFAULT_LOCAL_PROVIDER_ENV: &str = "infra/compose/local-providers.env";
const DEFAULT_RESTATE_CLOUD_ENV: &str = "infra/compose/restate-cloud.env";
const DEFAULT_K8S_MANIFEST: &str = "infra/k8s/base/all.yaml";
const DEFAULT_K8S_RENDERED_MANIFEST: &str = "infra/k8s/rendered.yaml";
const DEFAULT_K8S_NAMESPACE: &str = "jattg";
const DEFAULT_RESTATE_TUNNEL_NAME: &str = "jattg-personal";
const DEFAULT_RESTATE_REGION: &str = "us";
const DEFAULT_RESTATE_SERVICE_URL: &str = "http://coordinator:9080";
const DEFAULT_RESTATE_LOCAL_INGRESS: &str = "http://localhost:18080";
const DEFAULT_RESTATE_LOCAL_ADMIN: &str = "http://localhost:19070";
const DEFAULT_COORDINATOR_URL: &str = "http://localhost:9080";
const DEFAULT_RESTATE_INGRESS: &str = "http://localhost:8080";
const DEFAULT_SANDBOX_RUNNER_URL: &str = "http://localhost:9083";
const DEFAULT_RUNNER_REGISTRY_URL: &str = "http://localhost:9085";
const DEFAULT_NOTIFIER_URL: &str = "http://localhost:9086";
const DEFAULT_MEMORY_GATEWAY_URL: &str = "http://localhost:9087";
const DEFAULT_GOAL_STORE_URL: &str = "http://localhost:9088";
const DEFAULT_EVENT_GATEWAY_URL: &str = "http://localhost:9089";
const DEFAULT_CONTROL_MCP_URL: &str = "http://localhost:9090/mcp";

static CONFIG_PROFILE_OVERRIDE: OnceLock<Option<String>> = OnceLock::new();

#[derive(Debug, Parser)]
#[command(name = "coat")]
#[command(about = "COAT operator CLI for Joseph and the Amazing Technicolor Task Graph")]
#[command(
    long_about = "COAT — Coordinator Of Agentic Tasks — is the operator CLI for goals, plans, humans, memory, runners, events, and deployment workflows."
)]
struct Cli {
    #[arg(
        long,
        global = true,
        env = "COAT_PROFILE",
        help = "Select a COAT config profile without editing env or config files"
    )]
    config_profile: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(about = "Open a guided dialogue for common operator workflows")]
    Guide(GuideArgs),
    #[command(about = "Plan, inspect, and compile durable planning artifacts")]
    Plan(PlanCommand),
    #[command(about = "Submit, steer, inspect, branch, and cancel durable goals")]
    Goal(GoalCommand),
    #[command(about = "Respond to human approval and notification queues")]
    Human(HumanCommand),
    #[command(about = "Manage local, Kubernetes, Helm, and Restate deployment workflows")]
    Deploy(DeployCommand),
    #[command(about = "Manage external events, webhooks, schedules, and event buses")]
    Event(EventCommand),
    #[command(about = "Register, inspect, and test distributed runners")]
    Runner(RunnerCommand),
    #[command(about = "Write, search, join, edit, repair, and inspect durable memory")]
    Memory(MemoryCommand),
    #[command(about = "Inspect goal-store projections and audit records")]
    Store(StoreCommand),
    #[command(about = "Plan, create, snapshot, and clean sandbox workspaces")]
    Sandbox(SandboxCommand),
    #[command(about = "Plan, bump, and cut binary or chart releases")]
    Release(ReleaseCommand),
    #[command(about = "Configure provider auth and chat-client MCP integrations")]
    Setup(SetupCommand),
    Init(InitArgs),
}

#[derive(Debug, Args)]
struct GuideArgs {
    #[arg(long)]
    print: bool,
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long, default_value = ".")]
    path: PathBuf,
    #[arg(long)]
    force: bool,
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
    VoteCandidate(PlanVoteCandidateArgs),
    SelectCandidate(PlanSelectCandidateArgs),
    FollowUps(FollowUpsArgs),
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
struct PlanVoteCandidateArgs {
    #[command(flatten)]
    store: PlanStoreArgs,
    #[arg(long)]
    plan_id: Uuid,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct PlanSelectCandidateArgs {
    #[command(flatten)]
    store: PlanStoreArgs,
    #[arg(long)]
    plan_id: Uuid,
    #[arg(long)]
    file: PathBuf,
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
struct HelmCommand {
    #[command(subcommand)]
    command: HelmSubcommand,
}

#[derive(Debug, Subcommand)]
enum HelmSubcommand {
    Lint(HelmLintArgs),
    Template(HelmTemplateArgs),
    Upgrade(HelmUpgradeArgs),
    Rollback(HelmRollbackArgs),
    Package(HelmPackageArgs),
}

#[derive(Debug, Args)]
struct HelmLintArgs {
    #[arg(long, default_value = "helm")]
    helm: String,
    #[arg(long, default_value = "infra/helm/jattg")]
    chart: PathBuf,
    #[arg(short = 'f', long = "values")]
    values: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct HelmTemplateArgs {
    #[arg(long, default_value = "helm")]
    helm: String,
    #[arg(long, default_value = "jattg")]
    release: String,
    #[arg(long, default_value = "infra/helm/jattg")]
    chart: PathBuf,
    #[arg(short = 'f', long = "values")]
    values: Vec<PathBuf>,
    #[arg(long = "set")]
    set_values: Vec<String>,
    #[arg(long)]
    namespace: Option<String>,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long)]
    include_crds: bool,
}

#[derive(Debug, Args)]
struct HelmUpgradeArgs {
    #[arg(long, default_value = "helm")]
    helm: String,
    #[arg(long, default_value = "jattg")]
    release: String,
    #[arg(long, default_value = "infra/helm/jattg")]
    chart: PathBuf,
    #[arg(short = 'f', long = "values")]
    values: Vec<PathBuf>,
    #[arg(long = "set")]
    set_values: Vec<String>,
    #[arg(long, default_value = "jattg")]
    namespace: String,
    #[arg(long)]
    no_create_namespace: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    wait: bool,
    #[arg(long)]
    timeout: Option<String>,
}

#[derive(Debug, Args)]
struct HelmRollbackArgs {
    #[arg(long, default_value = "helm")]
    helm: String,
    #[arg(long, default_value = "jattg")]
    release: String,
    #[arg(long)]
    revision: Option<u32>,
    #[arg(long, default_value = "jattg")]
    namespace: String,
    #[arg(long)]
    wait: bool,
    #[arg(long)]
    timeout: Option<String>,
}

#[derive(Debug, Args)]
struct HelmPackageArgs {
    #[arg(long, default_value = "scripts/package-helm-chart.sh")]
    script: PathBuf,
    #[arg(long, default_value = "infra/helm/jattg")]
    chart_dir: PathBuf,
    #[arg(long, default_value = "dist/helm")]
    dist_dir: PathBuf,
    #[arg(long)]
    chart_version: Option<String>,
    #[arg(long)]
    app_version: Option<String>,
    #[arg(long)]
    release_url: Option<String>,
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
    Config(ConfigSetupArgs),
    LocalAuth(LocalAuthArgs),
    ChatClient(ChatClientArgs),
}

#[derive(Debug, Args)]
struct ConfigSetupArgs {
    #[arg(long)]
    write_project: bool,
    #[arg(long)]
    write_user: bool,
    #[arg(long)]
    show: bool,
    #[arg(long)]
    list_profiles: bool,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    force: bool,
    #[arg(long, default_value = ".")]
    project_root: PathBuf,
    #[arg(long, default_value = DEFAULT_USER_CONFIG)]
    user_config: PathBuf,
}

#[derive(Debug, Args)]
struct HumanCommand {
    #[command(subcommand)]
    command: HumanSubcommand,
}

#[derive(Debug, Subcommand)]
enum HumanSubcommand {
    Approve(ApproveArgs),
    Notify(NotifyArgs),
}

#[derive(Debug, Args)]
struct DeployCommand {
    #[command(subcommand)]
    command: DeploySubcommand,
}

#[derive(Debug, Subcommand)]
enum DeploySubcommand {
    Local(ComposeCommand),
    Cluster(K8sCommand),
    Chart(HelmCommand),
    Restate(RestateCommand),
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

impl Default for ChatClientArgs {
    fn default() -> Self {
        Self {
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
        }
    }
}

#[derive(Debug, Args)]
struct ComposeCommand {
    #[command(subcommand)]
    command: ComposeSubcommand,
}

#[derive(Debug, Subcommand)]
enum ComposeSubcommand {
    Preflight(ComposePreflightArgs),
    Up(ComposeUpArgs),
    Config(ComposeConfigArgs),
    Down(ComposeDownArgs),
}

#[derive(Debug, Args, Clone)]
struct ComposePreflightArgs {
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
    allow_uninitialized: bool,
    #[arg(long)]
    allow_stub_runners: bool,
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
    #[arg(long)]
    skip_preflight: bool,
    #[arg(long)]
    allow_uninitialized: bool,
    #[arg(long)]
    allow_stub_runners: bool,
    #[arg(long, env = "RESTATE_TUNNEL_NAME", default_value = "jattg-personal")]
    tunnel_name: String,
    #[arg(long, default_value = "http://coordinator:9080")]
    service_url: String,
    #[arg(value_name = "SERVICE")]
    services: Vec<String>,
}

impl Default for ComposeUpArgs {
    fn default() -> Self {
        Self {
            restate_cloud: false,
            restate_cloud_env_file: PathBuf::from("infra/compose/restate-cloud.env"),
            env_file: Vec::new(),
            profile: Vec::new(),
            detach: false,
            register_cloud: false,
            init_env: false,
            skip_preflight: false,
            allow_uninitialized: false,
            allow_stub_runners: false,
            tunnel_name: "jattg-personal".to_string(),
            service_url: "http://coordinator:9080".to_string(),
            services: Vec::new(),
        }
    }
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
    #[arg(long)]
    allow_placeholder_env: bool,
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
    Status(K8sStatusArgs),
    EphemeralJobs(EphemeralJobsCommand),
    ExecutorJob(ExecutorJobCommand),
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
struct ExecutorJobCommand {
    #[command(subcommand)]
    command: ExecutorJobSubcommand,
}

#[derive(Debug, Subcommand)]
enum ExecutorJobSubcommand {
    Render(ExecutorJobRenderArgs),
    Apply(ExecutorJobApplyArgs),
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

#[derive(Debug, Args, Clone)]
struct K8sStatusArgs {
    #[arg(long, default_value = "kubectl")]
    kubectl: String,
    #[arg(long)]
    context: Option<String>,
    #[arg(long)]
    kubeconfig: Option<PathBuf>,
    #[arg(long, default_value = "jattg")]
    namespace: String,
    #[arg(long)]
    timeout: Option<String>,
    #[arg(value_name = "DEPLOYMENT")]
    deployment: Vec<String>,
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

#[derive(Debug, Args, Clone)]
struct ExecutorJobRenderArgs {
    #[arg(long = "launch-plan")]
    launch_plan: PathBuf,
    #[arg(long, default_value = "infra/k8s/rendered-sandbox-executor-job.json")]
    output: PathBuf,
    #[arg(long, default_value = "jattg-sandboxes")]
    namespace: String,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    image: Option<String>,
    #[arg(long, default_value = "jattg-sandbox-task")]
    service_account: String,
    #[arg(long)]
    runtime_class: Option<String>,
    #[arg(long)]
    workspace_pvc: Option<String>,
    #[arg(long, default_value = "/workspace")]
    workspace_mount_path: String,
    #[arg(long, default_value_t = 0)]
    backoff_limit: i32,
    #[arg(long, default_value_t = 3600)]
    active_deadline_seconds: i64,
    #[arg(long, default_value_t = 3600)]
    ttl_seconds_after_finished: i32,
    #[arg(long = "executor-command")]
    executor_command: Vec<String>,
    #[arg(long = "env")]
    env: Vec<String>,
    #[arg(long = "label")]
    label: Vec<String>,
    #[arg(long = "annotation")]
    annotation: Vec<String>,
}

#[derive(Debug, Args, Clone)]
struct ExecutorJobApplyArgs {
    #[command(flatten)]
    render: ExecutorJobRenderArgs,
    #[arg(long, default_value = "kubectl")]
    kubectl: String,
    #[arg(long)]
    context: Option<String>,
    #[arg(long)]
    kubeconfig: Option<PathBuf>,
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
    let cli = Cli::parse();
    let _ = CONFIG_PROFILE_OVERRIDE.set(cli.config_profile.clone());
    match cli
        .command
        .unwrap_or(Commands::Guide(GuideArgs { print: false }))
    {
        command => {
            warn_if_project_not_initialized(&command)?;
            match command {
                Commands::Guide(args) => guide(args).await,
                Commands::Init(args) => init(args),
                Commands::Plan(args) => plan(args).await,
                Commands::Goal(args) => goal(args).await,
                Commands::Human(args) => human(args).await,
                Commands::Deploy(args) => deploy(args),
                Commands::Event(args) => event(args).await,
                Commands::Runner(args) => runner(args).await,
                Commands::Memory(args) => memory(args).await,
                Commands::Store(args) => store(args).await,
                Commands::Sandbox(args) => sandbox(args).await,
                Commands::Release(args) => release(args),
                Commands::Setup(args) => setup(args),
            }
        }
    }
}

async fn guide(args: GuideArgs) -> anyhow::Result<()> {
    if args.print {
        print_command_map();
        return Ok(());
    }

    let theme = ColorfulTheme::default();
    println!("COAT guided operator dialogue");
    let choices = [
        "Draft a durable plan JSON",
        "Draft a durable goal JSON",
        "Inspect latest goal progress",
        "Show the human queue",
        "Approve or reject a request",
        "Start the local Compose stack",
        "Configure COAT project/user config",
        "Configure local provider auth",
        "Install chat-client MCP/skill integration",
        "Show active plan follow-ups",
        "Print command map",
    ];
    let choice = Select::with_theme(&theme)
        .with_prompt("What do you want to do?")
        .items(&choices)
        .default(0)
        .interact()?;

    match choice {
        0 => guided_plan_draft(&theme).await,
        1 => guided_goal_draft(&theme),
        2 => {
            goal(GoalCommand {
                command: GoalSubcommand::Progress(GoalIdArgs {
                    restate_ingress: "http://localhost:8080".to_string(),
                    selector: GoalSelectorArgs {
                        goal_id: None,
                        latest: true,
                        goal_store_url: "http://localhost:9088".to_string(),
                    },
                }),
            })
            .await
        }
        3 => {
            human(HumanCommand {
                command: HumanSubcommand::Notify(NotifyArgs {
                    notifier_url: "http://localhost:9086".to_string(),
                    file: None,
                    threads: false,
                    queue: true,
                    thread_key: None,
                }),
            })
            .await
        }
        4 => guided_approval(&theme).await,
        5 => {
            if Confirm::with_theme(&theme)
                .with_prompt("Run `coat deploy local up --allow-stub-runners` now?")
                .default(true)
                .interact()?
            {
                let mut args = ComposeUpArgs::default();
                args.allow_stub_runners = true;
                deploy(DeployCommand {
                    command: DeploySubcommand::Local(ComposeCommand {
                        command: ComposeSubcommand::Up(args),
                    }),
                })
            } else {
                println!("Run later with: coat deploy local preflight");
                println!("Then run: coat deploy local up --allow-stub-runners");
                Ok(())
            }
        }
        6 => setup(SetupCommand {
            command: SetupSubcommand::Config(ConfigSetupArgs {
                write_project: false,
                write_user: false,
                show: false,
                list_profiles: false,
                profile: None,
                force: false,
                project_root: PathBuf::from("."),
                user_config: PathBuf::from(DEFAULT_USER_CONFIG),
            }),
        }),
        7 => setup(SetupCommand {
            command: SetupSubcommand::LocalAuth(LocalAuthArgs {
                output: PathBuf::from("infra/compose/local-providers.env"),
                write_env: false,
                check: false,
                print_commands: false,
            }),
        }),
        8 => setup(SetupCommand {
            command: SetupSubcommand::ChatClient(ChatClientArgs::default()),
        }),
        9 => {
            plan(PlanCommand {
                command: PlanSubcommand::FollowUps(FollowUpsArgs {
                    dir: PathBuf::from("docs/exec-plans/active"),
                    json: false,
                    include_empty: false,
                }),
            })
            .await
        }
        _ => {
            print_command_map();
            Ok(())
        }
    }
}

fn print_command_map() {
    println!("COAT command map");
    println!("  coat guide                         interactive workflow picker");
    println!("  coat plan <draft|list|show|revise|compile|follow-ups>");
    println!("  coat goal <draft|lint|submit|list|progress|tasks|steer|branch|restart|cancel>");
    println!("  coat human <approve|notify>");
    println!("  coat deploy local <preflight|up|config|down>");
    println!("  coat deploy cluster <render|apply|status|ephemeral-jobs|executor-job>");
    println!("  coat deploy chart <lint|template|upgrade|rollback|package>");
    println!("  coat deploy restate <cloud-env|tunnel-docker|register-cloud>");
    println!("  coat runner <list|status|register|dispatch>");
    println!("  coat memory <write|search|context|join|retract|edit|preview-edit|repair|events>");
    println!("  coat event <sources|register|ingest|emit|webhook|poll-sqs|trigger|triggers>");
    println!("  coat store <policy|goals|plans|tasks|events|artifacts|checkpoints|approvals>");
    println!("  coat setup <config|local-auth|chat-client>");
}

async fn guided_plan_draft(theme: &ColorfulTheme) -> anyhow::Result<()> {
    let title: String = Input::with_theme(theme)
        .with_prompt("Plan title")
        .interact_text()?;
    let objective: String = Input::with_theme(theme)
        .with_prompt("Objective")
        .interact_text()?;
    let output: String = Input::with_theme(theme)
        .with_prompt("Output JSON path")
        .default("examples/plan-draft-from-guide.json".to_string())
        .interact_text()?;
    plan(PlanCommand {
        command: PlanSubcommand::Draft(PlanDraftArgs {
            store: PlanStoreArgs {
                goal_store_url: "http://localhost:9088".to_string(),
            },
            file: None,
            source_plan_id: None,
            title: Some(title),
            objective: Some(objective.clone()),
            prompt: Some(objective),
            repo: None,
            mode: "interactive".to_string(),
            author: Some("operator".to_string()),
            summary: None,
            out: Some(PathBuf::from(output)),
            emit_only: true,
            acceptance_evidence: Vec::new(),
            constraint: Vec::new(),
            out_of_scope: Vec::new(),
            assumption: Vec::new(),
            open_question: Vec::new(),
            subgoal: Vec::new(),
            initial_task: Vec::new(),
        }),
    })
    .await
}

fn guided_goal_draft(theme: &ColorfulTheme) -> anyhow::Result<()> {
    let title: String = Input::with_theme(theme)
        .with_prompt("Goal title")
        .interact_text()?;
    let objective: String = Input::with_theme(theme)
        .with_prompt("Objective")
        .interact_text()?;
    let output: String = Input::with_theme(theme)
        .with_prompt("Output JSON path")
        .default("examples/goal-draft-from-guide.json".to_string())
        .interact_text()?;
    draft_goal(DraftGoalArgs {
        title,
        objective,
        repo: None,
        out: Some(PathBuf::from(output)),
        strict_review: true,
        human_steered: false,
        enable_branching: false,
        plan_summary: None,
        acceptance_evidence: Vec::new(),
        constraint: Vec::new(),
        out_of_scope: Vec::new(),
        assumption: Vec::new(),
        open_question: Vec::new(),
        review_preset: Vec::new(),
        subgoal: Vec::new(),
        initial_task: Vec::new(),
    })
}

async fn guided_approval(theme: &ColorfulTheme) -> anyhow::Result<()> {
    let goal_id_raw: String = Input::with_theme(theme)
        .with_prompt("Goal ID (leave blank to use latest)")
        .allow_empty(true)
        .interact_text()?;
    let approval_id_raw: String = Input::with_theme(theme)
        .with_prompt("Approval ID")
        .interact_text()?;
    let approved = Confirm::with_theme(theme)
        .with_prompt("Approve this request?")
        .default(true)
        .interact()?;
    let note: String = Input::with_theme(theme)
        .with_prompt("Optional note")
        .allow_empty(true)
        .interact_text()?;
    human(HumanCommand {
        command: HumanSubcommand::Approve(ApproveArgs {
            restate_ingress: "http://localhost:8080".to_string(),
            selector: GoalSelectorArgs {
                goal_id: if goal_id_raw.trim().is_empty() {
                    None
                } else {
                    Some(Uuid::parse_str(goal_id_raw.trim()).context("parse goal id")?)
                },
                latest: goal_id_raw.trim().is_empty(),
                goal_store_url: "http://localhost:9088".to_string(),
            },
            approval_id: Uuid::parse_str(approval_id_raw.trim()).context("parse approval id")?,
            approved,
            note: if note.trim().is_empty() {
                None
            } else {
                Some(note)
            },
        }),
    })
    .await
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

fn helm(args: HelmCommand) -> anyhow::Result<()> {
    match args.command {
        HelmSubcommand::Lint(args) => {
            run_status_command(&args.helm, helm_lint_args(&args), "run helm lint")
        }
        HelmSubcommand::Template(args) => {
            let command_args = helm_template_args(&args);
            if let Some(output) = args.output {
                let command_output = Command::new(&args.helm)
                    .args(&command_args)
                    .output()
                    .context("run helm template")?;
                if !command_output.status.success() {
                    bail!(
                        "helm template exited with {}: {}{}",
                        command_output.status,
                        String::from_utf8_lossy(&command_output.stderr),
                        String::from_utf8_lossy(&command_output.stdout)
                    );
                }
                if let Some(parent) = output.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&output, command_output.stdout)?;
                println!("rendered {}", output.display());
                Ok(())
            } else {
                run_status_command(&args.helm, command_args, "run helm template")
            }
        }
        HelmSubcommand::Upgrade(args) => {
            run_status_command(&args.helm, helm_upgrade_args(&args), "run helm upgrade")
        }
        HelmSubcommand::Rollback(args) => {
            run_status_command(&args.helm, helm_rollback_args(&args), "run helm rollback")
        }
        HelmSubcommand::Package(args) => helm_package(args),
    }
}

fn run_status_command(program: &str, args: Vec<String>, description: &str) -> anyhow::Result<()> {
    let status = Command::new(program)
        .args(&args)
        .status()
        .with_context(|| description.to_string())?;
    if !status.success() {
        bail!("{program} exited with {status}");
    }
    Ok(())
}

fn helm_lint_args(args: &HelmLintArgs) -> Vec<String> {
    let mut command_args = vec!["lint".to_string(), args.chart.display().to_string()];
    append_helm_values(&mut command_args, &args.values);
    command_args
}

fn helm_template_args(args: &HelmTemplateArgs) -> Vec<String> {
    let mut command_args = vec![
        "template".to_string(),
        args.release.clone(),
        args.chart.display().to_string(),
    ];
    append_helm_values(&mut command_args, &args.values);
    append_helm_set_values(&mut command_args, &args.set_values);
    if let Some(namespace) = args.namespace.as_deref() {
        command_args.push("--namespace".to_string());
        command_args.push(namespace.to_string());
    }
    if args.include_crds {
        command_args.push("--include-crds".to_string());
    }
    command_args
}

fn helm_upgrade_args(args: &HelmUpgradeArgs) -> Vec<String> {
    let mut command_args = vec![
        "upgrade".to_string(),
        "--install".to_string(),
        args.release.clone(),
        args.chart.display().to_string(),
        "--namespace".to_string(),
        args.namespace.clone(),
    ];
    if !args.no_create_namespace {
        command_args.push("--create-namespace".to_string());
    }
    append_helm_values(&mut command_args, &args.values);
    append_helm_set_values(&mut command_args, &args.set_values);
    if args.dry_run {
        command_args.push("--dry-run".to_string());
    }
    if args.wait {
        command_args.push("--wait".to_string());
    }
    if let Some(timeout) = args.timeout.as_deref() {
        command_args.push("--timeout".to_string());
        command_args.push(timeout.to_string());
    }
    command_args
}

fn helm_rollback_args(args: &HelmRollbackArgs) -> Vec<String> {
    let mut command_args = vec!["rollback".to_string(), args.release.clone()];
    if let Some(revision) = args.revision {
        command_args.push(revision.to_string());
    }
    command_args.push("--namespace".to_string());
    command_args.push(args.namespace.clone());
    if args.wait {
        command_args.push("--wait".to_string());
    }
    if let Some(timeout) = args.timeout.as_deref() {
        command_args.push("--timeout".to_string());
        command_args.push(timeout.to_string());
    }
    command_args
}

fn append_helm_values(command_args: &mut Vec<String>, values: &[PathBuf]) {
    for value in values {
        command_args.push("--values".to_string());
        command_args.push(value.display().to_string());
    }
}

fn append_helm_set_values(command_args: &mut Vec<String>, values: &[String]) {
    for value in values {
        command_args.push("--set".to_string());
        command_args.push(value.clone());
    }
}

fn helm_package(args: HelmPackageArgs) -> anyhow::Result<()> {
    let mut command = Command::new(&args.script);
    command.env("CHART_DIR", &args.chart_dir);
    command.env("DIST_DIR", &args.dist_dir);
    if let Some(chart_version) = args.chart_version {
        command.env("CHART_VERSION", chart_version);
    }
    if let Some(app_version) = args.app_version {
        command.env("APP_VERSION", app_version);
    }
    if let Some(release_url) = args.release_url {
        command.env("RELEASE_URL", release_url);
    }
    let status = command.status().context("run Helm chart package script")?;
    if !status.success() {
        bail!("Helm chart package script exited with {status}");
    }
    Ok(())
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
        StoreSubcommand::Policy(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
            get_url(
                &format!(
                    "{}/goal-store/policy",
                    args.goal_store_url.trim_end_matches('/')
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Goals(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
            get_url(
                &format!(
                    "{}/goal-store/goals",
                    args.goal_store_url.trim_end_matches('/')
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Plans(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
            get_url(
                &format!(
                    "{}/goal-store/plans",
                    args.goal_store_url.trim_end_matches('/')
                ),
                None,
            )
            .await
        }
        StoreSubcommand::AllTasks(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
            get_url(
                &format!(
                    "{}/goal-store/tasks",
                    args.goal_store_url.trim_end_matches('/')
                ),
                None,
            )
            .await
        }
        StoreSubcommand::Approvals(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
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
        StoreSubcommand::EventSourceApprovals(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
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
        StoreSubcommand::Goal(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
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
        StoreSubcommand::Tasks(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
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
        StoreSubcommand::Events(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
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
        StoreSubcommand::Artifacts(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
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
        StoreSubcommand::Checkpoints(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
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
        StoreSubcommand::RecordArtifacts(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
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
        StoreSubcommand::GoalApprovals(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
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
        PlanSubcommand::Draft(mut args) => {
            args.store = effective_plan_store_args(args.store)?;
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
        PlanSubcommand::List(mut args) => {
            args.store = effective_plan_store_args(args.store)?;
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
        PlanSubcommand::Show(mut args) => {
            args.store = effective_plan_store_args(args.store)?;
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
        PlanSubcommand::Revise(mut args) => {
            args.store = effective_plan_store_args(args.store)?;
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
        PlanSubcommand::Compile(mut args) => {
            args.store = effective_plan_store_args(args.store)?;
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
        PlanSubcommand::VoteCandidate(mut args) => {
            args.store = effective_plan_store_args(args.store)?;
            let request: PlanCandidateVoteRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/goal-store/plans/{}/candidate-votes",
                    args.store.goal_store_url.trim_end_matches('/'),
                    args.plan_id
                ),
                &request,
                None,
                None,
            )
            .await
        }
        PlanSubcommand::SelectCandidate(mut args) => {
            args.store = effective_plan_store_args(args.store)?;
            let request: PlanCandidateSelectionRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!(
                    "{}/goal-store/plans/{}/candidate-selection",
                    args.store.goal_store_url.trim_end_matches('/'),
                    args.plan_id
                ),
                &request,
                None,
                None,
            )
            .await
        }
        PlanSubcommand::FollowUps(args) => follow_ups(args),
    }
}

async fn human(args: HumanCommand) -> anyhow::Result<()> {
    match args.command {
        HumanSubcommand::Approve(args) => approve(args).await,
        HumanSubcommand::Notify(args) => notify(args).await,
    }
}

fn deploy(args: DeployCommand) -> anyhow::Result<()> {
    match args.command {
        DeploySubcommand::Local(args) => compose(args),
        DeploySubcommand::Cluster(args) => k8s(args),
        DeploySubcommand::Chart(args) => helm(args),
        DeploySubcommand::Restate(args) => restate(args),
    }
}

async fn sandbox(args: SandboxCommand) -> anyhow::Result<()> {
    match args.command {
        SandboxSubcommand::Plan(mut args) => {
            args.sandbox_runner_url = effective_sandbox_runner_url(&args.sandbox_runner_url)?;
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
        SandboxSubcommand::Create(mut args) => {
            args.sandbox_runner_url = effective_sandbox_runner_url(&args.sandbox_runner_url)?;
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
        SandboxSubcommand::Snapshot(mut args) => {
            args.sandbox_runner_url = effective_sandbox_runner_url(&args.sandbox_runner_url)?;
            let request = serde_json::json!({ "workspace_id": args.workspace_id });
            post_json_to_url(
                &format!("{}/snapshot", args.sandbox_runner_url.trim_end_matches('/')),
                &request,
                None,
                None,
            )
            .await
        }
        SandboxSubcommand::Cleanup(mut args) => {
            args.sandbox_runner_url = effective_sandbox_runner_url(&args.sandbox_runner_url)?;
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
        EventSubcommand::Sources(mut args) => {
            args.event_gateway_url = effective_event_gateway_url(&args.event_gateway_url)?;
            get_url(
                &format!(
                    "{}/event-sources",
                    args.event_gateway_url.trim_end_matches('/')
                ),
                args.token.as_deref(),
            )
            .await
        }
        EventSubcommand::Register(mut args) => {
            args.event_gateway_url = effective_event_gateway_url(&args.event_gateway_url)?;
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
        EventSubcommand::Ingest(mut args) => {
            args.event_gateway_url = effective_event_gateway_url(&args.event_gateway_url)?;
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
        EventSubcommand::Emit(mut args) => {
            args.event_gateway_url = effective_event_gateway_url(&args.event_gateway_url)?;
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
        EventSubcommand::Webhook(mut args) => {
            args.event_gateway_url = effective_event_gateway_url(&args.event_gateway_url)?;
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
        EventSubcommand::PollSqs(mut args) => {
            args.event_gateway_url = effective_event_gateway_url(&args.event_gateway_url)?;
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
        EventSubcommand::Trigger(mut args) => {
            args.event_gateway_url = effective_event_gateway_url(&args.event_gateway_url)?;
            let request: TriggeredGoalRequest = read_json_file(&args.file)?;
            post_json_to_url(
                &format!("{}/triggers", args.event_gateway_url.trim_end_matches('/')),
                &request,
                args.token.as_deref(),
                None,
            )
            .await
        }
        EventSubcommand::List(mut args) => {
            args.event_gateway_url = effective_event_gateway_url(&args.event_gateway_url)?;
            get_url(
                &format!("{}/events", args.event_gateway_url.trim_end_matches('/')),
                args.token.as_deref(),
            )
            .await
        }
        EventSubcommand::Triggers(mut args) => {
            args.event_gateway_url = effective_event_gateway_url(&args.event_gateway_url)?;
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
        MemorySubcommand::Write(mut args) => {
            args.memory_gateway_url = effective_memory_gateway_url(&args.memory_gateway_url)?;
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
        MemorySubcommand::Search(mut args) => {
            args.memory_gateway_url = effective_memory_gateway_url(&args.memory_gateway_url)?;
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
        MemorySubcommand::Context(mut args) => {
            args.memory_gateway_url = effective_memory_gateway_url(&args.memory_gateway_url)?;
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
        MemorySubcommand::Join(mut args) => {
            args.memory_gateway_url = effective_memory_gateway_url(&args.memory_gateway_url)?;
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
        MemorySubcommand::Retract(mut args) => {
            args.memory_gateway_url = effective_memory_gateway_url(&args.memory_gateway_url)?;
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
        MemorySubcommand::Edit(mut args) => {
            args.memory_gateway_url = effective_memory_gateway_url(&args.memory_gateway_url)?;
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
        MemorySubcommand::PreviewEdit(mut args) => {
            args.memory_gateway_url = effective_memory_gateway_url(&args.memory_gateway_url)?;
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
        MemorySubcommand::Repair(mut args) => {
            args.memory_gateway_url = effective_memory_gateway_url(&args.memory_gateway_url)?;
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
        MemorySubcommand::Events(mut args) => {
            args.memory_gateway_url = effective_memory_gateway_url(&args.memory_gateway_url)?;
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
        RunnerSubcommand::List(mut args) => {
            args.registry_url = effective_runner_registry_url(&args.registry_url)?;
            get_url(
                &format!("{}/runners", args.registry_url.trim_end_matches('/')),
                None,
            )
            .await
        }
        RunnerSubcommand::Status(mut args) => {
            args.registry_url = effective_runner_registry_url(&args.registry_url)?;
            get_url(
                &format!("{}/runners/status", args.registry_url.trim_end_matches('/')),
                None,
            )
            .await
        }
        RunnerSubcommand::Register(mut args) => {
            args.registry_url = effective_runner_registry_url(&args.registry_url)?;
            let registration: RunnerRegistration = read_json_file(&args.file)?;
            post_json_to_url(
                &format!("{}/runners", args.registry_url.trim_end_matches('/')),
                &registration,
                None,
                None,
            )
            .await
        }
        RunnerSubcommand::Dispatch(mut args) => {
            args.registry_url = effective_runner_registry_url(&args.registry_url)?;
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

async fn notify(mut args: NotifyArgs) -> anyhow::Result<()> {
    args.notifier_url = effective_notifier_url(&args.notifier_url)?;
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
    write_project_marker(&args.path, args.force)?;
    println!("initialized COAT project under {}", args.path.display());
    println!("next: coat setup config --list-profiles");
    println!("next: coat setup config --show");
    println!("next: coat setup local-auth");
    Ok(())
}

fn write_project_marker(path: &Path, force: bool) -> anyhow::Result<()> {
    let marker = path.join(COAT_PROJECT_MARKER);
    if marker.exists() && !force {
        println!("{} already exists", marker.display());
        return Ok(());
    }
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = CoatProjectConfig::default();
    fs::write(&marker, serde_json::to_string_pretty(&content)? + "\n")
        .with_context(|| format!("write {}", marker.display()))?;
    println!("wrote {}", marker.display());
    Ok(())
}

fn write_user_config(path: &Path, force: bool) -> anyhow::Result<()> {
    let path = expand_home_path(path)?;
    if path.exists() && !force {
        println!("{} already exists", path.display());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = CoatUserConfig::default();
    fs::write(&path, serde_json::to_string_pretty(&content)? + "\n")
        .with_context(|| format!("write {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn warn_if_project_not_initialized(command: &Commands) -> anyhow::Result<()> {
    let check = command_project_init_check(command);
    if check == ProjectInitCheck::None {
        return Ok(());
    }
    let cwd = env::current_dir().context("read current directory")?;
    if find_coat_project_root(&cwd).is_none() {
        let cli_config = load_resolved_coat_config()
            .map(|resolved| resolved.config.cli)
            .unwrap_or_default();
        match project_init_action(
            false,
            check,
            &cli_config,
            env_flag_enabled("COAT_ALLOW_UNINITIALIZED"),
        ) {
            ProjectInitAction::Proceed => {}
            ProjectInitAction::Warn => {
                eprintln!(
                    "warning: COAT project is not initialized; missing {COAT_PROJECT_MARKER} in {} or its parents. Run `coat init` before durable project workflows.",
                    cwd.display()
                );
            }
            ProjectInitAction::Fail => {
                bail!(
                    "COAT project is not initialized; missing {COAT_PROJECT_MARKER} in {} or its parents. Run `coat init`, or set COAT_ALLOW_UNINITIALIZED=1 only for intentional one-off operator commands.",
                    cwd.display()
                );
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectInitCheck {
    None,
    WarnOnly,
    Durable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectInitAction {
    Proceed,
    Warn,
    Fail,
}

fn command_project_init_check(command: &Commands) -> ProjectInitCheck {
    match command {
        Commands::Init(_) | Commands::Setup(_) | Commands::Guide(_) => ProjectInitCheck::None,
        Commands::Goal(command) => match &command.command {
            GoalSubcommand::Draft(_) | GoalSubcommand::Lint(_) | GoalSubcommand::ReviewChecks => {
                ProjectInitCheck::WarnOnly
            }
            GoalSubcommand::List(_)
            | GoalSubcommand::Submit(_)
            | GoalSubcommand::Status(_)
            | GoalSubcommand::Progress(_)
            | GoalSubcommand::Tasks(_)
            | GoalSubcommand::Steer(_)
            | GoalSubcommand::SteerStandard(_)
            | GoalSubcommand::Restart(_)
            | GoalSubcommand::Branch(_)
            | GoalSubcommand::SelectBranch(_)
            | GoalSubcommand::Cancel(_) => ProjectInitCheck::Durable,
        },
        Commands::Plan(command) => match &command.command {
            PlanSubcommand::Draft(args) if args.emit_only || args.out.is_some() => {
                ProjectInitCheck::WarnOnly
            }
            PlanSubcommand::FollowUps(_) => ProjectInitCheck::WarnOnly,
            PlanSubcommand::Draft(_)
            | PlanSubcommand::List(_)
            | PlanSubcommand::Show(_)
            | PlanSubcommand::Revise(_)
            | PlanSubcommand::Compile(_)
            | PlanSubcommand::VoteCandidate(_)
            | PlanSubcommand::SelectCandidate(_) => ProjectInitCheck::Durable,
        },
        Commands::Deploy(command) => match &command.command {
            DeploySubcommand::Local(_) => ProjectInitCheck::WarnOnly,
            DeploySubcommand::Cluster(_)
            | DeploySubcommand::Chart(_)
            | DeploySubcommand::Restate(_) => ProjectInitCheck::Durable,
        },
        Commands::Human(_)
        | Commands::Event(_)
        | Commands::Runner(_)
        | Commands::Memory(_)
        | Commands::Store(_)
        | Commands::Sandbox(_)
        | Commands::Release(_) => ProjectInitCheck::Durable,
    }
}

fn project_init_action(
    initialized: bool,
    check: ProjectInitCheck,
    cli: &CoatCliConfig,
    allow_uninitialized: bool,
) -> ProjectInitAction {
    if initialized || check == ProjectInitCheck::None {
        return ProjectInitAction::Proceed;
    }
    let warn = cli.warn_uninitialized.unwrap_or(true);
    let require = cli.require_project_for_durable_commands.unwrap_or(true);
    if check == ProjectInitCheck::Durable && require && !allow_uninitialized {
        ProjectInitAction::Fail
    } else if warn {
        ProjectInitAction::Warn
    } else {
        ProjectInitAction::Proceed
    }
}

fn env_flag_enabled(key: &str) -> bool {
    env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn find_coat_project_root(start: &Path) -> Option<PathBuf> {
    for candidate in start.ancestors() {
        if candidate.join(COAT_PROJECT_MARKER).is_file() {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

#[derive(Debug, Clone, Default)]
struct ResolvedCoatConfig {
    project_root: Option<PathBuf>,
    project_config_path: Option<PathBuf>,
    user_config_path: Option<PathBuf>,
    config: CoatConfig,
}

fn load_resolved_coat_config() -> anyhow::Result<ResolvedCoatConfig> {
    let cwd = env::current_dir().context("read current directory")?;
    let project_root = find_coat_project_root(&cwd);
    let mut resolved = ResolvedCoatConfig {
        project_root: project_root.clone(),
        project_config_path: None,
        user_config_path: None,
        config: CoatConfig::default(),
    };

    if let Some(root) = &project_root {
        let path = root.join(COAT_PROJECT_MARKER);
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let project: CoatProjectConfig =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        merge_coat_config(&mut resolved.config, project.config);
        resolved.project_config_path = Some(path);
    }

    let user_path = default_user_config_path()?;
    if user_path.is_file() {
        let raw = fs::read_to_string(&user_path)
            .with_context(|| format!("read {}", user_path.display()))?;
        let user: CoatUserConfig =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", user_path.display()))?;
        merge_coat_config(&mut resolved.config, user.config);
        resolved.user_config_path = Some(user_path);
    }

    let profile = CONFIG_PROFILE_OVERRIDE
        .get()
        .and_then(|profile| profile.clone())
        .filter(|profile| !profile.trim().is_empty())
        .or_else(|| env::var("COAT_PROFILE").ok())
        .filter(|profile| !profile.trim().is_empty())
        .or_else(|| resolved.config.active_profile.clone());
    if let Some(profile) = profile.as_deref() {
        apply_config_profile(&mut resolved.config, profile)?;
    }

    Ok(resolved)
}

fn merge_coat_config(base: &mut CoatConfig, overlay: CoatConfig) {
    replace_if_some(&mut base.active_profile, overlay.active_profile);
    merge_profile_configs(&mut base.profiles, overlay.profiles);
    merge_config_paths(&mut base.paths, overlay.paths);
    merge_service_endpoints(&mut base.service_endpoints, overlay.service_endpoints);
    merge_local_deploy_config(&mut base.local_deploy, overlay.local_deploy);
    merge_cloud_config(&mut base.cloud, overlay.cloud);
    merge_kubernetes_config(&mut base.kubernetes, overlay.kubernetes);
    merge_cli_config(&mut base.cli, overlay.cli);
    merge_operator_defaults(&mut base.defaults, overlay.defaults);
}

fn apply_config_profile(config: &mut CoatConfig, profile_name: &str) -> anyhow::Result<()> {
    let Some(profile) = config
        .profiles
        .iter()
        .find(|profile| profile.name == profile_name)
        .cloned()
    else {
        bail!(
            "COAT profile `{profile_name}` is not defined. Run `coat setup config --list-profiles`."
        );
    };
    config.active_profile = Some(profile.name.clone());
    merge_config_paths(&mut config.paths, profile.paths);
    merge_service_endpoints(&mut config.service_endpoints, profile.service_endpoints);
    merge_local_deploy_config(&mut config.local_deploy, profile.local_deploy);
    merge_cloud_config(&mut config.cloud, profile.cloud);
    merge_kubernetes_config(&mut config.kubernetes, profile.kubernetes);
    merge_cli_config(&mut config.cli, profile.cli);
    merge_operator_defaults(&mut config.defaults, profile.defaults);
    Ok(())
}

fn merge_profile_configs(base: &mut Vec<CoatProfileConfig>, overlay: Vec<CoatProfileConfig>) {
    for profile in overlay {
        if let Some(existing) = base
            .iter_mut()
            .find(|existing| existing.name == profile.name)
        {
            merge_profile_config(existing, profile);
        } else {
            base.push(profile);
        }
    }
}

fn merge_profile_config(base: &mut CoatProfileConfig, overlay: CoatProfileConfig) {
    base.kind = overlay.kind;
    if !overlay.description.is_empty() {
        base.description = overlay.description;
    }
    merge_config_paths(&mut base.paths, overlay.paths);
    merge_service_endpoints(&mut base.service_endpoints, overlay.service_endpoints);
    merge_local_deploy_config(&mut base.local_deploy, overlay.local_deploy);
    merge_cloud_config(&mut base.cloud, overlay.cloud);
    merge_kubernetes_config(&mut base.kubernetes, overlay.kubernetes);
    merge_cli_config(&mut base.cli, overlay.cli);
    merge_operator_defaults(&mut base.defaults, overlay.defaults);
}

fn merge_config_paths(base: &mut CoatConfigPaths, overlay: CoatConfigPaths) {
    replace_if_some(&mut base.project_root, overlay.project_root);
    replace_if_some(&mut base.local_provider_env, overlay.local_provider_env);
    replace_if_some(&mut base.restate_cloud_env, overlay.restate_cloud_env);
    replace_if_some(&mut base.data_dir, overlay.data_dir);
    replace_if_some(&mut base.cache_dir, overlay.cache_dir);
}

fn merge_service_endpoints(base: &mut CoatServiceEndpoints, overlay: CoatServiceEndpoints) {
    replace_if_some(&mut base.restate_ingress, overlay.restate_ingress);
    replace_if_some(&mut base.restate_admin, overlay.restate_admin);
    replace_if_some(&mut base.coordinator_url, overlay.coordinator_url);
    replace_if_some(&mut base.sandbox_runner_url, overlay.sandbox_runner_url);
    replace_if_some(&mut base.runner_registry_url, overlay.runner_registry_url);
    replace_if_some(&mut base.notifier_url, overlay.notifier_url);
    replace_if_some(&mut base.memory_gateway_url, overlay.memory_gateway_url);
    replace_if_some(&mut base.goal_store_url, overlay.goal_store_url);
    replace_if_some(&mut base.event_gateway_url, overlay.event_gateway_url);
    replace_if_some(&mut base.control_mcp_url, overlay.control_mcp_url);
}

fn merge_local_deploy_config(base: &mut CoatLocalDeployConfig, overlay: CoatLocalDeployConfig) {
    append_unique(&mut base.env_files, overlay.env_files);
    replace_if_some(
        &mut base.restate_cloud_env_file,
        overlay.restate_cloud_env_file,
    );
    replace_if_some(&mut base.allow_stub_runners, overlay.allow_stub_runners);
    replace_if_some(&mut base.allow_uninitialized, overlay.allow_uninitialized);
    append_unique(&mut base.profiles, overlay.profiles);
}

fn merge_cloud_config(base: &mut CoatCloudConfig, overlay: CoatCloudConfig) {
    replace_if_some(&mut base.provider, overlay.provider);
    replace_if_some(&mut base.region, overlay.region);
    replace_if_some(&mut base.secret_provider, overlay.secret_provider);
    replace_if_some(&mut base.object_store, overlay.object_store);
    merge_restate_cloud_config(&mut base.restate_cloud, overlay.restate_cloud);
}

fn merge_restate_cloud_config(base: &mut CoatRestateCloudConfig, overlay: CoatRestateCloudConfig) {
    replace_if_some(&mut base.env_file, overlay.env_file);
    replace_if_some(&mut base.tunnel_name, overlay.tunnel_name);
    replace_if_some(&mut base.region, overlay.region);
    replace_if_some(&mut base.service_url, overlay.service_url);
    replace_if_some(&mut base.local_ingress_url, overlay.local_ingress_url);
    replace_if_some(&mut base.local_admin_url, overlay.local_admin_url);
    replace_if_some(&mut base.coordinator_url, overlay.coordinator_url);
}

fn merge_kubernetes_config(base: &mut CoatKubernetesConfig, overlay: CoatKubernetesConfig) {
    replace_if_some(&mut base.distribution, overlay.distribution);
    replace_if_some(&mut base.kubectl, overlay.kubectl);
    replace_if_some(&mut base.context, overlay.context);
    replace_if_some(&mut base.kubeconfig, overlay.kubeconfig);
    replace_if_some(&mut base.namespace, overlay.namespace);
    replace_if_some(&mut base.manifest, overlay.manifest);
    replace_if_some(&mut base.rendered_manifest, overlay.rendered_manifest);
    replace_if_some(&mut base.helm_release, overlay.helm_release);
    replace_if_some(&mut base.helm_chart, overlay.helm_chart);
    append_unique(&mut base.helm_values, overlay.helm_values);
    replace_if_some(&mut base.image_registry, overlay.image_registry);
    replace_if_some(&mut base.service_account, overlay.service_account);
    replace_if_some(&mut base.secret_provider, overlay.secret_provider);
    replace_if_some(&mut base.workload_identity, overlay.workload_identity);
    replace_if_some(&mut base.object_store, overlay.object_store);
}

fn merge_cli_config(base: &mut CoatCliConfig, overlay: CoatCliConfig) {
    replace_if_some(&mut base.output_format, overlay.output_format);
    replace_if_some(&mut base.interactive_setup, overlay.interactive_setup);
    replace_if_some(&mut base.warn_uninitialized, overlay.warn_uninitialized);
    replace_if_some(
        &mut base.require_project_for_durable_commands,
        overlay.require_project_for_durable_commands,
    );
}

fn merge_operator_defaults(base: &mut CoatOperatorDefaults, overlay: CoatOperatorDefaults) {
    replace_if_some(&mut base.goal_store_url, overlay.goal_store_url);
    replace_if_some(&mut base.restate_ingress, overlay.restate_ingress);
    replace_if_some(&mut base.latest_goal_selector, overlay.latest_goal_selector);
}

fn replace_if_some<T>(slot: &mut Option<T>, value: Option<T>) {
    if value.is_some() {
        *slot = value;
    }
}

fn append_unique(values: &mut Vec<String>, additions: Vec<String>) {
    for value in additions {
        if !values.iter().any(|existing| existing == &value) {
            values.push(value);
        }
    }
}

fn default_user_config_path() -> anyhow::Result<PathBuf> {
    match env::var("COAT_CONFIG") {
        Ok(path) if !path.trim().is_empty() => expand_home_path(Path::new(&path)),
        _ => expand_home_path(Path::new(DEFAULT_USER_CONFIG)),
    }
}

fn config_path(value: &str, project_root: Option<&Path>) -> anyhow::Result<PathBuf> {
    let expanded = expand_home_path(Path::new(value))?;
    if expanded.is_absolute() {
        return Ok(expanded);
    }
    Ok(project_root
        .map(|root| root.join(expanded.clone()))
        .unwrap_or(expanded))
}

fn print_resolved_config(profile: Option<&str>) -> anyhow::Result<()> {
    let resolved = match profile {
        Some(profile) => {
            let mut resolved = load_resolved_coat_config()?;
            apply_config_profile(&mut resolved.config, profile)?;
            resolved
        }
        None => load_resolved_coat_config()?,
    };
    let mut output = serde_json::json!({
        "active_profile": resolved.config.active_profile,
        "project_root": resolved.project_root.as_ref().map(|path| path.display().to_string()),
        "project_config": resolved.project_config_path.as_ref().map(|path| path.display().to_string()),
        "user_config": resolved.user_config_path.as_ref().map(|path| path.display().to_string()),
        "user_config_default": default_user_config_path()?.display().to_string(),
        "config": resolved.config,
    });
    overlay_env_status(&mut output);
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn print_config_profiles() -> anyhow::Result<()> {
    let resolved = load_resolved_coat_config()?;
    println!("COAT config profiles");
    for profile in &resolved.config.profiles {
        let active = if Some(profile.name.as_str()) == resolved.config.active_profile.as_deref() {
            " active"
        } else {
            ""
        };
        println!("  {} ({:?}){active}", profile.name, profile.kind);
        if !profile.description.is_empty() {
            println!("    {}", profile.description);
        }
    }
    Ok(())
}

fn overlay_env_status(output: &mut serde_json::Value) {
    output["env_overrides"] = serde_json::json!({
        "COAT_CONFIG": env::var("COAT_CONFIG").ok(),
        "COAT_PROFILE": env::var("COAT_PROFILE").ok(),
        "COAT_GOAL_ID": env::var("COAT_GOAL_ID").ok().map(|_| "<set>"),
        "COAT_RESTATE_INGRESS": env::var("COAT_RESTATE_INGRESS").ok(),
        "COAT_GOAL_STORE_URL": env::var("COAT_GOAL_STORE_URL").ok(),
        "COAT_EVENT_GATEWAY_URL": env::var("COAT_EVENT_GATEWAY_URL").ok(),
        "COAT_MEMORY_GATEWAY_URL": env::var("COAT_MEMORY_GATEWAY_URL").ok(),
        "COAT_SANDBOX_RUNNER_URL": env::var("COAT_SANDBOX_RUNNER_URL").ok(),
        "COAT_RUNNER_REGISTRY": env::var("COAT_RUNNER_REGISTRY").ok(),
        "COAT_NOTIFIER_URL": env::var("COAT_NOTIFIER_URL").ok(),
        "COAT_CONTROL_MCP_URL": env::var("COAT_CONTROL_MCP_URL").ok(),
        "COAT_ALLOW_UNINITIALIZED": env::var("COAT_ALLOW_UNINITIALIZED").ok(),
    });
}

fn effective_restate_ingress(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_RESTATE_INGRESS {
        return Ok(value.to_string());
    }
    let config = load_resolved_coat_config()?.config;
    Ok(endpoint_from_config(
        value,
        DEFAULT_RESTATE_INGRESS,
        config
            .service_endpoints
            .restate_ingress
            .or(config.defaults.restate_ingress),
    ))
}

fn effective_sandbox_runner_url(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_SANDBOX_RUNNER_URL {
        return Ok(value.to_string());
    }
    let config = load_resolved_coat_config()?.config;
    Ok(endpoint_from_config(
        value,
        DEFAULT_SANDBOX_RUNNER_URL,
        config.service_endpoints.sandbox_runner_url,
    ))
}

fn effective_runner_registry_url(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_RUNNER_REGISTRY_URL {
        return Ok(value.to_string());
    }
    let config = load_resolved_coat_config()?.config;
    Ok(endpoint_from_config(
        value,
        DEFAULT_RUNNER_REGISTRY_URL,
        config.service_endpoints.runner_registry_url,
    ))
}

fn effective_notifier_url(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_NOTIFIER_URL {
        return Ok(value.to_string());
    }
    let config = load_resolved_coat_config()?.config;
    Ok(endpoint_from_config(
        value,
        DEFAULT_NOTIFIER_URL,
        config.service_endpoints.notifier_url,
    ))
}

fn effective_memory_gateway_url(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_MEMORY_GATEWAY_URL {
        return Ok(value.to_string());
    }
    let config = load_resolved_coat_config()?.config;
    Ok(endpoint_from_config(
        value,
        DEFAULT_MEMORY_GATEWAY_URL,
        config.service_endpoints.memory_gateway_url,
    ))
}

fn effective_goal_store_url(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_GOAL_STORE_URL {
        return Ok(value.to_string());
    }
    let config = load_resolved_coat_config()?.config;
    Ok(endpoint_from_config(
        value,
        DEFAULT_GOAL_STORE_URL,
        config
            .service_endpoints
            .goal_store_url
            .or(config.defaults.goal_store_url),
    ))
}

fn effective_event_gateway_url(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_EVENT_GATEWAY_URL {
        return Ok(value.to_string());
    }
    let config = load_resolved_coat_config()?.config;
    Ok(endpoint_from_config(
        value,
        DEFAULT_EVENT_GATEWAY_URL,
        config.service_endpoints.event_gateway_url,
    ))
}

fn effective_control_mcp_url(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_CONTROL_MCP_URL {
        return Ok(value.to_string());
    }
    let config = load_resolved_coat_config()?.config;
    Ok(endpoint_from_config(
        value,
        DEFAULT_CONTROL_MCP_URL,
        config.service_endpoints.control_mcp_url,
    ))
}

fn endpoint_from_config(value: &str, cli_default: &str, configured: Option<String>) -> String {
    if value == cli_default {
        configured.unwrap_or_else(|| value.to_string())
    } else {
        value.to_string()
    }
}

fn effective_goal_selector_args(mut args: GoalSelectorArgs) -> anyhow::Result<GoalSelectorArgs> {
    args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
    Ok(args)
}

fn effective_goal_id_args(mut args: GoalIdArgs) -> anyhow::Result<GoalIdArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_goal_tasks_args(mut args: GoalTasksArgs) -> anyhow::Result<GoalTasksArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_steer_goal_args(mut args: SteerGoalArgs) -> anyhow::Result<SteerGoalArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_steer_standard_goal_args(
    mut args: SteerStandardGoalArgs,
) -> anyhow::Result<SteerStandardGoalArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_restart_goal_args(mut args: RestartGoalArgs) -> anyhow::Result<RestartGoalArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_branch_goal_args(mut args: BranchGoalArgs) -> anyhow::Result<BranchGoalArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_select_branch_args(mut args: SelectBranchArgs) -> anyhow::Result<SelectBranchArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_cancel_goal_args(mut args: CancelGoalArgs) -> anyhow::Result<CancelGoalArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_approve_args(mut args: ApproveArgs) -> anyhow::Result<ApproveArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_plan_store_args(mut args: PlanStoreArgs) -> anyhow::Result<PlanStoreArgs> {
    args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
    Ok(args)
}

async fn goal(args: GoalCommand) -> anyhow::Result<()> {
    match args.command {
        GoalSubcommand::Draft(args) => draft_goal(args),
        GoalSubcommand::List(mut args) => {
            args.goal_store_url = effective_goal_store_url(&args.goal_store_url)?;
            list_goals(args).await
        }
        GoalSubcommand::Submit(mut args) => {
            args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
            submit_goal(args).await
        }
        GoalSubcommand::Status(args) => {
            let args = effective_goal_id_args(args)?;
            let goal_id = resolve_goal_id(&args.selector).await?;
            restate_post_without_body(&args.restate_ingress, goal_id, "status").await
        }
        GoalSubcommand::Progress(args) => {
            let args = effective_goal_id_args(args)?;
            let goal_id = resolve_goal_id(&args.selector).await?;
            restate_post_without_body(&args.restate_ingress, goal_id, "progress").await
        }
        GoalSubcommand::Tasks(args) => {
            let args = effective_goal_tasks_args(args)?;
            let goal_id = resolve_goal_id(&args.selector).await?;
            let query = task_query_from_args(&args)?;
            restate_post_json(&args.restate_ingress, goal_id, "tasks", &query).await
        }
        GoalSubcommand::Lint(args) => lint_goal(args),
        GoalSubcommand::Steer(args) => {
            let args = effective_steer_goal_args(args)?;
            let goal_id = resolve_goal_id(&args.selector).await?;
            let directive: SteeringDirective =
                read_goal_scoped_json_file(&args.file, goal_id, "SteeringDirective")?;
            restate_post_json(&args.restate_ingress, goal_id, "steer", &directive).await
        }
        GoalSubcommand::SteerStandard(args) => {
            steer_standard_goal(effective_steer_standard_goal_args(args)?).await
        }
        GoalSubcommand::ReviewChecks => review_checks(),
        GoalSubcommand::Restart(args) => {
            let args = effective_restart_goal_args(args)?;
            let goal_id = resolve_goal_id(&args.selector).await?;
            let request: RestartRequest =
                read_goal_scoped_json_file(&args.file, goal_id, "RestartRequest")?;
            restate_post_json(&args.restate_ingress, goal_id, "restart", &request).await
        }
        GoalSubcommand::Branch(args) => {
            let args = effective_branch_goal_args(args)?;
            let goal_id = resolve_goal_id(&args.selector).await?;
            let request: BranchRequest =
                read_goal_scoped_json_file(&args.file, goal_id, "BranchRequest")?;
            restate_post_json(&args.restate_ingress, goal_id, "branch", &request).await
        }
        GoalSubcommand::SelectBranch(args) => {
            let args = effective_select_branch_args(args)?;
            let goal_id = resolve_goal_id(&args.selector).await?;
            let request: BranchSelectionRequest =
                read_goal_scoped_json_file(&args.file, goal_id, "BranchSelectionRequest")?;
            restate_post_json(&args.restate_ingress, goal_id, "select_branch", &request).await
        }
        GoalSubcommand::Cancel(args) => {
            let args = effective_cancel_goal_args(args)?;
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
    let args = effective_approve_args(args)?;
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
        SetupSubcommand::Config(args) => config_setup(args),
        SetupSubcommand::LocalAuth(args) => local_auth_setup(args),
        SetupSubcommand::ChatClient(args) => chat_client_setup(args),
    }
}

fn config_setup(args: ConfigSetupArgs) -> anyhow::Result<()> {
    let default_action =
        !args.write_project && !args.write_user && !args.show && !args.list_profiles;
    if default_action {
        return interactive_config_setup(args);
    }
    if args.write_project {
        write_project_marker(&args.project_root, args.force)?;
    }
    if args.write_user {
        write_user_config(&args.user_config, args.force)?;
    }
    if args.list_profiles {
        print_config_profiles()?;
    }
    if args.show {
        print_resolved_config(args.profile.as_deref())?;
    }
    Ok(())
}

fn interactive_config_setup(args: ConfigSetupArgs) -> anyhow::Result<()> {
    let theme = ColorfulTheme::default();
    println!("COAT configuration setup");
    if Confirm::with_theme(&theme)
        .with_prompt("Write or refresh project config at .coat/project.json?")
        .default(true)
        .interact()?
    {
        write_project_marker(&args.project_root, args.force)?;
    }
    if Confirm::with_theme(&theme)
        .with_prompt("Write user config template at ~/.coat/config.json?")
        .default(false)
        .interact()?
    {
        write_user_config(&args.user_config, args.force)?;
    }
    if Confirm::with_theme(&theme)
        .with_prompt("Show resolved config paths and values?")
        .default(true)
        .interact()?
    {
        print_resolved_config(args.profile.as_deref())?;
    }
    Ok(())
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

    if selections.contains(&0) {
        let auth_choices = [
            "API key from env file or shell",
            "Runner-local Codex device/browser login",
            "Codex App Server URL",
        ];
        let auth_choice = Select::with_theme(&theme)
            .with_prompt("Codex runner auth mode")
            .items(&auth_choices)
            .default(1)
            .interact()?;
        env_text = replace_env_line(env_text, "CODEX_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "CODEX_REVIEW_RUNNER_MODE", "live");
        match auth_choice {
            0 => {
                env_text = replace_env_line(env_text, "CODEX_AUTH_MODE", "env_api_key");
                env_text = replace_env_line(
                    env_text,
                    "CODEX_RUNNER_LABELS_JSON",
                    &codex_labels_json("env_api_key", None),
                );
                env_text = replace_env_line(
                    env_text,
                    "CODEX_REVIEW_RUNNER_LABELS_JSON",
                    &codex_labels_json("env_api_key", Some("review")),
                );
            }
            1 => {
                env_text = replace_env_line(env_text, "CODEX_AUTH_MODE", "runner_local_device");
                env_text = replace_env_line(
                    env_text,
                    "CODEX_RUNNER_LABELS_JSON",
                    &codex_labels_json("runner_local_device", None),
                );
                env_text = replace_env_line(
                    env_text,
                    "CODEX_REVIEW_RUNNER_LABELS_JSON",
                    &codex_labels_json("runner_local_device", Some("review")),
                );
            }
            _ => {
                let app_server_url: String = Input::with_theme(&theme)
                    .with_prompt("Codex App Server URL")
                    .default("http://host.docker.internal:1455".to_string())
                    .interact_text()?;
                env_text = replace_env_line(env_text, "CODEX_AUTH_MODE", "app_server");
                env_text = replace_env_line(env_text, "CODEX_APP_SERVER_URL", &app_server_url);
                env_text = replace_env_line(
                    env_text,
                    "CODEX_RUNNER_LABELS_JSON",
                    &codex_labels_json("app_server", None),
                );
                env_text = replace_env_line(
                    env_text,
                    "CODEX_REVIEW_RUNNER_LABELS_JSON",
                    &codex_labels_json("app_server", Some("review")),
                );
            }
        }
    }

    if selections.contains(&1) {
        let auth_choices = [
            "API key/token from env file or shell",
            "Runner-local Claude Code device/browser login",
            "Brokered OAuth/device lease resolved by runner",
        ];
        let auth_choice = Select::with_theme(&theme)
            .with_prompt("Claude Code and staff-engineer auth mode")
            .items(&auth_choices)
            .default(1)
            .interact()?;
        let (auth_mode, device_label) = match auth_choice {
            0 => ("env_api_key", false),
            1 => ("runner_local_device", true),
            _ => ("oauth_device_broker", false),
        };
        env_text = replace_env_line(env_text, "CLAUDE_CODE_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "STAFF_ENGINEER_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "CLAUDE_CODE_AUTH_MODE", auth_mode);
        env_text = replace_env_line(env_text, "STAFF_ENGINEER_AUTH_MODE", auth_mode);
        env_text = replace_env_line(
            env_text,
            "CLAUDE_CODE_RUNNER_LABELS_JSON",
            &claude_labels_json("claude-code", auth_mode, device_label),
        );
        env_text = replace_env_line(
            env_text,
            "STAFF_ENGINEER_RUNNER_LABELS_JSON",
            &claude_labels_json("staff-engineer", auth_mode, device_label),
        );
    }

    if selections.contains(&2) {
        let bedrock_model: String = Input::with_theme(&theme)
            .with_prompt("Bedrock model id")
            .default("anthropic.claude-3-5-sonnet-20241022-v2:0".to_string())
            .interact_text()?;
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_KIND", "bedrock");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_AUTH_MODE", "aws_profile");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_MODEL", &bedrock_model);
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
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_LOCAL_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_KIND", &local_kind);
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_AUTH_MODE", "none");
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_MODEL", &local_model);
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_ENDPOINT", &local_endpoint);
    }

    if selections.contains(&5) {
        let hf_model: String = Input::with_theme(&theme)
            .with_prompt("Hugging Face endpoint model")
            .default("hf-endpoint-model".to_string())
            .interact_text()?;
        let hf_endpoint: String = Input::with_theme(&theme)
            .with_prompt("Hugging Face OpenAI-compatible endpoint")
            .default("https://api.endpoints.huggingface.cloud/v1".to_string())
            .interact_text()?;
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_KIND", "hugging_face");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_AUTH_MODE", "provider_token");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_MODEL", &hf_model);
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_ENDPOINT", &hf_endpoint);
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
        "preflight with: coat deploy local preflight --env-file {}",
        path.display()
    );
    println!(
        "use with: coat deploy local up --env-file {}",
        path.display()
    );
    Ok(())
}

fn populate_secret_env_values(mut env_text: String) -> String {
    for name in [
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "MODEL_PROVIDER_API_KEY",
        "MODEL_PROVIDER_RESEARCH_API_KEY",
        "HF_TOKEN",
        "HUGGINGFACE_TOKEN",
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

fn codex_labels_json(auth_mode: &str, lane: Option<&str>) -> String {
    let device_auth = auth_mode == "runner_local_device";
    let mut value = serde_json::json!({
        "pool": "default",
        "runtime": "codex",
        "auth.codex.device": device_auth.to_string(),
        "auth.codex.api_key": if device_auth { "false" } else { "env" },
        "auth.mode": auth_mode,
    });
    if let Some(lane) = lane {
        value["lane"] = serde_json::Value::String(lane.to_string());
    }
    value.to_string()
}

fn claude_labels_json(runtime: &str, auth_mode: &str, device_auth: bool) -> String {
    serde_json::json!({
        "pool": "default",
        "runtime": runtime,
        "auth.claude.device": device_auth.to_string(),
        "auth.claude.api_key": if device_auth { "false" } else { "env" },
        "auth.mode": auth_mode,
    })
    .to_string()
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
        "CODEX_API_KEY",
        "CODEX_AUTH_MODE",
        "CODEX_APP_SERVER_URL",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_AUTH_MODE",
        "STAFF_ENGINEER_AUTH_MODE",
        "AWS_PROFILE",
        "AWS_REGION",
        "AWS_DEFAULT_REGION",
        "MODEL_PROVIDER_AUTH_MODE",
        "MODEL_PROVIDER_API_KEY",
        "MODEL_PROVIDER_ENDPOINT",
        "MODEL_PROVIDER_RESEARCH_AUTH_MODE",
        "MODEL_PROVIDER_RESEARCH_API_KEY",
        "HF_TOKEN",
        "HUGGINGFACE_TOKEN",
        "LOCAL_MODEL_PROVIDER_ENDPOINT",
        "LOCAL_MODEL_PROVIDER_AUTH_MODE",
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
    println!(
        "  codex login   # runner-local device/browser auth; then set CODEX_AUTH_MODE=runner_local_device"
    );
    println!(
        "  claude login  # runner-local Claude Code auth; then set CLAUDE_CODE_AUTH_MODE=runner_local_device"
    );
    println!("  aws sso login --profile <profile>");
    println!("  ollama pull llama3.1");
    println!("  vllm serve <model> --host 0.0.0.0 --port 8000");
    println!("  hf auth login");
    println!("auth modes accepted by preflight:");
    println!(
        "  Codex: env_api_key, runner_local_device, app_server, oauth_device_broker, external_broker"
    );
    println!(
        "  Claude/staff-engineer: env_api_key, runner_local_device, oauth_device_broker, external_broker"
    );
    println!(
        "  Model providers: api_key_or_none, provider_token, aws_profile, workload_identity, none, external_broker"
    );
    println!("after auth, write an env file with:");
    println!("  coat setup local-auth --write-env --output infra/compose/local-providers.env");
    println!("  # or run `coat setup local-auth` interactively to flip selected lanes live");
    println!("then preflight and start Compose with that env file:");
    println!("  coat deploy local preflight --env-file infra/compose/local-providers.env");
    println!("  coat deploy local up --env-file infra/compose/local-providers.env");
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

fn chat_client_setup(mut args: ChatClientArgs) -> anyhow::Result<()> {
    args.mcp_url = effective_control_mcp_url(&args.mcp_url)?;
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
    println!("2. Preflight and start a local or remote control gateway. Local example:");
    println!("  coat deploy local preflight --env-file infra/compose/local-providers.env");
    println!("  coat deploy local up --env-file infra/compose/local-providers.env");
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
        ComposeSubcommand::Preflight(args) => {
            let env_files = effective_compose_env_files(args.env_file)?;
            let restate_cloud_env_file =
                effective_restate_cloud_env_file(&args.restate_cloud_env_file)?;
            run_local_compose_preflight(LocalComposePreflightInput {
                env_files: &env_files,
                restate_cloud: args.restate_cloud,
                restate_cloud_env_file: &restate_cloud_env_file,
                allow_uninitialized: effective_allow_uninitialized(args.allow_uninitialized)?,
                allow_stub_runners: effective_allow_stub_runners(args.allow_stub_runners)?,
            })
        }
        ComposeSubcommand::Up(mut args) => {
            args.env_file = effective_compose_env_files(args.env_file)?;
            args.profile = effective_compose_profiles(args.profile)?;
            args.restate_cloud_env_file =
                effective_restate_cloud_env_file(&args.restate_cloud_env_file)?;
            args.allow_uninitialized = effective_allow_uninitialized(args.allow_uninitialized)?;
            args.allow_stub_runners = effective_allow_stub_runners(args.allow_stub_runners)?;
            if args.restate_cloud {
                ensure_restate_cloud_env_file(&args.restate_cloud_env_file, args.init_env)?;
                if args.init_env {
                    return Ok(());
                }
            }
            if !args.skip_preflight {
                run_local_compose_preflight(LocalComposePreflightInput {
                    env_files: &args.env_file,
                    restate_cloud: args.restate_cloud,
                    restate_cloud_env_file: &args.restate_cloud_env_file,
                    allow_uninitialized: args.allow_uninitialized,
                    allow_stub_runners: args.allow_stub_runners,
                })?;
            }
            let register_cloud = args.register_cloud;
            let tunnel_name = effective_restate_tunnel_name(&args.tunnel_name)?;
            let service_url = effective_restate_service_url(&args.service_url)?;
            args.tunnel_name = tunnel_name.clone();
            args.service_url = service_url.clone();
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
        ComposeSubcommand::Config(mut args) => {
            args.env_file = effective_compose_env_files(args.env_file)?;
            args.profile = effective_compose_profiles(args.profile)?;
            args.restate_cloud_env_file =
                effective_restate_cloud_env_file(&args.restate_cloud_env_file)?;
            if args.restate_cloud && !args.allow_placeholder_env {
                ensure_restate_cloud_env_file(&args.restate_cloud_env_file, false)?;
            }
            run_docker_compose(
                compose_config_command_args(&args),
                "run docker compose config",
            )
        }
        ComposeSubcommand::Down(mut args) => {
            args.env_file = effective_compose_env_files(args.env_file)?;
            run_docker_compose(compose_down_command_args(&args), "run docker compose down")
        }
    }
}

struct LocalComposePreflightInput<'a> {
    env_files: &'a [PathBuf],
    restate_cloud: bool,
    restate_cloud_env_file: &'a Path,
    allow_uninitialized: bool,
    allow_stub_runners: bool,
}

fn run_local_compose_preflight(input: LocalComposePreflightInput<'_>) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("read current directory")?;
    let initialized = find_coat_project_root(&cwd).is_some();
    let mut failures = Vec::new();
    let mut warnings = Vec::new();

    if !Path::new("infra/compose/docker-compose.yml").is_file() {
        failures
            .push("missing infra/compose/docker-compose.yml; run from a COAT checkout".to_string());
    }

    let (docker_available, docker_detail) = probe_command("docker");
    if !docker_available {
        failures.push("docker CLI is not available on PATH".to_string());
    } else if !docker_detail.is_empty() {
        warnings.push(format!("docker CLI detected {docker_detail}"));
    }

    if input.restate_cloud && !input.restate_cloud_env_file.exists() {
        failures.push(format!(
            "{} is missing; run `coat deploy local up --restate-cloud --init-env` first",
            input.restate_cloud_env_file.display()
        ));
    }

    let env_values = match compose_env_values(input.env_files) {
        Ok(values) => values,
        Err(error) => {
            failures.push(error.to_string());
            BTreeMap::new()
        }
    };
    let (mut model_warnings, mut model_failures) = compose_model_preflight_findings(
        initialized,
        input.env_files,
        &env_values,
        input.allow_uninitialized,
        input.allow_stub_runners,
    );
    warnings.append(&mut model_warnings);
    failures.append(&mut model_failures);

    println!("COAT local Compose preflight");
    if failures.is_empty() && warnings.is_empty() {
        println!(
            "  ok: project init, Compose files, Docker, runner modes, and model env look usable"
        );
    }
    for warning in &warnings {
        println!("  warn: {warning}");
    }
    for failure in &failures {
        println!("  fail: {failure}");
    }

    if !failures.is_empty() {
        bail!(
            "local Compose preflight failed. Run `coat init`, configure models with `coat setup local-auth`, or pass `--allow-stub-runners` for an intentional stub smoke stack."
        );
    }
    Ok(())
}

fn effective_compose_env_files(env_files: Vec<PathBuf>) -> anyhow::Result<Vec<PathBuf>> {
    if env_files.is_empty() {
        let resolved = load_resolved_coat_config()?;
        let project_root = resolved.project_root.as_deref();
        let config_paths = resolved
            .config
            .local_deploy
            .env_files
            .iter()
            .map(|path| config_path(path, project_root))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let existing_config_paths = config_paths
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .collect::<Vec<_>>();
        if !existing_config_paths.is_empty() {
            eprintln!(
                "using local provider env file(s) from COAT config: {}",
                existing_config_paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return Ok(existing_config_paths);
        }
        if !config_paths.is_empty() {
            eprintln!(
                "configured local provider env file(s) do not exist yet: {}",
                config_paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let default = project_root
            .map(|root| root.join(DEFAULT_LOCAL_PROVIDER_ENV))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_LOCAL_PROVIDER_ENV));
        if default.exists() {
            eprintln!(
                "using default local provider env file: {}",
                default.display()
            );
            return Ok(vec![default]);
        }
    }
    Ok(env_files)
}

fn effective_compose_profiles(profiles: Vec<String>) -> anyhow::Result<Vec<String>> {
    if !profiles.is_empty() {
        return Ok(profiles);
    }
    Ok(load_resolved_coat_config()?.config.local_deploy.profiles)
}

fn effective_allow_stub_runners(flag: bool) -> anyhow::Result<bool> {
    Ok(flag
        || load_resolved_coat_config()?
            .config
            .local_deploy
            .allow_stub_runners
            .unwrap_or(false))
}

fn effective_allow_uninitialized(flag: bool) -> anyhow::Result<bool> {
    Ok(flag
        || load_resolved_coat_config()?
            .config
            .local_deploy
            .allow_uninitialized
            .unwrap_or(false))
}

fn effective_restate_cloud_env_file(path: &Path) -> anyhow::Result<PathBuf> {
    let default = Path::new(DEFAULT_RESTATE_CLOUD_ENV);
    if path != default {
        return Ok(path.to_path_buf());
    }
    let resolved = load_resolved_coat_config()?;
    match resolved
        .config
        .cloud
        .restate_cloud
        .env_file
        .or(resolved.config.local_deploy.restate_cloud_env_file)
    {
        Some(configured) => config_path(&configured, resolved.project_root.as_deref()),
        None => Ok(path.to_path_buf()),
    }
}

fn effective_restate_tunnel_name(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_RESTATE_TUNNEL_NAME {
        return Ok(value.to_string());
    }
    Ok(load_resolved_coat_config()?
        .config
        .cloud
        .restate_cloud
        .tunnel_name
        .unwrap_or_else(|| value.to_string()))
}

fn effective_restate_service_url(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_RESTATE_SERVICE_URL && value != DEFAULT_COORDINATOR_URL {
        return Ok(value.to_string());
    }
    Ok(load_resolved_coat_config()?
        .config
        .cloud
        .restate_cloud
        .service_url
        .unwrap_or_else(|| value.to_string()))
}

fn compose_env_values(env_files: &[PathBuf]) -> anyhow::Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for env_file in env_files {
        let content =
            fs::read_to_string(env_file).with_context(|| format!("read {}", env_file.display()))?;
        for (key, value) in parse_env_file_content(&content) {
            values.insert(key, value);
        }
    }
    for (key, value) in env::vars() {
        if key.starts_with("COAT_")
            || key.starts_with("CODEX_")
            || key.starts_with("CLAUDE_")
            || key.starts_with("ANTHROPIC_")
            || key.starts_with("OPENAI_")
            || key.starts_with("MODEL_PROVIDER_")
            || key.starts_with("LOCAL_MODEL_PROVIDER_")
            || key.starts_with("MEMORY_GATEWAY_")
            || key.starts_with("AWS_")
            || key.starts_with("HF_")
            || key.starts_with("HUGGINGFACE_")
        {
            values.insert(key, value);
        }
    }
    Ok(values)
}

fn parse_env_file_content(content: &str) -> BTreeMap<String, String> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let (key, value) = trimmed.split_once('=')?;
            Some((
                key.trim().to_string(),
                strip_env_value_comment(value.trim()).to_string(),
            ))
        })
        .collect()
}

fn strip_env_value_comment(value: &str) -> &str {
    value
        .split_once(" #")
        .map(|(value, _)| value)
        .unwrap_or(value)
        .trim()
}

fn compose_model_preflight_findings(
    initialized: bool,
    env_files: &[PathBuf],
    values: &BTreeMap<String, String>,
    allow_uninitialized: bool,
    allow_stub_runners: bool,
) -> (Vec<String>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut failures = Vec::new();

    if !initialized {
        let message = format!("missing {COAT_PROJECT_MARKER}; run `coat init` in this checkout");
        if allow_uninitialized {
            warnings.push(message);
        } else {
            failures.push(message);
        }
    }

    if env_files.is_empty() {
        warnings.push(format!(
            "no provider env file was supplied and {DEFAULT_LOCAL_PROVIDER_ENV} was not found"
        ));
    }

    let runner_modes = compose_runner_modes(values);
    let stub_lanes = runner_modes
        .iter()
        .filter(|(_, _, mode)| runner_mode_is_stub(mode))
        .map(|(name, key, _)| format!("{name} ({key})"))
        .collect::<Vec<_>>();
    if stub_lanes.len() == runner_modes.len() {
        let message = format!(
            "all Compose agent lanes are stubbed: {}",
            stub_lanes.join(", ")
        );
        if allow_stub_runners {
            warnings.push(message);
        } else {
            failures.push(message);
        }
    } else if !stub_lanes.is_empty() {
        warnings.push(format!("stubbed Compose lanes: {}", stub_lanes.join(", ")));
    }

    for (lane, key, mode) in runner_modes {
        if runner_mode_is_stub(&mode) {
            continue;
        }
        failures.extend(live_runner_setup_issues(lane, key, values));
    }

    if memory_embedding_needs_token(values) {
        warnings.push(
            "memory embeddings use the OpenAI endpoint but neither MEMORY_GATEWAY_EMBEDDING_TOKEN nor OPENAI_API_KEY is set".to_string(),
        );
    }

    (warnings, failures)
}

fn compose_runner_modes(
    values: &BTreeMap<String, String>,
) -> Vec<(&'static str, &'static str, String)> {
    [
        ("codex", "CODEX_RUNNER_MODE"),
        ("codex-reviewer", "CODEX_REVIEW_RUNNER_MODE"),
        ("claude-code", "CLAUDE_CODE_RUNNER_MODE"),
        ("staff-engineer", "STAFF_ENGINEER_RUNNER_MODE"),
        ("model-provider", "MODEL_PROVIDER_RUNNER_MODE"),
        (
            "model-provider-research",
            "MODEL_PROVIDER_RESEARCH_RUNNER_MODE",
        ),
        ("model-provider-local", "MODEL_PROVIDER_LOCAL_RUNNER_MODE"),
    ]
    .into_iter()
    .map(|(lane, key)| {
        (
            lane,
            key,
            env_value(values, key)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "stub".to_string()),
        )
    })
    .collect()
}

fn runner_mode_is_stub(mode: &str) -> bool {
    let normalized = mode.trim().to_ascii_lowercase();
    normalized.is_empty() || normalized == "stub"
}

fn live_runner_setup_issues(
    lane: &'static str,
    key: &'static str,
    values: &BTreeMap<String, String>,
) -> Vec<String> {
    match key {
        "CODEX_RUNNER_MODE" | "CODEX_REVIEW_RUNNER_MODE" => {
            if codex_auth_configured(values) {
                Vec::new()
            } else {
                vec![format!(
                    "{lane} is live but no Codex auth is configured; set OPENAI_API_KEY/CODEX_API_KEY, CODEX_AUTH_MODE=runner_local_device after `codex login`, or CODEX_APP_SERVER_URL with CODEX_AUTH_MODE=app_server"
                )]
            }
        }
        "CLAUDE_CODE_RUNNER_MODE" | "STAFF_ENGINEER_RUNNER_MODE" => {
            let auth_mode_key = if key == "STAFF_ENGINEER_RUNNER_MODE" {
                "STAFF_ENGINEER_AUTH_MODE"
            } else {
                "CLAUDE_CODE_AUTH_MODE"
            };
            if claude_auth_configured(values, auth_mode_key) {
                Vec::new()
            } else {
                vec![format!(
                    "{lane} is live but no Claude auth is configured; set ANTHROPIC_API_KEY/ANTHROPIC_AUTH_TOKEN/CLAUDE_CODE_OAUTH_TOKEN or {auth_mode_key}=runner_local_device after `claude login`"
                )]
            }
        }
        "MODEL_PROVIDER_RUNNER_MODE" => model_provider_setup_issues(
            lane,
            values,
            "MODEL_PROVIDER_KIND",
            "MODEL_PROVIDER_MODEL",
            "MODEL_PROVIDER_ENDPOINT",
        ),
        "MODEL_PROVIDER_RESEARCH_RUNNER_MODE" => model_provider_setup_issues(
            lane,
            values,
            "MODEL_PROVIDER_RESEARCH_KIND",
            "MODEL_PROVIDER_RESEARCH_MODEL",
            "MODEL_PROVIDER_RESEARCH_ENDPOINT",
        ),
        "MODEL_PROVIDER_LOCAL_RUNNER_MODE" => model_provider_setup_issues(
            lane,
            values,
            "LOCAL_MODEL_PROVIDER_KIND",
            "LOCAL_MODEL_PROVIDER_MODEL",
            "LOCAL_MODEL_PROVIDER_ENDPOINT",
        ),
        _ => Vec::new(),
    }
}

fn codex_auth_configured(values: &BTreeMap<String, String>) -> bool {
    any_env_present(values, &["OPENAI_API_KEY", "CODEX_API_KEY"])
        || auth_mode_allows_non_env_secret(values, "CODEX_AUTH_MODE")
        || (auth_mode_is(values, "CODEX_AUTH_MODE", "app_server")
            && env_present(values, "CODEX_APP_SERVER_URL"))
}

fn claude_auth_configured(values: &BTreeMap<String, String>, auth_mode_key: &str) -> bool {
    any_env_present(
        values,
        &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
        ],
    ) || auth_mode_allows_non_env_secret(values, auth_mode_key)
        || auth_mode_allows_non_env_secret(values, "CLAUDE_CODE_AUTH_MODE")
}

fn auth_mode_allows_non_env_secret(values: &BTreeMap<String, String>, key: &str) -> bool {
    env_value(values, key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "runner_local_device"
                    | "device"
                    | "device_auth"
                    | "browser"
                    | "oauth_device_broker"
                    | "external_broker"
            )
        })
        .unwrap_or(false)
}

fn auth_mode_is(values: &BTreeMap<String, String>, key: &str, expected: &str) -> bool {
    env_value(values, key)
        .map(|value| value.trim().eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn model_provider_setup_issues(
    lane: &'static str,
    values: &BTreeMap<String, String>,
    kind_key: &'static str,
    model_key: &'static str,
    endpoint_key: &'static str,
) -> Vec<String> {
    let mut issues = Vec::new();
    let kind = env_value(values, kind_key).unwrap_or_else(|| "open_ai_compatible".to_string());
    if !env_present(values, model_key) {
        issues.push(format!("{lane} is live but {model_key} is not set"));
    }
    let auth_mode_key = match kind_key {
        "MODEL_PROVIDER_RESEARCH_KIND" => "MODEL_PROVIDER_RESEARCH_AUTH_MODE",
        "LOCAL_MODEL_PROVIDER_KIND" => "LOCAL_MODEL_PROVIDER_AUTH_MODE",
        _ => "MODEL_PROVIDER_AUTH_MODE",
    };
    let auth_mode = env_value(values, auth_mode_key).unwrap_or_else(|| {
        if matches!(
            kind.as_str(),
            "ollama" | "vllm" | "llama_cpp" | "local_process"
        ) {
            "none".to_string()
        } else {
            "api_key_or_none".to_string()
        }
    });
    match kind.as_str() {
        "bedrock" => {
            if !any_env_present(values, &["AWS_REGION", "AWS_DEFAULT_REGION"]) {
                issues.push(format!("{lane} is bedrock-backed but no AWS region is set"));
            }
            if !any_env_present(values, &["AWS_PROFILE", "AWS_ACCESS_KEY_ID"])
                && !matches!(
                    auth_mode.as_str(),
                    "workload_identity" | "aws_profile" | "external_broker"
                )
            {
                issues.push(format!(
                    "{lane} is bedrock-backed but no AWS profile/access key or workload identity auth mode is set"
                ));
            }
        }
        "open_ai" => {
            if !env_present(values, "OPENAI_API_KEY")
                && !auth_mode_allows_non_env_secret(values, auth_mode_key)
            {
                issues.push(format!(
                    "{lane} is open_ai-backed but OPENAI_API_KEY or a brokered auth mode is not set"
                ));
            }
        }
        "hugging_face" => {
            if !env_present(values, endpoint_key) {
                issues.push(format!("{lane} is live but {endpoint_key} is not set"));
            }
            if auth_mode == "provider_token"
                && !any_env_present(
                    values,
                    &[
                        "MODEL_PROVIDER_API_KEY",
                        "MODEL_PROVIDER_RESEARCH_API_KEY",
                        "HF_TOKEN",
                        "HUGGINGFACE_TOKEN",
                    ],
                )
            {
                issues.push(format!(
                    "{lane} uses provider_token auth but no Hugging Face/model-provider token is set"
                ));
            }
        }
        _ => {
            if !env_present(values, endpoint_key) {
                issues.push(format!("{lane} is live but {endpoint_key} is not set"));
            }
        }
    }
    issues
}

fn memory_embedding_needs_token(values: &BTreeMap<String, String>) -> bool {
    let url = env_value(values, "MEMORY_GATEWAY_EMBEDDING_URL").unwrap_or_default();
    url.contains("api.openai.com")
        && !any_env_present(
            values,
            &["MEMORY_GATEWAY_EMBEDDING_TOKEN", "OPENAI_API_KEY"],
        )
}

fn any_env_present(values: &BTreeMap<String, String>, names: &[&str]) -> bool {
    names.iter().any(|name| env_present(values, name))
}

fn env_present(values: &BTreeMap<String, String>, name: &str) -> bool {
    env_value(values, name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn env_value(values: &BTreeMap<String, String>, name: &str) -> Option<String> {
    values
        .get(name)
        .cloned()
        .filter(|value| !value.trim().is_empty())
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
                "{} contains placeholders; edit it with RESTATE_ENVIRONMENT_ID, RESTATE_BEARER_TOKEN, RESTATE_CLOUD_REGION, and RESTATE_SIGNING_PUBLIC_KEY, then rerun `coat deploy local up --restate-cloud`",
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
            let args = effective_k8s_render_args(args)?;
            let source = effective_k8s_render_source()?;
            let manifest = fs::read_to_string(&source)
                .with_context(|| format!("read {}", source.display()))?;
            if let Some(parent) = args.output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&args.output, manifest)?;
            println!(
                "rendered {} from {}",
                args.output.display(),
                source.display()
            );
            Ok(())
        }
        K8sSubcommand::Apply(args) => apply_k8s_manifest(effective_k8s_apply_args(args)?),
        K8sSubcommand::Status(args) => k8s_status(effective_k8s_status_args(args)?),
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
        K8sSubcommand::ExecutorJob(args) => match args.command {
            ExecutorJobSubcommand::Render(args) => {
                render_executor_job(effective_executor_job_render_args(args)?)
            }
            ExecutorJobSubcommand::Apply(args) => {
                apply_executor_job(effective_executor_job_apply_args(args)?)
            }
        },
    }
}

fn effective_k8s_render_args(mut args: RenderArgs) -> anyhow::Result<RenderArgs> {
    if args.output == PathBuf::from(DEFAULT_K8S_RENDERED_MANIFEST) {
        let resolved = load_resolved_coat_config()?;
        if let Some(rendered) = resolved.config.kubernetes.rendered_manifest {
            args.output = config_path(&rendered, resolved.project_root.as_deref())?;
        }
    }
    Ok(args)
}

fn effective_k8s_render_source() -> anyhow::Result<PathBuf> {
    let resolved = load_resolved_coat_config()?;
    match resolved.config.kubernetes.manifest {
        Some(manifest) => config_path(&manifest, resolved.project_root.as_deref()),
        None => Ok(PathBuf::from(DEFAULT_K8S_MANIFEST)),
    }
}

fn effective_k8s_apply_args(mut args: K8sApplyArgs) -> anyhow::Result<K8sApplyArgs> {
    let resolved = load_resolved_coat_config()?;
    let project_root = resolved.project_root.as_deref();
    let k8s = resolved.config.kubernetes;
    if args.file == PathBuf::from(DEFAULT_K8S_MANIFEST) {
        if let Some(manifest) = k8s.manifest {
            args.file = config_path(&manifest, project_root)?;
        }
    }
    if args.kubectl == "kubectl" {
        if let Some(kubectl) = k8s.kubectl {
            args.kubectl = kubectl;
        }
    }
    if args.context.is_none() {
        args.context = k8s.context;
    }
    if args.kubeconfig.is_none() {
        args.kubeconfig = k8s
            .kubeconfig
            .map(|path| config_path(&path, project_root))
            .transpose()?;
    }
    if args.namespace.is_none() {
        args.namespace = k8s.namespace;
    }
    Ok(args)
}

fn effective_k8s_status_args(mut args: K8sStatusArgs) -> anyhow::Result<K8sStatusArgs> {
    let resolved = load_resolved_coat_config()?;
    let project_root = resolved.project_root.as_deref();
    let k8s = resolved.config.kubernetes;
    if args.kubectl == "kubectl" {
        if let Some(kubectl) = k8s.kubectl {
            args.kubectl = kubectl;
        }
    }
    if args.context.is_none() {
        args.context = k8s.context;
    }
    if args.kubeconfig.is_none() {
        args.kubeconfig = k8s
            .kubeconfig
            .map(|path| config_path(&path, project_root))
            .transpose()?;
    }
    if args.namespace == DEFAULT_K8S_NAMESPACE {
        if let Some(namespace) = k8s.namespace {
            args.namespace = namespace;
        }
    }
    Ok(args)
}

fn effective_executor_job_render_args(
    mut args: ExecutorJobRenderArgs,
) -> anyhow::Result<ExecutorJobRenderArgs> {
    let k8s = load_resolved_coat_config()?.config.kubernetes;
    if args.namespace == "jattg-sandboxes" {
        if let Some(namespace) = k8s.namespace {
            args.namespace = format!("{namespace}-sandboxes");
        }
    }
    if args.service_account == "jattg-sandbox-task" {
        if let Some(service_account) = k8s.service_account {
            args.service_account = service_account;
        }
    }
    Ok(args)
}

fn effective_executor_job_apply_args(
    mut args: ExecutorJobApplyArgs,
) -> anyhow::Result<ExecutorJobApplyArgs> {
    args.render = effective_executor_job_render_args(args.render)?;
    let resolved = load_resolved_coat_config()?;
    let project_root = resolved.project_root.as_deref();
    let k8s = resolved.config.kubernetes;
    if args.kubectl == "kubectl" {
        if let Some(kubectl) = k8s.kubectl {
            args.kubectl = kubectl;
        }
    }
    if args.context.is_none() {
        args.context = k8s.context;
    }
    if args.kubeconfig.is_none() {
        args.kubeconfig = k8s
            .kubeconfig
            .map(|path| config_path(&path, project_root))
            .transpose()?;
    }
    Ok(args)
}

fn apply_k8s_manifest(args: K8sApplyArgs) -> anyhow::Result<()> {
    if !args.file.exists() {
        bail!(
            "{} does not exist; pass --file or run `coat deploy cluster render --output {}` first",
            args.file.display(),
            args.file.display()
        );
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
            "{} does not exist; run `coat deploy cluster ephemeral-jobs render --output {}` first or pass --file",
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

fn render_executor_job(args: ExecutorJobRenderArgs) -> anyhow::Result<()> {
    let plan: SandboxLaunchPlan = read_json_file(&args.launch_plan)?;
    let manifest = executor_job_manifest(&plan, &args)?;
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &args.output,
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    println!(
        "rendered sandbox executor Job {} from {}",
        args.output.display(),
        args.launch_plan.display()
    );
    Ok(())
}

fn apply_executor_job(args: ExecutorJobApplyArgs) -> anyhow::Result<()> {
    render_executor_job(args.render.clone())?;
    let command_args = kubectl_apply_args(KubectlApplySpec {
        file: args.render.output,
        context: args.context,
        kubeconfig: args.kubeconfig,
        namespace: Some(args.render.namespace),
        dry_run: args.dry_run,
    })?;
    println!("{} {}", shell_quote(&args.kubectl), command_args.join(" "));
    let status = Command::new(&args.kubectl)
        .args(&command_args)
        .status()
        .context("run kubectl apply for sandbox executor Job")?;
    if !status.success() {
        bail!("kubectl apply exited with {status}");
    }
    Ok(())
}

fn k8s_status(args: K8sStatusArgs) -> anyhow::Result<()> {
    let deployments = if args.deployment.is_empty() {
        vec![
            "coordinator".to_string(),
            "goal-store".to_string(),
            "runner-registry".to_string(),
            "control-web".to_string(),
        ]
    } else {
        args.deployment.clone()
    };

    for deployment in deployments {
        let command_args = kubectl_rollout_status_args(&args, &deployment);
        println!("{} {}", shell_quote(&args.kubectl), command_args.join(" "));
        let status = Command::new(&args.kubectl)
            .args(&command_args)
            .status()
            .with_context(|| format!("run kubectl rollout status for {deployment}"))?;
        if !status.success() {
            bail!("kubectl rollout status for {deployment} exited with {status}");
        }
    }
    Ok(())
}

fn executor_job_manifest(
    plan: &SandboxLaunchPlan,
    args: &ExecutorJobRenderArgs,
) -> anyhow::Result<serde_json::Value> {
    let job_name = k8s_name(
        args.name
            .clone()
            .unwrap_or_else(|| format!("jattg-executor-{}", short_uuid(plan.task_id))),
    );
    let plan_config_name = k8s_name(format!("{job_name}-plan"));
    let image = args.image.clone().or_else(|| plan.image.clone()).unwrap_or_else(|| {
        "ghcr.io/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/jattg-agent-toolbox:latest"
            .to_string()
    });
    let runtime_class = args
        .runtime_class
        .clone()
        .or_else(|| plan.runtime_class.clone());
    let command = if args.executor_command.is_empty() {
        plan.command.clone()
    } else {
        args.executor_command.clone()
    };
    let extra_env = parse_kv_pairs(&args.env, "--env")?;
    let extra_labels = parse_kv_pairs(&args.label, "--label")?;
    let extra_annotations = parse_kv_pairs(&args.annotation, "--annotation")?;

    let mut labels = serde_json::Map::new();
    insert_json_string(
        &mut labels,
        "app.kubernetes.io/name",
        "jattg-sandbox-executor",
    );
    insert_json_string(&mut labels, "app.kubernetes.io/part-of", "jattg");
    insert_json_string(
        &mut labels,
        "app.kubernetes.io/component",
        "sandbox-executor",
    );
    insert_json_string(&mut labels, "jattg.dev/goal-id", plan.goal_id.to_string());
    insert_json_string(&mut labels, "jattg.dev/task-id", plan.task_id.to_string());
    insert_json_string(
        &mut labels,
        "jattg.dev/workspace-id",
        plan.workspace_id.to_string(),
    );
    insert_json_string(
        &mut labels,
        "jattg.dev/sandbox-backend",
        plan.backend.as_str(),
    );
    insert_json_string(
        &mut labels,
        "jattg.dev/network-access",
        network_access_as_str(&plan.network.access),
    );
    for (key, value) in &plan.network.network_policy_labels {
        insert_json_string(
            &mut labels,
            key.as_str(),
            sanitize_label_value(value.as_str()),
        );
    }
    for (key, value) in extra_labels {
        insert_json_string(&mut labels, key, sanitize_label_value(value));
    }

    let mut annotations = serde_json::Map::new();
    insert_json_string(
        &mut annotations,
        "jattg.dev/artifact-manifest-path",
        &plan.artifact_manifest_path,
    );
    insert_json_string(
        &mut annotations,
        "jattg.dev/checkpoint-manifest-path",
        &plan.checkpoint_manifest_path,
    );
    if let Some(egress) = &plan.network.egress_policy_ref {
        insert_json_string(&mut annotations, "jattg.dev/egress-policy-ref", egress);
    }
    if let Some(ingress) = &plan.network.ingress_policy_ref {
        insert_json_string(&mut annotations, "jattg.dev/ingress-policy-ref", ingress);
    }
    if let Some(apparmor) = &plan.security.apparmor_profile {
        insert_json_string(
            &mut annotations,
            "container.apparmor.security.beta.kubernetes.io/executor",
            apparmor,
        );
    }
    for (key, value) in extra_annotations {
        insert_json_string(&mut annotations, key, value);
    }

    let mut env = vec![
        env_var("COAT_GOAL_ID", plan.goal_id.to_string()),
        env_var("COAT_TASK_ID", plan.task_id.to_string()),
        env_var("COAT_WORKSPACE_ID", plan.workspace_id.to_string()),
        env_var("COAT_SANDBOX_BACKEND", plan.backend.as_str()),
        env_var(
            "COAT_SANDBOX_NETWORK_ACCESS",
            network_access_as_str(&plan.network.access),
        ),
        env_var("COAT_WORKSPACE_PATH", &plan.workspace_path),
        env_var("COAT_WORKSPACE_MOUNT_PATH", &args.workspace_mount_path),
        env_var("COAT_LAUNCH_PLAN_PATH", "/coat/sandbox-launch-plan.json"),
        env_var("COAT_ARTIFACT_MANIFEST_PATH", &plan.artifact_manifest_path),
        env_var(
            "COAT_CHECKPOINT_MANIFEST_PATH",
            &plan.checkpoint_manifest_path,
        ),
    ];
    if let Some(pids_limit) = plan.resources.pids_limit {
        env.push(env_var("COAT_PIDS_LIMIT", pids_limit.to_string()));
    }
    for (key, value) in &plan.environment {
        env.push(env_var(key.clone(), value));
    }
    for (key, value) in extra_env {
        env.push(env_var(key, value));
    }

    let mut container = serde_json::Map::new();
    container.insert("name".to_string(), serde_json::json!("executor"));
    container.insert("image".to_string(), serde_json::json!(image));
    container.insert(
        "imagePullPolicy".to_string(),
        serde_json::json!("IfNotPresent"),
    );
    if let Some((entrypoint, entrypoint_args)) = split_executor_command(&command) {
        container.insert("command".to_string(), serde_json::json!([entrypoint]));
        if !entrypoint_args.is_empty() {
            container.insert("args".to_string(), serde_json::json!(entrypoint_args));
        }
    }
    container.insert("env".to_string(), serde_json::Value::Array(env));
    container.insert(
        "volumeMounts".to_string(),
        serde_json::json!([
            {
                "name": "launch-plan",
                "mountPath": "/coat/sandbox-launch-plan.json",
                "subPath": "sandbox-launch-plan.json",
                "readOnly": true
            },
            {
                "name": "workspace",
                "mountPath": args.workspace_mount_path.clone()
            }
        ]),
    );
    if let Some(resources) = container_resources(&plan.resources) {
        container.insert("resources".to_string(), resources);
    }
    container.insert(
        "securityContext".to_string(),
        container_security_context(&plan.security),
    );

    let mut pod_spec = serde_json::Map::new();
    pod_spec.insert(
        "serviceAccountName".to_string(),
        serde_json::json!(args.service_account.clone()),
    );
    pod_spec.insert("restartPolicy".to_string(), serde_json::json!("Never"));
    if let Some(runtime_class) = runtime_class {
        pod_spec.insert(
            "runtimeClassName".to_string(),
            serde_json::json!(runtime_class),
        );
    }
    pod_spec.insert(
        "containers".to_string(),
        serde_json::Value::Array(vec![serde_json::Value::Object(container)]),
    );
    pod_spec.insert(
        "volumes".to_string(),
        serde_json::json!([
            {
                "name": "launch-plan",
                "configMap": {
                    "name": plan_config_name.clone()
                }
            },
            workspace_volume(args.workspace_pvc.as_deref())
        ]),
    );

    let config_map = serde_json::json!({
        "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {
            "name": plan_config_name,
            "namespace": args.namespace.clone(),
            "labels": labels.clone()
        },
        "data": {
            "sandbox-launch-plan.json": serde_json::to_string_pretty(plan)?
        }
    });
    let job = serde_json::json!({
        "apiVersion": "batch/v1",
            "kind": "Job",
            "metadata": {
            "name": job_name,
            "namespace": args.namespace.clone(),
            "labels": labels.clone(),
            "annotations": annotations.clone()
        },
        "spec": {
            "backoffLimit": args.backoff_limit,
            "activeDeadlineSeconds": args.active_deadline_seconds,
            "ttlSecondsAfterFinished": args.ttl_seconds_after_finished,
            "template": {
                "metadata": {
                    "labels": labels,
                    "annotations": annotations
                },
                "spec": pod_spec
            }
        }
    });
    Ok(serde_json::json!({
        "apiVersion": "v1",
        "kind": "List",
        "items": [config_map, job]
    }))
}

fn parse_kv_pairs(raw: &[String], flag_name: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let mut pairs = BTreeMap::new();
    for item in raw {
        let (key, value) = item
            .split_once('=')
            .with_context(|| format!("{flag_name} expects key=value, got {item:?}"))?;
        let key = key.trim();
        if key.is_empty() {
            bail!("{flag_name} key must not be empty");
        }
        pairs.insert(key.to_string(), value.trim().to_string());
    }
    Ok(pairs)
}

fn insert_json_string(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: impl Into<String>,
    value: impl Into<String>,
) {
    map.insert(key.into(), serde_json::Value::String(value.into()));
}

fn env_var(name: impl Into<String>, value: impl ToString) -> serde_json::Value {
    serde_json::json!({
        "name": name.into(),
        "value": value.to_string()
    })
}

fn split_executor_command(command: &[String]) -> Option<(String, Vec<String>)> {
    let (entrypoint, args) = command.split_first()?;
    Some((entrypoint.clone(), args.to_vec()))
}

fn container_resources(resources: &SandboxResourcePlan) -> Option<serde_json::Value> {
    let mut limits = serde_json::Map::new();
    if let Some(cpu_millis) = resources.cpu_limit_millis {
        insert_json_string(&mut limits, "cpu", format!("{cpu_millis}m"));
    }
    if let Some(memory_mb) = resources.memory_limit_mb {
        insert_json_string(&mut limits, "memory", format!("{memory_mb}Mi"));
    }
    if let Some(ephemeral_storage_mb) = resources.ephemeral_storage_mb {
        insert_json_string(
            &mut limits,
            "ephemeral-storage",
            format!("{ephemeral_storage_mb}Mi"),
        );
    }
    if limits.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "limits": limits.clone(),
        "requests": limits
    }))
}

fn container_security_context(security: &SandboxSecurityPlan) -> serde_json::Value {
    let mut context = serde_json::Map::new();
    context.insert(
        "readOnlyRootFilesystem".to_string(),
        security.read_only_rootfs.into(),
    );
    context.insert(
        "allowPrivilegeEscalation".to_string(),
        (!security.no_new_privileges).into(),
    );
    context.insert("runAsNonRoot".to_string(), security.run_as_non_root.into());
    if !security.drop_capabilities.is_empty() {
        context.insert(
            "capabilities".to_string(),
            serde_json::json!({ "drop": security.drop_capabilities.clone() }),
        );
    }
    if let Some(seccomp) = &security.seccomp_profile {
        context.insert("seccompProfile".to_string(), seccomp_profile(seccomp));
    }
    serde_json::Value::Object(context)
}

fn seccomp_profile(profile: &str) -> serde_json::Value {
    match profile {
        "RuntimeDefault" | "runtime_default" | "runtime-default" => {
            serde_json::json!({ "type": "RuntimeDefault" })
        }
        "Unconfined" | "unconfined" => serde_json::json!({ "type": "Unconfined" }),
        localhost => serde_json::json!({
            "type": "Localhost",
            "localhostProfile": localhost
        }),
    }
}

fn workspace_volume(pvc: Option<&str>) -> serde_json::Value {
    match pvc {
        Some(claim_name) => serde_json::json!({
            "name": "workspace",
            "persistentVolumeClaim": {
                "claimName": claim_name
            }
        }),
        None => serde_json::json!({
            "name": "workspace",
            "emptyDir": {}
        }),
    }
}

fn short_uuid(id: Uuid) -> String {
    id.to_string().chars().take(8).collect()
}

fn k8s_name(input: impl Into<String>) -> String {
    let mut value = input
        .into()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while value.contains("--") {
        value = value.replace("--", "-");
    }
    value = value.trim_matches('-').to_string();
    if value.is_empty() {
        return "jattg-executor".to_string();
    }
    value.truncate(63);
    value.trim_matches('-').to_string()
}

fn sanitize_label_value(value: impl Into<String>) -> String {
    let mut value = value
        .into()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    value.truncate(63);
    value
        .trim_matches(|ch| ch == '-' || ch == '_' || ch == '.')
        .to_string()
}

fn network_access_as_str(access: &NetworkAccess) -> &'static str {
    match access {
        NetworkAccess::Disabled => "disabled",
        NetworkAccess::Restricted => "restricted",
        NetworkAccess::Open => "open",
    }
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

fn kubectl_rollout_status_args(args: &K8sStatusArgs, deployment: &str) -> Vec<String> {
    let mut command_args = Vec::new();
    if let Some(context) = args.context.as_deref() {
        command_args.push("--context".to_string());
        command_args.push(context.to_string());
    }
    if let Some(kubeconfig) = &args.kubeconfig {
        command_args.push("--kubeconfig".to_string());
        command_args.push(kubeconfig.display().to_string());
    }
    command_args.push("--namespace".to_string());
    command_args.push(args.namespace.clone());
    command_args.push("rollout".to_string());
    command_args.push("status".to_string());
    command_args.push(if deployment.contains('/') {
        deployment.to_string()
    } else {
        format!("deployment/{deployment}")
    });
    if let Some(timeout) = args.timeout.as_deref() {
        command_args.push(format!("--timeout={timeout}"));
    }
    command_args
}

fn restate(args: RestateCommand) -> anyhow::Result<()> {
    match args.command {
        RestateSubcommand::CloudEnv(args) => {
            restate_cloud_env(effective_restate_cloud_env_args(args)?)
        }
        RestateSubcommand::TunnelDocker(args) => {
            restate_tunnel_docker(effective_restate_tunnel_docker_args(args)?)
        }
        RestateSubcommand::RegisterCloud(args) => {
            restate_register_cloud(effective_restate_register_cloud_args(args)?)
        }
    }
}

fn effective_restate_cloud_env_args(
    mut args: RestateCloudEnvArgs,
) -> anyhow::Result<RestateCloudEnvArgs> {
    let cloud = load_resolved_coat_config()?.config.cloud.restate_cloud;
    if args.tunnel_name == DEFAULT_RESTATE_TUNNEL_NAME {
        if let Some(tunnel_name) = cloud.tunnel_name.clone() {
            args.tunnel_name = tunnel_name;
        }
    }
    if args.region == DEFAULT_RESTATE_REGION {
        if let Some(region) = cloud.region.clone() {
            args.region = region;
        }
    }
    if args.ingress_url == DEFAULT_RESTATE_LOCAL_INGRESS {
        if let Some(ingress_url) = cloud.local_ingress_url.clone() {
            args.ingress_url = ingress_url;
        }
    }
    if args.admin_url == DEFAULT_RESTATE_LOCAL_ADMIN {
        if let Some(admin_url) = cloud.local_admin_url.clone() {
            args.admin_url = admin_url;
        }
    }
    if args.coordinator_url == DEFAULT_COORDINATOR_URL {
        if let Some(coordinator_url) = cloud.coordinator_url {
            args.coordinator_url = coordinator_url;
        }
    }
    Ok(args)
}

fn effective_restate_tunnel_docker_args(
    mut args: RestateTunnelDockerArgs,
) -> anyhow::Result<RestateTunnelDockerArgs> {
    let cloud = load_resolved_coat_config()?.config.cloud.restate_cloud;
    if args.tunnel_name == DEFAULT_RESTATE_TUNNEL_NAME {
        if let Some(tunnel_name) = cloud.tunnel_name.clone() {
            args.tunnel_name = tunnel_name;
        }
    }
    if args.region == DEFAULT_RESTATE_REGION {
        if let Some(region) = cloud.region {
            args.region = region;
        }
    }
    Ok(args)
}

fn effective_restate_register_cloud_args(
    mut args: RestateRegisterCloudArgs,
) -> anyhow::Result<RestateRegisterCloudArgs> {
    args.tunnel_name = effective_restate_tunnel_name(&args.tunnel_name)?;
    args.service_url = effective_restate_service_url(&args.service_url)?;
    Ok(args)
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
        "#   coat deploy restate register-cloud --tunnel-name {} --service-url {}",
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
        ChatClientArgs, Cli, Commands, ComposeConfigArgs, ComposeUpArgs, DEFAULT_GOAL_STORE_URL,
        DeploySubcommand, EphemeralJobsApplyArgs, ExecutorJobRenderArgs, HelmTemplateArgs,
        HelmUpgradeArgs, HumanSubcommand, K8sStatusArgs, KubectlApplySpec, PlanSubcommand,
        ProjectInitAction, ProjectInitCheck, apply_config_profile, bump_release_versions,
        chat_client_default_action, claude_mcp_json, codex_mcp_add_args,
        compose_config_command_args, compose_model_preflight_findings, compose_runner_modes,
        compose_up_command_args, endpoint_from_config, ensure_json_goal_id, executor_job_manifest,
        extract_follow_ups, helm_template_args, helm_upgrade_args, kubectl_apply_args,
        kubectl_ephemeral_jobs_apply_args, kubectl_rollout_status_args, latest_goal_id_from_value,
        merge_coat_config, parse_env_file_content, project_init_action, release_plan_json,
        replace_env_line, replace_toml_section_value, replace_yaml_root_value,
        restate_cloud_env_placeholders,
    };
    use clap::{CommandFactory, Parser};
    use coat_domain::{
        CoatCliConfig, CoatConfig, CoatLocalDeployConfig, CoatServiceEndpoints, NetworkAccess,
        SandboxBackend, SandboxLaunchPlan, SandboxNetworkPlan, SandboxResourcePlan,
        SandboxSecurityPlan,
    };
    use std::{collections::BTreeMap, path::PathBuf};
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
    fn visible_cli_hierarchy_hides_legacy_top_level_duplicates() {
        let command = Cli::command();
        let visible = command
            .get_subcommands()
            .filter(|command| !command.is_hide_set())
            .map(|command| command.get_name().to_string())
            .collect::<Vec<_>>();

        for expected in [
            "guide", "plan", "goal", "human", "deploy", "event", "runner", "memory", "store",
            "sandbox", "release", "setup", "init",
        ] {
            assert!(
                visible.contains(&expected.to_string()),
                "visible commands should include {expected}: {visible:?}"
            );
        }
        for legacy in [
            "compose",
            "k8s",
            "helm",
            "restate",
            "approve",
            "notify",
            "follow-ups",
        ] {
            assert!(
                !visible.contains(&legacy.to_string()),
                "legacy duplicate command should be hidden: {legacy}"
            );
        }
    }

    #[test]
    fn parses_root_config_profile_override() {
        let cli = Cli::parse_from([
            "coat",
            "--config-profile",
            "restate-cloud",
            "deploy",
            "local",
            "config",
            "--restate-cloud",
        ]);

        assert_eq!(cli.config_profile.as_deref(), Some("restate-cloud"));
        assert!(matches!(cli.command, Some(Commands::Deploy(_))));
    }

    #[test]
    fn canonical_hierarchy_parses_deploy_human_and_plan_followups() {
        let deploy = Cli::try_parse_from(["coat", "deploy", "local", "config"])
            .expect("parse deploy local config");
        assert!(matches!(
            deploy.command,
            Some(Commands::Deploy(ref deploy))
                if matches!(deploy.command, DeploySubcommand::Local(_))
        ));

        let preflight = Cli::try_parse_from([
            "coat",
            "deploy",
            "local",
            "preflight",
            "--allow-stub-runners",
        ])
        .expect("parse deploy local preflight");
        assert!(matches!(
            preflight.command,
            Some(Commands::Deploy(ref deploy))
                if matches!(deploy.command, DeploySubcommand::Local(_))
        ));

        let human = Cli::try_parse_from(["coat", "human", "notify", "--queue"])
            .expect("parse human notify");
        assert!(matches!(
            human.command,
            Some(Commands::Human(ref human))
                if matches!(human.command, HumanSubcommand::Notify(_))
        ));

        let follow_ups = Cli::try_parse_from(["coat", "plan", "follow-ups", "--json"])
            .expect("parse plan follow-ups");
        assert!(matches!(
            follow_ups.command,
            Some(Commands::Plan(ref plan))
                if matches!(plan.command, PlanSubcommand::FollowUps(_))
        ));
    }

    #[test]
    fn compose_preflight_blocks_uninitialized_all_stub_stack_by_default() {
        let values = BTreeMap::new();
        let (_warnings, failures) =
            compose_model_preflight_findings(false, &[], &values, false, false);

        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("missing .coat/project.json")),
            "uninitialized project should fail: {failures:?}"
        );
        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("all Compose agent lanes are stubbed")),
            "all-stub Compose should fail unless explicitly allowed: {failures:?}"
        );
    }

    #[test]
    fn compose_preflight_allows_intentional_stub_smoke_stack() {
        let values = BTreeMap::new();
        let (warnings, failures) =
            compose_model_preflight_findings(true, &[], &values, false, true);

        assert!(
            failures.is_empty(),
            "stub smoke should be allowed: {failures:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("all Compose agent lanes are stubbed")),
            "allowed stubs should still be visible: {warnings:?}"
        );
    }

    #[test]
    fn compose_preflight_requires_live_model_configuration() {
        let values = BTreeMap::from([
            ("MODEL_PROVIDER_RUNNER_MODE".to_string(), "live".to_string()),
            (
                "MODEL_PROVIDER_KIND".to_string(),
                "open_ai_compatible".to_string(),
            ),
        ]);
        let (_warnings, failures) =
            compose_model_preflight_findings(true, &[], &values, false, true);

        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("MODEL_PROVIDER_MODEL")),
            "live model-provider should require a model: {failures:?}"
        );
        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("MODEL_PROVIDER_ENDPOINT")),
            "live OpenAI-compatible provider should require an endpoint: {failures:?}"
        );
    }

    #[test]
    fn compose_preflight_accepts_runner_local_device_auth() {
        let values = BTreeMap::from([
            ("CODEX_RUNNER_MODE".to_string(), "live".to_string()),
            (
                "CODEX_AUTH_MODE".to_string(),
                "runner_local_device".to_string(),
            ),
            ("CLAUDE_CODE_RUNNER_MODE".to_string(), "live".to_string()),
            (
                "CLAUDE_CODE_AUTH_MODE".to_string(),
                "runner_local_device".to_string(),
            ),
            ("STAFF_ENGINEER_RUNNER_MODE".to_string(), "live".to_string()),
            (
                "STAFF_ENGINEER_AUTH_MODE".to_string(),
                "oauth_device_broker".to_string(),
            ),
            (
                "MODEL_PROVIDER_LOCAL_RUNNER_MODE".to_string(),
                "live".to_string(),
            ),
            (
                "LOCAL_MODEL_PROVIDER_KIND".to_string(),
                "ollama".to_string(),
            ),
            (
                "LOCAL_MODEL_PROVIDER_AUTH_MODE".to_string(),
                "none".to_string(),
            ),
            (
                "LOCAL_MODEL_PROVIDER_MODEL".to_string(),
                "llama3.1".to_string(),
            ),
            (
                "LOCAL_MODEL_PROVIDER_ENDPOINT".to_string(),
                "http://host.docker.internal:11434/v1".to_string(),
            ),
        ]);
        let (_warnings, failures) =
            compose_model_preflight_findings(true, &[PathBuf::from("env")], &values, false, true);

        assert!(
            failures.is_empty(),
            "device/browser and brokered auth modes should unblock live runner lanes: {failures:?}"
        );
    }

    #[test]
    fn compose_preflight_requires_explicit_codex_app_server_auth() {
        let values = BTreeMap::from([
            ("CODEX_RUNNER_MODE".to_string(), "live".to_string()),
            (
                "CODEX_APP_SERVER_URL".to_string(),
                "http://host.docker.internal:1455".to_string(),
            ),
        ]);
        let (_warnings, failures) =
            compose_model_preflight_findings(true, &[PathBuf::from("env")], &values, false, true);

        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("no Codex auth is configured")),
            "a bare app-server URL should not satisfy Codex auth without CODEX_AUTH_MODE=app_server: {failures:?}"
        );

        let values = BTreeMap::from([
            ("CODEX_RUNNER_MODE".to_string(), "live".to_string()),
            ("CODEX_AUTH_MODE".to_string(), "app_server".to_string()),
            (
                "CODEX_APP_SERVER_URL".to_string(),
                "http://host.docker.internal:1455".to_string(),
            ),
        ]);
        let (_warnings, failures) =
            compose_model_preflight_findings(true, &[PathBuf::from("env")], &values, false, true);

        assert!(
            failures.is_empty(),
            "explicit Codex App Server auth mode with a URL should satisfy preflight: {failures:?}"
        );
    }

    #[test]
    fn compose_env_parser_and_runner_modes_honor_non_stub_values() {
        let values = parse_env_file_content(
            r#"
            # comments are ignored
            CODEX_RUNNER_MODE=live
            MODEL_PROVIDER_LOCAL_RUNNER_MODE=live # comment
            LOCAL_MODEL_PROVIDER_MODEL=llama3.1
            LOCAL_MODEL_PROVIDER_ENDPOINT=http://host.docker.internal:11434/v1
            "#,
        );
        let modes = compose_runner_modes(&values);

        assert!(
            modes
                .iter()
                .any(|(lane, _key, mode)| *lane == "codex" && mode == "live")
        );
        assert!(
            modes
                .iter()
                .any(|(lane, _key, mode)| *lane == "model-provider-local" && mode == "live")
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
    fn kubectl_rollout_status_args_target_default_namespace_and_timeout() {
        let args = K8sStatusArgs {
            kubectl: "kubectl".to_string(),
            context: Some("prod".to_string()),
            kubeconfig: Some(PathBuf::from("/tmp/kubeconfig")),
            namespace: "jattg".to_string(),
            timeout: Some("120s".to_string()),
            deployment: Vec::new(),
        };

        assert_eq!(
            kubectl_rollout_status_args(&args, "coordinator"),
            vec![
                "--context",
                "prod",
                "--kubeconfig",
                "/tmp/kubeconfig",
                "--namespace",
                "jattg",
                "rollout",
                "status",
                "deployment/coordinator",
                "--timeout=120s",
            ]
        );
        assert_eq!(
            kubectl_rollout_status_args(&args, "statefulset/postgres"),
            vec![
                "--context",
                "prod",
                "--kubeconfig",
                "/tmp/kubeconfig",
                "--namespace",
                "jattg",
                "rollout",
                "status",
                "statefulset/postgres",
                "--timeout=120s",
            ]
        );
    }

    #[test]
    fn executor_job_manifest_projects_launch_plan_to_bounded_job() {
        let goal_id = Uuid::parse_str("018f8f2f-1fd8-7688-bb12-8bfb6b756602").expect("uuid");
        let task_id = Uuid::parse_str("018f8f2f-1fd8-7688-bb12-8bfb6b756603").expect("uuid");
        let workspace_id = Uuid::parse_str("018f8f2f-1fd8-7688-bb12-8bfb6b756604").expect("uuid");
        let plan = SandboxLaunchPlan {
            goal_id,
            task_id,
            workspace_id,
            backend: SandboxBackend::KubernetesJob,
            runtime_class: Some("gvisor".to_string()),
            image: Some("ghcr.io/example/custom-executor:0.1.0".to_string()),
            workspace_path: "/workspaces/goal/task".to_string(),
            artifact_manifest_path: "/workspace/artifacts/artifact-manifest.json".to_string(),
            checkpoint_manifest_path: "/workspace/checkpoints/checkpoint-manifest.json".to_string(),
            command: vec![
                "/bin/sh".to_string(),
                "-lc".to_string(),
                "cargo test --workspace".to_string(),
            ],
            environment: BTreeMap::from([("RUST_LOG".to_string(), "info".to_string())]),
            required_capabilities: Vec::new(),
            resources: SandboxResourcePlan {
                cpu_limit_millis: Some(750),
                memory_limit_mb: Some(1024),
                pids_limit: Some(128),
                ephemeral_storage_mb: Some(2048),
            },
            security: SandboxSecurityPlan {
                read_only_rootfs: true,
                no_new_privileges: true,
                run_as_non_root: true,
                seccomp_profile: Some("RuntimeDefault".to_string()),
                apparmor_profile: Some("runtime/default".to_string()),
                drop_capabilities: vec!["ALL".to_string()],
            },
            network: SandboxNetworkPlan {
                access: NetworkAccess::Restricted,
                deny_by_default: true,
                egress_policy_ref: Some("control-plane-only".to_string()),
                ingress_policy_ref: None,
                network_policy_labels: BTreeMap::from([(
                    "jattg.dev/network-profile".to_string(),
                    "control-plane-only".to_string(),
                )]),
                allowed_internal_services: vec!["runner-registry".to_string()],
            },
            git_result: None,
            object_prefix: None,
            warnings: Vec::new(),
        };
        let args = ExecutorJobRenderArgs {
            launch_plan: PathBuf::from("sandbox-launch-plan.json"),
            output: PathBuf::from("/tmp/executor-job.json"),
            namespace: "jattg-sandboxes".to_string(),
            name: Some("task-executor".to_string()),
            image: None,
            service_account: "jattg-sandbox-task".to_string(),
            runtime_class: None,
            workspace_pvc: Some("task-workspaces".to_string()),
            workspace_mount_path: "/workspace".to_string(),
            backoff_limit: 0,
            active_deadline_seconds: 900,
            ttl_seconds_after_finished: 120,
            executor_command: Vec::new(),
            env: vec!["EXTRA_FLAG=true".to_string()],
            label: vec!["owner=validator".to_string()],
            annotation: vec!["example.com/reason=smoke".to_string()],
        };

        let manifest = executor_job_manifest(&plan, &args).expect("manifest");
        assert_eq!(manifest["kind"], "List");
        let items = manifest["items"].as_array().expect("items");
        assert_eq!(items[0]["kind"], "ConfigMap");
        assert_eq!(items[1]["kind"], "Job");
        let job = &items[1];
        assert_eq!(job["metadata"]["name"], "task-executor");
        assert_eq!(
            job["metadata"]["labels"]["jattg.dev/task-id"],
            task_id.to_string()
        );
        assert_eq!(job["metadata"]["labels"]["owner"], "validator");
        assert_eq!(
            job["metadata"]["annotations"]["jattg.dev/egress-policy-ref"],
            "control-plane-only"
        );
        assert_eq!(job["spec"]["activeDeadlineSeconds"], 900);
        assert_eq!(
            job["spec"]["template"]["spec"]["runtimeClassName"],
            "gvisor"
        );
        let container = &job["spec"]["template"]["spec"]["containers"][0];
        assert_eq!(container["image"], "ghcr.io/example/custom-executor:0.1.0");
        assert_eq!(container["command"], serde_json::json!(["/bin/sh"]));
        assert_eq!(
            container["args"],
            serde_json::json!(["-lc", "cargo test --workspace"])
        );
        assert_eq!(container["resources"]["limits"]["cpu"], "750m");
        assert_eq!(container["resources"]["limits"]["memory"], "1024Mi");
        assert_eq!(
            container["securityContext"]["allowPrivilegeEscalation"],
            false
        );
        assert_eq!(
            job["spec"]["template"]["spec"]["volumes"][1]["persistentVolumeClaim"]["claimName"],
            "task-workspaces"
        );
    }

    #[test]
    fn helm_template_args_render_release_with_values_and_sets() {
        let args = HelmTemplateArgs {
            helm: "helm".to_string(),
            release: "jattg".to_string(),
            chart: PathBuf::from("infra/helm/jattg"),
            values: vec![PathBuf::from("operator-values.yaml")],
            set_values: vec!["global.imageTag=0.2.0".to_string()],
            namespace: Some("jattg".to_string()),
            output: Some(PathBuf::from("/tmp/jattg.yaml")),
            include_crds: true,
        };

        assert_eq!(
            helm_template_args(&args),
            vec![
                "template",
                "jattg",
                "infra/helm/jattg",
                "--values",
                "operator-values.yaml",
                "--set",
                "global.imageTag=0.2.0",
                "--namespace",
                "jattg",
                "--include-crds",
            ]
        );
    }

    #[test]
    fn helm_upgrade_args_install_with_namespace_values_wait_and_dry_run() {
        let args = HelmUpgradeArgs {
            helm: "helm".to_string(),
            release: "jattg".to_string(),
            chart: PathBuf::from("infra/helm/jattg"),
            values: vec![PathBuf::from("operator-values.yaml")],
            set_values: vec!["config.COAT_GOAL_STORE_BACKEND=postgres".to_string()],
            namespace: "jattg".to_string(),
            no_create_namespace: false,
            dry_run: true,
            wait: true,
            timeout: Some("10m".to_string()),
        };

        assert_eq!(
            helm_upgrade_args(&args),
            vec![
                "upgrade",
                "--install",
                "jattg",
                "infra/helm/jattg",
                "--namespace",
                "jattg",
                "--create-namespace",
                "--values",
                "operator-values.yaml",
                "--set",
                "config.COAT_GOAL_STORE_BACKEND=postgres",
                "--dry-run",
                "--wait",
                "--timeout",
                "10m",
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
            skip_preflight: false,
            allow_uninitialized: false,
            allow_stub_runners: false,
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
            allow_placeholder_env: false,
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
    fn coat_config_merge_preserves_project_defaults_and_user_overrides() {
        let mut base = CoatConfig {
            service_endpoints: CoatServiceEndpoints {
                goal_store_url: Some("http://localhost:9088".to_string()),
                ..CoatServiceEndpoints::default()
            },
            local_deploy: CoatLocalDeployConfig {
                env_files: vec!["infra/compose/local-providers.env".to_string()],
                allow_stub_runners: Some(false),
                ..CoatLocalDeployConfig::default()
            },
            ..CoatConfig::default()
        };
        let overlay = CoatConfig {
            service_endpoints: CoatServiceEndpoints {
                goal_store_url: Some("http://remote-goal-store:9088".to_string()),
                ..CoatServiceEndpoints::default()
            },
            local_deploy: CoatLocalDeployConfig {
                env_files: vec![
                    "infra/compose/local-providers.env".to_string(),
                    "~/.coat/local-providers.env".to_string(),
                ],
                allow_stub_runners: Some(true),
                ..CoatLocalDeployConfig::default()
            },
            ..CoatConfig::default()
        };

        merge_coat_config(&mut base, overlay);

        assert_eq!(
            base.service_endpoints.goal_store_url.as_deref(),
            Some("http://remote-goal-store:9088")
        );
        assert_eq!(
            base.local_deploy.env_files,
            vec![
                "infra/compose/local-providers.env".to_string(),
                "~/.coat/local-providers.env".to_string()
            ]
        );
        assert_eq!(base.local_deploy.allow_stub_runners, Some(true));
    }

    #[test]
    fn standard_config_profiles_drive_cloud_and_eks_defaults() {
        let mut restate_cloud = CoatConfig::project_defaults();
        apply_config_profile(&mut restate_cloud, "restate-cloud").expect("restate profile");
        assert_eq!(
            restate_cloud.defaults.restate_ingress.as_deref(),
            Some("http://localhost:18080")
        );
        assert_eq!(
            restate_cloud.cloud.restate_cloud.env_file.as_deref(),
            Some("infra/compose/restate-cloud.env")
        );
        assert_eq!(
            restate_cloud.local_deploy.profiles,
            vec!["restate-cloud".to_string()]
        );

        let mut eks = CoatConfig::project_defaults();
        apply_config_profile(&mut eks, "eks").expect("eks profile");
        assert_eq!(eks.kubernetes.namespace.as_deref(), Some("jattg"));
        assert_eq!(
            eks.kubernetes.helm_chart.as_deref(),
            Some("infra/helm/jattg")
        );
        assert_eq!(eks.cloud.object_store.as_deref(), Some("s3"));
        assert_eq!(
            eks.kubernetes.workload_identity.as_deref(),
            Some("irsa_or_eks_pod_identity")
        );
    }

    #[test]
    fn endpoint_resolution_only_replaces_builtin_defaults() {
        assert_eq!(
            endpoint_from_config(
                DEFAULT_GOAL_STORE_URL,
                DEFAULT_GOAL_STORE_URL,
                Some("http://profile-goal-store:9088".to_string())
            ),
            "http://profile-goal-store:9088"
        );
        assert_eq!(
            endpoint_from_config(
                "http://explicit-goal-store:9088",
                DEFAULT_GOAL_STORE_URL,
                Some("http://profile-goal-store:9088".to_string())
            ),
            "http://explicit-goal-store:9088"
        );
        assert_eq!(
            endpoint_from_config(DEFAULT_GOAL_STORE_URL, DEFAULT_GOAL_STORE_URL, None),
            DEFAULT_GOAL_STORE_URL
        );
    }

    #[test]
    fn project_init_policy_requires_initialized_durable_commands() {
        let cli = CoatCliConfig {
            warn_uninitialized: Some(true),
            require_project_for_durable_commands: Some(true),
            ..CoatCliConfig::default()
        };

        assert_eq!(
            project_init_action(false, ProjectInitCheck::Durable, &cli, false),
            ProjectInitAction::Fail
        );
        assert_eq!(
            project_init_action(false, ProjectInitCheck::Durable, &cli, true),
            ProjectInitAction::Warn
        );
        assert_eq!(
            project_init_action(false, ProjectInitCheck::WarnOnly, &cli, false),
            ProjectInitAction::Warn
        );
        assert_eq!(
            project_init_action(true, ProjectInitCheck::Durable, &cli, false),
            ProjectInitAction::Proceed
        );
    }

    #[test]
    fn project_init_policy_can_be_relaxed_by_config() {
        let cli = CoatCliConfig {
            warn_uninitialized: Some(false),
            require_project_for_durable_commands: Some(false),
            ..CoatCliConfig::default()
        };

        assert_eq!(
            project_init_action(false, ProjectInitCheck::Durable, &cli, false),
            ProjectInitAction::Proceed
        );
        assert_eq!(
            project_init_action(false, ProjectInitCheck::WarnOnly, &cli, false),
            ProjectInitAction::Proceed
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
