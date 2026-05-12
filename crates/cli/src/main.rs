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
    time::Duration,
};

use anyhow::{Context, bail};
use clap::{Args, CommandFactory, Parser, Subcommand};
use coat_domain::{
    BranchRequest, BranchSelectionRequest, ChildTaskRequest, CoatCliConfig, CoatCloudConfig,
    CoatConfig, CoatConfigPaths, CoatKubernetesConfig, CoatLlmGatewayConfig, CoatLocalDeployConfig,
    CoatModelRoutingConfig, CoatOperatorDefaults, CoatProfileConfig, CoatProjectConfig,
    CoatRestateCloudConfig, CoatServiceEndpoints, CoatUserConfig, ControlLoopMode,
    DelayedComputeThunkRequest, DelayedComputeThunkResumeRequest, EventSource, ExternalEvent,
    GoalAuthoringGuidance, GoalHierarchyRole, GoalPlan, GoalPriorityVoteRequest, GoalRecord,
    GoalSpec, GoalVoteDirection, GoalVoteSource, GraphColorRef, HumanApproval,
    MechanismBallotRequest, MechanismRoundRequest, MemoryContextRequest, MemoryEditPreviewRequest,
    MemoryEditRequest, MemoryJoinRequest, MemoryRepairRequest, MemoryRetractRequest,
    MemorySearchRequest, MemoryWriteRequest, NetworkAccess, NotificationRequest,
    PlanCandidateSelectionRequest, PlanCandidateVoteRequest, PlanCompileRequest, PlanDraftRequest,
    PlanQuestion, PlanQuestionStatus, PlanRevisionRequest, PlanningMode, RestartRequest,
    ReviewDoctrine, ReviewDoctrinePreset, RunnerDispatchRequest, RunnerRegistration,
    RunnerScalingRequest, SandboxLaunchPlan, SandboxResourcePlan, SandboxSecurityPlan,
    StandardReviewCheck, SteeringDirective, SteeringDirectiveKind, SubgoalSpec, TaskPriority,
    TaskPurpose, TaskPurposeKind, TaskQuery, TaskStatus, TriggeredGoalRequest, WebSearchRequest,
    WorkerKind,
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
const DEFAULT_TOOL_REGISTRY_URL: &str = "http://localhost:9084";
const DEFAULT_RUNNER_REGISTRY_URL: &str = "http://localhost:9085";
const DEFAULT_NOTIFIER_URL: &str = "http://localhost:9086";
const DEFAULT_MEMORY_GATEWAY_URL: &str = "http://localhost:9087";
const DEFAULT_GOAL_STORE_URL: &str = "http://localhost:9088";
const DEFAULT_EVENT_GATEWAY_URL: &str = "http://localhost:9089";
const DEFAULT_CONTROL_MCP_URL: &str = "http://localhost:9090/mcp";
const MODELS_DEV_API_URL: &str = "https://models.dev/api.json";
const DEFAULT_PROJECT_MODEL_INDEX: &str = ".coat/model-index.json";
const DEFAULT_USER_MODEL_INDEX: &str = "~/.coat/cache/models.dev.api.json";
const MAX_INDEXED_MODEL_CHOICES: usize = 40;
const MODEL_INDEX_REFRESH_DEBOUNCE_SECONDS: u64 = 60 * 60;

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
    #[command(about = "Open guided setup and human-queue workflows")]
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
    #[command(about = "Call MCP/tool-registry utilities")]
    Tool(ToolCommand),
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
    #[command(about = "Ask the runner registry for a bounded capacity recommendation")]
    CapacityPlan(RunnerCapacityPlanArgs),
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
        env = "COAT_RUNNER_REGISTRY_URL",
        default_value = "http://localhost:9085"
    )]
    registry_url: String,
}

#[derive(Debug, Args)]
struct RunnerRegisterArgs {
    #[arg(
        long,
        env = "COAT_RUNNER_REGISTRY_URL",
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
        env = "COAT_RUNNER_REGISTRY_URL",
        default_value = "http://localhost:9085"
    )]
    registry_url: String,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct RunnerCapacityPlanArgs {
    #[arg(
        long,
        env = "COAT_RUNNER_REGISTRY_URL",
        default_value = "http://localhost:9085"
    )]
    registry_url: String,
    #[arg(
        long,
        help = "Demand/supply request JSON; policy may be omitted to use config.runner_capacity"
    )]
    file: PathBuf,
    #[arg(
        long,
        help = "Do not fill an omitted/default request policy from COAT config"
    )]
    ignore_config_policy: bool,
}

#[derive(Debug, Args)]
struct ToolCommand {
    #[command(subcommand)]
    command: ToolSubcommand,
}

#[derive(Debug, Subcommand)]
enum ToolSubcommand {
    #[command(about = "List configured MCP/tool-registry tools")]
    List(ToolRegistryArgs),
    #[command(about = "Call a named MCP tool with JSON arguments")]
    Call(ToolCallArgs),
    #[command(about = "Route a web/reference search through coat_web_search")]
    WebSearch(ToolWebSearchArgs),
}

#[derive(Debug, Args)]
struct ToolRegistryArgs {
    #[arg(
        long,
        env = "COAT_TOOL_REGISTRY_URL",
        default_value = "http://localhost:9084"
    )]
    tool_registry_url: String,
    #[arg(
        long,
        env = "COAT_TOOL_REGISTRY_TOKEN",
        help = "Bearer token for the tool registry; falls back to MCP_TOOL_TOKEN when unset"
    )]
    token: Option<String>,
}

#[derive(Debug, Args)]
struct ToolCallArgs {
    #[command(flatten)]
    registry: ToolRegistryArgs,
    #[arg(long)]
    name: String,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct ToolWebSearchArgs {
    #[command(flatten)]
    registry: ToolRegistryArgs,
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
    ComputeGraph(GoalIdArgs),
    Tasks(GoalTasksArgs),
    Lint(GoalLintArgs),
    Steer(SteerGoalArgs),
    SteerStandard(SteerStandardGoalArgs),
    ReviewChecks,
    Vote(GoalVoteArgs),
    Mechanism(GoalMechanismCommand),
    Thunk(GoalThunkCommand),
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
struct GoalVoteArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
    #[arg(long, value_parser = ["up", "down"])]
    direction: String,
    #[arg(long, default_value_t = 1)]
    weight: u32,
    #[arg(long, default_value = "human", value_parser = ["human", "coordinator", "agent", "system"])]
    source: String,
    #[arg(long, default_value = "operator")]
    voter: String,
    #[arg(long)]
    reason: String,
    #[arg(long, value_parser = ["overarching_goal", "peer_goal", "subgoal"])]
    suggested_role: Option<String>,
}

#[derive(Debug, Args)]
struct GoalMechanismCommand {
    #[command(subcommand)]
    command: GoalMechanismSubcommand,
}

#[derive(Debug, Subcommand)]
enum GoalMechanismSubcommand {
    Start(GoalMechanismArgs),
    Ballot(GoalMechanismArgs),
}

#[derive(Debug, Args)]
struct GoalThunkCommand {
    #[command(subcommand)]
    command: GoalThunkSubcommand,
}

#[derive(Debug, Subcommand)]
enum GoalThunkSubcommand {
    Create(GoalThunkArgs),
}

#[derive(Debug, Args)]
struct GoalThunkArgs {
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
struct GoalMechanismArgs {
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
#[command(
    about = "Lint, render, install, rollback, and package the jattg Helm chart",
    after_help = "Examples:\n  coat deploy chart lint\n  coat deploy chart template --output /tmp/jattg.yaml\n  coat deploy chart upgrade --values path/to/operator-values.yaml --wait\n  coat deploy chart package --chart-version 0.0.3 --app-version 0.0.3"
)]
struct HelmCommand {
    #[command(subcommand)]
    command: HelmSubcommand,
}

#[derive(Debug, Subcommand)]
enum HelmSubcommand {
    #[command(about = "Run helm lint against the jattg chart")]
    Lint(HelmLintArgs),
    #[command(about = "Render the jattg Helm chart with optional values and set overrides")]
    Template(HelmTemplateArgs),
    #[command(about = "Install or upgrade the jattg Helm release")]
    Upgrade(HelmUpgradeArgs),
    #[command(about = "Rollback the jattg Helm release")]
    Rollback(HelmRollbackArgs),
    #[command(about = "Package the jattg Helm chart into dist artifacts")]
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
    #[command(about = "Run provider device/browser login flows and optional local preflight")]
    Login(LoginArgs),
    #[command(about = "Run AWS SSO login, update local env, and optionally preflight")]
    Sso(SsoArgs),
    #[command(about = "Refresh or inspect the external model index used by setup wizards")]
    ModelIndex(ModelIndexCommand),
    #[command(about = "Write or inspect COAT project and user config")]
    Config(ConfigSetupArgs),
    #[command(about = "Create local provider env files and guided auth setup")]
    LocalAuth(LocalAuthArgs),
    #[command(about = "Install control gateway MCP and skill integration")]
    ChatClient(ChatClientArgs),
}

#[derive(Debug, Args)]
struct LoginArgs {
    #[arg(long, help = "Run Codex device/browser login on this runner node")]
    codex: bool,
    #[arg(long, help = "Run Claude Code auth login on this runner node")]
    claude: bool,
    #[arg(
        long,
        value_name = "EMAIL",
        help = "Pass an email prefill to Claude Code auth login"
    )]
    claude_email: Option<String>,
    #[arg(long, help = "Force Claude Code organization SSO during auth login")]
    claude_sso: bool,
    #[arg(
        long,
        help = "Use Claude Console auth for API usage billing instead of subscription auth"
    )]
    claude_console: bool,
    #[arg(long, help = "Run Hugging Face CLI login on this runner node")]
    hf: bool,
    #[arg(
        long,
        value_name = "MODEL",
        help = "Pull an Ollama model needed by local model-provider runners"
    )]
    ollama_model: Vec<String>,
    #[arg(
        long,
        default_value = "infra/compose/local-providers.env",
        help = "Provider env file to use when --preflight is enabled"
    )]
    env_file: PathBuf,
    #[arg(long, help = "Run local Compose preflight after login/setup actions")]
    preflight: bool,
    #[arg(long, help = "Allow intentionally stubbed runners during preflight")]
    allow_stub_runners: bool,
    #[arg(long, help = "Print provider commands without running them")]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct SsoArgs {
    #[arg(long, help = "AWS profile to pass to `aws sso login --profile`")]
    profile: Option<String>,
    #[arg(
        long,
        default_value = "infra/compose/local-providers.env",
        help = "Provider env file to update or preflight"
    )]
    env_file: PathBuf,
    #[arg(long, help = "Write AWS_PROFILE and auth-mode values to the env file")]
    write_env: bool,
    #[arg(
        long,
        help = "Also configure the model-provider lane for live Bedrock routing"
    )]
    bedrock_live: bool,
    #[arg(long, help = "Run local Compose preflight after AWS SSO login")]
    preflight: bool,
    #[arg(long, help = "Allow intentionally stubbed runners during preflight")]
    allow_stub_runners: bool,
    #[arg(
        long,
        help = "Print AWS SSO and env/preflight actions without running them"
    )]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct ModelIndexCommand {
    #[command(subcommand)]
    command: ModelIndexSubcommand,
}

#[derive(Debug, Subcommand)]
enum ModelIndexSubcommand {
    #[command(about = "Download the models.dev model catalog into the local COAT cache")]
    Refresh(ModelIndexRefreshArgs),
    #[command(about = "Show indexed model choices for a provider")]
    Show(ModelIndexShowArgs),
}

#[derive(Debug, Args)]
struct ModelIndexRefreshArgs {
    #[arg(
        long,
        default_value = MODELS_DEV_API_URL,
        help = "Model catalog URL; defaults to the public models.dev API"
    )]
    url: String,
    #[arg(
        long,
        default_value = DEFAULT_USER_MODEL_INDEX,
        help = "Where to write the downloaded model index"
    )]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct ModelIndexShowArgs {
    #[arg(
        long,
        help = "Provider id from models.dev, such as openai or amazon-bedrock"
    )]
    provider: Option<String>,
    #[arg(long, default_value_t = 20, help = "Maximum models to print")]
    limit: usize,
    #[arg(
        long,
        help = "Show embedding model choices instead of general work-model choices"
    )]
    embeddings: bool,
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
    ResumeThunk(ResumeThunkArgs),
    Notify(NotifyArgs),
}

#[derive(Debug, Args)]
struct ResumeThunkArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[command(flatten)]
    selector: GoalSelectorArgs,
    #[arg(long)]
    thunk_id: Uuid,
    #[arg(long, default_value = "operator")]
    responder: String,
    #[arg(long)]
    response_summary: String,
}

#[derive(Debug, Args)]
#[command(
    about = "Manage local, Kubernetes, Helm, and Restate deployment workflows",
    after_help = "Examples:\n  coat deploy local preflight --allow-stub-runners\n  coat deploy local up --env-file infra/compose/local-providers.env\n  coat deploy cluster render --output infra/k8s/rendered.yaml\n  coat deploy chart template --output /tmp/jattg.yaml\n  coat deploy restate register-cloud --tunnel-name jattg-personal --service-url http://coordinator:9080"
)]
struct DeployCommand {
    #[command(subcommand)]
    command: DeploySubcommand,
}

#[derive(Debug, Subcommand)]
enum DeploySubcommand {
    #[command(about = "Run and inspect the local Docker Compose stack")]
    Local(ComposeCommand),
    #[command(about = "Render, apply, and inspect Kubernetes manifests and executor Jobs")]
    Cluster(K8sCommand),
    #[command(about = "Lint, render, install, rollback, and package the jattg Helm chart")]
    Chart(HelmCommand),
    #[command(about = "Prepare Restate Cloud env, tunnel, and service registration commands")]
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
#[command(
    about = "Run and inspect the local Docker Compose stack",
    after_help = "Examples:\n  coat deploy local preflight --allow-stub-runners\n  coat deploy local up --allow-stub-runners\n  coat deploy local config --env-file infra/compose/local-providers.env\n  coat deploy local down"
)]
struct ComposeCommand {
    #[command(subcommand)]
    command: ComposeSubcommand,
}

#[derive(Debug, Subcommand)]
enum ComposeSubcommand {
    #[command(about = "Check initialization, Docker, env files, runner modes, and model setup")]
    Preflight(ComposePreflightArgs),
    #[command(about = "Run docker compose up after preflight unless --skip-preflight is set")]
    Up(ComposeUpArgs),
    #[command(about = "Print the resolved docker compose config")]
    Config(ComposeConfigArgs),
    #[command(about = "Stop the local Compose stack")]
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
#[command(
    about = "Render, apply, and inspect Kubernetes manifests and executor Jobs",
    after_help = "Examples:\n  coat deploy cluster render --output infra/k8s/rendered.yaml\n  coat deploy cluster apply --file infra/k8s/rendered.yaml --dry-run=client\n  coat deploy cluster status --timeout 120s\n  coat deploy cluster executor-job render --launch-plan examples/sandbox-launch-plan-kubernetes-job.json --output /tmp/executor-job.json"
)]
struct K8sCommand {
    #[command(subcommand)]
    command: K8sSubcommand,
}

#[derive(Debug, Subcommand)]
enum K8sSubcommand {
    #[command(about = "Render the base Kubernetes fixture manifest")]
    Render(RenderArgs),
    #[command(about = "Apply or dry-run a Kubernetes manifest with kubectl")]
    Apply(K8sApplyArgs),
    #[command(about = "Show rollout status for COAT Kubernetes deployments")]
    Status(K8sStatusArgs),
    #[command(about = "Render or apply operator fixture Jobs for ephemeral runners")]
    EphemeralJobs(EphemeralJobsCommand),
    #[command(about = "Render or apply one sandbox executor Job from a launch plan")]
    ExecutorJob(ExecutorJobCommand),
}

#[derive(Debug, Args)]
#[command(about = "Render or apply operator fixture Jobs for ephemeral runners")]
struct EphemeralJobsCommand {
    #[command(subcommand)]
    command: EphemeralJobsSubcommand,
}

#[derive(Debug, Subcommand)]
enum EphemeralJobsSubcommand {
    #[command(about = "Render ephemeral runner Job fixtures")]
    Render(EphemeralJobsRenderArgs),
    #[command(about = "Apply or dry-run ephemeral runner Job fixtures")]
    Apply(EphemeralJobsApplyArgs),
}

#[derive(Debug, Args)]
#[command(about = "Render or apply one sandbox executor Job from a launch plan")]
struct ExecutorJobCommand {
    #[command(subcommand)]
    command: ExecutorJobSubcommand,
}

#[derive(Debug, Subcommand)]
enum ExecutorJobSubcommand {
    #[command(about = "Render a bounded Kubernetes Job from sandbox-launch-plan.json")]
    Render(ExecutorJobRenderArgs),
    #[command(about = "Render, then apply or dry-run a sandbox executor Job")]
    Apply(ExecutorJobApplyArgs),
}

#[derive(Debug, Args)]
#[command(
    about = "Prepare Restate Cloud env, tunnel, and service registration commands",
    after_help = "Examples:\n  coat deploy restate cloud-env --tunnel-name jattg-personal\n  coat deploy restate tunnel-docker --environment-id env_... --signing-public-key publickeyv1_...\n  coat deploy restate register-cloud --tunnel-name jattg-personal --service-url http://coordinator:9080"
)]
struct RestateCommand {
    #[command(subcommand)]
    command: RestateSubcommand,
}

