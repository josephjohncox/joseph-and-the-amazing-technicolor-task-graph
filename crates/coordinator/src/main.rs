use jattg_domain::{
    AgentRunRequest, AgentRunResult, DomainError, GoalSpec, GoalState, HumanApproval,
    HumanFeedback, SpawnPolicy, TaskStatus, ValidationReport, ValidationRequest,
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

    #[shared]
    async fn approve(approval: Json<HumanApproval>) -> HandlerResult<String>;

    #[shared]
    async fn status() -> HandlerResult<Json<Option<GoalState>>>;
}

pub struct GoalWorkflowImpl {
    spawn_policy: SpawnPolicy,
}

impl Default for GoalWorkflowImpl {
    fn default() -> Self {
        Self {
            spawn_policy: SpawnPolicy::default(),
        }
    }
}

impl GoalWorkflow for GoalWorkflowImpl {
    async fn run(
        &self,
        ctx: WorkflowContext<'_>,
        goal: Json<GoalSpec>,
    ) -> HandlerResult<Json<GoalState>> {
        let mut state = GoalState::new(goal.into_inner());
        ctx.set(STATE_KEY, Json(state.clone()));

        for _round in 0..MAX_FRONTIER_ROUNDS {
            if state.is_done() {
                ctx.set(STATE_KEY, Json(state.clone()));
                return Ok(Json(state));
            }
            if state.budget_exhausted() {
                state.status = jattg_domain::GoalStatus::Failed;
                ctx.set(STATE_KEY, Json(state.clone()));
                return Err(TerminalError::new("budget exhausted").into());
            }

            let runnable = state.runnable_tasks();
            if runnable.is_empty() {
                if state
                    .tasks
                    .values()
                    .any(|task| task.status == TaskStatus::WaitingApproval)
                {
                    state.status = jattg_domain::GoalStatus::WaitingApproval;
                } else if !state.is_done() {
                    state.status = jattg_domain::GoalStatus::Blocked;
                }
                ctx.set(STATE_KEY, Json(state.clone()));
                return Ok(Json(state));
            }

            for task in runnable {
                state.mark_running(task.id).map_err(domain_error)?;
                ctx.set(STATE_KEY, Json(state.clone()));

                let request = AgentRunRequest {
                    goal_id: state.goal.id,
                    task: task.clone(),
                    context_artifacts: state.final_artifacts.clone(),
                    coordinator_trace_id: None,
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
                        result,
                    }))
                    .call()
                    .await?
                    .into_inner();
                state.apply_validation(report).map_err(domain_error)?;
                ctx.set(STATE_KEY, Json(state.clone()));
            }
        }

        state.status = jattg_domain::GoalStatus::Blocked;
        ctx.set(STATE_KEY, Json(state.clone()));
        Ok(Json(state))
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

    async fn approve(
        &self,
        ctx: SharedWorkflowContext<'_>,
        approval: Json<HumanApproval>,
    ) -> HandlerResult<String> {
        let approval = approval.into_inner();
        ctx.resolve_promise(
            &format!("approval:{}", approval.approval_id),
            approval.approved,
        );
        Ok("approval signal accepted".to_string())
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
}

#[restate_sdk::service]
pub trait AgentRunner {
    async fn run_task(request: Json<AgentRunRequest>) -> HandlerResult<Json<AgentRunResult>>;
}

pub struct AgentRunnerImpl;

impl AgentRunner for AgentRunnerImpl {
    async fn run_task(
        &self,
        ctx: Context<'_>,
        request: Json<AgentRunRequest>,
    ) -> HandlerResult<Json<AgentRunResult>> {
        let request = request.into_inner();
        let name = format!("run_{}_task", request.task.role.as_str());
        let result = ctx
            .run(|| async move { Ok(Json(AgentRunResult::stub_done(&request.task))) })
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

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "jattg_coordinator=info,restate_sdk=info".to_string()),
        )
        .init();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9080".to_string());
    HttpServer::new(
        Endpoint::builder()
            .bind(GoalWorkflowImpl::default().serve())
            .bind(AgentRunnerImpl.serve())
            .bind(ValidationServiceImpl.serve())
            .build(),
    )
    .listen_and_serve(bind.parse().expect("valid BIND_ADDR"))
    .await;
}
