# 020 Restate Coordinator

## Objective

Make Restate the durable outer loop for goals and task-tree state.

## Implementation

- Expose `GoalWorkflow` with `run`, `cancel`, `inject_feedback`, `approve`, and `status`.
- Persist `GoalState` after every task transition.
- Call `AgentRunner` and `ValidationService` through durable Restate service calls.
- Use terminal errors for budget and policy failures that must not retry forever.
- Model approval and feedback as shared workflow signals.

## Tests

- Stub workflow accepts a goal and returns completed state.
- Workflow state is queryable through `status`.
- Cancel and approval handlers accept signals.
- Restart-resume integration test is added once Restate testcontainers are introduced.

## Acceptance

- `jattg-coordinator` starts on `:9080`.
- Restate discovery can register the coordinator endpoint.
