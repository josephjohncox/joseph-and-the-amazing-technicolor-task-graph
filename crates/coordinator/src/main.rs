//! Restate-backed durable coordinator service.
//!
//! Purpose: own `GoalState`, drive the durable task frontier, request approval,
//! dispatch bounded tasks to runners, validate results, and project read models.
//! Workers do not own global plan or completion truth.
//!
//! Architecture references:
//! - `ARCHITECTURE.md`
//! - `docs/design-docs/000-system-shape.md`
//! - `docs/exec-plans/active/020-restate-coordinator.md`

use coat_domain::{
    AgentRunRequest, AgentRunResult, ApprovalRequest, BranchRequest, BranchSelectionRequest,
    ComputeGraphSnapshot, ControlLoopMode, DelayedComputeThunkRequest,
    DelayedComputeThunkResumeRequest, DomainError, GoalPriorityVoteRequest, GoalProgress, GoalSpec,
    GoalState, GoalStoreSnapshotUpsertRequest, HumanApproval, HumanFeedback,
    MechanismBallotRequest, MechanismRoundRequest, NotificationDeliveryReport, NotificationEvent,
    NotificationRequest, RestartRequest, RunnerDispatchDecision, RunnerDispatchRequest,
    RunnerDispatchStatus, SpawnPolicy, StateEvent, SteeringDirective, SteeringDirectiveKind,
    TaskList, TaskQuery, TaskStatus, ValidationReport, ValidationRequest, WorkerRunStatus,
};
use restate_sdk::{prelude::*, serde::Json};

const STATE_KEY: &str = "state";
const MAX_FRONTIER_ROUNDS: usize = 32;
const DEFAULT_GOAL_STORE_URL: &str = "http://localhost:9088";
const MISSING_GOAL_STATE_STATUS: u16 = 404;

#[restate_sdk::object]
pub trait GoalWorkflow {
    async fn run(goal: Json<GoalSpec>) -> HandlerResult<Json<GoalState>>;

    async fn cancel(reason: String) -> HandlerResult<String>;

    async fn inject_feedback(feedback: Json<HumanFeedback>) -> HandlerResult<String>;

    async fn steer(directive: Json<SteeringDirective>) -> HandlerResult<Json<Option<GoalState>>>;

    async fn approve(approval: Json<HumanApproval>) -> HandlerResult<String>;

    async fn restart(request: Json<RestartRequest>) -> HandlerResult<Json<Option<GoalState>>>;

    async fn branch(request: Json<BranchRequest>) -> HandlerResult<Json<Option<GoalState>>>;

