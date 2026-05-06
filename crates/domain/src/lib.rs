use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub type GoalId = Uuid;
pub type TaskId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalSpec {
    pub id: GoalId,
    pub title: String,
    pub objective: String,
    pub repo: Option<String>,
    pub root_budget: Budget,
    pub done_criteria: DoneCriteria,
    #[serde(default)]
    pub default_execution: ExecutionProfile,
    #[serde(default)]
    pub initial_tasks: Vec<ChildTaskRequest>,
}

impl GoalSpec {
    pub fn new(title: impl Into<String>, objective: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            objective: objective.into(),
            repo: None,
            root_budget: Budget::default_goal(),
            done_criteria: DoneCriteria::default(),
            default_execution: ExecutionProfile::default(),
            initial_tasks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalState {
    pub goal: GoalSpec,
    pub tasks: BTreeMap<TaskId, TaskNode>,
    pub status: GoalStatus,
    #[serde(default)]
    pub approvals: Vec<ApprovalRequest>,
    #[serde(default)]
    pub final_artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub events: Vec<StateEvent>,
}

impl GoalState {
    pub fn new(goal: GoalSpec) -> Self {
        let root_id = Uuid::new_v4();
        let root = TaskNode {
            id: root_id,
            parent_id: None,
            goal_id: goal.id,
            depth: 0,
            status: TaskStatus::Runnable,
            role: WorkerKind::Planner,
            execution: goal
                .default_execution
                .clone()
                .with_role(WorkerKind::Planner),
            prompt: goal.objective.clone(),
            dependencies: Vec::new(),
            children: Vec::new(),
            budget: goal.root_budget.clone(),
            sandbox: SandboxProfile::default(),
            done_criteria: goal.done_criteria.clone(),
            result: None,
            attempts: 0,
        };

        let mut state = Self {
            goal,
            tasks: BTreeMap::from([(root_id, root)]),
            status: GoalStatus::Running,
            approvals: Vec::new(),
            final_artifacts: Vec::new(),
            events: Vec::new(),
        };
        state.events.push(StateEvent::new("goal_started"));
        state
    }

    pub fn runnable_tasks(&self) -> Vec<TaskNode> {
        self.tasks
            .values()
            .filter(|task| {
                task.status == TaskStatus::Runnable
                    && task.dependencies.iter().all(|id| {
                        self.tasks
                            .get(id)
                            .is_some_and(|dep| dep.status.is_terminal_ok())
                    })
            })
            .cloned()
            .collect()
    }

    pub fn is_done(&self) -> bool {
        self.status == GoalStatus::Done
            || (!self.tasks.is_empty()
                && self.tasks.values().all(|task| {
                    task.status.is_terminal_ok() || task.status == TaskStatus::Cancelled
                }))
    }

    pub fn budget_exhausted(&self) -> bool {
        self.tasks.values().all(|task| task.budget.is_exhausted())
    }

    pub fn mark_running(&mut self, task_id: TaskId) -> Result<(), DomainError> {
        let task = self.task_mut(task_id)?;
        task.status = TaskStatus::Running;
        task.attempts += 1;
        self.events
            .push(StateEvent::new(format!("task_running:{task_id}")));
        Ok(())
    }

    pub fn apply_agent_result(
        &mut self,
        result: AgentRunResult,
        policy: &SpawnPolicy,
    ) -> Result<(), DomainError> {
        let task = self.task_mut(result.task_id)?;
        task.result = result.artifacts.first().cloned();
        task.status = match result.status {
            WorkerRunStatus::Done => TaskStatus::NeedsValidation,
            WorkerRunStatus::Partial => TaskStatus::Runnable,
            WorkerRunStatus::Blocked => TaskStatus::Blocked,
            WorkerRunStatus::Failed => TaskStatus::Failed,
        };

        if !result.child_requests.is_empty() {
            let parent_snapshot = self.task(result.task_id)?.clone();
            policy.ensure_spawn_allowed(&parent_snapshot, &result.child_requests)?;
            for child in result.child_requests {
                let child_id = Uuid::new_v4();
                self.tasks
                    .get_mut(&result.task_id)
                    .ok_or(DomainError::TaskNotFound(result.task_id))?
                    .children
                    .push(child_id);
                self.tasks.insert(
                    child_id,
                    TaskNode::from_child_request(child_id, result.task_id, &parent_snapshot, child),
                );
            }
        }

        self.events
            .push(StateEvent::new(format!("agent_result:{}", result.task_id)));
        Ok(())
    }

    pub fn apply_validation(&mut self, report: ValidationReport) -> Result<(), DomainError> {
        let task = self.task_mut(report.task_id)?;
        task.status = report.status_after_validation.clone();
        if report.passed {
            self.final_artifacts.extend(report.artifacts.clone());
        }
        if self.tasks.values().all(|task| task.status.is_terminal_ok()) {
            self.status = GoalStatus::Done;
        }
        if self
            .tasks
            .values()
            .any(|task| task.status == TaskStatus::Blocked)
        {
            self.status = GoalStatus::Blocked;
        }
        self.events
            .push(StateEvent::new(format!("validated:{}", report.task_id)));
        Ok(())
    }

    pub fn cancel(&mut self, reason: impl Into<String>) {
        self.status = GoalStatus::Cancelled;
        for task in self.tasks.values_mut() {
            if !task.status.is_terminal() {
                task.status = TaskStatus::Cancelled;
            }
        }
        self.events
            .push(StateEvent::new(format!("cancelled:{}", reason.into())));
    }

    fn task(&self, task_id: TaskId) -> Result<&TaskNode, DomainError> {
        self.tasks
            .get(&task_id)
            .ok_or(DomainError::TaskNotFound(task_id))
    }

    fn task_mut(&mut self, task_id: TaskId) -> Result<&mut TaskNode, DomainError> {
        self.tasks
            .get_mut(&task_id)
            .ok_or(DomainError::TaskNotFound(task_id))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Running,
    WaitingApproval,
    Done,
    Blocked,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TaskNode {
    pub id: TaskId,
    pub parent_id: Option<TaskId>,
    pub goal_id: GoalId,
    pub depth: u32,
    pub status: TaskStatus,
    pub role: WorkerKind,
    pub execution: ExecutionProfile,
    pub prompt: String,
    #[serde(default)]
    pub dependencies: Vec<TaskId>,
    #[serde(default)]
    pub children: Vec<TaskId>,
    pub budget: Budget,
    pub sandbox: SandboxProfile,
    pub done_criteria: DoneCriteria,
    pub result: Option<ArtifactRef>,
    pub attempts: u32,
}

impl TaskNode {
    fn from_child_request(
        id: TaskId,
        parent_id: TaskId,
        parent: &TaskNode,
        req: ChildTaskRequest,
    ) -> Self {
        let role = req.role;
        let execution = req
            .execution
            .unwrap_or_else(|| parent.execution.clone().with_role(role.clone()));

        Self {
            id,
            parent_id: Some(parent_id),
            goal_id: parent.goal_id,
            depth: parent.depth + 1,
            status: TaskStatus::Runnable,
            role,
            execution,
            prompt: req.prompt,
            dependencies: req.dependencies,
            children: Vec::new(),
            budget: req.budget.unwrap_or_else(|| parent.budget.child_budget()),
            sandbox: req.sandbox.unwrap_or_else(|| parent.sandbox.clone()),
            done_criteria: req
                .done_criteria
                .unwrap_or_else(|| parent.done_criteria.clone()),
            result: None,
            attempts: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Runnable,
    Running,
    NeedsValidation,
    WaitingApproval,
    Done,
    Blocked,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Blocked | Self::Failed | Self::Cancelled
        )
    }

    pub fn is_terminal_ok(&self) -> bool {
        matches!(self, Self::Done)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerKind {
    Planner,
    Codex,
    StaffEngineerClaude,
    Research,
    Reviewer,
    Tester,
    Validator,
    PatchMerger,
    RustTool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct Budget {
    pub max_tokens: u64,
    pub remaining_tokens: u64,
    pub max_runtime_seconds: u64,
    pub remaining_runtime_seconds: u64,
    pub max_tool_calls: u64,
    pub remaining_tool_calls: u64,
    pub max_child_tasks: u32,
    pub remaining_child_tasks: u32,
    pub max_patch_size: u64,
}

impl Budget {
    pub fn default_goal() -> Self {
        Self {
            max_tokens: 2_000_000,
            remaining_tokens: 2_000_000,
            max_runtime_seconds: 14_400,
            remaining_runtime_seconds: 14_400,
            max_tool_calls: 2_000,
            remaining_tool_calls: 2_000,
            max_child_tasks: 64,
            remaining_child_tasks: 64,
            max_patch_size: 500_000,
        }
    }

    pub fn child_budget(&self) -> Self {
        Self {
            max_tokens: self.max_tokens / 4,
            remaining_tokens: self.remaining_tokens / 4,
            max_runtime_seconds: self.max_runtime_seconds / 4,
            remaining_runtime_seconds: self.remaining_runtime_seconds / 4,
            max_tool_calls: self.max_tool_calls / 4,
            remaining_tool_calls: self.remaining_tool_calls / 4,
            max_child_tasks: self.max_child_tasks.min(8),
            remaining_child_tasks: self.remaining_child_tasks.min(8),
            max_patch_size: self.max_patch_size / 2,
        }
    }

    pub fn is_exhausted(&self) -> bool {
        self.remaining_tokens == 0
            || self.remaining_runtime_seconds == 0
            || self.remaining_tool_calls == 0
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::default_goal()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SandboxProfile {
    pub filesystem: FilesystemAccess,
    pub network: NetworkAccess,
    pub approval_policy: ApprovalPolicy,
    pub isolated_runner: bool,
}

impl Default for SandboxProfile {
    fn default() -> Self {
        Self {
            filesystem: FilesystemAccess::WorkspaceWrite,
            network: NetworkAccess::Restricted,
            approval_policy: ApprovalPolicy::OnRequest,
            isolated_runner: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    ReadOnly,
    WorkspaceWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAccess {
    Disabled,
    Restricted,
    Open,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Never,
    OnRequest,
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct DoneCriteria {
    pub tests_pass: bool,
    pub artifact_exists: bool,
    pub validator_score_min: Option<f32>,
}

impl Default for DoneCriteria {
    fn default() -> Self {
        Self {
            tests_pass: true,
            artifact_exists: true,
            validator_score_min: Some(0.85),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionProfile {
    pub runner: RunnerSelector,
    pub model: ModelRoute,
    pub persona: PersonaSpec,
    pub mcp: McpContextRef,
    pub notifications: NotificationPolicy,
}

impl ExecutionProfile {
    pub fn with_role(mut self, role: WorkerKind) -> Self {
        self.runner.worker = Some(role.clone());
        self.persona = self.persona.with_default_name_for_role(&role);
        self
    }
}

impl Default for ExecutionProfile {
    fn default() -> Self {
        Self {
            runner: RunnerSelector::default(),
            model: ModelRoute::default(),
            persona: PersonaSpec::default(),
            mcp: McpContextRef::default(),
            notifications: NotificationPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct RunnerSelector {
    pub worker: Option<WorkerKind>,
    pub runner_id: Option<String>,
    #[serde(default)]
    pub required_capabilities: Vec<RunnerCapability>,
    #[serde(default)]
    pub required_labels: BTreeMap<String, String>,
    pub locality: RunnerLocality,
}

impl RunnerSelector {
    pub fn matches(&self, registration: &RunnerRegistration) -> bool {
        if let Some(expected_runner) = &self.runner_id {
            if expected_runner != &registration.runner_id {
                return false;
            }
        }
        if let Some(worker) = &self.worker {
            if !registration.roles.contains(worker) {
                return false;
            }
        }
        if self
            .required_capabilities
            .iter()
            .any(|capability| !registration.capabilities.contains(capability))
        {
            return false;
        }
        self.required_labels.iter().all(|(key, value)| {
            registration
                .labels
                .get(key)
                .is_some_and(|actual| actual == value)
        })
    }
}

impl Default for RunnerSelector {
    fn default() -> Self {
        Self {
            worker: None,
            runner_id: None,
            required_capabilities: Vec::new(),
            required_labels: BTreeMap::new(),
            locality: RunnerLocality::AnyNode,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerLocality {
    AnyNode,
    SameNode,
    LocalOnly,
    RemoteOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerCapability {
    Code,
    Research,
    Test,
    Review,
    McpTools,
    WorkspaceSandbox,
    Git,
    Browser,
    Notifications,
    LocalModels,
    Vllm,
    OpenAiCompatible,
    Gpu,
    NetworkOpen,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ModelRoute {
    pub strategy: ModelRoutingStrategy,
    #[serde(default)]
    pub required_features: Vec<ModelFeature>,
    #[serde(default)]
    pub candidates: Vec<ModelCandidate>,
    pub fallback: ModelFallbackPolicy,
}

impl ModelRoute {
    pub fn preferred_candidate<'a>(
        &'a self,
        registration: &'a RunnerRegistration,
    ) -> Option<&'a ModelCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| {
                registration
                    .models
                    .iter()
                    .any(|m| m.matches_candidate(candidate))
            })
            .min_by_key(|candidate| candidate.priority)
            .or_else(|| {
                registration
                    .models
                    .iter()
                    .min_by_key(|candidate| candidate.priority)
            })
    }
}

impl Default for ModelRoute {
    fn default() -> Self {
        Self {
            strategy: ModelRoutingStrategy::FirstAvailable,
            required_features: vec![ModelFeature::ToolUse, ModelFeature::JsonSchema],
            candidates: vec![ModelCandidate {
                provider: ModelProviderKind::Codex,
                model: "codex-default".to_string(),
                endpoint: None,
                priority: 100,
                weight: 1,
                context_window: None,
                features: vec![ModelFeature::ToolUse, ModelFeature::JsonSchema],
                labels: BTreeMap::new(),
            }],
            fallback: ModelFallbackPolicy::AllowFallback,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelRoutingStrategy {
    FirstAvailable,
    LowestLatency,
    LowestCost,
    HighestQuality,
    Weighted,
    StickyPerGoal,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelFallbackPolicy {
    DisallowFallback,
    AllowFallback,
    AllowLowerTierLocal,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ModelCandidate {
    pub provider: ModelProviderKind,
    pub model: String,
    pub endpoint: Option<String>,
    pub priority: u32,
    pub weight: u32,
    pub context_window: Option<u32>,
    #[serde(default)]
    pub features: Vec<ModelFeature>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

impl ModelCandidate {
    fn matches_candidate(&self, requested: &ModelCandidate) -> bool {
        self.provider == requested.provider
            && self.model == requested.model
            && requested
                .features
                .iter()
                .all(|feature| self.features.contains(feature))
            && requested
                .labels
                .iter()
                .all(|(key, value)| self.labels.get(key).is_some_and(|actual| actual == value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelProviderKind {
    Codex,
    OpenAi,
    OpenAiCompatible,
    Vllm,
    Ollama,
    LlamaCpp,
    Anthropic,
    HuggingFace,
    LocalProcess,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelFeature {
    ToolUse,
    JsonSchema,
    Streaming,
    Vision,
    LongContext,
    Reasoning,
    Embeddings,
    LocalWeights,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PersonaSpec {
    pub name: String,
    pub instructions_ref: Option<String>,
    #[serde(default)]
    pub inline_instructions: Vec<String>,
    pub risk_tolerance: RiskTolerance,
}

impl PersonaSpec {
    fn with_default_name_for_role(mut self, role: &WorkerKind) -> Self {
        if self.name == "default" || WorkerKind::all_names().contains(&self.name.as_str()) {
            self.name = role.as_str().to_string();
        }
        self
    }
}

impl Default for PersonaSpec {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            instructions_ref: None,
            inline_instructions: Vec::new(),
            risk_tolerance: RiskTolerance::Conservative,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskTolerance {
    Conservative,
    Balanced,
    Aggressive,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct McpContextRef {
    pub context_id: Option<String>,
    #[serde(default)]
    pub servers: Vec<McpServerRef>,
    #[serde(default)]
    pub secret_refs: Vec<SecretRef>,
    pub propagation: McpContextPropagation,
    pub token_ttl_seconds: Option<u64>,
}

impl Default for McpContextRef {
    fn default() -> Self {
        Self {
            context_id: None,
            servers: Vec::new(),
            secret_refs: Vec::new(),
            propagation: McpContextPropagation::CoordinatorIssued,
            token_ttl_seconds: Some(900),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpContextPropagation {
    CoordinatorIssued,
    RunnerResolvesRefs,
    WorkloadIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct McpServerRef {
    pub name: String,
    pub transport: McpTransport,
    pub uri: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub auth: McpAuthRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Http,
    Sse,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum McpAuthRef {
    None,
    Secret { secret: SecretRef },
    WorkloadIdentity { audience: String },
    OAuthDelegation { token_exchange_secret: SecretRef },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SecretRef {
    pub provider: SecretProvider,
    pub name: String,
    pub key: Option<String>,
    pub namespace: Option<String>,
    pub audience: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecretProvider {
    Env,
    KubernetesSecret,
    Vault,
    AwsSecretsManager,
    GcpSecretManager,
    AzureKeyVault,
    LocalFile,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct NotificationPolicy {
    #[serde(default)]
    pub events: Vec<NotificationEvent>,
    #[serde(default)]
    pub targets: Vec<NotificationTarget>,
    pub feedback_thread_key: Option<String>,
    pub escalation_seconds: Option<u64>,
}

impl Default for NotificationPolicy {
    fn default() -> Self {
        Self {
            events: vec![
                NotificationEvent::ApprovalRequested,
                NotificationEvent::HumanFeedbackRequested,
                NotificationEvent::TaskBlocked,
            ],
            targets: Vec::new(),
            feedback_thread_key: None,
            escalation_seconds: Some(3600),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct NotificationTarget {
    pub kind: NotificationTargetKind,
    pub address: String,
    pub secret_ref: Option<SecretRef>,
    pub require_ack: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationTargetKind {
    Thread,
    Webhook,
    Slack,
    Email,
    GitHub,
    Linear,
    Jira,
    PagerDuty,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEvent {
    HumanFeedbackRequested,
    ApprovalRequested,
    TaskBlocked,
    TaskFailed,
    GoalCompleted,
    BudgetWarning,
    RunnerLost,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct NotificationRequest {
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub event: NotificationEvent,
    pub message: String,
    pub policy: NotificationPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct NotificationDeliveryReport {
    pub target: Option<NotificationTarget>,
    pub delivered: bool,
    pub external_ref: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct RunnerRegistration {
    pub runner_id: String,
    pub node_id: String,
    pub endpoint: String,
    #[serde(default)]
    pub roles: Vec<WorkerKind>,
    #[serde(default)]
    pub capabilities: Vec<RunnerCapability>,
    #[serde(default)]
    pub models: Vec<ModelCandidate>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerRef>,
    pub max_concurrency: u32,
    pub lease_ttl_seconds: u64,
}

impl RunnerRegistration {
    pub fn can_run_task(&self, task: &TaskNode) -> bool {
        task.execution.runner.matches(self)
            && task
                .execution
                .model
                .preferred_candidate(self)
                .is_some_and(|candidate| {
                    task.execution
                        .model
                        .required_features
                        .iter()
                        .all(|feature| candidate.features.contains(feature))
                })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct RunnerHeartbeat {
    pub runner_id: String,
    pub node_id: String,
    pub running_tasks: u32,
    pub capacity_remaining: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct RunnerDispatchRequest {
    pub goal_id: GoalId,
    pub task: TaskNode,
    #[serde(default)]
    pub registered_runners: Vec<RunnerRegistration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct RunnerDispatchDecision {
    pub status: RunnerDispatchStatus,
    pub runner_id: Option<String>,
    pub runner_endpoint: Option<String>,
    pub model: Option<ModelCandidate>,
    pub mcp_context: McpContextRef,
    #[serde(default)]
    pub reasons: Vec<String>,
}

impl RunnerDispatchDecision {
    pub fn choose(request: RunnerDispatchRequest) -> Self {
        let selected = request
            .registered_runners
            .iter()
            .find(|registration| registration.can_run_task(&request.task));

        match selected {
            Some(registration) => Self {
                status: RunnerDispatchStatus::Matched,
                runner_id: Some(registration.runner_id.clone()),
                runner_endpoint: Some(registration.endpoint.clone()),
                model: request
                    .task
                    .execution
                    .model
                    .preferred_candidate(registration)
                    .cloned(),
                mcp_context: request.task.execution.mcp.clone(),
                reasons: vec![
                    "matched runner capabilities, labels, role, and model route".to_string(),
                ],
            },
            None => Self {
                status: RunnerDispatchStatus::NoMatch,
                runner_id: None,
                runner_endpoint: None,
                model: None,
                mcp_context: request.task.execution.mcp.clone(),
                reasons: vec![
                    "no registered runner satisfied the task execution profile".to_string(),
                ],
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerDispatchStatus {
    Matched,
    NoMatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactRef {
    pub kind: ArtifactKind,
    pub uri: String,
    pub description: String,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Patch,
    TestResult,
    Report,
    PullRequest,
    WorkspaceSnapshot,
    Schema,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct AgentRunRequest {
    pub goal_id: GoalId,
    pub task: TaskNode,
    #[serde(default)]
    pub context_artifacts: Vec<ArtifactRef>,
    pub coordinator_trace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct AgentRunResult {
    pub task_id: TaskId,
    pub status: WorkerRunStatus,
    pub summary: String,
    pub runner_id: Option<String>,
    pub model_used: Option<ModelCandidate>,
    pub mcp_context_used: Option<McpContextRef>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub child_requests: Vec<ChildTaskRequest>,
    pub confidence: f32,
    #[serde(default)]
    pub next_actions: Vec<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub notification_reports: Vec<NotificationDeliveryReport>,
}

impl AgentRunResult {
    pub fn stub_done(task: &TaskNode) -> Self {
        Self {
            task_id: task.id,
            status: WorkerRunStatus::Done,
            summary: format!(
                "stub {} worker completed task {}",
                task.role.as_str(),
                task.id
            ),
            runner_id: Some("stub-runner".to_string()),
            model_used: task.execution.model.candidates.first().cloned(),
            mcp_context_used: Some(task.execution.mcp.clone()),
            artifacts: vec![ArtifactRef {
                kind: ArtifactKind::Report,
                uri: format!("memory://task/{}", task.id),
                description: "stub worker result".to_string(),
                sha256: None,
            }],
            child_requests: Vec::new(),
            confidence: 0.9,
            next_actions: Vec::new(),
            diagnostics: Vec::new(),
            notification_reports: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRunStatus {
    Done,
    Partial,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ChildTaskRequest {
    pub role: WorkerKind,
    pub prompt: String,
    pub reason: String,
    #[serde(default)]
    pub dependencies: Vec<TaskId>,
    pub budget: Option<Budget>,
    pub sandbox: Option<SandboxProfile>,
    pub done_criteria: Option<DoneCriteria>,
    pub execution: Option<ExecutionProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ValidationRequest {
    pub goal_id: GoalId,
    pub task: TaskNode,
    pub result: AgentRunResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ValidationReport {
    pub goal_id: GoalId,
    pub task_id: TaskId,
    pub passed: bool,
    pub score: f32,
    pub status_after_validation: TaskStatus,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub missing_criteria: Vec<String>,
    #[serde(default)]
    pub child_requests: Vec<ChildTaskRequest>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
}

impl ValidationReport {
    pub fn from_result(req: ValidationRequest) -> Self {
        let mut reasons = Vec::new();
        let mut missing_criteria = Vec::new();
        let artifact_exists = !req.result.artifacts.is_empty();
        if req.task.done_criteria.artifact_exists && !artifact_exists {
            missing_criteria.push("artifact_exists".to_string());
        }
        if let Some(min_score) = req.task.done_criteria.validator_score_min {
            if req.result.confidence < min_score {
                missing_criteria.push("validator_score_min".to_string());
            }
        }
        let passed = req.result.status == WorkerRunStatus::Done && missing_criteria.is_empty();
        if passed {
            reasons.push("worker result satisfies current done criteria".to_string());
        } else {
            reasons.push("worker result needs retry, child tasks, or escalation".to_string());
        }
        Self {
            goal_id: req.goal_id,
            task_id: req.task.id,
            passed,
            score: req.result.confidence,
            status_after_validation: if passed {
                TaskStatus::Done
            } else {
                TaskStatus::Runnable
            },
            reasons,
            missing_criteria,
            child_requests: req.result.child_requests,
            artifacts: req.result.artifacts,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SpawnPolicy {
    pub max_depth: u32,
    pub max_children_per_task: usize,
    pub min_remaining_tokens: u64,
    pub min_remaining_runtime_seconds: u64,
}

impl Default for SpawnPolicy {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_children_per_task: 8,
            min_remaining_tokens: 8_000,
            min_remaining_runtime_seconds: 60,
        }
    }
}

impl SpawnPolicy {
    pub fn ensure_spawn_allowed(
        &self,
        parent: &TaskNode,
        requested: &[ChildTaskRequest],
    ) -> Result<(), DomainError> {
        if parent.depth >= self.max_depth {
            return Err(DomainError::SpawnDenied("max_depth exceeded".to_string()));
        }
        if requested.len() > self.max_children_per_task {
            return Err(DomainError::SpawnDenied(
                "max_children_per_task exceeded".to_string(),
            ));
        }
        if parent.budget.remaining_child_tasks < requested.len() as u32 {
            return Err(DomainError::SpawnDenied(
                "remaining_child_tasks exhausted".to_string(),
            ));
        }
        if parent.budget.remaining_tokens < self.min_remaining_tokens {
            return Err(DomainError::SpawnDenied(
                "remaining_tokens too low".to_string(),
            ));
        }
        if parent.budget.remaining_runtime_seconds < self.min_remaining_runtime_seconds {
            return Err(DomainError::SpawnDenied(
                "remaining_runtime_seconds too low".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub reason: String,
    pub status: ApprovalStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct HumanFeedback {
    pub message: String,
    pub task_id: Option<TaskId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct HumanApproval {
    pub approval_id: Uuid,
    pub approved: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct StateEvent {
    pub sequence: u64,
    pub message: String,
}

impl StateEvent {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            sequence: 0,
            message: message.into(),
        }
    }
}

impl WorkerKind {
    pub fn all_names() -> &'static [&'static str] {
        &[
            "planner",
            "codex",
            "staff_engineer_claude",
            "research",
            "reviewer",
            "tester",
            "validator",
            "patch_merger",
            "rust_tool",
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Codex => "codex",
            Self::StaffEngineerClaude => "staff_engineer_claude",
            Self::Research => "research",
            Self::Reviewer => "reviewer",
            Self::Tester => "tester",
            Self::Validator => "validator",
            Self::PatchMerger => "patch_merger",
            Self::RustTool => "rust_tool",
        }
    }
}

pub fn detect_cycles(tasks: &BTreeMap<TaskId, TaskNode>) -> Result<(), DomainError> {
    for task_id in tasks.keys().copied() {
        let mut seen = BTreeSet::new();
        visit(task_id, task_id, tasks, &mut seen)?;
    }
    Ok(())
}

fn visit(
    root: TaskId,
    current: TaskId,
    tasks: &BTreeMap<TaskId, TaskNode>,
    seen: &mut BTreeSet<TaskId>,
) -> Result<(), DomainError> {
    if !seen.insert(current) {
        return Err(DomainError::CycleDetected(root));
    }
    let Some(task) = tasks.get(&current) else {
        return Ok(());
    };
    for dep in &task.dependencies {
        visit(root, *dep, tasks, seen)?;
    }
    seen.remove(&current);
    Ok(())
}

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("task not found: {0}")]
    TaskNotFound(TaskId),
    #[error("spawn denied: {0}")]
    SpawnDenied(String),
    #[error("cycle detected from task: {0}")]
    CycleDetected(TaskId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_goal_has_runnable_root() {
        let state = GoalState::new(GoalSpec::new("test", "do the thing"));
        let runnable = state.runnable_tasks();
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].role, WorkerKind::Planner);
    }

    #[test]
    fn spawn_policy_blocks_depth_overflow() {
        let goal = GoalSpec::new("test", "do the thing");
        let mut parent = GoalState::new(goal).runnable_tasks().remove(0);
        parent.depth = 8;
        let request = ChildTaskRequest {
            role: WorkerKind::Tester,
            prompt: "test".to_string(),
            reason: "coverage".to_string(),
            dependencies: Vec::new(),
            budget: None,
            sandbox: None,
            done_criteria: None,
            execution: None,
        };
        assert!(
            SpawnPolicy::default()
                .ensure_spawn_allowed(&parent, &[request])
                .is_err()
        );
    }

    #[test]
    fn validation_requires_artifacts_when_requested() {
        let state = GoalState::new(GoalSpec::new("test", "do the thing"));
        let task = state.runnable_tasks().remove(0);
        let result = AgentRunResult {
            artifacts: Vec::new(),
            ..AgentRunResult::stub_done(&task)
        };
        let report = ValidationReport::from_result(ValidationRequest {
            goal_id: task.goal_id,
            task,
            result,
        });
        assert!(!report.passed);
        assert_eq!(report.status_after_validation, TaskStatus::Runnable);
    }

    #[test]
    fn child_task_inherits_execution_profile_with_new_role() {
        let goal = GoalSpec::new("test", "do the thing");
        let mut state = GoalState::new(goal);
        let parent = state.runnable_tasks().remove(0);
        let result = AgentRunResult {
            child_requests: vec![ChildTaskRequest {
                role: WorkerKind::Tester,
                prompt: "test it".to_string(),
                reason: "need evidence".to_string(),
                dependencies: Vec::new(),
                budget: None,
                sandbox: None,
                done_criteria: None,
                execution: None,
            }],
            ..AgentRunResult::stub_done(&parent)
        };

        state
            .apply_agent_result(result, &SpawnPolicy::default())
            .expect("child spawn");
        let child = state
            .tasks
            .values()
            .find(|task| task.parent_id == Some(parent.id))
            .expect("child task");
        assert_eq!(child.role, WorkerKind::Tester);
        assert_eq!(child.execution.runner.worker, Some(WorkerKind::Tester));
        assert_eq!(child.execution.persona.name, "tester");
    }

    #[test]
    fn runner_registration_matches_local_model_route_and_mcp_capability() {
        let mut goal = GoalSpec::new("test", "do the thing");
        goal.default_execution.runner.required_capabilities = vec![
            RunnerCapability::LocalModels,
            RunnerCapability::McpTools,
            RunnerCapability::Vllm,
        ];
        goal.default_execution.model = ModelRoute {
            strategy: ModelRoutingStrategy::FirstAvailable,
            required_features: vec![ModelFeature::ToolUse, ModelFeature::JsonSchema],
            candidates: vec![ModelCandidate {
                provider: ModelProviderKind::Vllm,
                model: "qwen3-coder-30b".to_string(),
                endpoint: Some("http://vllm:8000/v1".to_string()),
                priority: 10,
                weight: 1,
                context_window: Some(131_072),
                features: vec![ModelFeature::ToolUse, ModelFeature::JsonSchema],
                labels: BTreeMap::from([("gpu".to_string(), "a100".to_string())]),
            }],
            fallback: ModelFallbackPolicy::AllowFallback,
        };
        goal.default_execution.mcp.servers = vec![McpServerRef {
            name: "repo-tools".to_string(),
            transport: McpTransport::Http,
            uri: "http://tool-registry:9084/mcp".to_string(),
            allowed_tools: vec!["repo_status".to_string()],
            auth: McpAuthRef::Secret {
                secret: SecretRef {
                    provider: SecretProvider::KubernetesSecret,
                    name: "mcp-token".to_string(),
                    key: Some("token".to_string()),
                    namespace: Some("jattg".to_string()),
                    audience: Some("tool-registry".to_string()),
                },
            },
        }];

        let state = GoalState::new(goal);
        let task = state.runnable_tasks().remove(0);
        let registration = RunnerRegistration {
            runner_id: "runner-a".to_string(),
            node_id: "node-1".to_string(),
            endpoint: "http://runner-a:9099".to_string(),
            roles: vec![WorkerKind::Planner],
            capabilities: vec![
                RunnerCapability::LocalModels,
                RunnerCapability::McpTools,
                RunnerCapability::Vllm,
            ],
            models: task.execution.model.candidates.clone(),
            labels: BTreeMap::new(),
            mcp_servers: task.execution.mcp.servers.clone(),
            max_concurrency: 2,
            lease_ttl_seconds: 300,
        };

        let decision = RunnerDispatchDecision::choose(RunnerDispatchRequest {
            goal_id: task.goal_id,
            task,
            registered_runners: vec![registration],
        });
        assert_eq!(decision.status, RunnerDispatchStatus::Matched);
        assert_eq!(
            decision.model.expect("model").provider,
            ModelProviderKind::Vllm
        );
        assert_eq!(decision.mcp_context.servers.len(), 1);
    }

    #[test]
    fn examples_parse_against_domain_contracts() {
        serde_json::from_str::<GoalSpec>(include_str!("../../../examples/goal-vllm-mcp.json"))
            .expect("goal-vllm-mcp example parses");
        serde_json::from_str::<RunnerRegistration>(include_str!(
            "../../../examples/runner-vllm.json"
        ))
        .expect("runner-vllm example parses");
        serde_json::from_str::<NotificationRequest>(include_str!(
            "../../../examples/notification-approval.json"
        ))
        .expect("notification example parses");
    }
}
