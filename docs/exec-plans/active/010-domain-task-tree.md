# 010 Domain Task Tree

## Objective

Implement the shared Rust contracts that every coordinator, worker, validator, and CLI command uses.

## Implementation

- Define `GoalSpec`, `GoalState`, `TaskNode`, `TaskStatus`, `WorkerKind`, `Budget`, `SandboxProfile`, `DoneCriteria`, and `ArtifactRef`.
- Define worker and validator I/O contracts.
- Implement frontier selection, budget exhaustion checks, spawn policy, cancellation, and validation state transitions.
- Generate JSON schemas into `schemas/`.

## Tests

- New goal creates a runnable planner root.
- Spawn policy rejects depth and child-count overflow.
- Validation enforces artifact and score criteria.
- Schema generation round-trips all public contracts.

## Acceptance

- `cargo test -p jattg-domain` passes.
- `cargo run -p jattg-domain --bin generate-schemas -- schemas` writes schemas.
