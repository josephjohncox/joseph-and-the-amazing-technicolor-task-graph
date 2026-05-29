use coat_domain::{
    AgentRunResult, ApprovalPolicy, ApprovalStatus, ContinuationBoundary, ContinuationRef,
    ContinuationResumeAction, DelayedComputeThunk, DelayedComputeThunkKind,
    DelayedComputeThunkRequest, DelayedComputeThunkResumeRequest, DelayedComputeThunkStatus,
    DomainError, GoalSpec, GoalState, GoalStatus, GoalStoreSnapshot, HumanApproval, OperatorActor,
    OperatorTransition, RestartReason, RestartRequest, RestartScope, SpawnPolicy, TaskActor,
    TaskId, TaskStatus, ValidationReport, ValidationRequest, WaitRef, WaitRefKind, WorkerRunStatus,
};
use proptest::prelude::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
enum Transition {
    MarkRunnableRunning,
    WorkerDone,
    WorkerWaitingWithThunk,
    WorkerWaitingWithoutThunk,
    WorkerBlocked,
    WorkerFailed,
    ValidateNeedsValidation,
    RequestApproval,
    ApprovePending,
    RejectPending,
    ResumePendingThunk,
    RestartBlocked,
    RestartFailed,
    CancelGoal,
}

fn transition_strategy() -> impl Strategy<Value = Transition> {
    prop_oneof![
        Just(Transition::MarkRunnableRunning),
        Just(Transition::WorkerDone),
        Just(Transition::WorkerWaitingWithThunk),
        Just(Transition::WorkerWaitingWithoutThunk),
        Just(Transition::WorkerBlocked),
        Just(Transition::WorkerFailed),
        Just(Transition::ValidateNeedsValidation),
        Just(Transition::RequestApproval),
        Just(Transition::ApprovePending),
        Just(Transition::RejectPending),
        Just(Transition::ResumePendingThunk),
        Just(Transition::RestartBlocked),
        Just(Transition::RestartFailed),
        Just(Transition::CancelGoal),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn generated_operator_worker_sequences_preserve_state_invariants(
        transitions in prop::collection::vec(transition_strategy(), 1..48),
    ) {
        let mut goal = GoalSpec::new(
            "state-machine-validation",
            "validate generated operator and worker transitions",
        );
        goal.restart_policy.max_goal_restarts = 128;
        goal.restart_policy.max_task_restarts = 128;

        let mut state = GoalState::new(goal);
        assert_state_invariants(&state);

        for transition in transitions {
            apply_transition(&mut state, transition);
            assert_state_invariants(&state);
        }
    }
}

fn apply_transition(state: &mut GoalState, transition: Transition) {
    if state.status == GoalStatus::Cancelled {
        return;
    }

    match transition {
        Transition::MarkRunnableRunning => {
            if let Some(task_id) = state.runnable_tasks().first().map(|task| task.id) {
                let _ = state.mark_running(task_id);
            }
        }
        Transition::WorkerDone => {
            if let Some(task_id) = dispatchable_task_id(state) {
                apply_worker_status(state, task_id, WorkerRunStatus::Done, None);
            }
        }
        Transition::WorkerWaitingWithThunk => {
            if let Some(task_id) = dispatchable_task_id(state) {
                apply_worker_status(
                    state,
                    task_id,
                    WorkerRunStatus::Waiting,
                    Some(delayed_compute_thunk_request(state, task_id, "worker-wait")),
                );
            }
        }
        Transition::WorkerWaitingWithoutThunk => {
            if let Some(task_id) = dispatchable_task_id(state) {
                apply_worker_status(state, task_id, WorkerRunStatus::Waiting, None);
            }
        }
        Transition::WorkerBlocked => {
            if let Some(task_id) = dispatchable_task_id(state) {
                apply_worker_status(state, task_id, WorkerRunStatus::Blocked, None);
            }
        }
        Transition::WorkerFailed => {
            if let Some(task_id) = dispatchable_task_id(state) {
                apply_worker_status(state, task_id, WorkerRunStatus::Failed, None);
            }
        }
        Transition::ValidateNeedsValidation => {
            if let Some(task_id) = first_task_with_status(state, TaskStatus::NeedsValidation) {
                apply_validation(state, task_id);
            }
        }
        Transition::RequestApproval => {
            if let Some(task_id) = dispatchable_task_id(state) {
                if let Some(task) = state.tasks.get_mut(&task_id) {
                    task.sandbox.approval_policy = ApprovalPolicy::Always;
                }
                let _ = state.ensure_task_approval_or_request(task_id);
            }
        }
        Transition::ApprovePending => {
            if let Some(approval_id) = first_pending_approval(state) {
                let _ = state.apply_human_approval(HumanApproval {
                    approval_id,
                    approved: true,
                    note: Some("property test approval".to_string()),
                });
            }
        }
        Transition::RejectPending => {
            if let Some(approval_id) = first_pending_approval(state) {
                let _ = state.apply_human_approval(HumanApproval {
                    approval_id,
                    approved: false,
                    note: Some("property test rejection".to_string()),
                });
            }
        }
        Transition::ResumePendingThunk => {
            if let Some(thunk) = first_pending_thunk(state).cloned() {
                let _ = state.resume_delayed_compute_thunk(DelayedComputeThunkResumeRequest {
                    thunk_id: thunk.id,
                    responder: "property-test".to_string(),
                    response_summary: "resume generated wait".to_string(),
                    artifact_refs: Vec::new(),
                });
            }
        }
        Transition::RestartBlocked => {
            let _ = state.apply_restart_request(restart_request(
                state,
                RestartScope::Blocked,
                None,
                "restart blocked generated tasks",
            ));
        }
        Transition::RestartFailed => {
            let _ = state.apply_restart_request(restart_request(
                state,
                RestartScope::Failed,
                None,
                "restart failed generated tasks",
            ));
        }
        Transition::CancelGoal => state.cancel("property test cancellation"),
    }
}

fn dispatchable_task_id(state: &GoalState) -> Option<TaskId> {
    state
        .tasks
        .values()
        .find(|task| matches!(task.status, TaskStatus::Runnable | TaskStatus::Running))
        .map(|task| task.id)
}

fn first_task_with_status(state: &GoalState, status: TaskStatus) -> Option<TaskId> {
    state
        .tasks
        .values()
        .find(|task| task.status == status)
        .map(|task| task.id)
}

fn first_pending_approval(state: &GoalState) -> Option<uuid::Uuid> {
    state
        .approvals
        .iter()
        .find(|approval| approval.status == ApprovalStatus::Pending)
        .map(|approval| approval.id)
}

fn first_pending_thunk(state: &GoalState) -> Option<&DelayedComputeThunk> {
    state
        .delayed_compute_thunks
        .iter()
        .find(|thunk| thunk.status == DelayedComputeThunkStatus::Pending)
}

fn apply_worker_status(
    state: &mut GoalState,
    task_id: TaskId,
    status: WorkerRunStatus,
    thunk_request: Option<DelayedComputeThunkRequest>,
) {
    let Some(task) = state.tasks.get(&task_id).cloned() else {
        return;
    };
    let mut result = AgentRunResult::stub_done(&task);
    result.status = status;
    result.summary = format!("generated worker status {:?}", result.status);
    result.next_actions = vec!["retry, replan, resume, or cancel".to_string()];
    if let Some(thunk_request) = thunk_request {
        result.delayed_compute_thunks.push(thunk_request);
    }
    let _ = state.apply_agent_result(result, &SpawnPolicy::default());
}

fn apply_validation(state: &mut GoalState, task_id: TaskId) {
    let Some(task) = state.tasks.get(&task_id).cloned() else {
        return;
    };
    let result = AgentRunResult::stub_done(&task);
    let report = ValidationReport::from_result(ValidationRequest {
        goal_id: state.goal.id,
        task,
        result,
    });
    let _ = state.apply_validation(report);
}

fn delayed_compute_thunk_request(
    state: &GoalState,
    task_id: TaskId,
    suffix: &str,
) -> DelayedComputeThunkRequest {
    DelayedComputeThunkRequest {
        goal_id: state.goal.id,
        task_id: Some(task_id),
        kind: DelayedComputeThunkKind::HumanInput,
        reason: format!("generated wait for {suffix}"),
        requested_input: Some(format!("Provide recovery input for task {task_id}")),
        wait_ref: Some(WaitRef {
            kind: WaitRefKind::HumanThread,
            reference: format!("goal://{}/task/{task_id}/{suffix}", state.goal.id),
        }),
        continuation: ContinuationRef {
            continuation_id: format!("goal/{}/task/{task_id}/{suffix}", state.goal.id),
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

fn restart_request(
    state: &GoalState,
    scope: RestartScope,
    task_id: Option<TaskId>,
    message: &str,
) -> RestartRequest {
    RestartRequest {
        goal_id: state.goal.id,
        scope,
        reason: RestartReason::OperatorRequested,
        message: message.to_string(),
        task_id,
        reset_attempts: Some(false),
        preserve_artifacts: Some(true),
        operator: Some("property-test".to_string()),
    }
}

fn assert_state_invariants(state: &GoalState) {
    assert_terminal_states_are_only_done_or_cancelled(state);
    assert_progress_counters_are_coherent(state);
    assert_pending_waits_have_recovery_refs(state);
    assert_recoverable_states_are_actionable(state);
    assert_cancelled_goals_leave_no_pending_waits(state);
}

fn assert_terminal_states_are_only_done_or_cancelled(state: &GoalState) {
    for task in state.tasks.values() {
        if task.status.is_terminal() {
            assert!(
                matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled),
                "task {} has unexpected terminal status {:?}",
                task.id,
                task.status
            );
        }
    }
}

fn assert_progress_counters_are_coherent(state: &GoalState) {
    let progress = state.progress();
    let mut by_status = BTreeMap::new();
    for task in state.tasks.values() {
        *by_status.entry(task.status.clone()).or_insert(0) += 1;
    }

    let total_tasks = state.tasks.len() as u32;
    let open_tasks = state
        .tasks
        .values()
        .filter(|task| !task.status.is_terminal())
        .count() as u32;
    let terminal_ok_tasks = state
        .tasks
        .values()
        .filter(|task| task.status == TaskStatus::Done)
        .count() as u32;
    let blocked_tasks = *by_status.get(&TaskStatus::Blocked).unwrap_or(&0);
    let failed_tasks = *by_status.get(&TaskStatus::Failed).unwrap_or(&0);
    let waiting_approval_tasks = *by_status.get(&TaskStatus::WaitingApproval).unwrap_or(&0);
    let waiting_input_tasks = *by_status.get(&TaskStatus::WaitingInput).unwrap_or(&0);
    let pending_thunks = state
        .delayed_compute_thunks
        .iter()
        .filter(|thunk| thunk.status == DelayedComputeThunkStatus::Pending)
        .count() as u32;

    assert_eq!(progress.total_tasks, total_tasks);
    assert_eq!(progress.open_tasks, open_tasks);
    assert_eq!(progress.terminal_ok_tasks, terminal_ok_tasks);
    assert_eq!(progress.blocked_tasks, blocked_tasks);
    assert_eq!(progress.failed_tasks, failed_tasks);
    assert_eq!(progress.waiting_approval_tasks, waiting_approval_tasks);
    assert_eq!(progress.waiting_input_tasks, waiting_input_tasks);
    assert_eq!(progress.pending_delayed_compute_thunks, pending_thunks);
    assert_eq!(progress.compute_graph.open_thunks, pending_thunks);
    assert_eq!(progress.by_status, by_status);

    let expected_percent_done = if total_tasks == 0 {
        0.0
    } else {
        terminal_ok_tasks as f32 / total_tasks as f32
    };
    assert!((progress.percent_done - expected_percent_done).abs() < f32::EPSILON);
}

fn assert_pending_waits_have_recovery_refs(state: &GoalState) {
    for thunk in state
        .delayed_compute_thunks
        .iter()
        .filter(|thunk| thunk.status == DelayedComputeThunkStatus::Pending)
    {
        assert!(!thunk.reason.trim().is_empty());
        assert!(!thunk.continuation.continuation_id.trim().is_empty());
        assert!(!thunk.continuation.state_ref.trim().is_empty());
        assert!(!thunk.continuation.resume_actions.is_empty());
        if thunk.kind == DelayedComputeThunkKind::HumanInput {
            assert!(
                thunk
                    .requested_input
                    .as_ref()
                    .is_some_and(|input| !input.trim().is_empty()),
                "pending human-input thunk {} lacks a concrete prompt",
                thunk.id
            );
        }
    }

    for approval in state
        .approvals
        .iter()
        .filter(|approval| approval.status == ApprovalStatus::Pending)
    {
        assert!(!approval.reason.trim().is_empty());
        assert!(!approval.requested_action.trim().is_empty());
        let task_id = approval
            .task_id
            .expect("generated task approval should remain task-scoped");
        assert!(
            state.tasks.contains_key(&task_id),
            "pending approval {} references missing task {}",
            approval.id,
            task_id
        );
    }
}

fn assert_recoverable_states_are_actionable(state: &GoalState) {
    for task in state.tasks.values() {
        match task.status {
            TaskStatus::WaitingInput => assert!(
                has_pending_thunk_for_task(state, task.id),
                "waiting-input task {} has no pending thunk",
                task.id
            ),
            TaskStatus::WaitingApproval => assert!(
                has_pending_approval_for_task(state, task.id)
                    || has_pending_thunk_for_task(state, task.id),
                "waiting-approval task {} has no pending approval or thunk",
                task.id
            ),
            TaskStatus::Blocked => assert!(
                has_pending_approval_for_task(state, task.id)
                    || has_pending_thunk_for_task(state, task.id)
                    || blocked_task_without_wait_has_operator_recovery(state, task.id)
                    || restart_would_succeed(state, RestartScope::Task, Some(task.id))
                    || restart_would_succeed(state, RestartScope::Blocked, None),
                "blocked task {} has no recovery action",
                task.id
            ),
            TaskStatus::Failed => assert!(
                restart_would_succeed(state, RestartScope::Task, Some(task.id))
                    || restart_would_succeed(state, RestartScope::Failed, None),
                "failed task {} is not restartable",
                task.id
            ),
            _ => {}
        }
    }
}

fn assert_cancelled_goals_leave_no_pending_waits(state: &GoalState) {
    if state.status != GoalStatus::Cancelled {
        return;
    }

    assert!(
        state
            .delayed_compute_thunks
            .iter()
            .all(|thunk| thunk.status != DelayedComputeThunkStatus::Pending),
        "cancelled goal left a pending delayed compute thunk"
    );
    assert!(
        state
            .approvals
            .iter()
            .all(|approval| approval.status != ApprovalStatus::Pending),
        "cancelled goal left a pending approval"
    );
    assert!(
        state.tasks.values().all(|task| task.status.is_terminal()),
        "cancelled goal left a non-terminal task"
    );
}

fn has_pending_thunk_for_task(state: &GoalState, task_id: TaskId) -> bool {
    state.delayed_compute_thunks.iter().any(|thunk| {
        thunk.task_id == Some(task_id) && thunk.status == DelayedComputeThunkStatus::Pending
    })
}

fn has_pending_approval_for_task(state: &GoalState, task_id: TaskId) -> bool {
    state.approvals.iter().any(|approval| {
        approval.task_id == Some(task_id) && approval.status == ApprovalStatus::Pending
    })
}

fn blocked_task_without_wait_has_operator_recovery(state: &GoalState, task_id: TaskId) -> bool {
    let snapshot = GoalStoreSnapshot::from_state(state);
    let Some(task) = snapshot.tasks.iter().find(|task| task.task_id == task_id) else {
        return false;
    };
    if task.status != TaskStatus::Blocked {
        return false;
    }
    let Err(rejection) = TaskActor(task).can_apply(&OperatorTransition::TaskDispatched) else {
        return false;
    };
    let actions: Vec<_> = rejection
        .recovery_hints
        .iter()
        .map(|hint| hint.action.as_str())
        .collect();
    ["retry", "replan", "cancel_goal", "create_thunk"]
        .iter()
        .all(|action| actions.contains(action))
}

fn restart_would_succeed(state: &GoalState, scope: RestartScope, task_id: Option<TaskId>) -> bool {
    let mut clone = state.clone();
    clone
        .apply_restart_request(restart_request(
            state,
            scope,
            task_id,
            "restart dry-run for generated state",
        ))
        .is_ok()
}

#[test]
fn terminal_done_goal_rejects_stale_worker_validation_and_wait_mutations() {
    let mut state = completed_goal_state("terminal done");
    let task = state.tasks.values().next().expect("root task").clone();
    assert_eq!(state.status, GoalStatus::Done);
    assert_eq!(task.status, TaskStatus::Done);

    let snapshot_before = GoalStoreSnapshot::from_state(&state);
    let stale_result = AgentRunResult::stub_done(&task);
    assert_terminal_mutation_rejected(
        state.apply_agent_result(stale_result.clone(), &SpawnPolicy::default()),
        "goal is terminal",
    );
    assert_terminal_mutation_rejected(
        state.apply_validation(ValidationReport::from_result(ValidationRequest {
            goal_id: state.goal.id,
            task: task.clone(),
            result: stale_result,
        })),
        "goal is terminal",
    );
    assert_terminal_mutation_rejected(state.mark_running(task.id), "goal is terminal");
    assert_terminal_mutation_rejected(
        state.create_delayed_compute_thunk(delayed_compute_thunk_request(
            &state,
            task.id,
            "stale-done",
        )),
        "goal is terminal",
    );

    assert_eq!(GoalStoreSnapshot::from_state(&state), snapshot_before);
}

#[test]
fn cancelled_goal_rejects_late_worker_validation_dispatch_and_wait_mutations() {
    let mut state = GoalState::new(GoalSpec::new(
        "terminal cancelled",
        "cancelled goals should ignore late worker and operator mutations",
    ));
    let task_id = state.runnable_tasks().remove(0).id;
    state.cancel("operator stop");
    let task = state.tasks[&task_id].clone();
    assert_eq!(state.status, GoalStatus::Cancelled);
    assert_eq!(task.status, TaskStatus::Cancelled);

    let snapshot_before = GoalStoreSnapshot::from_state(&state);
    let stale_result = AgentRunResult::stub_done(&task);
    assert_terminal_mutation_rejected(
        state.apply_agent_result(stale_result.clone(), &SpawnPolicy::default()),
        "goal is terminal",
    );
    assert_terminal_mutation_rejected(
        state.apply_validation(ValidationReport::from_result(ValidationRequest {
            goal_id: state.goal.id,
            task: task.clone(),
            result: stale_result,
        })),
        "goal is terminal",
    );
    assert_terminal_mutation_rejected(state.mark_running(task.id), "goal is terminal");
    assert_terminal_mutation_rejected(
        state.create_delayed_compute_thunk(delayed_compute_thunk_request(
            &state,
            task.id,
            "stale-cancelled",
        )),
        "goal is terminal",
    );

    assert_eq!(GoalStoreSnapshot::from_state(&state), snapshot_before);
}

#[test]
fn terminal_task_rejects_late_worker_result_even_if_goal_was_not_refreshed() {
    let mut state = GoalState::new(GoalSpec::new(
        "terminal task",
        "terminal task nodes should not be reopened by stale worker results",
    ));
    let task_id = state.runnable_tasks().remove(0).id;
    state.tasks.get_mut(&task_id).expect("task").status = TaskStatus::Done;
    state.status = GoalStatus::Running;
    let task = state.tasks[&task_id].clone();
    let snapshot_before = GoalStoreSnapshot::from_state(&state);

    assert_terminal_mutation_rejected(
        state.apply_agent_result(AgentRunResult::stub_done(&task), &SpawnPolicy::default()),
        "task",
    );

    assert_eq!(GoalStoreSnapshot::from_state(&state), snapshot_before);
}

#[test]
fn goal_store_projection_is_deterministic_for_same_state() {
    let mut goal = GoalSpec::new(
        "projection determinism",
        "projecting the same durable state twice should produce identical records",
    );
    goal.restart_policy.enabled = false;
    let mut state = GoalState::new(goal);
    let task = state.runnable_tasks().remove(0);
    let result = AgentRunResult {
        status: WorkerRunStatus::Blocked,
        summary: "need operator context before continuing".to_string(),
        next_actions: vec!["answer the recovery prompt".to_string()],
        ..AgentRunResult::stub_done(&task)
    };

    state
        .apply_agent_result(result, &SpawnPolicy::default())
        .expect("blocked result gets repaired into an actionable wait");

    let first = GoalStoreSnapshot::from_state(&state);
    let second = GoalStoreSnapshot::from_state(&state);
    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_value(&first).expect("snapshot serializes"),
        serde_json::to_value(&second).expect("snapshot serializes")
    );
}

#[test]
fn non_stub_goal_cannot_be_satisfied_by_stub_result_through_state() {
    let mut goal = GoalSpec::new(
        "non stub satisfaction gate",
        "non-stub work should not be satisfiable with placeholder worker output",
    );
    goal.review_policy.enabled = false;
    goal.default_execution
        .runner
        .required_labels
        .insert("allow_stub_runners".to_string(), "false".to_string());
    let mut state = GoalState::new(goal);
    let task = state.runnable_tasks().remove(0);
    let result = AgentRunResult::stub_done(&task);
    state
        .apply_agent_result(result.clone(), &SpawnPolicy::default())
        .expect("worker result is recorded before validation rejects placeholder evidence");

    let report = ValidationReport::from_result(ValidationRequest {
        goal_id: task.goal_id,
        task,
        result,
    });
    assert!(!report.passed);
    assert!(
        report
            .missing_criteria
            .contains(&"stub_actor_output".to_string()),
        "{:?}",
        report.missing_criteria
    );
    state
        .apply_validation(report)
        .expect("failed validation is projected");

    assert_ne!(state.status, GoalStatus::Done);
    assert!(!state.satisfaction_report().satisfied);
    assert_eq!(state.progress().terminal_ok_tasks, 0);
}

fn completed_goal_state(title: &str) -> GoalState {
    let mut goal = GoalSpec::new(title, "complete once and reject stale transitions");
    goal.review_policy.enabled = false;
    let mut state = GoalState::new(goal);
    let task_id = state.runnable_tasks().remove(0).id;
    state.mark_running(task_id).expect("task starts");
    let task = state.tasks[&task_id].clone();
    let result = AgentRunResult::stub_done(&task);
    state
        .apply_agent_result(result.clone(), &SpawnPolicy::default())
        .expect("worker result applies");
    let task = state.tasks[&task_id].clone();
    state
        .apply_validation(ValidationReport::from_result(ValidationRequest {
            goal_id: state.goal.id,
            task,
            result,
        }))
        .expect("validation completes goal");
    state
}

fn assert_terminal_mutation_rejected<T>(result: Result<T, DomainError>, expected: &str) {
    let error = match result {
        Ok(_) => panic!("terminal mutation should reject"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains(expected),
        "expected error to contain {expected:?}, got {error}"
    );
}