    async fn select_branch(
        request: Json<BranchSelectionRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>>;

    async fn vote(request: Json<GoalPriorityVoteRequest>)
    -> HandlerResult<Json<Option<GoalState>>>;

    async fn resume_thunk(
        request: Json<DelayedComputeThunkResumeRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>>;

    async fn create_thunk(
        request: Json<DelayedComputeThunkRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>>;

    async fn mechanism_start(
        request: Json<MechanismRoundRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>>;

    async fn mechanism_ballot(
        request: Json<MechanismBallotRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>>;

    #[shared]
    async fn status() -> HandlerResult<Json<Option<GoalState>>>;

    #[shared]
    async fn progress() -> HandlerResult<Json<Option<GoalProgress>>>;

    #[shared]
    async fn compute_graph() -> HandlerResult<Json<Option<ComputeGraphSnapshot>>>;

    #[shared]
    async fn tasks(query: Json<TaskQuery>) -> HandlerResult<Json<Option<TaskList>>>;
}

pub struct GoalWorkflowImpl {
    spawn_policy: SpawnPolicy,
    client: reqwest::Client,
    notifier_url: String,
    goal_store_url: Option<String>,
    goal_store_required: bool,
}

impl Default for GoalWorkflowImpl {
    fn default() -> Self {
        Self {
            spawn_policy: SpawnPolicy::default(),
            client: reqwest::Client::new(),
            notifier_url: std::env::var("COAT_NOTIFIER_URL")
                .unwrap_or_else(|_| "http://localhost:9086".to_string()),
            goal_store_url: configured_goal_store_url(std::env::var("COAT_GOAL_STORE_URL").ok()),
            goal_store_required: env_bool("COAT_GOAL_STORE_REQUIRED", false),
        }
    }
}

impl GoalWorkflowImpl {
    async fn drive_state(
        &self,
        ctx: &ObjectContext<'_>,
        mut state: GoalState,
    ) -> HandlerResult<GoalState> {
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(ctx, &state, "drive_start").await?;
        let max_frontier_rounds = state.goal.control_policy.max_frontier_rounds as usize;

        for _round in 0..max_frontier_rounds.min(MAX_FRONTIER_ROUNDS) {
            if state.status == coat_domain::GoalStatus::Cancelled {
                ctx.set(STATE_KEY, Json(state.clone()));
                self.project_state(ctx, &state, "cancelled").await?;
                return Ok(state);
            }
            if state.is_done() {
                ctx.set(STATE_KEY, Json(state.clone()));
                self.project_state(ctx, &state, "done").await?;
                return Ok(state);
            }
            if state.status == coat_domain::GoalStatus::Paused {
                ctx.set(STATE_KEY, Json(state.clone()));
                self.project_state(ctx, &state, "paused").await?;
                return Ok(state);
            }
            if state.budget_exhausted() {
                state.status = coat_domain::GoalStatus::Blocked;
                for task in state.tasks.values_mut() {
                    if !task.status.is_terminal() && task.budget.is_exhausted() {
                        task.status = TaskStatus::Blocked;
                    }
                }
                state
                    .events
                    .push(coat_domain::StateEvent::new("budget_exhausted"));
                ctx.set(STATE_KEY, Json(state.clone()));
                self.project_state(ctx, &state, "budget_exhausted").await?;
                return Ok(state);
            }

            let runnable = state.runnable_tasks();
            if runnable.is_empty() {
                if state
                    .ensure_branch_frontier(&self.spawn_policy)
                    .map_err(domain_error)?
                {
                    ctx.set(STATE_KEY, Json(state.clone()));
                    continue;
                }
                if state
                    .ensure_review_frontier(&self.spawn_policy)
                    .map_err(domain_error)?
                {
                    ctx.set(STATE_KEY, Json(state.clone()));
                    continue;
                }
                let idle_status = frontier_idle_status(&state);
                if idle_status == coat_domain::GoalStatus::Running
                    && matches!(
                        state.goal.control_policy.mode,
                        ControlLoopMode::MonitorUntilCancelled
                            | ControlLoopMode::HumanSteeredContinuous
                    )
                {
                    state
                        .events
                        .push(coat_domain::StateEvent::new("control_loop_idle"));
                }
                state.status = idle_status;
                ctx.set(STATE_KEY, Json(state.clone()));
                self.project_state(ctx, &state, "frontier_idle").await?;
                return Ok(state);
            }

            for task in runnable {
                if let Some(approval) = state
                    .ensure_task_approval_or_request(task.id)
                    .map_err(domain_error)?
                {
                    ctx.set(STATE_KEY, Json(state.clone()));
                    let client = self.client.clone();
                    let notifier_url = self.notifier_url.clone();
                    let task = task.clone();
                    let approval_id = approval.id;
                    let reports = ctx
                        .run(|| async move {
                            Ok(Json(
                                notify_approval_requested(&client, &notifier_url, &task, &approval)
                                    .await,
                            ))
                        })
                        .name(format!("notify_approval_{approval_id}"))
                        .await?
                        .into_inner();
                    state
                        .record_approval_notification(approval_id, reports)
                        .map_err(domain_error)?;
                    ctx.set(STATE_KEY, Json(state.clone()));
                    self.project_state(ctx, &state, "approval_requested")
                        .await?;
                    return Ok(state);
                }
                state.mark_running(task.id).map_err(domain_error)?;
                ctx.set(STATE_KEY, Json(state.clone()));
                self.project_state(ctx, &state, "task_running").await?;

                let request = AgentRunRequest {
                    goal_id: state.goal.id,
                    task: task.clone(),
                    context_artifacts: state.final_artifacts.clone(),
                    coordinator_trace_id: None,
                    timeout_seconds: Some(state.goal.timeout_policy.task_timeout_seconds(&task)),
                };

                let result = ctx
                    .service_client::<AgentRunnerClient>()
                    .run_task(Json(request))
                    .call()
                    .await?
                    .into_inner();
                state
                    .apply_agent_result(result.clone(), &self.spawn_policy)
                    .map_err(domain_error)?;
                ctx.set(STATE_KEY, Json(state.clone()));

                let report = ctx
                    .service_client::<ValidationServiceClient>()
                    .validate(Json(ValidationRequest {
                        goal_id: state.goal.id,
                        task,
                        result: result.clone(),
                    }))
                    .call()
                    .await?
                    .into_inner();
                state.apply_validation(report).map_err(domain_error)?;
                if result.status == WorkerRunStatus::TimedOut {
                    state
                        .record_task_timeout_and_maybe_restart(
                            result.task_id,
                            state
                                .goal
                                .timeout_policy
                                .task_run_timeout_seconds
                                .or(state.goal.timeout_policy.runner_call_timeout_seconds)
                                .unwrap_or(0),
                            "worker run timed out",
                        )
                        .map_err(domain_error)?;
                }
                state
                    .ensure_branch_frontier(&self.spawn_policy)
                    .map_err(domain_error)?;
                state
                    .ensure_review_frontier(&self.spawn_policy)
                    .map_err(domain_error)?;
                ctx.set(STATE_KEY, Json(state.clone()));
                self.project_state(ctx, &state, "validation_applied")
                    .await?;
            }
        }

        state.status = coat_domain::GoalStatus::Blocked;
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(ctx, &state, "frontier_limit").await?;
        Ok(state)
    }

    async fn project_state(
        &self,
        ctx: &ObjectContext<'_>,
        state: &GoalState,
        reason: &'static str,
    ) -> HandlerResult<()> {
        let observation = observe_transition(state, reason);
        emit_transition_observation(&observation);
        let Some(goal_store_url) = self.goal_store_url.clone() else {
            return Ok(());
        };
        let client = self.client.clone();
        let required = self.goal_store_required;
        let request = GoalStoreSnapshotUpsertRequest::from_state(state, reason);
        let goal_id = state.goal.id;
        let step_name = format!("project_goal_{}_{}_{}", goal_id, reason, state.events.len());
        ctx.run(|| async move {
            let result = post_goal_snapshot(&client, &goal_store_url, &request).await;
            if let Err(error) = result {
                if required {
                    return Err(TerminalError::new(format!(
                        "goal-store projection failed: {error}"
                    ))
                    .into());
                }
                tracing::warn!(%goal_id, %reason, %error, "goal-store projection failed");
            }
            Ok(Json(()))
        })
        .name(step_name)
        .await?;
        Ok(())
    }

    async fn skip_cancelled_mutation(
        &self,
        ctx: &ObjectContext<'_>,
        state: &mut GoalState,
        event: &'static str,
        transition: &'static str,
    ) -> HandlerResult<bool> {
        let Some(skip) = cancelled_mutation_skip(&state.status, event, transition) else {
            return Ok(false);
        };
        self.record_skipped_mutation(ctx, state, skip).await?;
        Ok(true)
    }

    async fn record_skipped_mutation(
        &self,
        ctx: &ObjectContext<'_>,
        state: &mut GoalState,
        skip: SkippedMutation,
    ) -> HandlerResult<()> {
        state.events.push(StateEvent::new(skip.event));
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(ctx, state, skip.transition).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CoordinatorTransitionObservation {
    goal_id: coat_domain::GoalId,
    reason: &'static str,
    status: coat_domain::GoalStatus,
    total_tasks: usize,
    runnable_tasks: usize,
    waiting_approval_tasks: usize,
    waiting_input_tasks: usize,
    blocked_tasks: usize,
    failed_tasks: usize,
    done_tasks: usize,
    pending_approvals: usize,
    pending_delayed_compute_thunks: usize,
    open_mechanism_rounds: usize,
    ratification_required_mechanism_rounds: usize,
    compute_graph_nodes: usize,
    compute_graph_edges: usize,
    event_count: usize,
}

fn observe_transition(state: &GoalState, reason: &'static str) -> CoordinatorTransitionObservation {
    let progress = state.progress();
    CoordinatorTransitionObservation {
        goal_id: state.goal.id,
        reason,
        status: state.status.clone(),
        total_tasks: state.tasks.len(),
        runnable_tasks: state
            .tasks
            .values()
            .filter(|task| task.status == TaskStatus::Runnable)
            .count(),
        waiting_approval_tasks: state
            .tasks
            .values()
            .filter(|task| task.status == TaskStatus::WaitingApproval)
            .count(),
        waiting_input_tasks: state
            .tasks
            .values()
            .filter(|task| task.status == TaskStatus::WaitingInput)
            .count(),
        blocked_tasks: state
            .tasks
            .values()
            .filter(|task| task.status == TaskStatus::Blocked)
            .count(),
        failed_tasks: state
            .tasks
            .values()
            .filter(|task| task.status == TaskStatus::Failed)
            .count(),
        done_tasks: state
            .tasks
            .values()
            .filter(|task| task.status == TaskStatus::Done)
            .count(),
        pending_approvals: state
            .approvals
            .iter()
            .filter(|approval| approval.status == coat_domain::ApprovalStatus::Pending)
            .count(),
        pending_delayed_compute_thunks: state
            .delayed_compute_thunks
            .iter()
            .filter(|thunk| thunk.status == coat_domain::DelayedComputeThunkStatus::Pending)
            .count(),
        open_mechanism_rounds: progress.open_mechanism_rounds as usize,
        ratification_required_mechanism_rounds: progress.ratification_required_mechanism_rounds
            as usize,
        compute_graph_nodes: progress.compute_graph.nodes.len(),
        compute_graph_edges: progress.compute_graph.edges.len(),
        event_count: state.events.len(),
    }
}

fn emit_transition_observation(observation: &CoordinatorTransitionObservation) {
    let span = tracing::info_span!(
        "coordinator.transition",
        goal_id = %observation.goal_id,
        reason = observation.reason,
        status = ?observation.status,
    );
    let _entered = span.enter();
    tracing::info!(
        total_tasks = observation.total_tasks,
        runnable_tasks = observation.runnable_tasks,
        waiting_approval_tasks = observation.waiting_approval_tasks,
        waiting_input_tasks = observation.waiting_input_tasks,
        blocked_tasks = observation.blocked_tasks,
        failed_tasks = observation.failed_tasks,
        done_tasks = observation.done_tasks,
        pending_approvals = observation.pending_approvals,
        pending_delayed_compute_thunks = observation.pending_delayed_compute_thunks,
        open_mechanism_rounds = observation.open_mechanism_rounds,
        ratification_required_mechanism_rounds = observation.ratification_required_mechanism_rounds,
        compute_graph_nodes = observation.compute_graph_nodes,
        compute_graph_edges = observation.compute_graph_edges,
        event_count = observation.event_count,
        "coordinator state transition"
    );
}

fn steering_should_drive(kind: &SteeringDirectiveKind) -> bool {
    matches!(
        kind,
        SteeringDirectiveKind::InjectTask { .. }
            | SteeringDirectiveKind::RequestResearch { .. }
            | SteeringDirectiveKind::RequestStandardReview { .. }
            | SteeringDirectiveKind::Resume { .. }
            | SteeringDirectiveKind::UpdateDoneCriteria { .. }
            | SteeringDirectiveKind::ExpandDoneCriteria { .. }
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SkippedMutation {
    event: &'static str,
    transition: &'static str,
}

fn missing_goal_state_message(handler: &'static str) -> String {
    format!(
        "GoalWorkflow.{handler} has no initialized goal state; submit or run the goal before using this workflow object"
    )
}

fn missing_goal_state_error(handler: &'static str) -> HandlerError {
    TerminalError::new_with_code(
        MISSING_GOAL_STATE_STATUS,
        missing_goal_state_message(handler),
    )
    .into()
}

fn require_goal_state(state: Option<GoalState>, handler: &'static str) -> HandlerResult<GoalState> {
    state.ok_or_else(|| missing_goal_state_error(handler))
}

fn frontier_idle_status(state: &GoalState) -> coat_domain::GoalStatus {
    if state
        .tasks
        .values()
        .any(|task| task.status == TaskStatus::Failed)
    {
        coat_domain::GoalStatus::Failed
    } else if state
        .tasks
        .values()
        .any(|task| task.status == TaskStatus::Blocked)
    {
        coat_domain::GoalStatus::Blocked
    } else if state
        .tasks
        .values()
        .any(|task| task.status == TaskStatus::WaitingApproval)
    {
        coat_domain::GoalStatus::WaitingApproval
    } else if state
        .tasks
        .values()
        .any(|task| task.status == TaskStatus::WaitingInput)
        || state
            .delayed_compute_thunks
            .iter()
            .any(|thunk| thunk.status == coat_domain::DelayedComputeThunkStatus::Pending)
    {
        coat_domain::GoalStatus::Paused
    } else if matches!(
        state.goal.control_policy.mode,
        ControlLoopMode::MonitorUntilCancelled | ControlLoopMode::HumanSteeredContinuous
    ) {
        coat_domain::GoalStatus::Running
    } else if state.is_done() {
        coat_domain::GoalStatus::Done
    } else {
        coat_domain::GoalStatus::Blocked
    }
}

fn cancelled_mutation_skip(
    status: &coat_domain::GoalStatus,
    event: &'static str,
    transition: &'static str,
) -> Option<SkippedMutation> {
    (*status == coat_domain::GoalStatus::Cancelled).then_some(SkippedMutation { event, transition })
}

fn restart_stale_skip(error: &DomainError) -> Option<SkippedMutation> {
    match error {
        DomainError::RestartDenied(message)
            if message == "no restartable tasks matched the request" =>
        {
            Some(SkippedMutation {
                event: "restart_skipped:no_restartable_tasks",
                transition: "restart_skipped_no_restartable_tasks",
            })
        }
        _ => None,
    }
}

fn resume_thunk_stale_skip(error: &DomainError) -> Option<SkippedMutation> {
    match error {
        DomainError::SteeringDenied(message)
            if message.contains("delayed compute thunk") && message.contains("is not pending") =>
        {
            Some(SkippedMutation {
                event: "resume_thunk_skipped:not_pending",
                transition: "resume_thunk_skipped_not_pending",
            })
        }
        _ => None,
    }
}

fn create_thunk_duplicate_skip(
    state: &GoalState,
    continuation_id: &str,
) -> Option<SkippedMutation> {
    state
        .delayed_compute_thunks
        .iter()
        .any(|thunk| {
            thunk.status == coat_domain::DelayedComputeThunkStatus::Pending
                && thunk.continuation.continuation_id == continuation_id
        })
        .then_some(SkippedMutation {
            event: "create_thunk_skipped:continuation_exists",
            transition: "create_thunk_skipped_continuation_exists",
        })
}

impl GoalWorkflow for GoalWorkflowImpl {
    async fn run(
        &self,
        ctx: ObjectContext<'_>,
        goal: Json<GoalSpec>,
    ) -> HandlerResult<Json<GoalState>> {
        let incoming_goal = goal.into_inner();
        let state = match ctx.get::<Json<GoalState>>(STATE_KEY).await? {
            Some(Json(existing)) => existing,
            _ => GoalState::new(incoming_goal),
        };
        Ok(Json(self.drive_state(&ctx, state).await?))
    }

    async fn cancel(&self, ctx: ObjectContext<'_>, reason: String) -> HandlerResult<String> {
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "cancel",
        )?;
        if self
            .skip_cancelled_mutation(
                &ctx,
                &mut state,
                "cancel_skipped:goal_cancelled",
                "cancel_skipped_goal_cancelled",
            )
            .await?
        {
            return Ok(format!("cancel skipped; goal already cancelled: {reason}"));
        }
        state.cancel(reason.clone());
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(&ctx, &state, "cancelled").await?;
        Ok(format!("cancel requested: {reason}"))
    }

    async fn inject_feedback(
        &self,
        ctx: ObjectContext<'_>,
        feedback: Json<HumanFeedback>,
    ) -> HandlerResult<String> {
        let feedback = feedback.into_inner();
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "inject_feedback",
        )?;
        let task_suffix = feedback
            .task_id
            .map(|task_id| format!(":{task_id}"))
            .unwrap_or_default();
        state
            .events
            .push(StateEvent::new(format!("human_feedback{task_suffix}")));
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(&ctx, &state, "human_feedback").await?;
        Ok(format!("feedback accepted: {}", feedback.message))
    }

    async fn steer(
        &self,
        ctx: ObjectContext<'_>,
        directive: Json<SteeringDirective>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "steer",
        )?;
        let directive = directive.into_inner();
        if state.status == coat_domain::GoalStatus::Cancelled
            && !matches!(directive.kind, SteeringDirectiveKind::Cancel { .. })
        {
            state
                .events
                .push(StateEvent::new("steering_skipped:goal_cancelled"));
            ctx.set(STATE_KEY, Json(state.clone()));
            self.project_state(&ctx, &state, "steering_skipped_goal_cancelled")
                .await?;
            return Ok(Json(Some(state)));
        }
        let should_drive = steering_should_drive(&directive.kind);
        state
            .apply_steering(directive, &self.spawn_policy)
            .map_err(domain_error)?;
        if should_drive {
            state = self.drive_state(&ctx, state).await?;
        } else {
            ctx.set(STATE_KEY, Json(state.clone()));
            self.project_state(&ctx, &state, "steering_applied").await?;
        }
        Ok(Json(Some(state)))
    }

    async fn approve(
        &self,
        ctx: ObjectContext<'_>,
        approval: Json<HumanApproval>,
    ) -> HandlerResult<String> {
        let approval = approval.into_inner();
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "approve",
        )?;
        if self
            .skip_cancelled_mutation(
                &ctx,
                &mut state,
                "approval_skipped:goal_cancelled",
                "approval_skipped_goal_cancelled",
            )
            .await?
        {
            return Ok(format!(
                "approval {} skipped; goal cancelled",
                approval.approval_id
            ));
        }
        let updated = state
            .apply_human_approval(approval.clone())
            .map_err(domain_error)?;
        let status = if approval.approved {
            let driven = self.drive_state(&ctx, state).await?;
            format!("{:?}", driven.status)
        } else {
            ctx.set(STATE_KEY, Json(state.clone()));
            format!("{:?}", state.status)
        };
        Ok(format!(
            "approval {} {}; status {status}",
            updated.id,
            if approval.approved {
                "accepted"
            } else {
                "rejected"
            }
        ))
    }

    async fn restart(
        &self,
        ctx: ObjectContext<'_>,
        request: Json<RestartRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "restart",
        )?;
        if self
            .skip_cancelled_mutation(
                &ctx,
                &mut state,
                "restart_skipped:goal_cancelled",
                "restart_skipped_goal_cancelled",
            )
            .await?
        {
            return Ok(Json(Some(state)));
        }
        match state.apply_restart_request(request.into_inner()) {
            Ok(_) => {}
            Err(error) => {
                if let Some(skip) = restart_stale_skip(&error) {
                    self.record_skipped_mutation(&ctx, &mut state, skip).await?;
                    return Ok(Json(Some(state)));
                }
                return Err(domain_error(error));
            }
        }
        let state = self.drive_state(&ctx, state).await?;
        Ok(Json(Some(state)))
    }

    async fn branch(
        &self,
        ctx: ObjectContext<'_>,
        request: Json<BranchRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "branch",
        )?;
        if self
            .skip_cancelled_mutation(
                &ctx,
                &mut state,
                "branch_skipped:goal_cancelled",
                "branch_skipped_goal_cancelled",
            )
            .await?
        {
            return Ok(Json(Some(state)));
        }
        state
            .branch_task(request.into_inner(), &self.spawn_policy)
            .map_err(domain_error)?;
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(&ctx, &state, "branch_created").await?;
        Ok(Json(Some(state)))
    }

    async fn select_branch(
        &self,
        ctx: ObjectContext<'_>,
        request: Json<BranchSelectionRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "select_branch",
        )?;
        if self
            .skip_cancelled_mutation(
                &ctx,
                &mut state,
                "select_branch_skipped:goal_cancelled",
                "select_branch_skipped_goal_cancelled",
            )
            .await?
        {
            return Ok(Json(Some(state)));
        }
        state
            .apply_branch_selection(request.into_inner())
            .map_err(domain_error)?;
        let state = self.drive_state(&ctx, state).await?;
        Ok(Json(Some(state)))
    }

    async fn vote(
        &self,
        ctx: ObjectContext<'_>,
        request: Json<GoalPriorityVoteRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "vote",
        )?;
        if self
            .skip_cancelled_mutation(
                &ctx,
                &mut state,
                "goal_priority_vote_skipped:goal_cancelled",
                "goal_priority_vote_skipped_goal_cancelled",
            )
            .await?
        {
            return Ok(Json(Some(state)));
        }
        state
            .record_goal_priority_vote(request.into_inner())
            .map_err(domain_error)?;
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(&ctx, &state, "goal_priority_vote_recorded")
            .await?;
        Ok(Json(Some(state)))
    }

    async fn resume_thunk(
        &self,
        ctx: ObjectContext<'_>,
        request: Json<DelayedComputeThunkResumeRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "resume_thunk",
        )?;
        if self
            .skip_cancelled_mutation(
                &ctx,
                &mut state,
                "resume_thunk_skipped:goal_cancelled",
                "resume_thunk_skipped_goal_cancelled",
            )
            .await?
        {
            return Ok(Json(Some(state)));
        }
        match state.resume_delayed_compute_thunk(request.into_inner()) {
            Ok(_) => {}
            Err(error) => {
                if let Some(skip) = resume_thunk_stale_skip(&error) {
                    self.record_skipped_mutation(&ctx, &mut state, skip).await?;
                    return Ok(Json(Some(state)));
                }
                return Err(domain_error(error));
            }
        }
        let state = self.drive_state(&ctx, state).await?;
        Ok(Json(Some(state)))
    }