#[derive(Debug, Subcommand)]
enum RestateSubcommand {
    #[command(about = "Print a Restate Cloud env file template")]
    CloudEnv(RestateCloudEnvArgs),
    #[command(about = "Print a docker command for the Restate Cloud tunnel client")]
    TunnelDocker(RestateTunnelDockerArgs),
    #[command(about = "Register the coordinator service with Restate Cloud through the tunnel")]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LocalModelProviderPreset {
    label: &'static str,
    kind: &'static str,
    default_endpoint: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModelPreset {
    label: String,
    model: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModelParamPreset {
    label: &'static str,
    latency_class: Option<&'static str>,
    speed_tier: Option<&'static str>,
    temperature: Option<&'static str>,
    top_p: Option<&'static str>,
    max_output_tokens: Option<&'static str>,
    reasoning_effort: Option<&'static str>,
    timeout_seconds: Option<&'static str>,
    custom: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModelParamValues {
    latency_class: Option<String>,
    speed_tier: Option<String>,
    temperature: Option<String>,
    top_p: Option<String>,
    max_output_tokens: Option<String>,
    reasoning_effort: Option<String>,
    timeout_seconds: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ModelsDevIndex {
    #[serde(flatten)]
    providers: BTreeMap<String, ModelsDevProvider>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ModelsDevProvider {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    models: BTreeMap<String, ModelsDevModel>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ModelsDevModel {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    tool_call: Option<bool>,
    #[serde(default)]
    reasoning: Option<bool>,
    #[serde(default)]
    structured_output: Option<bool>,
    #[serde(default)]
    open_weights: Option<bool>,
    #[serde(default)]
    last_updated: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    modalities: Option<ModelsDevModelModalities>,
    #[serde(default)]
    limit: Option<ModelsDevModelLimit>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ModelsDevModelModalities {
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    output: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ModelsDevModelLimit {
    #[serde(default)]
    context: Option<u64>,
    #[serde(default)]
    output: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAiModelsResponse {
    #[serde(default)]
    data: Vec<OpenAiModelItem>,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAiModelItem {
    id: String,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaTagItem>,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagItem {
    name: String,
}

const CUSTOM_MODEL_ID: &str = "__coat_custom_model_id__";

const LOCAL_MODEL_PROVIDER_PRESETS: [LocalModelProviderPreset; 4] = [
    LocalModelProviderPreset {
        label: "Ollama on host.docker.internal:11434",
        kind: "ollama",
        default_endpoint: "http://host.docker.internal:11434/v1",
    },
    LocalModelProviderPreset {
        label: "vLLM OpenAI-compatible server on host.docker.internal:8000",
        kind: "vllm",
        default_endpoint: "http://host.docker.internal:8000/v1",
    },
    LocalModelProviderPreset {
        label: "llama.cpp OpenAI-compatible server on host.docker.internal:8080",
        kind: "llama_cpp",
        default_endpoint: "http://host.docker.internal:8080/v1",
    },
    LocalModelProviderPreset {
        label: "Custom OpenAI-compatible server",
        kind: "open_ai_compatible",
        default_endpoint: "http://host.docker.internal:8000/v1",
    },
];

const MODEL_PARAM_PRESETS: [ModelParamPreset; 9] = [
    ModelParamPreset {
        label: "Fast / low latency",
        latency_class: Some("fast"),
        speed_tier: None,
        temperature: Some("0.2"),
        top_p: Some("0.9"),
        max_output_tokens: Some("2048"),
        reasoning_effort: Some("low"),
        timeout_seconds: Some("60"),
        custom: false,
    },
    ModelParamPreset {
        label: "Fast completions / chat",
        latency_class: Some("fast"),
        speed_tier: None,
        temperature: Some("0.3"),
        top_p: Some("0.9"),
        max_output_tokens: Some("2048"),
        reasoning_effort: None,
        timeout_seconds: Some("45"),
        custom: false,
    },
    ModelParamPreset {
        label: "Speed tier / fastest provider lane",
        latency_class: Some("fast"),
        speed_tier: Some("speed"),
        temperature: Some("0.2"),
        top_p: Some("0.9"),
        max_output_tokens: Some("2048"),
        reasoning_effort: Some("low"),
        timeout_seconds: Some("60"),
        custom: false,
    },
    ModelParamPreset {
        label: "Balanced general work",
        latency_class: Some("balanced"),
        speed_tier: None,
        temperature: Some("0.3"),
        top_p: Some("0.95"),
        max_output_tokens: Some("4096"),
        reasoning_effort: Some("medium"),
        timeout_seconds: Some("120"),
        custom: false,
    },
    ModelParamPreset {
        label: "Deep review / reasoning",
        latency_class: Some("deep"),
        speed_tier: None,
        temperature: Some("0.2"),
        top_p: Some("0.95"),
        max_output_tokens: Some("8192"),
        reasoning_effort: Some("high"),
        timeout_seconds: Some("300"),
        custom: false,
    },
    ModelParamPreset {
        label: "XHigh reasoning / formal review",
        latency_class: Some("deep"),
        speed_tier: None,
        temperature: Some("0.1"),
        top_p: Some("0.95"),
        max_output_tokens: Some("16384"),
        reasoning_effort: Some("xhigh"),
        timeout_seconds: Some("600"),
        custom: false,
    },
    ModelParamPreset {
        label: "Deterministic JSON / tool output",
        latency_class: Some("balanced"),
        speed_tier: None,
        temperature: Some("0"),
        top_p: Some("1"),
        max_output_tokens: Some("4096"),
        reasoning_effort: Some("low"),
        timeout_seconds: Some("120"),
        custom: false,
    },
    ModelParamPreset {
        label: "Leave provider defaults unset",
        latency_class: None,
        speed_tier: None,
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        reasoning_effort: None,
        timeout_seconds: None,
        custom: false,
    },
    ModelParamPreset {
        label: "Custom runtime params",
        latency_class: None,
        speed_tier: None,
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        reasoning_effort: None,
        timeout_seconds: None,
        custom: true,
    },
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let _ = CONFIG_PROFILE_OVERRIDE.set(cli.config_profile.clone());

    let Some(command) = cli.command else {
        let mut command = Cli::command();
        command.print_long_help()?;
        println!();
        return Ok(());
    };

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
        Commands::Tool(args) => tool(args).await,
        Commands::Memory(args) => memory(args).await,
        Commands::Store(args) => store(args).await,
        Commands::Sandbox(args) => sandbox(args).await,
        Commands::Release(args) => release(args),
        Commands::Setup(args) => setup(args).await,
    }
}

async fn guide(args: GuideArgs) -> anyhow::Result<()> {
    if args.print {
        print_command_map();
        return Ok(());
    }

    let theme = ColorfulTheme::default();
    println!("COAT guided setup and human-queue dialogue");
    let choices = [
        "Show the human queue",
        "Approve or reject a request",
        "Configure COAT project/user config",
        "Run provider login/SSO setup",
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
        0 => {
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
        1 => guided_approval(&theme).await,
        2 => {
            setup(SetupCommand {
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
            })
            .await
        }
        3 => {
            setup(SetupCommand {
                command: SetupSubcommand::Login(LoginArgs {
                    codex: false,
                    claude: false,
                    claude_email: None,
                    claude_sso: false,
                    claude_console: false,
                    hf: false,
                    ollama_model: Vec::new(),
                    env_file: PathBuf::from("infra/compose/local-providers.env"),
                    preflight: false,
                    allow_stub_runners: false,
                    dry_run: false,
                }),
            })
            .await
        }
        4 => {
            setup(SetupCommand {
                command: SetupSubcommand::LocalAuth(LocalAuthArgs {
                    output: PathBuf::from("infra/compose/local-providers.env"),
                    write_env: false,
                    check: false,
                    print_commands: false,
                }),
            })
            .await
        }
        5 => {
            setup(SetupCommand {
                command: SetupSubcommand::ChatClient(ChatClientArgs::default()),
            })
            .await
        }
        6 => {
            plan(PlanCommand {
                command: PlanSubcommand::FollowUps(FollowUpsArgs {
                    dir: PathBuf::from("docs/exec-plans/active"),
                    json: false,
                    include_empty: false,
                }),
            })
            .await
        }
        7 => {
            print_command_map();
            Ok(())
        }
        _ => {
            print_command_map();
            Ok(())
        }
    }
}

fn print_command_map() {
    println!("COAT command map");
    println!("  coat guide                         guided setup and human-queue picker");
    println!("  coat plan <draft|list|show|revise|compile|follow-ups>");
    println!(
        "  coat goal <draft|lint|submit|list|progress|compute-graph|tasks|steer|vote|mechanism|thunk|branch|restart|cancel>"
    );
    println!("  coat human <approve|resume-thunk|notify>");
    println!("  coat deploy local <preflight|up|config|down>");
    println!("  coat deploy cluster <render|apply|status|ephemeral-jobs|executor-job>");
    println!("  coat deploy chart <lint|template|upgrade|rollback|package>");
    println!("  coat deploy restate <cloud-env|tunnel-docker|register-cloud>");
    println!("  coat runner <list|status|register|dispatch|capacity-plan>");
    println!("  coat tool <list|call|web-search>");
    println!("  coat memory <write|search|context|join|retract|edit|preview-edit|repair|events>");
    println!("  coat event <sources|register|ingest|emit|webhook|poll-sqs|trigger|triggers>");
    println!("  coat store <policy|goals|plans|tasks|events|artifacts|checkpoints|approvals>");
    println!("  coat setup <login|sso|model-index|config|local-auth|chat-client>");
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
        HumanSubcommand::ResumeThunk(args) => resume_thunk(args).await,
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
        RunnerSubcommand::CapacityPlan(mut args) => {
            args.registry_url = effective_runner_registry_url(&args.registry_url)?;
            let mut request: RunnerScalingRequest = read_json_file(&args.file)?;
            apply_capacity_plan_config_policy(&mut request, args.ignore_config_policy)?;
            post_json_to_url(
                &format!("{}/capacity/plan", args.registry_url.trim_end_matches('/')),
                &request,
                None,
                None,
            )
            .await
        }
    }
}

async fn tool(args: ToolCommand) -> anyhow::Result<()> {
    match args.command {
        ToolSubcommand::List(mut args) => {
            args.tool_registry_url = effective_tool_registry_url(&args.tool_registry_url)?;
            let token = tool_registry_token(args.token);
            get_url(
                &format!(
                    "{}/tools/list",
                    args.tool_registry_url.trim_end_matches('/')
                ),
                token.as_deref(),
            )
            .await
        }
        ToolSubcommand::Call(mut args) => {
            args.registry.tool_registry_url =
                effective_tool_registry_url(&args.registry.tool_registry_url)?;
            let arguments: serde_json::Value = read_json_file(&args.file)?;
            let token = tool_registry_token(args.registry.token);
            call_tool_registry(
                &args.registry.tool_registry_url,
                token.as_deref(),
                &args.name,
                arguments,
            )
            .await
        }
        ToolSubcommand::WebSearch(mut args) => {
            args.registry.tool_registry_url =
                effective_tool_registry_url(&args.registry.tool_registry_url)?;
            let arguments: serde_json::Value = read_json_file(&args.file)?;
            let _: WebSearchRequest = serde_json::from_value(arguments.clone())
                .with_context(|| format!("validate {}", args.file.display()))?;
            let token = tool_registry_token(args.registry.token);
            call_tool_registry(
                &args.registry.tool_registry_url,
                token.as_deref(),
                "coat_web_search",
                arguments,
            )
            .await
        }
    }
}

async fn call_tool_registry(
    tool_registry_url: &str,
    token: Option<&str>,
    name: &str,
    arguments: serde_json::Value,
) -> anyhow::Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": format!("coat-tool-{name}"),
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments
        }
    });
    let response = post_json_value_to_url(
        &format!("{}/mcp", tool_registry_url.trim_end_matches('/')),
        &request,
        token,
        None,
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn tool_registry_token(token: Option<String>) -> Option<String> {
    token.or_else(|| env::var("MCP_TOOL_TOKEN").ok())
}

fn apply_capacity_plan_config_policy(
    request: &mut RunnerScalingRequest,
    ignore_config_policy: bool,
) -> anyhow::Result<()> {
    let config = load_resolved_coat_config()?.config;
    apply_capacity_plan_policy_from_config(request, ignore_config_policy, &config);
    Ok(())
}

fn apply_capacity_plan_policy_from_config(
    request: &mut RunnerScalingRequest,
    ignore_config_policy: bool,
    config: &CoatConfig,
) {
    if ignore_config_policy || request.policy != coat_domain::CapacityScalingPolicy::default() {
        return;
    }
    let pool_key = request
        .demands
        .first()
        .map(|demand| demand.pool_key.as_str())
        .unwrap_or("default");
    if let Some(policy) = config.runner_capacity.policy_for(pool_key) {
        request.policy = policy;
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
            | GoalSubcommand::ComputeGraph(_)
            | GoalSubcommand::Tasks(_)
            | GoalSubcommand::Steer(_)
            | GoalSubcommand::SteerStandard(_)
            | GoalSubcommand::Vote(_)
            | GoalSubcommand::Mechanism(_)
            | GoalSubcommand::Thunk(_)
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
        | Commands::Tool(_)
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
    merge_model_routing_config(&mut base.model_routing, overlay.model_routing);
    merge_tool_routing_config(&mut base.tool_routing, overlay.tool_routing);
    merge_local_deploy_config(&mut base.local_deploy, overlay.local_deploy);
    merge_runner_capacity_config(&mut base.runner_capacity, overlay.runner_capacity);
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
    merge_model_routing_config(&mut config.model_routing, profile.model_routing);
    merge_local_deploy_config(&mut config.local_deploy, profile.local_deploy);
    merge_runner_capacity_config(&mut config.runner_capacity, profile.runner_capacity);
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
    merge_model_routing_config(&mut base.model_routing, overlay.model_routing);
    merge_local_deploy_config(&mut base.local_deploy, overlay.local_deploy);
    merge_runner_capacity_config(&mut base.runner_capacity, overlay.runner_capacity);
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
    replace_if_some(&mut base.tool_registry_url, overlay.tool_registry_url);
    replace_if_some(&mut base.notifier_url, overlay.notifier_url);
    replace_if_some(&mut base.memory_gateway_url, overlay.memory_gateway_url);
    replace_if_some(&mut base.goal_store_url, overlay.goal_store_url);
    replace_if_some(&mut base.event_gateway_url, overlay.event_gateway_url);
    replace_if_some(&mut base.control_mcp_url, overlay.control_mcp_url);
}

fn merge_model_routing_config(base: &mut CoatModelRoutingConfig, overlay: CoatModelRoutingConfig) {
    replace_if_some(&mut base.mode, overlay.mode);
    merge_llm_gateway_config(&mut base.gateway, overlay.gateway);
    append_unique(
        &mut base.direct_provider_secret_refs,
        overlay.direct_provider_secret_refs,
    );
}

fn merge_llm_gateway_config(base: &mut CoatLlmGatewayConfig, overlay: CoatLlmGatewayConfig) {
    replace_if_some(&mut base.provider, overlay.provider);
    replace_if_some(&mut base.base_url, overlay.base_url);
    replace_if_some(&mut base.chat_completions_url, overlay.chat_completions_url);
    replace_if_some(&mut base.auth_env, overlay.auth_env);
    append_unique(&mut base.secret_refs, overlay.secret_refs);
    replace_if_some(&mut base.default_model, overlay.default_model);
    replace_if_some(&mut base.work_model, overlay.work_model);
    replace_if_some(&mut base.research_model, overlay.research_model);
    replace_if_some(&mut base.chat_model, overlay.chat_model);
    replace_if_some(&mut base.embedding_model, overlay.embedding_model);
}

fn merge_tool_routing_config(
    base: &mut coat_domain::CoatToolRoutingConfig,
    overlay: coat_domain::CoatToolRoutingConfig,
) {
    merge_web_search_routing_config(&mut base.web_search, overlay.web_search);
}

fn merge_web_search_routing_config(
    base: &mut coat_domain::CoatWebSearchRoutingConfig,
    overlay: coat_domain::CoatWebSearchRoutingConfig,
) {
    replace_if_some(&mut base.enabled, overlay.enabled);
    replace_if_some(&mut base.mode, overlay.mode);
    replace_if_some(&mut base.provider, overlay.provider);
    replace_if_some(&mut base.base_url, overlay.base_url);
    replace_if_some(&mut base.auth_env, overlay.auth_env);
    append_unique(&mut base.secret_refs, overlay.secret_refs);
    replace_if_some(&mut base.default_limit, overlay.default_limit);
    replace_if_some(&mut base.max_limit, overlay.max_limit);
    replace_if_some(
        &mut base.route_via_runner_registry,
        overlay.route_via_runner_registry,
    );
    append_unique(
        &mut base.required_capabilities,
        overlay.required_capabilities,
    );
    append_unique(
        &mut base.required_model_features,
        overlay.required_model_features,
    );
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

fn merge_runner_capacity_config(
    base: &mut coat_domain::CoatRunnerCapacityConfig,
    overlay: coat_domain::CoatRunnerCapacityConfig,
) {
    replace_if_some(&mut base.default_policy, overlay.default_policy);
    for (lane, policy) in overlay.lane_policies {
        base.lane_policies.insert(lane, policy);
    }
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

fn append_unique<T: PartialEq>(values: &mut Vec<T>, additions: Vec<T>) {
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
        "COAT_RUNNER_REGISTRY_URL": env::var("COAT_RUNNER_REGISTRY_URL").ok(),
        "COAT_TOOL_REGISTRY_URL": env::var("COAT_TOOL_REGISTRY_URL").ok(),
        "COAT_TOOL_REGISTRY_TOKEN": env::var("COAT_TOOL_REGISTRY_TOKEN").ok().map(|_| "<set>"),
        "MCP_TOOL_TOKEN": env::var("MCP_TOOL_TOKEN").ok().map(|_| "<set>"),
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

fn effective_tool_registry_url(value: &str) -> anyhow::Result<String> {
    if value != DEFAULT_TOOL_REGISTRY_URL {
        return Ok(value.to_string());
    }
    let config = load_resolved_coat_config()?.config;
    Ok(endpoint_from_config(
        value,
        DEFAULT_TOOL_REGISTRY_URL,
        config.service_endpoints.tool_registry_url,
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

fn effective_goal_vote_args(mut args: GoalVoteArgs) -> anyhow::Result<GoalVoteArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_goal_mechanism_args(mut args: GoalMechanismArgs) -> anyhow::Result<GoalMechanismArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn effective_goal_thunk_args(mut args: GoalThunkArgs) -> anyhow::Result<GoalThunkArgs> {
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

fn effective_resume_thunk_args(mut args: ResumeThunkArgs) -> anyhow::Result<ResumeThunkArgs> {
    args.restate_ingress = effective_restate_ingress(&args.restate_ingress)?;
    args.selector = effective_goal_selector_args(args.selector)?;
    Ok(args)
}

fn goal_priority_vote_request(
    args: &GoalVoteArgs,
    goal_id: Uuid,
) -> anyhow::Result<GoalPriorityVoteRequest> {
    Ok(GoalPriorityVoteRequest {
        goal_id,
        voter: args.voter.clone(),
        source: parse_goal_vote_source(&args.source)?,
        direction: parse_goal_vote_direction(&args.direction)?,
        weight: args.weight,
        reason: args.reason.clone(),
        suggested_role: args
            .suggested_role
            .as_deref()
            .map(parse_goal_hierarchy_role)
            .transpose()?,
    })
}

fn parse_goal_vote_direction(value: &str) -> anyhow::Result<GoalVoteDirection> {
    match value {
        "up" => Ok(GoalVoteDirection::Up),
        "down" => Ok(GoalVoteDirection::Down),
        other => bail!("unsupported vote direction: {other}"),
    }
}

fn parse_goal_vote_source(value: &str) -> anyhow::Result<GoalVoteSource> {
    match value {
        "human" => Ok(GoalVoteSource::Human),
        "coordinator" => Ok(GoalVoteSource::Coordinator),
        "agent" => Ok(GoalVoteSource::Agent),
        "system" => Ok(GoalVoteSource::System),
        other => bail!("unsupported vote source: {other}"),
    }
}

fn parse_goal_hierarchy_role(value: &str) -> anyhow::Result<GoalHierarchyRole> {
    match value {
        "overarching_goal" => Ok(GoalHierarchyRole::OverarchingGoal),
        "peer_goal" => Ok(GoalHierarchyRole::PeerGoal),
        "subgoal" => Ok(GoalHierarchyRole::Subgoal),
        other => bail!("unsupported suggested role: {other}"),
    }
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
        GoalSubcommand::ComputeGraph(args) => {
            let args = effective_goal_id_args(args)?;
            let goal_id = resolve_goal_id(&args.selector).await?;
            restate_post_without_body(&args.restate_ingress, goal_id, "compute_graph").await
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
        GoalSubcommand::Vote(args) => {
            let args = effective_goal_vote_args(args)?;
            let goal_id = resolve_goal_id(&args.selector).await?;
            let request = goal_priority_vote_request(&args, goal_id)?;
            restate_post_json(&args.restate_ingress, goal_id, "vote", &request).await
        }
        GoalSubcommand::Mechanism(command) => match command.command {
            GoalMechanismSubcommand::Start(args) => {
                let args = effective_goal_mechanism_args(args)?;
                let goal_id = resolve_goal_id(&args.selector).await?;
                let request: MechanismRoundRequest =
                    read_goal_scoped_json_file(&args.file, goal_id, "MechanismRoundRequest")?;
                restate_post_json(&args.restate_ingress, goal_id, "mechanism_start", &request).await
            }
            GoalMechanismSubcommand::Ballot(args) => {
                let args = effective_goal_mechanism_args(args)?;
                let goal_id = resolve_goal_id(&args.selector).await?;
                let request: MechanismBallotRequest =
                    read_goal_scoped_json_file(&args.file, goal_id, "MechanismBallotRequest")?;
                restate_post_json(&args.restate_ingress, goal_id, "mechanism_ballot", &request)
                    .await
            }
        },
        GoalSubcommand::Thunk(command) => match command.command {
            GoalThunkSubcommand::Create(args) => {
                let args = effective_goal_thunk_args(args)?;
                let goal_id = resolve_goal_id(&args.selector).await?;
                let request: DelayedComputeThunkRequest =
                    read_goal_scoped_json_file(&args.file, goal_id, "DelayedComputeThunkRequest")?;
                restate_post_json(&args.restate_ingress, goal_id, "create_thunk", &request).await
            }
        },
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

async fn resume_thunk(args: ResumeThunkArgs) -> anyhow::Result<()> {
    let args = effective_resume_thunk_args(args)?;
    let goal_id = resolve_goal_id(&args.selector).await?;
    let request = DelayedComputeThunkResumeRequest {
        thunk_id: args.thunk_id,
        responder: args.responder,
        response_summary: args.response_summary,
        artifact_refs: Vec::new(),
    };
    restate_post_json(&args.restate_ingress, goal_id, "resume_thunk", &request).await
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

async fn setup(args: SetupCommand) -> anyhow::Result<()> {
    match args.command {
        SetupSubcommand::Login(args) => setup_login(args),
        SetupSubcommand::Sso(args) => setup_sso(args),
        SetupSubcommand::ModelIndex(args) => model_index_setup(args).await,
        SetupSubcommand::Config(args) => config_setup(args),
        SetupSubcommand::LocalAuth(args) => local_auth_setup(args).await,
        SetupSubcommand::ChatClient(args) => chat_client_setup(args),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalAuthAction {
    CodexLogin,
    ClaudeLogin {
        email: Option<String>,
        sso: bool,
        console: bool,
    },
    AwsSso {
        profile: String,
    },
    HuggingFaceLogin,
    OllamaPull {
        model: String,
    },
}

fn setup_login(args: LoginArgs) -> anyhow::Result<()> {
    let mut actions = login_actions_from_args(&args);
    if actions.is_empty() {
        actions = interactive_login_actions()?;
    }
    run_local_auth_actions(&actions, args.dry_run)?;
    if args.preflight {
        if args.dry_run {
            print_local_auth_preflight_command(&args.env_file, args.allow_stub_runners);
        } else {
            run_local_auth_preflight(&args.env_file, args.allow_stub_runners)?;
        }
    }
    Ok(())
}

fn setup_sso(args: SsoArgs) -> anyhow::Result<()> {
    let profile = resolve_aws_sso_profile(args.profile)?;
    let action = LocalAuthAction::AwsSso {
        profile: profile.clone(),
    };
    run_local_auth_actions(&[action], args.dry_run)?;

    if args.write_env || args.bedrock_live {
        let mut env_text = read_or_template_env_file(&args.env_file)?;
        env_text = replace_env_line(env_text, "AWS_PROFILE", &profile);
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_AUTH_MODE", "aws_profile");
        if args.bedrock_live {
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_RUNNER_MODE", "live");
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_KIND", "bedrock");
            if !env_present(&parse_env_file_content(&env_text), "MODEL_PROVIDER_MODEL") {
                if let Some(model) = load_model_index()?
                    .as_ref()
                    .and_then(|index| first_models_dev_model(index, "amazon-bedrock"))
                {
                    env_text = replace_env_line(env_text, "MODEL_PROVIDER_MODEL", &model);
                } else {
                    println!(
                        "no Bedrock model index found; leaving MODEL_PROVIDER_MODEL for the operator to set"
                    );
                }
            }
        }
        if args.dry_run {
            println!("would write {}", args.env_file.display());
        } else {
            write_local_provider_env(&args.env_file, &env_text)?;
        }
    }

    if args.preflight {
        if args.dry_run {
            print_local_auth_preflight_command(&args.env_file, args.allow_stub_runners);
        } else {
            run_local_auth_preflight(&args.env_file, args.allow_stub_runners)?;
        }
    }
    Ok(())
}

fn login_actions_from_args(args: &LoginArgs) -> Vec<LocalAuthAction> {
    let mut actions = Vec::new();
    if args.codex {
        actions.push(LocalAuthAction::CodexLogin);
    }
    if args.claude || args.claude_email.is_some() || args.claude_sso || args.claude_console {
        actions.push(LocalAuthAction::ClaudeLogin {
            email: args
                .claude_email
                .as_ref()
                .map(|email| email.trim().to_string()),
            sso: args.claude_sso,
            console: args.claude_console,
        });
    }
    if args.hf {
        actions.push(LocalAuthAction::HuggingFaceLogin);
    }
    actions.extend(
        args.ollama_model
            .iter()
            .filter(|model| !model.trim().is_empty())
            .map(|model| LocalAuthAction::OllamaPull {
                model: model.trim().to_string(),
            }),
    );
    actions
}

fn interactive_login_actions() -> anyhow::Result<Vec<LocalAuthAction>> {
    let theme = ColorfulTheme::default();
    let choices = [
        "Codex device/browser login",
        "Claude Code device/browser login",
        "Hugging Face CLI login",
        "Ollama pull a model",
    ];
    let selections = MultiSelect::with_theme(&theme)
        .with_prompt("Select login/setup actions to run")
        .items(&choices)
        .defaults(&[true, true, false, false])
        .interact()?;
    let mut actions = Vec::new();
    if selections.contains(&0) {
        actions.push(LocalAuthAction::CodexLogin);
    }
    if selections.contains(&1) {
        actions.push(prompt_claude_login_action(&theme)?);
    }
    if selections.contains(&2) {
        actions.push(LocalAuthAction::HuggingFaceLogin);
    }
    if selections.contains(&3) {
        let model: String = Input::with_theme(&theme)
            .with_prompt("Ollama model to pull")
            .interact_text()?;
        actions.push(LocalAuthAction::OllamaPull { model });
    }
    Ok(actions)
}

fn prompt_claude_login_action(theme: &ColorfulTheme) -> anyhow::Result<LocalAuthAction> {
    let auth_modes = [
        "Claude.ai subscription / default OAuth",
        "Force organization SSO",
        "Claude Console API billing",
    ];
    let mode = Select::with_theme(theme)
        .with_prompt("Claude Code login mode")
        .items(&auth_modes)
        .default(0)
        .interact()?;
    let email = Input::<String>::with_theme(theme)
        .with_prompt("Optional Claude email prefill")
        .allow_empty(true)
        .interact_text()?;
    Ok(LocalAuthAction::ClaudeLogin {
        email: if email.trim().is_empty() {
            None
        } else {
            Some(email.trim().to_string())
        },
        sso: mode == 1,
        console: mode == 2,
    })
}

fn resolve_aws_sso_profile(profile: Option<String>) -> anyhow::Result<String> {
    if let Some(profile) = profile.filter(|profile| !profile.trim().is_empty()) {
        return Ok(profile.trim().to_string());
    }
    if let Ok(profile) = env::var("AWS_PROFILE") {
        if !profile.trim().is_empty() {
            return Ok(profile.trim().to_string());
        }
    }
    let theme = ColorfulTheme::default();
    Input::with_theme(&theme)
        .with_prompt("AWS SSO profile")
        .default("default".to_string())
        .interact_text()
        .map_err(Into::into)
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

async fn local_auth_setup(args: LocalAuthArgs) -> anyhow::Result<()> {
    let default_action = !args.write_env && !args.check && !args.print_commands;
    if default_action {
        return interactive_local_auth_setup(args).await;
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

async fn interactive_local_auth_setup(args: LocalAuthArgs) -> anyhow::Result<()> {
    let theme = ColorfulTheme::default();
    println!("COAT local provider setup");
    let mut login_actions = Vec::new();
    let existing_env_file = args.output.exists();
    let mut env_text = read_or_template_env_file(&args.output)?;
    if existing_env_file {
        println!(
            "using existing local provider env {}",
            args.output.display()
        );
    } else {
        println!(
            "starting from local provider template; {} does not exist yet",
            args.output.display()
        );
    }
    let initial_values = parse_env_file_content(&env_text);
    if Confirm::with_theme(&theme)
        .with_prompt("Check installed provider CLIs and relevant environment variables?")
        .default(true)
        .interact()?
    {
        print_local_auth_checks();
    }
    let model_index = ensure_setup_model_index().await?;

    let profiles = [
        "Codex runners",
        "Claude Code and staff-engineer runners",
        "Shared LLM gateway for work, research, chat, and embeddings",
        "OpenAI hosted model, research, and embedding lanes",
        "AWS Bedrock",
        "Host-local Ollama",
        "Host-local vLLM/OpenAI-compatible server",
        "Hugging Face endpoint",
        "Control gateway Chat tab",
        "Memory stores and embedding models",
        "Routed web/reference search",
    ];
    let profile_defaults = local_auth_profile_defaults(existing_env_file, &initial_values);
    let selections = MultiSelect::with_theme(&theme)
        .with_prompt("Select provider surfaces to prepare")
        .items(&profiles)
        .defaults(&profile_defaults)
        .interact()?;

    let populate_from_env = Confirm::with_theme(&theme)
        .with_prompt("Copy currently exported secret env values into the local env file?")
        .default(false)
        .interact()?;
    if populate_from_env {
        env_text = populate_secret_env_values(env_text);
    }
    let mut primary_model_provider_configured = false;
    let mut research_model_provider_configured = false;
    let mut memory_gateway_configured = false;

    if selections.contains(&0) {
        let current_values = parse_env_file_content(&env_text);
        let auth_choices = [
            "API key from env file or shell",
            "Runner-local Codex device/browser login",
            "Codex App Server URL",
        ];
        let auth_choice = Select::with_theme(&theme)
            .with_prompt("Codex runner auth mode")
            .items(&auth_choices)
            .default(auth_choice_default(
                env_value(&current_values, "CODEX_AUTH_MODE").as_deref(),
                &["env_api_key", "runner_local_device", "app_server"],
                1,
            ))
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
                login_actions.push(LocalAuthAction::CodexLogin);
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
                    .default(env_or_default(
                        &current_values,
                        "CODEX_APP_SERVER_URL",
                        "http://host.docker.internal:1455",
                    ))
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
        let current_values = parse_env_file_content(&env_text);
        let auth_choices = [
            "API key/token from env file or shell",
            "Runner-local Claude Code device/browser login",
            "Brokered OAuth/device lease resolved by runner",
        ];
        let auth_choice = Select::with_theme(&theme)
            .with_prompt("Claude Code and staff-engineer auth mode")
            .items(&auth_choices)
            .default(auth_choice_default(
                env_value(&current_values, "CLAUDE_CODE_AUTH_MODE")
                    .or_else(|| env_value(&current_values, "STAFF_ENGINEER_AUTH_MODE"))
                    .as_deref(),
                &["env_api_key", "runner_local_device", "oauth_device_broker"],
                1,
            ))
            .interact()?;
        let (auth_mode, device_label) = match auth_choice {
            0 => ("env_api_key", false),
            1 => ("runner_local_device", true),
            _ => ("oauth_device_broker", false),
        };
        if auth_mode == "runner_local_device" {
            login_actions.push(LocalAuthAction::ClaudeLogin {
                email: None,
                sso: false,
                console: false,
            });
        }
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
        let current_values = parse_env_file_content(&env_text);
        let gateway_choices = [
            ("bifrost", "Bifrost OpenAI-compatible gateway"),
            ("litellm", "LiteLLM OpenAI-compatible gateway"),
            ("openrouter", "OpenRouter gateway"),
            ("docker_model_gateway", "Docker Model Gateway"),
            ("custom", "Custom OpenAI-compatible gateway"),
        ];
        let gateway_labels = gateway_choices
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>();
        let configured_gateway = env_value(&current_values, "COAT_LLM_GATEWAY_PROVIDER")
            .unwrap_or_else(|| "bifrost".to_string());
        let gateway_default = gateway_choices
            .iter()
            .position(|(key, _)| configured_gateway.eq_ignore_ascii_case(key))
            .unwrap_or(0);
        let gateway_choice = Select::with_theme(&theme)
            .with_prompt("Shared LLM gateway kind")
            .items(&gateway_labels)
            .default(gateway_default)
            .interact()?;
        let gateway_provider = gateway_choices[gateway_choice].0;
        let gateway_url: String = Input::with_theme(&theme)
            .with_prompt("Gateway OpenAI-compatible base URL")
            .default(env_or_default(
                &current_values,
                "COAT_LLM_GATEWAY_URL",
                if gateway_provider == "bifrost" {
                    "http://host.docker.internal:8080/openai"
                } else {
                    "http://host.docker.internal:4000/v1"
                },
            ))
            .interact_text()?;
        let auth_modes = [
            "Gateway bearer/API key from env file or shell",
            "No gateway bearer key",
            "Brokered/external gateway auth",
        ];
        let auth_choice = Select::with_theme(&theme)
            .with_prompt("Shared LLM gateway auth mode")
            .items(&auth_modes)
            .default(auth_choice_default(
                env_value(&current_values, "COAT_LLM_GATEWAY_AUTH_MODE").as_deref(),
                &["api_key_or_none", "none", "external_broker"],
                0,
            ))
            .interact()?;
        let gateway_auth_mode = match auth_choice {
            1 => "none",
            2 => "external_broker",
            _ => "api_key_or_none",
        };
        let live_models = discover_openai_compatible_models(&gateway_url).await;
        if live_models.is_empty() {
            println!(
                "no live models discovered from {}; choose or type provider-prefixed gateway model ids",
                gateway_url
            );
        }
        let gateway_presets = model_presets_with_configured(
            live_model_presets(live_models, "Custom gateway model id"),
            env_value(&current_values, "COAT_LLM_GATEWAY_DEFAULT_MODEL").as_deref(),
            "Configured gateway model",
        );
        let default_model = select_model_preset(
            &theme,
            "Default gateway model",
            &gateway_presets,
            env_value(&current_values, "COAT_LLM_GATEWAY_DEFAULT_MODEL")
                .as_deref()
                .unwrap_or_default(),
            "Custom gateway model id",
        )?;
        let work_model = if Confirm::with_theme(&theme)
            .with_prompt("Use default gateway model for work lane?")
            .default(
                !env_present(&current_values, "COAT_LLM_GATEWAY_WORK_MODEL")
                    || env_value_is_value(
                        &current_values,
                        "COAT_LLM_GATEWAY_WORK_MODEL",
                        &default_model,
                    ),
            )
            .interact()?
        {
            default_model.clone()
        } else {
            select_model_preset(
                &theme,
                "Gateway work model",
                &gateway_presets,
                env_value(&current_values, "COAT_LLM_GATEWAY_WORK_MODEL")
                    .as_deref()
                    .unwrap_or(&default_model),
                "Custom gateway work model id",
            )?
        };
        let research_model = if Confirm::with_theme(&theme)
            .with_prompt("Use default gateway model for research/review lane?")
            .default(
                !env_present(&current_values, "COAT_LLM_GATEWAY_RESEARCH_MODEL")
                    || env_value_is_value(
                        &current_values,
                        "COAT_LLM_GATEWAY_RESEARCH_MODEL",
                        &default_model,
                    ),
            )
            .interact()?
        {
            default_model.clone()
        } else {
            select_model_preset(
                &theme,
                "Gateway research/review model",
                &gateway_presets,
                env_value(&current_values, "COAT_LLM_GATEWAY_RESEARCH_MODEL")
                    .as_deref()
                    .unwrap_or(&default_model),
                "Custom gateway research model id",
            )?
        };
        let chat_model_value = if Confirm::with_theme(&theme)
            .with_prompt("Use default gateway model for the Control Chat tab?")
            .default(
                !env_present(&current_values, "COAT_LLM_GATEWAY_CHAT_MODEL")
                    || env_value_is_value(
                        &current_values,
                        "COAT_LLM_GATEWAY_CHAT_MODEL",
                        &default_model,
                    ),
            )
            .interact()?
        {
            default_model.clone()
        } else {
            select_model_preset(
                &theme,
                "Gateway chat model",
                &gateway_presets,
                env_value(&current_values, "COAT_LLM_GATEWAY_CHAT_MODEL")
                    .as_deref()
                    .unwrap_or(&default_model),
                "Custom gateway chat model id",
            )?
        };
        let gateway_params = select_model_param_values_with_env(
            &theme,
            "Gateway runtime params",
            "balanced",
            &current_values,
            "MODEL_PROVIDER",
        )?;
        env_text = replace_env_line(env_text, "COAT_LLM_GATEWAY_PROVIDER", gateway_provider);
        env_text = replace_env_line(env_text, "COAT_LLM_GATEWAY_URL", &gateway_url);
        env_text = replace_env_line(env_text, "COAT_LLM_GATEWAY_AUTH_MODE", gateway_auth_mode);
        env_text = replace_env_line(env_text, "COAT_LLM_GATEWAY_DEFAULT_MODEL", &default_model);
        env_text = replace_env_line(env_text, "COAT_LLM_GATEWAY_WORK_MODEL", &work_model);
        env_text = replace_env_line(env_text, "COAT_LLM_GATEWAY_RESEARCH_MODEL", &research_model);
        env_text = replace_env_line(env_text, "COAT_LLM_GATEWAY_CHAT_MODEL", &chat_model_value);
        env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_BACKEND", "configured");
        env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_PROVIDER", "llm_gateway");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_KIND", "open_ai_compatible");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_AUTH_MODE", gateway_auth_mode);
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_MODEL", "");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_ENDPOINT", "");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_RESEARCH_RUNNER_MODE", "live");
        env_text = replace_env_line(
            env_text,
            "MODEL_PROVIDER_RESEARCH_KIND",
            "open_ai_compatible",
        );
        env_text = replace_env_line(
            env_text,
            "MODEL_PROVIDER_RESEARCH_AUTH_MODE",
            gateway_auth_mode,
        );
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_RESEARCH_MODEL", "");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_RESEARCH_ENDPOINT", "");
        env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_COMPLETIONS_URL", "");
        env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_MODEL", "");
        env_text = apply_model_param_values(env_text, "MODEL_PROVIDER", &gateway_params);
        env_text = apply_model_param_values(env_text, "MODEL_PROVIDER_RESEARCH", &gateway_params);
        env_text = apply_model_param_values(env_text, "COAT_CONTROL_CHAT", &gateway_params);
        primary_model_provider_configured = true;
        research_model_provider_configured = true;
    }

    if selections.contains(&3) {
        let current_values = parse_env_file_content(&env_text);
        let openai_presets = model_presets_with_configured(
            model_index
                .as_ref()
                .map(|index| {
                    models_dev_provider_presets(
                        index,
                        "openai",
                        "Custom OpenAI model id",
                        MAX_INDEXED_MODEL_CHOICES,
                    )
                })
                .unwrap_or_else(|| custom_only_model_presets("Custom OpenAI model id")),
            env_value(&current_values, "MODEL_PROVIDER_MODEL")
                .filter(|_| env_value_is(&current_values, "MODEL_PROVIDER_KIND", "open_ai"))
                .as_deref(),
            "Configured OpenAI model",
        );
        let openai_model = select_model_preset(
            &theme,
            "OpenAI hosted work model type",
            &openai_presets,
            env_value(&current_values, "MODEL_PROVIDER_MODEL")
                .filter(|_| env_value_is(&current_values, "MODEL_PROVIDER_KIND", "open_ai"))
                .as_deref()
                .unwrap_or_default(),
            "Custom OpenAI model id",
        )?;
        let openai_params = select_model_param_values_with_env(
            &theme,
            "OpenAI hosted runtime params",
            "balanced",
            &current_values,
            "MODEL_PROVIDER",
        )?;
        let auth_modes = [
            "API key from env file or shell",
            "Brokered/external auth resolved by runner",
        ];
        let auth_choice = Select::with_theme(&theme)
            .with_prompt("OpenAI hosted model auth mode")
            .items(&auth_modes)
            .default(auth_choice_default(
                env_value(&current_values, "MODEL_PROVIDER_AUTH_MODE").as_deref(),
                &["api_key_or_none", "external_broker"],
                0,
            ))
            .interact()?;
        let openai_auth_mode = if auth_choice == 0 {
            "api_key_or_none"
        } else {
            "external_broker"
        };

        env_text = replace_env_line(env_text, "MODEL_PROVIDER_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_KIND", "open_ai");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_AUTH_MODE", openai_auth_mode);
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_MODEL", &openai_model);
        env_text = replace_env_line(
            env_text,
            "MODEL_PROVIDER_ENDPOINT",
            "https://api.openai.com/v1",
        );
        env_text = apply_model_param_values(env_text, "MODEL_PROVIDER", &openai_params);
        primary_model_provider_configured = true;

        let enable_research = Confirm::with_theme(&theme)
            .with_prompt("Use OpenAI hosted model provider for the research lane too?")
            .default(if existing_env_file {
                env_value_is(&current_values, "MODEL_PROVIDER_RESEARCH_KIND", "open_ai")
                    || env_value_is(
                        &current_values,
                        "MODEL_PROVIDER_RESEARCH_RUNNER_MODE",
                        "live",
                    )
            } else {
                true
            })
            .interact()?;
        if enable_research {
            let research_model = if Confirm::with_theme(&theme)
                .with_prompt("Use the same OpenAI model for research?")
                .default(
                    !env_present(&current_values, "MODEL_PROVIDER_RESEARCH_MODEL")
                        || env_value_is_value(
                            &current_values,
                            "MODEL_PROVIDER_RESEARCH_MODEL",
                            &openai_model,
                        ),
                )
                .interact()?
            {
                openai_model.clone()
            } else {
                select_model_preset(
                    &theme,
                    "OpenAI hosted research model type",
                    &openai_presets,
                    env_value(&current_values, "MODEL_PROVIDER_RESEARCH_MODEL")
                        .as_deref()
                        .unwrap_or(&openai_model),
                    "Custom OpenAI research model id",
                )?
            };
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_RESEARCH_RUNNER_MODE", "live");
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_RESEARCH_KIND", "open_ai");
            env_text = replace_env_line(
                env_text,
                "MODEL_PROVIDER_RESEARCH_AUTH_MODE",
                openai_auth_mode,
            );
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_RESEARCH_MODEL", &research_model);
            env_text = replace_env_line(
                env_text,
                "MODEL_PROVIDER_RESEARCH_ENDPOINT",
                "https://api.openai.com/v1",
            );
            env_text =
                apply_model_param_values(env_text, "MODEL_PROVIDER_RESEARCH", &openai_params);
            research_model_provider_configured = true;
        }

        if !selections.contains(&8)
            && Confirm::with_theme(&theme)
                .with_prompt("Use OpenAI hosted model as the Control Chat default?")
                .default(
                    !existing_env_file
                        || env_value_is(&current_values, "COAT_CONTROL_CHAT_PROVIDER", "openai")
                        || !env_present(&current_values, "COAT_CONTROL_CHAT_MODEL"),
                )
                .interact()?
        {
            env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_BACKEND", "configured");
            env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_PROVIDER", "openai");
            env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_COMPLETIONS_URL", "");
            env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_MODEL", &openai_model);
            env_text = apply_model_param_values(env_text, "COAT_CONTROL_CHAT", &openai_params);
        }

        if Confirm::with_theme(&theme)
            .with_prompt("Use OpenAI embeddings for memory gateway?")
            .default(if existing_env_file {
                env_value(&current_values, "MEMORY_GATEWAY_EMBEDDING_URL")
                    .is_some_and(|url| url.contains("api.openai.com"))
            } else {
                env_var_present("OPENAI_API_KEY")
            })
            .interact()?
        {
            env_text = configure_memory_stores_and_embeddings(
                &theme,
                env_text,
                model_index.as_ref(),
                Some((
                    "OpenAI hosted embeddings",
                    "open_ai",
                    "https://api.openai.com/v1",
                )),
                &mut login_actions,
            )
            .await?;
            memory_gateway_configured = true;
        }
    }

    if selections.contains(&4) {
        let current_values = parse_env_file_content(&env_text);
        let bedrock_presets = model_presets_with_configured(
            model_index
                .as_ref()
                .map(|index| {
                    models_dev_provider_presets(
                        index,
                        "amazon-bedrock",
                        "Custom Bedrock model id",
                        MAX_INDEXED_MODEL_CHOICES,
                    )
                })
                .unwrap_or_else(|| custom_only_model_presets("Custom Bedrock model id")),
            env_value(&current_values, "MODEL_PROVIDER_MODEL")
                .filter(|_| env_value_is(&current_values, "MODEL_PROVIDER_KIND", "bedrock"))
                .as_deref(),
            "Configured Bedrock model",
        );
        let bedrock_model = select_model_preset(
            &theme,
            "Bedrock model type",
            &bedrock_presets,
            env_value(&current_values, "MODEL_PROVIDER_MODEL")
                .filter(|_| env_value_is(&current_values, "MODEL_PROVIDER_KIND", "bedrock"))
                .as_deref()
                .unwrap_or_default(),
            "Custom Bedrock model id",
        )?;
        let bedrock_params = select_model_param_values_with_env(
            &theme,
            "Bedrock runtime params",
            "balanced",
            &current_values,
            "MODEL_PROVIDER",
        )?;
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_KIND", "bedrock");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_AUTH_MODE", "aws_profile");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_MODEL", &bedrock_model);
        env_text = apply_model_param_values(env_text, "MODEL_PROVIDER", &bedrock_params);
        primary_model_provider_configured = true;
        if Confirm::with_theme(&theme)
            .with_prompt("Run AWS SSO for this Bedrock profile during setup?")
            .default(!existing_env_file || env_present(&current_values, "AWS_PROFILE"))
            .interact()?
        {
            let profile = resolve_aws_sso_profile(env_value(&current_values, "AWS_PROFILE"))?;
            env_text = replace_env_line(env_text, "AWS_PROFILE", &profile);
            login_actions.push(LocalAuthAction::AwsSso { profile });
        }
    }

    if selections.contains(&5) || selections.contains(&6) {
        let current_values = parse_env_file_content(&env_text);
        let labels = local_model_provider_preset_labels();
        let default_index = env_value(&current_values, "LOCAL_MODEL_PROVIDER_KIND")
            .or_else(|| env_value(&current_values, "MODEL_PROVIDER_KIND"))
            .and_then(|kind| local_model_provider_index_for_kind(&kind))
            .unwrap_or_else(|| default_local_model_provider_index(selections.contains(&6)));
        let local_choice = Select::with_theme(&theme)
            .with_prompt("Local model provider kind")
            .items(&labels)
            .default(default_index)
            .interact()?;
        let preset = local_model_provider_preset(local_choice);
        let local_endpoint: String = Input::with_theme(&theme)
            .with_prompt("Local OpenAI-compatible endpoint from Compose containers")
            .default(env_or_default(
                &current_values,
                "LOCAL_MODEL_PROVIDER_ENDPOINT",
                preset.default_endpoint,
            ))
            .interact_text()?;
        let live_models = discover_local_provider_models(preset.kind, &local_endpoint).await;
        if live_models.is_empty() {
            println!(
                "no live models discovered from {}; choose a custom served model id",
                local_endpoint
            );
        }
        let local_presets = model_presets_with_configured(
            live_model_presets(live_models, "Custom local model id"),
            env_value(&current_values, "LOCAL_MODEL_PROVIDER_MODEL").as_deref(),
            "Configured local model",
        );
        let local_model = select_model_preset(
            &theme,
            "Local model type",
            &local_presets,
            env_value(&current_values, "LOCAL_MODEL_PROVIDER_MODEL")
                .as_deref()
                .unwrap_or_default(),
            "Custom local model id",
        )?;
        let local_params = select_model_param_values_with_env(
            &theme,
            "Local model runtime params",
            "fast",
            &current_values,
            "LOCAL_MODEL_PROVIDER",
        )?;
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_LOCAL_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_KIND", preset.kind);
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_AUTH_MODE", "none");
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_MODEL", &local_model);
        env_text = replace_env_line(env_text, "LOCAL_MODEL_PROVIDER_ENDPOINT", &local_endpoint);
        env_text = apply_model_param_values(env_text, "LOCAL_MODEL_PROVIDER", &local_params);
        if preset.kind == "ollama" {
            login_actions.push(LocalAuthAction::OllamaPull {
                model: local_model.clone(),
            });
        }
        if !primary_model_provider_configured
            && Confirm::with_theme(&theme)
                .with_prompt("Use this local model for the primary model-provider lane too?")
                .default(
                    !existing_env_file
                        || env_value_is(&current_values, "MODEL_PROVIDER_KIND", preset.kind),
                )
                .interact()?
        {
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_RUNNER_MODE", "live");
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_KIND", preset.kind);
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_AUTH_MODE", "none");
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_MODEL", &local_model);
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_ENDPOINT", &local_endpoint);
            env_text = apply_model_param_values(env_text, "MODEL_PROVIDER", &local_params);
        }
        if !research_model_provider_configured
            && Confirm::with_theme(&theme)
                .with_prompt("Use this local model for the research model-provider lane too?")
                .default(
                    !existing_env_file
                        || env_value_is(
                            &current_values,
                            "MODEL_PROVIDER_RESEARCH_KIND",
                            preset.kind,
                        ),
                )
                .interact()?
        {
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_RESEARCH_RUNNER_MODE", "live");
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_RESEARCH_KIND", preset.kind);
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_RESEARCH_AUTH_MODE", "none");
            env_text = replace_env_line(env_text, "MODEL_PROVIDER_RESEARCH_MODEL", &local_model);
            env_text = replace_env_line(
                env_text,
                "MODEL_PROVIDER_RESEARCH_ENDPOINT",
                &local_endpoint,
            );
            env_text = apply_model_param_values(env_text, "MODEL_PROVIDER_RESEARCH", &local_params);
        }
        if Confirm::with_theme(&theme)
            .with_prompt("Configure memory stores and embeddings from this local endpoint too?")
            .default(memory_surface_configured(&current_values))
            .interact()?
        {
            env_text = configure_memory_stores_and_embeddings(
                &theme,
                env_text,
                model_index.as_ref(),
                Some((preset.label, preset.kind, &local_endpoint)),
                &mut login_actions,
            )
            .await?;
            memory_gateway_configured = true;
        }
    }

    if selections.contains(&7) {
        let current_values = parse_env_file_content(&env_text);
        let hf_endpoint: String = Input::with_theme(&theme)
            .with_prompt("Hugging Face OpenAI-compatible endpoint")
            .default(env_or_default(
                &current_values,
                "MODEL_PROVIDER_ENDPOINT",
                "https://api.endpoints.huggingface.cloud/v1",
            ))
            .interact_text()?;
        let live_models = discover_openai_compatible_models(&hf_endpoint).await;
        if live_models.is_empty() {
            println!(
                "no live models discovered from {}; choose a custom Hugging Face endpoint model id",
                hf_endpoint
            );
        }
        let hf_presets = model_presets_with_configured(
            live_model_presets(live_models, "Custom Hugging Face endpoint model id"),
            env_value(&current_values, "MODEL_PROVIDER_MODEL")
                .filter(|_| env_value_is(&current_values, "MODEL_PROVIDER_KIND", "hugging_face"))
                .as_deref(),
            "Configured Hugging Face endpoint model",
        );
        let hf_model = select_model_preset(
            &theme,
            "Hugging Face endpoint model type",
            &hf_presets,
            env_value(&current_values, "MODEL_PROVIDER_MODEL")
                .filter(|_| env_value_is(&current_values, "MODEL_PROVIDER_KIND", "hugging_face"))
                .as_deref()
                .unwrap_or_default(),
            "Custom Hugging Face endpoint model id",
        )?;
        let hf_params = select_model_param_values_with_env(
            &theme,
            "Hugging Face runtime params",
            "balanced",
            &current_values,
            "MODEL_PROVIDER",
        )?;
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_RUNNER_MODE", "live");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_KIND", "hugging_face");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_AUTH_MODE", "provider_token");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_MODEL", &hf_model);
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_ENDPOINT", &hf_endpoint);
        env_text = apply_model_param_values(env_text, "MODEL_PROVIDER", &hf_params);
        if Confirm::with_theme(&theme)
            .with_prompt(
                "Configure memory stores and embeddings from this Hugging Face endpoint too?",
            )
            .default(memory_surface_configured(&current_values))
            .interact()?
        {
            env_text = configure_memory_stores_and_embeddings(
                &theme,
                env_text,
                model_index.as_ref(),
                Some(("Hugging Face endpoint", "hugging_face", &hf_endpoint)),
                &mut login_actions,
            )
            .await?;
            memory_gateway_configured = true;
        }
        login_actions.push(LocalAuthAction::HuggingFaceLogin);
    }

    if selections.contains(&8) {
        let current_values = parse_env_file_content(&env_text);
        let chat_choices = [
            "Use local model endpoint",
            "Use OpenAI hosted chat completions",
            "Leave chat stubbed",
        ];
        let choice = Select::with_theme(&theme)
            .with_prompt("Control gateway Chat tab backend")
            .items(&chat_choices)
            .default(control_chat_default_choice(&current_values))
            .interact()?;
        match choice {
            0 => {
                let url: String = Input::with_theme(&theme)
                    .with_prompt("Chat completions URL")
                    .default(env_or_default(
                        &current_values,
                        "COAT_CONTROL_CHAT_COMPLETIONS_URL",
                        "http://host.docker.internal:8000/v1/chat/completions",
                    ))
                    .interact_text()?;
                let live_models = discover_openai_compatible_models(&url).await;
                if live_models.is_empty() {
                    println!(
                        "no live chat models discovered from {}; choose a custom chat model id",
                        url
                    );
                }
                let chat_model_presets = model_presets_with_configured(
                    live_model_presets(live_models, "Custom local chat model id"),
                    env_value(&current_values, "COAT_CONTROL_CHAT_MODEL").as_deref(),
                    "Configured chat model",
                );
                let model = select_model_preset(
                    &theme,
                    "Local chat model type",
                    &chat_model_presets,
                    env_value(&current_values, "COAT_CONTROL_CHAT_MODEL")
                        .as_deref()
                        .unwrap_or_default(),
                    "Custom local chat model id",
                )?;
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_BACKEND", "configured");
                env_text =
                    replace_env_line(env_text, "COAT_CONTROL_CHAT_PROVIDER", "openai_compatible");
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_COMPLETIONS_URL", &url);
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_MODEL", &model);
                let chat_params = select_model_param_values_with_env(
                    &theme,
                    "Control Chat runtime params",
                    "fast",
                    &current_values,
                    "COAT_CONTROL_CHAT",
                )?;
                env_text = apply_model_param_values(env_text, "COAT_CONTROL_CHAT", &chat_params);
            }
            1 => {
                let openai_presets = model_presets_with_configured(
                    model_index
                        .as_ref()
                        .map(|index| {
                            models_dev_provider_presets(
                                index,
                                "openai",
                                "Custom OpenAI chat model id",
                                MAX_INDEXED_MODEL_CHOICES,
                            )
                        })
                        .unwrap_or_else(|| {
                            custom_only_model_presets("Custom OpenAI chat model id")
                        }),
                    env_value(&current_values, "COAT_CONTROL_CHAT_MODEL").as_deref(),
                    "Configured OpenAI chat model",
                );
                let model = select_model_preset(
                    &theme,
                    "OpenAI chat model type",
                    &openai_presets,
                    env_value(&current_values, "COAT_CONTROL_CHAT_MODEL")
                        .as_deref()
                        .unwrap_or_default(),
                    "Custom OpenAI chat model id",
                )?;
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_BACKEND", "configured");
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_PROVIDER", "openai");
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_COMPLETIONS_URL", "");
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_MODEL", &model);
                let chat_params = select_model_param_values_with_env(
                    &theme,
                    "OpenAI Control Chat runtime params",
                    "balanced",
                    &current_values,
                    "COAT_CONTROL_CHAT",
                )?;
                env_text = apply_model_param_values(env_text, "COAT_CONTROL_CHAT", &chat_params);
            }
            _ => {
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_BACKEND", "stub");
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_PROVIDER", "");
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_COMPLETIONS_URL", "");
                env_text = replace_env_line(env_text, "COAT_CONTROL_CHAT_MODEL", "");
            }
        }
    }

    if selections.contains(&9) && !memory_gateway_configured {
        env_text = configure_memory_stores_and_embeddings(
            &theme,
            env_text,
            model_index.as_ref(),
            None,
            &mut login_actions,
        )
        .await?;
    }

    if selections.contains(&10) {
        env_text = configure_web_search_routing(&theme, env_text)?;
    }

    let write_env = Confirm::with_theme(&theme)
        .with_prompt("Write local provider env file?")
        .default(true)
        .interact()?;
    let mut written_env = None;
    if write_env {
        let output: String = Input::with_theme(&theme)
            .with_prompt("Env file path")
            .default(args.output.display().to_string())
            .interact_text()?;
        let output = PathBuf::from(output);
        write_local_provider_env(&output, &env_text)?;
        written_env = Some(output);
    }

    if !login_actions.is_empty()
        && Confirm::with_theme(&theme)
            .with_prompt("Run selected login/setup actions now?")
            .default(true)
            .interact()?
    {
        run_local_auth_actions(&login_actions, false)?;
    }

    if let Some(output) = written_env {
        if Confirm::with_theme(&theme)
            .with_prompt("Run local Compose preflight with this env file now?")
            .default(true)
            .interact()?
        {
            run_local_auth_preflight(&output, true)?;
        }
    } else {
        print_local_auth_commands();
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmbeddingProviderChoice {
    Disabled,
    Suggested,
    OpenAiHosted,
    CustomOpenAiCompatible,
}

fn configure_web_search_routing(
    theme: &ColorfulTheme,
    mut env_text: String,
) -> anyhow::Result<String> {
    let current_values = parse_env_file_content(&env_text);
    let route_choices = [
        "Coordinator-owned durable research task",
        "Runner registry routed search",
        "Disabled",
    ];
    let existing_route = env_value(&current_values, "COAT_WEB_SEARCH_ROUTE").unwrap_or_else(|| {
        if env_truthy_value(env_value(&current_values, "COAT_WEB_SEARCH_ENABLED").as_deref()) {
            "coordinator_task".to_string()
        } else {
            "disabled".to_string()
        }
    });
    let route_default = match existing_route.as_str() {
        "runner_registry" => 1,
        "disabled" => 2,
        _ => 0,
    };
    let route_choice = Select::with_theme(theme)
        .with_prompt("Web/reference search route")
        .items(&route_choices)
        .default(route_default)
        .interact()?;

    if route_choice == 2 {
        env_text = replace_env_line(env_text, "COAT_WEB_SEARCH_ENABLED", "false");
        env_text = replace_env_line(env_text, "COAT_WEB_SEARCH_ROUTE", "coordinator_task");
        env_text = replace_env_line(env_text, "CODEX_NATIVE_WEB_SEARCH", "false");
        env_text = replace_env_line(env_text, "CLAUDE_CODE_NATIVE_WEB_SEARCH", "false");
        env_text = replace_env_line(env_text, "MODEL_PROVIDER_WEB_SEARCH_ENABLED", "false");
        env_text = replace_env_line(
            env_text,
            "MODEL_PROVIDER_RESEARCH_WEB_SEARCH_ENABLED",
            "false",
        );
        return Ok(env_text);
    }

    let route = if route_choice == 1 {
        "runner_registry"
    } else {
        "coordinator_task"
    };
    env_text = replace_env_line(env_text, "COAT_WEB_SEARCH_ENABLED", "true");
    env_text = replace_env_line(env_text, "COAT_WEB_SEARCH_ROUTE", route);
    env_text = replace_env_line(env_text, "COAT_WEB_SEARCH_PROVIDER", "agent_native");
    env_text = replace_env_line(
        env_text,
        "COAT_WEB_SEARCH_DEFAULT_LIMIT",
        &env_or_default(&current_values, "COAT_WEB_SEARCH_DEFAULT_LIMIT", "10"),
    );
    env_text = replace_env_line(
        env_text,
        "COAT_WEB_SEARCH_MAX_LIMIT",
        &env_or_default(&current_values, "COAT_WEB_SEARCH_MAX_LIMIT", "25"),
    );

    let runner_choices = [
        "Codex runner native web/search tools",
        "Claude Code runner native web/search tools",
        "Model-provider research runner MCP/search gateway",
    ];
    let runner_defaults = [
        env_truthy_value(env_value(&current_values, "CODEX_NATIVE_WEB_SEARCH").as_deref()),
        env_truthy_value(env_value(&current_values, "CLAUDE_CODE_NATIVE_WEB_SEARCH").as_deref()),
        env_truthy_value(
            env_value(
                &current_values,
                "MODEL_PROVIDER_RESEARCH_WEB_SEARCH_ENABLED",
            )
            .as_deref(),
        ),
    ];
    let selected_runners = MultiSelect::with_theme(theme)
        .with_prompt("Which runner lanes may advertise web_search?")
        .items(&runner_choices)
        .defaults(&runner_defaults)
        .interact()?;
    env_text = replace_env_line(
        env_text,
        "CODEX_NATIVE_WEB_SEARCH",
        bool_env(selected_runners.contains(&0)),
    );
    env_text = replace_env_line(
        env_text,
        "CLAUDE_CODE_NATIVE_WEB_SEARCH",
        bool_env(selected_runners.contains(&1)),
    );
    env_text = replace_env_line(
        env_text,
        "MODEL_PROVIDER_RESEARCH_WEB_SEARCH_ENABLED",
        bool_env(selected_runners.contains(&2)),
    );
    Ok(env_text)
}

async fn configure_memory_stores_and_embeddings(
    theme: &ColorfulTheme,
    mut env_text: String,
    model_index: Option<&ModelsDevIndex>,
    suggested_provider: Option<(&str, &str, &str)>,
    login_actions: &mut Vec<LocalAuthAction>,
) -> anyhow::Result<String> {
    println!("COAT memory store and embedding setup");
    let current_values = parse_env_file_content(&env_text);
    let store_choices = [
        "Local JSONL journal only",
        "Qdrant vector store",
        "Graphiti/Zep MCP graph store",
        "Qdrant + Graphiti/Zep",
    ];
    let default_store = match (
        env_present(&current_values, "MEMORY_GATEWAY_QDRANT_URL"),
        env_present(&current_values, "MEMORY_GATEWAY_GRAPHITI_MCP_URL"),
    ) {
        (true, true) => 3,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 0,
    };
    let store_choice = Select::with_theme(theme)
        .with_prompt("Memory store adapters")
        .items(&store_choices)
        .default(default_store)
        .interact()?;
    let use_qdrant = matches!(store_choice, 1 | 3);
    let use_graphiti = matches!(store_choice, 2 | 3);

    if use_qdrant {
        let qdrant_url: String = Input::with_theme(theme)
            .with_prompt("Qdrant URL from Compose containers")
            .default(
                env_value(&current_values, "MEMORY_GATEWAY_QDRANT_URL")
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "http://qdrant:6333".to_string()),
            )
            .interact_text()?;
        let qdrant_collection: String = Input::with_theme(theme)
            .with_prompt("Qdrant collection")
            .default(
                env_value(&current_values, "MEMORY_GATEWAY_QDRANT_COLLECTION")
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "jattg_memory".to_string()),
            )
            .interact_text()?;
        env_text = replace_env_line(env_text, "MEMORY_GATEWAY_QDRANT_URL", &qdrant_url);
        env_text = replace_env_line(
            env_text,
            "MEMORY_GATEWAY_QDRANT_COLLECTION",
            &qdrant_collection,
        );
    } else {
        env_text = replace_env_line(env_text, "MEMORY_GATEWAY_QDRANT_URL", "");
    }

    if use_graphiti {
        let graphiti_url: String = Input::with_theme(theme)
            .with_prompt("Graphiti/Zep MCP URL from Compose containers")
            .default(
                env_value(&current_values, "MEMORY_GATEWAY_GRAPHITI_MCP_URL")
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "http://graphiti-mcp:8000/mcp/".to_string()),
            )
            .interact_text()?;
        let graphiti_group: String = Input::with_theme(theme)
            .with_prompt("Graphiti/Zep group or namespace")
            .default(
                env_value(&current_values, "MEMORY_GATEWAY_GRAPHITI_GROUP_ID")
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "jattg".to_string()),
            )
            .interact_text()?;
        env_text = replace_env_line(env_text, "MEMORY_GATEWAY_GRAPHITI_MCP_URL", &graphiti_url);
        env_text = replace_env_line(
            env_text,
            "MEMORY_GATEWAY_GRAPHITI_GROUP_ID",
            &graphiti_group,
        );
    } else {
        env_text = replace_env_line(env_text, "MEMORY_GATEWAY_GRAPHITI_MCP_URL", "");
    }

    let mut embedding_options = vec![(
        "Disable embeddings".to_string(),
        EmbeddingProviderChoice::Disabled,
    )];
    if let Some((label, _, _)) = suggested_provider {
        embedding_options.push((
            format!("Use {label} as the embedding endpoint"),
            EmbeddingProviderChoice::Suggested,
        ));
    }
    embedding_options.push((
        "OpenAI hosted embeddings".to_string(),
        EmbeddingProviderChoice::OpenAiHosted,
    ));
    embedding_options.push((
        "Custom OpenAI-compatible embedding endpoint".to_string(),
        EmbeddingProviderChoice::CustomOpenAiCompatible,
    ));
    let embedding_labels = embedding_options
        .iter()
        .map(|(label, _)| label.as_str())
        .collect::<Vec<_>>();
    let default_embedding_index = if suggested_provider.is_some() {
        1
    } else if env_var_present("OPENAI_API_KEY") {
        embedding_options
            .iter()
            .position(|(_, choice)| *choice == EmbeddingProviderChoice::OpenAiHosted)
            .unwrap_or(0)
    } else {
        0
    };
    let embedding_choice = Select::with_theme(theme)
        .with_prompt("Embedding model provider")
        .items(&embedding_labels)
        .default(default_embedding_index)
        .interact()?;
    let embedding_choice = embedding_options
        .get(embedding_choice)
        .map(|(_, choice)| *choice)
        .unwrap_or(EmbeddingProviderChoice::Disabled);

    if embedding_choice == EmbeddingProviderChoice::Disabled {
        env_text = replace_env_line(env_text, "MEMORY_GATEWAY_EMBEDDING_URL", "");
        env_text = replace_env_line(env_text, "MEMORY_GATEWAY_EMBEDDING_MODEL", "");
        env_text = replace_env_line(env_text, "MEMORY_GATEWAY_EMBEDDING_DIMENSIONS", "");
        env_text = replace_env_line(
            env_text,
            "MEMORY_GATEWAY_EMBEDDING_SEND_DIMENSIONS",
            "false",
        );
        return Ok(env_text);
    }

    let (provider_label, provider_kind, base_endpoint): (String, String, String) =
        match embedding_choice {
            EmbeddingProviderChoice::Suggested => {
                let (label, kind, endpoint) = suggested_provider
                    .expect("suggested embedding option is only shown when provider exists");
                (label.to_string(), kind.to_string(), endpoint.to_string())
            }
            EmbeddingProviderChoice::OpenAiHosted => (
                "OpenAI hosted embeddings".to_string(),
                "open_ai".to_string(),
                "https://api.openai.com/v1".to_string(),
            ),
            EmbeddingProviderChoice::CustomOpenAiCompatible => {
                let provider_kind_choices = [
                    "Generic OpenAI-compatible endpoint",
                    "Ollama",
                    "vLLM",
                    "llama.cpp",
                    "Hugging Face endpoint",
                ];
                let provider_kind_values = [
                    "open_ai_compatible",
                    "ollama",
                    "vllm",
                    "llama_cpp",
                    "hugging_face",
                ];
                let provider_kind_choice = Select::with_theme(theme)
                    .with_prompt("Embedding endpoint kind")
                    .items(&provider_kind_choices)
                    .default(0)
                    .interact()?;
                let provider_kind = provider_kind_values
                    .get(provider_kind_choice)
                    .copied()
                    .unwrap_or("open_ai_compatible");
                let endpoint: String = Input::with_theme(theme)
                    .with_prompt("Embedding OpenAI-compatible base endpoint")
                    .default("http://host.docker.internal:8080/v1".to_string())
                    .interact_text()?;
                (
                    "Custom OpenAI-compatible embeddings".to_string(),
                    provider_kind.to_string(),
                    endpoint,
                )
            }
            EmbeddingProviderChoice::Disabled => unreachable!("handled above"),
        };

    let embedding_url = openai_embeddings_url(&base_endpoint);
    let embedding_presets = if provider_kind == "open_ai" {
        model_index
            .map(|index| {
                models_dev_embedding_presets(
                    index,
                    "openai",
                    "Custom OpenAI embedding model id",
                    MAX_INDEXED_MODEL_CHOICES,
                )
            })
            .unwrap_or_else(|| custom_only_model_presets("Custom OpenAI embedding model id"))
    } else {
        let live_models = discover_local_provider_models(&provider_kind, &base_endpoint).await;
        if live_models.is_empty() {
            println!(
                "no live embedding models discovered from {}; choose a custom served model id",
                base_endpoint
            );
        }
        live_model_presets(live_models, "Custom served embedding model id")
    };
    let embedding_presets = model_presets_with_configured(
        embedding_presets,
        env_value(&current_values, "MEMORY_GATEWAY_EMBEDDING_MODEL").as_deref(),
        "Configured embedding model",
    );
    let embedding_model = select_model_preset(
        theme,
        &format!("{provider_label} model"),
        &embedding_presets,
        "",
        "Custom embedding model id",
    )?;
    let default_dimensions = if provider_kind == "open_ai" {
        model_index
            .and_then(|index| models_dev_embedding_dimensions(index, "openai", &embedding_model))
            .map(|dimensions| dimensions.to_string())
            .unwrap_or_default()
    } else {
        env_value(&current_values, "MEMORY_GATEWAY_EMBEDDING_DIMENSIONS").unwrap_or_default()
    };
    let embedding_dimensions: String = Input::with_theme(theme)
        .with_prompt("Embedding dimensions (blank keeps provider default)")
        .allow_empty(true)
        .default(default_dimensions)
        .interact_text()?;
    let send_dimensions = if embedding_dimensions.trim().is_empty() {
        false
    } else {
        Confirm::with_theme(theme)
            .with_prompt("Send dimensions in embedding requests?")
            .default(false)
            .interact()?
    };

    env_text = replace_env_line(env_text, "MEMORY_GATEWAY_EMBEDDING_URL", &embedding_url);
    env_text = replace_env_line(env_text, "MEMORY_GATEWAY_EMBEDDING_MODEL", &embedding_model);
    env_text = replace_env_line(
        env_text,
        "MEMORY_GATEWAY_EMBEDDING_DIMENSIONS",
        embedding_dimensions.trim(),
    );
    env_text = replace_env_line(
        env_text,
        "MEMORY_GATEWAY_EMBEDDING_SEND_DIMENSIONS",
        if send_dimensions { "true" } else { "false" },
    );
    if provider_kind == "ollama" {
        login_actions.push(LocalAuthAction::OllamaPull {
            model: embedding_model,
        });
    }
    Ok(env_text)
}

fn write_local_provider_env(path: &Path, env_text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, env_text)?;
    println!("wrote {}", path.display());
    println!(
        "start with: coat deploy local up --env-file {}",
        path.display()
    );
    println!("`coat deploy local up` runs preflight before Compose starts");
    Ok(())
}

fn read_or_template_env_file(path: &Path) -> anyhow::Result<String> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(local_provider_env_template().to_string())
        }
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn local_auth_profile_defaults(
    existing_env_file: bool,
    values: &BTreeMap<String, String>,
) -> Vec<bool> {
    if !existing_env_file {
        return vec![
            true,
            true,
            false,
            env_var_present("OPENAI_API_KEY"),
            false,
            true,
            false,
            false,
            true,
            false,
            false,
        ];
    }
    vec![
        env_value_is(values, "CODEX_RUNNER_MODE", "live")
            || env_value_is(values, "CODEX_REVIEW_RUNNER_MODE", "live"),
        env_value_is(values, "CLAUDE_CODE_RUNNER_MODE", "live")
            || env_value_is(values, "STAFF_ENGINEER_RUNNER_MODE", "live"),
        env_present(values, "COAT_LLM_GATEWAY_URL")
            || env_present(values, "COAT_LLM_GATEWAY_DEFAULT_MODEL")
            || env_present(values, "COAT_LLM_GATEWAY_WORK_MODEL"),
        env_value_is(values, "MODEL_PROVIDER_KIND", "open_ai")
            || env_value_is(values, "MODEL_PROVIDER_RESEARCH_KIND", "open_ai"),
        env_value_is(values, "MODEL_PROVIDER_KIND", "bedrock"),
        env_value_is(values, "LOCAL_MODEL_PROVIDER_KIND", "ollama")
            || env_value_is(values, "MODEL_PROVIDER_KIND", "ollama")
            || env_value_is(values, "MODEL_PROVIDER_RESEARCH_KIND", "ollama"),
        ["vllm", "llama_cpp", "open_ai_compatible"]
            .iter()
            .any(|kind| {
                env_value_is(values, "LOCAL_MODEL_PROVIDER_KIND", kind)
                    || env_value_is(values, "MODEL_PROVIDER_KIND", kind)
                    || env_value_is(values, "MODEL_PROVIDER_RESEARCH_KIND", kind)
            }),
        env_value_is(values, "MODEL_PROVIDER_KIND", "hugging_face"),
        env_present(values, "COAT_CONTROL_CHAT_COMPLETIONS_URL")
            || env_present(values, "COAT_CONTROL_CHAT_MODEL"),
        memory_surface_configured(values),
        web_search_surface_configured(values),
    ]
}

fn web_search_surface_configured(values: &BTreeMap<String, String>) -> bool {
    env_truthy_value(env_value(values, "COAT_WEB_SEARCH_ENABLED").as_deref())
        || env_present(values, "COAT_WEB_SEARCH_URL")
        || env_truthy_value(env_value(values, "CODEX_NATIVE_WEB_SEARCH").as_deref())
        || env_truthy_value(env_value(values, "CLAUDE_CODE_NATIVE_WEB_SEARCH").as_deref())
        || env_truthy_value(env_value(values, "MODEL_PROVIDER_WEB_SEARCH_ENABLED").as_deref())
        || env_truthy_value(
            env_value(values, "MODEL_PROVIDER_RESEARCH_WEB_SEARCH_ENABLED").as_deref(),
        )
}

fn memory_surface_configured(values: &BTreeMap<String, String>) -> bool {
    [
        "MEMORY_GATEWAY_GRAPHITI_MCP_URL",
        "MEMORY_GATEWAY_QDRANT_URL",
        "MEMORY_GATEWAY_EMBEDDING_URL",
        "MEMORY_GATEWAY_EMBEDDING_MODEL",
    ]
    .iter()
    .any(|key| env_present(values, key))
}

fn auth_choice_default(existing: Option<&str>, choices: &[&str], fallback_index: usize) -> usize {
    existing
        .and_then(|existing| {
            choices
                .iter()
                .position(|choice| existing.eq_ignore_ascii_case(choice))
        })
        .unwrap_or(fallback_index)
}

fn local_model_provider_index_for_kind(kind: &str) -> Option<usize> {
    LOCAL_MODEL_PROVIDER_PRESETS
        .iter()
        .position(|preset| preset.kind.eq_ignore_ascii_case(kind))
}

fn control_chat_default_choice(values: &BTreeMap<String, String>) -> usize {
    if env_value_is(values, "COAT_CONTROL_CHAT_BACKEND", "stub") {
        return 2;
    }
    if env_value_is(values, "COAT_CONTROL_CHAT_PROVIDER", "openai") {
        return 1;
    }
    let url = env_value(values, "COAT_CONTROL_CHAT_COMPLETIONS_URL").unwrap_or_default();
    if !url.trim().is_empty() {
        if url.contains("api.openai.com") { 1 } else { 0 }
    } else if env_present(values, "COAT_CONTROL_CHAT_MODEL")
        && env_present(values, "OPENAI_API_KEY")
    {
        1
    } else {
        2
    }
}

async fn model_index_setup(args: ModelIndexCommand) -> anyhow::Result<()> {
    match args.command {
        ModelIndexSubcommand::Refresh(args) => refresh_model_index(args).await,
        ModelIndexSubcommand::Show(args) => show_model_index(args),
    }
}

async fn refresh_model_index(args: ModelIndexRefreshArgs) -> anyhow::Result<()> {
    let output = expand_home_path(&args.output)?;
    let _ = refresh_model_index_file(&args.url, &output, Duration::from_secs(30)).await?;
    println!("wrote model index {}", output.display());
    println!("source: {}", args.url);
    Ok(())
}

async fn refresh_model_index_file(
    url: &str,
    output: &Path,
    timeout: Duration,
) -> anyhow::Result<ModelsDevIndex> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .context("build model index HTTP client")?;
    let body = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("download model index from {url}"))?
        .error_for_status()
        .with_context(|| format!("download model index from {url}"))?
        .text()
        .await
        .with_context(|| format!("read model index response from {url}"))?;
    let index: ModelsDevIndex =
        serde_json::from_str(&body).context("model index response is not models.dev JSON")?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&output, format!("{body}\n"))
        .with_context(|| format!("write {}", output.display()))?;
    Ok(index)
}

async fn ensure_setup_model_index() -> anyhow::Result<Option<ModelsDevIndex>> {
    let existing = match load_model_index_with_source() {
        Ok(existing) => existing,
        Err(error) => {
            println!("warn: could not load cached model index ({error:#}); attempting refresh");
            None
        }
    };
    if env::var("COAT_MODEL_INDEX")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
    {
        if let Some((path, index)) = existing {
            println!("using explicit model index {}", path.display());
            return Ok(Some(index));
        }
        println!(
            "warn: COAT_MODEL_INDEX is set but no model index could be loaded; hosted model choices will use custom prompts"
        );
        return Ok(None);
    }

    match load_fresh_model_index_with_source(Duration::from_secs(
        MODEL_INDEX_REFRESH_DEBOUNCE_SECONDS,
    )) {
        Ok(Some((path, index))) => {
            println!(
                "using model index {} (fresh within {} minutes)",
                path.display(),
                MODEL_INDEX_REFRESH_DEBOUNCE_SECONDS / 60
            );
            return Ok(Some(index));
        }
        Ok(None) => {}
        Err(error) => {
            println!("warn: could not load fresh model index ({error:#}); attempting refresh");
        }
    }

    let output = expand_home_path(Path::new(DEFAULT_USER_MODEL_INDEX))?;
    match refresh_model_index_file(MODELS_DEV_API_URL, &output, Duration::from_secs(10)).await {
        Ok(index) => {
            println!("refreshed model index {}", output.display());
            Ok(Some(index))
        }
        Err(error) => {
            if let Some((path, index)) = existing {
                println!(
                    "warn: could not refresh model index ({error:#}); using cached {}",
                    path.display()
                );
                Ok(Some(index))
            } else {
                println!(
                    "warn: could not refresh model index ({error:#}); hosted model choices will use custom prompts"
                );
                Ok(None)
            }
        }
    }
}

fn show_model_index(args: ModelIndexShowArgs) -> anyhow::Result<()> {
    let (path, index) = load_model_index_with_source()?.with_context(|| {
        format!(
            "no model index found; run `coat setup model-index refresh` or set COAT_MODEL_INDEX"
        )
    })?;
    println!("model index: {}", path.display());
    match args.provider {
        Some(provider_id) => {
            let presets = if args.embeddings {
                models_dev_embedding_presets(
                    &index,
                    &provider_id,
                    "Custom embedding model id",
                    args.limit.max(1),
                )
            } else {
                models_dev_provider_presets(
                    &index,
                    &provider_id,
                    "Custom model id",
                    args.limit.max(1),
                )
            };
            if presets.len() == 1 && presets[0].model == CUSTOM_MODEL_ID {
                bail!(
                    "provider {provider_id:?} was not found in {} or has no matching models for this filter",
                    path.display()
                );
            }
            for preset in presets
                .into_iter()
                .filter(|preset| preset.model != CUSTOM_MODEL_ID)
            {
                println!("{}  {}", preset.model, preset.label);
            }
        }
        None => {
            for (provider_id, provider) in index.providers.iter().take(args.limit.max(1)) {
                let name = provider.name.as_deref().unwrap_or(provider_id);
                println!(
                    "{}  {}  models={}",
                    provider_id,
                    name,
                    provider.models.len()
                );
            }
        }
    }
    Ok(())
}

fn load_model_index() -> anyhow::Result<Option<ModelsDevIndex>> {
    Ok(load_model_index_with_source()?.map(|(_, index)| index))
}

fn load_model_index_with_source() -> anyhow::Result<Option<(PathBuf, ModelsDevIndex)>> {
    load_model_index_from_paths(model_index_candidate_paths()?)
}

fn load_model_index_from_paths(
    paths: Vec<PathBuf>,
) -> anyhow::Result<Option<(PathBuf, ModelsDevIndex)>> {
    for path in paths {
        if !path.exists() {
            continue;
        }
        let content =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let index = serde_json::from_str::<ModelsDevIndex>(&content)
            .with_context(|| format!("parse models.dev model index {}", path.display()))?;
        return Ok(Some((path, index)));
    }
    Ok(None)
}

fn load_fresh_model_index_with_source(
    max_age: Duration,
) -> anyhow::Result<Option<(PathBuf, ModelsDevIndex)>> {
    load_fresh_model_index_from_paths(model_index_candidate_paths()?, max_age)
}

fn load_fresh_model_index_from_paths(
    paths: Vec<PathBuf>,
    max_age: Duration,
) -> anyhow::Result<Option<(PathBuf, ModelsDevIndex)>> {
    for path in paths {
        if !model_index_cache_is_fresh(&path, max_age) {
            continue;
        }
        let content =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let index = serde_json::from_str::<ModelsDevIndex>(&content)
            .with_context(|| format!("parse models.dev model index {}", path.display()))?;
        return Ok(Some((path, index)));
    }
    Ok(None)
}

fn model_index_cache_is_fresh(path: &Path, max_age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age <= max_age)
}

fn model_index_candidate_paths() -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if let Ok(path) = env::var("COAT_MODEL_INDEX") {
        if !path.trim().is_empty() {
            paths.push(expand_home_path(Path::new(path.trim()))?);
        }
    }
    paths.push(PathBuf::from(DEFAULT_PROJECT_MODEL_INDEX));
    paths.push(expand_home_path(Path::new(DEFAULT_USER_MODEL_INDEX))?);
    Ok(paths)
}

fn models_dev_provider_presets(
    index: &ModelsDevIndex,
    provider_id: &str,
    custom_label: &str,
    limit: usize,
) -> Vec<ModelPreset> {
    models_dev_provider_presets_filtered(
        index,
        provider_id,
        custom_label,
        limit,
        models_dev_model_selectable,
    )
}

fn models_dev_embedding_presets(
    index: &ModelsDevIndex,
    provider_id: &str,
    custom_label: &str,
    limit: usize,
) -> Vec<ModelPreset> {
    models_dev_provider_presets_filtered(
        index,
        provider_id,
        custom_label,
        limit,
        models_dev_embedding_model_selectable,
    )
}

fn models_dev_provider_presets_filtered(
    index: &ModelsDevIndex,
    provider_id: &str,
    custom_label: &str,
    limit: usize,
    selectable: fn(&str, &ModelsDevModel) -> bool,
) -> Vec<ModelPreset> {
    let mut presets = models_dev_provider(index, provider_id)
        .map(|(provider_key, provider)| {
            let provider_key = provider_key.as_str();
            let mut choices = provider
                .models
                .iter()
                .filter_map(|(model_id, model)| {
                    if !selectable(model_id, model) {
                        return None;
                    }
                    let id = model.id.as_deref().unwrap_or(model_id);
                    let freshness = model
                        .last_updated
                        .as_deref()
                        .or(model.release_date.as_deref())
                        .unwrap_or("")
                        .to_string();
                    let context = model
                        .limit
                        .as_ref()
                        .and_then(|limit| limit.context)
                        .unwrap_or(0);
                    Some((
                        freshness,
                        context,
                        id.to_string(),
                        ModelPreset {
                            label: models_dev_model_label(provider_key, provider, id, model),
                            model: id.to_string(),
                        },
                    ))
                })
                .collect::<Vec<_>>();
            choices.sort_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then_with(|| right.1.cmp(&left.1))
                    .then_with(|| left.2.cmp(&right.2))
            });
            choices
                .into_iter()
                .take(limit)
                .map(|(_, _, _, preset)| preset)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    presets.push(custom_model_preset(custom_label));
    presets
}

fn models_dev_model_selectable(model_id: &str, model: &ModelsDevModel) -> bool {
    if model
        .status
        .as_deref()
        .is_some_and(|status| status.eq_ignore_ascii_case("deprecated"))
    {
        return false;
    }
    if models_dev_embedding_model_selectable(model_id, model) {
        return false;
    }
    model
        .modalities
        .as_ref()
        .map(|modalities| {
            modalities.output.is_empty()
                || modalities
                    .output
                    .iter()
                    .any(|modality| modality.eq_ignore_ascii_case("text"))
        })
        .unwrap_or(true)
}

fn models_dev_embedding_model_selectable(model_id: &str, model: &ModelsDevModel) -> bool {
    if model
        .status
        .as_deref()
        .is_some_and(|status| status.eq_ignore_ascii_case("deprecated"))
    {
        return false;
    }
    let text = format!(
        "{} {} {}",
        model_id,
        model.id.as_deref().unwrap_or_default(),
        model.name.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase();
    text.contains("embedding")
        || text.contains("embed")
        || model.modalities.as_ref().is_some_and(|modalities| {
            modalities
                .input
                .iter()
                .chain(modalities.output.iter())
                .any(|modality| modality.eq_ignore_ascii_case("embedding"))
        })
}

fn models_dev_embedding_dimensions(
    index: &ModelsDevIndex,
    provider_id: &str,
    model_id: &str,
) -> Option<u64> {
    models_dev_provider(index, provider_id).and_then(|(_, provider)| {
        provider.models.iter().find_map(|(key, model)| {
            let id = model.id.as_deref().unwrap_or(key);
            if id == model_id || key == model_id {
                model.limit.as_ref().and_then(|limit| limit.output)
            } else {
                None
            }
        })
    })
}

fn models_dev_provider<'a>(
    index: &'a ModelsDevIndex,
    provider_id: &str,
) -> Option<(&'a String, &'a ModelsDevProvider)> {
    index.providers.get_key_value(provider_id).or_else(|| {
        index.providers.iter().find(|(key, provider)| {
            provider
                .id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(provider_id))
                || provider
                    .name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(provider_id))
                || (provider_id == "amazon-bedrock"
                    && (key.to_ascii_lowercase().contains("bedrock")
                        || provider
                            .name
                            .as_deref()
                            .is_some_and(|name| name.to_ascii_lowercase().contains("bedrock"))))
        })
    })
}

fn first_models_dev_model(index: &ModelsDevIndex, provider_id: &str) -> Option<String> {
    models_dev_provider_presets(index, provider_id, "Custom model id", 1)
        .into_iter()
        .find(|preset| preset.model != CUSTOM_MODEL_ID)
        .map(|preset| preset.model)
}

fn models_dev_model_label(
    provider_id: &str,
    provider: &ModelsDevProvider,
    model_id: &str,
    model: &ModelsDevModel,
) -> String {
    let provider_name = provider.name.as_deref().unwrap_or(provider_id);
    let provider_key = provider.id.as_deref().unwrap_or(provider_id);
    let display_name = model.name.as_deref().unwrap_or(model_id);
    let mut parts = vec![format!("{provider_name}: {display_name} ({model_id})")];
    if let Some(family) = model.family.as_deref() {
        parts.push(format!("family={family}"));
    }
    if let Some(limit) = &model.limit {
        if let Some(context) = limit.context {
            parts.push(format!("ctx={context}"));
        }
        if let Some(output) = limit.output {
            parts.push(format!("out={output}"));
        }
    }
    let mut features = Vec::new();
    if model.tool_call.unwrap_or(false) {
        features.push("tools");
    }
    if model.structured_output.unwrap_or(false) {
        features.push("structured");
    }
    if model.reasoning.unwrap_or(false) {
        features.push("reasoning");
    }
    if model.open_weights.unwrap_or(false) {
        features.push("open-weights");
    }
    if !features.is_empty() {
        parts.push(features.join("+"));
    }
    if let Some(updated) = model
        .last_updated
        .as_deref()
        .or(model.release_date.as_deref())
    {
        parts.push(format!("updated={updated}"));
    }
    if provider_key != provider_id {
        parts.push(format!("provider_id={provider_key}"));
    }
    parts.join(" · ")
}

fn custom_model_preset(label: &str) -> ModelPreset {
    ModelPreset {
        label: label.to_string(),
        model: CUSTOM_MODEL_ID.to_string(),
    }
}

fn live_model_presets(model_ids: Vec<String>, custom_label: &str) -> Vec<ModelPreset> {
    let mut ids = model_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    let mut presets = ids
        .into_iter()
        .take(MAX_INDEXED_MODEL_CHOICES)
        .map(|id| ModelPreset {
            label: format!("Live served model: {id}"),
            model: id,
        })
        .collect::<Vec<_>>();
    presets.push(custom_model_preset(custom_label));
    presets
}

fn model_presets_with_configured(
    mut presets: Vec<ModelPreset>,
    configured_model: Option<&str>,
    label_prefix: &str,
) -> Vec<ModelPreset> {
    let Some(configured_model) = configured_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    else {
        return presets;
    };
    if configured_model == CUSTOM_MODEL_ID
        || presets
            .iter()
            .any(|preset| preset.model == configured_model)
    {
        return presets;
    }
    presets.insert(
        0,
        ModelPreset {
            label: format!("{label_prefix}: {configured_model}"),
            model: configured_model.to_string(),
        },
    );
    presets
}

fn custom_only_model_presets(custom_label: &str) -> Vec<ModelPreset> {
    vec![custom_model_preset(custom_label)]
}

async fn discover_openai_compatible_models(endpoint: &str) -> Vec<String> {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return Vec::new();
    };
    for endpoint in endpoint_discovery_candidates(endpoint) {
        let url = openai_models_url(&endpoint);
        match client.get(url).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(response) => {
                    let models: Vec<String> = response
                        .json::<OpenAiModelsResponse>()
                        .await
                        .map(|body| body.data.into_iter().map(|model| model.id).collect())
                        .unwrap_or_default();
                    if !models.is_empty() {
                        return models;
                    }
                }
                Err(_) => continue,
            },
            Err(_) => continue,
        }
    }
    Vec::new()
}

async fn discover_ollama_models(endpoint: &str) -> Vec<String> {
    let mut models = discover_openai_compatible_models(endpoint).await;
    if !models.is_empty() {
        return models;
    }
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return Vec::new();
    };
    for endpoint in endpoint_discovery_candidates(endpoint) {
        let url = ollama_tags_url(&endpoint);
        if let Ok(response) = client.get(url).send().await {
            if let Ok(response) = response.error_for_status() {
                models = response
                    .json::<OllamaTagsResponse>()
                    .await
                    .map(|body| body.models.into_iter().map(|model| model.name).collect())
                    .unwrap_or_default();
                if !models.is_empty() {
                    return models;
                }
            }
        }
    }
    models
}

async fn discover_local_provider_models(kind: &str, endpoint: &str) -> Vec<String> {
    match kind {
        "ollama" => discover_ollama_models(endpoint).await,
        _ => discover_openai_compatible_models(endpoint).await,
    }
}

fn openai_models_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{trimmed}/models")
    } else if trimmed.ends_with("/v1/embeddings") {
        format!("{}/models", trimmed.trim_end_matches("/embeddings"))
    } else if trimmed.ends_with("/v1/chat/completions") {
        format!("{}/models", trimmed.trim_end_matches("/chat/completions"))
    } else {
        format!("{trimmed}/v1/models")
    }
}

fn endpoint_discovery_candidates(endpoint: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let trimmed = endpoint.trim().trim_end_matches('/').to_string();
    if !trimmed.is_empty() {
        candidates.push(trimmed.clone());
    }
    if trimmed.contains("host.docker.internal") {
        candidates.push(trimmed.replace("host.docker.internal", "localhost"));
        candidates.push(trimmed.replace("host.docker.internal", "127.0.0.1"));
    } else if trimmed.contains("localhost") || trimmed.contains("127.0.0.1") {
        candidates.push(
            trimmed
                .replace("localhost", "host.docker.internal")
                .replace("127.0.0.1", "host.docker.internal"),
        );
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn openai_embeddings_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if trimmed.ends_with("/v1/embeddings") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/embeddings")
    } else if trimmed.ends_with("/v1/chat/completions") {
        format!(
            "{}/embeddings",
            trimmed.trim_end_matches("/chat/completions")
        )
    } else {
        format!("{trimmed}/v1/embeddings")
    }
}

fn ollama_tags_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    let base = trimmed
        .strip_suffix("/v1")
        .or_else(|| trimmed.strip_suffix("/v1/chat/completions"))
        .unwrap_or(trimmed);
    format!("{base}/api/tags")
}

