//! Operator-facing actor, event, and projection contracts.
//!
//! These types keep the SPA, TUI, MCP tools, and notification adapters pointed
//! at the same product model: a durable actor-style task graph. Restate remains
//! the orchestration authority; these contracts describe query projections,
//! action envelopes, and transition validation at the control-plane boundary.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AgentRunResult, ApprovalRecord, ApprovalStatus, ArtifactRef, CheckpointRef,
    ComputeGraphSnapshot, DelayedComputeThunk, DelayedComputeThunkStatus, GoalId, GoalProgress,
    GoalRecord, GoalStatus, ReviewOutput, TaskId, TaskPurposeKind, TaskRecord, TaskStatus,
    WorkerKind,
};

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum OperatorActorKind {
    Goal,
    Task,
    Thunk,
    WorkerRun,
    Review,
    Approval,
    Event,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatorActorStateClass {
    Active,
    Waiting,
    Recoverable,
    TerminalOk,
    TerminalStopped,
    Immutable,
}

impl OperatorActorStateClass {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::TerminalOk | Self::TerminalStopped)
    }

    pub fn is_recoverable(self) -> bool {
        matches!(self, Self::Waiting | Self::Recoverable)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorActorRef {
    pub kind: OperatorActorKind,
    pub id: String,
    pub goal_id: Option<GoalId>,
    pub task_id: Option<TaskId>,
}

impl OperatorActorRef {
    pub fn goal(goal_id: GoalId) -> Self {
        Self {
            kind: OperatorActorKind::Goal,
            id: goal_id.to_string(),
            goal_id: Some(goal_id),
            task_id: None,
        }
    }

    pub fn task(goal_id: GoalId, task_id: TaskId) -> Self {
        Self {
            kind: OperatorActorKind::Task,
            id: task_id.to_string(),
            goal_id: Some(goal_id),
            task_id: Some(task_id),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatorTransition {
    SubmitGoal,
    DraftAccepted,
    TaskDispatched,
    WorkerResultReceived,
    TaskBlocked,
    ThunkCreated,
    ThunkResumed,
    ApprovalRequested,
    ApprovalResolved,
    ReviewCompleted,
    BranchSelected,
    GoalSteered,
    GoalCancelled,
    GoalSatisfied,
}

impl OperatorTransition {
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::SubmitGoal | Self::DraftAccepted | Self::GoalSteered => "goal.updated",
            Self::TaskDispatched | Self::TaskBlocked => "task.updated",
            Self::WorkerResultReceived => "worker.completed",
            Self::ThunkCreated => "thunk.created",
            Self::ThunkResumed => "task.updated",
            Self::ApprovalRequested => "approval.requested",
            Self::ApprovalResolved => "task.updated",
            Self::ReviewCompleted => "review.completed",
            Self::BranchSelected => "task.updated",
            Self::GoalCancelled => "goal.cancelled",
            Self::GoalSatisfied => "goal.satisfied",
        }
    }

    pub fn actor_kinds(&self) -> &'static [OperatorActorKind] {
        match self {
            Self::SubmitGoal | Self::DraftAccepted | Self::GoalSteered | Self::GoalSatisfied => {
                &[OperatorActorKind::Goal]
            }
            Self::TaskDispatched | Self::TaskBlocked => &[OperatorActorKind::Task],
            Self::WorkerResultReceived => &[OperatorActorKind::Task, OperatorActorKind::WorkerRun],
            Self::ThunkCreated | Self::ThunkResumed => {
                &[OperatorActorKind::Task, OperatorActorKind::Thunk]
            }
            Self::ApprovalRequested | Self::ApprovalResolved => {
                &[OperatorActorKind::Task, OperatorActorKind::Approval]
            }
            Self::ReviewCompleted => &[OperatorActorKind::Task, OperatorActorKind::Review],
            Self::BranchSelected => &[OperatorActorKind::Goal],
            Self::GoalCancelled => &[
                OperatorActorKind::Goal,
                OperatorActorKind::Task,
                OperatorActorKind::Thunk,
                OperatorActorKind::Approval,
            ],
        }
    }

    pub fn targets_actor_kind(&self, kind: OperatorActorKind) -> bool {
        self.actor_kinds().contains(&kind)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorRecoveryHint {
    pub action: String,
    pub label: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorTransitionRejection {
    pub actor: OperatorActorRef,
    pub transition: OperatorTransition,
    pub current_status: String,
    pub message: String,
    #[serde(default)]
    pub recovery_hints: Vec<OperatorRecoveryHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct DurableEventEnvelope {
    pub event_id: Uuid,
    pub event_type: String,
    pub actor: OperatorActorRef,
    pub transition: OperatorTransition,
    pub idempotency_key: String,
    pub causation_id: Option<String>,
    pub correlation_id: Option<String>,
    pub restate_invocation_id: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorWorkspaceSnapshot {
    pub generated_at: String,
    #[serde(default)]
    pub goals: Vec<OperatorGoalSummary>,
    pub selected_goal: Option<OperatorGoalDetail>,
    #[serde(default)]
    pub actions: Vec<OperatorAction>,
    #[serde(default)]
    pub events: Vec<OperatorEvent>,
    #[serde(default)]
    pub worker_runs: Vec<OperatorWorkerRun>,
    #[serde(default)]
    pub evidence: Vec<OperatorEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorGoalSummary {
    pub goal_id: GoalId,
    pub title: String,
    pub objective: String,
    pub status: GoalStatus,
    pub percent_done: f32,
    pub open_tasks: u32,
    pub blocked_tasks: u32,
    pub failed_tasks: u32,
    pub satisfied: bool,
    pub updated_at: Option<String>,
}

impl From<GoalRecord> for OperatorGoalSummary {
    fn from(record: GoalRecord) -> Self {
        Self {
            goal_id: record.goal_id,
            title: record.title,
            objective: record.objective,
            status: record.status,
            percent_done: record.percent_done,
            open_tasks: record.open_tasks,
            blocked_tasks: record.blocked_tasks,
            failed_tasks: record.failed_tasks,
            satisfied: record.satisfied,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorGoalDetail {
    pub summary: OperatorGoalSummary,
    pub progress: Option<GoalProgress>,
    pub graph: Option<OperatorGraph>,
    #[serde(default)]
    pub tasks: Vec<TaskRecord>,
    #[serde(default)]
    pub actions: Vec<OperatorAction>,
    #[serde(default)]
    pub evidence: Vec<OperatorEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorGraph {
    pub goal_id: GoalId,
    pub compute_graph: ComputeGraphSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatorActionKind {
    AcceptDraft,
    ResumeThunk,
    ResolveApproval,
    RestartTask,
    ReplanTask,
    SelectBranch,
    CancelGoal,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorAction {
    pub action_id: String,
    pub kind: OperatorActionKind,
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub title: String,
    pub question: String,
    pub status: String,
    #[serde(default)]
    pub allowed_resolutions: Vec<OperatorActionResolutionKind>,
    pub approval: Option<ApprovalRecord>,
    pub thunk: Option<DelayedComputeThunk>,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatorActionResolutionKind {
    Continue,
    Answer,
    AddContext,
    Approve,
    Reject,
    Retry,
    Replan,
    CancelGoal,
    AcceptDraft,
    DiscardDraft,
    SelectBranch,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorActionResolution {
    pub action_id: String,
    pub resolution: OperatorActionResolutionKind,
    pub operator: Option<String>,
    pub response_summary: Option<String>,
    pub answer: Option<String>,
    #[serde(default)]
    pub artifact_refs: Vec<ArtifactRef>,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorEvent {
    pub event_id: String,
    pub event_type: String,
    pub goal_id: Option<GoalId>,
    pub task_id: Option<TaskId>,
    pub title: String,
    pub detail: String,
    pub created_at: Option<String>,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorEvidence {
    pub evidence_id: String,
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub title: String,
    pub uri: Option<String>,
    pub checkpoint: Option<CheckpointRef>,
    pub created_at: Option<String>,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorWorkerRun {
    pub run_id: String,
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub worker: WorkerKind,
    pub status: String,
    pub summary: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorEventAppendRequest {
    pub event: DurableEventEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorEventAppendResponse {
    pub accepted: bool,
    pub event_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct OperatorEventListResponse {
    #[serde(default)]
    pub events: Vec<DurableEventEnvelope>,
}

pub trait OperatorActor {
    fn actor_ref(&self) -> OperatorActorRef;
    fn status_label(&self) -> String;
    fn state_class(&self) -> OperatorActorStateClass;
    fn can_apply(&self, transition: &OperatorTransition)
    -> Result<(), OperatorTransitionRejection>;
}

fn reject_actor_scope(
    actor: OperatorActorRef,
    transition: OperatorTransition,
    current_status: String,
) -> OperatorTransitionRejection {
    OperatorTransitionRejection {
        actor,
        transition,
        current_status,
        message: "transition does not target this actor kind".to_string(),
        recovery_hints: vec![OperatorRecoveryHint {
            action: "inspect_actor".to_string(),
            label: "Inspect the matching actor".to_string(),
            reason: "route the command to the actor kind listed by the typed transition"
                .to_string(),
        }],
    }
}

fn reject_actor_state(
    actor: OperatorActorRef,
    transition: OperatorTransition,
    current_status: String,
    message: impl Into<String>,
    recovery_hints: Vec<OperatorRecoveryHint>,
) -> OperatorTransitionRejection {
    OperatorTransitionRejection {
        actor,
        transition,
        current_status,
        message: message.into(),
        recovery_hints,
    }
}

fn refresh_hint(reason: impl Into<String>) -> OperatorRecoveryHint {
    OperatorRecoveryHint {
        action: "refresh_actions".to_string(),
        label: "Refresh action queue".to_string(),
        reason: reason.into(),
    }
}

fn restart_hint(reason: impl Into<String>) -> OperatorRecoveryHint {
    OperatorRecoveryHint {
        action: "restart".to_string(),
        label: "Restart recoverable work".to_string(),
        reason: reason.into(),
    }
}

fn cancel_hint(reason: impl Into<String>) -> OperatorRecoveryHint {
    OperatorRecoveryHint {
        action: "cancel_goal".to_string(),
        label: "Cancel goal".to_string(),
        reason: reason.into(),
    }
}

pub struct GoalActor<'a>(pub &'a GoalRecord);

impl OperatorActor for GoalActor<'_> {
    fn actor_ref(&self) -> OperatorActorRef {
        OperatorActorRef::goal(self.0.goal_id)
    }

    fn status_label(&self) -> String {
        format!("{:?}", self.0.status).to_ascii_lowercase()
    }

    fn state_class(&self) -> OperatorActorStateClass {
        match self.0.status {
            GoalStatus::Running => OperatorActorStateClass::Active,
            GoalStatus::WaitingApproval | GoalStatus::Paused => OperatorActorStateClass::Waiting,
            GoalStatus::Blocked | GoalStatus::Failed => OperatorActorStateClass::Recoverable,
            GoalStatus::Done => OperatorActorStateClass::TerminalOk,
            GoalStatus::Cancelled => OperatorActorStateClass::TerminalStopped,
        }
    }

    fn can_apply(
        &self,
        transition: &OperatorTransition,
    ) -> Result<(), OperatorTransitionRejection> {
        if !transition.targets_actor_kind(OperatorActorKind::Goal) {
            return Err(reject_actor_scope(
                self.actor_ref(),
                *transition,
                self.status_label(),
            ));
        }
        let allowed = match self.0.status {
            GoalStatus::Done => {
                matches!(
                    transition,
                    OperatorTransition::SubmitGoal | OperatorTransition::GoalSatisfied
                )
            }
            GoalStatus::Cancelled => matches!(transition, OperatorTransition::SubmitGoal),
            GoalStatus::Running
            | GoalStatus::WaitingApproval
            | GoalStatus::Blocked
            | GoalStatus::Failed
            | GoalStatus::Paused => matches!(
                transition,
                OperatorTransition::SubmitGoal
                    | OperatorTransition::DraftAccepted
                    | OperatorTransition::BranchSelected
                    | OperatorTransition::GoalSteered
                    | OperatorTransition::GoalCancelled
                    | OperatorTransition::GoalSatisfied
            ),
        };
        allowed.then_some(()).ok_or_else(|| {
            reject_actor_state(
                self.actor_ref(),
                *transition,
                self.status_label(),
                "goal transition is not valid for the current goal state",
                vec![OperatorRecoveryHint {
                    action: "select_active_goal".to_string(),
                    label: "Select an active goal".to_string(),
                    reason: "completed or cancelled goals cannot be restarted in place".to_string(),
                }],
            )
        })
    }
}

pub struct TaskActor<'a>(pub &'a TaskRecord);

impl OperatorActor for TaskActor<'_> {
    fn actor_ref(&self) -> OperatorActorRef {
        OperatorActorRef::task(self.0.goal_id, self.0.task_id)
    }

    fn status_label(&self) -> String {
        format!("{:?}", self.0.status).to_ascii_lowercase()
    }

    fn state_class(&self) -> OperatorActorStateClass {
        match self.0.status {
            TaskStatus::Pending
            | TaskStatus::Runnable
            | TaskStatus::Running
            | TaskStatus::NeedsValidation => OperatorActorStateClass::Active,
            TaskStatus::WaitingApproval | TaskStatus::WaitingInput => {
                OperatorActorStateClass::Waiting
            }
            TaskStatus::Blocked | TaskStatus::Failed => OperatorActorStateClass::Recoverable,
            TaskStatus::Done => OperatorActorStateClass::TerminalOk,
            TaskStatus::Cancelled => OperatorActorStateClass::TerminalStopped,
        }
    }

    fn can_apply(
        &self,
        transition: &OperatorTransition,
    ) -> Result<(), OperatorTransitionRejection> {
        if !transition.targets_actor_kind(OperatorActorKind::Task) {
            return Err(reject_actor_scope(
                self.actor_ref(),
                *transition,
                self.status_label(),
            ));
        }
        let allowed = match transition {
            OperatorTransition::TaskDispatched => self.0.status == TaskStatus::Runnable,
            OperatorTransition::WorkerResultReceived => self.0.status == TaskStatus::Running,
            OperatorTransition::TaskBlocked => matches!(
                self.0.status,
                TaskStatus::Runnable
                    | TaskStatus::Running
                    | TaskStatus::WaitingApproval
                    | TaskStatus::WaitingInput
                    | TaskStatus::Blocked
                    | TaskStatus::Failed
            ),
            OperatorTransition::ThunkCreated => matches!(
                self.0.status,
                TaskStatus::Runnable
                    | TaskStatus::Running
                    | TaskStatus::WaitingInput
                    | TaskStatus::WaitingApproval
                    | TaskStatus::Blocked
                    | TaskStatus::Failed
            ),
            OperatorTransition::ApprovalRequested => matches!(
                self.0.status,
                TaskStatus::Runnable
                    | TaskStatus::Running
                    | TaskStatus::WaitingApproval
                    | TaskStatus::Blocked
                    | TaskStatus::Failed
            ),
            OperatorTransition::ThunkResumed | OperatorTransition::ApprovalResolved => matches!(
                self.0.status,
                TaskStatus::WaitingInput | TaskStatus::WaitingApproval | TaskStatus::Blocked
            ),
            OperatorTransition::ReviewCompleted => {
                self.0.purpose_kind == TaskPurposeKind::Review
                    && matches!(
                        self.0.status,
                        TaskStatus::Running | TaskStatus::NeedsValidation
                    )
            }
            OperatorTransition::GoalCancelled => true,
            _ => false,
        };
        allowed
            .then_some(())
            .ok_or_else(|| {
                let mut hints = vec![OperatorRecoveryHint {
                    action: "inspect_task".to_string(),
                    label: "Inspect task state".to_string(),
                    reason: "choose a recovery action matching the projected task status"
                        .to_string(),
                }];
                if self.state_class().is_recoverable() {
                    hints.push(restart_hint(
                        "blocked, failed, and waiting tasks should remain restartable, resumable, replan-able, or cancellable",
                    ));
                    hints.push(cancel_hint("cancel remains the explicit terminal stop path"));
                }
                reject_actor_state(
                    self.actor_ref(),
                    *transition,
                    self.status_label(),
                    "task transition is not valid for the current task state",
                    hints,
                )
            })
    }
}

pub struct ApprovalActor<'a>(pub &'a ApprovalRecord);

impl OperatorActor for ApprovalActor<'_> {
    fn actor_ref(&self) -> OperatorActorRef {
        OperatorActorRef {
            kind: OperatorActorKind::Approval,
            id: self.0.approval_id.to_string(),
            goal_id: Some(self.0.goal_id),
            task_id: self.0.task_id,
        }
    }

    fn status_label(&self) -> String {
        format!("{:?}", self.0.status).to_ascii_lowercase()
    }

    fn state_class(&self) -> OperatorActorStateClass {
        match self.0.status {
            ApprovalStatus::Pending => OperatorActorStateClass::Waiting,
            ApprovalStatus::Approved => OperatorActorStateClass::TerminalOk,
            ApprovalStatus::Rejected | ApprovalStatus::Cancelled => {
                OperatorActorStateClass::TerminalStopped
            }
        }
    }

    fn can_apply(
        &self,
        transition: &OperatorTransition,
    ) -> Result<(), OperatorTransitionRejection> {
        if !transition.targets_actor_kind(OperatorActorKind::Approval) {
            return Err(reject_actor_scope(
                self.actor_ref(),
                *transition,
                self.status_label(),
            ));
        }
        let allowed = match transition {
            OperatorTransition::ApprovalResolved => self.0.status == ApprovalStatus::Pending,
            OperatorTransition::GoalCancelled => matches!(
                self.0.status,
                ApprovalStatus::Pending | ApprovalStatus::Cancelled
            ),
            _ => false,
        };
        allowed.then_some(()).ok_or_else(|| {
            reject_actor_state(
                self.actor_ref(),
                *transition,
                self.status_label(),
                "approval transition requires a pending approval or idempotent cancellation",
                vec![refresh_hint(
                    "the approval may already have been resolved by another operator",
                )],
            )
        })
    }
}

pub struct ThunkActor<'a>(pub &'a DelayedComputeThunk);

impl OperatorActor for ThunkActor<'_> {
    fn actor_ref(&self) -> OperatorActorRef {
        OperatorActorRef {
            kind: OperatorActorKind::Thunk,
            id: self.0.id.to_string(),
            goal_id: Some(self.0.goal_id),
            task_id: self.0.task_id,
        }
    }

    fn status_label(&self) -> String {
        format!("{:?}", self.0.status).to_ascii_lowercase()
    }

    fn state_class(&self) -> OperatorActorStateClass {
        match self.0.status {
            DelayedComputeThunkStatus::Pending => OperatorActorStateClass::Waiting,
            DelayedComputeThunkStatus::Resumed => OperatorActorStateClass::TerminalOk,
            DelayedComputeThunkStatus::Cancelled | DelayedComputeThunkStatus::Expired => {
                OperatorActorStateClass::TerminalStopped
            }
        }
    }

    fn can_apply(
        &self,
        transition: &OperatorTransition,
    ) -> Result<(), OperatorTransitionRejection> {
        if !transition.targets_actor_kind(OperatorActorKind::Thunk) {
            return Err(reject_actor_scope(
                self.actor_ref(),
                *transition,
                self.status_label(),
            ));
        }
        let allowed = match transition {
            OperatorTransition::ThunkResumed => self.0.status == DelayedComputeThunkStatus::Pending,
            OperatorTransition::GoalCancelled => {
                matches!(
                    self.0.status,
                    DelayedComputeThunkStatus::Pending | DelayedComputeThunkStatus::Expired
                )
            }
            _ => false,
        };
        allowed.then_some(()).ok_or_else(|| {
            reject_actor_state(
                self.actor_ref(),
                *transition,
                self.status_label(),
                "thunk transition requires a pending delayed-compute prompt",
                vec![refresh_hint(
                    "the thunk may already have been resumed, cancelled, or expired",
                )],
            )
        })
    }
}

pub struct WorkerRunActor<'a> {
    pub result: &'a AgentRunResult,
    pub goal_id: Option<GoalId>,
}

impl OperatorActor for WorkerRunActor<'_> {
    fn actor_ref(&self) -> OperatorActorRef {
        OperatorActorRef {
            kind: OperatorActorKind::WorkerRun,
            id: format!("worker-run:{}", self.result.task_id),
            goal_id: self.goal_id,
            task_id: Some(self.result.task_id),
        }
    }

    fn status_label(&self) -> String {
        format!("{:?}", self.result.status).to_ascii_lowercase()
    }

    fn state_class(&self) -> OperatorActorStateClass {
        OperatorActorStateClass::Immutable
    }

    fn can_apply(
        &self,
        transition: &OperatorTransition,
    ) -> Result<(), OperatorTransitionRejection> {
        if !transition.targets_actor_kind(OperatorActorKind::WorkerRun) {
            return Err(reject_actor_scope(
                self.actor_ref(),
                *transition,
                self.status_label(),
            ));
        }
        let allowed = matches!(transition, OperatorTransition::WorkerResultReceived);
        allowed.then_some(()).ok_or_else(|| {
            reject_actor_state(
                self.actor_ref(),
                *transition,
                self.status_label(),
                "worker runs are immutable results; ingest them with worker_result_received"
                    .to_string(),
                vec![OperatorRecoveryHint {
                    action: "inspect_worker_run".to_string(),
                    label: "Inspect worker result".to_string(),
                    reason: "follow-up work should be a coordinator-owned task or recovery action"
                        .to_string(),
                }],
            )
        })
    }
}

pub struct ReviewActor<'a> {
    pub review: &'a ReviewOutput,
    pub goal_id: Option<GoalId>,
    pub task_id: Option<TaskId>,
}

impl OperatorActor for ReviewActor<'_> {
    fn actor_ref(&self) -> OperatorActorRef {
        OperatorActorRef {
            kind: OperatorActorKind::Review,
            id: self
                .task_id
                .map(|task_id| format!("review:{task_id}"))
                .unwrap_or_else(|| "review:unscoped".to_string()),
            goal_id: self.goal_id,
            task_id: self.task_id,
        }
    }

    fn status_label(&self) -> String {
        format!("{:?}", self.review.decision).to_ascii_lowercase()
    }

    fn state_class(&self) -> OperatorActorStateClass {
        OperatorActorStateClass::Immutable
    }

    fn can_apply(
        &self,
        transition: &OperatorTransition,
    ) -> Result<(), OperatorTransitionRejection> {
        if !transition.targets_actor_kind(OperatorActorKind::Review) {
            return Err(reject_actor_scope(
                self.actor_ref(),
                *transition,
                self.status_label(),
            ));
        }
        let allowed = matches!(transition, OperatorTransition::ReviewCompleted);
        allowed.then_some(()).ok_or_else(|| {
            reject_actor_state(
                self.actor_ref(),
                *transition,
                self.status_label(),
                "review actors only accept review_completed events",
                vec![OperatorRecoveryHint {
                    action: "create_follow_up_task".to_string(),
                    label: "Create follow-up task".to_string(),
                    reason: "changes requested by a review must become coordinator-owned work"
                        .to_string(),
                }],
            )
        })
    }
}

pub struct EventActor<'a>(pub &'a DurableEventEnvelope);

impl OperatorActor for EventActor<'_> {
    fn actor_ref(&self) -> OperatorActorRef {
        self.0.actor.clone()
    }

    fn status_label(&self) -> String {
        self.0.event_type.clone()
    }

    fn state_class(&self) -> OperatorActorStateClass {
        OperatorActorStateClass::Immutable
    }

    fn can_apply(
        &self,
        transition: &OperatorTransition,
    ) -> Result<(), OperatorTransitionRejection> {
        let allowed = transition == &self.0.transition;
        allowed.then_some(()).ok_or_else(|| {
            reject_actor_state(
                self.actor_ref(),
                *transition,
                self.status_label(),
                "operator events are immutable; append a new event for another transition",
                vec![OperatorRecoveryHint {
                    action: "append_new_event".to_string(),
                    label: "Append a new event".to_string(),
                    reason: "event history is append-only and cannot be rewritten in place"
                        .to_string(),
                }],
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ApprovalRisk, ContinuationBoundary, ContinuationRef, GoalRecord, GoalSpec, GoalState,
        ReviewDecision, TaskPriority, TaskPurposeKind,
    };

    fn goal(status: GoalStatus) -> GoalRecord {
        GoalRecord {
            goal_id: Uuid::new_v4(),
            title: "test".to_string(),
            objective: "prove transitions".to_string(),
            repo: None,
            status,
            total_tasks: 1,
            open_tasks: 1,
            blocked_tasks: 0,
            failed_tasks: 0,
            percent_done: 0.0,
            root_task_id: None,
            satisfied: false,
            satisfaction_score: None,
            updated_at: None,
            payload_json: serde_json::json!({}),
        }
    }

    fn task(status: TaskStatus) -> TaskRecord {
        TaskRecord {
            goal_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            parent_task_id: None,
            subgoal_id: None,
            title: "task".to_string(),
            color: None,
            role: WorkerKind::Planner,
            status,
            purpose_kind: TaskPurposeKind::Work,
            depth: 0,
            priority: TaskPriority::Normal,
            priority_rank: 3,
            attempts: 0,
            runnable: false,
            tags: Vec::new(),
            result_uri: None,
            payload_json: serde_json::json!({}),
        }
    }

    #[test]
    fn transitions_declare_their_actor_scope() {
        assert!(OperatorTransition::GoalSteered.targets_actor_kind(OperatorActorKind::Goal));
        assert!(!OperatorTransition::GoalSteered.targets_actor_kind(OperatorActorKind::Task));
        assert!(
            OperatorTransition::WorkerResultReceived.targets_actor_kind(OperatorActorKind::Task)
        );
        assert!(
            OperatorTransition::WorkerResultReceived
                .targets_actor_kind(OperatorActorKind::WorkerRun)
        );
        assert!(OperatorTransition::GoalCancelled.targets_actor_kind(OperatorActorKind::Approval));
        assert!(OperatorTransition::GoalCancelled.targets_actor_kind(OperatorActorKind::Thunk));
    }

    #[test]
    fn actor_state_classes_distinguish_recoverable_terminal_and_immutable() {
        assert_eq!(
            GoalActor(&goal(GoalStatus::Blocked)).state_class(),
            OperatorActorStateClass::Recoverable
        );
        assert_eq!(
            GoalActor(&goal(GoalStatus::Done)).state_class(),
            OperatorActorStateClass::TerminalOk
        );
        assert_eq!(
            TaskActor(&task(TaskStatus::WaitingInput)).state_class(),
            OperatorActorStateClass::Waiting
        );

        let state = GoalState::new(GoalSpec::new("worker", "classify worker run"));
        let task = state.tasks.values().next().expect("root task");
        let result = AgentRunResult::stub_done(task);
        assert_eq!(
            WorkerRunActor {
                result: &result,
                goal_id: Some(task.goal_id),
            }
            .state_class(),
            OperatorActorStateClass::Immutable
        );
    }

    #[test]
    fn goal_actor_rejects_task_transition_with_scope_guidance() {
        let record = goal(GoalStatus::Running);
        let rejection = GoalActor(&record)
            .can_apply(&OperatorTransition::TaskDispatched)
            .expect_err("goal actor should reject task-only transition");
        assert_eq!(rejection.actor.kind, OperatorActorKind::Goal);
        assert!(rejection.message.contains("does not target"));
        assert_eq!(rejection.recovery_hints[0].action, "inspect_actor");
    }

    #[test]
    fn terminal_goal_rejects_mutating_transition() {
        let record = goal(GoalStatus::Done);
        let rejection = GoalActor(&record)
            .can_apply(&OperatorTransition::GoalSteered)
            .expect_err("done goal rejects steering");
        assert_eq!(rejection.actor.kind, OperatorActorKind::Goal);
        assert!(!rejection.recovery_hints.is_empty());
    }

    #[test]
    fn recoverable_goal_accepts_cancel_transition() {
        let record = goal(GoalStatus::Blocked);
        GoalActor(&record)
            .can_apply(&OperatorTransition::GoalCancelled)
            .expect("blocked goal remains controllable");
    }

    #[test]
    fn runnable_task_can_be_dispatched() {
        let record = task(TaskStatus::Runnable);
        TaskActor(&record)
            .can_apply(&OperatorTransition::TaskDispatched)
            .expect("runnable task can dispatch");
    }

    #[test]
    fn worker_result_requires_running_task() {
        let running = task(TaskStatus::Running);
        TaskActor(&running)
            .can_apply(&OperatorTransition::WorkerResultReceived)
            .expect("running task can ingest worker result");

        let runnable = task(TaskStatus::Runnable);
        let rejection = TaskActor(&runnable)
            .can_apply(&OperatorTransition::WorkerResultReceived)
            .expect_err("runnable task has not been dispatched yet");
        assert_eq!(rejection.actor.kind, OperatorActorKind::Task);
        assert_eq!(rejection.current_status, "runnable");
    }

    #[test]
    fn recoverable_task_rejection_names_recovery_actions() {
        let record = task(TaskStatus::Blocked);
        let rejection = TaskActor(&record)
            .can_apply(&OperatorTransition::TaskDispatched)
            .expect_err("blocked task should not dispatch directly");
        let actions: Vec<_> = rejection
            .recovery_hints
            .iter()
            .map(|hint| hint.action.as_str())
            .collect();
        assert!(actions.contains(&"restart"));
        assert!(actions.contains(&"cancel_goal"));
    }

    #[test]
    fn task_actor_rejects_goal_only_transition_with_scope_guidance() {
        let record = task(TaskStatus::Running);
        let rejection = TaskActor(&record)
            .can_apply(&OperatorTransition::GoalSatisfied)
            .expect_err("task actor should reject goal-only satisfaction");
        assert_eq!(rejection.actor.kind, OperatorActorKind::Task);
        assert_eq!(rejection.recovery_hints[0].action, "inspect_actor");
    }

    #[test]
    fn done_task_rejects_worker_result_transition() {
        let record = task(TaskStatus::Done);
        let rejection = TaskActor(&record)
            .can_apply(&OperatorTransition::WorkerResultReceived)
            .expect_err("done task rejects further worker results");
        assert_eq!(rejection.actor.kind, OperatorActorKind::Task);
    }

    #[test]
    fn pending_approval_can_be_resolved() {
        let record = ApprovalRecord {
            approval_id: Uuid::new_v4(),
            goal_id: Uuid::new_v4(),
            task_id: None,
            status: ApprovalStatus::Pending,
            risk: ApprovalRisk::Low,
            reason: "need decision".to_string(),
            requested_action: "continue".to_string(),
            updated_at: None,
            payload_json: serde_json::json!({}),
        };
        ApprovalActor(&record)
            .can_apply(&OperatorTransition::ApprovalResolved)
            .expect("pending approval can resolve");
    }

    #[test]
    fn pending_approval_can_be_cancelled_with_goal() {
        let record = ApprovalRecord {
            approval_id: Uuid::new_v4(),
            goal_id: Uuid::new_v4(),
            task_id: None,
            status: ApprovalStatus::Pending,
            risk: ApprovalRisk::Low,
            reason: "need decision".to_string(),
            requested_action: "continue".to_string(),
            updated_at: None,
            payload_json: serde_json::json!({}),
        };
        ApprovalActor(&record)
            .can_apply(&OperatorTransition::GoalCancelled)
            .expect("pending approval can be closed by goal cancellation");
    }

    fn pending_thunk() -> DelayedComputeThunk {
        DelayedComputeThunk {
            id: Uuid::new_v4(),
            goal_id: Uuid::new_v4(),
            task_id: Some(Uuid::new_v4()),
            kind: crate::DelayedComputeThunkKind::HumanInput,
            status: DelayedComputeThunkStatus::Pending,
            reason: "need operator input".to_string(),
            requested_input: Some("continue?".to_string()),
            wait_ref: None,
            continuation: ContinuationRef {
                continuation_id: "continue-test".to_string(),
                boundary: ContinuationBoundary::TaskDispatch,
                state_ref: "memory://state".to_string(),
                resume_actions: vec![crate::ContinuationResumeAction::MarkRunnable],
            },
            timeout_seconds: None,
            created_at: "2026-05-14T00:00:00Z".to_string(),
            resumed_at: None,
            resume_record: None,
        }
    }

    #[test]
    fn pending_thunk_can_resume() {
        let thunk = pending_thunk();
        ThunkActor(&thunk)
            .can_apply(&OperatorTransition::ThunkResumed)
            .expect("pending thunk can resume");
    }

    #[test]
    fn pending_thunk_can_be_cancelled_with_goal() {
        let thunk = pending_thunk();
        ThunkActor(&thunk)
            .can_apply(&OperatorTransition::GoalCancelled)
            .expect("pending thunk can be closed by goal cancellation");
    }

    #[test]
    fn resumed_thunk_rejects_second_resume() {
        let mut thunk = pending_thunk();
        thunk.status = DelayedComputeThunkStatus::Resumed;
        let rejection = ThunkActor(&thunk)
            .can_apply(&OperatorTransition::ThunkResumed)
            .expect_err("resumed thunk rejects second resume");
        assert_eq!(rejection.actor.kind, OperatorActorKind::Thunk);
        assert_eq!(rejection.recovery_hints[0].action, "refresh_actions");
    }

    #[test]
    fn worker_run_is_ingested_as_result() {
        let state = GoalState::new(GoalSpec::new("worker", "ingest result"));
        let task = state.tasks.values().next().expect("root task");
        let result = AgentRunResult::stub_done(task);
        WorkerRunActor {
            result: &result,
            goal_id: Some(task.goal_id),
        }
        .can_apply(&OperatorTransition::WorkerResultReceived)
        .expect("worker result can be ingested");
    }

    #[test]
    fn worker_run_rejects_dispatch_transition() {
        let state = GoalState::new(GoalSpec::new("worker", "reject dispatch"));
        let task = state.tasks.values().next().expect("root task");
        let result = AgentRunResult::stub_done(task);
        let rejection = WorkerRunActor {
            result: &result,
            goal_id: Some(task.goal_id),
        }
        .can_apply(&OperatorTransition::TaskDispatched)
        .expect_err("worker result does not dispatch");
        assert_eq!(rejection.actor.kind, OperatorActorKind::WorkerRun);
    }

    #[test]
    fn review_actor_accepts_completed_review() {
        let review = ReviewOutput {
            decision: ReviewDecision::ChangesRequested,
            reward: 0.3,
            findings: Vec::new(),
            objective_results: Vec::new(),
            gate_results: Vec::new(),
            retry_recommended: true,
            unification_summary: None,
        };
        ReviewActor {
            review: &review,
            goal_id: Some(Uuid::new_v4()),
            task_id: Some(Uuid::new_v4()),
        }
        .can_apply(&OperatorTransition::ReviewCompleted)
        .expect("review output is a completed review event");
    }

    #[test]
    fn durable_event_rejects_rewrite_transition() {
        let goal_id = Uuid::new_v4();
        let event = DurableEventEnvelope {
            event_id: Uuid::new_v4(),
            event_type: "goal.updated".to_string(),
            actor: OperatorActorRef::goal(goal_id),
            transition: OperatorTransition::GoalSteered,
            idempotency_key: format!("goal:{goal_id}:steer:1"),
            causation_id: None,
            correlation_id: None,
            restate_invocation_id: None,
            created_at: "2026-05-14T00:00:00Z".to_string(),
            payload_json: serde_json::json!({}),
        };
        let rejection = EventActor(&event)
            .can_apply(&OperatorTransition::GoalCancelled)
            .expect_err("append-only event cannot be rewritten");
        assert_eq!(rejection.actor.kind, OperatorActorKind::Goal);
        assert_eq!(rejection.recovery_hints[0].action, "append_new_event");
    }
}