    async fn create_thunk(
        &self,
        ctx: ObjectContext<'_>,
        request: Json<DelayedComputeThunkRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "create_thunk",
        )?;
        if self
            .skip_cancelled_mutation(
                &ctx,
                &mut state,
                "create_thunk_skipped:goal_cancelled",
                "create_thunk_skipped_goal_cancelled",
            )
            .await?
        {
            return Ok(Json(Some(state)));
        }
        let request = request.into_inner();
        if let Some(skip) =
            create_thunk_duplicate_skip(&state, &request.continuation.continuation_id)
        {
            self.record_skipped_mutation(&ctx, &mut state, skip).await?;
            return Ok(Json(Some(state)));
        }
        state
            .create_delayed_compute_thunk(request)
            .map_err(domain_error)?;
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(&ctx, &state, "delayed_compute_thunk_created")
            .await?;
        Ok(Json(Some(state)))
    }

    async fn mechanism_start(
        &self,
        ctx: ObjectContext<'_>,
        request: Json<MechanismRoundRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "mechanism_start",
        )?;
        if self
            .skip_cancelled_mutation(
                &ctx,
                &mut state,
                "mechanism_start_skipped:goal_cancelled",
                "mechanism_start_skipped_goal_cancelled",
            )
            .await?
        {
            return Ok(Json(Some(state)));
        }
        state
            .start_mechanism_round(request.into_inner())
            .map_err(domain_error)?;
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(&ctx, &state, "mechanism_round_started")
            .await?;
        Ok(Json(Some(state)))
    }

    async fn mechanism_ballot(
        &self,
        ctx: ObjectContext<'_>,
        request: Json<MechanismBallotRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let mut state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "mechanism_ballot",
        )?;
        if self
            .skip_cancelled_mutation(
                &ctx,
                &mut state,
                "mechanism_ballot_skipped:goal_cancelled",
                "mechanism_ballot_skipped_goal_cancelled",
            )
            .await?
        {
            return Ok(Json(Some(state)));
        }
        state
            .record_mechanism_ballot(request.into_inner())
            .map_err(domain_error)?;
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(&ctx, &state, "mechanism_ballot_recorded")
            .await?;
        Ok(Json(Some(state)))
    }

    async fn status(&self, ctx: SharedObjectContext<'_>) -> HandlerResult<Json<Option<GoalState>>> {
        Ok(Json(Some(require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "status",
        )?)))
    }

    async fn progress(
        &self,
        ctx: SharedObjectContext<'_>,
    ) -> HandlerResult<Json<Option<GoalProgress>>> {
        let state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "progress",
        )?;
        Ok(Json(Some(state.progress())))
    }

    async fn compute_graph(
        &self,
        ctx: SharedObjectContext<'_>,
    ) -> HandlerResult<Json<Option<ComputeGraphSnapshot>>> {
        let state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "compute_graph",
        )?;
        Ok(Json(Some(state.compute_graph())))
    }

    async fn tasks(
        &self,
        ctx: SharedObjectContext<'_>,
        query: Json<TaskQuery>,
    ) -> HandlerResult<Json<Option<TaskList>>> {
        let state = require_goal_state(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
            "tasks",
        )?;
        Ok(Json(Some(state.find_tasks(&query.into_inner()))))
    }
}

