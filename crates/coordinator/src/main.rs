use coat_domain::{
    AgentRunRequest, AgentRunResult, ApprovalRequest, BranchRequest, BranchSelectionRequest,
    ControlLoopMode, DomainError, GoalProgress, GoalSpec, GoalState,
    GoalStoreSnapshotUpsertRequest, HumanApproval, HumanFeedback, NotificationDeliveryReport,
    NotificationEvent, NotificationRequest, RestartRequest, RunnerDispatchDecision,
    RunnerDispatchRequest, RunnerDispatchStatus, SpawnPolicy, SteeringDirective, TaskList,
    TaskQuery, TaskStatus, ValidationReport, ValidationRequest, WorkerRunStatus,
};
use restate_sdk::{prelude::*, serde::Json};

const STATE_KEY: &str = "state";
const MAX_FRONTIER_ROUNDS: usize = 32;

#[restate_sdk::workflow]
pub trait GoalWorkflow {
    async fn run(goal: Json<GoalSpec>) -> HandlerResult<Json<GoalState>>;

    #[shared]
    async fn cancel(reason: String) -> HandlerResult<String>;

    #[shared]
    async fn inject_feedback(feedback: Json<HumanFeedback>) -> HandlerResult<String>;

    async fn steer(directive: Json<SteeringDirective>) -> HandlerResult<Json<Option<GoalState>>>;

    async fn approve(approval: Json<HumanApproval>) -> HandlerResult<String>;

    async fn restart(request: Json<RestartRequest>) -> HandlerResult<Json<Option<GoalState>>>;

    async fn branch(request: Json<BranchRequest>) -> HandlerResult<Json<Option<GoalState>>>;

    async fn select_branch(
        request: Json<BranchSelectionRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>>;

    #[shared]
    async fn status() -> HandlerResult<Json<Option<GoalState>>>;

    #[shared]
    async fn progress() -> HandlerResult<Json<Option<GoalProgress>>>;

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
            goal_store_url: std::env::var("COAT_GOAL_STORE_URL")
                .ok()
                .filter(|url| !url.trim().is_empty()),
            goal_store_required: env_bool("COAT_GOAL_STORE_REQUIRED", false),
        }
    }
}