fn populate_secret_env_values(mut env_text: String) -> String {
    for name in [
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "COAT_LLM_GATEWAY_API_KEY",
        "COAT_CONTROL_CHAT_API_KEY",
        "MODEL_PROVIDER_API_KEY",
        "MODEL_PROVIDER_RESEARCH_API_KEY",
        "HF_TOKEN",
        "HUGGINGFACE_TOKEN",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "MEMORY_GATEWAY_GRAPHITI_TOKEN",
        "MEMORY_GATEWAY_QDRANT_TOKEN",
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

fn local_model_provider_preset_labels() -> Vec<&'static str> {
    LOCAL_MODEL_PROVIDER_PRESETS
        .iter()
        .map(|preset| preset.label)
        .collect()
}

fn default_local_model_provider_index(prefer_vllm: bool) -> usize {
    let preferred_kind = if prefer_vllm { "vllm" } else { "ollama" };
    LOCAL_MODEL_PROVIDER_PRESETS
        .iter()
        .position(|preset| preset.kind == preferred_kind)
        .unwrap_or(0)
}

fn local_model_provider_preset(index: usize) -> LocalModelProviderPreset {
    LOCAL_MODEL_PROVIDER_PRESETS
        .get(index)
        .copied()
        .unwrap_or(LOCAL_MODEL_PROVIDER_PRESETS[0])
}

fn model_preset_labels(presets: &[ModelPreset]) -> Vec<String> {
    presets.iter().map(|preset| preset.label.clone()).collect()
}

fn default_model_preset_index(presets: &[ModelPreset], default_model: &str) -> usize {
    presets
        .iter()
        .position(|preset| preset.model == default_model)
        .unwrap_or(0)
}

fn model_preset(presets: &[ModelPreset], index: usize) -> ModelPreset {
    presets
        .get(index)
        .cloned()
        .unwrap_or_else(|| presets[0].clone())
}

fn select_model_preset(
    theme: &ColorfulTheme,
    prompt: &str,
    presets: &[ModelPreset],
    default_model: &str,
    custom_prompt: &str,
) -> anyhow::Result<String> {
    let labels = model_preset_labels(presets);
    let choice = Select::with_theme(theme)
        .with_prompt(prompt)
        .items(&labels)
        .default(default_model_preset_index(presets, default_model))
        .interact()?;
    let preset = model_preset(presets, choice);
    if preset.model == CUSTOM_MODEL_ID {
        let mut input = Input::with_theme(theme);
        input = input.with_prompt(custom_prompt);
        if !default_model.trim().is_empty() {
            input = input.default(default_model.to_string());
        }
        input.interact_text().map_err(Into::into)
    } else {
        Ok(preset.model.to_string())
    }
}

fn model_param_preset_labels() -> Vec<&'static str> {
    MODEL_PARAM_PRESETS
        .iter()
        .map(|preset| preset.label)
        .collect()
}

