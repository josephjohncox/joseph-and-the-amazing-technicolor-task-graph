# 020 Restate Coordinator

## Objective

Make Restate the durable outer loop for goals and task-tree state.

## Implementation

- Expose `GoalWorkflow` with `run`, `cancel`, `inject_feedback`, `approve`, and `status`.
- Persist `GoalState` after every task transition.
- Call `AgentRunner` and `ValidationService` through durable Restate service calls.
- Use terminal errors for budget and policy failures that must not retry forever.
- Model feedback as a shared workflow signal.
- Model approvals as durable state transitions: create `ApprovalRequest`, notify, pause task dispatch, apply `HumanApproval`, and resume the frontier loop after acceptance.

## Tests

- Stub workflow accepts a goal and returns completed state.
- Workflow state is queryable through `status`.
- Cancel accepts a signal; approval updates durable state and resumes accepted work.
- Restart-resume integration test is added once Restate testcontainers are introduced.

## Acceptance

- `coat-coordinator` starts on `:9080`.
- Restate discovery can register the coordinator endpoint.
