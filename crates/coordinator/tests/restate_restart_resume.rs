//! RuntimeVerifier scaffold for proving Restate restart/resume behavior.
//!
//! This file is intentionally safe for normal CI: the live proof is ignored by
//! default and also checks an explicit env gate before touching Docker. The
//! live RuntimeVerifier slice should replace the explicit ready-state failure
//! with a Testcontainers-backed process runner for the ordered harness plan below.

use coat_domain::{
    ChildTaskRequest, ContinuationBoundary, ContinuationRef, ContinuationResumeAction,
    DelayedComputeThunkKind, DelayedComputeThunkRequest, GoalSpec, GoalState, GoalStatus,
    GoalStoreSnapshotUpsertRequest, StateEvent, TaskPriority, TaskStatus, WaitRef, WaitRefKind,
    WorkerKind,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::PathBuf,
    process::Command,
};

const ENABLE_ENV: &str = "COAT_RESTATE_RESTART_RESUME_TEST";
const RESTATE_IMAGE_ENV: &str = "COAT_RESTATE_TESTCONTAINERS_IMAGE";
const DEFAULT_RESTATE_IMAGE: &str = "docker.restate.dev/restatedev/restate:1.5";
const COORDINATOR_BIN_ENV: &str = "CARGO_BIN_EXE_coat-coordinator";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeVerifierConfig {
    restate_image: String,
    coordinator_bin: PathBuf,
}

impl RuntimeVerifierConfig {
    fn from_env() -> Option<Self> {
        Self::from_values(
            env::var(ENABLE_ENV).ok().as_deref(),
            env::var(RESTATE_IMAGE_ENV).ok().as_deref(),
            env::var_os(COORDINATOR_BIN_ENV).map(PathBuf::from),
        )
    }