fn default_model_param_preset_index(latency_class: &str) -> usize {
    MODEL_PARAM_PRESETS
        .iter()
        .position(|preset| preset.latency_class == Some(latency_class) && !preset.custom)
        .unwrap_or(1)
}

fn default_model_param_preset_index_for_values(
    default_latency_class: &str,
    defaults: &ModelParamValues,
) -> usize {
    if let Some(index) = MODEL_PARAM_PRESETS
        .iter()
        .position(|preset| !preset.custom && model_param_values_from_preset(*preset) == *defaults)
    {
        return index;
    }
    if [
        defaults.speed_tier.as_ref(),
        defaults.temperature.as_ref(),
        defaults.top_p.as_ref(),
        defaults.max_output_tokens.as_ref(),
        defaults.reasoning_effort.as_ref(),
        defaults.timeout_seconds.as_ref(),
    ]
    .iter()
    .any(|value| value.is_some())
    {
        return MODEL_PARAM_PRESETS
            .iter()
            .position(|preset| preset.custom)
            .unwrap_or_else(|| default_model_param_preset_index(default_latency_class));
    }
    default_model_param_preset_index(
        defaults
            .latency_class
            .as_deref()
            .unwrap_or(default_latency_class),
    )
}

fn model_param_preset(index: usize) -> ModelParamPreset {
    MODEL_PARAM_PRESETS.get(index).copied().unwrap_or_else(|| {
        MODEL_PARAM_PRESETS
            .iter()
            .copied()
            .find(|preset| {
                preset.latency_class == Some("balanced")
                    && preset.reasoning_effort == Some("medium")
                    && !preset.custom
            })
            .unwrap_or(MODEL_PARAM_PRESETS[0])
    })
}

