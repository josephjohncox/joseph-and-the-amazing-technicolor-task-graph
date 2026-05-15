# 010 Domain Task Tree

## Objective

Implement the shared Rust contracts that every coordinator, worker, validator, and CLI command uses.

## Implementation

- Define `GoalSpec`, `GoalState`, `TaskNode`, `TaskStatus`, `WorkerKind`, `Budget`, `SandboxProfile`, `DoneCriteria`, and `ArtifactRef`.
- Define worker and validator I/O contracts.
- Implement frontier selection, budget exhaustion checks, spawn policy, cancellation, and validation state transitions.
- Add `RestartPolicy`, `TimeoutPolicy`, `BranchingPolicy`, `BranchGroup`, `BranchVoteOutput`, and branch selection contracts.
- Add `GraphColorRef` and optional subgoal/task color hints for the Technicolor Task Graph. Color is presentation metadata, not coordinator policy.
- Generate JSON schemas into `schemas/`.

## Tests

- New goal creates a runnable planner root.
- Spawn policy rejects depth and child-count overflow.
- Validation enforces artifact and score criteria.
- Restart requests requeue blocked tasks under policy.
- Timeout policy can restart a task after a timed-out runner result.
- Branch groups spawn candidate tasks, vote tasks, and durable selections.
- Technicolor graph colors are assigned as optional visual hints for root, subgoal, and child tasks.
- Schema generation round-trips all public contracts.

## Follow-Ups

None currently. Future lifecycle contract growth, schema alignment, and
projection proof are tracked by the active master plan.

## Acceptance

- `cargo test -p coat-domain` passes.
- `make schemas` writes schemas.
