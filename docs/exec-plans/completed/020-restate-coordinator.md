# 020 Restate Coordinator

## Objective

Make Restate the durable outer loop for goals and task-tree state.

## Implementation

- Expose `GoalWorkflow` with `run`, `cancel`, `inject_feedback`, `approve`, `restart`, `branch`, `select_branch`, `status`, `progress`, and `tasks`.
- Persist `GoalState` after every task transition.
- Call `AgentRunner` and `ValidationService` through durable Restate service calls.
- Use terminal errors for budget and policy failures that must not retry forever.
- Model feedback as a shared workflow signal.
- Model approvals as durable state transitions: create `ApprovalRequest`, notify, pause task dispatch, apply `HumanApproval`, and resume the frontier loop after acceptance.
- Apply runner call timeouts as structured worker results, then let restart policy decide whether a task is requeued.
- Add branch competition frontier behavior: candidate branches, vote tasks, optional unification, and selected implementation records.
- Emit structured coordinator transition observations for projection reasons, task-status counts, pending approvals, and terminal task mix before optional goal-store projection.

## Tests

- Stub workflow accepts a goal and returns completed state.
- Workflow state is queryable through `status`.
- Cancel accepts a signal; approval updates durable state and resumes accepted work.
- Restart requeues matching tasks and resumes the frontier loop.
- Branch and select-branch mutate durable state and project the updated goal store snapshot.
- Transition observation tests cover approval pauses and terminal outcome counts.
- Restart-resume integration test is added once Restate testcontainers are introduced.

## Follow-Ups

- Superseded by the active master plan:
  `docs/exec-plans/active/160-live-durable-runtime-and-execution.md`.
  RuntimeVerifier owns the remaining Docker Testcontainers restart/resume proof,
  transition/projection observation assertions, and future OpenTelemetry sink
  checks.

## Acceptance

- `coat-coordinator` starts on `:9080`.
- Restate discovery can register the coordinator endpoint.