fn model_param_values_from_preset(preset: ModelParamPreset) -> ModelParamValues {
    ModelParamValues {
        latency_class: preset.latency_class.map(str::to_string),
        speed_tier: preset.speed_tier.map(str::to_string),
        temperature: preset.temperature.map(str::to_string),
        top_p: preset.top_p.map(str::to_string),
        max_output_tokens: preset.max_output_tokens.map(str::to_string),
        reasoning_effort: preset.reasoning_effort.map(str::to_string),
        timeout_seconds: preset.timeout_seconds.map(str::to_string),
    }
}

fn apply_model_param_values(
    mut env_text: String,
    prefix: &str,
    params: &ModelParamValues,
) -> String {
    for (suffix, value) in [
        ("LATENCY_CLASS", params.latency_class.as_deref()),
        ("SPEED_TIER", params.speed_tier.as_deref()),
        ("TEMPERATURE", params.temperature.as_deref()),
        ("TOP_P", params.top_p.as_deref()),
        ("MAX_OUTPUT_TOKENS", params.max_output_tokens.as_deref()),
        ("REASONING_EFFORT", params.reasoning_effort.as_deref()),
        ("TIMEOUT_SECONDS", params.timeout_seconds.as_deref()),
    ] {
        env_text = replace_env_line(
            env_text,
            &format!("{prefix}_{suffix}"),
            value.unwrap_or_default(),
        );
    }
    env_text
}

