use std::{fs, path::PathBuf, process::Command};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use coat_domain::{
    BranchRequest, BranchSelectionRequest, EventSource, ExternalEvent, GoalSpec, HumanApproval,
    MemoryContextRequest, MemoryJoinRequest, MemoryRepairRequest, MemorySearchRequest,
    MemoryWriteRequest, NotificationRequest, RestartRequest, RunnerDispatchRequest,
    RunnerRegistration, SteeringDirective, TaskPurposeKind, TaskQuery, TaskStatus,
    TriggeredGoalRequest, WorkerKind,
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
    Goal(GoalCommand),
    Event(EventCommand),
    Runner(RunnerCommand),
    Memory(MemoryCommand),
    Approve(ApproveArgs),
    Notify(NotifyArgs),
    Store(StoreCommand),
    Sandbox(SandboxCommand),
    Compose(ComposeCommand),
    K8s(K8sCommand),
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long, default_value = ".")]
    path: PathBuf,
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
    Ingest(EventFileArgs),
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

#[derive(Debug, Subcommand)]
enum RunnerSubcommand {
    List(RunnerListArgs),
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
    Submit(SubmitGoalArgs),
    Status(GoalIdArgs),
    Progress(GoalIdArgs),
    Tasks(GoalTasksArgs),
    Lint(GoalLintArgs),
    Steer(SteerGoalArgs),
    Restart(RestartGoalArgs),
    Branch(BranchGoalArgs),
    SelectBranch(SelectBranchArgs),
    Cancel(CancelGoalArgs),
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
    #[arg(long)]
    goal_id: Uuid,
}

#[derive(Debug, Args)]
struct GoalTasksArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[arg(long)]
    goal_id: Uuid,
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
    #[arg(long)]
    goal_id: Uuid,
    #[arg(long)]
    file: PathBuf,
}

#[derive(Debug, Args)]
struct RestartGoalArgs {
    #[arg(
        long,
        env = "COAT_RESTATE_INGRESS",
        default_value = "http://localhost:8080"
    )]
    restate_ingress: String,
    #[arg(long)]
    goal_id: Uuid,
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
    #[arg(long)]
    goal_id: Uuid,
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
    #[arg(long)]
    goal_id: Uuid,
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
    #[arg(long)]
    goal_id: Uuid,
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
    #[arg(long)]
    goal_id: Uuid,
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
    Goal(StoreGoalArgs),
    Tasks(StoreGoalArgs),
    Events(StoreGoalArgs),
    Artifacts(StoreGoalArgs),
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
struct ComposeCommand {
    #[command(subcommand)]
    command: ComposeSubcommand,
}

#[derive(Debug, Subcommand)]
enum ComposeSubcommand {
    Up,
    Down,
}

#[derive(Debug, Args)]
struct K8sCommand {
    #[command(subcommand)]
    command: K8sSubcommand,
}

#[derive(Debug, Subcommand)]
enum K8sSubcommand {
    Render(RenderArgs),
}

#[derive(Debug, Args)]
struct RenderArgs {
    #[arg(long, default_value = "infra/k8s/rendered.yaml")]
    output: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Commands::Init(args) => init(args),
        Commands::Goal(args) => goal(args).await,
        Commands::Event(args) => event(args).await,
        Commands::Runner(args) => runner(args).await,
        Commands::Memory(args) => memory(args).await,
        Commands::Approve(args) => approve(args).await,
        Commands::Notify(args) => notify(args).await,
        Commands::Store(args) => store(args).await,
        Commands::Sandbox(args) => sandbox(args).await,
        Commands::Compose(args) => compose(args),
        Commands::K8s(args) => k8s(args),
    }
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
    }
}

async fn sandbox(args: SandboxCommand) -> anyhow::Result<()> {
    match args.command {
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
            post_json_to_url(
                &format!("{}/events", args.event_gateway_url.trim_end_matches('/')),
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
        GoalSubcommand::Submit(args) => submit_goal(args).await,
        GoalSubcommand::Status(args) => {
            restate_post_without_body(&args.restate_ingress, args.goal_id, "status").await
        }
        GoalSubcommand::Progress(args) => {
            restate_post_without_body(&args.restate_ingress, args.goal_id, "progress").await
        }
        GoalSubcommand::Tasks(args) => {
            let query = task_query_from_args(&args)?;
            restate_post_json(&args.restate_ingress, args.goal_id, "tasks", &query).await
        }
        GoalSubcommand::Lint(args) => lint_goal(args),
        GoalSubcommand::Steer(args) => {
            let directive: SteeringDirective = read_json_file(&args.file)?;
            restate_post_json(&args.restate_ingress, args.goal_id, "steer", &directive).await
        }
        GoalSubcommand::Restart(args) => {
            let request: RestartRequest = read_json_file(&args.file)?;
            restate_post_json(&args.restate_ingress, args.goal_id, "restart", &request).await
        }
        GoalSubcommand::Branch(args) => {
            let request: BranchRequest = read_json_file(&args.file)?;
            restate_post_json(&args.restate_ingress, args.goal_id, "branch", &request).await
        }
        GoalSubcommand::SelectBranch(args) => {
            let request: BranchSelectionRequest = read_json_file(&args.file)?;
            restate_post_json(&args.restate_ingress, args.goal_id, "select_branch", &request).await
        }
        GoalSubcommand::Cancel(args) => {
            restate_post_json(&args.restate_ingress, args.goal_id, "cancel", &args.reason).await
        }
    }
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
    println!("{body}");
    Ok(())
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
    query.tags.extend(args.tag.clone());
    if args.runnable {
        query.runnable_only = true;
    }
    if args.limit.is_some() {
        query.limit = args.limit;
    }
    Ok(query)
}

fn parse_json_enum<T: serde::de::DeserializeOwned>(
    value: &str,
    type_name: &str,
) -> anyhow::Result<T> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .with_context(|| format!("parse {type_name} value '{value}'"))
}

async fn approve(args: ApproveArgs) -> anyhow::Result<()> {
    let approval = HumanApproval {
        approval_id: args.approval_id,
        approved: args.approved,
        note: args.note,
    };
    restate_post_json(&args.restate_ingress, args.goal_id, "approve", &approval).await
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
    command
        .arg("compose")
        .arg("-f")
        .arg("infra/compose/docker-compose.yml");
    match args.command {
        ComposeSubcommand::Up => {
            command.arg("up").arg("--build");
        }
        ComposeSubcommand::Down => {
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
    }
}