#[restate_sdk::service]
pub trait AgentRunner {
    async fn run_task(request: Json<AgentRunRequest>) -> HandlerResult<Json<AgentRunResult>>;
}

pub struct AgentRunnerImpl {
    client: reqwest::Client,
    registry_url: String,
    notifier_url: String,
    allow_local_stub_fallback: bool,
}

impl Default for AgentRunnerImpl {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
            registry_url: std::env::var("COAT_RUNNER_REGISTRY_URL")
                .unwrap_or_else(|_| "http://localhost:9085".to_string()),
            notifier_url: std::env::var("COAT_NOTIFIER_URL")
                .unwrap_or_else(|_| "http://localhost:9086".to_string()),
            allow_local_stub_fallback: env_bool("COAT_ALLOW_LOCAL_STUB_FALLBACK", true),
        }
    }
}

impl AgentRunner for AgentRunnerImpl {
    async fn run_task(
        &self,
        ctx: Context<'_>,
        request: Json<AgentRunRequest>,
    ) -> HandlerResult<Json<AgentRunResult>> {
        let request = request.into_inner();
        let name = format!("run_{}_task", request.task.role.as_str());
        let client = self.client.clone();
        let registry_url = self.registry_url.clone();
        let notifier_url = self.notifier_url.clone();
        let allow_local_stub_fallback = self.allow_local_stub_fallback;
        let result = ctx
            .run(|| async move {
                run_distributed_task(
                    client,
                    registry_url,
                    notifier_url,
                    allow_local_stub_fallback,
                    request,
                )
                .await
            })
            .name(name)
            .await?;
        Ok(result)
    }
}

#[restate_sdk::service]
pub trait ValidationService {
    async fn validate(request: Json<ValidationRequest>) -> HandlerResult<Json<ValidationReport>>;
}

pub struct ValidationServiceImpl;

impl ValidationService for ValidationServiceImpl {
    async fn validate(
        &self,
        ctx: Context<'_>,
        request: Json<ValidationRequest>,
    ) -> HandlerResult<Json<ValidationReport>> {
        let request = request.into_inner();
        let result = ctx
            .run(|| async move { Ok(Json(ValidationReport::from_result(request))) })
            .name("validate_worker_result")
            .await?;
        Ok(result)
    }
}

fn domain_error(error: DomainError) -> HandlerError {
    TerminalError::new(error.to_string()).into()
}