fn model_param_values_from_env(
    values: &BTreeMap<String, String>,
    prefix: &str,
) -> ModelParamValues {
    ModelParamValues {
        latency_class: env_value(values, &format!("{prefix}_LATENCY_CLASS")).and_then(non_empty),
        speed_tier: env_value(values, &format!("{prefix}_SPEED_TIER")).and_then(non_empty),
        temperature: env_value(values, &format!("{prefix}_TEMPERATURE")).and_then(non_empty),
        top_p: env_value(values, &format!("{prefix}_TOP_P")).and_then(non_empty),
        max_output_tokens: env_value(values, &format!("{prefix}_MAX_OUTPUT_TOKENS"))
            .and_then(non_empty),
        reasoning_effort: env_value(values, &format!("{prefix}_REASONING_EFFORT"))
            .and_then(non_empty),
        timeout_seconds: env_value(values, &format!("{prefix}_TIMEOUT_SECONDS"))
            .and_then(non_empty),
    }
}

fn select_model_param_values_with_env(
    theme: &ColorfulTheme,
    prompt: &str,
    default_latency_class: &str,
    values: &BTreeMap<String, String>,
    prefix: &str,
) -> anyhow::Result<ModelParamValues> {
    let mut defaults = model_param_values_from_env(values, prefix);
    if defaults.latency_class.is_none() {
        defaults.latency_class = Some(default_latency_class.to_string());
    }
    select_model_param_values_with_defaults(theme, prompt, default_latency_class, &defaults)
}

fn select_model_param_values_with_defaults(
    theme: &ColorfulTheme,
    prompt: &str,
    default_latency_class: &str,
    defaults: &ModelParamValues,
) -> anyhow::Result<ModelParamValues> {
    let labels = model_param_preset_labels();
    let choice = Select::with_theme(theme)
        .with_prompt(prompt)
        .items(&labels)
        .default(default_model_param_preset_index_for_values(
            default_latency_class,
            defaults,
        ))
        .interact()?;
    let preset = model_param_preset(choice);
    if !preset.custom {
        return Ok(model_param_values_from_preset(preset));
    }

    let latency_class: String = Input::with_theme(theme)
        .with_prompt("Latency class (fast, balanced, deep, batch, or blank)")
        .allow_empty(true)
        .default(
            defaults
                .latency_class
                .clone()
                .unwrap_or_else(|| default_latency_class.to_string()),
        )
        .interact_text()?;
    let speed_tier = prompt_optional_default(
        theme,
        "Provider speed tier (speed, priority, flex, auto, default, or blank)",
        defaults.speed_tier.as_deref(),
    )?;
    let temperature = prompt_optional_default(
        theme,
        "Temperature (blank keeps provider default)",
        defaults.temperature.as_deref(),
    )?;
    let top_p = prompt_optional_default(
        theme,
        "Top-p (blank keeps provider default)",
        defaults.top_p.as_deref(),
    )?;
    let max_output_tokens = prompt_optional_default(
        theme,
        "Max output tokens (blank keeps provider default)",
        defaults.max_output_tokens.as_deref(),
    )?;
    let reasoning_effort = prompt_optional_default(
        theme,
        "Reasoning effort (minimal, low, medium, high, xhigh, or blank)",
        defaults.reasoning_effort.as_deref(),
    )?;
    let timeout_seconds = prompt_optional_default(
        theme,
        "Timeout seconds (blank keeps provider default)",
        defaults.timeout_seconds.as_deref(),
    )?;

    Ok(ModelParamValues {
        latency_class: non_empty(latency_class),
        speed_tier: non_empty(speed_tier),
        temperature: non_empty(temperature),
        top_p: non_empty(top_p),
        max_output_tokens: non_empty(max_output_tokens),
        reasoning_effort: non_empty(reasoning_effort),
        timeout_seconds: non_empty(timeout_seconds),
    })
}

fn prompt_optional_default(
    theme: &ColorfulTheme,
    prompt: &str,
    default: Option<&str>,
) -> anyhow::Result<String> {
    let mut input = Input::with_theme(theme);
    input = input.with_prompt(prompt).allow_empty(true);
    if let Some(default) = default.filter(|value| !value.trim().is_empty()) {
        input = input.default(default.to_string());
    }
    input.interact_text().map_err(Into::into)
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
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
    let mut found = false;
    let mut lines = env_text
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                found = true;
                format!("{prefix}{value}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>();
    if !found {
        lines.push(format!("{prefix}{value}"));
    }
    lines.join("\n") + "\n"
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
        "COAT_LLM_GATEWAY_PROVIDER",
        "COAT_LLM_GATEWAY_URL",
        "COAT_LLM_GATEWAY_API_KEY",
        "COAT_LLM_GATEWAY_DEFAULT_MODEL",
        "COAT_LLM_GATEWAY_WORK_MODEL",
        "COAT_LLM_GATEWAY_RESEARCH_MODEL",
        "COAT_LLM_GATEWAY_CHAT_MODEL",
        "COAT_CONTROL_CHAT_BACKEND",
        "COAT_CONTROL_CHAT_PROVIDER",
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
        "COAT_CONTROL_CHAT_COMPLETIONS_URL",
        "COAT_CONTROL_CHAT_MODEL",
        "COAT_CONTROL_CHAT_API_KEY",
        "MEMORY_GATEWAY_GRAPHITI_MCP_URL",
        "MEMORY_GATEWAY_QDRANT_URL",
        "MEMORY_GATEWAY_EMBEDDING_URL",
        "MEMORY_GATEWAY_EMBEDDING_MODEL",
        "MEMORY_GATEWAY_GRAPHITI_TOKEN",
        "MEMORY_GATEWAY_QDRANT_TOKEN",
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
    println!("COAT-managed auth/setup flows:");
    println!("  coat setup model-index refresh");
    println!("  coat setup login --codex --claude --preflight");
    println!("  coat setup login --ollama-model <served-model> --preflight --allow-stub-runners");
    println!("  coat setup login --hf --preflight");
    println!("  coat setup sso --profile <profile> --write-env --bedrock-live --preflight");
    println!();
    println!("Use --dry-run on either command to see the provider CLI calls without running them.");
    println!(
        "`coat setup local-auth` can also run the selected login/setup actions and preflight in one guided flow."
    );
}

fn run_local_auth_actions(actions: &[LocalAuthAction], dry_run: bool) -> anyhow::Result<()> {
    if actions.is_empty() {
        println!("no login/setup actions selected");
        return Ok(());
    }
    for action in actions {
        let (program, args, description) = local_auth_action_command(action);
        println!("{description}");
        if dry_run {
            println!("dry-run: {}", shell_command(&program, &args));
        } else {
            run_program_args(&program, &args)?;
        }
    }
    Ok(())
}

fn local_auth_action_command(action: &LocalAuthAction) -> (String, Vec<String>, String) {
    match action {
        LocalAuthAction::CodexLogin => (
            "codex".to_string(),
            vec!["login".to_string()],
            "Codex device/browser login".to_string(),
        ),
        LocalAuthAction::ClaudeLogin {
            email,
            sso,
            console,
        } => {
            let mut args = vec!["auth".to_string(), "login".to_string()];
            if let Some(email) = email {
                args.push("--email".to_string());
                args.push(email.clone());
            }
            if *sso {
                args.push("--sso".to_string());
            }
            if *console {
                args.push("--console".to_string());
            }
            (
                "claude".to_string(),
                args,
                "Claude Code auth login (browser/device)".to_string(),
            )
        }
        LocalAuthAction::AwsSso { profile } => (
            "aws".to_string(),
            vec![
                "sso".to_string(),
                "login".to_string(),
                "--profile".to_string(),
                profile.clone(),
            ],
            format!("AWS SSO login for profile {profile}"),
        ),
        LocalAuthAction::HuggingFaceLogin => (
            "hf".to_string(),
            vec!["auth".to_string(), "login".to_string()],
            "Hugging Face CLI login".to_string(),
        ),
        LocalAuthAction::OllamaPull { model } => (
            "ollama".to_string(),
            vec!["pull".to_string(), model.clone()],
            format!("Ollama pull for model {model}"),
        ),
    }
}

fn run_local_auth_preflight(env_file: &Path, allow_stub_runners: bool) -> anyhow::Result<()> {
    let restate_cloud_env_file =
        effective_restate_cloud_env_file(Path::new(DEFAULT_RESTATE_CLOUD_ENV))?;
    run_local_compose_preflight(LocalComposePreflightInput {
        env_files: &[env_file.to_path_buf()],
        restate_cloud: false,
        restate_cloud_env_file: &restate_cloud_env_file,
        allow_uninitialized: effective_allow_uninitialized(false)?,
        allow_stub_runners: allow_stub_runners || effective_allow_stub_runners(false)?,
    })
}

fn print_local_auth_preflight_command(env_file: &Path, allow_stub_runners: bool) {
    let mut args = vec![
        "deploy".to_string(),
        "local".to_string(),
        "preflight".to_string(),
        "--env-file".to_string(),
        env_file.display().to_string(),
    ];
    if allow_stub_runners {
        args.push("--allow-stub-runners".to_string());
    }
    println!("dry-run: {}", shell_command("coat", &args));
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

    let (mut memory_warnings, mut memory_failures) = memory_store_preflight_findings(values);
    warnings.append(&mut memory_warnings);
    failures.append(&mut memory_failures);
    let (mut chat_warnings, mut chat_failures) = control_chat_preflight_findings(values);
    warnings.append(&mut chat_warnings);
    failures.append(&mut chat_failures);
    let (mut web_warnings, mut web_failures) = web_search_preflight_findings(values);
    warnings.append(&mut web_warnings);
    failures.append(&mut web_failures);

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
                    "{lane} is live but no Claude auth is configured; set ANTHROPIC_API_KEY/ANTHROPIC_AUTH_TOKEN/CLAUDE_CODE_OAUTH_TOKEN or {auth_mode_key}=runner_local_device after `coat setup login --claude`"
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
    if !model_provider_model_present(values, kind_key, model_key) {
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
            if !model_provider_endpoint_present(values, endpoint_key, &kind) {
                issues.push(format!("{lane} is live but {endpoint_key} is not set"));
            }
        }
    }
    issues
}

fn model_provider_model_present(
    values: &BTreeMap<String, String>,
    kind_key: &str,
    model_key: &str,
) -> bool {
    if env_present(values, model_key) {
        return true;
    }
    if !matches!(
        env_value(values, kind_key).as_deref(),
        Some("open_ai") | Some("open_ai_compatible") | None
    ) {
        return false;
    }
    match model_key {
        "MODEL_PROVIDER_RESEARCH_MODEL" => any_env_present(
            values,
            &[
                "COAT_LLM_GATEWAY_RESEARCH_MODEL",
                "COAT_LLM_GATEWAY_DEFAULT_MODEL",
            ],
        ),
        "MODEL_PROVIDER_MODEL" => any_env_present(
            values,
            &[
                "COAT_LLM_GATEWAY_WORK_MODEL",
                "COAT_LLM_GATEWAY_DEFAULT_MODEL",
            ],
        ),
        _ => false,
    }
}

fn model_provider_endpoint_present(
    values: &BTreeMap<String, String>,
    endpoint_key: &str,
    kind: &str,
) -> bool {
    env_present(values, endpoint_key)
        || matches!(kind, "open_ai" | "open_ai_compatible")
            && env_present(values, "COAT_LLM_GATEWAY_URL")
}

fn memory_embedding_needs_token(values: &BTreeMap<String, String>) -> bool {
    let url = env_value(values, "MEMORY_GATEWAY_EMBEDDING_URL").unwrap_or_default();
    url.contains("api.openai.com")
        && !any_env_present(
            values,
            &["MEMORY_GATEWAY_EMBEDDING_TOKEN", "OPENAI_API_KEY"],
        )
}

fn web_search_preflight_findings(values: &BTreeMap<String, String>) -> (Vec<String>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut failures = Vec::new();
    if !env_truthy_value(env_value(values, "COAT_WEB_SEARCH_ENABLED").as_deref()) {
        return (warnings, failures);
    }

    let route = env_value(values, "COAT_WEB_SEARCH_ROUTE")
        .unwrap_or_else(|| "coordinator_task".to_string());
    let provider =
        env_value(values, "COAT_WEB_SEARCH_PROVIDER").unwrap_or_else(|| "agent_native".to_string());
    let auth_mode = env_value(values, "COAT_WEB_SEARCH_AUTH_MODE")
        .unwrap_or_else(|| "api_key_or_none".to_string());
    let custom_provider = matches!(
        provider.as_str(),
        "custom" | "search_api" | "mcp_gateway" | "exa" | "tavily" | "brave" | "serpapi"
    );

    if custom_provider && !env_present(values, "COAT_WEB_SEARCH_URL") {
        failures.push(format!(
            "COAT_WEB_SEARCH_PROVIDER={provider} requires COAT_WEB_SEARCH_URL"
        ));
    }
    if custom_provider
        && auth_mode == "api_key_or_none"
        && !any_env_present(values, &["COAT_WEB_SEARCH_API_KEY"])
    {
        failures.push(format!(
            "COAT_WEB_SEARCH_PROVIDER={provider} uses api_key_or_none but COAT_WEB_SEARCH_API_KEY is not set"
        ));
    }
    if route == "runner_registry"
        && ![
            "CODEX_NATIVE_WEB_SEARCH",
            "CLAUDE_CODE_NATIVE_WEB_SEARCH",
            "MODEL_PROVIDER_WEB_SEARCH_ENABLED",
            "MODEL_PROVIDER_RESEARCH_WEB_SEARCH_ENABLED",
        ]
        .iter()
        .any(|key| env_truthy_value(env_value(values, key).as_deref()))
    {
        warnings.push(
            "COAT_WEB_SEARCH_ROUTE=runner_registry but no Compose runner lane advertises web_search; external runners may still satisfy it"
                .to_string(),
        );
    }

    (warnings, failures)
}