impl GoalWorkflowImpl {
    async fn drive_state(
        &self,
        ctx: &WorkflowContext<'_>,
        mut state: GoalState,
    ) -> HandlerResult<GoalState> {
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(ctx, &state, "drive_start").await?;
        let max_frontier_rounds = state.goal.control_policy.max_frontier_rounds as usize;

        for _round in 0..max_frontier_rounds.min(MAX_FRONTIER_ROUNDS) {
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
                state.status = coat_domain::GoalStatus::Failed;
                ctx.set(STATE_KEY, Json(state.clone()));
                self.project_state(ctx, &state, "budget_exhausted").await?;
                return Err(TerminalError::new("budget exhausted").into());
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
                if state
                    .tasks
                    .values()
                    .any(|task| task.status == TaskStatus::WaitingApproval)
                {
                    state.status = coat_domain::GoalStatus::WaitingApproval;
                } else if matches!(
                    state.goal.control_policy.mode,
                    ControlLoopMode::MonitorUntilCancelled
                        | ControlLoopMode::HumanSteeredContinuous
                ) {
                    state
                        .events
                        .push(coat_domain::StateEvent::new("control_loop_idle"));
                    state.status = coat_domain::GoalStatus::Running;
                } else if !state.is_done() {
                    state.status = coat_domain::GoalStatus::Blocked;
                }
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
        ctx: &WorkflowContext<'_>,
        state: &GoalState,
        reason: &'static str,
    ) -> HandlerResult<()> {
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
}

impl GoalWorkflow for GoalWorkflowImpl {
    async fn run(
        &self,
        ctx: WorkflowContext<'_>,
        goal: Json<GoalSpec>,
    ) -> HandlerResult<Json<GoalState>> {
        let incoming_goal = goal.into_inner();
        let state = match ctx.get::<Json<GoalState>>(STATE_KEY).await? {
            Some(Json(existing)) if existing.goal.id == incoming_goal.id => existing,
            _ => GoalState::new(incoming_goal),
        };
        Ok(Json(self.drive_state(&ctx, state).await?))
    }

    async fn cancel(
        &self,
        ctx: SharedWorkflowContext<'_>,
        reason: String,
    ) -> HandlerResult<String> {
        ctx.resolve_promise("cancel", reason.clone());
        Ok(format!("cancel requested: {reason}"))
    }

    async fn inject_feedback(
        &self,
        ctx: SharedWorkflowContext<'_>,
        feedback: Json<HumanFeedback>,
    ) -> HandlerResult<String> {
        ctx.resolve_promise("feedback", feedback.into_inner().message);
        Ok("feedback accepted".to_string())
    }

    async fn steer(
        &self,
        ctx: WorkflowContext<'_>,
        directive: Json<SteeringDirective>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let Some(Json(mut state)) = ctx.get::<Json<GoalState>>(STATE_KEY).await? else {
            return Ok(Json(None));
        };
        state
            .apply_steering(directive.into_inner(), &self.spawn_policy)
            .map_err(domain_error)?;
        ctx.set(STATE_KEY, Json(state.clone()));
        Ok(Json(Some(state)))
    }

    async fn approve(
        &self,
        ctx: WorkflowContext<'_>,
        approval: Json<HumanApproval>,
    ) -> HandlerResult<String> {
        let approval = approval.into_inner();
        let Some(Json(mut state)) = ctx.get::<Json<GoalState>>(STATE_KEY).await? else {
            return Err(TerminalError::new("goal state is not initialized").into());
        };
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
        ctx: WorkflowContext<'_>,
        request: Json<RestartRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let Some(Json(mut state)) = ctx.get::<Json<GoalState>>(STATE_KEY).await? else {
            return Ok(Json(None));
        };
        state
            .apply_restart_request(request.into_inner())
            .map_err(domain_error)?;
        let state = self.drive_state(&ctx, state).await?;
        Ok(Json(Some(state)))
    }

    async fn branch(
        &self,
        ctx: WorkflowContext<'_>,
        request: Json<BranchRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let Some(Json(mut state)) = ctx.get::<Json<GoalState>>(STATE_KEY).await? else {
            return Ok(Json(None));
        };
        state
            .branch_task(request.into_inner(), &self.spawn_policy)
            .map_err(domain_error)?;
        ctx.set(STATE_KEY, Json(state.clone()));
        self.project_state(&ctx, &state, "branch_created").await?;
        Ok(Json(Some(state)))
    }

    async fn select_branch(
        &self,
        ctx: WorkflowContext<'_>,
        request: Json<BranchSelectionRequest>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        let Some(Json(mut state)) = ctx.get::<Json<GoalState>>(STATE_KEY).await? else {
            return Ok(Json(None));
        };
        state
            .apply_branch_selection(request.into_inner())
            .map_err(domain_error)?;
        let state = self.drive_state(&ctx, state).await?;
        Ok(Json(Some(state)))
    }

    async fn status(
        &self,
        ctx: SharedWorkflowContext<'_>,
    ) -> HandlerResult<Json<Option<GoalState>>> {
        Ok(Json(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner),
        ))
    }

    async fn progress(
        &self,
        ctx: SharedWorkflowContext<'_>,
    ) -> HandlerResult<Json<Option<GoalProgress>>> {
        Ok(Json(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner)
                .map(|state| state.progress()),
        ))
    }

    async fn tasks(
        &self,
        ctx: SharedWorkflowContext<'_>,
        query: Json<TaskQuery>,
    ) -> HandlerResult<Json<Option<TaskList>>> {
        Ok(Json(
            ctx.get::<Json<GoalState>>(STATE_KEY)
                .await?
                .map(Json::into_inner)
                .map(|state| state.find_tasks(&query.into_inner())),
        ))
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
            registry_url: std::env::var("COAT_RUNNER_REGISTRY")
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
            "Approval {} is required to {}. Risk: {:?}. Reason: {}. Approve with: coat approve --goal-id {} --approval-id {} --approved true",
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
        test_evidence: Vec::new(),
        child_requests: Vec::new(),
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
        test_evidence: Vec::new(),
        child_requests: Vec::new(),
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

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "coat_coordinator=info,restate_sdk=info".to_string()),
        )
        .init();

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
