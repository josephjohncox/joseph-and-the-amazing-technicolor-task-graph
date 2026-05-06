# 070 Sandbox Tooling

## Objective

Create safe workspace lifecycle and deterministic Rust tool surfaces for workers.

## Implementation

- Create isolated workspace directories per task.
- Apply sandbox profiles for filesystem, network, and approval policy.
- Snapshot workspaces after task runs.
- Clean workspaces by goal/task ID.
- Expose repo status, test command, and artifact manifest tools through the tool registry.

## Implemented

- `coat-sandbox-runner` creates deterministic workspace IDs from goal/task IDs.
- `/workspaces` creates a per-task directory under `SANDBOX_WORKSPACE_ROOT`, writes `workspace-manifest.json`, and records a registry file for later snapshot/cleanup.
- `/snapshot` writes `snapshots/latest.json` when the workspace is known and is safe to call repeatedly.
- `/cleanup` removes the workspace and registry record only when the path is inside `SANDBOX_WORKSPACE_ROOT`; repeated cleanup returns `not_found`.
- Git and object-storage result refs are returned as contracts without mutating a source repository or uploading blobs.

## Tests

- Workspace creation returns a stable artifact reference.
- Snapshot and cleanup endpoints are idempotent.
- Tool registry lists known tools.
- Dangerous commands require approval metadata.

## Follow-Ups

- Add an approval-gated live git worktree creation mode that runs only for explicitly approved local repositories.
- Add content-addressed snapshot archives for object storage.
- Route approved test commands through sandbox runner instead of the MCP tool registry.

## Acceptance

- Sandbox runner and tool registry have health checks.
- Compose and Kubernetes expose internal service names for both.