fn control_chat_preflight_findings(
    values: &BTreeMap<String, String>,
) -> (Vec<String>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut failures = Vec::new();
    let backend = env_value(values, "COAT_CONTROL_CHAT_BACKEND")
        .unwrap_or_else(|| "configured".to_string())
        .to_ascii_lowercase();
    if backend == "stub" {
        return (warnings, failures);
    }
    if matches!(backend.as_str(), "runner_registry" | "auto") {
        warnings.push(
            "control gateway chat may use runner-registry discovery; keep this only for explicitly chat-labeled runners"
                .to_string(),
        );
        return (warnings, failures);
    }

    let has_control_model = env_present(values, "COAT_CONTROL_CHAT_MODEL");
    let has_gateway_chat_model = env_present(values, "COAT_LLM_GATEWAY_CHAT_MODEL")
        || env_present(values, "COAT_LLM_GATEWAY_DEFAULT_MODEL");
    let has_model = has_control_model || has_gateway_chat_model;
    let has_control_url = env_present(values, "COAT_CONTROL_CHAT_COMPLETIONS_URL");
    let has_gateway_chat_url = env_present(values, "COAT_LLM_GATEWAY_CHAT_COMPLETIONS_URL")
        || (env_present(values, "COAT_LLM_GATEWAY_URL") && has_gateway_chat_model);
    let has_url = has_control_url || has_gateway_chat_url;
    let direct_openai = env_value_is(values, "COAT_CONTROL_CHAT_PROVIDER", "openai")
        && has_model
        && any_env_present(values, &["OPENAI_API_KEY", "COAT_CONTROL_CHAT_API_KEY"]);

    if (has_control_url || env_present(values, "COAT_LLM_GATEWAY_CHAT_COMPLETIONS_URL"))
        && !has_model
    {
        failures.push(
            "control gateway chat has a URL/gateway configured but no chat model is set"
                .to_string(),
        );
    } else if has_model && !(has_url || direct_openai) {
        failures.push(
            "control gateway chat has a model but no matching COAT_CONTROL_CHAT_COMPLETIONS_URL, COAT_LLM_GATEWAY_URL, or direct OpenAI provider/key"
                .to_string(),
        );
    } else if !has_model && !has_url {
        warnings.push(
            "control gateway chat has no configured provider and will use the local stub"
                .to_string(),
        );
    }

    (warnings, failures)
}

fn memory_store_preflight_findings(
    values: &BTreeMap<String, String>,
) -> (Vec<String>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut failures = Vec::new();
    let embedding_url = env_present(values, "MEMORY_GATEWAY_EMBEDDING_URL");
    let embedding_model = env_present(values, "MEMORY_GATEWAY_EMBEDDING_MODEL");
    let qdrant_url = env_present(values, "MEMORY_GATEWAY_QDRANT_URL");

    if embedding_url && !embedding_model {
        failures.push(
            "MEMORY_GATEWAY_EMBEDDING_URL is set but MEMORY_GATEWAY_EMBEDDING_MODEL is missing"
                .to_string(),
        );
    }
    if embedding_model && !embedding_url {
        warnings.push(
            "MEMORY_GATEWAY_EMBEDDING_MODEL is set but MEMORY_GATEWAY_EMBEDDING_URL is missing; embeddings will be disabled"
                .to_string(),
        );
    }
    if qdrant_url && !(embedding_url && embedding_model) {
        warnings.push(
            "MEMORY_GATEWAY_QDRANT_URL is set but embeddings are not fully configured; vector memory will stay inactive"
                .to_string(),
        );
    }
    if memory_embedding_needs_token(values) {
        warnings.push(
            "memory embeddings use the OpenAI endpoint but neither MEMORY_GATEWAY_EMBEDDING_TOKEN nor OPENAI_API_KEY is set".to_string(),
        );
    }

    (warnings, failures)
}

fn any_env_present(values: &BTreeMap<String, String>, names: &[&str]) -> bool {
    names.iter().any(|name| env_present(values, name))
}