async fn run_distributed_task(
    client: reqwest::Client,
    registry_url: String,
    notifier_url: String,
    allow_local_stub_fallback: bool,
    request: AgentRunRequest,
) -> HandlerResult<Json<AgentRunResult>> {
    let dispatch = dispatch_task(&client, &registry_url, &request).await;
    let mut diagnostics = Vec::new();
    let decision = match dispatch {
        Ok(decision) => decision,
        Err(error) if allow_local_stub_fallback => {
            diagnostics.push(format!(
                "runner registry unavailable; using local stub fallback: {error}"
            ));
            return Ok(Json(local_stub_result(request, diagnostics)));
        }
        Err(error) => {
            diagnostics.push(format!("runner registry unavailable: {error}"));
            let reports = notify_blocked(&client, &notifier_url, &request, &diagnostics).await;
            return Ok(Json(blocked_result(
                request,
                None,
                None,
                None,
                diagnostics,
                reports,
            )));
        }
    };

    match decision.status {
        RunnerDispatchStatus::Matched => {
            diagnostics.extend(decision.reasons.clone());
            let Some(runner_endpoint) = decision.runner_endpoint.clone() else {
                diagnostics.push("dispatch matched without runner endpoint".to_string());
                let reports = notify_blocked(&client, &notifier_url, &request, &diagnostics).await;
                return Ok(Json(blocked_result(
                    request,
                    decision.runner_id,
                    decision.model,
                    Some(decision.mcp_context),
                    diagnostics,
                    reports,
                )));
            };

            match call_runner(&client, &runner_endpoint, &request).await {
                RunnerCallOutcome::Ok(mut result) => {
                    result.runner_id = result.runner_id.or(decision.runner_id);
                    result.model_used = result.model_used.or(decision.model);
                    result.mcp_context_used =
                        result.mcp_context_used.or(Some(decision.mcp_context));
                    result.diagnostics.extend(diagnostics);
                    Ok(Json(result))
                }
                RunnerCallOutcome::TimedOut { timeout_seconds } => {
                    diagnostics.push(format!(
                        "matched runner invocation timed out after {timeout_seconds}s"
                    ));
                    let reports =
                        notify_blocked(&client, &notifier_url, &request, &diagnostics).await;
                    Ok(Json(timeout_result(
                        request,
                        decision.runner_id,
                        decision.model,
                        Some(decision.mcp_context),
                        timeout_seconds,
                        diagnostics,
                        reports,
                    )))
                }
                RunnerCallOutcome::Err(error) if allow_local_stub_fallback => {
                    diagnostics.push(format!(
                        "matched runner invocation failed; using local stub fallback: {error}"
                    ));
                    Ok(Json(local_stub_result(request, diagnostics)))
                }
                RunnerCallOutcome::Err(error) => {
                    diagnostics.push(format!("matched runner invocation failed: {error}"));
                    let reports =
                        notify_blocked(&client, &notifier_url, &request, &diagnostics).await;
                    Ok(Json(blocked_result(
                        request,
                        decision.runner_id,
                        decision.model,
                        Some(decision.mcp_context),
                        diagnostics,
                        reports,
                    )))
                }
            }
        }
        RunnerDispatchStatus::NoMatch if allow_local_stub_fallback => {
            diagnostics.extend(decision.reasons);
            diagnostics
                .push("no distributed runner matched; using local stub fallback".to_string());
            Ok(Json(local_stub_result(request, diagnostics)))
        }
        RunnerDispatchStatus::NoMatch => {
            diagnostics.extend(decision.reasons);
            let reports = notify_blocked(&client, &notifier_url, &request, &diagnostics).await;
            Ok(Json(blocked_result(
                request,
                decision.runner_id,
                decision.model,
                Some(decision.mcp_context),
                diagnostics,
                reports,
            )))
        }
    }
}

enum RunnerCallOutcome {
    Ok(AgentRunResult),
    TimedOut { timeout_seconds: u64 },
    Err(String),
}

async fn call_runner(
    client: &reqwest::Client,
    runner_endpoint: &str,
    request: &AgentRunRequest,
) -> RunnerCallOutcome {
    let timeout_seconds = request.timeout_seconds.unwrap_or(3_600).max(1);
    let call = async {
        let response = client
            .post(format!(
                "{}/run-task",
                runner_endpoint.trim_end_matches('/')
            ))
            .json(request)
            .send()
            .await
            .map_err(|error| error.to_string())?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("runner failed with {status}: {body}"));
        }
        serde_json::from_str(&body)
            .map_err(|error| format!("parse runner response: {error}: {body}"))
    };
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_seconds), call).await {
        Ok(Ok(result)) => RunnerCallOutcome::Ok(result),
        Ok(Err(error)) => RunnerCallOutcome::Err(error),
        Err(_) => RunnerCallOutcome::TimedOut { timeout_seconds },
    }
}

async fn dispatch_task(
    client: &reqwest::Client,
    registry_url: &str,
    request: &AgentRunRequest,
) -> Result<RunnerDispatchDecision, String> {
    let response = client
        .post(format!("{}/dispatch", registry_url.trim_end_matches('/')))
        .json(&RunnerDispatchRequest {
            goal_id: request.goal_id,
            task: request.task.clone(),
            coordinator_node_id: std::env::var("COAT_COORDINATOR_NODE_ID").ok(),
            registered_runners: Vec::new(),
        })
        .send()
        .await
        .map_err(|error| error.to_string())?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("dispatch failed with {status}: {body}"));
    }
    serde_json::from_str(&body).map_err(|error| format!("parse dispatch response: {error}: {body}"))
}

async fn notify_approval_requested(
    client: &reqwest::Client,
    notifier_url: &str,
    task: &coat_domain::TaskNode,
    approval: &ApprovalRequest,
) -> Vec<NotificationDeliveryReport> {
    let notification = NotificationRequest {
        goal_id: approval.goal_id,
        task_id: approval.task_id,
        event: NotificationEvent::ApprovalRequested,
        message: format!(
            "Approval {} is required to {}. Risk: {:?}. Reason: {}. Approve with: coat human approve --goal-id {} --approval-id {} --approved true",
            approval.id,
            approval.requested_action,
            approval.risk,
            approval.reason,
            approval.goal_id,
            approval.id
        ),
        policy: task.execution.notifications.clone(),
    };

    send_notification(client, notifier_url, notification).await
}

async fn notify_blocked(
    client: &reqwest::Client,
    notifier_url: &str,
    request: &AgentRunRequest,
    diagnostics: &[String],
) -> Vec<NotificationDeliveryReport> {
    let notification = NotificationRequest {
        goal_id: request.goal_id,
        task_id: Some(request.task.id),
        event: NotificationEvent::TaskBlocked,
        message: format!(
            "No runner is available for {} task {}. {}",
            request.task.role.as_str(),
            request.task.id,
            diagnostics.join("; ")
        ),
        policy: request.task.execution.notifications.clone(),
    };

    send_notification(client, notifier_url, notification).await
}

async fn send_notification(
    client: &reqwest::Client,
    notifier_url: &str,
    notification: NotificationRequest,
) -> Vec<NotificationDeliveryReport> {
    match client
        .post(format!("{}/notify", notifier_url.trim_end_matches('/')))
        .json(&notification)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if status.is_success() {
                serde_json::from_str(&body).unwrap_or_else(|error| {
                    vec![NotificationDeliveryReport {
                        target: None,
                        delivered: false,
                        external_ref: None,
                        error: Some(format!("parse notifier response: {error}: {body}")),
                    }]
                })
            } else {
                vec![NotificationDeliveryReport {
                    target: None,
                    delivered: false,
                    external_ref: None,
                    error: Some(format!("notifier failed with {status}: {body}")),
                }]
            }
        }
        Err(error) => vec![NotificationDeliveryReport {
            target: None,
            delivered: false,
            external_ref: None,
            error: Some(error.to_string()),
        }],
    }
}

async fn post_goal_snapshot(
    client: &reqwest::Client,
    goal_store_url: &str,
    request: &GoalStoreSnapshotUpsertRequest,
) -> Result<(), String> {
    let response = client
        .post(format!(
            "{}/goal-store/snapshots",
            goal_store_url.trim_end_matches('/')
        ))
        .json(request)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let body = response.text().await.unwrap_or_default();
        Err(format!("goal-store returned {status}: {body}"))
    }
}

fn local_stub_result(request: AgentRunRequest, diagnostics: Vec<String>) -> AgentRunResult {
    let mut result = AgentRunResult::stub_done(&request.task);
    result.runner_id = Some("local-stub-fallback".to_string());
    result.diagnostics = diagnostics;
    result
}

fn blocked_result(
    request: AgentRunRequest,
    runner_id: Option<String>,
    model_used: Option<coat_domain::ModelCandidate>,
    mcp_context_used: Option<coat_domain::McpContextRef>,
    diagnostics: Vec<String>,
    notification_reports: Vec<NotificationDeliveryReport>,
) -> AgentRunResult {
    AgentRunResult {
        task_id: request.task.id,
        status: WorkerRunStatus::Blocked,
        summary: "no distributed runner is available for this task".to_string(),
        review: None,
        research: None,
        branch_vote: None,
        runner_id,
        model_used,
        mcp_context_used,
        sandbox_attestation: None,
        artifacts: Vec::new(),
        git_result: None,
        object_artifacts: Vec::new(),
        checkpoints: Vec::new(),
        test_evidence: Vec::new(),
        child_requests: Vec::new(),
        delayed_compute_thunks: Vec::new(),
        confidence: 0.0,
        next_actions: vec![
            "register a compatible runner".to_string(),
            "relax the task execution profile".to_string(),
            "enable local stub fallback for development".to_string(),
        ],
        diagnostics,
        notification_reports,
    }
}

