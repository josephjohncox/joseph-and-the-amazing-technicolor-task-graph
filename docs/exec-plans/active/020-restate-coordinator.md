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

## Tests

- Stub workflow accepts a goal and returns completed state.
- Workflow state is queryable through `status`.
- Cancel accepts a signal; approval updates durable state and resumes accepted work.
- Restart requeues matching tasks and resumes the frontier loop.
- Branch and select-branch mutate durable state and project the updated goal store snapshot.
- Restart-resume integration test is added once Restate testcontainers are introduced.

## Follow-Ups

- Add restart/resume integration tests with a real Restate runtime or testcontainer once the test harness is selected.
- Add metrics and trace assertions for durable fanout, approval pauses, restarts, validation retries, and projection failures.

## Acceptance

- `coat-coordinator` starts on `:9080`.
- Restate discovery can register the coordinator endpoint.