fn env_present(values: &BTreeMap<String, String>, name: &str) -> bool {
    env_value(values, name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn env_or_default(values: &BTreeMap<String, String>, name: &str, default: &str) -> String {
    env_value(values, name).unwrap_or_else(|| default.to_string())
}

fn env_value_is(values: &BTreeMap<String, String>, name: &str, expected: &str) -> bool {
    env_value(values, name).is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn env_value_is_value(values: &BTreeMap<String, String>, name: &str, expected: &str) -> bool {
    env_value(values, name).is_some_and(|value| value == expected)
}

fn env_truthy_value(value: Option<&str>) -> bool {
    value
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn bool_env(value: bool) -> &'static str {
    if value { "true" } else { "false" }
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
        CUSTOM_MODEL_ID, ChatClientArgs, Cli, Commands, ComposeConfigArgs, ComposeUpArgs,
        DEFAULT_GOAL_STORE_URL, DeploySubcommand, EphemeralJobsApplyArgs, ExecutorJobRenderArgs,
        GoalMechanismSubcommand, GoalSubcommand, GoalThunkSubcommand, HelmTemplateArgs,
        HelmUpgradeArgs, HumanSubcommand, K8sStatusArgs, KubectlApplySpec, LocalAuthAction,
        LoginArgs, MODEL_PARAM_PRESETS, ModelsDevIndex, PlanSubcommand, ProjectInitAction,
        ProjectInitCheck, SetupSubcommand, ToolSubcommand, apply_capacity_plan_policy_from_config,
        apply_config_profile, apply_model_param_values, bump_release_versions,
        chat_client_default_action, claude_mcp_json, codex_mcp_add_args,
        compose_config_command_args, compose_model_preflight_findings, compose_runner_modes,
        compose_up_command_args, default_local_model_provider_index,
        default_model_param_preset_index, default_model_preset_index,
        endpoint_discovery_candidates, endpoint_from_config, ensure_json_goal_id,
        executor_job_manifest, extract_follow_ups, helm_template_args, helm_upgrade_args,
        kubectl_apply_args, kubectl_ephemeral_jobs_apply_args, kubectl_rollout_status_args,
        latest_goal_id_from_value, live_model_presets, load_fresh_model_index_from_paths,
        local_auth_action_command, local_auth_profile_defaults, local_model_provider_preset,
        local_model_provider_preset_labels, login_actions_from_args, merge_coat_config,
        model_index_cache_is_fresh, model_param_preset, model_param_preset_labels,
        model_param_values_from_env, model_param_values_from_preset, model_preset,
        model_preset_labels, model_presets_with_configured, models_dev_embedding_dimensions,
        models_dev_embedding_presets, models_dev_provider_presets, openai_embeddings_url,
        parse_env_file_content, project_init_action, read_json_file, release_plan_json,
        replace_env_line, replace_toml_section_value, replace_yaml_root_value,
        restate_cloud_env_placeholders,
    };
    use clap::{CommandFactory, Parser};
    use coat_domain::{
        CapacityScalingPolicy, CoatCliConfig, CoatConfig, CoatLocalDeployConfig,
        CoatRunnerCapacityConfig, CoatServiceEndpoints, NetworkAccess, RunnerPoolDemand,
        RunnerScalingRequest, SandboxBackend, SandboxLaunchPlan, SandboxNetworkPlan,
        SandboxResourcePlan, SandboxSecurityPlan, WebSearchRequest,
    };
    use std::{collections::BTreeMap, fs, path::PathBuf, time::Duration};
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
            "guide", "plan", "goal", "human", "deploy", "event", "runner", "tool", "memory",
            "store", "sandbox", "release", "setup", "init",
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
    fn bare_coat_is_help_surface_not_dialogue() {
        let cli = Cli::parse_from(["coat"]);
        assert!(
            cli.command.is_none(),
            "bare coat should remain an explicit subcommand/help surface"
        );

        let help = Cli::command().render_long_help().to_string();
        for expected in [
            "Usage: coat",
            "guide",
            "deploy",
            "Open guided setup and human-queue workflows",
        ] {
            assert!(
                help.contains(expected),
                "root help should include {expected:?}; help was:\n{help}"
            );
        }
    }

    #[test]
    fn runner_help_exposes_capacity_planning_without_provisioning() {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut("runner")
            .expect("runner subcommand exists")
            .render_long_help()
            .to_string();

        for expected in [
            "capacity-plan",
            "bounded capacity recommendation",
            "Register, inspect, and test distributed runners",
        ] {
            assert!(
                help.contains(expected),
                "runner help should include {expected:?}; help was:\n{help}"
            );
        }

        let capacity_help = command
            .find_subcommand_mut("runner")
            .expect("runner subcommand exists")
            .find_subcommand_mut("capacity-plan")
            .expect("runner capacity-plan subcommand exists")
            .render_long_help()
            .to_string();
        assert!(
            capacity_help.contains("policy may be omitted to use config.runner_capacity"),
            "capacity-plan help should document config policy fallback; help was:\n{capacity_help}"
        );
    }

    #[test]
    fn tool_help_exposes_registry_and_web_search_calls() {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut("tool")
            .expect("tool subcommand exists")
            .render_long_help()
            .to_string();

        for expected in [
            "Call MCP/tool-registry utilities",
            "list",
            "call",
            "web-search",
        ] {
            assert!(
                help.contains(expected),
                "tool help should include {expected:?}; help was:\n{help}"
            );
        }

        let web_search_help = command
            .find_subcommand_mut("tool")
            .expect("tool subcommand exists")
            .find_subcommand_mut("web-search")
            .expect("tool web-search subcommand exists")
            .render_long_help()
            .to_string();
        for expected in ["--tool-registry-url", "--file", "COAT_TOOL_REGISTRY_TOKEN"] {
            assert!(
                web_search_help.contains(expected),
                "tool web-search help should include {expected:?}; help was:\n{web_search_help}"
            );
        }
    }

    #[test]
    fn deploy_help_documents_subcommands_and_examples() {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut("deploy")
            .expect("deploy subcommand exists")
            .render_long_help()
            .to_string();

        for expected in [
            "Run and inspect the local Docker Compose stack",
            "Render, apply, and inspect Kubernetes manifests and executor Jobs",
            "Lint, render, install, rollback, and package the jattg Helm chart",
            "Prepare Restate Cloud env, tunnel, and service registration commands",
            "coat deploy local preflight --allow-stub-runners",
            "coat deploy restate register-cloud",
        ] {
            assert!(
                help.contains(expected),
                "deploy help should include {expected:?}; help was:\n{help}"
            );
        }

        let mut command = Cli::command();
        let local_help = command
            .find_subcommand_mut("deploy")
            .expect("deploy subcommand exists")
            .find_subcommand_mut("local")
            .expect("deploy local subcommand exists")
            .render_long_help()
            .to_string();
        for expected in [
            "Check initialization, Docker, env files, runner modes, and model setup",
            "Run docker compose up after preflight unless --skip-preflight is set",
            "coat deploy local config --env-file infra/compose/local-providers.env",
        ] {
            assert!(
                local_help.contains(expected),
                "deploy local help should include {expected:?}; help was:\n{local_help}"
            );
        }
    }

    #[test]
    fn setup_help_documents_login_and_sso_flows() {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut("setup")
            .expect("setup subcommand exists")
            .render_long_help()
            .to_string();

        for expected in [
            "Run provider device/browser login flows and optional local preflight",
            "Run AWS SSO login, update local env, and optionally preflight",
            "Refresh or inspect the external model index used by setup wizards",
            "local-auth",
            "chat-client",
        ] {
            assert!(
                help.contains(expected),
                "setup help should include {expected:?}; help was:\n{help}"
            );
        }

        let login_help = command
            .find_subcommand_mut("setup")
            .expect("setup subcommand exists")
            .find_subcommand_mut("login")
            .expect("setup login subcommand exists")
            .render_long_help()
            .to_string();
        for expected in [
            "--claude-email",
            "--claude-sso",
            "--claude-console",
            "Run Claude Code auth login on this runner node",
            "Force Claude Code organization SSO during auth login",
        ] {
            assert!(
                login_help.contains(expected),
                "setup login help should include {expected:?}; help was:\n{login_help}"
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

        let resume_thunk = Cli::try_parse_from([
            "coat",
            "human",
            "resume-thunk",
            "--thunk-id",
            "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
            "--response-summary",
            "continue with smoke lane",
        ])
        .expect("parse human resume-thunk");
        assert!(matches!(
            resume_thunk.command,
            Some(Commands::Human(ref human))
                if matches!(human.command, HumanSubcommand::ResumeThunk(_))
        ));

        let goal_vote = Cli::try_parse_from([
            "coat",
            "goal",
            "vote",
            "--goal-id",
            "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
            "--direction",
            "up",
            "--reason",
            "promote umbrella objective",
        ])
        .expect("parse goal vote");
        assert!(matches!(
            goal_vote.command,
            Some(Commands::Goal(ref goal)) if matches!(goal.command, GoalSubcommand::Vote(_))
        ));

        let compute_graph = Cli::try_parse_from([
            "coat",
            "goal",
            "compute-graph",
            "--goal-id",
            "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
        ])
        .expect("parse goal compute-graph");
        assert!(matches!(
            compute_graph.command,
            Some(Commands::Goal(ref goal))
                if matches!(goal.command, GoalSubcommand::ComputeGraph(_))
        ));

        let mechanism = Cli::try_parse_from([
            "coat",
            "goal",
            "mechanism",
            "start",
            "--goal-id",
            "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
            "--file",
            "examples/mechanism-round-consensus.json",
        ])
        .expect("parse goal mechanism start");
        assert!(matches!(
            mechanism.command,
            Some(Commands::Goal(ref goal))
                if matches!(
                    goal.command,
                    GoalSubcommand::Mechanism(ref mechanism)
                        if matches!(mechanism.command, GoalMechanismSubcommand::Start(_))
                )
        ));

        let thunk = Cli::try_parse_from([
            "coat",
            "goal",
            "thunk",
            "create",
            "--goal-id",
            "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
            "--file",
            "examples/delayed-compute-thunk-human-input.json",
        ])
        .expect("parse goal thunk create");
        assert!(matches!(
            thunk.command,
            Some(Commands::Goal(ref goal))
                if matches!(
                    goal.command,
                    GoalSubcommand::Thunk(ref thunk)
                        if matches!(thunk.command, GoalThunkSubcommand::Create(_))
                )
        ));

        let follow_ups = Cli::try_parse_from(["coat", "plan", "follow-ups", "--json"])
            .expect("parse plan follow-ups");
        assert!(matches!(
            follow_ups.command,
            Some(Commands::Plan(ref plan))
                if matches!(plan.command, PlanSubcommand::FollowUps(_))
        ));

        let login = Cli::try_parse_from([
            "coat",
            "setup",
            "login",
            "--codex",
            "--claude",
            "--preflight",
        ])
        .expect("parse setup login");
        assert!(matches!(
            login.command,
            Some(Commands::Setup(ref setup))
                if matches!(setup.command, SetupSubcommand::Login(_))
        ));

        let sso = Cli::try_parse_from([
            "coat",
            "setup",
            "sso",
            "--profile",
            "jattg-dev",
            "--write-env",
            "--bedrock-live",
        ])
        .expect("parse setup sso");
        assert!(matches!(
            sso.command,
            Some(Commands::Setup(ref setup)) if matches!(setup.command, SetupSubcommand::Sso(_))
        ));

        let model_index = Cli::try_parse_from([
            "coat",
            "setup",
            "model-index",
            "refresh",
            "--output",
            "/tmp/models.dev.api.json",
        ])
        .expect("parse setup model-index refresh");
        assert!(matches!(
            model_index.command,
            Some(Commands::Setup(ref setup))
                if matches!(setup.command, SetupSubcommand::ModelIndex(_))
        ));

        let embedding_index = Cli::try_parse_from([
            "coat",
            "setup",
            "model-index",
            "show",
            "--provider",
            "openai",
            "--embeddings",
        ])
        .expect("parse setup model-index show --embeddings");
        assert!(matches!(
            embedding_index.command,
            Some(Commands::Setup(ref setup))
                if matches!(setup.command, SetupSubcommand::ModelIndex(_))
        ));

        let tool_list = Cli::try_parse_from(["coat", "tool", "list"]).expect("parse tool list");
        assert!(matches!(
            tool_list.command,
            Some(Commands::Tool(ref tool)) if matches!(tool.command, ToolSubcommand::List(_))
        ));

        let tool_web_search = Cli::try_parse_from([
            "coat",
            "tool",
            "web-search",
            "--file",
            "examples/web-search-request.json",
        ])
        .expect("parse tool web-search");
        assert!(matches!(
            tool_web_search.command,
            Some(Commands::Tool(ref tool))
                if matches!(tool.command, ToolSubcommand::WebSearch(_))
        ));

        let tool_call = Cli::try_parse_from([
            "coat",
            "tool",
            "call",
            "--name",
            "subagent_policy",
            "--file",
            "examples/tool-subagent-policy-request.json",
        ])
        .expect("parse tool call");
        assert!(matches!(
            tool_call.command,
            Some(Commands::Tool(ref tool)) if matches!(tool.command, ToolSubcommand::Call(_))
        ));
    }

    #[test]
    fn web_search_example_is_a_valid_structured_tool_request() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/web-search-request.json");
        let request: WebSearchRequest =
            read_json_file(&path).expect("example web search request parses");

        assert!(
            request.query.contains("OpenAI Agents SDK"),
            "example should be a real research query"
        );
        assert_eq!(request.limit, Some(5));
        assert!(request.require_sources.unwrap_or(false));
        assert!(request.require_use_plan.unwrap_or(false));
        assert_eq!(request.allowed_providers.len(), 3);
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
    fn compose_preflight_accepts_shared_llm_gateway_for_work_and_research() {
        let values = BTreeMap::from([
            ("MODEL_PROVIDER_RUNNER_MODE".to_string(), "live".to_string()),
            (
                "MODEL_PROVIDER_KIND".to_string(),
                "open_ai_compatible".to_string(),
            ),
            (
                "MODEL_PROVIDER_RESEARCH_RUNNER_MODE".to_string(),
                "live".to_string(),
            ),
            (
                "MODEL_PROVIDER_RESEARCH_KIND".to_string(),
                "open_ai_compatible".to_string(),
            ),
            (
                "COAT_LLM_GATEWAY_URL".to_string(),
                "http://host.docker.internal:8080/openai".to_string(),
            ),
            (
                "COAT_LLM_GATEWAY_WORK_MODEL".to_string(),
                "openai/work-model".to_string(),
            ),
            (
                "COAT_LLM_GATEWAY_RESEARCH_MODEL".to_string(),
                "anthropic/research-model".to_string(),
            ),
        ]);
        let (_warnings, failures) =
            compose_model_preflight_findings(true, &[PathBuf::from("env")], &values, false, true);

        assert!(
            failures.is_empty(),
            "shared gateway model and endpoint refs should satisfy live model-provider lanes: {failures:?}"
        );
    }

    #[test]
    fn compose_preflight_keeps_control_chat_separate_from_runner_models() {
        let values = BTreeMap::from([
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
                "llama3.2".to_string(),
            ),
            (
                "LOCAL_MODEL_PROVIDER_ENDPOINT".to_string(),
                "http://host.docker.internal:11434/v1".to_string(),
            ),
            (
                "COAT_CONTROL_CHAT_MODEL".to_string(),
                "llama3.2".to_string(),
            ),
        ]);
        let (_warnings, failures) =
            compose_model_preflight_findings(true, &[PathBuf::from("env")], &values, false, true);

        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("control gateway chat has a model")),
            "control chat model without a gateway/provider must not fall through to local runners: {failures:?}"
        );
    }

    #[test]
    fn compose_preflight_accepts_direct_openai_control_chat() {
        let values = BTreeMap::from([
            (
                "COAT_CONTROL_CHAT_BACKEND".to_string(),
                "configured".to_string(),
            ),
            (
                "COAT_CONTROL_CHAT_PROVIDER".to_string(),
                "openai".to_string(),
            ),
            (
                "COAT_CONTROL_CHAT_MODEL".to_string(),
                "gpt-example".to_string(),
            ),
            ("OPENAI_API_KEY".to_string(), "set".to_string()),
        ]);
        let (_warnings, failures) =
            compose_model_preflight_findings(true, &[PathBuf::from("env")], &values, false, true);

        assert!(
            failures.is_empty(),
            "direct OpenAI chat config should satisfy control gateway chat preflight: {failures:?}"
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
                "served-model".to_string(),
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
    fn compose_preflight_accepts_local_model_reused_for_primary_and_research_lanes() {
        let values = BTreeMap::from([
            ("MODEL_PROVIDER_RUNNER_MODE".to_string(), "live".to_string()),
            ("MODEL_PROVIDER_KIND".to_string(), "ollama".to_string()),
            ("MODEL_PROVIDER_AUTH_MODE".to_string(), "none".to_string()),
            (
                "MODEL_PROVIDER_MODEL".to_string(),
                "served-model".to_string(),
            ),
            (
                "MODEL_PROVIDER_ENDPOINT".to_string(),
                "http://host.docker.internal:11434/v1".to_string(),
            ),
            (
                "MODEL_PROVIDER_RESEARCH_RUNNER_MODE".to_string(),
                "live".to_string(),
            ),
            (
                "MODEL_PROVIDER_RESEARCH_KIND".to_string(),
                "ollama".to_string(),
            ),
            (
                "MODEL_PROVIDER_RESEARCH_AUTH_MODE".to_string(),
                "none".to_string(),
            ),
            (
                "MODEL_PROVIDER_RESEARCH_MODEL".to_string(),
                "served-model".to_string(),
            ),
            (
                "MODEL_PROVIDER_RESEARCH_ENDPOINT".to_string(),
                "http://host.docker.internal:11434/v1".to_string(),
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
                "served-model".to_string(),
            ),
            (
                "LOCAL_MODEL_PROVIDER_ENDPOINT".to_string(),
                "http://host.docker.internal:11434/v1".to_string(),
            ),
        ]);
        let (warnings, failures) =
            compose_model_preflight_findings(true, &[PathBuf::from("env")], &values, false, true);

        assert!(
            failures.is_empty(),
            "local model reuse should satisfy all model-provider lanes: {failures:?}"
        );
        assert!(
            !warnings
                .iter()
                .any(|warning| warning.contains("model-provider")),
            "all model-provider lanes are live, so preflight should not report model-provider stubs: {warnings:?}"
        );
    }

    #[test]
    fn compose_preflight_validates_memory_store_embedding_pairs() {
        let values = BTreeMap::from([
            (
                "MEMORY_GATEWAY_EMBEDDING_URL".to_string(),
                "http://host.docker.internal:11434/v1/embeddings".to_string(),
            ),
            (
                "MEMORY_GATEWAY_QDRANT_URL".to_string(),
                "http://qdrant:6333".to_string(),
            ),
        ]);
        let (warnings, failures) =
            compose_model_preflight_findings(true, &[PathBuf::from("env")], &values, false, true);

        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("MEMORY_GATEWAY_EMBEDDING_MODEL")),
            "embedding endpoint without model should fail preflight: {failures:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("vector memory will stay inactive")),
            "Qdrant without a complete embedding pair should warn: {warnings:?}"
        );

        let values = BTreeMap::from([
            (
                "MEMORY_GATEWAY_EMBEDDING_URL".to_string(),
                "http://host.docker.internal:11434/v1/embeddings".to_string(),
            ),
            (
                "MEMORY_GATEWAY_EMBEDDING_MODEL".to_string(),
                "nomic-embed-text".to_string(),
            ),
            (
                "MEMORY_GATEWAY_QDRANT_URL".to_string(),
                "http://qdrant:6333".to_string(),
            ),
        ]);
        let (warnings, failures) =
            compose_model_preflight_findings(true, &[PathBuf::from("env")], &values, false, true);

        assert!(
            failures.is_empty(),
            "complete local embedding config should pass memory preflight: {failures:?}"
        );
        assert!(
            !warnings
                .iter()
                .any(|warning| warning.contains("vector memory will stay inactive")),
            "Qdrant plus embedding pair should not warn inactive vector memory: {warnings:?}"
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
            LOCAL_MODEL_PROVIDER_MODEL=served-model
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

        let appended =
            replace_env_line(env_text.to_string(), "MODEL_PROVIDER_LATENCY_CLASS", "fast");
        assert!(appended.contains("\nMODEL_PROVIDER_LATENCY_CLASS=fast\n"));
    }

    #[test]
    fn local_model_provider_presets_cover_supported_interactive_choices() {
        let labels = local_model_provider_preset_labels();
        assert!(
            labels
                .iter()
                .any(|label| label.contains("Ollama on host.docker.internal:11434")),
            "local provider selection should include Ollama: {labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("vLLM OpenAI-compatible")),
            "local provider selection should include vLLM: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains("llama.cpp")),
            "local provider selection should include llama.cpp: {labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("Custom OpenAI-compatible")),
            "local provider selection should include a custom endpoint: {labels:?}"
        );

        assert_eq!(
            local_model_provider_preset(default_local_model_provider_index(false)).kind,
            "ollama"
        );
        assert_eq!(
            local_model_provider_preset(default_local_model_provider_index(true)).kind,
            "vllm"
        );
        assert_eq!(local_model_provider_preset(usize::MAX).kind, "ollama");
    }

    #[test]
    fn local_auth_setup_defaults_from_existing_env_file_state() {
        let values = parse_env_file_content(
            r#"
            CODEX_RUNNER_MODE=live
            CODEX_AUTH_MODE=app_server
            CODEX_APP_SERVER_URL=http://host.docker.internal:1455
            MODEL_PROVIDER_LOCAL_RUNNER_MODE=live
            LOCAL_MODEL_PROVIDER_KIND=ollama
            LOCAL_MODEL_PROVIDER_MODEL=llama3.2
            LOCAL_MODEL_PROVIDER_ENDPOINT=http://host.docker.internal:11434/v1
            MODEL_PROVIDER_RUNNER_MODE=live
            MODEL_PROVIDER_KIND=ollama
            MODEL_PROVIDER_MODEL=llama3.2
            COAT_CONTROL_CHAT_MODEL=llama3.2
            MEMORY_GATEWAY_QDRANT_URL=http://qdrant:6333
            MEMORY_GATEWAY_EMBEDDING_URL=http://host.docker.internal:11434/v1/embeddings
            MEMORY_GATEWAY_EMBEDDING_MODEL=nomic-embed-text
            COAT_WEB_SEARCH_ENABLED=true
            COAT_WEB_SEARCH_ROUTE=coordinator_task
            "#,
        );

        let defaults = local_auth_profile_defaults(true, &values);
        assert!(
            defaults[0],
            "existing live Codex lane should be selected by default"
        );
        assert!(
            defaults[5],
            "existing Ollama lane should be selected by default"
        );
        assert!(
            defaults[8],
            "existing chat model should be selected by default"
        );
        assert!(
            defaults[9],
            "existing memory store or embedding config should be selected by default"
        );
        assert!(
            defaults[10],
            "existing routed web search config should be selected by default"
        );
        assert!(
            !defaults[3],
            "OpenAI hosted lane should not be selected when the existing env uses local Ollama"
        );

        let params = model_param_values_from_env(&values, "LOCAL_MODEL_PROVIDER");
        assert!(params.latency_class.is_none());

        let values = parse_env_file_content(
            r#"
            LOCAL_MODEL_PROVIDER_LATENCY_CLASS=fast
            LOCAL_MODEL_PROVIDER_TEMPERATURE=0.2
            LOCAL_MODEL_PROVIDER_TOP_P=0.9
            LOCAL_MODEL_PROVIDER_MAX_OUTPUT_TOKENS=2048
            LOCAL_MODEL_PROVIDER_REASONING_EFFORT=low
            LOCAL_MODEL_PROVIDER_TIMEOUT_SECONDS=60
            "#,
        );
        let params = model_param_values_from_env(&values, "LOCAL_MODEL_PROVIDER");
        assert_eq!(
            params,
            model_param_values_from_preset(MODEL_PARAM_PRESETS[0])
        );
    }

    #[test]
    fn model_presets_come_from_real_index_or_live_discovery() {
        let index: ModelsDevIndex = serde_json::from_str(
            r#"{
              "openai": {
                "id": "openai",
                "name": "OpenAI",
                "models": {
                  "gpt-real": {
                    "id": "gpt-real",
                    "name": "GPT Real",
                    "family": "gpt",
                    "tool_call": true,
                    "structured_output": true,
                    "last_updated": "2026-05-01",
                    "limit": {"context": 128000, "output": 16384}
                  },
                  "old-model": {
                    "id": "old-model",
                    "name": "Old Model",
                    "status": "deprecated"
                  },
                  "text-embedding-real": {
                    "id": "text-embedding-real",
                    "name": "Text Embedding Real",
                    "last_updated": "2026-05-02",
                    "limit": {"context": 8192, "output": 1536},
                    "modalities": {"input": ["text"], "output": ["text"]}
                  }
                }
              },
              "amazon-bedrock": {
                "id": "amazon-bedrock",
                "name": "Amazon Bedrock",
                "models": {
                  "amazon.real-pro-v1:0": {
                    "id": "amazon.real-pro-v1:0",
                    "name": "Real Pro",
                    "reasoning": true,
                    "last_updated": "2026-04-15"
                  }
                }
              }
            }"#,
        )
        .expect("sample models.dev index parses");

        let openai_presets =
            models_dev_provider_presets(&index, "openai", "Custom OpenAI chat model id", 20);
        let openai_labels = model_preset_labels(&openai_presets);
        assert!(
            openai_labels
                .iter()
                .any(|label| label.contains("OpenAI: GPT Real")),
            "OpenAI choices should be rendered from the model index: {openai_labels:?}"
        );
        assert!(
            openai_presets
                .iter()
                .any(|preset| preset.model == "gpt-real"),
            "OpenAI choices should include model ids from the model index"
        );
        assert!(
            !openai_presets
                .iter()
                .any(|preset| preset.model == "old-model"),
            "deprecated models should not be offered by default"
        );
        assert!(
            !openai_presets
                .iter()
                .any(|preset| preset.model == "text-embedding-real"),
            "embedding models should not be offered as general work-model presets"
        );
        assert!(
            openai_presets
                .iter()
                .any(|preset| preset.model == CUSTOM_MODEL_ID),
            "indexed model choices should still include a custom escape hatch"
        );

        let bedrock_presets =
            models_dev_provider_presets(&index, "amazon-bedrock", "Custom Bedrock model id", 20);
        assert!(
            bedrock_presets
                .iter()
                .any(|preset| preset.model == "amazon.real-pro-v1:0"),
            "Bedrock choices should come from the model index"
        );

        let live_presets = live_model_presets(
            vec![
                "served-b".to_string(),
                "served-a".to_string(),
                "served-a".to_string(),
            ],
            "Custom local model id",
        );
        assert_eq!(live_presets[0].model, "served-a");
        assert_eq!(live_presets[1].model, "served-b");
        assert_eq!(model_preset(&live_presets, usize::MAX).model, "served-a");
        assert_eq!(
            model_preset(
                &openai_presets,
                default_model_preset_index(&openai_presets, "gpt-real")
            )
            .model,
            "gpt-real"
        );

        let embedding_presets =
            models_dev_embedding_presets(&index, "openai", "Custom OpenAI embedding model id", 20);
        assert!(
            embedding_presets
                .iter()
                .any(|preset| preset.model == "text-embedding-real"),
            "embedding choices should come from the model index: {embedding_presets:?}"
        );
        assert!(
            !embedding_presets
                .iter()
                .any(|preset| preset.model == "gpt-real"),
            "chat-only model ids should not be offered as hosted embedding presets"
        );
        assert_eq!(
            models_dev_embedding_dimensions(&index, "openai", "text-embedding-real"),
            Some(1536)
        );
        assert_eq!(
            openai_embeddings_url("http://host.docker.internal:11434/v1"),
            "http://host.docker.internal:11434/v1/embeddings"
        );
        assert_eq!(
            openai_embeddings_url("http://host.docker.internal:11434/v1/chat/completions"),
            "http://host.docker.internal:11434/v1/embeddings"
        );
    }

    #[test]
    fn setup_model_choices_keep_configured_model_when_discovery_is_empty() {
        let presets = model_presets_with_configured(
            live_model_presets(Vec::new(), "Custom local model id"),
            Some("llama3.2"),
            "Configured local model",
        );

        assert_eq!(presets[0].model, "llama3.2");
        assert!(
            presets[0].label.contains("Configured local model"),
            "configured model should be visible as a first-class option: {presets:?}"
        );
        assert!(
            presets.iter().any(|preset| preset.model == CUSTOM_MODEL_ID),
            "custom escape hatch should remain present"
        );

        let candidates = endpoint_discovery_candidates("http://host.docker.internal:11434/v1");
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate == "http://localhost:11434/v1"),
            "CLI discovery should try a host-local alias for Compose endpoints: {candidates:?}"
        );
    }

    #[test]
    fn model_param_presets_apply_fast_and_custom_runtime_env() {
        let labels = model_param_preset_labels();
        assert!(
            labels.iter().any(|label| label.contains("Fast")),
            "runtime param presets should expose a fast lane: {labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("Fast completions")),
            "runtime param presets should expose a fast completions lane: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains("Speed tier")),
            "runtime param presets should expose a provider speed tier lane: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains("Deep review")),
            "runtime param presets should expose a deep review lane: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label.contains("XHigh")),
            "runtime param presets should expose an xhigh reasoning lane: {labels:?}"
        );

        let fast = model_param_preset(default_model_param_preset_index("fast"));
        assert_eq!(fast.latency_class, Some("fast"));
        let xhigh = MODEL_PARAM_PRESETS
            .iter()
            .copied()
            .find(|preset| preset.reasoning_effort == Some("xhigh"))
            .expect("xhigh preset exists");
        assert_eq!(xhigh.latency_class, Some("deep"));
        assert_eq!(xhigh.max_output_tokens, Some("16384"));
        let speed = MODEL_PARAM_PRESETS
            .iter()
            .copied()
            .find(|preset| preset.speed_tier == Some("speed"))
            .expect("speed-tier preset exists");
        assert_eq!(speed.latency_class, Some("fast"));
        assert_eq!(
            model_param_preset(usize::MAX).reasoning_effort,
            Some("medium"),
            "out-of-range runtime preset should fall back to balanced"
        );

        let params = model_param_values_from_preset(fast);
        let env_text = "MODEL_PROVIDER_TEMPERATURE=\nMODEL_PROVIDER_TIMEOUT_SECONDS=\n";
        let updated = apply_model_param_values(env_text.to_string(), "MODEL_PROVIDER", &params);
        let parsed = parse_env_file_content(&updated);

        assert_eq!(
            parsed
                .get("MODEL_PROVIDER_LATENCY_CLASS")
                .map(String::as_str),
            Some("fast")
        );
        assert_eq!(
            parsed
                .get("MODEL_PROVIDER_SPEED_TIER")
                .map(|value| value.trim())
                .unwrap_or_default(),
            "",
            "plain fast preset should not force provider speed tiering"
        );
        assert_eq!(
            parsed.get("MODEL_PROVIDER_TEMPERATURE").map(String::as_str),
            Some("0.2")
        );
        assert_eq!(
            parsed
                .get("MODEL_PROVIDER_MAX_OUTPUT_TOKENS")
                .map(String::as_str),
            Some("2048")
        );
        assert_eq!(
            parsed
                .get("MODEL_PROVIDER_REASONING_EFFORT")
                .map(String::as_str),
            Some("low")
        );
        assert_eq!(
            parsed
                .get("MODEL_PROVIDER_TIMEOUT_SECONDS")
                .map(String::as_str),
            Some("60")
        );

        let speed_updated = apply_model_param_values(
            String::new(),
            "MODEL_PROVIDER",
            &model_param_values_from_preset(speed),
        );
        let speed_parsed = parse_env_file_content(&speed_updated);
        assert_eq!(
            speed_parsed
                .get("MODEL_PROVIDER_SPEED_TIER")
                .map(String::as_str),
            Some("speed")
        );
    }

    #[test]
    fn model_index_cache_freshness_debounces_setup_refresh() {
        let path = std::env::temp_dir().join(format!(
            "coat-model-index-freshness-{}.json",
            uuid::Uuid::new_v4()
        ));
        fs::write(
            &path,
            r#"{"openai":{"id":"openai","name":"OpenAI","models":{"gpt-real":{"id":"gpt-real"}}}}"#,
        )
        .expect("write temp model index");

        assert!(
            model_index_cache_is_fresh(&path, Duration::from_secs(60 * 60)),
            "newly written model indexes should debounce setup refresh"
        );
        let (loaded_path, loaded_index) =
            load_fresh_model_index_from_paths(vec![path.clone()], Duration::from_secs(60 * 60))
                .expect("fresh model index should parse")
                .expect("fresh model index should load");
        assert_eq!(loaded_path, path);
        assert!(
            loaded_index.providers.contains_key("openai"),
            "fresh debounced cache should be the model index used by setup"
        );
        fs::remove_file(&path).expect("remove temp model index");
        assert!(
            !model_index_cache_is_fresh(&path, Duration::from_secs(60 * 60)),
            "missing model indexes cannot satisfy setup refresh debounce"
        );
        assert!(
            load_fresh_model_index_from_paths(vec![path], Duration::from_secs(60 * 60))
                .expect("missing model index should not error")
                .is_none(),
            "missing model indexes should force a refresh attempt before setup options"
        );
    }

    #[test]
    fn login_actions_are_coat_owned_provider_commands() {
        let args = LoginArgs {
            codex: true,
            claude: true,
            claude_email: Some("engineer@example.com".to_string()),
            claude_sso: true,
            claude_console: false,
            hf: true,
            ollama_model: vec!["served-model".to_string()],
            env_file: PathBuf::from("infra/compose/local-providers.env"),
            preflight: true,
            allow_stub_runners: false,
            dry_run: true,
        };
        let actions = login_actions_from_args(&args);
        assert_eq!(
            actions,
            vec![
                LocalAuthAction::CodexLogin,
                LocalAuthAction::ClaudeLogin {
                    email: Some("engineer@example.com".to_string()),
                    sso: true,
                    console: false,
                },
                LocalAuthAction::HuggingFaceLogin,
                LocalAuthAction::OllamaPull {
                    model: "served-model".to_string()
                },
            ]
        );

        let (program, command_args, description) =
            local_auth_action_command(&LocalAuthAction::ClaudeLogin {
                email: Some("engineer@example.com".to_string()),
                sso: true,
                console: false,
            });
        assert_eq!(program, "claude");
        assert_eq!(
            command_args,
            ["auth", "login", "--email", "engineer@example.com", "--sso"].map(String::from)
        );
        assert!(description.contains("auth login"));

        let (program, command_args, description) =
            local_auth_action_command(&LocalAuthAction::AwsSso {
                profile: "jattg-dev".to_string(),
            });
        assert_eq!(program, "aws");
        assert_eq!(
            command_args,
            ["sso", "login", "--profile", "jattg-dev"].map(String::from)
        );
        assert!(description.contains("jattg-dev"));
    }

    #[test]
    fn coat_config_merge_preserves_project_defaults_and_user_overrides() {
        let mut base = CoatConfig {
            service_endpoints: CoatServiceEndpoints {
                goal_store_url: Some("http://localhost:9088".to_string()),
                tool_registry_url: Some("http://localhost:9084".to_string()),
                ..CoatServiceEndpoints::default()
            },
            local_deploy: CoatLocalDeployConfig {
                env_files: vec!["infra/compose/local-providers.env".to_string()],
                allow_stub_runners: Some(false),
                ..CoatLocalDeployConfig::default()
            },
            runner_capacity: CoatRunnerCapacityConfig {
                default_policy: Some(CapacityScalingPolicy::recommend_only(2)),
                lane_policies: BTreeMap::from([(
                    "research".to_string(),
                    CapacityScalingPolicy::recommend_only(3),
                )]),
            },
            ..CoatConfig::default()
        };
        let overlay = CoatConfig {
            service_endpoints: CoatServiceEndpoints {
                goal_store_url: Some("http://remote-goal-store:9088".to_string()),
                tool_registry_url: Some("http://remote-tool-registry:9084".to_string()),
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
            runner_capacity: CoatRunnerCapacityConfig {
                default_policy: Some(CapacityScalingPolicy::bounded_ephemeral(5)),
                lane_policies: BTreeMap::from([(
                    "review".to_string(),
                    CapacityScalingPolicy::recommend_only(1),
                )]),
            },
            ..CoatConfig::default()
        };

        merge_coat_config(&mut base, overlay);

        assert_eq!(
            base.service_endpoints.goal_store_url.as_deref(),
            Some("http://remote-goal-store:9088")
        );
        assert_eq!(
            base.service_endpoints.tool_registry_url.as_deref(),
            Some("http://remote-tool-registry:9084")
        );
        assert_eq!(
            base.local_deploy.env_files,
            vec![
                "infra/compose/local-providers.env".to_string(),
                "~/.coat/local-providers.env".to_string()
            ]
        );
        assert_eq!(base.local_deploy.allow_stub_runners, Some(true));
        assert_eq!(
            base.runner_capacity.default_policy,
            Some(CapacityScalingPolicy::bounded_ephemeral(5))
        );
        assert_eq!(
            base.runner_capacity.policy_for("research"),
            Some(CapacityScalingPolicy::recommend_only(3))
        );
        assert_eq!(
            base.runner_capacity.policy_for("review"),
            Some(CapacityScalingPolicy::recommend_only(1))
        );
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
        assert_eq!(
            restate_cloud.runner_capacity.default_policy,
            Some(CapacityScalingPolicy::recommend_only(4))
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
        assert_eq!(
            eks.runner_capacity.default_policy.as_ref().map(|policy| (
                policy.enabled,
                policy.mode.clone(),
                policy.max_runners,
                policy.headroom_runners,
                policy.max_scale_up_step,
            )),
            Some((
                true,
                coat_domain::CapacityScalingMode::RecommendOnly,
                24,
                1,
                4,
            ))
        );
    }

    #[test]
    fn capacity_plan_uses_resolved_config_policy_when_request_policy_is_default() {
        let mut request = RunnerScalingRequest {
            generated_at_unix_seconds: 0,
            policy: CapacityScalingPolicy::default(),
            demands: vec![RunnerPoolDemand {
                pool_key: "research".to_string(),
                worker: None,
                required_capabilities: Vec::new(),
                required_labels: BTreeMap::new(),
                queued_tasks: 5,
                running_tasks: 0,
                blocked_tasks: 0,
                unmatched_tasks: 1,
                event_backlog: 2,
                priority_boost: 0,
            }],
            supplies: Vec::new(),
        };
        let config = CoatConfig {
            runner_capacity: CoatRunnerCapacityConfig {
                default_policy: Some(CapacityScalingPolicy::recommend_only(2)),
                lane_policies: BTreeMap::from([(
                    "research".to_string(),
                    CapacityScalingPolicy::bounded_ephemeral(7),
                )]),
            },
            ..CoatConfig::default()
        };

        apply_capacity_plan_policy_from_config(&mut request, false, &config);

        assert_eq!(request.policy, CapacityScalingPolicy::bounded_ephemeral(7));

        apply_capacity_plan_policy_from_config(&mut request, false, &config);
        assert_eq!(
            request.policy,
            CapacityScalingPolicy::bounded_ephemeral(7),
            "explicit or already-filled request policy should not be overwritten"
        );

        let mut ignored = RunnerScalingRequest {
            policy: CapacityScalingPolicy::default(),
            ..request
        };
        apply_capacity_plan_policy_from_config(&mut ignored, true, &config);
        assert_eq!(ignored.policy, CapacityScalingPolicy::default());
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
