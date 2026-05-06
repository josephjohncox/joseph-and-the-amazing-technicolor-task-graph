use std::{
    collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

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
    #[serde(default)]
    pub authoring: GoalAuthoringGuidance,
    #[serde(default)]
    pub plan: GoalPlan,
    pub root_budget: Budget,
    pub done_criteria: DoneCriteria,
    #[serde(default)]
    pub review_policy: ReviewPolicy,
    #[serde(default)]
    pub control_policy: ControlLoopPolicy,
    #[serde(default)]
    pub research_policy: ResearchPolicy,
    #[serde(default)]
    pub memory_policy: MemoryPolicy,
    #[serde(default)]
    pub approval_policy: ApprovalGatePolicy,
    #[serde(default)]
    pub restart_policy: RestartPolicy,
    #[serde(default)]
    pub timeout_policy: TimeoutPolicy,
    #[serde(default)]
    pub branching_policy: BranchingPolicy,
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
            authoring: GoalAuthoringGuidance::default(),
            plan: GoalPlan::default(),
            root_budget: Budget::default_goal(),
            done_criteria: DoneCriteria::default(),
            review_policy: ReviewPolicy::default(),
            control_policy: ControlLoopPolicy::default(),
            research_policy: ResearchPolicy::default(),
            memory_policy: MemoryPolicy::default(),
            approval_policy: ApprovalGatePolicy::default(),
            restart_policy: RestartPolicy::default(),
            timeout_policy: TimeoutPolicy::default(),
            branching_policy: BranchingPolicy::default(),
            default_execution: ExecutionProfile::default(),
            initial_tasks: Vec::new(),
        }
    }

    pub fn quality_report(&self) -> GoalQualityReport {
        let mut missing = Vec::new();
        let mut warnings = Vec::new();
        let mut suggested_next_actions = Vec::new();

        if self.title.trim().len() < 4 {
            missing.push("title is missing or too short".to_string());
        }
        if self.objective.trim().len() < 20 {
            missing.push("objective should be concrete enough for a reviewer".to_string());
        }
        if !self.done_criteria.tests_pass
            && !self.done_criteria.artifact_exists
            && self.done_criteria.validator_score_min.is_none()
        {
            missing.push("done criteria do not require tests, artifacts, or score".to_string());
        }
        if self.root_budget.is_exhausted() {
            missing.push("root budget is already exhausted".to_string());
        }
        if self.control_policy.max_frontier_rounds == 0 {
            missing.push("control policy allows zero frontier rounds".to_string());
        }
        if self.timeout_policy.enabled
            && self.timeout_policy.goal_timeout_seconds.is_none()
            && self.timeout_policy.task_run_timeout_seconds.is_none()
            && self.timeout_policy.idle_timeout_seconds.is_none()
        {
            warnings.push("timeout policy is enabled without any concrete timeout".to_string());
        }
        if self.restart_policy.enabled && self.restart_policy.max_goal_restarts == 0 {
            warnings.push("restart policy is enabled but max_goal_restarts is zero".to_string());
        }
        if self.branching_policy.enabled && self.branching_policy.max_candidates_per_group < 2 {
            warnings.push(
                "branching policy is enabled but max_candidates_per_group is below two".to_string(),
            );
        }
        if self.branching_policy.enabled
            && self.branching_policy.voting.enabled
            && self.branching_policy.voting.min_votes == 0
        {
            warnings.push("branch voting is enabled but min_votes is zero".to_string());
        }
        if self.review_policy.enabled && self.review_policy.min_reviews == 0 {
            warnings.push("review policy is enabled but min_reviews is zero".to_string());
        }
        if self.review_policy.doctrine.enabled
            && self.review_policy.doctrine.resolved_objectives().is_empty()
        {
            missing.push(
                "review doctrine is enabled but no review objectives are selected".to_string(),
            );
        }
        if self.review_policy.doctrine.enabled
            && self.review_policy.doctrine.coverage.require_gate_results
            && self.review_policy.doctrine.resolved_validation_gates().is_empty()
        {
            missing.push(
                "review doctrine requires gate results but no validation gates are selected"
                    .to_string(),
            );
        }
        if !self.review_policy.doctrine.enabled {
            warnings.push(
                "review doctrine is disabled; opt in for typed quality, testing, style, and formal-methods review goals".to_string(),
            );
        }
        if !self.review_policy.enabled {
            warnings.push(
                "review policy is disabled; non-trivial work should keep a critic gate".to_string(),
            );
        }
        if !self.research_policy.require_sources && !self.plan.subgoals.is_empty() {
            warnings.push(
                "research sources are optional; current or external claims may need sources"
                    .to_string(),
            );
        }
        if self.initial_tasks.is_empty() && self.plan.subgoals.is_empty() {
            warnings.push(
                "goal has no subgoals or initial tasks; the planner root must decompose everything"
                    .to_string(),
            );
            suggested_next_actions
                .push("add plan.subgoals or initial_tasks for known work".to_string());
        }
        if self.approval_policy.enabled
            && !self.approval_policy.require_for_secret_access
            && !self.approval_policy.require_for_brokered_user_auth
        {
            warnings.push("approval policy does not gate secret or brokered user auth".to_string());
        }
        if self
            .default_execution
            .mcp
            .auth_distribution
            .allow_secret_sync
        {
            warnings.push(
                "auth distribution allows secret sync; prefer runner-local or brokered leases"
                    .to_string(),
            );
        }
        if self.default_execution.notifications.targets.is_empty() {
            warnings.push(
                "no notification target is configured for approval or blocked-task threads"
                    .to_string(),
            );
        }

        let mut subgoal_ids = BTreeSet::new();
        for subgoal in &self.plan.subgoals {
            if subgoal.id.trim().is_empty() {
                missing.push("plan contains a subgoal with an empty id".to_string());
            }
            if !subgoal_ids.insert(subgoal.id.clone()) {
                missing.push(format!("duplicate subgoal id: {}", subgoal.id));
            }
            if subgoal.title.trim().is_empty() {
                missing.push(format!("subgoal {} has no title", subgoal.id));
            }
        }
        for task in &self.initial_tasks {
            if task.prompt.trim().is_empty() {
                missing.push("initial task has an empty prompt".to_string());
            }
            if task.reason.trim().is_empty() {
                warnings.push("initial task is missing a reason for distribution".to_string());
            }
            if let Some(subgoal_id) = &task.subgoal_id {
                if !self.plan.subgoals.is_empty() && !subgoal_ids.contains(subgoal_id) {
                    warnings.push(format!(
                        "initial task references subgoal {} that is not in plan.subgoals",
                        subgoal_id
                    ));
                }
            }
        }

        let score =
            (1.0 - (missing.len() as f32 * 0.2) - (warnings.len() as f32 * 0.05)).clamp(0.0, 1.0);
        if missing.is_empty() && warnings.is_empty() {
            suggested_next_actions
                .push("submit the goal or run it against the local stub stack".to_string());
        }

        GoalQualityReport {
            goal_id: self.id,
            ready: missing.is_empty(),
            score,
            missing,
            warnings,
            suggested_next_actions,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub struct GoalAuthoringGuidance {
    #[serde(default)]
    pub intake_summary: String,
    #[serde(default)]
    pub acceptance_evidence: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub struct GoalPlan {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub subgoals: Vec<SubgoalSpec>,
    #[serde(default)]
    pub distribution_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SubgoalSpec {
    pub id: String,
    pub title: String,
    pub objective: String,
    pub owner_role: WorkerKind,
    #[serde(default)]
    pub priority: TaskPriority,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub acceptance_evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalQualityReport {
    pub goal_id: GoalId,
    pub ready: bool,
    pub score: f32,
    #[serde(default)]
    pub missing: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub suggested_next_actions: Vec<String>,
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
    pub review_rounds: Vec<ReviewRound>,
    #[serde(default)]
    pub satisfaction: Option<SatisfactionReport>,
    #[serde(default)]
    pub learning_signals: Vec<LearningSignal>,
    #[serde(default)]
    pub steering_directives: Vec<SteeringDirective>,
    #[serde(default)]
    pub restart_history: Vec<RestartRecord>,
    #[serde(default)]
    pub branch_groups: Vec<BranchGroup>,
    #[serde(default)]
    pub branch_votes: Vec<BranchVoteRecord>,
    #[serde(default)]
    pub timeout_events: Vec<TimeoutEvent>,
    #[serde(default)]
    pub memory_events: Vec<MemoryEvent>,
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
            purpose: TaskPurpose::Work,
            title: goal.title.clone(),
            subgoal_id: None,
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
            review_doctrine: goal.review_policy.doctrine.clone(),
            priority: TaskPriority::High,
            tags: vec!["root".to_string()],
            result: None,
            attempts: 0,
        };

        let mut state = Self {
            goal,
            tasks: BTreeMap::from([(root_id, root)]),
            status: GoalStatus::Running,
            approvals: Vec::new(),
            final_artifacts: Vec::new(),
            review_rounds: Vec::new(),
            satisfaction: None,
            learning_signals: Vec::new(),
            steering_directives: Vec::new(),
            restart_history: Vec::new(),
            branch_groups: Vec::new(),
            branch_votes: Vec::new(),
            timeout_events: Vec::new(),
            memory_events: Vec::new(),
            events: Vec::new(),
        };
        let initial_tasks = state.goal.initial_tasks.clone();
        if !initial_tasks.is_empty() {
            let root_snapshot = state
                .tasks
                .get(&root_id)
                .expect("root task is inserted before initial tasks")
                .clone();
            for request in initial_tasks {
                let child_id = Uuid::new_v4();
                state
                    .tasks
                    .get_mut(&root_id)
                    .expect("root task exists while inserting initial tasks")
                    .children
                    .push(child_id);
                state.tasks.insert(
                    child_id,
                    TaskNode::from_child_request(child_id, root_id, &root_snapshot, request),
                );
                state
                    .events
                    .push(StateEvent::new(format!("initial_task_spawned:{child_id}")));
            }
        }
        state.events.push(StateEvent::new("goal_started"));
        state
    }

    pub fn runnable_tasks(&self) -> Vec<TaskNode> {
        let mut tasks: Vec<_> = self
            .tasks
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
            .collect();
        tasks.sort_by(|left, right| {
            right
                .priority
                .rank()
                .cmp(&left.priority.rank())
                .then_with(|| left.depth.cmp(&right.depth))
                .then_with(|| left.id.cmp(&right.id))
        });
        tasks
    }

    pub fn is_done(&self) -> bool {
        self.status == GoalStatus::Done
            || self
                .satisfaction
                .as_ref()
                .is_some_and(|report| report.satisfied)
    }

    pub fn budget_exhausted(&self) -> bool {
        self.tasks.values().all(|task| task.budget.is_exhausted())
    }

    pub fn progress(&self) -> GoalProgress {
        let mut by_status = BTreeMap::new();
        let mut task_progress = Vec::with_capacity(self.tasks.len());
        for task in self.tasks.values() {
            *by_status.entry(task.status.clone()).or_insert(0) += 1;
            task_progress.push(self.task_progress(task));
        }
        task_progress.sort_by(|left, right| {
            right
                .priority
                .rank()
                .cmp(&left.priority.rank())
                .then_with(|| left.depth.cmp(&right.depth))
                .then_with(|| left.task_id.cmp(&right.task_id))
        });

        let total_tasks = self.tasks.len() as u32;
        let terminal_ok = self
            .tasks
            .values()
            .filter(|task| task.status.is_terminal_ok())
            .count() as u32;
        let open_tasks = self
            .tasks
            .values()
            .filter(|task| !task.status.is_terminal())
            .count() as u32;
        let blocked_tasks = *by_status.get(&TaskStatus::Blocked).unwrap_or(&0);
        let failed_tasks = *by_status.get(&TaskStatus::Failed).unwrap_or(&0);
        let waiting_approval_tasks = *by_status.get(&TaskStatus::WaitingApproval).unwrap_or(&0);
        let runnable_tasks = task_progress
            .iter()
            .filter(|task| task.runnable)
            .map(|task| task.task_id)
            .collect();
        let percent_done = if total_tasks == 0 {
            0.0
        } else {
            terminal_ok as f32 / total_tasks as f32
        };

        GoalProgress {
            goal_id: self.goal.id,
            title: self.goal.title.clone(),
            status: self.status.clone(),
            total_tasks,
            open_tasks,
            terminal_ok_tasks: terminal_ok,
            blocked_tasks,
            failed_tasks,
            waiting_approval_tasks,
            percent_done,
            by_status,
            subgoals: self.subgoal_progress(),
            runnable_tasks,
            next_tasks: task_progress.into_iter().take(10).collect(),
            satisfaction: self.satisfaction.clone(),
        }
    }

    pub fn find_tasks(&self, query: &TaskQuery) -> TaskList {
        let mut tasks: Vec<_> = self
            .tasks
            .values()
            .filter(|task| query.matches(task, self))
            .map(|task| self.task_progress(task))
            .collect();
        tasks.sort_by(|left, right| {
            right
                .priority
                .rank()
                .cmp(&left.priority.rank())
                .then_with(|| left.depth.cmp(&right.depth))
                .then_with(|| left.task_id.cmp(&right.task_id))
        });
        if let Some(limit) = query.limit {
            tasks.truncate(limit);
        }
        TaskList {
            goal_id: self.goal.id,
            query: query.clone(),
            tasks,
            progress: self.progress(),
        }
    }

    fn task_progress(&self, task: &TaskNode) -> TaskProgress {
        TaskProgress {
            task_id: task.id,
            parent_id: task.parent_id,
            title: task.title.clone(),
            subgoal_id: task.subgoal_id.clone(),
            status: task.status.clone(),
            role: task.role.clone(),
            purpose_kind: TaskPurposeKind::from(&task.purpose),
            depth: task.depth,
            priority: task.priority.clone(),
            tags: task.tags.clone(),
            attempts: task.attempts,
            dependency_count: task.dependencies.len() as u32,
            child_count: task.children.len() as u32,
            runnable: task.status == TaskStatus::Runnable
                && task.dependencies.iter().all(|id| {
                    self.tasks
                        .get(id)
                        .is_some_and(|dep| dep.status.is_terminal_ok())
                }),
            blocked_by: task
                .dependencies
                .iter()
                .filter(|id| {
                    self.tasks
                        .get(id)
                        .is_some_and(|dep| !dep.status.is_terminal_ok())
                })
                .copied()
                .collect(),
            result: task.result.clone(),
        }
    }

    fn subgoal_progress(&self) -> Vec<SubgoalProgress> {
        let mut subgoals = Vec::new();
        let mut seen = BTreeSet::new();
        for spec in &self.goal.plan.subgoals {
            seen.insert(spec.id.clone());
            subgoals.push(self.subgoal_progress_for(
                spec.id.clone(),
                Some(spec.title.clone()),
                Some(spec.owner_role.clone()),
                spec.dependencies.clone(),
            ));
        }

        let mut dynamic_ids: Vec<_> = self
            .tasks
            .values()
            .filter_map(|task| task.subgoal_id.clone())
            .filter(|id| !seen.contains(id))
            .collect();
        dynamic_ids.sort();
        dynamic_ids.dedup();
        for id in dynamic_ids {
            subgoals.push(self.subgoal_progress_for(id, None, None, Vec::new()));
        }
        subgoals
    }

    fn subgoal_progress_for(
        &self,
        subgoal_id: String,
        title: Option<String>,
        owner_role: Option<WorkerKind>,
        dependencies: Vec<String>,
    ) -> SubgoalProgress {
        let matching: Vec<&TaskNode> = self
            .tasks
            .values()
            .filter(|task| task.subgoal_id.as_deref() == Some(subgoal_id.as_str()))
            .collect();
        let total_tasks = matching.len() as u32;
        let terminal_ok_tasks = matching
            .iter()
            .filter(|task| task.status.is_terminal_ok())
            .count() as u32;
        let open_tasks = matching
            .iter()
            .filter(|task| !task.status.is_terminal())
            .count() as u32;
        let status = if total_tasks == 0 {
            SubgoalStatus::Planned
        } else if matching
            .iter()
            .any(|task| task.status == TaskStatus::Failed)
        {
            SubgoalStatus::Failed
        } else if matching
            .iter()
            .any(|task| task.status == TaskStatus::Blocked)
        {
            SubgoalStatus::Blocked
        } else if open_tasks == 0 {
            SubgoalStatus::Done
        } else if matching
            .iter()
            .any(|task| task.status == TaskStatus::Running)
        {
            SubgoalStatus::Running
        } else {
            SubgoalStatus::Open
        };
        let runnable_tasks = matching
            .iter()
            .filter(|task| {
                task.status == TaskStatus::Runnable
                    && task.dependencies.iter().all(|id| {
                        self.tasks
                            .get(id)
                            .is_some_and(|dep| dep.status.is_terminal_ok())
                    })
            })
            .map(|task| task.id)
            .collect();
        SubgoalProgress {
            subgoal_id,
            title,
            owner_role,
            status,
            total_tasks,
            open_tasks,
            terminal_ok_tasks,
            runnable_tasks,
            dependencies,
        }
    }

    pub fn satisfaction_report(&self) -> SatisfactionReport {
        let policy = &self.goal.review_policy;
        let work_tasks: Vec<&TaskNode> = self
            .tasks
            .values()
            .filter(|task| task.purpose.is_work_like())
            .collect();
        let work_done =
            !work_tasks.is_empty() && work_tasks.iter().all(|task| task.status.is_terminal_ok());
        let reviews_passed = self
            .tasks
            .values()
            .filter(|task| task.purpose.is_review() && task.status.is_terminal_ok())
            .count() as u32;
        let reviews_required = if policy.enabled {
            policy.min_reviews
        } else {
            0
        };
        let review_gate_passed = match policy.join_strategy {
            ReviewJoinStrategy::AllRequired => reviews_passed >= reviews_required,
            ReviewJoinStrategy::AnyRequired => reviews_required == 0 || reviews_passed > 0,
            ReviewJoinStrategy::Quorum { min_passed } => reviews_passed >= min_passed,
        };
        let unification_done = !policy.enabled
            || !policy.require_unification
            || self
                .tasks
                .values()
                .any(|task| task.purpose.is_unification() && task.status.is_terminal_ok());
        let all_tasks_terminal = !self.tasks.is_empty()
            && self
                .tasks
                .values()
                .all(|task| task.status.is_terminal_ok() || task.status == TaskStatus::Cancelled);
        let open_tasks = self
            .tasks
            .values()
            .filter(|task| !task.status.is_terminal())
            .count() as u32;
        let score = if self.learning_signals.is_empty() {
            if all_tasks_terminal { 1.0 } else { 0.0 }
        } else {
            self.learning_signals
                .iter()
                .map(|signal| signal.reward)
                .sum::<f32>()
                / self.learning_signals.len() as f32
        };
        let latest_decision = self
            .learning_signals
            .iter()
            .rev()
            .find(|signal| {
                matches!(
                    signal.source,
                    LearningSignalSource::CriticReview | LearningSignalSource::ReviewUnification
                )
            })
            .and_then(|signal| signal.decision.clone());
        let open_findings = self
            .learning_signals
            .iter()
            .map(|signal| signal.findings_count)
            .sum();
        let blocking_review_decision = matches!(
            latest_decision,
            Some(
                ReviewDecision::ChangesRequested
                    | ReviewDecision::Blocked
                    | ReviewDecision::Inconclusive
            )
        );

        let mut reasons = Vec::new();
        if !work_done {
            reasons.push("actor work is not complete".to_string());
        }
        if policy.enabled && !review_gate_passed {
            reasons.push("required critic reviews have not passed".to_string());
        }
        if policy.enabled && !unification_done {
            reasons.push("review unification has not completed".to_string());
        }
        if score < policy.min_satisfaction_score {
            reasons.push("satisfaction score is below policy threshold".to_string());
        }
        if open_tasks > 0 {
            reasons.push("durable task tree still has open tasks".to_string());
        }
        if blocking_review_decision {
            reasons.push("latest critic or unifier decision does not accept the work".to_string());
        }
        if self
            .branch_groups
            .iter()
            .any(|group| group.status != BranchGroupStatus::Selected)
        {
            reasons.push("one or more branch groups have not selected a candidate".to_string());
        }

        SatisfactionReport {
            satisfied: work_done
                && review_gate_passed
                && unification_done
                && self
                    .branch_groups
                    .iter()
                    .all(|group| group.status == BranchGroupStatus::Selected)
                && open_tasks == 0
                && score >= policy.min_satisfaction_score
                && !blocking_review_decision
                && self
                    .tasks
                    .values()
                    .all(|task| task.status != TaskStatus::Failed),
            score,
            work_done,
            reviews_required,
            reviews_passed,
            unification_done,
            all_tasks_terminal,
            open_tasks,
            latest_decision,
            open_findings,
            reasons,
        }
    }

    pub fn mark_running(&mut self, task_id: TaskId) -> Result<(), DomainError> {
        let task = self.task_mut(task_id)?;
        task.status = TaskStatus::Running;
        task.attempts += 1;
        self.events
            .push(StateEvent::new(format!("task_running:{task_id}")));
        Ok(())
    }

    pub fn ensure_task_approval_or_request(
        &mut self,
        task_id: TaskId,
    ) -> Result<Option<ApprovalRequest>, DomainError> {
        let task = self.task(task_id)?.clone();
        let next_attempt = task.attempts + 1;
        if self.approvals.iter().any(|approval| {
            approval.task_id == Some(task_id)
                && approval.attempt == next_attempt
                && approval.status == ApprovalStatus::Approved
        }) {
            return Ok(None);
        }
        if self.approvals.iter().any(|approval| {
            approval.task_id == Some(task_id)
                && approval.attempt == next_attempt
                && approval.status == ApprovalStatus::Pending
        }) {
            self.task_mut(task_id)?.status = TaskStatus::WaitingApproval;
            self.refresh_goal_status();
            return Ok(self
                .approvals
                .iter()
                .find(|approval| {
                    approval.task_id == Some(task_id)
                        && approval.attempt == next_attempt
                        && approval.status == ApprovalStatus::Pending
                })
                .cloned());
        }
        if self.approvals.iter().any(|approval| {
            approval.task_id == Some(task_id)
                && approval.attempt == next_attempt
                && approval.status == ApprovalStatus::Rejected
        }) {
            self.task_mut(task_id)?.status = TaskStatus::Blocked;
            self.refresh_goal_status();
            return Err(DomainError::ApprovalDenied(format!(
                "task {task_id} approval was rejected"
            )));
        }

        let evaluation = self.goal.approval_policy.evaluate(&task);
        if !evaluation.required {
            return Ok(None);
        }

        let request = ApprovalRequest {
            id: Uuid::new_v4(),
            goal_id: self.goal.id,
            task_id: Some(task_id),
            attempt: next_attempt,
            reason: evaluation.reason.clone(),
            status: ApprovalStatus::Pending,
            risk: evaluation.risk,
            reason_codes: evaluation.reason_codes,
            sandbox: task.sandbox.clone(),
            requested_action: format!(
                "run {} task {} attempt {}",
                task.role.as_str(),
                task.id,
                next_attempt
            ),
            notification_reports: Vec::new(),
        };
        self.task_mut(task_id)?.status = TaskStatus::WaitingApproval;
        self.approvals.push(request.clone());
        self.refresh_goal_status();
        self.events.push(StateEvent::new(format!(
            "approval_requested:{}:{}",
            request.id, task_id
        )));
        Ok(Some(request))
    }

    pub fn record_approval_notification(
        &mut self,
        approval_id: Uuid,
        reports: Vec<NotificationDeliveryReport>,
    ) -> Result<(), DomainError> {
        let approval = self
            .approvals
            .iter_mut()
            .find(|approval| approval.id == approval_id)
            .ok_or(DomainError::ApprovalNotFound(approval_id))?;
        approval.notification_reports = reports;
        Ok(())
    }

    pub fn apply_human_approval(
        &mut self,
        approval: HumanApproval,
    ) -> Result<ApprovalRequest, DomainError> {
        let index = self
            .approvals
            .iter()
            .position(|request| request.id == approval.approval_id)
            .ok_or(DomainError::ApprovalNotFound(approval.approval_id))?;
        let task_id = self.approvals[index].task_id;
        self.approvals[index].status = if approval.approved {
            ApprovalStatus::Approved
        } else {
            ApprovalStatus::Rejected
        };
        let updated = self.approvals[index].clone();
        if let Some(task_id) = task_id {
            let task = self.task_mut(task_id)?;
            match updated.status {
                ApprovalStatus::Approved if task.status == TaskStatus::WaitingApproval => {
                    task.status = TaskStatus::Runnable;
                }
                ApprovalStatus::Rejected => {
                    task.status = TaskStatus::Blocked;
                }
                _ => {}
            }
        }
        self.refresh_goal_status();
        self.events.push(StateEvent::new(format!(
            "approval_{}:{}",
            if approval.approved {
                "approved"
            } else {
                "rejected"
            },
            approval.approval_id
        )));
        Ok(updated)
    }

    pub fn apply_agent_result(
        &mut self,
        result: AgentRunResult,
        policy: &SpawnPolicy,
    ) -> Result<(), DomainError> {
        let primary_artifacts = result_artifacts(&result);
        let task = self.task_mut(result.task_id)?;
        task.result = primary_artifacts.first().cloned();
        task.status = match result.status {
            WorkerRunStatus::Done => TaskStatus::NeedsValidation,
            WorkerRunStatus::Partial => TaskStatus::Runnable,
            WorkerRunStatus::Blocked => TaskStatus::Blocked,
            WorkerRunStatus::Failed | WorkerRunStatus::TimedOut => TaskStatus::Failed,
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
        let purpose = self.task(report.task_id)?.purpose.clone();
        if let Some(vote) = report.branch_vote.clone() {
            self.record_branch_vote(report.task_id, vote)?;
        }
        let task = self.task_mut(report.task_id)?;
        task.status = report.status_after_validation.clone();
        if report.passed {
            self.final_artifacts.extend(report.artifacts.clone());
        }
        if matches!(purpose, TaskPurpose::Unification { .. }) && report.passed {
            for round in &mut self.review_rounds {
                if round.unification_task_id == Some(report.task_id) {
                    round.status = ReviewRoundStatus::Unified;
                }
            }
        }
        if matches!(purpose, TaskPurpose::BranchUnification { .. }) && report.passed {
            for group in &mut self.branch_groups {
                if group.unification_task_id == Some(report.task_id)
                    && group.status == BranchGroupStatus::ReadyForUnification
                {
                    group.status = BranchGroupStatus::ReadyForSelection;
                }
            }
        }
        self.record_learning_signal(&report, &purpose);
        self.try_auto_select_branch_groups();
        self.refresh_goal_status();
        self.events
            .push(StateEvent::new(format!("validated:{}", report.task_id)));
        Ok(())
    }

    pub fn ensure_branch_frontier(&mut self, policy: &SpawnPolicy) -> Result<bool, DomainError> {
        if !self.goal.branching_policy.enabled {
            return Ok(false);
        }

        let mut spawned = false;
        let group_ids: Vec<Uuid> = self.branch_groups.iter().map(|group| group.id).collect();
        for group_id in group_ids {
            let Some(index) = self
                .branch_groups
                .iter()
                .position(|group| group.id == group_id)
            else {
                continue;
            };
            let group = self.branch_groups[index].clone();
            match group.status {
                BranchGroupStatus::CandidatesSpawned => {
                    if !self.tasks_terminal_ok(&group.candidate_task_ids) {
                        continue;
                    }
                    if self.goal.branching_policy.voting.enabled && group.voter_task_ids.is_empty()
                    {
                        self.spawn_branch_votes(index, policy)?;
                        spawned = true;
                    } else {
                        self.branch_groups[index].status = BranchGroupStatus::ReadyForSelection;
                        self.try_auto_select_branch_groups();
                    }
                }
                BranchGroupStatus::VotingSpawned => {
                    if !self.tasks_terminal_ok(&group.voter_task_ids) {
                        continue;
                    }
                    if self.goal.branching_policy.voting.require_unification
                        && group.unification_task_id.is_none()
                    {
                        self.spawn_branch_unifier(index, policy)?;
                        spawned = true;
                    } else {
                        self.branch_groups[index].status = BranchGroupStatus::ReadyForSelection;
                        self.try_auto_select_branch_groups();
                    }
                }
                BranchGroupStatus::ReadyForUnification => {
                    if group
                        .unification_task_id
                        .is_some_and(|task_id| self.task_terminal_ok(task_id))
                    {
                        self.branch_groups[index].status = BranchGroupStatus::ReadyForSelection;
                        self.try_auto_select_branch_groups();
                    }
                }
                BranchGroupStatus::ReadyForSelection
                | BranchGroupStatus::Selected
                | BranchGroupStatus::Cancelled => {}
            }
        }
        if spawned {
            self.refresh_goal_status();
        }
        Ok(spawned)
    }

    pub fn ensure_review_frontier(&mut self, policy: &SpawnPolicy) -> Result<bool, DomainError> {
        if !self.goal.review_policy.enabled {
            self.refresh_goal_status();
            return Ok(false);
        }
        if self
            .tasks
            .values()
            .any(|task| task.status == TaskStatus::Failed || task.status == TaskStatus::Blocked)
        {
            self.refresh_goal_status();
            return Ok(false);
        }

        let work_subjects: Vec<TaskId> = self
            .tasks
            .values()
            .filter(|task| task.purpose.is_work_like())
            .filter(|task| task.status.is_terminal_ok())
            .map(|task| task.id)
            .collect();
        let work_done = !work_subjects.is_empty()
            && self
                .tasks
                .values()
                .filter(|task| task.purpose.is_work_like())
                .all(|task| task.status.is_terminal_ok());
        if !work_done {
            self.refresh_goal_status();
            return Ok(false);
        }

        if self.review_rounds.is_empty() && self.goal.review_policy.max_review_rounds > 0 {
            return self.spawn_review_round(policy, 1, work_subjects);
        }

        let mut spawned = false;
        let ready_rounds: Vec<usize> = self
            .review_rounds
            .iter()
            .enumerate()
            .filter(|(_, round)| {
                round.unification_task_id.is_none()
                    && !round.reviewer_task_ids.is_empty()
                    && round.reviewer_task_ids.iter().all(|task_id| {
                        self.tasks
                            .get(task_id)
                            .is_some_and(|task| task.status.is_terminal_ok())
                    })
            })
            .map(|(index, _)| index)
            .collect();

        for index in ready_rounds {
            if self.goal.review_policy.require_unification {
                let root = self.root_task()?.clone();
                let subject_ids = self.review_rounds[index].reviewer_task_ids.clone();
                let round = self.review_rounds[index].round;
                let request = ChildTaskRequest {
                    role: self.goal.review_policy.unifier_role.clone(),
                    purpose: Some(TaskPurpose::Unification {
                        subject_ids: subject_ids.clone(),
                        round,
                    }),
                    title: Some(format!("Unify review round {round}")),
                    subgoal_id: Some(format!("review-round-{round}")),
                    prompt: format!(
                        "Unify critic reviews for goal '{}'. Decide whether the actor output satisfies the objective and identify any retry work.",
                        self.goal.title
                    ),
                    reason: "join reviewer branches into a single satisfaction decision"
                        .to_string(),
                    dependencies: subject_ids,
                    budget: None,
                    sandbox: None,
                    done_criteria: Some(DoneCriteria {
                        tests_pass: false,
                        artifact_exists: true,
                        validator_score_min: Some(
                            self.goal.review_policy.actor_critic.reward_threshold,
                        ),
                    }),
                    execution: None,
                    priority: TaskPriority::High,
                    tags: vec!["review".to_string(), "unification".to_string()],
                };
                policy.ensure_spawn_allowed(&root, std::slice::from_ref(&request))?;
                let task_id = self.insert_child_task(root.id, &root, request)?;
                self.review_rounds[index].unification_task_id = Some(task_id);
                self.review_rounds[index].status = ReviewRoundStatus::ReadyForUnification;
                self.events.push(StateEvent::new(format!(
                    "review_unification_spawned:{task_id}"
                )));
                spawned = true;
            } else {
                self.review_rounds[index].status = ReviewRoundStatus::Unified;
            }
        }

        if !spawned {
            spawned = self.maybe_spawn_next_review_round(policy, &work_subjects)?;
        }
        if !spawned {
            spawned = self.maybe_spawn_actor_retry(policy)?;
        }

        self.refresh_goal_status();
        Ok(spawned)
    }

    pub fn apply_steering(
        &mut self,
        directive: SteeringDirective,
        spawn_policy: &SpawnPolicy,
    ) -> Result<(), DomainError> {
        if !self.goal.control_policy.human_steering_enabled {
            return Err(DomainError::SteeringDenied(
                "human steering is disabled for this goal".to_string(),
            ));
        }
        if self.steering_directives.len() as u32 >= self.goal.control_policy.max_steering_events {
            return Err(DomainError::SteeringDenied(
                "max_steering_events exceeded".to_string(),
            ));
        }

        let event_message = match &directive.kind {
            SteeringDirectiveKind::AddConstraint { constraint } => {
                self.events
                    .push(StateEvent::new(format!("constraint_added:{constraint}")));
                "steering_constraint_added".to_string()
            }
            SteeringDirectiveKind::UpdateObjective { objective_delta } => {
                if !self.goal.control_policy.allow_goal_updates {
                    return Err(DomainError::SteeringDenied(
                        "goal updates are disabled".to_string(),
                    ));
                }
                self.goal.objective = format!(
                    "{}\n\nSteering update: {objective_delta}",
                    self.goal.objective
                );
                "steering_objective_updated".to_string()
            }
            SteeringDirectiveKind::InjectTask {
                role,
                prompt,
                reason,
            } => {
                if !self.goal.control_policy.allow_task_injection {
                    return Err(DomainError::SteeringDenied(
                        "task injection is disabled".to_string(),
                    ));
                }
                let root = self.root_task()?.clone();
                let request = ChildTaskRequest {
                    role: role.clone(),
                    purpose: Some(TaskPurpose::Work),
                    title: Some("Steered work task".to_string()),
                    subgoal_id: None,
                    prompt: prompt.clone(),
                    reason: reason.clone(),
                    dependencies: Vec::new(),
                    budget: None,
                    sandbox: None,
                    done_criteria: None,
                    execution: None,
                    priority: TaskPriority::Normal,
                    tags: vec!["steering".to_string()],
                };
                spawn_policy.ensure_spawn_allowed(&root, std::slice::from_ref(&request))?;
                let task_id = self.insert_child_task(root.id, &root, request)?;
                format!("steering_task_injected:{task_id}")
            }
            SteeringDirectiveKind::RequestResearch { question, reason } => {
                if !self.goal.research_policy.enabled {
                    return Err(DomainError::SteeringDenied(
                        "research tasks are disabled for this goal".to_string(),
                    ));
                }
                let root = self.root_task()?.clone();
                let request = ChildTaskRequest {
                    role: WorkerKind::Research,
                    purpose: Some(TaskPurpose::Research {
                        question: question.clone(),
                    }),
                    title: Some("Steered research task".to_string()),
                    subgoal_id: Some("research".to_string()),
                    prompt: format!(
                        "Answer this research question with sources, confidence, and an information-use plan: {question}"
                    ),
                    reason: reason.clone(),
                    dependencies: Vec::new(),
                    budget: None,
                    sandbox: None,
                    done_criteria: Some(DoneCriteria {
                        tests_pass: false,
                        artifact_exists: true,
                        validator_score_min: Some(self.goal.research_policy.min_confidence),
                    }),
                    execution: None,
                    priority: TaskPriority::High,
                    tags: vec!["steering".to_string(), "research".to_string()],
                };
                spawn_policy.ensure_spawn_allowed(&root, std::slice::from_ref(&request))?;
                let task_id = self.insert_child_task(root.id, &root, request)?;
                format!("steering_research_requested:{task_id}")
            }
            SteeringDirectiveKind::Pause { reason } => {
                self.status = GoalStatus::Paused;
                format!("steering_paused:{reason}")
            }
            SteeringDirectiveKind::Resume { reason } => {
                if self.status == GoalStatus::Paused || self.status == GoalStatus::Blocked {
                    self.status = GoalStatus::Running;
                }
                format!("steering_resumed:{reason}")
            }
            SteeringDirectiveKind::Cancel { reason } => {
                self.cancel(reason.clone());
                format!("steering_cancelled:{reason}")
            }
        };

        self.steering_directives.push(directive);
        self.events.push(StateEvent::new(event_message));
        Ok(())
    }

    pub fn apply_restart_request(
        &mut self,
        request: RestartRequest,
    ) -> Result<RestartRecord, DomainError> {
        if request.goal_id != self.goal.id {
            return Err(DomainError::RestartDenied(
                "restart request goal_id does not match workflow goal".to_string(),
            ));
        }
        if !self.goal.restart_policy.enabled {
            return Err(DomainError::RestartDenied(
                "restart policy is disabled".to_string(),
            ));
        }
        if !self
            .goal
            .restart_policy
            .allowed_scopes
            .contains(&request.scope)
        {
            return Err(DomainError::RestartDenied(format!(
                "restart scope {:?} is not allowed",
                request.scope
            )));
        }
        if !self
            .goal
            .restart_policy
            .allowed_reasons
            .contains(&request.reason)
            && request.reason != RestartReason::Other
        {
            return Err(DomainError::RestartDenied(format!(
                "restart reason {:?} is not allowed",
                request.reason
            )));
        }

        let restarted_task_ids = self.restart_task_ids_for_request(&request)?;
        if restarted_task_ids.is_empty() {
            return Err(DomainError::RestartDenied(
                "no restartable tasks matched the request".to_string(),
            ));
        }
        let is_goal_restart = request.scope == RestartScope::Goal;
        let goal_restart_count = self
            .restart_history
            .iter()
            .filter(|record| record.scope == RestartScope::Goal)
            .count() as u32;
        if is_goal_restart && goal_restart_count >= self.goal.restart_policy.max_goal_restarts {
            return Err(DomainError::RestartDenied(
                "max_goal_restarts exceeded".to_string(),
            ));
        }
        for task_id in &restarted_task_ids {
            let task_restart_count = self
                .restart_history
                .iter()
                .filter(|record| record.restarted_task_ids.contains(task_id))
                .count() as u32;
            if task_restart_count >= self.goal.restart_policy.max_task_restarts {
                return Err(DomainError::RestartDenied(format!(
                    "max_task_restarts exceeded for task {task_id}"
                )));
            }
        }

        let reset_attempts = request
            .reset_attempts
            .unwrap_or(self.goal.restart_policy.reset_attempts_on_restart);
        let preserve_artifacts = request
            .preserve_artifacts
            .unwrap_or(self.goal.restart_policy.preserve_artifacts);
        for task_id in &restarted_task_ids {
            let task = self.task_mut(*task_id)?;
            task.status = TaskStatus::Runnable;
            task.result = None;
            if reset_attempts {
                task.attempts = 0;
            }
        }
        if !preserve_artifacts {
            self.final_artifacts.clear();
        }
        if matches!(
            self.status,
            GoalStatus::Blocked | GoalStatus::Failed | GoalStatus::Cancelled | GoalStatus::Paused
        ) {
            self.status = GoalStatus::Running;
        }

        let record = RestartRecord {
            id: Uuid::new_v4(),
            scope: request.scope,
            reason: request.reason,
            message: request.message,
            task_id: request.task_id,
            restarted_task_ids,
            operator: request.operator,
        };
        self.events.push(StateEvent::new(format!(
            "restart_applied:{}:{}",
            record.id,
            record.restarted_task_ids.len()
        )));
        self.restart_history.push(record.clone());
        self.refresh_goal_status();
        Ok(record)
    }

    pub fn record_task_timeout_and_maybe_restart(
        &mut self,
        task_id: TaskId,
        timeout_seconds: u64,
        message: impl Into<String>,
    ) -> Result<bool, DomainError> {
        let message = message.into();
        let event = TimeoutEvent {
            id: Uuid::new_v4(),
            goal_id: self.goal.id,
            task_id: Some(task_id),
            action: self.goal.timeout_policy.on_task_timeout.clone(),
            timeout_seconds,
            message: message.clone(),
        };
        self.timeout_events.push(event.clone());
        self.events.push(StateEvent::new(format!(
            "task_timed_out:{task_id}:{}",
            event.timeout_seconds
        )));
        match event.action {
            TimeoutAction::RestartIfAllowed => {
                let request = RestartRequest {
                    goal_id: self.goal.id,
                    scope: RestartScope::Task,
                    reason: RestartReason::TaskTimedOut,
                    message,
                    task_id: Some(task_id),
                    reset_attempts: None,
                    preserve_artifacts: None,
                    operator: Some("coordinator_timeout_policy".to_string()),
                };
                self.apply_restart_request(request)?;
                Ok(true)
            }
            TimeoutAction::BlockAndNotify | TimeoutAction::RequestHumanApproval => {
                self.task_mut(task_id)?.status = TaskStatus::Blocked;
                self.refresh_goal_status();
                Ok(false)
            }
            TimeoutAction::Fail => {
                self.task_mut(task_id)?.status = TaskStatus::Failed;
                self.refresh_goal_status();
                Ok(false)
            }
        }
    }

    pub fn branch_task(
        &mut self,
        request: BranchRequest,
        policy: &SpawnPolicy,
    ) -> Result<BranchGroup, DomainError> {
        if request.goal_id != self.goal.id {
            return Err(DomainError::BranchDenied(
                "branch request goal_id does not match workflow goal".to_string(),
            ));
        }
        if !self.goal.branching_policy.enabled {
            return Err(DomainError::BranchDenied(
                "branching policy is disabled".to_string(),
            ));
        }
        if self.branch_groups.len() as u32 >= self.goal.branching_policy.max_branch_groups {
            return Err(DomainError::BranchDenied(
                "max_branch_groups exceeded".to_string(),
            ));
        }
        let candidate_count = request
            .candidate_count
            .clamp(2, self.goal.branching_policy.max_candidates_per_group);
        let target = self.branch_target_task(&request)?;
        if target.parent_id.is_none() && !self.goal.branching_policy.branch_on_root {
            return Err(DomainError::BranchDenied(
                "branch_on_root is disabled".to_string(),
            ));
        }
        if target.subgoal_id.is_some() && !self.goal.branching_policy.branch_on_subgoals {
            return Err(DomainError::BranchDenied(
                "branch_on_subgoals is disabled".to_string(),
            ));
        }
        if target.status == TaskStatus::Running {
            return Err(DomainError::BranchDenied(
                "cannot branch a task that is currently running".to_string(),
            ));
        }

        let group_id = Uuid::new_v4();
        let mut requests = Vec::with_capacity(candidate_count as usize);
        for index in 0..candidate_count {
            let role = request
                .candidate_roles
                .get(index as usize)
                .cloned()
                .unwrap_or_else(|| target.role.clone());
            let execution = request
                .candidate_executions
                .get(index as usize)
                .cloned()
                .unwrap_or_else(|| target.execution.clone().with_role(role.clone()));
            let prompt = request
                .prompt_overrides
                .get(index as usize)
                .cloned()
                .unwrap_or_else(|| {
                    format!(
                        "{}\n\nBranch candidate {} of {} for group {}. Produce an independently reviewable implementation with artifacts and tradeoffs.",
                        target.prompt,
                        index + 1,
                        candidate_count,
                        group_id
                    )
                });
            let mut tags = target.tags.clone();
            tags.extend([
                "branch".to_string(),
                "candidate".to_string(),
                format!("branch-group:{group_id}"),
            ]);
            requests.push(ChildTaskRequest {
                role,
                purpose: Some(TaskPurpose::CandidateBranch {
                    group_id,
                    original_task_id: target.id,
                    candidate_index: index + 1,
                }),
                title: Some(format!("{} branch candidate {}", target.title, index + 1)),
                subgoal_id: target.subgoal_id.clone(),
                prompt,
                reason: request.reason.clone(),
                dependencies: target.dependencies.clone(),
                budget: Some(target.budget.child_budget()),
                sandbox: Some(target.sandbox.clone()),
                done_criteria: Some(target.done_criteria.clone()),
                execution: Some(execution),
                priority: target.priority.clone(),
                tags,
            });
        }
        if self.goal.branching_policy.require_model_diversity {
            let unique_models: BTreeSet<String> = requests
                .iter()
                .filter_map(|candidate| candidate.execution.as_ref())
                .filter_map(|execution| execution.model.candidates.first())
                .map(|model| format!("{}:{}", model.provider.as_str(), model.model))
                .collect();
            if unique_models.len() < requests.len().min(2) {
                return Err(DomainError::BranchDenied(
                    "model diversity is required but candidate executions do not differ"
                        .to_string(),
                ));
            }
        }
        policy.ensure_spawn_allowed(&target, &requests)?;

        let mut candidate_task_ids = Vec::with_capacity(requests.len());
        for child in requests {
            candidate_task_ids.push(self.insert_child_task(target.id, &target, child)?);
        }
        if self.goal.branching_policy.cancel_original_on_branch && !target.status.is_terminal() {
            self.task_mut(target.id)?.status = TaskStatus::Cancelled;
        }
        let group = BranchGroup {
            id: group_id,
            original_task_id: target.id,
            subgoal_id: target.subgoal_id.clone(),
            reason: request.reason,
            selection_strategy: request.selection_strategy.unwrap_or_else(|| {
                self.goal
                    .branching_policy
                    .default_selection_strategy
                    .clone()
            }),
            candidate_task_ids,
            voter_task_ids: Vec::new(),
            unification_task_id: None,
            selected_task_id: None,
            status: BranchGroupStatus::CandidatesSpawned,
            operator: request.operator,
        };
        self.branch_groups.push(group.clone());
        self.events.push(StateEvent::new(format!(
            "branch_group_spawned:{}:{}",
            group.id,
            group.candidate_task_ids.len()
        )));
        self.refresh_goal_status();
        Ok(group)
    }

    pub fn apply_branch_selection(
        &mut self,
        request: BranchSelectionRequest,
    ) -> Result<BranchGroup, DomainError> {
        if request.goal_id != self.goal.id {
            return Err(DomainError::BranchDenied(
                "branch selection goal_id does not match workflow goal".to_string(),
            ));
        }
        let index = self
            .branch_groups
            .iter()
            .position(|group| group.id == request.group_id)
            .ok_or(DomainError::BranchGroupNotFound(request.group_id))?;
        if !self.branch_groups[index]
            .candidate_task_ids
            .contains(&request.selected_task_id)
        {
            return Err(DomainError::BranchDenied(format!(
                "selected task {} is not a candidate in branch group {}",
                request.selected_task_id, request.group_id
            )));
        }
        if !self.task_terminal_ok(request.selected_task_id) {
            return Err(DomainError::BranchDenied(
                "selected branch candidate is not successfully validated".to_string(),
            ));
        }
        self.branch_groups[index].selected_task_id = Some(request.selected_task_id);
        self.branch_groups[index].status = BranchGroupStatus::Selected;
        self.events.push(StateEvent::new(format!(
            "branch_selected:{}:{}:{:?}:{}",
            request.group_id, request.selected_task_id, request.selector, request.reason
        )));
        self.refresh_goal_status();
        Ok(self.branch_groups[index].clone())
    }

    fn maybe_spawn_next_review_round(
        &mut self,
        policy: &SpawnPolicy,
        work_subjects: &[TaskId],
    ) -> Result<bool, DomainError> {
        let Some(last_round) = self.review_rounds.last() else {
            return Ok(false);
        };
        if last_round.status != ReviewRoundStatus::Unified
            || last_round.round >= self.goal.review_policy.max_review_rounds
        {
            return Ok(false);
        }
        let reviewed: BTreeSet<TaskId> = self
            .review_rounds
            .iter()
            .flat_map(|round| round.subject_task_ids.iter().copied())
            .collect();
        let has_unreviewed_work = work_subjects
            .iter()
            .any(|task_id| !reviewed.contains(task_id));
        if !has_unreviewed_work {
            return Ok(false);
        }
        self.spawn_review_round(policy, last_round.round + 1, work_subjects.to_vec())
    }

    fn maybe_spawn_actor_retry(&mut self, policy: &SpawnPolicy) -> Result<bool, DomainError> {
        let actor_critic = &self.goal.review_policy.actor_critic;
        if !actor_critic.enabled || actor_critic.max_actor_retries == 0 {
            return Ok(false);
        }
        if self
            .tasks
            .values()
            .any(|task| !task.status.is_terminal() && task.status != TaskStatus::Cancelled)
        {
            return Ok(false);
        }
        let report = self.satisfaction_report();
        if report.satisfied {
            return Ok(false);
        }
        let blocking_decision = matches!(
            report.latest_decision,
            Some(
                ReviewDecision::ChangesRequested
                    | ReviewDecision::Blocked
                    | ReviewDecision::Inconclusive
            )
        );
        if !blocking_decision
            && report.score >= actor_critic.reward_threshold
            && report.score >= self.goal.review_policy.min_satisfaction_score
        {
            return Ok(false);
        }
        let retry_count = self
            .tasks
            .values()
            .filter(|task| matches!(task.purpose, TaskPurpose::ActorRetry { .. }))
            .count() as u32;
        if retry_count >= actor_critic.max_actor_retries {
            return Ok(false);
        }
        let dependency = self
            .review_rounds
            .iter()
            .rev()
            .find_map(|round| round.unification_task_id)
            .ok_or_else(|| {
                DomainError::InvariantViolation("actor retry requires unification task".to_string())
            })?;
        if !self
            .tasks
            .get(&dependency)
            .is_some_and(|task| task.status.is_terminal_ok())
        {
            return Ok(false);
        }
        let root = self.root_task()?.clone();
        let request = ChildTaskRequest {
            role: actor_critic.actor_retry_role.clone(),
            purpose: Some(TaskPurpose::ActorRetry {
                subject_id: root.id,
                round: retry_count + 1,
            }),
            title: Some(format!("Actor retry round {}", retry_count + 1)),
            subgoal_id: Some(format!("actor-retry-{}", retry_count + 1)),
            prompt: format!(
                "Revise goal '{}' using critic and unifier feedback. Produce an improved actor artifact and explicit evidence for the next review round.",
                self.goal.title
            ),
            reason: "critic reward fell below threshold; bounded actor retry requested".to_string(),
            dependencies: vec![dependency],
            budget: None,
            sandbox: None,
            done_criteria: Some(DoneCriteria {
                tests_pass: false,
                artifact_exists: true,
                validator_score_min: Some(actor_critic.reward_threshold),
            }),
            execution: None,
            priority: TaskPriority::High,
            tags: vec!["actor_retry".to_string(), "review_feedback".to_string()],
        };
        policy.ensure_spawn_allowed(&root, std::slice::from_ref(&request))?;
        let task_id = self.insert_child_task(root.id, &root, request)?;
        self.events
            .push(StateEvent::new(format!("actor_retry_spawned:{task_id}")));
        Ok(true)
    }

    fn spawn_review_round(
        &mut self,
        policy: &SpawnPolicy,
        round: u32,
        subject_task_ids: Vec<TaskId>,
    ) -> Result<bool, DomainError> {
        let root = self.root_task()?.clone();
        let min_reviews = self.goal.review_policy.min_reviews.max(1) as usize;
        let reviewer_roles = if self.goal.review_policy.reviewer_roles.is_empty() {
            vec![self.goal.review_policy.actor_critic.critic_role.clone()]
        } else {
            self.goal.review_policy.reviewer_roles.clone()
        };
        let requests: Vec<ChildTaskRequest> = (0..min_reviews)
            .map(|index| {
                let role = reviewer_roles[index % reviewer_roles.len()].clone();
                ChildTaskRequest {
                    role,
                    purpose: Some(TaskPurpose::Review {
                        subject_id: subject_task_ids[0],
                        round,
                    }),
                    title: Some(format!("Critic review round {round}")),
                    subgoal_id: Some(format!("review-round-{round}")),
                    prompt: format!(
                        "Critique goal '{}'. Review actor artifacts against the objective, done criteria, budget, safety constraints, and missing evidence. Return structured findings and a satisfaction score.",
                        self.goal.title
                    ),
                    reason: "actor output requires independent critic review before goal satisfaction".to_string(),
                    dependencies: subject_task_ids.clone(),
                    budget: None,
                    sandbox: None,
                    done_criteria: Some(DoneCriteria {
                        tests_pass: false,
                        artifact_exists: true,
                        validator_score_min: Some(self.goal.review_policy.actor_critic.reward_threshold),
                    }),
                    execution: None,
                    priority: TaskPriority::High,
                    tags: vec!["review".to_string(), "critic".to_string()],
                }
            })
            .collect();
        policy.ensure_spawn_allowed(&root, &requests)?;

        let mut reviewer_task_ids = Vec::with_capacity(requests.len());
        for request in requests {
            reviewer_task_ids.push(self.insert_child_task(root.id, &root, request)?);
        }
        self.review_rounds.push(ReviewRound {
            round,
            subject_task_ids,
            reviewer_task_ids: reviewer_task_ids.clone(),
            unification_task_id: None,
            status: ReviewRoundStatus::ReviewsSpawned,
        });
        self.events.push(StateEvent::new(format!(
            "review_round_spawned:{round}:{}",
            reviewer_task_ids.len()
        )));
        self.refresh_goal_status();
        Ok(true)
    }

    fn insert_child_task(
        &mut self,
        parent_id: TaskId,
        parent_snapshot: &TaskNode,
        request: ChildTaskRequest,
    ) -> Result<TaskId, DomainError> {
        let child_id = Uuid::new_v4();
        self.tasks
            .get_mut(&parent_id)
            .ok_or(DomainError::TaskNotFound(parent_id))?
            .children
            .push(child_id);
        self.tasks.insert(
            child_id,
            TaskNode::from_child_request(child_id, parent_id, parent_snapshot, request),
        );
        Ok(child_id)
    }

    fn restart_task_ids_for_request(
        &self,
        request: &RestartRequest,
    ) -> Result<Vec<TaskId>, DomainError> {
        let mut task_ids = match request.scope {
            RestartScope::Task => vec![request.task_id.ok_or_else(|| {
                DomainError::RestartDenied("task restart requires task_id".to_string())
            })?],
            RestartScope::Goal => self
                .tasks
                .values()
                .filter(|task| {
                    matches!(
                        task.status,
                        TaskStatus::Blocked
                            | TaskStatus::Failed
                            | TaskStatus::Cancelled
                            | TaskStatus::WaitingApproval
                            | TaskStatus::Running
                    )
                })
                .map(|task| task.id)
                .collect(),
            RestartScope::Failed => self
                .tasks
                .values()
                .filter(|task| task.status == TaskStatus::Failed)
                .map(|task| task.id)
                .collect(),
            RestartScope::Blocked => self
                .tasks
                .values()
                .filter(|task| task.status == TaskStatus::Blocked)
                .map(|task| task.id)
                .collect(),
            RestartScope::TimedOut => {
                let timed_out: BTreeSet<TaskId> = self
                    .timeout_events
                    .iter()
                    .filter_map(|event| event.task_id)
                    .collect();
                self.tasks
                    .values()
                    .filter(|task| timed_out.contains(&task.id))
                    .map(|task| task.id)
                    .collect()
            }
        };
        if request.scope == RestartScope::Goal && task_ids.is_empty() {
            if let Some(root) = self.tasks.values().find(|task| task.parent_id.is_none()) {
                task_ids.push(root.id);
            }
        }
        task_ids.sort();
        task_ids.dedup();
        Ok(task_ids)
    }

    fn branch_target_task(&self, request: &BranchRequest) -> Result<TaskNode, DomainError> {
        if let Some(task_id) = request.target_task_id {
            return Ok(self.task(task_id)?.clone());
        }
        if let Some(subgoal_id) = &request.subgoal_id {
            return self
                .tasks
                .values()
                .find(|task| {
                    task.subgoal_id.as_deref() == Some(subgoal_id.as_str())
                        && !task.purpose.is_review()
                        && !task.purpose.is_unification()
                })
                .cloned()
                .ok_or_else(|| {
                    DomainError::BranchDenied(format!(
                        "no branchable task found for subgoal {subgoal_id}"
                    ))
                });
        }
        self.root_task().cloned()
    }

    fn spawn_branch_votes(
        &mut self,
        group_index: usize,
        policy: &SpawnPolicy,
    ) -> Result<(), DomainError> {
        let root = self.root_task()?.clone();
        let group = self.branch_groups[group_index].clone();
        let voting = self.goal.branching_policy.voting.clone();
        let voter_roles = if voting.voter_roles.is_empty() {
            vec![WorkerKind::Reviewer]
        } else {
            voting.voter_roles
        };
        let vote_count = voting.min_votes.max(1) as usize;
        let requests: Vec<ChildTaskRequest> = (0..vote_count)
            .map(|index| {
                let role = voter_roles[index % voter_roles.len()].clone();
                ChildTaskRequest {
                    role,
                    purpose: Some(TaskPurpose::BranchVote {
                        group_id: group.id,
                        candidate_task_ids: group.candidate_task_ids.clone(),
                    }),
                    title: Some(format!("Vote on branch group {}", group.id)),
                    subgoal_id: group
                        .subgoal_id
                        .clone()
                        .or_else(|| Some(format!("branch-group-{}", group.id))),
                    prompt: format!(
                        "Compare branch candidates for group {} and vote for one implementation. Consider correctness, evidence, maintainability, risk, tests, and goal fit. Return a structured branch_vote with selected_task_id.",
                        group.id
                    ),
                    reason: "branch candidates require independent vote before selection"
                        .to_string(),
                    dependencies: group.candidate_task_ids.clone(),
                    budget: None,
                    sandbox: None,
                    done_criteria: Some(DoneCriteria {
                        tests_pass: false,
                        artifact_exists: true,
                        validator_score_min: Some(
                            self.goal.review_policy.actor_critic.reward_threshold,
                        ),
                    }),
                    execution: None,
                    priority: TaskPriority::High,
                    tags: vec![
                        "branch".to_string(),
                        "vote".to_string(),
                        format!("branch-group:{}", group.id),
                    ],
                }
            })
            .collect();
        policy.ensure_spawn_allowed(&root, &requests)?;
        let mut voter_task_ids = Vec::with_capacity(requests.len());
        for request in requests {
            voter_task_ids.push(self.insert_child_task(root.id, &root, request)?);
        }
        self.branch_groups[group_index].voter_task_ids = voter_task_ids;
        self.branch_groups[group_index].status = BranchGroupStatus::VotingSpawned;
        self.events.push(StateEvent::new(format!(
            "branch_votes_spawned:{}:{}",
            group.id,
            self.branch_groups[group_index].voter_task_ids.len()
        )));
        Ok(())
    }

    fn spawn_branch_unifier(
        &mut self,
        group_index: usize,
        policy: &SpawnPolicy,
    ) -> Result<(), DomainError> {
        let root = self.root_task()?.clone();
        let group = self.branch_groups[group_index].clone();
        let mut dependencies = group.candidate_task_ids.clone();
        dependencies.extend(group.voter_task_ids.iter().copied());
        let request = ChildTaskRequest {
            role: self.goal.branching_policy.voting.unifier_role.clone(),
            purpose: Some(TaskPurpose::BranchUnification {
                group_id: group.id,
                candidate_task_ids: group.candidate_task_ids.clone(),
                voter_task_ids: group.voter_task_ids.clone(),
            }),
            title: Some(format!("Unify branch group {}", group.id)),
            subgoal_id: group
                .subgoal_id
                .clone()
                .or_else(|| Some(format!("branch-group-{}", group.id))),
            prompt: format!(
                "Unify branch group {}. Read candidate artifacts and vote tasks, choose the implementation that best satisfies the goal, and return a structured branch_vote with selected_task_id plus rationale.",
                group.id
            ),
            reason: "join branch votes into a single candidate selection".to_string(),
            dependencies,
            budget: None,
            sandbox: None,
            done_criteria: Some(DoneCriteria {
                tests_pass: false,
                artifact_exists: true,
                validator_score_min: Some(self.goal.review_policy.actor_critic.reward_threshold),
            }),
            execution: None,
            priority: TaskPriority::Critical,
            tags: vec![
                "branch".to_string(),
                "unification".to_string(),
                format!("branch-group:{}", group.id),
            ],
        };
        policy.ensure_spawn_allowed(&root, std::slice::from_ref(&request))?;
        let task_id = self.insert_child_task(root.id, &root, request)?;
        self.branch_groups[group_index].unification_task_id = Some(task_id);
        self.branch_groups[group_index].status = BranchGroupStatus::ReadyForUnification;
        self.events.push(StateEvent::new(format!(
            "branch_unifier_spawned:{}:{}",
            group.id, task_id
        )));
        Ok(())
    }

    fn record_branch_vote(
        &mut self,
        voter_task_id: TaskId,
        vote: BranchVoteOutput,
    ) -> Result<(), DomainError> {
        let group = self
            .branch_groups
            .iter()
            .find(|group| group.id == vote.group_id)
            .ok_or(DomainError::BranchGroupNotFound(vote.group_id))?;
        if !group.candidate_task_ids.contains(&vote.selected_task_id) {
            return Err(DomainError::BranchDenied(format!(
                "vote selected task {} outside branch group {}",
                vote.selected_task_id, vote.group_id
            )));
        }
        self.branch_votes.retain(|record| {
            !(record.group_id == vote.group_id && record.voter_task_id == voter_task_id)
        });
        self.branch_votes.push(BranchVoteRecord {
            voter_task_id,
            group_id: vote.group_id,
            selected_task_id: vote.selected_task_id,
            confidence: vote.confidence,
            rationale: vote.rationale,
        });
        self.events.push(StateEvent::new(format!(
            "branch_vote_recorded:{}:{}:{}",
            vote.group_id, voter_task_id, vote.selected_task_id
        )));
        Ok(())
    }

    fn try_auto_select_branch_groups(&mut self) {
        let selectable: Vec<Uuid> = self
            .branch_groups
            .iter()
            .filter(|group| group.status == BranchGroupStatus::ReadyForSelection)
            .filter(|group| group.selection_strategy != BranchSelectionStrategy::Human)
            .map(|group| group.id)
            .collect();
        for group_id in selectable {
            let Some(index) = self
                .branch_groups
                .iter()
                .position(|group| group.id == group_id)
            else {
                continue;
            };
            let Some(selected_task_id) = self.auto_selected_branch_task(&self.branch_groups[index])
            else {
                continue;
            };
            if self.task_terminal_ok(selected_task_id) {
                self.branch_groups[index].selected_task_id = Some(selected_task_id);
                self.branch_groups[index].status = BranchGroupStatus::Selected;
                self.events.push(StateEvent::new(format!(
                    "branch_auto_selected:{group_id}:{selected_task_id}"
                )));
            }
        }
    }

    fn auto_selected_branch_task(&self, group: &BranchGroup) -> Option<TaskId> {
        match group.selection_strategy {
            BranchSelectionStrategy::Human => None,
            BranchSelectionStrategy::VoterQuorum | BranchSelectionStrategy::UnifierDecision => {
                let mut votes: BTreeMap<TaskId, (u32, f32)> = BTreeMap::new();
                for vote in self
                    .branch_votes
                    .iter()
                    .filter(|vote| vote.group_id == group.id)
                {
                    let entry = votes.entry(vote.selected_task_id).or_insert((0, 0.0));
                    entry.0 += 1;
                    entry.1 += vote.confidence;
                }
                votes
                    .into_iter()
                    .max_by(|left, right| {
                        left.1
                            .0
                            .cmp(&right.1.0)
                            .then_with(|| left.1.1.total_cmp(&right.1.1))
                    })
                    .map(|(task_id, _)| task_id)
            }
            BranchSelectionStrategy::HighestScore => group
                .candidate_task_ids
                .iter()
                .filter_map(|task_id| {
                    self.learning_signals
                        .iter()
                        .rev()
                        .find(|signal| signal.task_id == *task_id)
                        .map(|signal| (*task_id, signal.reward))
                })
                .max_by(|left, right| left.1.total_cmp(&right.1))
                .map(|(task_id, _)| task_id),
        }
    }

    fn tasks_terminal_ok(&self, task_ids: &[TaskId]) -> bool {
        !task_ids.is_empty()
            && task_ids
                .iter()
                .all(|task_id| self.task_terminal_ok(*task_id))
    }

    fn task_terminal_ok(&self, task_id: TaskId) -> bool {
        self.tasks
            .get(&task_id)
            .is_some_and(|task| task.status.is_terminal_ok())
    }

    fn record_learning_signal(&mut self, report: &ValidationReport, purpose: &TaskPurpose) {
        let source = match purpose {
            TaskPurpose::Work | TaskPurpose::CandidateBranch { .. } => {
                LearningSignalSource::ActorValidation
            }
            TaskPurpose::Review { .. } | TaskPurpose::BranchVote { .. } => {
                LearningSignalSource::CriticReview
            }
            TaskPurpose::Unification { .. } | TaskPurpose::BranchUnification { .. } => {
                LearningSignalSource::ReviewUnification
            }
            TaskPurpose::ActorRetry { .. } => LearningSignalSource::ActorRetry,
            TaskPurpose::Research { .. } => LearningSignalSource::Research,
        };
        self.learning_signals.push(LearningSignal {
            task_id: report.task_id,
            source,
            reward: report.score,
            decision: report.review.as_ref().map(|review| review.decision.clone()),
            findings_count: report
                .review
                .as_ref()
                .map(|review| review.findings.len() as u32)
                .unwrap_or(0),
            notes: report.reasons.clone(),
        });
    }

    fn refresh_goal_status(&mut self) {
        let report = self.satisfaction_report();
        self.satisfaction = Some(report.clone());
        self.status = if self
            .tasks
            .values()
            .any(|task| task.status == TaskStatus::Failed)
        {
            GoalStatus::Failed
        } else if self
            .tasks
            .values()
            .any(|task| task.status == TaskStatus::Blocked)
        {
            GoalStatus::Blocked
        } else if self
            .tasks
            .values()
            .any(|task| task.status == TaskStatus::WaitingApproval)
        {
            GoalStatus::WaitingApproval
        } else if report.satisfied {
            GoalStatus::Done
        } else {
            GoalStatus::Running
        };
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

    fn root_task(&self) -> Result<&TaskNode, DomainError> {
        self.tasks
            .values()
            .find(|task| task.parent_id.is_none())
            .ok_or(DomainError::InvariantViolation(
                "root task missing".to_string(),
            ))
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
    Paused,
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
    #[serde(default)]
    pub purpose: TaskPurpose,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subgoal_id: Option<String>,
    pub execution: ExecutionProfile,
    pub prompt: String,
    #[serde(default)]
    pub dependencies: Vec<TaskId>,
    #[serde(default)]
    pub children: Vec<TaskId>,
    pub budget: Budget,
    pub sandbox: SandboxProfile,
    pub done_criteria: DoneCriteria,
    #[serde(default)]
    pub review_doctrine: ReviewDoctrine,
    #[serde(default)]
    pub priority: TaskPriority,
    #[serde(default)]
    pub tags: Vec<String>,
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
        let purpose = req.purpose.unwrap_or(TaskPurpose::Work);
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
            purpose,
            title: req.title.unwrap_or_else(|| parent.title.clone()),
            subgoal_id: req.subgoal_id,
            execution,
            prompt: req.prompt,
            dependencies: req.dependencies,
            children: Vec::new(),
            budget: req.budget.unwrap_or_else(|| parent.budget.child_budget()),
            sandbox: req.sandbox.unwrap_or_else(|| parent.sandbox.clone()),
            done_criteria: req
                .done_criteria
                .unwrap_or_else(|| parent.done_criteria.clone()),
            review_doctrine: req
                .review_doctrine
                .unwrap_or_else(|| parent.review_doctrine.clone()),
            priority: req.priority,
            tags: req.tags,
            result: None,
            attempts: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Critical,
    High,
    Normal,
    Low,
    Background,
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::Normal
    }
}

impl TaskPriority {
    pub fn rank(&self) -> u8 {
        match self {
            Self::Critical => 5,
            Self::High => 4,
            Self::Normal => 3,
            Self::Low => 2,
            Self::Background => 1,
        }
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
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TaskPurpose {
    Work,
    Review {
        subject_id: TaskId,
        round: u32,
    },
    Unification {
        subject_ids: Vec<TaskId>,
        round: u32,
    },
    ActorRetry {
        subject_id: TaskId,
        round: u32,
    },
    CandidateBranch {
        group_id: Uuid,
        original_task_id: TaskId,
        candidate_index: u32,
    },
    BranchVote {
        group_id: Uuid,
        candidate_task_ids: Vec<TaskId>,
    },
    BranchUnification {
        group_id: Uuid,
        candidate_task_ids: Vec<TaskId>,
        voter_task_ids: Vec<TaskId>,
    },
    Research {
        question: String,
    },
}

impl Default for TaskPurpose {
    fn default() -> Self {
        Self::Work
    }
}

impl TaskPurpose {
    pub fn is_work_like(&self) -> bool {
        matches!(
            self,
            Self::Work | Self::ActorRetry { .. } | Self::CandidateBranch { .. }
        )
    }

    pub fn is_review(&self) -> bool {
        matches!(self, Self::Review { .. })
    }

    pub fn is_unification(&self) -> bool {
        matches!(
            self,
            Self::Unification { .. } | Self::BranchUnification { .. }
        )
    }

    pub fn is_research(&self) -> bool {
        matches!(self, Self::Research { .. })
    }

    pub fn branch_group_id(&self) -> Option<Uuid> {
        match self {
            Self::CandidateBranch { group_id, .. }
            | Self::BranchVote { group_id, .. }
            | Self::BranchUnification { group_id, .. } => Some(*group_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskPurposeKind {
    Work,
    Review,
    Unification,
    ActorRetry,
    CandidateBranch,
    BranchVote,
    BranchUnification,
    Research,
}

impl From<&TaskPurpose> for TaskPurposeKind {
    fn from(value: &TaskPurpose) -> Self {
        match value {
            TaskPurpose::Work => Self::Work,
            TaskPurpose::Review { .. } => Self::Review,
            TaskPurpose::Unification { .. } => Self::Unification,
            TaskPurpose::ActorRetry { .. } => Self::ActorRetry,
            TaskPurpose::CandidateBranch { .. } => Self::CandidateBranch,
            TaskPurpose::BranchVote { .. } => Self::BranchVote,
            TaskPurpose::BranchUnification { .. } => Self::BranchUnification,
            TaskPurpose::Research { .. } => Self::Research,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalProgress {
    pub goal_id: GoalId,
    pub title: String,
    pub status: GoalStatus,
    pub total_tasks: u32,
    pub open_tasks: u32,
    pub terminal_ok_tasks: u32,
    pub blocked_tasks: u32,
    pub failed_tasks: u32,
    pub waiting_approval_tasks: u32,
    pub percent_done: f32,
    #[serde(default)]
    pub by_status: BTreeMap<TaskStatus, u32>,
    #[serde(default)]
    pub subgoals: Vec<SubgoalProgress>,
    #[serde(default)]
    pub runnable_tasks: Vec<TaskId>,
    #[serde(default)]
    pub next_tasks: Vec<TaskProgress>,
    pub satisfaction: Option<SatisfactionReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TaskProgress {
    pub task_id: TaskId,
    pub parent_id: Option<TaskId>,
    pub title: String,
    pub subgoal_id: Option<String>,
    pub status: TaskStatus,
    pub role: WorkerKind,
    pub purpose_kind: TaskPurposeKind,
    pub depth: u32,
    pub priority: TaskPriority,
    #[serde(default)]
    pub tags: Vec<String>,
    pub attempts: u32,
    pub dependency_count: u32,
    pub child_count: u32,
    pub runnable: bool,
    #[serde(default)]
    pub blocked_by: Vec<TaskId>,
    pub result: Option<ArtifactRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SubgoalProgress {
    pub subgoal_id: String,
    pub title: Option<String>,
    pub owner_role: Option<WorkerKind>,
    pub status: SubgoalStatus,
    pub total_tasks: u32,
    pub open_tasks: u32,
    pub terminal_ok_tasks: u32,
    #[serde(default)]
    pub runnable_tasks: Vec<TaskId>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubgoalStatus {
    Planned,
    Open,
    Running,
    Done,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub struct TaskQuery {
    #[serde(default)]
    pub subgoal_id: Option<String>,
    #[serde(default)]
    pub statuses: Vec<TaskStatus>,
    #[serde(default)]
    pub roles: Vec<WorkerKind>,
    #[serde(default)]
    pub purpose_kinds: Vec<TaskPurposeKind>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub runnable_only: bool,
    pub limit: Option<usize>,
}

impl TaskQuery {
    fn matches(&self, task: &TaskNode, state: &GoalState) -> bool {
        if let Some(subgoal_id) = &self.subgoal_id {
            if task.subgoal_id.as_deref() != Some(subgoal_id.as_str()) {
                return false;
            }
        }
        if !self.statuses.is_empty() && !self.statuses.contains(&task.status) {
            return false;
        }
        if !self.roles.is_empty() && !self.roles.contains(&task.role) {
            return false;
        }
        let purpose_kind = TaskPurposeKind::from(&task.purpose);
        if !self.purpose_kinds.is_empty() && !self.purpose_kinds.contains(&purpose_kind) {
            return false;
        }
        if !self.tags.is_empty()
            && !self
                .tags
                .iter()
                .all(|tag| task.tags.iter().any(|task_tag| task_tag == tag))
        {
            return false;
        }
        if self.runnable_only
            && !(task.status == TaskStatus::Runnable
                && task.dependencies.iter().all(|id| {
                    state
                        .tasks
                        .get(id)
                        .is_some_and(|dep| dep.status.is_terminal_ok())
                }))
        {
            return false;
        }
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TaskList {
    pub goal_id: GoalId,
    pub query: TaskQuery,
    #[serde(default)]
    pub tasks: Vec<TaskProgress>,
    pub progress: GoalProgress,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ApprovalGatePolicy {
    pub enabled: bool,
    pub require_for_network_open: bool,
    pub require_for_non_isolated_runner: bool,
    pub require_for_secret_access: bool,
    #[serde(default = "default_true")]
    pub require_for_brokered_user_auth: bool,
    pub require_for_dangerous_mcp_tools: bool,
    pub require_for_privileged_runner_capabilities: bool,
    pub require_for_workspace_write_outside_isolation: bool,
    pub never_policy_requires_isolation: bool,
    pub approval_timeout_seconds: Option<u64>,
}

impl Default for ApprovalGatePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            require_for_network_open: true,
            require_for_non_isolated_runner: true,
            require_for_secret_access: true,
            require_for_brokered_user_auth: true,
            require_for_dangerous_mcp_tools: true,
            require_for_privileged_runner_capabilities: true,
            require_for_workspace_write_outside_isolation: true,
            never_policy_requires_isolation: true,
            approval_timeout_seconds: Some(3600),
        }
    }
}

impl ApprovalGatePolicy {
    pub fn evaluate(&self, task: &TaskNode) -> ApprovalEvaluation {
        if !self.enabled {
            return ApprovalEvaluation::not_required();
        }

        let mut reason_codes = Vec::new();
        let mut risk = ApprovalRisk::Low;
        if task.sandbox.approval_policy == ApprovalPolicy::Always {
            reason_codes.push(ApprovalReasonCode::SandboxPolicyAlways);
            risk = risk.max(ApprovalRisk::Medium);
        }
        if task.sandbox.approval_policy == ApprovalPolicy::Never
            && self.never_policy_requires_isolation
            && !task.sandbox.isolated_runner
        {
            reason_codes.push(ApprovalReasonCode::NeverPolicyOutsideIsolation);
            risk = risk.max(ApprovalRisk::Critical);
        }
        if self.require_for_network_open && task.sandbox.network == NetworkAccess::Open {
            reason_codes.push(ApprovalReasonCode::NetworkOpen);
            risk = risk.max(ApprovalRisk::High);
        }
        if self.require_for_non_isolated_runner && !task.sandbox.isolated_runner {
            reason_codes.push(ApprovalReasonCode::NonIsolatedRunner);
            risk = risk.max(ApprovalRisk::High);
        }
        if self.require_for_workspace_write_outside_isolation
            && task.sandbox.filesystem == FilesystemAccess::WorkspaceWrite
            && !task.sandbox.isolated_runner
        {
            reason_codes.push(ApprovalReasonCode::WorkspaceWriteOutsideIsolation);
            risk = risk.max(ApprovalRisk::High);
        }
        if self.require_for_secret_access && task.execution.requires_secret_access() {
            reason_codes.push(ApprovalReasonCode::SecretAccess);
            risk = risk.max(ApprovalRisk::High);
        }
        if self.require_for_brokered_user_auth && task.execution.requires_brokered_user_auth() {
            reason_codes.push(ApprovalReasonCode::BrokeredUserAuth);
            risk = risk.max(ApprovalRisk::Critical);
        }
        if self.require_for_dangerous_mcp_tools && task.execution.mcp_uses_dangerous_tools() {
            reason_codes.push(ApprovalReasonCode::DangerousMcpTool);
            risk = risk.max(ApprovalRisk::Medium);
        }
        if self.require_for_privileged_runner_capabilities
            && task.execution.runner.requires_privileged_capability()
        {
            reason_codes.push(ApprovalReasonCode::PrivilegedRunnerCapability);
            risk = risk.max(ApprovalRisk::High);
        }

        if reason_codes.is_empty() {
            return ApprovalEvaluation::not_required();
        }
        let reason = format!(
            "approval required for {} task {} before attempt {}: {}",
            task.role.as_str(),
            task.id,
            task.attempts + 1,
            reason_codes
                .iter()
                .map(ApprovalReasonCode::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        );
        ApprovalEvaluation {
            required: true,
            risk,
            reason_codes,
            reason,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ApprovalEvaluation {
    pub required: bool,
    pub risk: ApprovalRisk,
    #[serde(default)]
    pub reason_codes: Vec<ApprovalReasonCode>,
    pub reason: String,
}

impl ApprovalEvaluation {
    fn not_required() -> Self {
        Self {
            required: false,
            risk: ApprovalRisk::Low,
            reason_codes: Vec::new(),
            reason: "approval not required".to_string(),
        }
    }
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRisk {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalReasonCode {
    SandboxPolicyAlways,
    NeverPolicyOutsideIsolation,
    NetworkOpen,
    NonIsolatedRunner,
    WorkspaceWriteOutsideIsolation,
    SecretAccess,
    BrokeredUserAuth,
    DangerousMcpTool,
    PrivilegedRunnerCapability,
}

impl ApprovalReasonCode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::SandboxPolicyAlways => "sandbox_policy_always",
            Self::NeverPolicyOutsideIsolation => "never_policy_outside_isolation",
            Self::NetworkOpen => "network_open",
            Self::NonIsolatedRunner => "non_isolated_runner",
            Self::WorkspaceWriteOutsideIsolation => "workspace_write_outside_isolation",
            Self::SecretAccess => "secret_access",
            Self::BrokeredUserAuth => "brokered_user_auth",
            Self::DangerousMcpTool => "dangerous_mcp_tool",
            Self::PrivilegedRunnerCapability => "privileged_runner_capability",
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ReviewPolicy {
    pub enabled: bool,
    pub min_reviews: u32,
    pub max_review_rounds: u32,
    pub join_strategy: ReviewJoinStrategy,
    pub require_unification: bool,
    #[serde(default)]
    pub reviewer_roles: Vec<WorkerKind>,
    pub unifier_role: WorkerKind,
    pub min_satisfaction_score: f32,
    pub actor_critic: ActorCriticPolicy,
}

impl Default for ReviewPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            min_reviews: 1,
            max_review_rounds: 2,
            join_strategy: ReviewJoinStrategy::AllRequired,
            require_unification: true,
            reviewer_roles: vec![WorkerKind::Reviewer],
            unifier_role: WorkerKind::PatchMerger,
            min_satisfaction_score: 0.85,
            actor_critic: ActorCriticPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewJoinStrategy {
    AllRequired,
    AnyRequired,
    Quorum { min_passed: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ActorCriticPolicy {
    pub enabled: bool,
    pub critic_role: WorkerKind,
    pub actor_retry_role: WorkerKind,
    pub max_actor_retries: u32,
    pub reward_threshold: f32,
}

impl Default for ActorCriticPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            critic_role: WorkerKind::Reviewer,
            actor_retry_role: WorkerKind::Planner,
            max_actor_retries: 1,
            reward_threshold: 0.85,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ReviewRound {
    pub round: u32,
    #[serde(default)]
    pub subject_task_ids: Vec<TaskId>,
    #[serde(default)]
    pub reviewer_task_ids: Vec<TaskId>,
    pub unification_task_id: Option<TaskId>,
    pub status: ReviewRoundStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRoundStatus {
    ReviewsSpawned,
    ReadyForUnification,
    Unified,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SatisfactionReport {
    pub satisfied: bool,
    pub score: f32,
    pub work_done: bool,
    pub reviews_required: u32,
    pub reviews_passed: u32,
    pub unification_done: bool,
    pub all_tasks_terminal: bool,
    pub open_tasks: u32,
    pub latest_decision: Option<ReviewDecision>,
    pub open_findings: u32,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct LearningSignal {
    pub task_id: TaskId,
    pub source: LearningSignalSource,
    pub reward: f32,
    pub decision: Option<ReviewDecision>,
    pub findings_count: u32,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearningSignalSource {
    ActorValidation,
    CriticReview,
    ReviewUnification,
    ActorRetry,
    Research,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ControlLoopPolicy {
    pub mode: ControlLoopMode,
    pub max_frontier_rounds: u32,
    pub max_idle_rounds: u32,
    pub human_steering_enabled: bool,
    pub allow_goal_updates: bool,
    pub allow_task_injection: bool,
    pub max_steering_events: u32,
    #[serde(default)]
    pub stop_conditions: Vec<ControlStopCondition>,
}

impl Default for ControlLoopPolicy {
    fn default() -> Self {
        Self {
            mode: ControlLoopMode::BoundedUntilSatisfied,
            max_frontier_rounds: 128,
            max_idle_rounds: 4,
            human_steering_enabled: true,
            allow_goal_updates: true,
            allow_task_injection: true,
            max_steering_events: 128,
            stop_conditions: vec![
                ControlStopCondition::GoalSatisfied,
                ControlStopCondition::Blocked,
                ControlStopCondition::BudgetExhausted,
                ControlStopCondition::Cancelled,
                ControlStopCondition::HumanPaused,
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlLoopMode {
    BoundedUntilSatisfied,
    MonitorUntilCancelled,
    HumanSteeredContinuous,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlStopCondition {
    GoalSatisfied,
    Blocked,
    BudgetExhausted,
    Cancelled,
    HumanPaused,
    MaxFrontierRounds,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct RestartPolicy {
    pub enabled: bool,
    pub max_goal_restarts: u32,
    pub max_task_restarts: u32,
    pub reset_attempts_on_restart: bool,
    pub preserve_artifacts: bool,
    #[serde(default)]
    pub allowed_scopes: Vec<RestartScope>,
    #[serde(default)]
    pub allowed_reasons: Vec<RestartReason>,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_goal_restarts: 3,
            max_task_restarts: 5,
            reset_attempts_on_restart: false,
            preserve_artifacts: true,
            allowed_scopes: vec![
                RestartScope::Goal,
                RestartScope::Task,
                RestartScope::Blocked,
            ],
            allowed_reasons: vec![
                RestartReason::OperatorRequested,
                RestartReason::RunnerLost,
                RestartReason::TaskTimedOut,
                RestartReason::GoalTimedOut,
                RestartReason::ConfigChanged,
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct TimeoutPolicy {
    pub enabled: bool,
    pub goal_timeout_seconds: Option<u64>,
    pub task_run_timeout_seconds: Option<u64>,
    pub idle_timeout_seconds: Option<u64>,
    pub approval_timeout_seconds: Option<u64>,
    pub runner_dispatch_timeout_seconds: Option<u64>,
    pub runner_call_timeout_seconds: Option<u64>,
    pub on_goal_timeout: TimeoutAction,
    pub on_task_timeout: TimeoutAction,
}

impl TimeoutPolicy {
    pub fn task_timeout_seconds(&self, task: &TaskNode) -> u64 {
        let configured = if self.enabled {
            self.runner_call_timeout_seconds
                .or(self.task_run_timeout_seconds)
        } else {
            None
        };
        configured
            .unwrap_or(task.budget.remaining_runtime_seconds)
            .min(task.budget.remaining_runtime_seconds.max(1))
            .max(1)
    }

    pub fn dispatch_timeout_seconds(&self) -> u64 {
        if self.enabled {
            self.runner_dispatch_timeout_seconds.unwrap_or(30).max(1)
        } else {
            30
        }
    }
}

impl Default for TimeoutPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            goal_timeout_seconds: Some(14_400),
            task_run_timeout_seconds: Some(3_600),
            idle_timeout_seconds: Some(900),
            approval_timeout_seconds: Some(3_600),
            runner_dispatch_timeout_seconds: Some(30),
            runner_call_timeout_seconds: Some(3_600),
            on_goal_timeout: TimeoutAction::BlockAndNotify,
            on_task_timeout: TimeoutAction::RestartIfAllowed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutAction {
    BlockAndNotify,
    Fail,
    RestartIfAllowed,
    RequestHumanApproval,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct RestartRequest {
    pub goal_id: GoalId,
    pub scope: RestartScope,
    pub reason: RestartReason,
    pub message: String,
    pub task_id: Option<TaskId>,
    #[serde(default)]
    pub reset_attempts: Option<bool>,
    #[serde(default)]
    pub preserve_artifacts: Option<bool>,
    pub operator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct RestartRecord {
    pub id: Uuid,
    pub scope: RestartScope,
    pub reason: RestartReason,
    pub message: String,
    pub task_id: Option<TaskId>,
    #[serde(default)]
    pub restarted_task_ids: Vec<TaskId>,
    pub operator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestartScope {
    Goal,
    Task,
    Failed,
    Blocked,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestartReason {
    OperatorRequested,
    RunnerLost,
    TaskTimedOut,
    GoalTimedOut,
    ConfigChanged,
    ModelChanged,
    ReviewRejected,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct TimeoutEvent {
    pub id: Uuid,
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub action: TimeoutAction,
    pub timeout_seconds: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct BranchingPolicy {
    pub enabled: bool,
    pub max_branch_groups: u32,
    pub max_candidates_per_group: u32,
    pub branch_on_root: bool,
    pub branch_on_subgoals: bool,
    pub cancel_original_on_branch: bool,
    pub require_model_diversity: bool,
    pub default_selection_strategy: BranchSelectionStrategy,
    pub voting: BranchVotingPolicy,
}

impl Default for BranchingPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_branch_groups: 8,
            max_candidates_per_group: 4,
            branch_on_root: true,
            branch_on_subgoals: true,
            cancel_original_on_branch: true,
            require_model_diversity: false,
            default_selection_strategy: BranchSelectionStrategy::Human,
            voting: BranchVotingPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct BranchVotingPolicy {
    pub enabled: bool,
    pub min_votes: u32,
    #[serde(default)]
    pub voter_roles: Vec<WorkerKind>,
    pub require_unification: bool,
    pub unifier_role: WorkerKind,
}

impl Default for BranchVotingPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            min_votes: 2,
            voter_roles: vec![WorkerKind::Reviewer, WorkerKind::Tester],
            require_unification: true,
            unifier_role: WorkerKind::PatchMerger,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BranchSelectionStrategy {
    Human,
    VoterQuorum,
    HighestScore,
    UnifierDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct BranchRequest {
    pub goal_id: GoalId,
    pub target_task_id: Option<TaskId>,
    pub subgoal_id: Option<String>,
    pub reason: String,
    pub candidate_count: u32,
    #[serde(default)]
    pub candidate_roles: Vec<WorkerKind>,
    #[serde(default)]
    pub candidate_executions: Vec<ExecutionProfile>,
    #[serde(default)]
    pub prompt_overrides: Vec<String>,
    pub selection_strategy: Option<BranchSelectionStrategy>,
    pub operator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct BranchSelectionRequest {
    pub goal_id: GoalId,
    pub group_id: Uuid,
    pub selected_task_id: TaskId,
    pub selector: BranchSelector,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BranchSelector {
    Human,
    VoterQuorum,
    HighestScore,
    Unifier,
    CoordinatorPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct BranchGroup {
    pub id: Uuid,
    pub original_task_id: TaskId,
    pub subgoal_id: Option<String>,
    pub reason: String,
    pub selection_strategy: BranchSelectionStrategy,
    #[serde(default)]
    pub candidate_task_ids: Vec<TaskId>,
    #[serde(default)]
    pub voter_task_ids: Vec<TaskId>,
    pub unification_task_id: Option<TaskId>,
    pub selected_task_id: Option<TaskId>,
    pub status: BranchGroupStatus,
    pub operator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BranchGroupStatus {
    CandidatesSpawned,
    VotingSpawned,
    ReadyForUnification,
    ReadyForSelection,
    Selected,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct BranchVoteOutput {
    pub group_id: Uuid,
    pub selected_task_id: TaskId,
    #[serde(default)]
    pub ranked_task_ids: Vec<TaskId>,
    pub confidence: f32,
    pub rationale: String,
}

impl BranchVoteOutput {
    pub fn for_task_purpose(purpose: &TaskPurpose) -> Option<Self> {
        match purpose {
            TaskPurpose::BranchVote {
                group_id,
                candidate_task_ids,
            }
            | TaskPurpose::BranchUnification {
                group_id,
                candidate_task_ids,
                ..
            } => candidate_task_ids.first().map(|selected_task_id| Self {
                group_id: *group_id,
                selected_task_id: *selected_task_id,
                ranked_task_ids: candidate_task_ids.clone(),
                confidence: 0.75,
                rationale: "stub branch vote selected the first candidate".to_string(),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct BranchVoteRecord {
    pub voter_task_id: TaskId,
    pub group_id: Uuid,
    pub selected_task_id: TaskId,
    pub confidence: f32,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SteeringDirective {
    pub id: Uuid,
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub operator: Option<String>,
    pub message: String,
    pub kind: SteeringDirectiveKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SteeringDirectiveKind {
    AddConstraint {
        constraint: String,
    },
    UpdateObjective {
        objective_delta: String,
    },
    InjectTask {
        role: WorkerKind,
        prompt: String,
        reason: String,
    },
    RequestResearch {
        question: String,
        reason: String,
    },
    Pause {
        reason: String,
    },
    Resume {
        reason: String,
    },
    Cancel {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ResearchPolicy {
    pub enabled: bool,
    pub question_first: bool,
    pub require_sources: bool,
    pub require_use_plan: bool,
    pub max_search_depth: u32,
    pub min_confidence: f32,
    #[serde(default)]
    pub allowed_providers: Vec<SearchProviderKind>,
    #[serde(default)]
    pub source_quality_order: Vec<SourceQuality>,
}

impl Default for ResearchPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            question_first: true,
            require_sources: true,
            require_use_plan: true,
            max_search_depth: 5,
            min_confidence: 0.75,
            allowed_providers: vec![
                SearchProviderKind::Web,
                SearchProviderKind::Docs,
                SearchProviderKind::Mcp,
                SearchProviderKind::Memory,
            ],
            source_quality_order: vec![
                SourceQuality::Primary,
                SourceQuality::OfficialDocs,
                SourceQuality::PeerReviewed,
                SourceQuality::ReputableSecondary,
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchProviderKind {
    Web,
    Docs,
    Mcp,
    Memory,
    Repo,
    Tracker,
    Database,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceQuality {
    Primary,
    OfficialDocs,
    PeerReviewed,
    ReputableSecondary,
    Community,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ResearchOutput {
    pub question: String,
    pub answer: String,
    #[serde(default)]
    pub sources: Vec<SourceArtifact>,
    pub confidence: f32,
    pub use_plan: InformationUsePlan,
    #[serde(default)]
    pub open_questions: Vec<String>,
}

impl ResearchOutput {
    pub fn for_task_purpose(purpose: &TaskPurpose) -> Option<Self> {
        match purpose {
            TaskPurpose::Research { question } => Some(Self {
                question: question.clone(),
                answer: "stub research result; live search worker is not enabled".to_string(),
                sources: vec![SourceArtifact {
                    title: "stub research source".to_string(),
                    uri: "memory://research/stub".to_string(),
                    quality: SourceQuality::Unknown,
                    captured_at: None,
                    quote: None,
                    summary: "placeholder source proving the structured research contract"
                        .to_string(),
                    confidence: 0.75,
                }],
                confidence: 0.75,
                use_plan: InformationUsePlan {
                    facts_to_use: vec![
                        "Treat this as a placeholder until a live research worker runs."
                            .to_string(),
                    ],
                    facts_to_avoid: Vec::new(),
                    proposed_task_updates: Vec::new(),
                    validation_checks: vec![
                        "replace stub source with live source capture".to_string(),
                    ],
                },
                open_questions: Vec::new(),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SourceArtifact {
    pub title: String,
    pub uri: String,
    pub quality: SourceQuality,
    pub captured_at: Option<String>,
    pub quote: Option<String>,
    pub summary: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct InformationUsePlan {
    #[serde(default)]
    pub facts_to_use: Vec<String>,
    #[serde(default)]
    pub facts_to_avoid: Vec<String>,
    #[serde(default)]
    pub proposed_task_updates: Vec<ChildTaskRequest>,
    #[serde(default)]
    pub validation_checks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryPolicy {
    pub enabled: bool,
    pub store: MemoryStoreRef,
    #[serde(default)]
    pub vector: VectorMemoryPolicy,
    #[serde(default)]
    pub embedding: EmbeddingPolicy,
    #[serde(default)]
    pub retrieval: MemoryRetrievalPolicy,
    pub write_policy: MemoryWritePolicy,
    pub fork_join: MemoryForkJoinPolicy,
    #[serde(default)]
    pub scopes: Vec<MemoryScope>,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            store: MemoryStoreRef {
                kind: MemoryStoreKind::ZepGraphiti,
                endpoint: Some("http://graphiti-mcp:8000/mcp/".to_string()),
                namespace: Some("coat".to_string()),
                mcp_server_name: Some("graphiti-memory".to_string()),
                secret_refs: Vec::new(),
            },
            vector: VectorMemoryPolicy::default(),
            embedding: EmbeddingPolicy::default(),
            retrieval: MemoryRetrievalPolicy::default(),
            write_policy: MemoryWritePolicy::ReviewedFactsOnly,
            fork_join: MemoryForkJoinPolicy::default(),
            scopes: vec![
                MemoryScope::Goal,
                MemoryScope::Task,
                MemoryScope::Repo,
                MemoryScope::Persona,
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryStoreRef {
    pub kind: MemoryStoreKind,
    pub endpoint: Option<String>,
    pub namespace: Option<String>,
    pub mcp_server_name: Option<String>,
    #[serde(default)]
    pub secret_refs: Vec<SecretRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStoreKind {
    ZepGraphiti,
    Letta,
    PostgresPgvector,
    Qdrant,
    LanceDb,
    Sqlite,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct VectorMemoryPolicy {
    pub enabled: bool,
    pub store: MemoryStoreRef,
    pub collection: String,
    pub write_embeddings: bool,
    pub search_embeddings: bool,
    pub hybrid_search: bool,
    pub top_k: u32,
    pub min_score: Option<f32>,
}

impl Default for VectorMemoryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            store: MemoryStoreRef {
                kind: MemoryStoreKind::Qdrant,
                endpoint: Some("http://qdrant:6333".to_string()),
                namespace: Some("coat_memory".to_string()),
                mcp_server_name: Some("qdrant-memory".to_string()),
                secret_refs: Vec::new(),
            },
            collection: "coat_memory".to_string(),
            write_embeddings: true,
            search_embeddings: true,
            hybrid_search: true,
            top_k: 8,
            min_score: Some(0.2),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct EmbeddingPolicy {
    pub provider: EmbeddingProviderKind,
    pub model: String,
    pub endpoint: Option<String>,
    pub dimensions: u32,
    pub batch_size: u32,
    pub normalize: bool,
    #[serde(default)]
    pub secret_refs: Vec<SecretRef>,
}

impl Default for EmbeddingPolicy {
    fn default() -> Self {
        Self {
            provider: EmbeddingProviderKind::OpenAi,
            model: "text-embedding-3-large".to_string(),
            endpoint: Some("https://api.openai.com/v1/embeddings".to_string()),
            dimensions: 3072,
            batch_size: 64,
            normalize: true,
            secret_refs: vec![SecretRef {
                provider: SecretProvider::Env,
                name: "OPENAI_API_KEY".to_string(),
                key: None,
                namespace: None,
                audience: Some("embedding-provider".to_string()),
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingProviderKind {
    OpenAi,
    OpenAiCompatible,
    HuggingFaceTei,
    SentenceTransformers,
    VoyageAi,
    Cohere,
    Ollama,
    LocalProcess,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryRetrievalPolicy {
    pub retrieve_before_task: bool,
    pub max_hits: u32,
    pub min_score: Option<f32>,
    pub include_branch_memories: bool,
    pub rerank: bool,
    pub fusion: RetrievalFusion,
}

impl Default for MemoryRetrievalPolicy {
    fn default() -> Self {
        Self {
            retrieve_before_task: true,
            max_hits: 8,
            min_score: Some(0.2),
            include_branch_memories: false,
            rerank: false,
            fusion: RetrievalFusion::GraphThenVector,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalFusion {
    DenseOnly,
    HybridRrf,
    GraphThenVector,
    VectorThenGraph,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWritePolicy {
    Disabled,
    AppendOnly,
    ReviewedFactsOnly,
    UnifierCurated,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryForkJoinPolicy {
    pub inherit_parent_context: bool,
    pub write_branch_memories: bool,
    pub join_strategy: MemoryJoinStrategy,
}

impl Default for MemoryForkJoinPolicy {
    fn default() -> Self {
        Self {
            inherit_parent_context: true,
            write_branch_memories: true,
            join_strategy: MemoryJoinStrategy::UnifierCurated,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryJoinStrategy {
    AppendOnly,
    CriticCurated,
    UnifierCurated,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Goal,
    Task,
    Repo,
    Persona,
    User,
    Global,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryEvent {
    pub task_id: Option<TaskId>,
    pub scope: MemoryScope,
    pub action: MemoryEventAction,
    pub store_kind: MemoryStoreKind,
    pub key: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEventAction {
    Read,
    Write,
    Fork,
    Join,
    Invalidate,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryEpisode {
    pub title: String,
    pub content: String,
    pub source: MemoryEpisodeSource,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryEpisodeSource {
    pub source_type: MemoryEpisodeSourceType,
    pub uri: Option<String>,
    pub actor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEpisodeSourceType {
    Actor,
    Critic,
    Unifier,
    Research,
    Human,
    Tool,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryWriteRequest {
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub scope: MemoryScope,
    pub key: Option<String>,
    pub episode: MemoryEpisode,
    pub store: Option<MemoryStoreRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryWriteResponse {
    pub event: MemoryEvent,
    pub key: String,
    pub stored_locally: bool,
    pub external_ref: Option<String>,
    #[serde(default)]
    pub adapter_reports: Vec<MemoryAdapterReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemorySearchRequest {
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub query: String,
    #[serde(default)]
    pub scopes: Vec<MemoryScope>,
    pub limit: Option<usize>,
    pub store: Option<MemoryStoreRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemorySearchResponse {
    #[serde(default)]
    pub hits: Vec<MemorySearchHit>,
    #[serde(default)]
    pub events: Vec<MemoryEvent>,
    #[serde(default)]
    pub adapter_reports: Vec<MemoryAdapterReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemorySearchHit {
    pub key: String,
    pub scope: MemoryScope,
    pub score: f32,
    pub summary: String,
    pub source: MemoryEpisodeSource,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryContextRequest {
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub objective: String,
    #[serde(default)]
    pub scopes: Vec<MemoryScope>,
    pub limit: Option<usize>,
    pub store: Option<MemoryStoreRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryContextResponse {
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub query: String,
    #[serde(default)]
    pub hits: Vec<MemorySearchHit>,
    pub use_plan: InformationUsePlan,
    #[serde(default)]
    pub adapter_reports: Vec<MemoryAdapterReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryJoinRequest {
    pub goal_id: GoalId,
    pub parent_task_id: Option<TaskId>,
    #[serde(default)]
    pub branch_task_ids: Vec<TaskId>,
    pub unifier_task_id: Option<TaskId>,
    #[serde(default)]
    pub promote_keys: Vec<String>,
    #[serde(default)]
    pub invalidate_keys: Vec<String>,
    pub decision: Option<ReviewDecision>,
    pub reason: String,
    pub store: Option<MemoryStoreRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryJoinResponse {
    #[serde(default)]
    pub promoted: Vec<MemoryEvent>,
    #[serde(default)]
    pub invalidated: Vec<MemoryEvent>,
    #[serde(default)]
    pub adapter_reports: Vec<MemoryAdapterReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryRepairRequest {
    pub goal_id: Option<GoalId>,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub store_kinds: Vec<MemoryStoreKind>,
    #[serde(default)]
    pub include_invalidated: bool,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryRepairResponse {
    pub scanned: usize,
    pub selected: usize,
    pub repaired: usize,
    pub skipped: usize,
    #[serde(default)]
    pub adapter_reports: Vec<MemoryAdapterReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct MemoryAdapterReport {
    pub store_kind: MemoryStoreKind,
    pub operation: String,
    pub attempted: bool,
    pub success: bool,
    pub external_ref: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionProfile {
    pub runner: RunnerSelector,
    pub model: ModelRoute,
    pub persona: PersonaSpec,
    pub mcp: McpContextRef,
    pub notifications: NotificationPolicy,
    #[serde(default)]
    pub results: ResultChannelPolicy,
}

impl ExecutionProfile {
    pub fn with_role(mut self, role: WorkerKind) -> Self {
        self.runner.worker = Some(role.clone());
        self.persona = self.persona.with_default_name_for_role(&role);
        self
    }

    fn requires_secret_access(&self) -> bool {
        !self.mcp.secret_refs.is_empty()
            || self
                .mcp
                .servers
                .iter()
                .any(|server| server.auth.requires_secret_access())
            || self
                .notifications
                .targets
                .iter()
                .any(|target| target.secret_ref.is_some())
    }

    fn requires_brokered_user_auth(&self) -> bool {
        self.mcp.auth_distribution.requires_brokered_user_approval()
            || self
                .mcp
                .servers
                .iter()
                .any(|server| server.auth.requires_brokered_user_auth())
    }

    fn mcp_uses_dangerous_tools(&self) -> bool {
        self.mcp.servers.iter().any(|server| {
            server
                .allowed_tools
                .iter()
                .any(|tool| dangerous_tool_name(tool))
        })
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
            results: ResultChannelPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub struct ResultChannelPolicy {
    #[serde(default)]
    pub git: GitResultPolicy,
    #[serde(default)]
    pub object_storage: ObjectStoragePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct GitResultPolicy {
    pub enabled: bool,
    pub remote: Option<String>,
    pub base_ref: Option<String>,
    pub branch_prefix: String,
    pub worktree_root: Option<String>,
    pub push_on_success: bool,
    pub require_clean_diff: bool,
    pub include_patch_artifact: bool,
}

impl GitResultPolicy {
    pub fn branch_for(&self, goal_id: GoalId, task_id: TaskId) -> String {
        let prefix = self.branch_prefix.trim_end_matches('/');
        format!("{prefix}/{goal_id}/{task_id}")
    }
}

impl Default for GitResultPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            remote: Some("origin".to_string()),
            base_ref: Some("HEAD".to_string()),
            branch_prefix: "coat/task".to_string(),
            worktree_root: Some("/worktrees".to_string()),
            push_on_success: false,
            require_clean_diff: true,
            include_patch_artifact: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ObjectStoragePolicy {
    pub enabled: bool,
    pub store: Option<ObjectStoreRef>,
    pub key_prefix_template: String,
    pub require_manifest: bool,
    pub max_inline_bytes: u64,
}

impl Default for ObjectStoragePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            store: None,
            key_prefix_template: "goals/{goal_id}/tasks/{task_id}".to_string(),
            require_manifest: true,
            max_inline_bytes: 262_144,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ObjectStoreRef {
    pub kind: ObjectStoreKind,
    pub bucket: String,
    pub prefix: Option<String>,
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub force_path_style: bool,
    pub secret_ref: Option<SecretRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectStoreKind {
    AwsS3,
    S3Compatible,
    Minio,
    CloudflareR2,
    GcsS3,
    Other,
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

    fn requires_privileged_capability(&self) -> bool {
        self.required_capabilities
            .iter()
            .any(RunnerCapability::requires_approval)
            || self.required_labels.iter().any(|(key, value)| {
                approval_sensitive_label(key.as_str()) || approval_sensitive_label(value.as_str())
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

impl RunnerLocality {
    fn mismatch_reason(
        &self,
        coordinator_node_id: Option<&str>,
        registration: &RunnerRegistration,
    ) -> Option<String> {
        match self {
            Self::AnyNode => None,
            Self::SameNode | Self::LocalOnly => match coordinator_node_id {
                Some(node_id) if node_id == registration.node_id => None,
                Some(node_id) => Some(format!(
                    "locality requires node {node_id}, runner is on {}",
                    registration.node_id
                )),
                None => Some("locality requires a coordinator node id".to_string()),
            },
            Self::RemoteOnly => match coordinator_node_id {
                Some(node_id) if node_id != registration.node_id => None,
                Some(node_id) => Some(format!(
                    "locality requires a remote node, runner is also on {node_id}"
                )),
                None => Some("remote locality requires a coordinator node id".to_string()),
            },
        }
    }
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
    GitWorktree,
    Git,
    ObjectStorage,
    S3Compatible,
    Browser,
    Notifications,
    LocalModels,
    Vllm,
    OpenAiCompatible,
    Gpu,
    NetworkOpen,
}

impl RunnerCapability {
    fn requires_approval(&self) -> bool {
        matches!(self, Self::NetworkOpen)
    }
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
        self.select_candidate(registration, None, None)
    }

    pub fn select_candidate<'a>(
        &'a self,
        registration: &'a RunnerRegistration,
        goal_id: Option<GoalId>,
        task_id: Option<TaskId>,
    ) -> Option<&'a ModelCandidate> {
        let mut candidates = self.compatible_candidates(registration);
        if candidates.is_empty() {
            return None;
        }

        match self.strategy {
            ModelRoutingStrategy::FirstAvailable => {
                candidates.sort_by(candidate_priority_order);
                candidates.first().copied()
            }
            ModelRoutingStrategy::LowestLatency => {
                candidates.sort_by(|left, right| {
                    label_i64(left, &["latency_ms", "p50_latency_ms", "p95_latency_ms"])
                        .unwrap_or(i64::MAX)
                        .cmp(
                            &label_i64(right, &["latency_ms", "p50_latency_ms", "p95_latency_ms"])
                                .unwrap_or(i64::MAX),
                        )
                        .then_with(|| candidate_priority_order(left, right))
                });
                candidates.first().copied()
            }
            ModelRoutingStrategy::LowestCost => {
                candidates.sort_by(|left, right| {
                    label_i64(left, &["cost_microusd_per_1k", "cost_per_million_microusd"])
                        .unwrap_or(i64::MAX)
                        .cmp(
                            &label_i64(
                                right,
                                &["cost_microusd_per_1k", "cost_per_million_microusd"],
                            )
                            .unwrap_or(i64::MAX),
                        )
                        .then_with(|| candidate_priority_order(left, right))
                });
                candidates.first().copied()
            }
            ModelRoutingStrategy::HighestQuality => {
                candidates.sort_by(|left, right| {
                    model_quality_score(right)
                        .cmp(&model_quality_score(left))
                        .then_with(|| candidate_priority_order(left, right))
                });
                candidates.first().copied()
            }
            ModelRoutingStrategy::Weighted => {
                select_weighted_candidate(candidates, goal_id, task_id, false)
            }
            ModelRoutingStrategy::StickyPerGoal => {
                select_weighted_candidate(candidates, goal_id, task_id, true)
            }
        }
    }

    fn compatible_candidates<'a>(
        &'a self,
        registration: &'a RunnerRegistration,
    ) -> Vec<&'a ModelCandidate> {
        let exact: Vec<&ModelCandidate> = self
            .candidates
            .iter()
            .flat_map(|requested| {
                registration
                    .models
                    .iter()
                    .filter(move |actual| actual.matches_candidate(requested))
            })
            .filter(|candidate| self.has_required_features(candidate))
            .collect();

        if !exact.is_empty() {
            return exact;
        }

        if self.fallback == ModelFallbackPolicy::DisallowFallback {
            return Vec::new();
        }

        registration
            .models
            .iter()
            .filter(|candidate| self.has_required_features(candidate))
            .filter(|candidate| {
                self.fallback == ModelFallbackPolicy::AllowFallback
                    || candidate.provider.is_local_provider()
            })
            .collect()
    }

    fn has_required_features(&self, candidate: &ModelCandidate) -> bool {
        self.required_features
            .iter()
            .all(|feature| candidate.features.contains(feature))
    }

    pub fn dispatch_score(
        &self,
        candidate: &ModelCandidate,
        goal_id: GoalId,
        task_id: TaskId,
    ) -> i64 {
        match self.strategy {
            ModelRoutingStrategy::FirstAvailable => 1_000_000 - candidate.priority as i64,
            ModelRoutingStrategy::LowestLatency => {
                1_000_000
                    - label_i64(
                        candidate,
                        &["latency_ms", "p50_latency_ms", "p95_latency_ms"],
                    )
                    .unwrap_or(999_999)
            }
            ModelRoutingStrategy::LowestCost => {
                1_000_000
                    - label_i64(
                        candidate,
                        &["cost_microusd_per_1k", "cost_per_million_microusd"],
                    )
                    .unwrap_or(999_999)
            }
            ModelRoutingStrategy::HighestQuality => model_quality_score(candidate),
            ModelRoutingStrategy::Weighted => {
                let bucket = stable_model_hash(Some(goal_id), Some(task_id), candidate);
                (bucket % candidate.weight.max(1) as u64) as i64
                    + (1_000_000 - candidate.priority as i64)
            }
            ModelRoutingStrategy::StickyPerGoal => {
                let bucket = stable_model_hash(Some(goal_id), None, candidate);
                (bucket % candidate.weight.max(1) as u64) as i64
                    + (1_000_000 - candidate.priority as i64)
            }
        }
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

fn candidate_priority_order(left: &&ModelCandidate, right: &&ModelCandidate) -> std::cmp::Ordering {
    left.priority
        .cmp(&right.priority)
        .then_with(|| left.provider.as_str().cmp(right.provider.as_str()))
        .then_with(|| left.model.cmp(&right.model))
}

fn label_i64(candidate: &ModelCandidate, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| candidate.labels.get(*key))
        .and_then(|value| value.parse::<i64>().ok())
}

fn model_quality_score(candidate: &ModelCandidate) -> i64 {
    let tier_score = match candidate.labels.get("quality_tier").map(String::as_str) {
        Some("frontier") => 1_000_000,
        Some("high") => 750_000,
        Some("medium") => 500_000,
        Some("low") => 250_000,
        _ => 0,
    };
    let feature_score = candidate.features.len() as i64 * 10_000;
    let context_score = candidate.context_window.unwrap_or_default() as i64 / 128;
    tier_score + feature_score + context_score - candidate.priority as i64
}

fn select_weighted_candidate<'a>(
    mut candidates: Vec<&'a ModelCandidate>,
    goal_id: Option<GoalId>,
    task_id: Option<TaskId>,
    sticky_goal_only: bool,
) -> Option<&'a ModelCandidate> {
    candidates.sort_by(candidate_priority_order);
    let total_weight: u64 = candidates
        .iter()
        .map(|candidate| candidate.weight.max(1) as u64)
        .sum();
    if total_weight == 0 {
        return candidates.first().copied();
    }
    let mut bucket = if sticky_goal_only {
        stable_route_hash(goal_id, None)
    } else {
        stable_route_hash(goal_id, task_id)
    } % total_weight;
    for candidate in candidates {
        let weight = candidate.weight.max(1) as u64;
        if bucket < weight {
            return Some(candidate);
        }
        bucket -= weight;
    }
    None
}

fn stable_route_hash(goal_id: Option<GoalId>, task_id: Option<TaskId>) -> u64 {
    let mut hasher = DefaultHasher::new();
    goal_id.hash(&mut hasher);
    task_id.hash(&mut hasher);
    hasher.finish()
}

fn stable_model_hash(
    goal_id: Option<GoalId>,
    task_id: Option<TaskId>,
    candidate: &ModelCandidate,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    goal_id.hash(&mut hasher);
    task_id.hash(&mut hasher);
    candidate.provider.as_str().hash(&mut hasher);
    candidate.model.hash(&mut hasher);
    hasher.finish()
}

fn dangerous_tool_name(tool: &str) -> bool {
    let tool = tool.to_ascii_lowercase();
    [
        "add", "apply", "approve", "create", "delete", "deploy", "exec", "merge", "publish",
        "push", "run", "shell", "update", "write",
    ]
    .iter()
    .any(|needle| tool.contains(needle))
}

fn approval_sensitive_label(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        "danger",
        "deploy",
        "host",
        "privileged",
        "production",
        "root",
        "unrestricted",
    ]
    .iter()
    .any(|needle| value.contains(needle))
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

impl ModelProviderKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::OpenAi => "open_ai",
            Self::OpenAiCompatible => "open_ai_compatible",
            Self::Vllm => "vllm",
            Self::Ollama => "ollama",
            Self::LlamaCpp => "llama_cpp",
            Self::Anthropic => "anthropic",
            Self::HuggingFace => "hugging_face",
            Self::LocalProcess => "local_process",
            Self::Other => "other",
        }
    }

    fn is_local_provider(&self) -> bool {
        matches!(
            self,
            Self::Vllm | Self::Ollama | Self::LlamaCpp | Self::LocalProcess
        )
    }
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
    #[serde(default)]
    pub auth_distribution: AuthDistributionPolicy,
}

impl Default for McpContextRef {
    fn default() -> Self {
        Self {
            context_id: None,
            servers: Vec::new(),
            secret_refs: Vec::new(),
            propagation: McpContextPropagation::CoordinatorIssued,
            token_ttl_seconds: Some(900),
            auth_distribution: AuthDistributionPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpContextPropagation {
    CoordinatorIssued,
    RunnerResolvesRefs,
    WorkloadIdentity,
    RunnerLocalOnly,
    #[serde(rename = "oauth_device_broker")]
    OAuthDeviceBroker,
    ExternalBroker,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct AuthDistributionPolicy {
    pub mode: AuthDistributionMode,
    #[serde(default)]
    pub allowed_materials: Vec<AuthMaterialKind>,
    #[serde(default)]
    pub required_runner_labels: BTreeMap<String, String>,
    pub lease_ttl_seconds: Option<u64>,
    pub renewal: AuthRenewalPolicy,
    pub allow_node_local_device_session: bool,
    pub allow_secret_sync: bool,
    pub require_human_approval_for_brokered_user_auth: bool,
}

impl Default for AuthDistributionPolicy {
    fn default() -> Self {
        Self {
            mode: AuthDistributionMode::RunnerResolvesRefs,
            allowed_materials: vec![
                AuthMaterialKind::ApiToken,
                AuthMaterialKind::McpBearerToken,
                AuthMaterialKind::OAuthAccessToken,
                AuthMaterialKind::OAuthRefreshToken,
                AuthMaterialKind::WorkloadIdentityToken,
            ],
            required_runner_labels: BTreeMap::new(),
            lease_ttl_seconds: Some(900),
            renewal: AuthRenewalPolicy::ManualOrBrokered,
            allow_node_local_device_session: true,
            allow_secret_sync: false,
            require_human_approval_for_brokered_user_auth: true,
        }
    }
}

impl AuthDistributionPolicy {
    fn requires_brokered_user_approval(&self) -> bool {
        self.require_human_approval_for_brokered_user_auth
            && match self.mode {
                AuthDistributionMode::OAuthDeviceBroker | AuthDistributionMode::ExternalBroker => {
                    true
                }
                AuthDistributionMode::CoordinatorIssuesLease => self
                    .allowed_materials
                    .iter()
                    .any(AuthMaterialKind::is_user_delegated),
                AuthDistributionMode::RunnerLocalOnly
                | AuthDistributionMode::RunnerResolvesRefs
                | AuthDistributionMode::WorkloadIdentity => false,
            }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthDistributionMode {
    RunnerLocalOnly,
    RunnerResolvesRefs,
    CoordinatorIssuesLease,
    WorkloadIdentity,
    #[serde(rename = "oauth_device_broker")]
    OAuthDeviceBroker,
    ExternalBroker,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMaterialKind {
    ApiToken,
    McpBearerToken,
    #[serde(rename = "oauth_access_token")]
    OAuthAccessToken,
    #[serde(rename = "oauth_refresh_token")]
    OAuthRefreshToken,
    DeviceAuthSession,
    WorkloadIdentityToken,
    LocalCliSession,
    ServiceAccount,
    Other,
}

impl AuthMaterialKind {
    fn is_user_delegated(&self) -> bool {
        matches!(
            self,
            Self::OAuthAccessToken
                | Self::OAuthRefreshToken
                | Self::DeviceAuthSession
                | Self::LocalCliSession
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthRenewalPolicy {
    None,
    ManualOrBrokered,
    BrokerCanRefresh,
    RunnerCanRefresh,
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
    Secret {
        secret: SecretRef,
    },
    WorkloadIdentity {
        audience: String,
    },
    #[serde(rename = "oauth_delegation")]
    OAuthDelegation {
        token_exchange_secret: SecretRef,
    },
    DeviceAuthSession {
        session_ref: SecretRef,
        refresh_ref: Option<SecretRef>,
        provider: DeviceAuthProvider,
        #[serde(default = "default_true")]
        node_local: bool,
    },
    BrokeredUserSession {
        broker: SecretRef,
        provider: DeviceAuthProvider,
        #[serde(default)]
        requested_scopes: Vec<String>,
    },
}

impl McpAuthRef {
    fn requires_secret_access(&self) -> bool {
        matches!(
            self,
            Self::Secret { .. }
                | Self::OAuthDelegation { .. }
                | Self::DeviceAuthSession { .. }
                | Self::BrokeredUserSession { .. }
        )
    }

    fn requires_brokered_user_auth(&self) -> bool {
        matches!(self, Self::BrokeredUserSession { .. })
            || matches!(
                self,
                Self::DeviceAuthSession {
                    node_local: false,
                    ..
                }
            )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceAuthProvider {
    Codex,
    ClaudeCode,
    OpenAi,
    Anthropic,
    Github,
    Google,
    Other,
}

fn default_true() -> bool {
    true
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
    OnePassword,
    Bitwarden,
    Doppler,
    Sops,
    ExternalBroker,
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
        self.evaluate_for_task(task, None).matched
    }

    pub fn evaluate_for_task(
        &self,
        task: &TaskNode,
        coordinator_node_id: Option<&str>,
    ) -> RunnerMatchEvaluation {
        let mut reasons = Vec::new();
        let selector = &task.execution.runner;

        if let Some(expected_runner) = &selector.runner_id {
            if expected_runner != &self.runner_id {
                reasons.push(format!(
                    "runner_id mismatch: requested {expected_runner}, runner is {}",
                    self.runner_id
                ));
            }
        }
        if let Some(worker) = &selector.worker {
            if !self.roles.contains(worker) {
                reasons.push(format!(
                    "runner does not advertise role {}",
                    worker.as_str()
                ));
            }
        }
        for capability in &selector.required_capabilities {
            if !self.capabilities.contains(capability) {
                reasons.push(format!("missing capability {capability:?}"));
            }
        }
        for (key, value) in &selector.required_labels {
            match self.labels.get(key) {
                Some(actual) if actual == value => {}
                Some(actual) => reasons.push(format!(
                    "label {key} mismatch: requested {value}, runner has {actual}"
                )),
                None => reasons.push(format!("missing label {key}={value}")),
            }
        }
        if let Some(reason) = selector.locality.mismatch_reason(coordinator_node_id, self) {
            reasons.push(reason);
        }
        reasons.extend(self.mcp_mismatch_reasons(&task.execution.mcp));

        let selected_model =
            task.execution
                .model
                .select_candidate(self, Some(task.goal_id), Some(task.id));
        if selected_model.is_none() {
            reasons.push("no compatible model for task model route".to_string());
        }

        if !reasons.is_empty() {
            return RunnerMatchEvaluation {
                matched: false,
                selected_model: None,
                score: 0,
                reasons,
            };
        }

        let selected_model = selected_model.cloned();
        let model_score = selected_model
            .as_ref()
            .map(|model| {
                task.execution
                    .model
                    .dispatch_score(model, task.goal_id, task.id)
            })
            .unwrap_or_default();
        let mut score = model_score;
        score += self.capabilities.len() as i64 * 100;
        score += selector.required_labels.len() as i64 * 500;
        if selector.runner_id.as_deref() == Some(self.runner_id.as_str()) {
            score += 10_000;
        }
        if matches!(
            selector.locality,
            RunnerLocality::SameNode | RunnerLocality::LocalOnly
        ) {
            score += 1_000;
        }

        RunnerMatchEvaluation {
            matched: true,
            selected_model,
            score,
            reasons: vec![format!(
                "runner {} on node {} satisfies role, capability, locality, MCP, and model constraints",
                self.runner_id, self.node_id
            )],
        }
    }

    fn mcp_mismatch_reasons(&self, mcp: &McpContextRef) -> Vec<String> {
        if mcp.servers.is_empty()
            && mcp.secret_refs.is_empty()
            && mcp.auth_distribution.required_runner_labels.is_empty()
        {
            return Vec::new();
        }

        let mut reasons = Vec::new();
        if (!mcp.servers.is_empty() || !mcp.secret_refs.is_empty())
            && !self.capabilities.contains(&RunnerCapability::McpTools)
        {
            reasons.push("task has MCP context but runner lacks mcp_tools capability".to_string());
        }
        for (label, expected) in &mcp.auth_distribution.required_runner_labels {
            match self.labels.get(label) {
                Some(actual) if actual == expected => {}
                Some(actual) => reasons.push(format!(
                    "runner label {}={} does not satisfy MCP auth distribution requirement {}={}",
                    label, actual, label, expected
                )),
                None => reasons.push(format!(
                    "runner lacks MCP auth distribution label {}={}",
                    label, expected
                )),
            }
        }
        if !mcp.servers.is_empty() && !self.mcp_servers.is_empty() {
            for requested in &mcp.servers {
                if !self.mcp_servers.iter().any(|available| {
                    available.name == requested.name || available.uri == requested.uri
                }) {
                    reasons.push(format!(
                        "runner does not advertise requested MCP server {} ({})",
                        requested.name, requested.uri
                    ));
                }
            }
        }
        reasons
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunnerMatchEvaluation {
    pub matched: bool,
    pub selected_model: Option<ModelCandidate>,
    pub score: i64,
    pub reasons: Vec<String>,
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
    pub coordinator_node_id: Option<String>,
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
    pub candidates: Vec<RunnerDispatchCandidate>,
    #[serde(default)]
    pub rejections: Vec<RunnerDispatchRejection>,
    #[serde(default)]
    pub reasons: Vec<String>,
}

impl RunnerDispatchDecision {
    pub fn choose(request: RunnerDispatchRequest) -> Self {
        let mut candidates = Vec::new();
        let mut rejections = Vec::new();

        for registration in &request.registered_runners {
            let evaluation = registration
                .evaluate_for_task(&request.task, request.coordinator_node_id.as_deref());
            if evaluation.matched {
                candidates.push(RunnerDispatchCandidate {
                    runner_id: registration.runner_id.clone(),
                    node_id: registration.node_id.clone(),
                    runner_endpoint: registration.endpoint.clone(),
                    model: evaluation.selected_model,
                    score: evaluation.score,
                    reasons: evaluation.reasons,
                });
            } else {
                rejections.push(RunnerDispatchRejection {
                    runner_id: registration.runner_id.clone(),
                    node_id: registration.node_id.clone(),
                    reasons: evaluation.reasons,
                });
            }
        }

        candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.runner_id.cmp(&right.runner_id))
        });

        let selected = candidates.first().cloned();

        match selected {
            Some(selected) => Self {
                status: RunnerDispatchStatus::Matched,
                runner_id: Some(selected.runner_id.clone()),
                runner_endpoint: Some(selected.runner_endpoint.clone()),
                model: selected.model.clone(),
                mcp_context: request.task.execution.mcp.clone(),
                candidates,
                rejections,
                reasons: vec![
                    format!(
                        "selected runner {} with dispatch score {}",
                        selected.runner_id, selected.score
                    ),
                    "matched runner capabilities, labels, locality, MCP, role, and model route"
                        .to_string(),
                ],
            },
            None => Self {
                status: RunnerDispatchStatus::NoMatch,
                runner_id: None,
                runner_endpoint: None,
                model: None,
                mcp_context: request.task.execution.mcp.clone(),
                candidates,
                rejections,
                reasons: vec![
                    "no registered runner satisfied the task execution profile".to_string(),
                ],
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct RunnerDispatchCandidate {
    pub runner_id: String,
    pub node_id: String,
    pub runner_endpoint: String,
    pub model: Option<ModelCandidate>,
    pub score: i64,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct RunnerDispatchRejection {
    pub runner_id: String,
    pub node_id: String,
    #[serde(default)]
    pub reasons: Vec<String>,
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
    GitBranch,
    GitCommit,
    GitWorktree,
    ObjectStorageObject,
    ObjectStoragePrefix,
    ArtifactManifest,
    Schema,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct GitResultRef {
    pub repo: Option<String>,
    pub remote: Option<String>,
    pub base_ref: Option<String>,
    pub branch: String,
    pub worktree_path: Option<String>,
    pub commit: Option<String>,
    pub pushed: bool,
    pub pull_request_url: Option<String>,
    pub diff_uri: Option<String>,
}

impl GitResultRef {
    pub fn as_artifact(&self) -> ArtifactRef {
        ArtifactRef {
            kind: if self.commit.is_some() {
                ArtifactKind::GitCommit
            } else {
                ArtifactKind::GitBranch
            },
            uri: self
                .commit
                .as_ref()
                .map(|commit| format!("git+branch://{}?commit={commit}", self.branch))
                .unwrap_or_else(|| format!("git+branch://{}", self.branch)),
            description: format!("git result branch {}", self.branch),
            sha256: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ObjectStorageArtifactRef {
    pub store: ObjectStoreRef,
    pub key: String,
    pub uri: String,
    pub content_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub sha256: Option<String>,
    pub description: String,
}

impl ObjectStorageArtifactRef {
    pub fn as_artifact(&self) -> ArtifactRef {
        ArtifactRef {
            kind: ArtifactKind::ObjectStorageObject,
            uri: self.uri.clone(),
            description: self.description.clone(),
            sha256: self.sha256.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct AgentRunRequest {
    pub goal_id: GoalId,
    pub task: TaskNode,
    #[serde(default)]
    pub context_artifacts: Vec<ArtifactRef>,
    pub coordinator_trace_id: Option<String>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct AgentRunResult {
    pub task_id: TaskId,
    pub status: WorkerRunStatus,
    pub summary: String,
    #[serde(default)]
    pub review: Option<ReviewOutput>,
    #[serde(default)]
    pub research: Option<ResearchOutput>,
    #[serde(default)]
    pub branch_vote: Option<BranchVoteOutput>,
    pub runner_id: Option<String>,
    pub model_used: Option<ModelCandidate>,
    pub mcp_context_used: Option<McpContextRef>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub git_result: Option<GitResultRef>,
    #[serde(default)]
    pub object_artifacts: Vec<ObjectStorageArtifactRef>,
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
            review: ReviewOutput::for_task_purpose(&task.purpose),
            research: ResearchOutput::for_task_purpose(&task.purpose),
            branch_vote: BranchVoteOutput::for_task_purpose(&task.purpose),
            runner_id: Some("stub-runner".to_string()),
            model_used: task.execution.model.candidates.first().cloned(),
            mcp_context_used: Some(task.execution.mcp.clone()),
            artifacts: vec![ArtifactRef {
                kind: ArtifactKind::Report,
                uri: format!("memory://task/{}", task.id),
                description: "stub worker result".to_string(),
                sha256: None,
            }],
            git_result: if task.execution.results.git.enabled {
                Some(GitResultRef {
                    repo: None,
                    remote: task.execution.results.git.remote.clone(),
                    base_ref: task.execution.results.git.base_ref.clone(),
                    branch: task.execution.results.git.branch_for(task.goal_id, task.id),
                    worktree_path: task
                        .execution
                        .results
                        .git
                        .worktree_root
                        .as_ref()
                        .map(|root| {
                            format!(
                                "{}/{}/{}",
                                root.trim_end_matches('/'),
                                task.goal_id,
                                task.id
                            )
                        }),
                    commit: None,
                    pushed: false,
                    pull_request_url: None,
                    diff_uri: None,
                })
            } else {
                None
            },
            object_artifacts: Vec::new(),
            child_requests: Vec::new(),
            confidence: 0.9,
            next_actions: Vec::new(),
            diagnostics: Vec::new(),
            notification_reports: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ReviewOutput {
    pub decision: ReviewDecision,
    pub reward: f32,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    pub retry_recommended: bool,
    pub unification_summary: Option<String>,
}

impl ReviewOutput {
    pub fn for_task_purpose(purpose: &TaskPurpose) -> Option<Self> {
        match purpose {
            TaskPurpose::Review { .. } => Some(Self {
                decision: ReviewDecision::Accept,
                reward: 0.9,
                findings: Vec::new(),
                retry_recommended: false,
                unification_summary: None,
            }),
            TaskPurpose::Unification { .. } | TaskPurpose::BranchUnification { .. } => Some(Self {
                decision: ReviewDecision::Accept,
                reward: 0.9,
                findings: Vec::new(),
                retry_recommended: false,
                unification_summary: Some(
                    "stub unification accepted reviewer evidence".to_string(),
                ),
            }),
            TaskPurpose::BranchVote { .. } => Some(Self {
                decision: ReviewDecision::Accept,
                reward: 0.8,
                findings: Vec::new(),
                retry_recommended: false,
                unification_summary: Some("stub vote selected a branch candidate".to_string()),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Accept,
    ChangesRequested,
    Blocked,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ReviewFinding {
    pub severity: ReviewSeverity,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub suggested_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRunStatus {
    Done,
    Partial,
    Blocked,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ChildTaskRequest {
    pub role: WorkerKind,
    pub purpose: Option<TaskPurpose>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subgoal_id: Option<String>,
    pub prompt: String,
    pub reason: String,
    #[serde(default)]
    pub dependencies: Vec<TaskId>,
    pub budget: Option<Budget>,
    pub sandbox: Option<SandboxProfile>,
    pub done_criteria: Option<DoneCriteria>,
    #[serde(default)]
    pub review_doctrine: Option<ReviewDoctrine>,
    pub execution: Option<ExecutionProfile>,
    #[serde(default)]
    pub priority: TaskPriority,
    #[serde(default)]
    pub tags: Vec<String>,
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
    #[serde(default)]
    pub review: Option<ReviewOutput>,
    #[serde(default)]
    pub research: Option<ResearchOutput>,
    #[serde(default)]
    pub branch_vote: Option<BranchVoteOutput>,
    pub status_after_validation: TaskStatus,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub missing_criteria: Vec<String>,
    #[serde(default)]
    pub child_requests: Vec<ChildTaskRequest>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub git_result: Option<GitResultRef>,
    #[serde(default)]
    pub object_artifacts: Vec<ObjectStorageArtifactRef>,
}

impl ValidationReport {
    pub fn from_result(req: ValidationRequest) -> Self {
        let mut reasons = Vec::new();
        let mut missing_criteria = Vec::new();
        let review = req.result.review.clone();
        let research = req.result.research.clone();
        let branch_vote = req.result.branch_vote.clone();
        let effective_score = review
            .as_ref()
            .map(|review| review.reward)
            .or_else(|| research.as_ref().map(|research| research.confidence))
            .unwrap_or(req.result.confidence);
        let status_after_validation = match req.result.status {
            WorkerRunStatus::Blocked => TaskStatus::Blocked,
            WorkerRunStatus::Failed | WorkerRunStatus::TimedOut => TaskStatus::Failed,
            WorkerRunStatus::Partial => TaskStatus::Runnable,
            WorkerRunStatus::Done => TaskStatus::Done,
        };

        if matches!(
            req.result.status,
            WorkerRunStatus::Blocked | WorkerRunStatus::Failed | WorkerRunStatus::TimedOut
        ) {
            let artifacts = result_artifacts(&req.result);
            return Self {
                goal_id: req.goal_id,
                task_id: req.task.id,
                passed: false,
                score: effective_score,
                review,
                research,
                branch_vote,
                status_after_validation,
                reasons: vec![format!("worker returned {:?}", req.result.status)],
                missing_criteria,
                child_requests: req.result.child_requests,
                artifacts,
                git_result: req.result.git_result,
                object_artifacts: req.result.object_artifacts,
            };
        }

        let artifact_exists = !req.result.artifacts.is_empty()
            || req.result.git_result.is_some()
            || !req.result.object_artifacts.is_empty();
        if req.task.done_criteria.artifact_exists && !artifact_exists {
            missing_criteria.push("artifact_exists".to_string());
        }
        if let Some(min_score) = req.task.done_criteria.validator_score_min {
            if effective_score < min_score
                && (req.task.purpose.is_work_like() || req.task.purpose.is_research())
            {
                missing_criteria.push("validator_score_min".to_string());
            }
        }
        if req.task.purpose.is_research() {
            match &research {
                Some(research) => {
                    if research.sources.is_empty() {
                        missing_criteria.push("research_sources".to_string());
                    }
                    if research.use_plan.facts_to_use.is_empty()
                        && research.use_plan.proposed_task_updates.is_empty()
                        && research.use_plan.validation_checks.is_empty()
                    {
                        missing_criteria.push("information_use_plan".to_string());
                    }
                }
                None => missing_criteria.push("research_output".to_string()),
            }
        }
        if matches!(
            req.task.purpose,
            TaskPurpose::BranchVote { .. } | TaskPurpose::BranchUnification { .. }
        ) && branch_vote.is_none()
        {
            missing_criteria.push("branch_vote_output".to_string());
        }
        let passed = req.result.status == WorkerRunStatus::Done && missing_criteria.is_empty();
        if let Some(review) = &review {
            reasons.push(format!(
                "review decision {:?} with reward {:.2}",
                review.decision, review.reward
            ));
            if review.retry_recommended {
                reasons.push("critic recommended actor retry".to_string());
            }
        }
        if let Some(research) = &research {
            reasons.push(format!(
                "research answered '{}' with {} sources and confidence {:.2}",
                research.question,
                research.sources.len(),
                research.confidence
            ));
        }
        if let Some(vote) = &branch_vote {
            reasons.push(format!(
                "branch vote selected {} with confidence {:.2}",
                vote.selected_task_id, vote.confidence
            ));
        }
        if passed {
            reasons.push("worker result satisfies current done criteria".to_string());
        } else {
            reasons.push("worker result needs retry, child tasks, or escalation".to_string());
        }
        let artifacts = result_artifacts(&req.result);
        Self {
            goal_id: req.goal_id,
            task_id: req.task.id,
            passed,
            score: effective_score,
            review,
            research,
            branch_vote,
            status_after_validation: if passed {
                TaskStatus::Done
            } else {
                TaskStatus::Runnable
            },
            reasons,
            missing_criteria,
            child_requests: req.result.child_requests,
            artifacts,
            git_result: req.result.git_result,
            object_artifacts: req.result.object_artifacts,
        }
    }
}

fn result_artifacts(result: &AgentRunResult) -> Vec<ArtifactRef> {
    let mut artifacts = result.artifacts.clone();
    if let Some(git_result) = &result.git_result {
        artifacts.push(git_result.as_artifact());
    }
    artifacts.extend(
        result
            .object_artifacts
            .iter()
            .map(ObjectStorageArtifactRef::as_artifact),
    );
    artifacts
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
    pub attempt: u32,
    pub reason: String,
    pub status: ApprovalStatus,
    pub risk: ApprovalRisk,
    #[serde(default)]
    pub reason_codes: Vec<ApprovalReasonCode>,
    pub sandbox: SandboxProfile,
    pub requested_action: String,
    #[serde(default)]
    pub notification_reports: Vec<NotificationDeliveryReport>,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct GoalStorePolicy {
    pub state_authority: GoalStateAuthority,
    pub read_model_backend: GoalReadModelBackend,
    pub event_backend: GoalEventBackend,
    pub projection_mode: GoalStoreProjectionMode,
    pub protocol_package: String,
    pub protocol_version: String,
}

impl Default for GoalStorePolicy {
    fn default() -> Self {
        Self {
            state_authority: GoalStateAuthority::RestateWorkflow,
            read_model_backend: GoalReadModelBackend::Postgres,
            event_backend: GoalEventBackend::PostgresOutbox,
            projection_mode: GoalStoreProjectionMode::BestEffort,
            protocol_package: "coat.v1".to_string(),
            protocol_version: "coat.v1".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct EventSource {
    pub id: String,
    pub kind: EventSourceKind,
    pub enabled: bool,
    pub description: String,
    pub namespace: Option<String>,
    pub webhook: Option<WebhookEventSource>,
    pub schedule: Option<ScheduledEventSource>,
    pub calendar: Option<CalendarEventSource>,
    pub route: EventGoalRoute,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventSourceKind {
    Webhook,
    CloudEventsWebhook,
    Cron,
    CalendarPoll,
    CalendarPush,
    Queue,
    PubSub,
    GitHubWebhook,
    GitLabWebhook,
    JiraWebhook,
    LinearWebhook,
    SlackEvent,
    Email,
    ObjectStorage,
    Kubernetes,
    Manual,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct WebhookEventSource {
    pub path: String,
    pub auth: WebhookAuthPolicy,
    pub accepts_cloudevents: bool,
    pub max_payload_bytes: u64,
    pub dedupe_header: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct WebhookAuthPolicy {
    pub kind: WebhookAuthKind,
    pub secret_ref: Option<SecretRef>,
    pub header_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebhookAuthKind {
    None,
    SharedSecretHeader,
    HmacSha256,
    BearerToken,
    Basic,
    Mtls,
    OidcJwt,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ScheduledEventSource {
    pub schedule: ScheduleSpec,
    pub missed_run_policy: MissedRunPolicy,
    pub jitter_seconds: u64,
    pub max_catch_up_runs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ScheduleSpec {
    pub kind: ScheduleKind,
    pub expression: String,
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    Cron,
    RRule,
    IntervalSeconds,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MissedRunPolicy {
    Skip,
    FireOnce,
    CatchUpBounded,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct CalendarEventSource {
    pub provider: CalendarProvider,
    pub calendar_id: String,
    pub watch_channel_id: Option<String>,
    pub sync_token_ref: Option<SecretRef>,
    pub lookahead_seconds: u64,
    pub poll_interval_seconds: u64,
    pub mcp_context: Option<McpContextRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalendarProvider {
    GoogleCalendar,
    OutlookCalendar,
    CalDav,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct EventGoalRoute {
    pub mode: EventRouteMode,
    pub goal_template: Option<GoalTriggerTemplate>,
    pub target_goal_id: Option<GoalId>,
    pub steering_directive: Option<SteeringDirective>,
    pub require_approval: bool,
    pub dedupe_window_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventRouteMode {
    RecordOnly,
    CreateGoal,
    CreateResearchGoal,
    SteerGoal,
    HumanReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalTriggerTemplate {
    pub title_template: String,
    pub objective_template: String,
    pub repo: Option<String>,
    pub worker_role: WorkerKind,
    pub done_criteria: DoneCriteria,
    pub budget: Budget,
    pub execution: ExecutionProfile,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Default for GoalTriggerTemplate {
    fn default() -> Self {
        Self {
            title_template: "Respond to {{event_type}}".to_string(),
            objective_template:
                "Investigate and respond to event {{event_id}} from {{source_id}}: {{subject}}"
                    .to_string(),
            repo: None,
            worker_role: WorkerKind::Planner,
            done_criteria: DoneCriteria::default(),
            budget: Budget::default_goal(),
            execution: ExecutionProfile::default(),
            tags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ExternalEvent {
    pub id: String,
    pub source_id: String,
    pub source_kind: EventSourceKind,
    pub event_type: String,
    pub subject: Option<String>,
    pub dedupe_key: String,
    pub occurred_at: Option<String>,
    pub received_at: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TriggeredGoalRequest {
    pub event: ExternalEvent,
    pub route: EventGoalRoute,
    pub goal: Option<GoalSpec>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TriggeredGoalResponse {
    pub accepted: bool,
    pub status: TriggeredGoalStatus,
    pub event_id: String,
    pub goal_id: Option<GoalId>,
    pub deduped: bool,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggeredGoalStatus {
    Recorded,
    Submitted,
    AwaitingHumanReview,
    Deduped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ProtocolMetadata {
    pub protocol_version: String,
    pub idempotency_key: String,
    pub trace_id: Option<String>,
    pub causation_id: Option<String>,
    pub correlation_id: Option<String>,
    pub created_at: Option<String>,
}

impl ProtocolMetadata {
    pub fn new(idempotency_key: impl Into<String>) -> Self {
        Self {
            protocol_version: "coat.v1".to_string(),
            idempotency_key: idempotency_key.into(),
            trace_id: None,
            causation_id: None,
            correlation_id: None,
            created_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStateAuthority {
    RestateWorkflow,
    ExternalGoalStore,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalReadModelBackend {
    Postgres,
    PostgresPgvector,
    Sqlite,
    Jsonl,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalEventBackend {
    RestateJournal,
    PostgresOutbox,
    Kafka,
    Redpanda,
    Nats,
    Jsonl,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStoreProjectionMode {
    Disabled,
    BestEffort,
    Required,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalRecord {
    pub goal_id: GoalId,
    pub title: String,
    pub objective: String,
    pub repo: Option<String>,
    pub status: GoalStatus,
    pub total_tasks: u32,
    pub open_tasks: u32,
    pub blocked_tasks: u32,
    pub failed_tasks: u32,
    pub percent_done: f32,
    pub root_task_id: Option<TaskId>,
    pub satisfied: bool,
    pub satisfaction_score: Option<f32>,
    pub updated_at: Option<String>,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TaskRecord {
    pub goal_id: GoalId,
    pub task_id: TaskId,
    pub parent_task_id: Option<TaskId>,
    pub subgoal_id: Option<String>,
    pub title: String,
    pub role: WorkerKind,
    pub status: TaskStatus,
    pub purpose_kind: TaskPurposeKind,
    pub depth: u32,
    pub priority: TaskPriority,
    pub priority_rank: u8,
    pub attempts: u32,
    pub runnable: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    pub result_uri: Option<String>,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalEventKind {
    Submitted,
    StateProjected,
    TaskStarted,
    TaskCompleted,
    TaskBlocked,
    ApprovalRequested,
    ApprovalDecided,
    SteeringApplied,
    ValidationRecorded,
    ArtifactRecorded,
    Cancelled,
    Failed,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalEventRecord {
    pub event_id: Uuid,
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub sequence: u64,
    pub kind: GoalEventKind,
    pub message: String,
    pub actor: Option<String>,
    pub idempotency_key: String,
    pub created_at: Option<String>,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ApprovalRecord {
    pub approval_id: Uuid,
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub status: ApprovalStatus,
    pub risk: ApprovalRisk,
    pub reason: String,
    pub requested_action: String,
    pub updated_at: Option<String>,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalArtifactRecord {
    pub goal_id: GoalId,
    pub task_id: Option<TaskId>,
    pub artifact: ArtifactRef,
    pub git_result: Option<GitResultRef>,
    pub object_artifact: Option<ObjectStorageArtifactRef>,
    pub created_at: Option<String>,
    #[serde(default)]
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalStoreSnapshot {
    pub goal: GoalRecord,
    #[serde(default)]
    pub tasks: Vec<TaskRecord>,
    #[serde(default)]
    pub artifacts: Vec<GoalArtifactRecord>,
    #[serde(default)]
    pub approvals: Vec<ApprovalRecord>,
    #[serde(default)]
    pub events: Vec<GoalEventRecord>,
    #[serde(default)]
    pub full_state_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalStoreSnapshotUpsertRequest {
    pub metadata: ProtocolMetadata,
    pub projection_reason: String,
    pub snapshot: GoalStoreSnapshot,
}

impl GoalStoreSnapshotUpsertRequest {
    pub fn from_state(state: &GoalState, projection_reason: impl Into<String>) -> Self {
        let projection_reason = projection_reason.into();
        let snapshot = GoalStoreSnapshot::from_state(state);
        Self {
            metadata: ProtocolMetadata::new(format!(
                "goal:{}:projection:{}:{}",
                state.goal.id,
                projection_reason,
                snapshot
                    .events
                    .last()
                    .map(|event| event.sequence)
                    .unwrap_or(0)
            )),
            projection_reason,
            snapshot,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalStoreSnapshotUpsertResponse {
    pub accepted: bool,
    pub goal: GoalRecord,
    pub task_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalStoreEventAppendRequest {
    pub metadata: ProtocolMetadata,
    pub event: GoalEventRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct GoalStoreEventAppendResponse {
    pub accepted: bool,
    pub sequence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalStoreGoalResponse {
    pub found: bool,
    pub goal: Option<GoalRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalStoreTaskListResponse {
    pub goal_id: GoalId,
    #[serde(default)]
    pub tasks: Vec<TaskRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalStoreEventListResponse {
    pub goal_id: GoalId,
    #[serde(default)]
    pub events: Vec<GoalEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct GoalStoreArtifactListResponse {
    pub goal_id: GoalId,
    #[serde(default)]
    pub artifacts: Vec<GoalArtifactRecord>,
}

impl GoalStoreSnapshot {
    pub fn from_state(state: &GoalState) -> Self {
        let progress = state.progress();
        let root_task_id = state
            .tasks
            .values()
            .find(|task| task.parent_id.is_none())
            .map(|task| task.id);
        let artifacts = state
            .final_artifacts
            .iter()
            .cloned()
            .map(|artifact| GoalArtifactRecord {
                goal_id: state.goal.id,
                task_id: None,
                artifact,
                git_result: None,
                object_artifact: None,
                created_at: None,
                payload_json: serde_json::Value::Null,
            })
            .collect();
        let approvals = state
            .approvals
            .iter()
            .cloned()
            .map(ApprovalRecord::from)
            .collect();
        let events = state
            .events
            .iter()
            .enumerate()
            .map(|(index, event)| GoalEventRecord::from_state_event(state.goal.id, index, event))
            .collect();
        Self {
            goal: GoalRecord {
                goal_id: state.goal.id,
                title: state.goal.title.clone(),
                objective: state.goal.objective.clone(),
                repo: state.goal.repo.clone(),
                status: state.status.clone(),
                total_tasks: progress.total_tasks,
                open_tasks: progress.open_tasks,
                blocked_tasks: progress.blocked_tasks,
                failed_tasks: progress.failed_tasks,
                percent_done: progress.percent_done,
                root_task_id,
                satisfied: state
                    .satisfaction
                    .as_ref()
                    .is_some_and(|report| report.satisfied),
                satisfaction_score: state.satisfaction.as_ref().map(|report| report.score),
                updated_at: None,
                payload_json: to_json_value(&state.goal),
            },
            tasks: state
                .tasks
                .values()
                .map(|task| TaskRecord::from_task(state, task))
                .collect(),
            artifacts,
            approvals,
            events,
            full_state_json: to_json_value(state),
        }
    }
}

impl TaskRecord {
    pub fn from_task(state: &GoalState, task: &TaskNode) -> Self {
        let progress = state.task_progress(task);
        Self {
            goal_id: task.goal_id,
            task_id: task.id,
            parent_task_id: task.parent_id,
            subgoal_id: task.subgoal_id.clone(),
            title: progress.title,
            role: task.role.clone(),
            status: task.status.clone(),
            purpose_kind: TaskPurposeKind::from(&task.purpose),
            depth: task.depth,
            priority: task.priority.clone(),
            priority_rank: task.priority.rank(),
            attempts: task.attempts,
            runnable: progress.runnable,
            tags: task.tags.clone(),
            result_uri: task.result.as_ref().map(|artifact| artifact.uri.clone()),
            payload_json: to_json_value(task),
        }
    }
}

impl GoalEventRecord {
    fn from_state_event(goal_id: GoalId, index: usize, event: &StateEvent) -> Self {
        let sequence = if event.sequence == 0 {
            index as u64 + 1
        } else {
            event.sequence
        };
        Self {
            event_id: Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("coat://goal/{goal_id}/event/{sequence}/{}", event.message).as_bytes(),
            ),
            goal_id,
            task_id: None,
            sequence,
            kind: classify_state_event(&event.message),
            message: event.message.clone(),
            actor: Some("coordinator".to_string()),
            idempotency_key: format!("goal:{goal_id}:event:{sequence}"),
            created_at: None,
            payload_json: to_json_value(event),
        }
    }
}

impl From<ApprovalRequest> for ApprovalRecord {
    fn from(value: ApprovalRequest) -> Self {
        Self {
            approval_id: value.id,
            goal_id: value.goal_id,
            task_id: value.task_id,
            status: value.status.clone(),
            risk: value.risk.clone(),
            reason: value.reason.clone(),
            requested_action: value.requested_action.clone(),
            updated_at: None,
            payload_json: to_json_value(&value),
        }
    }
}

fn classify_state_event(message: &str) -> GoalEventKind {
    if message == "goal_started" {
        GoalEventKind::Submitted
    } else if message.starts_with("validation_passed") || message.starts_with("validation_failed") {
        GoalEventKind::ValidationRecorded
    } else if message.starts_with("approval_") {
        GoalEventKind::ApprovalDecided
    } else if message.starts_with("cancelled:") {
        GoalEventKind::Cancelled
    } else if message.starts_with("task_blocked") {
        GoalEventKind::TaskBlocked
    } else if message.starts_with("task_completed") {
        GoalEventKind::TaskCompleted
    } else {
        GoalEventKind::StateProjected
    }
}

fn to_json_value<T: Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
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
    #[error("steering denied: {0}")]
    SteeringDenied(String),
    #[error("restart denied: {0}")]
    RestartDenied(String),
    #[error("branch denied: {0}")]
    BranchDenied(String),
    #[error("branch group not found: {0}")]
    BranchGroupNotFound(Uuid),
    #[error("approval not found: {0}")]
    ApprovalNotFound(Uuid),
    #[error("approval denied: {0}")]
    ApprovalDenied(String),
    #[error("cycle detected from task: {0}")]
    CycleDetected(TaskId),
    #[error("invariant violation: {0}")]
    InvariantViolation(String),
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
    fn initial_tasks_become_queryable_subgoal_tasks() {
        let mut goal = GoalSpec::new(
            "planned",
            "implement structured planning and progress reporting",
        );
        goal.plan.subgoals.push(SubgoalSpec {
            id: "implementation".to_string(),
            title: "Implementation".to_string(),
            objective: "Implement the contract".to_string(),
            owner_role: WorkerKind::Codex,
            priority: TaskPriority::Critical,
            dependencies: Vec::new(),
            tags: vec!["progress".to_string()],
            acceptance_evidence: vec!["tests pass".to_string()],
        });
        goal.initial_tasks.push(ChildTaskRequest {
            role: WorkerKind::Codex,
            purpose: Some(TaskPurpose::Work),
            title: Some("Implement progress".to_string()),
            subgoal_id: Some("implementation".to_string()),
            prompt: "implement progress contract".to_string(),
            reason: "known first implementation task".to_string(),
            dependencies: Vec::new(),
            budget: None,
            sandbox: None,
            done_criteria: None,
            execution: None,
            priority: TaskPriority::Critical,
            tags: vec!["progress".to_string()],
        });

        let state = GoalState::new(goal);
        let progress = state.progress();

        assert_eq!(state.tasks.len(), 2);
        assert_eq!(progress.total_tasks, 2);
        assert_eq!(progress.subgoals.len(), 1);
        assert_eq!(progress.subgoals[0].subgoal_id, "implementation");
        assert_eq!(progress.subgoals[0].total_tasks, 1);

        let task_list = state.find_tasks(&TaskQuery {
            subgoal_id: Some("implementation".to_string()),
            tags: vec!["progress".to_string()],
            ..TaskQuery::default()
        });
        assert_eq!(task_list.tasks.len(), 1);
        assert_eq!(task_list.tasks[0].title, "Implement progress");
    }

    #[test]
    fn goal_quality_report_flags_vague_goals() {
        let mut goal = GoalSpec::new("x", "fix");
        goal.done_criteria.tests_pass = false;
        goal.done_criteria.artifact_exists = false;
        goal.done_criteria.validator_score_min = None;

        let report = goal.quality_report();

        assert!(!report.ready);
        assert!(
            report
                .missing
                .iter()
                .any(|missing| missing.contains("objective"))
        );
        assert!(
            report
                .missing
                .iter()
                .any(|missing| missing.contains("done criteria"))
        );
    }

    #[test]
    fn spawn_policy_blocks_depth_overflow() {
        let goal = GoalSpec::new("test", "do the thing");
        let mut parent = GoalState::new(goal).runnable_tasks().remove(0);
        parent.depth = 8;
        let request = ChildTaskRequest {
            role: WorkerKind::Tester,
            purpose: None,
            title: None,
            subgoal_id: None,
            prompt: "test".to_string(),
            reason: "coverage".to_string(),
            dependencies: Vec::new(),
            budget: None,
            sandbox: None,
            done_criteria: None,
            execution: None,
            priority: TaskPriority::Normal,
            tags: Vec::new(),
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
    fn git_result_satisfies_artifact_evidence() {
        let mut goal = GoalSpec::new("git", "write results through a git branch");
        goal.default_execution.results.git.enabled = true;
        let state = GoalState::new(goal);
        let task = state.runnable_tasks().remove(0);
        let mut result = AgentRunResult::stub_done(&task);
        result.artifacts.clear();

        let report = ValidationReport::from_result(ValidationRequest {
            goal_id: task.goal_id,
            task,
            result,
        });

        assert!(report.passed);
        assert!(report.git_result.is_some());
        assert!(
            report
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == ArtifactKind::GitBranch)
        );
    }

    #[test]
    fn validation_preserves_blocked_worker_status() {
        let state = GoalState::new(GoalSpec::new("test", "do the thing"));
        let task = state.runnable_tasks().remove(0);
        let result = AgentRunResult {
            status: WorkerRunStatus::Blocked,
            artifacts: Vec::new(),
            confidence: 0.0,
            ..AgentRunResult::stub_done(&task)
        };
        let report = ValidationReport::from_result(ValidationRequest {
            goal_id: task.goal_id,
            task,
            result,
        });
        assert!(!report.passed);
        assert_eq!(report.status_after_validation, TaskStatus::Blocked);
    }

    #[test]
    fn child_task_inherits_execution_profile_with_new_role() {
        let goal = GoalSpec::new("test", "do the thing");
        let mut state = GoalState::new(goal);
        let parent = state.runnable_tasks().remove(0);
        let result = AgentRunResult {
            child_requests: vec![ChildTaskRequest {
                role: WorkerKind::Tester,
                purpose: None,
                title: Some("Test child".to_string()),
                subgoal_id: Some("tests".to_string()),
                prompt: "test it".to_string(),
                reason: "need evidence".to_string(),
                dependencies: Vec::new(),
                budget: None,
                sandbox: None,
                done_criteria: None,
                execution: None,
                priority: TaskPriority::Normal,
                tags: vec!["tests".to_string()],
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
                    namespace: Some("coat".to_string()),
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
            coordinator_node_id: None,
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
    fn model_route_can_disallow_unlisted_fallbacks() {
        let mut goal = GoalSpec::new("test", "do the thing");
        goal.default_execution.model = ModelRoute {
            strategy: ModelRoutingStrategy::FirstAvailable,
            required_features: vec![ModelFeature::ToolUse],
            candidates: vec![ModelCandidate {
                provider: ModelProviderKind::Vllm,
                model: "specific-local-model".to_string(),
                endpoint: Some("http://vllm:8000/v1".to_string()),
                priority: 10,
                weight: 1,
                context_window: None,
                features: vec![ModelFeature::ToolUse],
                labels: BTreeMap::new(),
            }],
            fallback: ModelFallbackPolicy::DisallowFallback,
        };
        let task = GoalState::new(goal).runnable_tasks().remove(0);
        let registration = RunnerRegistration {
            runner_id: "runner-a".to_string(),
            node_id: "node-1".to_string(),
            endpoint: "http://runner-a:9099".to_string(),
            roles: vec![WorkerKind::Planner],
            capabilities: Vec::new(),
            models: vec![ModelCandidate {
                provider: ModelProviderKind::OpenAiCompatible,
                model: "other-model".to_string(),
                endpoint: Some("http://router:8000/v1".to_string()),
                priority: 20,
                weight: 1,
                context_window: None,
                features: vec![ModelFeature::ToolUse],
                labels: BTreeMap::new(),
            }],
            labels: BTreeMap::new(),
            mcp_servers: Vec::new(),
            max_concurrency: 1,
            lease_ttl_seconds: 300,
        };

        let decision = RunnerDispatchDecision::choose(RunnerDispatchRequest {
            goal_id: task.goal_id,
            task,
            coordinator_node_id: None,
            registered_runners: vec![registration],
        });
        assert_eq!(decision.status, RunnerDispatchStatus::NoMatch);
    }

    #[test]
    fn dispatch_ranks_candidates_by_model_strategy() {
        let mut goal = GoalSpec::new("routing", "pick the best critic model");
        goal.default_execution.model = ModelRoute {
            strategy: ModelRoutingStrategy::HighestQuality,
            required_features: vec![ModelFeature::ToolUse, ModelFeature::JsonSchema],
            candidates: vec![
                ModelCandidate {
                    provider: ModelProviderKind::OpenAiCompatible,
                    model: "local-small-fast".to_string(),
                    endpoint: Some("http://router:8000/v1".to_string()),
                    priority: 5,
                    weight: 1,
                    context_window: Some(32_768),
                    features: vec![ModelFeature::ToolUse, ModelFeature::JsonSchema],
                    labels: BTreeMap::from([("quality_tier".to_string(), "medium".to_string())]),
                },
                ModelCandidate {
                    provider: ModelProviderKind::Vllm,
                    model: "qwen3-coder-30b".to_string(),
                    endpoint: Some("http://vllm:8000/v1".to_string()),
                    priority: 20,
                    weight: 1,
                    context_window: Some(131_072),
                    features: vec![
                        ModelFeature::ToolUse,
                        ModelFeature::JsonSchema,
                        ModelFeature::LongContext,
                    ],
                    labels: BTreeMap::from([("quality_tier".to_string(), "high".to_string())]),
                },
            ],
            fallback: ModelFallbackPolicy::AllowFallback,
        };
        let task = GoalState::new(goal).runnable_tasks().remove(0);
        let small_runner = RunnerRegistration {
            runner_id: "small".to_string(),
            node_id: "cpu-node".to_string(),
            endpoint: "http://small:9091".to_string(),
            roles: vec![WorkerKind::Planner],
            capabilities: Vec::new(),
            models: vec![task.execution.model.candidates[0].clone()],
            labels: BTreeMap::new(),
            mcp_servers: Vec::new(),
            max_concurrency: 1,
            lease_ttl_seconds: 300,
        };
        let qwen_runner = RunnerRegistration {
            runner_id: "qwen".to_string(),
            node_id: "gpu-node".to_string(),
            endpoint: "http://qwen:9091".to_string(),
            roles: vec![WorkerKind::Planner],
            capabilities: Vec::new(),
            models: vec![task.execution.model.candidates[1].clone()],
            labels: BTreeMap::new(),
            mcp_servers: Vec::new(),
            max_concurrency: 1,
            lease_ttl_seconds: 300,
        };

        let decision = RunnerDispatchDecision::choose(RunnerDispatchRequest {
            goal_id: task.goal_id,
            task,
            coordinator_node_id: None,
            registered_runners: vec![small_runner, qwen_runner],
        });

        assert_eq!(decision.status, RunnerDispatchStatus::Matched);
        assert_eq!(decision.runner_id.as_deref(), Some("qwen"));
        assert_eq!(decision.candidates.len(), 2);
        assert_eq!(
            decision.model.expect("selected model").model,
            "qwen3-coder-30b"
        );
    }

    #[test]
    fn dispatch_rejections_explain_locality_and_mcp_mismatches() {
        let mut goal = GoalSpec::new("mcp", "requires local MCP context");
        goal.default_execution.runner.locality = RunnerLocality::SameNode;
        goal.default_execution.mcp.servers = vec![McpServerRef {
            name: "tool-registry".to_string(),
            transport: McpTransport::Http,
            uri: "http://tool-registry:9084/mcp".to_string(),
            allowed_tools: vec!["repo_status".to_string()],
            auth: McpAuthRef::None,
        }];
        let task = GoalState::new(goal).runnable_tasks().remove(0);
        let remote_without_mcp = RunnerRegistration {
            runner_id: "remote".to_string(),
            node_id: "remote-node".to_string(),
            endpoint: "http://remote:9091".to_string(),
            roles: vec![WorkerKind::Planner],
            capabilities: Vec::new(),
            models: task.execution.model.candidates.clone(),
            labels: BTreeMap::new(),
            mcp_servers: Vec::new(),
            max_concurrency: 1,
            lease_ttl_seconds: 300,
        };

        let decision = RunnerDispatchDecision::choose(RunnerDispatchRequest {
            goal_id: task.goal_id,
            task,
            coordinator_node_id: Some("control-node".to_string()),
            registered_runners: vec![remote_without_mcp],
        });

        assert_eq!(decision.status, RunnerDispatchStatus::NoMatch);
        let reasons = &decision.rejections[0].reasons;
        assert!(reasons.iter().any(|reason| reason.contains("locality")));
        assert!(reasons.iter().any(|reason| reason.contains("mcp_tools")));
    }

    #[test]
    fn restart_request_requeues_blocked_task() {
        let mut state = GoalState::new(GoalSpec::new(
            "restart",
            "restart blocked tasks without creating a new workflow",
        ));
        let root_id = state.runnable_tasks().remove(0).id;
        state.tasks.get_mut(&root_id).unwrap().status = TaskStatus::Blocked;
        state.status = GoalStatus::Blocked;

        let record = state
            .apply_restart_request(RestartRequest {
                goal_id: state.goal.id,
                scope: RestartScope::Task,
                reason: RestartReason::OperatorRequested,
                message: "try the task again after config repair".to_string(),
                task_id: Some(root_id),
                reset_attempts: Some(true),
                preserve_artifacts: Some(true),
                operator: Some("test".to_string()),
            })
            .expect("restart applied");

        assert_eq!(record.restarted_task_ids, vec![root_id]);
        assert_eq!(state.tasks[&root_id].status, TaskStatus::Runnable);
        assert_eq!(state.status, GoalStatus::Running);
    }

    #[test]
    fn task_timeout_can_restart_under_policy() {
        let mut state = GoalState::new(GoalSpec::new(
            "timeout",
            "restart timed out work when the policy permits it",
        ));
        let root_id = state.runnable_tasks().remove(0).id;
        state.tasks.get_mut(&root_id).unwrap().status = TaskStatus::Failed;

        let restarted = state
            .record_task_timeout_and_maybe_restart(root_id, 10, "runner timed out")
            .expect("timeout policy");

        assert!(restarted);
        assert_eq!(state.tasks[&root_id].status, TaskStatus::Runnable);
        assert_eq!(state.timeout_events.len(), 1);
        assert_eq!(state.restart_history.len(), 1);
    }

    #[test]
    fn branch_group_spawns_candidates_votes_and_auto_selects() {
        let mut goal = GoalSpec::new(
            "branch",
            "let multiple candidate implementations compete and select a winner",
        );
        goal.review_policy.enabled = false;
        goal.branching_policy.enabled = true;
        goal.branching_policy.default_selection_strategy = BranchSelectionStrategy::VoterQuorum;
        goal.branching_policy.voting.min_votes = 1;
        goal.branching_policy.voting.require_unification = false;

        let mut state = GoalState::new(goal);
        let root_id = state.runnable_tasks().remove(0).id;
        let group = state
            .branch_task(
                BranchRequest {
                    goal_id: state.goal.id,
                    target_task_id: Some(root_id),
                    subgoal_id: None,
                    reason: "compare two implementations".to_string(),
                    candidate_count: 2,
                    candidate_roles: Vec::new(),
                    candidate_executions: Vec::new(),
                    prompt_overrides: Vec::new(),
                    selection_strategy: Some(BranchSelectionStrategy::VoterQuorum),
                    operator: Some("test".to_string()),
                },
                &SpawnPolicy::default(),
            )
            .expect("branch group");

        assert_eq!(group.candidate_task_ids.len(), 2);
        assert_eq!(state.tasks[&root_id].status, TaskStatus::Cancelled);

        for candidate_id in group.candidate_task_ids.clone() {
            let candidate = state.tasks[&candidate_id].clone();
            let result = AgentRunResult::stub_done(&candidate);
            state
                .apply_agent_result(result.clone(), &SpawnPolicy::default())
                .expect("candidate result");
            state
                .apply_validation(ValidationReport::from_result(ValidationRequest {
                    goal_id: candidate.goal_id,
                    task: candidate,
                    result,
                }))
                .expect("candidate validation");
        }

        assert!(
            state
                .ensure_branch_frontier(&SpawnPolicy::default())
                .expect("spawn vote")
        );
        let vote_task = state
            .tasks
            .values()
            .find(|task| matches!(task.purpose, TaskPurpose::BranchVote { .. }))
            .cloned()
            .expect("vote task");
        let vote_result = AgentRunResult::stub_done(&vote_task);
        state
            .apply_agent_result(vote_result.clone(), &SpawnPolicy::default())
            .expect("vote result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: vote_task.goal_id,
                task: vote_task,
                result: vote_result,
            }))
            .expect("vote validation");
        state
            .ensure_branch_frontier(&SpawnPolicy::default())
            .expect("auto select");

        let selected_group = state
            .branch_groups
            .iter()
            .find(|candidate| candidate.id == group.id)
            .expect("branch group");
        assert_eq!(selected_group.status, BranchGroupStatus::Selected);
        assert_eq!(
            selected_group.selected_task_id,
            group.candidate_task_ids.first().copied()
        );
    }

    #[test]
    fn review_frontier_forks_reviews_and_joins_unification() {
        let mut state = GoalState::new(GoalSpec::new("reviewed", "ship reviewed work"));
        let root = state.runnable_tasks().remove(0);
        let actor_result = AgentRunResult::stub_done(&root);
        state
            .apply_agent_result(actor_result.clone(), &SpawnPolicy::default())
            .expect("actor result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: root.goal_id,
                task: root,
                result: actor_result,
            }))
            .expect("actor validation");

        assert!(!state.is_done());
        assert!(
            state
                .ensure_review_frontier(&SpawnPolicy::default())
                .expect("spawn review")
        );
        let review = state
            .tasks
            .values()
            .find(|task| task.purpose.is_review())
            .cloned()
            .expect("review task");
        assert_eq!(review.role, WorkerKind::Reviewer);

        let review_result = AgentRunResult::stub_done(&review);
        state
            .apply_agent_result(review_result.clone(), &SpawnPolicy::default())
            .expect("review result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: review.goal_id,
                task: review,
                result: review_result,
            }))
            .expect("review validation");

        assert!(
            state
                .ensure_review_frontier(&SpawnPolicy::default())
                .expect("spawn unification")
        );
        let unifier = state
            .tasks
            .values()
            .find(|task| task.purpose.is_unification())
            .cloned()
            .expect("unifier task");
        assert_eq!(unifier.role, WorkerKind::PatchMerger);

        let unifier_result = AgentRunResult::stub_done(&unifier);
        state
            .apply_agent_result(unifier_result.clone(), &SpawnPolicy::default())
            .expect("unifier result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: unifier.goal_id,
                task: unifier,
                result: unifier_result,
            }))
            .expect("unifier validation");

        assert_eq!(state.status, GoalStatus::Done);
        assert!(state.satisfaction.expect("satisfaction").satisfied);
        assert_eq!(state.learning_signals.len(), 3);
    }

    #[test]
    fn low_satisfaction_spawns_bounded_actor_retry_then_second_review_round() {
        let mut goal = GoalSpec::new("retry", "improve until reviewed");
        goal.review_policy.min_satisfaction_score = 0.95;
        goal.review_policy.actor_critic.reward_threshold = 0.85;
        goal.review_policy.max_review_rounds = 2;
        let mut state = GoalState::new(goal);

        let root = state.runnable_tasks().remove(0);
        let actor_result = AgentRunResult::stub_done(&root);
        state
            .apply_agent_result(actor_result.clone(), &SpawnPolicy::default())
            .expect("actor result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: root.goal_id,
                task: root,
                result: actor_result,
            }))
            .expect("actor validation");
        state
            .ensure_review_frontier(&SpawnPolicy::default())
            .expect("spawn review");

        let review = state
            .tasks
            .values()
            .find(|task| task.purpose.is_review())
            .cloned()
            .expect("review task");
        let review_result = AgentRunResult::stub_done(&review);
        state
            .apply_agent_result(review_result.clone(), &SpawnPolicy::default())
            .expect("review result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: review.goal_id,
                task: review,
                result: review_result,
            }))
            .expect("review validation");
        state
            .ensure_review_frontier(&SpawnPolicy::default())
            .expect("spawn unifier");

        let unifier = state
            .tasks
            .values()
            .find(|task| task.purpose.is_unification())
            .cloned()
            .expect("unifier task");
        let unifier_result = AgentRunResult::stub_done(&unifier);
        state
            .apply_agent_result(unifier_result.clone(), &SpawnPolicy::default())
            .expect("unifier result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: unifier.goal_id,
                task: unifier,
                result: unifier_result,
            }))
            .expect("unifier validation");

        assert!(
            state
                .ensure_review_frontier(&SpawnPolicy::default())
                .expect("spawn actor retry")
        );
        let retry = state
            .tasks
            .values()
            .find(|task| matches!(task.purpose, TaskPurpose::ActorRetry { .. }))
            .cloned()
            .expect("actor retry task");
        assert_eq!(retry.role, WorkerKind::Planner);

        let retry_result = AgentRunResult::stub_done(&retry);
        state
            .apply_agent_result(retry_result.clone(), &SpawnPolicy::default())
            .expect("retry result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: retry.goal_id,
                task: retry,
                result: retry_result,
            }))
            .expect("retry validation");

        assert!(
            state
                .ensure_review_frontier(&SpawnPolicy::default())
                .expect("spawn second review round")
        );
        assert_eq!(state.review_rounds.len(), 2);
    }

    #[test]
    fn changes_requested_review_blocks_satisfaction_even_with_high_reward() {
        let mut state = GoalState::new(GoalSpec::new("changes", "review should block"));
        let root = state.runnable_tasks().remove(0);
        let actor_result = AgentRunResult::stub_done(&root);
        state
            .apply_agent_result(actor_result.clone(), &SpawnPolicy::default())
            .expect("actor result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: root.goal_id,
                task: root,
                result: actor_result,
            }))
            .expect("actor validation");
        state
            .ensure_review_frontier(&SpawnPolicy::default())
            .expect("spawn review");

        let review = state
            .tasks
            .values()
            .find(|task| task.purpose.is_review())
            .cloned()
            .expect("review task");
        let mut review_result = AgentRunResult::stub_done(&review);
        review_result.review = Some(ReviewOutput {
            decision: ReviewDecision::ChangesRequested,
            reward: 0.99,
            findings: vec![ReviewFinding {
                severity: ReviewSeverity::High,
                title: "Missing proof".to_string(),
                body: "The actor did not provide enough evidence.".to_string(),
                evidence: vec!["memory://evidence".to_string()],
                suggested_action: Some("Add evidence before accepting.".to_string()),
            }],
            retry_recommended: true,
            unification_summary: None,
        });
        state
            .apply_agent_result(review_result.clone(), &SpawnPolicy::default())
            .expect("review result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: review.goal_id,
                task: review,
                result: review_result,
            }))
            .expect("review validation");

        let report = state.satisfaction_report();
        assert!(!report.satisfied);
        assert_eq!(
            report.latest_decision,
            Some(ReviewDecision::ChangesRequested)
        );
        assert_eq!(report.open_findings, 1);
    }

    #[test]
    fn changes_requested_unifier_spawns_actor_retry_even_with_high_reward() {
        let mut state = GoalState::new(GoalSpec::new(
            "retry on decision",
            "review decision should drive retry",
        ));
        let root = state.runnable_tasks().remove(0);
        let actor_result = AgentRunResult::stub_done(&root);
        state
            .apply_agent_result(actor_result.clone(), &SpawnPolicy::default())
            .expect("actor result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: root.goal_id,
                task: root,
                result: actor_result,
            }))
            .expect("actor validation");
        state
            .ensure_review_frontier(&SpawnPolicy::default())
            .expect("spawn review");

        let review = state
            .tasks
            .values()
            .find(|task| task.purpose.is_review())
            .cloned()
            .expect("review task");
        let review_result = AgentRunResult::stub_done(&review);
        state
            .apply_agent_result(review_result.clone(), &SpawnPolicy::default())
            .expect("review result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: review.goal_id,
                task: review,
                result: review_result,
            }))
            .expect("review validation");
        state
            .ensure_review_frontier(&SpawnPolicy::default())
            .expect("spawn unifier");

        let unifier = state
            .tasks
            .values()
            .find(|task| task.purpose.is_unification())
            .cloned()
            .expect("unifier task");
        let mut unifier_result = AgentRunResult::stub_done(&unifier);
        unifier_result.review = Some(ReviewOutput {
            decision: ReviewDecision::ChangesRequested,
            reward: 0.99,
            findings: vec![ReviewFinding {
                severity: ReviewSeverity::Medium,
                title: "Needs iteration".to_string(),
                body: "The unifier requested one more actor pass.".to_string(),
                evidence: vec!["memory://review".to_string()],
                suggested_action: Some("Run bounded actor retry.".to_string()),
            }],
            retry_recommended: true,
            unification_summary: Some("retry required".to_string()),
        });
        state
            .apply_agent_result(unifier_result.clone(), &SpawnPolicy::default())
            .expect("unifier result");
        state
            .apply_validation(ValidationReport::from_result(ValidationRequest {
                goal_id: unifier.goal_id,
                task: unifier,
                result: unifier_result,
            }))
            .expect("unifier validation");

        assert!(
            state
                .ensure_review_frontier(&SpawnPolicy::default())
                .expect("spawn retry")
        );
        assert!(
            state
                .tasks
                .values()
                .any(|task| matches!(task.purpose, TaskPurpose::ActorRetry { .. }))
        );
    }

    #[test]
    fn steering_can_inject_research_task() {
        let mut state = GoalState::new(GoalSpec::new("steer", "allow guided research"));
        let directive = SteeringDirective {
            id: Uuid::new_v4(),
            goal_id: state.goal.id,
            task_id: None,
            operator: Some("operator".to_string()),
            message: "research memory substrate".to_string(),
            kind: SteeringDirectiveKind::RequestResearch {
                question: "which memory layer should we use?".to_string(),
                reason: "need sourced decision".to_string(),
            },
        };

        state
            .apply_steering(directive, &SpawnPolicy::default())
            .expect("steering applies");

        let research = state
            .tasks
            .values()
            .find(|task| task.purpose.is_research())
            .expect("research task");
        assert_eq!(research.role, WorkerKind::Research);
        assert_eq!(state.steering_directives.len(), 1);
    }

    #[test]
    fn default_isolated_restricted_task_does_not_need_approval() {
        let mut state = GoalState::new(GoalSpec::new("approval", "default task"));
        let task_id = state.runnable_tasks().remove(0).id;
        let request = state
            .ensure_task_approval_or_request(task_id)
            .expect("approval evaluation succeeds");

        assert!(request.is_none());
        assert!(state.approvals.is_empty());
        assert_eq!(
            state.task(task_id).expect("task").status,
            TaskStatus::Runnable
        );
    }

    #[test]
    fn network_open_task_waits_for_approval_then_resumes() {
        let mut state = GoalState::new(GoalSpec::new("approval", "network open task"));
        let task_id = state.runnable_tasks().remove(0).id;
        state.task_mut(task_id).expect("task").sandbox.network = NetworkAccess::Open;

        let request = state
            .ensure_task_approval_or_request(task_id)
            .expect("approval evaluation succeeds")
            .expect("approval requested");

        assert_eq!(request.status, ApprovalStatus::Pending);
        assert_eq!(request.risk, ApprovalRisk::High);
        assert!(
            request
                .reason_codes
                .contains(&ApprovalReasonCode::NetworkOpen)
        );
        assert_eq!(
            state.task(task_id).expect("task").status,
            TaskStatus::WaitingApproval
        );
        assert_eq!(state.status, GoalStatus::WaitingApproval);

        state
            .apply_human_approval(HumanApproval {
                approval_id: request.id,
                approved: true,
                note: Some("network is expected for this task".to_string()),
            })
            .expect("approval applies");

        assert_eq!(
            state.task(task_id).expect("task").status,
            TaskStatus::Runnable
        );
        assert!(
            state
                .ensure_task_approval_or_request(task_id)
                .expect("approved attempt is allowed")
                .is_none()
        );
    }

    #[test]
    fn never_policy_outside_isolation_is_forced_to_critical_approval() {
        let mut state = GoalState::new(GoalSpec::new("approval", "unsafe never policy"));
        let task_id = state.runnable_tasks().remove(0).id;
        let task = state.task_mut(task_id).expect("task");
        task.sandbox.approval_policy = ApprovalPolicy::Never;
        task.sandbox.isolated_runner = false;

        let request = state
            .ensure_task_approval_or_request(task_id)
            .expect("approval evaluation succeeds")
            .expect("approval requested");

        assert_eq!(request.risk, ApprovalRisk::Critical);
        assert!(
            request
                .reason_codes
                .contains(&ApprovalReasonCode::NeverPolicyOutsideIsolation)
        );
    }

    #[test]
    fn rejected_approval_blocks_task() {
        let mut state = GoalState::new(GoalSpec::new("approval", "reject dangerous task"));
        let task_id = state.runnable_tasks().remove(0).id;
        state.task_mut(task_id).expect("task").sandbox.network = NetworkAccess::Open;
        let request = state
            .ensure_task_approval_or_request(task_id)
            .expect("approval evaluation succeeds")
            .expect("approval requested");

        state
            .apply_human_approval(HumanApproval {
                approval_id: request.id,
                approved: false,
                note: Some("network access is not allowed".to_string()),
            })
            .expect("rejection applies");

        assert_eq!(
            state.task(task_id).expect("task").status,
            TaskStatus::Blocked
        );
        assert_eq!(state.status, GoalStatus::Blocked);
    }

    #[test]
    fn device_auth_session_counts_as_secret_access_without_distribution() {
        let mut state = GoalState::new(GoalSpec::new("approval", "node local Codex auth"));
        let task_id = state.runnable_tasks().remove(0).id;
        state
            .task_mut(task_id)
            .expect("task")
            .execution
            .mcp
            .servers
            .push(McpServerRef {
                name: "codex-app-server".to_string(),
                transport: McpTransport::Http,
                uri: "http://codex-runner:9091/app-server".to_string(),
                allowed_tools: vec!["thread.run".to_string()],
                auth: McpAuthRef::DeviceAuthSession {
                    session_ref: SecretRef {
                        provider: SecretProvider::LocalFile,
                        name: "/var/lib/coat/codex/auth.json".to_string(),
                        key: None,
                        namespace: Some("node-a".to_string()),
                        audience: Some("codex".to_string()),
                    },
                    refresh_ref: None,
                    provider: DeviceAuthProvider::Codex,
                    node_local: true,
                },
            });

        let request = state
            .ensure_task_approval_or_request(task_id)
            .expect("approval evaluation succeeds")
            .expect("approval requested");

        assert_eq!(request.risk, ApprovalRisk::High);
        assert!(
            request
                .reason_codes
                .contains(&ApprovalReasonCode::SecretAccess)
        );
        assert!(
            !request
                .reason_codes
                .contains(&ApprovalReasonCode::BrokeredUserAuth)
        );
    }

    #[test]
    fn brokered_user_auth_requires_critical_approval() {
        let mut state = GoalState::new(GoalSpec::new("approval", "broker Claude auth"));
        let task_id = state.runnable_tasks().remove(0).id;
        let task = state.task_mut(task_id).expect("task");
        task.execution.mcp.auth_distribution.mode = AuthDistributionMode::OAuthDeviceBroker;
        task.execution.mcp.servers.push(McpServerRef {
            name: "claude-code".to_string(),
            transport: McpTransport::Stdio,
            uri: "claude".to_string(),
            allowed_tools: vec!["issue_to_pr".to_string()],
            auth: McpAuthRef::BrokeredUserSession {
                broker: SecretRef {
                    provider: SecretProvider::ExternalBroker,
                    name: "human-auth-broker".to_string(),
                    key: Some("claude".to_string()),
                    namespace: Some("coat".to_string()),
                    audience: Some("claude-code".to_string()),
                },
                provider: DeviceAuthProvider::ClaudeCode,
                requested_scopes: vec!["inference".to_string()],
            },
        });

        let request = state
            .ensure_task_approval_or_request(task_id)
            .expect("approval evaluation succeeds")
            .expect("approval requested");

        assert_eq!(request.risk, ApprovalRisk::Critical);
        assert!(
            request
                .reason_codes
                .contains(&ApprovalReasonCode::SecretAccess)
        );
        assert!(
            request
                .reason_codes
                .contains(&ApprovalReasonCode::BrokeredUserAuth)
        );
    }

    #[test]
    fn auth_distribution_labels_constrain_runner_matching() {
        let mut goal = GoalSpec::new("dispatch", "node-local auth should route to tagged runner");
        goal.default_execution.runner.worker = Some(WorkerKind::Planner);
        goal.default_execution.mcp.auth_distribution.mode = AuthDistributionMode::RunnerLocalOnly;
        goal.default_execution
            .mcp
            .auth_distribution
            .required_runner_labels
            .insert("auth.codex.device".to_string(), "true".to_string());
        let task = GoalState::new(goal).runnable_tasks().remove(0);
        let base_runner = RunnerRegistration {
            runner_id: "codex-a".to_string(),
            node_id: "node-a".to_string(),
            endpoint: "http://codex-a:9091".to_string(),
            roles: vec![WorkerKind::Planner],
            capabilities: vec![RunnerCapability::Code],
            models: task.execution.model.candidates.clone(),
            labels: BTreeMap::new(),
            mcp_servers: Vec::new(),
            max_concurrency: 1,
            lease_ttl_seconds: 300,
        };

        let missing = base_runner.evaluate_for_task(&task, None);
        assert!(!missing.matched);
        assert!(missing.reasons.iter().any(|reason| {
            reason.contains("runner lacks MCP auth distribution label auth.codex.device=true")
        }));

        let mut tagged_runner = base_runner;
        tagged_runner
            .labels
            .insert("auth.codex.device".to_string(), "true".to_string());
        let tagged = tagged_runner.evaluate_for_task(&task, None);
        assert!(tagged.matched, "{:?}", tagged.reasons);
    }

    #[test]
    fn research_validation_requires_sources_and_use_plan() {
        let state = GoalState::new(GoalSpec::new("research", "answer sourced question"));
        let mut task = state.runnable_tasks().remove(0);
        task.purpose = TaskPurpose::Research {
            question: "what should we use?".to_string(),
        };
        task.done_criteria.validator_score_min = Some(0.75);
        let mut result = AgentRunResult::stub_done(&task);
        result.research = Some(ResearchOutput {
            question: "what should we use?".to_string(),
            answer: "insufficient".to_string(),
            sources: Vec::new(),
            confidence: 0.8,
            use_plan: InformationUsePlan {
                facts_to_use: Vec::new(),
                facts_to_avoid: Vec::new(),
                proposed_task_updates: Vec::new(),
                validation_checks: Vec::new(),
            },
            open_questions: Vec::new(),
        });

        let report = ValidationReport::from_result(ValidationRequest {
            goal_id: task.goal_id,
            task,
            result,
        });

        assert!(!report.passed);
        assert!(
            report
                .missing_criteria
                .contains(&"research_sources".to_string())
        );
        assert!(
            report
                .missing_criteria
                .contains(&"information_use_plan".to_string())
        );
    }

    #[test]
    fn examples_parse_against_domain_contracts() {
        serde_json::from_str::<GoalSpec>(include_str!("../../../examples/goal-vllm-mcp.json"))
            .expect("goal-vllm-mcp example parses");
        serde_json::from_str::<GoalSpec>(include_str!(
            "../../../examples/goal-template-structured.json"
        ))
        .expect("goal-template-structured example parses");
        serde_json::from_str::<GoalSpec>(include_str!("../../../examples/goal-clean-plan.json"))
            .expect("goal-clean-plan example parses");
        serde_json::from_str::<GoalSpec>(include_str!(
            "../../../examples/goal-branching-competition.json"
        ))
        .expect("goal-branching-competition example parses");
        serde_json::from_str::<RestartRequest>(include_str!(
            "../../../examples/restart-request-task.json"
        ))
        .expect("restart-request example parses");
        serde_json::from_str::<BranchRequest>(include_str!(
            "../../../examples/branch-request-root.json"
        ))
        .expect("branch-request example parses");
        serde_json::from_str::<BranchSelectionRequest>(include_str!(
            "../../../examples/branch-selection.json"
        ))
        .expect("branch-selection example parses");
        serde_json::from_str::<TaskQuery>(include_str!(
            "../../../examples/task-query-subgoal.json"
        ))
        .expect("task-query example parses");
        serde_json::from_str::<RunnerRegistration>(include_str!(
            "../../../examples/runner-vllm.json"
        ))
        .expect("runner-vllm example parses");
        serde_json::from_str::<RunnerDispatchRequest>(include_str!(
            "../../../examples/dispatch-smoke.json"
        ))
        .expect("dispatch-smoke example parses");
        serde_json::from_str::<AgentRunRequest>(include_str!(
            "../../../examples/agent-run-smoke.json"
        ))
        .expect("agent-run-smoke example parses");
        serde_json::from_str::<ReviewOutput>(include_str!(
            "../../../examples/review-output-changes-requested.json"
        ))
        .expect("review-output example parses");
        serde_json::from_str::<SteeringDirective>(include_str!(
            "../../../examples/steering-request-research.json"
        ))
        .expect("steering example parses");
        serde_json::from_str::<ResearchOutput>(include_str!(
            "../../../examples/research-output-memory-substrate.json"
        ))
        .expect("research-output example parses");
        serde_json::from_str::<MemoryWriteRequest>(include_str!(
            "../../../examples/memory-write-fact.json"
        ))
        .expect("memory-write example parses");
        serde_json::from_str::<MemorySearchRequest>(include_str!(
            "../../../examples/memory-search.json"
        ))
        .expect("memory-search example parses");
        serde_json::from_str::<MemoryContextRequest>(include_str!(
            "../../../examples/memory-context.json"
        ))
        .expect("memory-context example parses");
        serde_json::from_str::<MemoryJoinRequest>(include_str!(
            "../../../examples/memory-join.json"
        ))
        .expect("memory-join example parses");
        serde_json::from_str::<MemoryRepairRequest>(include_str!(
            "../../../examples/memory-repair.json"
        ))
        .expect("memory-repair example parses");
        serde_json::from_str::<McpContextRef>(include_str!(
            "../../../examples/auth-distribution-codex-device.json"
        ))
        .expect("codex device auth distribution example parses");
        serde_json::from_str::<McpContextRef>(include_str!(
            "../../../examples/auth-distribution-claude-brokered.json"
        ))
        .expect("claude brokered auth distribution example parses");
        serde_json::from_str::<NotificationRequest>(include_str!(
            "../../../examples/notification-approval.json"
        ))
        .expect("notification example parses");
        serde_json::from_str::<EventSource>(include_str!(
            "../../../examples/event-source-calendar-schedule.json"
        ))
        .expect("calendar event-source example parses");
        serde_json::from_str::<EventSource>(include_str!(
            "../../../examples/event-source-webhook-hmac.json"
        ))
        .expect("webhook hmac event-source example parses");
        serde_json::from_str::<ExternalEvent>(include_str!(
            "../../../examples/external-event-calendar.json"
        ))
        .expect("external-event example parses");
        serde_json::from_str::<TriggeredGoalRequest>(include_str!(
            "../../../examples/triggered-goal-webhook.json"
        ))
        .expect("triggered-goal example parses");
    }
}