fn timeout_result(
    request: AgentRunRequest,
    runner_id: Option<String>,
    model_used: Option<coat_domain::ModelCandidate>,
    mcp_context_used: Option<coat_domain::McpContextRef>,
    timeout_seconds: u64,
    diagnostics: Vec<String>,
    notification_reports: Vec<NotificationDeliveryReport>,
) -> AgentRunResult {
    AgentRunResult {
        task_id: request.task.id,
        status: WorkerRunStatus::TimedOut,
        summary: format!("runner timed out after {timeout_seconds}s"),
        review: None,
        research: None,
        branch_vote: None,
        runner_id,
        model_used,
        mcp_context_used,
        sandbox_attestation: None,
        artifacts: Vec::new(),
        git_result: None,
        object_artifacts: Vec::new(),
        checkpoints: Vec::new(),
        test_evidence: Vec::new(),
        child_requests: Vec::new(),
        delayed_compute_thunks: Vec::new(),
        confidence: 0.0,
        next_actions: vec![
            "restart the task if policy allows".to_string(),
            "increase timeout policy if the work is expected to run longer".to_string(),
            "route the task to a faster or more capable runner".to_string(),
        ],
        diagnostics,
        notification_reports,
    }
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn configured_goal_store_url(raw: Option<String>) -> Option<String> {
    match raw.map(|value| value.trim().to_string()) {
        Some(value)
            if value.is_empty()
                || value.eq_ignore_ascii_case("disabled")
                || value.eq_ignore_ascii_case("none") =>
        {
            None
        }
        Some(value) => Some(value),
        None => Some(DEFAULT_GOAL_STORE_URL.to_string()),
    }
}

fn restate_identity_keys() -> Vec<String> {
    let mut keys = Vec::new();
    for name in ["RESTATE_IDENTITY_KEYS", "RESTATE_SIGNING_PUBLIC_KEY"] {
        if let Ok(raw) = std::env::var(name) {
            keys.extend(
                raw.split([',', ' ', '\n', '\t'])
                    .map(str::trim)
                    .filter(|key| !key.is_empty())
                    .map(ToOwned::to_owned),
            );
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use coat_domain::{
        ChildTaskRequest, ContinuationBoundary, ContinuationRef, ContinuationResumeAction,
        DelayedComputeThunkKind, DelayedComputeThunkResumeRequest, RestartReason, RestartScope,
        TaskPriority, WaitRef, WaitRefKind, WorkerKind,
    };

    fn human_input_thunk_request(
        state: &GoalState,
        task_id: coat_domain::TaskId,
        continuation_id: &str,
    ) -> DelayedComputeThunkRequest {
        DelayedComputeThunkRequest {
            goal_id: state.goal.id,
            task_id: Some(task_id),
            kind: DelayedComputeThunkKind::HumanInput,
            reason: "need operator input".to_string(),
            requested_input: Some("choose route".to_string()),
            wait_ref: Some(WaitRef {
                kind: WaitRefKind::HumanThread,
                reference: "thread://operator/runtime-verifier".to_string(),
            }),
            continuation: ContinuationRef {
                continuation_id: continuation_id.to_string(),
                boundary: ContinuationBoundary::TaskDispatch,
                state_ref: format!("goal/{}/task/{task_id}", state.goal.id),
                resume_actions: vec![
                    ContinuationResumeAction::ApplyFeedback,
                    ContinuationResumeAction::MarkRunnable,
                ],
            },
            timeout_seconds: Some(300),
        }
    }

    #[test]
    fn goal_store_projection_defaults_to_local_read_model() {
        assert_eq!(
            configured_goal_store_url(None).as_deref(),
            Some(DEFAULT_GOAL_STORE_URL)
        );
        assert_eq!(
            configured_goal_store_url(Some(" http://goal-store:9088 ".to_string())).as_deref(),
            Some("http://goal-store:9088")
        );
        assert_eq!(
            configured_goal_store_url(Some("disabled".to_string())),
            None
        );
        assert_eq!(configured_goal_store_url(Some(String::new())), None);
    }

    #[test]
    fn steering_drive_policy_only_runs_progressing_controls() {
        assert!(steering_should_drive(&SteeringDirectiveKind::InjectTask {
            role: coat_domain::WorkerKind::Planner,
            prompt: "recover blocked work".to_string(),
            reason: "operator requested recovery".to_string(),
        }));
        assert!(steering_should_drive(
            &SteeringDirectiveKind::RequestResearch {
                question: "what changed?".to_string(),
                reason: "operator requested current facts".to_string(),
            }
        ));
        assert!(steering_should_drive(&SteeringDirectiveKind::Resume {
            reason: "continue".to_string(),
        }));
        assert!(!steering_should_drive(&SteeringDirectiveKind::Pause {
            reason: "hold".to_string(),
        }));
        assert!(!steering_should_drive(&SteeringDirectiveKind::Cancel {
            reason: "stop".to_string(),
        }));
        assert!(!steering_should_drive(
            &SteeringDirectiveKind::EvaluateGoalCompletion {
                reason: "inspect only".to_string(),
            }
        ));
    }

    #[test]
    fn missing_goal_state_is_explicit_terminal_read_error() {
        let error = require_goal_state(None, "status")
            .err()
            .expect("missing goal state must be explicit");
        let error_ref: &(dyn std::error::Error + Send + Sync) = error.as_ref();
        let message = error_ref.to_string();

        assert!(message.contains("Terminal error [404]"));
        assert!(message.contains("GoalWorkflow.status has no initialized goal state"));
        assert!(!message.contains("null"));
    }

    #[test]
    fn missing_goal_state_error_is_shared_by_mutation_handlers() {
        for handler in [
            "cancel",
            "inject_feedback",
            "steer",
            "approve",
            "branch",
            "select_branch",
            "vote",
            "restart",
            "resume_thunk",
            "create_thunk",
            "mechanism_start",
            "mechanism_ballot",
        ] {
            let error = require_goal_state(None, handler)
                .err()
                .expect("missing goal state must be explicit");
            let error_ref: &(dyn std::error::Error + Send + Sync) = error.as_ref();
            let message = error_ref.to_string();
            assert!(
                message.contains(&format!(
                    "GoalWorkflow.{handler} has no initialized goal state"
                )),
                "{handler} returned unexpected error: {message}"
            );
            assert!(message.contains("Terminal error [404]"));
        }
    }

    #[test]
    fn missing_goal_state_error_is_shared_by_read_handlers() {
        for handler in ["status", "progress", "compute_graph", "tasks"] {
            let error = require_goal_state(None, handler)
                .err()
                .expect("missing goal state must be explicit");
            let error_ref: &(dyn std::error::Error + Send + Sync) = error.as_ref();
            let message = error_ref.to_string();
            assert!(
                message.contains(&format!(
                    "GoalWorkflow.{handler} has no initialized goal state"
                )),
                "{handler} returned unexpected error: {message}"
            );
            assert!(message.contains("Terminal error [404]"));
        }
    }

    #[test]
    fn cancelled_control_skip_is_explicit() {
        assert_eq!(
            cancelled_mutation_skip(
                &coat_domain::GoalStatus::Cancelled,
                "restart_skipped:goal_cancelled",
                "restart_skipped_goal_cancelled",
            ),
            Some(SkippedMutation {
                event: "restart_skipped:goal_cancelled",
                transition: "restart_skipped_goal_cancelled",
            })
        );
        assert_eq!(
            cancelled_mutation_skip(
                &coat_domain::GoalStatus::Blocked,
                "restart_skipped:goal_cancelled",
                "restart_skipped_goal_cancelled",
            ),
            None
        );
    }

    #[test]
    fn stale_control_skips_are_explicit_noops() {
        assert_eq!(
            restart_stale_skip(&DomainError::RestartDenied(
                "no restartable tasks matched the request".to_string(),
            )),
            Some(SkippedMutation {
                event: "restart_skipped:no_restartable_tasks",
                transition: "restart_skipped_no_restartable_tasks",
            })
        );
        assert_eq!(
            restart_stale_skip(&DomainError::RestartDenied(
                "restart policy is disabled".to_string(),
            )),
            None
        );
        assert_eq!(
            resume_thunk_stale_skip(&DomainError::SteeringDenied(
                "delayed compute thunk 018f8f2f-1fd8-7688-bb12-8bfb6b756602 is not pending"
                    .to_string(),
            )),
            Some(SkippedMutation {
                event: "resume_thunk_skipped:not_pending",
                transition: "resume_thunk_skipped_not_pending",
            })
        );
    }

    #[test]
    fn duplicate_create_thunk_is_explicit_noop() {
        let mut state = GoalState::new(GoalSpec::new(
            "duplicate thunk",
            "prove duplicate delayed compute thunk creation is not repeated",
        ));
        let task_id = *state.tasks.keys().next().expect("root task");
        let request = human_input_thunk_request(&state, task_id, "runtime-verifier/operator-input");
        state.create_delayed_compute_thunk(request).expect("thunk");

        assert_eq!(
            create_thunk_duplicate_skip(&state, "runtime-verifier/operator-input"),
            Some(SkippedMutation {
                event: "create_thunk_skipped:continuation_exists",
                transition: "create_thunk_skipped_continuation_exists",
            })
        );
        assert_eq!(
            create_thunk_duplicate_skip(&state, "runtime-verifier/other-input"),
            None
        );
    }

    #[test]
    fn cancelled_create_thunk_does_not_block_fresh_wait_with_same_continuation() {
        let mut state = GoalState::new(GoalSpec::new(
            "recreated thunk",
            "old cancelled waits should not suppress a new pending wait",
        ));
        let task_id = *state.tasks.keys().next().expect("root task");
        let request = human_input_thunk_request(&state, task_id, "runtime-verifier/operator-input");
        state.create_delayed_compute_thunk(request).expect("thunk");
        state.delayed_compute_thunks[0].status = coat_domain::DelayedComputeThunkStatus::Cancelled;

        assert_eq!(
            create_thunk_duplicate_skip(&state, "runtime-verifier/operator-input"),
            None
        );
    }

    #[test]
    fn frontier_idle_status_keeps_waiting_thunks_paused() {
        let mut state = GoalState::new(GoalSpec::new(
            "waiting thunk",
            "prove waiting delayed compute thunks do not become blocked at idle",
        ));
        let task_id = *state.tasks.keys().next().expect("root task");
        let request = human_input_thunk_request(&state, task_id, "runtime-verifier/operator-input");
        state.create_delayed_compute_thunk(request).expect("thunk");

        assert_eq!(
            frontier_idle_status(&state),
            coat_domain::GoalStatus::Paused
        );
    }

    #[test]
    fn cancelled_goal_stays_terminal_and_mutation_skips_do_not_reopen_it() {
        let mut state = GoalState::new(GoalSpec::new(
            "cancelled terminal",
            "cancelled goals must not be reopened by repeatable controls",
        ));
        let task_id = *state.tasks.keys().next().expect("root task");
        let thunk = state
            .create_delayed_compute_thunk(human_input_thunk_request(
                &state,
                task_id,
                "runtime-verifier/operator-input",
            ))
            .expect("thunk");
        state.cancel("operator stopped the goal");

        assert_eq!(state.status, coat_domain::GoalStatus::Cancelled);
        assert_eq!(
            state.tasks.get(&task_id).expect("task").status,
            TaskStatus::Cancelled
        );
        assert_eq!(
            state.delayed_compute_thunks[0].status,
            coat_domain::DelayedComputeThunkStatus::Cancelled
        );
        assert_eq!(state.progress().status, coat_domain::GoalStatus::Cancelled);
        assert_eq!(state.progress().open_tasks, 0);

        assert_eq!(
            cancelled_mutation_skip(
                &state.status,
                "restart_skipped:goal_cancelled",
                "restart_skipped_goal_cancelled",
            ),
            Some(SkippedMutation {
                event: "restart_skipped:goal_cancelled",
                transition: "restart_skipped_goal_cancelled",
            })
        );
        let restart_error = state
            .apply_restart_request(RestartRequest {
                goal_id: state.goal.id,
                scope: RestartScope::Task,
                reason: RestartReason::OperatorRequested,
                message: "try to reopen cancelled work".to_string(),
                task_id: Some(task_id),
                reset_attempts: None,
                preserve_artifacts: None,
                operator: Some("tester".to_string()),
            })
            .expect_err("cancelled goal restart should be explicit");
        assert!(
            matches!(
                restart_error,
                DomainError::RestartDenied(ref message)
                    if message == "goal is terminal: Cancelled"
            ),
            "unexpected restart error: {restart_error}"
        );
        let resume_error = state
            .resume_delayed_compute_thunk(DelayedComputeThunkResumeRequest {
                thunk_id: thunk.id,
                responder: "operator".to_string(),
                response_summary: "stale response after cancellation".to_string(),
                artifact_refs: Vec::new(),
            })
            .expect_err("cancelled thunk resume should be explicit");
        assert!(
            matches!(
                resume_error,
                DomainError::SteeringDenied(ref message)
                    if message == &format!("delayed compute thunk {} is not pending", thunk.id)
            ),
            "unexpected resume error: {resume_error}"
        );
        assert_eq!(state.status, coat_domain::GoalStatus::Cancelled);
        assert_eq!(
            state.tasks.get(&task_id).expect("task").status,
            TaskStatus::Cancelled
        );
    }

    #[test]
    fn serialized_restart_steer_resume_and_reads_project_same_state() {
        let mut state = GoalState::new(GoalSpec::new(
            "serialized controls",
            "repeatable controls should mutate coordinator state coherently",
        ));
        let task_id = *state.tasks.keys().next().expect("root task");
        let first_thunk = state
            .create_delayed_compute_thunk(human_input_thunk_request(
                &state,
                task_id,
                "runtime-verifier/operator-input",
            ))
            .expect("thunk");
        state.status = frontier_idle_status(&state);

        let waiting_projection =
            GoalStoreSnapshotUpsertRequest::from_state(&state, "delayed_compute_thunk_created");
        assert_eq!(state.status, coat_domain::GoalStatus::Paused);
        assert_eq!(state.progress().waiting_input_tasks, 1);
        assert_eq!(waiting_projection.snapshot.compute_graph.open_thunks, 1);

        let restart_record = state
            .apply_restart_request(RestartRequest {
                goal_id: state.goal.id,
                scope: RestartScope::Task,
                reason: RestartReason::OperatorRequested,
                message: "restart waiting task".to_string(),
                task_id: Some(task_id),
                reset_attempts: None,
                preserve_artifacts: Some(true),
                operator: Some("tester".to_string()),
            })
            .expect("restart waiting task");
        assert_eq!(restart_record.restarted_task_ids, vec![task_id]);
        assert_eq!(
            state.delayed_compute_thunks[0].status,
            coat_domain::DelayedComputeThunkStatus::Cancelled
        );
        assert_eq!(
            state.tasks.get(&task_id).expect("task").status,
            TaskStatus::Runnable
        );
        assert_eq!(state.status, coat_domain::GoalStatus::Running);

        state
            .apply_steering(
                SteeringDirective {
                    id: state.goal.id,
                    goal_id: state.goal.id,
                    task_id: None,
                    operator: Some("tester".to_string()),
                    message: "pause before resuming".to_string(),
                    kind: SteeringDirectiveKind::Pause {
                        reason: "operator hold".to_string(),
                    },
                },
                &SpawnPolicy::default(),
            )
            .expect("pause");
        assert_eq!(state.status, coat_domain::GoalStatus::Paused);
        state
            .apply_steering(
                SteeringDirective {
                    id: state.goal.id,
                    goal_id: state.goal.id,
                    task_id: None,
                    operator: Some("tester".to_string()),
                    message: "resume after hold".to_string(),
                    kind: SteeringDirectiveKind::Resume {
                        reason: "operator continue".to_string(),
                    },
                },
                &SpawnPolicy::default(),
            )
            .expect("resume");
        assert_eq!(state.status, coat_domain::GoalStatus::Running);

        let second_thunk = state
            .create_delayed_compute_thunk(human_input_thunk_request(
                &state,
                task_id,
                "runtime-verifier/second-input",
            ))
            .expect("second thunk");
        state
            .resume_delayed_compute_thunk(DelayedComputeThunkResumeRequest {
                thunk_id: second_thunk.id,
                responder: "operator".to_string(),
                response_summary: "continue".to_string(),
                artifact_refs: Vec::new(),
            })
            .expect("resume pending thunk");

        let progress = state.progress();
        let graph = state.compute_graph();
        let task_list = state.find_tasks(&TaskQuery::default());
        let projection = GoalStoreSnapshotUpsertRequest::from_state(&state, "resume_thunk");

        assert_eq!(progress.status, state.status);
        assert_eq!(progress.total_tasks, state.tasks.len() as u32);
        assert_eq!(progress.pending_delayed_compute_thunks, 0);
        assert_eq!(progress.compute_graph, graph);
        assert_eq!(task_list.progress, progress);
        assert_eq!(projection.snapshot.goal.status, state.status);
        assert_eq!(projection.snapshot.goal.open_tasks, progress.open_tasks);
        assert_eq!(projection.snapshot.compute_graph.open_thunks, 0);

        let stale_resume = state
            .resume_delayed_compute_thunk(DelayedComputeThunkResumeRequest {
                thunk_id: second_thunk.id,
                responder: "operator".to_string(),
                response_summary: "duplicate continue".to_string(),
                artifact_refs: Vec::new(),
            })
            .expect_err("duplicate resume should be explicit");
        assert_eq!(
            resume_thunk_stale_skip(&stale_resume),
            Some(SkippedMutation {
                event: "resume_thunk_skipped:not_pending",
                transition: "resume_thunk_skipped_not_pending",
            })
        );
        let stale_restart = state
            .apply_restart_request(RestartRequest {
                goal_id: state.goal.id,
                scope: RestartScope::Blocked,
                reason: RestartReason::OperatorRequested,
                message: "stale blocked restart".to_string(),
                task_id: None,
                reset_attempts: None,
                preserve_artifacts: None,
                operator: Some("tester".to_string()),
            })
            .expect_err("stale blocked restart should be explicit");
        assert!(
            matches!(
                stale_restart,
                DomainError::RestartDenied(ref message)
                    if message == "no restartable tasks matched the request"
            ),
            "unexpected restart error: {stale_restart}"
        );
        assert_eq!(
            first_thunk.status,
            coat_domain::DelayedComputeThunkStatus::Pending
        );
    }

    #[test]
    fn transition_observation_captures_approval_pause() {
        let mut state = GoalState::new(GoalSpec::new(
            "approval observation",
            "prove approval pauses are observable",
        ));
        let task_id = *state.tasks.keys().next().expect("root task");
        let sandbox = state.tasks[&task_id].sandbox.clone();
        state.tasks.get_mut(&task_id).expect("task").status = TaskStatus::WaitingApproval;
        state.status = coat_domain::GoalStatus::WaitingApproval;
        state.approvals.push(ApprovalRequest {
            id: state.goal.id,
            goal_id: state.goal.id,
            task_id: Some(task_id),
            attempt: 0,
            reason: "network open".to_string(),
            status: coat_domain::ApprovalStatus::Pending,
            risk: coat_domain::ApprovalRisk::High,
            reason_codes: vec![coat_domain::ApprovalReasonCode::NetworkOpen],
            sandbox,
            requested_action: "run networked task".to_string(),
            notification_reports: Vec::new(),
        });

        let observation = observe_transition(&state, "approval_requested");

        assert_eq!(observation.reason, "approval_requested");
        assert_eq!(observation.status, coat_domain::GoalStatus::WaitingApproval);
        assert_eq!(observation.total_tasks, 1);
        assert_eq!(observation.runnable_tasks, 0);
        assert_eq!(observation.waiting_approval_tasks, 1);
        assert_eq!(observation.waiting_input_tasks, 0);
        assert_eq!(observation.pending_approvals, 1);
        assert_eq!(observation.pending_delayed_compute_thunks, 0);
        assert_eq!(observation.compute_graph_nodes, 2);
        assert_eq!(observation.compute_graph_edges, 1);
    }

    #[test]
    fn transition_observation_captures_delayed_compute_graph_state() {
        let mut state = GoalState::new(GoalSpec::new(
            "thunk observation",
            "prove suspended continuations are observable",
        ));
        let task_id = *state.tasks.keys().next().expect("root task");
        let request = human_input_thunk_request(&state, task_id, "runtime-verifier/operator-input");
        state.create_delayed_compute_thunk(request).expect("thunk");

        let observation = observe_transition(&state, "delayed_compute_thunk_created");

        assert_eq!(observation.waiting_input_tasks, 1);
        assert_eq!(observation.pending_delayed_compute_thunks, 1);
        assert_eq!(observation.compute_graph_nodes, 5);
        assert_eq!(observation.compute_graph_edges, 4);
    }

    #[test]
    fn projection_captures_blocked_tasks_waiting_tasks_and_thunks() {
        let mut goal = GoalSpec::new(
            "blocked waiting projection",
            "prove blocked and waiting delayed compute work are visible in projections.",
        );
        goal.initial_tasks.push(ChildTaskRequest {
            role: WorkerKind::Codex,
            purpose: None,
            title: Some("operator input child".to_string()),
            subgoal_id: None,
            color: None,
            prompt: "wait for operator input".to_string(),
            reason: "test projection state".to_string(),
            dependencies: Vec::new(),
            budget: None,
            sandbox: None,
            done_criteria: None,
            review_doctrine: None,
            execution: None,
            priority: TaskPriority::Normal,
            tags: Vec::new(),
        });
        let mut state = GoalState::new(goal);
        let root_task_id = state
            .tasks
            .values()
            .find(|task| task.parent_id.is_none())
            .expect("root task")
            .id;
        let waiting_task_id = state
            .tasks
            .values()
            .find(|task| task.parent_id == Some(root_task_id))
            .expect("child task")
            .id;
        state
            .tasks
            .get_mut(&root_task_id)
            .expect("root task")
            .status = TaskStatus::Blocked;
        let request =
            human_input_thunk_request(&state, waiting_task_id, "runtime-verifier/operator-input");
        state.create_delayed_compute_thunk(request).expect("thunk");
        state.status = frontier_idle_status(&state);

        let projection =
            coat_domain::GoalStoreSnapshotUpsertRequest::from_state(&state, "frontier_idle");

        assert_eq!(
            projection.snapshot.goal.status,
            coat_domain::GoalStatus::Blocked
        );
        assert_eq!(projection.snapshot.goal.blocked_tasks, 1);
        assert_eq!(projection.snapshot.compute_graph.open_thunks, 1);
        assert_eq!(
            projection.snapshot.compute_graph.waiting_tasks,
            vec![waiting_task_id]
        );
        assert_eq!(projection.snapshot.compute_graph.nodes.len(), 6);
        assert_eq!(projection.snapshot.compute_graph.edges.len(), 5);
    }

    #[test]
    fn transition_observation_captures_terminal_mix() {
        let mut state = GoalState::new(GoalSpec::new(
            "terminal observation",
            "prove final task outcomes are observable",
        ));
        let task_id = *state.tasks.keys().next().expect("root task");
        state.tasks.get_mut(&task_id).expect("task").status = TaskStatus::Done;
        state.status = coat_domain::GoalStatus::Done;
        let initial_event_count = state.events.len();
        state
            .events
            .push(coat_domain::StateEvent::new("validation_applied"));

        let observation = observe_transition(&state, "done");

        assert_eq!(observation.status, coat_domain::GoalStatus::Done);
        assert_eq!(observation.done_tasks, 1);
        assert_eq!(observation.blocked_tasks, 0);
        assert_eq!(observation.failed_tasks, 0);
        assert_eq!(observation.compute_graph_nodes, 2);
        assert_eq!(observation.compute_graph_edges, 1);
        assert_eq!(observation.event_count, initial_event_count + 1);
    }
}

#[tokio::main]
async fn main() {
    coat_observability::init_tracing("coat-coordinator", "coat_coordinator=info,restate_sdk=info");

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9080".to_string());
    let mut endpoint = Endpoint::builder()
        .bind(GoalWorkflowImpl::default().serve())
        .bind(AgentRunnerImpl::default().serve())
        .bind(ValidationServiceImpl.serve());
    for identity_key in restate_identity_keys() {
        endpoint = endpoint
            .identity_key(&identity_key)
            .expect("valid Restate identity key");
    }
    HttpServer::new(endpoint.build())
        .listen_and_serve(bind.parse().expect("valid BIND_ADDR"))
        .await;
}