    fn from_values(
        enable_value: Option<&str>,
        restate_image: Option<&str>,
        coordinator_bin: Option<PathBuf>,
    ) -> Option<Self> {
        if !enable_value.is_some_and(env_value_enabled) {
            return None;
        }

        Some(Self {
            restate_image: restate_image
                .map(str::to_string)
                .unwrap_or_else(|| DEFAULT_RESTATE_IMAGE.to_string()),
            coordinator_bin: coordinator_bin
                .unwrap_or_else(|| PathBuf::from("target/debug/coat-coordinator")),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeVerifierSkipReason {
    Disabled,
    DockerUnavailable,
}

impl RuntimeVerifierSkipReason {
    fn message(self) -> &'static str {
        match self {
            Self::Disabled => {
                "set COAT_RESTATE_RESTART_RESUME_TEST=1 to enable the live Restate proof"
            }
            Self::DockerUnavailable => {
                "Docker is unavailable; skipping the Testcontainers Restate harness"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeVerifierFailureReason {
    MissingCoordinatorBinary(PathBuf),
    UnpinnedRestateImage(String),
}

impl RuntimeVerifierFailureReason {
    fn message(&self) -> String {
        match self {
            Self::MissingCoordinatorBinary(path) => format!(
                "coordinator binary does not exist at {}; run through cargo so {COORDINATOR_BIN_ENV} is set",
                path.display()
            ),
            Self::UnpinnedRestateImage(image) => format!(
                "{RESTATE_IMAGE_ENV} must be pinned for durable replay evidence, got {image}"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeVerifierLiveStatus {
    Skipped(RuntimeVerifierSkipReason),
    Failed(RuntimeVerifierFailureReason),
    Ready(RuntimeVerifierConfig),
}

impl RuntimeVerifierLiveStatus {
    fn classify(
        config: Option<RuntimeVerifierConfig>,
        docker_available: bool,
        coordinator_bin_exists: bool,
    ) -> Self {
        let Some(config) = config else {
            return Self::Skipped(RuntimeVerifierSkipReason::Disabled);
        };

        if !docker_available {
            return Self::Skipped(RuntimeVerifierSkipReason::DockerUnavailable);
        }

        if config.restate_image.ends_with(":latest") {
            return Self::Failed(RuntimeVerifierFailureReason::UnpinnedRestateImage(
                config.restate_image,
            ));
        }

        if !coordinator_bin_exists {
            return Self::Failed(RuntimeVerifierFailureReason::MissingCoordinatorBinary(
                config.coordinator_bin,
            ));
        }

        Self::Ready(config)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeVerifierStep {
    StartRestateWithPersistentData,
    StartCoordinator,
    RegisterDeployment,
    SubmitGoal,
    CaptureBeforeRestart,
    RestartCoordinator,
    CaptureAfterCoordinatorRestart,
    RestartRestate,
    CaptureAfterRestateRestart,
}

impl RuntimeVerifierStep {
    fn name(self) -> &'static str {
        match self {
            Self::StartRestateWithPersistentData => "start_restate_with_persistent_data",
            Self::StartCoordinator => "start_coordinator",
            Self::RegisterDeployment => "register_deployment",
            Self::SubmitGoal => "submit_goal",
            Self::CaptureBeforeRestart => "capture_before_restart",
            Self::RestartCoordinator => "restart_coordinator",
            Self::CaptureAfterCoordinatorRestart => "capture_after_coordinator_restart",
            Self::RestartRestate => "restart_restate",
            Self::CaptureAfterRestateRestart => "capture_after_restate_restart",
        }
    }

    fn captures_durable_counters(self) -> bool {
        matches!(
            self,
            Self::CaptureBeforeRestart
                | Self::CaptureAfterCoordinatorRestart
                | Self::CaptureAfterRestateRestart
        )
    }
}

const RUNTIME_VERIFIER_STEPS: [RuntimeVerifierStep; 9] = [
    RuntimeVerifierStep::StartRestateWithPersistentData,
    RuntimeVerifierStep::StartCoordinator,
    RuntimeVerifierStep::RegisterDeployment,
    RuntimeVerifierStep::SubmitGoal,
    RuntimeVerifierStep::CaptureBeforeRestart,
    RuntimeVerifierStep::RestartCoordinator,
    RuntimeVerifierStep::CaptureAfterCoordinatorRestart,
    RuntimeVerifierStep::RestartRestate,
    RuntimeVerifierStep::CaptureAfterRestateRestart,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RuntimeVerifierRestartBoundary {
    BeforeRestart,
    AfterCoordinatorRestart,
    AfterRestateRestart,
}

impl RuntimeVerifierRestartBoundary {
    fn step(self) -> RuntimeVerifierStep {
        match self {
            Self::BeforeRestart => RuntimeVerifierStep::CaptureBeforeRestart,
            Self::AfterCoordinatorRestart => RuntimeVerifierStep::CaptureAfterCoordinatorRestart,
            Self::AfterRestateRestart => RuntimeVerifierStep::CaptureAfterRestateRestart,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeVerifierProjectionCounters {
    total_tasks: u32,
    open_tasks: u32,
    done_tasks: usize,
    compute_graph_nodes: usize,
    compute_graph_edges: usize,
    open_thunks: u32,
    waiting_tasks: usize,
    event_count: usize,
    last_event_sequence: u64,
}

impl RuntimeVerifierProjectionCounters {
    fn from_projection(request: &GoalStoreSnapshotUpsertRequest) -> Self {
        Self {
            total_tasks: request.snapshot.goal.total_tasks,
            open_tasks: request.snapshot.goal.open_tasks,
            done_tasks: request
                .snapshot
                .tasks
                .iter()
                .filter(|task| task.status == TaskStatus::Done)
                .count(),
            compute_graph_nodes: request.snapshot.compute_graph.nodes.len(),
            compute_graph_edges: request.snapshot.compute_graph.edges.len(),
            open_thunks: request.snapshot.compute_graph.open_thunks,
            waiting_tasks: request.snapshot.compute_graph.waiting_tasks.len(),
            event_count: request.snapshot.events.len(),
            last_event_sequence: request
                .snapshot
                .events
                .last()
                .map(|event| event.sequence)
                .unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeVerifierRestartObservation {
    boundary: RuntimeVerifierRestartBoundary,
    projection_idempotency_key: String,
    counters: RuntimeVerifierProjectionCounters,
    completed_durable_step_executions: BTreeMap<&'static str, u32>,
}

#[derive(Debug, Default)]
struct RuntimeVerifierDurableStepLedger {
    completed: BTreeSet<&'static str>,
    execution_counts: BTreeMap<&'static str, u32>,
}

impl RuntimeVerifierDurableStepLedger {
    fn run_completed_step_once(&mut self, name: &'static str, mut operation: impl FnMut()) {
        if self.completed.insert(name) {
            *self.execution_counts.entry(name).or_insert(0) += 1;
            operation();
        }
    }

    fn execution_counts(&self) -> BTreeMap<&'static str, u32> {
        self.execution_counts.clone()
    }
}

fn runtime_verifier_goal_state() -> GoalState {
    let mut goal = GoalSpec::new(
        "runtime verifier replay",
        "Prove completed coordinator steps remain durable across restart and replay.",
    );
    goal.id = "018f8f2f-1fd8-7688-bb12-8bfb6b756602"
        .parse()
        .expect("stable RuntimeVerifier goal id");
    GoalState::new(goal)
}

fn mark_root_done(state: &mut GoalState) {
    let root_task_id = state
        .tasks
        .values()
        .find(|task| task.parent_id.is_none())
        .expect("root task")
        .id;
    state
        .tasks
        .get_mut(&root_task_id)
        .expect("root task")
        .status = TaskStatus::Done;
    state.status = GoalStatus::Done;
    state
        .events
        .push(StateEvent::new(format!("validated:{root_task_id}")));
}

fn projection_request(state: &GoalState, reason: &'static str) -> GoalStoreSnapshotUpsertRequest {
    GoalStoreSnapshotUpsertRequest::from_state(state, reason)
}

fn restart_observation(
    state: &GoalState,
    boundary: RuntimeVerifierRestartBoundary,
    ledger: &RuntimeVerifierDurableStepLedger,
) -> RuntimeVerifierRestartObservation {
    let projection = projection_request(state, boundary.step().name());
    RuntimeVerifierRestartObservation {
        boundary,
        projection_idempotency_key: projection.metadata.idempotency_key.clone(),
        counters: RuntimeVerifierProjectionCounters::from_projection(&projection),
        completed_durable_step_executions: ledger.execution_counts(),
    }
}

fn env_value_enabled(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn docker_is_available() -> bool {
    Command::new("docker")
        .arg("version")
        .arg("--format")
        .arg("{{.Server.Version}}")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[test]
fn runtime_verifier_is_disabled_without_explicit_env_gate() {
    if env::var_os(ENABLE_ENV).is_some() {
        eprintln!("{ENABLE_ENV} is set; skipping default-disabled assertion");
        return;
    }

    assert_eq!(RuntimeVerifierConfig::from_env(), None);
}

#[test]
fn runtime_verifier_config_requires_explicit_enablement() {
    for value in [
        None,
        Some(""),
        Some("0"),
        Some("false"),
        Some("no"),
        Some("off"),
    ] {
        assert_eq!(
            RuntimeVerifierConfig::from_values(
                value,
                Some("docker.restate.dev/restatedev/restate:1.5"),
                Some(PathBuf::from("/tmp/coat-coordinator")),
            ),
            None,
            "{value:?} must not enable the live RuntimeVerifier"
        );
    }
}

#[test]
fn runtime_verifier_config_defaults_are_replay_safe() {
    let config =
        RuntimeVerifierConfig::from_values(Some("yes"), None, None).expect("enabled config");

    assert_eq!(config.restate_image, DEFAULT_RESTATE_IMAGE);
    assert_eq!(
        config.coordinator_bin,
        PathBuf::from("target/debug/coat-coordinator")
    );
    assert!(
        !config.restate_image.ends_with(":latest"),
        "default Restate image must stay pinned for replay evidence"
    );
}

#[test]
fn runtime_verifier_default_restate_image_is_pinned() {
    assert_ne!(
        DEFAULT_RESTATE_IMAGE,
        "docker.restate.dev/restatedev/restate:latest"
    );
    assert!(
        DEFAULT_RESTATE_IMAGE.contains(":1.5"),
        "default Restate image should remain pinned for replayable evidence"
    );
}

#[test]
fn runtime_verifier_live_status_reports_clear_skip_failure_and_ready_states() {
    let config = RuntimeVerifierConfig::from_values(
        Some("1"),
        Some("docker.restate.dev/restatedev/restate:1.5"),
        Some(PathBuf::from("/tmp/coat-coordinator")),
    );

    assert_eq!(
        RuntimeVerifierLiveStatus::classify(None, true, true),
        RuntimeVerifierLiveStatus::Skipped(RuntimeVerifierSkipReason::Disabled)
    );
    assert_eq!(
        RuntimeVerifierLiveStatus::classify(config.clone(), false, true),
        RuntimeVerifierLiveStatus::Skipped(RuntimeVerifierSkipReason::DockerUnavailable)
    );
    assert_eq!(
        RuntimeVerifierLiveStatus::classify(config.clone(), true, false),
        RuntimeVerifierLiveStatus::Failed(RuntimeVerifierFailureReason::MissingCoordinatorBinary(
            PathBuf::from("/tmp/coat-coordinator")
        ))
    );
    assert_eq!(
        RuntimeVerifierLiveStatus::classify(
            RuntimeVerifierConfig::from_values(
                Some("1"),
                Some("docker.restate.dev/restatedev/restate:latest"),
                Some(PathBuf::from("/tmp/coat-coordinator")),
            ),
            true,
            true,
        ),
        RuntimeVerifierLiveStatus::Failed(RuntimeVerifierFailureReason::UnpinnedRestateImage(
            "docker.restate.dev/restatedev/restate:latest".to_string()
        ))
    );
    assert_eq!(
        RuntimeVerifierLiveStatus::classify(config.clone(), true, true),
        RuntimeVerifierLiveStatus::Ready(config.expect("config"))
    );
}

#[test]
fn runtime_verifier_harness_steps_order_restart_resume_boundaries() {
    let step_names: Vec<_> = RUNTIME_VERIFIER_STEPS
        .iter()
        .map(|step| step.name())
        .collect();

    assert_eq!(
        step_names,
        vec![
            "start_restate_with_persistent_data",
            "start_coordinator",
            "register_deployment",
            "submit_goal",
            "capture_before_restart",
            "restart_coordinator",
            "capture_after_coordinator_restart",
            "restart_restate",
            "capture_after_restate_restart",
        ]
    );
    assert_eq!(
        RUNTIME_VERIFIER_STEPS
            .iter()
            .filter(|step| step.captures_durable_counters())
            .count(),
        3,
        "the live proof must compare counters before restart, after coordinator restart, and after Restate restart"
    );
}

#[test]
fn runtime_verifier_restart_boundaries_capture_ordered_observations() {
    let mut state = runtime_verifier_goal_state();
    let mut ledger = RuntimeVerifierDurableStepLedger::default();

    ledger.run_completed_step_once("submit_goal", || {
        state
            .events
            .push(StateEvent::new("runtime_verifier:submitted"));
    });
    ledger.run_completed_step_once("validate_root_task", || {
        mark_root_done(&mut state);
    });

    let before_restart = restart_observation(
        &state,
        RuntimeVerifierRestartBoundary::BeforeRestart,
        &ledger,
    );
    let after_coordinator_restart = restart_observation(
        &state,
        RuntimeVerifierRestartBoundary::AfterCoordinatorRestart,
        &ledger,
    );
    let after_restate_restart = restart_observation(
        &state,
        RuntimeVerifierRestartBoundary::AfterRestateRestart,
        &ledger,
    );

    assert_eq!(
        [
            before_restart.boundary,
            after_coordinator_restart.boundary,
            after_restate_restart.boundary,
        ],
        [
            RuntimeVerifierRestartBoundary::BeforeRestart,
            RuntimeVerifierRestartBoundary::AfterCoordinatorRestart,
            RuntimeVerifierRestartBoundary::AfterRestateRestart,
        ]
    );
    assert_eq!(
        before_restart.counters, after_coordinator_restart.counters,
        "coordinator restart must not change the completed durable projection"
    );
    assert_eq!(
        before_restart.counters, after_restate_restart.counters,
        "Restate restart must replay to the same completed durable projection"
    );
    assert_ne!(
        before_restart.projection_idempotency_key,
        after_coordinator_restart.projection_idempotency_key,
        "each restart-boundary projection uses its own idempotency key"
    );
    assert_eq!(
        before_restart.completed_durable_step_executions,
        after_restate_restart.completed_durable_step_executions
    );
}

#[test]
fn runtime_verifier_completed_durable_steps_are_not_reexecuted_on_replay() {
    let mut state = runtime_verifier_goal_state();
    let mut ledger = RuntimeVerifierDurableStepLedger::default();

    ledger.run_completed_step_once("validate_root_task", || {
        mark_root_done(&mut state);
    });
    let first_completion_projection = projection_request(&state, "validation_applied");

    ledger.run_completed_step_once("validate_root_task", || {
        panic!("completed durable validation step must not run again after replay");
    });
    let replay_projection = projection_request(&state, "validation_applied");

    assert_eq!(
        ledger.execution_counts().get("validate_root_task"),
        Some(&1),
        "the completed validation durable step should execute exactly once"
    );
    assert_eq!(
        RuntimeVerifierProjectionCounters::from_projection(&first_completion_projection),
        RuntimeVerifierProjectionCounters::from_projection(&replay_projection),
        "replay of a completed durable step must not mutate projection counters"
    );
    assert_eq!(
        first_completion_projection.metadata.idempotency_key,
        replay_projection.metadata.idempotency_key,
        "replay of the same completed projection must reuse the idempotency key"
    );
}

#[test]
fn runtime_verifier_projection_requests_are_idempotent_for_replay() {
    let state = runtime_verifier_goal_state();

    let first = projection_request(&state, "drive_start");
    let replay = projection_request(&state, "drive_start");

    assert_eq!(
        first.metadata.idempotency_key,
        replay.metadata.idempotency_key
    );
    assert_eq!(first.snapshot, replay.snapshot);
    assert_eq!(
        first.metadata.idempotency_key,
        format!("goal:{}:projection:drive_start:1", state.goal.id)
    );
    assert_eq!(
        RuntimeVerifierProjectionCounters::from_projection(&first),
        RuntimeVerifierProjectionCounters {
            total_tasks: 1,
            open_tasks: 1,
            done_tasks: 0,
            compute_graph_nodes: 2,
            compute_graph_edges: 1,
            open_thunks: 0,
            waiting_tasks: 0,
            event_count: 1,
            last_event_sequence: 1,
        }
    );
}

#[test]
fn runtime_verifier_projection_counters_advance_only_when_state_advances() {
    let mut state = runtime_verifier_goal_state();
    let before = projection_request(&state, "drive_start");

    mark_root_done(&mut state);
    let after = projection_request(&state, "validation_applied");
    let replay_after_restart = projection_request(&state, "validation_applied");

    assert_ne!(
        before.metadata.idempotency_key,
        after.metadata.idempotency_key
    );
    assert_eq!(
        after.metadata.idempotency_key, replay_after_restart.metadata.idempotency_key,
        "replaying the same completed state must not mint a new projection key"
    );
    assert_eq!(
        RuntimeVerifierProjectionCounters::from_projection(&after),
        RuntimeVerifierProjectionCounters {
            total_tasks: 1,
            open_tasks: 0,
            done_tasks: 1,
            compute_graph_nodes: 2,
            compute_graph_edges: 1,
            open_thunks: 0,
            waiting_tasks: 0,
            event_count: 2,
            last_event_sequence: 2,
        }
    );
    assert_eq!(
        RuntimeVerifierProjectionCounters::from_projection(&after),
        RuntimeVerifierProjectionCounters::from_projection(&replay_after_restart),
        "counter comparison is the deterministic stand-in for the live restart/resume assertion"
    );
}

#[test]
fn runtime_verifier_projection_counters_include_compute_graph_waits() {
    let mut state = runtime_verifier_goal_state();
    let task_id = state.runnable_tasks().remove(0).id;
    state
        .create_delayed_compute_thunk(DelayedComputeThunkRequest {
            goal_id: state.goal.id,
            task_id: Some(task_id),
            kind: DelayedComputeThunkKind::ExternalEvent,
            reason: "wait for runtime verifier callback".to_string(),
            requested_input: Some("callback payload".to_string()),
            wait_ref: Some(WaitRef {
                kind: WaitRefKind::WebhookCorrelation,
                reference: "webhook://runtime-verifier/restart".to_string(),
            }),
            continuation: ContinuationRef {
                continuation_id: "runtime-verifier/restart-callback".to_string(),
                boundary: ContinuationBoundary::ExternalCallback,
                state_ref: format!("goal/{}/task/{task_id}", state.goal.id),
                resume_actions: vec![ContinuationResumeAction::MarkRunnable],
            },
            timeout_seconds: Some(60),
        })
        .expect("delayed compute thunk");

    let projection = projection_request(&state, "delayed_compute_thunk_created");

    assert_eq!(
        RuntimeVerifierProjectionCounters::from_projection(&projection),
        RuntimeVerifierProjectionCounters {
            total_tasks: 1,
            open_tasks: 1,
            done_tasks: 0,
            compute_graph_nodes: 5,
            compute_graph_edges: 4,
            open_thunks: 1,
            waiting_tasks: 1,
            event_count: 2,
            last_event_sequence: 2,
        }
    );
}

#[test]
fn runtime_verifier_projection_counters_include_blocked_waiting_and_thunks() {
    let mut goal = GoalSpec::new(
        "runtime verifier blocked waiting",
        "Prove blocked work and waiting delayed compute thunks project together.",
    );
    goal.initial_tasks.push(ChildTaskRequest {
        role: WorkerKind::Codex,
        purpose: None,
        title: Some("operator input child".to_string()),
        subgoal_id: None,
        color: None,
        prompt: "wait for operator input".to_string(),
        reason: "test projection counters".to_string(),
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
    state
        .create_delayed_compute_thunk(DelayedComputeThunkRequest {
            goal_id: state.goal.id,
            task_id: Some(waiting_task_id),
            kind: DelayedComputeThunkKind::ExternalEvent,
            reason: "wait for runtime verifier callback".to_string(),
            requested_input: Some("callback payload".to_string()),
            wait_ref: Some(WaitRef {
                kind: WaitRefKind::WebhookCorrelation,
                reference: "webhook://runtime-verifier/restart".to_string(),
            }),
            continuation: ContinuationRef {
                continuation_id: "runtime-verifier/restart-callback".to_string(),
                boundary: ContinuationBoundary::ExternalCallback,
                state_ref: format!("goal/{}/task/{waiting_task_id}", state.goal.id),
                resume_actions: vec![ContinuationResumeAction::MarkRunnable],
            },
            timeout_seconds: Some(60),
        })
        .expect("delayed compute thunk");
    state.status = GoalStatus::Blocked;

    let projection = projection_request(&state, "frontier_idle");

    assert_eq!(projection.snapshot.goal.blocked_tasks, 1);
    assert_eq!(
        RuntimeVerifierProjectionCounters::from_projection(&projection),
        RuntimeVerifierProjectionCounters {
            total_tasks: 2,
            open_tasks: 2,
            done_tasks: 0,
            compute_graph_nodes: 6,
            compute_graph_edges: 5,
            open_thunks: 1,
            waiting_tasks: 1,
            event_count: 3,
            last_event_sequence: 3,
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "live RuntimeVerifier proof requires COAT_RESTATE_RESTART_RESUME_TEST=1 and Docker"]
async fn restate_restart_resume_proof_entrypoint() {
    let config = RuntimeVerifierConfig::from_env();
    let coordinator_bin_exists = config
        .as_ref()
        .map(|config| config.coordinator_bin.exists())
        .unwrap_or(false);
    match RuntimeVerifierLiveStatus::classify(config, docker_is_available(), coordinator_bin_exists)
    {
        RuntimeVerifierLiveStatus::Skipped(reason) => {
            eprintln!("skipping live RuntimeVerifier proof: {}", reason.message());
        }
        RuntimeVerifierLiveStatus::Failed(reason) => {
            panic!(
                "live RuntimeVerifier proof misconfigured: {}",
                reason.message()
            );
        }
        RuntimeVerifierLiveStatus::Ready(config) => {
            assert_eq!(
                RUNTIME_VERIFIER_STEPS
                    .iter()
                    .filter(|step| step.captures_durable_counters())
                    .count(),
                3,
                "live proof must capture durable counters at each restart boundary"
            );
            panic!(
                "live RuntimeVerifier proof is explicitly gated but the Docker/Testcontainers \
                 execution harness is not implemented in this crate yet. Ready config: image={}, \
                 coordinator={}. Required ordered steps: [{}]",
                config.restate_image,
                config.coordinator_bin.display(),
                RUNTIME_VERIFIER_STEPS
                    .iter()
                    .map(|step| step.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
}
