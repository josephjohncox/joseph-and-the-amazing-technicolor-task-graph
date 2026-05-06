# 070 Sandbox Tooling

## Objective

Create safe workspace lifecycle and deterministic Rust tool surfaces for workers.

## Implementation

- Create isolated git workspaces per task.
- Apply sandbox profiles for filesystem, network, and approval policy.
- Snapshot workspaces after task runs.
- Clean workspaces by goal/task ID.
- Expose repo status, test command, and artifact manifest tools through the tool registry.

## Tests

- Workspace creation returns a stable artifact reference.
- Snapshot and cleanup endpoints are idempotent.
- Tool registry lists known tools.
- Dangerous commands require approval metadata.

## Acceptance

- Sandbox runner and tool registry have health checks.
- Compose and Kubernetes expose internal service names for both.
